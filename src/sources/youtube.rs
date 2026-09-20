// Acceso a la API interna de YouTube (innertube).
//
// Dos ideas hacen que funcione sin tokens ni ejecutar el player.js:
//  1. Clientes que devuelven URLs directas (sin firma cifrada ni parámetro "n"): el
//     principal es VISIONOS, el mismo que usa yt-dlp por defecto (yt-dlp es de dominio
//     público, Unlicense; de ahí salen estos datos).
//  2. Una sesión anónima de navegador: se abre antes la página del vídeo y se reutilizan las
//     cookies que YouTube reparte a cualquier visitante y su `visitorData`. Sin ella, YouTube
//     responde «confirma que no eres un bot» o corta la descarga a los ~60 s.
//
// Si YouTube cambia algo, lo normal es que solo haya que tocar `clients()` o `Session`.

use crate::Res;
use serde_json::{json, Value};

struct Client {
    name: &'static str,
    id: u32,
    ua: &'static str,
    context: Value,
}

const VISIONOS_UA: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

fn clients() -> Vec<Client> {
    vec![
        Client {
            name: "VISIONOS",
            id: 101,
            ua: VISIONOS_UA,
            context: json!({
                "clientName": "VISIONOS", "clientVersion": "1.02",
                "deviceMake": "Apple", "deviceModel": "RealityDevice17,1",
                "userAgent": VISIONOS_UA,
                "osName": "visionOS", "osVersion": "26.5.23O471"
            }),
        },
        Client {
            name: "ANDROID_VR",
            id: 28,
            ua: "com.google.android.apps.youtube.vr.oculus/1.60.19 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
            context: json!({
                "clientName": "ANDROID_VR", "clientVersion": "1.60.19",
                "deviceMake": "Oculus", "deviceModel": "Quest 3",
                "osName": "Android", "osVersion": "12L", "androidSdkVersion": 32
            }),
        },
        Client {
            name: "ANDROID",
            id: 3,
            ua: "com.google.android.youtube/20.10.38 (Linux; U; Android 11) gzip",
            context: json!({
                "clientName": "ANDROID", "clientVersion": "20.10.38",
                "osName": "Android", "osVersion": "11", "androidSdkVersion": 30
            }),
        },
        Client {
            name: "IOS",
            id: 5,
            ua: "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
            context: json!({
                "clientName": "IOS", "clientVersion": "20.10.4",
                "deviceMake": "Apple", "deviceModel": "iPhone16,2",
                "osName": "iPhone", "osVersion": "18.3.2.22D82"
            }),
        },
    ]
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
    /// MP4 ya completo (vídeo + audio), solo hasta 360p
    Muxed,
}

#[derive(Clone)]
pub struct Format {
    pub itag: u32,
    pub url: String,
    pub kind: Kind,
    pub codec: String, // avc1, av01, mp4a...
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64,
    pub size: u64,
}

pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub seconds: u64,
    pub formats: Vec<Format>,
    pub ua: &'static str,
}

fn is_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn parse_video_id(input: &str) -> Res<String> {
    if is_id(input) {
        return Ok(input.to_string());
    }
    // Admite "youtu.be/..." sin esquema
    let rest = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))
        .unwrap_or(input);
    let (host, path_query) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").trim_start_matches("music.");
    let (path, query) = path_query.split_once('?').unwrap_or((path_query, ""));
    let query = query.split('#').next().unwrap_or("");
    let path = path.split('#').next().unwrap_or("");

    let id = match host {
        "youtu.be" => path.split('/').next().unwrap_or("").to_string(),
        "youtube.com" => {
            let from_query = query.split('&').find_map(|kv| kv.strip_prefix("v=")).map(str::to_string);
            let mut segs = path.split('/');
            let from_path = match (segs.next(), segs.next()) {
                (Some("shorts" | "embed" | "live" | "v"), Some(id)) => Some(id.to_string()),
                _ => None,
            };
            from_query.or(from_path).unwrap_or_default()
        }
        _ => return Err(format!("\"{input}\" no parece una URL ni un ID de YouTube").into()),
    };
    if is_id(&id) {
        Ok(id)
    } else {
        Err(format!("No encuentro el ID del vídeo en \"{input}\"").into())
    }
}

/// Sesión anónima de navegador (lo que hace yt-dlp antes de pedir el player).
#[derive(Default, Clone)]
struct Session {
    cookies: String,
    visitor_data: String,
    sts: Option<u64>,
    /// Versión actual del cliente web (para la búsqueda)
    web_version: Option<String>,
}

