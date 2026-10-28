//! Every instruction of every event script of the installs (`/event/*.emevd.dcx` that the dictionaries name and the
//! archives hold: Sekiro's 30 and Elden Ring's 589, the base game) through [`emedf::decode_event`], against the schema in
//! `src/emedf`. The expected values come from an independent census of the same scripts (a separate Python reader of the
//! EMEVD header, events, instructions and parameters, and of the EMEDF files themselves), run for this test:
//!
//! - Sekiro: 42,885 instructions of 258 distinct bank:number pairs, all typed; Elden Ring: 107,639 of 316, 107,635 typed,
//!   the other 4 are `11:3`, which the community schema does not have (refused as "not in the schema").
//! - The layout is right wherever a parameter shows it: each of the 6,920 (Sekiro) and 34,483 (Elden Ring) parameters
//!   that fill an instruction's arguments with an event argument lands on exactly one argument of the typed instruction
//!   (same offset, same size).
//! - The roles: flags, entities, event ids and map parts counted by whether a parameter fills them. The map arguments
//!   name maps of the install: Elden Ring's decimal map numbers 64 of 65 (the other is the literal 16), its byte quads 133
//!   of 210 without a wildcard (73 of the others are the all-zero quad, 4 are `m12_00`, which has no files), Sekiro's
//!   (area, block) pairs 277 of 283. The initialisers name events some script of the install defines (Sekiro 4,350 of
//!   4,520 calls, Elden Ring 16,593 of 16,606).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::emedf::{self, Role};
use fromsoft_formats::emevd::Script;
use fromsoft_formats::eventflag::{self, Scope};
use fromsoft_formats::mapid::MapId;
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// What the scripts of one game came to.
#[derive(Default)]
struct Tally {
    scripts: usize,
    events: usize,
    instructions: usize,
    typed: usize,
    distinct: BTreeSet<(i32, i32)>,
    distinct_typed: BTreeSet<(i32, i32)>,
    refused: BTreeMap<&'static str, usize>,
    refused_instructions: BTreeSet<(i32, i32)>,
    banks: BTreeMap<i32, usize>,
    literal: BTreeMap<String, usize>,
    substituted: BTreeMap<String, usize>,
    parameters: usize,
    exact: usize,
    maps: BTreeMap<&'static str, usize>,
    started: BTreeMap<(i32, i32, bool), usize>,
    flags: BTreeMap<(&'static str, &'static str), usize>,
}

fn count(m: &mut BTreeMap<String, usize>, role: Role) {
    *m.entry(format!("{role:?}")).or_default() += 1;
}

fn named(m: &BTreeMap<String, usize>, role: &str) -> usize {
    m.get(role).copied().unwrap_or(0)
}

