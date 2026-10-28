//! The typed parameters of every action ([`fromsoft_formats::fxr::actions`]) against the fxr library's reading of the same
//! files (`oracles.fxr.{sekiro,elden}/params.jsonl`: EvenTorset/fxr 2b6b52f, each action's `toJSON()`), over the files the
//! oracle holds: for every action type and every list-length layout the installs have, the smallest file that holds it, and
//! every tenth file. For each action of the library's reading there is an action of ours with the same type, the same set of
//! parameter names and the same value in every one (properties compared function, loop, keyframes, tangents and modifiers).
//!
//! The library reads a spot or point light's multipliers and fade-out time scaled (it divides, as its notes say) and names
//! the later words of Dark Souls III's layout by Sekiro's: those two light types are compared in `fxr_census.rs` with its
//! factors and are skipped here, as is the one Sekiro file the library refuses.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt` and
//! `oracles.fxr.{sekiro,elden}/{params,curves}.jsonl`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::fxr::actions::Typed;
use fromsoft_formats::fxr::{self, Function, Fxr, Param, Property};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn numbers(a: &Value) -> Option<Vec<f64>> {
    match a {
        Value::Number(n) => Some(vec![n.as_f64()?]),
        Value::Array(items) => items.iter().map(Value::as_f64).collect(),
        _ => None,
    }
}

fn floats(v: &[f32]) -> Vec<f64> {
    v.iter().map(|&x| f64::from(x)).collect()
}

fn same_modifiers(p: &Property, json: Option<&Value>) -> std::result::Result<(), String> {
    let list = json.and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    if list.len() != p.modifiers.len() {
        return Err(format!(
            "{} modifiers, the library {}",
            p.modifiers.len(),
            list.len()
        ));
    }
    for (m, j) in p.modifiers.iter().zip(list) {
        let kind = m.kind().map_err(|e| e.to_string())?;
        let int = |k: &str| j.get(k).and_then(Value::as_i64);
        let seeds = |s: &[i32]| match j.get("seed") {
            Some(Value::Number(n)) => s.len() == 1 && n.as_i64() == Some(i64::from(s[0])),
            Some(Value::Array(a)) => {
                a.iter().map(Value::as_i64).collect::<Vec<_>>()
                    == s.iter().map(|&x| Some(i64::from(x))).collect::<Vec<_>>()
            }
            _ => false,
        };
        let num = |k: &str, v: &[f32]| j.get(k).and_then(numbers) == Some(floats(v));
        let ok = match (&kind, j.get("type").and_then(Value::as_str)) {
            (fxr::ModifierKind::RandomDelta { seeds: s, max }, Some("RandomDelta")) => {
                seeds(s) && num("max", max)
            }
            (fxr::ModifierKind::RandomRange { seeds: s, min, max }, Some("RandomRange")) => {
                seeds(s) && num("min", min) && num("max", max)
            }
            (fxr::ModifierKind::RandomFraction { seeds: s, max }, Some("RandomFraction")) => {
                seeds(s) && num("max", max)
            }
            (
                fxr::ModifierKind::ExternalValue1 {
                    external_value,
                    factor,
                },
                Some("ExternalValue1"),
            )
            | (
                fxr::ModifierKind::ExternalValue2 {
                    external_value,
                    factor,
                },
                Some("ExternalValue2"),
            ) => {
                int("externalValue") == Some(i64::from(*external_value))
                    && j.get("factor")
                        .is_some_and(|f| property_matches(factor, f).is_ok())
            }
            _ => false,
        };
        if !ok {
            return Err(format!("modifier {kind:?} against {j}"));
        }
    }
    Ok(())
}

