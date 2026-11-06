//! Every map layout the dictionaries name that the installs hold (Sekiro's 10, Elden Ring's 1,347 with Shadow of the
//! Erdtree's archives, 935 without), through [`MapLayout`]: the totals the strict reader returns, then the fields read
//! from the layouts' records against an independent census of the same files (a separate parser of the MSB sections and
//! records, run over the decoded layouts, reading no output of this crate). The census was made on the base game (its
//! numbers in parentheses below); with the expansion installed the Elden Ring numbers asserted are this test's own
//! (with the expansion's archives left out it gives the census's).
//!
//! - Every model states how many parts name it (7,242 Sekiro and 75,291 (53,503) Elden Ring models), which
//!   `MapLayout::read` checks.
//! - Elden Ring's regions and events end with the extra block: 50,608 (36,409) regions and 10,286 (7,585) events read,
//!   769 (625) regions and 142 (131) events name a map, and the map words of the extra blocks and of the parts'
//!   tile-load configs (101,517 (77,782) of them) name the maps of the install except the 91 that name two overworld
//!   tiles it does not hold.
//! - The alternate overworld tiles (`_10`) have the alternate parents (`_11`, `_12`) in the install, whose layouts
//!   place 3,578 of their far assets.
//! - The entity block's draw controls and the collision parts' draw-param selectors.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and
//! `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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
    // every layout the archives hold, read on the archive's threads
    let maps: Vec<&str> = maps
        .into_iter()
        .filter(|map| archive.contains(&format!("/map/mapstudio/{map}.msb.dcx")))
        .collect();
    let read = archive.read_each(
        maps.iter().map(|map| format!("/map/mapstudio/{map}.msb.dcx")),
        &decoder,
        inputs::threads(),
        |_, bytes| MapLayout::read(bytes?),
    )?;
    let mut out = BTreeMap::new();
    for (map, layout) in maps.into_iter().zip(read) {
        out.insert(map.to_owned(), layout?);
    }
    Ok(Some((archive, out)))
}

#[test]
fn every_layout_of_the_installs_reads_with_its_totals() -> Result<()> {
    for (game, maps, models, parts, regions, events) in [
        (Game::Sekiro, 10, 7_242, 55_352, 8_712, 1_626),
        (Game::EldenRing, 1_347, 75_291, 676_654, 50_608, 10_286),
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
            assert_eq!((region_maps, event_maps), (769, 142));
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
    // 769 + 142 extra-block words and 100,606 tile-load words: all but 91 of the 101,517 name a map of the install
    assert_eq!(tile_loads, 100_606);
    assert_eq!(
        absent.iter().map(String::as_str).collect::<Vec<_>>(),
        ["m60_42_40_10", "m60_47_42_10"]
    );
    assert_eq!(named, 101_517 - 91);
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
        assert_eq!(used, 18_887);
        assert_eq!(ids_without_flag, 1_205);
        assert_eq!((reflection, above, point, rtao), (196, 240, 97, 6_676));
        assert_eq!(collisions, 4_737);
        // the byte at +0x35 of a collision's type data is 255 on 4,351 of the 4,737; the scene config's byte at +32 takes
        // the file ids of the overworld draw-param files; on 382 of the 386 overrides it restates the override (1, 2 and 3
        // as 10, 20 and 30)
        assert_eq!(overrides, 386);
        assert_eq!(restated, 382);
        assert_eq!(sub_ids, 534);
        assert_eq!(
            sub_values.into_iter().collect::<Vec<_>>(),
            [0, 10, 20, 21, 30, 31, 40, 41, 42, 50, 55, 60, 65, 70, 75, 78]
        );
    }
    Ok(())
}

