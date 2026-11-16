//! Offline preview: renders the paged display to an animated GIF so the
//! result can be checked (and shown in the README) without OLED hardware.

use crate::{Animation, HEIGHT, Phase, WIDTH, page_cycle_length};
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

/// Shows four to eight pages of the animated sequence.
pub fn render_gif(lines: &[String], page_interval_ms: u64) -> Vec<u8> {
    const FRAME_MS: u64 = 100;
    let cycle = page_cycle_length(lines);
    let pages = if cycle == 0 { 0 } else { cycle.clamp(4, 8) } as u64;

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

    let mut animation = Animation::default();
    let mut returning_to_start = false;
    loop {
        let bits = animation.render(lines, page_interval_ms);
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
        out.extend_from_slice(&((FRAME_MS / 10) as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0, 0x2C, 0, 0, 0, 0]);
        out.extend_from_slice(&(width as u16).to_le_bytes());
        out.extend_from_slice(&(height as u16).to_le_bytes());
        out.extend_from_slice(&[0, 2]); // no local palette, min code size 2
        for chunk in lzw(&pixels).chunks(255) {
            out.push(chunk.len() as u8);
            out.extend_from_slice(chunk);
        }
        out.push(0);
        if pages == 0 {
            break;
        }
        animation.advance(FRAME_MS, lines, page_interval_ms);
        if !returning_to_start && animation.page_index >= pages {
            animation.page_index = 0;
            returning_to_start = true;
        }
        if returning_to_start && animation.phase == Phase::Hold {
            break;
        }
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
    fn gif_is_well_formed_and_contains_animation_frames() {
        let gif = render_gif(&sample_lines(), 5000);
        assert!(gif.starts_with(b"GIF89a"));
        assert_eq!(gif.last(), Some(&0x3B));
        assert_eq!(u16::from_le_bytes([gif[6], gif[7]]) as usize, WIDTH * CELL);
        let control = gif
            .windows(4)
            .position(|bytes| bytes == [0x21, 0xF9, 4, 0x04])
            .unwrap();
        assert_eq!(u16::from_le_bytes([gif[control + 4], gif[control + 5]]), 10);
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
