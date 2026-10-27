//! Every FMG of the installs' message binders (`/msg/<language>/*.msgbnd.dcx` that the dictionaries name and the archives
//! hold): 504 Sekiro and 3,968 Elden Ring members, all read by [`Fmg::read`] with its layout checks (ids ascending and
//! never repeated, the group table contiguous, the string count the groups' total, the size field the file's length).
//! The counts are those of an independent census of the same members (a separate parser of the FMG header, groups and
//! offset table): 381,243 and 7,526,881 offset slots, of which 114,867 and 5,481,630 hold no string (offset 0).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::fmg::Fmg;
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::BTreeSet;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn every_fmg_of_the_installs_reads() -> Result<()> {
    for (game, data, files, strings) in [
        (Game::Sekiro, Data::Sekiro, 504, 381_243 - 114_867),
        (Game::EldenRing, Data::Elden, 3_968, 7_526_881 - 5_481_630),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
        ) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
        let dictionary = NameDictionary::read_for(dictionary, game)?;
        let paths: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .map(String::as_str)
            .filter(|p| p.starts_with("/msg/") && p.ends_with(".msgbnd.dcx"))
            .collect();
        let (mut read, mut entries) = (0, 0);
        for path in paths.into_iter().filter(|p| archive.contains(p)) {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            for file in &binder.files {
                let fmg = Fmg::read(&decoder.decode_if_dcx(&file.bytes)?)?;
                read += 1;
                entries += fmg.entries.len();
            }
        }
        assert_eq!((read, entries), (files, strings), "{}", game.name());
    }
    Ok(())
}
