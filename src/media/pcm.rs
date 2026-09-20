// Decodifica el AAC de YouTube a PCM de 16 bits entrelazado. Es lo único que usa una librería
// ajena (symphonia, solo el decodificador AAC); todo lo que se hace con el PCM es código propio.

use crate::media::mp4::AacStream;
use crate::Res;
use std::path::Path;
use symphonia_codec_aac::AacDecoder;
use symphonia_core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia_core::packet::PacketRef;
use symphonia_core::units::{Duration, Timestamp};

#[derive(Clone, Copy, Debug)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u8,
}

/// Destino del PCM (WAV, FLAC, MP3...). Recibe las muestras según se decodifican, así que un
/// vídeo de horas no se carga entero en memoria.
pub trait PcmSink {
    /// Se llama una vez, antes de la primera muestra.
    fn start(&mut self, format: PcmFormat) -> Res<()>;
    /// Muestras `i16` entrelazadas (L R L R...).
    fn write(&mut self, samples: &[i16]) -> Res<()>;
    /// Se llama al terminar, para cerrar el archivo (cabeceras, último bloque...).
    fn finish(&mut self) -> Res<()>;
}

/// Decodifica un M4A con audio AAC-LC y lo vuelca en `sink`.
pub fn decode_aac_to(input: &Path, sink: &mut dyn PcmSink) -> Res<()> {
    let aac = AacStream::open(input)?;
    if aac.channels > 2 {
        return Err(format!("Audio de {} canales no soportado (solo mono o estéreo)", aac.channels).into());
    }
    let mut params = AudioCodecParameters::new();
    params.for_codec(CODEC_ID_AAC).with_sample_rate(aac.sample_rate).with_extra_data(aac.asc.clone().into_boxed_slice());
    let mut decoder = AacDecoder::try_new(&params, &AudioDecoderOptions::default()).map_err(|e| format!("Decodificador AAC: {e}"))?;

    sink.start(PcmFormat { sample_rate: aac.sample_rate, channels: aac.channels })?;
    let (mut pts, mut pcm) = (0i64, Vec::<i16>::new());
    aac.for_each_frame(|frame| {
        let packet = PacketRef::new(0, Timestamp::new(pts), Duration::new(1024), frame);
        pts += 1024;
        match decoder.decode_ref(&packet) {
            Ok(buf) => {
                buf.copy_to_vec_interleaved::<i16>(&mut pcm);
                sink.write(&pcm)
            }
            // Un fotograma dañado no debe tirar toda la canción: se sustituye por silencio
            Err(_) => {
                let silence = vec![0i16; 1024 * aac.channels as usize];
                sink.write(&silence)
            }
        }
    })?;
    sink.finish()
}