#[test]
fn every_stock_instruction_is_typed_where_the_schema_has_it() -> Result<()> {
    for (game, data, scripts) in [
        (Game::Sekiro, Data::Sekiro, 30),
        (Game::EldenRing, Data::Elden, 589),
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
        let held = |p: &&str| archive.contains(p);
        let event_paths: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .map(String::as_str)
            .filter(|p| p.starts_with("/event/") && p.ends_with(".emevd.dcx") && !p[7..].contains('/'))
            .filter(held)
            .collect();
        // the maps of the install: those with a layout or an event script
        let mut maps: BTreeSet<String> = BTreeSet::new();
        for path in dictionary
            .names
            .values()
            .flatten()
            .map(String::as_str)
            .filter(held)
        {
            let stem = path
                .strip_prefix("/map/mapstudio/")
                .or_else(|| path.strip_prefix("/event/"))
                .and_then(|p| {
                    p.strip_suffix(".msb.dcx")
                        .or_else(|| p.strip_suffix(".emevd.dcx"))
                });
            if let Some(stem) = stem
                && stem.len() == 12
                && MapId::from_name(stem).is_some()
            {
                maps.insert(stem.to_owned());
            }
        }
        let mut loaded = Vec::new();
        for path in &event_paths {
            loaded.push((*path, Script::read(&archive.read(path, &decoder)?)?));
        }
        let defined: BTreeSet<i64> = loaded.iter().flat_map(|(_, s)| &s.events).map(|e| e.id).collect();
        let mut t = Tally {
            scripts: loaded.len(),
            ..Tally::default()
        };
        for (path, script) in &loaded {
            // the map an event script belongs to (`/event/m10_00_00_00.emevd.dcx`), none for the common scripts
            let own = path
                .strip_prefix("/event/")
                .and_then(|p| p.strip_suffix(".emevd.dcx"))
                .and_then(MapId::from_name);
            let kind = match own {
                None => "common",
                Some(id) if id.area == 60 || id.area == 61 => "tile",
                Some(_) => "map",
            };
            for event in &script.events {
                t.events += 1;
                let results = emedf::decode_event(game, event);
                for (instruction, result) in event.instructions.iter().zip(&results) {
                    t.instructions += 1;
                    t.distinct.insert((instruction.bank, instruction.id));
                    match result {
                        Ok(typed) => {
                            t.typed += 1;
                            t.distinct_typed.insert((instruction.bank, instruction.id));
                            *t.banks.entry(instruction.bank).or_default() += 1;
                            for arg in &typed.args {
                                if arg.substituted {
                                    count(&mut t.substituted, arg.def.role);
                                } else {
                                    count(&mut t.literal, arg.def.role);
                                }
                                if arg.substituted || !matches!(arg.def.role, Role::Flag | Role::FlagBase) {
                                    continue;
                                }
                                // a flag number against the script's own map; zero and -1 name no flag
                                let Some(flag) = arg.value.as_word().filter(|&f| f != 0 && f != u32::MAX)
                                else {
                                    continue;
                                };
                                let outcome = match (eventflag::scope(game, flag), own) {
                                    (None, _) => "not a flag",
                                    (Some(Scope::Global), _) => "global",
                                    (Some(Scope::Map(_)), None) => "mapped",
                                    (Some(Scope::Map(id)), Some(own)) if id.covers(own) => "own",
                                    (Some(Scope::Map(_)), Some(_)) => "other",
                                };
                                *t.flags.entry((kind, outcome)).or_default() += 1;
                            }
                            if let Some(init) = typed.initialisation() {
                                let known = defined.contains(&i64::from(init.event));
                                *t.started
                                    .entry((instruction.bank, instruction.id, known))
                                    .or_default() += 1;
                            } else if matches!((instruction.bank, instruction.id), (2000, 0) | (2000, 6)) {
                                panic!("an initialiser with a substituted event id");
                            }
                            *t.maps.entry(classify_map(typed, &maps)).or_default() += 1;
                        }
                        Err(refusal) => {
                            *t.refused.entry(refusal.reason()).or_default() += 1;
                            t.refused_instructions.insert((instruction.bank, instruction.id));
                        }
                    }
                }
                for usage in emedf::argument_uses(game, event) {
                    t.parameters += 1;
                    t.exact += usize::from(usage.target.is_some());
                }
            }
        }
        t.maps.remove("none");
        assert_eq!(t.scripts, scripts, "{} scripts", game.name());
        match game {
            Game::Sekiro => {
                assert_eq!((t.events, t.instructions, t.typed), (1_703, 42_885, 42_885));
                assert_eq!((t.distinct.len(), t.distinct_typed.len()), (258, 258));
                assert!(t.refused.is_empty());
                assert_eq!(t.parameters, 6_920);
                assert_eq!(t.exact, 6_920);
                assert_eq!(
                    t.banks,
                    BTreeMap::from([
                        (0, 2_254),
                        (1, 129),
                        (3, 4_255),
                        (4, 2_604),
                        (5, 91),
                        (11, 182),
                        (12, 50),
                        (13, 54),
                        (1000, 3_780),
                        (1001, 1_179),
                        (1003, 2_936),
                        (1005, 6),
                        (1014, 1_434),
                        (2000, 4_609),
                        (2002, 65),
                        (2003, 4_358),
                        (2004, 10_540),
                        (2005, 810),
                        (2006, 1_758),
                        (2007, 261),
                        (2008, 163),
                        (2009, 71),
                        (2010, 216),
                        (2011, 450),
                        (2012, 630),
                    ])
                );
                for (role, literal, substituted) in [
                    ("Entity", 16_393, 5_091),
                    ("EventId", 4_520, 0),
                    ("EventSlot", 2_382, 0),
                    ("Flag", 8_279, 606),
                    ("FlagBase", 43, 0),
                    ("FlagBitCount", 43, 0),
                    ("MapArea", 283, 10),
                    ("MapBlock", 283, 10),
                    ("MapWord", 0, 0),
                ] {
                    assert_eq!(named(&t.literal, role), literal, "literal {role}");
                    assert_eq!(named(&t.substituted, role), substituted, "substituted {role}");
                }
                // a started event is one some script of the install defines, for 4,350 of 4,520 calls
                assert_eq!(
                    t.started,
                    BTreeMap::from([
                        ((2000, 0, false), 170),
                        ((2000, 0, true), 2_212),
                        ((2000, 6, true), 2_138)
                    ])
                );
                // a flag number against the script it is in: of the map scripts' 5,353 literal flags 3,145 name no map, 2,088
                // their own map's block and 120 another's
                assert_eq!(
                    t.flags,
                    BTreeMap::from([
                        (("common", "global"), 2_518),
                        (("common", "mapped"), 223),
                        (("map", "global"), 3_145),
                        (("map", "other"), 120),
                        (("map", "own"), 2_088),
                    ])
                );
                assert_eq!(
                    t.maps,
                    BTreeMap::from([
                        ("(area, block) names an installed map family", 277),
                        ("(area, block) names none", 6),
                        ("byte map substituted", 10),
                    ])
                );
            }
            Game::EldenRing => {
                assert_eq!((t.events, t.instructions, t.typed), (4_908, 107_639, 107_635));
                assert_eq!((t.distinct.len(), t.distinct_typed.len()), (316, 315));
                assert_eq!(t.refused, BTreeMap::from([("not in the schema", 4)]));
                assert_eq!(t.refused_instructions, BTreeSet::from([(11, 3)]));
                assert_eq!(t.parameters, 34_483);
                assert_eq!(t.exact, 34_483);
                assert_eq!(
                    t.banks,
                    BTreeMap::from([
                        (0, 7_075),
                        (1, 165),
                        (3, 12_168),
                        (4, 9_704),
                        (5, 182),
                        (11, 8),
                        (1000, 11_079),
                        (1001, 3_580),
                        (1003, 9_616),
                        (1004, 622),
                        (1005, 17),
                        (1014, 4_241),
                        (2000, 17_341),
                        (2001, 18),
                        (2002, 145),
                        (2003, 13_209),
                        (2004, 12_308),
                        (2005, 2_877),
                        (2006, 1_629),
                        (2007, 359),
                        (2008, 115),
                        (2009, 808),
                        (2010, 289),
                        (2011, 47),
                        (2012, 33),
                    ])
                );
                for (role, literal, substituted) in [
                    ("Entity", 18_768, 22_602),
                    ("EventId", 16_606, 0),
                    ("EventSlot", 16_606, 0),
                    ("Flag", 18_440, 6_648),
                    ("FlagBase", 166, 129),
                    ("FlagBitCount", 295, 0),
                    ("MapArea", 260, 21),
                    ("MapBlock", 260, 21),
                    ("MapRegion", 253, 21),
                    ("MapIndex", 253, 21),
                    ("MapWord", 65, 8),
                ] {
                    assert_eq!(named(&t.literal, role), literal, "literal {role}");
                    assert_eq!(named(&t.substituted, role), substituted, "substituted {role}");
                }
                assert_eq!(
                    t.started,
                    BTreeMap::from([
                        ((2000, 0, false), 9),
                        ((2000, 0, true), 6_117),
                        ((2000, 6, false), 4),
                        ((2000, 6, true), 10_476),
                    ])
                );
                // of the flags that name a map in a dungeon's script, 3,616 of 3,775 are the dungeon's own block's; in a tile's
                // script 2,686 of 3,212 are the tile's own
                assert_eq!(
                    t.flags,
                    BTreeMap::from([
                        (("common", "global"), 3_831),
                        (("common", "mapped"), 1_407),
                        (("map", "global"), 2_196),
                        (("map", "other"), 159),
                        (("map", "own"), 3_616),
                        (("tile", "global"), 3_391),
                        (("tile", "other"), 526),
                        (("tile", "own"), 2_686),
                    ])
                );
                assert_eq!(
                    t.maps,
                    BTreeMap::from([
                        ("(area, block) names an installed map family", 7),
                        ("byte map names an installed map", 133),
                        ("byte map names no installed map", 77),
                        ("byte map substituted", 21),
                        ("byte map with a wildcard", 43),
                        ("map word names an installed map", 64),
                        ("map word names no installed map", 1),
                        ("map word substituted", 8),
                    ])
                );
            }
        }
    }
    Ok(())
}

