//! Every action of every effect file of both installs (2,913 files in Sekiro's 8 `sfxbnd` binders, 12,155 in Elden Ring's 220)
//! read as its typed struct ([`fromsoft_formats::fxr::actions`]): no action of a type the files hold is refused, the typed
//! reading counts each type as an independent walker (a stdlib Python reader of the on-disk layout) does, and what the typed
//! structs do not name (words and properties past the fxr project's names) is counted per type.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game
//! that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::fxr::actions::{NAMES, Typed};
use fromsoft_formats::fxr::resources::{ResourceKind, Resources, read_resource_list};
use fromsoft_formats::fxr::{self, Fxr, enums};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Actions of each type over all files, from the independent walker (`census_actions.py`).
const SEKIRO_ACTIONS: &[(i16, usize)] = &[
    (0, 111431),
    (1, 10794),
    (15, 295),
    (34, 582),
    (35, 20878),
    (36, 248),
    (46, 136),
    (55, 4196),
    (60, 1014),
    (64, 430),
    (65, 431),
    (75, 947),
    (83, 28),
    (84, 988),
    (105, 2616),
    (106, 1993),
    (113, 26),
    (120, 39),
    (121, 41),
    (122, 72),
    (128, 21825),
    (129, 21248),
    (130, 21248),
    (131, 21248),
    (132, 543),
    (133, 125),
    (199, 20916),
    (200, 523),
    (201, 54),
    (300, 8740),
    (301, 1512),
    (399, 11573),
    (400, 17690),
    (401, 554),
    (402, 65),
    (403, 451),
    (404, 2640),
    (405, 425),
    (500, 11905),
    (501, 9549),
    (502, 23),
    (503, 348),
    (600, 587),
    (601, 29),
    (602, 103),
    (603, 7638),
    (604, 1600),
    (605, 1010),
    (606, 282),
    (607, 167),
    (608, 485),
    (609, 834),
    (732, 520),
    (733, 1),
    (734, 91),
    (10000, 47),
    (10012, 2123),
    (10013, 51),
    (10100, 2910),
    (10300, 76),
    (10301, 1),
    (10400, 2910),
    (10500, 2910),
    (11000, 13),
];
/// Actions whose typed form holds something the struct does not name, by type, from this test's first run: the state-config
/// map and the random selector keep their field lists in `Extra::field_lists` (by design), three Sekiro billboards in Dark
/// Souls III's layout have one word past its names, and two Elden Ring distortions have words past the project's.
const SEKIRO_EXTRA: &[(i16, usize)] = &[(199, 20916), (201, 54), (603, 3)];
const ELDEN_EXTRA: &[(i16, usize)] = &[(199, 88403), (201, 94), (607, 2)];
const ELDEN_ACTIONS: &[(i16, usize)] = &[
    (0, 480016),
    (1, 35506),
    (11, 33),
    (15, 869),
    (34, 1481),
    (35, 81215),
    (36, 1104),
    (46, 124),
    (55, 17436),
    (60, 4336),
    (64, 1431),
    (65, 2498),
    (75, 3519),
    (83, 141),
    (84, 3254),
    (105, 10364),
    (106, 2438),
    (113, 117),
    (120, 84),
    (121, 185),
    (122, 53),
    (123, 24),
    (128, 88805),
    (129, 86669),
    (130, 86669),
    (131, 86669),
    (132, 5228),
    (133, 920),
    (199, 88403),
    (200, 2042),
    (201, 94),
    (300, 38788),
    (301, 2667),
    (399, 47350),
    (400, 70747),
    (401, 2363),
    (402, 84),
    (403, 2202),
    (404, 11682),
    (405, 1727),
    (500, 48404),
    (501, 38617),
    (502, 300),
    (503, 1484),
    (600, 3375),
    (601, 6),
    (602, 2424),
    (603, 17392),
    (604, 13852),
    (605, 1493),
    (606, 122),
    (607, 1359),
    (608, 1338),
    (609, 4999),
    (700, 10884),
    (701, 288),
    (702, 844),
    (731, 14),
    (732, 923),
    (734, 535),
    (10000, 99),
    (10001, 5),
    (10012, 6410),
    (10013, 186),
    (10014, 97),
    (10015, 3052),
    (10100, 12005),
    (10300, 106),
    (10400, 12005),
    (10500, 12005),
    (11000, 17),
];

/// Per action type: the stored (properties 1, properties 2, field words 1, field words 2) and how many actions store it.
type Layouts = BTreeMap<i16, BTreeMap<(usize, usize, usize, usize), usize>>;

