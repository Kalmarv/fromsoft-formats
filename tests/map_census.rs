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

/// Elden Ring's maps other than the open world (`m60`, `m61`) and the draw-param files they use: the files of the map's
/// own two-block stem where it has any, else those of its family's `mAA_00`, else none.
#[test]
fn a_dungeon_without_draw_param_files_of_its_own_uses_its_familys() -> Result<()> {
    let Some((archive, layouts)) = layouts(Game::EldenRing)? else {
        return Ok(());
    };
    let (mut own, mut family, mut none) = (
        BTreeMap::<&str, usize>::new(),
        BTreeMap::<&str, usize>::new(),
        BTreeMap::<&str, usize>::new(),
    );
    for map in layouts
        .keys()
        .filter(|m| !m.starts_with("m60") && !m.starts_with("m61"))
    {
        let stems = fromsoft_formats::area::draw_params(&archive, Game::EldenRing, map);
        let area = &map[..3];
        let has = |stem: &str| archive.contains(&fromsoft_formats::area::draw_param_path(stem));
        let own_stem = &map[..6];
        let has_own = (0..10_000).any(|id| has(&format!("{own_stem}_{id:04}")));
        let (bucket, stem_prefix) = if has_own {
            (&mut own, own_stem.to_owned())
        } else if (0..10_000).any(|id| has(&format!("{area}_00_{id:04}"))) {
            (&mut family, format!("{area}_00"))
        } else {
            (&mut none, String::new())
        };
        if stem_prefix.is_empty() {
            assert!(stems.is_empty(), "{map}");
        } else {
            assert!(
                !stems.is_empty() && stems.iter().all(|s| s.starts_with(&stem_prefix)),
                "{map}: {stems:?}"
            );
        }
        *bucket.entry(area).or_default() += 1;
    }
    // 58 maps have files of their own, 21 use the family's (20 of m30, one of m11) and the 27 of m31 and m32 have none
    let total = |m: &BTreeMap<&str, usize>| m.values().sum::<usize>();
    assert_eq!((total(&own), total(&family), total(&none)), (58, 21, 27));
    assert_eq!(family, BTreeMap::from([("m11", 1), ("m30", 20)]));
    assert_eq!(none, BTreeMap::from([("m31", 19), ("m32", 8)]));
    Ok(())
}