/// A property against the library's JSON of it: a number or array (constant, no modifiers), `{value, modifiers}`, or a function.
fn property_matches(p: &Property, v: &Value) -> std::result::Result<(), String> {
    match v {
        Value::Number(_) | Value::Array(_) => {
            let want = numbers(v).ok_or("not numbers")?;
            let got = p.constant().map(|c| floats(&c));
            if got != Some(want.clone()) || !p.modifiers.is_empty() {
                return Err(format!(
                    "{got:?} with {} modifiers against {want:?}",
                    p.modifiers.len()
                ));
            }
            Ok(())
        }
        Value::Object(o) if o.contains_key("value") => {
            let want = o.get("value").and_then(numbers);
            let got = p.constant().map(|c| floats(&c));
            if got != want {
                return Err(format!("{got:?} against {want:?}"));
            }
            same_modifiers(p, o.get("modifiers"))
        }
        Value::Object(o) => {
            let function = o.get("function").and_then(Value::as_str).unwrap_or("");
            if function != format!("{:?}", p.function)
                || o.get("loop").and_then(Value::as_bool) != Some(p.looped)
            {
                return Err(format!("function {:?} loop {} against {v}", p.function, p.looped));
            }
            same_modifiers(p, o.get("modifiers"))?;
            let k = p.keyframes().ok_or("no keyframes")?;
            let frames = o
                .get("keyframes")
                .and_then(Value::as_array)
                .ok_or("no keyframes in the library")?;
            if frames.len() != k.positions.len() {
                return Err(format!(
                    "{} keyframes against {}",
                    k.positions.len(),
                    frames.len()
                ));
            }
            let tangent_names = match p.function {
                Function::Bezier => ["p1", "p2"],
                _ => ["t1", "t2"],
            };
            for (i, f) in frames.iter().enumerate() {
                if f.get("position").and_then(Value::as_f64) != Some(f64::from(k.positions[i]))
                    || f.get("value").and_then(numbers) != Some(floats(&k.values[i]))
                {
                    return Err(format!(
                        "keyframe {i}: {:?} {:?} against {f}",
                        k.positions[i], k.values[i]
                    ));
                }
                if let Some(t) = &k.tangents {
                    let ok = f.get(tangent_names[0]).and_then(numbers) == Some(floats(&t.first[i]))
                        && f.get(tangent_names[1]).and_then(numbers) == Some(floats(&t.second[i]));
                    if !ok {
                        return Err(format!("keyframe {i} tangents against {f}"));
                    }
                }
            }
            Ok(())
        }
        _ => Err(format!("unexpected {v}")),
    }
}

/// How the library presents a stored value (its `ActionDataConversion` read functions, sourced from its code): the units and
/// signs its editor shows, which the files do not store.
enum Shown {
    /// As stored.
    Stored,
    /// The first component of a vector negated.
    NegateX,
    /// An integer divided by `n` (frames at `n` to the second).
    IntOver(f64),
    /// A boolean inverted.
    Not,
    /// A property multiplied by a factor (compared on its values; its tangents are not compared).
    Scaled(f64),
    /// Not compared (the library merges or renames the parameter).
    Skip,
}

fn shown(ty: i64, name: &str, uniform_scale: bool) -> Shown {
    use std::f64::consts::PI;
    match (ty, name) {
        (35, "offset" | "rotation") | (36, "offset") => Shown::NegateX,
        (64 | 65 | 83 | 84 | 105 | 106 | 121 | 122, "turnInterval") => Shown::IntOver(50.0),
        (65 | 105 | 106 | 122, "followRotation") => Shown::Not,
        (607, "stirSpeed") => Shown::Scaled(180.0 / PI),
        (10300 | 10301 | 10303, "fadeOutTime") | (10500, "initialSimulationTime") => Shown::IntOver(30.0),
        (
            10000 | 10001,
            "emissionIntervalMin"
            | "emissionIntervalMax"
            | "particleDuration"
            | "particleRandomTurnIntervalMax",
        ) => Shown::IntOver(30.0),
        (10000 | 10001, "particleAngularAccelerationZ") => Shown::Scaled(180.0 / PI),
        // a uniform-scale particle's Y size is added to its X size by the library
        (10000 | 10001, n)
            if uniform_scale && (n.starts_with("particleSize") || n.starts_with("particleGrowth")) =>
        {
            Shown::Skip
        }
        (10015, _) => Shown::Skip,
        _ => Shown::Stored,
    }
}

