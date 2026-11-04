//! Writes every texture of the TPF packages in an installed game's archives whose paths hold one of the filters to
//! stdout as a tar stream of DDS files, so a frame captured from the running game can be tied to the files its textures
//! came from without writing them to disk.
//!
//! ```text
//! cargo run --release --example texture_tar -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>... | locum-cli index -o textures.db --tar -
//! ```
//!
//! The first arguments are `flver_keys`'s. A package is read where the archives hold it on its own (`.tpf.dcx`) or as a
//! member of a binder of the kinds that hold them (`.tpfbnd.dcx`, `.texbnd.dcx`, `.partsbnd.dcx`, `.ffxbnd.dcx`,
//! `.chrbnd.dcx`, a map's split `.tpfbhd`), one level deep. Each
//! texture is written as its DDS file, as the package stores it, named `<archive path>/<texture name>.dds`, or
//! `<binder path>/<member>/<texture name>.dds` for a package in a binder. The stream holds game data: keep what is made
//! of it out of any repository.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::tpf::Package;
use fromsoft_formats::{Binder, Game, Install, Keys, NameDictionary, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;

/// A texture to write: its name in the stream and its DDS file.
type Entry = (String, Vec<u8>);

/// Whether a stored path names a TPF package.
fn is_package(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tpf") || lower.ends_with(".tpf.dcx")
}

/// The textures of the package `bytes` (compressed or not) under `at`.
fn textures(at: &str, bytes: &[u8], decoder: &Decoder<'_>) -> Result<Vec<Entry>, String> {
    let bytes = decoder.decode_if_dcx(bytes).map_err(|e| format!("{at}: {e}"))?;
    let package = Package::read(&bytes).map_err(|e| format!("{at}: {e}"))?;
    Ok(package
        .textures
        .into_iter()
        .map(|t| (format!("{at}/{}.dds", t.name), t.dds))
        .collect())
}

/// The textures of a binder's packages; errors of single members are reported and skipped.
fn binder_textures(
    path: &str,
    binder: fromsoft_formats::Result<Binder>,
    decoder: &Decoder<'_>,
) -> Result<Vec<Entry>, String> {
    let binder = binder.map_err(|e| format!("{path}: {e}"))?;
    let mut out = Vec::new();
    for file in &binder.files {
        let Some(name) = file.name.as_deref() else {
            continue;
        };
        if !is_package(name) {
            continue;
        }
        let member = name.rsplit(['\\', '/']).next().unwrap_or(name);
        match textures(&format!("{path}/{member}"), &file.bytes, decoder) {
            Ok(t) => out.extend(t),
            Err(e) => eprintln!("{e}"),
        }
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, dictionary, filters @ ..] = args.as_slice() else {
        return Err("usage: texture_tar <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>...".into());
    };
    if filters.is_empty() {
        return Err(
            "give at least one filter (a part of the archive paths to read, like /asset/aet/ or /chr/)"
                .into(),
        );
    }
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let keys = Keys::from_folder(keys, game, None)?;
    let archive = Install::new(root, game, keys).open()?;
    let names = NameDictionary::read_for(dictionary, game)?;
    let named: Vec<&str> = names
        .names
        .values()
        .flatten()
        .map(String::as_str)
        .filter(|p| filters.iter().any(|f| p.contains(f.as_str())))
        .filter(|p| archive.contains(p))
        .collect();
    let packages: Vec<&str> = named.iter().copied().filter(|p| is_package(p)).collect();
    // The kinds of binder that hold packages (`.mapinfotexbnd.dcx` ends as `.texbnd.dcx` does); the rest (geometry,
    // collision, animation, maps' objects) hold none and are many. A split binder is named by its index (`.tpfbhd`),
    // its payload read beside it.
    const HOLDERS: [&str; 6] = [
        "tpfbnd.dcx",
        "texbnd.dcx",
        "partsbnd.dcx",
        "ffxbnd.dcx",
        "chrbnd.dcx",
        "tpfbhd",
    ];
    let binders: Vec<&str> = named
        .iter()
        .copied()
        .filter(|p| HOLDERS.iter().any(|h| p.to_ascii_lowercase().ends_with(h)))
        .collect();
    eprintln!(
        "{} packages and {} binders of the dictionary match and are in the archives",
        packages.len(),
        binders.len()
    );
    let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let mut tar = tar::Builder::new(std::io::BufWriter::new(std::io::stdout().lock()));
    let (mut written, mut failed) = (0usize, 0usize);
    let mut write = |result: Result<Vec<Entry>, String>| -> std::io::Result<()> {
        match result {
            Ok(entries) => {
                for (name, dds) in entries {
                    let mut header = tar::Header::new_gnu();
                    header.set_size(dds.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    tar.append_data(&mut header, name.trim_start_matches('/'), dds.as_slice())?;
                    written += 1;
                }
            }
            Err(e) => {
                eprintln!("{e}");
                failed += 1;
            }
        }
        Ok(())
    };
    // In batches, so no more than a batch's textures are held at once.
    for batch in packages.chunks(256) {
        let results = archive
            .read_each(batch, &decoder, threads, |path, bytes| {
                bytes
                    .map_err(|e| format!("{path}: {e}"))
                    .and_then(|b| textures(path, b, &decoder))
            })
            .map_err(|e| format!("{e:?}"))?;
        for r in results {
            write(r)?;
        }
    }
    for batch in binders.chunks(64) {
        let results = archive
            .read_each_binder(batch, &decoder, threads, |path, binder| {
                binder_textures(path, binder, &decoder)
            })
            .map_err(|e| format!("{e:?}"))?;
        for r in results {
            write(r)?;
        }
    }
    tar.into_inner()?.flush()?;
    eprintln!("{written} textures written; {failed} packages or binders could not be read");
    Ok(())
}
