// Escritor WAV (PCM de 16 bits) con etiquetas LIST/INFO. Sin dependencias.

use crate::media::Tags;
use crate::media::pcm::{PcmFormat, PcmSink};
use crate::Res;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

pub struct WavWriter {
    out: BufWriter<File>,
    tags: Option<Tags>,
    data_bytes: u64,
}

impl WavWriter {
    pub fn create(path: &Path, tags: Option<&Tags>) -> Res<WavWriter> {
        Ok(WavWriter { out: BufWriter::new(File::create(path)?), tags: tags.cloned(), data_bytes: 0 })
    }
}

fn info_chunk(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    let mut chunk = id.to_vec();
    chunk.extend((data.len() as u32).to_le_bytes());
    chunk.extend(&data);
    if data.len() % 2 == 1 {
        chunk.push(0); // los bloques RIFF se alinean a 2 bytes
    }
    chunk
}

/// Bloque `LIST`/`INFO`: título, artista, álbum y año
fn list_info(t: &Tags) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    body.extend(info_chunk(b"INAM", &t.title));
    body.extend(info_chunk(b"IART", &t.artist));
    if let Some(a) = &t.album {
        body.extend(info_chunk(b"IPRD", a));
    }
    if let Some(y) = &t.year {
        body.extend(info_chunk(b"ICRD", y));
    }
    let mut out = b"LIST".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

impl PcmSink for WavWriter {
    fn start(&mut self, f: PcmFormat) -> Res<()> {
        let (ch, rate) = (f.channels as u32, f.sample_rate);
        let mut h = Vec::new();
        h.extend(b"RIFF");
        h.extend(0u32.to_le_bytes()); // tamaño: se rellena al terminar
        h.extend(b"WAVEfmt ");
        h.extend(16u32.to_le_bytes());
        h.extend(1u16.to_le_bytes()); // PCM
        h.extend((ch as u16).to_le_bytes());
        h.extend(rate.to_le_bytes());
        h.extend((rate * ch * 2).to_le_bytes()); // bytes por segundo
        h.extend(((ch * 2) as u16).to_le_bytes()); // bytes por fotograma
        h.extend(16u16.to_le_bytes());
        if let Some(t) = &self.tags {
            h.extend(list_info(t));
        }
        h.extend(b"data");
        h.extend(0u32.to_le_bytes()); // tamaño: se rellena al terminar
        self.out.write_all(&h)?;
        Ok(())
    }

    fn write(&mut self, samples: &[i16]) -> Res<()> {
        for s in samples {
            self.out.write_all(&s.to_le_bytes())?;
        }
        self.data_bytes += samples.len() as u64 * 2;
        Ok(())
    }

    fn finish(&mut self) -> Res<()> {
        if self.data_bytes > u32::MAX as u64 - 4096 {
            return Err("El audio supera los 4 GB que admite un WAV".into());
        }
        if self.data_bytes % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        self.out.flush()?;
        let total = self.out.get_ref().metadata()?.len();
        let file = self.out.get_mut();
        // Tamaño del bloque `data` (últimos 4 bytes de la cabecera) y del RIFF
        let data_size_pos = total - self.data_bytes - (self.data_bytes % 2) - 4;
        file.seek(SeekFrom::Start(data_size_pos))?;
        file.write_all(&(self.data_bytes as u32).to_le_bytes())?;
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&((total - 8) as u32).to_le_bytes())?;
        file.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_valido_con_etiquetas() {
        let path = std::env::temp_dir().join(format!("resonda-wav-{}.wav", std::process::id()));
        let tags = Tags { title: "Título".into(), artist: "Artista".into(), album: Some("Álbum".into()), year: Some("1999".into()), cover: None };
        let mut w = WavWriter::create(&path, Some(&tags)).unwrap();
        w.start(PcmFormat { sample_rate: 44100, channels: 2 }).unwrap();
        w.write(&[1, -1, 2, -2, 300, -300]).unwrap();
        w.finish().unwrap();
        let b = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize, b.len() - 8);
        assert_eq!(&b[8..12], b"WAVE");
        let data = b.windows(4).position(|w| w == b"data").unwrap();
        assert_eq!(u32::from_le_bytes(b[data + 4..data + 8].try_into().unwrap()), 12);
        assert_eq!(&b[data + 8..data + 10], &1i16.to_le_bytes());
        assert!(b.windows(4).any(|w| w == b"IART") && b.windows(7).any(|w| w == "Título".as_bytes()[..7].as_ref()));
    }
}