fn scaled_values_match(p: &Property, v: &Value, factor: f64) -> std::result::Result<(), String> {
    let scale = |x: &[f32]| x.iter().map(|&c| factor * f64::from(c)).collect::<Vec<_>>();
    match v {
        Value::Number(_) | Value::Array(_) => {
            let got = p.constant().map(|c| scale(&c));
            if got == numbers(v) {
                Ok(())
            } else {
                Err(format!("{got:?} against {v}"))
            }
        }
        Value::Object(o) if o.contains_key("value") => {
            let got = p.constant().map(|c| scale(&c));
            if got == o.get("value").and_then(numbers) {
                Ok(())
            } else {
                Err(format!("{got:?} against {v}"))
            }
        }
        Value::Object(o) => {
            let k = p.keyframes().ok_or("no keyframes")?;
            let frames = o
                .get("keyframes")
                .and_then(Value::as_array)
                .ok_or("no keyframes in the library")?;
            if frames.len() != k.positions.len() {
                return Err("keyframe count".into());
            }
            for (i, f) in frames.iter().enumerate() {
                if f.get("value").and_then(numbers) != Some(scale(&k.values[i])) {
                    return Err(format!("keyframe {i}: {:?} against {f}", k.values[i]));
                }
            }
            Ok(())
        }
        _ => Err(format!("unexpected {v}")),
    }
}

fn param_matches(
    ty: i64,
    name: &str,
    ours: &Param<'_>,
    v: &Value,
    uniform_scale: bool,
) -> std::result::Result<(), String> {
    let bad = |what: String| Err(what);
    match (shown(ty, name, uniform_scale), ours) {
        (Shown::Skip, _) => Ok(()),
        (Shown::NegateX, Param::Vector(x)) => {
            let want: Vec<f64> = x
                .iter()
                .enumerate()
                .map(|(i, &c)| if i == 0 { -f64::from(c) } else { f64::from(c) })
                .collect();
            if numbers(v) == Some(want.clone()) {
                Ok(())
            } else {
                bad(format!("{x:?} with x negated is {want:?}, the library {v}"))
            }
        }
        (Shown::IntOver(n), Param::Int(i)) => {
            if v.as_f64() == Some(f64::from(*i) / n) {
                Ok(())
            } else {
                bad(format!("{i} / {n} against {v}"))
            }
        }
        (Shown::Not, Param::Bool(b)) => {
            if v.as_bool() == Some(!*b) {
                Ok(())
            } else {
                bad(format!("not {b} against {v}"))
            }
        }
        (Shown::Scaled(f), Param::Property(p)) => scaled_values_match(p, v, f),
        (Shown::Stored, Param::Property(p)) => property_matches(p, v),
        (Shown::Stored, Param::Bool(b)) => (v.as_bool() == Some(*b))
            .then_some(())
            .ok_or_else(|| format!("bool {b} against {v}")),
        (Shown::Stored, Param::Int(i)) => (v.as_i64() == Some(i64::from(*i)))
            .then_some(())
            .ok_or_else(|| format!("int {i} against {v}")),
        (Shown::Stored, Param::Float(f)) => (v.as_f64() == Some(f64::from(*f)))
            .then_some(())
            .ok_or_else(|| format!("float {f} against {v}")),
        (Shown::Stored, Param::Vector(x)) => (numbers(v) == Some(floats(x)))
            .then_some(())
            .ok_or_else(|| format!("vector {x:?} against {v}")),
        (shown, p) => bad(format!(
            "conversion {} does not apply to {p:?}",
            match shown {
                Shown::NegateX => "negate x",
                Shown::IntOver(_) => "int over",
                Shown::Not => "not",
                _ => "scale",
            }
        )),
    }
}

