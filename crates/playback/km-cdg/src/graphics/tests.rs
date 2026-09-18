//! Tests for the renderer, built from hand-written packets.
//!
//! Nothing here reads a real `.cdg`. The corpus this was measured against is machine-local, so the
//! fixtures are synthetic — the same rule the MIDI corpus has followed since M1, and the better test
//! either way. What the real files contributed is the *shapes* asserted here: the
//! P and Q bits, the off-screen tile, the trailing partial packet and the stream of rubbish are all
//! things a real file did.

use super::*;

/// A CD+G instruction packet. `data` is padded out to the sixteen data bytes.
fn packet(instruction: u8, data: &[u8]) -> [u8; PACKET_BYTES] {
    let mut out = [0u8; PACKET_BYTES];
    out[0] = CMD_CDG;
    out[1] = instruction;
    out[4..4 + data.len()].copy_from_slice(data);
    out
}

fn filler() -> [u8; PACKET_BYTES] {
    [0u8; PACKET_BYTES]
}

/// A tile packet covering one whole tile in `color1`.
fn solid_tile(instruction: u8, row: u8, column: u8, color0: u8, color1: u8) -> [u8; PACKET_BYTES] {
    let mut data = vec![color0, color1, row, column];
    data.extend(std::iter::repeat_n(0x3F, TILE_HEIGHT));
    packet(instruction, &data)
}

/// The palette that makes index 0 black and index 1 white.
fn black_and_white() -> [u8; PACKET_BYTES] {
    // Twelve bits over two six-bit bytes: `xxrrrrgg`, then `xxggbbbb`. All ones is 0x3F, 0x3F.
    let mut data = vec![0x00, 0x00, 0x3F, 0x3F];
    data.extend(std::iter::repeat_n(0u8, 12));
    packet(LOAD_CLUT_LO, &data)
}

fn stream_of(packets: Vec<[u8; PACKET_BYTES]>) -> GraphicsStream {
    let bytes: Vec<u8> = packets.iter().flatten().copied().collect();
    GraphicsStream::from_bytes(&bytes)
}

#[test]
fn a_palette_entry_expands_to_the_full_range() {
    let mut screen = Screen::new();
    screen.apply(&black_and_white());
    assert_eq!(screen.color(0), 0xFF00_0000);
    // 15 has to become 255, not 240: `v * 17`, never `v << 4`.
    assert_eq!(screen.color(1), 0xFFFF_FFFF);
}

#[test]
fn a_tile_block_draws_exactly_its_own_six_by_twelve() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[0]));
    // Row 1, column 1 is the block at (6, 12) — the top left of the visible area.
    screen.apply(&solid_tile(TILE_BLOCK, 1, 1, 0, 1));

    for y in 12..24 {
        for x in 6..12 {
            assert_eq!(screen.index_at(x, y), 1, "inside the tile at ({x}, {y})");
        }
    }
    assert_eq!(screen.index_at(5, 12), 0, "one pixel left of the tile");
    assert_eq!(screen.index_at(12, 12), 0, "one pixel right of the tile");
    assert_eq!(screen.index_at(6, 11), 0, "one pixel above the tile");
    assert_eq!(screen.index_at(6, 24), 0, "one pixel below the tile");
}

#[test]
fn a_tile_bitmap_puts_the_leftmost_pixel_in_the_high_bit() {
    let mut screen = Screen::new();
    // Only bit 5 set: the leftmost of the six pixels, on the first row of the tile.
    let mut data = vec![0, 1, 0, 0, 0x20];
    data.extend(std::iter::repeat_n(0u8, 11));
    screen.apply(&packet(TILE_BLOCK, &data));
    assert_eq!(screen.index_at(0, 0), 1, "leftmost pixel");
    for x in 1..6 {
        assert_eq!(screen.index_at(x, 0), 0, "pixel {x} of the first row");
    }
}

