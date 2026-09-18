//! Renders a karaoke file to a WAV, with no audio device involved.
//!
//! ```text
//! cargo run -p km-audio --example render_wav -- song.kar out.wav [soundfont.sf2] //!     [--transpose -2] [--tempo 1.1] [--melody on] [--volume 0.8] [--max-ms 90000]
//! ```
//!
//! With a SoundFont it produces real audio, which is the quickest way to hear whether a file plays
//! correctly. Without one it uses the built-in sine synthesizer, which sounds nothing like music but
//! proves the sequencing and render path work — useful where no `.sf2` is available.
//!
//! The melody channel is detected here for convenience. In the real application it is read from the
//! package, having been decided once at packaging time.

use std::sync::Arc;

use km_audio::{
    Bank, RenderOptions, SoundFontSource, TestToneSource, render, render_keeping_source, write_wav,
};
use km_song::{ParseOptions, Song};
use km_suitability::Analysis;

const USAGE: &str = "usage: render_wav <song.kar> <out.wav> [soundfont.sf2] [options]

options:
  --transpose <-6..6>    shift the key, in semitones
  --tempo <0.75..1.25>   playback speed, as a multiple of the written tempo
  --melody <on|off>      guide melody audible (default off)
  --volume <0..1>        music volume
  --max-ms <ms>          stop after this much, for comparing banks over equal lengths
  --only-channel <0-15>  silence every other channel, to hear one part on its own
  --dump-channels        report the tuning each channel ended on (needs a SoundFont)
  --fixes                apply every correction detection finds, suggested ones included
";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut positional: Vec<String> = Vec::new();
    let mut options = RenderOptions::default();
    let mut args = std::env::args().skip(1);
    let mut only_channel: Option<u8> = None;
    let mut dump_channels = false;
    // Off by default, so a render says what the file says. The A/B this exists for is one flag
    // apart: the same song, the same bank, with the corrections and without.
    let mut apply_fixes = false;

    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--transpose" => options.settings.transpose = value()?.parse()?,
            "--tempo" => options.settings.tempo_ratio = value()?.parse()?,
            "--melody" => {
                options.settings.melody_enabled = matches!(value()?.as_str(), "on" | "true" | "1")
            }
            "--volume" => options.volume = value()?.parse()?,
            "--max-ms" => options.max_ms = value()?.parse()?,
            "--only-channel" => only_channel = Some(value()?.parse()?),
            "--dump-channels" => dump_channels = true,
            "--fixes" => apply_fixes = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}").into());
            }
            other => positional.push(other.to_owned()),
        }
    }
    // Clamped here so the printed summary reports what will actually be used.
    options.settings = options.settings.clamped();

    let input = positional.first().ok_or(USAGE)?.clone();
    let output = positional.get(1).ok_or(USAGE)?.clone();
    let soundfont = positional.get(2).cloned();

    let bytes = std::fs::read(&input)?;
    let mut parsed = Song::parse(&bytes, &ParseOptions::default())?;
    if let Some(wanted) = only_channel {
        // Note-ons only. Note-offs, controllers and bends stay, so the channel that is left keeps
        // the state the file gives it and nothing on a silenced channel is left sounding.
        parsed.events.retain(|event| {
            !matches!(event.kind, km_song::EventKind::NoteOn { channel, .. } if channel != wanted)
        });
    }
    let song = Arc::new(parsed);

    let analysis = Analysis::of(&song);
    let melody = analysis.melody_channel();
    // Every correction detection finds, and not only the ones that apply themselves: a suggested
    // correction is judged by listening, and this is where the listening happens.
    let detected = km_fixes::detect(&song);
    if apply_fixes {
        options.fixes = km_fixes::resolve(&detected);
    }
    println!("{input}");
    println!("  format      {:?}", song.flavor);
    println!("  duration    {} ms", song.duration_ms());
    println!("  notes       {}", song.note_count());
    match melody {
        Some(channel) => println!(
            "  melody      channel {channel} ({})",
            if options.settings.melody_enabled {
                "audible"
            } else {
                "muted"
            }
        ),
        None => println!("  melody      none detected (the toggle does nothing)"),
    }
    println!("  suitability {}/10", analysis.suitability_value());
    match (detected.is_empty(), apply_fixes) {
        (true, _) => println!("  fixes       none detected"),
        (false, true) => {
            for line in km_fixes::describe(&detected) {
                println!("  fix         {line}");
            }
        }
        (false, false) => println!(
            "  fixes       {} detected, not applied (pass --fixes)",
            detected.len()
        ),
    }
    println!(
        "  settings    transpose {:+}, tempo {:.2}x, volume {:.2}",
        options.settings.transpose, options.settings.tempo_ratio, options.volume
    );

    let sample_rate = 44_100;
    let rendered = match &soundfont {
        Some(path) => {
            println!("  synth       SoundFont {path}");
            // `Bank::load` rather than `SoundFontSource::from_path`, which parses the same file and
            // then has nothing left to ask about it. A bank that loaded with records missing plays
            // -- so without this line the difference between a whole bank and a gutted one is
            // inaudible until the missing instrument's first note, which is the wrong way to find
            // out while comparing banks.
            let bank = Bank::load(path)?;
            if !bank.defects().is_empty() {
                println!("  defects     {}", bank.defects());
            }
            let source = SoundFontSource::from_bank(&bank, sample_rate)?;
            let (rendered, source) =
                render_keeping_source(source, Arc::clone(&song), melody, &options);
            if dump_channels {
                print_channels(&source);
            }
            rendered
        }
        None => {
            println!("  synth       built-in test tones (pass a .sf2 for real audio)");
            let source = TestToneSource::new(sample_rate);
            render(source, Arc::clone(&song), melody, &options)
        }
    };

    write_wav(&output, &rendered)?;
    println!(
        "wrote {output}: {} ms, peak {:.3}{}",
        rendered.duration_ms(),
        rendered.peak(),
        if rendered.reached_end {
            ""
        } else {
            " (stopped at the length limit)"
        }
    );
    if rendered.is_silent() {
        eprintln!("warning: the render is silent");
    }
    Ok(())
}

/// Reports the tuning each channel ended the render holding.
///
/// **Only the channels that asked for something**, because a bend range of 2 and a tune of 0 are
/// what sixteen untouched channels read as and a full table hides the answer. A file that sets a
/// pitch bend range and does not appear here is a file whose setup went nowhere.
fn print_channels(source: &SoundFontSource) {
    println!("  channels    bend range and tune, where a file asked for either");
    let mut said = false;
    for channel in 0..16u8 {
        let range = source.channel_pitch_bend_range(channel);
        let tune = source.channel_tune(channel);
        let keys = (0..128u8)
            .filter(|key| source.channel_key_tune(channel, *key) != 0.0)
            .count();
        if range == 2.0 && tune == 0.0 && keys == 0 {
            continue;
        }
        said = true;
        print!("    ch{channel:<2} bend range {range:>6.2}   tune {tune:+.2}");
        if keys > 0 {
            print!("   {keys} keys retuned");
        }
        println!();
    }
    if !said {
        println!("    none: every channel is at the General MIDI default");
    }
}
