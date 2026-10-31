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
    /// Every authored binder spelling and its archive presence, before selection.
    pub candidates: Option<OutfitCandidates>,
    /// Why the initializer's records could not be read; no selection is performed here.
    pub unresolved: Vec<String>,
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
    /// Decode the four protector references and stored sex byte without resolving any part spelling.
    pub fn read_initializer(self, row: &Row) -> Result<InitializerRecord> {
        let view = crate::bytes::View {
            data: &row.data,
            format: "CharaInitParam",
            be: false,
            base: 0,
        };
        let mut protectors = [0; 4];
        for (index, value) in protectors.iter_mut().enumerate() {
            *value = word(row, view.element(32, index, 4)?)?;
        }
        Ok(InitializerRecord {
            id: row.id,
            protectors,
            sex: view.u8(SEX)?,
        })
    }

    /// Decode the stored model, category and gender-mode fields; no archive candidate is chosen.
    pub fn read_protector(self, row: &Row) -> Result<ProtectorRecord> {
        let view = crate::bytes::View {
            data: &row.data,
            format: "EquipParamProtector",
            be: false,
            base: 0,
        };
        Ok(ProtectorRecord {
            id: row.id,
            model: view.u16(self.model)?,
            category: view.u8(self.category)?,
            gender: view.u8(self.gender)?,
        })
    }

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

/// Supported initializer fields, before any presentation or part-level selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct InitializerRecord {
    /// The initializer's PARAM identifier.
    pub id: i32,
    /// Head, body, arms and legs, in stored slot order; negative values name no row.
    pub protectors: [i32; 4],
    /// `npcPlayerSex`, retained as stored without assuming its meaning for every game.
    pub sex: u8,
}

/// Supported protector fields, without selecting any binder spelling or form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProtectorRecord {
    /// The protector's PARAM identifier.
    pub id: i32,
    /// `equipModelId`.
    pub model: u16,
    /// `equipModelCategory`.
    pub category: u8,
    /// `equipModelGender`, including unknown values.
    pub gender: u8,
}

impl InitializerRecord {
    /// Stored protector references paired with their slot names; no negative sentinel is removed.
    pub fn slots(self) -> [(&'static str, i32); 4] {
        let [head, body, arms, legs] = self.protectors;
        [("hd", head), ("bd", body), ("am", arms), ("lg", legs)]
    }
}

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

/// One possible binder spelling. Presence is a file fact, not a preference.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PartCandidate {
    /// Part stem, without its binder extension or archive folder.
    pub name: String,
    /// Stored-mode-permitted spelling (`m`, `f`, or `a`).
    pub gender: String,
    /// Whether this is the `_l` spelling rather than the unsuffixed spelling.
    pub low_suffix: bool,
    /// Whether the supplied archive inventory contains the binder.
    pub present: bool,
}

/// All supported spellings of one authored protector slot, without selecting any form.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct OutfitSlotCandidates {
    /// Authored protector row identifier.
    pub protector: i32,
    /// Slot prefix (`hd`, `bd`, `am`, or `lg`).
    pub slot: String,
    /// Prefix established by the slot/category pair (Sekiro's face may be `fc`).
    pub prefix: String,
    /// Stored model identifier.
    pub model: u16,
    /// Stored gender mode.
    pub gender_mode: u8,
    /// Whether the supported mode leaves several gender spellings possible.
    pub gender_by_presence: bool,
    /// Every supported gender and level spelling, including absent files.
    pub candidates: Vec<PartCandidate>,
}

/// Initializer equipment facts and file inventory, before presentation and level policy.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct OutfitCandidates {
    /// The authored initializer identifier.
    pub initializer: i32,
    /// Nonempty protector slots, in initializer order.
    pub slots: Vec<OutfitSlotCandidates>,
    /// Unsupported fields or dangling row references; absence or ambiguity is left to a selection pass.
    pub unresolved: Vec<String>,
}

