//! Cajas MP4: leer bytes con comprobación de límites, construir cajas y recorrer el árbol.
//!
//! Los archivos de entrada no son de fiar (vienen de la red), así que toda lectura de bytes
//! pasa por funciones con comprobación de límites: un MP4 corrupto da error, no un pánico.

use crate::io::read_exact_at;
use crate::Res;
use std::fs::File;

// ---------- lectura de bytes con comprobación de límites ----------

pub(super) fn rd(b: &[u8], o: usize, n: usize) -> Res<&[u8]> {
    o.checked_add(n)
        .and_then(|end| b.get(o..end))
        .ok_or_else(|| "MP4 corrupto: lectura fuera de rango".into())
}
pub(super) fn u8_at(b: &[u8], o: usize) -> Res<u8> {
    Ok(rd(b, o, 1)?[0])
}
pub(super) fn u32_at(b: &[u8], o: usize) -> Res<u32> {
    Ok(u32::from_be_bytes(rd(b, o, 4)?.try_into().unwrap()))
}
pub(super) fn i32_at(b: &[u8], o: usize) -> Res<i32> {
    Ok(i32::from_be_bytes(rd(b, o, 4)?.try_into().unwrap()))
}
pub(super) fn u64_at(b: &[u8], o: usize) -> Res<u64> {
    Ok(u64::from_be_bytes(rd(b, o, 8)?.try_into().unwrap()))
}
pub(super) fn i64_at(b: &[u8], o: usize) -> Res<i64> {
    Ok(i64::from_be_bytes(rd(b, o, 8)?.try_into().unwrap()))
}
pub(super) fn flags_at(b: &[u8], o: usize) -> Res<u32> {
    let f = rd(b, o + 1, 3)?;
    Ok(u32::from_be_bytes([0, f[0], f[1], f[2]]))
}
/// Escribe una duración de 32 o 64 bits en `o`, comprobando que cabe.
pub(super) fn put_dur(b: &mut [u8], o: usize, v: u64, big: bool) -> Res<()> {
    let n = if big { 8 } else { 4 };
    let dst = b.get_mut(o..o + n).ok_or("MP4 corrupto: cabecera demasiado corta")?;
    if big {
        dst.copy_from_slice(&v.to_be_bytes());
    } else {
        dst.copy_from_slice(&(v as u32).to_be_bytes());
    }
    Ok(())
}

// ---------- construcción de cajas ----------

pub(super) fn w32(vals: impl IntoIterator<Item = u32>) -> Vec<u8> {
    vals.into_iter().flat_map(u32::to_be_bytes).collect()
}

pub(super) fn bx(typ: &[u8; 4], parts: Vec<Vec<u8>>) -> Vec<u8> {
    let len: usize = parts.iter().map(Vec::len).sum();
    let mut out = Vec::with_capacity(len + 8);
    out.extend_from_slice(&((len + 8) as u32).to_be_bytes());
    out.extend_from_slice(typ);
    for p in parts {
        out.extend_from_slice(&p);
    }
    out
}

pub(super) fn full_box(typ: &[u8; 4], version: u8, flags: u32, mut parts: Vec<Vec<u8>>) -> Vec<u8> {
    let vf = (u32::from(version) << 24) | (flags & 0x00ff_ffff);
    parts.insert(0, vf.to_be_bytes().to_vec());
    bx(typ, parts)
}

// ---------- lectura de cajas ----------

#[derive(Clone)]
pub(super) struct BoxNode {
    pub(super) typ: [u8; 4],
    pub(super) start: usize,
    pub(super) hdr: usize,
    pub(super) body: usize,
    pub(super) end: usize,
}

pub(super) fn children(buf: &[u8], start: usize, end: usize) -> Res<Vec<BoxNode>> {
    let mut out = Vec::new();
    let mut o = start;
    while o + 8 <= end {
        let mut size = u32_at(buf, o)? as usize;
        let mut hdr = 8;
        let typ: [u8; 4] = rd(buf, o + 4, 4)?.try_into().unwrap();
        if size == 1 {
            size = u64_at(buf, o + 8)? as usize;
            hdr = 16;
        } else if size == 0 {
            size = end - o;
        }
        if size < hdr || o + size > end {
            return Err(format!("Caja MP4 corrupta ({})", String::from_utf8_lossy(&typ)).into());
        }
        out.push(BoxNode { typ, start: o, hdr, body: o + hdr, end: o + size });
        o += size;
    }
    Ok(out)
}

pub(super) fn find(buf: &[u8], parent: &BoxNode, typ: &[u8; 4]) -> Res<BoxNode> {
    children(buf, parent.body, parent.end)?
        .into_iter()
        .find(|c| &c.typ == typ)
        .ok_or_else(|| format!("Falta la caja {} en el MP4 de entrada", String::from_utf8_lossy(typ)).into())
}

pub(super) fn top_of(moov: &[u8]) -> BoxNode {
    BoxNode { typ: *b"moov", start: 0, hdr: 8, body: 8, end: moov.len() }
}

/// Recorre el archivo de arriba abajo y se queda solo con moov y moof (los mdat se leen luego).
pub(super) type Fragments = Vec<(u64, Vec<u8>)>;

pub(super) fn scan_file(file: &File, size: u64) -> Res<(Vec<u8>, Fragments)> {
    let mut moov = None;
    let mut moofs = Vec::new();
    let mut o = 0u64;
    while o + 8 <= size {
        let mut head = [0u8; 16];
        let n = 16.min((size - o) as usize);
        read_exact_at(file, &mut head[..n], o)?;
        let mut len = u32_at(&head, 0)? as u64;
        let typ = &head[4..8];
        if len == 1 {
            len = u64_at(&head, 8)?;
        } else if len == 0 {
            len = size - o;
        }
        if len < 8 || o + len > size {
            return Err(format!("Archivo MP4 incompleto o corrupto (caja {})", String::from_utf8_lossy(typ)).into());
        }
        if typ == b"moov" || typ == b"moof" {
            let mut b = vec![0u8; len as usize];
            read_exact_at(file, &mut b, o)?;
            if typ == b"moov" {
                moov = Some(b);
            } else {
                moofs.push((o, b));
            }
        }
        o += len;
    }
    Ok((moov.ok_or("El MP4 de entrada no tiene moov")?, moofs))
}
