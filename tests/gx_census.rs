//! The GX items of every model in the map, character, part, asset and object binders of each installed game against
//! the community's descriptors (Smithbox's `GX_Item_Descriptors.json`, `oracles.smithbox.<game>/gx-item-descriptors.json`):
//! the crate's table of names is the descriptors' for the items the installs store, every item whose id and version the
//! table names stores no more words than the descriptor lists, and the properties the `Material` accessors read lie in
//! the ranges below.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (a FLVER walker over the
//! same archive entries).
//!
//! Elden Ring's counts are those of the path dictionary that names all 97,118 entries of the base archives
//! (`tests/inputs`: UXM's list and the 87 Smithbox paths that hash onto the entries UXM's list leaves unnamed); the
//! Python reader walked the same archives and named the same entries.
mod inputs;

use fromsoft_formats::{
    Game, NameDictionary,
    dcx::Decoder,
    flver::{
        Model,
        gx::{self, WordKind},
    },
};
use inputs::{OracleSet, Query, Tool, expect_found, oodle, oracle};
use std::collections::BTreeMap;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
mod common;

const SUFFIXES: [&str; 5] = [
    ".geombnd.dcx",
    ".mapbnd.dcx",
    ".partsbnd.dcx",
    ".chrbnd.dcx",
    ".objbnd.dcx",
];

/// The type code of the descriptor a word kind stands for.
fn code(kind: WordKind) -> u64 {
    match kind {
        WordKind::Unknown => 0,
        WordKind::Int => 1,
        WordKind::Float => 2,
        WordKind::Choice => 3,
        WordKind::Flag => 4,
    }
}

#[test]
fn the_table_is_the_descriptors() -> Result<()> {
    let Some(path) = oracle(
        OracleSet::Smithbox,
        inputs::Game::Elden,
        "gx-item-descriptors.json",
    ) else {
        return Ok(());
    };
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let list = json["list"].as_array().ok_or("list")?;
    let mut compared = 0;
    for info in gx::items() {
        let id = String::from_utf8(info.id.to_vec())?;
        let d = list
            .iter()
            .find(|d| d["ID"] == id.as_str() && d["Unk04"] == u64::from(info.version))
            .ok_or_else(|| format!("{id} v{} is not in the descriptors", info.version))?;
        let words = d["Items"].as_array().ok_or("Items")?;
        assert_eq!(words.len(), info.words.len(), "{id} v{}", info.version);
        for (w, n) in words.iter().zip(info.words) {
            assert_eq!(w["Name"].as_str(), Some(n.name), "{id} v{}", info.version);
            assert_eq!(w["Type"].as_u64(), Some(code(n.kind)), "{id} {}", n.name);
        }
        assert_eq!(d["Category"].as_str(), info.category, "{id}");
        compared += 1;
    }
    expect_found(
        Query::Oracle(
            OracleSet::Smithbox,
            inputs::Game::Elden,
            "gx-item-descriptors.json",
        ),
        "items compared with the descriptors",
        compared,
    );
    assert_eq!(compared, gx::items().len());
    assert_eq!(compared, 24);
    Ok(())
}

/// What a game's models hold.
#[derive(Default)]
struct Totals {
    models: usize,
    materials: usize,
    items: usize,
    /// Items by id and version (`GX03v100`).
    by_item: BTreeMap<String, usize>,
    /// Items the table does not name, by id.
    unnamed: BTreeMap<String, usize>,
    /// The materials with each property, and the lowest and highest value of its first word.
    emissive: (usize, f32, f32),
    translucency: (usize, f32, f32),
    albedo: (usize, f32, f32),
    parallax: usize,
    water: usize,
}

fn track(slot: &mut (usize, f32, f32), v: f32) {
    if slot.0 == 0 {
        slot.1 = v;
        slot.2 = v;
    }
    slot.0 += 1;
    slot.1 = slot.1.min(v);
    slot.2 = slot.2.max(v);
}

fn census(game: Game, data: inputs::Game) -> Result<Option<Totals>> {
    let (Some(archive), Some(dll), Some(names)) = (
        common::archive(game)?,
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let names = NameDictionary::read_for(names, game)?;
    let mut t = Totals::default();
    common::each_leaf(&archive, &names, &SUFFIXES, &decoder, |label, bytes| {
        if !bytes.starts_with(b"FLVER\0") {
            return Ok(());
        }
        let model = Model::read(bytes).map_err(|e| format!("{label}: {e}"))?;
        t.models += 1;
        for m in &model.materials {
            t.materials += 1;
            for item in &m.gx {
                t.items += 1;
                let id = String::from_utf8_lossy(&item.id).into_owned();
                match item.info() {
                    Some(info) => {
                        *t.by_item.entry(format!("{id}v{}", item.version)).or_default() += 1;
                        let fields = item.fields().map_err(|e| format!("{label}: {id}: {e}"))?;
                        assert!(
                            fields.is_some_and(|f| f.len() <= info.words.len()),
                            "{label}: {id}"
                        );
                    }
                    None => {
                        assert!(item.fields()?.is_none());
                        *t.unnamed.entry(id).or_default() += 1;
                    }
                }
            }
            if let Some(v) = m.emissive_intensity()? {
                track(&mut t.emissive, v);
            }
            if let Some(v) = m.translucency_scale()? {
                track(&mut t.translucency, v);
            }
            if let Some(c) = m.albedo_colour()? {
                track(&mut t.albedo, c.multiplier);
                assert!(c.rgb.iter().all(|x| (0.0..=1.0).contains(x)), "{label}: {c:?}");
            }
            if let Some(p) = m.parallax()? {
                assert!(
                    p.iter().all(|l| (l.offset, l.middle_height) == (0., 0.5)),
                    "{label}: {p:?}"
                );
                t.parallax += 1;
            }
            if let Some(w) = m.water()? {
                assert_eq!(w.depth_fog_density, 1., "{label}");
                t.water += 1;
            }
        }
        Ok(())
    })?;
    Ok(Some(t))
}

fn counts(pairs: &[(&str, usize)]) -> BTreeMap<String, usize> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
}

