// Codificador FLAC propio (sin dependencias): bloques de 4096 muestras, predictores fijos de
// orden 0-4, codificación Rice con particiones y decorrelación estéreo (L/R, L/S, S/R, M/S).
// Sin pérdida: lo que se decodifica es idéntico a lo que entró.

use crate::media::Tags;
use crate::media::pcm::{PcmFormat, PcmSink};
use crate::Res;
use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

const BLOCK: usize = 4096;

/// Escribe bits de más a menos significativo.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, value: u64, bits: u32) {
        debug_assert!(bits <= 32 && (bits == 64 || value < (1u64 << bits)));
        self.acc = (self.acc << bits) | value;
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.bytes.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }
    fn put_signed(&mut self, v: i64, bits: u32) {
        self.put((v as u64) & ((1u64 << bits) - 1), bits);
    }
    /// Código Rice: cociente en unario (ceros y un 1) y `k` bits de resto
    fn rice(&mut self, v: i64, k: u32) {
        let u = ((v << 1) ^ (v >> 63)) as u64; // signo entrelazado
        let mut q = u >> k;
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(1, q as u32 + 1);
        if k > 0 {
            self.put(u & ((1u64 << k) - 1), k);
        }
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }
    fn bit_len(&self) -> u64 {
        self.bytes.len() as u64 * 8 + self.n as u64
    }
}

fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut c, &b| {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
        c
    })
}

fn crc16(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |mut c, &b| {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
        }
        c
    })
}

/// Residuo del predictor fijo de orden `order` (las primeras `order` muestras van como calentamiento)
fn residual(x: &[i32], order: usize) -> Vec<i64> {
    let s = |i: usize| x[i] as i64;
    (order..x.len())
        .map(|i| match order {
            0 => s(i),
            1 => s(i) - s(i - 1),
            2 => s(i) - 2 * s(i - 1) + s(i - 2),
            3 => s(i) - 3 * s(i - 1) + 3 * s(i - 2) - s(i - 3),
            _ => s(i) - 4 * s(i - 1) + 6 * s(i - 2) - 4 * s(i - 3) + s(i - 4),
        })
        .collect()
}

/// Mejor parámetro Rice para `res` y los bits que cuesta. El óptimo cae muy cerca de log2 de la
/// media del residuo, así que basta probar tres valores en vez de los 15 posibles.
fn best_rice(res: &[i64]) -> (u32, u64) {
    let zigzag = |v: i64| ((v << 1) ^ (v >> 63)) as u64;
    let mean = res.iter().map(|&v| zigzag(v)).sum::<u64>() / res.len().max(1) as u64;
    let k0 = (63 - mean.max(1).leading_zeros()).min(14);
    let mut best = (0, u64::MAX);
    for k in k0.saturating_sub(1)..=(k0 + 1).min(14) {
        let bits: u64 = res.iter().map(|&v| (zigzag(v) >> k) + 1 + k as u64).sum();
        if bits < best.1 {
            best = (k, bits);
        }
    }
    best
}

/// Reparte el residuo en 2^orden particiones eligiendo el orden que menos bits usa
fn plan_partitions(res: &[i64], order_len: usize, block: usize) -> (u32, Vec<u32>) {
    let mut best: Option<(u64, u32, Vec<u32>)> = None;
    for po in 0..=6u32 {
        let parts = 1usize << po;
        if !block.is_multiple_of(parts) || block / parts <= order_len {
            break;
        }
        let per = block / parts;
        let (mut total, mut ks) = (0u64, Vec::new());
        for p in 0..parts {
            let start = if p == 0 { 0 } else { p * per - order_len };
            let end = (p + 1) * per - order_len;
            let (k, bits) = best_rice(&res[start..end]);
            total += bits + 4;
            ks.push(k);
        }
        if best.as_ref().is_none_or(|b| total < b.0) {
            best = Some((total, po, ks));
        }
    }
    let (_, po, ks) = best.unwrap_or((0, 0, vec![0]));
    (po, ks)
}

