//! The point sets of the custom primitives of the installs' compressed meshes (`havok_formats::PointSet`): every mesh of
//! the map, object and asset collision is read, and each `0x52` record decodes to points that lie in the box of the
//! primitive's leaf.
//!
//! The counts are those of the census that found the record layout (its own decoder, in the collision checks of the
//! census over both installs): Sekiro has 27 records in 10 meshes (all objects, one section each), 304 points, the tag
//! words 0x3D4D (21), 0x3C9D (4), 0x3CC7 and 0x3C8B, at most 36 points a record; Elden Ring has 28 records in 28 meshes
//! (all assets), 336 points, the tag word 0x3C24, at most 12 points a record. The map `h` and `l` sets have none. The
//! extern shape references (Elden Ring: 8 in the `_h` assets and 254 in the `_l`) are not point sets.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::BTreeMap;

use fromsoft_formats::Game;
use havok_formats::{CustomKind, compressed_mesh, mesh_trees};
use hkwalk::{Member, Result, Set, is_compressed_mesh, shapes};

/// What the meshes of a game hold of point sets.
#[derive(Debug, Default, PartialEq)]
struct Tally {
    meshes: usize,
    meshes_with_points: usize,
    records: usize,
    points: usize,
    max_points: usize,
    tags: BTreeMap<u32, usize>,
    /// Points inside the box of their leaf, grown by the shape's convex radius, and the worst excess of any point.
    inside: usize,
    outside: usize,
    worst_excess: f64,
    /// Extern shape references (a custom primitive that is not a point set).
    externs: usize,
    /// Point sets whose points are not all distinct.
    repeated_points: usize,
}

impl Tally {
    fn merge(&mut self, o: Self) {
        self.meshes += o.meshes;
        self.meshes_with_points += o.meshes_with_points;
        self.records += o.records;
        self.points += o.points;
        self.max_points = self.max_points.max(o.max_points);
        for (tag, n) in o.tags {
            *self.tags.entry(tag).or_default() += n;
        }
        self.inside += o.inside;
        self.outside += o.outside;
        self.worst_excess = self.worst_excess.max(o.worst_excess);
        self.externs += o.externs;
        self.repeated_points += o.repeated_points;
    }
}

fn visit(t: &mut Tally, m: &Member<'_>) -> Result<()> {
    for shape in shapes(m.tag)? {
        if !is_compressed_mesh(shape.type_name()) {
            continue;
        }
        let data = shape.field("data")?.dereference()?;
        let mesh = compressed_mesh(data)?;
        t.meshes += 1;
        let points: Vec<_> = mesh.custom.iter().filter(|c| c.points.is_some()).collect();
        t.externs += mesh
            .custom
            .iter()
            .filter(|c| matches!(c.kind, CustomKind::Extern(_)))
            .count();
        // a primitive that carries points is a 0x52 record, and the other way round
        assert_eq!(
            points.len(),
            mesh.custom
                .iter()
                .filter(|c| c.kind == CustomKind::Unread)
                .count(),
            "{}",
            m.name
        );
        if points.is_empty() {
            continue;
        }
        t.meshes_with_points += 1;
        let radius = f64::from(shape.field("convexRadius")?.float()?);
        let trees = mesh_trees(data)?;
        for custom in points {
            let set = custom.points.as_ref().unwrap();
            t.records += 1;
            t.points += set.vertices.len();
            t.max_points = t.max_points.max(set.vertices.len());
            *t.tags.entry(set.tag).or_default() += 1;
            // the key is section << 8 | primitive << 1
            let section = (custom.key >> 8) as usize;
            let primitive = ((custom.key >> 1) & 0x7f) as usize;
            let leaf = trees.sections[section]
                .leaf(primitive)
                .ok_or("a point set without a leaf")?;
            let mut distinct = set.vertices.clone();
            distinct.sort_by(|a, b| a.partial_cmp(b).unwrap());
            distinct.dedup();
            if distinct.len() != set.vertices.len() {
                t.repeated_points += 1;
            }
            for v in &set.vertices {
                let excess = (0..3)
                    .map(|a| {
                        let x = f64::from(v[a]);
                        (leaf.aabb[0][a] - radius - x)
                            .max(x - leaf.aabb[1][a] - radius)
                            .max(0.0)
                    })
                    .fold(0.0, f64::max);
                if excess <= 1e-4 {
                    t.inside += 1;
                } else {
                    t.outside += 1;
                }
                t.worst_excess = t.worst_excess.max(excess);
            }
        }
    }
    Ok(())
}

fn tally(game: Game) -> Result<Option<Tally>> {
    let Some(install) = hkwalk::open(game, &[Set::Mesh])? else {
        return Ok(None);
    };
    let t = install.run(visit, Tally::merge)?;
    eprintln!(
        "{} point sets: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn sekiros_point_sets_decode_inside_their_leaf_boxes() -> Result<()> {
    let Some(t) = tally(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!(
        (t.meshes_with_points, t.records, t.points, t.max_points),
        (10, 27, 304, 36)
    );
    assert_eq!(
        t.tags,
        BTreeMap::from([(0x3c8b, 1), (0x3c9d, 4), (0x3cc7, 1), (0x3d4d, 21)])
    );
    assert_eq!((t.inside, t.outside), (304, 0), "worst excess {}", t.worst_excess);
    assert_eq!(t.externs, 0);
    Ok(())
}

#[test]
fn elden_rings_point_sets_decode_inside_their_leaf_boxes() -> Result<()> {
    let Some(t) = tally(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!(
        (t.meshes_with_points, t.records, t.points, t.max_points),
        (28, 28, 336, 12)
    );
    assert_eq!(t.tags, BTreeMap::from([(0x3c24, 28)]));
    assert_eq!((t.inside, t.outside), (336, 0), "worst excess {}", t.worst_excess);
    // the extern shape references are the other custom primitives: 8 in the `_h` assets, 254 in the `_l`
    assert_eq!(t.externs, 262);
    Ok(())
}
