//! Every effect file of both installs (`.fxr` members of `/sfx/*.ffxbnd.dcx`: 2,913 in Sekiro's 8 binders, 12,155 in Elden
//! Ring's 220) read, against the fxr library's reading of the same files (`oracles.fxr.<game>/lights.jsonl`: EvenTorset/fxr
//! 2b6b52f, which parses every Elden Ring file and 2,910 of Sekiro's): the count of each action type of every file, and
//! every point and spot light's parameters. An independent census (a Python walker with its own layout reading, and the
//! container census's list of binders that hold `.fxr` members) gives the counts asserted at the end.
//!
//! The three Sekiro files the library refuses (`f000800042` to `f000800044`) are checked by the counts of the independent
//! walker; the five Sekiro point lights in Dark Souls III's layout are checked for the fields the layouts share (the library
//! names the later ones by Sekiro's layout).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt` and
//! `oracles.fxr.{sekiro,elden}/lights.jsonl`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::fxr::{self, Function, Fxr, Light, Property, Revision};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A property against the library's reading of it: a number or array (constant), or a function with keyframes.
fn property_matches(p: &Property, v: &Value, scale: f64) -> bool {
    let numbers = |a: &Value| -> Option<Vec<f64>> {
        match a {
            Value::Number(n) => Some(vec![n.as_f64()?]),
            Value::Array(items) => items.iter().map(Value::as_f64).collect(),
            _ => None,
        }
    };
    match v {
        Value::Number(_) | Value::Array(_) => {
            let Some(want) = numbers(v) else { return false };
            let got: Option<Vec<f64>> = p
                .constant()
                .map(|c| c.into_iter().map(|x| f64::from(x) * scale).collect());
            got == Some(want) && p.modifiers.is_empty()
        }
        // a constant with modifiers is written as {value, modifiers}
        Value::Object(o) if o.contains_key("value") => {
            let want = o.get("value").and_then(numbers);
            let got: Option<Vec<f64>> = p
                .constant()
                .map(|c| c.into_iter().map(|x| f64::from(x) * scale).collect());
            let modifiers = o.get("modifiers").and_then(Value::as_array).map_or(0, Vec::len);
            want.is_some() && got == want && modifiers == p.modifiers.len()
        }
        Value::Object(o) => {
            let function = o.get("function").and_then(Value::as_str).unwrap_or("");
            if function != format!("{:?}", p.function)
                || o.get("loop").and_then(Value::as_bool) != Some(p.looped)
            {
                return false;
            }
            let modifiers = o.get("modifiers").and_then(Value::as_array).map_or(0, Vec::len);
            if modifiers != p.modifiers.len() {
                return false;
            }
            let Some(k) = p.keyframes() else { return false };
            let Some(frames) = o.get("keyframes").and_then(Value::as_array) else {
                return false;
            };
            if frames.len() != k.positions.len() {
                return false;
            }
            let tangent_names = match p.function {
                Function::Bezier => ["p1", "p2"],
                _ => ["t1", "t2"],
            };
            frames.iter().enumerate().all(|(i, f)| {
                f.get("position").and_then(Value::as_f64) == Some(f64::from(k.positions[i]))
                    && f.get("value").and_then(numbers).is_some_and(|w| {
                        w == k.values[i]
                            .iter()
                            .map(|&x| f64::from(x) * scale)
                            .collect::<Vec<_>>()
                    })
                    && match (&k.tangents, function) {
                        (None, _) => true,
                        (Some(t), _) => [(&t.first, tangent_names[0]), (&t.second, tangent_names[1])]
                            .iter()
                            .all(|(rows, name)| {
                                f.get(*name).and_then(numbers).is_some_and(|w| {
                                    w == rows[i].iter().map(|&x| f64::from(x) * scale).collect::<Vec<_>>()
                                })
                            }),
                    }
            })
        }
        _ => false,
    }
}

