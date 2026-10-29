//! The topology of the stock compressed-mesh trees against the tree builder of `havok-formats` (`sah_shape`), on every
//! compressed mesh of both installs.
//!
//! Topology. Leaf boxes are the primitives' decoded vertices grown by the shape's convex radius; a section is used when
//! every primitive has a box. (1) Every maximal subtree of 3 to 16 leaves of a section tree is rebuilt from its leaves'
//! boxes by `sah_shape` (which merges up to `MERGE_LEAVES` = 16 leaves from the bottom, always the two nodes whose union has
//! the least surface area) and its inner nodes compared as sets of leaves; the subtrees of every size from 3 to 17 leaves
//! are also compared with a pure bottom-up merge (`sah_shape_with(.., usize::MAX)`): the rate falls from two thirds at 16
//! leaves to under a tenth at 17. (2) The first child of a stock node has the larger surface in 20,184,136 nodes of Sekiro
//! and 53,597,280 of Elden Ring, the smaller in 417,276 and 1,699,537. (3) A section is a subtree cut at 127
//! primitives: of the pairs of sibling sections of the mesh trees all but 6 (Sekiro) and 82 (Elden Ring) hold more than
//! 127 together. (4) The total surface of the inner nodes of `sah_shape` trees against the stock trees and
//! `balanced_shape` trees.
//!
//! The counts are those of the census over the same files (probe mode `topogate`, which compiles a copy of the
//! builder's text): they are not copied from a run of this test. Needs `installs.{sekiro,elden}`,
//! `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is
//! skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::{BTreeMap, BTreeSet};

use fromsoft_formats::Game;
use havok_formats::{
    Aabb, BvNodeKind, MERGE_LEAVES, Shape, balanced_shape, compressed_mesh, mesh_sections, mesh_trees,
    sah_shape, sah_shape_with,
};
use hkwalk::{Member, Result, Set, is_compressed_mesh, shapes};

#[derive(Debug, Default, PartialEq)]
struct Totals {
    sections_used: usize,
    sections_skipped: usize,
    // (1)
    subtrees: usize,
    subtrees_exact: usize,
    clusters: usize,
    clusters_reproduced: usize,
    /// by leaf count 3..=17: (subtrees, subtrees a pure bottom-up merge gives)
    by_size: BTreeMap<usize, (usize, usize)>,
    // (2)
    first_larger: usize,
    first_equal: usize,
    first_smaller: usize,
    equal_two_leaves: usize,
    equal_other_first_more: usize,
    equal_other_first_as_many: usize,
    equal_other_first_fewer: usize,
    // (3)
    sibling_pairs: usize,
    sibling_pairs_over_127: usize,
    // (4)
    costed: usize,
    sah_no_worse: usize,
    stock_surface: f64,
    sah_surface: f64,
    balanced_surface: f64,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        self.sections_used += o.sections_used;
        self.sections_skipped += o.sections_skipped;
        self.subtrees += o.subtrees;
        self.subtrees_exact += o.subtrees_exact;
        self.clusters += o.clusters;
        self.clusters_reproduced += o.clusters_reproduced;
        for (n, (a, b)) in o.by_size {
            let slot = self.by_size.entry(n).or_default();
            slot.0 += a;
            slot.1 += b;
        }
        self.first_larger += o.first_larger;
        self.first_equal += o.first_equal;
        self.first_smaller += o.first_smaller;
        self.equal_two_leaves += o.equal_two_leaves;
        self.equal_other_first_more += o.equal_other_first_more;
        self.equal_other_first_as_many += o.equal_other_first_as_many;
        self.equal_other_first_fewer += o.equal_other_first_fewer;
        self.sibling_pairs += o.sibling_pairs;
        self.sibling_pairs_over_127 += o.sibling_pairs_over_127;
        self.costed += o.costed;
        self.sah_no_worse += o.sah_no_worse;
        self.stock_surface += o.stock_surface;
        self.sah_surface += o.sah_surface;
        self.balanced_surface += o.balanced_surface;
    }
}

