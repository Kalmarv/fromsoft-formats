//! Elden Ring's shader names and sampler names over every material binary of the install (`shader`), soulstruct's
//! per-shader tables of the texture coordinate sets against the models that use the shaders, and Sekiro's shader names
//! through the same grammar.
//!
//! The expected numbers are a census made with an independent Python reader of the same files (a MATBIN walker, the
//! documented grammar and map table written again, a FLVER layout walker over the same archive entries); the second test's
//! oracle is `oracles.soulstruct.elden/shader-uv-slots.json`, soulstruct 12b6918's `SHADER_UV_SLOTS` and `SHADER_GROUP_UV`
//! converted to JSON.
mod inputs;

use fromsoft_formats::{
    Archive, Binder, Game, NameDictionary,
    dcx::Decoder,
    flver::Model,
    matbin::Material,
    shader::{Map, Profile, sampler_map, stem},
};
use inputs::{OracleSet, Query, Tool, expect_found, oodle, oracle};
use std::collections::{BTreeMap, BTreeSet};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
mod common;

/// The binders that hold Elden Ring's material binaries: the game's, the two expansions' and SpeedTree's.
const BINDERS: [&str; 4] = [
    "/material/allmaterial.matbinbnd.dcx",
    "/material/allmaterial_dlc01.matbinbnd.dcx",
    "/material/allmaterial_dlc02.matbinbnd.dcx",
    "/material/speedtree.matbinbnd.dcx",
];

fn materials(archive: &Archive, decoder: &Decoder<'_>) -> Result<Vec<Material>> {
    let mut out = Vec::new();
    for path in BINDERS {
        let binder = Binder::read(&archive.read(path, decoder)?, decoder)?;
        for file in &binder.files {
            out.push(Material::read(&file.bytes).map_err(|e| format!("{path}: {e}"))?);
        }
    }
    Ok(out)
}

#[test]
fn every_shader_and_sampler_name_of_the_install() -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(Game::EldenRing)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let materials = materials(&archive, &decoder)?;
    let mut stems = BTreeSet::new();
    let mut unparsed_stems = BTreeSet::new();
    let mut unparsed_materials = 0usize;
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut families: BTreeMap<String, usize> = BTreeMap::new();
    let mut blends: BTreeMap<u32, usize> = BTreeMap::new();
    let mut overlays: BTreeMap<String, usize> = BTreeMap::new();
    let (mut characters, mut maps, mut specific) = (0usize, 0usize, 0usize);
    let (mut cloth, mut sss, mut alpha, mut edge, mut glow, mut snow) = (0usize, 0, 0, 0, 0, 0);
    let (mut samplers, mut known, mut other, mut none) = (0usize, 0usize, 0usize, 0usize);
    let mut known_by: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut other_names = BTreeSet::new();
    let mut none_names = BTreeSet::new();
    for m in &materials {
        let name = stem(&m.shader);
        stems.insert(name.to_owned());
        match Profile::parse(name) {
            None => {
                unparsed_stems.insert(name.to_owned());
                unparsed_materials += 1;
            }
            Some(p) => {
                assert_eq!(p.stem(), name, "a profile rebuilds its stem");
                *categories.entry(p.category.clone()).or_default() += 1;
                *families.entry(p.family().to_owned()).or_default() += 1;
                *blends.entry(p.multi_blend_count()).or_default() += 1;
                if let Some(o) = p.overlay() {
                    *overlays.entry(o.to_owned()).or_default() += 1;
                }
                characters += usize::from(p.is_character());
                maps += usize::from(p.is_map());
                specific += usize::from(p.is_specific_character());
                cloth += usize::from(p.has_suffix("cloth"));
                sss += usize::from(p.has_suffix("SSS"));
                alpha += usize::from(p.has_suffix("Alpha"));
                edge += usize::from(p.has_suffix("Edge"));
                glow += usize::from(p.has_suffix("Emissive") || p.has_suffix("Glow"));
                snow += usize::from(p.has_tag("Snow"));
            }
        }
        for s in &m.samplers {
            samplers += 1;
            match sampler_map(&s.kind) {
                Some(Map::Known(kind)) => {
                    known += 1;
                    *known_by.entry(kind.name()).or_default() += 1;
                }
                Some(Map::Other(n)) => {
                    other += 1;
                    other_names.insert(n);
                }
                None => {
                    none += 1;
                    none_names.insert(s.kind.clone());
                }
            }
        }
    }
    println!(
        "materials {}, shader stems {} ({} outside the grammar, on {unparsed_materials} materials)",
        materials.len(),
        stems.len(),
        unparsed_stems.len()
    );
    println!("categories {categories:?}\nmulti-blend counts {blends:?}\noverlays {overlays:?}");
    println!(
        "families {} distinct; characters {characters}, maps {maps}, one character's {specific}, cloth {cloth}, SSS {sss}, \
         Alpha {alpha}, Edge {edge}, Emissive/Glow {glow}, Snow {snow}",
        families.len()
    );
    println!(
        "samplers {samplers}: known {known} {known_by:?}, other {other} ({} names), none {none} ({} names)",
        other_names.len(),
        none_names.len()
    );
    expect_found(
        Query::Install(inputs::Game::Elden, None),
        "material binaries",
        materials.len(),
    );
    assert_eq!(materials.len(), 20_619);
    assert_eq!(
        (stems.len(), unparsed_stems.len(), unparsed_materials),
        (975, 283, 752)
    );
    assert_eq!(
        categories,
        BTreeMap::from([
            ("C".to_owned(), 8_082),
            ("CS".to_owned(), 109),
            ("M".to_owned(), 11_016),
            ("P".to_owned(), 280),
            ("PC".to_owned(), 2),
            ("S".to_owned(), 378),
        ])
    );
    assert_eq!(families.len(), 232);
    for (family, n) in [
        ("AMSN", 10_796),
        ("DetailBlend", 5_592),
        ("Fur", 911),
        ("Crystal", 269),
        ("Hair", 159),
        ("Shell", 84),
        ("Water", 53),
    ] {
        assert_eq!(families.get(family), Some(&n), "{family}");
    }
    assert_eq!(
        blends,
        BTreeMap::from([(1, 16_679), (2, 1_644), (3, 908), (4, 617), (5, 19)])
    );
    assert_eq!(
        (overlays.len(), overlays.values().sum::<usize>()),
        (10, 5_314),
        "{overlays:?}"
    );
    assert_eq!(
        (overlays.get("N"), overlays.get("AN")),
        (Some(&4_289), Some(&873))
    );
    assert_eq!((characters, maps, specific), (8_471, 11_016, 425));
    assert_eq!(
        (cloth, sss, alpha, edge, glow, snow),
        (1_293, 207, 144, 914, 590, 211)
    );
    assert_eq!((samplers, known, other, none), (152_138, 149_307, 562, 2_269));
    assert_eq!((other_names.len(), none_names.len()), (47, 28));
    assert_eq!(
        known_by,
        BTreeMap::from([
            ("3Mask", 19),
            ("Albedo", 59_034),
            ("BlendEdge", 422),
            ("BlendMask", 16),
            ("Displacement", 625),
            ("DistortionDepth", 312),
            ("Emissive", 930),
            ("Mask1", 5_863),
            ("Mask3", 2_507),
            ("Metallic", 15_810),
            ("Normal", 62_747),
            ("Reflectance", 253),
            ("Shadowmap", 21),
            ("Shininess", 156),
            ("UniqueOpacityMask", 4),
            ("Vector", 588),
        ])
    );
    // every sampler name the table does not know is an effect or sky map; the unnamed ones are the legacy samplers
    assert!(none_names.iter().all(|n| n.starts_with("g_")), "{none_names:?}");
    Ok(())
}

