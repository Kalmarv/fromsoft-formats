//! The collision writers on every stock collision file of both installs: each compressed mesh and extern mesh is rebuilt
//! from its decoded triangles with `havok_formats::{build_compressed_mesh, replace_compressed_mesh, replace_extern_mesh}`,
//! written into the file's own image, and read back.
//!
//! Three things are checked. (1) The nibble rule of the tree encoder, applied with the stock trees' own topology and each
//! section's stored domain, reproduces the stored nodes at the rates a census of the same files measured (below); the
//! stock builder's topologies are not reproduced, so (2) every mesh is rebuilt with the crate's own: the decoded geometry
//! comes back within the quantisation of the codecs, the written file reads back to exactly the arrays the builder says it
//! wrote, every triangle lies in its leaf's box, and the simd tree names every primitive with exact boxes. (3) Every extern
//! mesh is rewritten in the form its class stores and reads back, with the `numShapeKeyBits` the stock rule gives
//! (`bitlen(n)` for class 2, `bitlen(n) + 1` for class 3: equal to the stock values in all 277,544 shapes).
//!
//! The counts are those of the census over the same files (probe modes `cascade`, `skipcount`, `externbits`, written before
//! the code): they are not copied from a run of this code. Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`,
//! `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::{BTreeMap, HashMap};

use fromsoft_formats::Game;
use havok_formats::{
    BuiltMesh, BvNodeKind, BvTree, ExternSource, MeshSource, NodeWidth, Tagfile, Value,
    build_compressed_mesh, compressed_mesh, encode_tree, extern_mesh, mesh_sections, mesh_trees,
    replace_compressed_mesh, replace_extern_mesh, shape_of, simd_tree,
};
use hkwalk::{Member, Result, Set, is_compressed_mesh, shapes};

type Box3 = [[f64; 3]; 2];

/// How the stock trees' nodes compare with the tree encoder's, by kind: nodes the encoder reproduces, and all nodes.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct Rate {
    equal: usize,
    total: usize,
}

#[derive(Debug, Default, PartialEq)]
struct Totals {
    // (1) the nibble rule on the stock topology
    inner: Rate,
    leaf: Rate,
    root: Rate,
    sections_skipped: usize,
    // (2) the rebuilt compressed meshes
    rebuilt: usize,
    triangles: usize,
    skipped: BTreeMap<&'static str, usize>,
    dropped: usize,
    geometry_violations: usize,
    read_back_differences: usize,
    outside: usize,
    simd_inexact: usize,
    sections: usize,
    shared_vertices: usize,
    // (3) the extern meshes
    extern_aabb: usize,
    extern_simd: usize,
    extern_empty: usize,
    extern_differences: usize,
    extern_key_bits_off_rule: usize,
}

impl Totals {
    fn merge(&mut self, o: Self) {
        for (a, b) in [
            (&mut self.inner, o.inner),
            (&mut self.leaf, o.leaf),
            (&mut self.root, o.root),
        ] {
            a.equal += b.equal;
            a.total += b.total;
        }
        self.sections_skipped += o.sections_skipped;
        self.rebuilt += o.rebuilt;
        self.triangles += o.triangles;
        for (k, v) in o.skipped {
            *self.skipped.entry(k).or_default() += v;
        }
        self.dropped += o.dropped;
        self.geometry_violations += o.geometry_violations;
        self.read_back_differences += o.read_back_differences;
        self.outside += o.outside;
        self.simd_inexact += o.simd_inexact;
        self.sections += o.sections;
        self.shared_vertices += o.shared_vertices;
        self.extern_aabb += o.extern_aabb;
        self.extern_simd += o.extern_simd;
        self.extern_empty += o.extern_empty;
        self.extern_differences += o.extern_differences;
        self.extern_key_bits_off_rule += o.extern_key_bits_off_rule;
    }
}

fn bitlen(x: u64) -> u32 {
    64 - x.leading_zeros()
}