/// Half the surface area of a box.
fn surface(b: &Aabb) -> f64 {
    let e = |a: usize| (b[1][a] - b[0][a]).max(0.0);
    e(0) * e(1) + e(1) * e(2) + e(0) * e(2)
}

fn union(a: &Aabb, b: &Aabb) -> Aabb {
    let mut out = *a;
    for axis in 0..3 {
        out[0][axis] = out[0][axis].min(b[0][axis]);
        out[1][axis] = out[1][axis].max(b[1][axis]);
    }
    out
}

/// For each node of a pre-order tree given as (is leaf, payload or second child): the box of the leaves below it (by
/// `leaf_box`) and the payloads below it.
fn below(nodes: &[(bool, usize)], leaf_box: &dyn Fn(usize) -> Aabb) -> Vec<(Aabb, Vec<usize>)> {
    let mut out: Vec<Option<(Aabb, Vec<usize>)>> = vec![None; nodes.len()];
    for i in (0..nodes.len()).rev() {
        let (leaf, v) = nodes[i];
        out[i] = Some(if leaf {
            (leaf_box(v), vec![v])
        } else {
            let a = out[i + 1].clone().unwrap();
            let b = out[v].clone().unwrap();
            let mut leaves = a.1;
            leaves.extend(b.1);
            (union(&a.0, &b.0), leaves)
        });
    }
    out.into_iter().map(Option::unwrap).collect()
}

/// The inner nodes of a shape as sets of the box indices below them.
fn clusters(shape: &[Shape], order: &[usize]) -> BTreeSet<BTreeSet<usize>> {
    let mut sets: Vec<Option<BTreeSet<usize>>> = vec![None; shape.len()];
    let mut out = BTreeSet::new();
    for i in (0..shape.len()).rev() {
        let set = match shape[i] {
            Shape::Leaf { payload } => BTreeSet::from([order[payload as usize]]),
            Shape::Internal { second_child } => {
                let mut a = sets[i + 1].clone().unwrap();
                a.extend(sets[second_child].clone().unwrap());
                out.insert(a.clone());
                a
            }
        };
        sets[i] = Some(set);
    }
    out
}

