//! Lists the keys of the vertex and index bytes of every FLVER in the binders of an installed game's archives whose paths
//! hold one of the filters (binders only: paths ending in `bnd.dcx`), so a frame captured from the running game can be tied to the files its meshes came from.
//!
//! ```text
//! cargo run --release --example flver_keys -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>... > keys.tsv
//! ```
//!
//! The first arguments are `list_binder`'s; the dictionary is a file of archive paths, one per line (archives store only
//! their hashes). Each line written is `key<TAB>bytes<TAB>name`: the key is XXH3-128 as 32 lowercase hex digits (the
//! key locum names captured bytes by; `locum-cli index --keys keys.tsv` reads the list), and the name is
//! `<binder path>/<member>` and what the bytes are: `#block` (the data block, which Sekiro uploads as one buffer),
//! `#vb<n>` (a vertex buffer, in file order) or `#faces<n>` (a face set's indices). See [`fromsoft_formats::flver::Storage`].
//!
//! A vertex buffer's or face set's line has a fourth column, a JSON object of the mesh that draws it: `mesh` (its index
//! in the FLVER), `material`, `mtd` (the material definition it names, as stored) and `textures` (each binding's `slot`
//! and `path`, as the FLVER stores them), and `definition`, what the game's own definition of that name holds: an Elden
//! Ring MATBIN's `shader`, `source`, `params` (by name) and `samplers` (`kind`, `path`), or a Sekiro MTD's `shader`,
//! `params` and `textures` (`kind`, `uv`, `path`). `locum-cli draw` and `bindings --json` show it with the file's name,
//! so a draw's buffers name the material and its authored values to set beside the constants the draw was given.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::flver::{MeshBytes, Model, Storage};
use fromsoft_formats::names::stem;
use fromsoft_formats::{Archive, Binder, Game, Install, Keys, NameDictionary, dcx::Decoder, matbin, mtd};
use primase_foundations::compress::oodle::{Oodle, Version};
use serde_json::{Map, Number, Value as Json, json};
use std::collections::HashMap;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;
use xxhash_rust::xxh3::xxh3_128;

/// A float as JSON, in its shortest form (`0.3`, not the `0.30000001192092896` of its `f64`).
fn num(v: f32) -> Json {
    format!("{v}")
        .parse::<f64>()
        .ok()
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number)
}

fn nums(v: &[f32]) -> Json {
    Json::Array(v.iter().copied().map(num).collect())
}

/// A MATBIN parameter's value as JSON: a bool, a number or a list of them (a `Float3`'s two stored extra floats left
/// out; a truncated value is null).
fn matbin_value(v: &matbin::Value) -> Json {
    match v {
        matbin::Value::Bool(b) => json!(b),
        matbin::Value::Int(i) => json!(i),
        matbin::Value::Int2(i) => json!(i),
        matbin::Value::Float(f) => num(*f),
        matbin::Value::Float2(f) => nums(f),
        matbin::Value::Float3(f, _) => nums(f),
        matbin::Value::Float4(f) => nums(f),
        matbin::Value::Float5(f) => nums(f),
        matbin::Value::Truncated { .. } => Json::Null,
    }
}

/// An MTD parameter's value as JSON: one number where it holds one.
fn mtd_value(v: &mtd::Value) -> Json {
    match v {
        mtd::Value::Bool(b) => json!(b),
        mtd::Value::Int(i) if i.len() == 1 => json!(i[0]),
        mtd::Value::Int(i) => json!(i),
        mtd::Value::Float(f) if f.len() == 1 => num(f[0]),
        mtd::Value::Float(f) => nums(f),
    }
}

