//! The animation binders of each install's characters (`/chr/*.anibnd.dcx`) and the binders their markers name
//! (`Binder::division_markers`): Sekiro's player keeps its animations in seven binders its `.txt` markers name, Elden Ring's
//! characters in divisions their `.blf` lists name (96 binders, 261 lists), and the player's in eleven by `.txt` markers.
//! Every list reads as `blf::LoadList` and has the same three commands.
//!
//! The expected numbers are a census made with an independent Python reader of the same binders (the member names of every
//! animation binder at the top level of the archive).
mod inputs;

use fromsoft_formats::{Binder, Game, NameDictionary, blf::LoadList, dcx::Decoder};
use inputs::{OracleSet, Query, Tool, expect_found, oodle, oracle};
use std::collections::BTreeMap;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Totals {
    binders: usize,
    /// Binders with at least one marker whose binder the archive holds, and how many they name, by count.
    with_companions: usize,
    companions: usize,
    by_count: BTreeMap<usize, usize>,
    /// Markers (`<id>_...`) naming no binder of the archive.
    markers_without_binder: usize,
    /// `.blf` members, the binders that hold them, and the commands of each list as (entry id, kind, parameters) triples.
    lists: usize,
    binders_with_lists: usize,
    shapes: BTreeMap<Vec<(u64, String, [u32; 3])>, usize>,
    versions: BTreeMap<u32, usize>,
}

fn census(game: Game, data: inputs::Game) -> Result<Option<Totals>> {
    let (Some(archive), Some(dll), Some(names)) = (
        common::archive(game)?,
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
    let names = NameDictionary::read_for(names, game)?;
    let mut t = Totals::default();
    for (hash, paths) in &names.names {
        let Some(path) = paths
            .iter()
            .find(|p| p.starts_with("/chr/") && p.ends_with(".anibnd.dcx"))
        else {
            continue;
        };
        if !archive.entries.contains_key(hash) {
            continue;
        }
        t.binders += 1;
        let binder = Binder::read(&archive.read_hash(*hash, &decoder)?, &decoder)?;
        let id = path
            .strip_prefix("/chr/")
            .and_then(|p| p.strip_suffix(".anibnd.dcx"))
            .ok_or("a chr path")?;
        let prefix = format!("{id}_");
        let markers = binder.division_markers(&prefix);
        let named: Vec<&String> = markers
            .iter()
            .filter(|set| archive.contains(&format!("/chr/{set}.anibnd.dcx")))
            .collect();
        t.markers_without_binder += markers.len() - named.len();
        if !named.is_empty() {
            t.with_companions += 1;
            t.companions += named.len();
            *t.by_count.entry(named.len()).or_default() += 1;
        }
        let mut lists = 0;
        for file in &binder.files {
            let is_list = file
                .name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().ends_with(".blf"));
            if !is_list {
                continue;
            }
            lists += 1;
            let list = LoadList::parse(&file.bytes)
                .map_err(|e| format!("{path}: {}: {e}", file.name.as_deref().unwrap_or("")))?;
            *t.versions.entry(list.version).or_default() += 1;
            let shape = list
                .commands
                .iter()
                .map(|c| (c.first_entry_id, c.kind.clone(), c.parameters))
                .collect();
            *t.shapes.entry(shape).or_default() += 1;
        }
        t.lists += lists;
        t.binders_with_lists += usize::from(lists > 0);
    }
    Ok(Some(t))
}

#[test]
fn sekiros_player_names_its_binders_by_text_markers() -> Result<()> {
    let Some(t) = census(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    println!(
        "{} animation binders, {} with companions ({} in all {:?}), {} markers without a binder, {} lists",
        t.binders, t.with_companions, t.companions, t.by_count, t.markers_without_binder, t.lists
    );
    expect_found(
        Query::Install(inputs::Game::Sekiro, None),
        "animation binders",
        t.binders,
    );
    assert_eq!(t.binders, 142);
    assert_eq!((t.with_companions, t.companions), (1, 7));
    assert_eq!(t.by_count, BTreeMap::from([(7, 1)]));
    assert_eq!((t.markers_without_binder, t.lists), (0, 0));
    Ok(())
}

#[test]
fn elden_rings_characters_name_their_divisions() -> Result<()> {
    let Some(t) = census(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    println!(
        "{} animation binders, {} with companions ({} in all {:?}), {} markers without a binder, {} lists in {} binders",
        t.binders,
        t.with_companions,
        t.companions,
        t.by_count,
        t.markers_without_binder,
        t.lists,
        t.binders_with_lists
    );
    expect_found(
        Query::Install(inputs::Game::Elden, None),
        "animation binders",
        t.binders,
    );
    assert_eq!(t.binders, 567);
    // 96 characters keep a list for each division (261 lists), the player 11 text markers
    assert_eq!((t.with_companions, t.companions), (97, 272));
    assert_eq!(
        t.by_count,
        BTreeMap::from([(2, 61), (3, 17), (4, 9), (5, 3), (6, 5), (7, 1), (11, 1)])
    );
    // two of the player's text markers name no binder of the archive
    assert_eq!(t.markers_without_binder, 2);
    assert_eq!((t.lists, t.binders_with_lists), (261, 96));
    assert_eq!(t.versions, BTreeMap::from([(301, 261)]));
    // every list: the animations (entry id 1,000,000,000), `div_data.txt` (6,000,000) and `ver_0001.txt` (9,999,999)
    assert_eq!(
        t.shapes,
        BTreeMap::from([(
            vec![
                (1_000_000_000, "_HKX".to_owned(), [16, 2, 1]),
                (6_000_000, "_TXT".to_owned(), [16, 2, 1]),
                (9_999_999, "_TXT".to_owned(), [16, 2, 1]),
            ],
            261
        )])
    );
    Ok(())
}
