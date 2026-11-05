# Guarantees and limits

## What it guarantees

- **Checked reads.** Every read of untrusted bytes goes through a bounds-checked view. Data that is shorter than it says, points outside itself, overlaps another part or holds a value the reader does not know is an `Error::Invalid` naming the format, the byte offset and what was expected. The readers are written to return that error rather than panic; this is exercised by hand-built bad inputs in the unit tests and by census tests that read the files of real installs, not by fuzzing, so a panic on a crafted file is a bug worth reporting.
- **Nothing half-read.** Most readers require every byte of a file to be accounted for (up to a few bytes of zero padding), so a layout the reader does not know is refused instead of silently misread. Where a reader deliberately leaves part of a file uninterpreted (event scripts, GPARAM's opaque blocks, the type data of MSB regions and events), it keeps those bytes as stored or counts them.
- **Values as stored.** Readers return what the file states, not what a game does with it: no unit conversion, no coordinate-system change, no guesses at the meaning of unknown fields (they are kept under their offsets).
- **Bounded work.** Decoded sizes are capped by the `max_output` you give the `Decoder`, and most readers limit how many bytes a file's records may name, as a multiple of the input's size, so a hostile file cannot make them allocate or copy without limit.
- **Deterministic.** Opening an install on any number of threads gives the same archive.
- **Narrow side effects.** The crate has no `unsafe` code, makes no network connections and starts no processes. Only `archive` (the install's archives) and `game_params` (Elden Ring's `regulation.bin`) read files, and a lint enforces that.
- **Lints.** The library denies `clippy::indexing_slicing`, `clippy::arithmetic_side_effects` and `clippy::cast_possible_truncation` outside tests, `unsafe_code`, and (`#![deny(missing_docs)]`) any public item without documentation. Exactly two items opt out of the strict lints, each with a stated reason: `affine::narrow` (a float narrowed to the nearest `f32`) and the vertex helper `exact_i32` in `flver` (a value checked integral and in range before the cast). The two modules that read files from disk, `archive` and `game_params`, also allow the lints that otherwise forbid file access.

## Writers

The crate writes DCX envelopes (`dcx::frame`, `dcx::encode`), BND4 and BHF4/BDF4 binders with their name hash tables (`Binder::write`), TPF packages (`Package::write`), FMG text files (`Fmg::write`), PARAM files (`Param::write`), the AES layer of `regulation.bin` (`regulation`), MTD and MATBIN materials, EMEVD event scripts, TAE event files, MSB map layouts and FLVER models. Each writer's documentation says where its output can differ from the stored bytes. They are checked by rebuilding the stock files of both installs (and the 29 `regulation.bin` files held) from their parsed fields. The structure comes back byte for byte; the compressed payloads of an envelope that is compressed again do not.

## What it does not do

- It does not repack the BHD5/BDT archives or write formats other than those listed above. A file it writes is not the original's bytes where a compressor decides them: the zlib, Kraken and zstd payloads it makes decode to the same data under the same header but are not the shipped payloads.
- It does not decode pixels, convert models, meshes or animations to other formats, or interpret scripts and flags. It is a reader of the formats, not a converter.
- It does not supply keys, dictionaries or the Oodle library, and it does not authenticate the SHA digests an archive index states (they are parsed and kept).
- It does not enumerate archive paths: the indexes store hashes only, so you pass a path list (`NameDictionary`) for the names you want to find.
- It reads only the formats and versions of Sekiro and Elden Ring (plus FMG's Dark Souls layout and GPARAM versions 3 and 6, which other games use). Other versions, and files from other FromSoftware games, are refused by name.
- It is not a game or a mod loader.

## Versions read

It was written and checked against the Steam builds of Sekiro 1.06 (`sekiro.exe` 1.6.0.0) and Elden Ring (`eldenring.exe` 2.7.2.0).

- Archives and containers: BHD5/BDT archive indexes and their entries (RSA-wrapped index, AES-128 ranges), the path hashes of both games, DCX envelopes (DFLT, KRAK and ZSTD) and BND4 binders, including split BHF4/BDF4 binders.
- Models and textures: FLVER2 models (versions `0x20013`, `0x20014`, `0x20016`, `0x2001a`, including Elden Ring's SpeedTree layouts), TPF texture packages (PC, DDS payloads kept whole, the pixel layout of each format code named), CLM2 cloth companions, `.grass` meshes and the community's names for the words of a material's GX items (`flver::gx`).
- Materials and shaders: Sekiro's MTD material definitions, Elden Ring's MATBIN materials, shader METAPARAM files (version 6), and Elden Ring's shader names and sampler names split into their parts (`shader`).
- Maps: MSB map layouts (Sekiro's MSBS and Elden Ring's MSBE), BTL light banks (versions 15, 16 and 18), EDGE hang edges (Sekiro), GPARAM draw parameters (versions 3, 5 and 6), EMEVD event scripts (the 64-bit layout of both games) and the part-state rules they set in Sekiro. Elden Ring's legacy maps are placed among the open-world tiles from the regulation's `WorldMapLegacyConvParam` (`overworld`).
- Effects: FXR effect files (version 5): the state machine, the tree of nodes, configs and actions, every action of the 72 types the installs hold typed (`fxr::actions`, `Fxr::typed`), the property functions evaluated and their modifiers decoded, the value sets named (`fxr::enums`), and the files an effect refers to (`fxr::resources`).
- Game data: PARAM files and their typed views, FMG text and the NPC names built from it, TAE animation events (version `0x1000D`) and, from a template file you supply, their event names and typed parameters (`tae::template`), the `.blf` load lists of divided animation binders, the names of the hit-material classes, Elden Ring's PC character saves (`save`), and map and object collision in Havok tagfiles.
