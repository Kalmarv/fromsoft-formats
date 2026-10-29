//! Rebuilds the compressed-mesh collision of one binder member from its decoded triangles with the stock layout
//! (`Layout::Stock`, paired quads) and writes the binder back, with a control that goes through the same steps and leaves
//! the Havok file as it is.
//!
//! ```text
//! cargo run --example rebuild_collision -- scan    <sekiro|elden> <game folder> <keys folder> <oodle library> <name dictionary> [<path filter>]
//! cargo run --example rebuild_collision -- where   <sekiro|elden> <game folder> <keys folder> <oodle library> <name dictionary> <model name or prefix, comma-separated>
//! cargo run --example rebuild_collision -- rebuild <sekiro|elden> <game folder> <keys folder> <oodle library> <archive path> <member filter> <out folder>
//! ```
//!
//! `scan` lists the compressed meshes of the object (Sekiro `/obj/*.objbnd.dcx`) or asset (Elden Ring
//! `/asset/aeg/*.geomhkxbnd.dcx`) binders whose path holds the filter, with their triangle counts and boxes.
//!
//! `where` lists the parts of the map layouts that place a model by name or name prefix (a position to find the object by in the game).
//!
//! `rebuild` reads the binder at the archive path, takes the one Havok member whose name holds the member filter and
//! writes two binders named as the stock one, into `<out>/control/` and `<out>/rebuilt/`. Both go through the same
//! steps (the Havok file written from its image, the binder written, the DCX envelope framed again with the stock
//! envelope's own variant), so that they differ in the collision only: the control writes the image as it was read (the
//! writer's bytes for an unedited file are the file's), the rebuild replaces each compressed mesh with
//! `build_compressed_mesh_with(Layout::Stock, PairedQuads)` over the triangles it decodes to, in the source order of the
//! file's own `triangleIndexToShapeKey` (so that the per-triangle tables of the shape still name the same triangles).
//! The checks it makes are printed; a failed one is an error. The outputs hold game data: keep them out of any repository.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]
#![allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::print_stdout)]

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use fromsoft_formats::dcx::{self, Decoder, Encoder, Envelope};
use fromsoft_formats::msb::MapLayout;
use fromsoft_formats::{Binder, Game, Install, Keys, NameDictionary};
use havok_formats::{
    BuildOptions, BuiltMesh, Compendium, Layout, MeshSource, Primitives, Tagfile, Value,
    build_compressed_mesh_with, compressed_mesh, replace_compressed_mesh,
};
use primase_foundations::compress::oodle::{Oodle, Version};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn game_of(name: &str) -> Result<Game> {
    match name {
        "sekiro" => Ok(Game::Sekiro),
        "elden" => Ok(Game::EldenRing),
        other => Err(format!("unknown game {other:?}").into()),
    }
}

fn is_mesh_binder(game: Game, path: &str) -> bool {
    match game {
        Game::Sekiro => path.starts_with("/obj/") && path.ends_with(".objbnd.dcx"),
        Game::EldenRing => path.starts_with("/asset/aeg/") && path.ends_with(".geomhkxbnd.dcx"),
    }
}

fn is_mesh_class(class: &str) -> bool {
    matches!(
        class,
        "fsnpCustomParamCompressedMeshShape" | "hknpCompressedMeshShape"
    )
}

/// Every shape of every physics body of a tagfile.
fn shapes<'t, 'a>(tag: &'t Tagfile<'a>) -> Result<Vec<Value<'t, 'a>>> {
    let mut out = Vec::new();
    let variants = tag.root()?.field("namedVariants")?.array()?;
    for i in 0..variants.len() {
        let physics = variants.get(i)?.field("variant")?.dereference()?;
        if physics.type_name() != "hknpPhysicsSceneData" {
            continue;
        }
        let systems = physics.field("systemDatas")?.array()?;
        for j in 0..systems.len() {
            let bodies = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
            for k in 0..bodies.len() {
                out.push(bodies.get(k)?.field("shape")?.dereference()?);
            }
        }
    }
    Ok(out)
}

/// A Havok member of a binder, with the compendium that reads it (none: it has its own types).
struct HavokMember {
    index: usize,
    name: String,
    compendium: Option<Compendium>,
}

