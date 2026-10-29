//! The sections of the stock compressed meshes against the primitive tree of `havok-formats` (`StockTree`) and its cut, on
//! every compressed mesh of both installs that has the custom parameter (`pParam`) with the source triangles.
//!
//! For each mesh the source triangles are the rows of `pParam.triangleDataArray` that the mesh holds (a row's entry of
//! `triangleIndexToShapeKey` gives its key, and the key's section is the stock section). The rows are paired into quads
//! (`pair_quads`), the quads are the first leaves (in the order the pairing takes them) and the other rows the next (in
//! row order), each leaf's box is its vertices' box grown by the shape's convex radius, a `StockTree` is built over them and
//! cut into sections of at most 127 leaves and 255 distinct vertices, and the sections are compared with the stock ones
//! (as sets of rows, and in order).
//!
//! The tree and the cut are not the stock tool in every detail, and the vertices here are the ones the files keep after
//! quantising them, where the stock tool worked from the ones before: a different rounding of a box can send a leaf to
//! another bin or another child. Of the meshes with two sections or more, 592 of 2,613 in Sekiro and 4,577 of 14,218 in
//! Elden Ring come out with exactly their stock sections in their stock order, and the mesh tree above the sections is the
//! stock mesh tree in all of them; 202,885 of the 264,453 stock sections of those meshes in Sekiro and 441,322 of 713,169 in
//! Elden Ring are among the sections the cut finds.
//!
//! The counts are those of the census over the same files with the scratch probe's copy of the modules (probe mode
//! `treecrate`), not copied from a run of this test. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::{BTreeMap, BTreeSet};

use fromsoft_formats::Game;
use havok_formats::{Box3, QuadCandidate, StockTree, compressed_mesh, mesh_trees, pair_quads, shape_of};
use hkwalk::{Member, Result, Set, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    meshes: usize,
    multi_section_meshes: usize,
    exact_meshes: usize,
    exact_mesh_trees: usize,
    stock_sections: usize,
    sections_found: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.meshes += o.meshes;
        self.multi_section_meshes += o.multi_section_meshes;
        self.exact_meshes += o.exact_meshes;
        self.exact_mesh_trees += o.exact_mesh_trees;
        self.stock_sections += o.stock_sections;
        self.sections_found += o.sections_found;
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
        let data = shape.field("data")?.dereference()?;
        let mesh = compressed_mesh(data)?;
        let radius = shape.field("convexRadius")?.float()?;
        let key_bits = shape.field("numShapeKeyBits")?.unsigned()? as u32;
        let rows = param.field("triangleDataArray")?.array()?;
        let map = shape.field("triangleIndexToShapeKey")?.array()?;
        let by_key: BTreeMap<u32, [[f32; 3]; 3]> =
            mesh.triangles.iter().map(|t| (t.key, t.vertices)).collect();
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
        if source.is_empty() {
            continue;
        }
        let mut stock: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
        for (i, row) in source.iter().enumerate() {
            stock.entry(row.key >> 8).or_default().insert(i);
        }
        // the leaves: quads in the order the pairing takes them, then the other rows
        let offered: Vec<QuadCandidate> = source
            .iter()
            .map(|row| QuadCandidate {
                vertices: row.vertices,
                data: row.data,
            })
            .collect();
        let mut taken = vec![false; source.len()];
        let mut leaves: Vec<Vec<usize>> = Vec::new();
        for [first, second] in pair_quads(&offered) {
            taken[first] = true;
            taken[second] = true;
            leaves.push(vec![first, second]);
        }
        for (i, &used) in taken.iter().enumerate() {
            if !used {
                leaves.push(vec![i]);
            }
        }
        let boxes: Vec<Box3> = leaves
            .iter()
            .map(|rows| {
                let mut low = [f32::MAX; 3];
                let mut high = [f32::MIN; 3];
                for &r in rows {
                    for p in &source[r].vertices {
                        for axis in 0..3 {
                            low[axis] = low[axis].min(p[axis]);
                            high[axis] = high[axis].max(p[axis]);
                        }
                    }
                }
                [low.map(|x| x - radius), high.map(|x| x + radius)]
            })
            .collect();
        let tree = StockTree::build(&boxes);
        let items = tree.cut(127, &|under: &[usize]| {
            let mut seen: BTreeSet<[u32; 3]> = BTreeSet::new();
            for &leaf in under {
                for &r in &leaves[leaf] {
                    for p in &source[r].vertices {
                        seen.insert(p.map(f32::to_bits));
                    }
                }
            }
            seen.len() <= 255
        });
        let found: Vec<BTreeSet<usize>> = items
            .iter()
            .map(|&node| {
                tree.leaves_under(node)
                    .into_iter()
                    .flat_map(|leaf| leaves[leaf].clone())
                    .collect()
            })
            .collect();
        let stock_sections: Vec<&BTreeSet<usize>> = stock.values().collect();
        t.meshes += 1;
        if stock_sections.len() < 2 {
            continue;
        }
        let found_set: BTreeSet<&BTreeSet<usize>> = found.iter().collect();
        let in_order =
            stock_sections.len() == found.len() && stock_sections.iter().zip(&found).all(|(a, b)| *a == b);
        t.multi_section_meshes += 1;
        t.stock_sections += stock_sections.len();
        t.sections_found += stock_sections.iter().filter(|s| found_set.contains(**s)).count();
        if in_order {
            t.exact_meshes += 1;
            let trees = mesh_trees(data)?;
            t.exact_mesh_trees += usize::from(tree.shape_above(&items) == shape_of(&trees.mesh));
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
        "{} collision sections: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn sekiros_sections_are_mostly_what_the_cut_finds() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.meshes, t.multi_section_meshes), (3_213, 2_613));
    assert_eq!((t.exact_meshes, t.exact_mesh_trees), (592, 592));
    assert_eq!((t.stock_sections, t.sections_found), (264_453, 202_885));
    Ok(())
}

#[test]
fn elden_rings_sections_are_mostly_what_the_cut_finds() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.meshes, t.multi_section_meshes), (22_775, 14_218));
    assert_eq!((t.exact_meshes, t.exact_mesh_trees), (4_577, 4_577));
    assert_eq!((t.stock_sections, t.sections_found), (713_169, 441_322));
    Ok(())
}
