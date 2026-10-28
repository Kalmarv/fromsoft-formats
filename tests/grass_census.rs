//! The grass meshes (`.grass`) of the map and asset binders of each installed game through [`Grass`]: every file reads, its
//! volumes form the tree the reader checks, a volume's mask word is the set of grass types its faces' vertices grow, the
//! box of a volume holds the vertices of its faces, and the six densities lie in 0 to 1.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (the layout from the
//! community's reader and the binary template, the tree and the masks read from the data).
mod inputs;

use fromsoft_formats::{Game, NameDictionary, dcx::Decoder, grass::Grass};
use inputs::{OracleSet, Query, Tool, expect_found, oodle, oracle};
use std::collections::BTreeMap;
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Totals {
    files: usize,
    volumes: usize,
    vertices: usize,
    faces: usize,
    /// Volumes by number of children.
    children: BTreeMap<usize, usize>,
    /// Vertices by the number of grass types with a density above 0.
    types_per_vertex: BTreeMap<usize, usize>,
    density_range: (f32, f32),
    /// Vertices whose densities sum to more than one.
    over_one: usize,
    mask_differs: usize,
    box_misses_a_vertex: usize,
    /// Faces with a stored normal of length zero, and (of those with a normal and an area) the ones whose normal is not within
    /// 0.001 of one in length and the ones whose cosine with the cross product of their edges is above -0.999.
    zero_normals: usize,
    non_unit_normals: usize,
    normals_not_against_the_cross_product: usize,
}