/// The UV attributes a mesh's vertices carry (all the same on one mesh), none for a mesh without vertices.
fn uv_count(mesh: &fromsoft_formats::flver::Mesh) -> Option<usize> {
    mesh.vertices.first().map(|v| v.uvs.len())
}

#[test]
fn soulstructs_uv_slot_table_against_the_models() -> Result<()> {
    let (Some(archive), Some(dll), Some(names), Some(table)) = (
        common::archive(Game::EldenRing)?,
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, inputs::Game::Elden, "dictionary.txt"),
        oracle(OracleSet::Soulstruct, inputs::Game::Elden, "shader-uv-slots.json"),
    ) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let table: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(table)?)?;
    let slots = table["slots"].as_object().ok_or("slots")?;
    let groups = table["groups"].as_object().ok_or("groups")?;
    assert_eq!((slots.len(), groups.len()), (176, 168));
    // the table is consistent with itself: every UV a group names is one of its shader's slots
    let mut inconsistent = Vec::new();
    for (stem, by_group) in groups {
        let tuple: BTreeSet<&str> = slots
            .get(stem)
            .and_then(|t| t.as_array())
            .map(|t| t.iter().filter_map(|u| u.as_str()).collect())
            .unwrap_or_default();
        for (group, uv) in by_group.as_object().ok_or("group map")? {
            if !tuple.contains(uv.as_str().ok_or("uv name")?) {
                inconsistent.push(format!("{stem} {group}"));
            }
        }
    }
    assert!(
        inconsistent.is_empty(),
        "UVs outside their shader's tuple: {inconsistent:?}"
    );
    // the sources of the materials: the stem of each material binary's shader, by the source name a model's material states
    let mut by_source: BTreeMap<String, String> = BTreeMap::new();
    for m in materials(&archive, &decoder)? {
        by_source.insert(m.source.to_lowercase(), stem(&m.shader).to_owned());
    }
    let names = NameDictionary::read_for(names, Game::EldenRing)?;
    let mut observed: BTreeMap<String, BTreeMap<usize, usize>> = BTreeMap::new();
    let (mut models, mut speedtree, mut with_material, mut without_material) =
        (0usize, 0usize, 0usize, 0usize);
    let visited = common::each_leaf(
        &archive,
        &names,
        &[
            ".geombnd.dcx",
            ".mapbnd.dcx",
            ".partsbnd.dcx",
            ".chrbnd.dcx",
            ".objbnd.dcx",
        ],
        &decoder,
        |label, bytes| {
            if !bytes.starts_with(b"FLVER\0") {
                return Ok(());
            }
            let model = Model::read(bytes).map_err(|e| format!("{label}: {e}"))?;
            if model.speedtree {
                speedtree += 1;
                return Ok(());
            }
            models += 1;
            for mesh in &model.meshes {
                let Some(uvs) = uv_count(mesh) else {
                    continue;
                };
                let material = model.materials.get(mesh.material).ok_or("mesh material")?;
                match by_source.get(&material.mtd.to_lowercase()) {
                    Some(stem) => {
                        with_material += 1;
                        *observed.entry(stem.clone()).or_default().entry(uvs).or_default() += 1;
                    }
                    None => without_material += 1,
                }
            }
            Ok(())
        },
    )?;
    let norm = |s: &str| s.strip_suffix("_cloth").unwrap_or(s).to_owned();
    let (mut in_table, mut agreeing, mut disagreeing) = (0usize, 0usize, Vec::new());
    let (mut meshes_in_table, mut meshes_agreeing) = (0usize, 0usize);
    for (stem, counts) in &observed {
        let Some(tuple) = slots.get(&norm(stem)).and_then(|t| t.as_array()) else {
            continue;
        };
        in_table += 1;
        meshes_in_table += counts.values().sum::<usize>();
        match counts.get(&tuple.len()) {
            Some(n) => {
                agreeing += 1;
                meshes_agreeing += n;
            }
            None => disagreeing.push((stem.clone(), tuple.len(), counts.clone())),
        }
    }
    println!(
        "{visited} archive entries, {models} models read ({speedtree} SpeedTree skipped), {with_material} meshes with a \
         material binary, {without_material} without; {} shader stems seen, {in_table} in the table: the tuple's length occurs \
         among their meshes' UV counts for {agreeing}, never for {}; meshes of those stems {meshes_in_table}, of them \
         {meshes_agreeing} with exactly the tuple's length",
        observed.len(),
        disagreeing.len()
    );
    expect_found(
        Query::Install(inputs::Game::Elden, None),
        "models in map, character, part, asset and object binders",
        models,
    );
    assert_eq!((models, speedtree), (21_493, 121));
    assert_eq!((with_material, without_material), (75_826, 11_193));
    assert_eq!(observed.len(), 451);
    assert!(disagreeing.is_empty(), "{disagreeing:?}");
    assert_eq!((in_table, agreeing), (180, 180));
    assert_eq!((meshes_in_table, meshes_agreeing), (56_770, 55_811));
    // 168 of the table's 176 stems are in the grammar; the eight that are not are legacy and test shaders
    let outside: Vec<&String> = slots.keys().filter(|s| Profile::parse(s).is_none()).collect();
    assert_eq!(outside.len(), 8, "{outside:?}");
    Ok(())
}

