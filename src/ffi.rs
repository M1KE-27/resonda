// API C del núcleo (ver include/resonda.h). La usa la interfaz gráfica de Qt.
//
// Reglas: ningún pánico cruza la frontera C (se convierten en error), y toda cadena que
// devuelve `char*` es propiedad del llamante y se libera con `rs_string_free`.

// El contrato de seguridad de cada función (punteros válidos, propiedad, hilos) está
// documentado en include/resonda.h, que es la referencia para quien la use desde C.
#![allow(clippy::missing_safety_doc)]

use crate::pipeline::{self, Output, Stage, Target};
use crate::sources::spotify::{self, Collection};
use crate::sources::youtube::{self, Kind, VideoInfo};
use crate::{new_agent, Cancelled, Res};
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub const RS_OK: c_int = 0;
pub const RS_ERROR: c_int = 1;
pub const RS_CANCELLED: c_int = 2;

pub const RS_FORMAT_MP4: c_int = 0;
pub const RS_FORMAT_M4A: c_int = 1;
pub const RS_FORMAT_AAC: c_int = 2;
pub const RS_FORMAT_WAV: c_int = 3;
pub const RS_FORMAT_FLAC: c_int = 4;
pub const RS_FORMAT_MP3: c_int = 5;

/// Formato numérico de la API C -> contenedor de salida y si es solo audio.
fn output_of(format: c_int, kbps: u32) -> Res<(Output, bool)> {
    Ok(match format {
        RS_FORMAT_MP4 => (Output::Mp4, false),
        RS_FORMAT_M4A => (Output::Mp4, true),
        RS_FORMAT_AAC => (Output::Adts, true),
        RS_FORMAT_WAV => (Output::Wav, true),
        RS_FORMAT_FLAC => (Output::Flac, true),
        RS_FORMAT_MP3 => (Output::Mp3(kbps), true),
        other => return Err(format!("Formato desconocido: {other}").into()),
    })
}

pub struct RsInfo {
    agent: ureq::Agent,
    info: VideoInfo,
    title: CString,
    id: CString,
}

pub struct RsCancel(AtomicBool);

#[repr(C)]
pub struct RsOptions {
    /// Altura máxima en píxeles (0 = la mejor disponible)
    pub max_height: u32,
    /// Formato de salida: RS_FORMAT_MP4, _M4A, _AAC, _WAV, _FLAC o _MP3
    pub format: c_int,
    /// Solo MP3: calidad en kbps (128, 192, 256, 320); 0 = 192
    pub bitrate_kbps: u32,
    /// Carpeta de salida (UTF-8); NULL = carpeta actual
    pub output_dir: *const c_char,
}

/// stage: 0 = vídeo, 1 = audio, 2 = uniendo. `done`/`total` son bytes del conjunto de la
/// descarga (no de la etapa). Puede llamarse desde varios hilos a la vez.
pub type RsProgressCb = extern "C" fn(stage: c_int, done: u64, total: u64, user: *mut c_void);

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

fn to_c(s: &str) -> CString {
    CString::new(s.replace('\0', " ")).unwrap_or_default()
}

unsafe fn set_err(out: *mut *mut c_char, msg: &str) {
    if !out.is_null() {
        *out = to_c(msg).into_raw();
    }
}

