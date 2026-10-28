//! The community's event templates (Smithbox's `TAE.Template.SDT.xml` and `TAE.Template.ER.xml`) against every event of every
//! distinct event file (`.tae`) of the animation binders of the installs (`/chr/*.anibnd.dcx` and the animation binders inside
//! the object, part and asset binders): how many events have a template, whether the asserted fields hold their values,
//! which events are stored shorter than their template, which float fields store something that is not a float, and how
//! many stored values the templates' named values do not cover.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (a TAE walker and a template
//! reader written from the file layout and the XML alone).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt` and
//! `oracles.smithbox.{sekiro,elden}/tae-template.xml`; a game that is not configured is skipped.
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::{
    Tae,
    template::{Templates, Value},
};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Query, Tool, expect_found, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// What a game's events hold, read through its templates.
#[derive(Default)]
struct Totals {
    files: usize,
    events: usize,
    with_template: usize,
    without_template: BTreeMap<i32, usize>,
    kinds_in_data: BTreeSet<i32>,
    asserted: usize,
    /// Events stored shorter than their template, and of those the ones lacking a field that is neither asserted zero nor
    /// padding.
    short: usize,
    short_with_a_named_field_missing: usize,
    trailing_nonzero: usize,
    /// Float fields that store a word that is not a finite float, by (event type, field index).
    words: BTreeMap<(i32, usize), usize>,
    /// Fields that list named values, read, and the ones whose value no entry names, by (event type, field index).
    listed: usize,
    unlisted: BTreeMap<(i32, usize), usize>,
}

