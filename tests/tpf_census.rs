//! Whole-install PC texture-package differential against independent C# metadata.
mod common;
mod inputs;

use fromsoft_formats::{
    Binder, NameDictionary,
    dcx::Decoder,
    tpf::{Package, TextureKind},
};
use inputs::{Game, OracleSet, Tool, oodle, oracle, threads};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02X}")).collect()
}
fn visit(
    bytes: &[u8],
    id: &str,
    decoder: &Decoder<'_>,
    expected: &BTreeMap<String, serde_json::Value>,
    seen: &mut Vec<String>,
    counts: &mut (usize, usize),
    depth: usize,
) -> Result<()> {
    if depth > 8 {
        return Err("nested binder limit".into());
    }
    if bytes.starts_with(b"TPF\0") {
        let want = expected.get(id).ok_or_else(|| format!("unexpected TPF {id}"))?;
        seen.push(id.to_owned());
        assert_eq!(sha(bytes), want["sha256"].as_str().ok_or("sha field")?, "{id}");
        let package = Package::read(bytes)?;
        assert_eq!(
            package.flags,
            u8::try_from(want["flags"].as_u64().ok_or("flags")?)?
        );
        assert_eq!(
            package.encoding,
            want["encoding"].as_u64().ok_or("encoding")? as u8
        );
        let textures = want["textures"].as_array().ok_or("textures")?;
        assert_eq!(textures.len(), package.textures.len(), "{id}");
        for (t, w) in package.textures.iter().zip(textures) {
            let kind = match t.kind {
                TextureKind::Image2D => 0,
                TextureKind::Cube => 1,
                TextureKind::Volume => 2,
            };
            assert_eq!(t.name, w["name"].as_str().ok_or("name")?, "{id}");
            assert_eq!(
                (
                    t.format as u64,
                    kind,
                    t.mipmaps as u64,
                    t.flags as u64,
                    t.dds.len() as u64
                ),
                (
                    w["format"].as_u64().ok_or("format")?,
                    w["kind"].as_u64().ok_or("kind")?,
                    w["mipmaps"].as_u64().ok_or("mipmaps")?,
                    w["flags"].as_u64().ok_or("flags")?,
                    w["size"].as_u64().ok_or("size")?
                ),
                "{id}"
            );
            assert_eq!(sha(&t.dds), w["sha256"].as_str().ok_or("dds hash")?, "{id}");
            counts.1 += 1;
        }
        counts.0 += 1;
    } else if bytes.starts_with(b"BND4") {
        let b = Binder::read(bytes, decoder)?;
        for (i, f) in b.files.iter().enumerate() {
            visit(
                &f.bytes,
                &format!("{id}/b{i}"),
                decoder,
                expected,
                seen,
                counts,
                depth + 1,
            )?;
        }
    }
    Ok(())
}
#[test]
fn all_texture_packages() -> Result<()> {
    let Some(archive) = common::archive(fromsoft_formats::Game::Sekiro)? else {
        return Ok(());
    };
    let Some(oracle) = oracle(OracleSet::SoulsFormats, Game::Sekiro, "tpf") else {
        return Ok(());
    };
    let Some(names) = inputs::oracle(OracleSet::Uxm, Game::Sekiro, "dictionary.txt") else {
        return Ok(());
    };
    let Some(dll) = oodle(Tool::Oodle6) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 512 * 1024 * 1024,
    };
    let names = NameDictionary::read(names)?;
    let mut expected = BTreeMap::new();
    for line in std::fs::read_to_string(oracle.join("packages.jsonl"))?.lines() {
        let value: serde_json::Value = serde_json::from_str(line)?;
        expected.insert(value["id"].as_str().ok_or("id")?.to_owned(), value);
    }
    // shard, hash, the reference's message: the entries it cannot decode (the archive census's schema)
    let mut rejected = BTreeMap::new();
    for line in std::fs::read_to_string(oracle.join("errors.tsv"))?.lines() {
        let f: Vec<_> = line.splitn(3, '\t').collect();
        let [shard, hash, message] = f[..] else {
            return Err(format!("errors.tsv row: {line}").into());
        };
        rejected.insert(format!("{shard}/{hash}"), message.to_owned());
    }
    // each entry checked on the scan's threads against the reference (a split binder's index kept, with the path of its
    // payload, for the second pass); the results merged in hash order
    struct Checked {
        seen: Vec<String>,
        counts: (usize, usize),
        split: bool,
        refused: Option<String>,
        index: Option<(String, Vec<u8>)>,
    }
    let mut checked = common::scan_map(
        &archive,
        &decoder,
        |hash| {
            !names
                .names
                .get(&hash)
                .and_then(|x| x.first())
                .is_some_and(|n| n.ends_with("bdt"))
        },
        |hash, bytes| {
            let mut c = Checked {
                seen: Vec::new(),
                counts: (0, 0),
                split: false,
                refused: None,
                index: None,
            };
            let id = format!("{}/{hash:08x}", archive.entries[&hash].shard);
            let result = (|| -> Result<()> {
                let bytes = bytes?;
                if bytes.starts_with(b"BHF4") {
                    let name = names
                        .names
                        .get(&hash)
                        .and_then(|x| x.first())
                        .ok_or("unnamed split binder")?;
                    c.index = Some((format!("{}bdt", &name[..name.len() - 3]), bytes.to_vec()));
                } else {
                    visit(bytes, &id, &decoder, &expected, &mut c.seen, &mut c.counts, 0)?;
                }
                Ok(())
            })();
            c.refused = result.err().map(|e| e.to_string());
            (id, c)
        },
    )?;
    // the split binders: each payload read in one more pass and read with the index the scan kept, its members checked
    let split: Vec<(u64, String, String, Vec<u8>)> = checked
        .iter_mut()
        .filter_map(|(&hash, (id, c))| {
            c.index
                .take()
                .map(|(payload, index)| (hash, id.clone(), payload, index))
        })
        .collect();
    let by_payload: BTreeMap<&str, (&str, &[u8])> = split
        .iter()
        .map(|(_, id, payload, index)| (payload.as_str(), (id.as_str(), index.as_slice())))
        .collect();
    let members = archive.read_each(
        split.iter().map(|(_, _, payload, _)| payload),
        &decoder,
        threads(),
        |payload, bytes| {
            let (id, index) = by_payload[payload];
            let mut c = Checked {
                seen: Vec::new(),
                counts: (0, 0),
                split: false,
                refused: None,
                index: None,
            };
            let result = (|| -> Result<()> {
                let binder = Binder::read_split(index, bytes?, &decoder)?;
                for (i, f) in binder.files.iter().enumerate() {
                    visit(
                        &f.bytes,
                        &format!("{id}/s{i}"),
                        &decoder,
                        &expected,
                        &mut c.seen,
                        &mut c.counts,
                        0,
                    )?;
                }
                c.split = true;
                Ok(())
            })();
            c.refused = result.err().map(|e| e.to_string());
            c
        },
    )?;
    for ((hash, ..), c) in split.iter().zip(members) {
        checked
            .get_mut(hash)
            .ok_or("a split binder the scan did not give")?
            .1 = c;
    }
    let mut counts = (0, 0);
    let mut failures = Vec::new();
    let mut split = 0;
    for (id, c) in checked.into_values() {
        for seen in c.seen {
            if expected.remove(&seen).is_none() {
                failures.push(format!("{seen}: checked twice"));
            }
        }
        counts = (counts.0 + c.counts.0, counts.1 + c.counts.1);
        split += usize::from(c.split);
        if let Some(e) = c.refused {
            // both decoders reject it: agreement on the entry, the reasons reported side by side
            if let Some(theirs) = rejected.remove(&id) {
                println!("Agreed rejection {id}: ours {e}; reference {theirs}");
            } else {
                failures.push(format!("{id}: {e}"));
            }
        }
    }
    println!(
        "TPF packages {}, textures {}, split binders {}, remaining oracle {}, failures {}",
        counts.0,
        counts.1,
        split,
        expected.len(),
        failures.len()
    );
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty());
    assert!(expected.is_empty());
    assert!(rejected.is_empty(), "reference rejection not reproduced");
    Ok(())
}