#[test]
fn an_xor_tile_applied_twice_returns_the_screen_to_where_it_was() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[5]));
    let before: Vec<u8> = (0..WIDTH).map(|x| screen.index_at(x, 12)).collect();

    let tile = solid_tile(TILE_BLOCK_XOR, 1, 1, 0, 3);
    screen.apply(&tile);
    assert_ne!(
        screen.index_at(6, 12),
        5,
        "the first xor must change something"
    );
    screen.apply(&tile);

    let after: Vec<u8> = (0..WIDTH).map(|x| screen.index_at(x, 12)).collect();
    assert_eq!(before, after);
}

#[test]
fn memory_preset_fills_the_whole_screen_and_border_preset_fills_only_the_border() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[7]));
    assert_eq!(
        screen.index_at(0, 0),
        7,
        "the border is inside a memory preset"
    );
    assert_eq!(screen.index_at(150, 100), 7);

    screen.apply(&packet(BORDER_PRESET, &[2]));
    assert_eq!(screen.border(), 2);
    assert_eq!(screen.index_at(0, 0), 2, "top left corner");
    assert_eq!(screen.index_at(5, 100), 2, "last border column on the left");
    assert_eq!(
        screen.index_at(WIDTH - 1, HEIGHT - 1),
        2,
        "bottom right corner"
    );
    assert_eq!(
        screen.index_at(6, 12),
        7,
        "the first visible pixel is untouched"
    );
    assert_eq!(screen.index_at(150, 100), 7, "the middle is untouched");
}

#[test]
fn the_p_and_q_subchannel_bits_are_ignored() {
    // Real rips keep them set in the top two bits of every byte. Masking is the format rather than a
    // defensive measure, so the same stream with and without them must draw the same picture.
    let clean = vec![
        packet(MEMORY_PRESET, &[9]),
        black_and_white(),
        solid_tile(TILE_BLOCK, 3, 4, 0, 1),
        packet(BORDER_PRESET, &[6]),
        solid_tile(TILE_BLOCK_XOR, 3, 4, 2, 5),
    ];

    let mut plain = Screen::new();
    let mut dirtied = Screen::new();
    for clean_packet in &clean {
        plain.apply(clean_packet);
        let mut with_pq = *clean_packet;
        for byte in &mut with_pq {
            *byte |= 0xC0;
        }
        dirtied.apply(&with_pq);
    }

    for y in (0..HEIGHT).step_by(7) {
        for x in (0..WIDTH).step_by(5) {
            assert_eq!(
                plain.index_at(x, y),
                dirtied.index_at(x, y),
                "pixel ({x}, {y}) differs once the P and Q bits are set"
            );
        }
    }
    assert_eq!(plain.color(1), dirtied.color(1));
}

#[test]
fn scroll_preset_shifts_the_screen_and_fills_what_it_vacates() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[3]));
    // Horizontal command 2 is "scroll left by one tile"; the color fills the vacated strip.
    screen.apply(&packet(SCROLL_PRESET, &[8, 0x20, 0x00]));

    assert_eq!(screen.index_at(0, 0), 3, "content that stayed on screen");
    for x in WIDTH - 6..WIDTH {
        assert_eq!(screen.index_at(x, 50), 8, "vacated column {x}");
    }
}

#[test]
fn scroll_copy_wraps_what_falls_off_the_edge() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[3]));
    // A marker in the leftmost tile, which a leftward copy-scroll must bring round to the right.
    screen.apply(&solid_tile(TILE_BLOCK, 0, 0, 0, 7));
    screen.apply(&packet(SCROLL_COPY, &[8, 0x20, 0x00]));

    for x in WIDTH - 6..WIDTH {
        assert_eq!(screen.index_at(x, 5), 7, "wrapped column {x}");
    }
}

