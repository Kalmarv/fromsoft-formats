//! Lists the map pieces of a layout whose model draws a material matching a pattern: each part with its model, its
//! placement as stored and the matching materials' names and MTD paths.
//!
//! ```text
//! cargo run --example find_material -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <map> <pattern>
//! cargo run --example find_material -- sekiro "D:/Games/Sekiro" ./keys "D:/Games/Sekiro/oo2core_6_win64.dll" m11_01_00_00 Moon
//! ```
//!
//! Arguments as `list_binder`'s, then the map (`m11_01_00_00`) and a case-insensitive pattern the material's name or MTD
//! path contains. The layout is `/map/mapstudio/<map>.msb.dcx`; a map piece's model `mNNNNNN` is the binder
//! `/map/<map>/<map>_NNNNNN.mapbnd.dcx`.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::flver::Model;
use fromsoft_formats::msb::{MapLayout, PartKind};
use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::collections::BTreeMap;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, map, pattern] = args.as_slice() else {
        return Err("usage: find_material <sekiro|elden> <game folder> <keys folder> <oodle library | -> <map> <pattern>".into());
    };
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };
    let pattern = pattern.to_lowercase();
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let keys = Keys::from_folder(keys, game, None)?;
    let install = Install::new(root, game, keys).open()?;
    let layout = MapLayout::read(&install.read(&format!("/map/mapstudio/{map}.msb.dcx"), &decoder)?)?;
    // each map piece model's matching materials, read once
    let mut found: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut unread = 0;
    for model in &layout.models {
        let Some(digits) = model.name.strip_prefix('m').filter(|d| d.len() == 6) else { continue };
        let path = format!("/map/{map}/{map}_{digits}.mapbnd.dcx");
        let Ok(bytes) = install.read(&path, &decoder) else {
            unread += 1;
            continue;
        };
        let binder = Binder::read(&bytes, &decoder)?;
        for file in &binder.files {
            if !file.name.as_deref().is_some_and(|n| n.to_lowercase().ends_with(".flver")) {
                continue;
            }
            let flver = Model::read(&file.bytes)?;
            for m in &flver.materials {
                if m.name.to_lowercase().contains(&pattern) || m.mtd.to_lowercase().contains(&pattern) {
                    found.entry(model.name.clone()).or_default().push((m.name.clone(), m.mtd.clone()));
                }
            }
        }
    }
    println!("{} models, {} map piece binders not found", layout.models.len(), unread);
    for part in &layout.parts {
        if part.kind != PartKind::MapPiece {
            continue;
        }
        let Some(model) = part.model.and_then(|i| layout.models.get(i)) else { continue };
        if let Some(materials) = found.get(&model.name) {
            println!(
                "{} model {} at {:?} rotation {:?} scale {:?}: {:?}",
                part.name, model.name, part.position, part.rotation_degrees, part.scale, materials
            );
        }
    }
    Ok(())
}
