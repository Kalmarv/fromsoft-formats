//! The stored format code of every texture of every texture package of each installed game (the split `.tpfbhd` binders,
//! `.tpf.dcx` files and the `texbnd`, `tpfbnd`, `objbnd`, `partsbnd`, `ffxbnd` and `fgbnd` binders) against the pixel format
//! of the DDS file it holds: each code stands for one pixel layout ([`PixelFamily`]), under the DDS dialects below.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (a TPF walker and a DDS header
//! reader).
//!
//! Elden Ring's counts are those of the path dictionary that names all 97,118 entries of the base archives
//! (`tests/inputs`: UXM's list and the 87 Smithbox paths that hash onto the entries UXM's list leaves unnamed); the
//! Python reader walked the same archives and named the same entries.
mod inputs;

use fromsoft_formats::{
    Game, NameDictionary,
    dcx::Decoder,
    tpf::{Package, PixelFamily},
};
use inputs::{OracleSet, Query, Tool, expect_found, oodle, oracle};
use std::collections::BTreeMap;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const SUFFIXES: [&str; 8] = [
    ".tpfbhd",
    ".tpf.dcx",
    ".texbnd.dcx",
    ".tpfbnd.dcx",
    ".objbnd.dcx",
    ".partsbnd.dcx",
    ".ffxbnd.dcx",
    ".fgbnd.dcx",
];

fn u32_at(dds: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(dds.get(at..at + 4)?.try_into().ok()?))
}

/// How a DDS file states its pixel format: `DX10:<DXGI format>`, `fourcc:<name>` (the number for a FourCC that is not
/// text), or for channel masks `rgb:<flags>:bits<n>:<red>/<green>/<blue>/<alpha masks>`.
fn dds_format(dds: &[u8]) -> Option<String> {
    let flags = u32_at(dds, 80)?;
    let fourcc = dds.get(84..88)?;
    if fourcc == b"DX10" {
        return Some(format!("DX10:{}", u32_at(dds, 128)?));
    }
    if flags & 4 != 0 {
        return Some(if fourcc.iter().all(u8::is_ascii_alphanumeric) {
            format!("fourcc:{}", String::from_utf8_lossy(fourcc))
        } else {
            format!("fourcc:{}", u32_at(dds, 84)?)
        });
    }
    Some(format!(
        "rgb:{flags:x}:bits{}:{:x}/{:x}/{:x}/{:x}",
        u32_at(dds, 88)?,
        u32_at(dds, 92)?,
        u32_at(dds, 96)?,
        u32_at(dds, 100)?,
        u32_at(dds, 104)?
    ))
}

#[derive(Default)]
struct Totals {
    packages: usize,
    textures: usize,
    /// Textures by format code and DDS pixel format.
    by_code: BTreeMap<u8, BTreeMap<String, usize>>,
    /// Textures whose code names no family.
    no_family: usize,
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
    common::each_leaf(&archive, &names, &SUFFIXES, &decoder, |label, bytes| {
        if !bytes.starts_with(b"TPF\0") {
            return Ok(());
        }
        let package = Package::read(bytes).map_err(|e| format!("{label}: {e}"))?;
        t.packages += 1;
        for texture in &package.textures {
            t.textures += 1;
            t.no_family += usize::from(texture.pixel_family().is_none());
            let format = dds_format(&texture.dds).ok_or_else(|| format!("{label}: a short DDS header"))?;
            *t.by_code
                .entry(texture.format)
                .or_default()
                .entry(format)
                .or_default() += 1;
        }
        Ok(())
    })?;
    Ok(Some(t))
}

fn table(rows: &[(u8, &[(&str, usize)])]) -> BTreeMap<u8, BTreeMap<String, usize>> {
    rows.iter()
        .map(|(code, formats)| {
            (
                *code,
                formats.iter().map(|(f, n)| ((*f).to_owned(), *n)).collect(),
            )
        })
        .collect()
}

#[test]
fn sekiros_format_codes() -> Result<()> {
    let Some(t) = census(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    println!(
        "{} packages, {} textures: {:?}",
        t.packages, t.textures, t.by_code
    );
    expect_found(
        Query::Install(inputs::Game::Sekiro, None),
        "texture packages",
        t.packages,
    );
    assert_eq!((t.packages, t.textures, t.no_family), (14_474, 30_517, 0));
    assert_eq!(
        t.by_code,
        table(&[
            (0, &[("DX10:72", 5_232), ("fourcc:DXT1", 1_955)]),
            (1, &[("DX10:72", 2_383), ("fourcc:DXT1", 17)]),
            (5, &[("DX10:78", 1)]),
            (6, &[("rgb:41:bits16:7c00/3e0/1f/8000", 2)]),
            (10, &[("DX10:93", 1), ("rgb:40:bits32:ff0000/ff00/ff/0", 1)]),
            (102, &[("DX10:98", 9_473), ("DX10:99", 590)]),
            (103, &[("fourcc:ATI1", 997)]),
            (104, &[("fourcc:ATI2", 39)]),
            (
                105,
                &[("DX10:29", 1), ("rgb:41:bits32:ff/ff00/ff0000/ff000000", 28)]
            ),
            (106, &[("DX10:98", 6_386), ("DX10:99", 2)]),
            (107, &[("DX10:98", 1_112)]),
            (108, &[("fourcc:DXT1", 59)]),
            (110, &[("fourcc:DXT5", 4)]),
            (112, &[("DX10:99", 2)]),
            (115, &[("DX10:95", 2_232)]),
        ])
    );
    Ok(())
}

#[test]
fn elden_rings_format_codes() -> Result<()> {
    let Some(t) = census(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    println!(
        "{} packages, {} textures: {:?}",
        t.packages, t.textures, t.by_code
    );
    expect_found(
        Query::Install(inputs::Game::Elden, None),
        "texture packages",
        t.packages,
    );
    assert_eq!((t.packages, t.textures, t.no_family), (274_440, 301_807, 0));
    assert_eq!(
        t.by_code,
        table(&[
            (0, &[("DX10:72", 12_703), ("fourcc:DXT1", 3_393)]),
            (1, &[("DX10:72", 4_201)]),
            (9, &[("rgb:41:bits32:ff0000/ff00/ff/ff000000", 3)]),
            (10, &[("DX10:93", 4)]),
            (22, &[("fourcc:113", 1_193)]),
            (102, &[("DX10:98", 210_421), ("DX10:99", 686)]),
            (103, &[("DX10:80", 14), ("fourcc:ATI1", 7_092)]),
            (104, &[("fourcc:ATI2", 92)]),
            (105, &[("rgb:41:bits32:ff/ff00/ff0000/ff000000", 974)]),
            (106, &[("DX10:98", 9_653)]),
            (107, &[("DX10:98", 3_770)]),
            (108, &[("fourcc:DXT1", 3_072)]),
            (115, &[("DX10:95", 44_536)]),
        ])
    );
    // the order of the channels tells 9 from 105
    assert_eq!(PixelFamily::of_code(9), Some(PixelFamily::B8G8R8A8));
    assert_eq!(PixelFamily::of_code(105), Some(PixelFamily::R8G8B8A8));
    Ok(())
}
