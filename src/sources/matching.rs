// Emparejar una canción de Spotify con el resultado de YouTube correcto.
// Idea y umbrales inspirados en la lógica de spotDL (MIT): la duración manda (una versión de
// otra duración casi siempre es otra grabación), luego título y artista, y se penalizan las
// palabras que delatan otra versión (remix, live, cover...). Implementación propia.

use crate::sources::spotify::Track;
use crate::sources::youtube::SearchResult;

/// Palabras que indican otra versión, salvo que ya estén en el título de Spotify.
const FORBIDDEN: &[&str] = &[
    "bassboosted", "bassboost", "remix", "remastered", "remaster", "reverb", "live", "acoustic",
    "8d", "concert", "acapella", "slowed", "sped", "instrumental", "cover", "karaoke", "reaction",
    "nightcore", "tutorial", "mashup", "mix", "anniversary", "demo", "session", "mono", "edit",
    "extended", "take", "preview",
];

fn fold(c: char) -> Option<char> {
    let c = c.to_lowercase().next()?;
    Some(match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        c if c.is_alphanumeric() => c,
        _ => ' ',
    })
}

/// Minúsculas, sin acentos ni signos.
fn norm(s: &str) -> String {
    let s: String = s.chars().filter_map(fold).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Título "limpio" para comparar: quita "(feat. X)", "- Remastered 2009", "[...]", etc.
fn core_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // " - Remastered 2009", " - Live at ...": lo que va tras un guion separado
    let out = out.split(" - ").next().unwrap_or("").to_string();
    norm(&out)
}

fn tokens(s: &str) -> Vec<&str> {
    s.split_whitespace().collect()
}

/// % de palabras de `needle` que aparecen en `hay`
fn coverage(needle: &str, hay: &str) -> f64 {
    let hay_words = tokens(hay);
    let words = tokens(needle);
    if words.is_empty() {
        return 0.0;
    }
    let found = words.iter().filter(|w| hay_words.contains(w)).count();
    found as f64 / words.len() as f64 * 100.0
}

/// Palabras de la "versión" que pide Spotify: lo que va tras " - " o entre paréntesis
/// ("Remastered 2009", "Radio Edit"), salvo colaboraciones ("feat.").
fn version_tokens(title: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some((_, after)) = title.split_once(" - ") {
        parts.push(after.to_string());
    }
    let mut depth = 0;
    let mut cur = String::new();
    for c in title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    parts.push(std::mem::take(&mut cur));
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => {}
        }
    }
    parts
        .iter()
        .map(|p| norm(p))
        .filter(|p| !p.starts_with("feat") && !p.starts_with("with ") && !p.starts_with("ft "))
        .flat_map(|p| p.split_whitespace().map(str::to_string).collect::<Vec<_>>())
        .collect()
}

/// Igual si uno es prefijo del otro ("remaster" ~ "remastered")
fn same_word(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 6 && (a.starts_with(b) || b.starts_with(a)))
}

fn is_year(w: &str) -> bool {
    w.len() == 4 && w.starts_with(['1', '2']) && w.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub result: SearchResult,
    pub score: f64,
}

/// Puntúa un candidato de 0 a 100 (o `None` si se descarta).
pub fn score(track: &Track, r: &SearchResult) -> Option<f64> {
    let title = core_title(&track.title);
    let cand_title = norm(&r.title);
    let cand_channel = norm(&r.channel);

    // 1) Duración: exp(-0.1·Δs), como spotDL. A más de ~14 s de diferencia es otra versión.
    let diff = (track.seconds() - r.seconds as f64).abs();
    let time = (-0.1 * diff).exp() * 100.0;
    if time < 25.0 {
        return None;
    }

    // 2) Título: cuántas palabras del título de Spotify están en el del vídeo
    let name = coverage(&title, &cand_title);
    if name <= 60.0 {
        return None;
    }

    // 3) Artista: el principal debe aparecer en el título o en el canal (los canales
    //    automáticos se llaman "Artista - Topic")
    let main = norm(track.main_artist());
    let haystack = format!("{cand_title} {cand_channel}");
    let artist = if main.is_empty() || haystack.contains(&main) {
        100.0
    } else {
        coverage(&main, &haystack)
    };
    if artist < 70.0 {
        return None;
    }

    // 4) Palabras que delatan otra versión
    let song_words = norm(&track.title);
    let penalties = FORBIDDEN
        .iter()
        .filter(|w| tokens(&cand_title).contains(w) && !tokens(&song_words).contains(w))
        .count() as f64
        * 15.0;

    // 5) Bonos: canal oficial de audio ("Topic") o del propio artista
    let mut bonus = 0.0;
    if r.channel.ends_with("- Topic") {
        bonus += 8.0;
    } else if !main.is_empty() && cand_channel.contains(&main) {
        bonus += 4.0;
    }

    // 6) Versión que pide Spotify ("Remastered 2009"): premio si el vídeo la nombra
    let version = version_tokens(&track.title);
    if !version.is_empty() {
        let cand_words = tokens(&cand_title);
        let hit = version.iter().filter(|v| cand_words.iter().any(|c| same_word(v, c))).count();
        bonus += 6.0 * hit as f64 / version.len() as f64;
    }

    // 7) Títulos "limpios": premio al título exacto; pequeña pena por cada palabra de más
    //    ("official video", "4k", "lyrics"...) y fuerte si es un año que no es el de la canción.
    let known: Vec<String> = tokens(&song_words)
        .into_iter()
        .chain(tokens(&norm(&track.artists.join(" "))))
        .map(str::to_string)
        .chain(version.iter().cloned())
        .collect();
    let mut extra_pen: f64 = 0.0;
    for w in tokens(&cand_title) {
        if !known.iter().any(|k| same_word(k, w)) {
            extra_pen += if is_year(w) { 4.0 } else { 0.75 };
        }
    }
    bonus -= extra_pen.min(8.0);
    if cand_title == title || cand_title == format!("{main} {title}") {
        bonus += 5.0;
    }

    let base = 0.35 * name + 0.25 * artist + 0.40 * time;
    // Sin tope en 100: así el empate lo decide la calidad de la coincidencia, no el orden
    Some((base + bonus - penalties).max(0.0))
}

