//! The community's archive path lists as a census input: Smithbox's file dictionaries (`oracles.smithbox.<game>/file-dictionary.json`,
//! a JSON `Entries` list of paths with the archive each is said to be in and a `Validated` flag), read as a
//! [`NameDictionary`] and held to each game's archive index: how many index entries the list names, whether the archive it
//! says holds a path is the one that does, and what its `Validated` flag is worth. The list is read in place and is not
//! in this repository.
//!
//! The expected numbers are a census made with an independent Python reader of the same indexes (the BHD5 tables of each
//! install and the games' path hashes).
mod inputs;

use fromsoft_formats::{Game, NameDictionary};
use inputs::{OracleSet, Query, expect_found, oracle};
use std::collections::BTreeMap;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// One line of the list.
struct Line {
    path: String,
    archive: String,
    validated: bool,
}

fn lines(path: &std::path::Path) -> Result<Vec<Line>> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    json["Entries"]
        .as_array()
        .ok_or("Entries")?
        .iter()
        .map(|e| {
            Ok(Line {
                path: e["Path"].as_str().ok_or("Path")?.to_owned(),
                archive: e["Archive"].as_str().ok_or("Archive")?.to_owned(),
                validated: e["Validated"].as_bool().ok_or("Validated")?,
            })
        })
        .collect()
}

/// What the list says of a game's index.
struct Census {
    /// Entries in the archive index, and the ones the list names.
    entries: usize,
    named: usize,
    /// Lines of the list, distinct paths after the dictionary's normalisation, lines whose path is in the index.
    lines: usize,
    distinct: usize,
    in_index: usize,
    /// Lines in the index by (`Validated`, whether the path is in the index) .
    flags: BTreeMap<(bool, bool), usize>,
    /// Lines in the index whose `Archive` is not the pair that holds the path, by (said, actual).
    wrong_archive: BTreeMap<(String, String), usize>,
}

fn census(game: Game, data: inputs::Game) -> Result<Option<Census>> {
    let (Some(archive), Some(list)) = (
        common::archive(game)?,
        oracle(OracleSet::Smithbox, data, "file-dictionary.json"),
    ) else {
        return Ok(None);
    };
    let lines = lines(&list)?;
    let text = lines
        .iter()
        .map(|l| l.path.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let names = NameDictionary::parse_for(&text, game)?;
    let mut c = Census {
        entries: archive.entries.len(),
        named: archive
            .entries
            .keys()
            .filter(|h| names.names.contains_key(h))
            .count(),
        lines: lines.len(),
        distinct: names.names.values().map(Vec::len).sum(),
        in_index: 0,
        flags: BTreeMap::new(),
        wrong_archive: BTreeMap::new(),
    };
    for l in &lines {
        let hash = archive.hash(&l.path);
        let entry = archive.entries.get(&hash);
        *c.flags.entry((l.validated, entry.is_some())).or_default() += 1;
        let Some(entry) = entry else {
            continue;
        };
        c.in_index += 1;
        let pair = archive
            .pair(entry.shard)
            .map(|p| p.name().to_ascii_lowercase())
            .unwrap_or_default();
        // Sekiro's list leaves the field empty
        if !l.archive.is_empty() && l.archive != pair {
            *c.wrong_archive.entry((l.archive.clone(), pair)).or_default() += 1;
        }
    }
    Ok(Some(c))
}

#[test]
fn sekiros_list_names_the_index() -> Result<()> {
    let Some(c) = census(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    expect_found(
        Query::Oracle(OracleSet::Smithbox, inputs::Game::Sekiro, "file-dictionary.json"),
        "index entries",
        c.entries,
    );
    println!(
        "{} entries, {} named; {} lines, {} distinct, {} in the index; {:?}",
        c.entries, c.named, c.lines, c.distinct, c.in_index, c.flags
    );
    assert_eq!((c.entries, c.named), (8_765, 8_752));
    assert_eq!((c.lines, c.distinct, c.in_index), (8_949, 8_949, 8_752));
    // the `Validated` flag is exact for Sekiro: every validated path is in the index and no other is
    assert_eq!(
        c.flags,
        BTreeMap::from([((false, false), 197), ((true, true), 8_752)])
    );
    assert!(c.wrong_archive.is_empty());
    Ok(())
}

#[test]
fn elden_rings_list_names_the_index() -> Result<()> {
    let Some(c) = census(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    expect_found(
        Query::Oracle(OracleSet::Smithbox, inputs::Game::Elden, "file-dictionary.json"),
        "index entries",
        c.entries,
    );
    println!(
        "{} entries, {} named; {} lines, {} distinct, {} in the index; {:?} {:?}",
        c.entries, c.named, c.lines, c.distinct, c.in_index, c.flags, c.wrong_archive
    );
    // the list names every entry of the base archives
    assert_eq!((c.entries, c.named), (97_118, 97_118));
    // 130,951 lines, five of them repeats of another line; five lines repeat a path the index holds
    assert_eq!((c.lines, c.distinct, c.in_index), (130_951, 130_946, 97_123));
    // the `Validated` flag is not exact: 10,467 paths marked false are in the index, 33,750 marked true are not (they belong to
    // the expansion's archives, which this install's index lacks)
    assert_eq!(
        c.flags,
        BTreeMap::from([
            ((false, false), 78),
            ((false, true), 10_467),
            ((true, false), 33_750),
            ((true, true), 86_656),
        ])
    );
    // five paths the list files under the expansion's archive are in Data3
    assert_eq!(
        c.wrong_archive,
        BTreeMap::from([(("dlc".to_owned(), "data3".to_owned()), 5)])
    );
    Ok(())
}
