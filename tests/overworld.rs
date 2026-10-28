//! Elden Ring's `WorldMapLegacyConvParam` and the legacy maps' places in the overworld
//! ([`fromsoft_formats::overworld`]) on the install's regulation.
//!
//! The counts are those of an independent census (a separate parser of the decoded param's header and rows, and its own
//! derivation of the places by the rules, over Elden Ring 1.17.3's `regulation.bin`). The collision check reads the
//! Academy of Raya Lucaria's `m14_00_00_00` and its two overworld tiles, `m60_35_45_00` and `m60_35_46_00`.
//!
//! Needs `installs.elden`, `keys.elden` and `tools.oodle6`; a test whose game is not configured does nothing.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use fromsoft_formats::collision;
use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::{read_legacy_placements, read_param, regulation_binder};
use fromsoft_formats::mapid::MapId;
use fromsoft_formats::overworld::{ConvParam, Manual, Placements, Source, tile_origin};
use fromsoft_formats::param::Param;
use fromsoft_formats::paramdef::ParamDefs;
use fromsoft_formats::{Archive, Binder, Game, Install, Keys};
use havok_formats::Compendium;
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn configured() -> Option<(Install, inputs::OodleLibrary)> {
    let (root, folder, dll) = (install(Data::Elden)?, keys(Data::Elden)?, oodle(Tool::Oodle6)?);
    let keys = Keys::from_folder(folder, Game::EldenRing, None).ok()?;
    Some((Install::new(root, Game::EldenRing, keys), dll))
}

fn id(a: u8, x: u8, z: u8) -> MapId {
    MapId::from_bytes([a, x, z, 0])
}

#[test]
fn every_row_reads_and_every_map_the_rules_can_place_is_placed() -> Result<()> {
    let Some((install, dll)) = configured() else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = install.open()?;
    let param = read_param(&install, &archive, &decoder, "WorldMapLegacyConvParam.param")?;
    let conv = ConvParam::from_param(&param, &Game::EldenRing.legacy_conv().unwrap())?;
    assert_eq!(conv.rows.len(), 196);
    assert_eq!(conv.rows.iter().filter(|r| r.is_base_point).count(), 194);
    assert_eq!(conv.rows.iter().filter(|r| r.disabled_in_nt).count(), 183);
    let placements = read_legacy_placements(&install, &archive, &decoder)?;
    assert_eq!(placements, Placements::from_conv(&conv));
    let by_source = |pick: fn(&Source) -> bool| placements.maps.values().filter(|p| pick(&p.source)).count();
    assert_eq!(by_source(|s| matches!(s, Source::BaseRow(_))), 83);
    assert_eq!(by_source(|s| matches!(s, Source::SharedOrigin { .. })), 10);
    assert_eq!(by_source(|s| matches!(s, Source::Chained { .. })), 2);
    assert_eq!(by_source(|s| matches!(s, Source::Manual(_))), 5);
    // the maps the rules cannot place are those of the areas 37, 38, 39 (block 0) and 70: none is in the base game's archives
    let unplaced: Vec<String> = placements.unplaced.iter().map(|m| m.to_string()).collect();
    assert_eq!(unplaced.len(), 15, "{unplaced:?}");
    for m in &placements.unplaced {
        assert!(matches!(m.area, 37 | 38 | 39 | 70), "{m}");
    }
    // an independent derivation (a separate parser of the decoded param and the rules written out again in a script) renders
    // the 100 places to the millimetre as `map tile x y z` lines in map order; this reading renders the same text
    let round = |v: f64| format!("{:.3}", (v * 1000.).round() / 1000. + 0.0);
    let rendered: String = placements
        .maps
        .iter()
        .map(|(map, p)| {
            format!(
                "{map} {} {} {} {}\n",
                p.tile,
                round(p.offset[0]),
                round(p.offset[1]),
                round(p.offset[2])
            )
        })
        .collect();
    let digest: String = Sha256::digest(rendered.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest, "acbaa2b8c420fe5272f794a0b1530b33e508ded4bf2e2f8d9dcb0c2509ce30f3",
        "{rendered}"
    );
    // the Academy
    let academy = placements.get(id(14, 0, 0)).unwrap();
    assert_eq!(academy.tile, id(60, 35, 45));
    assert_eq!(academy.source, Source::BaseRow(133));
    assert!(
        academy
            .offset
            .iter()
            .zip([-112.058, 310.577, 194.278])
            .all(|(a, b)| (a - b).abs() < 1e-3),
        "{:?}",
        academy.offset
    );
    // the manual cases
    assert_eq!(
        placements.get(id(11, 71, 0)).unwrap().source,
        Source::Manual(Manual::SameOrigin(id(11, 5, 0)))
    );
    assert_eq!(
        placements.get(id(60, 0, 0)).unwrap().global_origin(),
        Some((60, [10624., 0., 8576.]))
    );
    Ok(())
}

