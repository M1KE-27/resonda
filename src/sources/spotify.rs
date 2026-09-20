// Lectura de METADATOS de Spotify (título, artistas, duración, portada) a partir de las páginas
// públicas "embed", sin cuenta ni credenciales. El audio NO sale de Spotify (está protegido con
// DRM y no se toca): cada canción se busca y se descarga desde YouTube (ver `matching`).

use crate::Res;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Track,
    Album,
    Playlist,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Kind::Track => "track",
            Kind::Album => "album",
            Kind::Playlist => "playlist",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub duration_ms: u64,
    pub album: Option<String>,
    pub year: Option<String>,
    pub cover_url: Option<String>,
}

impl Track {
    pub fn seconds(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
    pub fn main_artist(&self) -> &str {
        self.artists.first().map_or("", String::as_str)
    }
    pub fn label(&self) -> String {
        format!("{} - {}", self.artists.join(", "), self.title)
    }
}

pub struct Collection {
    pub kind: Kind,
    pub name: String,
    pub tracks: Vec<Track>,
}

/// Reconoce enlaces `https://open.spotify.com/[intl-xx/]track|album|playlist/ID` y URIs
/// `spotify:track:ID`.
pub fn parse_link(input: &str) -> Option<(Kind, String)> {
    let input = input.trim();
    let kind_of = |s: &str| match s {
        "track" => Some(Kind::Track),
        "album" => Some(Kind::Album),
        "playlist" => Some(Kind::Playlist),
        _ => None,
    };
    let is_id = |s: &str| s.len() == 22 && s.bytes().all(|b| b.is_ascii_alphanumeric());

    if let Some(rest) = input.strip_prefix("spotify:") {
        let mut parts = rest.split(':');
        let (k, id) = (kind_of(parts.next()?)?, parts.next()?);
        return is_id(id).then(|| (k, id.to_string()));
    }
    let rest = input.strip_prefix("https://").or_else(|| input.strip_prefix("http://")).unwrap_or(input);
    let (host, path) = rest.split_once('/')?;
    if host != "open.spotify.com" {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let mut segs = path.split('/').filter(|s| !s.is_empty()).skip_while(|s| s.starts_with("intl-"));
    let k = kind_of(segs.next()?)?;
    let id = segs.next()?;
    is_id(id).then(|| (k, id.to_string()))
}

fn next_data(html: &str) -> Res<Value> {
    let tag = html.find("id=\"__NEXT_DATA__\"").ok_or("Spotify no devolvió datos (¿enlace privado o inexistente?)")?;
    let start = html[tag..].find('>').ok_or("Página de Spotify ilegible")? + tag + 1;
    let end = html[start..].find("</script>").ok_or("Página de Spotify ilegible")? + start;
    Ok(serde_json::from_str(&html[start..end])?)
}

/// La imagen más grande de una lista `[{url, maxWidth}]`
fn best_image(images: &Value) -> Option<String> {
    images
        .as_array()?
        .iter()
        .filter_map(|i| Some((i["maxWidth"].as_u64().or(i["width"].as_u64()).unwrap_or(0), i["url"].as_str()?)))
        .max_by_key(|(w, _)| *w)
        .map(|(_, u)| u.to_string())
}

fn track_id(uri: &str) -> String {
    uri.rsplit(':').next().unwrap_or("").to_string()
}

fn year_of(entity: &Value) -> Option<String> {
    entity["releaseDate"]["isoString"].as_str().map(|s| s.chars().take(4).collect())
}

/// Lee una canción, un álbum o una playlist.
pub fn fetch(agent: &ureq::Agent, kind: Kind, id: &str) -> Res<Collection> {
    let mut resp = agent
        .get(format!("https://open.spotify.com/embed/{}/{id}", kind.path()))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .call()?;
    if !resp.status().is_success() {
        return Err(format!("Spotify respondió HTTP {}", resp.status().as_u16()).into());
    }
    let html = resp.body_mut().read_to_string()?;
    let data = next_data(&html)?;
    let e = &data["props"]["pageProps"]["state"]["data"]["entity"];
    if e.is_null() {
        return Err("Spotify no devolvió la ficha (¿enlace privado o inexistente?)".into());
    }
    let name = e["name"].as_str().or(e["title"].as_str()).unwrap_or("").to_string();
    let cover = best_image(&e["visualIdentity"]["image"])
        .or_else(|| e["coverArt"]["sources"][0]["url"].as_str().map(str::to_string));
    let year = year_of(e);

    let tracks = match kind {
        Kind::Track => vec![Track {
            id: id.to_string(),
            title: name.clone(),
            artists: e["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect()).unwrap_or_default(),
            duration_ms: e["duration"].as_u64().unwrap_or(0),
            album: None,
            year,
            cover_url: cover,
        }],
        Kind::Album | Kind::Playlist => e["trackList"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|t| {
                        let title = t["title"].as_str()?.to_string();
                        Some(Track {
                            id: track_id(t["uri"].as_str()?),
                            title,
                            artists: t["subtitle"].as_str().unwrap_or("").split(',').map(|a| a.trim().replace('\u{a0}', " ")).filter(|a| !a.is_empty()).collect(),
                            duration_ms: t["duration"].as_u64().unwrap_or(0),
                            album: (kind == Kind::Album).then(|| name.clone()),
                            year: year.clone(),
                            cover_url: cover.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    };
    if tracks.is_empty() {
        return Err("No se encontraron canciones en ese enlace de Spotify".into());
    }
    Ok(Collection { kind, name, tracks })
}

/// Datos más precisos de una canción de una lista (artistas exactos y portada de su álbum).
pub fn refine(agent: &ureq::Agent, track: &Track) -> Track {
    match fetch(agent, Kind::Track, &track.id) {
        Ok(c) => {
            let t = &c.tracks[0];
            Track {
                artists: if t.artists.is_empty() { track.artists.clone() } else { t.artists.clone() },
                duration_ms: if t.duration_ms > 0 { t.duration_ms } else { track.duration_ms },
                cover_url: t.cover_url.clone().or_else(|| track.cover_url.clone()),
                year: t.year.clone().or_else(|| track.year.clone()),
                ..track.clone()
            }
        }
        Err(_) => track.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enlaces_de_spotify() {
        let id = "4cOdK2wGLETKBW3PvgPWqT";
        for url in [
            format!("https://open.spotify.com/track/{id}"),
            format!("https://open.spotify.com/track/{id}?si=abc123"),
            format!("https://open.spotify.com/intl-es/track/{id}?si=abc"),
            format!("open.spotify.com/track/{id}"),
            format!("spotify:track:{id}"),
        ] {
            assert_eq!(parse_link(&url), Some((Kind::Track, id.to_string())), "{url}");
        }
        assert_eq!(parse_link("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M").unwrap().0, Kind::Playlist);
        assert_eq!(parse_link("https://open.spotify.com/album/0ETFjACtuP2ADo6LFhL6HN").unwrap().0, Kind::Album);
        assert_eq!(parse_link("https://open.spotify.com/artist/0gxyHStUsqpMadRV0Di1Qt"), None);
        assert_eq!(parse_link("https://youtu.be/dQw4w9WgXcQ"), None);
        assert_eq!(parse_link("https://open.spotify.com/track/corto"), None);
    }
}
