//! Lists the keys of the vertex and index bytes of every FLVER in the binders of an installed game's archives whose paths
//! hold one of the filters (binders only: paths ending in `bnd.dcx`), so a frame captured from the running game can be tied to the files its meshes came from.
//!
//! ```text
//! cargo run --release --example flver_keys -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>... > keys.tsv
//! ```
//!
//! The first arguments are `list_binder`'s; the dictionary is a file of archive paths, one per line (archives store only
//! their hashes). Each line written is `key<TAB>bytes<TAB>name`: the key is XXH3-128 as 32 lowercase hex digits (the
//! key locum names captured bytes by; `locum-cli index --keys keys.tsv` reads the list), and the name is
//! `<binder path>/<member>` and what the bytes are: `#block` (the data block, which Sekiro uploads as one buffer),
//! `#vb<n>` (a vertex buffer, in file order) or `#faces<n>` (a face set's indices). See [`fromsoft_formats::flver::Storage`].
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::flver::Storage;
use fromsoft_formats::{Binder, Game, Install, Keys, NameDictionary, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;
use xxhash_rust::xxh3::xxh3_128;

/// The lines of one binder's FLVERs, and its error if it could not be read.
fn lines(
    path: &str,
    binder: fromsoft_formats::Result<Binder>,
    decoder: &Decoder<'_>,
) -> Result<Vec<String>, String> {
    let binder = binder.map_err(|e| format!("{path}: {e}"))?;
    let mut out = Vec::new();
    for file in &binder.files {
        let Some(name) = file.name.as_deref() else {
            continue;
        };
        let member = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let lower = member.to_ascii_lowercase();
        if !(lower.ends_with(".flver") || lower.ends_with(".flver.dcx")) {
            continue;
        }
        let bytes = decoder
            .decode_if_dcx(&file.bytes)
            .map_err(|e| format!("{path}/{member}: {e}"))?;
        let storage = Storage::read(&bytes).map_err(|e| format!("{path}/{member}: {e}"))?;
        let block = storage
            .data_block(&bytes)
            .ok_or_else(|| format!("{path}/{member}: the data block is past the file's end"))?;
        let mut push = |what: String, part: Option<&[u8]>| match part {
            Some(b) => out.push(format!(
                "{:032x}\t{}\t{path}/{member}#{what}",
                xxh3_128(b),
                b.len()
            )),
            None => eprintln!("{path}/{member}#{what}: past the data block's end"),
        };
        push("block".into(), Some(block));
        for (i, vb) in storage.vertex_buffers.iter().enumerate() {
            let len = vb.stride.checked_mul(vb.count);
            push(
                format!("vb{i}"),
                len.and_then(|l| block.get(vb.offset..vb.offset.checked_add(l)?)),
            );
        }
        for (i, f) in storage.face_sets.iter().enumerate() {
            let len = f.count.checked_mul(f.width as usize / 8);
            push(
                format!("faces{i}"),
                len.and_then(|l| block.get(f.offset..f.offset.checked_add(l)?)),
            );
        }
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, dictionary, filters @ ..] = args.as_slice() else {
        return Err("usage: flver_keys <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>...".into());
    };
    if filters.is_empty() {
        return Err(
            "give at least one filter (a part of the archive paths to read, like /asset/aeg/aeg096/)".into(),
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
        // binders (`.geombnd.dcx`, `.partsbnd.dcx`, `.chrbnd.dcx`, `.mapbnd.dcx`); textures, navmeshes and the rest hold no FLVER
        .filter(|p| p.ends_with("bnd.dcx"))
        .collect();
    let paths: Vec<&str> = named.iter().copied().filter(|p| archive.contains(p)).collect();
    eprintln!(
        "{} binder paths of the dictionary match; {} of them are in the archives",
        named.len(),
        paths.len()
    );
    let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let results = archive
        .read_each_binder(&paths, &decoder, threads, |path, binder| {
            lines(path, binder, &decoder)
        })
        .map_err(|e| format!("{e:?}"))?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let (mut written, mut failed) = (0usize, 0usize);
    for r in results {
        match r {
            Ok(lines) => {
                for l in lines {
                    writeln!(out, "{l}")?;
                    written += 1;
                }
            }
            Err(e) => {
                eprintln!("{e}");
                failed += 1;
            }
        }
    }
    out.flush()?;
    eprintln!("{written} keys written; {failed} binders could not be read");
    Ok(())
}