fn havok_members(binder: &Binder) -> Result<Vec<HavokMember>> {
    let mut compendia = Vec::new();
    let mut tags = Vec::new();
    for (index, f) in binder.files.iter().enumerate() {
        let name = f.name.as_deref().unwrap_or("").to_ascii_lowercase();
        if name.ends_with(".compendium") && f.bytes.get(4..8) == Some(b"TCM0") {
            compendia.push(Compendium::read(&f.bytes)?);
        } else if name.ends_with(".hkx") && f.bytes.get(4..8) == Some(b"TAG0") {
            tags.push((index, name));
        }
    }
    let mut out = Vec::new();
    for (index, name) in tags {
        let bytes = &binder.files[index].bytes;
        let compendium = if Tagfile::read(bytes, None).is_ok() {
            None
        } else {
            let c = compendia
                .iter()
                .position(|c| Tagfile::read(bytes, Some(c)).is_ok())
                .ok_or_else(|| format!("{name}: no compendium reads it"))?;
            Some(compendia.swap_remove(c))
        };
        out.push(HavokMember {
            index,
            name,
            compendium,
        });
    }
    Ok(out)
}

fn bounds_of(triangles: impl Iterator<Item = [[f32; 3]; 3]>) -> [[f32; 3]; 2] {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for t in triangles {
        for v in t {
            for a in 0..3 {
                lo[a] = lo[a].min(v[a]);
                hi[a] = hi[a].max(v[a]);
            }
        }
    }
    [lo, hi]
}

fn scan(
    game: Game,
    archive: &fromsoft_formats::Archive,
    decoder: &Decoder<'_>,
    dictionary: &str,
    filter: &str,
) -> Result<()> {
    let names = NameDictionary::read_for(dictionary, game)?;
    let mut paths: Vec<&String> = names
        .names
        .iter()
        .filter(|(hash, _)| archive.entries.contains_key(*hash))
        .flat_map(|(_, p)| p)
        .filter(|p| is_mesh_binder(game, p) && p.contains(filter))
        .collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        let outer = archive.read(path, decoder)?;
        let binder = Binder::read(&outer, decoder)?;
        for m in havok_members(&binder)? {
            let bytes = &binder.files[m.index].bytes;
            let tag = Tagfile::read(bytes, m.compendium.as_ref())?;
            for shape in shapes(&tag)? {
                if !is_mesh_class(shape.type_name()) {
                    continue;
                }
                let data = shape.field("data")?.dereference()?;
                let mesh = compressed_mesh(data)?;
                let b = bounds_of(mesh.triangles.iter().map(|t| t.vertices));
                let sections = data.field("meshTree")?.field("sections")?.array()?.len();
                let (up, top) = floor_area(&mesh);
                println!(
                    "{path} {} {}: {} triangles, {} custom, {} dead, {sections} sections, box {:.2?}..{:.2?}, upward-facing area {up:.2} m2 up to height {top:.2}",
                    m.name,
                    shape.type_name(),
                    mesh.triangles.len(),
                    mesh.custom.len(),
                    mesh.dead_primitives,
                    b[0],
                    b[1]
                );
            }
        }
    }
    Ok(())
}

/// The parts of every map layout that place a model whose name starts with one of the comma-separated `model` names: map, part, position, rotation and scale.
fn find_parts(
    game: Game,
    archive: &fromsoft_formats::Archive,
    decoder: &Decoder<'_>,
    dictionary: &str,
    model: &str,
) -> Result<()> {
    let names = NameDictionary::read_for(dictionary, game)?;
    // `MAP_FILTER=m10_00` limits the layouts read to those whose path holds it
    let map_filter = std::env::var("MAP_FILTER").unwrap_or_default();
    let mut paths: Vec<&String> = names
        .names
        .iter()
        .filter(|(hash, _)| archive.entries.contains_key(*hash))
        .flat_map(|(_, p)| p)
        .filter(|p| {
            p.starts_with("/map/mapstudio/") && p.ends_with(".msb.dcx") && p.contains(map_filter.as_str())
        })
        .collect();
    paths.sort();
    paths.dedup();
    for path in paths {
        let layout = MapLayout::read(&archive.read(path, decoder)?)?;
        for part in &layout.parts {
            let Some(m) = part.model.and_then(|i| layout.models.get(i)) else {
                continue;
            };
            let name = m.name.to_ascii_lowercase();
            if model
                .split(',')
                .any(|wanted| name.starts_with(&wanted.to_ascii_lowercase()))
            {
                println!(
                    "{path} {} {:?} at {:.2?} rotation {:.1?} scale {:.2?} entity {} matrix {:?}",
                    part.name,
                    part.kind,
                    part.position,
                    part.rotation_degrees,
                    part.scale,
                    part.entity_id,
                    part.matrix().ok()
                );
            }
        }
    }
    Ok(())
}

