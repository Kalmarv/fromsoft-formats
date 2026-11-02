//! `Archive::scan` gives every entry of each install once, with the bytes `Archive::read_hash` returns for it or the same
//! error: the whole-archive pass is the entry-by-entry read in another order, nothing more. Each entry is compared by
//! its length and MD5 (or its error's text), every entry of both installs, read both ways.
mod inputs;

use fromsoft_formats::{Archive, Binder, Game, dcx::Decoder};
use inputs::{Tool, oodle, threads};
use primase_foundations::hash::md5;
use primase_foundations::parallel::{Threads, map};
use std::collections::BTreeMap;
use std::sync::Mutex;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// An entry as a read gave it: its length and MD5, or the error's text.
type Seen = std::result::Result<(usize, [u8; 16]), String>;

fn seen(r: fromsoft_formats::Result<&[u8]>) -> Seen {
    r.map(|b| (b.len(), md5(b))).map_err(|e| e.to_string())
}

fn compare(game: Game) -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(game)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 31,
    };
    let started = std::time::Instant::now();
    let scanned: Mutex<BTreeMap<u64, Vec<Seen>>> = Mutex::new(BTreeMap::new());
    let s = archive
        .scan(
            &decoder,
            threads(),
            |_| true,
            |hash, r| {
                let s = seen(r);
                scanned
                    .lock()
                    .expect("no visit panics")
                    .entry(hash)
                    .or_default()
                    .push(s);
            },
        )
        .map_err(|p| p.to_string())?;
    let scan_time = started.elapsed();
    let scanned = scanned.into_inner().map_err(|e| e.to_string())?;

    let started = std::time::Instant::now();
    let hashes: Vec<u64> = archive.entries.keys().copied().collect();
    let each: Vec<Seen> = map(Threads(threads()), &hashes, |_, &h| {
        match Archive::read_hash(&archive, h, &decoder) {
            Ok(b) => seen(Ok(&b)),
            Err(e) => seen(Err(e)),
        }
    })
    .map_err(|p| p.to_string())?;
    let each_time = started.elapsed();

    assert_eq!(scanned.len(), hashes.len(), "{game:?}: entries scanned");
    let mut differ = Vec::new();
    // each pair's entries, those refused both ways and those that differ, by shard: a pair's reads stand on their own
    let mut pairs: BTreeMap<u8, [usize; 3]> = BTreeMap::new();
    for (h, want) in hashes.iter().zip(&each) {
        let got = &scanned[h];
        assert_eq!(
            got.len(),
            1,
            "{game:?}: entry {h:08x} visited {} times",
            got.len()
        );
        let pair = pairs.entry(archive.entries[h].shard).or_default();
        pair[0] += 1;
        pair[1] += usize::from(want.is_err());
        if &got[0] != want {
            pair[2] += 1;
            differ.push(format!("{h:08x}: scan {:?}, read_hash {want:?}", got[0]));
        }
    }
    for (shard, [entries, refused, differ]) in &pairs {
        let stem = archive.pair(*shard).map_or("an unknown pair", |p| p.stem);
        eprintln!("{game:?} {stem}: {entries} entries, {refused} refused both ways, {differ} differ");
    }
    for stem in &archive.absent {
        eprintln!("{game:?} {stem}: not installed");
    }
    assert_eq!(
        pairs.len(),
        archive.headers.len(),
        "{game:?}: a pair the install holds gave no entry"
    );
    let errors = each.iter().filter(|s| s.is_err()).count();
    eprintln!(
        "{game:?}: {} entries ({errors} refused both ways), {} reads of {:.2} GB; scan {:.1} s, entry by entry {:.1} s, {} threads",
        hashes.len(),
        s.windows,
        s.bytes as f64 / 1e9,
        scan_time.as_secs_f64(),
        each_time.as_secs_f64(),
        threads()
    );
    assert!(
        differ.is_empty(),
        "{game:?}: {} entries differ:\n{}",
        differ.len(),
        differ.join("\n")
    );
    Ok(())
}

#[test]
fn sekiro_scan_equals_read_hash_on_every_entry() -> Result<()> {
    compare(Game::Sekiro)
}

#[test]
fn elden_ring_scan_equals_read_hash_on_every_entry() -> Result<()> {
    compare(Game::EldenRing)
}

