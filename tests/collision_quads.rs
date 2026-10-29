//! The quads of the stock compressed meshes against the pairing of `havok-formats` (`pair_quads`), on every compressed mesh
//! of both installs that has the custom parameter (`pParam`) with the source triangles.
//!
//! The source triangles of a mesh are the rows of its `pParam.triangleDataArray`; each row's entry of
//! `triangleIndexToShapeKey` gives the key the triangle got, so the stock quads are the pairs of rows whose keys differ in
//! the half bit only. The pairing is run on the rows of each section (the stock tool pairs within a section) with the
//! row's `primitiveDataIndex` as the data two triangles must share, and its pairs are compared with the stock ones.
//!
//! It is not the stock rule in every detail: it finds 99.4% of Sekiro's stock quads (3,485,775 of 3,507,426) and 99.2% of
//! Elden Ring's (7,505,319 of 7,568,285), and 99.5% and 99.4% of what it finds are stock quads. Where it picks a stock pair
//! it picks the stock first half first, every time. The Sekiro object meshes come out lowest (28,801 of 32,485 stock quads),
//! the misses being triangles on an edge that eight or more triangles share.
//!
//! The counts are those of the census over the same files with the scratch probe's copy of the pairing (probe mode
//! `quadcrate`), not copied from a run of this test. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::{BTreeMap, BTreeSet};

use fromsoft_formats::Game;
use havok_formats::{QuadCandidate, compressed_mesh, pair_quads};
use hkwalk::{Member, Result, Set, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    meshes: usize,
    meshes_exact: usize,
    stock: usize,
    found: usize,
    both: usize,
    both_first_half_first: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.meshes += o.meshes;
        self.meshes_exact += o.meshes_exact;
        self.stock += o.stock;
        self.found += o.found;
        self.both += o.both;
        self.both_first_half_first += o.both_first_half_first;
    }
}

struct Row {
    key: u32,
    data: u64,
    vertices: [[f32; 3]; 3],
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    for shape in shapes(m.tag)? {
        if shape.type_name() != "fsnpCustomParamCompressedMeshShape" {
            continue;
        }
        let param = shape.field("pParam")?.dereference()?;
        let mesh = compressed_mesh(shape.field("data")?.dereference()?)?;
        let key_bits = shape.field("numShapeKeyBits")?.unsigned()? as u32;
        let rows = param.field("triangleDataArray")?.array()?;
        let map = shape.field("triangleIndexToShapeKey")?.array()?;
        let by_key: BTreeMap<u32, [[f32; 3]; 3]> =
            mesh.triangles.iter().map(|t| (t.key, t.vertices)).collect();
        // the rows that have a triangle, with the key the file gives them
        let mut source: Vec<Row> = Vec::with_capacity(rows.len());
        for i in 0..rows.len() {
            let data = rows.get(i)?.field("primitiveDataIndex")?.unsigned()?;
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
            if let Some(vertices) = by_key.get(&key) {
                source.push(Row {
                    key,
                    data,
                    vertices: *vertices,
                });
            }
        }
        let mut sections: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        let mut by_primitive: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, row) in source.iter().enumerate() {
            sections.entry(row.key >> 8).or_default().push(i);
            by_primitive.entry(row.key >> 1).or_default().push(i);
        }
        let mut stock: BTreeSet<(usize, usize)> = BTreeSet::new();
        for rows in by_primitive.values() {
            if let &[a, b] = rows.as_slice() {
                stock.insert((a.min(b), a.max(b)));
            }
        }
        let mut found: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut first_half_first = 0usize;
        for rows in sections.values() {
            let offered: Vec<QuadCandidate> = rows
                .iter()
                .map(|&i| QuadCandidate {
                    vertices: source[i].vertices,
                    data: source[i].data,
                })
                .collect();
            for [first, second] in pair_quads(&offered) {
                let (a, b) = (rows[first], rows[second]);
                let pair = (a.min(b), a.max(b));
                if stock.contains(&pair) && source[a].key & 1 == 0 {
                    first_half_first += 1;
                }
                found.insert(pair);
            }
        }
        t.meshes += 1;
        t.meshes_exact += usize::from(stock == found);
        t.stock += stock.len();
        t.found += found.len();
        t.both += stock.intersection(&found).count();
        t.both_first_half_first += first_half_first;
    }
    Ok(())
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let Some(install) = hkwalk::open(game, &[Set::Mesh])? else {
        return Ok(None);
    };
    let t = install.run(visit, Totals::merge)?;
    eprintln!(
        "{} collision quads: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn sekiros_quads_are_what_the_pairing_finds() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.meshes, t.meshes_exact), (3_333, 1_710));
    assert_eq!((t.stock, t.found, t.both), (3_507_426, 3_502_109, 3_485_775));
    assert_eq!(t.both_first_half_first, t.both);
    Ok(())
}

#[test]
fn elden_rings_quads_are_what_the_pairing_finds() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.meshes, t.meshes_exact), (22_778, 18_114));
    assert_eq!((t.stock, t.found, t.both), (7_568_285, 7_549_658, 7_505_319));
    assert_eq!(t.both_first_half_first, t.both);
    Ok(())
}
