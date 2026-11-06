//! Every metaparam of the install's shader bundles (`/shader/shaderbdle.shaderbdlebnd.dcx`, 248 bundles) reads, with
//! the counts of what they declare, from the install and from the bundles cut from it (`corpus.sekiro/shader-binders`):
//! 232 of version 6 and 16 of version 4 (the old lightmap shaders and `sfx_AMSN_cloth`), each lightmap one its
//! version-6 sibling but for the version and a zero shader ID. The 232 version-6 files were compared field by field with the third-party SoulsFormatsNEXT reader
//! (`METAPARAM`, built from its source; it takes Sekiro's colour parameters, type 12 where it says 13, only with that
//! one value allowed): textures, render passes, CAParams and MtdParams, no difference. Each program's extension agrees
//! with the shader type of its DXBC container, counted by stage.
//!
//! Needs `installs.sekiro` and `tools.oodle6`, or `corpus.sekiro/shader-binders`.
#![allow(clippy::unwrap_used)]
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::metaparam::Metaparam;
use fromsoft_formats::{Archive, Binder};
use inputs::{Game, Tool, install, oodle};
use std::collections::{BTreeMap, BTreeSet};
mod common;

#[test]
fn every_metaparam_of_the_install() {
    let (Some(root), Some(dll)) = (install(Game::Sekiro), oodle(Tool::Oodle6)) else {
        return;
    };
    let dll = common::oodle(dll).unwrap();
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1024 * 1024 * 1024,
    };
    let Some(sekiro_keys) = inputs::keys(inputs::Game::Sekiro) else {
        return;
    };
    let archive = Archive::open_game(
        root,
        fromsoft_formats::Game::Sekiro,
        &fromsoft_formats::Keys::from_folder(sekiro_keys, fromsoft_formats::Game::Sekiro, None).unwrap(),
    )
    .unwrap();
    let outer = Binder::read(
        &archive
            .read("/shader/shaderbdle.shaderbdlebnd.dcx", &decoder)
            .unwrap(),
        &decoder,
    )
    .unwrap();
    assert_eq!(outer.files.len(), 248);
    let mut versions = BTreeMap::new();
    let mut declared = [0usize; 4];
    let mut stages: BTreeMap<(String, u32), usize> = BTreeMap::new();
    for bundle in &outer.files {
        for f in Binder::read(&bundle.bytes, &decoder).unwrap().files {
            let name = f.name.as_deref().unwrap();
            let extension = name.rsplit_once('.').unwrap().1.to_owned();
            if extension == "metaparam" {
                let m = Metaparam::read(&f.bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
                *versions.entry(m.version).or_insert(0) += 1;
                add(&mut declared, &m);
                continue;
            }
            let chunks = dxcontainer::chunks(&f.bytes).unwrap();
            let code = chunks
                .iter()
                .find(|c| &c.kind == b"SHEX" || &c.kind == b"SHDR")
                .unwrap();
            let shader_type = u32::from_le_bytes(code.data[..4].try_into().unwrap()) >> 16;
            *stages.entry((extension, shader_type)).or_default() += 1;
        }
    }
    assert_eq!(versions, BTreeMap::from([(4, 16), (6, 232)]));
    assert_eq!(declared, DECLARED);
    // .ppo pixel (0), .vpo vertex (1), .hpo hull (3), .dpo domain (4)
    let stages: Vec<_> = stages.into_iter().collect();
    assert_eq!(
        stages,
        [
            (("dpo".into(), 4), 37),
            (("hpo".into(), 3), 37),
            (("ppo".into(), 0), 10_607),
            (("vpo".into(), 1), 5_747)
        ]
    );
}

/// The textures, render passes, CAParams and MtdParams of the 248 bundles' metaparams: 1,352, 1,160, 773 and 1,535 of
/// the 232 of version 6 (counted before version 4 was read), the rest version 4's.
const DECLARED: [usize; 4] = [1352 + 201, 1160 + 80, 773 + 54, 1535 + 1];