/// Un canal codificado como subtrama: elige entre constante, fijo (orden 0-4) y sin comprimir
fn subframe(x: &[i32], bps: u32) -> Bits {
    let n = x.len();
    let mut out = Bits::default();
    if x.iter().all(|&v| v == x[0]) {
        out.put(0, 8); // 0 | constante | sin bits desperdiciados
        out.put_signed(x[0] as i64, bps);
        return out;
    }
    let mut best: Option<Bits> = None;
    for order in 0..=4usize.min(n.saturating_sub(1)) {
        let res = residual(x, order);
        let (po, ks) = plan_partitions(&res, order, n);
        let mut b = Bits::default();
        b.put((0b001000 + order as u64) << 1, 8); // 0 | fijo+orden | sin bits desperdiciados
        for &v in &x[..order] {
            b.put_signed(v as i64, bps);
        }
        b.put(0, 2); // método Rice de 4 bits
        b.put(po as u64, 4);
        let per = n >> po;
        for (p, &k) in ks.iter().enumerate() {
            let start = if p == 0 { 0 } else { p * per - order };
            let end = (p + 1) * per - order;
            b.put(k as u64, 4);
            for &v in &res[start..end] {
                b.rice(v, k);
            }
        }
        if best.as_ref().is_none_or(|c| b.bit_len() < c.bit_len()) {
            best = Some(b);
        }
    }
    let mut verbatim = Bits::default();
    verbatim.put(0b000001 << 1, 8);
    for &v in x {
        verbatim.put_signed(v as i64, bps);
    }
    match best {
        Some(b) if b.bit_len() < verbatim.bit_len() => b,
        _ => verbatim,
    }
}

pub struct FlacWriter {
    out: BufWriter<File>,
    tags: Option<Tags>,
    format: Option<PcmFormat>,
    pending: Vec<i16>,
    frame_no: u64,
    total: u64,
    min_frame: u32,
    max_frame: u32,
}

impl FlacWriter {
    pub fn create(path: &Path, tags: Option<&Tags>) -> Res<FlacWriter> {
        Ok(FlacWriter {
            out: BufWriter::new(File::create(path)?),
            tags: tags.cloned(),
            format: None,
            pending: Vec::new(),
            frame_no: 0,
            total: 0,
            min_frame: u32::MAX,
            max_frame: 0,
        })
    }

    fn metadata_block(kind: u8, last: bool, body: &[u8]) -> Vec<u8> {
        let mut b = vec![kind | if last { 0x80 } else { 0 }];
        b.extend(&(body.len() as u32).to_be_bytes()[1..]);
        b.extend(body);
        b
    }

    fn streaminfo(&self, f: PcmFormat) -> Vec<u8> {
        let mut b = Bits::default();
        b.put(BLOCK as u64, 16);
        b.put(BLOCK as u64, 16);
        b.put(if self.min_frame == u32::MAX { 0 } else { self.min_frame as u64 }, 24);
        b.put(self.max_frame as u64, 24);
        b.put(f.sample_rate as u64, 20);
        b.put(f.channels as u64 - 1, 3);
        b.put(15, 5); // 16 bits por muestra
        b.put(self.total, 36);
        b.bytes.extend([0u8; 16]); // MD5 = 0: "no calculado", válido según el formato
        b.bytes
    }

    fn encode_block(&mut self, frames: usize) -> Res<()> {
        let f = self.format.ok_or("FLAC sin iniciar")?;
        let ch = f.channels as usize;
        let block: Vec<i16> = self.pending.drain(..frames * ch).collect();
        let chan = |c: usize| -> Vec<i32> { (0..frames).map(|i| block[i * ch + c] as i32).collect() };

        // (código de asignación de canales, subtramas). El canal "side" lleva 17 bits por muestra.
        let (assign, subs): (u64, Vec<Bits>) = if ch == 1 {
            (0, vec![subframe(&chan(0), 16)])
        } else {
            let (l, r) = (chan(0), chan(1));
            let mid: Vec<i32> = l.iter().zip(&r).map(|(a, b)| (a + b) >> 1).collect();
            let side: Vec<i32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
            let (sl, sr, sm, ss) = (subframe(&l, 16), subframe(&r, 16), subframe(&mid, 16), subframe(&side, 17));
            let cost = |a: &Bits, b: &Bits| a.bit_len() + b.bit_len();
            let options = [(1u64, cost(&sl, &sr)), (8, cost(&sl, &ss)), (9, cost(&ss, &sr)), (10, cost(&sm, &ss))];
            let best = options.iter().min_by_key(|o| o.1).unwrap().0;
            match best {
                8 => (8, vec![sl, ss]),
                9 => (9, vec![ss, sr]),
                10 => (10, vec![sm, ss]),
                _ => (1, vec![sl, sr]),
            }
        };

        let mut h = Bits::default();
        h.put(0xFFF8 >> 1, 15); // sincronización (14 bits) + reservado
        h.put(0, 1); // tamaño de bloque fijo
        let (bs_code, bs_extra) = if frames == BLOCK { (0b1100, None) } else { (0b0111, Some(frames as u64 - 1)) };
        h.put(bs_code, 4);
        let (sr_code, sr_extra): (u64, Option<u64>) = match f.sample_rate {
            44100 => (0b1001, None),
            48000 => (0b1010, None),
            32000 => (0b1000, None),
            88200 => (0b0001, None),
            96000 => (0b0010, None),
            _ => (0b0000, None), // lo dice STREAMINFO
        };
        h.put(sr_code, 4);
        h.put(assign, 4); // 0 = mono, 1 = L/R, 8 = L/S, 9 = S/R, 10 = M/S
        h.put(0b100, 3); // 16 bits por muestra
        h.put(0, 1);
        // número de trama en UTF-8 ampliado
        let n = self.frame_no;
        if n < 0x80 {
            h.put(n, 8);
        } else {
            let mut nbytes = 2u32;
            while n >= 1u64 << (5 * nbytes + 1) {
                nbytes += 1;
            }
            let lead = (0xFF00u64 >> nbytes) & 0xFF;
            h.put(lead | (n >> (6 * (nbytes - 1))), 8);
            for i in (0..nbytes - 1).rev() {
                h.put(0x80 | ((n >> (6 * i)) & 0x3F), 8);
            }
        }
        if let Some(e) = bs_extra {
            h.put(e, 16);
        }
        if let Some(e) = sr_extra {
            h.put(e, 16);
        }
        let mut frame = h.bytes;
        frame.push(crc8(&frame));

        let mut body = Bits::default();
        for s in &subs {
            // las subtramas no están alineadas a byte: se concatenan bit a bit
            for &b in &s.bytes {
                body.put(b as u64, 8);
            }
            if s.n > 0 {
                body.put(s.acc & ((1 << s.n) - 1), s.n);
            }
        }
        body.align();
        frame.extend(&body.bytes);
        let c = crc16(&frame);
        frame.extend(c.to_be_bytes());

        self.min_frame = self.min_frame.min(frame.len() as u32);
        self.max_frame = self.max_frame.max(frame.len() as u32);
        self.out.write_all(&frame)?;
        self.frame_no += 1;
        self.total += frames as u64;
        Ok(())
    }
}

