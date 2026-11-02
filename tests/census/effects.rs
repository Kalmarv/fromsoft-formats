//! The effect census: every effect file of an install's effect binders (`/sfx/*.ffxbnd.dcx`), and for each, what in the
//! install names it, so what starts it. Five kinds of record name effects:
//!
//! - a param row: a value in a field that names effects (Smithbox's paramdefs type the rows; the table and field names
//!   are the community's, from those paramdefs). A field names them where its name or its table's says so (`sfx` or `ffx`: `Magic.castSfxId`, `Bullet.sfxId_Hit`,
//!   `SpEffectVfxParam.midstSfxId`, the fields of `HitEffectSfxParam` named for the kind of hit) and its values agree:
//!   at least three distinct values of 1,000 or more in it are effect files, and they are at least half of its
//!   distinct values of 1,000 or more. The values leave the enumerations, sizes, flags and dummy polygons whose names
//!   say `sfx` and whose small values match low effect ids by chance; the name leaves the prices, sort keys, throw
//!   types and message and special-effect ids whose values match effect ids by chance (in Elden Ring with its DLC,
//!   `EquipParamAccessory.sellValue`, 3 of its 4 values of 1,000 or more; `WeatherParam.SoundId`, 3 of 3);
//! - an animation event: a field the community's event template names `FFX ID` (event 96 "Spawn One Shot FFX", 95, 104,
//!   110, 112, 116, 118 to 122, and the deprecated id of events 114 and 115, whose effect the param of the spell or item
//!   gives), in every event file of the animation binders (`*.anibnd.dcx` and those inside object, part and asset binders);
//! - another effect that is reached: its `SFXReference` actions ([`fromsoft_formats::fxr::Fxr::spawned`]), followed down
//!   the spawns (an effect only an unreached effect spawns is unreached);
//! - a map layout: an SFX or wind SFX region (`/map/mapstudio/*.msb.dcx`);
//! - an event script: an instruction argument the instruction table names `... SFX ID` (`Spawn Oneshot SFX`, `Create
//!   Asset-following SFX`, `Set Wind SFX`), literal, or filled from the event's own arguments by an initialiser that
//!   starts the event with them (in any script of the install).
//!
//! Where the configuration holds Smithbox's Elden Ring param meta, the param rule is held to it: each field the meta
//! opens as an effect file (an `ExtRefs` to an `.fxr` path) of each table read is counted as taken by the rule too, or
//! left by it for the reason the rule gives, and each field the rule takes that the meta does not open is named, as is
//! each field whose values alone the rule would take but whose name does not say `sfx` or `ffx`. In Elden Ring with its
//! DLC the rule takes 21 effect fields the meta does not open, every value of 1,000 or more in each an effect file: the
//! 15 of `HitEffectSfxParam`, `MapMimicryEstablishmentParam.mimicrySfxId0` to `2` and `WeatherParam.SfxId`,
//! `SfxIdOutdoor` and `WindSfxId`. (Sekiro's param meta opens no field as an effect file.)
//!
//! An effect none of them names is unreached: something the census does not read starts it (the game's code, a param a
//! field name does not mark, an id computed from another), or nothing does. The census counts the effects each kind
//! reaches, each combination of kinds, and per field, event field and instruction argument the records that name an
//! effect file and those that hold a positive id no effect file has; the unreached ids are listed. Every archive entry is
//! read once, on the scan's threads ([`common::scan_map`]).
use super::Report;
use crate::{common, inputs};
use fromsoft_formats::{
    Binder, Game, Install, Keys, NameDictionary,
    dcx::Decoder,
    emedf, emevd,
    fxr::{self, Fxr},
    game_params::param_binder,
    msb::MapLayout,
    param::Param,
    paramdef::ParamDefs,
    tae::{
        Tae,
        template::{Templates, Value},
    },
};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The kinds of record that name an effect, as bits of an effect's reach.
const PARAMS: u8 = 1;
const EVENTS: u8 = 2;
const EFFECTS: u8 = 4;
const MAPS: u8 = 8;
const SCRIPTS: u8 = 16;
const KINDS: [(u8, &str); 5] = [
    (PARAMS, "params"),
    (EVENTS, "animation events"),
    (EFFECTS, "other effects"),
    (MAPS, "map layouts"),
    (SCRIPTS, "event scripts"),
];