const BASE_COOKIES: &str = "PREF=hl=en&tz=UTC; SOCS=CAI";

fn between<'a>(s: &'a str, start: &str) -> Option<&'a str> {
    let from = s.find(start)? + start.len();
    Some(&s[from..])
}

impl Session {
    /// Abre una página de YouTube (la del vídeo, o la portada para buscar) como un navegador.
    fn fetch(agent: &ureq::Agent, url: &str) -> Res<Session> {
        let mut resp = agent
            .get(url)
            .header("User-Agent", VISIONOS_UA)
            .header("Accept-Language", "en-us,en;q=0.5")
            .header("Cookie", BASE_COOKIES)
            .call()?;

        // Cookies que YouTube reparte a cualquier visitante (VISITOR_INFO1_LIVE, YSC...)
        let mut jar: Vec<(String, String)> = Vec::new();
        for v in resp.headers().get_all("set-cookie") {
            let Some((name, value)) = v.to_str().ok().and_then(|s| s.split(';').next()).and_then(|kv| kv.split_once('='))
            else {
                continue;
            };
            jar.retain(|(n, _)| n != name.trim());
            jar.push((name.trim().to_string(), value.to_string()));
        }
        let mut cookies = BASE_COOKIES.to_string();
        for (n, v) in &jar {
            cookies.push_str(&format!("; {n}={v}"));
        }

        let html = resp.body_mut().read_to_string()?;
        let visitor_data = between(&html, "\"VISITOR_DATA\":\"")
            .and_then(|r| r.split('"').next())
            .unwrap_or_default()
            .to_string();
        let sts = between(&html, "\"STS\":")
            .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>())
            .and_then(|d| d.parse().ok());
        let web_version = between(&html, "\"INNERTUBE_CLIENT_VERSION\":\"")
            .and_then(|r| r.split('"').next())
            .map(str::to_string);
        Ok(Session { cookies, visitor_data, sts, web_version })
    }
}