/// The cost of a shape: the surfaces of its inner nodes added up.
fn cost(shape: &[Shape], order: &[usize], boxes: &[Aabb]) -> f64 {
    let nodes: Vec<(bool, usize)> = shape
        .iter()
        .map(|s| match *s {
            Shape::Leaf { payload } => (true, order[payload as usize]),
            Shape::Internal { second_child } => (false, second_child),
        })
        .collect();
    let b = below(&nodes, &|p| boxes[p]);
    nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.0)
        .map(|(i, _)| surface(&b[i].0))
        .sum()
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    for shape in shapes(m.tag)? {
        if !is_compressed_mesh(shape.type_name()) {
            continue;
        }
        let data = shape.field("data")?.dereference()?;
        let radius = f64::from(shape.field("convexRadius")?.float()?);
        let mesh = compressed_mesh(data)?;
        let trees = mesh_trees(data)?;
        let sections = mesh_sections(data)?;
        let mut prim_box: BTreeMap<(usize, usize), Aabb> = BTreeMap::new();
        for tri in &mesh.triangles {
            let key = ((tri.key >> 8) as usize, ((tri.key >> 1) & 127) as usize);
            for v in tri.vertices {
                let p = v.map(f64::from);
                let slot = prim_box.entry(key).or_insert([p, p]);
                for a in 0..3 {
                    slot[0][a] = slot[0][a].min(p[a]);
                    slot[1][a] = slot[1][a].max(p[a]);
                }
            }
        }
        // (3) sibling sections of the mesh tree
        let mut section_of_node: BTreeMap<usize, usize> = BTreeMap::new();
        for (s, &leaf) in trees.mesh_leaves.iter().enumerate() {
            section_of_node.insert(leaf, s);
        }
        for (i, n) in trees.mesh.nodes().iter().enumerate() {
            if let BvNodeKind::Internal { second_child } = n.kind
                && let (Some(&a), Some(&b)) =
                    (section_of_node.get(&(i + 1)), section_of_node.get(&second_child))
                {
                    t.sibling_pairs += 1;
                    t.sibling_pairs_over_127 +=
                        usize::from(sections[a].primitives + sections[b].primitives > 127);
                }
        }
        for (si, sec) in trees.sections.iter().enumerate() {
            if (0..sections[si].primitives).any(|p| !prim_box.contains_key(&(si, p))) {
                t.sections_skipped += 1;
                continue;
            }
            t.sections_used += 1;
            let leaf_box = |p: usize| -> Aabb {
                let b = prim_box[&(si, p)];
                [b[0].map(|x| x - radius), b[1].map(|x| x + radius)]
            };
            let nodes: Vec<(bool, usize)> = sec
                .tree
                .nodes()
                .iter()
                .map(|n| match n.kind {
                    BvNodeKind::Leaf { payload } => (true, payload as usize),
                    BvNodeKind::Internal { second_child } => (false, second_child),
                })
                .collect();
            let under = below(&nodes, &leaf_box);
            let mut parent = vec![usize::MAX; nodes.len()];
            for (i, &(leaf, v)) in nodes.iter().enumerate() {
                if !leaf {
                    parent[i + 1] = i;
                    parent[v] = i;
                }
            }
            // (2) the order of the children
            for (i, &(leaf, v)) in nodes.iter().enumerate() {
                if leaf {
                    continue;
                }
                let (sa, sb) = (surface(&under[i + 1].0), surface(&under[v].0));
                if sa > sb {
                    t.first_larger += 1;
                } else if sa < sb {
                    t.first_smaller += 1;
                } else {
                    t.first_equal += 1;
                    let (na, nb) = (under[i + 1].1.len(), under[v].1.len());
                    if na == 1 && nb == 1 {
                        t.equal_two_leaves += 1;
                    } else if na > nb {
                        t.equal_other_first_more += 1;
                    } else if na == nb {
                        t.equal_other_first_as_many += 1;
                    } else {
                        t.equal_other_first_fewer += 1;
                    }
                }
            }
            let stock_clusters = |root: usize| -> BTreeSet<BTreeSet<usize>> {
                let mut out = BTreeSet::new();
                let mut stack = vec![root];
                while let Some(i) = stack.pop() {
                    if !nodes[i].0 {
                        out.insert(under[i].1.iter().copied().collect::<BTreeSet<_>>());
                        stack.push(i + 1);
                        stack.push(nodes[i].1);
                    }
                }
                out
            };
            let rebuilt = |root: usize, merge_up_to: usize| -> Result<BTreeSet<BTreeSet<usize>>> {
                let mut items = under[root].1.clone();
                items.sort_unstable();
                let boxes: Vec<Aabb> = items.iter().map(|&p| leaf_box(p)).collect();
                let (shape, order) = sah_shape_with(&boxes, merge_up_to)?;
                Ok(clusters(&shape, &order)
                    .into_iter()
                    .map(|c| c.into_iter().map(|q| items[q]).collect())
                    .collect())
            };
            // (1) every subtree of 3 to 17 leaves against a pure bottom-up merge
            for (i, (_, leaves)) in under.iter().enumerate().take(nodes.len()) {
                let n = leaves.len();
                if (3..=17).contains(&n) {
                    let slot = t.by_size.entry(n).or_default();
                    slot.0 += 1;
                    slot.1 += usize::from(rebuilt(i, usize::MAX)? == stock_clusters(i));
                }
            }
            // (1) the maximal subtrees of 3 to 16 leaves against `sah_shape`
            for u in 0..nodes.len() {
                let n = under[u].1.len();
                if !(3..=MERGE_LEAVES).contains(&n) || (u != 0 && under[parent[u]].1.len() <= MERGE_LEAVES) {
                    continue;
                }
                let want = stock_clusters(u);
                let mine = rebuilt(u, MERGE_LEAVES)?;
                t.subtrees += 1;
                t.subtrees_exact += usize::from(mine == want);
                t.clusters += want.len();
                t.clusters_reproduced += want.intersection(&mine).count();
            }
            // (4) the cost of the trees
            let n_leaves = sections[si].primitives;
            if n_leaves >= 2 {
                let all: Vec<Aabb> = (0..n_leaves).map(leaf_box).collect();
                let stock: f64 = nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| !n.0)
                    .map(|(i, _)| surface(&under[i].0))
                    .sum();
                let (s1, o1) = sah_shape(&all)?;
                let (s2, o2) = balanced_shape(&all)?;
                let (c1, c2) = (cost(&s1, &o1, &all), cost(&s2, &o2, &all));
                t.costed += 1;
                t.sah_no_worse += usize::from(c1 <= stock * (1.0 + 1e-12));
                t.stock_surface += stock;
                t.sah_surface += c1;
                t.balanced_surface += c2;
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
        "{} collision topology: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

#[test]
fn sekiros_trees_follow_the_surface_area_rule_below_17_leaves() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.sections_used, t.sections_skipped), (272_551, 677));
    // (1)
    assert_eq!((t.subtrees, t.subtrees_exact), (2_043_084, 1_591_211));
    assert_eq!((t.clusters, t.clusters_reproduced), (19_692_840, 18_624_635));
    assert_eq!(t.by_size[&3], (3_193_680, 3_141_293));
    assert_eq!(t.by_size[&16], (136_232, 92_012));
    assert_eq!(t.by_size[&17], (119_228, 10_615));
    // (2)
    assert_eq!(
        (t.first_larger, t.first_equal, t.first_smaller),
        (20_184_136, 913_005, 417_276)
    );
    assert_eq!(
        (
            t.equal_two_leaves,
            t.equal_other_first_more,
            t.equal_other_first_as_many,
            t.equal_other_first_fewer
        ),
        (834_964, 37_033, 33_697, 7_311)
    );
    // (3)
    assert_eq!((t.sibling_pairs, t.sibling_pairs_over_127), (99_164, 99_158));
    // (4) the builder's trees cost less than the stock ones (0.924 of them), the median split more (1.343)
    assert_eq!((t.costed, t.sah_no_worse), (272_471, 237_143));
    assert!(t.sah_surface < 0.95 * t.stock_surface, "{t:?}");
    assert!(t.balanced_surface > 1.25 * t.stock_surface, "{t:?}");
    Ok(())
}