/// (1) The tree encoder on every section tree of a mesh, with the stock shape and the section's stored domain.
fn nibble_rule(
    t: &mut Totals,
    data: Value<'_, '_>,
    radius: f64,
    mesh: &havok_formats::CompressedMesh,
) -> Result<()> {
    let trees = mesh_trees(data)?;
    let sections = mesh_sections(data)?;
    let mut prim_box: BTreeMap<(usize, usize), Box3> = BTreeMap::new();
    for tri in &mesh.triangles {
        let key = (tri.key >> 8) as usize;
        let prim = ((tri.key >> 1) & 127) as usize;
        for v in tri.vertices {
            let p = v.map(f64::from);
            let slot = prim_box.entry((key, prim)).or_insert([p, p]);
            for a in 0..3 {
                slot[0][a] = slot[0][a].min(p[a]);
                slot[1][a] = slot[1][a].max(p[a]);
            }
        }
    }
    for (si, section) in trees.sections.iter().enumerate() {
        // every primitive of the section needs a box (a dead or custom primitive has none)
        if (0..sections[si].primitives).any(|p| !prim_box.contains_key(&(si, p))) {
            t.sections_skipped += 1;
            continue;
        }
        let leaf_box = |p: u32| -> Option<Box3> {
            let b = prim_box.get(&(si, p as usize))?;
            Some([b[0].map(|x| x - radius), b[1].map(|x| x + radius)])
        };
        let shape = shape_of(&section.tree);
        let encoded = encode_tree(NodeWidth::Four, sections[si].domain, &shape, &leaf_box)?;
        for (i, node) in section.tree.nodes().iter().enumerate() {
            let equal = encoded.bytes[i * 4..i * 4 + 3] == node.axes;
            let rate = match node.kind {
                BvNodeKind::Leaf { .. } => &mut t.leaf,
                BvNodeKind::Internal { .. } if i == 0 => &mut t.root,
                BvNodeKind::Internal { .. } => &mut t.inner,
            };
            rate.total += 1;
            rate.equal += usize::from(equal);
        }
    }
    Ok(())
}

/// The error the codecs allow a vertex of the rebuilt triangle with key `key`, per axis: half a code step of the coarser of
/// the section's packed codec and the shared-vertex grid, and the rounding of the `f32` decode (`code * scale + minimum`
/// is three roundings at the magnitude of the box's coordinates, up to the units in the last place of the largest of them;
/// a mesh whose box reaches 300 m has 3.6e-5 m of that, a third of a step of its shared grid). The decode is the game's, and
/// the reader replicates it, so the rounding belongs to what a rebuilt vertex reads back as.
fn allowed(built: &BuiltMesh, key: u32, axis: usize) -> f64 {
    let section = &built.sections[(key >> 8) as usize];
    let packed = f64::from(section.codec[3 + axis]);
    let ranges = [2_097_151.0, 2_097_151.0, 4_194_303.0];
    let shared = (f64::from(built.domain[1][axis]) - f64::from(built.domain[0][axis])) / ranges[axis];
    let packed_far = f64::from(section.codec[axis]) + packed * [2047.0, 2047.0, 1023.0][axis];
    let magnitude = [
        f64::from(built.domain[0][axis]),
        f64::from(built.domain[1][axis]),
        f64::from(section.codec[axis]),
        packed_far,
    ]
    .into_iter()
    .fold(0.0, |m: f64, x| m.max(x.abs()));
    0.5001 * packed.max(shared) + 2.0 * f64::from(f32::EPSILON) * magnitude
}

/// (2) Rebuild a mesh from its decoded triangles; what the rebuild decodes to, or `None` for a mesh that is not rebuilt.
fn rebuild(
    t: &mut Totals,
    shape: Value<'_, '_>,
    mesh: &havok_formats::CompressedMesh,
) -> Result<Option<BuiltMesh>> {
    if mesh.triangles.is_empty() || !mesh.custom.is_empty() {
        let reason = match (mesh.triangles.is_empty(), mesh.custom.is_empty()) {
            (true, true) => "no primitives",
            (true, false) => "custom primitives only",
            _ => "custom primitives and triangles",
        };
        *t.skipped.entry(reason).or_default() += 1;
        return Ok(None);
    }
    // the decoded triangles on shared vertices (by bit pattern)
    let mut index: HashMap<[u32; 3], u32> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut triangles: Vec<[u32; 3]> = Vec::new();
    for tri in &mesh.triangles {
        let mut corners = [0u32; 3];
        for (c, v) in corners.iter_mut().zip(tri.vertices) {
            *c = *index.entry(v.map(f32::to_bits)).or_insert_with(|| {
                vertices.push(v);
                u32::try_from(vertices.len() - 1).unwrap()
            });
        }
        triangles.push(corners);
    }
    let radius = shape.field("convexRadius")?.float()?;
    let built = build_compressed_mesh(&MeshSource {
        vertices: &vertices,
        triangles: &triangles,
        convex_radius: radius,
    })?;
    t.dropped += built.triangle_keys.iter().filter(|k| k.is_none()).count();
    // the decoded geometry is the input to the quantisation of the codecs
    let decoded = built.triangles()?;
    let by_key: BTreeMap<u32, [[f32; 3]; 3]> = decoded.iter().map(|d| (d.key, d.vertices)).collect();
    for (i, source) in mesh.triangles.iter().enumerate() {
        let Some(key) = built.triangle_keys[i] else {
            continue;
        };
        let got = by_key[&key];
        let ok = (0..3).all(|c| {
            (0..3).all(|axis| {
                let want = f64::from(source.vertices[c][axis]);
                (f64::from(got[c][axis]) - want).abs()
                    <= allowed(&built, key, axis) + 4e-6 * (want.abs() + 1.0)
            })
        });
        t.geometry_violations += usize::from(!ok);
    }
    t.rebuilt += 1;
    t.triangles += mesh.triangles.len();
    t.sections += built.sections.len();
    t.shared_vertices += built.shared_vertices.len();
    Ok(Some(built))
}