#[test]
fn sekiros_gx_items() -> Result<()> {
    let Some(t) = census(Game::Sekiro, inputs::Game::Sekiro)? else {
        return Ok(());
    };
    println!(
        "{} models, {} materials, {} items; emissive {:?}, translucency {:?}, albedo {:?}, parallax {}, water {}",
        t.models, t.materials, t.items, t.emissive, t.translucency, t.albedo, t.parallax, t.water
    );
    expect_found(Query::Install(inputs::Game::Sekiro, None), "models", t.models);
    assert_eq!((t.models, t.materials, t.items), (14_310, 43_252, 70_948));
    assert_eq!(
        t.by_item,
        counts(&[
            ("GX00v100", 1),
            ("GX00v101", 29),
            ("GX00v102", 43_043),
            ("GX03v100", 251),
            ("GX04v100", 6_369),
            ("GX05v100", 2_062),
            ("GX06v101", 2),
            ("GX07v104", 2),
            ("GX09v100", 2),
            ("GX14v101", 193),
            ("GX15v100", 382),
            ("GX20v100", 995),
            ("GX21v100", 2),
            ("GX22v100", 804),
            ("GX23v100", 2),
            ("GX24v100", 804),
            ("GX25v100", 2),
            ("GX26v100", 804),
            ("GX27v100", 2),
            ("GX30v100", 22),
            ("GX32v100", 251),
            ("GX39v100", 3),
            ("GX80v100", 6),
            ("GX81v100", 6),
        ])
    );
    // the items the table does not name are the material parameter records
    assert_eq!(t.unnamed, counts(&[("GXMD", 14_909)]));
    assert_eq!((t.emissive.0, t.emissive.1, t.emissive.2), (251, 0.7, 3.32));
    assert_eq!(
        (t.translucency.0, t.translucency.1, t.translucency.2),
        (2_062, 1., 10.)
    );
    assert_eq!((t.albedo.0, t.albedo.1, t.albedo.2), (193, 0.8, 1.));
    assert_eq!((t.parallax, t.water), (2, 2));
    Ok(())
}

#[test]
fn elden_rings_gx_items() -> Result<()> {
    let Some(t) = census(Game::EldenRing, inputs::Game::Elden)? else {
        return Ok(());
    };
    println!(
        "{} models, {} materials, {} items; emissive {:?}, translucency {:?}, albedo {:?}, parallax {}, water {}",
        t.models, t.materials, t.items, t.emissive, t.translucency, t.albedo, t.parallax, t.water
    );
    expect_found(Query::Install(inputs::Game::Elden, None), "models", t.models);
    assert_eq!((t.models, t.materials, t.items), (21_681, 86_791, 113_293));
    assert_eq!(
        t.by_item,
        counts(&[
            ("GX00v100", 1),
            ("GX00v102", 85_906),
            ("GX03v100", 1),
            ("GX04v100", 136),
            ("GX05v100", 5_130),
            ("GX14v101", 688),
            ("GX15v100", 857),
            ("GX20v100", 165),
            ("GX22v100", 162),
            ("GX24v100", 162),
            ("GX26v100", 162),
            ("GX30v100", 1),
            ("GX32v100", 1),
        ])
    );
    assert_eq!(
        t.unnamed,
        counts(&[("GX18", 565), ("GXMD", 19_237), ("GXUD", 119)])
    );
    assert_eq!((t.emissive.0, t.emissive.1, t.emissive.2), (1, 0.6, 0.6));
    assert_eq!(
        (t.translucency.0, t.translucency.1, t.translucency.2),
        (5_130, 1., 1.)
    );
    assert_eq!(t.albedo.0, 688);
    assert!(
        (t.albedo.1 - 0.79311).abs() < 1e-5 && (t.albedo.2 - 1.359988).abs() < 1e-5,
        "{:?}",
        t.albedo
    );
    assert_eq!((t.parallax, t.water), (0, 0));
    Ok(())
}
