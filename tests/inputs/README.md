# Real-data inputs

What this crate's real-data tests read, as [tests/inputs/mod.rs](mod.rs) declares them: the keys of the configuration
file the `GAME_DATA` environment variable names (a TOML file; its layout is that of
[primase-testdata](https://github.com/Kalmarv/primase-testdata), with a sample in that repository), the oracle sets and
the files of each, and the debug keys `GAME_DATA_DEBUG` sets. A test whose input is not configured skips with a line
naming its key.

Everything here is the user's own: game installs, archive keys, an Oodle library from the game, and reference output
made from the installs by independent readers. None of it is distributed with this repository. The tests that read
them are `collision.rs`, `havok_animation.rs` and every `*_census.rs`; the others (`emevd.rs`, `flver.rs`, `matbin.rs`,
`msb.rs`, `mtd.rs`, `shared_ranges.rs` and the unit tests in `src/`) use hand-built fixtures and run anywhere.

<!-- real-data inputs: made by primase_testdata::doc::tables from tests/inputs/mod.rs -->

#### Configuration keys

| key | gate | about | source |
| --- | --- | --- | --- |
| `installs.elden` | - | Elden Ring's `Game` folder (Data0 to Data3, `sd/sd`, DLC archives when installed) | the user's own install |
| `installs.sekiro` | yes | Sekiro's install folder (Data1 to Data5 BHD/BDT pairs, the Oodle 6 DLL) | the user's own install |
| `keys.elden` | - | the user's Elden Ring archive keys, one PKCS#1 `RSA PUBLIC KEY` PEM per archive named after its stem, and `regulation.key`, the key of `regulation.bin` as 64 hexadecimal digits (`Keys::from_folder` can hold it to a SHA-256 the caller states) | the user's own copies of the community unpacker's published keys (UXM's ArchiveKeys.cs) and the key SoulsFormats' RegulationDecryptor names for the game, written out as files; none is distributed here |
| `keys.sekiro` | yes | the user's Sekiro archive keys: one PKCS#1 `RSA PUBLIC KEY` PEM per archive, Data1.pem to Data5.pem | the user's own copy of the community unpacker's published keys (UXM's ArchiveKeys.cs), written out as PEM files; none is distributed here |
| `tools.oodle6` | yes | an Oodle version-6 DLL (`oo2core_6_win64.dll`); on Windows only: elsewhere oozextract decodes and nothing is configured | the game's own DLL from the user's install |
| `scratch.root` | - | where real-data tests write their outputs (the OS temporary folder when unset) | - |

#### Oracle sets (`<oracles.root>/<set>/<game>/`)

| set | about | source |
| --- | --- | --- |
| `oracles.corpus` | the games' own files cut from their installs (shader containers, EBX documents, Havok tagfiles, DCX files, CAS frames) | the user's own installs: files extracted as stored |
| `oracles.soulsformats` | SoulsFormats' readings of FromSoftware archives and files (rigid parts, skins, parts, census, frames, formats) | independent: SoulsFormats 9c0da6b (Sekiro) and SoulsFormatsNEXT ee1dd61 (Elden Ring), through a small dump harness that is not part of this repository |
| `oracles.smithbox` | Smithbox's Paramdex-derived paramdefs (`defs`, XML, with `FirstVersion`/`RemovedVersion`) and the `regulation.bin` of each Elden Ring release it ships | third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM` (MIT repository; the defs and regulations carry no licence of their own, so they are read in place and never copied into a repository) |
| `oracles.uxm` | UXM's path dictionaries (`dictionary.txt`, UTF-8, one archive path per line) | third-party: UXM-Selective-Unpack 9501be8's dictionaries |

#### Declared files

