// Orquestación de una descarga completa: elegir formatos, descargar y unir.
// Es lo que comparten la CLI y la interfaz gráfica.

use crate::sources::download::download_file;
use crate::sources::youtube::{self, Format, Kind, VideoInfo};
use crate::media::Tags;
use crate::sources::spotify::Track;
use crate::media::flac::FlacWriter;
use crate::media::mp3::Mp3Writer;
use crate::media::wav::WavWriter;
use crate::media::{mp4, pcm};
use crate::sources::matching;
use crate::{Cancelled, Res};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Video,
    Audio,
    Muxing,
}

/// Contenedor del archivo final.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Output {
    /// MP4 (vídeo+audio) o M4A (solo audio), según el plan
    Mp4,
    /// AAC crudo (.aac), solo audio
    Adts,
    /// PCM sin comprimir (.wav), solo audio
    Wav,
    /// Sin pérdida (.flac), solo audio
    Flac,
    /// MP3 (.mp3) a los kbps indicados (0 = 192), solo audio
    Mp3(u32),
}

impl Output {
    /// Extensión del archivo. `audio_only` distingue MP4 (vídeo) de M4A (solo audio en MP4).
    pub fn extension(self, audio_only: bool) -> &'static str {
        match self {
            Output::Mp4 if audio_only => "m4a",
            Output::Mp4 => "mp4",
            Output::Adts => "aac",
            Output::Wav => "wav",
            Output::Flac => "flac",
            Output::Mp3(_) => "mp3",
        }
    }
}

/// Cómo escribir el resultado: contenedor y etiquetas opcionales.
#[derive(Clone, Copy)]
pub struct Target<'a> {
    pub output: Output,
    pub tags: Option<&'a Tags>,
}

impl Target<'_> {
    pub const MP4: Target<'static> = Target { output: Output::Mp4, tags: None };
}

pub enum Plan {
    /// MP4 ya completo de YouTube (vídeo+audio, hasta 360p): se guarda tal cual.
    Direct(Format),
    /// Vídeo y audio por separado (o solo audio) que hay que unir con nuestro muxer.
    Merge { video: Option<Format>, audio: Format },
}

impl Plan {
    /// Bytes previstos (vídeo, audio) si se conocen todos; sirve para un progreso global.
    pub fn planned_bytes(&self) -> Option<(u64, u64)> {
        let (v, a) = match self {
            Plan::Direct(f) => (f.size, 0),
            Plan::Merge { video, audio } => (video.as_ref().map_or(0, |v| v.size), audio.size),
        };
        let known = match self {
            Plan::Direct(_) => v > 0,
            Plan::Merge { video, .. } => a > 0 && video.as_ref().is_none_or(|f| f.size > 0),
        };
        known.then_some((v, a))
    }
}

pub fn plan(info: &VideoInfo, max_height: u32, audio_only: bool) -> Res<Plan> {
    if !audio_only && max_height <= 360 {
        if let Some(f) = info.formats.iter().find(|f| f.kind == Kind::Muxed && f.height <= max_height) {
            return Ok(Plan::Direct(f.clone()));
        }
    }
    let (video, audio) = youtube::pick_formats(info, max_height, audio_only)?;
    Ok(Plan::Merge { video, audio })
}

pub fn safe_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let name = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let name: String = name.chars().take(150).collect();
    if name.is_empty() {
        "video".into()
    } else {
        name
    }
}

