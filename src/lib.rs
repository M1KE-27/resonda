// Núcleo de resonda: lo usan la CLI (main.rs) y la interfaz gráfica (a través de ffi.rs).

pub mod ffi;
mod io;
pub mod media;
pub mod pipeline;
pub mod sources;

use std::time::Duration;

pub type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Error especial: la operación se canceló a petición del usuario (no es un fallo).
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cancelado")
    }
}

impl std::error::Error for Cancelled {}

pub fn new_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
}
