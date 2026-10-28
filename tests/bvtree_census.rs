//! The bounding-volume trees of every compressed mesh the installs' map and object collision hold: every mesh tree
//! (5-byte nodes) and every section tree (4-byte nodes) decodes, their leaves name each section and each primitive
//! exactly once, every section's `leafIndex` is its leaf's node, and each mesh's simd tree names its triangles, quads and
//! custom primitives once each. The counts are those of the census that found the rules (Sekiro 3,344 mesh trees and
//! 273,228 section trees; Elden Ring 22,899 and 722,405, once the name dictionary names every archive entry) and of a
//! second decoder run over the same files.
//!
//! Every triangle and quad is also tested against the box of its leaf, and every simd leaf against its primitive: the
//! counts inside, off by rounding and outside are asserted as found, so a change to a decoder or to the box rule moves them.
//!
//! The meshes are read two ways: walked in the tagfiles (`direct`, which holds every mesh) and through
//! [`collision::read_map`] (the map `h` and `l` binders) and [`collision::read_bodies`] (Sekiro's `obj/*.objbnd`, Elden
//! Ring's `asset/aeg/*_{l,h}.geomhkxbnd`), whose [`Soup`](fromsoft_formats::collision::Soup) carries the counts of the
//! meshes they read (`wired`). The two agree except for the meshes those readers do not return as a mesh: `read_map`
//! leaves a plain `hknpCompressedMeshShape` body unread, and `read_bodies` returns the extern shapes of a mesh made of
//! them alone and no mesh body. The trees of such a mesh are decoded and checked all the same: a broken one is refused.
//!
//! The binders are those the games' name dictionaries give. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::collision::{BodyGeometry, read_bodies, read_map};
use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::{Archive, Binder, Game, NameDictionary};
use havok_formats::{BoundsCheck, Compendium, CustomKind, SimdCheck, Tagfile, compressed_mesh};
use inputs::{Game as Data, OracleSet, Tool, oodle, oracle};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The tree counts of a set of meshes.
#[derive(Debug, Default, Clone, PartialEq)]
struct Counts {
    bounds: BoundsCheck,
    simd: SimdCheck,
}

impl Counts {
    fn merge(&mut self, other: &Self) {
        self.bounds.merge(&other.bounds);
        self.simd.merge(&other.simd);
    }
}

/// What one game's collision binders hold.
#[derive(Debug, Default, Clone, PartialEq)]
struct Totals {
    /// Tagfiles read.
    files: usize,
    /// Every compressed mesh, walked in the tagfile.
    direct: Counts,
    /// The meshes `read_map` and `read_bodies` return as meshes.
    wired: Counts,
    /// Meshes made of extern shapes alone (no triangle), which `read_bodies` returns as those shapes.
    extern_only: usize,
    /// Bodies `read_map` does not read, by shape class.
    unread: BTreeMap<String, usize>,
}

impl Totals {
    fn merge(&mut self, other: Self) {
        self.files += other.files;
        self.direct.merge(&other.direct);
        self.wired.merge(&other.wired);
        self.extern_only += other.extern_only;
        for (class, n) in other.unread {
            *self.unread.entry(class).or_default() += n;
        }
    }
}

/// Whether a binder is one of the game's compressed-mesh collision sets, and whether it is a map binder (split) or a
/// single object binder.
fn kind(game: Game, path: &str) -> Option<bool> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let map = path.starts_with("/map/")
        && path.ends_with(".hkxbhd")
        && (name.starts_with('h') || name.starts_with('l'));
    let object = match game {
        Game::Sekiro => path.starts_with("/obj/") && path.ends_with(".objbnd.dcx"),
        Game::EldenRing => path.starts_with("/asset/aeg/") && path.ends_with(".geomhkxbnd.dcx"),
    };
    if map { Some(true) } else { object.then_some(false) }
}

/// Every compressed mesh of a tagfile, whatever reads it: its tree counts, and how many are made of extern shapes alone.
fn walk(bytes: &[u8], compendium: Option<&Compendium>) -> Result<(Counts, usize)> {
    let tag = Tagfile::read(bytes, compendium)?;
    let mut counts = Counts::default();
    let mut extern_only = 0;
    let variants = tag.root()?.field("namedVariants")?.array()?;
    for i in 0..variants.len() {
        let physics = variants.get(i)?.field("variant")?.dereference()?;
        if physics.type_name() != "hknpPhysicsSceneData" {
            continue;
        }
        let systems = physics.field("systemDatas")?.array()?;
        for j in 0..systems.len() {
            let bodies = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
            for k in 0..bodies.len() {
                let shape = bodies.get(k)?.field("shape")?.dereference()?;
                if !matches!(
                    shape.type_name(),
                    "fsnpCustomParamCompressedMeshShape" | "hknpCompressedMeshShape"
                ) {
                    continue;
                }
                let mesh = compressed_mesh(shape.field("data")?.dereference()?)?;
                counts.bounds.merge(&mesh.bounds);
                counts.simd.merge(&mesh.simd);
                if mesh.triangles.is_empty()
                    && mesh
                        .custom
                        .iter()
                        .any(|c| matches!(c.kind, CustomKind::Extern(_)))
                {
                    extern_only += 1;
                }
            }
        }
    }
    Ok((counts, extern_only))
}

