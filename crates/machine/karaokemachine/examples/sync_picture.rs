//! Takes the published picture of the lyric sync editor, and stages the folder the song sync page is
//! pictured on.
//!
//! ```text
//! cargo run -p karaokemachine --example sync_picture -- picture <pack.kmpkg> <song> <line> <out.png>
//! cargo run -p karaokemachine --example sync_picture -- stage <pack.kmpkg> <folder> <words.txt> <song>...
//! ```
//!
//! `<song>` is a song's number inside the package, and `<line>` is the line being tapped, counted
//! from zero.
//!
//! `picture` draws the tapping screen part-way through that line. `stage` writes each named song
//! into `<folder>` as a MIDI file without its words, and the first song's words into `<words.txt>`
//! as the text the editor takes. `tools/dev/sync-pictures.sh` runs both.
//!
//! **Both read a released package, and never a corpus.** The pictures publish whole lines of words,
//! so the songs have to be ones whose words anybody may publish. The `The animated picture is of a
//! public-domain carol` decision in docs/decisions/repository.md holds the argument.

use std::path::{Path, PathBuf};

use km_app::sync::{Picture, picture, words_as_typed};
use km_kmpkg::Package;
use km_song::kar_write::without_words;
use km_song::{ParseOptions, Song};

/// The picture's size. The editor's window is 1280 by 720, and this is that window at the density
/// the television pictures are published at.
const SIZE: (u32, u32) = (1920, 1080);

const USAGE: &str = "usage: sync_picture picture <pack.kmpkg> <song> <line> <out.png>\n       \
                     sync_picture stage <pack.kmpkg> <folder> <words.txt> <song>...";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["picture", pack, song, line, out] => {
            let package = Package::open(pack)?;
            let (title, bytes) = english_song(&package, number(song)?)?;
            picture(
                &Picture {
                    song: &bytes,
                    title: &title,
                    line: line.parse()?,
                    size: SIZE,
                },
                Path::new(out),
            )?;
            println!("wrote {out}");
        }
        ["stage", pack, folder, words, songs @ ..] if !songs.is_empty() => {
            let package = Package::open(pack)?;
            let folder = PathBuf::from(folder);
            std::fs::create_dir_all(&folder)?;
            for (index, song) in songs.iter().enumerate() {
                let (title, bytes) = english_song(&package, number(song)?)?;
                // A title is a file name once the characters a file system refuses are out of it.
                let name: String = title
                    .chars()
                    .filter(|c| c.is_alphanumeric() || " ,'-".contains(*c))
                    .collect();
                std::fs::write(folder.join(format!("{name}.mid")), without_words(&bytes)?)?;
                if index == 0 {
                    let song = Song::parse(&bytes, &ParseOptions::default())?;
                    std::fs::write(words, words_as_typed(&song))?;
                }
                println!("staged {name}.mid");
            }
        }
        _ => anyhow::bail!(USAGE),
    }
    Ok(())
}

fn number(text: &str) -> anyhow::Result<u32> {
    text.parse()
        .map_err(|_| anyhow::anyhow!("{text}: not a song number"))
}

/// One song of the package: its title and its bytes.
///
/// A published picture is in English, so a song filed as anything else is refused.
fn english_song(package: &Package, number: u32) -> anyhow::Result<(String, Vec<u8>)> {
    let entry = package
        .manifest()
        .song(number)
        .ok_or_else(|| anyhow::anyhow!("the package holds no song {number}"))?;
    anyhow::ensure!(
        entry.language.as_deref() == Some("en"),
        "{}: not filed as English",
        entry.title
    );
    Ok((entry.title.clone(), package.read_song(number)?))
}
