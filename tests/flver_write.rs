//! The FLVER writer: a model built here writes and reads back; the layout read from a file writes the model back to the
//! file; values that do not fit their encoding are refused; and every FLVER of both installs that the reader takes writes
//! back to its bytes.
//!
//! The real-data counts were measured with an independent Python implementation of the layout (a parser and a writer of
//! its own, with the vertex and index data kept as bytes, run over the same files): all of Sekiro's 14,064 distinct FLVERs
//! and Elden Ring's 20,557 are written back byte for byte, with the rules the module's note states.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`
//! for the real-data test; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::flver::{
    Bone, Channel, Dummy, FaceSet, GxItem, Layout, Material, Member, Mesh, Model, Texture, Tiling, Vertex,
};
use fromsoft_formats::{Binder, Game, NameDictionary, dcx::Decoder};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn member(offset: usize, kind: u32, semantic: u32) -> Member {
    Member {
        unknown: 0,
        stored: true,
        offset,
        kind,
        semantic,
        index: 0,
    }
}

/// The `string_bytes` word of a material: twice the UTF-16 units of its strings, terminators included.
fn string_bytes(m: &Material) -> u32 {
    let units = |s: &str| s.encode_utf16().count() as u32 + 1;
    2 * (units(&m.name)
        + units(&m.mtd)
        + m.textures
            .iter()
            .map(|t| units(&t.kind) + units(&t.path))
            .sum::<u32>())
}

fn vertex(position: [f32; 3], normal_w: i32) -> Vertex {
    Vertex {
        position,
        normal: [0.; 3],
        normal_w,
        bone_weights: [1., 0., 0., 0.],
        bone_indices: [0; 4],
        uvs: Channel::from([[0.5, 0.25, 0., 0.]]),
        ..Vertex::default()
    }
}

/// A one-bone, one-mesh model of version `0x2001a`: three vertices (a Float3 position, a UByte4 normal whose fourth byte
/// is `normal_w`, two int16 UV words, four normalised weight bytes and four bone index bytes), two face sets, one material
/// with two textures and a GX item, and a dummy.
fn model() -> Model {
    let mut material = Material {
        name: "mat".into(),
        mtd: "N:\\x\\m.mtd".into(),
        textures: vec![
            Texture {
                path: "N:\\x\\a.tga".into(),
                kind: "g_Diffuse".into(),
                scale: [1., 2.],
                tiling: [Tiling::Repeat, Tiling::Clamp],
                unknown: [0., 0.5, 1.],
            },
            Texture {
                path: String::new(),
                kind: "g_Bump".into(),
                scale: [1.; 2],
                tiling: [Tiling::None; 2],
                unknown: [0.; 3],
            },
        ],
        string_bytes: 0,
        index: 0,
        gx: vec![GxItem {
            id: *b"GX00",
            version: 102,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        }],
    };
    material.string_bytes = string_bytes(&material);
    Model {
        bones: vec![Bone {
            name: "root".into(),
            translation: [0.; 3],
            rotation: [0.; 3],
            scale: [1.; 3],
            parent: -1,
            child: -1,
            next_sibling: -1,
            previous_sibling: -1,
            bounds_min: [f32::MAX; 3],
            bounds_max: [-f32::MAX; 3],
            flags: 1,
        }],
        dummies: vec![Dummy {
            position: [1., 2., 3.],
            forward: [0., 0., 1.],
            upward: [0., 1., 0.],
            reference_id: 200,
            parent_bone: -1,
            attach_bone: 0,
            color: [255, 255, 255, 255],
            flag1: true,
            use_upward_vector: false,
            unk30: 0,
            unk34: 0,
        }],
        version: 0x2001a,
        speedtree: false,
        layouts: vec![vec![
            member(0, 2, 0),
            member(12, 0x11, 3),
            member(16, 0x15, 5),
            member(20, 0x13, 1),
            member(24, 0x11, 2),
        ]],
        materials: vec![material],
        meshes: vec![Mesh {
            dynamic: true,
            material: 0,
            node: 0,
            palette: vec![0],
            layouts: vec![0],
            vertices: vec![
                vertex([1., 2., 3.], 5),
                vertex([-1., 0.5, 4.], 6),
                vertex([0., 8., -2.], 7),
            ],
            faces: vec![
                FaceSet {
                    flags: 0,
                    strip: false,
                    cull: true,
                    unknown_06: 1,
                    indices: vec![0, 1, 2],
                },
                FaceSet {
                    flags: 0x0100_0000,
                    strip: false,
                    cull: true,
                    unknown_06: 1,
                    indices: vec![2, 1, 0],
                },
            ],
        }],
    }
}