fn fetch_player(agent: &ureq::Agent, client: &Client, session: &Session, video_id: &str) -> Res<Value> {
    let mut context = client.context.clone();
    context["hl"] = json!("en");
    context["timeZone"] = json!("UTC");
    context["utcOffsetMinutes"] = json!(0);
    let mut playback = json!({ "html5Preference": "HTML5_PREF_WANTS" });
    if let Some(sts) = session.sts {
        playback["signatureTimestamp"] = json!(sts);
    }
    let body = json!({
        "context": { "client": context },
        "videoId": video_id,
        "playbackContext": { "contentPlaybackContext": playback },
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let mut req = agent
        .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
        .header("User-Agent", client.ua)
        .header("Accept-Language", "en-us,en;q=0.5")
        .header("Origin", "https://www.youtube.com")
        .header("X-YouTube-Client-Name", client.id.to_string())
        .header("X-YouTube-Client-Version", context["clientVersion"].as_str().unwrap_or(""));
    if !session.visitor_data.is_empty() {
        req = req.header("X-Goog-Visitor-Id", &session.visitor_data);
    }
    if !session.cookies.is_empty() {
        req = req.header("Cookie", &session.cookies);
    }
    let mut resp = req.send_json(&body)?;
    if !resp.status().is_success() {
        return Err(format!("YouTube respondió HTTP {}", resp.status().as_u16()).into());
    }
    Ok(resp.body_mut().read_json::<Value>()?)
}

fn normalize(f: &Value, muxed: bool) -> Option<Format> {
    let mime = f["mimeType"].as_str()?;
    let url = f["url"].as_str()?;
    let (typ, params) = mime.split_once(';')?;
    let (kind, container) = typ.split_once('/')?;
    if container != "mp4" {
        return None;
    }
    let codecs = params.split("codecs=\"").nth(1)?.split('"').next()?;
    Some(Format {
        itag: f["itag"].as_u64()? as u32,
        url: url.to_string(),
        kind: match (muxed, kind) {
            (true, _) => Kind::Muxed,
            (_, "video") => Kind::Video,
            _ => Kind::Audio,
        },
        codec: codecs.split('.').next()?.to_string(),
        height: f["height"].as_u64().unwrap_or(0) as u32,
        fps: f["fps"].as_u64().unwrap_or(0) as u32,
        bitrate: f["bitrate"].as_u64().unwrap_or(0),
        size: f["contentLength"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
    })
}

fn try_client(agent: &ureq::Agent, client: &Client, session: &Session, id: &str) -> Res<VideoInfo> {
    let p = fetch_player(agent, client, session, id)?;
    let status = &p["playabilityStatus"];
    if status["status"].as_str() != Some("OK") {
        return Err(format!(
            "{}: {}",
            status["status"].as_str().unwrap_or("ERROR"),
            status["reason"].as_str().unwrap_or("vídeo no disponible")
        )
        .into());
    }
    if p["videoDetails"]["isLive"].as_bool() == Some(true) {
        return Err("Los directos no están soportados".into());
    }

    let list = |key: &str, muxed: bool| -> Vec<Format> {
        p["streamingData"][key]
            .as_array()
            .map(|a| a.iter().filter_map(|f| normalize(f, muxed)).collect())
            .unwrap_or_default()
    };
    let mut formats = list("adaptiveFormats", false);
    if !formats.iter().any(|f| f.kind == Kind::Video) || !formats.iter().any(|f| f.kind == Kind::Audio) {
        return Err(format!("El cliente {} no devolvió formatos MP4 de vídeo y audio", client.name).into());
    }
    formats.extend(list("formats", true));
    Ok(VideoInfo {
        id: id.to_string(),
        title: p["videoDetails"]["title"].as_str().unwrap_or(id).to_string(),
        seconds: p["videoDetails"]["lengthSeconds"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
        formats,
        ua: client.ua,
    })
}

/// ¿Sirve YouTube el vídeo completo con este cliente? Sin token de origen (PO token) solo entrega
/// los primeros ~60 s de casi todos los vídeos y responde 403 a partir de ahí. Pedir el primer
/// byte no lo detecta (siempre responde 206): se pide el ÚLTIMO byte del formato más pequeño.
fn streams_fully(agent: &ureq::Agent, info: &VideoInfo) -> Res<bool> {
    let smallest = info
        .formats
        .iter()
        .filter(|f| f.kind == Kind::Video && f.size > 0)
        .min_by_key(|f| f.size)
        .ok_or("No hay formatos de vídeo con tamaño conocido")?;
    let last = smallest.size - 1;
    let resp = agent
        .get(&smallest.url)
        .header("User-Agent", info.ua)
        .header("Range", format!("bytes={last}-{last}"))
        .call()?;
    match resp.status().as_u16() {
        200 | 206 => Ok(true),
        403 => Ok(false),
        code => Err(format!("HTTP {code} al comprobar el stream").into()),
    }
}

const TRUNCATED: &str = "YouTube solo entrega los primeros ~60 segundos de este vídeo a las apps sin sesión iniciada \
(exige un token de origen que esta herramienta no genera), así que no se puede descargar completo";

/// Devuelve título, duración y los formatos descargables (solo MP4). Prueba los clientes por
/// orden y se queda con el primero que sirve el vídeo completo; si ninguno lo hace, lo dice.
///
/// YouTube corta a veces (de forma intermitente) un vídeo que minutos antes servía entero, así que
/// si todos los clientes salen truncados se reintenta con una sesión nueva antes de rendirse.
pub fn get_video_info(agent: &ureq::Agent, input: &str) -> Res<VideoInfo> {
    const ATTEMPTS: u32 = 3;
    let id = parse_video_id(input)?;
    let mut last_error: Box<dyn std::error::Error + Send + Sync> = "No se pudo obtener el vídeo".into();

    for attempt in 1..=ATTEMPTS {
        let mut truncated = false;
        // Si la página no se puede abrir se sigue sin sesión: algunos clientes aún funcionan así
        let session = Session::fetch(agent, &format!("https://www.youtube.com/watch?v={id}")).unwrap_or_default();
        for client in clients() {
            let result = try_client(agent, &client, &session, &id).and_then(|info| streams_fully(agent, &info).map(|ok| (info, ok)));
            match result {
                Ok((info, true)) => return Ok(info),
                Ok((_, false)) => truncated = true,
                Err(e) => last_error = e,
            }
        }
        if !truncated {
            break; // otro tipo de error (no disponible, privado...): reintentar no lo arregla
        }
        last_error = TRUNCATED.into();
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(1500 * u64::from(attempt)));
        }
    }
    Err(last_error)
}

/// Elige qué descargar.
///  - Vídeo: la mayor altura <= max_height; a igual altura prefiere H.264 (avc1), que
///    reproduce en todas partes. Por encima de 1080p YouTube solo ofrece AV1.
///  - Audio: el AAC de mayor bitrate.
pub fn pick_formats(info: &VideoInfo, max_height: u32, audio_only: bool) -> Res<(Option<Format>, Format)> {
    let audio = info
        .formats
        .iter()
        .filter(|f| f.kind == Kind::Audio && f.codec == "mp4a")
        .max_by_key(|f| f.bitrate)
        .cloned()
        .ok_or("No hay pista de audio AAC disponible")?;
    if audio_only {
        return Ok((None, audio));
    }

    let videos: Vec<&Format> = info
        .formats
        .iter()
        .filter(|f| f.kind == Kind::Video && (f.codec == "avc1" || f.codec == "av01") && f.height <= max_height)
        .collect();
    let top = videos.iter().map(|f| f.height).max().ok_or_else(|| format!("No hay vídeo de {max_height}p o menos"))?;
    let video = videos
        .into_iter()
        .filter(|f| f.height == top)
        .max_by_key(|f| (f.codec == "avc1", f.fps, f.bitrate))
        .cloned()
        .unwrap();
    Ok((Some(video), audio))
}

// ---------- búsqueda ----------

/// La sesión de búsqueda se reutiliza (unos minutos) para no abrir YouTube en cada consulta:
/// con una playlist de 50 canciones serían 100 aperturas de una página de 1 MB.
fn search_session(agent: &ureq::Agent) -> Session {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Session)>> = Mutex::new(None);

    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, s)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(600) {
            return s.clone();
        }
    }
    let session = Session::fetch(agent, "https://www.youtube.com/").unwrap_or_default();
    *cache = Some((Instant::now(), session.clone()));
    session
}


