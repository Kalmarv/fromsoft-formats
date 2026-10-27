//! Every map layout the dictionaries name that the installs hold (Sekiro's 10, Elden Ring's 935: the base game, no
//! expansion archive), through [`MapLayout`]: the totals the strict reader returns, then the fields read from the layouts'
//! records against an independent census of the same files (a separate parser of the MSB sections and records, run
//! over the decoded layouts, reading no output of this crate).
//!
//! - Every model states how many parts name it (7,242 Sekiro and 53,503 Elden Ring models), which `MapLayout::read`
//!   checks.
//! - Elden Ring's regions and events end with the extra block: 36,409 regions and 7,585 events read, 625 regions and
//!   131 events name a map, and the map words of the extra blocks and of the parts' tile-load configs (77,782 of them)
//!   name the maps of the install except the 91 that name two overworld tiles it does not hold.
//! - The alternate overworld tiles (`_10`) have the alternate parents (`_11`, `_12`) in the install, whose layouts place
//!   3,578 of their far assets.
//! - The entity block's draw controls and the collision parts' draw-param selectors.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The layouts of every map the game's dictionary names and its archive holds, by map name.
fn layouts(game: Game) -> Result<Option<(Archive, BTreeMap<String, MapLayout>)>> {
    let (data, dll) = match game {
        Game::Sekiro => (Data::Sekiro, oodle(Tool::Oodle6)),
        Game::EldenRing => (Data::Elden, oodle(Tool::Oodle6)),
    };
    let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
        install(data),
        keys(data),
        dll,
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
    let dictionary = NameDictionary::read_for(dictionary, game)?;
    let maps: BTreeSet<&str> = dictionary
        .names
        .values()
        .flatten()
        .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
        .filter(|m| !m.contains('/'))
        .collect();
    let mut out = BTreeMap::new();
    for map in maps {
        let path = format!("/map/mapstudio/{map}.msb.dcx");
        if archive.contains(&path) {
            out.insert(map.to_owned(), MapLayout::read(&archive.read(&path, &decoder)?)?);
        }
    }
    Ok(Some((archive, out)))
}

#[test]
fn every_layout_of_the_installs_reads_with_its_totals() -> Result<()> {
    for (game, maps, models, parts, regions, events) in [
        (Game::Sekiro, 10, 7_242, 55_352, 8_712, 1_626),
        (Game::EldenRing, 935, 53_503, 457_108, 36_409, 7_585),
    ] {
        let Some((_, layouts)) = layouts(game)? else {
            continue;
        };
        assert_eq!(layouts.len(), maps, "{} maps", game.name());
        let (mut m, mut p, mut r, mut e) = (0, 0, 0, 0);
        let (mut region_maps, mut event_maps, mut region_extra_typed) = (0, 0, 0);
        for layout in layouts.values() {
            m += layout.models.len();
            p += layout.parts.len();
            // each model's stated count is the number of parts that name it (checked on reading); say it again here
            let mut named = vec![0u32; layout.models.len()];
            for part in &layout.parts {
                if let Some(i) = part.model {
                    named[i] += 1;
                }
            }
            assert!(
                layout
                    .models
                    .iter()
                    .zip(&named)
                    .all(|(model, n)| model.instances == *n)
            );
            let regions = layout.map_regions()?;
            let events = layout.map_events()?;
            r += regions.len();
            e += events.len();
            region_maps += regions.iter().filter(|x| x.extra_map.is_some()).count();
            event_maps += events.iter().filter(|x| x.extra_map.is_some()).count();
            region_extra_typed += regions.iter().filter(|x| x.type_name.is_some()).count();
        }
        assert_eq!((m, p, r, e), (models, parts, regions, events), "{}", game.name());
        if game == Game::EldenRing {
            assert_eq!((region_maps, event_maps), (625, 131));
        } else {
            assert_eq!((region_maps, event_maps), (0, 0));
        }
        assert!(region_extra_typed > 0);
    }
    Ok(())
}

#[test]
fn map_words_name_the_installed_maps() -> Result<()> {
    let Some((_, layouts)) = layouts(Game::EldenRing)? else {
        return Ok(());
    };
    let (mut named, mut absent, mut tile_loads) = (0, BTreeSet::new(), 0);
    let mut ids = Vec::new();
    for layout in layouts.values() {
        ids.extend(layout.map_regions()?.iter().filter_map(|r| r.extra_map));
        ids.extend(layout.map_events()?.iter().filter_map(|e| e.extra_map));
        let tiles: Vec<_> = layout.parts.iter().filter_map(|p| p.tile_load).collect();
        tile_loads += tiles.len();
        ids.extend(tiles);
    }
    for id in ids {
        let name = id.name().unwrap_or_else(|| panic!("{id} is no map name"));
        if layouts.contains_key(&name) {
            named += 1;
        } else {
            absent.insert(name);
        }
    }
    // 625 + 131 extra-block words and 77,026 tile-load words: all but 91 of the 77,782 name a map of the install
    assert_eq!(tile_loads, 77_026);
    assert_eq!(
        absent.iter().map(String::as_str).collect::<Vec<_>>(),
        ["m60_42_40_10", "m60_47_42_10"]
    );
    assert_eq!(named, 77_782 - 91);
    Ok(())
}