fn census(game: Game, data: Data) -> Result<Option<(Totals, Templates)>> {
    let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(template)) = (
        install(data),
        keys(data),
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
        oracle(OracleSet::Smithbox, data, "tae-template.xml"),
    ) else {
        return Ok(None);
    };
    let templates = Templates::parse(&std::fs::read_to_string(template)?)?;
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
    let dictionary = NameDictionary::read_for(dictionary, game)?;
    let paths: BTreeSet<&str> = dictionary
        .names
        .values()
        .flatten()
        .map(String::as_str)
        .filter(|p| {
            [".anibnd.dcx", ".objbnd.dcx", ".partsbnd.dcx", ".geombnd.dcx"]
                .iter()
                .any(|suffix| p.ends_with(suffix))
        })
        .filter(|p| archive.contains(p))
        .collect();
    let mut t = Totals::default();
    let mut distinct = BTreeSet::new();
    for path in &paths {
        let mut todo = vec![Binder::read(&archive.read(path, &decoder)?, &decoder)?];
        while let Some(binder) = todo.pop() {
            for member in &binder.files {
                if member.bytes.starts_with(b"BND4") {
                    todo.push(Binder::read(&member.bytes, &decoder)?);
                }
                let named_tae = member
                    .name
                    .as_deref()
                    .is_some_and(|n| n.to_ascii_lowercase().ends_with(".tae"));
                if !named_tae {
                    continue;
                }
                let tae = Tae::read_member(&binder, member)?;
                if !distinct.insert(Sha256::digest(&member.bytes).to_vec()) {
                    continue;
                }
                t.files += 1;
                for animation in &tae.animations {
                    for event in &animation.events {
                        t.events += 1;
                        t.kinds_in_data.insert(event.kind);
                        let Some(template) = templates.get(event.kind) else {
                            *t.without_template.entry(event.kind).or_default() += 1;
                            continue;
                        };
                        t.with_template += 1;
                        let decoded = template.decode(&event.parameters)?;
                        t.trailing_nonzero += usize::from(decoded.trailing_nonzero);
                        if !decoded.missing.is_empty() {
                            t.short += 1;
                            t.short_with_a_named_field_missing +=
                                usize::from(decoded.missing.iter().any(|f| !f.is_padding()));
                        }
                        for a in &decoded.arguments {
                            t.asserted += usize::from(a.field.assert.is_some());
                            match &a.value {
                                Value::Word(_) => {
                                    *t.words.entry((event.kind, a.index)).or_default() += 1;
                                }
                                Value::Int(v) if !a.field.entries.is_empty() => {
                                    t.listed += 1;
                                    if a.field.entry(*v).is_none() {
                                        *t.unlisted.entry((event.kind, a.index)).or_default() += 1;
                                    }
                                }
                                Value::Bool(_) | Value::Int(_) | Value::Unsigned(_) | Value::Float(_) | Value::Bytes(_) => {}
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Some((t, templates)))
}

#[test]
fn sekiros_events_through_the_community_templates() -> Result<()> {
    let Some((t, templates)) = census(Game::Sekiro, Data::Sekiro)? else {
        return Ok(());
    };
    println!(
        "{} event files, {} events, {} with a template; asserted fields {}; short {} ({} lacking a named field); \
         unlisted values {:?} of {} listed",
        t.files, t.events, t.with_template, t.asserted, t.short, t.short_with_a_named_field_missing, t.unlisted, t.listed
    );
    expect_found(Query::Install(Data::Sekiro, None), "event files", t.files);
    assert_eq!((t.files, t.events, t.with_template), (326, 337_924, 337_847));
    assert_eq!(
        t.without_template,
        BTreeMap::from([(340, 57), (341, 18), (350, 2)])
    );
    assert_eq!(t.kinds_in_data.len(), 109);
    assert_eq!(templates.len(), 190);
    assert_eq!(
        templates.iter().filter(|e| e.name.is_some()).count(),
        148,
        "events the template names"
    );
    assert_eq!(
        templates
            .iter()
            .filter(|e| t.kinds_in_data.contains(&e.kind))
            .count(),
        106
    );
    // every asserted field holds its value (decode refuses one that does not), every boolean is 0 or 1
    assert_eq!(t.asserted, 553_355);
    assert_eq!((t.short, t.short_with_a_named_field_missing), (4_627, 317));
    assert_eq!(t.trailing_nonzero, 0);
    // float fields that store a negative integer: events 708 (fields 3, 10 and 12: -1), 730 and 156
    assert_eq!(
        t.words,
        BTreeMap::from([
            ((156, 0), 39),
            ((156, 1), 27),
            ((708, 3), 1_497),
            ((708, 10), 1_497),
            ((708, 12), 1_497),
            ((730, 0), 419),
            ((730, 1), 429),
        ])
    );
    assert_eq!(t.listed, 215_388);
    assert_eq!(t.unlisted.values().sum::<usize>(), 13_142);
    assert_eq!(t.unlisted.len(), 5);
    assert_eq!(t.unlisted.get(&(0, 0)), Some(&6_826));
    assert_eq!(t.unlisted.get(&(700, 5)), Some(&6_261));
    Ok(())
}

#[test]
fn elden_rings_events_through_the_community_templates() -> Result<()> {
    let Some((t, templates)) = census(Game::EldenRing, Data::Elden)? else {
        return Ok(());
    };
    println!(
        "{} event files, {} events, {} with a template; asserted fields {}; short {} ({} lacking a named field); \
         unlisted values {:?} of {} listed",
        t.files, t.events, t.with_template, t.asserted, t.short, t.short_with_a_named_field_missing, t.unlisted, t.listed
    );
    expect_found(Query::Install(Data::Elden, None), "event files", t.files);
    assert_eq!((t.files, t.events, t.with_template), (1_071, 1_313_057, 1_313_057));
    assert!(t.without_template.is_empty());
    assert_eq!(t.kinds_in_data.len(), 124);
    assert_eq!(templates.len(), 156);
    assert_eq!(templates.iter().filter(|e| e.name.is_some()).count(), 141);
    assert_eq!(
        templates
            .iter()
            .filter(|e| t.kinds_in_data.contains(&e.kind))
            .count(),
        124
    );
    assert_eq!(t.asserted, 2_050_123);
    assert_eq!((t.short, t.short_with_a_named_field_missing), (268, 63));
    assert_eq!(t.trailing_nonzero, 0);
    assert!(t.words.is_empty(), "{:?}", t.words);
    assert_eq!(t.listed, 1_867_190);
    assert_eq!(t.unlisted.values().sum::<usize>(), 4_069);
    assert_eq!(t.unlisted.len(), 8);
    assert_eq!(t.unlisted.get(&(700, 5)), Some(&2_800));
    assert_eq!(t.unlisted.get(&(1, 1)), Some(&694));
    Ok(())
}
