//! Collision as FromSoftware stores it in Havok tagfiles: map compressed meshes with the fsnp material table that maps
//! their shape keys to HitMtrlParam IDs, and the extern triangle meshes of the `f` binders ([`read_map`]), and object
//! bodies as meshes and convex shapes ([`read_bodies`]). The tagfile, compressed mesh and convex shape readers are `havok-formats`'.
use crate::affine::{compose, pose, pose_columns};
use crate::{Result, invalid};
use havok_formats::convex::ConvexShape;
use havok_formats::{BoundsCheck, Compendium, CustomKind, SimdCheck, Tagfile, Value};
use std::collections::BTreeMap;

/// A map mesh as a triangle soup: triangles in the file's frame, each with its HitMtrlParam ID, and counts of what the file leaves
/// unresolved. The counts are reported, never hidden: nothing is dropped silently.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Soup {
    /// Triangle-list positions in the file's source frame, after rigid-body pose.
    pub triangles: Vec<[[f32; 3]; 3]>,
    /// HitMtrlParam IDs; -1 is an explicitly counted absent shape-key binding.
    pub materials: Vec<i32>,
    /// Triangles whose shape key the material table does not map (their material is -1).
    pub unmapped: usize,
    /// Triangles of extern mesh bodies (`hknpExternMeshShape`, the `f` binders'): stored whole, with no shape key and
    /// no HitMtrlParam ID (their material is -1, and they are not among `unmapped`).
    pub extern_triangles: usize,
    /// Primitives the compressed mesh marks dead (indices `0xde, 0xad, 0xde, 0xad`); they contribute no triangles.
    pub dead_primitives: usize,
    /// Entries of the shape-key table that are the invalid all-ones key.
    pub invalid_material_keys: usize,
    /// Triangle-data rows no shape key takes, at the end or among the others.
    pub unkeyed_triangle_data: usize,
    /// Shape keys whose material depends on where the unkeyed rows sit, which the file does not state: their rows
    /// could be any of several naming different materials. Their triangles have no material (-1) and are unmapped.
    pub placement_dependent_keys: usize,
    /// The shape classes of bodies [`read_map`] has no decoder for, one per body: listed, not read.
    pub unread_bodies: Vec<String>,
    /// Custom primitives whose data is not an extern shape reference ([`havok_formats::CustomKind::Unread`]): not read.
    pub unread_custom_primitives: usize,
    /// The bounding-volume trees of the compressed meshes read: decoded and checked for every mesh (a tree that does not
    /// hold together is refused), with the triangles and quads counted against the boxes of their leaves.
    pub bounds: BoundsCheck,
    /// The simd trees of those meshes, decoded and tested against the meshes' primitives the same way.
    pub simd: SimdCheck,
}

/// The materials a compressed mesh's fsnp parameter gives its shape keys, with what it leaves unresolved.
#[derive(Default)]
struct KeyMaterials {
    materials: BTreeMap<u32, i32>,
    /// All-ones (invalid) keys of the table.
    invalid_keys: usize,
    /// Triangle-data rows beyond the key table.
    unkeyed_rows: usize,
    /// Keys whose candidate rows name different materials.
    placement_dependent: usize,
}