#[test]
fn a_model_built_here_writes_and_reads_back() {
    let built = model();
    let layout = Layout::for_model(&built);
    let bytes = built.write(&layout).unwrap();
    let read = Model::read(&bytes).unwrap();
    assert_eq!(read.version, 0x2001a);
    assert_eq!(
        (
            read.bones.len(),
            read.dummies.len(),
            read.materials.len(),
            read.meshes.len()
        ),
        (1, 1, 1, 1)
    );
    assert_eq!(read.bones[0].name, "root");
    assert_eq!(read.dummies[0], built.dummies[0]);
    let (a, b) = (&built.materials[0], &read.materials[0]);
    assert_eq!(
        (&a.name, &a.mtd, a.string_bytes, a.index),
        (&b.name, &b.mtd, b.string_bytes, b.index)
    );
    assert_eq!(a.gx, b.gx);
    for (x, y) in a.textures.iter().zip(&b.textures) {
        assert_eq!(
            (&x.path, &x.kind, x.scale, x.tiling, x.unknown),
            (&y.path, &y.kind, y.scale, y.tiling, y.unknown)
        );
    }
    let (a, b) = (&built.meshes[0], &read.meshes[0]);
    assert_eq!(
        (a.dynamic, a.material, a.node, &a.palette, &a.layouts),
        (b.dynamic, b.material, b.node, &b.palette, &b.layouts)
    );
    for (x, y) in a.vertices.iter().zip(&b.vertices) {
        assert_eq!(
            (
                x.position,
                x.normal,
                x.normal_w,
                x.bone_weights,
                x.bone_indices,
                &x.uvs
            ),
            (
                y.position,
                y.normal,
                y.normal_w,
                y.bone_weights,
                y.bone_indices,
                &y.uvs
            )
        );
    }
    for (x, y) in a.faces.iter().zip(&b.faces) {
        assert_eq!(
            (x.flags, x.strip, x.cull, x.unknown_06, &x.indices),
            (y.flags, y.strip, y.cull, y.unknown_06, &y.indices)
        );
    }
    // the layout read from the bytes writes the model to the same bytes
    let stored = Layout::read(&bytes).unwrap();
    assert_eq!(read.write(&stored).unwrap(), bytes);
    // and says what the made layout does, but for the GX area it keeps
    assert_eq!(
        (
            stored.bounds,
            stored.face_counts,
            stored.header_5d,
            stored.header_68,
            &stored.mesh_bounds,
            &stored.face_set_widths
        ),
        (
            layout.bounds,
            layout.face_counts,
            layout.header_5d,
            layout.header_68,
            &layout.mesh_bounds,
            &layout.face_set_widths
        )
    );
    assert_eq!(stored.skeletons, layout.skeletons);
    assert!(stored.gx.is_some() && layout.gx.is_none());
}