| key | status | gate | about | source |
| --- | --- | --- | --- | --- |
| `oracles.corpus.sekiro/havok` | live | yes | one area's high-resolution map collision as its binder stores it: loose Havok 2016.2 tagfiles and the compendium they share, every body and compressed mesh read; tests/collision.rs; schema in https://github.com/Kalmarv/primase/tree/main/toolkit/havok/README.md | the game's own files from the user's install: the extraction example https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/examples/corpus_sekiro_havok.rs (the area's `h*.hkxbhd`/`.hkxbdt` binder, each member out of its DCX envelope) |
| `oracles.corpus.sekiro/havok-soup` | live | yes | an independent decoder's raw triangle soup of each `havok` tagfile, `<name>.hkx.soup`; schema in https://github.com/Kalmarv/primase/tree/main/toolkit/havok/README.md; tests/collision.rs | independent: a clean-room decoder written from the data alone (no code of this crate, no third-party code), a small Python-standard-library program (5f6dde8) run over `corpus.sekiro/havok`; its README states the format facts and its counts.tsv the per-file triangles |
| `oracles.soulsformats.elden/census` | live | - | an independent census of installs.elden in oracles.soulsformats.sekiro/census's schema (the archives' decrypted headers as `<name>.plain`, entries.tsv, binders.tsv, errors.tsv, samples; hashes 16 hex digits; an archive's shard is its index in UXM's Elden Ring archive list); tests/flver_census.rs | independent: SoulsFormatsNEXT ee1dd61 with UXM 9501be8's keys, RSA and path hash, the Elden Ring harness's `census` mode (not part of this repository) |
| `oracles.soulsformats.elden/format` | live | - | an independent reader's field-by-field reading of Elden Ring files read from installs.elden (JSON lines, floats as hex bit patterns): `msb.jsonl` (every map layout: its models, every part with kind, model, transform, entity IDs, shadow controls and hit filter, and its spawn point regions with name, position and rotation), `btl.jsonl` (every light bank: its version and lights, or the reference's refusal), `havok.jsonl` (chosen tiles' map collision and assets' collision: every compressed mesh shape's stored sections, vertices, primitives, shape keys and extern shape instances), `matbin.jsonl` (every material binder's members in member order: shader and source paths, key, every parameter's name, key, type and value, every sampler's type, path, key and two trailing floats), `metaparam.jsonl` (every shader bundle's metaparam: binder, bundle and member names, header words, texture slots, render passes, CAParams and MtdParams); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/btl_census.rs, tests/havok_census.rs, tests/matbin_census.rs, tests/metaparam_census.rs | independent: SoulsFormatsNEXT ee1dd61 with UXM 9501be8's keys and path hash, and HKLib ac40bc9 for Havok, the Elden Ring harness's format modes (not part of this repository) |
| `oracles.soulsformats.sekiro/census` | live | yes | reference archive outputs; schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md | independent: SoulsFormats 9c0da6b with UXM's Sekiro dictionary, the dump harness's census mode (not part of this repository) |
| `oracles.soulsformats.sekiro/materials` | live | yes | independent MTD fields and block lengths (mtd.jsonl, mtd-boundaries.jsonl); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/mtd_census.rs | independent: SoulsFormats 9c0da6b, through a small dump harness that is not part of this repository (`mtd` mode): the fields from its MTD reader, the consumed block lengths from its writer's rewrite of each file |
| `oracles.soulsformats.sekiro/tpf` | live | yes | independent texture-package census (packages.jsonl, area-dds/, errors.tsv); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/tpf_census.rs | independent: SoulsFormats 9c0da6b, through a small dump harness that is not part of this repository (`tpf` mode); area-dds/ holds every distinct DDS under `/map/` |
| `oracles.smithbox.elden/defs` | live | - | the Elden Ring paramdefs (`ER/Defs`: 179 XML files, 29 with version-gated fields); tests/paramdef_census.rs | third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/ER/Defs`, read in place |
| `oracles.smithbox.elden/regulations` | live | - | the 28 Elden Ring regulations of Smithbox's releases, one folder `<release> (<regulation number>)` each holding `regulation.bin` (1.00.0 to 1.17.3); tests/paramdef_census.rs | third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/ER/Regulations`, read in place (encrypted with the install's `regulation.key`) |
| `oracles.smithbox.sekiro/defs` | live | yes | the Sekiro paramdefs (`SDT/Defs`: 160 XML files, none version-gated); tests/paramdef_census.rs | third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/SDT/Defs`, read in place |
| `oracles.uxm.elden/dictionary.txt` | live | - | caller-provided UTF-8 Elden Ring path dictionary (names all 97,118 entries of the base archives) | third-party: UXM-Selective-Unpack 9501be8's Elden Ring path dictionary, then a `#smithbox-er-file-dictionary` section of the 87 paths of Smithbox 3485939's `ER-File-Dictionary.json` (MIT; a list of archive paths, taken as data) that hash onto the 87 archive entries UXM's list leaves unnamed |
| `oracles.uxm.sekiro/dictionary.txt` | live | yes | caller-provided UTF-8 Sekiro path dictionary | third-party: UXM's Sekiro path dictionary |

<!-- end of real-data inputs -->
