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
