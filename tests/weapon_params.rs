//! The public row decoder retains the counts established by the installed weapon/hold census.
mod common;
mod inputs;

use fromsoft_formats::game_params::read_param;
use fromsoft_formats::weapon_params::layouts;
use fromsoft_formats::{Game, Install, Keys, dcx::Decoder};
use inputs::{Game as Data, Tool, install, keys, oodle};

#[test]
fn every_installed_weapon_and_hold_row_keeps_the_established_field_counts()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(library) = oodle(Tool::Oodle6) else {
        return Ok(());
    };
    let codec = common::oodle(library)?;
    let decoder = Decoder {
        oodle: Some(&codec),
        max_output: 1 << 30,
    };
    for (game, data, expected) in [
        (Game::Sekiro, Data::Sekiro, (173, 66, 38, 33, 38)),
        (Game::EldenRing, Data::Elden, (3636, 3533, 3052, 70, 776)),
    ] {
        let (Some(root), Some(folder)) = (install(data), keys(data)) else {
            continue;
        };
        let installed = Install::new(root, game, Keys::from_folder(folder, game, None)?);
        let archive = installed.open()?;
        let weapons = read_param(&installed, &archive, &decoder, "EquipParamWeapon.param")?;
        let holds = read_param(&installed, &archive, &decoder, "WepAbsorpPosParam.param")?;
        let (weapon_layout, hold_layout) = layouts(game);
        let decoded = weapons
            .rows
            .iter()
            .map(|r| weapon_layout.read(r))
            .collect::<fromsoft_formats::Result<Vec<_>>>()?;
        let models = decoded.iter().filter(|r| r.model > 0).count();
        let traces = decoded
            .iter()
            .filter(|r| r.traces.iter().any(|t| t.effect > 0))
            .count();
        let mut dummies = 0;
        for row in &holds.rows {
            for h in hold_layout.read(row)? {
                dummies += [h.right, h.left, h.both, h.left_hang, h.right_hang, h.left_both]
                    .into_iter()
                    .filter(|h| h.dummy > 0)
                    .count();
            }
        }
        let actual = (decoded.len(), models, traces, holds.rows.len(), dummies);
        assert_eq!(actual, expected, "{game:?}");
        println!("{game:?}: weapon/model/trace/hold/dummy counts {actual:?}");
    }
    Ok(())
}
