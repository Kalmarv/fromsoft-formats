//! The map layout writer: layouts built here write back to their bytes, take edits to the parts' fixed fields and refuse
//! what a view cannot change; and every map layout of both installs (Sekiro's 10, Elden Ring's 935 files, 887 distinct)
//! writes back to its bytes.
//!
//! The real-data counts were measured with an independent Python implementation of the layout (a parser and a writer of
//! its own, run over the same files): every one of the 897 distinct layouts is written byte for byte from its sections.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`
//! for the real-data test; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::{Archive, Game, Keys, NameDictionary};
use inputs::{Game as Data, OracleSet, Tool, install, keys, oodle, oracle};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn put64(data: &mut [u8], offset: usize, value: usize) {
    data[offset..offset + 8].copy_from_slice(&(value as u64).to_le_bytes());
}

/// A Sekiro-chain map whose parts section holds these part records (every other section empty).
fn map_with_parts(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut data = b"MSB \x01\0\0\0\x10\0\0\0\0\0\x01\xff".to_vec();
    let names = [
        "MODEL_PARAM_ST",
        "EVENT_PARAM_ST",
        "POINT_PARAM_ST",
        "ROUTE_PARAM_ST",
        "LAYER_PARAM_ST",
        "PARTS_PARAM_ST",
        "MAPSTUDIO_PARTS_POSE_ST",
        "MAPSTUDIO_BONE_NAME_STRING",
    ];
    for (i, name) in names.into_iter().enumerate() {
        let records: &[Vec<u8>] = if i == 5 { parts } else { &[] };
        let start = data.len();
        let table = 16 + 8 * (records.len() + 1);
        data.resize(start + table, 0);
        data[start..start + 4].copy_from_slice(&35u32.to_le_bytes());
        data[start + 4..start + 8].copy_from_slice(&((records.len() + 1) as u32).to_le_bytes());
        put64(&mut data, start + 8, start + table);
        data.extend(name.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        data.resize(data.len().next_multiple_of(8), 0);
        for (k, r) in records.iter().enumerate() {
            let at = data.len();
            put64(&mut data, start + 16 + 8 * k, at);
            data.extend_from_slice(r);
        }
        let next = if i == 7 { 0 } else { data.len() };
        put64(&mut data, start + table - 8, next);
    }
    data
}

/// One map-piece part record (no model): header, name, sib path, the entity block, eight bytes of type data.
fn part_record(name: &str, entity_id: i32, groups: [i32; 8]) -> Vec<u8> {
    let mut r = vec![0u8; 160];
    let text = |r: &mut Vec<u8>, field: usize, s: &str| {
        let at = r.len();
        put64(r, field, at);
        r.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        r.resize(r.len().next_multiple_of(8), 0);
    };
    text(&mut r, 0, name);
    text(&mut r, 24, "sib");
    r[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    r[56..68].copy_from_slice(&[1f32.to_le_bytes(); 3].concat());
    r[68..76].fill(255);
    let entity = r.len();
    put64(&mut r, 96, entity);
    r.resize(entity + 0x54, 0);
    r[entity..entity + 4].copy_from_slice(&entity_id.to_le_bytes());
    for (k, g) in groups.iter().enumerate() {
        r[entity + 0x1c + 4 * k..entity + 0x20 + 4 * k].copy_from_slice(&g.to_le_bytes());
    }
    let type_data = r.len();
    put64(&mut r, 104, type_data);
    r.resize(r.len() + 8, 0);
    r
}

fn two_parts() -> Vec<u8> {
    map_with_parts(&[
        part_record("m000000_0000", 5, [-1; 8]),
        part_record("m000001_0000", -1, [1117201, -1, 0, 1116201, -1, -1, -1, -1]),
    ])
}

#[test]
fn a_layout_writes_back_to_the_bytes_it_was_read_from() {
    for bytes in [map_with_parts(&[]), two_parts()] {
        let layout = MapLayout::read(&bytes).unwrap();
        assert_eq!(layout.write().unwrap(), bytes);
    }
}

#[test]
fn edits_to_the_parts_fixed_fields_are_written() {
    let bytes = two_parts();
    let mut layout = MapLayout::read(&bytes).unwrap();
    {
        let p = &mut layout.parts[1];
        p.position = [10.5, -2.0, 3.25];
        p.rotation_degrees = [0.0, 90.0, 180.0];
        p.scale = [2.0, 2.0, 2.0];
        p.entity_id = 1117999;
        p.entity_group_ids = [7, 6, 5, 4, 3, 2, 1, 0];
        p.shadows.cast = true;
        p.shadows.static_cast = 9;
        p.draw.lod_param_id = Some(3);
    }
    let written = layout.write().unwrap();
    assert_eq!(written.len(), bytes.len());
    assert_ne!(written, bytes);
    let again = MapLayout::read(&written).unwrap();
    let (a, b) = (&layout.parts[1], &again.parts[1]);
    assert_eq!(
        (
            a.position,
            a.rotation_degrees,
            a.scale,
            a.entity_id,
            a.entity_group_ids
        ),
        (
            b.position,
            b.rotation_degrees,
            b.scale,
            b.entity_id,
            b.entity_group_ids
        )
    );
    assert_eq!((a.shadows, a.draw), (b.shadows, b.draw));
    // the first part is as it was, and so is everything outside the second part's record
    assert_eq!(layout.parts[0].position, again.parts[0].position);
    assert_eq!(again.parts[0].entity_id, 5);
    assert_eq!(again.write().unwrap(), written);
}

#[test]
fn what_a_view_cannot_change_is_refused() {
    let layout = MapLayout::read(&two_parts()).unwrap();
    let refused = |edit: &dyn Fn(&mut MapLayout)| {
        let mut l = layout.clone();
        edit(&mut l);
        l.write().is_err()
    };
    assert!(!refused(&|_| {}), "the unedited layout writes");
    // a name changes the record's size; a kind and a model point elsewhere
    assert!(refused(&|l| l.parts[0].name = "renamed".into()));
    assert!(refused(&|l| l.parts[0].kind = PartKind::Enemy));
    assert!(refused(&|l| l.parts[0].model = Some(0)));
    // a field the part's kind has no place for
    assert!(refused(&|l| l.parts[0].hit_filter = Some(1)));
    assert!(refused(&|l| l.parts[0].npc_param = Some(1)));
    assert!(refused(&|l| l.parts[0].sfx_param_relative_id = Some(1)));
    // a view of a record the section lacks, and two views of one
    assert!(refused(&|l| l.parts[1].index = 2));
    assert!(refused(&|l| l.parts[1].index = 0));
    // sections that are not the generation's chain
    assert!(refused(&|l| l.sections[3].name = "ROUTE".into()));
    assert!(refused(&|l| {
        l.sections.pop();
    }));
}

#[test]
fn the_layout_must_still_read() {
    let mut layout = MapLayout::read(&two_parts()).unwrap();
    // a model states how many parts name it: a model section record with a count no part matches
    let mut model = vec![0u8; 40];
    let name_at = model.len();
    put64(&mut model, 0, name_at);
    model.extend("m0".encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    model.resize(model.len().next_multiple_of(8), 0);
    let sib_at = model.len();
    put64(&mut model, 16, sib_at);
    model.extend("s".encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    model.resize(model.len().next_multiple_of(8), 0);
    model[24..28].copy_from_slice(&3u32.to_le_bytes());
    layout.sections[0].records.push(model);
    let error = layout.write().unwrap_err().to_string();
    assert!(error.contains("instances"), "{error}");
}

#[test]
fn every_map_layout_of_the_installs_writes_back_to_its_bytes() -> Result<()> {
    for (game, data, expected) in [
        (Game::Sekiro, Data::Sekiro, 10),
        (Game::EldenRing, Data::Elden, 935),
    ] {
        let (Some(root), Some(folder), Some(dll), Some(dictionary)) = (
            install(data),
            keys(data),
            oodle(Tool::Oodle6),
            oracle(OracleSet::Uxm, data, "dictionary.txt"),
        ) else {
            continue;
        };
        let dll = common::oodle(dll)?;
        let decoder = Decoder {
            oodle: Some(&dll),
            max_output: 1 << 30,
        };
        let archive = Archive::open_game(root, game, &Keys::from_folder(folder, game, None)?)?;
        let dictionary = NameDictionary::read_for(dictionary, game)?;
        let maps: BTreeSet<&str> = dictionary
            .names
            .values()
            .flatten()
            .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
            .filter(|m| !m.contains('/'))
            .collect();
        let mut written = 0;
        for map in maps {
            let path = format!("/map/mapstudio/{map}.msb.dcx");
            if !archive.contains(&path) {
                continue;
            }
            let bytes = archive.read(&path, &decoder)?;
            let layout = MapLayout::read(&bytes).map_err(|e| format!("{path}: {e}"))?;
            let out = layout.write().map_err(|e| format!("{path}: {e}"))?;
            if out != bytes {
                return Err(format!("{path}: write() differs from the stored bytes").into());
            }
            written += 1;
        }
        println!(
            "{}: {written} map layouts written back byte for byte",
            game.name()
        );
        assert_eq!(written, expected, "{}", game.name());
    }
    Ok(())
}
