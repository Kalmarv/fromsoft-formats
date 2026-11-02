//! The effect census of each configured game (`tests/census/effects.rs`): every effect binder, animation event file, map
//! layout and event script reads, every effect file is counted once under exactly one combination of the kinds of record
//! that reach it (or none), and the only param tables the census goes without are the three of Sekiro's that the param
//! reader does not read (the two behaviour banks' defaults and the menu colour table, whose headers carry format flags 1
//! and 2), and where Smithbox's param meta is configured, the param rule takes beyond it only fields that hold effect
//! files alone. `cargo run --release --example census -- effects <game>` prints the same report and what changed since the
//! last run.
mod census;
mod common;
mod inputs;

use fromsoft_formats::Game;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn what_starts_each_effect() -> Result<()> {
    for game in [Game::Sekiro, Game::EldenRing] {
        let Some(report) = census::effects::census(game)? else {
            continue;
        };
        println!("{}", report.text(&format!("{} effects census", game.name())));
        let refused: Vec<&String> = report
            .failures
            .iter()
            .filter(|f| !f.starts_with("param "))
            .collect();
        assert!(
            refused.is_empty(),
            "{}: entries refused: {refused:?}",
            game.name()
        );
        let tables: Vec<&str> = report
            .failures
            .iter()
            .filter_map(|f| f.strip_prefix("param ")?.split(':').next())
            .collect();
        let expected: &[&str] = match game {
            Game::Sekiro => &[
                "default_AIStandardInfoBank",
                "default_EnemyBehaviorBank",
                "MenuColorTableParam",
            ],
            Game::EldenRing => &[],
        };
        assert_eq!(
            tables,
            expected,
            "{}: param tables the census goes without",
            game.name()
        );
        // the param rule against Smithbox's param meta, where it is configured: each field the meta opens is a field of
        // its table, and each field the rule takes beyond the meta holds effect files only
        let against_meta: Vec<&String> = report
            .counts
            .keys()
            .filter(|name| {
                name.contains("naming no field of the table's def")
                    || name.starts_with("param fields the rule takes beyond the param meta that hold values")
            })
            .collect();
        assert!(
            against_meta.is_empty(),
            "{}: the param rule against the param meta: {against_meta:?}",
            game.name()
        );
        let files = report.get("effect files (distinct ids)");
        assert!(files > 0, "{}: no effect files", game.name());
        let by_combination: u64 = report
            .counts
            .iter()
            .filter(|(name, _)| name.starts_with("effects reached by exactly: "))
            .map(|(_, n)| n)
            .sum();
        assert_eq!(
            by_combination,
            files,
            "{}: each effect under one combination",
            game.name()
        );
        assert_eq!(
            report.get("effects reached by exactly: nothing"),
            report.get("effects unreached"),
            "{}",
            game.name()
        );
    }
    Ok(())
}