/// Sekiro's material definitions (`/mtd/allmaterialbnd.mtdbnd.dcx`) through the same grammar: its map shaders (`M[Multiple]`,
/// `M[Translucency]`, `M[Sky]`) fit it, its character and generic shaders (`Character_AMSN_[DetailBlend]`, `GXFlver_*`, `Fur_NTC`)
/// do not.
#[test]
fn sekiros_shader_stems() -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(Game::Sekiro)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let binder = Binder::read(
        &archive.read("/mtd/allmaterialbnd.mtdbnd.dcx", &decoder)?,
        &decoder,
    )?;
    let (mut materials, mut parsed_materials) = (0usize, 0usize);
    let (mut stems, mut parsed_stems) = (BTreeSet::new(), BTreeSet::new());
    let mut categories: BTreeMap<String, usize> = BTreeMap::new();
    let mut families = BTreeSet::new();
    for file in binder.files.iter().filter(|f| {
        f.name
            .as_deref()
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".mtd"))
    }) {
        let definition = fromsoft_formats::mtd::MaterialDefinition::read(&file.bytes)
            .map_err(|e| format!("{:?}: {e}", file.name))?;
        materials += 1;
        let name = stem(&definition.shader);
        stems.insert(name.to_owned());
        if let Some(p) = Profile::parse(name) {
            assert_eq!(p.stem(), name);
            parsed_materials += 1;
            parsed_stems.insert(name.to_owned());
            *categories.entry(p.category.clone()).or_default() += 1;
            families.insert(p.family().to_owned());
        }
    }
    expect_found(
        Query::Install(inputs::Game::Sekiro, None),
        "material definitions",
        materials,
    );
    assert_eq!((materials, stems.len()), (3_170, 257));
    assert_eq!((parsed_stems.len(), parsed_materials), (56, 1_077));
    assert_eq!(
        categories,
        BTreeMap::from([("C".to_owned(), 8), ("M".to_owned(), 1_069)])
    );
    assert_eq!(families.len(), 23);
    Ok(())
}