/// Every row whose two ends are both placed says the same thing as the places do, to within 5 cm, except the rows listed.
#[test]
fn the_rows_agree_with_the_places_they_produce() -> Result<()> {
    let Some((install, dll)) = configured() else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = install.open()?;
    let param = read_param(&install, &archive, &decoder, "WorldMapLegacyConvParam.param")?;
    let conv = ConvParam::from_param(&param, &Game::EldenRing.legacy_conv().unwrap())?;
    let placements = Placements::from_conv(&conv);
    let origin = |map: MapId| -> Option<(u8, [f64; 3])> {
        if let Some((area, at)) = tile_origin(map) {
            return Some((area, [at[0], 0., at[1]]));
        }
        placements.get(map)?.global_origin()
    };
    let (mut both, mut agree, mut off) = (0, 0, Vec::new());
    for row in &conv.rows {
        let (Some((a, o)), Some((b, p))) = (origin(row.src), origin(row.dst)) else {
            continue;
        };
        if a != b {
            continue;
        }
        both += 1;
        let worst = (0..3)
            .map(|k| (o[k] + f64::from(row.src_position[k]) - p[k] - f64::from(row.dst_position[k])).abs())
            .fold(0., f64::max);
        if worst < 0.05 {
            agree += 1;
        } else {
            off.push(row.id);
        }
    }
    eprintln!("rows with both ends placed {both}, agreeing {agree}, not: {off:?}");
    assert_eq!((both, agree), (184, 169));
    assert_eq!(
        off,
        [
            10, 17, 127, 130, 131, 132, 136, 139, 222, 223, 1113, 1114, 1115, 1116, 1133
        ]
    );
    Ok(())
}

/// The triangles of collision model `model` (`h354690`) of `map` (`m60_35_46_00`) from the map's high-resolution collision binder.
fn collision_of(
    archive: &Archive,
    decoder: &Decoder<'_>,
    map: &str,
    model: &str,
) -> Result<Vec<[[f32; 3]; 3]>> {
    let base = format!("{}/h{}", Game::EldenRing.map_folder(map)?, &map[1..]);
    let binder = Binder::read_split(
        &archive.read(&format!("{base}.hkxbhd"), decoder)?,
        &archive.read(&format!("{base}.hkxbdt"), decoder)?,
        decoder,
    )?;
    let leaf = |f: &fromsoft_formats::BinderFile| {
        let name = f.name.as_deref().unwrap_or("").to_ascii_lowercase();
        name.rsplit(['/', '\\'])
            .next()
            .unwrap_or("")
            .trim_end_matches(".dcx")
            .to_owned()
    };
    let compendium = binder
        .files
        .iter()
        .find(|f| leaf(f).ends_with(".compendium"))
        .ok_or("no compendium")?;
    let compendium = Compendium::read(&decoder.decode_if_dcx(&compendium.bytes)?)?;
    let wanted = format!("h{}_{}.hkx", &map[1..], &model[1..]);
    let file = binder
        .files
        .iter()
        .find(|f| leaf(f) == wanted)
        .ok_or(format!("no {wanted}"))?;
    Ok(collision::read_map(&decoder.decode_if_dcx(&file.bytes)?, Some(&compendium))?.triangles)
}

