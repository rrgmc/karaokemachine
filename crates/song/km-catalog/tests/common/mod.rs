//! What both catalog test files need to put a package on disk.

use km_kmpkg::{Package, PackageBuilder, PackageMeta, SongEntry};

/// A package's metadata, named for its id.
pub fn meta(id: &str) -> PackageMeta {
    PackageMeta {
        id: id.to_owned(),
        name: format!("Package {id}"),
        version: "1.0.0".to_owned(),
        publisher: None,
        created: None,
        volume: None,
    }
}

/// Builds a package on disk with the given songs, each with distinct content.
pub fn build_package(dir: &std::path::Path, id: &str, songs: Vec<SongEntry>) -> Package {
    let path = dir.join(format!("{id}.kmpkg"));
    let mut builder = PackageBuilder::new(meta(id));
    for entry in songs {
        let bytes = format!("midi bytes for {}", entry.number).into_bytes();
        builder.add(entry, bytes).expect("add");
    }
    builder.write(&path).expect("write");
    Package::open(&path).expect("open")
}
