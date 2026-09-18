//! The CD+G renderer: packets in, a picture out.
//!
//! # The surface holds palette indices, not colors
//!
//! This is not an optimization and undoing it would break real discs. `LOAD_CLUT` replaces the
//! palette, and everything already on screen changes color with it — which is how a great many
//! discs fade words in and out without redrawing a single tile. A surface of resolved pixels would
//! make that instruction do nothing at all, and the failure would look like "the fades are missing"
//! rather than like a bug.
//!
//! # Every byte is masked
//!
//! A `.cdg` is a copy of the CD's R–W subchannels, and real rips keep the P and Q bits in the top
//! two bits of every byte. Measured across a real corpus: every file examined had them set. So the
//! low six bits are the data and `& 0x3F` is not defensive, it is the format.
//!
//! # Damage is counted, never raised — and most of what looks like damage is not
//!
//! A renderer that returned an error on the first unrecognized packet would refuse files that play
//! perfectly, so anything unrecognized increments a counter and is skipped.
//!
//! The sharper lesson, learned from a real file and easy to get backwards: **a packet whose command
//! byte is not 9 is not damaged, it is simply not CD+G.** A `.cdg` is a copy of the R–W subchannel,
//! and a rip that kept all of it carries other applications' packs alongside the graphics ones. One
//! disc measured here is 74% such packs and renders three clean lines of karaoke. Treating them as
//! corruption — which this crate did for exactly one afternoon — would have a packaging check reject
//! songs that work.
//!
//! The same turned out to be true one level down, and it is why [`GraphicsStats`] counts two things
//! rather than one. A CD+G command carrying an instruction nobody implements is *also* not damage:
//! 223 files of 2,849 contain some, the worst is 29% of its packets, and all of them render
//! perfectly — instruction 9 alone is almost the whole of it, across discs from several publishers.
//! A tile addressed off the screen is a different matter and is counted separately: those are real
//! rip damage, and 129 files carry between one and fourteen of them. They are still not worth
//! refusing a song over, because a handful of dropped tiles in a hundred thousand packets is a
//! blemish nobody sees.
//!
//! **What actually separates a worthless file from a good one is
//! [`GraphicsStats::tiles_written`]**: no tiles means no words, whatever else is in the file. It is
//! the only signal a packaging check should refuse a song over — and across 2,849 real files and
//! 208 million packets, not one file failed it and not one replay panicked.

#[cfg(test)]
mod tests;

use std::sync::Mutex;

use crate::{
    BORDER_X, BORDER_Y, HEIGHT, PACKET_BYTES, PACKETS_PER_SECOND, VISIBLE_HEIGHT, VISIBLE_WIDTH,
    WIDTH,
};

/// The six low bits of a subcode byte are the CD+G data; the top two are the P and Q subchannels.
const MASK: u8 = 0x3F;

/// The one command byte that carries a CD+G instruction. Every other packet is filler.
const CMD_CDG: u8 = 9;

const MEMORY_PRESET: u8 = 1;
const BORDER_PRESET: u8 = 2;
const TILE_BLOCK: u8 = 6;
const SCROLL_PRESET: u8 = 20;
const SCROLL_COPY: u8 = 24;
const DEFINE_TRANSPARENT: u8 = 28;
const LOAD_CLUT_LO: u8 = 30;
const LOAD_CLUT_HI: u8 = 31;
const TILE_BLOCK_XOR: u8 = 38;

const TILE_WIDTH: usize = 6;
const TILE_HEIGHT: usize = 12;

const W: usize = WIDTH as usize;
const H: usize = HEIGHT as usize;
const VW: usize = VISIBLE_WIDTH as usize;
const VH: usize = VISIBLE_HEIGHT as usize;

/// How far the position may run *backwards* before the screen is rebuilt from packet zero.
///
/// **Not zero, and this is the entire reason the constant exists.** `km-app`'s display smooths
/// `position_ms` between audio callbacks, so it can overshoot the true position by a fraction of a
/// period and then step back. Rebuilding a hundred thousand packets on that wobble would be a hitch
/// on every drawn frame, appearing as a flicker nobody could place. A quarter of a second is far
/// above any smoothing jitter and far below any seek a person would ask for.
pub const REWIND_SLACK_MS: u32 = 250;