/// The area of the triangles that face up (normal within about 18 degrees of +y) and the height of the highest of them.
fn floor_area(mesh: &havok_formats::CompressedMesh) -> (f32, f32) {
    let mut area = 0f32;
    let mut top = f32::NEG_INFINITY;
    for t in &mesh.triangles {
        let [a, b, c] = t.vertices;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 0.0 && n[1] / len > 0.95 {
            area += len / 2.0;
            top = top.max(a[1]).max(b[1]).max(c[1]);
        }
    }
    (area, top)
}

/// The centroid of the largest upward-facing triangle within 0.3 of the highest one: a point on the shape's top surface
/// to stand a character at (shape-local; the part's rotation and position take it into the map).
fn stand_point(mesh: &havok_formats::CompressedMesh) -> Option<([f32; 3], f32)> {
    let up = |t: &havok_formats::Triangle| {
        let [a, b, c] = t.vertices;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        (len > 0.0 && n[1] / len > 0.95).then_some((
            len / 2.0,
            [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ],
        ))
    };
    let top = mesh
        .triangles
        .iter()
        .filter(|t| up(t).is_some())
        .map(|t| t.vertices.iter().map(|v| v[1]).fold(f32::NEG_INFINITY, f32::max))
        .fold(f32::NEG_INFINITY, f32::max);
    mesh.triangles
        .iter()
        .filter_map(|t| up(t).map(|(area, centre)| (area, centre)))
        .filter(|(_, c)| c[1] >= top - 0.3)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(area, centre)| (centre, area))
}

/// The quantisation the codecs allow a vertex of the rebuilt triangle with `key`, per axis (as the crate's rewrite test).
fn allowed(built: &BuiltMesh, key: u32, axis: usize) -> f64 {
    let packed = f64::from(built.sections[(key >> 8) as usize].codec[3 + axis]);
    let ranges = [2_097_151.0, 2_097_151.0, 4_194_303.0];
    let shared = (f64::from(built.domain[1][axis]) - f64::from(built.domain[0][axis])) / ranges[axis];
    0.5001 * packed.max(shared)
}

/// What a rebuilt mesh has to show when the new file is read.
struct Expect {
    shape_index: usize,
    built: BuiltMesh,
    /// The source triangles in source order (`None`: a row of the key table that holds no triangle).
    source: Vec<Option<[[f32; 3]; 3]>>,
    bits_before: u64,
}

