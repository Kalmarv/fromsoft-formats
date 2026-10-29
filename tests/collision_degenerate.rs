//! The degenerate-triangle test of `havok-formats` (`is_degenerate_triangle`, the stock tool's own) on the source rows of
//! every compressed mesh of both installs that has the custom parameter (`pParam`).
//!
//! A row whose entry of `triangleIndexToShapeKey` names a triangle of the mesh is a kept triangle; the others are left out.
//! No kept triangle is degenerate by the test (24,704,951 in Sekiro, 66,147,210 in Elden Ring). Of the left-out rows whose
//! three source vertices a kept row also uses (the position taken from the first such row), the test calls 13,345 of 87,100
//! degenerate in Sekiro and 267,763 of 549,405 in Elden Ring; the others use a vertex no kept row uses (27,874 and 538,167).
//!
//! The counts are those of the census over the same files with the scratch probe's copy of the test (probe mode
//! `degencrate`), not copied from a run of this test. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::BTreeMap;

use fromsoft_formats::Game;
use havok_formats::{DEFAULT_DEGENERATE_THRESHOLD, compressed_mesh, is_degenerate_triangle};
use hkwalk::{Member, Result, Set, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    kept: usize,
    kept_degenerate: usize,
    left_out_known: usize,
    left_out_degenerate: usize,
    left_out_unknown: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.kept += o.kept;
        self.kept_degenerate += o.kept_degenerate;
        self.left_out_known += o.left_out_known;
        self.left_out_degenerate += o.left_out_degenerate;
        self.left_out_unknown += o.left_out_unknown;
    }
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    for shape in shapes(m.tag)? {
        if shape.type_name() != "fsnpCustomParamCompressedMeshShape" {
            continue;
        }
        let mesh = compressed_mesh(shape.field("data")?.dereference()?)?;
        let key_bits = shape.field("numShapeKeyBits")?.unsigned()? as u32;
        let rows = shape
            .field("pParam")?
            .dereference()?
            .field("triangleDataArray")?
            .array()?;
        let map = shape.field("triangleIndexToShapeKey")?.array()?;
        let by_key: BTreeMap<u32, [[f32; 3]; 3]> =
            mesh.triangles.iter().map(|t| (t.key, t.vertices)).collect();
        let mut found: Vec<([u64; 3], Option<[[f32; 3]; 3]>)> = Vec::with_capacity(rows.len());
        let mut position: BTreeMap<u64, [f32; 3]> = BTreeMap::new();
        for i in 0..rows.len() {
            let row = rows.get(i)?;
            let indices = [
                row.field("vertexIndexA")?.unsigned()?,
                row.field("vertexIndexB")?.unsigned()?,
                row.field("vertexIndexC")?.unsigned()?,
            ];
            let word = if i < map.len() {
                map.get(i)?.unsigned()? as u32
            } else {
                u32::MAX
            };
            let key = if (1..=32).contains(&key_bits) {
                word >> (32 - key_bits)
            } else {
                0
            };
            let vertices = by_key.get(&key).copied();
            if let Some(v) = vertices {
                for (k, index) in indices.into_iter().enumerate() {
                    position.entry(index).or_insert(v[k]);
                }
            }
            found.push((indices, vertices));
        }
        for (indices, vertices) in found {
            let threshold = DEFAULT_DEGENERATE_THRESHOLD;
            match vertices {
                Some(v) => {
                    t.kept += 1;
                    t.kept_degenerate += usize::from(is_degenerate_triangle(v[0], v[1], v[2], threshold));
                }
                None => match indices.map(|i| position.get(&i).copied()) {
                    [Some(a), Some(b), Some(c)] => {
                        t.left_out_known += 1;
                        t.left_out_degenerate += usize::from(is_degenerate_triangle(a, b, c, threshold));
                    }
                    _ => t.left_out_unknown += 1,
                },
            }
        }
    }
    Ok(())
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let Some(install) = hkwalk::open(game, &[Set::Mesh])? else {
        return Ok(None);
    };
    let t = install.run(visit, Totals::merge)?;
    eprintln!(
        "{} collision degenerate rows: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn no_kept_triangle_of_sekiro_is_degenerate() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.kept, t.kept_degenerate), (24_704_951, 0));
    assert_eq!((t.left_out_known, t.left_out_degenerate), (87_100, 13_345));
    assert_eq!(t.left_out_unknown, 27_874);
    Ok(())
}

#[test]
fn no_kept_triangle_of_elden_ring_is_degenerate() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.kept, t.kept_degenerate), (66_147_210, 0));
    assert_eq!((t.left_out_known, t.left_out_degenerate), (549_405, 267_763));
    assert_eq!(t.left_out_unknown, 538_167);
    Ok(())
}
