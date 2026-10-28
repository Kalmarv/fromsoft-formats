//! A timed read of an install's archives, stage by stage, with a digest of everything it decoded: the benchmark for the
//! read path (index unwrap, BDT read and decryption, DCX decoding, binder splitting, FLVER parsing). Run before and after
//! a change to the read path: the times compare, the digest must not change.
//!
//! ```text
//! cargo run --release --example read_bench -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> \
//!     [--threads N] [--sample K] [--max-bytes N] [--no-parse] [--only HASH]
//! ```
//!
//! Every `K`th entry (hash order, default 1: all of them) whose stored size is at most `--max-bytes` (default 256 MiB) is
//! read as stored, decoded when it is a DCX, split when it is a BND4 and, for each member that is a FLVER, parsed. The
//! digest folds in each entry's hash and decoded bytes and every parsed model's vertex data. `--no-parse` stops after
//! decoding. The numbers are for one process on a warm file cache: state which when comparing.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    reason = "a benchmark program"
)]

use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder, flver::Model};
use primase_foundations::compress::oodle::{Oodle, Version};
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Stage {
    time: Duration,
    count: u64,
    bytes: u64,
}
impl Stage {
    fn add(&mut self, since: Instant, bytes: usize) {
        self.time += since.elapsed();
        self.count += 1;
        self.bytes += bytes as u64;
    }
    fn show(&self, name: &str) {
        let mb = self.bytes as f64 / 1_048_576.0;
        let s = self.time.as_secs_f64();
        println!(
            "{name:<28} {s:9.3} s {:>9} items {mb:10.1} MiB {:8.1} MiB/s",
            self.count,
            mb / s.max(1e-9)
        );
    }
}

/// How many vertices store each number of values in each variable-length channel (lengths of 11 and more share a slot).
#[derive(Default)]
struct Census {
    channels: [[u64; 12]; 6],
    vertices: u64,
}
impl Census {
    const NAMES: [&'static str; 6] = [
        "uvs",
        "tangents",
        "tangent_indices",
        "colors",
        "layer_bytes",
        "speedtree",
    ];
    fn add(&mut self, model: &Model) {
        for v in model.meshes.iter().flat_map(|m| &m.vertices) {
            self.vertices += 1;
            let lens = [
                v.uvs.len(),
                v.tangents.len(),
                v.tangent_indices.len(),
                v.colors.len(),
                v.layer_bytes.len(),
                v.speedtree.len(),
            ];
            for (channel, len) in self.channels.iter_mut().zip(lens) {
                channel[len.min(11)] += 1;
            }
        }
    }
    fn show(&self) {
        println!(
            "vertices {}; per channel, how many vertices store 0, 1, 2 ... values (last slot: 11 or more)",
            self.vertices
        );
        for (name, c) in Self::NAMES.iter().zip(&self.channels) {
            println!("  {name:<16} {c:?}");
        }
    }
}