/// What applying one packet did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// Changed what is on screen.
    Drew,
    /// Understood, and changed nothing visible. Filler packets, which are most of a stream.
    Ignored,
    /// A CD+G instruction this renderer does not implement.
    ///
    /// Counted rather than raised, and **counted rather than judged**: 223 files of 2,849 measured
    /// contain some, the worst is 29% of its packets, and every one of them renders perfectly.
    /// Instruction 9 alone accounts for almost all of it and appears on discs from several
    /// publishers, so this is a manufacturer's extension rather than damage.
    UnknownInstruction,
    /// A tile addressed off the 300x216 screen.
    ///
    /// Separate from an unknown instruction because it means something different: an instruction
    /// nobody implements is a disc using an extension, whereas a tile at column 60 is a packet whose
    /// bytes are wrong. 129 files of 2,849 carry between one and fourteen.
    OffScreenTile,
}

/// The CD+G screen: a palette and 300x216 indices into it.
#[derive(Debug, Clone)]
pub struct Screen {
    indices: Vec<u8>,
    /// Reused by [`Screen::scroll`] so a scrolling disc allocates nothing per packet.
    scratch: Vec<u8>,
    clut: [u32; 16],
    border: u8,
    h_offset: u8,
    v_offset: u8,
    transparent: Option<u8>,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    /// A blank screen: every pixel index 0, every palette entry opaque black.
    #[must_use]
    pub fn new() -> Self {
        Self {
            indices: vec![0; W * H],
            scratch: vec![0; W * H],
            clut: [0xFF00_0000; 16],
            border: 0,
            h_offset: 0,
            v_offset: 0,
            transparent: None,
        }
    }

    /// Returns to the blank state, for replaying a stream from its beginning.
    pub fn reset(&mut self) {
        self.indices.fill(0);
        self.clut = [0xFF00_0000; 16];
        self.border = 0;
        self.h_offset = 0;
        self.v_offset = 0;
        self.transparent = None;
    }

    /// The palette index at a pixel of the whole screen, border included.
    #[must_use]
    pub fn index_at(&self, x: u32, y: u32) -> u8 {
        self.indices[y as usize * W + x as usize]
    }

    /// A palette entry, as opaque `0xAARRGGBB`.
    #[must_use]
    pub fn color(&self, index: u8) -> u32 {
        self.clut[usize::from(index & 0x0F)]
    }

    /// The palette index the border is filled with.
    #[must_use]
    pub fn border(&self) -> u8 {
        self.border
    }

    /// Where the visible window sits within the screen, from the last scroll instruction.
    #[must_use]
    pub fn offsets(&self) -> (u8, u8) {
        (self.h_offset, self.v_offset)
    }

    /// The palette index a disc asked to be treated as transparent, if it asked.
    ///
    /// **Recorded and not acted on.** CD+G transparency exists so a hardware unit can key the
    /// graphics over the disc's own video, which an audio CD does not have. Here the picture goes
    /// exactly where the wallpaper goes and must be opaque — letting a photograph show through the
    /// words would be worse than anything a disc intended — so the index is drawn in its own palette
    /// color, which is what the author saw on any display that was not keying. It appeared 36 times
    /// in an 800,000-packet sample, so the choice costs nothing either way; it still had to be made.
    #[must_use]
    pub fn transparent(&self) -> Option<u8> {
        self.transparent
    }

