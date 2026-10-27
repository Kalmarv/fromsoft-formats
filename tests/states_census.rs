//! Every event script of the installs (`/event/*.emevd.dcx` that the dictionaries name and the archives hold: Sekiro's 30
//! and Elden Ring's 589, the base game) through [`Script::read`] and [`Rules::read`]. An independent census of the same
//! scripts (a separate parser of the EMEVD header, events, instructions and parameters) finds the `2000:6` calls: Sekiro's
//! 2,138 name the function first, of which 142 are the part-state functions 20004110 (50), 20004112 (57) and 20004114
//! (35), none undecidable; Elden Ring's 10,480 name a slot (below 100) first and the function second.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::emevd::Script;
use fromsoft_formats::states::Rules;
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn the_event_scripts_name_their_common_functions_where_the_game_puts_them() -> Result<()> {
    for (game, data, scripts) in [(Game::Sekiro, Data::Sekiro, 30), (Game::EldenRing, Data::Elden, 589)] {
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
            .filter(|p| p.starts_with("/event/") && p.ends_with(".emevd.dcx") && !p[7..].contains('/'))
            .collect();
        let (mut read, mut decided, mut undecidable) = (0, BTreeMap::<i32, usize>::new(), 0);
        let mut other = BTreeMap::<i32, usize>::new();
        for path in paths.into_iter().filter(|p| archive.contains(p)) {
            let rules = Rules::read(&Script::read(&archive.read(path, &decoder)?)?, game);
            read += 1;
            undecidable += rules.undecidable_calls;
            for (function, n) in rules.decided {
                *decided.entry(function).or_default() += n;
            }
            for (function, n) in rules.other_calls {
                *other.entry(function).or_default() += n;
            }
        }
        assert_eq!(read, scripts, "{} scripts", game.name());
        let calls = |m: &BTreeMap<i32, usize>| m.values().sum::<usize>();
        match game {
            Game::Sekiro => {
                assert_eq!(decided, BTreeMap::from([(20004110, 50), (20004112, 57), (20004114, 35)]));
                assert_eq!(undecidable, 0);
                // the other calls of the 2,138: the function numbers are 7 or 8 digits
                assert_eq!(calls(&decided) + undecidable + calls(&other), 2_138);
                assert!(other.keys().all(|&f| f >= 1_000_000), "{:?}", other.keys().next());
            }
            Game::EldenRing => {
                assert!(decided.is_empty());
                assert_eq!(calls(&other), 10_480);
                // read with the slot as the function, 2000:6's first word would be below 100 in every call
                let slots = other.range(..100).map(|(_, n)| n).sum::<usize>();
                assert!(slots * 10 < calls(&other), "{slots} calls name a function below 100");
            }
        }
    }
    Ok(())
}