fn add(declared: &mut [usize; 4], m: &Metaparam) {
    declared[0] += m.textures.len();
    declared[1] += m.passes.len();
    declared[2] += m.ca_params.len();
    declared[3] += m.mtd_params.len();
}

#[test]
fn every_metaparam_of_the_bundles_cut_from_the_install() {
    let Some(dir) = inputs::oracle(inputs::OracleSet::Corpus, inputs::Game::Sekiro, "shader-binders") else {
        return;
    };
    let dir = dir.join("shaderbdle");
    let mut bundles: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    bundles.sort();
    assert_eq!(bundles.len(), 248);
    let read = |name: &str| {
        let bytes = std::fs::read(dir.join(name).join(format!("{name}.metaparam"))).unwrap();
        let m = Metaparam::read(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        (bytes, m)
    };
    let mut declared = [[0usize; 4]; 2];
    let mut old = Vec::new();
    for name in &bundles {
        let (bytes, m) = read(name);
        match m.version {
            6 => add(&mut declared[1], &m),
            4 => {
                add(&mut declared[0], &m);
                assert_eq!(m.shader_id, 0, "{name}");
                old.push(name.clone());
                // the lightmap shaders are their version-6 siblings with neither version nor shader ID
                if let Some(sibling) = name.strip_suffix("_lightmap") {
                    let (sibling_bytes, six) = read(sibling);
                    assert_eq!(six.version, 6, "{sibling}");
                    assert_eq!(
                        Metaparam {
                            version: 6,
                            shader_id: six.shader_id,
                            ..m
                        },
                        six,
                        "{name}"
                    );
                    let differ: Vec<usize> = (0..bytes.len().max(sibling_bytes.len()))
                        .filter(|&i| bytes.get(i) != sibling_bytes.get(i))
                        .collect();
                    assert!(
                        differ.iter().all(|&i| i == 8 || (0x58..0x5c).contains(&i)),
                        "{name}: {differ:x?}"
                    );
                }
            }
            v => panic!("{name}: version {v}"),
        }
    }
    let lightmap = old.iter().filter(|n| n.ends_with("_lightmap")).count();
    assert_eq!((old.len(), lightmap), (16, 15), "{old:?}");
    assert!(old.contains(&"sfx_AMSN_cloth".to_owned()));
    // version 6 as counted from the install itself, before version 4 was read
    assert_eq!(declared[1], [1352, 1160, 773, 1535]);
    let total: Vec<usize> = (0..4).map(|i| declared[0][i] + declared[1][i]).collect();
    assert_eq!(total, DECLARED, "version 4: {:?}", declared[0]);
}

/// A JSON value with every negative integer taken as its 32-bit word: the reference writes some words signed.
fn words(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Number(n) => match n.as_i64() {
            Some(i) if i < 0 => Value::from(i + (1i64 << 32)),
            _ => Value::Number(n),
        },
        Value::Array(a) => Value::Array(a.into_iter().map(words).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, words(v))).collect()),
        other => other,
    }
}