/// One binder's tagfiles, walked and through the reader that fits its kind.
fn read_binder(archive: &Archive, decoder: &Decoder<'_>, path: &str, map: bool) -> Result<Totals> {
    let header = archive.read(path, decoder)?;
    let binder = if map {
        let payload = archive.read(&path.replace(".hkxbhd", ".hkxbdt"), decoder)?;
        Binder::read_split(&header, &payload, decoder)?
    } else {
        Binder::read(&header, decoder)?
    };
    let mut compendia = Vec::new();
    let mut members = Vec::new();
    for f in &binder.files {
        let name = f.name.as_deref().unwrap_or("").to_ascii_lowercase();
        let hkx = name.ends_with(".hkx") || name.ends_with(".hkx.dcx");
        if !hkx && !name.ends_with(".compendium") && !name.ends_with(".compendium.dcx") {
            continue;
        }
        let bytes = decoder.decode_if_dcx(&f.bytes)?;
        if bytes.get(4..8) == Some(b"TCM0") {
            compendia.push(Compendium::read(&bytes)?);
        } else if hkx && bytes.get(4..8) == Some(b"TAG0") {
            members.push((name, bytes));
        }
    }
    let mut totals = Totals::default();
    for (name, bytes) in &members {
        // the member's own types, or the first compendium that has them
        let compendium = std::iter::once(None)
            .chain(compendia.iter().map(Some))
            .find(|c| Tagfile::read(bytes, *c).is_ok())
            .flatten();
        let fail = |e: &dyn std::fmt::Display| format!("{path}: {name}: {e}");
        let (direct, extern_only) = walk(bytes, compendium).map_err(|e| fail(&e))?;
        totals.direct.merge(&direct);
        totals.extern_only += extern_only;
        if map {
            let soup = read_map(bytes, compendium).map_err(|e| fail(&e))?;
            totals.wired.bounds.merge(&soup.bounds);
            totals.wired.simd.merge(&soup.simd);
            for class in soup.unread_bodies {
                *totals.unread.entry(class).or_default() += 1;
            }
        } else {
            for body in read_bodies(bytes, compendium).map_err(|e| fail(&e))? {
                if let BodyGeometry::Mesh(soup) = body.geometry {
                    totals.wired.bounds.merge(&soup.bounds);
                    totals.wired.simd.merge(&soup.simd);
                }
            }
        }
        totals.files += 1;
    }
    Ok(totals)
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (Some(archive), Some(dll), Some(dictionary)) = (
        common::archive(game).map_err(|e| e.to_string())?,
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let names = NameDictionary::read_for(dictionary, game)?;
    let mut binders: Vec<(String, bool)> = names
        .names
        .iter()
        .filter(|(hash, _)| archive.entries.contains_key(*hash))
        .flat_map(|(_, paths)| paths)
        .filter_map(|p| kind(game, p).map(|map| (p.clone(), map)))
        .collect();
    binders.sort();
    binders.dedup();
    let next = AtomicUsize::new(0);
    let total = Mutex::new(Totals::default());
    let failures = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                let decoder = Decoder {
                    oodle: Some(&dll),
                    max_output: 1 << 30,
                };
                let mut mine = Totals::default();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, map)) = binders.get(i) else {
                        break;
                    };
                    match read_binder(&archive, &decoder, path, *map) {
                        Ok(t) => mine.merge(t),
                        Err(e) => failures.lock().unwrap().push(format!("{path}: {e}")),
                    }
                }
                total.lock().unwrap().merge(mine);
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} binders failed: {failures:#?}",
        failures.len()
    );
    let total = total.into_inner().unwrap();
    eprintln!(
        "{} bounding-volume trees: {} binders, {total:?}",
        game.name(),
        binders.len()
    );
    Ok(Some(total))
}