/// Which of the census's map classes an instruction falls in (`"none"`: it names no map).
fn classify_map(typed: &emedf::Typed, maps: &BTreeSet<String>) -> &'static str {
    let word: Vec<_> = typed.with_role(Role::MapWord).collect();
    if let Some(arg) = word.first() {
        if arg.substituted {
            return "map word substituted";
        }
        let named = arg
            .value
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .and_then(MapId::from_decimal)
            .and_then(MapId::name)
            .is_some_and(|n| maps.contains(&n));
        return if named {
            "map word names an installed map"
        } else {
            "map word names no installed map"
        };
    }
    let parts: Vec<_> = [Role::MapArea, Role::MapBlock, Role::MapRegion, Role::MapIndex]
        .into_iter()
        .filter_map(|role| typed.with_role(role).next())
        .collect();
    let has = |role: Role| typed.with_role(role).next().is_some();
    if !(has(Role::MapArea) && has(Role::MapBlock)) {
        return "none";
    }
    if parts.iter().any(|a| a.substituted) {
        return "byte map substituted";
    }
    let Some(id) = typed.map() else {
        return "none";
    };
    if id.has_wildcard() {
        return "byte map with a wildcard";
    }
    if !has(Role::MapRegion) {
        let family = format!("m{:02}_{:02}_", id.area, id.x);
        return if maps.iter().any(|m| m.starts_with(&family)) {
            "(area, block) names an installed map family"
        } else {
            "(area, block) names none"
        };
    }
    if id.name().is_some_and(|n| maps.contains(&n)) {
        "byte map names an installed map"
    } else {
        "byte map names no installed map"
    }
}