/// (3) An extern mesh rewritten in the form its class stores.
fn rewrite_extern(
    t: &mut Totals,
    image: &mut havok_formats::TagfileImage,
    tag: &Tagfile<'_>,
    shape: Value<'_, '_>,
) -> Result<Option<usize>> {
    let mesh = extern_mesh(shape)?;
    let bv = shape.field("boundingVolumeData")?.dereference()?;
    let aabb = bv.has_field("aabbTree");
    if mesh.triangles.is_empty() {
        t.extern_empty += 1;
        return Ok(None);
    }
    let bits = shape.field("numShapeKeyBits")?.unsigned()?;
    let rule = u64::from(bitlen(mesh.triangles.len() as u64)) + u64::from(!aabb);
    t.extern_key_bits_off_rule += usize::from(bits != rule);
    if aabb {
        t.extern_aabb += 1;
    } else {
        t.extern_simd += 1;
    }
    let (item, _) = tag.item_at(shape.offset).unwrap();
    replace_extern_mesh(
        image,
        item,
        &ExternSource {
            vertices: &mesh.vertices,
            triangles: &mesh.triangles,
            materials: Some(&mesh.materials),
        },
    )?;
    Ok(Some(item))
}

/// The tree and simd checks of a rewritten extern mesh; the number of differences found.
fn check_extern(
    shape: Value<'_, '_>,
    tag: &Tagfile<'_>,
    want: &havok_formats::ExternMesh,
    bits: u64,
) -> Result<usize> {
    let mut differences = 0;
    let got = extern_mesh(shape)?;
    differences += usize::from(&got != want);
    differences += usize::from(shape.field("numShapeKeyBits")?.unsigned()? != bits);
    let bv = shape.field("boundingVolumeData")?.dereference()?;
    let simd = simd_tree(bv)?;
    if bv.has_field("aabbTree") {
        differences += usize::from(simd.nodes.len() != 2);
        let tree = bv.field("aabbTree")?;
        let nodes = tree.field("nodes")?.array()?;
        let first = nodes.get(0)?;
        let raw = tag.bytes(first.offset, nodes.len() * 6)?;
        let domain = {
            let d = tree.field("domain")?;
            let lo = d.field("min")?.vector::<4>()?;
            let hi = d.field("max")?.vector::<4>()?;
            [[lo[0], lo[1], lo[2]], [hi[0], hi[1], hi[2]]]
        };
        let decoded = BvTree::decode(raw, NodeWidth::Six, domain, 0)?;
        let leaves = decoded.leaf_nodes(want.triangles.len())?;
        for (i, corners) in want.triangles.iter().enumerate() {
            for &v in corners {
                differences +=
                    usize::from(decoded.excess_ulps(leaves[i], want.vertices[v as usize]).unwrap() != 0.0);
            }
        }
    } else {
        let mut keys: Vec<u32> = simd.leaves().map(|(k, _)| k).collect();
        keys.sort_unstable();
        differences +=
            usize::from(keys != (0..u32::try_from(want.triangles.len()).unwrap()).collect::<Vec<_>>());
    }
    Ok(differences)
}

