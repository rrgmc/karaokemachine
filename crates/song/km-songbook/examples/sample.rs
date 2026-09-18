//! Writes a synthetic song book, so a person can open one.
//!
//! The same trick `km-cdg/examples/preview.rs` uses, and for the same reason: a column rule half a
//! point out, a baseline off by one row or a truncation that eats a character too many all produce
//! output that still *looks* like output. Tests assert the arithmetic; this is how the typography
//! gets judged.
//!
//! ```sh
//! cargo run -p km-songbook --example sample
//! ```
//!
//! It deliberately includes the awkward cases: a long artist run to show the blanking, a title and
//! a lyric line too wide for their columns, an accented section, a song with no artist, one with no
//! first line, prefixed and unprefixed packages side by side, and a row of Japanese to show what
//! the base-14 bargain actually costs on the page.

use std::path::PathBuf;

use km_songbook::{BookSong, BookStyle, Entry, SortKey};
use km_songcode::SongCode;

/// A folded sort key, standing in for `km_song::text::fold`, which this crate cannot see.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| match ch {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

fn entry(
    section: &str,
    artist: Option<&str>,
    bank: u16,
    number: u16,
    title: &str,
    first_line: Option<&str>,
) -> Entry {
    Entry {
        song: BookSong {
            artist: artist.map(str::to_owned),
            number: SongCode::in_bank(bank, number)
                .expect("a bank and slot the example spells right"),
            title: title.to_owned(),
            first_line: first_line.map(str::to_owned),
        },
        section: section.to_owned(),
        section_sort: section.to_lowercase(),
        sort: SortKey {
            artist_missing: artist.is_none(),
            artist: fold(artist.unwrap_or_default()),
            title: fold(title),
        },
    }
}

fn main() {
    let mut entries = vec![
        entry(
            "Portuguese",
            Some("Legião Urbana"),
            3,
            500,
            "Tempo Perdido",
            Some("Todos os dias quando acordo não tenho mais o tempo que passou"),
        ),
        entry(
            "Portuguese",
            Some("Legião Urbana"),
            3,
            512,
            "Faroeste Caboclo",
            Some("Não tinha medo o tal João de Santo Cristo"),
        ),
        entry(
            "Portuguese",
            Some("Legião Urbana"),
            3,
            530,
            "Eduardo e Mônica",
            Some("Quem um dia irá dizer que existe razão"),
        ),
        entry(
            "Portuguese",
            Some("Ângela Ro Ro"),
            3,
            88,
            "Amor Meu",
            Some("Amor meu"),
        ),
        entry(
            "Portuguese",
            Some("Cazuza"),
            0,
            204,
            "Exagerado",
            Some("Amor da minha vida, daqui até a eternidade"),
        ),
        entry(
            "Portuguese",
            Some("Os Vips"),
            0,
            516,
            "Faça Alguma Coisa Pelo Nosso Amor Que Eu Já Não Aguento Mais",
            Some("Faça alguma coisa pelo nosso amor porque eu já não aguento esperar mais nada"),
        ),
        entry("Portuguese", None, 0, 999, "Título Sem Intérprete", None),
        entry(
            "English",
            Some("Madonna"),
            7,
            204,
            "Like a Prayer",
            Some("Life is a mystery, everyone must stand alone"),
        ),
        entry(
            "English",
            Some("Queen"),
            7,
            301,
            "Bohemian Rhapsody",
            Some("Is this the real life? Is this just fantasy?"),
        ),
        entry(
            "English",
            Some("Queen"),
            7,
            302,
            "Somebody to Love",
            Some("Can anybody find me somebody to love"),
        ),
        entry(
            "English",
            Some("Pink Floyd"),
            7,
            400,
            "Wish You Were Here (Live)",
            Some("So, so you think you can tell heaven from hell"),
        ),
        // What the base-14 bargain costs, put on the page rather than in a footnote.
        entry(
            "Japanese",
            Some("矢代亜紀"),
            9,
            319,
            "アイを信じたい",
            Some("きっとあなたと"),
        ),
    ];

    // Enough rows to push past one page and show the heading arithmetic and the artist repeat.
    for n in 0..60 {
        entries.push(entry(
            "English",
            Some("Roberto Carlos"),
            7,
            100 + n,
            &format!("Filler Song Number {n}"),
            Some("A line of words that goes on for long enough to need cutting off somewhere"),
        ));
    }

    let style = BookStyle {
        subtitle: Some(format!("{} songs · sample", entries.len())),
        ..BookStyle::default()
    };
    let book = km_songbook::build(entries, "No language recorded", style);

    let out: PathBuf = std::env::args().nth(1).map_or_else(
        || PathBuf::from("target/km-songbook-sample.pdf"),
        PathBuf::from,
    );
    if let Some(parent) = out.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pdf = book.render();
    std::fs::write(&out, &pdf).expect("write the sample");

    println!(
        "{} songs over {} pages",
        book.row_count(),
        book.page_count()
    );
    println!("{} bytes -> {}", pdf.len(), out.display());
    let replaced = book.replaced();
    if replaced.count > 0 {
        let sample: String = replaced.sample.iter().collect();
        println!(
            "{} characters could not be drawn and became '?': {sample}",
            replaced.count
        );
    }
}
