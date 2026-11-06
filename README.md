# fromsoft-formats

[![crates.io: fromsoft-formats](https://img.shields.io/crates/v/fromsoft-formats?label=fromsoft-formats)](https://crates.io/crates/fromsoft-formats) [![docs.rs](https://img.shields.io/docsrs/fromsoft-formats)](https://docs.rs/fromsoft-formats) [![CI](https://img.shields.io/github/actions/workflow/status/Kalmarv/fromsoft-formats/ci.yml?branch=main&label=CI)](https://github.com/Kalmarv/fromsoft-formats/actions/workflows/ci.yml)

A Rust library that reads and writes the data files of Sekiro: Shadows Die Twice and Elden Ring. It opens a game install's archives with keys you supply and parses the files inside them.

## Features

- Opens the BHD5/BDT archives of an install and reads entries by archive path, one at a time or in parallel
- Decodes DCX envelopes (DFLT, KRAK and ZSTD) and reads BND4 binders, including split BHF4/BDF4 binders
- Reads FLVER models, TPF texture packages, CLM2 cloth data and `.grass` meshes
- Reads MTD and MATBIN materials, shader METAPARAM files and Elden Ring shader and sampler names
- Reads MSB map layouts, BTL light banks, GPARAM draw parameters and Sekiro's EDGE hang edges
- Reads EMEVD event scripts and types their instructions against the community's EMEDF schema
- Reads FXR effects, with all 72 action types found in the two games' files typed
- Reads PARAM tables, binds them to Paramdex definitions and decrypts and encrypts Elden Ring's `regulation.bin`
- Reads FMG text, TAE animation events (checked against event templates you supply) and Elden Ring PC saves
- Reads map and object collision from Havok tagfiles through [havok-formats](https://github.com/Kalmarv/havok-formats)
- Writes DCX, BND4, TPF, FMG, PARAM, `regulation.bin`, MTD, MATBIN, EMEVD, TAE, MSB and FLVER files
- Rewrites a stock file it has read to the same bytes, except for compressed payloads it compresses again
- Returns an `Error::Invalid` naming the format and byte offset for data it does not accept, instead of panicking
- Keeps values as the file stores them, with no unit or coordinate conversion
- Contains no `unsafe` code and ships no keys, path dictionaries, native libraries or game files

## Getting Started

### Requirements

- Rust 1.99 or later (edition 2024).
- Your own copy of Sekiro or Elden Ring and its archive keys, one PKCS#1 `RSA PUBLIC KEY` PEM per archive in a folder (`Data1.pem` to `Data5.pem` for Sekiro; `Data0.pem` to `Data3.pem`, `DLC.pem`, `sd.pem` and `regulation.key` for Elden Ring).
- For Kraken-compressed files on Windows, the game's own `oo2core_6_win64.dll`. On other platforms a portable decoder is used and no library is needed.
- A path list (`NameDictionary`) if you want to turn archive path hashes back into names, since the archives store only hashes.

### Installation

```sh
cargo add fromsoft-formats
cargo add primase-foundations --features compress
```

`primase-foundations` provides the `Oodle` handle that the DCX decoder takes.

### Usage

This program lists the members of Sekiro's param binder:

```rust
use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One PEM per archive (Data1.pem ... Data5.pem), from a folder you hold.
    let keys = Keys::from_folder("keys", Game::Sekiro, None)?;
    let archive = Install::new("D:/Games/Sekiro", Game::Sekiro, keys).open()?;

    // The game's own Oodle library on Windows; pass None elsewhere for the portable decoder.
    // SAFETY: the library is the game's own.
    let oodle = unsafe { Oodle::open(Some(Path::new("D:/Games/Sekiro/oo2core_6_win64.dll")), Version::V6) }?;
    let decoder = Decoder { oodle: Some(&oodle), max_output: 1 << 30 };

    let bytes = archive.read("/param/gameparam/gameparam.parambnd.dcx", &decoder)?;
    for file in &Binder::read(&bytes, &decoder)?.files {
        println!("{:?} {} bytes", file.name, file.bytes.len());
    }
    Ok(())
}
```

A reader of a single file takes its decompressed bytes and returns its contents or an error:

```rust
use fromsoft_formats::{Error, flver::Model};

let err = Model::read(b"not a FLVER file").unwrap_err();
assert!(matches!(&err, Error::Invalid { format: "FLVER", offset: 0, .. }));
```

The repository has example programs that take the game, game folder, keys folder and Oodle library (or `-`) as their first arguments:

```sh
cargo run --example list_binder -- sekiro "D:/Games/Sekiro" ./keys "D:/Games/Sekiro/oo2core_6_win64.dll" /param/gameparam/gameparam.parambnd.dcx
```

- `list_binder`: lists the members of one binder in the archives
- `extract_binder`: writes the members of one binder to a folder
- `find_material`: lists the map pieces of a layout whose materials match a pattern
- `flver_keys`: lists hashes of the vertex and index data of FLVER models, for matching captured frames to files
- `texture_tar`: writes the textures of matching TPF packages to stdout as a tar stream of DDS files
- `rebuild_collision`: rebuilds a binder's compressed-mesh collision and writes it back
- `read_bench`: times the archive read path stage by stage

Run the tests with `cargo test --workspace`. They need no game files; tests that read a real install skip unless the `GAME_DATA` configuration is set, as described in [docs/testing.md](docs/testing.md). See [CONTRIBUTING.md](CONTRIBUTING.md) for the checks CI runs and how to report a file the crate cannot read, and [docs/](docs/) for the crate's guarantees and limits. The API reference is on [docs.rs](https://docs.rs/fromsoft-formats).

Licensed under GPL-3.0-or-later; see [LICENSE](LICENSE).