impl PcmSink for FlacWriter {
    fn start(&mut self, f: PcmFormat) -> Res<()> {
        self.format = Some(f);
        let mut head = b"fLaC".to_vec();
        head.extend(Self::metadata_block(0, false, &self.streaminfo(f)));
        if let Some(t) = &self.tags {
            let mut v = Vec::new();
            let vendor = b"resonda";
            v.extend((vendor.len() as u32).to_le_bytes());
            v.extend(vendor);
            let mut comments = vec![format!("TITLE={}", t.title), format!("ARTIST={}", t.artist)];
            if let Some(a) = &t.album {
                comments.push(format!("ALBUM={a}"));
            }
            if let Some(y) = &t.year {
                comments.push(format!("DATE={y}"));
            }
            v.extend((comments.len() as u32).to_le_bytes());
            for c in &comments {
                v.extend((c.len() as u32).to_le_bytes());
                v.extend(c.as_bytes());
            }
            let has_cover = t.cover.is_some();
            head.extend(Self::metadata_block(4, !has_cover, &v));
            if let Some(cover) = &t.cover {
                let mime: &[u8] = if cover.starts_with(&[0x89, b'P']) { b"image/png" } else { b"image/jpeg" };
                let mut p = Vec::new();
                p.extend(3u32.to_be_bytes()); // portada frontal
                p.extend((mime.len() as u32).to_be_bytes());
                p.extend(mime);
                p.extend(0u32.to_be_bytes()); // descripción vacía
                p.extend([0u8; 16]); // ancho, alto, profundidad y colores: sin indicar
                p.extend((cover.len() as u32).to_be_bytes());
                p.extend(cover);
                head.extend(Self::metadata_block(6, true, &p));
            }
        }
        // Sin etiquetas, STREAMINFO tiene que llevar la marca de "último bloque"
        if self.tags.is_none() {
            head[4] |= 0x80;
        }
        self.out.write_all(&head)?;
        Ok(())
    }

    fn write(&mut self, samples: &[i16]) -> Res<()> {
        let ch = self.format.ok_or("FLAC sin iniciar")?.channels as usize;
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= BLOCK * ch {
            self.encode_block(BLOCK)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Res<()> {
        let f = self.format.ok_or("FLAC sin iniciar")?;
        let rest = self.pending.len() / f.channels as usize;
        if rest > 0 {
            self.encode_block(rest)?;
        }
        self.out.flush()?;
        // STREAMINFO se reescribe con el total de muestras y los tamaños de trama reales
        let info = self.streaminfo(f);
        let file = self.out.get_mut();
        file.seek(SeekFrom::Start(8))?;
        file.write_all(&info)?;
        file.flush()?;
        Ok(())
    }
}