/// `Archive::read_each` gives each path, in the order asked, what `Archive::read` gives it: every shader and material
/// binder, one of them asked twice, and a path the index lacks.
fn read_each_is_read(game: Game) -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(game)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 31,
    };
    let mut paths: Vec<&str> = game
        .shader_binders()
        .iter()
        .chain(game.material_binders())
        .copied()
        .collect();
    paths.push("/no/such/entry.dcx");
    paths.push(paths[0]);
    let got = archive
        .read_each(&paths, &decoder, threads(), |path, r| (path.to_owned(), seen(r)))
        .map_err(|p| p.to_string())?;
    let want: Vec<(String, Seen)> = paths
        .iter()
        .map(|&path| match archive.read(path, &decoder) {
            Ok(b) => (path.to_owned(), seen(Ok(&b))),
            Err(e) => (path.to_owned(), seen(Err(e))),
        })
        .collect();
    assert!(
        want.iter().filter(|(_, s)| s.is_ok()).count() >= 2,
        "{game:?}: binders read"
    );
    assert!(
        want[want.len() - 2].1.is_err(),
        "{game:?}: the missing path is refused"
    );
    assert_eq!(got, want, "{game:?}");
    Ok(())
}

#[test]
fn sekiro_read_each_is_read() -> Result<()> {
    read_each_is_read(Game::Sekiro)
}

#[test]
fn elden_ring_read_each_is_read() -> Result<()> {
    read_each_is_read(Game::EldenRing)
}

/// A binder as its members' names, lengths and MD5s, or the error's text.
type Members = std::result::Result<Vec<(Option<String>, usize, [u8; 16])>, String>;

fn members(binder: fromsoft_formats::Result<Binder>) -> Members {
    binder
        .map(|b| {
            b.files
                .iter()
                .map(|f| (f.name.clone(), f.bytes.len(), md5(&f.bytes)))
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// `Archive::read_each_binder` gives each path, in the order asked, the binder `Binder::read` reads from what
/// `Archive::read` gives (a split binder's index with `Binder::read_split` and the payload beside it): the shader
/// binders, a map's collision binder (split), it asked twice, and a binder and a split binder the index lacks.
fn read_each_binder_is_read(game: Game) -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(game)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 31,
    };
    let split: Vec<String> = ["m11_01_00_00", "m10_00_00_00"]
        .iter()
        .flat_map(|m| {
            [
                format!("/map/{m}/h{}.hkxbhd", &m[1..]),
                format!("/map/{}/{m}/h{}.hkxbhd", &m[..3], &m[1..]),
            ]
        })
        .filter(|p| archive.contains(p))
        .collect();
    assert!(!split.is_empty(), "{game:?}: no collision binder");
    let mut paths: Vec<String> = game.shader_binders().iter().map(|p| (*p).to_owned()).collect();
    paths.extend(split.iter().cloned());
    paths.extend([
        "/no/such/entry.bnd.dcx".to_owned(),
        "/no/such/entry.hkxbhd".to_owned(),
        split[0].clone(),
    ]);
    let got = archive
        .read_each_binder(&paths, &decoder, threads(), |_, binder| members(binder))
        .map_err(|p| p.to_string())?;
    let want: Vec<Members> = paths
        .iter()
        .map(|path| {
            members(match path.strip_suffix("bhd") {
                Some(stem) => archive.read(path, &decoder).and_then(|index| {
                    Binder::read_split(&index, &archive.read(&format!("{stem}bdt"), &decoder)?, &decoder)
                }),
                None => archive
                    .read(path, &decoder)
                    .and_then(|b| Binder::read(&b, &decoder)),
            })
        })
        .collect();
    assert!(
        want[want.len() - 3].is_err() && want[want.len() - 2].is_err(),
        "{game:?}: the missing paths"
    );
    assert!(
        want.last()
            .is_some_and(|m| m.as_ref().is_ok_and(|m| !m.is_empty())),
        "{game:?}: the split binder"
    );
    assert_eq!(got, want, "{game:?}");
    Ok(())
}

#[test]
fn sekiro_read_each_binder_is_read() -> Result<()> {
    read_each_binder_is_read(Game::Sekiro)
}

#[test]
fn elden_ring_read_each_binder_is_read() -> Result<()> {
    read_each_binder_is_read(Game::EldenRing)
}
