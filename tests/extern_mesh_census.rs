//! Every `f` collision binder the installs hold (the map areas' `f<area>.hkxbhd`/`.hkxbdt`), through
//! [`collision::read_map`]: every member tagfile reads, every body is an `hknpExternMeshShape` of plain `hkGeometry`,
//! and the totals are those of the census that found them (12,688 Sekiro bodies in 1,145 files, 539,652 vertices and
//! 537,436 triangles; 264,858 Elden Ring bodies in 1,850 files, 2,233,804 vertices and 1,707,650 triangles).
//!
//! The binders are found from the games' MSB map names, not only from the dictionaries: a binder either archive holds
//! that its dictionary does not name is read too (the test counts them).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::collision::read_map;
use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::{Archive, Binder, Game, NameDictionary};
use havok_formats::{Compendium, Tagfile, extern_mesh};
use inputs::{Game as Data, OracleSet, Tool, oodle, oracle};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// What the `f` binders of one game hold.
#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    binders: usize,
    /// Binders no dictionary path names.
    unnamed_binders: usize,
    files: usize,
    bodies: usize,
    vertices: usize,
    triangles: usize,
    /// Triangles `read_map` returned (the sum over files).
    soup_triangles: usize,
    /// Files by the SDK version string of their tagfile.
    sdk_2016: usize,
    sdk_2018: usize,
    /// Triangles with a repeated corner.
    degenerate: usize,
}

/// The paths of the `f` binders the game's maps would have, and those its dictionary names.
fn binders(game: Game, archive: &Archive, names: &NameDictionary) -> (Vec<String>, usize) {
    let dictionary: BTreeSet<&str> = names
        .names
        .values()
        .flatten()
        .map(String::as_str)
        .filter(|p| {
            p.starts_with("/map/")
                && p.ends_with(".hkxbhd")
                && p.rsplit('/').next().is_some_and(|n| n.starts_with('f'))
        })
        .filter(|p| archive.contains(p))
        .collect();
    let mut all = dictionary.clone();
    let maps: BTreeSet<&str> = names
        .names
        .values()
        .flatten()
        .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
        .filter(|m| !m.contains('/'))
        .collect();
    let derived: Vec<String> = maps
        .iter()
        .flat_map(|map| {
            let rest = &map[1..];
            let flat = format!("/map/{map}/f{rest}.hkxbhd");
            let tiled = format!("/map/{}/{map}/f{rest}.hkxbhd", &map[..3]);
            match game {
                Game::Sekiro => vec![flat],
                Game::EldenRing => vec![tiled, flat],
            }
        })
        .filter(|p| archive.contains(p))
        .collect();
    all.extend(derived.iter().map(String::as_str));
    let unnamed = all.len() - dictionary.len();
    (all.into_iter().map(str::to_owned).collect(), unnamed)
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (Some(archive), Some(dll), Some(dictionary)) = (
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
    let names = NameDictionary::read_for(dictionary, game)?;
    let (paths, unnamed) = binders(game, &archive, &names);
    let mut t = Totals {
        binders: paths.len(),
        unnamed_binders: unnamed,
        ..Totals::default()
    };
    for path in &paths {
        let header = archive.read(path, &decoder)?;
        let payload = archive.read(&path.replace(".hkxbhd", ".hkxbdt"), &decoder)?;
        let binder = Binder::read_split(&header, &payload, &decoder)?;
        let mut compendia = Vec::new();
        for f in &binder.files {
            let bytes = decoder.decode_if_dcx(&f.bytes)?;
            if bytes.get(4..8) == Some(b"TCM0") {
                compendia.push(Compendium::read(&bytes)?);
            }
        }
        for f in &binder.files {
            let name = f.name.as_deref().unwrap_or("");
            if !name.ends_with(".hkx") && !name.ends_with(".hkx.dcx") {
                continue;
            }
            let bytes = decoder.decode_if_dcx(&f.bytes)?;
            // a file with its own types ignores the compendium; one without them needs the binder's
            let comp = compendia.first();
            let soup = read_map(&bytes, comp).map_err(|e| format!("{path}: {name}: {e}"))?;
            assert!(
                soup.unread_bodies.is_empty(),
                "{path}: {name}: {:?}",
                soup.unread_bodies
            );
            let tag = Tagfile::read(&bytes, comp)?;
            let variants = tag.root()?.field("namedVariants")?.array()?;
            let mut bodies = 0;
            for i in 0..variants.len() {
                let systems = variants
                    .get(i)?
                    .field("variant")?
                    .dereference()?
                    .field("systemDatas")?
                    .array()?;
                for j in 0..systems.len() {
                    let cinfos = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
                    for k in 0..cinfos.len() {
                        let shape = cinfos.get(k)?.field("shape")?.dereference()?;
                        let mesh = extern_mesh(shape).map_err(|e| format!("{path}: {name}: {e}"))?;
                        bodies += 1;
                        t.vertices += mesh.vertices.len();
                        t.triangles += mesh.triangles.len();
                        t.degenerate += mesh
                            .triangles
                            .iter()
                            .filter(|[a, b, c]| a == b || b == c || a == c)
                            .count();
                    }
                }
            }
            t.bodies += bodies;
            t.soup_triangles += soup.triangles.len();
            assert_eq!(soup.extern_triangles, soup.triangles.len(), "{path}: {name}");
            assert!(soup.materials.iter().all(|&m| m == -1));
            assert_eq!(soup.unmapped, 0);
            match &tag.version {
                b"20160200" => t.sdk_2016 += 1,
                b"20180100" => t.sdk_2018 += 1,
                other => return Err(format!("{path}: {name}: SDK {other:?}").into()),
            }
            t.files += 1;
        }
    }
    eprintln!("{} f binders: {t:?}", game.name());
    Ok(Some(t))
}

#[test]
fn sekiros_f_binders_read_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.binders, t.unnamed_binders, t.files), (9, 0, 1_145));
    assert_eq!((t.sdk_2016, t.sdk_2018, t.degenerate), (1_145, 0, 0));
    assert_eq!((t.bodies, t.vertices, t.triangles), (12_688, 539_652, 537_436));
    assert_eq!(t.soup_triangles, t.triangles);
    Ok(())
}

#[test]
fn elden_rings_f_binders_read_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.binders, t.unnamed_binders, t.files), (603, 0, 1_850));
    assert_eq!((t.sdk_2016, t.sdk_2018, t.degenerate), (67, 1_783, 0));
    assert_eq!(
        (t.bodies, t.vertices, t.triangles),
        (264_858, 2_233_804, 1_707_650)
    );
    assert_eq!(t.soup_triangles, t.triangles);
    Ok(())
}