/// Elden Ring's metaparams (every bundle of its four bundle binders, 1,063) field by field against SoulsFormatsNEXT
/// ee1dd61's `METAPARAM` (`metaparam.jsonl` of `oracles.soulsformats.elden/format`, one record per bundle: its binder,
/// bundle and member names, the header words, every texture slot, render pass, CAParam and MtdParam, floats as bit
/// patterns). Words compare as 32-bit words. The reference skips the word Elden Ring stores at 0x20 of some texture
/// records ([`fromsoft_formats::metaparam::Texture::unknown_20`]), so it is not compared.
#[test]
fn every_elden_ring_metaparam_matches_an_independent_reading() {
    use inputs::{OracleSet, Query, expect_found, oracle};
    use serde_json::json;
    let (Some(archive), Some(format), Some(dll)) = (
        common::archive(fromsoft_formats::Game::EldenRing).unwrap(),
        oracle(OracleSet::SoulsFormats, Game::Elden, "format"),
        oodle(Tool::Oodle6),
    ) else {
        return;
    };
    let dll = common::oodle(dll).unwrap();
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: u32::MAX as usize,
    };
    let leaf = |n: &str| n.rsplit(['\\', '/']).next().unwrap_or(n).to_owned();
    let records = std::fs::read_to_string(format.join("metaparam.jsonl")).unwrap();
    // the binders the records name, read on the archive's threads, their bundles kept by name
    let named: BTreeSet<String> = records
        .lines()
        .map(|line| {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            record["binder"].as_str().unwrap().to_owned()
        })
        .collect();
    let read = archive
        .read_each_binder(&named, &decoder, inputs::threads(), |_, binder| {
            binder
                .unwrap()
                .files
                .into_iter()
                .map(|f| (leaf(f.name.as_deref().unwrap_or("")), f.bytes))
                .collect::<BTreeMap<String, Vec<u8>>>()
        })
        .unwrap();
    let binders: BTreeMap<String, BTreeMap<String, Vec<u8>>> = named.into_iter().zip(read).collect();
    let (mut compared, mut failures) = (0usize, Vec::new());
    for line in records.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        let binder = record["binder"].as_str().unwrap().to_owned();
        let bundles = &binders[&binder];
        let bundle = leaf(record["bundle"].as_str().unwrap());
        let name = record["name"].as_str().unwrap();
        let member = Binder::read(&bundles[&bundle], &decoder)
            .unwrap()
            .files
            .into_iter()
            .find(|f| f.name.as_deref().is_some_and(|n| leaf(n) == name))
            .unwrap();
        let m = match Metaparam::read(&member.bytes) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{bundle}: we refuse what the reference reads: {e}"));
                continue;
            }
        };
        let bits = |f: f32| format!("{:08x}", f.to_bits());
        let ours = json!({
            "unk30": m.unknown_30,
            "cbuffer_size": m.constant_buffer_size,
            "flags": m.flags,
            "unk40": m.unknown_40,
            "gxmd_key": m.gxmd_key,
            "ca_block_size": m.ca_param_block_size,
            "unk50": m.unknown_50,
            "ca_param_components": m.ca_param_components,
            "shader_id": m.shader_id,
            "textures": m.textures.iter().map(|t| json!({
                "name": t.name, "slot": t.slot, "kind": t.kind, "unk0c": t.unknown_0c,
                "default_texture": t.default_texture, "uv_group": t.uv_group,
            })).collect::<Vec<_>>(),
            "render_passes": m.passes.iter().map(|p| json!({
                "id": p.pass_id, "pixel": p.pixel_samplers, "hull": p.hull_samplers, "vertex": p.vertex_samplers,
            })).collect::<Vec<_>>(),
            "ca_params": m.ca_params.iter().map(|p| json!({
                "name": p.name, "sequence": p.sequence_index, "param": p.param_id, "component": p.component,
                "default": bits(p.default), "unk40": p.unknown_40, "key": p.key,
            })).collect::<Vec<_>>(),
            "mtd_params": m.mtd_params.iter().map(|p| json!({
                "name": p.name, "offset": p.start, "index": p.sequence_index, "value_type": p.value_type,
                "not_shader_input": p.not_shader_input, "srgb": p.srgb, "key": p.key,
                "default": p.default.iter().map(|&f| bits(f)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        let mut theirs = record.clone();
        for k in ["binder", "bundle", "name"] {
            theirs.as_object_mut().unwrap().remove(k);
        }
        compared += 1;
        if words(ours) != words(theirs) {
            failures.push(format!("{bundle}: a field differs"));
        }
    }
    println!("metaparams compared {compared}, mismatches {}", failures.len());
    for f in failures.iter().take(20) {
        println!("MISMATCH {f}");
    }
    expect_found(
        Query::Oracle(OracleSet::SoulsFormats, Game::Elden, "format"),
        "metaparam records",
        compared,
    );
    assert!(failures.is_empty());
}