/// The unedited image's bytes, the rebuilt image's bytes, and what the rebuilt meshes must show.
fn rebuild(binder: &Binder, member: &HavokMember) -> Result<(Vec<u8>, Vec<u8>, Vec<Expect>)> {
    let bytes = &binder.files[member.index].bytes;
    let tag = Tagfile::read(bytes, member.compendium.as_ref())?;
    let mut image = tag.image()?;
    let control = tag.image()?.write()?;
    let mut expects = Vec::new();
    for (shape_index, shape) in shapes(&tag)?.into_iter().enumerate() {
        if !is_mesh_class(shape.type_name()) {
            continue;
        }
        let data = shape.field("data")?.dereference()?;
        let mesh = compressed_mesh(data)?;
        if !mesh.custom.is_empty() {
            return Err(format!(
                "shape {shape_index} holds {} custom primitives: not rebuilt",
                mesh.custom.len()
            )
            .into());
        }
        let radius = shape.field("convexRadius")?.float()?;
        let bits = shape.field("numShapeKeyBits")?.unsigned()?;
        // the source order: the key table's own, so that the per-triangle tables of the shape keep their meaning
        let by_key: BTreeMap<u32, [[f32; 3]; 3]> =
            mesh.triangles.iter().map(|t| (t.key, t.vertices)).collect();
        let mut source: Vec<Option<[[f32; 3]; 3]>> = Vec::new();
        if let Ok(table) = shape.field("triangleIndexToShapeKey") {
            let table = table.array()?;
            let shift = u32::try_from(32u64.saturating_sub(bits))?;
            for i in 0..table.len() {
                let raw = u32::try_from(table.get(i)?.unsigned()?)?;
                source.push(if raw == u32::MAX {
                    None
                } else {
                    Some(
                        *by_key
                            .get(&(raw >> shift))
                            .ok_or_else(|| format!("key table row {i} names a key the mesh lacks"))?,
                    )
                });
            }
            let used = source.iter().flatten().count();
            if used != mesh.triangles.len() {
                return Err(format!(
                    "the key table names {used} triangles, the mesh decodes {}",
                    mesh.triangles.len()
                )
                .into());
            }
        } else {
            source.extend(mesh.triangles.iter().map(|t| Some(t.vertices)));
        }
        // vertices shared by bit pattern; a row that holds no triangle is a collapsed one the builder leaves out
        let mut index: HashMap<[u32; 3], u32> = HashMap::new();
        let mut vertices: Vec<[f32; 3]> = Vec::new();
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        for row in &source {
            let Some(t) = row else {
                triangles.push([0, 0, 0]);
                continue;
            };
            let mut corners = [0u32; 3];
            for (c, v) in corners.iter_mut().zip(t) {
                *c = *index.entry(v.map(f32::to_bits)).or_insert_with(|| {
                    vertices.push(*v);
                    u32::try_from(vertices.len() - 1).unwrap_or(u32::MAX)
                });
            }
            triangles.push(corners);
        }
        if vertices.is_empty() {
            vertices.push([0.0; 3]);
        }
        let built = build_compressed_mesh_with(
            &MeshSource {
                vertices: &vertices,
                triangles: &triangles,
                convex_radius: radius,
            },
            &BuildOptions {
                primitives: Primitives::PairedQuads,
                layout: Layout::Stock,
                drop_degenerate: None,
            },
        )?;
        let (item, _) = tag.item_at(shape.offset).ok_or("shape item")?;
        replace_compressed_mesh(&mut image, item, &built)?;
        expects.push(Expect {
            shape_index,
            built,
            source,
            bits_before: bits,
        });
    }
    Ok((control, image.write()?, expects))
}