#[derive(Clone, Debug)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub seconds: u64,
}

/// "3:34" o "1:02:03" -> segundos
fn parse_clock(s: &str) -> Option<u64> {
    let mut total = 0;
    for part in s.split(':') {
        total = total * 60 + part.trim().parse::<u64>().ok()?;
    }
    Some(total)
}

fn collect_videos(v: &Value, out: &mut Vec<SearchResult>) {
    match v {
        Value::Object(map) => {
            if let Some(r) = map.get("videoRenderer") {
                let text = |x: &Value| x["runs"].as_array().map(|a| a.iter().filter_map(|r| r["text"].as_str()).collect::<String>());
                let clock = r["lengthText"]["simpleText"].as_str().and_then(parse_clock);
                if let (Some(id), Some(title), Some(seconds)) = (r["videoId"].as_str(), text(&r["title"]), clock) {
                    out.push(SearchResult {
                        id: id.to_string(),
                        title,
                        channel: text(&r["ownerText"]).unwrap_or_default(),
                        seconds,
                    });
                }
            } else {
                map.values().for_each(|x| collect_videos(x, out));
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_videos(x, out)),
        _ => {}
    }
}

/// Busca vídeos en YouTube (solo vídeos, sin directos ni listas) y devuelve los resultados en
/// el orden de YouTube.
pub fn search(agent: &ureq::Agent, query: &str) -> Res<Vec<SearchResult>> {
    let session = search_session(agent);
    let version = session.web_version.clone().unwrap_or_else(|| "2.20260918.00.00".to_string());
    let body = json!({
        "context": { "client": {
            "clientName": "WEB", "clientVersion": version, "hl": "en", "gl": "US",
            "userAgent": VISIONOS_UA, "visitorData": session.visitor_data,
        }},
        "query": query,
        "params": "EgIQAQ%3D%3D", // filtro: solo vídeos
    });
    let mut req = agent
        .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
        .header("User-Agent", VISIONOS_UA)
        .header("Origin", "https://www.youtube.com")
        .header("X-YouTube-Client-Name", "1")
        .header("X-YouTube-Client-Version", &version);
    if !session.visitor_data.is_empty() {
        req = req.header("X-Goog-Visitor-Id", &session.visitor_data);
    }
    if !session.cookies.is_empty() {
        req = req.header("Cookie", &session.cookies);
    }
    let mut resp = req.send_json(&body)?;
    if !resp.status().is_success() {
        return Err(format!("La búsqueda de YouTube respondió HTTP {}", resp.status().as_u16()).into());
    }
    let json: Value = resp.body_mut().read_json()?;
    let mut out = Vec::new();
    collect_videos(&json, &mut out);
    Ok(out)
}