#[test]
fn the_made_layout_measures_the_model() {
    let built = model();
    let layout = Layout::for_model(&built);
    assert_eq!(layout.bounds, [[-1., 0.5, -2.], [1., 8., 4.]]);
    // two triangle lists of one face each
    assert_eq!(layout.face_counts, [2, 2]);
    assert_eq!(layout.face_set_widths, [16, 16]);
    assert_eq!(
        (layout.index_width, layout.header_5d, layout.header_68),
        (0, 2, 4)
    );
    let boxes = layout.mesh_bounds[0].as_ref().unwrap();
    assert_eq!(boxes.len(), 9);
    assert_eq!(
        boxes[..6].iter().map(|w| f32::from_bits(*w)).collect::<Vec<_>>(),
        [-1., 0.5, -2., 1., 8., 4.]
    );
    let skeleton = layout.skeletons.as_ref().unwrap();
    assert_eq!(
        (skeleton.base.len(), skeleton.all.len(), skeleton.base[0].node),
        (1, 1, 0)
    );
    // an index above 16 bits asks for 32
    let mut wide = built.clone();
    wide.meshes[0].faces[0].indices = vec![0, 1, 70_000];
    assert_eq!(Layout::for_model(&wide).face_set_widths, [32, 16]);
}

#[test]
fn what_does_not_fit_is_refused() {
    let built = model();
    let layout = Layout::for_model(&built);
    let refused = |edit: &dyn Fn(&mut Model, &mut Layout)| {
        let (mut m, mut l) = (built.clone(), layout.clone());
        edit(&mut m, &mut l);
        m.write(&l).is_err()
    };
    assert!(!refused(&|_, _| {}), "the unedited model writes");
    // a normal component a signed byte cannot hold, and a bone index a byte cannot
    assert!(refused(&|m, _| m.meshes[0].vertices[0].normal = [1.5, 0., 0.]));
    assert!(refused(
        &|m, _| m.meshes[0].vertices[0].bone_indices = [300, 0, 0, 0]
    ));
    // an index a 16-bit face set cannot hold
    assert!(refused(&|m, _| m.meshes[0].faces[0].indices = vec![0, 1, 70_000]));
    // a layout of another model
    assert!(refused(&|_, l| {
        l.mesh_bounds.pop();
    }));
    assert!(refused(&|_, l| {
        l.face_set_widths.pop();
    }));
    assert!(refused(&|_, l| l.face_set_widths[0] = 8));
    // a vertex that lacks an attribute the layout stores
    assert!(refused(&|m, _| m.meshes[0].vertices[1].uvs = Channel::default()));
    // a string with a NUL in it, a material that does not exist, a version the writer does not know
    assert!(refused(&|m, _| m.materials[0].name = "a\0b".into()));
    assert!(refused(&|m, _| m.meshes[0].material = 3));
    assert!(refused(&|m, _| m.version = 0x20010));
}

#[test]
fn materials_that_share_a_list_share_it() {
    // two materials with one GX list between them write it once; a third with another list writes its own
    let mut built = model();
    built.materials.push(built.materials[0].clone());
    let mut third = built.materials[0].clone();
    third.gx[0].data = vec![9; 8];
    built.materials.push(third);
    built.meshes[0].material = 0;
    let layout = Layout::for_model(&built);
    let read = Model::read(&built.write(&layout).unwrap()).unwrap();
    assert_eq!(read.materials[0].gx, read.materials[1].gx);
    assert_ne!(read.materials[0].gx, read.materials[2].gx);
    let stored = Layout::read(&built.write(&layout).unwrap()).unwrap();
    let offsets = &stored.gx.as_ref().unwrap().offsets;
    assert_eq!(offsets[0], offsets[1]);
    assert_ne!(offsets[0], offsets[2]);
}

