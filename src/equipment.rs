//! Part-built human NPCs: an MSB enemy's `CharaInitID` names a `CharaInitParam` row, which names
//! `EquipParamProtector` rows, which name the character part binders (`/parts/<part>.partsbnd.dcx`) that dress it.
//! `NpcParam` has display masks, not this equipment link. The initializer's four protector IDs are at +32..48 and its
//! `npcPlayerSex` at +228 in both games; the protector's `equipModelId`, `equipModelCategory` and `equipModelGender`
//! are at +160, +208 and +209 in Sekiro (Paramdex's SDT v4 definitions) and +164, +212 and +213 in Elden Ring
//! ([`OutfitLayout`]). Only the stated fields are read; no face-generation, body-shape or runtime equipment
//! replacement is invented.
use crate::game_params::{NpcPlacement, npc_placements, read_param};
use crate::param::{Param, Row};
use crate::{Archive, Binder, Game, Install, NameDictionary, Result, dcx::Decoder, flver::Model, invalid};

/// The outfit one enemy placement of a map names: the character parts it is built from, and what could not be
/// resolved.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct NpcOutfit {
    /// The NPC param row the placement names.
    pub npc: i32,
    /// The initializer the placement names; `None` where the MSB stores a negative id (it names none).
    pub init: Option<i32>,
    /// The map the placement is in.
    pub map: String,
    /// The placement's part name in the map.
    pub placement: String,
    /// The part names resolved (`bd_m_9500`), each with a part binder in the archive.
    pub parts: Vec<String>,
    /// Why a slot could not be resolved, one sentence each.
    pub unresolved: Vec<String>,
    /// Every part the read named by which binders the archive holds, not by a field the param states ([`GENDER_BY_PRESENCE`],
    /// [`LEVEL_SUFFIX_BY_PRESENCE`]).
    pub chosen_by_presence: Vec<PresenceChoice>,
}

/// A placement the MSB gives no character initializer (its stored id is negative): it has no authored outfit to resolve.
pub const NO_INITIALIZER: &str = "enemy placement that names no character initializer: no authored outfit";

/// A part named by which binder exists.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PresenceChoice {
    /// Which ambiguity the archive settled ([`GENDER_BY_PRESENCE`] or [`LEVEL_SUFFIX_BY_PRESENCE`]).
    pub reason: &'static str,
    /// The protector row and the part binder chosen.
    pub example: String,
}

/// The protector states no gender the part's spelling (`m`, `f` or `a`) follows (Sekiro's modes 0 and 1: unspecified, not
/// split by sex; Elden Ring's mode 3, split by `npcPlayerSex`, whose values are not known to be `m` or `f`), so the
/// spelling is the one the archive holds.
pub const GENDER_BY_PRESENCE: &str = "outfit part whose gender spelling (m, f or a) was chosen by which binder the archive holds: the protector states no gender";

/// A part is held with the level suffix `_l` and without it; the param does not say which, so `_l` is taken where the
/// archive holds it (also where it holds both), else the one without.
pub const LEVEL_SUFFIX_BY_PRESENCE: &str = "outfit part named with or without the level suffix _l by which binder the archive holds: the param does not say which";

fn row<'a>(param: &'a Param, name: &str, id: i32) -> Result<&'a Row> {
    param
        .rows
        .iter()
        .find(|r| r.id == id)
        .ok_or_else(|| invalid("NPC outfit", 0, format!("{name} has no row {id}")))
}

fn word(row: &Row, at: usize) -> Result<i32> {
    let bytes: [u8; 4] = at
        .checked_add(4)
        .and_then(|end| row.data.get(at..end))
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| invalid("NPC outfit", at, format!("short row {}", row.id)))?;
    Ok(i32::from_le_bytes(bytes))
}

/// Where a game's outfit params keep the fields the read uses: the layout revision and row size of each param (the pair
/// the offsets are for, as for [`crate::param::NpcLayout`]) and the byte offsets in an `EQUIP_PARAM_PROTECTOR_ST` row.
/// The initializer's four slots (+32, +36, +40, +44: head, body, arms, legs) and `npcPlayerSex` (+228) are the same in
/// both games.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutfitLayout {
    /// `CHARACTER_INIT_PARAM`: data version and row size.
    pub initializer: (u16, usize),
    /// `EQUIP_PARAM_PROTECTOR_ST`: data version and row size.
    pub protector: (u16, usize),
    /// `equipModelId` (u16) in a protector row.
    pub model: usize,
    /// `equipModelCategory` (u8) in a protector row.
    pub category: usize,
    /// `equipModelGender` (u8) in a protector row.
    pub gender: usize,
}