/// Elden Ring's maps other than the open world (`m60`, `m61`) and the draw-param files they use: the files of the map's
/// own two-block stem where it has any, else those of its family's `mAA_00`, else (the caves and tunnels, `m31` and `m32`)
/// the catacombs' `m30_00`, as the game's `GparamRefSettingsParam` says, else none.
#[test]
fn a_dungeon_without_draw_param_files_of_its_own_uses_its_familys() -> Result<()> {
    let Some((archive, layouts)) = layouts(Game::EldenRing)? else {
        return Ok(());
    };
    let (mut own, mut family, mut referenced, mut none) = (
        BTreeMap::<&str, usize>::new(),
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
        } else if matches!(area, "m31" | "m32") && (0..10_000).any(|id| has(&format!("m30_00_{id:04}"))) {
            (&mut referenced, "m30_00".to_owned())
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
    // 72 maps have files of their own, 33 use the family's (20 of m30, 4 of m21, 3 of m42, 2 each of m40 and m41, one each
    // of m11 and m43) and the 27 of m31 and m32 use m30_00's
    let total = |m: &BTreeMap<&str, usize>| m.values().sum::<usize>();
    assert_eq!(
        (total(&own), total(&family), total(&referenced), total(&none)),
        (72, 33, 27, 0)
    );
    assert_eq!(
        family,
        BTreeMap::from([
            ("m11", 1),
            ("m21", 4),
            ("m30", 20),
            ("m40", 2),
            ("m41", 2),
            ("m42", 3),
            ("m43", 1)
        ])
    );
    assert_eq!(referenced, BTreeMap::from([("m31", 19), ("m32", 8)]));
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
                ("Asset", [624_987, 624_987, 624_987, 437_542, 137_708, 108_377, 3]),
                ("Collision", [4_737, 4_737, 4_737, 1_108, 1_286, 1_096, 400]),
                ("ConnectCollision", [1_125, 1_125, 1_125, 171, 149, 302, 492]),
                ("DummyAsset", [3_971, 3_971, 0, 53, 149, 1, 0]),
                ("DummyEnemy", [9_453, 9_453, 0, 4_370, 4, 1, 0]),
                ("Enemy", [25_399, 25_399, 0, 19_274, 1, 6, 0]),
                ("MapPiece", [6_051, 6_051, 0, 3_896, 1_509, 1_402, 0]),
                ("Player", [931, 931, 0, 309, 0, 3, 0]),
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
        // parts under no draw group, those with display groups of their own, those whose display groups a collision displays
        let mut zero = [0usize; 3];
        for (name, layout) in &layouts {
            files += 1;
            let z = layout.zero_draw_census()?;
            for (total, n) in zero.iter_mut().zip([z.parts, z.with_display, z.display_met]) {
                *total += n;
            }
            if game == Game::EldenRing && name == "m10_00_00_00" {
                // the layout the running-game picture with draw groups zero and display groups set was made from
                assert_eq!(
                    (z.parts, z.with_display, z.display_met),
                    (292, 171, 171),
                    "{name}"
                );
            }
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
        // display group and 1,421 that meet none; Elden Ring's 631,038 have 491,821, 91,860 and 47,357
        let expected_census = if game == Game::Sekiro {
            [1_304, 999, 51_508, 654, 49_433, 1_421]
        } else {
            [4_737, 1_108, 631_038, 491_821, 91_860, 47_357]
        };
        assert_eq!(census, expected_census, "{} groups", game.name());
        // the same independent census, over the parts under no draw group: Sekiro 654 (271 with display groups of their own,
        // 230 of those meeting the collisions' display groups); Elden Ring 491,821 (379,757; 14,443)
        let expected_zero = if game == Game::Sekiro {
            [654, 271, 230]
        } else {
            [491_821, 379_757, 14_443]
        };
        assert_eq!(zero, expected_zero, "{} zero draw groups", game.name());
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
            assert_eq!(display_tails.values().sum::<usize>(), 624_987 + 4_737 + 1_125);
            assert_eq!(
                display_tails
                    .iter()
                    .filter(|(t, _)| **t != [0, 0, 255, 255])
                    .map(|(_, n)| n)
                    .sum::<usize>(),
                4
            );
        }
    }
    // the totals below are over both games: only when both installs are configured
    if games_read < 2 {
        return Ok(());
    }
    // 1,357 maps: the region index within the type is the per-type counter on all 59,320 regions; the number at +44 is
    // unique in every map and is 0 to n-1 in 533; the event number at +8 equals the record index on 481 of 11,912 events,
    // is unique in every map and dense in 1,090; the event's index within its type is the counter on 11,905 of them (in
    // the base game's 9,211 on all but the two events of type 0xffffffff)
    assert_eq!((files, regions, region_counter), (1_357, 59_320, 59_320));
    assert_eq!((numbers_unique, numbers_dense), (1_357, 533));
    assert_eq!((events, event_counter, event_is_index), (11_912, 11_905, 481));
    assert_eq!((event_ids_unique, event_ids_dense), (1_357, 1_090));
    Ok(())
}