#[test]
fn alternate_overworld_tiles_have_alternate_parents_that_place_their_far_assets() -> Result<()> {
    let Some((_, layouts)) = layouts(Game::EldenRing)? else {
        return Ok(());
    };
    let alternates: Vec<&String> = layouts
        .keys()
        .filter(|m| m.starts_with("m60_") && m.ends_with("_10"))
        .collect();
    assert_eq!(alternates.len(), 32);
    let (mut placed, mut by_level) = (0, [0usize; 2]);
    for tile in alternates {
        let parents = Game::EldenRing.parent_maps(tile);
        assert_eq!(parents.len(), 2, "{tile}");
        for (level, (parent, _)) in parents.iter().enumerate() {
            let layout = layouts
                .get(parent)
                .unwrap_or_else(|| panic!("{tile}: no parent {parent}"));
            let prefix = format!("{tile}-");
            let n = layout
                .parts
                .iter()
                .filter(|p| p.name.starts_with(&prefix))
                .count();
            placed += n;
            by_level[level] += n;
        }
    }
    // `_11` places 1,190 and `_12` 2,388 of the alternate tiles' far assets (85 more name two tiles the install lacks)
    assert_eq!((placed, by_level), (3_578, [1_190, 2_388]));
    // the base tiles' parents are unchanged: every `_00` tile has both
    for tile in layouts
        .keys()
        .filter(|m| m.starts_with("m60_") && m.ends_with("_00") && m.len() == 12)
    {
        let parents = Game::EldenRing.parent_maps(tile);
        assert_eq!(parents.len(), 2);
        assert!(parents.iter().all(|(p, _)| layouts.contains_key(p)), "{tile}");
    }
    Ok(())
}

#[test]
fn the_entity_blocks_draw_controls_and_the_collisions_draw_param_selectors() -> Result<()> {
    if let Some((_, layouts)) = layouts(Game::Sekiro)? {
        let mut lod = BTreeMap::<u8, usize>::new();
        let mut kinds = BTreeMap::<&str, usize>::new();
        for layout in layouts.values() {
            for p in &layout.parts {
                if let Some(id) = p.draw.lod_param_id {
                    *lod.entry(id).or_default() += 1;
                    *kinds.entry(p.kind.word()).or_default() += 1;
                }
                assert!(!p.draw.use_parts_draw_param && p.draw.parts_draw_param_id == 0);
            }
        }
        // 1,616 map pieces and 12 objects name a LodParam row: 3, 31, 40, 41, 42, 60, 61 and 63
        assert_eq!(kinds, BTreeMap::from([("map_piece", 1_616), ("object", 12)]));
        assert_eq!(
            lod.keys().copied().collect::<Vec<_>>(),
            [3, 31, 40, 41, 42, 60, 61, 63]
        );
    }
    if let Some((_, layouts)) = layouts(Game::EldenRing)? {
        let (mut used, mut ids_without_flag, mut reflection, mut above, mut point, mut rtao) =
            (0, 0, 0, 0, 0, 0);
        let (mut collisions, mut overrides, mut sub_ids, mut restated) = (0, 0, 0, 0);
        let mut sub_values = BTreeSet::new();
        for layout in layouts.values() {
            let gparam = layout.gparam_configs()?;
            for p in &layout.parts {
                assert_eq!(p.draw.lod_param_id, None);
                used += usize::from(p.draw.use_parts_draw_param);
                ids_without_flag +=
                    usize::from(!p.draw.use_parts_draw_param && p.draw.parts_draw_param_id != 0);
                reflection += usize::from(p.draw.draw_only_in_reflection);
                above += usize::from(p.draw.enable_on_above_shadow);
                point += usize::from(p.draw.disable_point_light_effect);
                rtao += usize::from(p.draw.disable_rtao);
            }
            for g in &gparam {
                let part = &layout.parts[g.part];
                if part.kind == PartKind::Collision {
                    collisions += 1;
                    overrides += usize::from(g.gparam_override.is_some());
                    sub_ids += usize::from(g.scene_sub_id.is_some());
                    sub_values.extend(g.scene_sub_id);
                    // the override 1, 2, 3 is the scene byte 10, 20, 30; 0, 21, 31, 41 are the same byte
                    restated += usize::from(match (g.gparam_override, g.scene_sub_id) {
                        (Some(o), Some(s)) => s == o || (1..=3).contains(&o) && s == o * 10,
                        _ => false,
                    });
                } else {
                    assert!(g.gparam_override.is_none());
                }
            }
        }
        assert_eq!(used, 11_583);
        assert_eq!(ids_without_flag, 439);
        assert_eq!((reflection, above, point, rtao), (156, 198, 97, 6_655));
        assert_eq!(collisions, 3_266);
        // the byte at +0x35 of a collision's type data is 255 on 3,225 of the 3,266; the scene config's byte at +32 takes
        // the file ids of the overworld draw-param files; on 40 of the 41 overrides it restates the override (1, 2 and 3
        // as 10, 20 and 30)
        assert_eq!(overrides, 41);
        assert_eq!(restated, 40);
        assert_eq!(sub_ids, 128);
        assert_eq!(
            sub_values.into_iter().collect::<Vec<_>>(),
            [0, 10, 20, 21, 30, 31, 40, 41, 42]
        );
    }
    Ok(())
}
