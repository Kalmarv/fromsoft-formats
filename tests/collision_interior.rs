//! The interior flags (`triangleIsInterior`) of the stock compressed meshes against the pass of `havok-formats`
//! (`interior_keys`), on every compressed mesh of both installs.
//!
//! For each mesh that has any flag set on a key of one of its triangles or quads, the primitives are read with their
//! vertex identities and decoded positions (`interior_primitives`), the pass is run, and the keys it flags are compared
//! with the keys the file flags: a key is the shape key of a triangle, or of either half of a quad. A mesh with no flag
//! set is not tested (the pass was not run when it was made: 362 of Sekiro's meshes and 10,698 of Elden Ring's
//! (5,777)).
//!
//! The pass reproduces every flag of every such mesh of Sekiro and of Elden Ring's base game: no key is wrong in 2,982
//! meshes (25,383,271 keys, 8,080,712 of them flagged) of Sekiro and in 17,122 meshes (65,846,233 keys, 25,136,262
//! flagged) of the base game. With Shadow of the Erdtree installed 27,046 of Elden Ring's 27,048 meshes (86,366,666
//! keys, 31,771,977 flagged) are exact and 4 keys of the other two are not, 2 in each of the expansion's `m28_00_00_00`
//! high and low collision meshes (shape 0 of `h28_00_00_00_000000` and `l28_00_00_00_000000`): why is not established.
//! One mesh of Sekiro (19 quads) and 57 of Elden Ring (22,620; (53, 22,196)) have quads stored as flat
//! (`primitiveStoresIsFlatConvex` is 255), which the pass takes as polygons; no other mesh has one.
//!
//! The counts are those of the census over the same files with the scratch probe's copy of the pass (probe mode
//! `intsim`), not copied from a run of this test: the Elden Ring counts in parentheses are the census's of the base
//! game; with Shadow of the Erdtree installed the counts asserted are this test's own (with the expansion's archives
//! left out it gives the census's). Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and
//! `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::BTreeSet;

use fromsoft_formats::Game;
use havok_formats::{interior_keys, interior_primitives};
use hkwalk::{Member, Result, Set, is_compressed_mesh, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    meshes_with_flags: usize,
    meshes_without_flags: usize,
    meshes_exact: usize,
    keys: usize,
    flagged: usize,
    wrong: usize,
    /// Each mesh with a wrong key: its binder, member and shape (in the order of `shapes`), and its wrong keys.
    wrong_meshes: Vec<(String, usize)>,
    flat_quad_meshes: usize,
    flat_quads: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.meshes_with_flags += o.meshes_with_flags;
        self.meshes_without_flags += o.meshes_without_flags;
        self.meshes_exact += o.meshes_exact;
        self.keys += o.keys;
        self.flagged += o.flagged;
        self.wrong += o.wrong;
        self.wrong_meshes.extend(o.wrong_meshes);
        self.flat_quad_meshes += o.flat_quad_meshes;
        self.flat_quads += o.flat_quads;
    }
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    for (index, shape) in shapes(m.tag)?.into_iter().enumerate() {
        if !is_compressed_mesh(shape.type_name()) {
            continue;
        }
        let data = shape.field("data")?.dereference()?;
        let prims = interior_primitives(data)?;
        let bits = shape.field("triangleIsInterior")?.field("storage")?;
        let num_bits = bits.field("numBits")?.signed()? as usize;
        let words = bits.field("words")?.array()?;
        let stock = |key: u32| -> Result<bool> {
            let key = key as usize;
            Ok(key < num_bits && (words.get(key / 32)?.unsigned()? as u32 >> (key % 32)) & 1 == 1)
        };
        let mut keys: BTreeSet<u32> = BTreeSet::new();
        for p in &prims {
            keys.insert(p.key);
            if p.quad {
                keys.insert(p.key | 1);
            }
        }
        let mut flagged = 0usize;
        for &key in &keys {
            flagged += usize::from(stock(key)?);
        }
        if flagged == 0 {
            t.meshes_without_flags += 1;
            continue;
        }
        t.meshes_with_flags += 1;
        t.flat_quad_meshes += usize::from(prims.iter().any(|p| p.flat));
        t.flat_quads += prims.iter().filter(|p| p.flat).count();
        let mine: BTreeSet<u32> = interior_keys(&prims).into_iter().collect();
        let mut wrong = 0usize;
        for &key in &keys {
            wrong += usize::from(stock(key)? != mine.contains(&key));
        }
        t.keys += keys.len();
        t.flagged += flagged;
        t.wrong += wrong;
        t.meshes_exact += usize::from(wrong == 0);
        if wrong != 0 {
            t.wrong_meshes
                .push((format!("{} {} shape {index}", m.binder, m.name), wrong));
        }
    }
    Ok(())
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let Some(install) = hkwalk::open(game, &[Set::Mesh])? else {
        return Ok(None);
    };
    let mut t = install.run(visit, Totals::merge)?;
    t.wrong_meshes.sort();
    eprintln!(
        "{} collision interior flags: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn sekiros_interior_flags_are_what_the_pass_computes() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.meshes_with_flags, t.meshes_without_flags), (2_982, 362));
    assert_eq!((t.keys, t.flagged), (25_383_271, 8_080_712));
    assert_eq!((t.wrong, t.meshes_exact), (0, 2_982));
    assert_eq!((t.flat_quad_meshes, t.flat_quads), (1, 19));
    Ok(())
}

#[test]
fn elden_rings_interior_flags_are_what_the_pass_computes() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.meshes_with_flags, t.meshes_without_flags), (27_048, 10_698));
    assert_eq!((t.keys, t.flagged), (86_366_666, 31_771_977));
    assert_eq!((t.wrong, t.meshes_exact), (4, 27_046));
    let wrong: Vec<_> = t.wrong_meshes.iter().map(|(m, n)| (m.as_str(), *n)).collect();
    assert_eq!(
        wrong,
        [
            (
                r"/map/m28/m28_00_00_00/h28_00_00_00.hkxbhd m28_00_00_00\h28_00_00_00_000000.hkx.dcx shape 0",
                2
            ),
            (
                r"/map/m28/m28_00_00_00/l28_00_00_00.hkxbhd m28_00_00_00\l28_00_00_00_000000.hkx.dcx shape 0",
                2
            ),
        ]
    );
    assert_eq!((t.flat_quad_meshes, t.flat_quads), (57, 22_620));
    Ok(())
}