    /// Applies one packet, and says what that did.
    ///
    /// `packet` must be [`crate::PACKET_BYTES`] long.
    pub fn apply(&mut self, packet: &[u8; PACKET_BYTES]) -> Applied {
        match packet[0] & MASK {
            CMD_CDG => {
                let data: &[u8; 16] = packet[4..20].try_into().unwrap_or(&[0; 16]);
                match packet[1] & MASK {
                    MEMORY_PRESET => {
                        // The repeat count in `data[1]` is deliberately ignored: the instruction is
                        // idempotent, discs send it up to sixteen times for reliability over a
                        // scratched disc, and honoring the counter would only add a way to get it
                        // wrong.
                        self.indices.fill(data[0] & 0x0F);
                        Applied::Drew
                    }
                    BORDER_PRESET => {
                        self.border_preset(data[0] & 0x0F);
                        Applied::Drew
                    }
                    TILE_BLOCK => self.tile_block(data, false),
                    TILE_BLOCK_XOR => self.tile_block(data, true),
                    SCROLL_PRESET => self.scroll(data, false),
                    SCROLL_COPY => self.scroll(data, true),
                    DEFINE_TRANSPARENT => {
                        self.transparent = Some(data[0] & 0x0F);
                        Applied::Ignored
                    }
                    LOAD_CLUT_LO => {
                        self.load_clut(data, 0);
                        Applied::Drew
                    }
                    LOAD_CLUT_HI => {
                        self.load_clut(data, 8);
                        Applied::Drew
                    }
                    _ => Applied::UnknownInstruction,
                }
            }
            // Any other command is a subcode pack belonging to something that is not CD+G, and that
            // is **normal, not damage**. Mostly it is zero — the filler a CD carries whether or not
            // anything is being drawn — but a rip that kept the whole R-W subchannel carries other
            // applications' packs too, and one real file measured here is 74% of them and renders
            // perfectly. Counting these as damage would make a packaging check refuse songs that
            // play, which is exactly the mistake this crate's "skip and carry on" rule exists to
            // avoid. What tells a worthless file from a good one is `tiles_written`, not this.
            _ => Applied::Ignored,
        }
    }

    fn border_preset(&mut self, color: u8) {
        self.border = color;
        let top = BORDER_Y as usize;
        let bottom = H - BORDER_Y as usize;
        let left = BORDER_X as usize;
        let right = W - BORDER_X as usize;
        self.indices[..top * W].fill(color);
        self.indices[bottom * W..].fill(color);
        for row in self.indices[top * W..bottom * W].as_chunks_mut::<W>().0 {
            row[..left].fill(color);
            row[right..].fill(color);
        }
    }

    fn tile_block(&mut self, data: &[u8; 16], xor: bool) -> Applied {
        let color0 = data[0] & 0x0F;
        let color1 = data[1] & 0x0F;
        let x0 = usize::from(data[3] & 0x3F) * TILE_WIDTH;
        let y0 = usize::from(data[2] & 0x1F) * TILE_HEIGHT;
        if x0 + TILE_WIDTH > W || y0 + TILE_HEIGHT > H {
            // A real disc never addresses a tile off the screen; a corrupt packet does. Counted with
            // the unrecognized ones, because both mean the same thing about the stream.
            return Applied::OffScreenTile;
        }

        for (dy, byte) in data[4..4 + TILE_HEIGHT].iter().enumerate() {
            let bits = byte & MASK;
            let start = (y0 + dy) * W + x0;
            let row = &mut self.indices[start..start + TILE_WIDTH];
            for (dx, cell) in row.iter_mut().enumerate() {
                // Bit 5 is the leftmost pixel of the six.
                let value = if bits & (0x20 >> dx) == 0 {
                    color0
                } else {
                    color1
                };
                *cell = if xor { *cell ^ value } else { value };
            }
        }
        Applied::Drew
    }

    fn load_clut(&mut self, data: &[u8; 16], base: usize) {
        for (i, pair) in data.as_chunks::<2>().0.iter().enumerate() {
            let (high, low) = (pair[0] & MASK, pair[1] & MASK);
            // Twelve bits across two six-bit bytes: `xxrrrrgg` then `xxggbbbb`.
            let red = high >> 2;
            let green = ((high & 0x03) << 2) | ((low & 0x30) >> 4);
            let blue = low & 0x0F;
            self.clut[base + i] = argb(red, green, blue);
        }
    }

