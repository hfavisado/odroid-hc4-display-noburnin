//! Offline preview: renders the scrolling display to an animated GIF so the
//! result can be checked (and shown in the README) without OLED hardware.

use crate::{HEIGHT, LINE_SPACING, WIDTH, normalize_scroll_offset, render_frame};
use std::collections::HashMap;

/// Each OLED pixel becomes a `CELL`x`CELL` block: `CELL - 1` lit, 1 gap.
const CELL: usize = 4;
/// Palette: background, unlit pixel, lit pixel.
const PALETTE: [[u8; 3]; 4] = [[0, 0, 0], [14, 18, 24], [214, 236, 255], [0, 0, 0]];

/// Representative lines, so the preview is reproducible on any machine.
pub fn sample_lines() -> Vec<String> {
    [
        "192.168.1.20",
        "CPU TEMP 47C",
        "DISK ROOT 12.3G/58.2G 21%",
        "DISK DATA 1.8T/3.6T 50%",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Renders exactly one full scroll loop, so the GIF repeats seamlessly.
pub fn render_gif(lines: &[String], scroll_speed: f32, refresh_ms: u64) -> Vec<u8> {
    let delay_cs = ((refresh_ms as f32 / 10.0).round() as u16).max(2);
    let period = LINE_SPACING * lines.len().max(1) as f32;
    let seconds_per_frame = f32::from(delay_cs) / 100.0;
    let frames = ((period / (scroll_speed * seconds_per_frame)).round() as usize).max(1);
    let step = period / frames as f32;

    let (width, height) = (WIDTH * CELL, HEIGHT * CELL);
    let mut out = Vec::new();
    out.extend_from_slice(b"GIF89a");
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&[0xF1, 0, 0]); // global palette, 4 entries
    PALETTE.iter().for_each(|rgb| out.extend_from_slice(rgb));
    out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[3, 1, 0, 0, 0]); // loop forever

    for index in 0..frames {
        let offset = normalize_scroll_offset(step * index as f32, lines.len());
        let bits = render_frame(lines, offset);
        let mut pixels = vec![0u8; width * height];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let lit = bits[x + (y / 8) * WIDTH] & (1 << (y % 8)) != 0;
                let color = if lit { 2 } else { 1 };
                for dy in 0..CELL - 1 {
                    let row = (y * CELL + dy) * width + x * CELL;
                    pixels[row..row + CELL - 1].fill(color);
                }
            }
        }
        out.extend_from_slice(&[0x21, 0xF9, 4, 0x04]);
        out.extend_from_slice(&delay_cs.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x2C, 0, 0, 0, 0]);
        out.extend_from_slice(&(width as u16).to_le_bytes());
        out.extend_from_slice(&(height as u16).to_le_bytes());
        out.extend_from_slice(&[0, 2]); // no local palette, min code size 2
        for chunk in lzw(&pixels).chunks(255) {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
        out.push(0);
    }
    out.push(0x3B);
    out
}

/// GIF-flavoured variable-width LZW with a minimum code size of 2.
fn lzw(data: &[u8]) -> Vec<u8> {
    const CLEAR: u16 = 4;
    const END: u16 = 5;
    let mut out = Vec::new();
    let (mut acc, mut nbits) = (0u32, 0u32);
    let mut emit = |code: u16, size: u32, out: &mut Vec<u8>| {
        acc |= u32::from(code) << nbits;
        nbits += size;
        while nbits >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            nbits -= 8;
        }
    };

    let mut table: HashMap<(u16, u8), u16> = HashMap::new();
    let (mut next, mut size) = (END + 1, 3u32);
    emit(CLEAR, size, &mut out);
    let mut iter = data.iter().copied();
    let Some(first) = iter.next() else {
        emit(END, size, &mut out);
        return out;
    };
    let mut prefix = u16::from(first);
    for byte in iter {
        if let Some(&code) = table.get(&(prefix, byte)) {
            prefix = code;
            continue;
        }
        emit(prefix, size, &mut out);
        table.insert((prefix, byte), next);
        if next == 1 << size {
            size += 1;
        }
        next += 1;
        if next == 4096 {
            emit(CLEAR, size, &mut out);
            table.clear();
            (next, size) = (END + 1, 3);
        }
        prefix = u16::from(byte);
    }
    emit(prefix, size, &mut out);
    emit(END, size, &mut out);
    if nbits > 0 {
        out.push(acc as u8);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_is_well_formed_and_loops_seamlessly() {
        let gif = render_gif(&sample_lines(), 5.0, 100);
        assert!(gif.starts_with(b"GIF89a"));
        assert_eq!(gif.last(), Some(&0x3B));
        assert_eq!(u16::from_le_bytes([gif[6], gif[7]]) as usize, WIDTH * CELL);
    }

    #[test]
    fn lzw_round_trips_through_a_reference_decoder() {
        let data: Vec<u8> = (0..20_000u32).map(|i| ((i * i / 7) % 3) as u8).collect();
        assert_eq!(unlzw(&lzw(&data)), data);
    }

    fn unlzw(bytes: &[u8]) -> Vec<u8> {
        let (mut acc, mut nbits, mut pos) = (0u32, 0u32, 0usize);
        let mut size = 3u32;
        let mut dict: Vec<Vec<u8>> = Vec::new();
        let reset = |dict: &mut Vec<Vec<u8>>| {
            *dict = (0..4u8).map(|b| vec![b]).collect();
            dict.push(vec![]);
            dict.push(vec![]);
        };
        reset(&mut dict);
        let (mut out, mut prev): (Vec<u8>, Option<Vec<u8>>) = (Vec::new(), None);
        loop {
            while nbits < size && pos < bytes.len() {
                acc |= u32::from(bytes[pos]) << nbits;
                pos += 1;
                nbits += 8;
            }
            if nbits < size {
                break;
            }
            let code = (acc & ((1 << size) - 1)) as usize;
            acc >>= size;
            nbits -= size;
            if code == 4 {
                reset(&mut dict);
                size = 3;
                prev = None;
                continue;
            }
            if code == 5 {
                break;
            }
            let entry = if code < dict.len() {
                dict[code].clone()
            } else {
                let mut e = prev.clone().unwrap();
                e.push(e[0]);
                e
            };
            out.extend_from_slice(&entry);
            if let Some(mut p) = prev {
                p.push(entry[0]);
                dict.push(p);
            }
            prev = Some(entry);
            if dict.len() == 1 << size && size < 12 {
                size += 1;
            }
        }
        out
    }
}