/// The other three directions a scroll can go.
///
/// The two tests above between them cover exactly one of the four: a leftward `SCROLL_PRESET` and a
/// leftward `SCROLL_COPY`. `dy` had no coverage at all, which mattered because the two axes are
/// separate `match` arms over separate nibbles and either could have been transcribed with its 1 and
/// 2 the wrong way round — a mistake that shows up as a screen sliding the wrong way on the one
/// disc in a hundred that scrolls, and never on a rebuild.
#[test]
fn a_scroll_goes_the_way_the_command_says_on_both_axes() {
    // Command 1 is "right"/"down", command 2 is "left"/"up", on the horizontal and vertical bytes
    // respectively. Each vacates the strip it moved away from.
    for (byte, vacated) in [(0x10u8, 0..6usize), (0x20, W - 6..W)] {
        let mut screen = Screen::new();
        screen.apply(&packet(MEMORY_PRESET, &[3]));
        screen.apply(&packet(SCROLL_PRESET, &[9, byte, 0]));
        for x in vacated {
            assert_eq!(
                screen.index_at(x as u32, 100),
                9,
                "horizontal {byte:#04x} must vacate column {x}"
            );
        }
    }

    // Down: the top twelve rows are what the screen moved away from.
    let mut down = Screen::new();
    down.apply(&packet(MEMORY_PRESET, &[3]));
    down.apply(&packet(SCROLL_PRESET, &[9, 0, 0x10]));
    for y in 0..12 {
        assert_eq!(
            down.index_at(100, y),
            9,
            "a downward scroll vacates row {y}"
        );
    }
    assert_eq!(
        down.index_at(100, 12),
        3,
        "and moves the content down by one tile"
    );

    // Up: the bottom twelve.
    let mut up = Screen::new();
    up.apply(&packet(MEMORY_PRESET, &[3]));
    up.apply(&packet(SCROLL_PRESET, &[9, 0, 0x20]));
    for y in H as u32 - 12..H as u32 {
        assert_eq!(up.index_at(100, y), 9, "an upward scroll vacates row {y}");
    }
}

/// A copy-scroll wraps on the vertical axis too, and wraps by exactly one tile.
#[test]
fn a_vertical_copy_scroll_brings_the_bottom_round_to_the_top() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[3]));
    // The last tile row, which a downward copy-scroll must bring round to the top.
    screen.apply(&solid_tile(TILE_BLOCK, 17, 1, 0, 7));
    screen.apply(&packet(SCROLL_COPY, &[8, 0, 0x10]));

    for y in 0..12 {
        assert_eq!(screen.index_at(8, y), 7, "wrapped row {y}");
    }
    // Nothing was filled with the color a preset would have used: a copy has nothing to vacate.
    for y in 0..H as u32 {
        assert_ne!(screen.index_at(150, y), 8, "row {y} was filled, not copied");
    }
}

/// One packet may shift the screen *and* move the window, and the two are read from the same byte.
///
/// The coarse command is bits 4-5 and the fine offset is the low bits, so a packet asking for both
/// is the case where a mask written one bit too wide would take the other's value. Neither existing
/// scroll test sets both.
#[test]
fn one_packet_can_shift_the_screen_and_move_the_window() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[3]));
    // Horizontal: coarse 1 (right) with a fine offset of 3. Vertical: coarse 2 (up) with 1.
    screen.apply(&packet(SCROLL_PRESET, &[9, 0x13, 0x21]));

    assert_eq!(screen.offsets(), (3, 1), "the fine offset");
    for x in 0..6u32 {
        assert_eq!(
            screen.index_at(x, 100),
            9,
            "and the screen still moved right"
        );
    }
}

/// A scroll offset past the border is clamped, and the clamp is what keeps `blit_visible` in bounds.
///
/// The visible window is 288 wide inside a 300-wide screen, so the window's left edge may reach 11
/// and no further. A disc asking for 7 — the widest the three-bit field can say — would put the
/// right edge at 301 and index past the end of `indices`. The existing offset test asks for exactly
/// the largest legal value, so it would pass against no clamp at all.
#[test]
fn a_scroll_offset_past_the_border_is_clamped_rather_than_read_off_the_end() {
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[1]));
    screen.apply(&black_and_white());
    screen.apply(&packet(SCROLL_PRESET, &[0, 0x07, 0x0F]));

    assert_eq!(screen.offsets(), (5, 11), "7 and 15 are past the border");

    // The point of the clamp: this must not panic, and must fill the whole frame.
    let mut frame = Frame::new();
    screen.blit_visible(&mut frame.pixels);
    assert!(
        frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| *p == 0xFFFF_FFFFu32.to_ne_bytes()),
        "every pixel of the window came from inside the screen"
    );
}

