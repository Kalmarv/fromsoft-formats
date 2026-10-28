//! The params that decide which draw-param file Elden Ring uses ([`fromsoft_formats::gparam_params`]) against the install.
//! The expected values come from an independent census (a separate Python parser of the regulation's param rows with the
//! Smithbox definitions, and of the archive's file names, run for this test):
//!
//! - `WeatherParam`: 162 rows naming 82 distinct `GparamId`s; every one has a `m00_00_<id>_weatheroutdoor` file in the
//!   archive, which holds 84 such files (the two no row names are 98 and 150).
//! - `WeatherLotParam`: 164 rows with 272 weighted entries, every one naming a `WeatherParam` row: 19 distinct rows, reaching
//!   14 distinct file ids.
//! - `GparamRefSettingsParam`: 70 rows keyed by a map's decimal number, 2 of them with no target (-1); of the other 68, 56
//!   name a target whose files the archive holds and for these the files [`area::draw_params`] gives the map are the
//!   target's own stem's (the rule `area` uses agrees with the game's table); 12 name an expansion map's family.
//!
//! Needs `installs.elden`, `keys.elden`, `tools.oodle6`; skipped otherwise.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::area;
use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::read_param;
use fromsoft_formats::gparam_params::{
    GparamRefSettings, WeatherLotParam, WeatherParam, weather_outdoor_path,
};
use fromsoft_formats::{Game, Install, Keys};
use inputs::{Game as Data, Tool, install, keys, oodle};
use std::collections::BTreeSet;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn the_weather_and_reference_params_name_files_the_install_holds() -> Result<()> {
    let (Some(root), Some(folder), Some(dll)) =
        (install(Data::Elden), keys(Data::Elden), oodle(Tool::Oodle6))
    else {
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

    let weather = WeatherParam::from_param(&read_param(&install, &archive, &decoder, "WeatherParam.param")?)?;
    assert_eq!(weather.rows.len(), 162);
    let ids: BTreeSet<u32> = weather.rows.iter().map(|r| r.gparam_id).collect();
    assert_eq!(ids.len(), 82);
    assert!(weather.rows.iter().all(|r| archive.contains(&r.gparam_path())));
    let files: BTreeSet<u32> = (0..10_000)
        .filter(|&id| archive.contains(&weather_outdoor_path(id)))
        .collect();
    assert_eq!(files.len(), 84);
    assert_eq!(files.difference(&ids).copied().collect::<Vec<_>>(), [98, 150]);

    let lots = WeatherLotParam::from_param(&read_param(
        &install,
        &archive,
        &decoder,
        "WeatherLotParam.param",
    )?)?;
    assert_eq!(lots.rows.len(), 164);
    let (mut entries, mut reached) = (0usize, BTreeSet::new());
    for lot in &lots.rows {
        for entry in lot.weighted() {
            entries += 1;
            let row = weather.row(i32::from(entry.weather)).unwrap_or_else(|| {
                panic!("lot {} names weather {}, which has no row", lot.id, entry.weather)
            });
            reached.insert(row.id);
        }
    }
    assert_eq!((entries, reached.len()), (272, 19));
    let reached_files: BTreeSet<u32> = reached
        .iter()
        .filter_map(|&id| weather.row(id))
        .map(|r| r.gparam_id)
        .collect();
    assert_eq!(reached_files.len(), 14);

    let refs = GparamRefSettings::from_param(&read_param(
        &install,
        &archive,
        &decoder,
        "GparamRefSettings.param",
    )?)?;
    assert_eq!(refs.rows.len(), 70);
    let (mut agree, mut expansion, mut none) = (0, 0, 0);
    for row in &refs.rows {
        let map = row.map().and_then(|m| m.name()).expect("a map's decimal number");
        let Some(target) = row.target_map().and_then(|t| t.name()) else {
            none += 1;
            continue;
        };
        let stems = area::draw_params(&archive, Game::EldenRing, &map);
        if stems.is_empty() {
            // an expansion map: the base archives hold none of its files nor its family's
            expansion += 1;
            continue;
        }
        assert!(
            stems.iter().all(|s| s.starts_with(&target[..6])),
            "{map} -> {target}: {stems:?}"
        );
        agree += 1;
    }
    assert_eq!((agree, expansion, none), (56, 12, 2));
    Ok(())
}
