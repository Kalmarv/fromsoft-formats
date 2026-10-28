//! Every light bank of a game against an independent reader's reading of the same files (`btl.jsonl` of the game's
//! format oracle: SoulsFormatsNEXT ee1dd61's BTL for Elden Ring, `oracles.soulsformats.elden/format`): the stored
//! version and every light's name, type, colours, powers, shadow flag, cone, position, rotation and radius, floats as
//! bit patterns. A bank the reference refuses must be refused by ours too, except the version 15 banks: the reference
//! lists only versions 1, 2, 5, 6, 16 and 18, refuses 15 by its version, and ours reads them with their 240-byte light
//! record. Their light is checked against an independent reading of the same bytes (a Python parser of the record).
mod inputs;

use fromsoft_formats::{Game, btl::LightBank, dcx::Decoder};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
mod common;

#[test]
fn every_light_bank_matches_an_independent_reading() -> Result<()> {
    for (game, format) in [(
        Game::EldenRing,
        inputs::oracle(inputs::OracleSet::SoulsFormats, inputs::Game::Elden, "format"),
    )] {
        let (Some(archive), Some(oracle), Some(dll)) = (
            common::archive(game)?,
            format,
            inputs::oodle(inputs::Tool::Oodle6),
        ) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let (mut banks, mut lights, mut agreed_rejections) = (0usize, 0usize, 0usize);
        let mut version_15 = Vec::new();
        let mut failures = Vec::new();
        for line in std::fs::read_to_string(oracle.join("btl.jsonl"))?.lines() {
            let record: serde_json::Value = serde_json::from_str(line)?;
            let path = record["path"].as_str().ok_or("path")?;
            let bank = match (
                LightBank::read(&archive.read(path, &decoder)?),
                record.get("error"),
            ) {
                (Err(e), Some(reason)) => {
                    // both refuse it: agreement; the reasons are each reader's own words
                    println!("AGREED REJECTION {path}: ours {e}; reference {reason}");
                    agreed_rejections += 1;
                    continue;
                }
                (Err(e), None) => {
                    failures.push(format!("{path}: we refuse what the reference reads: {e}"));
                    continue;
                }
                (Ok(bank), Some(reason))
                    if bank.version == 15
                        && reason
                            .as_str()
                            .is_some_and(|r| r.starts_with("Read Int32: 0xF | Expected: ")) =>
                {
                    version_15.push((path.to_owned(), bank));
                    continue;
                }
                (Ok(_), Some(_)) => {
                    failures.push(format!("{path}: we read what the reference refuses"));
                    continue;
                }
                (Ok(bank), None) => bank,
            };
            banks += 1;
            let want = record["lights"].as_array().ok_or("lights")?;
            if Some(u64::from(bank.version)) != record["version"].as_u64() || want.len() != bank.lights.len()
            {
                failures.push(format!("{path}: version or light count"));
                continue;
            }
            for (l, w) in bank.lights.iter().zip(want) {
                lights += 1;
                let rgb = |a: [u8; 3]| a.iter().map(|&x| i64::from(x)).collect::<Vec<_>>();
                let ints = |v: &serde_json::Value| {
                    v.as_array()
                        .map(|a| a.iter().filter_map(serde_json::Value::as_i64).collect::<Vec<_>>())
                };
                let kind = match l.kind {
                    fromsoft_formats::btl::Kind::Point => 0,
                    fromsoft_formats::btl::Kind::Spot => 1,
                    fromsoft_formats::btl::Kind::Directional => 2,
                };
                let same = Some(l.name.as_str()) == w["name"].as_str()
                    && Some(kind) == w["type"].as_i64()
                    && Some(rgb(l.diffuse)) == ints(&w["diffuse"])
                    && Some(rgb(l.specular)) == ints(&w["specular"])
                    && l.diffuse_power.to_bits() == common::bits(&w["diffuse_power"])?
                    && l.specular_power.to_bits() == common::bits(&w["specular_power"])?
                    && Some(l.cast_shadows) == w["cast_shadows"].as_bool()
                    && l.cone_angle.to_bits() == common::bits(&w["cone_angle"])?
                    && l.position.map(f32::to_bits) == common::bits3(&w["position"])?
                    && l.rotation.map(f32::to_bits) == common::bits3(&w["rotation"])?
                    && l.radius.to_bits() == common::bits(&w["radius"])?;
                if !same {
                    failures.push(format!("{path}: light {}", l.name));
                }
            }
        }
        println!(
            "{}: light banks {banks}, lights {lights}, agreed rejections {agreed_rejections}, mismatches {}",
            game.name(),
            failures.len()
        );
        for f in failures.iter().take(40) {
            println!("MISMATCH {f}");
        }
        // the two version 15 banks: m60_42_33_00 has no light, m60_42_36_00 one point light (record bytes 0 to 232
        // read as version 18: type 0, diffuse F0 B4 78, power 0.15, position -24.54, 95.0, 71.94; the 8 bytes after 232 zero)
        let counts = version_15
            .iter()
            .map(|(path, bank)| (path.rsplit('/').next().unwrap_or(path), bank.lights.len()))
            .collect::<Vec<_>>();
        assert_eq!(
            counts,
            [("m60_42_33_00_0000.btl.dcx", 0), ("m60_42_36_00_0000.btl.dcx", 1)],
            "{}: version 15 banks",
            game.name()
        );
        let light = &version_15[1].1.lights[0];
        assert_eq!(
            (light.kind, light.diffuse),
            (fromsoft_formats::btl::Kind::Point, [0xF0, 0xB4, 0x78])
        );
        assert!((light.diffuse_power - 0.15).abs() < 1e-6);
        assert!(
            light
                .position
                .iter()
                .zip([-24.54, 95.0, 71.94])
                .all(|(a, b)| (a - b).abs() < 0.005)
        );
        assert!(light.auxiliary[&232] == 0 && light.auxiliary[&236] == 0);
        println!(
            "{}: version 15 banks read beyond the reference: {}",
            game.name(),
            version_15.len()
        );
        assert!(banks > 0 && failures.is_empty());
    }
    Ok(())
}

