//! The community's event templates (Smithbox's `TAE.Template.SDT.xml` and `TAE.Template.ER.xml`) against every event of every
//! distinct event file (`.tae`) of the animation binders of the installs (`/chr/*.anibnd.dcx` and the animation binders inside
//! the object, part and asset binders): how many events have a template, whether the asserted fields hold their values,
//! which events are stored shorter than their template, which float fields store something that is not a float, and how
//! many stored values the templates' named values do not cover.
//!
//! The templates are held to a second source, soulstruct's table of the event types of the Dark Souls Remastered and III era
//! (`oracles.soulstruct.elden/tae-event-classes.json`: for each type it lists, the name and the fields of the data): which types
//! both name, whether a field of one straddles the edge of a field of the other, which fields they type as an integer and a
//! float, and, over the stored events, where a type's data holds a nonzero byte outside the fields soulstruct lists and which of
//! the words the two disagree on are floats.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (a TAE walker and a template
//! reader written from the file layout and the XML alone).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt` and
//! `oracles.smithbox.{sekiro,elden}/tae-template.xml`; a game that is not configured is skipped, and the second source's
//! checks need `oracles.soulstruct.elden/tae-event-classes.json` (the comparison of the two tables alone needs no install).
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::{
    Tae,
    template::{FieldKind, Templates, Value},
};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Query, Tool, expect_found, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
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
    /// What soulstruct's table says of the events, where its oracle is configured.
    second: Option<SecondTotals>,
}

/// How soulstruct types a field, and the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Int,
    Float,
    Flag,
}

fn soulstruct_kind(ty: &str) -> Kind {
    match ty {
        "float" => Kind::Float,
        "bool" => Kind::Flag,
        _ => Kind::Int,
    }
}

fn template_kind(kind: FieldKind) -> Kind {
    match kind {
        FieldKind::F32 | FieldKind::F64 => Kind::Float,
        FieldKind::Bool => Kind::Flag,
        _ => Kind::Int,
    }
}

/// One field of a class in soulstruct's table.
struct Slot {
    offset: usize,
    width: usize,
    ty: String,
    pad: bool,
}

/// One event type soulstruct lists: its enum name, and the fields of its class (none where it has no class).
struct Listed {
    name: String,
    fields: Option<Vec<Slot>>,
}

fn soulstruct_events(path: &Path) -> Result<BTreeMap<i32, Listed>> {
    let table: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut events = BTreeMap::new();
    for (kind, event) in table["events"].as_object().ok_or("events")? {
        let fields = match event["fields"].as_array() {
            None => None,
            Some(list) => {
                let mut slots = Vec::new();
                for f in list {
                    slots.push(Slot {
                        offset: usize::try_from(f[0].as_u64().ok_or("offset")?)?,
                        width: usize::try_from(f[1].as_u64().ok_or("width")?)?,
                        ty: f[2].as_str().ok_or("type")?.to_owned(),
                        pad: f[4].as_bool().ok_or("pad")?,
                    });
                }
                Some(slots)
            }
        };
        events.insert(
            kind.parse()?,
            Listed {
                name: event["name"].as_str().ok_or("name")?.to_owned(),
                fields,
            },
        );
    }
    Ok(events)
}

/// Soulstruct's table held to a game's templates.
#[derive(Default)]
struct Comparison {
    /// Event types both list.
    shared: usize,
    only_soulstruct: Vec<i32>,
    only_template: usize,
    /// Shared types soulstruct has a class for, and the ones it gives a name (not `UnkNNN`).
    with_class: usize,
    named: usize,
    /// Fields of the two at the same offset and width (padding aside), and of those the ones typed differently: (event type,
    /// offset, width, soulstruct's kind, the template's).
    compared: usize,
    differing: Vec<(i32, usize, usize, Kind, Kind)>,
    /// Types with a field of one that straddles the edge of a field of the other (a field inside one is a refinement).
    overlapping: BTreeSet<i32>,
}

