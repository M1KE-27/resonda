//! Audio AAC de un M4A: lectura fotograma a fotograma y extracción a AAC crudo (ADTS).

use super::boxes::*;
use super::track::{read_track, Track};
use crate::io::read_exact_at;
use crate::Res;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

// ---------- AAC crudo (ADTS) ----------

pub(super) const ADTS_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// Cabecera ADTS de 7 bytes (MPEG-4, sin CRC) para un fotograma AAC de `payload` bytes.
pub(super) fn adts_header(profile: u8, freq_index: u8, channels: u8, payload: usize) -> [u8; 7] {
    let len = payload + 7;
    [
        0xff,
        0xf1,
        (profile << 6) | (freq_index << 2) | (channels >> 2),
        ((channels & 3) << 6) | ((len >> 11) & 3) as u8,
        ((len >> 3) & 0xff) as u8,
        (((len & 7) << 5) as u8) | 0x1f,
        0xfc,
    ]
}

/// Lee el AudioSpecificConfig de la caja `esds` (descriptores anidados con longitud variable).
pub(super) fn audio_specific_config(esds: &[u8]) -> Option<Vec<u8>> {
    pub(super) fn walk(b: &[u8], mut pos: usize, end: usize) -> Option<Vec<u8>> {
        while pos < end {
            let tag = *b.get(pos)?;
            pos += 1;
            let mut len = 0usize;
            for _ in 0..4 {
                let byte = *b.get(pos)?;
                pos += 1;
                len = (len << 7) | (byte & 0x7f) as usize;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            let body_end = pos.checked_add(len)?.min(end);
            match tag {
                0x03 => return walk(b, pos + 3, body_end), // ES_ID (2) + banderas (1)
                0x04 => return walk(b, pos + 13, body_end), // tipo, flujo, búfer, tasas
                0x05 => return b.get(pos..body_end).map(<[u8]>::to_vec),
                _ => {}
            }
            pos = body_end;
        }
        None
    }
    walk(esds, 4, esds.len()) // 4 = versión y banderas de la caja
}

/// Audio AAC de un M4A fragmentado de YouTube, listo para leerlo fotograma a fotograma.
pub struct AacStream {
    /// AudioSpecificConfig (los 2+ bytes de configuración del códec)
    pub asc: Vec<u8>,
    pub object_type: u8,
    pub freq_index: u8,
    pub channels: u8,
    pub sample_rate: u32,
    path: PathBuf,
    track: Track,
}

impl AacStream {
    pub fn open(input: &Path) -> Res<AacStream> {
        let track = read_track(input)?;
        if &track.fourcc != b"mp4a" {
            return Err("La pista de audio no es AAC".into());
        }
        let entry = track.stsd.body + 8; // primera entrada de la tabla de descripciones
        let entry_end = entry + u32_at(&track.moov, entry)? as usize;
        let esds = children(&track.moov, entry + 36, entry_end.min(track.moov.len()))? // 8 de cabecera + 28 de campos de audio
            .into_iter()
            .find(|c| &c.typ == b"esds")
            .ok_or("Falta la caja esds")?;
        let asc = audio_specific_config(&track.moov[esds.body..esds.end]).ok_or("AudioSpecificConfig ilegible")?;
        if asc.len() < 2 {
            return Err("AudioSpecificConfig demasiado corto".into());
        }
        let object_type = asc[0] >> 3;
        let freq_index = ((asc[0] & 7) << 1) | (asc[1] >> 7);
        let channels = (asc[1] >> 3) & 0x0f;
        if !(1..=4).contains(&object_type) || freq_index as usize >= ADTS_RATES.len() || channels == 0 || channels > 7 {
            return Err(format!("Perfil AAC no soportado (tipo {object_type}, {channels} canales)").into());
        }
        Ok(AacStream {
            sample_rate: ADTS_RATES[freq_index as usize],
            asc,
            object_type,
            freq_index,
            channels,
            path: input.to_path_buf(),
            track,
        })
    }

    /// Llama a `f` con cada fotograma AAC, en orden.
    pub fn for_each_frame(&self, mut f: impl FnMut(&[u8]) -> Res<()>) -> Res<()> {
        let file = File::open(&self.path)?;
        let (mut buf, mut idx) = (Vec::new(), 0usize);
        for chunk in &self.track.chunks {
            buf.resize(chunk.len as usize, 0);
            read_exact_at(&file, &mut buf, chunk.src)?;
            let mut pos = 0usize;
            for _ in 0..chunk.count {
                let size = self.track.samples.get(idx).ok_or("Tabla de muestras incoherente")?.size as usize;
                f(buf.get(pos..pos + size).ok_or("Fotograma fuera de rango")?)?;
                pos += size;
                idx += 1;
            }
        }
        Ok(())
    }
}

/// Saca el audio AAC de un M4A fragmentado de YouTube a un archivo `.aac` (ADTS), sin recodificar.
pub fn extract_aac_adts(input: &Path, out_path: &Path) -> Res<u64> {
    let aac = AacStream::open(input)?;
    let mut out = File::create(out_path)?;
    let mut total = 0u64;
    aac.for_each_frame(|frame| {
        out.write_all(&adts_header(aac.object_type - 1, aac.freq_index, aac.channels, frame.len()))?;
        out.write_all(frame)?;
        total += 7 + frame.len() as u64;
        Ok(())
    })?;
    out.flush()?;
    Ok(total)
}