/// Shape key to material.
///
/// The key table is indexed like the triangle data, which may hold more rows than it: rows no key takes, at the end
/// (Elden Ring stores some tables shorter than their data) or among the others (one Sekiro castle mesh has 24 such rows
/// in its first 8,000; Elden Ring asset meshes have up to 46). The file does not state where those rows sit, and a row's
/// vertex ids do not settle it: a row whose ids are new agrees with any triangle, and in Elden Ring meshes several
/// placements agree with every triangle. So the row of the key at index `i` is one of `i ..= i + beyond`. A key whose
/// candidate rows all name one material has it wherever the unkeyed rows sit; a key whose candidates name different
/// materials depends on their placement, has no material (-1) and is counted, never given one candidate's. A triangle
/// whose key the table does not map has no material (-1) and is counted as unmapped.
fn material_keys(shape: Value<'_, '_>, has_triangles: bool) -> Result<KeyMaterials> {
    let parameter = shape.field("pParam")?.dereference()?;
    let triangle_data = parameter.field("triangleDataArray")?.array()?;
    let primitive_data = parameter.field("primitiveDataArray")?.array()?;
    let keys = shape.field("triangleIndexToShapeKey")?.array()?;
    let bits = shape.field("numShapeKeyBits")?.unsigned()?;
    // The shape's key width is the tree's: every compressed mesh of the Sekiro and Elden Ring installs stores the same
    // count in both. A tree that disagrees would put the keys' section and primitive bits elsewhere.
    let tree_bits = shape
        .field("data")?
        .dereference()?
        .field("meshTree")?
        .field("bitsPerKey")?
        .signed()?;
    if i64::try_from(bits).ok() != Some(tree_bits) {
        return Err(invalid(
            "collision",
            shape.offset,
            format!("numShapeKeyBits is {bits}, the mesh tree's bitsPerKey {tree_bits}"),
        ));
    }
    // A mesh of no triangles has no key bits: its rows identify nothing, and are counted as no key's.
    if bits == 0 && !has_triangles && keys.len() <= triangle_data.len() {
        return Ok(KeyMaterials {
            unkeyed_rows: triangle_data.len(),
            ..KeyMaterials::default()
        });
    }
    if !(1..=32).contains(&bits) || (!triangle_data.is_empty() && keys.len() > triangle_data.len()) {
        return Err(invalid(
            "collision",
            shape.offset,
            format!(
                "invalid shape-key table bits={bits} keys={} data={}",
                keys.len(),
                triangle_data.len()
            ),
        ));
    }
    let keyed = if triangle_data.is_empty() { 0 } else { keys.len() };
    let beyond = triangle_data.len().saturating_sub(keyed);
    // Each row's material, in row order.
    let rows: Vec<i32> = (0..triangle_data.len())
        .map(|i| {
            let group = triangle_data.get(i)?.field("primitiveDataIndex")?.unsigned()?;
            let material =
                primitive_data
                    .get(usize::try_from(group).map_err(|_| {
                        invalid("collision", shape.offset, "primitive data index exceeds usize")
                    })?)?
                    .field("materialNameData")?
                    .unsigned()?;
            i32::try_from(material).map_err(|_| invalid("collision", shape.offset, "material exceeds i32"))
        })
        .collect::<Result<_>>()?;
    let mut result = KeyMaterials {
        unkeyed_rows: beyond,
        ..KeyMaterials::default()
    };
    for i in 0..keyed {
        let raw = u32::try_from(keys.get(i)?.unsigned()?)
            .map_err(|_| invalid("collision", shape.offset, "shape key exceeds u32"))?;
        // Invalid shape-key rows do not identify any mesh primitive.
        // Preserve their count instead of overwriting a fictitious key.
        if raw == u32::MAX {
            result.invalid_keys = result.invalid_keys.saturating_add(1);
            continue;
        }
        let key = raw >> u32::try_from(32u64.saturating_sub(bits)).unwrap_or(0);
        let candidates = rows.get(i..=i.saturating_add(beyond)).ok_or_else(|| {
            invalid(
                "collision",
                shape.offset,
                format!("shape-key row {i} has no triangle data"),
            )
        })?;
        let Some(&first) = candidates.first() else {
            continue;
        };
        if candidates.iter().any(|m| *m != first) {
            result.placement_dependent = result.placement_dependent.saturating_add(1);
            continue;
        }
        if result.materials.insert(key, first).is_some_and(|m| m != first) {
            return Err(invalid(
                "collision",
                shape.offset,
                format!("conflicting shape-key materials: key {key:x}, row {i}, new {first}"),
            ));
        }
    }
    Ok(result)
}