/// Every FLVER of the install that the reader takes, written back; returns (models, refused by the reader, bones, face
/// sets), or `None` when the game is not configured.
fn round_trips(game: Game) -> Result<Option<(usize, usize, usize, usize)>> {
    let (data, dll) = match game {
        Game::Sekiro => (inputs::Game::Sekiro, inputs::oodle(inputs::Tool::Oodle6)),
        Game::EldenRing => (inputs::Game::Elden, inputs::oodle(inputs::Tool::Oodle6)),
    };
    let (Some(archive), Some(dll), Some(names)) = (
        common::archive(game)?,
        dll,
        inputs::oracle(inputs::OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let names = NameDictionary::read_for(names, game)?;
    let mut seen = Totals::default();
    let mut failures = Vec::new();
    for (&hash, entry) in &archive.entries {
        let name = names.names.get(&hash).and_then(|x| x.first());
        if name.is_some_and(|n| n.ends_with("bdt")) {
            continue;
        }
        let id = format!("{}/{hash:016x}", entry.shard);
        let result = (|| -> Result<()> {
            let bytes = archive.read_hash(hash, &decoder)?;
            if bytes.starts_with(b"BHF4") {
                let name = name.ok_or("unnamed split binder")?;
                let payload = archive.read(&format!("{}bdt", &name[..name.len() - 3]), &decoder)?;
                for (i, f) in Binder::read_split(&bytes, &payload, &decoder)?
                    .files
                    .iter()
                    .enumerate()
                {
                    visit(
                        &f.bytes,
                        &format!("{id}/s{i}"),
                        &decoder,
                        &mut seen,
                        &mut failures,
                        0,
                    )?;
                }
            } else {
                visit(&bytes, &id, &decoder, &mut seen, &mut failures, 0)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            // an entry the container reader refuses has no FLVER to count
            println!("{id}: {e}");
        }
    }
    for f in failures.iter().take(40) {
        println!("FAIL {f}");
    }
    assert!(
        failures.is_empty(),
        "{} FLVERs do not write back to their bytes",
        failures.len()
    );
    Ok(Some((seen.models, seen.refused, seen.bones, seen.face_sets)))
}

#[derive(Default)]
struct Totals {
    models: usize,
    refused: usize,
    bones: usize,
    face_sets: usize,
}

fn visit(
    bytes: &[u8],
    id: &str,
    decoder: &Decoder<'_>,
    seen: &mut Totals,
    failures: &mut Vec<String>,
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        return Err("nested binder limit".into());
    }
    if bytes.starts_with(b"FLVER\0") {
        let Ok(model) = Model::read(bytes) else {
            seen.refused += 1;
            return Ok(());
        };
        seen.models += 1;
        seen.bones += model.bones.len();
        seen.face_sets += model.meshes.iter().map(|m| m.faces.len()).sum::<usize>();
        let layout = Layout::read(bytes).map_err(|e| format!("{id}: layout: {e}"))?;
        match model.write(&layout) {
            Ok(written) if written == bytes => {}
            Ok(_) => failures.push(format!("{id}: written differently")),
            Err(e) => failures.push(format!("{id}: {e}")),
        }
    } else if bytes.starts_with(b"BND4") {
        for (i, f) in Binder::read(bytes, decoder)?.files.iter().enumerate() {
            visit(
                &f.bytes,
                &format!("{id}/b{i}"),
                decoder,
                seen,
                failures,
                depth + 1,
            )?;
        }
    }
    Ok(())
}

#[test]
fn sekiros_flvers_write_back_to_their_bytes() -> Result<()> {
    let Some((models, refused, bones, face_sets)) = round_trips(Game::Sekiro)? else {
        return Ok(());
    };
    println!(
        "Sekiro: {models} FLVERs written back, {refused} refused by the reader, {bones} bones, {face_sets} face sets"
    );
    // 14,459 FLVERs (the container census), of which the reader takes all but the refused
    assert_eq!(models + refused, 14_459);
    // the bones and face sets of the census of the same models (tests/flver_census.rs)
    assert_eq!((bones, face_sets), (129_270, 110_365));
    Ok(())
}

#[test]
fn elden_rings_flvers_write_back_to_their_bytes() -> Result<()> {
    let Some((models, refused, bones, face_sets)) = round_trips(Game::EldenRing)? else {
        return Ok(());
    };
    println!(
        "Elden Ring: {models} FLVERs written back, {refused} refused by the reader, {bones} bones, {face_sets} face sets"
    );
    assert_eq!(models + refused, 22_169);
    assert_eq!((bones, face_sets), (535_503, 277_113));
    Ok(())
}