fn compare(templates: &Templates, events: &BTreeMap<i32, Listed>) -> Comparison {
    let mut c = Comparison {
        only_template: templates.iter().filter(|t| !events.contains_key(&t.kind)).count(),
        ..Comparison::default()
    };
    for (&kind, listed) in events {
        let Some(template) = templates.get(kind) else {
            c.only_soulstruct.push(kind);
            continue;
        };
        c.shared += 1;
        c.named += usize::from(!listed.name.starts_with("Unk"));
        let Some(slots) = &listed.fields else {
            continue;
        };
        c.with_class += 1;
        let mut theirs = Vec::new();
        let mut at = 0;
        for field in &template.fields {
            if !field.is_padding() {
                theirs.push((at, field.kind.width(), template_kind(field.kind)));
            }
            at += field.kind.width();
        }
        for slot in slots.iter().filter(|s| !s.pad) {
            let ours = soulstruct_kind(&slot.ty);
            for &(offset, width, kind_there) in &theirs {
                let (end, end_there) = (slot.offset + slot.width, offset + width);
                let overlap = slot.offset < end_there && offset < end;
                let nested = (slot.offset <= offset && end_there <= end)
                    || (offset <= slot.offset && end <= end_there);
                if overlap && !nested {
                    c.overlapping.insert(kind);
                }
                if (offset, width) == (slot.offset, slot.width) {
                    c.compared += 1;
                    if ours != kind_there {
                        c.differing.push((kind, offset, width, ours, kind_there));
                    }
                }
            }
        }
    }
    c
}

/// Soulstruct's table with what the comparison with a game's templates picked out of it.
struct Second {
    events: BTreeMap<i32, Listed>,
    only_soulstruct: BTreeSet<i32>,
    /// (event type, offset) of the 4-byte fields one source types as a float and the other as an integer.
    floats: BTreeSet<(i32, usize)>,
}

impl Second {
    fn new(templates: &Templates, events: BTreeMap<i32, Listed>) -> Self {
        let c = compare(templates, &events);
        Self {
            only_soulstruct: c.only_soulstruct.iter().copied().collect(),
            floats: c
                .differing
                .iter()
                .filter(|d| d.2 == 4 && (d.3 == Kind::Float || d.4 == Kind::Float))
                .map(|d| (d.0, d.1))
                .collect(),
            events,
        }
    }
}

/// What the stored events say about soulstruct's table.
#[derive(Default)]
struct SecondTotals {
    /// Events whose type soulstruct lists, and of those the ones whose type it has a class for.
    listed: usize,
    with_class: usize,
    /// Types soulstruct lists and the template does not that occur in the data.
    only_soulstruct_seen: BTreeSet<i32>,
    /// By event type with a class: its events, and the ones with a nonzero byte outside the fields the class lists.
    outside: BTreeMap<i32, (usize, usize)>,
    /// By (event type, offset) of a 4-byte field the sources type as a float and an integer: the stored words that are zero,
    /// a finite float (1e-20 to 1e12 in magnitude), and something else.
    words: BTreeMap<(i32, usize), [usize; 3]>,
}

impl SecondTotals {
    fn observe(&mut self, second: &Second, kind: i32, parameters: &[u8]) {
        let Some(listed) = second.events.get(&kind) else {
            return;
        };
        self.listed += 1;
        if second.only_soulstruct.contains(&kind) {
            self.only_soulstruct_seen.insert(kind);
        }
        let Some(fields) = &listed.fields else {
            return;
        };
        self.with_class += 1;
        let outside = parameters.iter().enumerate().any(|(at, &byte)| {
            byte != 0
                && !fields
                    .iter()
                    .any(|f| !f.pad && (f.offset..f.offset + f.width).contains(&at))
        });
        let seen = self.outside.entry(kind).or_default();
        seen.0 += 1;
        seen.1 += usize::from(outside);
        for &(_, offset) in second.floats.iter().filter(|(k, _)| *k == kind) {
            let Some(word) = parameters
                .get(offset..offset + 4)
                .and_then(|b| <[u8; 4]>::try_from(b).ok())
            else {
                continue;
            };
            let word = u32::from_le_bytes(word);
            let float = f32::from_bits(word);
            let class = if word == 0 || word == 0x8000_0000 {
                0
            } else if float.is_finite() && (1e-20..=1e12).contains(&float.abs()) {
                1
            } else {
                2
            };
            self.words.entry((kind, offset)).or_default()[class] += 1;
        }
    }

