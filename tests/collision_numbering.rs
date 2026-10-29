//! The numbering of a section's primitives (`section_numbering` of `havok-formats`) against the section trees of every
//! compressed mesh of both installs.
//!
//! The leaves of a section tree, in file order, carry the index each primitive is stored at. The stock tool numbers the
//! primitives in the pre-order of the tree and then sorts them by data-run index with its quicksort; a section names one
//! run, so the result depends on the count alone. Every section tree of two leaves or more follows it: 273,148 in Sekiro
//! and 721,282 in Elden Ring (all the section trees there are, with the 80 and 1,123 that have a single leaf).
//!
//! The counts are those of the census over the same files with the scratch probe's copy of the rule (probe mode
//! `numperm`), not copied from a run of this test. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use fromsoft_formats::Game;
use havok_formats::{BvNodeKind, mesh_trees, section_numbering};
use hkwalk::{Member, Result, Set, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    followed: usize,
    not_followed: usize,
    single_leaf: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.followed += o.followed;
        self.not_followed += o.not_followed;
        self.single_leaf += o.single_leaf;
    }
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    for shape in shapes(m.tag)? {
        if !shape.type_name().contains("CompressedMesh") {
            continue;
        }
        let data = shape.field("data")?.dereference()?;
        for section in &mesh_trees(data)?.sections {
            let payloads: Vec<usize> = section
                .tree
                .nodes()
                .iter()
                .filter_map(|node| match node.kind {
                    BvNodeKind::Leaf { payload } => Some(payload as usize),
                    BvNodeKind::Internal { .. } => None,
                })
                .collect();
            if payloads.len() < 2 {
                t.single_leaf += 1;
                continue;
            }
            // the leaf of pre-order rank r carries the index p with ranks[p] == r
            let mut expected = vec![0usize; payloads.len()];
            for (p, &rank) in section_numbering(payloads.len()).iter().enumerate() {
                expected[rank] = p;
            }
            if expected == payloads {
                t.followed += 1;
            } else {
                t.not_followed += 1;
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
        "{} collision section numbering: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn every_section_tree_of_sekiro_numbers_its_primitives_the_stock_way() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.followed, t.not_followed, t.single_leaf), (273_148, 0, 80));
    Ok(())
}

#[test]
fn every_section_tree_of_elden_ring_numbers_its_primitives_the_stock_way() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.followed, t.not_followed, t.single_leaf), (721_282, 0, 1_123));
    Ok(())
}