#[test]
fn elden_rings_trees_follow_the_surface_area_rule_below_17_leaves() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.sections_used, t.sections_skipped), (720_770, 1_635));
    assert_eq!((t.subtrees, t.subtrees_exact), (5_454_605, 4_101_023));
    assert_eq!((t.clusters, t.clusters_reproduced), (52_922_096, 49_443_727));
    assert_eq!(t.by_size[&3], (8_281_964, 8_103_833));
    assert_eq!(t.by_size[&16], (359_481, 237_979));
    assert_eq!(t.by_size[&17], (317_884, 27_467));
    assert_eq!(
        (t.first_larger, t.first_equal, t.first_smaller),
        (53_597_280, 2_465_256, 1_699_537)
    );
    assert_eq!(
        (
            t.equal_two_leaves,
            t.equal_other_first_more,
            t.equal_other_first_as_many,
            t.equal_other_first_fewer
        ),
        (2_279_779, 119_836, 44_000, 21_641)
    );
    assert_eq!((t.sibling_pairs, t.sibling_pairs_over_127), (274_943, 274_861));
    assert_eq!((t.costed, t.sah_no_worse), (719_647, 668_361));
    assert!(t.sah_surface < 0.95 * t.stock_surface, "{t:?}");
    assert!(t.balanced_surface > 1.25 * t.stock_surface, "{t:?}");
    Ok(())
}
