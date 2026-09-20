//! Formatos de salida: MP4/M4A/AAC (muxer propio), WAV, FLAC (codificador propio) y MP3.
//! `pcm` decodifica el AAC de YouTube y alimenta a WAV, FLAC y MP3.

pub mod flac;
pub mod mp3;
pub mod mp4;
pub mod pcm;
mod tags;
pub mod wav;

pub use tags::Tags;