pub fn default_output(info: &VideoInfo, dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{}.{ext}", safe_name(&info.title)))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// Ejecuta el plan y deja el resultado en `out`. `on_progress(etapa, hecho, total)` puede
/// llamarse desde varios hilos. Si `cancel` pasa a true se aborta con `Cancelled` y no queda
/// ningún archivo temporal.
pub fn execute(
    agent: &ureq::Agent,
    info: &VideoInfo,
    plan: &Plan,
    out: &Path,
    target: Target,
    on_progress: &(dyn Fn(Stage, u64, u64) + Sync),
    cancel: &AtomicBool,
) -> Res<()> {
    let Target { output, tags } = target;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }

    // Todo se escribe en un temporal y solo se renombra al destino si termina bien: un fallo o
    // una cancelación nunca tocan un archivo que ya existiera con ese nombre.
    let tmp = with_suffix(out, ".part");
    let result = match plan {
        Plan::Direct(f) => download_file(agent, &f.url, &tmp, f.size, info.ua, &|d, t| on_progress(Stage::Video, d, t), cancel),
        Plan::Merge { video, audio } => {
            let mut parts: Vec<PathBuf> = Vec::new();
            let result = (|| -> Res<()> {
                if let Some(v) = video {
                    parts.push(with_suffix(out, ".video.part"));
                    let dest = parts.last().unwrap();
                    download_file(agent, &v.url, dest, v.size, info.ua, &|d, t| on_progress(Stage::Video, d, t), cancel)?;
                }
                parts.push(with_suffix(out, ".audio.part"));
                let dest = parts.last().unwrap();
                download_file(agent, &audio.url, dest, audio.size, info.ua, &|d, t| on_progress(Stage::Audio, d, t), cancel)?;

                if cancel.load(Ordering::Relaxed) {
                    return Err(Box::new(Cancelled));
                }
                on_progress(Stage::Muxing, 0, 0);
                let audio = parts.last().unwrap();
                match output {
                    Output::Mp4 => mp4::mux_mp4(&parts, &tmp, tags, |_, _| {}).map(|_| ())?,
                    Output::Adts => mp4::extract_aac_adts(audio, &tmp).map(|_| ())?,
                    Output::Wav => pcm::decode_aac_to(audio, &mut WavWriter::create(&tmp, tags)?)?,
                    Output::Flac => pcm::decode_aac_to(audio, &mut FlacWriter::create(&tmp, tags)?)?,
                    Output::Mp3(kbps) => pcm::decode_aac_to(audio, &mut Mp3Writer::create(&tmp, tags, kbps)?)?,
                }
                Ok(())
            })();
            for p in &parts {
                let _ = std::fs::remove_file(p);
            }
            result
        }
    };
    match result {
        Ok(()) => Ok(std::fs::rename(&tmp, out)?),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---------- canciones de Spotify (metadatos de Spotify, audio de YouTube) ----------

/// Qué vídeo de YouTube se eligió para una canción y con qué confianza.
#[derive(Debug, Clone)]
pub struct Matched {
    pub video_id: String,
    pub title: String,
    pub score: f64,
}

/// Busca la canción en YouTube y devuelve los mejores candidatos (título, artista y duración),
/// del mejor al peor. Si la primera consulta no da una coincidencia muy clara, se prueba otra y
/// se juntan los resultados.
pub fn find_on_youtube(agent: &ureq::Agent, track: &Track) -> Res<Vec<Matched>> {
    let artists = track.artists.join(" ");
    let queries = [format!("{artists} - {}", track.title), format!("{} {} topic", track.main_artist(), track.title)];
    let mut pool: Vec<youtube::SearchResult> = Vec::new();
    let mut ranked = Vec::new();
    for q in &queries {
        for r in youtube::search(agent, q)? {
            if !pool.iter().any(|p| p.id == r.id) {
                pool.push(r);
            }
        }
        ranked = matching::ranked(track, &pool);
        if ranked.first().is_some_and(|b| b.score >= 100.0) {
            break;
        }
    }
    if ranked.is_empty() {
        return Err(format!("No encontré «{}» en YouTube con un título y una duración parecidos", track.label()).into());
    }
    Ok(ranked.into_iter().take(4).map(|b| Matched { video_id: b.result.id, title: b.result.title, score: b.score.min(100.0) }).collect())
}

fn fetch_bytes(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let resp = agent.get(url).call().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let mut buf = Vec::new();
    resp.into_body().into_reader().take(10 * 1024 * 1024).read_to_end(&mut buf).ok()?;
    (!buf.is_empty()).then_some(buf)
}

/// Descarga una canción de Spotify en el formato de audio pedido (M4A, AAC, WAV, FLAC o MP3), con
/// sus etiquetas y su portada.
/// Si el mejor candidato no se puede bajar completo (YouTube corta algunos vídeos), prueba el
/// siguiente. Devuelve la ruta del archivo y el vídeo de YouTube que se usó.
pub fn download_spotify_track(
    agent: &ureq::Agent,
    track: &Track,
    dir: &Path,
    output: Option<&Path>,
    format: Output,
    on_progress: &(dyn Fn(Stage, u64, u64) + Sync),
    cancel: &AtomicBool,
) -> Res<(PathBuf, Matched)> {
    let candidates = find_on_youtube(agent, track)?;
    let tags = Tags {
        title: track.title.clone(),
        artist: track.artists.join(", "),
        album: track.album.clone(),
        year: track.year.clone(),
        cover: track.cover_url.as_deref().and_then(|u| fetch_bytes(agent, u)),
    };
    let ext = format.extension(true);
    let out = output.map(Path::to_path_buf).unwrap_or_else(|| dir.join(format!("{}.{ext}", safe_name(&track.label()))));

    let mut last_error: Box<dyn std::error::Error + Send + Sync> = "Sin candidatos".into();
    for matched in candidates {
        if cancel.load(Ordering::Relaxed) {
            return Err(Box::new(Cancelled));
        }
        let attempt = || -> Res<()> {
            let info = youtube::get_video_info(agent, &matched.video_id)?;
            let plan = plan(&info, 0, true)?;
            execute(agent, &info, &plan, &out, Target { output: format, tags: Some(&tags) }, on_progress, cancel)
        };
        match attempt() {
            Ok(()) => return Ok((out, matched)),
            Err(e) if e.downcast_ref::<Cancelled>().is_some() => return Err(e),
            Err(e) => last_error = e, // se prueba con el siguiente candidato
        }
    }
    Err(last_error)
}
