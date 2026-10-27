//! The alignment padding of every MTD of both installs: zero everywhere but one Elden Ring file.
mod common;
mod inputs;

use fromsoft_formats::{Binder, Game, dcx::Decoder, mtd::MaterialDefinition};
use inputs::{Tool, oodle};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
/// A file's name and its padding records (offset, bytes).
type Padding = (String, Vec<(usize, Vec<u8>)>);

/// Every MTD of the game's material binders as (name, padding records), or `None` when the install is not configured.
fn padding(game: Game) -> Result<Option<Vec<Padding>>> {
    let (Some(archive), Some(dll)) = (common::archive(game)?, oodle(Tool::Oodle6)) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 512 * 1024 * 1024,
    };
    let mut all = vec![];
    for path in ["/mtd/allmaterialbnd.mtdbnd.dcx", "/mtd/speedtree.mtdbnd.dcx"] {
        // Sekiro has no SpeedTree binder
        if game == Game::Sekiro && path.contains("speedtree") {
            continue;
        }
        let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
        for file in &binder.files {
            let name = file.name.clone().unwrap_or_default();
            if !name.to_lowercase().ends_with(".mtd") {
                continue;
            }
            let bytes = if file.bytes.starts_with(b"DCX\0") {
                decoder.decode(&file.bytes)?
            } else {
                file.bytes.clone()
            };
            let m = MaterialDefinition::read(&bytes).map_err(|e| format!("{name}: {e}"))?;
            all.push((name, m.padding.into_iter().map(|p| (p.offset, p.bytes)).collect()));
        }
    }
    Ok(Some(all))
}

#[test]
fn sekiro_mtd_padding_is_zero_everywhere() -> Result<()> {
    let Some(all) = padding(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!(all.len(), 3170);
    assert!(all.iter().all(|(_, p)| p.is_empty()));
    println!("Sekiro: {} MTDs, no non-zero alignment padding", all.len());
    Ok(())
}

#[test]
fn elden_ring_mtd_padding_is_zero_but_in_one_file() -> Result<()> {
    let Some(all) = padding(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!(all.len(), 2602);
    let odd: Vec<_> = all.iter().filter(|(_, p)| !p.is_empty()).collect();
    println!("Elden Ring: {} MTDs; padding records: {odd:?}", all.len());
    assert_eq!(odd.len(), 1);
    let (name, records) = odd[0];
    assert!(name.ends_with("c9757_crystal.mtd"), "{name}");
    assert_eq!(records.len(), 19);
    assert_eq!(records[0], (0xc6, vec![0x72, 0x34]));
    Ok(())
}