#[derive(Default)]
struct Totals {
    files: usize,
    actions: BTreeMap<i16, usize>,
    refused: BTreeMap<String, usize>,
    /// per type: actions whose typed form has something the struct does not name
    with_extra: BTreeMap<i16, usize>,
    /// enum-typed parameters: (enum, value) -> count, over the constant ones
    enum_values: BTreeMap<(String, i32), usize>,
    /// enum-typed parameters that are not constants (animated blend modes and the like)
    enum_animated: usize,
    /// enum values the fxr project names no member for: (action, parameter, enum, value) -> count
    enum_unknown: BTreeMap<(i16, String, String, i32), usize>,
    /// the resource members of every binder, by binder stem
    resources: BTreeMap<String, Resources>,
    /// the numbers the typed actions name, by (binder stem, kind): (positive ids, animated parameters, unset ones)
    named: BTreeMap<(String, ResourceKind), (BTreeSet<i32>, usize, usize)>,
    /// the entries of the resource lists (Elden Ring) by binder stem
    listed: Vec<(String, fxr::resources::Listed)>,
    /// modifiers by (type number, components), read as typed
    modifiers: BTreeMap<(u16, usize), usize>,
    /// per type: the distinct (stored properties1, properties2, fields1, fields2) lists
    layouts: Layouts,
}