    /// The types with an event that holds a nonzero byte outside the class's fields: (events, such events).
    fn outside_types(&self) -> BTreeMap<i32, (usize, usize)> {
        self.outside
            .iter()
            .filter(|(_, v)| v.1 > 0)
            .map(|(k, v)| (*k, *v))
            .collect()
    }
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
    let second = oracle(OracleSet::Soulstruct, Data::Elden, "tae-event-classes.json")
        .map(|path| soulstruct_events(&path))
        .transpose()?
        .map(|events| Second::new(&templates, events));
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
    let mut t = Totals {
        second: second.as_ref().map(|_| SecondTotals::default()),
        ..Totals::default()
    };
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
                        if let (Some(second), Some(totals)) = (&second, &mut t.second) {
                            totals.observe(second, event.kind, &event.parameters);
                        }
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
                                Value::Bool(_)
                                | Value::Int(_)
                                | Value::Unsigned(_)
                                | Value::Float(_)
                                | Value::Bytes(_) => {}
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
        t.files,
        t.events,
        t.with_template,
        t.asserted,
        t.short,
        t.short_with_a_named_field_missing,
        t.unlisted,
        t.listed
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
    // soulstruct's table over the stored events: 319,835 events are of a type it lists (none of a type only it lists), and of those
    // 319,829 of a type it has a class for; seven classes leave a nonzero byte of some events outside their fields (the layouts of
    // the earlier games, shorter than Sekiro's); of the words typed as a float by one source and an integer by the other, the
    // template is right for the jump table's argument A (floats where it is not zero) and for event 130 (integers), soulstruct
    // for event 730 (integers)
    if let Some(second) = &t.second {
        assert_eq!((second.listed, second.with_class), (319_835, 319_829));
        assert!(second.only_soulstruct_seen.is_empty());
        assert_eq!(
            second.outside_types(),
            BTreeMap::from([
                (96, (8_005, 2_021)),
                (120, (4, 4)),
                (130, (185, 185)),
                (151, (151, 151)),
                (601, (2_071, 136)),
                (715, (372, 372)),
                (720, (155, 155)),
            ])
        );
        assert_eq!(
            second.words,
            BTreeMap::from([
                ((0, 4), [76_495, 134, 0]),
                ((130, 8), [0, 0, 185]),
                ((130, 12), [0, 0, 185]),
                ((730, 0), [226, 0, 860]),
                ((730, 4), [213, 0, 873]),
            ])
        );
    }
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
        t.files,
        t.events,
        t.with_template,
        t.asserted,
        t.short,
        t.short_with_a_named_field_missing,
        t.unlisted,
        t.listed
    );
    expect_found(Query::Install(Data::Elden, None), "event files", t.files);
    assert_eq!(
        (t.files, t.events, t.with_template),
        (1_071, 1_313_057, 1_313_057)
    );
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
    // soulstruct's table over the stored events (see Sekiro's test)
    if let Some(second) = &t.second {
        assert_eq!((second.listed, second.with_class), (1_274_243, 1_274_080));
        assert!(second.only_soulstruct_seen.is_empty());
        assert_eq!(
            second.outside_types(),
            BTreeMap::from([
                (2, (6_876, 5_917)),
                (64, (1_921, 1_809)),
                (96, (20_848, 1_642)),
                (120, (1_481, 1_481)),
                (129, (408_400, 408_390)),
                (145, (2_828, 2_828)),
                (193, (4_562, 4_517)),
                (601, (4_071, 4_071)),
                (700, (37_568, 34_213)),
            ])
        );
        assert_eq!(
            second.words,
            BTreeMap::from([
                ((0, 4), [304_824, 5_052, 0]),
                ((194, 8), [4, 0, 0]),
                ((730, 0), [1, 0, 115]),
                ((730, 4), [1, 0, 115]),
            ])
        );
    }
    Ok(())
}

/// The kinds a game's differing fields come in, as (soulstruct's, the template's) with their counts.
fn kind_pairs(c: &Comparison) -> BTreeMap<(Kind, Kind), usize> {
    let mut pairs = BTreeMap::new();
    for d in &c.differing {
        *pairs.entry((d.3, d.4)).or_default() += 1;
    }
    pairs
}