/// Read every supported spelling and record its presence. No gender or level suffix is preferred.
pub fn outfit_candidates(
    game: Game,
    initializers: &Param,
    protectors: &Param,
    id: i32,
    has: impl Fn(&str) -> bool,
) -> Result<OutfitCandidates> {
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
    let init = layout.read_initializer(row(initializers, "CharaInitParam", id)?)?;
    let sex = if game == Game::Sekiro {
        match init.sex {
            0 => "m",
            1 => "f",
            _ => return Err(invalid("NPC outfit", SEX, "unknown npcPlayerSex")),
        }
    } else {
        ""
    };
    let mut out = OutfitCandidates {
        initializer: id,
        ..Default::default()
    };
    for (slot, equipped) in init.slots() {
        if equipped < 0 {
            continue;
        }
        let Some(row) = protectors.rows.iter().find(|row| row.id == equipped) else {
            out.unresolved.push(format!(
                "protector {equipped}: EquipParamProtector has no such row"
            ));
            continue;
        };
        let protector = layout.read_protector(row)?;
        let Some(prefix) = part_prefix(game, slot, protector.category) else {
            out.unresolved.push(format!(
                "protector {equipped}: equipModelCategory {} is not a {slot} part",
                protector.category
            ));
            continue;
        };
        let (genders, by_presence): (&[&str], bool) = match (game, protector.gender) {
            (Game::Sekiro, 3) => (std::slice::from_ref(&sex), false),
            (Game::Sekiro, 4) => (&["a"], false),
            (Game::Sekiro, 0 | 1) => (&["m", "f", "a"], true),
            (Game::EldenRing, 1 | 4) => (&["m"], false),
            (Game::EldenRing, 2) => (&["f"], false),
            (Game::EldenRing, 3) => (&["m", "f"], true),
            _ => {
                out.unresolved.push(format!(
                    "protector {equipped}: unknown equipModelGender {}",
                    protector.gender
                ));
                continue;
            }
        };
        let mut candidates = Vec::new();
        for gender in genders {
            for (suffix, low_suffix) in [("_l", true), ("", false)] {
                let name = format!("{prefix}_{gender}_{:04}{suffix}", protector.model);
                let present = has(&name);
                candidates.push(PartCandidate {
                    name,
                    gender: (*gender).to_owned(),
                    low_suffix,
                    present,
                });
            }
        }
        out.slots.push(OutfitSlotCandidates {
            protector: equipped,
            slot: slot.to_owned(),
            prefix: prefix.to_owned(),
            model: protector.model,
            gender_mode: protector.gender,
            gender_by_presence: by_presence,
            candidates,
        });
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
    let params = if placements.iter().any(|p| p.character_init.is_some()) {
        Some((
            read_param(install, archive, decoder, "CharaInitParam.param")?,
            read_param(install, archive, decoder, "EquipParamProtector.param")?,
        ))
    } else {
        None
    };
    Ok(placements
        .into_iter()
        .map(|p| {
            let mut unresolved = Vec::new();
            let candidates = match (p.character_init, params.as_ref()) {
                (Some(init), Some((initializers, protectors))) => {
                    match outfit_candidates(install.game, initializers, protectors, init, |name| {
                        archive.contains(&format!("/parts/{name}.partsbnd.dcx"))
                    }) {
                        Ok(raw) => Some(raw),
                        Err(why) => {
                            unresolved.push(why.to_string());
                            None
                        }
                    }
                }
                _ => None,
            };
            NpcOutfit {
                npc: p.row,
                init: p.character_init,
                map: p.map,
                placement: p.part,
                candidates,
                unresolved,
            }
        })
        .collect())
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

    #[test]
    fn typed_initializer_and_protector_fields_keep_negative_and_unknown_values() {
        for game in [Game::Sekiro, Game::EldenRing] {
            let init = initializer(game, 9000, 1, 1, 79113);
            let prot = protector(game, 79113, 1379, 2, 255);
            let layout = OutfitLayout::of(game);
            let row = layout.read_initializer(&init.rows[0]).unwrap();
            assert_eq!(row.protectors, [-1, 79113, -1, -1]);
            assert_eq!(row.sex, 1);
            assert_eq!(row.slots()[1], ("bd", 79113));
            assert_eq!(
                layout.read_protector(&prot.rows[0]).unwrap(),
                ProtectorRecord {
                    id: 79113,
                    model: 1379,
                    category: 2,
                    gender: 255
                }
            );
            let mut short = init.rows[0].clone();
            short.data.truncate(35);
            assert!(layout.read_initializer(&short).is_err());
            let mut short = prot.rows[0].clone();
            short.data.truncate(layout.gender);
            assert!(layout.read_protector(&short).is_err());
        }
    }

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
