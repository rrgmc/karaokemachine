//! Writes a synthetic `.cdg` for checking that an MP3+G song's words keep time with its sound.
//!
//! Eight word blocks sit on a line. From the first second on, block `n` turns from white to yellow
//! **at the start of second `n`**, and a fresh white line replaces the old one every eight
//! seconds. Paired with an MP3 that beeps at the start of each second, such as ffmpeg's
//! `sine=frequency=440:beep_factor=4`, each block should light on its beep. Nobody's content is in
//! it, so it can be built anywhere and carried in a test package.
//!
//! ```sh
//! cargo run -p km-cdg --example sync_song -- out.cdg            # 30 seconds
//! cargo run -p km-cdg --example sync_song -- out.cdg --seconds 60
//! ```

use std::path::PathBuf;

use km_cdg::{PACKET_BYTES, PACKETS_PER_SECOND};

const CMD_CDG: u8 = 9;
const MEMORY_PRESET: u8 = 1;
const BORDER_PRESET: u8 = 2;
const TILE_BLOCK: u8 = 6;
const LOAD_CLUT_LO: u8 = 30;

/// Palette indices.
const BACKGROUND: u8 = 0;
const WHITE: u8 = 1;
const YELLOW: u8 = 2;

/// The line: eight blocks, each four tiles wide and two high, five tile columns apart.
const WORDS: u32 = 8;
const WORD_COLUMNS: u8 = 4;
const WORD_ROWS: u8 = 2;
const FIRST_COLUMN: u8 = 5;
const COLUMN_STEP: u8 = 5;
const LINE_ROW: u8 = 8;

/// How far ahead of its first word a line is drawn: a third of a second, long enough for its
/// sixty-four tiles and short enough that the previous line's last word is seen lit.
const LINE_LEAD: u32 = PACKETS_PER_SECOND / 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut seconds = 30u32;
    while let Some(arg) = args.next() {
        match arg.to_string_lossy().as_ref() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or("--seconds needs a number")?
                    .to_string_lossy()
                    .parse()?;
            }
            _ if out.is_none() => out = Some(arg.into()),
            other => return Err(format!("unexpected argument {other:?}").into()),
        }
    }
    let out = out.ok_or("usage: sync_song <out.cdg> [--seconds N]")?;
    if seconds < 2 {
        return Err("--seconds must be at least 2".into());
    }

    let mut song = Song::new(seconds * PACKETS_PER_SECOND);

    // Dark blue, white, yellow: twelve bits each, as `xxrrrrgg xxggbbbb`.
    let mut clut = [0u8; 16];
    for (i, (r, g, b)) in [(0u8, 0u8, 6u8), (15, 15, 15), (15, 13, 0)]
        .iter()
        .enumerate()
    {
        clut[i * 2] = (r << 2) | (g >> 2);
        clut[i * 2 + 1] = ((g & 0x03) << 4) | b;
    }
    let mut at = 0;
    at = song.put(at, LOAD_CLUT_LO, &clut);
    at = song.put(at, MEMORY_PRESET, &[BACKGROUND]);
    song.put(at, BORDER_PRESET, &[BACKGROUND]);

    // Second `s` (from 1) lights word `(s - 1) % 8` of the line that started at its multiple of 8.
    for second in 1..seconds {
        let word = (second - 1) % WORDS;
        let start = second * PACKETS_PER_SECOND;
        if word == 0 {
            let mut p = start - LINE_LEAD;
            for w in 0..WORDS {
                p = song.word(p, w, WHITE);
            }
        }
        song.word(start, word, YELLOW);
    }

    std::fs::write(&out, song.bytes())?;
    println!(
        "{}: {seconds} s, {} packets",
        out.display(),
        song.packets.len()
    );
    Ok(())
}

struct Song {
    packets: Vec<[u8; PACKET_BYTES]>,
}

impl Song {
    fn new(packets: u32) -> Self {
        Self {
            packets: vec![[0u8; PACKET_BYTES]; packets as usize],
        }
    }

    /// Writes one instruction at packet `at` and returns the packet after it.
    fn put(&mut self, at: u32, instruction: u8, data: &[u8]) -> u32 {
        let packet = &mut self.packets[at as usize];
        packet[0] = CMD_CDG;
        packet[1] = instruction;
        packet[4..4 + data.len()].copy_from_slice(data);
        at + 1
    }

    /// Fills word `word`'s tiles with `color`, from packet `at`.
    fn word(&mut self, mut at: u32, word: u32, color: u8) -> u32 {
        let column = FIRST_COLUMN + COLUMN_STEP * word as u8;
        for row in 0..WORD_ROWS {
            for dx in 0..WORD_COLUMNS {
                let mut data = [0u8; 16];
                data[0] = BACKGROUND;
                data[1] = color;
                data[2] = LINE_ROW + row;
                data[3] = column + dx;
                // Every pixel of the tile in `color`.
                data[4..].fill(0x3F);
                at = self.put(at, TILE_BLOCK, &data);
            }
        }
        at
    }

    fn bytes(&self) -> Vec<u8> {
        self.packets.concat()
    }
}
