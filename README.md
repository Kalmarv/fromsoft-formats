# fromsoft-formats

A Rust library that reads the data files of FromSoftware's Sekiro: Shadows Die Twice and Elden Ring: the installed archives, the containers around files, and the models, maps, materials, textures, params, text, events and collision inside them.

It was written and checked against the Steam builds of Sekiro 1.06 (`sekiro.exe` 1.6.0.0) and Elden Ring
(`eldenring.exe` 2.7.2.0). It reads, by name:

- **Archives and containers.** BHD5/BDT archive indexes and their entries (RSA-wrapped index, AES-128 ranges), the path hashes of both games, DCX envelopes (DFLT, KRAK and ZSTD) and BND4 binders, including split BHF4/BDF4 binders.
- **Models and textures.** FLVER2 models (versions `0x20013`, `0x20014`, `0x20016`, `0x2001a`, including Elden Ring's SpeedTree layouts), TPF texture packages (PC, DDS payloads kept whole), CLM2 cloth companions.
- **Materials and shaders.** Sekiro's MTD material definitions, Elden Ring's MATBIN materials, shader METAPARAM files (version 6).
- **Maps.** MSB map layouts (Sekiro's MSBS and Elden Ring's MSBE: models, parts, regions, events), BTL light banks (versions 16 and 18), EDGE hang edges (Sekiro), GPARAM draw parameters (versions 3, 5 and 6), EMEVD event scripts (Sekiro's 64-bit layout) and the part-state rules they set.
- **Game data.** PARAM files (the NPC and grass params, and Sekiro's part-built NPC outfits through the equipment params; Elden Ring is refused there), FMG text and the NPC names built from it, TAE animation events (version `0x1000D`), and map and object collision in Havok tagfiles (through [`havok-formats`](https://github.com/Kalmarv/havok-formats)).

It needs a few things from you, because it ships none of them: the archive keys of your own install (one RSA public key per archive, and for Elden Ring the key of `regulation.bin`), which you keep as files in a folder and load with `Keys::from_folder`; the crate keeps them only in the `Keys` value you pass, the game's Oodle library (or, off Windows, a portable decoder) for Kraken-compressed files, and, to turn path hashes back into paths, a path list of your choosing. The crate holds no key material, no digests of it, no dictionary, no native library and no game file, and this repository does not contain or link to any.

## Example

This lists the members of Sekiro's param binder. It is `examples/list_binder.rs` in short form, and the same steps are compiled as a doc test in `src/lib.rs`.

```rust,no_run
use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One PKCS#1 `RSA PUBLIC KEY` PEM per archive (`Data1.pem` ... `Data5.pem`), which you hold.
    let keys = Keys::from_folder("keys", Game::Sekiro, None)?;
    let archive = Install::new("D:/Games/Sekiro", Game::Sekiro, keys).open()?;

    // Kraken-compressed files need Oodle: the game's own library on Windows, a portable decoder elsewhere.
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

The reader of a single file takes its decompressed bytes and returns the file's contents, or an error that says what it refused and where:

```rust
use fromsoft_formats::{Error, flver::Model};

let err = Model::read(b"not a FLVER file").unwrap_err();
assert!(matches!(&err, Error::Invalid { format: "FLVER", offset: 0, .. }));
println!("{err}");
```

Add it with `cargo add fromsoft-formats` (or `fromsoft-formats = "0.2"` under `[dependencies]`). The API reference is on [docs.rs](https://docs.rs/fromsoft-formats).

## What it guarantees

- **Checked reads.** Every read of untrusted bytes goes through a bounds-checked view. Data that is shorter than it says, points outside itself, overlaps another part or holds a value the reader does not know is an `Error::Invalid` naming the format, the byte offset and what was expected. The readers are written to return that error rather than panic; this is exercised by hand-built bad inputs in the unit tests and by census tests that read the files of real installs, not by fuzzing, so a panic on a crafted file is a bug worth reporting.
- **Nothing half-read.** Most readers require every byte of a file to be accounted for (up to a few bytes of zero padding), so a layout the reader does not know is refused instead of silently misread. Where a reader deliberately leaves part of a file uninterpreted (event scripts, GPARAM's opaque blocks, the type data of MSB regions and events), it keeps those bytes as stored or counts them.
- **Values as stored.** Readers return what the file states, not what a game does with it: no unit conversion, no coordinate-system change, no guesses at the meaning of unknown fields (they are kept under their offsets).
- **Bounded work.** Decoded sizes are capped by the `max_output` you give the `Decoder`, and most readers limit how many bytes a file's records may name, as a multiple of the input's size, so a hostile file cannot make them allocate or copy without limit.
- **Deterministic.** Opening an install on any number of threads gives the same archive.
- **Narrow side effects.** The crate has no `unsafe` code, makes no network connections and starts no processes. Only `archive` (the install's archives) and `game_params` (Elden Ring's `regulation.bin`) read files, and a lint enforces that.
- **Lints.** The library denies `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::cast_possible_truncation` outside tests, `unsafe_code`, and (`#![deny(missing_docs)]`) any public item without documentation. Exactly two items opt out of the strict lints, each with a stated reason: `affine::narrow` (a float narrowed to the nearest `f32`) and the vertex helper `exact_i32` in `flver` (a value checked integral and in range before the cast). The two modules that read files from disk, `archive` and `game_params`, also allow the lints that otherwise forbid file access.

## What it does not do

- It does not write or repack anything: every reader is read-only.
- It does not decode pixels, convert models, meshes or animations to other formats, or interpret scripts and flags. It is a reader of the formats, not a converter.
- It does not supply keys, dictionaries or the Oodle library, and it does not authenticate the SHA digests an archive index states (they are parsed and kept).
- It does not enumerate archive paths: the indexes store hashes only, so you pass a path list (`NameDictionary`) for the names you want to find.
- It reads only the formats and versions above (plus FMG's Dark Souls layout and GPARAM versions 3 and 6, which other games use). Other versions, and files from other FromSoftware games, are refused by name.
- It is not a game or a mod loader.

## Tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

These need no game files: the unit tests and the tests on hand-built fixtures run everywhere, and the tests that read a real install skip with a line naming the missing input.

To run those against your own copies, write a TOML file and point the `GAME_DATA` environment variable at it. Every section and key is optional. Paths below are placeholders:

```toml
[installs]
sekiro = 'D:/placeholder/Sekiro'
elden = 'D:/placeholder/ELDEN RING/Game'
[keys]                                   # folders of the PEM files you hold
sekiro = 'D:/placeholder/keys/sekiro'    # Data1.pem ... Data5.pem
elden = 'D:/placeholder/keys/elden'      # Data0.pem ... Data3.pem, DLC.pem, sd.pem, sd_dlc02.pem, regulation.key
[tools]
oodle6 = 'D:/placeholder/Sekiro/oo2core_6_win64.dll'
[oracles]
root = 'D:/placeholder/oracles'          # reference output made by other readers, see below
[scratch]
root = 'D:/placeholder/scratch'
[run]
require = 'gate'                         # a missing gate input fails the test instead of skipping it
gate = 'smoke'                           # or 'full': how much the heavy tests cover
```

[tests/inputs/README.md](tests/inputs/README.md) lists every input the tests read, what it is and where it comes from. Besides the installs, keys and Oodle library, some tests compare this crate's reading with the output of independent readers (SoulsFormats, SoulsFormatsNEXT, UXM's path lists) kept under `oracles.root`; those files are made by the user from their own installs and are not part of this repository. The file layout comes from [primase-testdata](https://github.com/Kalmarv/primase-testdata), which also has a sample.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md), including how to report a file this crate cannot read.

## Related work

The layouts follow the community's documentation of these formats, chiefly SoulsFormats and SoulsFormatsNEXT (names of unknown fields follow them), Paramdex and Smithbox (param layouts), and UXM (archive lists and path hashes); the tests check this crate's readings against theirs. Used by [primase](https://github.com/Kalmarv/primase), a game-asset conversion library.

## Licence

GPL-3.0-or-later; see [LICENSE](LICENSE). Dependencies that are forks of other projects keep their upstream licences.