/// The high half of the palette is written to the high half, and leaves the low half alone.
///
/// `LOAD_CLUT_LO` and `LOAD_CLUT_HI` are one function and a base of 0 or 8. A base of 0 for both —
/// the transcription mistake available here — makes a sixteen-color disc render in eight, which
/// looks like a dull disc rather than like a bug.
#[test]
fn the_high_palette_load_writes_the_high_eight_and_only_those() {
    let mut screen = Screen::new();
    screen.apply(&black_and_white());
    assert_eq!(screen.color(1), 0xFFFF_FFFF);

    // The same all-ones entry, in the second slot of the high half: index 9.
    let mut data = vec![0x00, 0x00, 0x3F, 0x3F];
    data.extend(std::iter::repeat_n(0u8, 12));
    screen.apply(&packet(LOAD_CLUT_HI, &data));

    assert_eq!(screen.color(9), 0xFFFF_FFFF, "written to the high half");
    assert_eq!(
        screen.color(8),
        0xFF00_0000,
        "and the rest of it stayed black"
    );
    assert_eq!(
        screen.color(1),
        0xFFFF_FFFF,
        "the low half was not overwritten"
    );
}

/// A reset is the whole state, not the pixels alone.
///
/// `FrameReader` calls this to replay a stream from its start after a seek, so anything it forgets
/// is state from *before* the seek surviving into the rebuilt picture — a palette, a border color or
/// a window offset that the replayed packets may never set again because the disc set them once.
#[test]
fn a_reset_forgets_the_palette_and_the_window_as_well_as_the_pixels() {
    let mut screen = Screen::new();
    screen.apply(&black_and_white());
    screen.apply(&packet(MEMORY_PRESET, &[1]));
    screen.apply(&packet(BORDER_PRESET, &[2]));
    screen.apply(&packet(DEFINE_TRANSPARENT, &[1]));
    screen.apply(&packet(SCROLL_PRESET, &[0, 0x05, 0x0B]));

    screen.reset();

    assert_eq!(screen.index_at(150, 100), 0, "the pixels");
    assert_eq!(screen.color(1), 0xFF00_0000, "the palette");
    assert_eq!(screen.border(), 0, "the border");
    assert_eq!(screen.offsets(), (0, 0), "the window");
    assert_eq!(screen.transparent(), None, "and the transparency index");
}

#[test]
fn a_scroll_offset_moves_the_visible_window() {
    // The offset moves the window down and to the right, revealing more of the bottom-right border
    // and hiding more of the top-left one. So the tile that proves it is the *last* one on screen.
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[0]));
    screen.apply(&black_and_white());
    // Row 17, column 49 is the block at (294, 204) — entirely inside the bottom-right border.
    screen.apply(&solid_tile(TILE_BLOCK, 17, 49, 0, 1));

    // With no offset the window is (6, 12) to (294, 204), so that tile falls outside it. With the
    // largest offset a scroll may ask for it is (11, 23) to (299, 215), and the tile's own top-left
    // pixel lands at this spot in the output.
    let corner = (181 * VW + 283) * 4;

    let mut frame = Frame::new();
    screen.blit_visible(&mut frame.pixels);
    assert_eq!(
        &frame.pixels[corner..corner + 4],
        &0xFF00_0000u32.to_ne_bytes(),
        "hidden in the border"
    );

    screen.apply(&packet(SCROLL_PRESET, &[0, 0x05, 0x0B]));
    assert_eq!(screen.offsets(), (5, 11));
    screen.blit_visible(&mut frame.pixels);
    let (pixels, pitch) = frame.pixels();
    assert_eq!(pitch, VW * 4);
    assert_eq!(
        &pixels[corner..corner + 4],
        &0xFFFF_FFFFu32.to_ne_bytes(),
        "now inside the window"
    );
}