/// A light's typed fields against the library's, by the library's names. `shared_only` limits the check to the fields
/// both layouts of a point light have at the same index.
fn light_matches(l: &Light, w: &Value, shared_only: bool) -> std::result::Result<(), String> {
    let f = |name: &str| w.get(name).cloned().unwrap_or(Value::Null);
    let num = |name: &str, got: f64| f(name).as_f64() == Some(got);
    let flag = |name: &str, got: bool| f(name).as_bool() == Some(got);
    let mut bad = Vec::new();
    let mut check = |name: &str, ok: bool| {
        if !ok {
            bad.push(name.to_owned());
        }
    };
    let (flicker, shadows, separate, darkness, fade) = match l {
        Light::Point(p) => (
            &p.flicker,
            p.shadows,
            p.separate_specular,
            p.shadow_darkness,
            p.fade_out_frames,
        ),
        Light::Spot(s) => (
            &s.flicker,
            s.shadows,
            s.separate_specular,
            s.shadow_darkness,
            s.fade_out_frames,
        ),
    };
    let props: Vec<(&str, &Property)> = match l {
        Light::Point(p) => vec![
            ("diffuseColor", &p.diffuse_color),
            ("specularColor", &p.specular_color),
            ("radius", &p.radius),
        ],
        Light::Spot(s) => vec![
            ("diffuseColor", &s.diffuse_color),
            ("specularColor", &s.specular_color),
            ("diffuseMultiplier", &s.diffuse_multiplier),
            ("specularMultiplier", &s.specular_multiplier),
            ("near", &s.near),
            ("far", &s.far),
            ("radiusX", &s.radius_x),
            ("radiusY", &s.radius_y),
        ],
    };
    for (name, p) in props {
        // the library presents a spot light's brightness multipliers divided by ten (the files store ten times that)
        let scale = if matches!(l, Light::Spot(_)) && name.ends_with("Multiplier") {
            0.1
        } else {
            1.0
        };
        check(name, property_matches(p, &f(name), scale));
    }
    if let Light::Point(p) = l {
        for (name, q) in [
            ("diffuseMultiplier", &p.diffuse_multiplier),
            ("specularMultiplier", &p.specular_multiplier),
        ] {
            if let Some(q) = q {
                check(name, property_matches(q, &f(name), 1.0));
            }
        }
    }
    check("jitterAndFlicker", flag("jitterAndFlicker", flicker.enabled));
    check(
        "jitterAcceleration",
        num("jitterAcceleration", f64::from(flicker.acceleration)),
    );
    check("jitterX", num("jitterX", f64::from(flicker.jitter[0])));
    check("jitterY", num("jitterY", f64::from(flicker.jitter[1])));
    check("jitterZ", num("jitterZ", f64::from(flicker.jitter[2])));
    check(
        "flickerIntervalMin",
        num("flickerIntervalMin", f64::from(flicker.interval[0])),
    );
    check(
        "flickerIntervalMax",
        num("flickerIntervalMax", f64::from(flicker.interval[1])),
    );
    check(
        "flickerBrightness",
        num("flickerBrightness", f64::from(flicker.brightness)),
    );
    check("shadows", flag("shadows", shadows));
    check("separateSpecular", flag("separateSpecular", separate));
    let legacy = matches!(l, Light::Point(p) if p.revision == Revision::Legacy);
    if !(shared_only && legacy) {
        check("fadeOutTime", num("fadeOutTime", f64::from(fade) / 30.0));
        check("shadowDarkness", num("shadowDarkness", f64::from(darkness)));
        match l {
            Light::Point(p) => {
                check(
                    "maxViewDistance",
                    num("maxViewDistance", f64::from(p.max_view_distance)),
                );
                if let Some(v) = p.volume_density {
                    check("volumeDensity", num("volumeDensity", f64::from(v)));
                }
                if let Some(v) = p.phase_function {
                    check("phaseFunction", flag("phaseFunction", v));
                }
                if let Some(v) = p.asymmetry_param {
                    check("asymmetryParam", num("asymmetryParam", f64::from(v)));
                }
                if let Some(v) = p.adaptation_exponent {
                    check("adaptationExponent", num("adaptationExponent", f64::from(v)));
                }
            }
            Light::Spot(s) => {
                check("volumeDensity", num("volumeDensity", f64::from(s.volume_density)));
                if let Some(v) = s.phase_function {
                    check("phaseFunction", flag("phaseFunction", v));
                }
                if let Some(v) = s.asymmetry_param {
                    check("asymmetryParam", num("asymmetryParam", f64::from(v)));
                }
                if let Some(v) = s.adaptation_exponent {
                    check("adaptationExponent", num("adaptationExponent", f64::from(v)));
                }
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.join(","))
    }
}

#[derive(Default, Debug)]
struct Totals {
    binders: usize,
    members: usize,
    fxr_files: usize,
    other_members: usize,
    ids: BTreeSet<i32>,
    actions: BTreeMap<i16, usize>,
    point_lights: usize,
    point_files: usize,
    spot_lights: usize,
    spot_files: usize,
    light_shafts: usize,
    legacy_lights: usize,
    refused: BTreeMap<String, usize>,
    not_in_oracle: usize,
    action_mismatches: Vec<String>,
    light_mismatches: Vec<String>,
    reference_mismatches: usize,
    id_mismatches: usize,
    lights_not_selected_in_state_zero: usize,
    layouts: BTreeMap<(usize, usize, usize, usize), usize>,
    max_depth: usize,
    properties: usize,
    bounds_not_extremes: usize,
    /// type 0 actions that hold a field word, a property or a field list
    placeholders_with_content: usize,
    /// configs with a static node transform, nodes with several configs and no state-config map, lights with a rotated
    /// transform among their ancestors'
    transforms: usize,
    unmapped_multi_config_nodes: usize,
    rotated_lights: usize,
}

fn tally_nodes(totals: &mut Totals, node: &fxr::Node) -> Result<()> {
    if node.configs.len() > 1 && node.state_configs().is_none() {
        totals.unmapped_multi_config_nodes += 1;
    }
    for c in &node.configs {
        if c.static_transform()?.is_some() {
            totals.transforms += 1;
        }
    }
    for child in &node.children {
        tally_nodes(totals, child)?;
    }
    Ok(())
}

fn tally(totals: &mut Totals, fxr: &Fxr) {
    fn depth(n: &fxr::Node) -> usize {
        1 + n.children.iter().map(depth).max().unwrap_or(0)
    }
    totals.max_depth = totals.max_depth.max(depth(&fxr.root));
    for a in fxr.actions() {
        *totals.actions.entry(a.id).or_default() += 1;
        if a.id == 0
            && !(a.fields1.is_empty()
                && a.fields2.is_empty()
                && a.properties1.is_empty()
                && a.properties2.is_empty()
                && a.field_lists.is_empty())
        {
            totals.placeholders_with_content += 1;
        }
        for p in a.properties1.iter().chain(&a.properties2) {
            totals.properties += 1;
            if let Some(k) = p.keyframes() {
                let c = p.components();
                let extreme = |f: &dyn Fn(f32, f32) -> f32, take: &[f32]| -> bool {
                    (0..c).all(|j| {
                        let m = k.values.iter().map(|v| v[j]).reduce(f).unwrap_or(0.0);
                        m == take[j]
                    })
                };
                if !(extreme(&f32::min, &k.minimum) && extreme(&f32::max, &k.maximum)) {
                    totals.bounds_not_extremes += 1;
                }
            }
        }
    }
}

#[test]
fn every_effect_file_reads_and_its_lights_agree_with_the_fxr_library() -> Result<()> {
    for (game, data, expect) in [
        (Game::Sekiro, Data::Sekiro, Expect::sekiro()),
        (Game::EldenRing, Data::Elden, Expect::elden()),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(lights)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
            oracle(OracleSet::Fxr, data, "lights.jsonl"),
        ) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let archive: Archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
        let dictionary = NameDictionary::read_for(dictionary, game)?;
        let paths: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .map(String::as_str)
            .filter(|p| p.starts_with("/sfx/") && p.contains(".ffxbnd"))
            .filter(|p| archive.contains(p))
            .collect();
        // the library's lines by (binder stem, file)
        let mut oracle_lines: BTreeMap<(String, String), Value> = BTreeMap::new();
        for line in std::fs::read_to_string(lights)?.lines() {
            let v: Value = serde_json::from_str(line)?;
            oracle_lines.insert(
                (
                    v["binder"].as_str().unwrap().to_owned(),
                    v["file"].as_str().unwrap().to_owned(),
                ),
                v,
            );
        }
        let mut t = Totals::default();
        for path in paths {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            t.binders += 1;
            let stem = fromsoft_formats::names::stem(path);
            for member in &binder.files {
                t.members += 1;
                let name = member.name.clone().unwrap_or_default();
                let Some(id) = fxr::member_id(&name) else {
                    t.other_members += 1;
                    continue;
                };
                t.fxr_files += 1;
                t.ids.insert(id);
                let read = match Fxr::read(&member.bytes) {
                    Ok(f) => f,
                    Err(e) => {
                        let reason = e.to_string();
                        println!("REFUSED {path} {name}: {reason}");
                        *t.refused.entry(reason.chars().take(60).collect()).or_default() += 1;
                        continue;
                    }
                };
                if read.id != id {
                    t.id_mismatches += 1;
                }
                if read.references.iter().copied().collect::<BTreeSet<_>>() != read.spawned() {
                    t.reference_mismatches += 1;
                }
                tally(&mut t, &read);
                tally_nodes(&mut t, &read.root).map_err(|e| format!("{path} {name}: {e}"))?;
                let lights = read.lights().map_err(|e| format!("{path} {name}: {e}"))?;
                let file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
                // layouts of every light action, including a refusal by counts
                for a in read.actions() {
                    if a.id == fxr::POINT_LIGHT || a.id == fxr::SPOT_LIGHT {
                        *t.layouts
                            .entry((
                                a.properties1.len(),
                                a.properties2.len(),
                                a.fields1.len(),
                                a.fields2.len(),
                            ))
                            .or_default() += 1;
                    }
                    if a.id == fxr::LIGHT_SHAFT {
                        t.light_shafts += 1;
                    }
                }
                let (mut points, mut spots) = (0, 0);
                for l in &lights {
                    match &l.light {
                        Light::Point(p) => {
                            points += 1;
                            if p.revision == Revision::Legacy {
                                t.legacy_lights += 1;
                            }
                        }
                        Light::Spot(_) => spots += 1,
                    }
                    if !l.active_in_state(0) {
                        t.lights_not_selected_in_state_zero += 1;
                    }
                    let chain = l
                        .static_transforms(0)
                        .map_err(|e| format!("{path} {name}: {e}"))?;
                    if chain.iter().any(|s| s.rotation_degrees.iter().any(|&r| r != 0.0)) {
                        t.rotated_lights += 1;
                    }
                }
                t.point_lights += points;
                t.spot_lights += spots;
                t.point_files += usize::from(points > 0);
                t.spot_files += usize::from(spots > 0);
                // the library
                let Some(w) = oracle_lines.get(&(stem.clone(), file.clone())) else {
                    t.not_in_oracle += 1;
                    continue;
                };
                if w.get("error").is_some() {
                    continue;
                }
                // action counts: the library walks the tree's actions
                let mut ours: BTreeMap<String, usize> = BTreeMap::new();
                for a in read.actions() {
                    *ours.entry(a.id.to_string()).or_default() += 1;
                }
                let theirs: BTreeMap<String, usize> = w["counts"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as usize))
                    .collect();
                // the library models some actions as structure (the state-config map 199, the spawned effect 132, the
                // level-of-detail thresholds 133), adds a termination (700) where a file has none and has no type 0
                // (the empty placeholder); every other type is one action in both
                let structural = ["0", "132", "133", "199", "700"];
                ours.retain(|k, _| !structural.contains(&k.as_str()));
                let theirs: BTreeMap<String, usize> = theirs
                    .into_iter()
                    .filter(|(k, _)| !structural.contains(&k.as_str()))
                    .collect();
                if ours != theirs {
                    t.action_mismatches.push(format!("{stem}/{file}"));
                }
                // every light of ours pairs with a light of the library that agrees on every field, each used once
                let mut want: Vec<&Value> = w["lights"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|l| l["type"] != 10003)
                    .collect();
                if want.len() != lights.len() {
                    t.light_mismatches.push(format!(
                        "{stem}/{file}: {} lights, the library reads {}",
                        lights.len(),
                        want.len()
                    ));
                    continue;
                }
                for g in &lights {
                    let mut last = String::new();
                    let found = want.iter().position(|w| match light_matches(&g.light, w, true) {
                        Ok(()) => true,
                        Err(fields) => {
                            last = fields;
                            false
                        }
                    });
                    match found {
                        Some(i) => {
                            want.swap_remove(i);
                        }
                        None => t.light_mismatches.push(format!("{stem}/{file}: {last}")),
                    }
                }
            }
        }
        println!(
            "{}: {} binders, {} members ({} effect files, {} others), {} ids, {} point lights in {} files, {} spot lights in {} files, {} light shafts, {} legacy-layout, {} refused {:?}, not in the library's list {}, action-count mismatches {}, light mismatches {}, header-list mismatches {}, id mismatches {}, lights not chosen by state 0 {}, tree depth {}, {} properties, bounds not the extremes {}, configs with a static transform {}, multi-config nodes without a state map {}, lights with a rotated transform above them {}",
            game.name(),
            t.binders,
            t.members,
            t.fxr_files,
            t.other_members,
            t.ids.len(),
            t.point_lights,
            t.point_files,
            t.spot_lights,
            t.spot_files,
            t.light_shafts,
            t.legacy_lights,
            t.refused.values().sum::<usize>(),
            t.refused,
            t.not_in_oracle,
            t.action_mismatches.len(),
            t.light_mismatches.len(),
            t.reference_mismatches,
            t.id_mismatches,
            t.lights_not_selected_in_state_zero,
            t.max_depth,
            t.properties,
            t.bounds_not_extremes,
            t.transforms,
            t.unmapped_multi_config_nodes,
            t.rotated_lights,
        );
        println!(
            "layouts (properties1, properties2, fields1, fields2): {:?}",
            t.layouts
        );
        println!("actions: {:?}", t.actions);
        for m in t.action_mismatches.iter().take(20) {
            println!("ACTION MISMATCH {m}");
        }
        for m in t.light_mismatches.iter().take(20) {
            println!("LIGHT MISMATCH {m}");
        }
        assert!(t.refused.is_empty(), "{}: refused {:?}", game.name(), t.refused);
        assert!(
            t.action_mismatches.is_empty() && t.light_mismatches.is_empty(),
            "{}",
            game.name()
        );
        assert_eq!(
            (t.reference_mismatches, t.id_mismatches),
            (0, 0),
            "{}",
            game.name()
        );
        assert_eq!(
            (t.binders, t.fxr_files, t.other_members, t.ids.len()),
            (expect.binders, expect.files, expect.others, expect.ids),
            "{}: files",
            game.name()
        );
        assert_eq!(
            (
                t.point_lights,
                t.point_files,
                t.spot_lights,
                t.spot_files,
                t.light_shafts,
                t.legacy_lights
            ),
            expect.lights,
            "{}: lights",
            game.name()
        );
        assert_eq!(t.not_in_oracle, 0, "{}", game.name());
        assert_eq!(
            t.placeholders_with_content,
            0,
            "{}: type 0 actions with content",
            game.name()
        );
        assert_eq!(
            t.actions.values().sum::<usize>(),
            expect.actions,
            "{}: actions",
            game.name()
        );
        assert_eq!(t.properties, expect.properties, "{}: properties", game.name());
        assert_eq!(
            t.layouts.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>(),
            expect.layouts,
            "{}: light layouts (properties1, properties2, fields1, fields2)",
            game.name()
        );
        assert_eq!(
            (t.transforms, t.unmapped_multi_config_nodes, t.rotated_lights),
            expect.transforms,
            "{}: configs with a static transform, multi-config nodes without a state map, lights with a rotated transform",
            game.name()
        );
        assert_eq!(
            (
                t.lights_not_selected_in_state_zero,
                t.bounds_not_extremes,
                t.max_depth
            ),
            expect.shape,
            "{}: lights not chosen by state 0, keyframed properties whose bounds are not the extremes, tree depth",
            game.name()
        );
    }
    Ok(())
}

