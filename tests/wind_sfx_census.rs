//! The wind SFX regions (`MapLayout::wind_sfx_region`) of every map layout of both installs (Sekiro's 10, Elden Ring's
//! 935): each reads, and what its region index names.
//!
//! The totals asserted are those of an independent census (a Python walk of the decompressed layouts that reads the
//! region records' type data itself and no output of this crate): Elden Ring 72 wind SFX regions in 11 maps, whose
//! index names the region itself in 66, a wind area (type 18) in 5 and none (-1) in 1, the effects 808003 (66),
//! 808001 (4), 808006 (1) and 800803 (1); Sekiro 106 in 9 maps, a wind area in 86, none in 15, the region itself in 5.
//! Every float after the index is -1.0.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::msb::MapLayout;
use fromsoft_formats::{Game, Install, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn wind_sfx_regions_of_both_installs() -> Result<()> {
    // (game, data set, regions, maps holding one, naming itself, a wind area, none, effects)
    let elden_effects = [(800803, 1), (808001, 4), (808003, 66), (808006, 1)];
    for (game, data, regions, region_maps, itself, area, none, effects) in [
        (Game::Sekiro, Data::Sekiro, 106, 9, 5, 86, 15, None),
        (
            Game::EldenRing,
            Data::Elden,
            72,
            11,
            66,
            5,
            1,
            Some(&elden_effects[..]),
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
        let install = Install::new(root, game, Keys::from_folder(folder, game, None)?);
        let archive = install.open()?;
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let dictionary = NameDictionary::read_for(dictionary, game)?;
        let maps: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
            .filter(|m| !m.contains('/'))
            .collect();
        let (mut count, mut with, mut own, mut to_area, mut unnamed) = (0, 0, 0, 0, 0);
        let mut by_effect = BTreeMap::new();
        for map in maps {
            let path = format!("/map/mapstudio/{map}.msb.dcx");
            if !archive.contains(&path) {
                continue;
            }
            let layout = MapLayout::read(&archive.read(&path, &decoder)?)?;
            let all = layout.map_regions()?;
            let mut here = 0;
            for region in &all {
                let Some(wind) = layout.wind_sfx_region(region)? else {
                    assert_ne!(region.type_code, 6);
                    continue;
                };
                here += 1;
                assert_eq!(wind.unknown_float, -1.0, "{map} {}", region.name);
                *by_effect.entry(wind.effect_id).or_insert(0) += 1;
                match wind.wind_area {
                    None => unnamed += 1,
                    Some(i) if i == region.index => own += 1,
                    Some(i) => {
                        assert_eq!(
                            all[i].type_code, 18,
                            "{map} {} names {}",
                            region.name, all[i].name
                        );
                        to_area += 1;
                    }
                }
            }
            count += here;
            with += usize::from(here > 0);
        }
        println!(
            "{}: {count} wind SFX regions in {with} maps; index names itself {own}, a wind area {to_area}, none \
             {unnamed}; effects {by_effect:?}",
            game.name()
        );
        assert_eq!(
            (count, with, own, to_area, unnamed),
            (regions, region_maps, itself, area, none),
            "{}",
            game.name()
        );
        if let Some(effects) = effects {
            assert_eq!(by_effect.into_iter().collect::<Vec<_>>(), effects);
        }
    }
    Ok(())
}
