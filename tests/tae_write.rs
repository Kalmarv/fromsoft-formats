//! The TAE writer: files built here read back as written and refuse what the format cannot store; and every distinct event
//! file of the animation binders of both installs (Sekiro's 326, Elden Ring's 1,071) writes back to its bytes.
//!
//! The real-data counts were measured with an independent Python implementation of the layout (a parser and a writer of
//! its own, run over the same files): every one of the 326 and 1,071 is written byte for byte.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`
//! for the real-data test; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::{Animation, Event, EventGroup, Header, Tae};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn event(kind: i32, start: f32, end: f32, parameters: Vec<u8>) -> Event {
    Event {
        kind,
        unk04: 0,
        start,
        end,
        parameters,
    }
}

fn standard() -> Header {
    Header::Standard {
        loops: true,
        imports_hkx: false,
        allow_delay_load: true,
        import_hkx_source: -1,
    }
}

/// Three animations (a run of two numbers and a third): one with three events of types 0, 5 and 943, two event groups (one
/// empty) and a motion file name; an import of the first without events; one with a single event.
fn sample() -> Tae {
    Tae {
        id: 1010,
        event_bank: 7,
        skeleton_name: Some("skeleton.hkx".into()),
        sib_name: None,
        group: 0,
        animations: vec![
            Animation {
                id: 3000,
                header: standard(),
                file_name: Some("a000_003000.hkt".into()),
                events: vec![
                    event(0, 0.0, 0.5, vec![0; 16]),
                    event(5, 0.5, 100.0, vec![]),
                    event(943, 0.0, 0.5, vec![0; 16]),
                ],
                event_groups: vec![
                    EventGroup {
                        group_type: 7,
                        events: vec![0, 2],
                    },
                    EventGroup {
                        group_type: 9,
                        events: vec![],
                    },
                ],
            },
            Animation {
                id: 3001,
                header: Header::ImportOther {
                    from: 3000,
                    unknown: -1,
                },
                file_name: None,
                events: vec![],
                event_groups: vec![],
            },
            Animation {
                id: 3005,
                header: standard(),
                file_name: None,
                events: vec![event(1, 1.0, 1.0, vec![0; 16])],
                event_groups: vec![],
            },
        ],
    }
}

/// An empty motion file name reads back as `Some("")` or `None` according to what follows it; they write the same.
fn name(n: &Option<String>) -> Option<&str> {
    n.as_deref().filter(|n| !n.is_empty())
}

#[test]
fn a_file_built_here_reads_back_as_written_and_writes_the_same_bytes_again() {
    let tae = sample();
    let bytes = tae.write().unwrap();
    let read = Tae::read(&bytes).unwrap();
    assert_eq!((read.id, read.event_bank), (1010, 7));
    assert_eq!(read.skeleton_name.as_deref(), Some("skeleton.hkx"));
    assert_eq!(read.sib_name, None);
    assert_eq!(read.animations.len(), 3);
    for (a, b) in tae.animations.iter().zip(&read.animations) {
        assert_eq!((a.id, &a.header), (b.id, &b.header));
        assert_eq!(name(&a.file_name), name(&b.file_name), "animation {}", a.id);
        assert_eq!(a.event_groups, b.event_groups, "animation {}", a.id);
        assert_eq!(a.events.len(), b.events.len());
        for (x, y) in a.events.iter().zip(&b.events) {
            assert_eq!(
                (x.kind, x.unk04, x.start, x.end),
                (y.kind, y.unk04, y.start, y.end)
            );
            // the stored parameters carry the padding to the next record
            assert!(y.parameters.starts_with(&x.parameters));
            assert!(y.parameters[x.parameters.len()..].iter().all(|&b| b == 0));
        }
    }
    // a second trip changes nothing
    assert_eq!(read.write().unwrap(), bytes);
}

/// The offset the one animation's one event stores for its parameters, and where its data record is.
fn parameter_offset(bytes: &[u8]) -> (u64, u64) {
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let body = u64_at(0x78) as usize;
    let headers = u64_at(body) as usize;
    let data = u64_at(headers + 16);
    (data, u64_at(data as usize + 8))
}

#[test]
fn event_type_943_stores_no_parameter_offset() {
    for (kind, stores_none) in [(943, true), (5, false), (942, false), (944, false)] {
        let tae = Tae {
            id: 1,
            event_bank: 0,
            skeleton_name: None,
            sib_name: None,
            group: 0,
            animations: vec![Animation {
                id: 0,
                header: standard(),
                file_name: None,
                events: vec![event(kind, 0.0, 0.5, vec![])],
                event_groups: vec![],
            }],
        };
        let bytes = tae.write().unwrap();
        let (data, stored) = parameter_offset(&bytes);
        assert_eq!(stored, if stores_none { 0 } else { data + 16 }, "type {kind}");
        // either way the reader takes the file
        assert_eq!(Tae::read(&bytes).unwrap().animations[0].events[0].kind, kind);
    }
}

#[test]
fn a_file_without_animations_writes_and_reads_back() {
    let tae = Tae {
        id: 5,
        event_bank: 3,
        skeleton_name: Some("c0000.hkx".into()),
        sib_name: Some("c0000.sib".into()),
        group: 0,
        animations: vec![],
    };
    let bytes = tae.write().unwrap();
    assert_eq!(Tae::read(&bytes).unwrap(), tae);
}

#[test]
fn what_the_format_cannot_store_is_refused() {
    let base = sample();
    let refused = |edit: &dyn Fn(&mut Tae)| {
        let mut tae = base.clone();
        edit(&mut tae);
        tae.write().is_err()
    };
    assert!(!refused(&|_| {}), "the unedited sample writes");
    // numbers that do not rise
    assert!(refused(&|t| t.animations[1].id = 3000));
    assert!(refused(&|t| t.animations.swap(0, 2)));
    // numbers the groups table cannot hold
    assert!(refused(&|t| t.animations[2].id = i64::from(i32::MAX) + 1));
    // an animation with no mini header
    assert!(refused(&|t| t.animations[2].header = Header::None));
    // a time that is not a number
    assert!(refused(&|t| t.animations[2].events[0].start = f32::NAN));
    // a group naming an event the animation lacks
    assert!(refused(&|t| t.animations[0].event_groups[0].events.push(3)));
    // a name with a NUL in it
    assert!(refused(&|t| t.skeleton_name = Some("a\0b".into())));
}

#[test]
fn every_event_file_of_the_installs_writes_back_to_its_bytes() -> Result<()> {
    for (game, data, expected) in [
        (Game::Sekiro, Data::Sekiro, 326),
        (Game::EldenRing, Data::Elden, 1_071),
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
                    if !named_tae || !distinct.insert(Sha256::digest(&member.bytes).to_vec()) {
                        continue;
                    }
                    let tae = Tae::read_member(&binder, member)?;
                    let written = tae
                        .write()
                        .map_err(|e| format!("{path} {:?}: {e}", member.name))?;
                    if written != member.bytes {
                        return Err(format!(
                            "{path} {:?}: write() differs from the stored bytes",
                            member.name
                        )
                        .into());
                    }
                }
            }
        }
        println!(
            "{}: {} distinct event files written back byte for byte",
            game.name(),
            distinct.len()
        );
        assert_eq!(distinct.len(), expected, "{}", game.name());
    }
    Ok(())
}