/// Ejecuta `f` sin dejar escapar pánicos hacia C.
fn guarded<T>(f: impl FnOnce() -> Res<T>) -> Res<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => Err("Error interno inesperado".into()),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Consulta un vídeo (URL o ID). Devuelve NULL si falla y deja el motivo en `*err`.
#[no_mangle]
pub unsafe extern "C" fn rs_info_fetch(input: *const c_char, err: *mut *mut c_char) -> *mut RsInfo {
    let result = guarded(|| {
        if input.is_null() {
            return Err("URL vacía".into());
        }
        let input = CStr::from_ptr(input).to_str().map_err(|_| "La URL no es UTF-8 válido")?;
        let agent = new_agent();
        let info = youtube::get_video_info(&agent, input.trim())?;
        Ok(Box::new(RsInfo { agent, title: to_c(&info.title), id: to_c(&info.id), info }))
    });
    match result {
        Ok(b) => Box::into_raw(b),
        Err(e) => {
            set_err(err, &e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_info_free(info: *mut RsInfo) {
    if !info.is_null() {
        drop(Box::from_raw(info));
    }
}

/// Título (propiedad de `info`; válido hasta `rs_info_free`).
#[no_mangle]
pub unsafe extern "C" fn rs_info_title(info: *const RsInfo) -> *const c_char {
    info.as_ref().map_or(std::ptr::null(), |i| i.title.as_ptr())
}

/// ID de 11 caracteres (propiedad de `info`).
#[no_mangle]
pub unsafe extern "C" fn rs_info_id(info: *const RsInfo) -> *const c_char {
    info.as_ref().map_or(std::ptr::null(), |i| i.id.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn rs_info_seconds(info: *const RsInfo) -> u64 {
    info.as_ref().map_or(0, |i| i.info.seconds)
}

/// Rellena `out` (capacidad `cap`) con las alturas de vídeo disponibles, de mayor a menor,
/// sin repetir. Devuelve cuántas hay en total (puede ser mayor que `cap`).
#[no_mangle]
pub unsafe extern "C" fn rs_info_heights(info: *const RsInfo, out: *mut u32, cap: usize) -> usize {
    let Some(i) = info.as_ref() else { return 0 };
    let mut heights: Vec<u32> = i
        .info
        .formats
        .iter()
        .filter(|f| f.kind == Kind::Video && (f.codec == "avc1" || f.codec == "av01"))
        .map(|f| f.height)
        .collect();
    heights.sort_unstable_by(|a, b| b.cmp(a));
    heights.dedup();
    if !out.is_null() {
        for (n, h) in heights.iter().take(cap).enumerate() {
            *out.add(n) = *h;
        }
    }
    heights.len()
}

#[no_mangle]
pub extern "C" fn rs_cancel_new() -> *mut RsCancel {
    Box::into_raw(Box::new(RsCancel(AtomicBool::new(false))))
}

/// Pide cancelar la descarga que use este token. Seguro desde cualquier hilo.
#[no_mangle]
pub unsafe extern "C" fn rs_cancel_set(c: *const RsCancel) {
    if let Some(c) = c.as_ref() {
        c.0.store(true, Ordering::Relaxed);
    }
}

/// Deja el token listo para reutilizarlo en otra descarga.
#[no_mangle]
pub unsafe extern "C" fn rs_cancel_reset(c: *const RsCancel) {
    if let Some(c) = c.as_ref() {
        c.0.store(false, Ordering::Relaxed);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_cancel_free(c: *mut RsCancel) {
    if !c.is_null() {
        drop(Box::from_raw(c));
    }
}

/// Descarga el vídeo. Bloquea hasta terminar: llamarla desde un hilo de trabajo.
/// Devuelve RS_OK (y la ruta del archivo en `*out_path`), RS_CANCELLED o RS_ERROR (motivo
/// en `*err`). `cancel` y `progress` pueden ser NULL.
#[no_mangle]
pub unsafe extern "C" fn rs_download(
    info: *const RsInfo,
    opts: *const RsOptions,
    progress: Option<RsProgressCb>,
    user: *mut c_void,
    cancel: *const RsCancel,
    out_path: *mut *mut c_char,
    err: *mut *mut c_char,
) -> c_int {
    let user = SendPtr(user);
    let result = guarded(|| {
        let info = info.as_ref().ok_or("info nula")?;
        let opts = opts.as_ref().ok_or("opciones nulas")?;
        let dir = if opts.output_dir.is_null() {
            PathBuf::from(".")
        } else {
            PathBuf::from(CStr::from_ptr(opts.output_dir).to_str().map_err(|_| "La carpeta no es UTF-8 válido")?)
        };
        let (output, audio_only) = output_of(opts.format, opts.bitrate_kbps)?;
        let max_height = if opts.max_height == 0 { u32::MAX } else { opts.max_height };

        let plan = pipeline::plan(&info.info, max_height, audio_only)?;
        let out = pipeline::default_output(&info.info, &dir, output.extension(audio_only));
        let planned = plan.planned_bytes();

        // Progreso global: bytes del conjunto de la descarga, no de cada etapa.
        let user = &user;
        let on_progress = move |stage: Stage, done: u64, total: u64| {
            let Some(cb) = progress else { return };
            let (d, t) = match (planned, stage) {
                (Some((v, a)), Stage::Video) => (done, v + a),
                (Some((v, a)), Stage::Audio) => (v + done, v + a),
                (Some((v, a)), Stage::Muxing) => (v + a, v + a),
                (None, _) => (done, total),
            };
            cb(stage as c_int, d, t, user.0);
        };

        let never = AtomicBool::new(false);
        let flag = cancel.as_ref().map_or(&never, |c| &c.0);
        pipeline::execute(&info.agent, &info.info, &plan, &out, Target { output, tags: None }, &on_progress, flag)?;
        Ok(out)
    });

    match result {
        Ok(path) => {
            if !out_path.is_null() {
                *out_path = to_c(&path.to_string_lossy()).into_raw();
            }
            RS_OK
        }
        Err(e) if e.downcast_ref::<Cancelled>().is_some() => RS_CANCELLED,
        Err(e) => {
            set_err(err, &e.to_string());
            RS_ERROR
        }
    }
}

// ---------- Spotify (metadatos de Spotify; el audio se descarga de YouTube) ----------

pub struct RsSpotify {
    agent: ureq::Agent,
    collection: Collection,
    name: CString,
    cover: CString,
    labels: Vec<CString>,
}

/// 1 si `input` es un enlace o URI de Spotify (canción, álbum o playlist).
#[no_mangle]
pub unsafe extern "C" fn rs_is_spotify_link(input: *const c_char) -> c_int {
    if input.is_null() {
        return 0;
    }
    CStr::from_ptr(input).to_str().ok().and_then(spotify::parse_link).is_some() as c_int
}

/// Lee los datos (no el audio) de un enlace de Spotify. Devuelve NULL si falla (motivo en `*err`).
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_fetch(input: *const c_char, err: *mut *mut c_char) -> *mut RsSpotify {
    let result = guarded(|| {
        if input.is_null() {
            return Err("Enlace vacío".into());
        }
        let text = CStr::from_ptr(input).to_str().map_err(|_| "El enlace no es UTF-8 válido")?;
        let (kind, id) = spotify::parse_link(text).ok_or("No es un enlace de Spotify válido")?;
        let agent = new_agent();
        let collection = spotify::fetch(&agent, kind, &id)?;
        let cover = collection.tracks.first().and_then(|t| t.cover_url.clone()).unwrap_or_default();
        let labels = collection.tracks.iter().map(|t| to_c(&t.label())).collect();
        Ok(Box::new(RsSpotify { agent, name: to_c(&collection.name), cover: to_c(&cover), labels, collection }))
    });
    match result {
        Ok(b) => Box::into_raw(b),
        Err(e) => {
            set_err(err, &e.to_string());
            std::ptr::null_mut()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rs_spotify_free(s: *mut RsSpotify) {
    if !s.is_null() {
        drop(Box::from_raw(s));
    }
}

/// 0 = canción, 1 = álbum, 2 = playlist
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_kind(s: *const RsSpotify) -> c_int {
    match s.as_ref().map(|s| s.collection.kind) {
        Some(spotify::Kind::Album) => 1,
        Some(spotify::Kind::Playlist) => 2,
        _ => 0,
    }
}

/// Nombre de la canción, del álbum o de la playlist (propiedad de `s`).
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_name(s: *const RsSpotify) -> *const c_char {
    s.as_ref().map_or(std::ptr::null(), |s| s.name.as_ptr())
}

/// URL de la portada (propiedad de `s`; puede ser cadena vacía).
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_cover(s: *const RsSpotify) -> *const c_char {
    s.as_ref().map_or(std::ptr::null(), |s| s.cover.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn rs_spotify_count(s: *const RsSpotify) -> usize {
    s.as_ref().map_or(0, |s| s.collection.tracks.len())
}

/// "Artista - Título" de la canción `i` (propiedad de `s`).
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_track_label(s: *const RsSpotify, i: usize) -> *const c_char {
    s.as_ref().and_then(|s| s.labels.get(i)).map_or(std::ptr::null(), |c| c.as_ptr())
}

#[no_mangle]
pub unsafe extern "C" fn rs_spotify_track_seconds(s: *const RsSpotify, i: usize) -> u64 {
    s.as_ref().and_then(|s| s.collection.tracks.get(i)).map_or(0, |t| t.duration_ms / 1000)
}

/// Descarga la canción `i` en el formato pedido, con etiquetas y portada, buscándola en YouTube. Bloquea:
/// llamar desde un hilo de trabajo. `output_dir` es la carpeta de destino. Devuelve RS_OK (ruta en
/// `*out_path`, vídeo de YouTube usado en `*matched`), RS_CANCELLED o RS_ERROR (motivo en `*err`).
#[no_mangle]
pub unsafe extern "C" fn rs_spotify_download(
    s: *const RsSpotify,
    i: usize,
    output_dir: *const c_char,
    format: c_int,
    bitrate_kbps: u32,
    progress: Option<RsProgressCb>,
    user: *mut c_void,
    cancel: *const RsCancel,
    out_path: *mut *mut c_char,
    matched: *mut *mut c_char,
    err: *mut *mut c_char,
) -> c_int {
    let user = SendPtr(user);
    let result = guarded(|| {
        let s = s.as_ref().ok_or("Spotify nulo")?;
        let track = s.collection.tracks.get(i).ok_or("Índice de canción fuera de rango")?;
        let dir = if output_dir.is_null() {
            PathBuf::from(".")
        } else {
            PathBuf::from(CStr::from_ptr(output_dir).to_str().map_err(|_| "La carpeta no es UTF-8 válido")?)
        };
        // Spotify es solo música: el vídeo MP4 se sustituye por M4A
        let (output, _) = output_of(if format == RS_FORMAT_MP4 { RS_FORMAT_M4A } else { format }, bitrate_kbps)?;
        let track = if s.collection.kind == spotify::Kind::Track { track.clone() } else { spotify::refine(&s.agent, track) };
        let user = &user;
        let on_progress = move |stage: Stage, done: u64, total: u64| {
            if let Some(cb) = progress {
                cb(stage as c_int, done, total, user.0);
            }
        };
        let never = AtomicBool::new(false);
        let flag = cancel.as_ref().map_or(&never, |c| &c.0);
        pipeline::download_spotify_track(&s.agent, &track, &dir, None, output, &on_progress, flag)
    });
    match result {
        Ok((path, m)) => {
            if !out_path.is_null() {
                *out_path = to_c(&path.to_string_lossy()).into_raw();
            }
            if !matched.is_null() {
                *matched = to_c(&m.title).into_raw();
            }
            RS_OK
        }
        Err(e) if e.downcast_ref::<Cancelled>().is_some() => RS_CANCELLED,
        Err(e) => {
            set_err(err, &e.to_string());
            RS_ERROR
        }
    }
}
