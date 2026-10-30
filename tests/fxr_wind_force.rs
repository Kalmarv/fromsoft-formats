//! The wind force action's parameters named from what Elden Ring does with them ([`fromsoft_formats::fxr::actions::WindForce`],
//! the names in `TRACED_PARAMS`), against the force field the running game built from one such action.
//!
//! The oracle: in a capture of Elden Ring 1.17.3 (offline, Stormveil Castle), the force field that blew on the player's
//! cloth was spawned by map `m10_00_00_00`'s wind effect region `_808003_1001` from effect `f000808003`. The game copies
//! the action's fields into the field it creates, and the values below are the field's bytes at the places its force reads
//! them, read from the game's memory (the same values held for all 1,499 steps of the capture, and the cloth's forces
//! computed from them equal the game's bit for bit). Each value is asserted under its name here, so a name on the wrong
//! word fails.
//!
//! Needs `installs.elden`, `keys.elden` and `tools.oodle6`; skipped when Elden Ring is not configured.
#![allow(clippy::unwrap_used, clippy::float_cmp)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::fxr::actions::{TRACED_PARAMS, Typed, WindForce, library_name};
use fromsoft_formats::fxr::{self, Fxr};
use fromsoft_formats::{Archive, Binder, Game, Keys};
use inputs::{Game as Data, Tool, install, keys, oodle};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The effect file the region names, in the binder of effects every map shares.
const BINDER: &str = "/sfx/sfxbnd_commoneffects.ffxbnd.dcx";
const EFFECT: i32 = 808_003;

#[test]
fn the_wind_force_parameters_hold_what_the_game_builds_its_force_field_from() -> Result<()> {
    let (Some(root), Some(folder), Some(dll)) =
        (install(Data::Elden), keys(Data::Elden), oodle(Tool::Oodle6))
    else {
        eprintln!("Elden Ring is not configured: skipped");
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = Archive::open_game(
        root,
        Game::EldenRing,
        &Keys::from_folder(folder, Game::EldenRing, None)?,
    )?;
    let binder = Binder::read(&archive.read(BINDER, &decoder)?, &decoder)?;
    let member = binder
        .files
        .iter()
        .find(|m| m.name.as_deref().and_then(fxr::member_id) == Some(EFFECT))
        .ok_or("the binder has no f000808003")?;
    let effect = Fxr::read(&member.bytes)?;
    let winds: Vec<WindForce> = effect
        .actions()
        .into_iter()
        .filter(|a| a.id == WindForce::ID)
        .map(|a| match Typed::read(a, Game::EldenRing)? {
            Typed::WindForce(w) => Ok(w),
            other => Err(format!("action {} read as {}", a.id, other.name()).into()),
        })
        .collect::<Result<_>>()?;
    let [w] = winds.as_slice() else {
        return Err(format!("{} wind force actions, the effect has one", winds.len()).into());
    };
    // the field's values, by the name of the word the game copies each from
    assert_eq!(w.unk_ds3_f1_31, 0, "the field's kind");
    assert_eq!(w.local, 0);
    assert_eq!(w.distance_falloff, 0);
    assert_eq!(w.distance_falloff_radius, 0.0);
    assert_eq!(w.force_random_multiplier_min, 12.0);
    assert_eq!(w.force_random_multiplier_max, 15.0);
    assert_eq!(w.wave_length, 1.0);
    assert_eq!(w.wave_offset, 1.0);
    assert_eq!(w.unk_ds3_f1_42, 0.1);
    assert_eq!(w.weight, 1.0);
    assert_eq!(w.gust, 3.0);
    assert_eq!(w.gust_length, 1.0);
    assert_eq!(w.gust_scale, 1.0);
    assert_eq!(w.gust_offset, 1.0);
    assert_eq!(w.gust_rise, 5.0);
    assert_eq!(w.gust_fall, 10.0);
    assert_eq!(w.gust_rest, 60.0);

    // every renamed parameter is one the library names, and `params` gives it under the new name
    let names: Vec<&str> = w.params().iter().map(|(n, _)| *n).collect();
    for &(ty, ours, theirs) in TRACED_PARAMS {
        assert_eq!(ty, WindForce::ID);
        assert!(names.contains(&ours), "{ours} is not among the parameters");
        assert!(!names.contains(&theirs), "{theirs} is still named");
        assert_eq!(library_name(ty, ours), theirs);
    }
    assert_eq!(library_name(WindForce::ID, "forceMultiplier"), "forceMultiplier");
    Ok(())
}