#[test]
fn transparency_is_recorded_and_not_acted_on() {
    let mut screen = Screen::new();
    screen.apply(&black_and_white());
    screen.apply(&packet(MEMORY_PRESET, &[1]));
    screen.apply(&packet(DEFINE_TRANSPARENT, &[1]));
    assert_eq!(screen.transparent(), Some(1));

    let mut frame = Frame::new();
    screen.blit_visible(&mut frame.pixels);
    assert_eq!(
        &frame.pixels[..4],
        &0xFFFF_FFFFu32.to_ne_bytes(),
        "the picture must stay opaque and keep the disc's own color"
    );
}

#[test]
fn an_unknown_instruction_is_counted_and_skipped() {
    let mut screen = Screen::new();
    assert_eq!(
        screen.apply(&packet(17, &[1, 2, 3])),
        Applied::UnknownInstruction
    );
    assert_eq!(screen.apply(&filler()), Applied::Ignored);
}

#[test]
fn a_subcode_pack_that_is_not_cdg_is_not_damage() {
    // Learned from a real file, and worth a test of its own because getting it wrong is expensive:
    // one disc in the sample corpus is 74% packs with a command byte of 5, and it renders three
    // legible lines of karaoke. A rip that kept the whole R-W subchannel carries other
    // applications' packs, and counting those as corruption would have a packaging check refuse a
    // song that plays perfectly.
    let mut screen = Screen::new();
    let mut other_application = [0x05u8; PACKET_BYTES];
    other_application[1] = 0x05;
    assert_eq!(screen.apply(&other_application), Applied::Ignored);

    let mut packets = vec![other_application; 400];
    packets.push(black_and_white());
    packets.extend((0..3).map(|i| solid_tile(TILE_BLOCK, 1, i, 0, 1)));
    let stats = stream_of(packets).stats();
    assert_eq!(stats.unknown_instructions, 0, "not one of those is damage");
    assert_eq!(stats.tiles_written, 3);
}

#[test]
fn a_tile_addressed_off_the_screen_is_counted_rather_than_panicking() {
    let mut screen = Screen::new();
    // Column 49 is the last that fits; 50 and up run off the right edge.
    assert_eq!(
        screen.apply(&solid_tile(TILE_BLOCK, 0, 49, 0, 1)),
        Applied::Drew
    );
    assert_eq!(
        screen.apply(&solid_tile(TILE_BLOCK, 0, 50, 0, 1)),
        Applied::OffScreenTile
    );
    assert_eq!(
        screen.apply(&solid_tile(TILE_BLOCK, 18, 0, 0, 1)),
        Applied::OffScreenTile
    );
}

#[test]
fn a_trailing_partial_packet_is_dropped_and_counted() {
    let mut bytes: Vec<u8> = filler().to_vec();
    bytes.extend_from_slice(&[0u8; 7]);
    let stream = GraphicsStream::from_bytes(&bytes);
    assert_eq!(stream.packets(), 1);
    assert_eq!(stream.trailing_bytes(), 7);
}

#[test]
fn a_streams_length_is_its_packet_count_over_three_hundred() {
    let stream = stream_of(vec![filler(); 450]);
    assert_eq!(stream.duration_ms(), 1500);
}

#[test]
fn a_stream_of_rubbish_draws_nothing_and_does_not_panic() {
    // Deterministic nonsense rather than a random crate: the point is that no byte pattern reaches a
    // panic, and a fixed sequence makes any failure reproducible.
    let mut state = 0x1234_5678u32;
    let bytes: Vec<u8> = (0..PACKET_BYTES * 500)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 16) as u8
        })
        .collect();

    let stats = GraphicsStream::from_bytes(&bytes).stats();
    assert_eq!(stats.packets, 500);
    // `tiles_written` is what separates a disc from a file that merely has bytes in it — only about
    // one packet in 64 even claims to be CD+G, so almost nothing here reaches a tile.
    assert!(
        stats.tiles_written < 5,
        "rubbish must not be mistaken for words, got {}",
        stats.tiles_written
    );
}