impl OutfitLayout {
    /// The layout of `game`'s outfit params (Paramdex's definitions as Smithbox ships them, container-rule offsets).
    pub const fn of(game: Game) -> OutfitLayout {
        match game {
            Game::Sekiro => OutfitLayout {
                initializer: (4, 472),
                protector: (4, 508),
                model: 160,
                category: 208,
                gender: 209,
            },
            Game::EldenRing => OutfitLayout {
                initializer: (2, 320),
                protector: (6, 416),
                model: 164,
                category: 212,
                gender: 213,
            },
        }
    }
}

/// The initializer's protector slots and the part prefix each takes.
const SLOTS: [(&str, usize); 4] = [("hd", 32), ("bd", 36), ("am", 40), ("lg", 44)];
const SEX: usize = 228;

/// The part prefix a protector of `category` gives in `slot`: head 5 `hd`, body 2 `bd`, arms 1 `am`, legs 6 `lg`, and in
/// Sekiro also category 3 in the head slot, the face `fc`. Any other category (Elden Ring's 4, hair; Sekiro's 0, the
/// row with no model) is not that slot's part.
fn part_prefix(game: Game, slot: &str, category: u8) -> Option<&'static str> {
    match (slot, category) {
        ("hd", 5) => Some("hd"),
        ("hd", 3) if game == Game::Sekiro => Some("fc"),
        ("bd", 2) => Some("bd"),
        ("am", 1) => Some("am"),
        ("lg", 6) => Some("lg"),
        _ => None,
    }
}

/// What one initializer's outfit resolved to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutfitParts {
    /// The part names resolved (`bd_m_9500`), each with a part binder in the archive.
    pub parts: Vec<String>,
    /// Why a slot could not be resolved, one sentence each.
    pub unresolved: Vec<String>,
    /// Every part named by which binders the archive holds ([`NpcOutfit::chosen_by_presence`]).
    pub chosen_by_presence: Vec<PresenceChoice>,
}