/// The values of the words the light record names, by field, over every bank of a game: how many lights hold each
/// value (as the `f32` bits, or the integer).
#[derive(Default)]
struct Words {
    banks: usize,
    lights: usize,
    by_field: std::collections::BTreeMap<&'static str, std::collections::BTreeMap<u32, usize>>,
}

impl Words {
    /// How many lights hold `value` in `field`.
    fn count(&self, field: &str, value: f32) -> usize {
        self.by_field
            .get(field)
            .and_then(|c| c.get(&value.to_bits()))
            .copied()
            .unwrap_or(0)
    }

    /// The values of `field` and how many lights hold each, by value.
    fn values(&self, field: &str) -> Vec<(u32, usize)> {
        self.by_field[field].iter().map(|(v, n)| (*v, *n)).collect()
    }

    /// The number of distinct values of `field` and its least and greatest (the fields it is asked of are non-negative,
    /// so the bit patterns order like the floats).
    fn spread(&self, field: &str) -> (usize, f32, f32) {
        let c = &self.by_field[field];
        let lo = c.keys().next().copied().unwrap_or(0);
        let hi = c.keys().next_back().copied().unwrap_or(0);
        (c.len(), f32::from_bits(lo), f32::from_bits(hi))
    }
}

fn tally(game: Game, data: inputs::Game) -> Result<Option<Words>> {
    let (Some(archive), Some(dll), Some(names)) = (
        common::archive(game)?,
        inputs::oodle(inputs::Tool::Oodle6),
        inputs::oracle(inputs::OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let names = fromsoft_formats::NameDictionary::read_for(names, game)?;
    let mut w = Words::default();
    common::each_leaf(&archive, &names, &[".btl.dcx"], &decoder, |label, bytes| {
        let bank = LightBank::read(bytes).map_err(|e| format!("{label}: {e}"))?;
        w.banks += 1;
        for l in &bank.lights {
            w.lights += 1;
            let mut add = |field: &'static str, bits: u32| {
                *w.by_field.entry(field).or_default().entry(bits).or_default() += 1;
            };
            add("flicker_interval_min", l.flicker_interval[0].to_bits());
            add("flicker_interval_max", l.flicker_interval[1].to_bits());
            add("flicker_brightness", l.flicker_brightness.to_bits());
            add("event_id", l.event_id as u32);
            add("near_clip", l.near_clip.to_bits());
            add("shadow_model_cull_flip", u32::from(l.shadow_model_cull_flip));
            add("sharpness", l.sharpness.to_bits());
            add("horizontal_angle", l.horizontal_angle.to_bits());
            add("near_distance", l.near_distance.to_bits());
            add("enable_distance", l.enable_distance.to_bits());
            add("volume_density", l.volume_density.to_bits());
            add("exposure_weight", l.exposure_weight.to_bits());
            // the named words are not in the auxiliary map, and the words nobody names are
            assert!(
                [116, 120, 124, 128, 156, 164, 184, 188, 196, 208, 224]
                    .iter()
                    .all(|o| !l.auxiliary.contains_key(o))
                    && l.auxiliary.contains_key(&152)
                    && l.auxiliary.contains_key(&160),
                "{label}"
            );
        }
        Ok(())
    })?;
    Ok(Some(w))
}

/// The eleven words the community's editor names (and the exposure weight) over every light of Sekiro: the values and
/// how many lights hold them, from an independent Python reading of the same banks.
#[test]
fn sekiros_named_words() -> Result<()> {
    let Some(w) = tally(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    inputs::expect_found(
        inputs::Query::Install(inputs::Game::Sekiro, None),
        "light banks",
        w.banks,
    );
    assert_eq!((w.banks, w.lights), (18, 735));
    assert_eq!(w.spread("flicker_interval_min"), (2, 0., 0.2));
    assert_eq!(
        (
            w.count("flicker_interval_min", 0.),
            w.count("flicker_interval_min", 0.2)
        ),
        (680, 55)
    );
    assert_eq!(w.spread("flicker_interval_max"), (3, 0., 1.));
    assert_eq!(
        (
            w.count("flicker_interval_max", 0.),
            w.count("flicker_interval_max", 0.8),
            w.count("flicker_interval_max", 1.)
        ),
        (646, 55, 34)
    );
    assert_eq!(
        (
            w.count("flicker_brightness", 1.),
            w.count("flicker_brightness", 0.6)
        ),
        (646, 89)
    );
    assert_eq!(w.values("event_id"), vec![(-1i32 as u32, 735)]);
    assert_eq!(w.values("near_clip"), vec![(1f32.to_bits(), 735)]);
    assert_eq!(w.values("shadow_model_cull_flip"), vec![(0, 6), (1, 729)]);
    assert_eq!(w.spread("sharpness"), (33, 0.2, 30.));
    assert_eq!(w.spread("horizontal_angle"), (16, 0., 90.));
    assert_eq!(w.count("horizontal_angle", 0.), 668);
    assert_eq!(w.spread("near_distance"), (20, 0., 34.));
    assert_eq!(w.count("near_distance", 0.), 670);
    assert_eq!(w.spread("enable_distance"), (8, 0., 1000.));
    assert_eq!(w.count("enable_distance", 0.), 685);
    assert_eq!(w.spread("volume_density"), (17, 0., 10.));
    assert_eq!(
        (w.count("volume_density", 1.), w.count("volume_density", 0.)),
        (302, 193)
    );
    assert_eq!(
        (w.count("exposure_weight", 0.), w.count("exposure_weight", 1.)),
        (715, 20)
    );
    Ok(())
}

/// The same over every light of Elden Ring: no flicker, no event, no exposure weight.
#[test]
fn elden_rings_named_words() -> Result<()> {
    let Some(w) = tally(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    inputs::expect_found(
        inputs::Query::Install(inputs::Game::Elden, None),
        "light banks",
        w.banks,
    );
    assert_eq!((w.banks, w.lights), (109, 11_229));
    assert_eq!(w.values("flicker_interval_min"), vec![(0, 11_229)]);
    assert_eq!(w.values("flicker_interval_max"), vec![(0, 11_229)]);
    assert_eq!(w.values("flicker_brightness"), vec![(1f32.to_bits(), 11_229)]);
    assert_eq!(w.values("event_id"), vec![(-1i32 as u32, 11_229)]);
    assert_eq!(w.values("near_clip"), vec![(1f32.to_bits(), 11_229)]);
    assert_eq!(w.values("shadow_model_cull_flip"), vec![(0, 9_600), (1, 1_629)]);
    assert_eq!(w.spread("sharpness"), (70, 0.01, 80.));
    assert_eq!(w.spread("horizontal_angle"), (28, 0., 100.));
    assert_eq!(w.count("horizontal_angle", 0.), 11_007);
    assert_eq!(w.spread("near_distance"), (23, 0., 60.));
    assert_eq!(w.count("near_distance", 0.), 10_998);
    assert_eq!(w.spread("enable_distance"), (28, 0., 1000.));
    assert_eq!(
        (w.count("enable_distance", 40.), w.count("enable_distance", 0.)),
        (4_928, 1_611)
    );
    assert_eq!(w.spread("volume_density"), (16, 0., 10.));
    assert_eq!(
        (w.count("volume_density", 0.), w.count("volume_density", 1.)),
        (8_009, 2_221)
    );
    assert_eq!(w.values("exposure_weight"), vec![(0, 11_229)]);
    Ok(())
}
