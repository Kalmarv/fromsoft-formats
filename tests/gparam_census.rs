//! How a draw-param file (the graphics environment of a map) is named and switched, against the installs.
//!
//! - A collision part's scene config names a file: the first scene byte is the hundreds digit of the file's id and, in
//!   Elden Ring, the byte at +32 its last two digits ([`PartGparam::draw_param_id`]). Of Sekiro's 1,304 collisions every
//!   one names a file the install holds; of Elden Ring's 3,266, 3,019 do, 37 name a file their map's family lacks and 210
//!   stand in the 27 maps (m31, m32) that have no draw-param file at all. (An independent census of the same layouts and the
//!   file names, a separate parser of the MSB records, gives the same three numbers.)
//! - The events that switch a file ([`gparam_events`]): Sekiro's 43 sub id calls name 0, 10, 20, 30, 40 and 50, and 35 name
//!   an area that has a file of exactly that number; Elden Ring's 11 override activations name 0, 1, 2, 3, 4 and 500, of
//!   the 10 that belong to a map 6 name an existing file, and 14 deactivations.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::area::{self, DrawParamId};
use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::emevd::Script;
use fromsoft_formats::gparam_events::{self, Switch};
use fromsoft_formats::mapid::MapId;
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use primase_foundations::compress::oodle::Oodle;
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The archive of `game`, the Oodle library its files need and its path dictionary, when all are configured.
fn open(game: Game, data: Data) -> Result<Option<(Archive, Oodle, NameDictionary)>> {
    let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
        install(data),
        keys(data),
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
    Ok(Some((archive, dll, NameDictionary::read_for(dictionary, game)?)))
}

/// The ids of the draw-param files of the area's map, cached by the part of the name that decides them.
fn files<'a>(
    cache: &'a mut BTreeMap<String, Vec<DrawParamId>>,
    archive: &Archive,
    game: Game,
    map: &str,
) -> &'a Vec<DrawParamId> {
    let key = if map.starts_with("m60") || map.starts_with("m61") {
        map[..3].to_owned()
    } else {
        map[..6].to_owned()
    };
    cache.entry(key).or_insert_with(|| {
        area::draw_params(archive, game, map)
            .iter()
            .filter_map(|s| DrawParamId::from_stem(s))
            .map(|(_, id)| id)
            .collect()
    })
}

#[test]
fn a_collisions_scene_config_names_a_draw_param_file_that_exists() -> Result<()> {
    for (game, data, expected) in [
        // (names an existing file, names one the map's files lack, stands in a map with no file)
        (Game::Sekiro, Data::Sekiro, [1_304, 0, 0]),
        (Game::EldenRing, Data::Elden, [3_019, 37, 210]),
    ] {
        let Some((archive, dll, dictionary)) = open(game, data)? else {
            continue;
        };
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let maps: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
            .filter(|m| !m.contains('/'))
            .collect();
        let mut cache = BTreeMap::new();
        let (mut exists, mut absent, mut no_files) = (0, 0, 0);
        for map in maps {
            let path = format!("/map/mapstudio/{map}.msb.dcx");
            if !archive.contains(&path) {
                continue;
            }
            let layout = MapLayout::read(&archive.read(&path, &decoder)?)?;
            let ids = files(&mut cache, &archive, game, map);
            for config in layout.gparam_configs()? {
                if layout.parts[config.part].kind != PartKind::Collision {
                    continue;
                }
                let id = config
                    .draw_param_id()
                    .expect("every collision stores a scene config");
                if ids.is_empty() {
                    no_files += 1;
                } else if ids.contains(&id) {
                    exists += 1;
                } else {
                    absent += 1;
                }
            }
        }
        assert_eq!([exists, absent, no_files], expected, "{}", game.name());
    }
    Ok(())
}

#[test]
fn the_events_that_switch_a_draw_param_file_name_the_numbers_the_files_have() -> Result<()> {
    for (game, data, scripts) in [
        (Game::Sekiro, Data::Sekiro, 30),
        (Game::EldenRing, Data::Elden, 589),
    ] {
        let Some((archive, dll, dictionary)) = open(game, data)? else {
            continue;
        };
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let paths: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .map(String::as_str)
            .filter(|p| p.starts_with("/event/") && p.ends_with(".emevd.dcx") && !p[7..].contains('/'))
            .filter(|p| archive.contains(p))
            .collect();
        assert_eq!(paths.len(), scripts, "{} scripts", game.name());
        let mut cache = BTreeMap::new();
        let mut subs = BTreeSet::new();
        // calls, calls that name a file that exists, calls that name none, calls of a script that belongs to no map
        let (mut calls, mut named, mut unnamed, mut common) = (0, 0, 0, 0);
        let mut clears = 0;
        for path in paths {
            let script = Script::read(&archive.read(path, &decoder)?)?;
            let own = path
                .strip_prefix("/event/")
                .and_then(|p| p.strip_suffix(".emevd.dcx"))
                .and_then(MapId::from_name)
                .and_then(MapId::name);
            for call in gparam_events::calls(game, &script) {
                match call.switch {
                    Switch::AreaSubId {
                        area: Some(area),
                        block: Some(block),
                        sub: Some(sub),
                        ..
                    } => {
                        calls += 1;
                        subs.insert(i64::from(sub));
                        let map = format!("m{area:02}_{block:02}_00_00");
                        let ids = files(&mut cache, &archive, game, &map);
                        if ids.iter().any(|id| id.number() == u16::from(sub)) {
                            named += 1;
                        } else {
                            unnamed += 1;
                        }
                    }
                    Switch::Override { id: Some(id), .. } => {
                        calls += 1;
                        subs.insert(i64::from(id));
                        let Some(map) = &own else {
                            common += 1;
                            continue;
                        };
                        let ids = files(&mut cache, &archive, game, map);
                        if ids.iter().any(|i| i64::from(i.number()) == i64::from(id)) {
                            named += 1;
                        } else {
                            unnamed += 1;
                        }
                    }
                    Switch::ClearOverride { .. } => clears += 1,
                    other => panic!("a call with a parameter-filled field in a stock script: {other:?}"),
                }
            }
        }
        match game {
            Game::Sekiro => {
                assert_eq!((calls, named, unnamed, common, clears), (43, 35, 8, 0, 0));
                assert_eq!(subs, BTreeSet::from([0, 10, 20, 30, 40, 50]));
            }
            Game::EldenRing => {
                assert_eq!((calls, named, unnamed, common, clears), (11, 6, 4, 1, 14));
                assert_eq!(subs, BTreeSet::from([0, 1, 2, 3, 4, 500]));
            }
        }
    }
    Ok(())
}
