//! The real-data inputs this crate's tests read, each with what it is, where it comes from and whether it is a gate
//! input (one that fails instead of skipping when the `GAME_DATA` file sets `[run] require = 'gate'`;
//! `primase_testdata::inputs!`; README.md here lists them). Integration tests reach it
//! with `mod inputs;`, unit tests with `#[cfg(test)] #[path = "../tests/inputs/mod.rs"] mod inputs;` in the
//! crate root, and tests/real_data_inputs.rs checks that every query the crate's sources write is declared
//! here. Declare an input here when a test first asks for it.

primase_testdata::inputs! {
    games {
        Elden = "elden",
        Sekiro = "sekiro",
    }
    installs {
        Elden {
            about: "Elden Ring's `Game` folder (Data0 to Data3, `sd/sd`, DLC archives when installed)",
            source: "the user's own install",
            gate: false,
        },
        Sekiro {
            about: "Sekiro's install folder (Data1 to Data5 BHD/BDT pairs, the Oodle 6 DLL)",
            source: "the user's own install",
            gate: true,
        },
    }
    keys {
        Elden {
            about: "the user's Elden Ring archive keys, one PKCS#1 `RSA PUBLIC KEY` PEM per archive named after its stem, and `regulation.key`, the key of `regulation.bin` as 64 hexadecimal digits (`Keys::from_folder` can hold it to a SHA-256 the caller states)",
            source: "the user's own copies of the community unpacker's published keys (UXM's ArchiveKeys.cs) and the key SoulsFormats' RegulationDecryptor names for the game, written out as files; none is distributed here",
            gate: false,
        },
        Sekiro {
            about: "the user's Sekiro archive keys: one PKCS#1 `RSA PUBLIC KEY` PEM per archive, Data1.pem to Data5.pem",
            source: "the user's own copy of the community unpacker's published keys (UXM's ArchiveKeys.cs), written out as PEM files; none is distributed here",
            gate: true,
        },
    }
    tools {
        Oodle6 = "oodle6" {
            about: "an Oodle version-6 DLL (`oo2core_6_win64.dll`); on Windows only: elsewhere oozextract decodes and nothing is configured",
            source: "the game's own DLL from the user's install",
            gate: true,
        },
    }
    oracle_sets {
        Corpus = "corpus" {
            about: "the games' own files cut from their installs (shader containers, EBX documents, Havok tagfiles, DCX files, CAS frames)",
            source: "the user's own installs: files extracted as stored",
        },
        SoulsFormats = "soulsformats" {
            about: "SoulsFormats' readings of FromSoftware archives and files (rigid parts, skins, parts, census, frames, formats)",
            source: "independent: SoulsFormats 9c0da6b (Sekiro) and SoulsFormatsNEXT ee1dd61 (Elden Ring), through a small dump harness that is not part of this repository",
        },
        Smithbox = "smithbox" {
            about: "Smithbox's Paramdex-derived paramdefs (`defs`, XML, with `FirstVersion`/`RemovedVersion`) and the `regulation.bin` of each Elden Ring release it ships",
            source: "third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM` (MIT repository; the defs and regulations carry no licence of their own, so they are read in place and never copied into a repository)",
        },
        Fxr = "fxr" {
            about: "the fxr library's readings of the games' effect files (one JSON line per `.fxr`: action counts and every light's parameters)",
            source: "independent: EvenTorset/fxr 2b6b52f (Unlicense), `FXR.read` with `Game.Sekiro` / `Game.EldenRing`, through a small harness that is not part of this repository",
        },
        Uxm = "uxm" {
            about: "UXM's path dictionaries (`dictionary.txt`, UTF-8, one archive path per line)",
            source: "third-party: UXM-Selective-Unpack 9501be8's dictionaries",
        },
    }
    declared {
        Query::Oracle(OracleSet::Corpus, Game::Sekiro, "havok") => {
            status: Live,
            gate: true,
            about: r#"one area's high-resolution map collision as its binder stores it: loose Havok 2016.2 tagfiles and the compendium they share, every body and compressed mesh read; tests/collision.rs; schema in https://github.com/Kalmarv/primase/tree/main/toolkit/havok/README.md"#,
            source: r#"the game's own files from the user's install: the extraction example https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/examples/corpus_sekiro_havok.rs (the area's `h*.hkxbhd`/`.hkxbdt` binder, each member out of its DCX envelope)"#,
        },
        Query::Oracle(OracleSet::Corpus, Game::Sekiro, "havok-soup") => {
            status: Live,
            gate: true,
            about: r#"an independent decoder's raw triangle soup of each `havok` tagfile, `<name>.hkx.soup`; schema in https://github.com/Kalmarv/primase/tree/main/toolkit/havok/README.md; tests/collision.rs"#,
            source: r#"independent: a clean-room decoder written from the data alone (no code of this crate, no third-party code), a small Python-standard-library program (5f6dde8) run over `corpus.sekiro/havok`; its README states the format facts and its counts.tsv the per-file triangles"#,
        },
        Query::Oracle(OracleSet::SoulsFormats, Game::Elden, "census") => {
            status: Live,
            gate: false,
            about: r#"an independent census of installs.elden in oracles.soulsformats.sekiro/census's schema (the archives' decrypted headers as `<name>.plain`, entries.tsv, binders.tsv, errors.tsv, samples; hashes 16 hex digits; an archive's shard is its index in UXM's Elden Ring archive list); tests/flver_census.rs"#,
            source: r#"independent: SoulsFormatsNEXT ee1dd61 with UXM 9501be8's keys, RSA and path hash, the Elden Ring harness's `census` mode (not part of this repository)"#,
        },
        Query::Oracle(OracleSet::SoulsFormats, Game::Elden, "format") => {
            status: Live,
            gate: false,
            about: r#"an independent reader's field-by-field reading of Elden Ring files read from installs.elden (JSON lines, floats as hex bit patterns): `msb.jsonl` (every map layout: its models, every part with kind, model, transform, entity IDs, shadow controls and hit filter, and its spawn point regions with name, position and rotation), `btl.jsonl` (every light bank: its version and lights, or the reference's refusal), `havok.jsonl` (chosen tiles' map collision and assets' collision: every compressed mesh shape's stored sections, vertices, primitives, shape keys and extern shape instances), `matbin.jsonl` (every material binder's members in member order: shader and source paths, key, every parameter's name, key, type and value, every sampler's type, path, key and two trailing floats), `metaparam.jsonl` (every shader bundle's metaparam: binder, bundle and member names, header words, texture slots, render passes, CAParams and MtdParams); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/btl_census.rs, tests/havok_census.rs, tests/matbin_census.rs, tests/metaparam_census.rs"#,
            source: r#"independent: SoulsFormatsNEXT ee1dd61 with UXM 9501be8's keys and path hash, and HKLib ac40bc9 for Havok, the Elden Ring harness's format modes (not part of this repository)"#,
        },
        Query::Oracle(OracleSet::SoulsFormats, Game::Sekiro, "census") => {
            status: Live,
            gate: true,
            about: r#"reference archive outputs; schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md"#,
            source: r#"independent: SoulsFormats 9c0da6b with UXM's Sekiro dictionary, the dump harness's census mode (not part of this repository)"#,
        },
        Query::Oracle(OracleSet::SoulsFormats, Game::Sekiro, "materials") => {
            status: Live,
            gate: true,
            about: r#"independent MTD fields and block lengths (mtd.jsonl, mtd-boundaries.jsonl); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/mtd_census.rs"#,
            source: r#"independent: SoulsFormats 9c0da6b, through a small dump harness that is not part of this repository (`mtd` mode): the fields from its MTD reader, the consumed block lengths from its writer's rewrite of each file"#,
        },
        Query::Oracle(OracleSet::SoulsFormats, Game::Sekiro, "tpf") => {
            status: Live,
            gate: true,
            about: r#"independent texture-package census (packages.jsonl, area-dds/, errors.tsv); schema in https://github.com/Kalmarv/primase/tree/main/worlds/fromsoft/README.md; tests/tpf_census.rs"#,
            source: r#"independent: SoulsFormats 9c0da6b, through a small dump harness that is not part of this repository (`tpf` mode); area-dds/ holds every distinct DDS under `/map/`"#,
        },
        Query::Oracle(OracleSet::Smithbox, Game::Elden, "defs") => {
            status: Live,
            gate: false,
            about: r#"the Elden Ring paramdefs (`ER/Defs`: 179 XML files, 29 with version-gated fields); tests/paramdef_census.rs"#,
            source: r#"third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/ER/Defs`, read in place"#,
        },
        Query::Oracle(OracleSet::Smithbox, Game::Elden, "regulations") => {
            status: Live,
            gate: false,
            about: r#"the 28 Elden Ring regulations of Smithbox's releases, one folder `<release> (<regulation number>)` each holding `regulation.bin` (1.00.0 to 1.17.3); tests/paramdef_census.rs"#,
            source: r#"third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/ER/Regulations`, read in place (encrypted with the install's `regulation.key`)"#,
        },
        Query::Oracle(OracleSet::Smithbox, Game::Sekiro, "defs") => {
            status: Live,
            gate: true,
            about: r#"the Sekiro paramdefs (`SDT/Defs`: 160 XML files, none version-gated); tests/paramdef_census.rs"#,
            source: r#"third-party: Smithbox fdde3225's `Smithbox.Data/Assets/PARAM/SDT/Defs`, read in place"#,
        },
        Query::Oracle(OracleSet::Fxr, Game::Elden, "lights.jsonl") => {
            status: Live,
            gate: false,
            about: r#"the fxr library's reading of Elden Ring's 12,155 effect files: per file the id, the count of each action type and every PointLight and SpotLight with the parameters the library names (a property as a number, an array or a function with keyframes); tests/fxr_census.rs"#,
            source: r#"independent: EvenTorset/fxr 2b6b52f (Unlicense, version 32.1.3) built privately, its `FXR.read(bytes, Game.EldenRing)` over the `.fxr` members of the install's `/sfx/*.ffxbnd.dcx` binders (the harness is not part of this repository)"#,
        },
        Query::Oracle(OracleSet::Fxr, Game::Sekiro, "lights.jsonl") => {
            status: Live,
            gate: true,
            about: r#"the fxr library's reading of Sekiro's 2,913 effect files (as for Elden Ring; it refuses three, `f000800042` to `f000800044`, by an action-603 field it types as a boolean); tests/fxr_census.rs"#,
            source: r#"independent: EvenTorset/fxr 2b6b52f (Unlicense, version 32.1.3) built privately, its `FXR.read(bytes, Game.Sekiro)` over the `.fxr` members of the install's `/sfx/*.ffxbnd.dcx` binders (the harness is not part of this repository)"#,
        },
        Query::Oracle(OracleSet::Uxm, Game::Elden, "dictionary.txt") => {
            status: Live,
            gate: false,
            about: r#"caller-provided UTF-8 Elden Ring path dictionary (names all 97,118 entries of the base archives)"#,
            source: r#"third-party: UXM-Selective-Unpack 9501be8's Elden Ring path dictionary, then a `#smithbox-er-file-dictionary` section of the 87 paths of Smithbox 3485939's `ER-File-Dictionary.json` (MIT; a list of archive paths, taken as data) that hash onto the 87 archive entries UXM's list leaves unnamed"#,
        },
        Query::Oracle(OracleSet::Uxm, Game::Sekiro, "dictionary.txt") => {
            status: Live,
            gate: true,
            about: r#"caller-provided UTF-8 Sekiro path dictionary"#,
            source: r#"third-party: UXM's Sekiro path dictionary"#,
        },
    }
    debug {}
}
