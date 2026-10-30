//! `save::Save` against an independent reading of one Elden Ring PC save (`oracles.saves.elden/ER0000.sl2` and its
//! `ER0000.walk.json`, a Python walk written from the community save editor's slot order). The two find the same
//! occupied slots by different rules: the reader goes by the profile's flags, the walk by the slot's version word. For
//! every occupied slot they agree on the version, the seven `ChrAsm` words, and the item id in each of the 22 equipment
//! slots. Every handle the walk resolves holds the item id the slot word holds, apart from the armour pieces' type bits.
//! The save is read only, from a copy taken from the save folder.
//!
//! Needs `oracles.saves`; a run without it skips.
mod inputs;

use fromsoft_formats::save::{ArmStyle, Save};
use inputs::{Game, OracleSet, oracle};
use serde_json::Value;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn words(v: &Value, key: &str) -> Result<Vec<Option<u64>>> {
    Ok(v[key]
        .as_array()
        .ok_or_else(|| format!("walk: no {key}"))?
        .iter()
        .map(Value::as_u64)
        .collect())
}

#[test]
fn the_save_reader_agrees_with_an_independent_walk() -> Result<()> {
    let (Some(file), Some(walk)) = (
        oracle(OracleSet::Saves, Game::Elden, "ER0000.sl2"),
        oracle(OracleSet::Saves, Game::Elden, "ER0000.walk.json"),
    ) else {
        return Ok(());
    };
    let save = Save::read(&std::fs::read(file)?)?;
    let walk: Value = serde_json::from_str(&std::fs::read_to_string(walk)?)?;
    let walked = walk["slots"].as_array().ok_or("walk: no slots")?;
    assert_eq!(
        save.slots.iter().map(|s| s.index).collect::<Vec<_>>(),
        walked
            .iter()
            .map(|s| s["index"].as_u64().and_then(|i| usize::try_from(i).ok()))
            .collect::<Option<Vec<_>>>()
            .ok_or("walk: an index that is not a number")?,
        "occupied slots"
    );
    let mut resolved = 0usize;
    for (slot, w) in save.slots.iter().zip(walked) {
        assert_eq!(
            Some(u64::from(slot.version)),
            w["version"].as_u64(),
            "slot {} version",
            slot.index
        );
        let h = &slot.hold;
        let head = [
            h.arm_style.word(),
            h.left_hand,
            h.right_hand,
            h.left_arrow,
            h.right_arrow,
            h.left_bolt,
            h.right_bolt,
        ];
        assert_eq!(
            head.iter().map(|v| Some(u64::from(*v))).collect::<Vec<_>>(),
            words(w, "chr_asm")?,
            "slot {} ChrAsm",
            slot.index
        );
        let e = &slot.equipped;
        // the 22 slot words in the save's order, with the words the reader does not name left out (None here)
        let named: [Option<Option<u32>>; 22] = [
            Some(e.left_hand[0]),
            Some(e.right_hand[0]),
            Some(e.left_hand[1]),
            Some(e.right_hand[1]),
            Some(e.left_hand[2]),
            Some(e.right_hand[2]),
            Some(e.arrows[0]),
            Some(e.bolts[0]),
            Some(e.arrows[1]),
            Some(e.bolts[1]),
            None,
            None,
            Some(e.head),
            Some(e.chest),
            Some(e.arms),
            Some(e.legs),
            None,
            Some(e.talismans[0]),
            Some(e.talismans[1]),
            Some(e.talismans[2]),
            Some(e.talismans[3]),
            None,
        ];
        let (items, handles, records) = (words(w, "items")?, words(w, "handles")?, words(w, "records")?);
        for (i, read) in named.iter().enumerate() {
            let stored = items[i].ok_or("walk: an item that is not a number")?;
            if let Some(read) = read {
                let expected = (stored != u64::from(u32::MAX)).then_some(stored);
                assert_eq!(
                    read.map(u64::from),
                    expected,
                    "slot {} equipment word {i}",
                    slot.index
                );
            }
            if handles[i] != Some(0) {
                let record = records[i]
                    .ok_or_else(|| format!("slot {} word {i}: the walk resolves no record", slot.index))?;
                assert_eq!(
                    record & 0x0FFF_FFFF,
                    stored & 0x0FFF_FFFF,
                    "slot {} word {i} record",
                    slot.index
                );
                resolved += 1;
            }
        }
    }
    // this save: one character, held in both hands in the right, an Uchigatana in the right hand's first slot
    assert_eq!(save.slots.len(), 1);
    let slot = &save.slots[0];
    assert_eq!(
        (slot.index, slot.version, slot.hold.arm_style),
        (0, 262, ArmStyle::RightBoth)
    );
    assert_eq!(
        (slot.right_weapon(), slot.left_weapon()),
        (Some(9_000_000), Some(41_000_000))
    );
    assert_eq!(resolved, 12, "equipped handles the walk resolves");
    Ok(())
}
