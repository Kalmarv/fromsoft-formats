//! Every param of the installs' param binders (Sekiro's `gameparam.parambnd.dcx`, Elden Ring's `regulation.bin`) through
//! [`Param::read`], the grass params through [`GrassParam`], with the source selection census moved beside its registered pass.
//!
//! The counts are those of an independent census of the same files (a separate parser of the PARAM header and rows,
//! run over the installs' decoded binders, Elden Ring 1.17.3): a file ends with up to eight zero
//! bytes, no gap lies between its parts, and the only members refused are Sekiro's three params of the 32-bit form.
//!
//! Needs `installs.elden`, `keys.elden`, `installs.sekiro`, `keys.sekiro` and `tools.oodle6`; a test whose game is not
//! configured does nothing.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::param_binder;
use fromsoft_formats::param::{GrassParam, Param};
use fromsoft_formats::{Game, Install, Keys};
use inputs::{Game as Data, Tool, install, keys, oodle};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn configured(game: Game) -> Option<(Install, inputs::OodleLibrary)> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (root, folder, dll) = (install(data)?, keys(data)?, oodle(Tool::Oodle6)?);
    let keys = Keys::from_folder(folder, game, None).ok()?;
    Some((Install::new(root, game, keys), dll))
}

/// The members of the game's param binder that are params, by name, with `Param::read`'s verdict.
fn members(
    install: &Install,
    decoder: &Decoder<'_>,
) -> Result<Vec<(String, std::result::Result<Param, String>)>> {
    let archive = install.open()?;
    let binder = param_binder(install, &archive, decoder)?;
    Ok(binder
        .files
        .iter()
        .map(|f| {
            let name = f.name.clone().unwrap_or_default();
            (name, Param::read(&f.bytes).map_err(|e| e.to_string()))
        })
        .collect())
}

#[test]
fn every_param_of_the_installs_reads_but_the_32_bit_form() -> Result<()> {
    for (game, total, refused) in [
        (Game::EldenRing, 194, &[][..]),
        (
            Game::Sekiro,
            138,
            &[
                "default_AIStandardInfoBank",
                "default_EnemyBehaviorBank",
                "MenuColorTableParam",
            ][..],
        ),
    ] {
        let Some((install, dll)) = configured(game) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let members = members(&install, &decoder)?;
        assert_eq!(members.len(), total, "{} members", game.name());
        let mut refusals = Vec::new();
        let mut rows = 0;
        for (name, read) in &members {
            match read {
                Ok(param) => {
                    rows += param.rows.len();
                    // every row of a 0x85-form param shares one name offset: the empty string, or none has one
                    let named = param.rows.iter().filter(|r| r.name.is_some()).count();
                    assert!(named == 0 || named == param.rows.len(), "{name}");
                    assert!(
                        param
                            .rows
                            .iter()
                            .all(|r| r.name.as_deref().is_none_or(str::is_empty)),
                        "{name}: a non-empty row name"
                    );
                    assert!(param.unk06 <= 2, "{name}: unk06 {}", param.unk06);
                }
                Err(why) => {
                    assert!(why.contains("format flags"), "{name}: {why}");
                    refusals.push(
                        name.rsplit(['/', '\\'])
                            .next()
                            .unwrap()
                            .trim_end_matches(".param")
                            .to_owned(),
                    );
                }
            }
        }
        refusals.sort();
        let mut want: Vec<String> = refused.iter().map(|s| (*s).to_owned()).collect();
        want.sort();
        assert_eq!(refusals, want, "{}: the members refused", game.name());
        println!(
            "{}: {} of {} read, {rows} rows",
            game.name(),
            total - refusals.len(),
            total
        );
    }
    Ok(())
}

#[test]
fn the_grass_params_read_in_both_games() -> Result<()> {
    for (game, leaves, version, row_size, rows, model1_named) in [
        (Game::Sekiro, &["GrassTypeParam.param"][..], 6, 244, 166, false),
        (
            Game::EldenRing,
            &[
                "GrassTypeParam.param",
                "GrassTypeParam_Lv1.param",
                "GrassTypeParam_Lv2.param",
            ][..],
            1,
            276,
            632,
            true,
        ),
    ] {
        let Some((install, dll)) = configured(game) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let archive = install.open()?;
        let (mut count, mut negative, mut with_model1) = (0, 0, 0);
        for leaf in leaves {
            let grass = GrassParam::from_param(&read_param(&install, &archive, &decoder, leaf)?)?;
            assert_eq!(
                (grass.data_version, grass.row_size),
                (version, row_size),
                "{leaf}"
            );
            count += grass.rows.len();
            negative += grass.rows.iter().filter(|r| r.lod_range < 0).count();
            with_model1 += grass.rows.iter().filter(|r| !r.model1.is_empty()).count();
            // every Sekiro row names its model; some Elden Ring rows (row 2999) name none
            for r in grass.rows.iter().filter(|_| game == Game::Sekiro) {
                assert!(!r.model0.is_empty(), "{leaf} row {}: no model", r.id);
            }
        }
        assert_eq!(count, rows, "{}: grass rows", game.name());
        assert_eq!(
            with_model1 > 0,
            model1_named,
            "{}: second model names",
            game.name()
        );
        // five Sekiro rows hold lodRange -1 (65535 as an unsigned), no Elden Ring row is negative
        assert_eq!(
            negative,
            if game == Game::Sekiro { 5 } else { 0 },
            "{}: negative lodRange",
            game.name()
        );
    }
    Ok(())
}