    fn scroll(&mut self, data: &[u8; 16], copy: bool) -> Applied {
        let color = data[0] & 0x0F;
        let horizontal = data[1] & MASK;
        let vertical = data[2] & MASK;

        // The offset moves the visible window within the screen; the command shifts the screen
        // itself by a whole tile. A packet may do either, both, or neither.
        self.h_offset = (horizontal & 0x07).min(BORDER_X as u8 - 1);
        self.v_offset = (vertical & 0x0F).min(BORDER_Y as u8 - 1);

        let dx: isize = match (horizontal & 0x30) >> 4 {
            1 => TILE_WIDTH as isize,
            2 => -(TILE_WIDTH as isize),
            _ => 0,
        };
        let dy: isize = match (vertical & 0x30) >> 4 {
            1 => TILE_HEIGHT as isize,
            2 => -(TILE_HEIGHT as isize),
            _ => 0,
        };
        if dx != 0 || dy != 0 {
            self.shift(dx, dy, copy, color);
        }
        Applied::Drew
    }

    /// Moves the whole screen by `(dx, dy)`.
    ///
    /// `copy` is what separates the two scroll instructions: `SCROLL_COPY` wraps what falls off one
    /// edge round to the other, `SCROLL_PRESET` fills the vacated strip with a color.
    fn shift(&mut self, dx: isize, dy: isize, copy: bool, color: u8) {
        let (w, h) = (W as isize, H as isize);
        self.scratch.fill(color);
        for y in 0..h {
            for x in 0..w {
                let (mut sx, mut sy) = (x - dx, y - dy);
                if copy {
                    sx = sx.rem_euclid(w);
                    sy = sy.rem_euclid(h);
                } else if sx < 0 || sx >= w || sy < 0 || sy >= h {
                    continue;
                }
                let (to, from) = ((y * w + x) as usize, (sy * w + sx) as usize);
                self.scratch[to] = self.indices[from];
            }
        }
        std::mem::swap(&mut self.indices, &mut self.scratch);
    }

    /// Writes the visible window as `ARGB8888`.
    ///
    /// `out` must be `VISIBLE_WIDTH * VISIBLE_HEIGHT * 4` bytes.
    ///
    /// **Native-endian bytes, deliberately.** SDL's `*_8888` names are endian-dependent packed
    /// formats, so the native-endian bytes of a packed `0xAARRGGBB` match `PixelFormat::ARGB8888`
    /// on either endianness. Writing the channels out by hand in a fixed order would be right on one
    /// and silently wrong-colored on the other.
    pub fn blit_visible(&self, out: &mut [u8]) {
        let x0 = BORDER_X as usize + usize::from(self.h_offset);
        let y0 = BORDER_Y as usize + usize::from(self.v_offset);
        for (row, out_row) in out
            .as_chunks_mut::<{ VW * 4 }>()
            .0
            .iter_mut()
            .take(VH)
            .enumerate()
        {
            let start = (y0 + row) * W + x0;
            let indices = &self.indices[start..start + VW];
            // `as_chunks_mut` hands out `&mut [u8; 4]` rather than a slice, so the pixel is a whole
            // assignment instead of a `copy_from_slice` that could in principle mismatch.
            for (index, pixel) in indices
                .iter()
                .zip(out_row.as_chunks_mut::<4>().0.iter_mut())
            {
                *pixel = self.color(*index).to_ne_bytes();
            }
        }
    }
}

/// Expands a twelve-bit CD+G color to opaque `0xAARRGGBB`.
fn argb(red: u8, green: u8, blue: u8) -> u32 {
    // Four bits to eight by multiplying by 17, not by shifting left four: 15 has to become 255, and
    // `15 << 4` is 240. A palette whose white is 94% white is the kind of wrong that looks like a
    // washed-out screen and gets blamed on the display.
    let expand = |v: u8| u32::from(v & 0x0F) * 17;
    0xFF00_0000 | (expand(red) << 16) | (expand(green) << 8) | expand(blue)
}

/// A parsed CD+G stream: every packet of a file, in order.
#[derive(Debug, Clone)]
pub struct GraphicsStream {
    packets: Vec<[u8; PACKET_BYTES]>,
    trailing_bytes: usize,
}

