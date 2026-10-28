//! The event-script writer on every event script of both installs (`/event/*.emevd.dcx` that the dictionaries name and
//! the archives hold): `write` returns the stored bytes of Elden Ring's 589 and of 26 of Sekiro's 30. The other four
//! differ only in the offset word of an event with no parameters (6 events), which the writer stores as -1.
//!
//! The counts were measured with an independent Python implementation of the layout (a parser and a writer of its
//! own, run over the same files): 446 distinct Elden Ring files and Sekiro's 30, every Elden Ring file equal, and the
//! four Sekiro files each with an event whose empty parameter list stores the running offset (0 in 4 events, 13,152
//! and 10,592 in the other two) where the writer stores -1.
mod common;
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::emevd::Script;
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// (scripts, written byte for byte, events whose empty parameter list's offset differs), or `None` when the game is
/// not configured.
fn round_trips(game: Game, data: Data) -> Result<Option<(usize, usize, usize)>> {
    let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
        install(data),
        keys(data),
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
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
        .filter(|p| p.starts_with("/event/") && p.ends_with(".emevd.dcx") && !p[7..].contains('/'))
        .collect();
    let (mut scripts, mut exact, mut offsets) = (0, 0, 0);
    for path in paths.into_iter().filter(|p| archive.contains(p)) {
        let bytes = archive.read(path, &decoder)?;
        let script = Script::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let written = script.write().map_err(|e| format!("{path}: {e}"))?;
        scripts += 1;
        if written == bytes {
            exact += 1;
            continue;
        }
        // not byte for byte: the same size, the same script, and every differing byte in the parameter-offset word of an
        // event without parameters
        assert_eq!(written.len(), bytes.len(), "{path}");
        assert_eq!(Script::read(&written)?, script, "{path}");
        let events_at = 0x90usize;
        let mut events = BTreeSet::new();
        for (at, (a, b)) in written.iter().zip(&bytes).enumerate() {
            if a == b {
                continue;
            }
            let within = at
                .checked_sub(events_at)
                .filter(|w| *w < script.events.len() * 0x30);
            let (event, field) = within
                .map(|w| (w / 0x30, w % 0x30))
                .ok_or("a difference outside the events")?;
            assert!(
                (32..40).contains(&field),
                "{path}: byte {at:#x} is field {field} of event {event}"
            );
            assert!(
                script.events[event].parameters.is_empty(),
                "{path}: event {event}"
            );
            events.insert(event);
        }
        offsets += events.len();
    }
    Ok(Some((scripts, exact, offsets)))
}

#[test]
fn sekiro_event_scripts_write_back_but_for_six_empty_offsets() -> Result<()> {
    let Some(counts) = round_trips(Game::Sekiro, Data::Sekiro)? else {
        return Ok(());
    };
    println!("Sekiro scripts, byte for byte, events with another empty-list offset: {counts:?}");
    assert_eq!(counts, (30, 26, 6));
    Ok(())
}

#[test]
fn elden_ring_event_scripts_write_back_to_their_bytes() -> Result<()> {
    let Some(counts) = round_trips(Game::EldenRing, Data::Elden)? else {
        return Ok(());
    };
    println!("Elden Ring scripts, byte for byte, events with another empty-list offset: {counts:?}");
    assert_eq!(counts, (589, 589, 0));
    Ok(())
}
