//! Whole-install FLVER census against an independent decoder, for each game configured (`tests/census/flver.rs`):
//! SoulsFormats' census of Sekiro, SoulsFormatsNEXT's of Elden Ring. Every model must agree with the reference, and the
//! counts of what the reference does not compare must equal an independent census of the same models.
//! `cargo run --release --example census -- flver <game>` prints the same report and what changed since the last run.
mod census;
mod common;
mod inputs;

use fromsoft_formats::Game;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn all_flver_models() -> Result<()> {
    for game in [Game::Sekiro, Game::EldenRing] {
        let Some(report) = census::flver::census(game)? else {
            continue;
        };
        println!("{}", report.text(&format!("{} FLVER census", game.name())));
        // an independent census of the same models (a separate parser of the bone and face-set records): the bones'
        // chains, boxes and flags, the dummies' attachment, and the face sets' word at +6
        let want: &[(&str, u64)] = match game {
            Game::Sekiro => &[
                ("bones", 129_270),
                ("box valid", 66_485),
                ("box sentinel", 62_785),
                ("box neither", 0),
                ("first child is not the lowest-indexed child", 75),
                ("flags above the low four bits", 0),
                ("bit 1 set", 50_370),
                ("bit 1 set on a bone that has children or is named", 0),
                ("bit 2 set", 10_341),
                ("bit 2 set on a bone no dummy names", 1_964),
                ("bones a dummy names without bit 2", 27),
                ("face sets", 110_365),
                ("dummies", 20_268),
                ("dummies with flag1", 13_469),
                ("dummies with an attach bone", 13_449),
                ("dummies in single-bone models", 132),
                ("single-bone dummies with an attach bone and flag1", 34),
                ("single-bone dummies with an attach bone and no flag1", 8),
                ("single-bone dummies with no attach bone and flag1", 0),
                ("single-bone dummies with no attach bone and no flag1", 90),
                ("other dummies where flag1 follows the attach bone", 20_108),
                ("other dummies with flag1 and no attach bone", 28),
                ("other dummies with an attach bone and no flag1", 0),
            ],
            Game::EldenRing => &[
                ("bones", 535_503),
                ("box valid", 139_011),
                ("box sentinel", 396_492),
                ("box neither", 0),
                ("first child is not the lowest-indexed child", 37),
                ("flags above the low four bits", 0),
                ("bit 1 set", 363_132),
                ("bit 1 set on a bone that has children or is named", 0),
                ("bit 2 set", 24_493),
                ("bit 2 set on a bone no dummy names", 0),
                ("bones a dummy names without bit 2", 771),
                ("face sets", 277_113),
                ("dummies", 88_340),
                ("dummies with flag1", 69_004),
                ("dummies with an attach bone", 71_059),
                ("dummies in single-bone models", 2_205),
                ("single-bone dummies with an attach bone and flag1", 29),
                ("single-bone dummies with an attach bone and no flag1", 2_157),
                ("single-bone dummies with no attach bone and flag1", 0),
                ("single-bone dummies with no attach bone and no flag1", 19),
                ("other dummies where flag1 follows the attach bone", 86_033),
                ("other dummies with flag1 and no attach bone", 102),
                ("other dummies with an attach bone and no flag1", 0),
            ],
        };

        for &(key, n) in want {
            assert_eq!(report.get(key), n, "{}: {key}", game.name());
        }
        let (ones, maximum) = match game {
            Game::Sekiro => (85_996, 371),
            Game::EldenRing => (149_048, 285),
        };
        let words = report.values.get("face set word +6").ok_or("no face set words")?;
        assert_eq!(
            words.get(&1).copied(),
            Some(ones),
            "{}: face set word +6 is 1",
            game.name()
        );
        assert_eq!(
            words.keys().next_back().copied(),
            Some(maximum),
            "{}: face set word +6 maximum",
            game.name()
        );
        assert!(
            report.failures.is_empty(),
            "{}: {} disagreements with the reference",
            game.name(),
            report.failures.len()
        );
    }
    Ok(())
}
