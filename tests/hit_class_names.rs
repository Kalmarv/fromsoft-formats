//! The crate's names of the HitMtrlParam classes (`collision::hit_class_name`) against the community's English row names
//! (Smithbox's `Param Row Names/English/HitMtrlParam.json`, `oracles.smithbox.<game>/hit-material-names.json`): a row it names
//! is the class's name, a row it leaves blank has none, and every class the table names is a row of the list. The list
//! holds 51 rows for each game (IDs 0 to 80 in Sekiro, 0 to 62 in Elden Ring), 50 of them named in Sekiro and 18 in
//! Elden Ring.
mod inputs;

use fromsoft_formats::{Game, collision::hit_class_name};
use inputs::{OracleSet, Query, expect_found, oracle};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn rows(path: &std::path::Path) -> Result<BTreeMap<i32, String>> {
    let json: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(path)?.trim_start_matches('\u{feff}'))?;
    json["Entries"]
        .as_array()
        .ok_or("Entries")?
        .iter()
        .map(|e| {
            Ok((
                i32::try_from(e["ID"].as_i64().ok_or("ID")?)?,
                e["Entries"][0].as_str().ok_or("name")?.to_owned(),
            ))
        })
        .collect()
}

fn check(game: Game, data: inputs::Game, rows_named: (usize, usize)) -> Result<()> {
    let Some(path) = oracle(OracleSet::Smithbox, data, "hit-material-names.json") else {
        return Ok(());
    };
    let rows = rows(&path)?;
    let named = rows.values().filter(|n| !n.is_empty()).count();
    expect_found(
        Query::Oracle(OracleSet::Smithbox, data, "hit-material-names.json"),
        "rows",
        rows.len(),
    );
    assert_eq!((rows.len(), named), rows_named);
    for (id, name) in &rows {
        let ours = hit_class_name(game, *id);
        assert_eq!(
            ours,
            (!name.is_empty()).then_some(name.as_str()),
            "{game:?} row {id}"
        );
    }
    for class in 0..100 {
        if hit_class_name(game, class).is_some() {
            assert!(
                rows.contains_key(&class),
                "{game:?}: class {class} is not a row of the list"
            );
        }
    }
    Ok(())
}

#[test]
fn sekiros_class_names_are_the_row_names() -> Result<()> {
    check(Game::Sekiro, inputs::Game::Sekiro, (51, 50))
}

#[test]
fn elden_rings_class_names_are_the_row_names() -> Result<()> {
    check(Game::EldenRing, inputs::Game::Elden, (51, 18))
}
