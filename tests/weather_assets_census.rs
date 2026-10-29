//! Elden Ring's weather asset replace param (`WeatherAssetParam`) and the maps' weather asset creation limits
//! (`WeatherAssetLimits`), over the install: every row read, compared field by field with the paramdef reading of the
//! same params, and every asset a row names found among the models of its sky map's layout, for each sky map the
//! archives hold.
//!
//! The totals asserted are those of an independent census (a Python walk of the params typed by the Smithbox paramdefs
//! and of the decoded layouts, reading no output of this crate): 1,007 rows naming 27 sky maps, of which the archives
//! hold the 18 listed below; `isFireAsh` 0 in 412 rows, 1 in 382 and 2 in 213 (the later addition's maps only); 403
//! rows with a creation limit; 1,599 asset slots in use; no two rows of one map, weather, fire-ash state and limits;
//! every named asset placed by its sky map's layout. `MapDefaultInfoParam`: 875 rows, by creation limit -1: 114, 0: 69,
//! 1: 125, 2: 71, 3: 72, 4: 198, 21: 8, 31: 10, 41: 19, 42: 8, 50: 145, 60: 26, 65: 1, 70: 8, 75: 1.
//!
//! Needs `installs.elden`, `keys.elden`, `tools.oodle6` and `oracles.smithbox.elden/defs`; skipped when not configured.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::{param_binder, read_weather_asset_limits, read_weather_assets};
use fromsoft_formats::msb::MapLayout;
use fromsoft_formats::param::{Param, asset_model_name};
use fromsoft_formats::paramdef::ParamDefs;
use fromsoft_formats::{Game, Install, Keys};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The sky maps the base game's archives hold a layout for.
const SKY_MAPS: [&str; 18] = [
    "m10_00_00_99",
    "m11_00_00_99",
    "m11_05_00_99",
    "m11_10_00_99",
    "m11_71_00_99",
    "m12_03_00_99",
    "m13_00_00_99",
    "m14_00_00_99",
    "m15_00_00_99",
    "m16_00_00_99",
    "m34_10_00_99",
    "m34_11_00_99",
    "m34_12_00_99",
    "m34_13_00_99",
    "m34_14_00_99",
    "m34_15_00_99",
    "m39_20_00_99",
    "m60_00_00_99",
];

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

#[test]
fn weather_assets_and_limits_of_the_install() -> Result<()> {
    let (Some(root), Some(folder), Some(dll), Some(defs_dir)) = (
        install(Data::Elden),
        keys(Data::Elden),
        oodle(Tool::Oodle6),
        oracle(OracleSet::Smithbox, Data::Elden, "defs"),
    ) else {
        return Ok(());
    };
    let install = Install::new(
        root,
        Game::EldenRing,
        Keys::from_folder(folder, Game::EldenRing, None)?,
    );
    let archive = install.open()?;
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let param = read_weather_assets(&install, &archive, &decoder)?;
    let limits = read_weather_asset_limits(&install, &archive, &decoder)?;

    // the rows against the paramdef reading
    let binder = param_binder(&install, &archive, &decoder)?;
    let defs = defs(&defs_dir)?;
    let stored = Param::read(&binder.member("WeatherAssetReplaceParam.param").unwrap().bytes)?;
    let typed = defs.bind(&stored, binder.regulation_number())?;
    assert_eq!(typed.rows().count(), param.rows.len());
    let int = |t: &fromsoft_formats::paramdef::TypedRow<'_>, name: &str| {
        t.get(name).and_then(|v| v.as_i64()).unwrap()
    };
    for (row, t) in param.rows.iter().zip(typed.rows()) {
        assert_eq!(t.id, row.id);
        assert_eq!(int(&t, "mapId"), i64::from(row.map), "row {}", row.id);
        assert_eq!(
            int(&t, "TransitionSrcWeather"),
            i64::from(row.weather),
            "row {}",
            row.id
        );
        assert_eq!(int(&t, "isFireAsh"), i64::from(row.fire_ash), "row {}", row.id);
        let assets: Vec<i64> = (0..8)
            .map(|k| int(&t, &format!("AssetId{k}")))
            .filter(|&a| a > 0)
            .collect();
        assert_eq!(
            assets,
            row.assets.iter().map(|&a| i64::from(a)).collect::<Vec<_>>(),
            "row {}",
            row.id
        );
        let set: Vec<i64> = (0..4)
            .map(|k| int(&t, &format!("CreateAssetLimitId{k}")))
            .filter(|&l| l != -1)
            .collect();
        assert_eq!(
            set,
            row.limits.iter().map(|&l| i64::from(l)).collect::<Vec<_>>(),
            "row {}",
            row.id
        );
    }
    let stored = Param::read(&binder.member("MapDefaultInfoParam.param").unwrap().bytes)?;
    let typed = defs.bind(&stored, binder.regulation_number())?;
    assert_eq!(typed.rows().count(), limits.maps.len());
    for t in typed.rows() {
        assert_eq!(
            Some(int(&t, "WeatherCreateAssetLimitId")),
            limits.of(t.id).map(i64::from),
            "map {}",
            t.id
        );
    }

    // the totals
    let mut fire_ash = BTreeMap::<u8, usize>::new();
    let mut keys = BTreeSet::new();
    let (mut limited, mut slots) = (0, 0);
    let mut by_map = BTreeMap::<String, BTreeSet<String>>::new();
    for r in &param.rows {
        *fire_ash.entry(r.fire_ash).or_default() += 1;
        limited += usize::from(!r.limits.is_empty());
        slots += r.assets.len();
        assert!(
            keys.insert((r.map, r.weather, r.fire_ash, r.limits.clone())),
            "row {}",
            r.id
        );
        if r.fire_ash == 2 {
            assert!(
                ["m2", "m61"].iter().any(|p| r.map_name().starts_with(p)),
                "row {}",
                r.id
            );
        }
        by_map
            .entry(r.map_name())
            .or_default()
            .extend(r.assets.iter().map(|&a| asset_model_name(a)));
    }
    assert_eq!(param.rows.len(), 1_007);
    assert_eq!(fire_ash, BTreeMap::from([(0, 412), (1, 382), (2, 213)]));
    assert_eq!((limited, slots, by_map.len()), (403, 1_599, 27));
    let mut per_limit = BTreeMap::<i8, usize>::new();
    for &l in limits.maps.values() {
        *per_limit.entry(l).or_default() += 1;
    }
    assert_eq!(
        per_limit,
        BTreeMap::from([
            (-1, 114),
            (0, 69),
            (1, 125),
            (2, 71),
            (3, 72),
            (4, 198),
            (21, 8),
            (31, 10),
            (41, 19),
            (42, 8),
            (50, 145),
            (60, 26),
            (65, 1),
            (70, 8),
            (75, 1),
        ])
    );

    // every named asset is placed by its sky map's layout
    let mut held = Vec::new();
    for (map, assets) in &by_map {
        let path = format!("/map/mapstudio/{map}.msb.dcx");
        if !archive.contains(&path) {
            continue;
        }
        held.push(map.as_str());
        let layout = MapLayout::read(&archive.read(&path, &decoder)?)?;
        let placed: BTreeSet<String> = layout
            .parts
            .iter()
            .filter_map(|p| p.model.map(|i| layout.models[i].name.to_ascii_uppercase()))
            .collect();
        let missing: Vec<_> = assets.iter().filter(|a| !placed.contains(*a)).collect();
        assert!(missing.is_empty(), "{map}: {missing:?}");
    }
    assert_eq!(held, SKY_MAPS);
    Ok(())
}