/// The draw and display blocks of every part, and the numbers regions and events store besides their record index,
/// against an independent census of the same files (a separate parser of the records, reading no output of this crate).
#[test]
fn draw_blocks_and_the_numbers_of_regions_and_events() -> Result<()> {
    // per kind: parts, parts with a draw block, with a display block, and with non-zero display, draw and collision-mask
    // words in the draw block, and with non-zero display words in the display block
    let expected = [
        (
            Game::Sekiro,
            vec![
                ("Collision", [1_304, 1_304, 1_304, 999, 1_185, 1_060, 11]),
                ("ConnectCollision", [582, 0, 582, 0, 0, 0, 73]),
                ("DummyEnemy", [206, 0, 0, 0, 0, 0, 0]),
                ("DummyObject", [125, 0, 0, 0, 0, 0, 0]),
                ("Enemy", [1_557, 1_557, 0, 1, 2, 5, 0]),
                ("MapPiece", [40_970, 40_970, 0, 28_522, 40_741, 40_879, 0]),
                ("Object", [10_538, 10_538, 0, 5_840, 10_113, 8_561, 0]),
                ("Player", [70, 0, 0, 0, 0, 0, 0]),
            ],
        ),
        (
            Game::EldenRing,
            vec![
                ("Asset", [421_253, 421_253, 421_253, 281_090, 106_461, 80_643, 3]),
                ("Collision", [3_266, 3_266, 3_266, 906, 1_053, 883, 341]),
                ("ConnectCollision", [800, 800, 800, 130, 110, 161, 322]),
                ("DummyAsset", [2_661, 2_661, 0, 53, 77, 1, 0]),
                ("DummyEnemy", [7_108, 7_108, 0, 3_028, 4, 1, 0]),
                ("Enemy", [16_776, 16_776, 0, 11_849, 1, 6, 0]),
                ("MapPiece", [4_522, 4_522, 0, 2_886, 1_444, 1_341, 0]),
                ("Player", [722, 722, 0, 203, 0, 3, 0]),
            ],
        ),
    ];
    let (mut regions, mut region_counter, mut events, mut event_counter) = (0, 0, 0, 0);
    let (mut event_is_index, mut event_ids_unique, mut event_ids_dense) = (0, 0, 0);
    let (mut numbers_unique, mut numbers_dense, mut files) = (0, 0, 0);
    let mut games_read = 0;
    for (game, table) in expected {
        let Some((_, layouts)) = layouts(game)? else {
            continue;
        };
        games_read += 1;
        let mut kinds = BTreeMap::<String, [usize; 7]>::new();
        let (mut tails, mut conditions) = (BTreeMap::<[u8; 4], usize>::new(), BTreeSet::new());
        let mut display_tails = BTreeMap::<[u8; 4], usize>::new();
        // collisions, collisions that set a display group, drawn parts, parts with no draw group, parts under a draw group
        // that one collision's display groups include, and parts under none that any does
        let mut census = [0usize; 6];
        for layout in layouts.values() {
            files += 1;
            let c = layout.group_census()?;
            for (total, n) in census.iter_mut().zip([
                c.collisions,
                c.collisions_with_groups,
                c.parts,
                c.no_groups,
                c.met,
                c.unmet,
            ]) {
                *total += n;
            }
            for p in &layout.parts {
                kinds.entry(format!("{:?}", p.kind)).or_default()[0] += 1;
            }
            for info in layout.draw_infos()? {
                let row = kinds
                    .entry(format!("{:?}", layout.parts[info.part].kind))
                    .or_default();
                if let Some(draw) = info.draw {
                    row[1] += 1;
                    row[3] += usize::from(draw.display_groups.iter().any(|&w| w != 0));
                    row[4] += usize::from(draw.draw_groups.iter().any(|&w| w != 0));
                    row[5] += usize::from(draw.collision_mask.iter().any(|&w| w != 0));
                    *tails.entry(draw.tail).or_default() += 1;
                    conditions.insert(draw.conditions);
                }
                if let Some(display) = info.display2 {
                    row[2] += 1;
                    row[6] += usize::from(display.display_groups.iter().any(|&w| w != 0));
                    *display_tails.entry(display.tail).or_default() += 1;
                }
            }
            let mut counters = BTreeMap::<u32, u32>::new();
            let list = layout.map_regions()?;
            for r in &list {
                let counter = counters.entry(r.type_code).or_default();
                region_counter += usize::from(r.index_in_type == *counter);
                *counter += 1;
            }
            regions += list.len();
            let mut numbers: Vec<i32> = list.iter().map(|r| r.number).collect();
            numbers.sort_unstable();
            numbers.dedup();
            numbers_unique += usize::from(numbers.len() == list.len());
            numbers_dense += usize::from(numbers.iter().copied().eq(0..list.len() as i32));
            let mut counters = BTreeMap::<u32, u32>::new();
            let list = layout.map_events()?;
            for e in &list {
                let counter = counters.entry(e.type_code).or_default();
                event_counter += usize::from(e.index_in_type == *counter);
                *counter += 1;
                event_is_index += usize::from(e.event_id as usize == e.index);
            }
            events += list.len();
            let mut ids: Vec<i32> = list.iter().map(|e| e.event_id).collect();
            ids.sort_unstable();
            ids.dedup();
            event_ids_unique += usize::from(ids.len() == list.len());
            event_ids_dense += usize::from(ids.iter().copied().eq(0..list.len() as i32));
        }
        assert_eq!(
            kinds.iter().map(|(k, v)| (k.as_str(), *v)).collect::<Vec<_>>(),
            table,
            "{}",
            game.name()
        );
        // against the independent census (a separate parser of the part records, the same union of the collisions' display words),
        // over the map pieces, objects and assets: Sekiro's 51,508 parts have 654 with no draw group, 49,433 that meet a
        // display group and 1,421 that meet none; Elden Ring's 425,775 have 317,870, 70,713 and 37,192
        let expected_census = if game == Game::Sekiro {
            [1_304, 999, 51_508, 654, 49_433, 1_421]
        } else {
            [3_266, 906, 425_775, 317_870, 70_713, 37_192]
        };
        assert_eq!(census, expected_census, "{} groups", game.name());
        // the draw block's fourth field is 0 in Sekiro; in Elden Ring -1 as a signed word, then a byte that is 1 on 177 map
        // pieces
        if game == Game::Sekiro {
            assert_eq!(tails, BTreeMap::from([([0, 0, 0, 0], 54_369)]));
            assert_eq!(conditions.len(), 4);
            // the display block's last four bytes: -1 as a signed word on all but the connect collisions that store a
            // number (12 in Sekiro's maps), whose first byte is then 1
            assert_eq!(display_tails.values().sum::<usize>(), 1_304 + 582);
            assert_eq!(
                display_tails
                    .iter()
                    .filter(|(t, _)| **t != [0, 0, 255, 255])
                    .map(|(_, n)| n)
                    .sum::<usize>(),
                12
            );
        } else {
            assert_eq!(
                tails.keys().copied().collect::<Vec<_>>(),
                [[255, 255, 0, 0], [255, 255, 1, 0]]
            );
            assert_eq!(tails[&[255, 255, 1, 0]], 177);
            assert_eq!(conditions.len(), 8);
            assert_eq!(display_tails.values().sum::<usize>(), 421_253 + 3_266 + 800);
            assert_eq!(
                display_tails
                    .iter()
                    .filter(|(t, _)| **t != [0, 0, 255, 255])
                    .map(|(_, n)| n)
                    .sum::<usize>(),
                3
            );
        }
    }
    // the totals below are over both games: only when both installs are configured
    if games_read < 2 {
        return Ok(());
    }
    // 945 maps: the region index within the type is the per-type counter on all 45,121 regions; the number at +44 is
    // unique in every map and is 0 to n-1 in 419; the event number at +8 equals the record index on 367 of 9,211 events,
    // is unique in every map and dense in 731; the event's index within its type is the counter on all but the two events of
    // type 0xffffffff
    assert_eq!((files, regions, region_counter), (945, 45_121, 45_121));
    assert_eq!((numbers_unique, numbers_dense), (945, 419));
    assert_eq!((events, event_counter, event_is_index), (9_211, 9_209, 367));
    assert_eq!((event_ids_unique, event_ids_dense), (945, 731));
    Ok(())
}
