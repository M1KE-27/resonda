//! Metadatos compartidos por todos los formatos de audio.

/// Metadatos que se guardan en el MP4/M4A (formato "ilst" de iTunes, lo entienden todos los reproductores).
#[derive(Clone, Default)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub year: Option<String>,
    /// Imagen de portada (JPEG o PNG)
    pub cover: Option<Vec<u8>>,
}