/// Whether our typed action `t` is the library's `json`: every parameter we name is in the library's JSON with the same value.
/// (The library's JSON also holds parameters its other games name and the layout of this one lacks, at their defaults; those
/// are returned so the test can count them.)
fn action_matches<'a>(t: &Typed, json: &'a Value) -> std::result::Result<Vec<&'a str>, String> {
    let obj = json.as_object().ok_or("not an object")?;
    let mine = t.params();
    let ty = obj.get("type").and_then(Value::as_i64).unwrap_or(-1);
    let uniform_scale = obj
        .get("particleUniformScale")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for (name, p) in &mine {
        let Some(v) = obj.get(*name) else {
            // the library presents a rich model's U and V parameters as one two-component UV parameter
            if ty == 10015
                && matches!(
                    *name,
                    "offsetU" | "offsetV" | "speedU" | "speedV" | "speedMultiplierU" | "speedMultiplierV"
                )
            {
                continue;
            }
            // and keeps an effect's rate of time (action 10500's property) off the action
            if ty == 10500 && *name == "rateOfTime" {
                continue;
            }
            return Err(format!("{name}: absent from the library's JSON"));
        };
        param_matches(ty, name, p, v, uniform_scale).map_err(|e| format!("{name}: {e}"))?;
    }
    let have: BTreeSet<&str> = mine.iter().map(|(n, _)| *n).collect();
    Ok(obj
        .keys()
        .map(String::as_str)
        .filter(|k| *k != "type" && !have.contains(k))
        .collect())
}

#[test]
fn typed_parameters_agree_with_the_fxr_library() -> Result<()> {
    for (game, data) in [(Game::Sekiro, Data::Sekiro), (Game::EldenRing, Data::Elden)] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(params)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
            oracle(OracleSet::Fxr, data, "params.jsonl"),
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
        let mut lines: BTreeMap<(String, String), Value> = BTreeMap::new();
        for line in std::fs::read_to_string(params)?.lines() {
            let v: Value = serde_json::from_str(line)?;
            lines.insert(
                (
                    v["binder"].as_str().unwrap().to_owned(),
                    v["file"].as_str().unwrap().to_owned(),
                ),
                v,
            );
        }
        let mut collected: Vec<(String, Vec<Typed>, Vec<Value>)> = Vec::new();
        let mut refused = 0usize;
        for path in paths {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            let stem = fromsoft_formats::names::stem(path);
            for member in &binder.files {
                let name = member.name.clone().unwrap_or_default();
                if fxr::member_id(&name).is_none() {
                    continue;
                }
                let file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
                let Some(line) = lines.get(&(stem.to_owned(), file.clone())) else {
                    continue;
                };
                if line.get("error").is_some() {
                    refused += 1;
                    continue;
                }
                let read = Fxr::read(&member.bytes)?;
                let mut ours: Vec<Typed> = Vec::new();
                for a in read.actions() {
                    if a.id != 0 {
                        ours.push(Typed::read(a, game)?);
                    }
                }
                let theirs: Vec<Value> = line["actions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|a| !a.is_null() && a["type"] != 0)
                    .cloned()
                    .collect();
                collected.push((format!("{path} {file}"), ours, theirs));
            }
        }
        let files = collected.len();
        // the types the library models one for one: the same number of actions in all the files together
        let mut count_ours: BTreeMap<i64, usize> = BTreeMap::new();
        let mut count_theirs: BTreeMap<i64, usize> = BTreeMap::new();
        for (_, ours, theirs) in &collected {
            for t in ours {
                *count_ours.entry(i64::from(t.id())).or_default() += 1;
            }
            for j in theirs {
                *count_theirs.entry(j["type"].as_i64().unwrap()).or_default() += 1;
            }
        }
        let comparable: BTreeSet<i64> = count_ours
            .iter()
            .filter(|(t, n)| count_theirs.get(t) == Some(n))
            .map(|(t, _)| *t)
            .collect();
        let not_comparable: BTreeMap<i64, (usize, usize)> = count_ours
            .keys()
            .chain(count_theirs.keys())
            .filter(|t| !comparable.contains(t))
            .map(|t| {
                (
                    *t,
                    (
                        count_ours.get(t).copied().unwrap_or(0),
                        count_theirs.get(t).copied().unwrap_or(0),
                    ),
                )
            })
            .collect();
        let mut by_type: BTreeMap<i64, usize> = BTreeMap::new();
        let mut library_only_names: BTreeMap<(i64, String), usize> = BTreeMap::new();
        let mut failures: Vec<String> = Vec::new();
        let mut actions = 0usize;
        let mut generic = 0usize;
        for (label, ours, theirs) in &collected {
            let mut used = vec![false; ours.len()];
            for j in theirs {
                let ty = j["type"].as_i64().unwrap();
                if !comparable.contains(&ty) || ty == 609 || ty == 11000 {
                    continue;
                }
                // the library keeps an action whose lists fit none of its layouts as plain words (`$generic`): Elden Ring files
                // hold 5 action-10500 records in Sekiro's layout, which the library does not recognise there
                if j.get("$generic").is_some() {
                    generic += 1;
                    continue;
                }
                let mut why = String::new();
                let mut extra_names: Vec<&str> = Vec::new();
                let hit = (0..ours.len()).find(|&i| {
                    if used[i] || i64::from(ours[i].id()) != ty {
                        return false;
                    }
                    match action_matches(&ours[i], j) {
                        Ok(extra) => {
                            extra_names = extra;
                            true
                        }
                        Err(e) => {
                            if why.is_empty() {
                                why = e;
                            }
                            false
                        }
                    }
                });
                match hit {
                    Some(i) => {
                        used[i] = true;
                        actions += 1;
                        *by_type.entry(ty).or_default() += 1;
                        for n in extra_names {
                            *library_only_names.entry((ty, n.to_owned())).or_default() += 1;
                        }
                    }
                    None if failures.len() < 20 => failures.push(format!("{label}: action {ty}: {why}")),
                    None => {}
                }
            }
        }
        println!("  types the library does not model one for one (ours, the library's): {not_comparable:?}");
        println!(
            "  names only the library has (its other games' parameters at their defaults): {library_only_names:?}"
        );
        println!(
            "{game:?}: {files} files compared ({refused} refused by the library), {actions} actions equal ({generic} left as plain words by the library), by type {by_type:?}"
        );
        assert!(
            failures.is_empty(),
            "{game:?}: {} failures, first: {:#?}",
            failures.len(),
            failures
        );
        assert_eq!(refused, usize::from(game == Game::Sekiro));
        assert_eq!(files, if game == Game::Sekiro { 334 } else { 1246 });
    }
    Ok(())
}