/// Read the rebuilt Havok file back: every shape's triangles against the builder's arrays and the source triangles.
fn check(written: &[u8], member: &HavokMember, original: &[u8], expects: &[Expect]) -> Result<()> {
    let again = Tagfile::read(written, member.compendium.as_ref())?;
    let before = Tagfile::read(original, member.compendium.as_ref())?;
    let shapes_again = shapes(&again)?;
    let shapes_before = shapes(&before)?;
    if shapes_again.len() != shapes_before.len() {
        return Err("the rebuilt file has another number of bodies".into());
    }
    for e in expects {
        let shape = shapes_again[e.shape_index];
        let data = shape.field("data")?.dereference()?;
        let mesh = compressed_mesh(data)?;
        let want = e.built.triangles()?;
        let same = mesh.triangles.len() == want.len()
            && mesh
                .triangles
                .iter()
                .zip(&want)
                .all(|(a, b)| a.key == b.key && a.vertices == b.vertices);
        if !same {
            return Err(format!(
                "shape {}: the file does not read back to the builder's arrays",
                e.shape_index
            )
            .into());
        }
        if mesh.bounds.outside != 0
            || mesh.simd.leaves_exact != mesh.simd.leaves
            || mesh.simd.leaves != e.built.primitives.len()
        {
            return Err(format!(
                "shape {}: bounds or simd tree check failed: {:?} {:?}",
                e.shape_index, mesh.bounds, mesh.simd
            )
            .into());
        }
        // each source row's new key names the triangle the old key named (to the codecs' quantisation)
        let table = shape.field("triangleIndexToShapeKey")?.array()?;
        if table.len() != e.source.len() {
            return Err("the key table changed its length".into());
        }
        let bits = shape.field("numShapeKeyBits")?.unsigned()?;
        let shift = u32::try_from(32u64.saturating_sub(bits))?;
        let by_key: BTreeMap<u32, [[f32; 3]; 3]> =
            mesh.triangles.iter().map(|t| (t.key, t.vertices)).collect();
        let before_table = shapes_before[e.shape_index]
            .field("triangleIndexToShapeKey")?
            .array()?;
        let before_bits = shapes_before[e.shape_index]
            .field("numShapeKeyBits")?
            .unsigned()?;
        let before_shift = u32::try_from(32u64.saturating_sub(before_bits))?;
        let mut same_key = 0usize;
        let mut worst = 0f64;
        let mut rows = 0usize;
        let mut left_out = 0usize;
        for (i, row) in e.source.iter().enumerate() {
            let raw = u32::try_from(table.get(i)?.unsigned()?)?;
            match (row, raw) {
                (None, u32::MAX) => left_out += 1,
                (None, _) => return Err(format!("row {i} holds no triangle but names a key").into()),
                (Some(_), u32::MAX) => return Err(format!("row {i} lost its triangle").into()),
                (Some(t), raw) => {
                    let key = raw >> shift;
                    same_key +=
                        usize::from(u32::try_from(before_table.get(i)?.unsigned()?)? >> before_shift == key);
                    let got = by_key
                        .get(&key)
                        .ok_or_else(|| format!("row {i}: key {key:#x} is not in the mesh"))?;
                    for c in 0..3 {
                        for axis in 0..3 {
                            let d = (f64::from(got[c][axis]) - f64::from(t[c][axis])).abs();
                            let limit =
                                allowed(&e.built, key, axis) + 4e-6 * (f64::from(t[c][axis]).abs() + 1.0);
                            if d > limit {
                                return Err(format!(
                                    "row {i}: vertex {c} axis {axis} moved {d} (allowed {limit})"
                                )
                                .into());
                            }
                            worst = worst.max(d);
                        }
                    }
                    rows += 1;
                }
            }
        }
        let b0 = bounds_of(e.source.iter().flatten().copied());
        let b1 = bounds_of(mesh.triangles.iter().map(|t| t.vertices));
        // the per-triangle data of the shape, which the rebuild leaves alone
        let rows_before = shapes_before[e.shape_index]
            .field("pParam")?
            .dereference()?
            .field("triangleDataArray")?
            .array()?
            .len();
        let rows_after = shape
            .field("pParam")?
            .dereference()?
            .field("triangleDataArray")?
            .array()?
            .len();
        if rows_before != rows_after {
            return Err("pParam.triangleDataArray changed its length".into());
        }
        let sections = data.field("meshTree")?.field("sections")?.array()?.len();
        // the 2016.2 layout wraps the 16-bit value in a one-member class
        let runs = |v: Value<'_, '_>| -> Result<Vec<(u64, u64, u64)>> {
            let a = v
                .field("data")?
                .dereference()?
                .field("meshTree")?
                .field("primitiveDataRuns")?
                .array()?;
            (0..a.len())
                .map(|i| {
                    let run = a.get(i)?;
                    let value = run.field("value")?;
                    let value = if value.has_field("data") {
                        value.field("data")?
                    } else {
                        value
                    };
                    Ok((
                        value.unsigned()?,
                        run.field("index")?.unsigned()?,
                        run.field("count")?.unsigned()?,
                    ))
                })
                .collect()
        };
        println!(
            "  primitive data runs (value, index, count): stock {:x?}, rebuilt {:x?}",
            runs(shapes_before[e.shape_index])?,
            runs(shape)?
        );
        println!(
            "  stand point (shape-local), stock and rebuilt: {:.3?} / {:.3?}",
            stand_point(&compressed_mesh(
                shapes_before[e.shape_index].field("data")?.dereference()?
            )?),
            stand_point(&mesh)
        );
        let quads = want.iter().filter(|t| t.key & 1 == 1).count();
        println!(
            "rebuilt shape {}: {} triangles ({quads} quad halves) in {sections} sections, key bits {} -> {bits}; {rows} source rows keep their triangle (worst vertex shift {worst:.2e}), {same_key} of them under the stock key, {left_out} empty rows kept; pParam rows {rows_after}\n  box {:.4?}..{:.4?}\n  source box {:.4?}..{:.4?}",
            e.shape_index,
            mesh.triangles.len(),
            e.bits_before,
            b1[0],
            b1[1],
            b0[0],
            b0[1]
        );
    }
    Ok(())
}

