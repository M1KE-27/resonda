//! Muxer MP4 propio.
//!
//! Entrada: uno o más MP4 fragmentados (moov + moof/mdat, como los que sirve YouTube), cada uno
//! con una sola pista. Salida: un MP4 clásico (no fragmentado, con el moov al principio) con todas
//! las pistas intercaladas, listo para reproducir en cualquier sitio.
//!
//! - `boxes`: bytes y cajas MP4 (lectura con límites y construcción)
//! - `track`: análisis de una pista fragmentada
//! - `build`: tablas de muestras y moov de salida
//! - `ilst`: etiquetas (título, artista, portada)
//! - `aac`: fotogramas AAC y extracción a `.aac`

mod aac;
mod boxes;
mod build;
mod ilst;
mod track;

pub use aac::{extract_aac_adts, AacStream};

use crate::io::read_exact_at;
use crate::media::Tags;
use crate::Res;
use boxes::{bx, find, top_of, u32_at, u8_at, w32};
use build::build_moov;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use track::{read_track, Track};

/// Une varios MP4 fragmentados (una pista cada uno) en un único MP4.
/// Devuelve el tamaño del archivo resultante.
pub fn mux_mp4(inputs: &[PathBuf], out_path: &Path, tags: Option<&Tags>, mut on_progress: impl FnMut(u64, u64)) -> Res<u64> {
    let mut tracks = inputs.iter().map(|p| read_track(p)).collect::<Res<Vec<_>>>()?;

    // Intercala los fragmentos de todas las pistas por tiempo
    let mut entries: Vec<(usize, usize)> =
        tracks.iter().enumerate().flat_map(|(ti, t)| (0..t.chunks.len()).map(move |ci| (ti, ci))).collect();
    entries.sort_by(|a, b| {
        let (ta, tb) = (tracks[a.0].chunks[a.1].time, tracks[b.0].chunks[b.1].time);
        ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0))
    });
    let mut total = 0u64;
    for &(ti, ci) in &entries {
        let c = &mut tracks[ti].chunks[ci];
        c.rel = total;
        total += c.len;
    }

    let mut brands = vec![b"isom".to_vec(), b"iso2".to_vec(), b"mp41".to_vec()];
    for t in &tracks {
        if &t.fourcc == b"avc1" || &t.fourcc == b"av01" {
            brands.push(t.fourcc.to_vec());
        }
    }
    let ftyp = bx(b"ftyp", vec![b"isom".to_vec(), w32([512]), brands.concat()]);

    let first = &tracks[0];
    let mvhd = find(&first.moov, &top_of(&first.moov), b"mvhd")?;
    let movie_ts = u32_at(&first.moov, mvhd.body + if u8_at(&first.moov, mvhd.body)? == 1 { 20 } else { 12 })?;

    // El tamaño del moov depende de si los offsets caben en 32 bits, y los offsets dependen
    // del tamaño del moov: se resuelve iterando.
    let big_mdat = total + 8 > 0xffff_ffff;
    let mdat_head: u64 = if big_mdat { 16 } else { 8 };
    let mut use64 = false;
    let mut moov = None;
    for _ in 0..3 {
        let set_offsets = |tracks: &mut Vec<Track>, start: u64| {
            for &(ti, ci) in &entries {
                let c = &mut tracks[ti].chunks[ci];
                c.out = start + c.rel;
            }
        };
        set_offsets(&mut tracks, 0);
        let size = build_moov(&tracks, movie_ts, use64, tags)?.len() as u64;
        let data_start = ftyp.len() as u64 + size + mdat_head;
        if !use64 && data_start + total > 0xffff_ffff {
            use64 = true;
            continue;
        }
        set_offsets(&mut tracks, data_start);
        let m = build_moov(&tracks, movie_ts, use64, tags)?;
        if m.len() as u64 == size {
            moov = Some(m);
            break;
        }
    }
    let moov = moov.ok_or("No se pudo calcular la estructura del MP4")?;

    let mut mdat = Vec::new();
    if big_mdat {
        mdat.extend_from_slice(&1u32.to_be_bytes());
        mdat.extend_from_slice(b"mdat");
        mdat.extend_from_slice(&(total + 16).to_be_bytes());
    } else {
        mdat.extend_from_slice(&((total + 8) as u32).to_be_bytes());
        mdat.extend_from_slice(b"mdat");
    }

    let inputs: Vec<File> = tracks.iter().map(|t| File::open(&t.path)).collect::<Result<_, _>>()?;
    let mut out = File::create(out_path)?;
    out.write_all(&ftyp)?;
    out.write_all(&moov)?;
    out.write_all(&mdat)?;
    let mut buf = Vec::new();
    let mut done = 0u64;
    for &(ti, ci) in &entries {
        let c = &tracks[ti].chunks[ci];
        buf.resize(c.len as usize, 0);
        read_exact_at(&inputs[ti], &mut buf, c.src)?;
        out.write_all(&buf)?;
        done += c.len;
        on_progress(done, total);
    }
    out.flush()?;
    Ok(ftyp.len() as u64 + moov.len() as u64 + mdat_head + total)
}

#[cfg(test)]
mod tests {
    use super::aac::{adts_header, audio_specific_config};
    use super::boxes::{children, full_box};
    use super::*;