/// Decode a map collision tagfile (with the compendium it shares, if any): every compressed mesh body as posed
/// triangles with HitMtrlParam IDs, and every extern mesh body (`hknpExternMeshShape`, the shape of every body of the
/// `f` binders' files: 12,688 in Sekiro's, 264,858 in Elden Ring's) as posed triangles with no HitMtrlParam ID
/// (-1; counted in [`Soup::extern_triangles`]). A body of another shape class is not read: its class is listed in
/// [`Soup::unread_bodies`], never taken for an empty body.
pub fn read_map(bytes: &[u8], compendium: Option<&Compendium>) -> Result<Soup> {
    let tag = Tagfile::read(bytes, compendium)?;
    let variants = tag.root()?.field("namedVariants")?.array()?;
    let mut soup = Soup::default();
    for i in 0..variants.len() {
        let systems = variants
            .get(i)?
            .field("variant")?
            .dereference()?
            .field("systemDatas")?
            .array()?;
        for j in 0..systems.len() {
            let bodies = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
            for k in 0..bodies.len() {
                let body = bodies.get(k)?;
                let p = body.field("position")?.vector::<4>()?;
                let q = body.field("orientation")?.vector::<4>()?;
                if p[..3].iter().chain(q.iter()).any(|f| !f.is_finite()) {
                    return Err(invalid("collision", body.offset, "nonfinite body transform"));
                }
                let shape = body.field("shape")?.dereference()?;
                if shape.type_name() == "hknpExternMeshShape" {
                    extern_mesh_triangles(&mut soup, shape, body.offset, p, q)?;
                    continue;
                }
                if shape.type_name() != "fsnpCustomParamCompressedMeshShape" {
                    soup.unread_bodies.push(shape.type_name().to_owned());
                    continue;
                }
                let mesh = havok_formats::compressed_mesh(shape.field("data")?.dereference()?)?;
                if mesh.custom.iter().any(|c| c.kind != CustomKind::Unread) {
                    return Err(invalid(
                        "collision",
                        shape.offset,
                        "a map mesh referencing extern shapes is not read",
                    ));
                }
                soup.unread_custom_primitives =
                    soup.unread_custom_primitives.saturating_add(mesh.custom.len());
                let keys = material_keys(shape, !mesh.triangles.is_empty())?;
                soup.invalid_material_keys = soup.invalid_material_keys.saturating_add(keys.invalid_keys);
                soup.unkeyed_triangle_data = soup.unkeyed_triangle_data.saturating_add(keys.unkeyed_rows);
                soup.placement_dependent_keys = soup
                    .placement_dependent_keys
                    .saturating_add(keys.placement_dependent);
                soup.dead_primitives = soup.dead_primitives.saturating_add(mesh.dead_primitives);
                soup.bounds.merge(&mesh.bounds);
                soup.simd.merge(&mesh.simd);
                for triangle in mesh.triangles {
                    let mat = keys.materials.get(&triangle.key).copied().unwrap_or(-1);
                    soup.unmapped = soup.unmapped.saturating_add(usize::from(mat == -1));
                    soup.materials.push(mat);
                    let vertices = triangle.vertices.map(|v| pose(v, p, q));
                    if vertices.iter().flatten().any(|f| !f.is_finite()) {
                        return Err(invalid(
                            "collision",
                            body.offset,
                            "nonfinite transformed position",
                        ));
                    }
                    soup.triangles.push(vertices);
                }
            }
        }
    }
    Ok(soup)
}
/// An extern mesh body's triangles ([`havok_formats::extern_mesh`]), posed by the body, added to `soup` with no
/// HitMtrlParam ID. A triangle that stores a material other than -1 is refused: the files bind none (every triangle of
/// both installs' `f` binders stores -1), so another value would be a binding this reader does not understand.
fn extern_mesh_triangles(
    soup: &mut Soup,
    shape: Value<'_, '_>,
    body_offset: usize,
    p: [f32; 4],
    q: [f32; 4],
) -> Result<()> {
    let mesh = havok_formats::extern_mesh(shape)?;
    if let Some(&material) = mesh.materials.iter().find(|&&m| m != -1) {
        return Err(invalid(
            "collision",
            shape.offset,
            format!("an extern mesh triangle stores material {material}, not -1"),
        ));
    }
    let posed: Vec<[f32; 3]> = mesh.vertices.iter().map(|&v| pose(v, p, q)).collect();
    if posed.iter().flatten().any(|f| !f.is_finite()) {
        return Err(invalid(
            "collision",
            body_offset,
            "nonfinite transformed position",
        ));
    }
    for corners in &mesh.triangles {
        let mut triangle = [[0.0; 3]; 3];
        for (out, &i) in triangle.iter_mut().zip(corners) {
            // `extern_mesh` checked every index against the vertices
            *out = *usize::try_from(i)
                .ok()
                .and_then(|i| posed.get(i))
                .ok_or_else(|| invalid("collision", shape.offset, "extern mesh index out of range"))?;
        }
        soup.triangles.push(triangle);
        soup.materials.push(-1);
    }
    soup.extern_triangles = soup.extern_triangles.saturating_add(mesh.triangles.len());
    Ok(())
}