/// How two byte strings differ: the bytes that differ, in runs, when they are the same length.
fn difference(a: &[u8], b: &[u8]) -> String {
    if a == b {
        return "identical to the stock bytes".to_owned();
    }
    if a.len() != b.len() {
        return "of another length than the stock file".to_owned();
    }
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        if x != y {
            match runs.last_mut() {
                Some(last) if last.1 + 1 >= i => last.1 = i,
                _ => runs.push((i, i)),
            }
        }
    }
    let bytes: usize = runs.iter().map(|r| r.1 - r.0 + 1).sum();
    format!(
        "{bytes} bytes differ from the stock file in {} runs (first {:?})",
        runs.len(),
        &runs[..runs.len().min(6)]
    )
}

fn write_out(out: &Path, label: &str, name: &str, bytes: &[u8]) -> Result<()> {
    let dir = out.join(label);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(name), bytes)?;
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: rebuild_collision scan <game> <folder> <keys> <oodle> <dictionary> [<filter>] | rebuild <game> <folder> <keys> <oodle> <archive path> <member filter> <out folder>";
    let [mode, game, root, keys, oodle, rest @ ..] = args.as_slice() else {
        return Err(usage.into());
    };
    let game = game_of(game)?;
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open(Some(Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let encoder = Encoder { oodle: Some(&oodle) };
    let archive = Install::new(root, game, Keys::from_folder(keys, game, None)?).open()?;
    match (mode.as_str(), rest) {
        ("scan", [dictionary]) => scan(game, &archive, &decoder, dictionary, ""),
        ("where", [dictionary, model]) => find_parts(game, &archive, &decoder, dictionary, model),
        ("scan", [dictionary, filter]) => scan(game, &archive, &decoder, dictionary, filter),
        ("rebuild", [path, filter, out]) => {
            let out = Path::new(out);
            let name = path.rsplit('/').next().unwrap_or(path);
            let stored = archive.read_stored(archive.hash(path))?;
            let envelope = Envelope::parse(&stored, 1 << 30)?;
            let outer = decoder.decode(&stored)?;
            println!(
                "{path}: envelope {:?}, {} bytes stored, {} decoded",
                envelope.format,
                stored.len(),
                outer.len()
            );
            let binder = Binder::read(&outer, &decoder)?;
            // the binder written as it is reads and writes to itself (checks the writer on this binder)
            let again = binder.write(&encoder)?;
            println!(
                "binder written again: {} bytes, {} the stock bytes",
                again.len(),
                if again == outer {
                    "equal to"
                } else {
                    "NOT equal to"
                }
            );
            let filter = filter.to_ascii_lowercase();
            let members: Vec<HavokMember> = havok_members(&binder)?
                .into_iter()
                .filter(|m| m.name.contains(&filter))
                .collect();
            let [member] = members.as_slice() else {
                return Err(format!(
                    "{} Havok members match {filter:?}: {:?}",
                    members.len(),
                    members.iter().map(|m| &m.name).collect::<Vec<_>>()
                )
                .into());
            };
            let original = binder.files[member.index].bytes.clone();
            println!(
                "member {} ({} bytes, compendium {})",
                member.name,
                original.len(),
                member.compendium.is_some()
            );
            let (control, rebuilt, expects) = rebuild(&binder, member)?;
            println!(
                "control tagfile {} the stock bytes",
                if control == original {
                    "equals"
                } else {
                    "DIFFERS from"
                }
            );
            if control != original {
                return Err("the unedited image does not write the stock bytes".into());
            }
            check(&rebuilt, member, &original, &expects)?;
            println!(
                "rebuilt tagfile: {} bytes (stock {}), {}",
                rebuilt.len(),
                original.len(),
                difference(&original, &rebuilt)
            );
            for (label, bytes) in [("control", &control), ("rebuilt", &rebuilt)] {
                let mut edited = binder.clone();
                edited.files[member.index].bytes.clone_from(bytes);
                let bnd = edited.write(&encoder)?;
                let framed = dcx::encode(&envelope.format, &bnd, &encoder)?;
                // the file reads back: envelope, binder, member
                let back = Binder::read(&decoder.decode(&framed)?, &decoder)?;
                if back.files[member.index].bytes != *bytes {
                    return Err(
                        format!("{label}: the member does not read back from the written binder").into(),
                    );
                }
                if back.files.len() != binder.files.len() {
                    return Err(format!("{label}: the member count changed").into());
                }
                write_out(out, label, name, &framed)?;
                println!(
                    "{label}: {name} written, {} bytes (stock {})",
                    framed.len(),
                    stored.len()
                );
            }
            Ok(())
        }
        _ => Err(usage.into()),
    }
}