/// The parts of initializer `id` of `initializers`, whose protectors are rows of `protectors`, for `game`; `has` says
/// whether the archive holds the part binder of a name (`bd_m_9040_l`). The two params must be the layout of
/// [`OutfitLayout::of`]. A slot whose protector's category is not the slot's, or whose binder the archive lacks, is
/// reported in `unresolved`, not guessed.
pub fn resolve_outfit(
    game: Game,
    initializers: &Param,
    protectors: &Param,
    id: i32,
    has: impl Fn(&str) -> bool,
) -> Result<OutfitParts> {
    let layout = OutfitLayout::of(game);
    if initializers.param_type != "CHARACTER_INIT_PARAM"
        || (initializers.data_version, initializers.row_size) != layout.initializer
        || protectors.param_type != "EQUIP_PARAM_PROTECTOR_ST"
        || (protectors.data_version, protectors.row_size) != layout.protector
    {
        return Err(invalid(
            "NPC outfit",
            0,
            format!(
                "only {}'s CHARACTER_INIT_PARAM version {} with rows of {} bytes and EQUIP_PARAM_PROTECTOR_ST version {} with rows of {} are established",
                game.name(),
                layout.initializer.0,
                layout.initializer.1,
                layout.protector.0,
                layout.protector.1
            ),
        ));
    }
    let init = row(initializers, "CharaInitParam", id)?;
    // Sekiro's split-by-sex protectors take the initializer's sex as the part's letter; Elden Ring's `npcPlayerSex`
    // values (0 and 1, named "Type 0" and "Type 1") are not known to be male and female, so it is not read there.
    let sex = if game == Game::Sekiro {
        match *init
            .data
            .get(SEX)
            .ok_or_else(|| invalid("NPC outfit", SEX, "short npcPlayerSex"))?
        {
            0 => "m",
            1 => "f",
            _ => return Err(invalid("NPC outfit", SEX, "unknown npcPlayerSex")),
        }
    } else {
        ""
    };
    let mut out = OutfitParts::default();
    for (slot, at) in SLOTS {
        let equipped = word(init, at)?;
        if equipped < 0 {
            continue;
        }
        // a slot naming a row the param lacks (65 of Elden Ring's 13,092) leaves that slot unresolved, not the outfit
        let Some(protector) = protectors.rows.iter().find(|r| r.id == equipped) else {
            out.unresolved.push(format!(
                "protector {equipped}: EquipParamProtector has no such row"
            ));
            continue;
        };
        let model: [u8; 2] = protector
            .data
            .get(layout.model..layout.model.saturating_add(2))
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| invalid("NPC outfit", layout.model, "short equipModelId"))?;
        let model = u16::from_le_bytes(model);
        let byte = |at: usize, what: &str| {
            protector
                .data
                .get(at)
                .copied()
                .ok_or_else(|| invalid("NPC outfit", at, format!("short {what}")))
        };
        let category = byte(layout.category, "equipModelCategory")?;
        let gender = byte(layout.gender, "equipModelGender")?;
        let Some(prefix) = part_prefix(game, slot, category) else {
            out.unresolved.push(format!(
                "protector {equipped}: equipModelCategory {category} is not a {slot} part"
            ));
            continue;
        };
        // What the gender mode says the letter is. Sekiro: 3 splits by the initializer's sex, 4 shares one part (`a`),
        // 0 and 1 are unspecified (the archive says). Elden Ring: 1 male, 2 female, 4 none (every such part is `m`, no
        // protector has an `a` part) and 3 splits by a sex whose letter is not known (the archive says).
        let (genders, by_presence): (&[&str], bool) = match (game, gender) {
            (Game::Sekiro, 3) => (std::slice::from_ref(&sex), false),
            (Game::Sekiro, 4) => (&["a"], false),
            (Game::Sekiro, 0 | 1) => (&["m", "f", "a"], true),
            (Game::EldenRing, 1 | 4) => (&["m"], false),
            (Game::EldenRing, 2) => (&["f"], false),
            (Game::EldenRing, 3) => (&["m", "f"], true),
            _ => {
                out.unresolved
                    .push(format!("protector {equipped}: unknown equipModelGender {gender}"));
                continue;
            }
        };
        let mut candidates = Vec::new();
        let mut suffix_choices = Vec::new();
        for gender in genders {
            let held: Vec<String> = ["_l", ""]
                .iter()
                .map(|suffix| format!("{prefix}_{gender}_{model:04}{suffix}"))
                .filter(|name| has(name))
                .collect();
            // `_l` is taken first; reported where that was a pick (both held, or only the plain name)
            if let Some(name) = held.first() {
                if held.len() > 1 || !name.ends_with("_l") {
                    suffix_choices.push(PresenceChoice {
                        reason: LEVEL_SUFFIX_BY_PRESENCE,
                        example: format!("protector {equipped}: {name} of {}", held.join(", ")),
                    });
                }
                candidates.push(name.clone());
            }
        }
        candidates.sort();
        candidates.dedup();
        match candidates.as_slice() {
            [name] => {
                out.chosen_by_presence.extend(
                    suffix_choices
                        .into_iter()
                        .filter(|c| c.example.contains(&format!(": {name} of "))),
                );
                if by_presence {
                    out.chosen_by_presence.push(PresenceChoice {
                        reason: GENDER_BY_PRESENCE,
                        example: format!("protector {equipped}: {name}"),
                    });
                }
                if !out.parts.contains(name) {
                    out.parts.push(name.clone());
                }
            }
            [] => out.unresolved.push(format!(
                "protector {equipped}: no {prefix} part binder for model {model}, gender mode {gender}"
            )),
            _ => out.unresolved.push(format!(
                "protector {equipped}: ambiguous part binders {}",
                candidates.join(", ")
            )),
        }
    }
    Ok(out)
}

/// Every authored outfit for the character `model`, optionally of one NPC param row `npc`, in map/part order, including
/// distinct placements that share an initializer. The caller picks among the placements.
pub fn npc_outfits(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
    dictionary: &NameDictionary,
    model: &str,
    npc: Option<i32>,
) -> Result<Vec<NpcOutfit>> {
    let placements: Vec<_> = npc_placements(archive, decoder, dictionary)?
        .into_iter()
        .filter(|p| p.model == model && npc.is_none_or(|npc| p.row == npc))
        .collect();
    outfits_from_placements(install, archive, decoder, placements)
}