/// What an independent census (a Python walker, and the container census's list of binders with `.fxr` members) counts.
struct Expect {
    binders: usize,
    files: usize,
    others: usize,
    ids: usize,
    /// point lights, files with one, spot lights, files with one, light shafts, point lights in the legacy layout
    lights: (usize, usize, usize, usize, usize, usize),
    actions: usize,
    properties: usize,
    /// light layouts by (properties1, properties2, fields1, fields2) with their counts
    layouts: Vec<((usize, usize, usize, usize), usize)>,
    /// lights whose config state 0 does not choose, keyframed properties whose stored bounds are not the extremes of
    /// their values, the deepest tree
    shape: (usize, usize, usize),
    /// configs with a static transform, multi-config nodes without a state map, lights with a rotated transform above
    transforms: (usize, usize, usize),
}

impl Expect {
    fn sekiro() -> Expect {
        Expect {
            binders: 8,
            files: 2_913,
            others: 598,
            ids: 2_669,
            lights: (834, 714, 13, 11, 0, 5),
            actions: 354_778,
            properties: 935_942,
            layouts: vec![
                ((10, 2, 2, 25), 5),
                ((10, 5, 2, 26), 82),
                ((10, 5, 2, 28), 40),
                ((10, 5, 2, 29), 707),
                ((11, 0, 21, 0), 1),
                ((11, 0, 23, 0), 4),
                ((11, 0, 24, 0), 8),
            ],
            shape: (2, 87, 10),
            transforms: (20_878, 0, 167),
        }
    }
    fn elden() -> Expect {
        Expect {
            binders: 220,
            files: 12_155,
            others: 13_679,
            ids: 11_912,
            lights: (4_999, 4_369, 17, 15, 0, 0),
            actions: 1_463_977,
            properties: 3_688_703,
            layouts: vec![
                ((10, 5, 2, 31), 513),
                ((10, 5, 2, 33), 4_486),
                ((11, 0, 26, 0), 5),
                ((11, 0, 28, 0), 12),
            ],
            shape: (76, 432, 14),
            transforms: (81_215, 0, 725),
        }
    }
}
