//! The census of one format in a game's install: our reader over every file, checked against the oracle the real-data
//! tests use, with the counts and the values of each field the census follows (`tests/census`). The report is kept
//! under the scratch root as `census/<format>-<game>.json`, and what changed since the kept one is listed.
//!
//! ```text
//! cargo run --release --example census -- <format> <game>...
//!     formats: flver
//!     games:   sekiro, elden
//! ```
//!
//! It reads the `GAME_DATA` configuration the real-data tests read. Exits 1 when a census has a failure (a disagreement
//! with the oracle), 2 when it cannot run.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a program: it reads the configuration and the oracle, and writes its report"
)]

#[path = "../tests/census/mod.rs"]
mod census;
#[path = "../tests/common/mod.rs"]
mod common;
#[path = "../tests/inputs/mod.rs"]
mod inputs;

use fromsoft_formats::Game;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "usage: census <format> <game>...  (formats: flver; games: sekiro, elden)";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("census: {e}");
            ExitCode::from(2)
        }
    }
}

/// Runs each census the arguments name; whether every one passed.
fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (format, games) = args.split_first().ok_or(USAGE)?;
    if games.is_empty() {
        return Err(USAGE.into());
    }
    let mut passed = true;
    for name in games {
        let game = match name.as_str() {
            "sekiro" => Game::Sekiro,
            "elden" => Game::EldenRing,
            _ => return Err(format!("unknown game {name}; {USAGE}").into()),
        };
        let start = Instant::now();
        let report = match format.as_str() {
            "flver" => census::flver::census(game)?,
            _ => return Err(format!("unknown format {format}; {USAGE}").into()),
        };
        let report = report.ok_or_else(|| {
            format!("{name}: the GAME_DATA configuration lacks an input the {format} census needs")
        })?;
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "{}",
            report.text(&format!("{} {format} census ({seconds:.1} s)", game.name()))
        );
        let kept = inputs::scratch()
            .join("census")
            .join(format!("{format}-{name}.json"));
        match std::fs::read_to_string(&kept) {
            Ok(text) => {
                let before: census::Report = serde_json::from_str(&text)?;
                let changes = report.changes(&before);
                println!(
                    "changes since the kept report {}: {}",
                    kept.display(),
                    changes.len()
                );
                for change in &changes {
                    println!("  {change}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("no kept report at {}: this run's is the first", kept.display());
            }
            Err(e) => return Err(format!("{}: {e}", kept.display()).into()),
        }
        if let Some(folder) = kept.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&kept, serde_json::to_string_pretty(&report)?)?;
        passed &= report.failures.is_empty();
    }
    Ok(passed)
}