/// What one archive entry names: effects it holds (with the effects each spawns), and ids it names, each with the label
/// of the field or argument that names it and the kind of record. Event-script arguments filled from the event's own
/// arguments are kept as uses to resolve against every script's initialisers.
#[derive(Default)]
struct Found {
    /// For each effect file the entry holds: its id, the effects it spawns, and the binder.
    effects: Vec<(i32, BTreeSet<i32>, String)>,
    /// (kind bit, label, id) for each record that names an id.
    names: Vec<(u8, String, i64)>,
    /// (event id, byte offset in its arguments, label): an SFX argument an event parameter fills.
    script_uses: Vec<(i64, usize, String)>,
    /// (event id started, the words passed) for each initialiser.
    initialisations: Vec<(i64, Vec<u32>)>,
    report: Report,
    refused: Option<String>,
}

/// The census of `game`'s effects, or `None` when the configuration lacks the install, its keys, the Oodle library, the
/// name dictionary, the paramdefs or the event template.
pub fn census(game: Game) -> Result<Option<Report>> {
    let data = match game {
        Game::Sekiro => inputs::Game::Sekiro,
        Game::EldenRing => inputs::Game::Elden,
    };
    let (Some(root), Some(keys), Some(dll), Some(names), Some(defs_dir), Some(template)) = (
        inputs::install(data),
        inputs::keys(data),
        inputs::oodle(inputs::Tool::Oodle6),
        inputs::oracle(inputs::OracleSet::Uxm, data, "dictionary.txt"),
        inputs::oracle(inputs::OracleSet::Smithbox, data, "defs"),
        inputs::oracle(inputs::OracleSet::Smithbox, data, "tae-template.xml"),
    ) else {
        return Ok(None);
    };
    let Some(archive) = common::archive(game)? else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    // Paramdex's param meta, where the configuration holds it (Elden Ring's): the fields it opens as effect files
    let meta = match data {
        inputs::Game::Elden => inputs::oracle(inputs::OracleSet::Smithbox, data, "param-meta"),
        _ => None,
    };
    let names = NameDictionary::read_for(names, game)?;
    let templates = Templates::parse(&std::fs::read_to_string(template)?)?;
    // the event template's fields that hold an effect id, by event type: (field index, label)
    let mut effect_fields: BTreeMap<i32, Vec<(usize, String)>> = BTreeMap::new();
    for t in templates.iter() {
        for (i, f) in t.fields.iter().enumerate() {
            if f.name.as_deref().is_some_and(|n| n.contains("FFX ID")) {
                effect_fields.entry(t.kind).or_default().push((
                    i,
                    format!(
                        "event {} {:?} field {:?}",
                        t.kind,
                        t.name.as_deref().unwrap_or(""),
                        f.name.as_deref().unwrap_or("")
                    ),
                ));
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Entry {
        Effects,
        Animations,
        Layout,
        Script,
    }
    let kind_of = |path: &str| -> Option<Entry> {
        if path.starts_with("/sfx/") && path.ends_with(".ffxbnd.dcx") {
            Some(Entry::Effects)
        } else if [".anibnd.dcx", ".objbnd.dcx", ".partsbnd.dcx", ".geombnd.dcx"]
            .iter()
            .any(|s| path.ends_with(s))
        {
            Some(Entry::Animations)
        } else if path.starts_with("/map/mapstudio/") && path.ends_with(".msb.dcx") {
            Some(Entry::Layout)
        } else if path.starts_with("/event/") && path.ends_with(".emevd.dcx") {
            Some(Entry::Script)
        } else {
            None
        }
    };
    let entry_of = |hash: u64| -> Option<(&str, Entry)> {
        names
            .names
            .get(&hash)?
            .iter()
            .find_map(|p| kind_of(p).map(|k| (p.as_str(), k)))
    };
    let found = common::scan_map(
        &archive,
        &decoder,
        |hash| entry_of(hash).is_some(),
        |hash, bytes| {
            let mut f = Found::default();
            let Some((path, kind)) = entry_of(hash) else {
                return f;
            };
            let result = (|| -> Result<()> {
                let bytes = bytes?;
                match kind {
                    Entry::Effects => effects(bytes, &decoder, path, &mut f),
                    Entry::Animations => events(bytes, &decoder, &templates, &effect_fields, &mut f),
                    Entry::Layout => layout(bytes, &mut f),
                    Entry::Script => script(game, bytes, &mut f),
                }
            })();
            f.refused = result.err().map(|e| format!("{path}: {e}"));
            f
        },
    )?;

    let mut report = Report::default();
    // every effect file with the binders that hold it, and what each spawns
    let mut holders: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    let mut spawns: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    let mut named: Vec<(u8, String, i64)> = Vec::new();
    let mut uses = Vec::new();
    let mut initialisations: BTreeMap<i64, Vec<Vec<u32>>> = BTreeMap::new();
    for (_, f) in found {
        for (id, spawned, binder) in f.effects {
            holders.entry(id).or_default().push(binder);
            spawns.entry(id).or_default().extend(spawned);
        }
        named.extend(f.names);
        uses.extend(f.script_uses);
        for (event, words) in f.initialisations {
            initialisations.entry(event).or_default().push(words);
        }
        report.add(f.report);
        if let Some(e) = f.refused {
            report.failures.push(e);
        }
    }
    // an SFX argument filled from the event's arguments: the word each initialiser of the event passes at that offset
    for (event, source, label) in uses {
        let calls = initialisations.get(&event).map_or(&[][..], Vec::as_slice);
        report.flag(
            &format!("{label}, filled by an event parameter: no initialiser starts the event"),
            calls.is_empty(),
        );
        for words in calls {
            match words.get(source / 4) {
                Some(&w) if source % 4 == 0 => {
                    named.push((
                        SCRIPTS,
                        format!("{label} (from an initialiser's argument)"),
                        i64::from(w as i32),
                    ));
                }
                _ => report.count(
                    &format!("{label}, filled by an event parameter: an initialiser passes no word there"),
                    1,
                ),
            }
        }
    }
    // the params: one file, read whole
    named.extend(params(
        game,
        &root,
        &keys,
        &archive,
        &decoder,
        &defs_dir,
        meta.as_deref(),
        &holders,
        &mut report,
    )?);
    // each field: the records that name an effect file, and those that hold a positive id that none is
    let mut reach: BTreeMap<i32, u8> = holders.keys().map(|&id| (id, 0)).collect();
    for (kind, label, id) in &named {
        let hit = i32::try_from(*id).ok().and_then(|id| reach.get_mut(&id));
        match hit {
            Some(bits) => {
                *bits |= kind;
                report.count(&format!("{label}: records naming an effect file"), 1);
            }
            None if *id > 0 => report.count(
                &format!("{label}: records holding a positive id no effect file has"),
                1,
            ),
            None => {}
        }
    }
    // an effect spawned by a reached effect is reached through it, and so on down the spawns
    let mut todo: Vec<i32> = reach
        .iter()
        .filter(|(_, b)| **b != 0)
        .map(|(id, _)| *id)
        .collect();
    while let Some(id) = todo.pop() {
        for spawned in spawns.get(&id).into_iter().flatten() {
            match reach.get_mut(spawned) {
                Some(bits) if *bits & EFFECTS == 0 => {
                    let first = *bits == 0;
                    *bits |= EFFECTS;
                    if first {
                        todo.push(*spawned);
                    }
                }
                Some(_) => {}
                None => report.count("an effect's SFXReference action: spawns naming no effect file", 1),
            }
        }
    }
    report.count(
        "an effect's SFXReference action: spawns, by reached and unreached effects",
        spawns.values().map(|s| s.len() as u64).sum(),
    );
    report.count("effect files (distinct ids)", holders.len() as u64);
    report.count(
        "effect files held by more than one binder",
        holders.values().filter(|b| b.len() > 1).count() as u64,
    );
    for (bit, name) in KINDS {
        report.count(
            &format!("effects reached by {name}"),
            reach.values().filter(|&&b| b & bit != 0).count() as u64,
        );
    }
    for (&id, &bits) in &reach {
        let combination: Vec<&str> = KINDS
            .iter()
            .filter(|(bit, _)| bits & bit != 0)
            .map(|(_, name)| *name)
            .collect();
        let label = if combination.is_empty() {
            report.value("unreached effect ids", id);
            for binder in &holders[&id] {
                report.count(&format!("unreached effects in {}", family(binder)), 1);
            }
            "nothing".to_owned()
        } else {
            combination.join(" + ")
        };
        report.count(&format!("effects reached by exactly: {label}"), 1);
    }
    report.count(
        "effects unreached",
        reach.values().filter(|&&b| b == 0).count() as u64,
    );
    Ok(Some(report))
}

/// The family of an effect binder (`/sfx/sfxbnd_<name>.ffxbnd.dcx`): its name with a character's number written `cNNNN`
/// and a mimic's `mimicN` (one binder each), the common and map binders as they are.
fn family(binder: &str) -> String {
    let name = binder
        .rsplit('/')
        .next()
        .unwrap_or(binder)
        .trim_start_matches("sfxbnd_")
        .trim_end_matches(".ffxbnd.dcx");
    let numbered = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    };
    if numbered("mimic") {
        "the mimic binders (mimicN)".to_owned()
    } else if numbered("c") {
        "the character binders (cNNNN)".to_owned()
    } else {
        format!("sfxbnd_{name}")
    }
}

/// An effect binder: each effect file it holds, with the effects it spawns.
fn effects(bytes: &[u8], decoder: &Decoder<'_>, path: &str, f: &mut Found) -> Result<()> {
    let binder = Binder::read(bytes, decoder)?;
    f.report.count("effect binders", 1);
    for member in &binder.files {
        let Some(id) = member.name.as_deref().and_then(fxr::member_id) else {
            // the binder's textures, models and animations, by extension
            let name = member.name.as_deref().unwrap_or("");
            let kind = name
                .rsplit(['\\', '/'])
                .next()
                .unwrap_or(name)
                .split_once('.')
                .map_or("", |x| x.1);
            f.report.count(
                &format!(
                    "effect binder members that are not effect files (.{})",
                    kind.to_ascii_lowercase()
                ),
                1,
            );
            continue;
        };
        match Fxr::read(&member.bytes) {
            Ok(fx) => f.effects.push((id, fx.spawned(), path.to_owned())),
            Err(e) => f.report.failures.push(format!("{path}: effect {id}: {e}")),
        }
    }
    Ok(())
}

/// The event files of an animation binder (and of the animation binders inside it): each event field the template names
/// `FFX ID`.
fn events(
    bytes: &[u8],
    decoder: &Decoder<'_>,
    templates: &Templates,
    effect_fields: &BTreeMap<i32, Vec<(usize, String)>>,
    f: &mut Found,
) -> Result<()> {
    let mut todo = vec![Binder::read(bytes, decoder)?];
    while let Some(binder) = todo.pop() {
        for member in &binder.files {
            if member.bytes.starts_with(b"BND4") {
                todo.push(Binder::read(&member.bytes, decoder)?);
                continue;
            }
            let is_tae = member
                .name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".tae"));
            if !is_tae {
                continue;
            }
            let tae = Tae::read_member(&binder, member)?;
            f.report.count("animation event files", 1);
            for animation in &tae.animations {
                for event in &animation.events {
                    // an event of a kind the template does not define may name an effect the census cannot see
                    let Some(template) = templates.get(event.kind) else {
                        f.report.count(
                            &format!(
                                "animation events of kind {} the template does not define",
                                event.kind
                            ),
                            1,
                        );
                        continue;
                    };
                    let Some(fields) = effect_fields.get(&event.kind) else {
                        continue;
                    };
                    let decoded = template.decode(&event.parameters)?;
                    for (index, label) in fields {
                        match decoded
                            .arguments
                            .iter()
                            .find(|a| a.index == *index)
                            .map(|a| &a.value)
                        {
                            Some(Value::Int(id)) => f.names.push((EVENTS, label.clone(), *id)),
                            Some(_) => f.report.count(&format!("{label}: records holding no integer"), 1),
                            None => f
                                .report
                                .count(&format!("{label}: records stored too short to hold it"), 1),
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// A map layout: its SFX and wind SFX regions.
fn layout(bytes: &[u8], f: &mut Found) -> Result<()> {
    let layout = MapLayout::read(bytes)?;
    f.report.count("map layouts", 1);
    for region in layout.map_regions()? {
        if let Some(sfx) = layout.sfx_region(&region)? {
            f.names
                .push((MAPS, "map SFX region".to_owned(), i64::from(sfx.effect_id)));
        }
        if let Some(wind) = layout.wind_sfx_region(&region)? {
            f.names
                .push((MAPS, "map wind SFX region".to_owned(), i64::from(wind.effect_id)));
        }
    }
    Ok(())
}

/// An event script: each instruction argument named `... SFX ID`, literal or filled from the event's arguments, and every
/// initialiser (the event it starts and the words it passes).
fn script(game: Game, bytes: &[u8], f: &mut Found) -> Result<()> {
    let script = emevd::Script::read(bytes)?;
    f.report.count("event scripts", 1);
    for event in &script.events {
        let typed = emedf::decode_event(game, event);
        for t in typed.iter().flatten() {
            if let Some(init) = t.initialisation() {
                f.initialisations.push((i64::from(init.event), init.words));
            }
            for arg in &t.args {
                if !arg.def.name.contains("SFX ID") || arg.substituted {
                    continue;
                }
                if let Some(id) = arg.value.as_i64() {
                    f.names.push((
                        SCRIPTS,
                        format!("event script {:?} argument {:?}", t.def.name, arg.def.name),
                        id,
                    ));
                }
            }
        }
        for u in emedf::argument_uses(game, event) {
            let Some(target) = u.target else { continue };
            if !target.name.contains("SFX ID") {
                continue;
            }
            let name = typed
                .get(u.instruction)
                .and_then(|t| t.as_ref().ok())
                .map_or("?", |t| t.def.name);
            f.script_uses.push((
                event.id,
                u.source,
                format!("event script {name:?} argument {:?}", target.name),
            ));
        }
    }
    Ok(())
}

/// The param rows: each value of each field that names effects by its values (the rule above), as (kind, label, id).
/// A table the reader or the paramdefs refuse is a failure; the census goes on without it.
#[allow(clippy::too_many_arguments)]
fn params(
    game: Game,
    root: &std::path::Path,
    keys: &std::path::Path,
    archive: &fromsoft_formats::Archive,
    decoder: &Decoder<'_>,
    defs_dir: &std::path::Path,
    meta_dir: Option<&std::path::Path>,
    holders: &BTreeMap<i32, Vec<String>>,
    report: &mut Report,
) -> Result<Vec<(u8, String, i64)>> {
    let install = Install::new(root, game, Keys::from_folder(keys, game, None)?);
    let binder = param_binder(&install, archive, decoder)?;
    let mut files: Vec<_> = std::fs::read_dir(defs_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();
    let texts = files
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?;
    let defs = ParamDefs::from_xml(texts.iter().map(String::as_str))?;
    // the param meta's effect fields by param type: a meta file is named as its paramdef is
    let meta: Option<BTreeMap<String, BTreeSet<String>>> = match meta_dir {
        None => None,
        Some(dir) => {
            let by_stem = meta_effect_fields(dir)?;
            let mut by_type = BTreeMap::new();
            for (file, text) in files.iter().zip(&texts) {
                let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
                if let (Some(kind), Some(fields)) = (element_text(text, "ParamType"), by_stem.get(stem)) {
                    by_type.insert(kind.to_owned(), fields.clone());
                }
            }
            Some(by_type)
        }
    };
    // the meta's effect fields of the tables read, as `param <table>.<field>`, and the fields the tables have
    let mut meta_named: BTreeSet<String> = BTreeSet::new();
    let mut table_fields: BTreeSet<String> = BTreeSet::new();
    // each integer field of each table: how many rows hold each value
    let mut fields: BTreeMap<String, BTreeMap<i64, u64>> = BTreeMap::new();
    for member in &binder.files {
        let Some(name) = member.name.as_deref() else {
            continue;
        };
        let leaf = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let Some(table) = leaf.strip_suffix(".param") else {
            continue;
        };
        let stored = match Param::read(&member.bytes) {
            Ok(stored) => stored,
            Err(e) => {
                report.failures.push(format!("param {table}: {e}"));
                continue;
            }
        };
        let typed = match defs.bind(&stored, binder.regulation_number()) {
            Ok(t) => t,
            Err(e) => {
                report.failures.push(format!("param {table}: {e}"));
                continue;
            }
        };
        report.count("param tables", 1);
        if let Some(named) = meta.as_ref().and_then(|m| m.get(&stored.param_type)) {
            meta_named.extend(named.iter().map(|f| format!("param {table}.{f}")));
        }
        for row in typed.rows() {
            for (field, value) in row.fields() {
                table_fields.insert(format!("param {table}.{field}"));
                if let Some(v) = value.as_i64() {
                    *fields
                        .entry(format!("param {table}.{field}"))
                        .or_default()
                        .entry(v)
                        .or_default() += 1;
                }
            }
        }
    }
    let is_effect = |v: i64| i32::try_from(v).is_ok_and(|v| holders.contains_key(&v));
    let mut named = Vec::new();
    const META: &str = "param meta effect fields (an ExtRefs to an .fxr) of the tables read";
    if meta.is_some() {
        for label in meta_named.iter().filter(|l| !table_fields.contains(*l)) {
            report.count(&format!("{META}, naming no field of the table's def: {label}"), 1);
        }
    }
    for (label, values) in fields {
        let large = values.keys().filter(|&&v| v >= 1000).count();
        let hits = values.keys().filter(|&&v| v >= 1000 && is_effect(v)).count();
        let by_values = hits >= 3 && hits * 2 >= large;
        let lower = label.to_ascii_lowercase();
        let by_name = lower.contains("sfx") || lower.contains("ffx");
        let taken = by_values && by_name;
        if meta.is_some() {
            match (meta_named.contains(&label), taken) {
                (true, true) => report.count(&format!("{META}: taken by the rule too"), 1),
                (true, false) if large == 0 => {
                    report.count(&format!("{META}, left by the rule: no value of 1,000 or more"), 1);
                }
                (true, false) if hits < 3 => report.count(
                    &format!("{META}, left by the rule: {hits} of its values of 1,000 or more are effect files"),
                    1,
                ),
                (true, false) => report.count(
                    &format!(
                        "{META}, left by the rule: effect files under half of its values of 1,000 or more: {label} ({hits} of {large})"
                    ),
                    1,
                ),
                (false, true) if hits != large => report.count(
                    &format!(
                        "param fields the rule takes beyond the param meta that hold values of 1,000 or more no effect file has: {label} ({hits} of {large})"
                    ),
                    1,
                ),
                (false, true) => report.count(
                    &format!(
                        "param fields the rule takes that the param meta does not open as effect files: {label} ({hits} of {large})"
                    ),
                    1,
                ),
                (false, false) => {}
            }
        }
        if by_values && !by_name {
            report.count(
                &format!(
                    "param fields whose values match effect files but whose name does not say sfx or ffx: {label} ({hits} of {large})"
                ),
                1,
            );
        }
        if !taken {
            continue;
        }
        report.count("param fields that name effects", 1);
        for (v, rows) in values {
            named.extend(std::iter::repeat_n((PARAMS, label.clone(), v), rows as usize));
        }
    }
    Ok(named)
}

/// The fields each param meta file (`<paramdef>.xml`, UTF-16 or UTF-8) opens as effect files: the elements of its
/// `Field` list whose `ExtRefs` names an `.fxr` path, by the file's stem.
fn meta_effect_fields(dir: &std::path::Path) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "xml") {
            continue;
        }
        let bytes = std::fs::read(&path)?;
        let (text, _) = match bytes.get(..2) {
            Some([0xff, 0xfe]) => encoding_rs::UTF_16LE.decode_with_bom_removal(&bytes),
            _ => encoding_rs::UTF_8.decode_with_bom_removal(&bytes),
        };
        let list = text
            .split_once("<Field>")
            .and_then(|(_, rest)| rest.split_once("</Field>"))
            .map(|(list, _)| list)
            .unwrap_or_default();
        let mut fields = BTreeSet::new();
        for tag in list.split('<').skip(1) {
            let tag = tag.split_once('>').map_or(tag, |(t, _)| t);
            let (name, attributes) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
            let refs = attributes
                .split_once("ExtRefs=\"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(refs, _)| refs);
            if refs.is_some_and(|r| r.to_ascii_lowercase().contains(".fxr")) {
                fields.insert(name.to_owned());
            }
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        out.insert(stem, fields);
    }
    Ok(out)
}

/// The text of the first `<name>...</name>` element of an XML text.
fn element_text<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let (_, rest) = text.split_once(&format!("<{name}>"))?;
    rest.split_once(&format!("</{name}>")).map(|(t, _)| t.trim())
}