/// A body's geometry as its file stores it.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyGeometry {
    /// A compressed mesh: its triangles posed by the body, in the file frame, and their HitMtrl IDs.
    Mesh(Soup),
    /// A convex shape (box, polytope, capsule, cylinder, plain convex, triangle, sphere): its stored core and convex
    /// radius in its own space,
    /// a capsule's stored segment, the shape's placement in the body's space where a compressed mesh holds it as an
    /// extern shape (`instance`, columns: rotation x, y and z, then the translation), and the body's pose in the file
    /// frame. It stores no material.
    Convex {
        /// The shape's core and convex radius, in its own space.
        core: ConvexShape,
        /// A capsule's segment end points `a`, `b` (its radius is the core's).
        capsule: Option<[[f32; 3]; 2]>,
        /// The shape's placement in the body's space, where a compressed mesh holds it as an extern shape.
        instance: Option<[[f32; 3]; 4]>,
        /// The body's position in the file frame.
        position: [f32; 3],
        /// The body's orientation, a unit quaternion `[x, y, z, w]`.
        orientation: [f32; 4],
    },
}

/// The convex shape classes a body or an extern shape instance may be, read as their core ([`ConvexShape::read`]).
const CONVEX_SHAPES: [&str; 7] = [
    "hknpBoxShape",
    "hknpConvexPolytopeShape",
    "hknpCapsuleShape",
    "hknpCylinderShape",
    "hknpConvexShape",
    "hknpTriangleShape",
    "hknpSphereShape",
];

/// The affine map (columns) from a convex shape's space to the file frame: its extern instance's transform, if any,
/// then the body's pose; composed in f64.
pub fn shape_to_file(
    instance: Option<&[[f32; 3]; 4]>,
    position: [f32; 3],
    orientation: [f32; 4],
) -> [[f32; 3]; 4] {
    let body = pose_columns(position, orientation);
    match instance {
        None => body,
        Some(inner) => {
            let wide = |m: &[[f32; 3]; 4]| m.map(|c| c.map(f64::from));
            compose(&wide(&body), &wide(inner)).map(|c| c.map(crate::affine::narrow))
        }
    }
}

/// A convex shape's geometry: its core, a capsule's segment, its extern `instance` placement and the body's pose.
fn convex(
    shape: Value<'_, '_>,
    p: [f32; 4],
    q: [f32; 4],
    instance: Option<[[f32; 3]; 4]>,
) -> Result<BodyGeometry> {
    if !CONVEX_SHAPES.contains(&shape.type_name()) {
        return Err(invalid(
            "collision",
            shape.offset,
            format!("unsupported extern shape {}", shape.type_name()),
        ));
    }
    let capsule = if shape.type_name() == "hknpCapsuleShape" {
        let end = |name: &str| -> Result<[f32; 3]> {
            let v = shape.field(name)?.vector::<4>()?;
            if v[..3].iter().all(|f| f.is_finite()) {
                Ok([v[0], v[1], v[2]])
            } else {
                Err(invalid("collision", shape.offset, "nonfinite capsule segment"))
            }
        };
        Some([end("a")?, end("b")?])
    } else {
        None
    };
    Ok(BodyGeometry::Convex {
        core: ConvexShape::read(shape)?,
        capsule,
        instance,
        position: [p[0], p[1], p[2]],
        orientation: q,
    })
}