/// The keyframed properties the library evaluates (`curves.jsonl`: stepped, linear, Bezier and Hermite, up to 60 per action
/// type, parameter and function, at the keyframes, between them and past the ends) against [`Property::value_at`].
#[test]
fn keyframed_properties_evaluate_as_the_fxr_library_does() -> Result<()> {
    for (game, data) in [(Game::Sekiro, Data::Sekiro), (Game::EldenRing, Data::Elden)] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(curves)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
            oracle(OracleSet::Fxr, data, "curves.jsonl"),
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
        let mut wanted: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
        for line in std::fs::read_to_string(curves)?.lines() {
            let v: Value = serde_json::from_str(line)?;
            wanted
                .entry((
                    v["binder"].as_str().unwrap().to_owned(),
                    v["file"].as_str().unwrap().to_owned(),
                ))
                .or_default()
                .push(v);
        }
        let total: usize = wanted.values().map(Vec::len).sum();
        let (mut matched, mut points, mut worst) = (0usize, 0usize, 0.0f64);
        let mut by_function: BTreeMap<String, usize> = BTreeMap::new();
        let mut failures: Vec<String> = Vec::new();
        for path in paths {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            let stem = fromsoft_formats::names::stem(path);
            for member in &binder.files {
                let name = member.name.clone().unwrap_or_default();
                if fxr::member_id(&name).is_none() {
                    continue;
                }
                let file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
                let Some(records) = wanted.get(&(stem.to_owned(), file.clone())) else {
                    continue;
                };
                let read = Fxr::read(&member.bytes)?;
                let typed: Vec<Typed> = read
                    .actions()
                    .into_iter()
                    .filter(|a| a.id != 0)
                    .map(|a| Typed::read(a, game))
                    .collect::<std::result::Result<_, _>>()?;
                for rec in records {
                    let ty = rec["type"].as_i64().unwrap();
                    let pname = rec["name"].as_str().unwrap();
                    let frames = rec["keyframes"].as_array().unwrap();
                    let candidates = typed
                        .iter()
                        .filter(|t| i64::from(t.id()) == ty)
                        .flat_map(|t| t.params())
                        .filter_map(|(n, p)| match p {
                            Param::Property(prop) if n == pname => Some(prop.clone()),
                            _ => None,
                        });
                    let found = candidates.into_iter().find(|p| {
                        p.keyframes().is_some_and(|k| {
                            k.positions.len() == frames.len()
                                && frames.iter().enumerate().all(|(i, f)| {
                                    let (a, b) = if p.function == Function::Bezier {
                                        ("p1", "p2")
                                    } else {
                                        ("t1", "t2")
                                    };
                                    let tangents = k.tangents.as_ref().is_none_or(|t| {
                                        numbers(&f[a]) == Some(floats(&t.first[i]))
                                            && numbers(&f[b]) == Some(floats(&t.second[i]))
                                    });
                                    f["position"].as_f64() == Some(f64::from(k.positions[i]))
                                        && f["value"]
                                            .as_f64()
                                            .map(|v| vec![v])
                                            .or_else(|| numbers(&f["value"]))
                                            == Some(floats(&k.values[i]))
                                        && tangents
                                })
                        }) && p.looped == rec["loop"].as_bool().unwrap_or(false)
                            && format!("{:?}", p.function)
                                == rec["function"]
                                    .as_i64()
                                    .map(|f| {
                                        [
                                            "Zero", "One", "Constant", "Stepped", "Linear", "Bezier",
                                            "Hermite",
                                        ][usize::try_from(f).unwrap()]
                                    })
                                    .unwrap_or("")
                    });
                    let Some(p) = found else {
                        if failures.len() < 20 {
                            failures.push(format!(
                                "{path} {file}: no property {ty}/{pname} with the library's keyframes"
                            ));
                        }
                        continue;
                    };
                    matched += 1;
                    *by_function.entry(format!("{:?}", p.function)).or_default() += 1;
                    for pt in rec["points"].as_array().unwrap() {
                        let x = pt[0].as_f64().unwrap();
                        let want = numbers(&pt[1]).unwrap();
                        // a probe at a whole number of loop durations, rounded to the nearest f32, may wrap where the
                        // library's double does not (a key at the duration is one such place)
                        if p.looped {
                            let duration = frames
                                .iter()
                                .filter_map(|f| f["position"].as_f64())
                                .fold(0.0f64, f64::max);
                            let turns = x / duration;
                            if duration > 0.0 && (turns - turns.round()).abs() * duration < 1e-6 {
                                continue;
                            }
                        }
                        let got = p.value_at(x as f32);
                        points += 1;
                        for (g, w) in got.iter().zip(&want) {
                            let scale = frames
                                .iter()
                                .filter_map(|f| numbers(&f["value"]))
                                .flatten()
                                .fold(1.0f64, |m, v| m.max(v.abs()));
                            let err = (f64::from(*g) - w).abs() / w.abs().max(scale);
                            worst = worst.max(err);
                            // Hermite is the game's own function; the library's is an approximation of it (0.5% at the worst)
                            let allowed = if p.function == Function::Hermite {
                                6e-3
                            } else {
                                2e-4
                            };
                            if err > allowed && failures.len() < 20 {
                                failures.push(format!(
                                    "{path} {file}: {ty}/{pname} ({:?}) at {x}: {g} against {w}",
                                    p.function
                                ));
                            }
                        }
                    }
                }
            }
        }
        println!(
            "{game:?}: {matched} of {total} properties matched, {points} points, worst relative error {worst:e}, by function {by_function:?}"
        );
        assert!(failures.is_empty(), "{game:?}: {failures:#?}");
        assert_eq!(matched, total);
    }
    Ok(())
}
