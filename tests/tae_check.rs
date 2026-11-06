//! A TAE file held to the community's event templates ([`Tae::check_templates`], [`Tae::write_checked`]): the refusals
//! name the animation, the event and the field; and every event of every distinct event file of both installs passes
//! the Smithbox templates for its game.
//!
//! The real-data counts are those of an independent Python reader of the same files and templates (a TAE walker and a
//! template reader written from the file layout and the XML alone): Sekiro's 326 distinct event files hold 337,924
//! events, 337,847 of a type the template defines (types 340, 341 and 350 have none) and 553,355 asserted fields that
//! hold their value; Elden Ring's base game's 1,071 hold 1,313,057 events, all of a defined type, and 2,050,123
//! asserted fields that hold theirs. With Shadow of the Erdtree installed Elden Ring's are this test's own: 1,282
//! files, 1,635,135 events, all of a defined type, and 2,603,817 asserted fields (with the expansion's archives left
//! out it gives the reader's).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6`, `oracles.uxm.{sekiro,elden}/dictionary.txt`
//! and `oracles.smithbox.{sekiro,elden}/tae-template.xml` for the real-data test; a game that is not configured is
//! skipped.
mod common;
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::tae::{Animation, Event, Header, Tae, TemplateCheck, template::Templates};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const XML: &str = r#"<event_template game="SDT">
	<event id="8" name="Thing"><u8 assert="0"/><s16 name="Row" ref="SpEffectParam"/><aob length="2"/></event>
	<event id="9" name="Flag"><b name="On"/><s32 name="Zero" assert="0"/></event>
</event_template>"#;

/// A file of one animation (number 3000) whose events are of the given types with the given parameter bytes.
fn tae(events: Vec<(i32, Vec<u8>)>) -> Tae {
    Tae {
        id: 1,
        event_bank: 0,
        skeleton_name: None,
        sib_name: None,
        group: 0,
        animations: vec![Animation {
            id: 3000,
            header: Header::Standard {
                loops: false,
                imports_hkx: false,
                allow_delay_load: false,
                import_hkx_source: -1,
            },
            file_name: None,
            events: events
                .into_iter()
                .map(|(kind, parameters)| Event {
                    kind,
                    unk04: 0,
                    start: 0.0,
                    end: 1.0,
                    parameters,
                })
                .collect(),
            event_groups: vec![],
        }],
    }
}

#[test]
fn a_file_that_keeps_its_templates_asserts_passes_and_is_counted() {
    let templates = Templates::parse(XML).unwrap();
    let file = tae(vec![
        (8, vec![0, 5, 0, 9, 8, 0, 0, 0]),
        // a type the templates do not define is not checked
        (99, vec![1, 2, 3]),
        // an event stored shorter than its template is checked as far as it goes
        (8, vec![0]),
        (9, vec![1, 0, 0, 0, 0]),
    ]);
    assert_eq!(
        file.check_templates(&templates).unwrap(),
        TemplateCheck {
            events: 4,
            templated: 3,
            asserted_fields: 3
        }
    );
    assert_eq!(file.write_checked(&templates).unwrap(), file.write().unwrap());
}

#[test]
fn a_broken_assert_is_refused_by_animation_event_and_field() {
    let templates = Templates::parse(XML).unwrap();
    let file = tae(vec![(8, vec![0, 5, 0, 9, 8]), (8, vec![1, 5, 0, 9, 8])]);
    let error = file.check_templates(&templates).unwrap_err().to_string();
    for part in [
        "animation 3000 event 1 (type 8, Thing)",
        "field 0",
        "stores 1",
        "asserts 0",
    ] {
        assert!(error.contains(part), "{part:?} missing from: {error}");
    }
    // a named field, past the first byte
    let file = tae(vec![(9, vec![0, 1, 0, 0, 0])]);
    let error = file.check_templates(&templates).unwrap_err().to_string();
    for part in ["event 0 (type 9, Flag)", "field 1 (Zero)", "byte 0x1", "stores 1"] {
        assert!(error.contains(part), "{part:?} missing from: {error}");
    }
    // nothing is written for it
    assert!(file.write_checked(&templates).is_err());
    assert!(file.write().is_ok());
}

#[test]
fn a_value_the_template_cannot_hold_is_refused_with_the_event_named() {
    let templates = Templates::parse(XML).unwrap();
    // a boolean byte that is neither 0 nor 1
    let file = tae(vec![(9, vec![2, 0, 0, 0, 0])]);
    let error = file.check_templates(&templates).unwrap_err().to_string();
    assert!(error.contains("animation 3000 event 0 (type 9, Flag)"), "{error}");
    assert!(error.contains("boolean"), "{error}");
}

#[test]
fn every_event_of_the_installs_keeps_the_templates_asserts() -> Result<()> {
    // per game: events, events of a defined type, asserted fields held
    for (game, data, expected) in [
        (Game::Sekiro, Data::Sekiro, (337_924, 337_847, 553_355)),
        (Game::EldenRing, Data::Elden, (1_635_135, 1_635_135, 2_603_817)),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary), Some(template)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
            oracle(OracleSet::Smithbox, data, "tae-template.xml"),
        ) else {
            continue;
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
        // each binder's event files at any depth, checked on the archive's threads: the content's digest and its
        // check (counted for the first of a content, as is its error)
        let binders =
            archive.read_each_binder(&paths, &decoder, inputs::threads(), |path, binder| -> Result<_> {
                let mut files = Vec::new();
                let mut todo = vec![binder?];
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
                        let check = Tae::read_member(&binder, member)
                            .map_err(|e| e.to_string())
                            .and_then(|tae| {
                                tae.check_templates(&templates)
                                    .map_err(|e| format!("{path} {:?}: {e}", member.name))
                            })
                            .map(|check| (check.events, check.templated, check.asserted_fields));
                        files.push((Sha256::digest(&member.bytes).to_vec(), check));
                    }
                }
                Ok(files)
            })?;
        let mut distinct = BTreeSet::new();
        let mut total = TemplateCheck::default();
        for binder in binders {
            for (digest, check) in binder? {
                if !distinct.insert(digest) {
                    continue;
                }
                let (events, templated, asserted_fields) = check?;
                total.events += events;
                total.templated += templated;
                total.asserted_fields += asserted_fields;
            }
        }
        println!(
            "{}: {} distinct event files, {} events of which {} have a template, {} asserted fields held",
            game.name(),
            distinct.len(),
            total.events,
            total.templated,
            total.asserted_fields
        );
        assert_eq!(
            (total.events, total.templated, total.asserted_fields),
            expected,
            "{}",
            game.name()
        );
    }
    Ok(())
}