/// One rigid body of an object's collision file.
#[derive(Debug)]
pub struct Body {
    /// The body's stored name.
    pub name: String,
    /// `motionType`, as stored: 0 static, 1 keyframed, 2 dynamic. Map bodies are all 0; among object and asset bodies
    /// only 2 stores a positive mass (the meaning is inferred from that, not from a name in the files).
    pub motion_type: u64,
    /// The Havok class name of the shape (for a compressed mesh that holds extern shapes, of the shape read).
    pub shape: String,
    /// What the shape holds.
    pub geometry: BodyGeometry,
}

/// Object bodies as their file stores them: compressed meshes as posed triangles in the file frame with their
/// materials (-1 where the shape key binds none; a plain compressed mesh binds none), convex shapes as their stored
/// core, radius and pose ([`BodyGeometry`]). Unsupported geometry is an error, never an empty successful body.
pub fn read_bodies(bytes: &[u8], compendium: Option<&Compendium>) -> Result<Vec<Body>> {
    let tag = Tagfile::read(bytes, compendium)?;
    let variants = tag.root()?.field("namedVariants")?.array()?;
    let mut result = Vec::new();
    for i in 0..variants.len() {
        let variant = variants.get(i)?.field("variant")?.dereference()?;
        if variant.type_name() != "hknpPhysicsSceneData" {
            continue;
        }
        let systems = variant.field("systemDatas")?.array()?;
        for j in 0..systems.len() {
            let bodies = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
            for k in 0..bodies.len() {
                let body = bodies.get(k)?;
                let p = body.field("position")?.vector::<4>()?;
                let q = body.field("orientation")?.vector::<4>()?;
                if p[..3].iter().chain(q.iter()).any(|f| !f.is_finite()) {
                    return Err(invalid("collision", body.offset, "nonfinite body transform"));
                }
                let shape = body.field("shape")?.dereference()?;
                let name = body.field("name")?.string()?;
                let motion_type = body.field("motionType")?.unsigned()?;
                let mut add = |shape: Value<'_, '_>, geometry| {
                    result.push(Body {
                        name: name.clone(),
                        motion_type,
                        shape: shape.type_name().into(),
                        geometry,
                    })
                };
                match shape.type_name() {
                    "fsnpCustomParamCompressedMeshShape" | "hknpCompressedMeshShape" => {
                        let mesh = havok_formats::compressed_mesh(shape.field("data")?.dereference()?)?;
                        let instances = havok_formats::extern_shapes(shape, &mesh)?;
                        let keys = if shape.type_name() == "fsnpCustomParamCompressedMeshShape" {
                            material_keys(shape, !mesh.triangles.is_empty())?
                        } else {
                            KeyMaterials::default()
                        };
                        let mut soup = Soup {
                            invalid_material_keys: keys.invalid_keys,
                            unkeyed_triangle_data: keys.unkeyed_rows,
                            placement_dependent_keys: keys.placement_dependent,
                            dead_primitives: mesh.dead_primitives,
                            bounds: mesh.bounds,
                            simd: mesh.simd,
                            unread_custom_primitives: mesh
                                .custom
                                .iter()
                                .filter(|c| c.kind == CustomKind::Unread)
                                .count(),
                            ..Soup::default()
                        };
                        for tri in mesh.triangles {
                            let material = keys.materials.get(&tri.key).copied().unwrap_or(-1);
                            soup.unmapped = soup.unmapped.saturating_add(usize::from(material == -1));
                            soup.materials.push(material);
                            let vertices = tri.vertices.map(|v| pose(v, p, q));
                            if vertices.iter().flatten().any(|f| !f.is_finite()) {
                                return Err(invalid(
                                    "collision",
                                    body.offset,
                                    "nonfinite transformed position",
                                ));
                            }
                            soup.triangles.push(vertices);
                        }
                        // a mesh made of extern shapes only is those shapes, not an empty mesh besides them
                        if !soup.triangles.is_empty() || instances.is_empty() {
                            add(shape, BodyGeometry::Mesh(soup));
                        }
                        for instance in instances {
                            add(
                                instance.shape,
                                convex(instance.shape, p, q, Some(instance.transform))?,
                            );
                        }
                    }
                    kind if CONVEX_SHAPES.contains(&kind) => add(shape, convex(shape, p, q, None)?),
                    other => {
                        return Err(invalid(
                            "collision",
                            shape.offset,
                            format!("unsupported object shape {other}"),
                        ));
                    }
                }
            }
        }
    }
    Ok(result)
}