fn census(game: Game, data: inputs::Game) -> Result<Option<Totals>> {
    let (Some(archive), Some(dll), Some(names)) = (
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
    let names = NameDictionary::read_for(names, game)?;
    let mut t = Totals {
        density_range: (f32::MAX, f32::MIN),
        ..Totals::default()
    };
    common::each_leaf(
        &archive,
        &names,
        &[".mapbnd.dcx", ".geombnd.dcx"],
        &decoder,
        |label, bytes| {
            if !label.to_ascii_lowercase().ends_with(".grass") {
                return Ok(());
            }
            let grass = Grass::read(bytes).map_err(|e| format!("{label}: {e}"))?;
            t.files += 1;
            t.volumes += grass.volumes.len();
            t.vertices += grass.vertices.len();
            t.faces += grass.faces.len();
            for volume in &grass.volumes {
                let n =
                    usize::from(volume.first_child.is_some()) + usize::from(volume.second_child.is_some());
                *t.children.entry(n).or_default() += 1;
                let mut mask = 0u32;
                let mut misses = false;
                for face in &grass.faces[volume.faces.clone()] {
                    for &i in &face.vertices {
                        let vertex = &grass.vertices[i];
                        for (k, d) in vertex.densities.iter().enumerate() {
                            mask |= u32::from(*d > 0.) << k;
                        }
                        misses |= (0..3).any(|a| {
                            let p = f64::from(vertex.position[a]);
                            p < f64::from(volume.bounds[0][a]) - 1e-3
                                || p > f64::from(volume.bounds[1][a]) + 1e-3
                        });
                    }
                }
                t.mask_differs += usize::from(mask != volume.grass_types);
                t.box_misses_a_vertex += usize::from(misses);
            }
            for vertex in &grass.vertices {
                let positive = vertex.densities.iter().filter(|d| **d > 0.).count();
                *t.types_per_vertex.entry(positive).or_default() += 1;
                for d in vertex.densities {
                    t.density_range = (t.density_range.0.min(d), t.density_range.1.max(d));
                }
                t.over_one +=
                    usize::from(vertex.densities.iter().map(|d| f64::from(*d)).sum::<f64>() > 1.0001);
            }
            for face in &grass.faces {
                let [a, b, c] = face.vertices.map(|i| grass.vertices[i].position.map(f64::from));
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let cross = [
                    u[1] * w[2] - u[2] * w[1],
                    u[2] * w[0] - u[0] * w[2],
                    u[0] * w[1] - u[1] * w[0],
                ];
                let normal = face.normal.map(f64::from);
                let (cl, nl) = (
                    cross.iter().map(|x| x * x).sum::<f64>().sqrt(),
                    normal.iter().map(|x| x * x).sum::<f64>().sqrt(),
                );
                if nl == 0. {
                    t.zero_normals += 1;
                } else if cl > 1e-12 {
                    t.non_unit_normals += usize::from((nl - 1.).abs() > 1e-3);
                    let dot: f64 = cross.iter().zip(normal).map(|(x, y)| x * y).sum();
                    t.normals_not_against_the_cross_product += usize::from(dot / (cl * nl) > -0.999);
                }
            }
            Ok(())
        },
    )?;
    Ok(Some(t))
}

#[test]
fn sekiros_grass_meshes() -> Result<()> {
    let Some(t) = census(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    println!(
        "{} files, {} volumes {:?}, {} vertices {:?}, {} faces; densities {:?}, {} over one; masks differing {}, boxes missing a vertex {}; normals: {} zero, {} not unit, {} not against the cross product",
        t.files,
        t.volumes,
        t.children,
        t.vertices,
        t.types_per_vertex,
        t.faces,
        t.density_range,
        t.over_one,
        t.mask_differs,
        t.box_misses_a_vertex,
        t.zero_normals,
        t.non_unit_normals,
        t.normals_not_against_the_cross_product
    );
    expect_found(
        Query::Install(inputs::Game::Sekiro, None),
        "grass meshes",
        t.files,
    );
    assert_eq!(
        (t.files, t.volumes, t.vertices, t.faces),
        (772, 15_344, 723_967, 1_065_442)
    );
    assert_eq!(t.children, BTreeMap::from([(0, 7_937), (1, 242), (2, 7_165)]));
    assert_eq!(
        t.types_per_vertex,
        BTreeMap::from([
            (0, 133_503),
            (1, 248_286),
            (2, 183_381),
            (3, 102_187),
            (4, 40_767),
            (5, 1_677),
            (6, 14_166)
        ])
    );
    assert_eq!(t.density_range, (0., 1.));
    assert_eq!(t.over_one, 284_661);
    assert_eq!((t.mask_differs, t.box_misses_a_vertex), (0, 0));
    // the normal points against the cross product of the face's edges on all but 123 faces; 532 faces store none
    assert_eq!(
        (
            t.zero_normals,
            t.non_unit_normals,
            t.normals_not_against_the_cross_product
        ),
        (532, 128_192, 123)
    );
    Ok(())
}

#[test]
fn elden_rings_grass_meshes() -> Result<()> {
    let Some(t) = census(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    println!(
        "{} files, {} volumes {:?}, {} vertices {:?}, {} faces; densities {:?}, {} over one; masks differing {}, boxes missing a vertex {}; normals: {} zero, {} not unit, {} not against the cross product",
        t.files,
        t.volumes,
        t.children,
        t.vertices,
        t.types_per_vertex,
        t.faces,
        t.density_range,
        t.over_one,
        t.mask_differs,
        t.box_misses_a_vertex,
        t.zero_normals,
        t.non_unit_normals,
        t.normals_not_against_the_cross_product
    );
    expect_found(Query::Install(inputs::Game::Elden, None), "grass meshes", t.files);
    assert_eq!(
        (t.files, t.volumes, t.vertices, t.faces),
        (1_925, 89_852, 4_490_801, 7_799_267)
    );
    assert_eq!(t.children, BTreeMap::from([(0, 45_879), (1, 19), (2, 43_954)]));
    assert_eq!(
        t.types_per_vertex,
        BTreeMap::from([
            (0, 574_942),
            (1, 1_041_497),
            (2, 1_413_705),
            (3, 751_222),
            (4, 324_111),
            (5, 166_553),
            (6, 218_771)
        ])
    );
    assert_eq!(t.density_range, (0., 1.));
    assert_eq!(t.over_one, 1_169_828);
    assert_eq!((t.mask_differs, t.box_misses_a_vertex), (0, 0));
    assert_eq!(
        (
            t.zero_normals,
            t.non_unit_normals,
            t.normals_not_against_the_cross_product
        ),
        (674, 492_851, 469)
    );
    Ok(())
}