/// The box of a triangle soup, minimum then maximum corner, in `f64`.
fn bounds(triangles: &[[[f32; 3]; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for v in triangles.iter().flatten() {
        for k in 0..3 {
            lo[k] = lo[k].min(f64::from(v[k]));
            hi[k] = hi[k].max(f64::from(v[k]));
        }
    }
    (lo, hi)
}

/// The Academy of Raya Lucaria `m14_00_00_00` placed in `m60_35_46_00` by its row meets that tile's collision where the two
/// maps connect: the three trigger quads `h901008` to `h901010` of the Academy, 27.936 m squares, are centred on the tile's
/// `h354690` quad (30.73 m) to within 1 cm in X and Z, the lowest at the tile quad's height, and the Academy's `h000800` stands on the tile's `h354603`
/// (the same edge in X and Z to within 1 cm, the one's floor 6 cm under the other's top). An ordinary tile origin would not:
/// the quads' centres are 256 m apart under the neighbouring tile's origin and 128 m under a tile corner's.
#[test]
fn the_academy_lines_up_with_its_overworld_tile() -> Result<()> {
    let Some((install, dll)) = configured() else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let archive = install.open()?;
    let placements = read_legacy_placements(&install, &archive, &decoder)?;
    let tile = id(60, 35, 46);
    let at = placements
        .get(id(14, 0, 0))
        .unwrap()
        .origin_in_frame_of(tile)
        .unwrap();
    let shifted = |(lo, hi): ([f64; 3], [f64; 3])| -> ([f64; 3], [f64; 3]) {
        (
            [lo[0] + at[0], lo[1] + at[1], lo[2] + at[2]],
            [hi[0] + at[0], hi[1] + at[1], hi[2] + at[2]],
        )
    };
    let (tile_lo, tile_hi) = bounds(&collision_of(&archive, &decoder, "m60_35_46_00", "h354690")?);
    assert!(
        (tile_hi[0] - tile_lo[0] - 30.73).abs() < 0.01,
        "{tile_lo:?} {tile_hi:?}"
    );
    for model in ["h901008", "h901009", "h901010"] {
        let (lo, hi) = shifted(bounds(&collision_of(&archive, &decoder, "m14_00_00_00", model)?));
        assert!((hi[0] - lo[0] - 27.936).abs() < 0.01 && (hi[2] - lo[2] - 27.936).abs() < 0.01);
        for k in [0, 2] {
            let (a, b) = ((lo[k] + hi[k]) / 2., (tile_lo[k] + tile_hi[k]) / 2.);
            assert!((a - b).abs() < 0.01, "{model} axis {k}: {a} against {b}");
        }
        if model == "h901008" {
            assert!(
                (lo[1] - tile_lo[1]).abs() < 0.01,
                "height {} against {}",
                lo[1],
                tile_lo[1]
            );
        }
    }
    // the tower on the tile's block
    let (tower_lo, tower_hi) = shifted(bounds(&collision_of(
        &archive,
        &decoder,
        "m14_00_00_00",
        "h000800",
    )?));
    let (_, block_hi) = bounds(&collision_of(&archive, &decoder, "m60_35_46_00", "h354603")?);
    assert!(
        (tower_hi[0] - block_hi[0]).abs() < 0.01,
        "x edge {} against {}",
        tower_hi[0],
        block_hi[0]
    );
    assert!(
        (tower_hi[2] - block_hi[2]).abs() < 0.01,
        "z edge {} against {}",
        tower_hi[2],
        block_hi[2]
    );
    assert!(
        (tower_lo[1] - block_hi[1]).abs() < 0.1,
        "floor {} against top {}",
        tower_lo[1],
        block_hi[1]
    );
    // under another tile's origin the same quads are far apart
    let other = placements
        .get(id(14, 0, 0))
        .unwrap()
        .origin_in_frame_of(id(60, 35, 45))
        .unwrap();
    assert!((other[2] - at[2] - 256.).abs() < 1e-6);
    Ok(())
}

fn defs(dir: &std::path::Path) -> Result<ParamDefs> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();
    let texts = files
        .iter()
        .map(std::fs::read_to_string)
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(ParamDefs::from_xml(texts.iter().map(String::as_str))?)
}

/// The rows of `conv` are the paramdef-typed values of the same param (an independent decoder: Paramdex's definition laid
/// out by the container rule, every field by name).
fn same_as_typed(param: &Param, conv: &ConvParam, defs: &ParamDefs, regulation: Option<u32>) -> Result<()> {
    let typed = defs.bind(param, regulation)?;
    assert_eq!(typed.rows().count(), conv.rows.len());
    for (typed, row) in typed.rows().zip(&conv.rows) {
        assert_eq!(typed.id, row.id);
        let int = |name: &str| typed.get(name).unwrap().as_i64().unwrap();
        let float = |name: &str| typed.get(name).unwrap().as_f64().unwrap();
        assert_eq!(
            [int("srcAreaNo"), int("srcGridXNo"), int("srcGridZNo")],
            [row.src.area, row.src.x, row.src.z].map(i64::from)
        );
        assert_eq!(
            [int("dstAreaNo"), int("dstGridXNo"), int("dstGridZNo")],
            [row.dst.area, row.dst.x, row.dst.z].map(i64::from)
        );
        for (k, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
            assert_eq!(
                float(&format!("srcPos{axis}")),
                f64::from(row.src_position[k]),
                "row {}",
                row.id
            );
            assert_eq!(
                float(&format!("dstPos{axis}")),
                f64::from(row.dst_position[k]),
                "row {}",
                row.id
            );
        }
        assert_eq!(int("isBasePoint"), i64::from(row.is_base_point));
        assert_eq!(int("disableParam_NT"), i64::from(row.disabled_in_nt));
    }
    Ok(())
}

/// A second decoder (the param typed by Smithbox's Paramdex definition) reads the same 196 rows as the hand-laid layout,
/// field for field, in the install's regulation; and the other 28 Elden Ring regulations of Smithbox's releases are
/// counted: which hold the param, in which layout, how many rows, how many maps those rows place.
#[test]
fn a_second_decoder_agrees_and_the_regulations_are_counted() -> Result<()> {
    let (Some((install, dll)), Some(defs_dir), Some(regulations)) = (
        configured(),
        oracle(OracleSet::Smithbox, Data::Elden, "defs"),
        oracle(OracleSet::Smithbox, Data::Elden, "regulations"),
    ) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let defs = defs(&defs_dir)?;
    let archive = install.open()?;
    let binder = fromsoft_formats::game_params::param_binder(&install, &archive, &decoder)?;
    let param = Param::read(
        &binder
            .member("WorldMapLegacyConvParam.param")
            .ok_or("no param")?
            .bytes,
    )?;
    let conv = ConvParam::from_param(&param, &Game::EldenRing.legacy_conv().unwrap())?;
    same_as_typed(&param, &conv, &defs, binder.regulation_number())?;
    // the 28 other regulations
    let key = install.keys.regulation().ok_or("no regulation key")?;
    let mut folders: Vec<_> = std::fs::read_dir(&regulations)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("regulation.bin").exists())
        .collect();
    folders.sort();
    let mut table = Vec::new();
    let mut shapes = std::collections::BTreeMap::<(usize, usize), usize>::new();
    for folder in &folders {
        let label = folder.file_name().unwrap().to_string_lossy().into_owned();
        let binder = regulation_binder(key, &std::fs::read(folder.join("regulation.bin"))?, &decoder)?;
        let Some(member) = binder.member("WorldMapLegacyConvParam.param") else {
            table.push(format!("{label}: no WorldMapLegacyConvParam"));
            continue;
        };
        let param = Param::read(&member.bytes)?;
        let shape = (param.data_version, param.row_size, param.rows.len());
        match ConvParam::from_param(&param, &Game::EldenRing.legacy_conv().unwrap()) {
            Ok(conv) => {
                same_as_typed(&param, &conv, &defs, binder.regulation_number())?;
                let placed = Placements::from_conv(&conv);
                *shapes.entry((conv.rows.len(), placed.maps.len())).or_default() += 1;
                table.push(format!(
                    "{label}: layout {shape:?}, {} maps placed, {} unplaced",
                    placed.maps.len(),
                    placed.unplaced.len()
                ));
            }
            Err(e) => table.push(format!("{label}: layout {shape:?} not read: {e}")),
        }
    }
    for line in &table {
        println!("{line}");
    }
    assert_eq!(table.len(), 28);
    // 1.00.0 to 1.10 (16 releases) hold 146 rows placing 81 maps, 1.12.1 on (12) the 196 rows of the install placing 100
    assert_eq!(shapes, [((146, 81), 16), ((196, 100), 12)].into());
    Ok(())
}
