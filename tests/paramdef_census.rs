//! Every param of every regulation we hold, typed by its paramdef ([`ParamDefs::bind`]): both installs' param binders
//! (Elden Ring's `regulation.bin` 1.17.3, Sekiro's `gameparam.parambnd.dcx`) and the 28 Elden Ring regulations of
//! Smithbox's releases, read in place.
//!
//! The definitions are `oracles.smithbox.<game>/defs`: Paramdex's, as Smithbox ships them with `FirstVersion` and
//! `RemovedVersion` (upstream Paramdex has neither, and its Elden Ring definitions size 188 of the install's 194
//! params). The crate holds none of them; the regulations are `oracles.smithbox.elden/regulations`, one folder named
//! `<release> (<regulation number>)` per release holding its `regulation.bin`, decrypted with the install's
//! `regulation.key`.
//!
//! The counts are those of an independent census of the same files (a Python reader with its own PARAM parser, XML
//! parser, layout rule and value decoder, whose per-param digests of every row equal this crate's). Every param of a binder must bind (the layout size at the
//! binder's regulation number is the row size), every row must decode (text fields, no undeclared set bit), and the
//! values the hand-written readers of this crate hold (the NPC masks and stats, the grass models) must be the typed
//! values.
//!
//! Needs `installs.elden`, `keys.elden`, `installs.sekiro`, `keys.sekiro`, `tools.oodle6`, `oracles.smithbox.*` and
//! `scratch.root` (the per-param digests are written there for the independent census); a test whose input is not
//! configured does nothing.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use std::fmt::Write as _;
use std::path::Path;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::game_params::{param_binder, regulation_binder};
use fromsoft_formats::param::{
    AssetGeometryParam, DrawDistScaleParam, GrassParam, NpcParam, NpcStats, Param, PartsDrawParam,
};
use fromsoft_formats::paramdef::{ParamDefs, TypedParam, Value};
use fromsoft_formats::{Binder, Game, Install, Keys};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use sha2::{Digest, Sha256};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn defs(dir: &Path) -> Result<ParamDefs> {
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

/// A typed value as the independent census writes it: integers in decimal, floats as bit patterns, text as the hex of
/// its UTF-8.
fn canonical(out: &mut String, v: &Value) {
    match v {
        Value::Signed(n) => write!(out, "{n}"),
        Value::Unsigned(n) => write!(out, "{n}"),
        Value::Float(f) => write!(out, "f{:08x}", f.to_bits()),
        Value::Double(f) => write!(out, "d{:016x}", f.to_bits()),
        Value::Text(s) => {
            out.push('s');
            for b in s.bytes() {
                let _ = write!(out, "{b:02x}");
            }
            Ok(())
        }
        Value::Bytes(_) => Ok(()),
    }
    .unwrap();
}

fn digest(typed: &TypedParam<'_>) -> String {
    let mut hash = Sha256::new();
    let mut line = String::new();
    for row in typed.rows() {
        line.clear();
        let _ = write!(line, "{}", row.id);
        for (name, value) in row.fields() {
            let _ = write!(line, "\t{name}=");
            canonical(&mut line, &value);
        }
        line.push('\n');
        hash.update(line.as_bytes());
    }
    hash.finalize().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// What a binder's params came to.
#[derive(Default, Debug)]
struct Tally {
    members: usize,
    /// Members `Param::read` refused (the 32-bit form).
    unread: Vec<String>,
    bound: usize,
    rows: usize,
    cells: usize,
    /// Params whose definition's data version is not the file's.
    version_differs: usize,
    /// Members with no row (their size is not checked).
    empty: usize,
    refused: Vec<(String, String)>,
}

fn leaf(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_end_matches(".param")
        .to_owned()
}

/// Bind every member of `binder`; the digest lines go to `digests` as `label<TAB>member<TAB>type<TAB>rows<TAB>digest`.
fn bind_all(label: &str, binder: &Binder, defs: &ParamDefs, digests: &mut String) -> Tally {
    let regulation = binder.regulation_number();
    let mut tally = Tally::default();
    for file in &binder.files {
        tally.members += 1;
        let name = leaf(file.name.as_deref().unwrap_or(""));
        let Ok(param) = Param::read(&file.bytes) else {
            tally.unread.push(name);
            continue;
        };
        match defs.bind(&param, regulation) {
            Ok(typed) => {
                tally.bound += 1;
                tally.rows += param.rows.len();
                tally.cells += param.rows.len() * typed.layout.columns.len();
                tally.version_differs += usize::from(!typed.data_version_matches());
                tally.empty += usize::from(param.rows.is_empty());
                let _ = writeln!(
                    digests,
                    "{label}\t{name}\t{}\t{}\t{}",
                    param.param_type,
                    param.rows.len(),
                    digest(&typed)
                );
            }
            Err(e) => tally.refused.push((name, e.to_string())),
        }
    }
    tally
}

/// The rows [`cross_check`] compared, by reader.
#[derive(Debug, Default, PartialEq, Eq)]
struct Checked {
    npc: usize,
    grass: usize,
    parts_draw: usize,
    asset_geometry: usize,
    draw_dist_scale: usize,
}

/// The typed values of `NPC_PARAM_ST`, the grass params, the outfit-free stats and the texture-switch params
/// (`PartsDrawParam`, `AssetEnvironmentGeometryParam`, `LoadBalancerDrawDistScaleParam`) equal what the hand-written
/// readers read from the same rows.
fn cross_check(game: Game, binder: &Binder, defs: &ParamDefs) -> Checked {
    let regulation = binder.regulation_number();
    let mut checked = Checked::default();
    for file in &binder.files {
        let Ok(param) = Param::read(&file.bytes) else {
            continue;
        };
        match param.param_type.as_str() {
            "NPC_PARAM_ST" => {
                let typed = defs.bind(&param, regulation).unwrap();
                let masks = NpcParam::from_param(&param, &game.npc_param()).unwrap();
                let stats = NpcStats::from_param(&param, &game.npc_stats()).unwrap();
                for ((row, mask), stat) in typed.rows().zip(&masks.rows).zip(&stats) {
                    assert_eq!(row.id, mask.id);
                    let mut bits = 0u32;
                    for k in 0..32u32 {
                        let v = row.get(&format!("modelDispMask{k}")).unwrap();
                        assert!(matches!(v, Value::Unsigned(0 | 1)), "{v:?}");
                        bits |= u32::try_from(v.as_i64().unwrap()).unwrap() << k;
                    }
                    assert_eq!(bits, mask.display_mask, "row {} display mask", row.id);
                    for (name, value, _) in stat.fields() {
                        let typed_value = row.get(name).unwrap().as_f64().unwrap();
                        assert_eq!(typed_value, value, "row {} {name}", row.id);
                    }
                    checked.npc += 1;
                }
            }
            "GRASS_TYPE_PARAM_ST" => {
                let typed = defs.bind(&param, regulation).unwrap();
                let grass = GrassParam::from_param(&param).unwrap();
                let model0 = if game == Game::Sekiro {
                    "modelName"
                } else {
                    "model0Name"
                };
                for (row, hand) in typed.rows().zip(&grass.rows) {
                    assert_eq!(row.id, hand.id);
                    assert_eq!(
                        row.get("lodRange").unwrap().as_i64(),
                        Some(i64::from(hand.lod_range))
                    );
                    assert_eq!(
                        row.get("baseDensity").unwrap().as_f64(),
                        Some(f64::from(hand.base_density))
                    );
                    assert_eq!(
                        row.get("spacing").unwrap().as_f64(),
                        Some(f64::from(hand.spacing))
                    );
                    let text = |n: &str| row.get(n).unwrap().as_str().unwrap().to_owned();
                    assert_eq!(text(model0), hand.model0);
                    assert_eq!(text("simpleModelName"), hand.simple_model);
                    assert_eq!(text("flatTextureName"), hand.flat_texture);
                    assert_eq!(text("billboardTextureName"), hand.billboard_texture);
                    if game == Game::EldenRing {
                        assert_eq!(text("model1Name"), hand.model1);
                    }
                    checked.grass += 1;
                }
            }
            "PARTS_DRAW_PARAM_ST" if game == Game::EldenRing => {
                let typed = defs.bind(&param, regulation).unwrap();
                let hand = PartsDrawParam::from_param(&param).unwrap();
                for (row, hand) in typed.rows().zip(&hand.rows) {
                    assert_eq!(row.id, hand.id);
                    let float = |n: &str| row.get(n).unwrap().as_f64().unwrap();
                    assert_eq!(float("tex_lv01_BorderDist"), f64::from(hand.tex_lv01_border));
                    assert_eq!(float("tex_lv01_PlayDist"), f64::from(hand.tex_lv01_play));
                    assert_eq!(
                        row.get("lodType").unwrap().as_i64(),
                        Some(i64::from(hand.lod_type))
                    );
                    checked.parts_draw += 1;
                }
            }
            "ASSET_GEOMETORY_PARAM_ST" if game == Game::EldenRing => {
                let typed = defs.bind(&param, regulation).unwrap();
                let hand = AssetGeometryParam::from_param(&param).unwrap();
                for (row, hand) in typed.rows().zip(&hand.rows) {
                    assert_eq!(row.id, hand.id);
                    assert_eq!(
                        row.get("refDrawParamId").unwrap().as_i64(),
                        Some(i64::from(hand.draw_param))
                    );
                    checked.asset_geometry += 1;
                }
            }
            "LOAD_BALANCER_DRAW_DIST_SCALE_PARAM_ST" if game == Game::EldenRing => {
                let typed = defs.bind(&param, regulation).unwrap();
                let hand = DrawDistScaleParam::from_param(&param).unwrap();
                for (row, hand) in typed.rows().zip(&hand.rows) {
                    assert_eq!(row.id, hand.id);
                    for (k, &level) in hand.levels.iter().enumerate() {
                        let typed_value = row.get(&format!("Lv{k:02}")).unwrap().as_f64().unwrap();
                        assert_eq!(typed_value, f64::from(level), "row {} Lv{k:02}", row.id);
                    }
                    checked.draw_dist_scale += 1;
                }
            }
            _ => {}
        }
    }
    checked
}

fn write_digests(name: &str, text: &str) {
    let dir = inputs::scratch().join("paramdef-census");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
}

fn configured(game: Game) -> Option<(Install, inputs::OodleLibrary)> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (root, folder, dll) = (install(data)?, keys(data)?, oodle(Tool::Oodle6)?);
    let keys = Keys::from_folder(folder, game, None).ok()?;
    Some((Install::new(root, game, keys), dll))
}

#[test]
fn every_param_of_the_installs_binds() -> Result<()> {
    // (game, defs folder's game, members, read by Param::read, rows, params whose def revision differs, rows the
    // hand-written readers read; the texture-switch readers are Elden Ring's)
    let checked = |npc, grass, parts_draw, asset_geometry, draw_dist_scale| Checked {
        npc,
        grass,
        parts_draw,
        asset_geometry,
        draw_dist_scale,
    };
    for (game, set, members, unread, rows, differs, expected) in [
        (
            Game::EldenRing,
            Data::Elden,
            194,
            0,
            179_358,
            10,
            checked(7_045, 632, 385, 24_106, 154),
        ),
        (
            Game::Sekiro,
            Data::Sekiro,
            138,
            3,
            36_083,
            0,
            checked(1_279, 166, 0, 0, 0),
        ),
    ] {
        let (Some((install, dll)), Some(dir)) = (configured(game), oracle(OracleSet::Smithbox, set, "defs"))
        else {
            continue;
        };
        let defs = defs(&dir)?;
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let binder = param_binder(&install, &install.open()?, &decoder)?;
        let mut digests = String::new();
        let tally = bind_all("install", &binder, &defs, &mut digests);
        println!("{}: {tally:?}", game.name());
        assert!(tally.refused.is_empty(), "{:?}", tally.refused);
        assert_eq!(tally.members, members);
        assert_eq!(tally.unread.len(), unread, "{:?}", tally.unread);
        assert_eq!(tally.bound, members - unread);
        assert_eq!(tally.rows, rows);
        assert_eq!(tally.version_differs, differs);
        assert_eq!(tally.empty, 0);
        match game {
            Game::EldenRing => assert_eq!(binder.regulation_number(), Some(11_730_034)),
            Game::Sekiro => assert_eq!(binder.regulation_number(), None),
        }
        assert_eq!(cross_check(game, &binder, &defs), expected);
        write_digests(
            &format!("{}-install.tsv", game.name().replace(' ', "-")),
            &digests,
        );
    }
    Ok(())
}

#[test]
fn every_param_of_smithbox_regulations_binds() -> Result<()> {
    let (Some((_, _)), Some(defs_dir), Some(regs)) = (
        configured(Game::EldenRing),
        oracle(OracleSet::Smithbox, Data::Elden, "defs"),
        oracle(OracleSet::Smithbox, Data::Elden, "regulations"),
    ) else {
        return Ok(());
    };
    let defs = defs(&defs_dir)?;
    let keys = Keys::from_folder(keys(Data::Elden).unwrap(), Game::EldenRing, None)?;
    let key = keys.regulation().unwrap();
    let dll = oodle(Tool::Oodle6).unwrap();
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let mut folders: Vec<_> = std::fs::read_dir(&regs)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("regulation.bin").exists())
        .collect();
    folders.sort();
    let mut digests = String::new();
    let (mut members, mut bound, mut rows, mut cells, mut differs) = (0, 0, 0, 0, 0);
    let mut refused = Vec::new();
    for folder in &folders {
        let label = folder.file_name().unwrap().to_string_lossy().into_owned();
        let binder = regulation_binder(key, &std::fs::read(folder.join("regulation.bin"))?, &decoder)?;
        // the folder names the regulation number the binder's version spells
        let number = binder.regulation_number().unwrap();
        assert!(label.ends_with(&format!("({number})")), "{label}: {number}");
        let tally = bind_all(&label, &binder, &defs, &mut digests);
        assert!(tally.unread.is_empty(), "{label}: {:?}", tally.unread);
        println!("{label}: {} members, {} rows", tally.members, tally.rows);
        members += tally.members;
        bound += tally.bound;
        rows += tally.rows;
        cells += tally.cells;
        differs += tally.version_differs;
        refused.extend(
            tally
                .refused
                .into_iter()
                .map(|(n, e)| format!("{label} {n}: {e}")),
        );
    }
    println!(
        "{} regulations: {bound} of {members} params typed, {rows} rows, {cells} cells, {differs} definition revisions differ from the file's",
        folders.len()
    );
    assert!(refused.is_empty(), "{refused:#?}");
    assert_eq!(folders.len(), 28);
    assert_eq!(members, 5_295);
    assert_eq!(bound, members);
    assert_eq!(rows, 4_137_031);
    // the install's 10 are counted with them in the regulation census's 151 of 5,489
    assert_eq!(differs, 141);
    write_digests("smithbox-regulations.tsv", &digests);
    Ok(())
}
