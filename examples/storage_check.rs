//! Reads every FLVER of the configured archives with `flver::Storage` and `flver::Model` and checks that they agree:
//! each mesh's face sets hold as many indices as the storage says, and its vertex buffers as many vertices.
//! cargo run --release --example storage_check -- <game folder> <keys folder> <oodle library> <dictionary.txt>
#![allow(unsafe_code, clippy::unwrap_used)]
use fromsoft_formats::flver::{Model, Storage};
use fromsoft_formats::{Archive, Binder, Game, Keys, NameDictionary, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::path::Path;

fn walk(bytes: &[u8], d: &Decoder<'_>, f: &mut dyn FnMut(&[u8])) {
    if bytes.starts_with(b"BND4") {
        for file in Binder::read(bytes, d).unwrap().files {
            walk(&file.bytes, d, f);
        }
    } else if bytes.starts_with(b"FLVER\0L\0") {
        f(bytes);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let archive = Archive::open_game(
        &a[0],
        Game::Sekiro,
        &Keys::from_folder(&a[1], Game::Sekiro, None).unwrap(),
    )
    .unwrap();
    let names = NameDictionary::read(&a[3]).unwrap();
    // SAFETY: the game's own library, named by the caller.
    let oodle = unsafe { Oodle::open(Some(Path::new(&a[2])), Version::V6) }.unwrap();
    let d = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let (mut models, mut refused, mut disagree) = (0, 0, 0);
    for (hash, paths) in &names.names {
        if !archive.entries.contains_key(hash)
            || !paths.iter().any(|p| {
                [".mapbnd.dcx", ".objbnd.dcx", ".chrbnd.dcx", ".partsbnd.dcx"]
                    .iter()
                    .any(|s| p.ends_with(s))
            })
        {
            continue;
        }
        let Ok(bytes) = archive.read_hash(*hash, &d) else {
            continue;
        };
        walk(&bytes, &d, &mut |flver| {
            models += 1;
            let (Ok(s), Ok(m)) = (Storage::read(flver), Model::read(flver)) else {
                refused += 1;
                return;
            };
            let ok = s.meshes.len() == m.meshes.len()
                && m.meshes.iter().zip(&s.meshes).all(|(mesh, b)| {
                    mesh.faces
                        .iter()
                        .zip(&b.face_sets)
                        .all(|(f, &i)| f.indices.len() == s.face_sets[i].count)
                        && b.vertex_buffers
                            .iter()
                            .all(|&i| s.vertex_buffers[i].count == mesh.vertices.len())
                });
            if !ok {
                disagree += 1;
            }
        });
    }
    println!("{models} models, {refused} refused by either reader, {disagree} where the two disagree");
}
