//! Construcción del MP4 de salida: tablas de muestras y reescritura del moov.

use super::boxes::*;
use super::ilst::build_udta;
use super::track::Track;
use crate::media::Tags;
use crate::Res;

// ---------- tablas de muestras del MP4 de salida ----------

pub(super) fn run_length<T: PartialEq + Copy>(values: impl Iterator<Item = T>) -> Vec<(T, u32)> {
    let mut runs: Vec<(T, u32)> = Vec::new();
    for v in values {
        match runs.last_mut() {
            Some((last, n)) if *last == v => *n += 1,
            _ => runs.push((v, 1)),
        }
    }
    runs
}

pub(super) fn build_stbl(t: &Track, use64: bool) -> Vec<u8> {
    let s = &t.samples;
    let mut parts = vec![t.moov[t.stsd.start..t.stsd.end].to_vec()];

    let stts = run_length(s.iter().map(|x| x.dur));
    parts.push(full_box(
        b"stts",
        0,
        0,
        vec![w32([stts.len() as u32]), w32(stts.iter().flat_map(|&(d, n)| [n, d]))],
    ));

    let sync: Vec<u32> = s.iter().enumerate().filter(|(_, x)| x.sync).map(|(i, _)| i as u32 + 1).collect();
    if sync.len() != s.len() {
        parts.push(full_box(b"stss", 0, 0, vec![w32([sync.len() as u32]), w32(sync)]));
    }

    if s.iter().any(|x| x.cto != 0) {
        let ctts = run_length(s.iter().map(|x| x.cto));
        let signed = s.iter().any(|x| x.cto < 0);
        parts.push(full_box(
            b"ctts",
            signed as u8,
            0,
            vec![w32([ctts.len() as u32]), w32(ctts.iter().flat_map(|&(c, n)| [n, c as u32]))],
        ));
    }

    // stsc: primer chunk, muestras por chunk, índice de descripción (siempre 1)
    let mut stsc: Vec<[u32; 3]> = Vec::new();
    for (i, c) in t.chunks.iter().enumerate() {
        if stsc.last().is_none_or(|e| e[1] != c.count) {
            stsc.push([i as u32 + 1, c.count, 1]);
        }
    }
    parts.push(full_box(b"stsc", 0, 0, vec![w32([stsc.len() as u32]), w32(stsc.into_iter().flatten())]));
    parts.push(full_box(b"stsz", 0, 0, vec![w32([0, s.len() as u32]), w32(s.iter().map(|x| x.size))]));

    if use64 {
        let offs = t.chunks.iter().flat_map(|c| c.out.to_be_bytes()).collect();
        parts.push(full_box(b"co64", 0, 0, vec![w32([t.chunks.len() as u32]), offs]));
    } else {
        parts.push(full_box(b"stco", 0, 0, vec![w32([t.chunks.len() as u32]), w32(t.chunks.iter().map(|c| c.out as u32))]));
    }
    bx(b"stbl", parts)
}

// ---------- reescritura del moov ----------

pub(super) enum Patch {
    Keep,
    Drop,
    Replace(Vec<u8>),
}

/// Copia un árbol de cajas aplicando `patch`: sustituye, elimina o conserva (descendiendo
/// si es un contenedor).
pub(super) fn rewrite(buf: &[u8], node: &BoxNode, patch: &dyn Fn(&BoxNode) -> Res<Patch>) -> Res<Option<Vec<u8>>> {
    match patch(node)? {
        Patch::Replace(v) => return Ok(Some(v)),
        Patch::Drop => return Ok(None),
        Patch::Keep => {}
    }
    if !matches!(&node.typ, b"trak" | b"mdia" | b"minf" | b"edts") {
        return Ok(Some(buf[node.start..node.end].to_vec()));
    }
    let mut kids = Vec::new();
    for c in children(buf, node.body, node.end)? {
        if let Some(r) = rewrite(buf, &c, patch)? {
            kids.push(r);
        }
    }
    Ok(Some(bx(&node.typ, kids)))
}

pub(super) fn build_trak(t: &Track, track_id: u32, movie_ts: u32, use64: bool) -> Res<Vec<u8>> {
    let b = &t.moov;
    let dur_movie = t.movie_duration(movie_ts);

    let patch = |n: &BoxNode| -> Res<Patch> {
        Ok(match &n.typ {
            b"tkhd" => {
                let mut c = b[n.start..n.end].to_vec();
                let v1 = u8_at(&c, n.hdr)? == 1;
                let id_off = n.hdr + if v1 { 20 } else { 12 };
                c.get_mut(id_off..id_off + 4).ok_or("tkhd corrupto")?.copy_from_slice(&track_id.to_be_bytes());
                put_dur(&mut c, n.hdr + if v1 { 28 } else { 20 }, dur_movie, v1)?;
                Patch::Replace(c)
            }
            b"mdhd" => {
                let mut c = b[n.start..n.end].to_vec();
                let v1 = u8_at(&c, n.hdr)? == 1;
                put_dur(&mut c, n.hdr + if v1 { 24 } else { 16 }, t.duration, v1)?;
                Patch::Replace(c)
            }
            b"elst" => {
                // La duración de cada segmento tiene que ajustarse a la pista ya completa
                let mut c = b[n.start..n.end].to_vec();
                let v1 = u8_at(&c, n.hdr)? == 1;
                let entry = if v1 { 20 } else { 12 };
                for i in 0..u32_at(&c, n.hdr + 4)? as usize {
                    let o = n.hdr + 8 + i * entry;
                    let mt = if v1 { i64_at(&c, o + 8)? } else { i32_at(&c, o + 4)? as i64 };
                    if mt == -1 {
                        continue;
                    }
                    let seg = (dur_movie as f64 - mt as f64 / t.timescale as f64 * movie_ts as f64).round().max(0.0);
                    put_dur(&mut c, o, seg as u64, v1)?;
                }
                Patch::Replace(c)
            }
            b"stbl" => Patch::Replace(build_stbl(t, use64)),
            b"udta" => Patch::Drop,
            _ => Patch::Keep,
        })
    };
    rewrite(b, &t.trak, &patch)?.ok_or_else(|| "trak vacío".into())
}

pub(super) fn build_moov(tracks: &[Track], movie_ts: u32, use64: bool, tags: Option<&Tags>) -> Res<Vec<u8>> {
    let first = &tracks[0];
    let mvhd = find(&first.moov, &top_of(&first.moov), b"mvhd")?;
    let mut copy = first.moov[mvhd.start..mvhd.end].to_vec();
    let v1 = u8_at(&copy, mvhd.hdr)? == 1;
    let dur = tracks.iter().map(|t| t.movie_duration(movie_ts)).max().unwrap_or(0);
    put_dur(&mut copy, mvhd.hdr + if v1 { 24 } else { 16 }, dur, v1)?;
    let n = copy.len();
    copy[n - 4..].copy_from_slice(&(tracks.len() as u32 + 1).to_be_bytes()); // next_track_ID

    let mut parts = vec![copy];
    for (i, t) in tracks.iter().enumerate() {
        parts.push(build_trak(t, i as u32 + 1, movie_ts, use64)?);
    }
    if let Some(t) = tags {
        parts.push(build_udta(t));
    }
    Ok(bx(b"moov", parts))
}