    /// MP4 fragmentado mínimo: una pista de "vídeo" con 3 muestras (10+20+30 bytes).
    fn tiny_fragmented() -> (Vec<u8>, Vec<u8>) {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        let mut mdhd = vec![0u8; 24];
        mdhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        let stsd = full_box(b"stsd", 0, 0, vec![w32([1]), bx(b"avc1", vec![vec![0; 78]])]);
        let minf = bx(b"minf", vec![bx(b"stbl", vec![stsd])]);
        let mdia = bx(b"mdia", vec![bx(b"mdhd", vec![mdhd]), bx(b"hdlr", vec![vec![0; 24]]), minf]);
        let trak = bx(b"trak", vec![bx(b"tkhd", vec![vec![0; 84]]), mdia]);
        let mvex = bx(b"mvex", vec![full_box(b"trex", 0, 0, vec![w32([1, 1, 0, 0, 0])])]);
        let moov = bx(b"moov", vec![bx(b"mvhd", vec![mvhd]), trak, mvex]);

        let payload: Vec<u8> = (0..60u8).collect();
        let build_moof = |data_offset: u32| {
            bx(
                b"moof",
                vec![
                    full_box(b"mfhd", 0, 0, vec![w32([1])]),
                    bx(
                        b"traf",
                        vec![
                            full_box(b"tfhd", 0, 0x02_0008, vec![w32([1, 100])]),
                            full_box(b"tfdt", 0, 0, vec![w32([0])]),
                            full_box(b"trun", 0, 0x201, vec![w32([3, data_offset, 10, 20, 30])]),
                        ],
                    ),
                ],
            )
        };
        let moof = build_moof(build_moof(0).len() as u32 + 8);
        let mut file = bx(b"ftyp", vec![b"dash".to_vec(), w32([0])]);
        file.extend(moov);
        file.extend(moof);
        file.extend(bx(b"mdat", vec![payload.clone()]));
        (file, payload)
    }

    fn mux_bytes(bytes: &[u8], name: &str) -> Res<Vec<u8>> {
        let dir = std::env::temp_dir().join(format!("resonda-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let (input, output) = (dir.join("in.mp4"), dir.join("out.mp4"));
        std::fs::write(&input, bytes)?;
        let result = mux_mp4(&[input], &output, None, |_, _| {}).and_then(|_| Ok(std::fs::read(&output)?));
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    #[test]
    fn remux_conserva_los_datos_y_deja_moov_delante() {
        let (input, payload) = tiny_fragmented();
        let out = mux_bytes(&input, "ok").unwrap();
        assert_eq!(&out[4..8], b"ftyp");
        let boxes: Vec<_> = children(&out, 0, out.len()).unwrap().iter().map(|b| b.typ).collect();
        assert_eq!(boxes, [*b"ftyp", *b"moov", *b"mdat"]);
        assert_eq!(&out[out.len() - payload.len()..], &payload[..]);
        // 3 muestras de 10, 20 y 30 bytes en stsz
        let pos = out.windows(4).position(|w| w == b"stsz").unwrap();
        assert_eq!(u32_at(&out, pos + 12).unwrap(), 3);
        assert_eq!(
            [u32_at(&out, pos + 16).unwrap(), u32_at(&out, pos + 20).unwrap(), u32_at(&out, pos + 24).unwrap()],
            [10, 20, 30]
        );
    }

    #[test]
    fn las_etiquetas_se_escriben_y_no_rompen_los_offsets() {
        let (input, payload) = tiny_fragmented();
        let dir = std::env::temp_dir().join(format!("resonda-tags-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (i, o) = (dir.join("in.mp4"), dir.join("out.mp4"));
        std::fs::write(&i, &input).unwrap();
        let tags = Tags {
            title: "Título ñ".into(),
            artist: "Artista".into(),
            album: Some("Álbum".into()),
            year: Some("1987".into()),
            cover: Some(vec![0xff, 0xd8, 1, 2, 3]),
        };
        mux_mp4(&[i], &o, Some(&tags), |_, _| {}).unwrap();
        let out = std::fs::read(&o).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        for needle in ["ilst", "covr", "Título ñ", "Artista", "Álbum", "1987"] {
            assert!(out.windows(needle.len()).any(|w| w == needle.as_bytes()), "falta {needle}");
        }
        // los datos de las muestras siguen donde apunta la tabla de offsets
        assert_eq!(&out[out.len() - payload.len()..], &payload[..]);
        let stco = out.windows(4).position(|w| w == b"stco").unwrap();
        let offset = u32_at(&out, stco + 12).unwrap() as usize;
        assert_eq!(&out[offset..offset + 10], &payload[..10]);
    }

    #[test]
    fn cabecera_adts() {
        // AAC-LC, 44,1 kHz, estéreo, 100 bytes de datos: la longitud del fotograma incluye la
        // propia cabecera (107)
        assert_eq!(adts_header(1, 4, 2, 100), [0xff, 0xf1, 0x50, 0x80, 0x0d, 0x7f, 0xfc]);
    }

    #[test]
    fn audio_specific_config_de_un_esds() {
        // esds típico: 03 len | ES_ID(2) flags(1) | 04 len | 13 bytes | 05 02 12 10
        let mut esds = vec![0, 0, 0, 0, 0x03, 0x19, 0, 0, 0, 0x04, 0x11];
        esds.extend([0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        esds.extend([0x05, 0x02, 0x12, 0x10]);
        assert_eq!(audio_specific_config(&esds), Some(vec![0x12, 0x10]));
    }

    #[test]
    fn entrada_corrupta_da_error_y_no_panico() {
        let (input, _) = tiny_fragmented();
        // Archivo truncado en todas las longitudes posibles
        for n in 0..input.len() {
            let _ = mux_bytes(&input[..n], "trunc");
        }
        // Un byte corrupto (0xFF y 0x00) en todas las posiciones
        for i in 0..input.len() {
            for v in [0xff, 0x00] {
                let mut bad = input.clone();
                bad[i] = v;
                let _ = mux_bytes(&bad, "flip");
            }
        }
    }
}