/// All map-authored initializer outfits, so that a survey of many characters reads the params once rather than
/// rereading all maps per character. Retains map/placement/NPC/initializer on every row, including unreadable outfits and their reasons.
pub fn npc_outfit_catalog(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
    dictionary: &NameDictionary,
) -> Result<Vec<NpcOutfit>> {
    let placements = npc_placements(archive, decoder, dictionary)?;
    outfits_from_placements(install, archive, decoder, placements)
}

fn outfits_from_placements(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
    placements: Vec<NpcPlacement>,
) -> Result<Vec<NpcOutfit>> {
    if placements.iter().all(|p| p.character_init.is_none()) {
        return Ok(placements
            .into_iter()
            .map(|p| NpcOutfit {
                npc: p.row,
                init: None,
                map: p.map,
                placement: p.part,
                parts: Vec::new(),
                unresolved: Vec::new(),
                chosen_by_presence: Vec::new(),
            })
            .collect());
    }
    let initializers = read_param(install, archive, decoder, "CharaInitParam.param")?;
    let protectors = read_param(install, archive, decoder, "EquipParamProtector.param")?;
    placements
        .into_iter()
        .map(|p| {
            let Some(init) = p.character_init else {
                return Ok(NpcOutfit {
                    npc: p.row,
                    init: None,
                    map: p.map,
                    placement: p.part,
                    parts: Vec::new(),
                    unresolved: Vec::new(),
                    chosen_by_presence: Vec::new(),
                });
            };
            let OutfitParts {
                parts,
                unresolved,
                chosen_by_presence,
            } = resolve_outfit(install.game, &initializers, &protectors, init, |name| {
                archive.contains(&format!("/parts/{name}.partsbnd.dcx"))
            })
            .unwrap_or_else(|why| OutfitParts {
                unresolved: vec![why.to_string()],
                ..Default::default()
            });
            Ok(NpcOutfit {
                npc: p.row,
                init: Some(init),
                map: p.map,
                placement: p.part,
                parts,
                unresolved,
                chosen_by_presence,
            })
        })
        .collect()
}

