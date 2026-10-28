//! The MTD writer on every MTD of both installs: `write` returns the stored bytes of each, and `write_canonical`
//! (every block the length it spans, zero padding) reads back as the same definition.
//!
//! The counts were measured with an independent Python implementation of the format (a parser and a writer of its
//! own, run over the same files): Sekiro 3,170 MTDs, of which 83 declare a block longer than the bytes it spans (3,087
//! equal their canonical form); Elden Ring 2,602 (2,517 of `allmaterialbnd` and 85 of `speedtree`), of which 92 do and
//! one holds non-zero alignment padding (2,509 equal their canonical form). Not one MTD declares a block shorter than
//! the bytes it spans.
mod common;
mod inputs;

use fromsoft_formats::{Binder, Game, dcx::Decoder, mtd::MaterialDefinition};
use inputs::{Tool, oodle};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// (MTDs, written back to their bytes by `write`, canonical form equal to the bytes, canonical form read back equal).
fn round_trips(game: Game) -> Result<Option<(usize, usize, usize, usize)>> {
    let (Some(archive), Some(dll)) = (common::archive(game)?, oodle(Tool::Oodle6)) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 512 * 1024 * 1024,
    };
    let (mut total, mut exact, mut canonical_exact, mut canonical_parse) = (0, 0, 0, 0);
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
            total += 1;
            let written = m.write().map_err(|e| format!("{name}: {e}"))?;
            if written == bytes {
                exact += 1;
            } else {
                return Err(format!("{name}: write() differs from the stored bytes").into());
            }
            let canonical = m.write_canonical().map_err(|e| format!("{name}: {e}"))?;
            canonical_exact += usize::from(canonical == bytes);
            let again =
                MaterialDefinition::read(&canonical).map_err(|e| format!("{name}: canonical form: {e}"))?;
            // the canonical form's own lengths are the spans, and it has no padding records
            let same = again.shader == m.shader
                && again.description == m.description
                && again.parameters == m.parameters
                && again.textures == m.textures
                && again.padding.is_empty()
                && again.block_lengths.iter().all(|b| b.declared == b.consumed);
            if !same {
                return Err(format!("{name}: the canonical form reads back as another definition").into());
            }
            canonical_parse += 1;
        }
    }
    Ok(Some((total, exact, canonical_exact, canonical_parse)))
}

#[test]
fn sekiro_mtds_write_back_to_their_bytes() -> Result<()> {
    let Some(counts) = round_trips(Game::Sekiro)? else {
        return Ok(());
    };
    println!("Sekiro MTDs (total, exact, canonical exact, canonical reads back): {counts:?}");
    assert_eq!(counts, (3170, 3170, 3087, 3170));
    Ok(())
}

#[test]
fn elden_ring_mtds_write_back_to_their_bytes() -> Result<()> {
    let Some(counts) = round_trips(Game::EldenRing)? else {
        return Ok(());
    };
    println!("Elden Ring MTDs (total, exact, canonical exact, canonical reads back): {counts:?}");
    assert_eq!(counts, (2602, 2602, 2509, 2602));
    Ok(())
}
