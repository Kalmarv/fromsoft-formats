//! `Archive::scan` gives every entry of each install once, with the bytes `Archive::read_hash` returns for it or the same
//! error: the whole-archive pass is the entry-by-entry read in another order, nothing more. Each entry is compared by
//! its length and MD5 (or its error's text), every entry of both installs, read both ways.
mod inputs;

use fromsoft_formats::{Archive, Game, dcx::Decoder};
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
    for (h, want) in hashes.iter().zip(&each) {
        let got = &scanned[h];
        assert_eq!(
            got.len(),
            1,
            "{game:?}: entry {h:08x} visited {} times",
            got.len()
        );
        if &got[0] != want {
            differ.push(format!("{h:08x}: scan {:?}, read_hash {want:?}", got[0]));
        }
    }
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