impl GraphicsStream {
    /// Parses a `.cdg` file's bytes.
    ///
    /// A trailing partial packet is dropped and counted rather than rejected: of 2,849 files
    /// measured, 2,848 were an exact multiple of 24 bytes and one was not, and refusing that one
    /// would be refusing a song over four spare bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        // `as_chunks` yields `&[u8; PACKET_BYTES]` directly, so this is a copy of the array rather
        // than a zeroed temporary plus a `copy_from_slice` into it, and the remainder comes back
        // from the same call instead of from the iterator afterwards.
        let (chunks, trailing) = bytes.as_chunks::<PACKET_BYTES>();
        Self {
            trailing_bytes: trailing.len(),
            packets: chunks.to_vec(),
        }
    }

    /// How many whole packets the file held.
    #[must_use]
    pub fn packets(&self) -> usize {
        self.packets.len()
    }

    /// Bytes at the end of the file that did not make up a whole packet.
    #[must_use]
    pub fn trailing_bytes(&self) -> usize {
        self.trailing_bytes
    }

    /// How long the graphics run, in milliseconds.
    ///
    /// **This is not the song's length**, and it is not even reliably a bound on it. Measured across
    /// 2,847 real pairs it agrees with the audio to within a tenth of a second on average, runs
    /// short on many because the words stop before the outro, and runs **past** the audio on 34 of
    /// them by up to 142 seconds — a tail of filler packets after the last tile. So an MP3+G song
    /// takes its duration from the MP3, always. See the `An MP3+G song's length is its audio's, and
    /// is counted rather than read` decision in `docs/decisions/`.
    #[must_use]
    pub fn duration_ms(&self) -> u32 {
        let ms = self.packets.len() as u64 * 1000 / u64::from(PACKETS_PER_SECOND);
        u32::try_from(ms).unwrap_or(u32::MAX)
    }

    /// Replays the whole stream and reports what was in it.
    ///
    /// Cheap enough to do at packaging time on every file — a six-minute song is a hundred thousand
    /// packets of trivial work — and it is the only way to tell a disc with a few bad packets from
    /// one that never draws a thing.
    #[must_use]
    pub fn stats(&self) -> GraphicsStats {
        let mut screen = Screen::new();
        let mut stats = GraphicsStats {
            packets: u32::try_from(self.packets.len()).unwrap_or(u32::MAX),
            unknown_instructions: 0,
            offscreen_tiles: 0,
            tiles_written: 0,
            duration_ms: self.duration_ms(),
            trailing_bytes: u32::try_from(self.trailing_bytes).unwrap_or(u32::MAX),
        };
        for packet in &self.packets {
            let is_tile = packet[0] & MASK == CMD_CDG
                && matches!(packet[1] & MASK, TILE_BLOCK | TILE_BLOCK_XOR);
            match screen.apply(packet) {
                Applied::Drew if is_tile => stats.tiles_written += 1,
                Applied::Drew | Applied::Ignored => {}
                Applied::UnknownInstruction => stats.unknown_instructions += 1,
                Applied::OffScreenTile => stats.offscreen_tiles += 1,
            }
        }
        stats
    }
}

/// What a whole replay of a stream found in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphicsStats {
    /// Whole packets in the file.
    pub packets: u32,
    /// CD+G packets carrying an instruction this renderer does not implement.
    ///
    /// **Diagnostic, and deliberately not a quality signal.** Measured over 2,849 real files: 223
    /// have some, the worst has 29% of its packets, and every one of them renders perfectly.
    /// Instruction 9 is almost all of it. A packaging check that refused a file over this number
    /// would refuse songs that work.
    pub unknown_instructions: u32,
    /// Tiles addressed off the 300x216 screen — real rip damage, unlike the count above.
    ///
    /// Also not worth refusing a song over: 129 files of 2,849 carry between one and fourteen, and
    /// a handful of dropped tiles among a hundred thousand packets is a blemish nobody sees.
    pub offscreen_tiles: u32,
    /// Tiles actually drawn. **Zero means the file has no words in it**, whatever else it contains,
    /// and this rather than the unusable count is what identifies a file not worth packaging.
    pub tiles_written: u32,
    /// How long the graphics run. See [`GraphicsStream::duration_ms`] — not the song's length.
    pub duration_ms: u32,
    /// Bytes at the end that did not make up a whole packet.
    pub trailing_bytes: u32,
}

