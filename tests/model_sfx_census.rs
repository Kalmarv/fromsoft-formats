//! The references from map data to effects that a placed light can come through, over every map layout of both installs
//! (Sekiro's 10, Elden Ring's 935) and the model effect params: SFX regions (`MapLayout::sfx_region`), an Elden Ring
//! asset's `sfxParamRelativeId` (`Part::sfx_param_relative_id`) and the rows of Sekiro's `ModelSfxParam` and Elden
//! Ring's `AssetModelSfxParam` (`ModelSfxParam`).
//!
//! The totals asserted are those of an independent census (a Python walk of the decoded layouts and of the params typed
//! by the Smithbox paramdefs, reading no output of this crate): 1,872 and 1,523 SFX regions, and for Elden Ring's
//! 421,253 asset parts of `AEGnnn_mmm` models 409,246 with relative id -1, 11,533 whose `(nnn * 1000 + mmm) * 1000 + id`
//! row the param holds and 474 whose row it lacks. A discriminating scan of every 16-bit word of the asset type data
//! found only the word at +0x1c to be -1 or a row's id in that way (4,100 positive values that name a row, 16 that do
//! not, 409,246 of -1; the next best word names a row for 966 parts and misses 69,758). The rows are also compared
//! field by field with the paramdef reading of the same params.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt` and
//! `oracles.smithbox.{sekiro,elden}/defs`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::{param_binder, read_model_sfx_param};
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::paramdef::ParamDefs;
use fromsoft_formats::{Game, Install, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::BTreeSet;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn defs(dir: &std::path::Path) -> Result<ParamDefs> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();
    let texts = files
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(ParamDefs::from_xml(texts.iter().map(String::as_str))?)
}

/// `AEGnnn_mmm` as the row key's base `nnn * 1000 + mmm`.
fn asset_model_key(name: &str) -> Option<i32> {
    let (n, m) = name.strip_prefix("AEG")?.split_once('_')?;
    (n.len() == 3 && m.len() == 3).then_some(())?;
    Some(n.parse::<i32>().ok()? * 1000 + m.parse::<i32>().ok()?)
}

#[test]
fn effect_references_of_both_installs() -> Result<()> {
    // (game, data set, SFX regions, maps holding one, start disabled, second byte set, param rows)
    for (game, data, regions, region_maps, disabled, second, rows) in [
        (Game::Sekiro, Data::Sekiro, 1_872, 9, 54, 13, 202),
        (Game::EldenRing, Data::Elden, 1_523, 104, 17, 0, 1_293),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(defs_dir)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
            oracle(OracleSet::Smithbox, data, "defs"),
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
        // the params, by the crate's reader and by the paramdef reading
        let param = read_model_sfx_param(&install, &archive, &decoder)?;
        assert_eq!(param.rows.len(), rows, "{} rows", game.name());
        let binder = param_binder(&install, &archive, &decoder)?;
        let leaf = match game {
            Game::Sekiro => "ModelSfxParam.param",
            Game::EldenRing => "AssetModelSfxParam.param",
        };
        let stored = fromsoft_formats::param::Param::read(&binder.member(leaf).unwrap().bytes)?;
        let defs = defs(&defs_dir)?;
        let typed = defs.bind(&stored, binder.regulation_number())?;
        let mut slots = 0;
        assert_eq!(typed.rows().count(), param.rows.len());
        for (row, t) in param.rows.iter().zip(typed.rows()) {
            assert_eq!(t.id, row.id);
            for (k, slot) in row.slots.iter().enumerate() {
                assert_eq!(
                    t.get(&format!("sfxId_{k}")).and_then(|v| v.as_i64()),
                    Some(i64::from(slot.effect_id)),
                    "{} row {} sfxId_{k}",
                    game.name(),
                    row.id
                );
                assert_eq!(
                    t.get(&format!("dmypolyId_{k}")).and_then(|v| v.as_i64()),
                    Some(i64::from(slot.dummy_id)),
                    "{} row {} dmypolyId_{k}",
                    game.name(),
                    row.id
                );
            }
            slots += row.used().count();
        }
        let ids: BTreeSet<i32> = param.rows.iter().map(|r| r.id).collect();
        let repeated = param.rows.len() - ids.len();
        println!(
            "{}: {} rows, {slots} used slots, {repeated} repeated ids",
            game.name(),
            param.rows.len()
        );
        assert_eq!(repeated, usize::from(game == Game::Sekiro));

        // the layouts
        let dictionary = NameDictionary::read_for(dictionary, game)?;
        let maps: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
            .filter(|m| !m.contains('/'))
            .collect();
        let (mut sfx, mut sfx_maps, mut off, mut second_set) = (0, 0, 0, 0);
        let (mut aeg, mut none, mut named, mut absent, mut others) = (0, 0, 0, 0, 0);
        for map in maps {
            let path = format!("/map/mapstudio/{map}.msb.dcx");
            if !archive.contains(&path) {
                continue;
            }
            let layout = MapLayout::read(&archive.read(&path, &decoder)?)?;
            let mut here = 0;
            for region in layout.map_regions()? {
                match layout.sfx_region(&region)? {
                    Some(s) => {
                        assert_eq!(region.type_code, 5);
                        here += 1;
                        off += usize::from(s.start_disabled);
                        second_set += usize::from(s.second_byte != 0);
                    }
                    None => assert_ne!(region.type_code, 5),
                }
            }
            sfx += here;
            sfx_maps += usize::from(here > 0);
            for part in &layout.parts {
                let rel = part.sfx_param_relative_id;
                match (game, part.kind) {
                    (Game::EldenRing, PartKind::Asset) => {
                        let rel = rel.expect("an Elden Ring asset states one");
                        let model = part.model.map(|i| layout.models[i].name.as_str());
                        let Some(key) = model.and_then(asset_model_key) else {
                            others += 1;
                            continue;
                        };
                        aeg += 1;
                        if rel < 0 {
                            none += 1;
                        } else if param.rows_with(key * 1000 + i32::from(rel)).next().is_some() {
                            named += 1;
                        } else {
                            absent += 1;
                        }
                    }
                    _ => assert_eq!(rel, None, "{} {:?}", game.name(), part.kind),
                }
            }
        }
        println!(
            "{}: {sfx} SFX regions in {sfx_maps} maps, {off} start disabled, {second_set} with the second byte; \
             AEG assets {aeg}: {none} without, {named} with a row, {absent} naming an absent row; {others} other asset models",
            game.name()
        );
        assert_eq!(
            (sfx, sfx_maps, off, second_set),
            (regions, region_maps, disabled, second)
        );
        if game == Game::EldenRing {
            assert_eq!((aeg, none, named, absent), (421_253, 409_246, 11_533, 474));
        }
    }
    Ok(())
}