/// The game's material definitions by the stem of their names ([`stem`]: what a FLVER material's `mtd` names): Elden
/// Ring's MATBINs, Sekiro's MTDs, each as JSON; the first binder holding a name gives it.
fn definitions(game: Game, archive: &Archive, decoder: &Decoder<'_>) -> HashMap<String, Json> {
    let mut out = HashMap::new();
    let binders = game
        .material_binders()
        .iter()
        .map(|p| (*p, true))
        .chain(game.mtd_binders().iter().map(|p| (*p, false)));
    for (path, binary) in binders {
        if !archive.contains(path) {
            continue;
        }
        let binder = match archive
            .read(path, decoder)
            .and_then(|b| Binder::read(&b, decoder))
        {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let (mut read, mut failed) = (0usize, 0usize);
        for file in &binder.files {
            let Some(name) = file.name.as_deref() else {
                continue;
            };
            let definition = if binary {
                matbin::Material::read(&file.bytes).map(|m| {
                    json!({
                        "shader": m.shader,
                        "source": m.source,
                        "params": m.params.iter().map(|p| (p.name.clone(), matbin_value(&p.value))).collect::<Map<_, _>>(),
                        "samplers": m.samplers.iter().map(|s| json!({"kind": s.kind, "path": s.path})).collect::<Vec<_>>(),
                    })
                })
            } else {
                mtd::MaterialDefinition::read(&file.bytes).map(|m| {
                    json!({
                        "shader": m.shader,
                        "params": m.parameters.iter().map(|p| (p.name.clone(), mtd_value(&p.value))).collect::<Map<_, _>>(),
                        "textures": m.textures.iter().map(|t| json!({"kind": t.kind, "uv": t.uv_number, "path": t.path})).collect::<Vec<_>>(),
                    })
                })
            };
            match definition {
                Ok(d) => {
                    out.entry(stem(name)).or_insert(d);
                    read += 1;
                }
                Err(e) => {
                    eprintln!("{path}/{name}: {e}");
                    failed += 1;
                }
            }
        }
        eprintln!("{path}: {read} material definitions read, {failed} refused");
    }
    out
}

/// The lines of one binder's FLVERs, and its error if it could not be read.
fn lines(
    path: &str,
    binder: fromsoft_formats::Result<Binder>,
    decoder: &Decoder<'_>,
    definitions: &HashMap<String, Json>,
) -> Result<Vec<String>, String> {
    let binder = binder.map_err(|e| format!("{path}: {e}"))?;
    let mut out = Vec::new();
    for file in &binder.files {
        let Some(name) = file.name.as_deref() else {
            continue;
        };
        let member = name.rsplit(['\\', '/']).next().unwrap_or(name);
        let lower = member.to_ascii_lowercase();
        if !(lower.ends_with(".flver") || lower.ends_with(".flver.dcx")) {
            continue;
        }
        let bytes = decoder
            .decode_if_dcx(&file.bytes)
            .map_err(|e| format!("{path}/{member}: {e}"))?;
        let storage = Storage::read(&bytes).map_err(|e| format!("{path}/{member}: {e}"))?;
        let block = storage
            .data_block(&bytes)
            .ok_or_else(|| format!("{path}/{member}: the data block is past the file's end"))?;
        // Each mesh's material, as the line of each buffer and face set it draws says.
        let abouts: Vec<String> = match Model::read(&bytes) {
            Ok(model) => model
                .meshes
                .iter()
                .enumerate()
                .map(|(i, mesh)| {
                    let Some(m) = model.materials.get(mesh.material) else {
                        return json!({"mesh": i}).to_string();
                    };
                    let mut about = json!({
                        "mesh": i,
                        "material": m.name,
                        "mtd": m.mtd,
                        "textures": m.textures.iter().map(|t| json!({"slot": t.kind, "path": t.path})).collect::<Vec<_>>(),
                    });
                    if let Some(d) = definitions.get(&stem(&m.mtd)) {
                        about["definition"] = d.clone();
                    }
                    about.to_string()
                })
                .collect(),
            Err(e) => {
                eprintln!("{path}/{member}: its materials: {e}");
                Vec::new()
            }
        };
        let mesh_of = |part: &dyn Fn(&MeshBytes) -> bool| {
            storage.meshes.iter().position(part).and_then(|m| abouts.get(m))
        };
        let mut push = |what: String, part: Option<&[u8]>, about: Option<&String>| match part {
            Some(b) => out.push(match about {
                Some(about) => format!(
                    "{:032x}\t{}\t{path}/{member}#{what}\t{about}",
                    xxh3_128(b),
                    b.len()
                ),
                None => format!("{:032x}\t{}\t{path}/{member}#{what}", xxh3_128(b), b.len()),
            }),
            None => eprintln!("{path}/{member}#{what}: past the data block's end"),
        };
        push("block".into(), Some(block), None);
        for (i, vb) in storage.vertex_buffers.iter().enumerate() {
            let len = vb.stride.checked_mul(vb.count);
            push(
                format!("vb{i}"),
                len.and_then(|l| block.get(vb.offset..vb.offset.checked_add(l)?)),
                mesh_of(&|m| m.vertex_buffers.contains(&i)),
            );
        }
        for (i, f) in storage.face_sets.iter().enumerate() {
            let len = f.count.checked_mul(f.width as usize / 8);
            push(
                format!("faces{i}"),
                len.and_then(|l| block.get(f.offset..f.offset.checked_add(l)?)),
                mesh_of(&|m| m.face_sets.contains(&i)),
            );
        }
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, dictionary, filters @ ..] = args.as_slice() else {
        return Err("usage: flver_keys <sekiro|elden> <game folder> <keys folder> <oodle library | -> <dictionary> <filter>...".into());
    };
    if filters.is_empty() {
        return Err(
            "give at least one filter (a part of the archive paths to read, like /asset/aeg/aeg096/)".into(),
        );
    }
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let keys = Keys::from_folder(keys, game, None)?;
    let archive = Install::new(root, game, keys).open()?;
    let names = NameDictionary::read_for(dictionary, game)?;
    let named: Vec<&str> = names
        .names
        .values()
        .flatten()
        .map(String::as_str)
        .filter(|p| filters.iter().any(|f| p.contains(f.as_str())))
        // binders (`.geombnd.dcx`, `.partsbnd.dcx`, `.chrbnd.dcx`, `.mapbnd.dcx`); textures, navmeshes and the rest hold no FLVER
        .filter(|p| p.ends_with("bnd.dcx"))
        .collect();
    let paths: Vec<&str> = named.iter().copied().filter(|p| archive.contains(p)).collect();
    let definitions = definitions(game, &archive, &decoder);
    eprintln!(
        "{} binder paths of the dictionary match; {} of them are in the archives",
        named.len(),
        paths.len()
    );
    let threads = std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN);
    let results = archive
        .read_each_binder(&paths, &decoder, threads, |path, binder| {
            lines(path, binder, &decoder, &definitions)
        })
        .map_err(|e| format!("{e:?}"))?;
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    let (mut written, mut failed) = (0usize, 0usize);
    for r in results {
        match r {
            Ok(lines) => {
                for l in lines {
                    writeln!(out, "{l}")?;
                    written += 1;
                }
            }
            Err(e) => {
                eprintln!("{e}");
                failed += 1;
            }
        }
    }
    out.flush()?;
    eprintln!("{written} keys written; {failed} binders could not be read");
    Ok(())
}