/// The 4-byte fields one source types as a float and the other as an integer: (event type, offset, width).
fn float_disagreements(c: &Comparison) -> Vec<(i32, usize, usize)> {
    c.differing
        .iter()
        .filter(|d| d.3 == Kind::Float || d.4 == Kind::Float)
        .map(|d| (d.0, d.1, d.2))
        .collect()
}

/// Soulstruct's table of the event types held to the templates of each game, field by field (offset, width, kind), without the
/// stored events.
#[test]
fn soulstructs_event_classes_against_the_community_templates() -> Result<()> {
    let Some(path) = oracle(OracleSet::Soulstruct, Data::Elden, "tae-event-classes.json") else {
        return Ok(());
    };
    let events = soulstruct_events(&path)?;
    assert_eq!(events.len(), 113);
    assert_eq!(events.values().filter(|e| e.fields.is_some()).count(), 108);
    if let Some(path) = oracle(OracleSet::Smithbox, Data::Sekiro, "tae-template.xml") {
        let templates = Templates::parse(&std::fs::read_to_string(path)?)?;
        let c = compare(&templates, &events);
        println!(
            "Sekiro: shared {}, only soulstruct {:?}, only template {}, with a class {}, named {}, fields compared {}, differing {:?}, \
             straddling {:?}",
            c.shared,
            c.only_soulstruct,
            c.only_template,
            c.with_class,
            c.named,
            c.compared,
            c.differing,
            c.overlapping
        );
        assert_eq!((c.shared, c.only_template), (106, 84));
        assert_eq!(c.only_soulstruct, [113, 136, 161, 194, 303, 317, 786]);
        assert_eq!((c.with_class, c.named), (103, 49));
        assert_eq!((c.compared, c.differing.len()), (234, 33));
        assert_eq!(
            kind_pairs(&c),
            BTreeMap::from([
                ((Kind::Int, Kind::Float), 3),
                ((Kind::Int, Kind::Flag), 26),
                ((Kind::Float, Kind::Int), 2),
                ((Kind::Flag, Kind::Int), 2),
            ])
        );
        // the jump table's argument A (a float for the template, an integer for soulstruct), the two words of event 130 (the
        // reverse) and the two of event 730 (a float for the template, an integer for soulstruct)
        assert_eq!(
            float_disagreements(&c),
            [(0, 4, 4), (130, 8, 4), (130, 12, 4), (730, 0, 4), (730, 4, 4)]
        );
        // event 112: soulstruct's two words at 4 and 8 against the template's 16-bit field at 4 and word at 6
        assert_eq!(c.overlapping, BTreeSet::from([112]));
    }
    if let Some(path) = oracle(OracleSet::Smithbox, Data::Elden, "tae-template.xml") {
        let templates = Templates::parse(&std::fs::read_to_string(path)?)?;
        let c = compare(&templates, &events);
        println!(
            "Elden Ring: shared {}, only soulstruct {:?}, only template {}, with a class {}, named {}, fields compared {}, \
             differing {:?}, straddling {:?}",
            c.shared,
            c.only_soulstruct,
            c.only_template,
            c.with_class,
            c.named,
            c.compared,
            c.differing,
            c.overlapping
        );
        assert_eq!((c.shared, c.only_template), (96, 60));
        assert_eq!(
            c.only_soulstruct,
            [
                130, 131, 136, 306, 308, 317, 510, 520, 720, 772, 786, 793, 794, 796, 797, 798, 799
            ]
        );
        assert_eq!((c.with_class, c.named), (94, 40));
        assert_eq!((c.compared, c.differing.len()), (224, 31));
        assert_eq!(
            kind_pairs(&c),
            BTreeMap::from([
                ((Kind::Int, Kind::Float), 3),
                ((Kind::Int, Kind::Flag), 25),
                ((Kind::Float, Kind::Int), 1),
                ((Kind::Flag, Kind::Int), 2),
            ])
        );
        assert_eq!(
            float_disagreements(&c),
            [(0, 4, 4), (194, 8, 4), (730, 0, 4), (730, 4, 4)]
        );
        assert!(c.overlapping.is_empty(), "{:?}", c.overlapping);
    }
    Ok(())
}