/// The number of drawable meshes (with vertices and at least one triangle) in the FLVERs of the character `id`
/// (`c2010`) from its `.chrbnd.dcx`. Each FLVER is decoded whole, vertices included, only to count; no materials,
/// textures or animations are read. A zero count marks a rig without geometry; a binder with no FLVER is an error
/// listing the members it holds, not an empty success.
pub fn character_mesh_count(archive: &Archive, decoder: &Decoder<'_>, id: &str) -> Result<usize> {
    if id.len() != 5 || !id.starts_with('c') || !id[1..].bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("character geometry", 0, "expected c and four digits"));
    }
    let path = format!("/chr/{id}.chrbnd.dcx");
    let binder = Binder::read(&archive.read(&path, decoder)?, decoder)?;
    let mut found = false;
    let mut meshes = 0;
    for file in &binder.files {
        if file
            .name
            .as_deref()
            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".flver"))
        {
            found = true;
            let drawn = Model::read(&decoder.decode_if_dcx(&file.bytes)?)?
                .meshes
                .iter()
                .filter(|mesh| {
                    !mesh.vertices.is_empty() && mesh.faces.iter().any(|f| f.triangulate().len() >= 3)
                })
                .count();
            meshes = usize::saturating_add(meshes, drawn);
        }
    }
    if !found {
        return Err(invalid(
            "character geometry",
            0,
            format!(
                "{path} has no FLVER; holds {:?}",
                binder
                    .files
                    .iter()
                    .map(|f| f.name.as_deref().unwrap_or("<unnamed>"))
                    .collect::<Vec<_>>()
            ),
        ));
    }
    Ok(meshes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-row param of `game`'s layout: `CHARACTER_INIT_PARAM` or `EQUIP_PARAM_PROTECTOR_ST`.
    fn param(game: Game, init: bool, id: i32, data: impl FnOnce(&mut Vec<u8>)) -> Param {
        let l = OutfitLayout::of(game);
        let ((data_version, row_size), param_type) = if init {
            (l.initializer, "CHARACTER_INIT_PARAM")
        } else {
            (l.protector, "EQUIP_PARAM_PROTECTOR_ST")
        };
        let mut bytes = vec![0; row_size];
        data(&mut bytes);
        Param {
            param_type: param_type.into(),
            data_version,
            unk06: 1,
            row_size,
            rows: vec![Row {
                id,
                name: None,
                data: bytes,
            }],
        }
    }

    /// An initializer with `slot` (0 head .. 3 legs) naming `protector` and the other slots unset.
    fn initializer(game: Game, id: i32, sex: u8, slot: usize, protector: i32) -> Param {
        param(game, true, id, |d| {
            for at in [32, 36, 40, 44] {
                d[at..at + 4].copy_from_slice(&(-1i32).to_le_bytes());
            }
            d[32 + 4 * slot..36 + 4 * slot].copy_from_slice(&protector.to_le_bytes());
            d[SEX] = sex;
        })
    }

    /// A protector of `model`, `category` and `gender` in `game`'s layout.
    fn protector(game: Game, id: i32, model: u16, category: u8, gender: u8) -> Param {
        let l = OutfitLayout::of(game);
        param(game, false, id, |d| {
            d[l.model..l.model + 2].copy_from_slice(&model.to_le_bytes());
            d[l.category] = category;
            d[l.gender] = gender;
        })
    }

    fn reasons(parts: &OutfitParts) -> Vec<&'static str> {
        parts.chosen_by_presence.iter().map(|c| c.reason).collect()
    }

    #[test]
    fn initializer_equipment_follows_protector_model_not_npc_or_equipment_id() {
        let g = Game::Sekiro;
        let initializers = initializer(g, 9000, 0, 0, 100000);
        // the Wolf's head: category 3 is the face, gender mode 3 takes the initializer's sex
        let protectors = protector(g, 100000, 200, 3, 3);
        let r = resolve_outfit(g, &initializers, &protectors, 9000, |p| p == "fc_m_0200_l").expect("outfit");
        assert_eq!(r.parts, ["fc_m_0200_l"]);
        assert!(r.unresolved.is_empty());
        assert!(reasons(&r).is_empty());
        assert!(resolve_outfit(g, &initializers, &protectors, 10002020, |_| true).is_err());
        // a head part (category 5) is `hd_`, not the face, even where the face binder exists
        let head = protector(g, 100000, 200, 5, 3);
        let r = resolve_outfit(g, &initializers, &head, 9000, |p| {
            matches!(p, "fc_m_0200_l" | "hd_m_0200_l")
        })
        .expect("outfit");
        assert_eq!(r.parts, ["hd_m_0200_l"]);
    }

    #[test]
    fn ambiguous_or_missing_part_bindings_are_explained_without_inventing_a_model() {
        let g = Game::Sekiro;
        let initializers = initializer(g, 7, 0, 1, 1);
        let protectors = protector(g, 1, 0, 2, 0);
        let r = resolve_outfit(g, &initializers, &protectors, 7, |_| true).expect("fields");
        assert!(r.parts.is_empty());
        assert!(r.unresolved.iter().any(|w| w.contains("ambiguous")));
        let r = resolve_outfit(g, &initializers, &protectors, 7, |_| false).expect("fields");
        assert!(r.parts.is_empty());
        assert!(r.unresolved.iter().any(|w| w.contains("no bd part")));
    }

    #[test]
    fn unspecified_sekiro_gender_follows_unique_held_binder_not_npc_player_sex() {
        let g = Game::Sekiro;
        let initializers = initializer(g, 15030, 1, 1, 101000);
        let protectors = protector(g, 101000, 9040, 2, 1);
        let r = resolve_outfit(g, &initializers, &protectors, 15030, |name| name == "bd_m_9040_l")
            .expect("fields");
        assert_eq!(r.parts, ["bd_m_9040_l"]);
        assert!(r.unresolved.is_empty());
        assert_eq!(reasons(&r), [GENDER_BY_PRESENCE]);
        let r = resolve_outfit(g, &initializers, &protectors, 15030, |name| {
            matches!(name, "bd_m_9040_l" | "bd_f_9040_l")
        })
        .expect("fields");
        assert!(r.parts.is_empty());
        assert!(r.unresolved.iter().any(|s| s.contains("ambiguous")));
    }

    #[test]
    fn a_protector_in_a_slot_of_another_category_is_refused() {
        for (g, wrong) in [
            (Game::Sekiro, 0),
            (Game::Sekiro, 5),
            (Game::EldenRing, 4),
            (Game::EldenRing, 5),
        ] {
            // a body slot (1) naming a protector of another category: no part, the reason named
            let initializers = initializer(g, 1, 0, 1, 10);
            let protectors = protector(g, 10, 100, wrong, 4);
            let r = resolve_outfit(g, &initializers, &protectors, 1, |_| true).expect("fields");
            assert!(r.parts.is_empty(), "{g:?} {wrong}");
            assert!(
                r.unresolved
                    .iter()
                    .any(|w| w.contains(&format!("equipModelCategory {wrong} is not a bd part"))),
                "{:?}",
                r.unresolved
            );
        }
    }

    #[test]
    fn elden_rings_part_letter_follows_the_gender_mode_and_never_the_shared_a() {
        let g = Game::EldenRing;
        // head slot, category 5 -> hd; gender 4 (none) and 1 (male) are `m`, 2 (female) is `f`
        for (gender, letter) in [(4u8, "m"), (1, "m"), (2, "f")] {
            let (init, prot) = (initializer(g, 1, 1, 0, 10), protector(g, 10, 7, 5, gender));
            let name = format!("hd_{letter}_0007_l");
            let r = resolve_outfit(g, &init, &prot, 1, |n| n == name || n == "hd_a_0007_l").expect("fields");
            assert_eq!(r.parts, [name], "gender {gender}");
            assert!(reasons(&r).is_empty());
        }
        // the plain name is taken when the `_l` sibling is not held, and said
        let (init, prot) = (initializer(g, 1, 0, 3, 10), protector(g, 10, 7, 6, 4));
        let r = resolve_outfit(g, &init, &prot, 1, |n| n == "lg_m_0007").expect("fields");
        assert_eq!(r.parts, ["lg_m_0007"]);
        assert_eq!(reasons(&r), [LEVEL_SUFFIX_BY_PRESENCE]);
        // gender 3 splits by a sex whose letter is not known: the one held binder, or no choice
        let (init, prot) = (initializer(g, 1, 0, 2, 10), protector(g, 10, 7, 1, 3));
        let r = resolve_outfit(g, &init, &prot, 1, |n| n == "am_f_0007_l").expect("fields");
        assert_eq!(r.parts, ["am_f_0007_l"]);
        assert_eq!(reasons(&r), [GENDER_BY_PRESENCE]);
        let r = resolve_outfit(g, &init, &prot, 1, |n| matches!(n, "am_f_0007_l" | "am_m_0007_l"))
            .expect("fields");
        assert!(r.parts.is_empty() && r.unresolved.iter().any(|w| w.contains("ambiguous")));
        // an unknown gender mode is reported, not guessed
        let (init, prot) = (initializer(g, 1, 0, 0, 10), protector(g, 10, 7, 5, 9));
        let r = resolve_outfit(g, &init, &prot, 1, |_| true).expect("fields");
        assert!(
            r.parts.is_empty()
                && r.unresolved
                    .iter()
                    .any(|w| w.contains("unknown equipModelGender 9"))
        );
    }

    #[test]
    fn one_games_params_are_not_read_with_the_others_offsets() {
        let (init, prot) = (
            initializer(Game::Sekiro, 1, 0, 0, 10),
            protector(Game::Sekiro, 10, 7, 5, 4),
        );
        let e = resolve_outfit(Game::EldenRing, &init, &prot, 1, |_| true)
            .unwrap_err()
            .to_string();
        assert!(e.contains("Elden Ring"), "{e}");
    }

    #[test]
    fn a_word_past_the_row_is_an_error_at_its_offset() {
        let row = Row {
            id: 3,
            name: None,
            data: vec![1, 0, 0, 0],
        };
        assert_eq!(word(&row, 0).unwrap(), 1);
        for at in [1, 4, usize::MAX - 1, usize::MAX] {
            let e = word(&row, at).unwrap_err().to_string();
            assert_eq!(e, format!("NPC outfit at {at:#x}: short row 3"));
        }
    }
}
