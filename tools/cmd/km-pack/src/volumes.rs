//! Dividing a description longer than one package into volumes.
//!
//! A package holds at most [`km_songcode::MAX_SLOT`] songs. A folder can hold more, and a tool that
//! packages a whole folder divides it rather than refusing it. See `A package holds at most 999 songs`
//! and `A package file can say which set it is a volume of` in `docs/decisions/packaging.md`.
//!
//! **Each volume is a package in every sense the machine has**: its own id, its own bank and its own
//! numbers from 1. The first volume's id is the set's id, which is what [`km_kmpkg::VolumeOf::of`]
//! means. A description that fits in one package comes back as it was given, with no volume.

use km_kmpkg::{PackageMeta, VolumeOf};

use crate::spec::Spec;

/// How a volume's number follows the set's name: `Name vol2`.
///
/// The curation tool's own default, so a set built by either tool names its files the same way.
pub const VOLUME_SUFFIX: &str = "vol";

/// Divides a description into as many as it takes to hold at most `per_volume` songs each.
///
/// The songs keep their order. Every volume of a set of two or more is numbered from its package's
/// `start_number`, carries `volume:`, and takes `<name> vol<n>` as its name. The first volume keeps
/// the description's id, and every later one gets a fresh one from [`PackageMeta::new_id`].
///
/// `out` is cleared on every volume of a set of two or more, because one path cannot name several
/// files; the caller names each one.
#[must_use]
pub fn split_into_volumes(spec: &Spec, per_volume: usize) -> Vec<Spec> {
    let per_volume = per_volume.max(1);
    if spec.songs.len() <= per_volume {
        return vec![spec.clone()];
    }

    let start = spec.package.start_number.max(1);
    spec.songs
        .chunks(per_volume)
        .enumerate()
        .map(|(index, songs)| {
            let number = u32::try_from(index + 1).expect("a volume count fits in u32");
            let mut volume = spec.clone();
            volume.package.id = if number == 1 {
                spec.package.id.clone()
            } else {
                PackageMeta::new_id()
            };
            volume.package.name = format!("{} {VOLUME_SUFFIX}{number}", spec.package.name);
            volume.package.volume = Some(VolumeOf {
                of: spec.package.id.clone(),
                name: spec.package.name.clone(),
                number,
            });
            volume.package.out = None;
            volume.songs = songs
                .iter()
                .zip(start..)
                .map(|(song, slot)| {
                    let mut song = song.clone();
                    song.number = Some(slot);
                    song
                })
                .collect();
            volume
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{SpecPackage, SpecSong};

    fn spec_of(count: u32) -> Spec {
        Spec {
            package: SpecPackage {
                id: "0123456789abcdef".to_owned(),
                name: "Party".to_owned(),
                version: "1.0.0".to_owned(),
                publisher: None,
                created: None,
                volume: None,
                default_language: None,
                encoding: None,
                start_number: 1,
                transcode: true,
                out: Some("party.kmpkg".to_owned()),
                uncurated: false,
            },
            root: None,
            songs: (1..=count)
                .map(|number| SpecSong {
                    file: format!("{number}.mid"),
                    number: Some(number),
                    ..SpecSong::default()
                })
                .collect(),
        }
    }

    #[test]
    fn a_description_that_fits_comes_back_as_it_was() {
        let spec = spec_of(3);
        let volumes = split_into_volumes(&spec, 999);
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].package.id, spec.package.id);
        assert_eq!(volumes[0].package.volume, None);
        assert_eq!(volumes[0].package.out.as_deref(), Some("party.kmpkg"));
    }

    #[test]
    fn a_long_description_becomes_volumes_numbered_from_one() {
        let spec = spec_of(5);
        let volumes = split_into_volumes(&spec, 2);
        assert_eq!(volumes.len(), 3);

        assert_eq!(
            volumes[0].package.id, spec.package.id,
            "the first is the set"
        );
        assert_ne!(volumes[1].package.id, spec.package.id);
        assert_ne!(volumes[1].package.id, volumes[2].package.id);
        assert_eq!(volumes[2].package.name, "Party vol3");
        assert_eq!(
            volumes[2].package.volume,
            Some(VolumeOf {
                of: spec.package.id.clone(),
                name: "Party".to_owned(),
                number: 3,
            })
        );

        let numbers: Vec<Vec<u32>> = volumes
            .iter()
            .map(|volume| volume.songs.iter().filter_map(|song| song.number).collect())
            .collect();
        assert_eq!(numbers, [vec![1, 2], vec![1, 2], vec![1]]);
        assert_eq!(volumes[1].songs[0].file, "3.mid", "the order is kept");
        assert!(volumes.iter().all(|volume| volume.package.out.is_none()));
    }
}