fn visit(t: &mut Totals, m: &Member<'_>) -> Result<()> {
    let tag = m.tag;
    let mut image = tag.image()?;
    let mut compressed: Vec<(usize, usize, BuiltMesh)> = Vec::new();
    let mut externs: Vec<(usize, havok_formats::ExternMesh, u64)> = Vec::new();
    for (k, shape) in shapes(tag)?.into_iter().enumerate() {
        if is_compressed_mesh(shape.type_name()) {
            let data = shape.field("data")?.dereference()?;
            let mesh = compressed_mesh(data)?;
            let radius = f64::from(shape.field("convexRadius")?.float()?);
            nibble_rule(t, data, radius, &mesh)?;
            if let Some(built) = rebuild(t, shape, &mesh)? {
                let (item, _) = tag.item_at(shape.offset).unwrap();
                replace_compressed_mesh(&mut image, item, &built)?;
                compressed.push((k, item, built));
            }
        } else if shape.type_name() == "hknpExternMeshShape" {
            let mesh = extern_mesh(shape)?;
            if rewrite_extern(t, &mut image, tag, shape)?.is_some() {
                let bv = shape.field("boundingVolumeData")?.dereference()?;
                let rule =
                    u64::from(bitlen(mesh.triangles.len() as u64)) + u64::from(!bv.has_field("aabbTree"));
                externs.push((k, mesh, rule));
            }
        }
    }
    if compressed.is_empty() && externs.is_empty() {
        return Ok(());
    }
    let written = image.write()?;
    let again = Tagfile::read(&written, m.compendium)?;
    let shapes_again = shapes(&again)?;
    for (k, _item, built) in &compressed {
        let shape = shapes_again[*k];
        let data = shape.field("data")?.dereference()?;
        let mesh = compressed_mesh(data)?;
        let want = built.triangles()?;
        let same = mesh.triangles.len() == want.len()
            && mesh
                .triangles
                .iter()
                .zip(&want)
                .all(|(a, b)| a.key == b.key && a.vertices == b.vertices);
        t.read_back_differences += usize::from(!same);
        t.outside += mesh.bounds.outside;
        let exact = mesh.simd.leaves == want.len()
            && mesh.simd.leaves_exact == want.len()
            && mesh.simd.children_looser == 0
            && mesh.simd.children_short == 0
            && mesh.simd.leaves_outside == 0;
        t.simd_inexact += usize::from(!exact);
    }
    for (k, want, bits) in &externs {
        t.extern_differences += check_extern(shapes_again[*k], &again, want, *bits)?;
    }
    Ok(())
}

fn totals(game: Game, sets: &[Set]) -> Result<Option<Totals>> {
    let Some(install) = hkwalk::open(game, sets)? else {
        return Ok(None);
    };
    let t = install.run(visit, Totals::merge)?;
    eprintln!(
        "{} collision rewrite: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

/// The expectations shared by both games: nothing dropped, nothing off, nothing different.
fn clean(t: &Totals) {
    assert_eq!(t.dropped, 0, "triangles the builder left out");
    assert_eq!(
        t.geometry_violations, 0,
        "triangles decoded further from the source than the codecs allow"
    );
    assert_eq!(
        t.read_back_differences, 0,
        "meshes that read back as something else than the builder says it wrote"
    );
    assert_eq!(t.outside, 0, "triangles outside their leaf boxes");
    assert_eq!(
        t.simd_inexact, 0,
        "simd trees that do not name every primitive with exact boxes"
    );
    assert_eq!(
        t.extern_differences, 0,
        "extern meshes that read back differently"
    );
    assert_eq!(
        t.extern_key_bits_off_rule, 0,
        "extern shapes whose numShapeKeyBits break the stock rule"
    );
}

#[test]
fn sekiros_collision_is_rewritten_and_reads_back() -> Result<()> {
    let Some(t) = totals(Game::Sekiro, &[Set::Mesh, Set::Extern])? else {
        return Ok(());
    };
    clean(&t);
    // (1) the nibble rule on the stock topology: inner nodes, leaves and roots reproduced
    assert_eq!((t.inner.equal, t.inner.total), (18_363_654, 21_241_946));
    assert_eq!((t.leaf.equal, t.leaf.total), (16_123_721, 21_786_968));
    assert_eq!((t.root.equal, t.root.total), (272_436, 272_471));
    assert_eq!(t.sections_skipped, 677);
    // (2) 3,344 meshes, 10 of them point sets without a triangle
    assert_eq!((t.rebuilt, t.triangles), (3_334, 25_397_391));
    assert_eq!(t.skipped, BTreeMap::from([("custom primitives only", 10)]));
    // (3) 12,688 extern bodies, all class 2
    assert_eq!((t.extern_aabb, t.extern_simd, t.extern_empty), (12_688, 0, 0));
    Ok(())
}

#[test]
fn elden_rings_collision_is_rewritten_and_reads_back() -> Result<()> {
    let Some(t) = totals(Game::EldenRing, &[Set::Mesh, Set::Extern])? else {
        return Ok(());
    };
    clean(&t);
    assert_eq!((t.inner.equal, t.inner.total), (51_652_581, 57_042_426));
    assert_eq!((t.leaf.equal, t.leaf.total), (47_318_288, 58_482_843));
    assert_eq!((t.root.equal, t.root.total), (719_470, 719_647));
    assert_eq!(t.sections_skipped, 1_635);
    assert_eq!((t.rebuilt, t.triangles), (22_831, 66_238_756));
    assert_eq!(
        t.skipped,
        BTreeMap::from([
            ("custom primitives and triangles", 36),
            ("custom primitives only", 29),
            ("no primitives", 3),
        ])
    );
    // 264,858 extern bodies: 40 of class 2, 264,816 of class 3, 2 without a triangle
    assert_eq!((t.extern_aabb, t.extern_simd, t.extern_empty), (40, 264_816, 2));
    Ok(())
}
