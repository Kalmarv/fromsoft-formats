//! Every event file (`.tae`) of the animation binders of the installs (`/chr/*.anibnd.dcx` and the animation binders inside
//! the object, part and asset binders), through [`Tae`]:
//! the header constants, the animation groups table and the times tables are checked as the files are read, and the
//! event groups are counted against an independent census of the same files (a separate parser of the header, the
//! animation table and the group arrays, reading no output of this crate), which found 326 distinct event files in
//! Sekiro's install and 1,071 in Elden Ring's.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::Tae;
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn every_event_file_of_the_animation_binders_reads_with_its_groups() -> Result<()> {
    // per game: distinct event files, their animations, event groups, empty event groups, animations with event groups
    for (game, data, expected) in [
        (Game::Sekiro, Data::Sekiro, [326, 15_769, 247_075, 2_509, 12_347]),
        (
            Game::EldenRing,
            Data::Elden,
            [1_071, 51_151, 943_080, 49_359, 45_101],
        ),
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
            .filter(|p| {
                [".anibnd.dcx", ".objbnd.dcx", ".partsbnd.dcx", ".geombnd.dcx"]
                    .iter()
                    .any(|suffix| p.ends_with(suffix))
            })
            .filter(|p| archive.contains(p))
            .collect();
        let mut distinct = BTreeSet::new();
        let (mut members, mut animations, mut groups, mut empty, mut with_groups) = (0, 0, 0, 0, 0);
        for path in &paths {
            let mut todo = vec![Binder::read(&archive.read(path, &decoder)?, &decoder)?];
            while let Some(binder) = todo.pop() {
                for member in &binder.files {
                    if member.bytes.starts_with(b"BND4") {
                        todo.push(Binder::read(&member.bytes, &decoder)?);
                    }
                    let named_tae = member
                        .name
                        .as_deref()
                        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".tae"));
                    if !named_tae {
                        continue;
                    }
                    members += 1;
                    let tae = Tae::read_member(&binder, member)?;
                    if !distinct.insert(Sha256::digest(&member.bytes).to_vec()) {
                        continue;
                    }
                    animations += tae.animations.len();
                    for animation in &tae.animations {
                        with_groups += usize::from(!animation.event_groups.is_empty());
                        groups += animation.event_groups.len();
                        empty += animation
                            .event_groups
                            .iter()
                            .filter(|g| g.events.is_empty())
                            .count();
                    }
                }
            }
        }
        println!(
            "{}: {} binders, {members} event files, {} distinct, {animations} animations, {groups} event groups of which {empty} empty, \
             {with_groups} animations with event groups",
            game.name(),
            paths.len(),
            distinct.len()
        );
        assert_eq!(
            [distinct.len(), animations, groups, empty, with_groups],
            expected,
            "{}",
            game.name()
        );
    }
    Ok(())
}