/// The name of a HitMtrlParam class, a row's ID modulo 100 (the IDs of triangles are `class + 100 * variant`), in `game`:
/// the community's row names (Smithbox's English list, from Paramdex for Sekiro), `None` for a class whose row it leaves
/// unnamed. Sekiro's list names 50 of the 51 rows (all but 0; ten are "Unused"), Elden Ring's 18. Of the 46 classes Elden
/// Ring's collision files use, 29 have no name anywhere (6.2% of its triangles); of Sekiro's, only class 0 (273,823
/// triangles). The names are labels, not meanings: the class 29 "None" of Sekiro is byte for byte the row 0, and Elden
/// Ring's differs from it.
pub fn hit_class_name(game: crate::Game, class: i32) -> Option<&'static str> {
    use crate::Game;
    Some(match (game, class) {
        (Game::Sekiro, 1) => "Stone",
        (Game::Sekiro, 2 | 25 | 26 | 39 | 41..=44 | 48 | 49) => "Unused",
        (Game::Sekiro, 3) => "Dirt",
        (Game::Sekiro, 4) => "Wood",
        (Game::Sekiro, 5) => "Grass",
        (Game::Sekiro, 6) => "Gravel",
        (Game::Sekiro, 7) => "Bamboo",
        (Game::Sekiro, 8) => "Tree",
        (Game::Sekiro, 9 | 10) => "Straw",
        (Game::Sekiro, 11) => "Iron",
        (Game::Sekiro, 12) => "Flesh",
        (Game::Sekiro, 13) => "Sand",
        (Game::Sekiro, 14) => "Bone",
        (Game::Sekiro, 15) => "Ash",
        (Game::Sekiro, 16) => "Tiles",
        (Game::Sekiro, 17) => "Cloth",
        (Game::Sekiro, 18) => "Dead Leaves",
        (Game::Sekiro, 19) => "Bell",
        (Game::Sekiro, 20) => "Water Puddle",
        (Game::Sekiro, 21) => "Water (Walkable)",
        (Game::Sekiro, 22) => "Water (Diveable)",
        (Game::Sekiro, 23) => "Poison Swamp",
        (Game::Sekiro, 24) => "Poison Swamp (with slowing)",
        (Game::Sekiro, 27) => "Mud",
        (Game::Sekiro, 28) => "Tatami",
        (Game::Sekiro, 29) => "None",
        (Game::Sekiro, 30) => "Snow",
        (Game::Sekiro, 31) => "Bush",
        (Game::Sekiro, 32) => "Wood Indoors",
        (Game::Sekiro, 33) => "Autumn Leaves",
        (Game::Sekiro, 34) => "Ice",
        (Game::Sekiro, 35) => "Water Slope",
        (Game::Sekiro, 36) => "Water Slope (swimming)",
        (Game::Sekiro, 37) => "Grass In Water",
        (Game::Sekiro, 38) => "Grass In Water (swimming)",
        (Game::Sekiro, 40) => "Cloud",
        (Game::Sekiro, 45) => "Underwater Dirt",
        (Game::Sekiro, 46) => "Underwater Stone",
        (Game::Sekiro, 47) => "Underwater Wood",
        (Game::Sekiro, 80) => "Miasma",
        (Game::EldenRing, 0) => "None",
        (Game::EldenRing, 1) => "Stock",
        (Game::EldenRing, 2) => "Rock",
        (Game::EldenRing, 3) => "Sand",
        (Game::EldenRing, 4) => "Wood",
        (Game::EldenRing, 5) => "Dirt",
        (Game::EldenRing, 6) => "Ore",
        (Game::EldenRing, 7) => "Lava",
        (Game::EldenRing, 8) => "Scarlet Tree",
        (Game::EldenRing, 11) => "Iron Grate",
        (Game::EldenRing, 12) => "Scarlet Mushroom",
        (Game::EldenRing, 14) => "Bone",
        (Game::EldenRing, 21) => "Water",
        (Game::EldenRing, 23) => "Shallow Poison Swamp",
        (Game::EldenRing, 24) => "Poison Swamp",
        (Game::EldenRing, 44) => "Rain Dirt",
        (Game::EldenRing, 46) => "Scarlet Swamp",
        (Game::EldenRing, 49) => "Lake of Rot",
        _ => return None,
    })
}