/// Candidatos válidos (por encima del umbral mínimo), del mejor al peor. A igualdad de
/// puntuación va antes el que YouTube puso antes.
pub fn ranked(track: &Track, results: &[SearchResult]) -> Vec<Scored> {
    let mut out: Vec<Scored> = results
        .iter()
        .filter_map(|r| score(track, r).map(|score| Scored { result: r.clone(), score }))
        .filter(|s| s.score >= 60.0)
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score)); // el orden es estable
    out
}

/// Mejor candidato, o `None`.
pub fn best(track: &Track, results: &[SearchResult]) -> Option<Scored> {
    ranked(track, results).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, secs: u64) -> Track {
        Track { title: title.into(), artists: vec![artist.into()], duration_ms: secs * 1000, ..Default::default() }
    }
    fn res(id: &str, title: &str, channel: &str, seconds: u64) -> SearchResult {
        SearchResult { id: id.into(), title: title.into(), channel: channel.into(), seconds }
    }

    #[test]
    fn elige_la_grabacion_correcta() {
        let t = track("Never Gonna Give You Up", "Rick Astley", 213);
        let rs = [
            res("live", "Foo Fighters With Rick Astley - Never Gonna Give You Up - Live", "Fan", 278),
            res("cover", "Never Gonna Give You Up (cover)", "Someone", 214),
            res("ok", "Rick Astley - Never Gonna Give You Up (Official Video)", "Rick Astley", 214),
            res("larga", "Rick Astley - Never Gonna Give You Up (10 hours)", "Loop", 36000),
        ];
        assert_eq!(best(&t, &rs).unwrap().result.id, "ok");
    }

    #[test]
    fn prefiere_el_canal_topic_a_igualdad() {
        let t = track("Come Together - Remastered 2009", "The Beatles", 259);
        let rs = [
            res("fan", "The Beatles - Come Together", "fucktown", 261),
            res("topic", "Come Together (Remastered 2009)", "The Beatles - Topic", 259),
        ];
        assert_eq!(best(&t, &rs).unwrap().result.id, "topic");
    }

    #[test]
    fn acentos_y_titulos_con_parentesis() {
        let t = track("Tití Me Preguntó (feat. Alguien)", "Bad Bunny", 243);
        let rs = [res("a", "Bad Bunny - Titi Me Pregunto | Un Verano Sin Ti", "Bad Bunny", 244)];
        assert_eq!(best(&t, &rs).unwrap().result.id, "a");
    }

    #[test]
    fn sin_coincidencia_devuelve_none() {
        let t = track("Canción Rarísima", "Nadie", 200);
        let rs = [res("x", "Otra cosa totalmente distinta", "Canal", 200), res("y", "Canción Rarísima - Nadie", "Nadie", 400)];
        assert!(best(&t, &rs).is_none());
    }

    #[test]
    fn prefiere_el_audio_limpio_al_video_oficial_y_al_de_fans() {
        let t = track("Never Gonna Give You Up", "Rick Astley", 213);
        let rs = [
            res("video", "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)", "Rick Astley", 214),
            res("fan", "【日本語字幕】Rick Astley - Never Gonna Give You Up", "Fan", 214),
            res("audio", "Never Gonna Give You Up", "Rick Astley", 214),
        ];
        assert_eq!(best(&t, &rs).unwrap().result.id, "audio");
    }

    #[test]
    fn descarta_otras_mezclas_y_otros_anos() {
        let t = track("Come Together - Remastered 2009", "The Beatles", 259);
        let rs = [
            res("aniv", "Come Together (50th Anniversary Mix)", "The Beatles", 259),
            res("y2015", "Come Together (Remastered 2015)", "The Beatles", 259),
            res("plano", "The Beatles - Come Together", "The Beatles", 259),
        ];
        assert_eq!(best(&t, &rs).unwrap().result.id, "plano");
    }

    #[test]
    fn respeta_las_versiones_que_pide_spotify() {
        // Si Spotify dice "Live", un vídeo "Live" no debe penalizarse
        let t = track("Song Name - Live", "Artista", 300);
        let rs = [res("l", "Artista - Song Name (Live)", "Artista", 301)];
        assert!(best(&t, &rs).is_some());
    }
}