/// What a game's collision holds, as the census that found the rules counted it and as a second decoder over the same
/// files counts it.
struct Expected {
    /// Tagfiles of the map `h`/`l` binders and of the object (Sekiro) or asset (Elden Ring) binders.
    files: usize,
    mesh_trees: usize,
    section_trees: usize,
    /// Leaves of the section trees: every primitive, dead and custom ones included.
    primitives: usize,
    nodes: usize,
    /// Triangles and quads tested against their leaf's box: inside, off by rounding, outside.
    boxes: (usize, usize, usize),
    inverted_boxes: usize,
    /// The simd trees: those with no leaf, and their nodes.
    leafless_simd_trees: usize,
    simd_nodes: usize,
    /// The simd leaves: boxes exactly the primitive's, larger than it, off by rounding, outside it; and the custom
    /// primitives' (they have no vertices to compare).
    simd_leaves: (usize, usize, usize, usize, usize),
    /// Child lanes whose box is exactly the union of the lanes below.
    simd_children_exact: usize,
}

fn check(t: &Totals, want: &Expected) {
    assert_eq!(t.files, want.files, "tagfiles");
    // the census's counts, over every mesh
    let (b, s) = (&t.direct.bounds, &t.direct.simd);
    assert_eq!(
        (
            b.mesh_trees,
            b.section_trees,
            b.mesh_leaves,
            b.section_leaves,
            b.nodes
        ),
        (
            want.mesh_trees,
            want.section_trees,
            want.section_trees,
            want.primitives,
            want.nodes
        ),
        "mesh trees, section trees, mesh leaves, section leaves, nodes"
    );
    assert_eq!(
        (b.inside, b.rounding, b.outside),
        want.boxes,
        "triangles and quads against their leaf boxes"
    );
    assert_eq!(b.primitives, want.boxes.0 + want.boxes.1 + want.boxes.2);
    assert_eq!(b.inverted_boxes, want.inverted_boxes);
    assert_eq!(
        (
            s.trees,
            s.leafless,
            s.nodes,
            s.children_exact,
            s.children_looser,
            s.children_short
        ),
        (
            want.mesh_trees,
            want.leafless_simd_trees,
            want.simd_nodes,
            want.simd_children_exact,
            0,
            0
        ),
        "simd trees"
    );
    let leaves = want.simd_leaves;
    assert_eq!(
        (
            s.leaves_exact,
            s.leaves_looser,
            s.leaves_rounding,
            s.leaves_outside
        ),
        (leaves.0, leaves.1, leaves.2, leaves.3),
        "simd leaf boxes"
    );
    assert_eq!(s.leaves, leaves.0 + leaves.1 + leaves.2 + leaves.3 + leaves.4);
    // the readers' share: the meshes they return as meshes, and the ones they do not
    let unread_meshes = t.unread.get("hknpCompressedMeshShape").copied().unwrap_or(0);
    assert_eq!(
        t.wired.bounds.mesh_trees + unread_meshes + t.extern_only,
        b.mesh_trees,
        "meshes read, left unread by read_map, and made of extern shapes alone"
    );
    assert_eq!(t.wired.bounds.outside, 0);
    assert_eq!(t.wired.simd.leaves_outside, 0);
}

/// Sekiro: 4,497 tagfiles of the map `h`/`l` and object collision (the `f` sets hold no compressed mesh): the census's
/// 3,344 mesh trees, 273,228 sections and 21,847,369 primitives, 21,846,092 of them triangles or quads (the rest
/// dead or custom). Every mesh is returned as a mesh by the readers.
#[test]
fn sekiros_trees_decode_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    check(
        &t,
        &Expected {
            files: 4_497,
            mesh_trees: 3_344,
            section_trees: 273_228,
            primitives: 21_847_369,
            nodes: 43_964_622,
            boxes: (21_748_039, 98_053, 0),
            inverted_boxes: 13_490,
            leafless_simd_trees: 15,
            simd_nodes: 10_845_939,
            simd_leaves: (21_686_014, 0, 0, 0, 0),
            simd_children_exact: 10_839_251,
        },
    );
    assert_eq!(t.wired, t.direct, "every Sekiro mesh is returned as a mesh");
    Ok(())
}

/// Elden Ring: 23,280 tagfiles of the map `h`/`l` and asset collision (the `f` sets hold no compressed mesh): 22,899
/// mesh trees of 722,405 sections and 58,630,675 primitives, 58,628,330 of them triangles or quads.
#[test]
fn elden_rings_trees_decode_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    check(
        &t,
        &Expected {
            files: 23_280,
            mesh_trees: 22_899,
            section_trees: 722_405,
            primitives: 58_630_675,
            nodes: 117_960_859,
            boxes: (58_537_786, 90_544, 0),
            inverted_boxes: 18_577,
            leafless_simd_trees: 347,
            simd_nodes: 22_831_521,
            simd_leaves: (58_575_737, 657, 1_953, 0, 35),
            simd_children_exact: 22_785_723,
        },
    );
    Ok(())
}