/// Named meanings retained alongside a triangle's authored HitMtrlParam ID. Unknown and unbound IDs are not discarded.
/// These distinguish the game's material classes, not a destination's collision surface vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HitMeaning {
    /// Stone or rock, including underwater stone.
    Stone,
    /// Dirt, ash, leaves, mud or snow.
    Earth,
    /// Wood, bamboo, tree or tatami.
    Wood,
    /// Grass, straw or bush.
    Grass,
    /// Gravel.
    Gravel,
    /// Iron or bell.
    Metal,
    /// Sand.
    Sand,
    /// Roof tiles.
    RoofTile,
    /// Ice.
    Ice,
    /// Water whose class explicitly says walkable, including the non-swimming slope.
    WalkableWater,
    /// Diveable/swimming water in Sekiro, water in Elden Ring.
    SwimmingWater,
    /// Sekiro's empty class, distinct from Elden Ring's differently authored `None` row.
    Empty,
    /// Sekiro's cloud class.
    Cloud,
    /// A named class with no more specific supported surface meaning.
    Other,
    /// A nonnegative authored ID whose class has no known meaning.
    Unknown,
    /// No shape-key material binding (negative ID).
    Unbound,
}

/// Interpret the class of an authored HitMtrlParam ID (the ID modulo 100), preserving game-specific numbering.
/// Walkable water is distinct from swimming water: class 21 is walkable in Sekiro and water in Elden Ring.
/// The original ID remains the caller's data; this function does not replace it with a default.
pub fn hit_meaning(game: crate::Game, id: i32) -> HitMeaning {
    use crate::Game;
    use HitMeaning::*;
    if id < 0 {
        return Unbound;
    }
    let class = id % 100;
    match (game, class) {
        (Game::Sekiro, 1 | 46) | (Game::EldenRing, 2) => Stone,
        (Game::Sekiro, 3 | 15 | 18 | 27 | 30 | 33 | 45) | (Game::EldenRing, 5 | 44) => Earth,
        (Game::Sekiro, 4 | 7 | 8 | 28 | 32 | 47) | (Game::EldenRing, 4 | 8) => Wood,
        (Game::Sekiro, 5 | 9 | 10 | 31 | 37 | 38) => Grass,
        (Game::Sekiro, 6) => Gravel,
        (Game::Sekiro, 11 | 19) | (Game::EldenRing, 11) => Metal,
        (Game::Sekiro, 13) | (Game::EldenRing, 3) => Sand,
        (Game::Sekiro, 16) => RoofTile,
        (Game::Sekiro, 34) => Ice,
        (Game::Sekiro, 21 | 35) => WalkableWater,
        (Game::Sekiro, 22 | 36) | (Game::EldenRing, 21) => SwimmingWater,
        (Game::Sekiro, 29) => Empty,
        (Game::Sekiro, 40) => Cloud,
        _ if hit_class_name(game, class).is_some() => Other,
        _ => Unknown,
    }
}