/// One picture, ready to upload.
///
/// `ARGB8888`, and always [`crate::VISIBLE_WIDTH`] by [`crate::VISIBLE_HEIGHT`] — a CD+G screen has
/// one size and cannot change it mid-song, which is why nothing here carries a resize path.
#[derive(Debug, Clone)]
pub struct Frame {
    pixels: Vec<u8>,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    /// An all-black picture.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: vec![0; VW * VH * 4],
        }
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        VISIBLE_WIDTH
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        VISIBLE_HEIGHT
    }

    /// The pixels and the row pitch in bytes.
    #[must_use]
    pub fn pixels(&self) -> (&[u8], usize) {
        (&self.pixels, VW * 4)
    }
}

/// Hands out pictures for a position, advancing the screen to reach it.
///
/// The consumer contract is deliberately identical to `km-video`'s frame reader — `take_frame_for`
/// then `recycle` — so the display has two branches of the same shape and the only thing a reader
/// has to notice about them is the pixel format. What is behind it is not the same at all: there is
/// no decoder thread and no queue here, because the work is done by whoever asks.
#[derive(Debug)]
pub struct FrameReader {
    // A mutex for the same reason `km-video`'s reader has one: this hangs off the machine, which is
    // shared across the control, API and display threads, so it must be `Sync`. Only the display
    // thread ever takes it, once per drawn frame, and nothing slow is held across it.
    inner: Mutex<Pull>,
}

#[derive(Debug)]
struct Pull {
    stream: GraphicsStream,
    screen: Screen,
    /// Packets applied so far.
    next: usize,
    /// Whether anything drew since the last picture was handed out.
    dirty: bool,
    spare: Option<Frame>,
}

impl FrameReader {
    /// Starts at the beginning of a stream.
    #[must_use]
    pub fn new(stream: GraphicsStream) -> Self {
        Self {
            inner: Mutex::new(Pull {
                stream,
                screen: Screen::new(),
                next: 0,
                dirty: true,
                spare: None,
            }),
        }
    }

    /// The picture as of `position_ms`, or `None` if nothing has changed since the last one.
    ///
    /// `None` means *keep the texture you have*, exactly as it does for a video, and it is the
    /// ordinary answer: the display draws far faster than a disc redraws.
    ///
    /// Advancing happens here, which is what makes a seek need no protocol at all. The position
    /// simply moves and the next call sees it — forwards by playing packets through, backwards by
    /// rebuilding from zero once the jump exceeds [`REWIND_SLACK_MS`].
    pub fn take_frame_for(&self, position_ms: u32) -> Option<Frame> {
        let Ok(mut pull) = self.inner.lock() else {
            // A poisoned lock means a panic while a picture was being made. Losing the picture is
            // survivable and taking the process down with it is not.
            return None;
        };

        let want = usize::try_from(u64::from(position_ms) * u64::from(PACKETS_PER_SECOND) / 1000)
            .unwrap_or(usize::MAX)
            .min(pull.stream.packets.len());

        if pull.applied_ms() > position_ms.saturating_add(REWIND_SLACK_MS) {
            pull.screen.reset();
            pull.next = 0;
            pull.dirty = true;
        }

        while pull.next < want {
            let packet = pull.stream.packets[pull.next];
            if pull.screen.apply(&packet) == Applied::Drew {
                pull.dirty = true;
            }
            pull.next += 1;
        }

        if !pull.dirty {
            return None;
        }
        let mut frame = pull.spare.take().unwrap_or_default();
        pull.screen.blit_visible(&mut frame.pixels);
        pull.dirty = false;
        Some(frame)
    }

    /// Takes a picture back, so the next one reuses its buffer.
    pub fn recycle(&self, frame: Frame) {
        if let Ok(mut pull) = self.inner.lock() {
            pull.spare = Some(frame);
        }
    }
}

impl Pull {
    fn applied_ms(&self) -> u32 {
        let ms = self.next as u64 * 1000 / u64::from(PACKETS_PER_SECOND);
        u32::try_from(ms).unwrap_or(u32::MAX)
    }
}