#[test]
fn stats_counts_the_tiles_a_real_looking_stream_draws() {
    let mut packets = vec![filler(); 10];
    packets.push(black_and_white());
    packets.extend((0..5).map(|i| solid_tile(TILE_BLOCK, 1, i, 0, 1)));
    packets.push(packet(17, &[]));
    let stats = stream_of(packets).stats();

    assert_eq!(stats.packets, 17);
    assert_eq!(stats.tiles_written, 5);
    assert_eq!(stats.unknown_instructions, 1);
    assert_eq!(stats.trailing_bytes, 0);
}

/// A stream that fills the screen with `first` at packet 0 and with `then` at packet 299.
fn two_fills(first: u8, then: u8) -> GraphicsStream {
    let mut packets = vec![filler(); 600];
    packets[0] = packet(MEMORY_PRESET, &[first]);
    packets[299] = packet(MEMORY_PRESET, &[then]);
    stream_of(packets)
}

#[test]
fn a_reader_hands_out_a_picture_only_when_something_drew() {
    let reader = FrameReader::new(two_fills(1, 2));

    // The first call always answers, with a blank screen: at position zero no packet has played
    // yet, and handing the display something black beats handing it nothing.
    let frame = reader
        .take_frame_for(0)
        .expect("the first call always draws");
    reader.recycle(frame);
    assert!(
        reader.take_frame_for(500).is_some(),
        "the fill at packet 0 has played by now"
    );
    assert!(
        reader.take_frame_for(600).is_none(),
        "only filler between the two fills, so the texture should be kept"
    );
    assert!(
        reader.take_frame_for(1000).is_some(),
        "packet 299 lands at one second"
    );
}

#[test]
fn a_reader_rebuilds_from_the_start_when_the_position_jumps_back() {
    let reader = FrameReader::new(two_fills(1, 2));
    let mut screen = Screen::new();
    screen.apply(&packet(MEMORY_PRESET, &[1]));
    let mut expected = Frame::new();
    screen.blit_visible(&mut expected.pixels);

    reader
        .take_frame_for(2000)
        .expect("advances to the second fill");
    let rewound = reader
        .take_frame_for(500)
        .expect("a real seek backwards must redraw");
    assert_eq!(
        rewound.pixels, expected.pixels,
        "after a seek to 500 ms only the first fill has played"
    );
}

#[test]
fn a_reader_ignores_a_position_that_wobbles_backwards() {
    // The display smooths `position_ms` between audio callbacks and can step back a little. That
    // must not rebuild the screen, or every drawn frame would carry a rebuild.
    let reader = FrameReader::new(two_fills(1, 2));
    reader.take_frame_for(1000).expect("first picture");
    assert!(
        reader.take_frame_for(1000 - REWIND_SLACK_MS / 2).is_none(),
        "a wobble inside the slack must not redraw"
    );
}

#[test]
fn a_reader_stops_at_the_end_of_the_stream() {
    let reader = FrameReader::new(two_fills(1, 2));
    // Far beyond the two seconds the stream holds. The last picture stands.
    assert!(reader.take_frame_for(600_000).is_some());
    assert!(reader.take_frame_for(900_000).is_none());
}

#[test]
fn a_recycled_frame_is_reused() {
    let reader = FrameReader::new(two_fills(1, 2));
    let frame = reader.take_frame_for(0).expect("first picture");
    let address = frame.pixels.as_ptr();
    reader.recycle(frame);
    let again = reader.take_frame_for(1000).expect("second picture");
    assert_eq!(
        again.pixels.as_ptr(),
        address,
        "the buffer should have been reused"
    );
}