/// Meaning of the MSB collision part's filter byte; the raw byte remains available for reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum HitFilterMeaning {
    /// Main physical map collision (Sekiro's filters 8 and 27; Elden Ring's filter 8).
    Physical,
    /// Another authored filter, retained rather than treated as main physical collision.
    Other,
}

/// Name a stored collision filter in the game's own numbering, without selecting or loading a file.
pub fn hit_filter_meaning(game: crate::Game, filter: u8) -> HitFilterMeaning {
    match (game, filter) {
        (crate::Game::Sekiro, 8 | 27) | (crate::Game::EldenRing, 8) => HitFilterMeaning::Physical,
        _ => HitFilterMeaning::Other,
    }
}

/// Named `motionType`; the dynamic interpretation is inferred from the positive masses of type 2 bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MotionKind {
    /// Stored type 0.
    Static,
    /// Stored type 1.
    Keyframed,
    /// Stored type 2, the bodies with positive mass.
    Dynamic,
    /// An unrecognised authored value; the caller retains the raw value.
    Unknown,
}

/// Name a stored body motion value without changing its pose or inferring animation.
pub fn motion_kind(value: u64) -> MotionKind {
    match value {
        0 => MotionKind::Static,
        1 => MotionKind::Keyframed,
        2 => MotionKind::Dynamic,
        _ => MotionKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::hit_class_name;
    use crate::Game;

    #[test]
    fn collision_meanings_keep_the_game_numbering_and_unknown_values() {
        use super::{
            HitFilterMeaning as F, HitMeaning as H, MotionKind as M, hit_filter_meaning, hit_meaning,
            motion_kind,
        };
        assert_eq!(hit_meaning(Game::Sekiro, 21), H::WalkableWater);
        assert_eq!(hit_meaning(Game::Sekiro, 22), H::SwimmingWater);
        assert_eq!(hit_meaning(Game::Sekiro, 36), H::SwimmingWater);
        assert_eq!(hit_meaning(Game::EldenRing, 21), H::SwimmingWater);
        assert_eq!(hit_meaning(Game::Sekiro, 29), H::Empty);
        assert_eq!(hit_meaning(Game::EldenRing, 0), H::Other);
        assert_eq!(hit_meaning(Game::EldenRing, 99), H::Unknown);
        assert_eq!(hit_meaning(Game::Sekiro, -1), H::Unbound);
        assert_eq!(hit_filter_meaning(Game::Sekiro, 27), F::Physical);
        assert_eq!(hit_filter_meaning(Game::EldenRing, 27), F::Other);
        assert_eq!(motion_kind(2), M::Dynamic);
        assert_eq!(motion_kind(99), M::Unknown);
    }

    #[test]
    fn classes_are_named_as_the_community_names_the_rows() {
        assert_eq!(hit_class_name(Game::Sekiro, 1), Some("Stone"));
        assert_eq!(hit_class_name(Game::Sekiro, 25), Some("Unused"));
        assert_eq!(hit_class_name(Game::Sekiro, 43), Some("Unused"));
        assert_eq!(hit_class_name(Game::Sekiro, 0), None);
        assert_eq!(hit_class_name(Game::EldenRing, 0), Some("None"));
        // the numbering is not shared: 3 is Dirt in Sekiro and Sand in Elden Ring
        assert_eq!(hit_class_name(Game::Sekiro, 3), Some("Dirt"));
        assert_eq!(hit_class_name(Game::EldenRing, 3), Some("Sand"));
        assert_eq!(hit_class_name(Game::EldenRing, 29), None);
        assert_eq!(hit_class_name(Game::Sekiro, -1), None);
        let named = |g| (0..100).filter(|c| hit_class_name(g, *c).is_some()).count();
        assert_eq!((named(Game::Sekiro), named(Game::EldenRing)), (50, 18));
    }
}