/// Folds every vertex field of `model` into `hasher`, as numbers (the same bytes whatever the fields' storage).
fn fold_model(hasher: &mut Sha256, model: &Model) {
    let mut put = |bits: &[u32]| {
        for b in bits {
            hasher.update(b.to_le_bytes());
        }
    };
    put(&[
        u32::try_from(model.meshes.len()).unwrap(),
        u32::try_from(model.bones.len()).unwrap(),
    ]);
    for mesh in &model.meshes {
        put(&[
            u32::try_from(mesh.vertices.len()).unwrap(),
            u32::try_from(mesh.faces.len()).unwrap(),
        ]);
        for v in &mesh.vertices {
            let f = |a: &[f32]| a.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
            put(&f(&v.position));
            put(&f(&v.normal));
            put(&[v.normal_w as u32]);
            put(&f(&v.bone_weights));
            put(&v.bone_indices.map(u32::from));
            put(&f(&v.bitangent));
            for (name, channel) in [
                (1u32, &v.uvs[..]),
                (2, &v.tangents[..]),
                (3, &v.colors[..]),
                (4, &v.speedtree[..]),
            ] {
                put(&[name, u32::try_from(channel.len()).unwrap()]);
                for c in channel {
                    put(&f(c));
                }
            }
            put(&[5, u32::try_from(v.layer_bytes.len()).unwrap()]);
            for (at, bytes) in &v.layer_bytes {
                put(&[u32::try_from(*at).unwrap(), u32::from_le_bytes(*bytes)]);
            }
            put(&[6, u32::try_from(v.tangent_indices.len()).unwrap()]);
            put(&v
                .tangent_indices
                .iter()
                .map(|&i| u32::from(i))
                .collect::<Vec<_>>());
        }
        for face in &mesh.faces {
            put(&[u32::try_from(face.indices.len()).unwrap()]);
            put(&face.indices);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (game, root, keys, oodle) = (args.next(), args.next(), args.next(), args.next());
    let (Some(game), Some(root), Some(keys), Some(oodle)) = (game, root, keys, oodle) else {
        return Err("usage: read_bench <sekiro|elden> <game folder> <keys folder> <oodle library | -> [--threads N] [--sample K] [--max-bytes N] [--no-parse]".into());
    };
    let (mut threads, mut sample, mut max_bytes, mut parse) = (1usize, 1usize, 256usize << 20, true);
    let mut only: Option<u64> = None;
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--threads" => threads = value()?.parse()?,
            "--sample" => sample = value()?.parse::<usize>()?.max(1),
            "--only" => only = Some(u64::from_str_radix(value()?.trim_start_matches("0x"), 16)?),
            "--max-bytes" => max_bytes = value()?.parse()?,
            "--no-parse" => parse = false,
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };
    let keys = Keys::from_folder(&keys, game, None)?;
    let t = Instant::now();
    let archive = Install::new(&root, game, keys)
        .with_threads(NonZeroUsize::new(threads).ok_or("threads")?)
        .open()?;
    let open = t.elapsed();
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(&oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let (mut read, mut decode, mut split, mut flver) = (
        Stage::default(),
        Stage::default(),
        Stage::default(),
        Stage::default(),
    );
    let mut hasher = Sha256::new();
    let mut census = Census::default();
    let (mut refused_decode, mut refused_split, mut refused_flver) = (0u64, 0u64, 0u64);
    let started = Instant::now();
    let mut biggest: Vec<(usize, u64)> = Vec::new();
    for (&hash, indexed) in archive.entries.iter().step_by(sample) {
        if indexed.entry.padded_size > max_bytes || only.is_some_and(|h| h != hash) {
            continue;
        }
        let t = Instant::now();
        let stored = archive.read_stored(hash)?;
        read.add(t, stored.len());
        let bytes = if stored.starts_with(b"DCX\0") {
            let t = Instant::now();
            match decoder.decode(&stored) {
                Ok(b) => {
                    decode.add(t, b.len());
                    b
                }
                Err(_) => {
                    refused_decode += 1;
                    continue;
                }
            }
        } else {
            stored
        };
        hasher.update(hash.to_le_bytes());
        hasher.update(Sha256::digest(&bytes));
        if !parse {
            continue;
        }
        if bytes.starts_with(b"BND4") {
            let t = Instant::now();
            match Binder::read(&bytes, &decoder) {
                Ok(binder) => {
                    split.add(t, bytes.len());
                    for file in &binder.files {
                        if file.bytes.starts_with(b"FLVER\0") {
                            let t = Instant::now();
                            match Model::read(&file.bytes) {
                                Ok(model) => {
                                    flver.add(t, file.bytes.len());
                                    {
                                        fold_model(&mut hasher, &model);
                                        census.add(&model);
                                        biggest.push((
                                            model.meshes.iter().map(|m| m.vertices.len()).sum(),
                                            hash,
                                        ));
                                    }
                                }
                                Err(_) => refused_flver += 1,
                            }
                        }
                    }
                }
                Err(_) => refused_split += 1,
            }
        } else if bytes.starts_with(b"FLVER\0") {
            let t = Instant::now();
            match Model::read(&bytes) {
                Ok(model) => {
                    flver.add(t, bytes.len());
                    {
                        fold_model(&mut hasher, &model);
                        census.add(&model);
                        biggest.push((model.meshes.iter().map(|m| m.vertices.len()).sum(), hash));
                    }
                }
                Err(_) => refused_flver += 1,
            }
        }
    }
    let total = started.elapsed();
    println!(
        "game {game:?}, {} entries, sample 1/{sample}, <= {max_bytes} bytes, {threads} thread(s)",
        archive.entries.len()
    );
    println!("{:<28} {:9.3} s", "open (index unwrap)", open.as_secs_f64());
    read.show("read + decrypt (BDT)");
    decode.show("DCX decode (output bytes)");
    split.show("binder split (+nested decode)");
    flver.show("FLVER parse");
    println!("{:<28} {:9.3} s", "entries loop total", total.as_secs_f64());
    biggest.sort_unstable_by(|a, b| b.cmp(a));
    biggest.truncate(3);
    for (vertices, hash) in &biggest {
        println!("a model of {vertices} vertices in the entry {hash:#x} (--only {hash:x})");
    }
    census.show();
    println!("refused: decode {refused_decode}, binder {refused_split}, flver {refused_flver}");
    println!(
        "digest {}",
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    Ok(())
}
