//! Etiquetas dentro del MP4/M4A (formato "ilst" de iTunes, lo entienden todos los reproductores).

use super::boxes::{bx, full_box, w32};
use crate::media::Tags;

pub(super) fn tag_item(typ: [u8; 4], data_type: u32, payload: &[u8]) -> Vec<u8> {
    // "data": tipo (1 = texto UTF-8, 13 = JPEG, 14 = PNG) + configuración regional (0) + contenido
    bx(&typ, vec![full_box(b"data", 0, data_type, vec![w32([0]), payload.to_vec()])])
}

pub(super) fn build_udta(t: &Tags) -> Vec<u8> {
    const NAM: [u8; 4] = [0xa9, b'n', b'a', b'm'];
    const ART: [u8; 4] = [0xa9, b'A', b'R', b'T'];
    const ALB: [u8; 4] = [0xa9, b'a', b'l', b'b'];
    const DAY: [u8; 4] = [0xa9, b'd', b'a', b'y'];
    let mut items = vec![tag_item(NAM, 1, t.title.as_bytes()), tag_item(ART, 1, t.artist.as_bytes())];
    if let Some(a) = &t.album {
        items.push(tag_item(ALB, 1, a.as_bytes()));
    }
    if let Some(y) = &t.year {
        items.push(tag_item(DAY, 1, y.as_bytes()));
    }
    if let Some(c) = &t.cover {
        let kind = if c.starts_with(&[0x89, b'P', b'N', b'G']) { 14 } else { 13 };
        items.push(tag_item(*b"covr", kind, c));
    }
    let hdlr = full_box(b"hdlr", 0, 0, vec![w32([0]), b"mdir".to_vec(), b"appl".to_vec(), w32([0, 0]), vec![0]]);
    bx(b"udta", vec![full_box(b"meta", 0, 0, vec![hdlr, bx(b"ilst", items)])])
}