#[test]
fn every_action_of_every_effect_file_reads_as_its_typed_struct() -> Result<()> {
    for (game, data, expected, expected_extra) in [
        (Game::Sekiro, Data::Sekiro, SEKIRO_ACTIONS, SEKIRO_EXTRA),
        (Game::EldenRing, Data::Elden, ELDEN_ACTIONS, ELDEN_EXTRA),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
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
        let mut t = Totals::default();
        for path in paths {
            let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
            let stem = fromsoft_formats::names::stem(path).to_owned();
            let index = Resources::index(&binder)?;
            for member in &binder.files {
                let name = member.name.clone().unwrap_or_default();
                if name.to_ascii_lowercase().ends_with(".ffxreslist") {
                    for entry in
                        read_resource_list(&member.bytes).map_err(|e| format!("{path} {name}: {e}"))?
                    {
                        t.listed.push((stem.clone(), entry));
                    }
                }
            }
            t.resources.insert(stem.clone(), index);
            for member in &binder.files {
                let name = member.name.clone().unwrap_or_default();
                if fxr::member_id(&name).is_none() {
                    continue;
                }
                let read = match Fxr::read(&member.bytes) {
                    Ok(f) => f,
                    Err(e) => return Err(format!("{path} {name}: {e}").into()),
                };
                t.files += 1;
                for a in read.actions() {
                    *t.actions.entry(a.id).or_default() += 1;
                    if a.id == 0 {
                        continue;
                    }
                    *t.layouts
                        .entry(a.id)
                        .or_default()
                        .entry((
                            a.properties1.len(),
                            a.properties2.len(),
                            a.fields1.len(),
                            a.fields2.len(),
                        ))
                        .or_default() += 1;
                    for p in a.properties1.iter().chain(&a.properties2) {
                        for m in &p.modifiers {
                            match m.kind() {
                                Ok(_) => {
                                    *t.modifiers.entry((m.type_number(), m.components())).or_default() += 1
                                }
                                Err(e) => return Err(format!("{path} {name}: action {}: {e}", a.id).into()),
                            }
                        }
                    }
                    match Typed::read(a, game) {
                        Ok(typed) => {
                            for r in typed.resources() {
                                let e = t.named.entry((stem.clone(), r.kind)).or_default();
                                match r.id {
                                    None => e.1 += 1,
                                    Some(id) if id > 0 => {
                                        e.0.insert(id);
                                    }
                                    Some(_) => e.2 += 1,
                                }
                            }
                            for (param, enum_name, value) in typed.enum_values() {
                                match value {
                                    None => t.enum_animated += 1,
                                    Some(v) => {
                                        *t.enum_values.entry((enum_name.to_owned(), v)).or_default() += 1;
                                        if enums::member_name(enum_name, v).is_none() {
                                            *t.enum_unknown
                                                .entry((a.id, param.to_owned(), enum_name.to_owned(), v))
                                                .or_default() += 1;
                                        }
                                    }
                                }
                            }
                            if !typed.extra().is_empty() {
                                *t.with_extra.entry(a.id).or_default() += 1;
                            }
                        }
                        Err(e) => {
                            let reason = format!(
                                "{} {}: {e}",
                                a.id,
                                NAMES.iter().find(|n| n.0 == a.id).map_or("?", |n| n.1)
                            );
                            if !t.refused.contains_key(&reason) {
                                println!("REFUSED {path} {name}: {reason}");
                            }
                            *t.refused.entry(reason).or_default() += 1;
                        }
                    }
                }
            }
        }
        // the numbers the effects name, found in their own binder, else in the common ones, else in any other
        let common = ["sfxbnd_commoneffects.ffxbnd", "sfxbnd_commoneffects_dlc02.ffxbnd"];
        let has = |res: &Resources, kind: ResourceKind, id: i32| match kind {
            ResourceKind::Texture(k) => res.texture(game, k, id).is_some(),
            ResourceKind::Model => res.model(id).is_some(),
            ResourceKind::Animation => res.animation(id).is_some(),
            ResourceKind::Sound => true,
        };
        // per kind: [own binder, common binder, other binder, nowhere], then animated and unset parameters
        let mut tally: BTreeMap<ResourceKind, [usize; 6]> = BTreeMap::new();
        for ((stem, kind), (ids, animated, unset)) in &t.named {
            let row = tally.entry(*kind).or_default();
            row[4] += animated;
            row[5] += unset;
            for &id in ids {
                let slot = if has(&t.resources[stem], *kind, id) {
                    0
                } else if common
                    .iter()
                    .any(|c| t.resources.get(*c).is_some_and(|r| has(r, *kind, id)))
                {
                    1
                } else if t.resources.values().any(|r| has(r, *kind, id)) {
                    2
                } else {
                    3
                };
                row[slot] += 1;
            }
        }
        println!(
            "  numbers named by (binder, kind): found in [own binder, common binder, other binder, nowhere, animated parameters, unset]: {tally:?}"
        );
        let mut listed = [0usize; 4];
        for (stem, entry) in &t.listed {
            let (kind, id) = match entry {
                fxr::resources::Listed::Texture(id, suffix) => {
                    let k = [
                        ResourceKind::Texture(fxr::resources::TextureKind::Albedo),
                        ResourceKind::Texture(fxr::resources::TextureKind::Normal),
                        ResourceKind::Texture(fxr::resources::TextureKind::Specular),
                        ResourceKind::Texture(fxr::resources::TextureKind::Detail),
                    ]
                    .into_iter()
                    .find(|k| matches!(k, ResourceKind::Texture(tk) if tk.suffix(game) == suffix));
                    match k {
                        Some(k) => (k, *id),
                        None => {
                            listed[3] += 1;
                            continue;
                        }
                    }
                }
                fxr::resources::Listed::Model(id) => (ResourceKind::Model, *id),
                fxr::resources::Listed::Animation(id) | fxr::resources::Listed::Skeleton(id) => {
                    (ResourceKind::Animation, *id)
                }
                fxr::resources::Listed::Asset(_) => {
                    listed[3] += 1;
                    continue;
                }
            };
            let slot = if has(&t.resources[stem], kind, id) {
                0
            } else if common
                .iter()
                .any(|c| t.resources.get(*c).is_some_and(|r| has(r, kind, id)))
            {
                1
            } else {
                2
            };
            listed[slot] += 1;
        }
        println!(
            "  resource list entries ({}): found in [own binder, common binder, nowhere, a suffix no kind has]: {listed:?}",
            t.listed.len()
        );
        println!("{game:?}: {} files", t.files);
        for (id, n) in &t.with_extra {
            println!(
                "  extra: action {id}: {n} of {} actions name something the struct does not",
                t.actions[id]
            );
        }
        println!("  modifiers (type, components): {:?}", t.modifiers);
        println!(
            "  enum values: {:?}; animated enum parameters {}",
            t.enum_values, t.enum_animated
        );
        println!(
            "  enum values the project names no member for: {:?}",
            t.enum_unknown
        );
        for (id, l) in &t.layouts {
            println!("  layouts of {id}: {l:?}");
        }
        assert!(t.refused.is_empty(), "{:?}", t.refused);
        let want: BTreeMap<i16, usize> = expected.iter().copied().collect();
        assert_eq!(
            t.actions, want,
            "{game:?}: actions per type against the independent walker"
        );
        let want_extra: BTreeMap<i16, usize> = expected_extra.iter().copied().collect();
        assert_eq!(
            t.with_extra, want_extra,
            "{game:?}: actions with words the structs do not name"
        );
        // every type the files hold is typed
        let typed: BTreeSet<i16> = NAMES.iter().map(|n| n.0).collect();
        for id in t.actions.keys().filter(|&&i| i != 0) {
            assert!(typed.contains(id), "{game:?}: action {id} has no typed struct");
        }
    }
    Ok(())
}
