// Descarga un archivo por trozos (Range) y en paralelo. YouTube limita mucho las
// peticiones de un solo golpe, pero los trozos de unos MB van a toda velocidad.

use crate::io::write_all_at;
use crate::{Cancelled, Res};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const CHUNK: u64 = 8 * 1024 * 1024;
const RETRIES: u32 = 4;
const CONCURRENCY: usize = 4;

fn fetch_range(agent: &ureq::Agent, url: &str, ua: &str, start: u64, end: u64) -> Res<Vec<u8>> {
    let want = (end - start + 1) as usize;
    let mut last = String::new();
    for attempt in 1..=RETRIES {
        let result = || -> Res<Vec<u8>> {
            let resp = agent
                .get(url)
                .header("User-Agent", ua)
                .header("Range", format!("bytes={start}-{end}"))
                .call()?;
            let code = resp.status().as_u16();
            if code != 206 && code != 200 {
                return Err(format!("HTTP {code}").into());
            }
            let mut buf = Vec::with_capacity(want);
            resp.into_body().into_reader().take(want as u64 + 1).read_to_end(&mut buf)?;
            if buf.len() != want {
                return Err("trozo incompleto".into());
            }
            Ok(buf)
        }();
        match result {
            Ok(buf) => return Ok(buf),
            Err(e) => {
                last = e.to_string();
                // Un 403 no se arregla reintentando: YouTube ha cortado el acceso
                if last.starts_with("HTTP 403") {
                    return Err(format!(
                        "YouTube cortó la descarga (HTTP 403 a partir del byte {start}): no entrega el resto de este vídeo a las apps sin sesión iniciada"
                    )
                    .into());
                }
                thread::sleep(Duration::from_millis(400 * u64::from(attempt * attempt)));
            }
        }
    }
    Err(format!("Fallo descargando bytes {start}-{end}: {last}").into())
}

// Algunos formatos no traen contentLength: se saca de Content-Range pidiendo un solo byte
fn probe_size(agent: &ureq::Agent, url: &str, ua: &str) -> Res<u64> {
    let resp = agent.get(url).header("User-Agent", ua).header("Range", "bytes=0-0").call()?;
    resp.headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| "No se pudo averiguar el tamaño del archivo".into())
}

/// Descarga `url` en `dest`. `size` = 0 si no se conoce de antemano.
/// Si `cancel` pasa a true, se detiene y devuelve `Cancelled`.
pub fn download_file(
    agent: &ureq::Agent,
    url: &str,
    dest: &Path,
    size: u64,
    ua: &str,
    on_progress: &(dyn Fn(u64, u64) + Sync),
    cancel: &AtomicBool,
) -> Res<()> {
    let size = if size == 0 { probe_size(agent, url, ua)? } else { size };
    let file = File::create(dest)?;
    file.set_len(size)?;

    let chunks: Vec<(u64, u64)> = (0..size).step_by(CHUNK as usize).map(|s| (s, (s + CHUNK).min(size) - 1)).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicU64::new(0);
    let failure: Mutex<Option<Box<dyn std::error::Error + Send + Sync>>> = Mutex::new(None);
    let fail = |e: Box<dyn std::error::Error + Send + Sync>| {
        failure.lock().unwrap().get_or_insert(e);
    };

    thread::scope(|scope| {
        for _ in 0..CONCURRENCY.min(chunks.len()) {
            scope.spawn(|| loop {
                if cancel.load(Ordering::Relaxed) {
                    fail(Box::new(Cancelled));
                }
                if failure.lock().unwrap().is_some() {
                    break;
                }
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(&(start, end)) = chunks.get(i) else { break };
                match fetch_range(agent, url, ua, start, end) {
                    Ok(buf) => {
                        if let Err(e) = write_all_at(&file, &buf, start) {
                            fail(Box::new(e));
                            break;
                        }
                        let total_done = done.fetch_add(buf.len() as u64, Ordering::SeqCst) + buf.len() as u64;
                        on_progress(total_done, size);
                    }
                    Err(e) => {
                        fail(e);
                        break;
                    }
                }
            });
        }
    });
    match failure.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
