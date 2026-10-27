//! The player's animation clips against its event files. The player's `c0000.anibnd.dcx` holds one event file per animation
//! group (`a50.tae`, numbered by its binder slot); the clips are in it and in its `c0000_*.anibnd.dcx` siblings, and a clip `aGGG_NNNNNN` is the record numbered `GGG * 1,000,000 +
//! NNNNNN` of the file of its group ([`Tae::read_member`], [`tae::clip_animation_id`], [`tae::resolve`]). An independent
//! census of the same binders (a separate parser of the TAE header, groups and animation table, searching each clip's
//! number in the file of its group) finds a record for 1,434 of Sekiro's 1,434 distinct player clips and 9,217 of Elden
//! Ring's 9,342; of the other 125, 89 have their record only in another group's file and 36 in no file.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::{self, Tae};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::BTreeSet;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The file name after the last separator, lowercased, without its extension.
fn stem(name: &str) -> String {
    fromsoft_formats::names::stem(name)
}

#[test]
fn the_players_clips_find_their_records_by_binder_slot() -> Result<()> {
    for (game, data, clips, found) in [
        (Game::Sekiro, Data::Sekiro, 1_434, 1_434),
        (Game::EldenRing, Data::Elden, 9_342, 9_217),
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
            .filter(|p| p.starts_with("/chr/c0000") && p.ends_with(".anibnd.dcx"))
            .filter(|p| archive.contains(p))
            .collect();
        let (mut files, mut names) = (Vec::new(), BTreeSet::<String>::new());
        for path in &paths {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            for member in &binder.files {
                let Some(name) = member.name.as_deref().map(str::to_ascii_lowercase) else {
                    continue;
                };
                if name.ends_with(".tae") {
                    files.push(Tae::read_member(&binder, member)?);
                } else if name.ends_with(".hkx") {
                    // a clip `aGGG_NNNNNN`; the variants `aGGG_NNNNNN_N` are other files
                    let stem = stem(&name);
                    if stem.len() == 11 && tae::clip_animation_id(&stem).is_some() {
                        names.insert(stem);
                    }
                }
            }
        }
        let located = names
            .iter()
            .filter(|s| {
                let number = tae::clip_animation_id(s).unwrap();
                tae::resolve(&files, number).unwrap().is_some()
            })
            .count();
        println!(
            "{}: {} event files, {} clips, {located} with a record",
            game.name(),
            files.len(),
            names.len()
        );
        assert_eq!((names.len(), located), (clips, found), "{}", game.name());
    }
    Ok(())
}
