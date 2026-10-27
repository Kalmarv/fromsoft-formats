//! Map models, part placements and map-local offsets of Sekiro (MSBS) and Elden Ring (MSBE).
//!
//! Part coordinates are already in the map's frame. Sekiro's MapOffset event
//! instead places map-local resources (for example collision); never apply it a
//! second time to parts. Euler angles are degrees, applied X, then Z, then Y.
//! The generations share the section chain and the record envelopes; the [`Generation`] the file's section chain
//! names supplies what differs.
use crate::affine::narrow;
use crate::{Result, bytes::View, invalid};

/// How a part's point-light shadow byte is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagByte {
    /// 0 or 1.
    Bool,
    /// 0 (off) or 0xff (-1, on).
    ZeroOrMinusOne,
}
/// One MSB generation: its section chain, record layouts and kind codes. The section chain (its names, in order)
/// identifies the generation. The number each section stores is a revision (Elden Ring's maps store several with
/// one layout; Sekiro's sections within one file differ): it is kept in [`Section::version`] and does not choose a
/// layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Generation {
    /// The generation's common name (`MSBS`, `MSBE`).
    pub name: &'static str,
    /// Section names in file order.
    pub sections: &'static [&'static str],
    /// Indices of sections that must have no records.
    pub empty: &'static [usize],
    /// Accepted model kind codes.
    pub model_kinds: &'static [u32],
    /// Part kind codes.
    pub part_kinds: &'static [(u32, PartKind)],
    /// Part record header size in bytes, and where it keeps the kind and the model index (u32 each).
    pub part_header: usize,
    /// Offset in a part record of its kind code (u32).
    pub part_kind: usize,
    /// Offset in a part record of its model index (u32; `u32::MAX` for none).
    pub part_model: usize,
    /// Header bytes with fixed contents: (offset, bytes).
    pub part_fixed: &'static [(usize, &'static [u8])],
    /// The header's payload offset slots: offset of the first u64 slot and the slot count. The entity block (96) and
    /// type data (104) slots are among them in every generation and must be set.
    pub part_payloads: (usize, usize),
    /// Entity block size, and a range of it that must be zero.
    pub entity_block: usize,
    /// A range of the entity block (offset, length) that must be zero, where the generation has one.
    pub entity_zero: Option<(usize, usize)>,
    /// How the generation stores the point-light shadow byte.
    pub point_light_shadow: FlagByte,
    /// Event type code of MapOffset records, in generations that have them.
    pub map_offset_event: Option<u32>,
    /// Region type code of checkpoint/warp spawn points, where known.
    pub spawn_region: Option<u32>,
}
impl Generation {
    /// Sekiro (MSBS): eight sections, ending with part poses and bone names.
    pub const SEKIRO: Self = Self {
        name: "MSBS",
        sections: &[
            "MODEL_PARAM_ST",
            "EVENT_PARAM_ST",
            "POINT_PARAM_ST",
            "ROUTE_PARAM_ST",
            "LAYER_PARAM_ST",
            "PARTS_PARAM_ST",
            "MAPSTUDIO_PARTS_POSE_ST",
            "MAPSTUDIO_BONE_NAME_STRING",
        ],
        empty: &[4, 6, 7],
        model_kinds: &[0, 1, 2, 4, 5],
        part_kinds: &[
            (0, PartKind::MapPiece),
            (1, PartKind::Object),
            (2, PartKind::Enemy),
            (4, PartKind::Player),
            (5, PartKind::Collision),
            (9, PartKind::DummyObject),
            (10, PartKind::DummyEnemy),
            (11, PartKind::ConnectCollision),
        ],
        part_header: 160,
        part_kind: 8,
        part_model: 16,
        part_fixed: &[(20, &[0; 4]), (68, &[255; 8]), (76, &[0; 4]), (136, &[0; 24])],
        part_payloads: (80, 7),
        entity_block: 0x54,
        entity_zero: Some((0x44, 16)),
        point_light_shadow: FlagByte::Bool,
        map_offset_event: Some(9),
        spawn_region: Some(8),
    };
    /// Elden Ring (MSBE): six sections ending with parts; assets take the place of objects. It has no MapOffset
    /// events; its spawn points are region type 8, as Sekiro's (the region header is laid out alike: name offset,
    /// type at 8, position at 20, rotation at 32).
    pub const ELDEN_RING: Self = Self {
        name: "MSBE",
        sections: &[
            "MODEL_PARAM_ST",
            "EVENT_PARAM_ST",
            "POINT_PARAM_ST",
            "ROUTE_PARAM_ST",
            "LAYER_PARAM_ST",
            "PARTS_PARAM_ST",
        ],
        empty: &[4],
        model_kinds: &[0, 2, 4, 5, 10],
        part_kinds: &[
            (0, PartKind::MapPiece),
            (2, PartKind::Enemy),
            (4, PartKind::Player),
            (5, PartKind::Collision),
            (9, PartKind::DummyAsset),
            (10, PartKind::DummyEnemy),
            (11, PartKind::ConnectCollision),
            (13, PartKind::Asset),
        ],
        part_header: 192,
        part_kind: 12,
        part_model: 20,
        part_fixed: &[(76, &[0; 4]), (168, &[0; 24])],
        part_payloads: (80, 11),
        entity_block: 0x40,
        entity_zero: None,
        point_light_shadow: FlagByte::ZeroOrMinusOne,
        map_offset_event: None,
        spawn_region: Some(8),
    };
    /// Both generations.
    pub const ALL: [&'static Self; 2] = [&Self::SEKIRO, &Self::ELDEN_RING];
    /// The generation whose section chain is `names`.
    pub fn find<S: AsRef<str>>(names: &[S]) -> Option<&'static Self> {
        Self::ALL.into_iter().find(|g| {
            g.sections.len() == names.len() && g.sections.iter().zip(names).all(|(a, b)| *a == b.as_ref())
        })
    }
}
/// The section names, read before choosing a generation: each section's name pointer sits at +8 and its last
/// offset points at the next section (zero after the last). At most 16 sections.
fn section_chain(v: &View<'_>) -> Result<Vec<String>> {
    let mut names = Vec::new();
    let mut start = 16;
    loop {
        if names.len() == 16 {
            return Err(invalid(v.format, start, "section chain too long"));
        }
        let table = SectionTable::at(v, start)?;
        names.push(text(v, v.size(v.at(start, 8)?)?)?);
        if table.next == 0 {
            return Ok(names);
        }
        if table.next <= start {
            return Err(invalid(v.format, table.next_at, "section chain goes backwards"));
        }
        if table.next >= v.data.len() {
            return Err(invalid(
                v.format,
                table.next_at,
                format!(
                    "next section at {:#x} is beyond the {} bytes of the file",
                    table.next,
                    v.data.len()
                ),
            ));
        }
        start = table.next;
    }
}

/// A section's header: its entry table, whose last slot points at the next section.
struct SectionTable {
    /// The slot count (the entries and the next pointer).
    slots: usize,
    /// The table's size in bytes.
    bytes: usize,
    /// Where the next-section pointer is stored.
    next_at: usize,
    /// The next section's offset, or zero after the last.
    next: usize,
}
impl SectionTable {
    /// The table of the section at `start`, which must lie within the data.
    fn at(v: &View<'_>, start: usize) -> Result<Self> {
        let count_at = v.at(start, 4)?;
        let slots =
            usize::try_from(v.u32(count_at)?).map_err(|_| invalid(v.format, count_at, "size overflow"))?;
        let Some(last) = slots.checked_sub(1) else {
            return Err(invalid(v.format, count_at, "section has no next pointer"));
        };
        let bytes = slots
            .checked_mul(8)
            .ok_or_else(|| invalid(v.format, count_at, "section size overflow"))?;
        let table_at = v.at(start, 16)?;
        v.slice(table_at, bytes)?;
        // `last * 8` is below `bytes`, which fits
        let next_at = v.at(table_at, last.saturating_mul(8))?;
        Ok(Self {
            slots,
            bytes,
            next_at,
            next: v.size(next_at)?,
        })
    }
}

/// A model a layout names (a map piece, object, character, collision).
#[derive(Clone, Debug)]
pub struct Model {
    /// The model's name (`m10_00_00_00_000000`, `c3000`).
    pub name: String,
    /// The model kind code as stored.
    pub kind: u32,
}
/// What a part places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    /// A piece of map geometry.
    MapPiece,
    /// A placed object (Sekiro).
    Object,
    /// An enemy or other character.
    Enemy,
    /// A player start.
    Player,
    /// Collision geometry.
    Collision,
    /// A placeholder object (Sekiro).
    DummyObject,
    /// A placeholder enemy.
    DummyEnemy,
    /// A connection between collisions.
    ConnectCollision,
    /// Elden Ring's placed asset geometry (its counterpart of Sekiro's objects).
    Asset,
    /// A placeholder asset (Elden Ring).
    DummyAsset,
}
/// One part of a map layout: a placed model, character or collision, as stored.
#[derive(Clone, Debug)]
pub struct Part {
    /// Original file order, stable even when names repeat.
    pub index: usize,
    /// The part's name.
    pub name: String,
    /// What it places.
    pub kind: PartKind,
    /// Its model's index in [`MapLayout::models`], if it has one.
    pub model: Option<usize>,
    /// Collision-part hit filter; absent on other part kinds.
    pub hit_filter: Option<u8>,
    /// An enemy's or dummy enemy's NPC param row (`NPCParamID`, type data +12 in both generations: two zero words,
    /// the think param row, then the NPC row, as SoulsFormats' `MSBS`/`MSBE` `EnemyBase.ReadTypeData` read them);
    /// absent on other part kinds.
    pub npc_param: Option<i32>,
    /// Its position, as stored.
    pub position: [f32; 3],
    /// Its rotation in degrees (X, Y, Z), as stored; see [`euler_matrix`] for the order they apply in.
    pub rotation_degrees: [f32; 3],
    /// Its scale, as stored.
    pub scale: [f32; 3],
    /// The part's own entity ID as stored (event scripts address it; -1 or 0: none).
    pub entity_id: i32,
    /// The entity groups it belongs to, as stored (unused slots -1 or 0); a script addressing a group addresses
    /// every member.
    pub entity_group_ids: [i32; 8],
    /// Source shadow controls, independent of material and placement state.
    pub shadows: ShadowFlags,
}
/// Raw entity-block shadow controls. Renderer-specific bytes are retained
/// uninterpreted. `only && !cast` is contradictory: retain and count it rather
/// than guessing a consumer's shadow mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ShadowFlags {
    /// Boolean byte at entity-block offset 0x0c; explicit false is retained.
    pub cast: bool,
    /// Boolean byte at entity-block offset 0x11; explicit false is retained.
    pub receive: bool,
    /// Boolean byte at entity-block offset 0x12, retained independently of `cast`.
    pub only: bool,
    /// Byte at entity-block offset 0x0a for point-light shadow casting (0/1 in MSBS, 0/0xff in MSBE).
    pub point_light_cast: bool,
    /// Uninterpreted byte at entity-block offset 0x0d; all byte values are retained.
    pub static_cast: u8,
    /// Uninterpreted byte at entity-block offset 0x0e; all byte values are retained.
    pub cascade3_cast: u8,
}
/// A MapOffset event (Sekiro): where a map-local resource is placed.
#[derive(Clone, Debug)]
pub struct MapOffset {
    /// The event's name.
    pub name: String,
    /// The offset position, as stored.
    pub position: [f32; 3],
    /// The rotation about y in degrees, as stored.
    pub degrees: f32,
}
/// One section of the file: its records as raw bytes. Record envelopes are validated; unrelated gameplay payloads
/// remain opaque.
#[derive(Clone, Debug)]
pub struct Section {
    /// The section's name (`PARTS_PARAM_ST`).
    pub name: String,
    /// The revision the section stores.
    pub version: u32,
    /// Its records, each the bytes from its offset to the next record's (or the section's end).
    pub records: Vec<Vec<u8>>,
}
/// A read map layout (`.msb.dcx` once decoded): the models and parts decoded, the other records retained.
#[derive(Clone, Debug)]
pub struct MapLayout {
    /// The generation the section chain names.
    pub generation: &'static Generation,
    /// The models, in file order.
    pub models: Vec<Model>,
    /// The parts, in file order.
    pub parts: Vec<Part>,
    /// The MapOffset events (none in Elden Ring).
    pub map_offsets: Vec<MapOffset>,
    /// Every section with its raw records, in file order.
    pub sections: Vec<Section>,
}

/// Ground-plane points of the parts drawn as models: map pieces, objects and assets, as stored x and z metres.
/// Enemy, player, collision and placeholder records are excluded. Each is the part's own place, never offset by the
/// map-offset event, which places map-local resources rather than the already-positioned parts.
pub fn drawn_part_points(layout: &MapLayout) -> Result<Vec<[f32; 2]>> {
    layout
        .parts
        .iter()
        .filter(|part| matches!(part.kind, PartKind::MapPiece | PartKind::Object | PartKind::Asset))
        .map(|part| part.matrix().map(|m| [m[3][0], m[3][2]]))
        .collect()
}
/// `N` floats of a region's shape data, read through `at` (the offset of each byte-offset into the data).
fn shape_floats<const N: usize>(r: &View<'_>, at: &dyn Fn(usize) -> Result<usize>) -> Result<[f32; N]> {
    let mut out = [0.; N];
    for (i, f) in out.iter_mut().enumerate() {
        *f = f32_at(r, at(i.saturating_mul(4))?)?;
    }
    Ok(out)
}
fn text(v: &View<'_>, mut o: usize) -> Result<String> {
    let start = o;
    let mut units = Vec::new();
    loop {
        let x = v.u16(o)?;
        o = v.at(o, 2)?;
        if x == 0 {
            break;
        }
        units.push(x);
    }
    String::from_utf16(&units).map_err(|e| invalid(v.format, start, e))
}
fn f32_at(v: &View<'_>, o: usize) -> Result<f32> {
    let f = f32::from_bits(v.u32(o)?);
    if !f.is_finite() {
        return Err(invalid(v.format, o, "non-finite value"));
    }
    Ok(f)
}
fn vector(v: &View<'_>, o: usize) -> Result<[f32; 3]> {
    Ok([f32_at(v, o)?, f32_at(v, v.at(o, 4)?)?, f32_at(v, v.at(o, 8)?)?])
}
fn pointer(v: &View<'_>, o: usize, min: usize) -> Result<usize> {
    let p = v.size(o)?;
    if p < min {
        return Err(invalid(v.format, o, "pointer into record header"));
    }
    v.slice(p, 1)?;
    Ok(p)
}

impl MapLayout {
    /// Parse section envelopes and the fields used by placement import, in the generation the section chain names.
    /// Other gameplay records are retained, without claiming to validate their schemas.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: "MSB",
            be: false,
        };
        v.expect(0, b"MSB \x01\0\0\0\x10\0\0\0\0\0\x01\xff")?;
        let names = section_chain(&v)?;
        let generation = Generation::find(&names)
            .ok_or_else(|| invalid(v.format, 16, format!("unknown section chain {names:?}")))?;
        let last = generation.sections.len().saturating_sub(1);
        let mut next = 16;
        let mut sections = Vec::new();
        for (section_index, &name) in generation.sections.iter().enumerate() {
            let start = next;
            let version = v.u32(start)?;
            let table = SectionTable::at(&v, start)?;
            let (n, bytes) = (table.slots, table.bytes);
            next = table.next;
            let end = if next == 0 { data.len() } else { next };
            if (section_index == last) != (next == 0) || end <= start || end > data.len() {
                return Err(invalid(v.format, table.next_at, "invalid section chain"));
            }
            let header_end = v.at(v.at(start, 16)?, bytes)?;
            if header_end > end {
                return Err(invalid(v.format, start, "section table overlaps next section"));
            }
            let section = View {
                data: v.slice(0, end)?,
                format: v.format,
                be: false,
            };
            let name_at = v.at(start, 8)?;
            let name_offset = v.size(name_at)?;
            if name_offset < header_end || text(&section, name_offset)? != name {
                return Err(invalid(v.format, name_at, "section name mismatch"));
            }
            let mut offsets = Vec::new();
            for i in 0..n.saturating_sub(1) {
                let slot = v.element(v.at(start, 16)?, i, 8)?;
                let p = v.size(slot)?;
                if p < header_end || p >= end || offsets.last().is_some_and(|&last| last >= p) {
                    return Err(invalid(v.format, slot, "invalid entry offset"));
                }
                offsets.push(p);
            }
            offsets.push(end);
            let records = offsets
                .windows(2)
                .map(|r| match r {
                    [from, to] => v.slice(*from, to.saturating_sub(*from)).map(<[u8]>::to_vec),
                    _ => Ok(Vec::new()),
                })
                .collect::<Result<Vec<_>>>()?;
            sections.push(Section {
                name: name.to_owned(),
                version,
                records,
            });
        }
        for &i in generation.empty {
            if sections.get(i).is_none_or(|s| !s.records.is_empty()) {
                return Err(invalid("MSB", 0, "nonempty unsupported section"));
            }
        }
        let mut models = Vec::new();
        for data in section_records(&sections, 0)? {
            let r = View {
                data,
                format: "MSB model",
                be: false,
            };
            r.slice(0, 40)?;
            let name = text(&r, pointer(&r, 0, 40)?)?;
            text(&r, pointer(&r, 16, 40)?)?;
            let type_offset = r.size(32)?;
            if type_offset != 0 {
                r.slice(type_offset, 1)?;
            }
            let kind = r.u32(8)?;
            if !generation.model_kinds.contains(&kind) {
                return Err(invalid(r.format, 8, "model kind"));
            }
            models.push(Model { name, kind });
        }
        let mut parts = Vec::new();
        let header = generation.part_header;
        for (index, data) in section_records(&sections, 5)?.iter().enumerate() {
            let r = View {
                data,
                format: "MSB part",
                be: false,
            };
            r.slice(0, header)?;
            let name = text(&r, pointer(&r, 0, header)?)?;
            text(&r, pointer(&r, 24, header)?)?;
            let code = r.u32(generation.part_kind)?;
            let kind = generation
                .part_kinds
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, k)| *k)
                .ok_or_else(|| invalid(r.format, generation.part_kind, "unknown part kind"))?;
            for &(offset, bytes) in generation.part_fixed {
                r.expect(offset, bytes)?;
            }
            let model = r.u32(generation.part_model)?;
            let model = if model == u32::MAX {
                None
            } else if (model as usize) < models.len() {
                Some(model as usize)
            } else {
                return Err(invalid(r.format, generation.part_model, "model index"));
            };
            let (first, count) = generation.part_payloads;
            for i in 0..count {
                let field = r.element(first, i, 8)?;
                let p = r.size(field)?;
                if p == 0 {
                    if matches!(field, 96 | 104) {
                        return Err(invalid(r.format, field, "missing part payload"));
                    }
                } else {
                    pointer(&r, field, header)?;
                }
            }
            // entity block: ID at +0, shadow controls from +0x0a, eight group IDs at +0x1c
            let entity = r.size(96)?;
            r.slice(entity, generation.entity_block)?;
            if let Some((offset, len)) = generation.entity_zero {
                r.expect(r.at(entity, offset)?, &vec![0; len])?;
            }
            let i32_at = |o: usize| r.u32(o).map(|v| v as i32);
            let entity_id = i32_at(entity)?;
            let mut entity_group_ids = [0i32; 8];
            for (k, g) in entity_group_ids.iter_mut().enumerate() {
                *g = i32_at(r.element(r.at(entity, 0x1c)?, k, 4)?)?;
            }
            parts.push(Part {
                index,
                name,
                kind,
                model,
                entity_id,
                entity_group_ids,
                shadows: ShadowFlags {
                    cast: r.boolean(r.at(entity, 0x0c)?)?,
                    receive: r.boolean(r.at(entity, 0x11)?)?,
                    only: r.boolean(r.at(entity, 0x12)?)?,
                    point_light_cast: match generation.point_light_shadow {
                        FlagByte::Bool => r.boolean(r.at(entity, 0x0a)?)?,
                        FlagByte::ZeroOrMinusOne => match r.u8_at(entity, 0x0a)? {
                            0 => false,
                            0xff => true,
                            _ => {
                                return Err(invalid(
                                    r.format,
                                    r.at(entity, 0x0a)?,
                                    "point-light shadow byte",
                                ));
                            }
                        },
                    },
                    static_cast: r.u8_at(entity, 0x0d)?,
                    cascade3_cast: r.u8_at(entity, 0x0e)?,
                },
                hit_filter: if kind == PartKind::Collision {
                    Some(r.u8(r.size(104)?)?)
                } else {
                    None
                },
                npc_param: if matches!(kind, PartKind::Enemy | PartKind::DummyEnemy) {
                    let data = r.size(104)?;
                    r.expect(data, &[0; 8])?;
                    Some(i32_at(r.at(data, 12)?)?)
                } else {
                    None
                },
                position: vector(&r, 32)?,
                rotation_degrees: vector(&r, 44)?,
                scale: vector(&r, 56)?,
            });
        }
        let mut map_offsets = Vec::new();
        for data in section_records(&sections, 1)? {
            let r = View {
                data,
                format: "MSB event",
                be: false,
            };
            r.slice(0, 40)?;
            if Some(r.u32(12)?) == generation.map_offset_event {
                let name = text(&r, pointer(&r, 0, 40)?)?;
                let base = pointer(&r, 24, 40)?;
                r.slice(base, 16)?;
                let p = pointer(&r, 32, 40)?;
                map_offsets.push(MapOffset {
                    name,
                    position: vector(&r, p)?,
                    degrees: f32_at(&r, r.at(p, 12)?)?,
                });
            }
        }
        Ok(Self {
            generation,
            models,
            parts,
            map_offsets,
            sections,
        })
    }
}
impl Part {
    /// Its placement in the map's frame ([`euler_matrix`] of its stored position, rotation and scale).
    pub fn matrix(&self) -> Result<[[f32; 3]; 4]> {
        euler_matrix(self.position, self.rotation_degrees, self.scale, self.index)
    }

    /// The IDs an event script can address this part by: its entity ID and its groups, the positive ones only
    /// (stored sentinels never match).
    pub fn entity_ids(&self) -> impl Iterator<Item = i32> + '_ {
        std::iter::once(self.entity_id)
            .chain(self.entity_group_ids)
            .filter(|&id| id > 0)
    }
}

impl MapLayout {
    /// A human enemy's equipment selector, distinct from NPCParamID. SoulsFormats MSBS/MSBE
    /// EnemyBase.ReadTypeData both read CharaInitID at type-data +0x18, after think/NPC/talk and platoon.
    /// A negative stored ID means no initializer; non-enemies have no such field.
    pub fn character_init(&self, index: usize) -> Result<Option<i32>> {
        let part = self
            .parts
            .get(index)
            .ok_or_else(|| invalid("MSB part", index, "no part"))?;
        if !matches!(part.kind, PartKind::Enemy | PartKind::DummyEnemy) {
            return Ok(None);
        }
        let data = self
            .sections
            .get(5)
            .and_then(|s| s.records.get(index))
            .ok_or_else(|| invalid("MSB part", index, "no source record"))?;
        let r = View {
            data,
            format: "MSB character initializer",
            be: false,
        };
        let at = r.size(104)?;
        r.expect(at, &[0; 8])?;
        let id = r.u32_at(at, 0x18)? as i32;
        Ok((id >= 0).then_some(id))
    }
}

/// The affine map (columns: the x, y and z axes, then the translation) a layout record's stored place states: scale,
/// X-Z-Y Euler rotation (degrees), then translation, in the map's own frame as the game stores it (y up, z toward the
/// viewer; no handedness change). Trigonometry and matrix products use f64 and round once to each f32 basis component;
/// the translation is the stored one exactly. `index` names the record in errors.
pub fn euler_matrix(
    position: [f32; 3],
    rotation_degrees: [f32; 3],
    scale: [f32; 3],
    index: usize,
) -> Result<[[f32; 3]; 4]> {
    if !position
        .iter()
        .chain(&rotation_degrees)
        .chain(&scale)
        .all(|x| x.is_finite())
    {
        return Err(invalid("MSB transform", index, "non-finite input"));
    }
    let [x, y, z] = rotation_degrees.map(|a| f64::from(a).to_radians());
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    // Columns of Ry * Rz * Rx, each multiplied by its local-axis scale.
    let basis = [
        [cy * cz, sz, -sy * cz],
        [sy * sx - cy * sz * cx, cz * cx, cy * sx + sy * sz * cx],
        [cy * sz * sx + sy * cx, -cz * sx, cy * cx - sy * sz * sx],
    ];
    let [basis_x, basis_y, basis_z] = basis;
    let [scale_x, scale_y, scale_z] = scale;
    let axis = |col: [f64; 3], s: f32| col.map(|x| narrow(x * f64::from(s)));
    let cols = [
        axis(basis_x, scale_x),
        axis(basis_y, scale_y),
        axis(basis_z, scale_z),
        position,
    ];
    if !cols.iter().flatten().all(|x| x.is_finite()) {
        return Err(invalid("MSB transform", index, "basis overflow"));
    }
    Ok(cols)
}
impl MapOffset {
    /// The map offset's placement of the map in the common frame ([`euler_matrix`] of its position and its rotation
    /// about y). It places map-local data only; parts are not compounded with it.
    pub fn matrix(&self) -> Result<[[f32; 3]; 4]> {
        euler_matrix(self.position, [0., self.degrees, 0.], [1.; 3], 0)
    }
}
/// The graphics config a part stores (SoulsFormats' `GparamConfig`, the part header's slot at +112): the value IDs it
/// takes from the groups of the draw-param file in force there. `light_set` selects the value set of the `LightSet
/// ParamEditor` group, `fog` of `FogParamEditor`, `light_scattering` of `LightScattering : ParamEditor` and `env_map` of
/// `Env Map:Editor` ([`crate::gparam`]). A collision part also stores a scene config (the slot at +120): four signed
/// bytes `scene` that choose draw-param files by the hundreds digit of their ID in Elden Ring (SoulsFormats'
/// `GparamSubID_Base` and three overrides) and that Sekiro's SoulsFormats names `EventIDs` and does not explain; they
/// are kept as stored, -1 meaning none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartGparam {
    /// The part's index in the layout's parts ([`Part::index`]).
    pub part: usize,
    /// The value set of the `LightSet ParamEditor` group.
    pub light_set: i32,
    /// The value set of `FogParamEditor`.
    pub fog: i32,
    /// The value set of `LightScattering : ParamEditor`.
    pub light_scattering: i32,
    /// The value set of `Env Map:Editor`.
    pub env_map: i32,
    /// A collision part's four scene bytes, as stored (-1: none).
    pub scene: Option<[i8; 4]>,
}

impl Generation {
    /// Where the four scene bytes sit in a part's scene config: Sekiro's block is 0x3c zero bytes, the four, a float and
    /// three zero words; Elden Ring's is 24 bytes (zeros, a transition time, a zero word), the four, and the rest.
    fn scene_ids_at(&self) -> usize {
        if self.name == Self::SEKIRO.name { 60 } else { 24 }
    }
}

impl MapLayout {
    /// The graphics config of every part that stores one, in part order. Slots are read where the generation puts them
    /// (the header's +112 and +120 in both), bounds-checked.
    pub fn gparam_configs(&self) -> Result<Vec<PartGparam>> {
        let mut out = Vec::new();
        let scene_ids = self.generation.scene_ids_at();
        for part in &self.parts {
            let data = self
                .sections
                .get(5)
                .and_then(|s| s.records.get(part.index))
                .ok_or_else(|| invalid("MSB part", part.index, "no record"))?;
            let r = View {
                data,
                format: "MSB part",
                be: false,
            };
            let config = r.size(112)?;
            if config == 0 {
                continue;
            }
            let word = |o: usize| r.u32(o).map(|v| v as i32);
            let scene = r.size(120)?;
            out.push(PartGparam {
                part: part.index,
                light_set: word(config)?,
                fog: word(r.at(config, 4)?)?,
                light_scattering: word(r.at(config, 8)?)?,
                env_map: word(r.at(config, 12)?)?,
                scene: if scene == 0 {
                    None
                } else {
                    let b = r.slice(r.at(scene, scene_ids)?, 4)?;
                    let &[a, b, c, d] = b else {
                        return Err(invalid(r.format, 120, "scene config"));
                    };
                    Some([a as i8, b as i8, c as i8, d as i8])
                },
            });
        }
        Ok(out)
    }
}

impl MapLayout {
    /// How many parts store a grass config: Elden Ring's header slot at +128 (SoulsFormats' `GrassConfig`, which its
    /// map pieces and assets may carry); Sekiro's layout has no such slot (its +128 slot is another record), so none.
    /// Only whether a part stores one is read; what the config names is [`MapLayout::grass_configs`].
    pub fn grass_parts(&self) -> Result<usize> {
        Ok(self.grass_configs()?.len())
    }

    /// The grass config of every part that stores one, in part order (Elden Ring's only): the six `GrassTypeParam` row
    /// ids SoulsFormats names `GrassParamId0..5` (0 or negative: none named), as stored.
    pub fn grass_configs(&self) -> Result<Vec<PartGrass>> {
        let mut out = Vec::new();
        if self.generation.name != Generation::ELDEN_RING.name {
            return Ok(out);
        }
        for part in &self.parts {
            let data = self
                .sections
                .get(5)
                .and_then(|s| s.records.get(part.index))
                .ok_or_else(|| invalid("MSB part", part.index, "no record"))?;
            let r = View {
                data,
                format: "MSB part",
                be: false,
            };
            let at = r.size(128)?;
            if at == 0 {
                continue;
            }
            let mut ids = [0i32; 6];
            for (i, id) in ids.iter_mut().enumerate() {
                *id = r.u32(r.element(at, i, 4)?)? as i32;
            }
            out.push(PartGrass {
                part: part.index,
                ids,
            });
        }
        Ok(out)
    }
}

/// The grass config a part stores (SoulsFormats' `GrassConfig`, the part header's slot at +128 in Elden Ring's layout):
/// the ids of up to six `GrassTypeParam` rows, whose model, textures and density the game grows on the part. The game
/// scatters the grass itself; the files hold no blade's position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartGrass {
    /// The part's index in the layout's parts ([`Part::index`]).
    pub part: usize,
    /// The six `GrassTypeParam` row ids, as stored (0 or negative: none).
    pub ids: [i32; 6],
}

/// A checkpoint/warp spawn region (type 8), position and rotation as stored in the layout's common frame. Its facing
/// convention and human-readable labels are the caller's to choose.
#[derive(Clone, Debug)]
pub struct SpawnPoint {
    /// The region's name.
    pub name: String,
    /// Its index among the layout's regions, in file order ([`MapLayout::region_names`]).
    pub region: usize,
    /// Its position, as stored.
    pub position: [f32; 3],
    /// Its rotation in degrees, as stored.
    pub rotation_degrees: [f32; 3],
}
impl MapLayout {
    /// The regions' section (`POINT_PARAM_ST`).
    fn regions(&self) -> Result<&Section> {
        self.sections
            .iter()
            .find(|s| s.name == "POINT_PARAM_ST")
            .ok_or_else(|| invalid("MSB", 0, "missing region section"))
    }

    /// Every region's name as stored, in file order, of every region type. Names repeat: a reader that keys regions
    /// by name renames the repeats over this whole list (SoulsFormats: `"<name> {n}"`).
    pub fn region_names(&self) -> Result<Vec<String>> {
        self.regions()?
            .records
            .iter()
            .map(|data| {
                let v = View {
                    data,
                    format: "MSB region",
                    be: false,
                };
                text(&v, pointer(&v, 0, 96)?)
            })
            .collect()
    }

    /// The checkpoint/warp spawn regions, in generations whose region code is known; an error otherwise.
    pub fn spawn_points(&self) -> Result<Vec<SpawnPoint>> {
        let code = self.generation.spawn_region.ok_or_else(|| {
            invalid(
                "MSB",
                0,
                format!("{}: spawn region code not known", self.generation.name),
            )
        })?;
        let mut points = Vec::new();
        for (region, data) in self.regions()?.records.iter().enumerate() {
            let v = View {
                data,
                format: "MSB region",
                be: false,
            };
            if v.u32(8)? != code {
                continue;
            }
            v.slice(0, 96)?;
            points.push(SpawnPoint {
                name: text(&v, pointer(&v, 0, 96)?)?,
                region,
                position: vector(&v, 20)?,
                rotation_degrees: vector(&v, 32)?,
            });
        }
        Ok(points)
    }
}

/// The shape a region marks out, in the region's own space (SoulsFormats' `MSB.Shape`: the shape code at the record's
/// +16 and the shape's floats at the offset in +72, in both generations). The dimensions are metres as stored.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum RegionShape {
    /// Code 0xFFFFFFFF: the region has no shape.
    None,
    /// Code 0: a point.
    Point,
    /// Code 1.
    Circle {
        /// Radius, metres.
        radius: f32,
    },
    /// Code 2.
    Sphere {
        /// Radius, metres.
        radius: f32,
    },
    /// Code 3.
    Cylinder {
        /// Radius, metres.
        radius: f32,
        /// Height, metres.
        height: f32,
    },
    /// Code 4.
    Rectangle {
        /// Width, metres.
        width: f32,
        /// Depth, metres.
        depth: f32,
    },
    /// Code 5.
    Box {
        /// Width, metres.
        width: f32,
        /// Depth, metres.
        depth: f32,
        /// Height, metres.
        height: f32,
    },
    /// Eight references to other regions, by their index in the section and the second word of each as stored.
    Composite {
        /// Each child's region index and the word stored after it.
        children: Vec<(i32, i32)>,
    },
    /// A shape code SoulsFormats does not name; its data is not read.
    Unknown {
        /// The stored shape code.
        code: u32,
    },
}

/// One region of the layout as stored (`POINT_PARAM_ST`): what every type
/// shares, and the type's own data as bytes. SoulsFormats' `MSBS`/`MSBE` `Region` read the header alike: name offset at
/// +0, type at +8, the index at +12, the shape at +16, position at +20, rotation (degrees) at +32, the shape data
/// offset at +72, the entity block offset at +80 (an activation part index, then the entity ID) and the type data
/// offset at +88. What a type's data holds (a sound's id, an effect, a message) is per type and is not read: the
/// bytes are kept, from the type data offset to the record's end (a record that stores more after it, which Elden
/// Ring's does, keeps those bytes here too).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Region {
    /// Its index in the layout's regions, in file order.
    pub index: usize,
    /// The region's name.
    pub name: String,
    /// The type code as stored.
    pub type_code: u32,
    /// The type's name where SoulsFormats names it ([`Generation::region_type`]).
    pub type_name: Option<&'static str>,
    /// The shape the region marks out.
    #[serde(flatten)]
    pub shape: RegionShape,
    /// In the map's frame, as stored: [`euler_matrix`] places it.
    pub position: [f32; 3],
    /// Its rotation in degrees, as stored.
    pub rotation_degrees: [f32; 3],
    /// The region's own entity ID as stored (event scripts address it; 0 or -1: none).
    pub entity_id: i32,
    /// The type's own data, unread, as hexadecimal digits when serialised.
    #[serde(serialize_with = "hex")]
    pub type_data: Vec<u8>,
}

/// One event of the layout as stored (`EVENT_PARAM_ST`): what every type
/// shares, and the type's own data as bytes. SoulsFormats' `MSBS`/`MSBE` `Event` read the header alike: the event ID at
/// +8, the type at +12, the index at +16, the base data offset at +24 (a part index, a region index, the entity ID) and
/// the type data offset at +32. The part and region an event names are resolved to their names where the index is in
/// range (-1: none).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Event {
    /// Its index in the layout's events, in file order.
    pub index: usize,
    /// The event's name.
    pub name: String,
    /// The event ID as stored (`EventID`).
    pub event_id: i32,
    /// The type code as stored.
    pub type_code: u32,
    /// The type's name where SoulsFormats names it ([`Generation::event_type`]).
    pub type_name: Option<&'static str>,
    /// The name of the part it names, if any.
    pub part: Option<String>,
    /// The name of the region it names, if any.
    pub region: Option<String>,
    /// The entity ID it stores.
    pub entity_id: i32,
    /// The type's own data, unread, as hexadecimal digits when serialised.
    #[serde(serialize_with = "hex")]
    pub type_data: Vec<u8>,
}

impl PartKind {
    /// The word for the kind (a lower-case word).
    pub fn word(self) -> &'static str {
        match self {
            PartKind::MapPiece => "map_piece",
            PartKind::Object => "object",
            PartKind::Enemy => "enemy",
            PartKind::Player => "player_start",
            PartKind::Collision => "collision",
            PartKind::DummyObject => "dummy_object",
            PartKind::DummyEnemy => "dummy_enemy",
            PartKind::ConnectCollision => "connect_collision",
            PartKind::Asset => "asset",
            PartKind::DummyAsset => "dummy_asset",
        }
    }
}

/// A part of a kind that places no map model: an enemy, a player start, a
/// dummy object, asset or enemy, a connect collision. What the layout stores of it, as stored: its place in the map's
/// frame, its model's name, its entity ID and groups, and an enemy's NPC param row. What an enemy does (its think
/// param, patrol, talk) lives in other parts of the record and in the event scripts and is not read.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct UnplacedPart {
    /// Its index in the layout's parts ([`Part::index`]).
    pub index: usize,
    /// The part's name.
    pub name: String,
    /// The kind's word ([`PartKind::word`]).
    pub kind: &'static str,
    /// Its model's name, if it has one.
    pub model: Option<String>,
    /// An enemy's NPC param row, as stored.
    pub npc_param: Option<i32>,
    /// Its position, as stored.
    pub position: [f32; 3],
    /// Its rotation in degrees, as stored.
    pub rotation_degrees: [f32; 3],
    /// Its scale, as stored.
    pub scale: [f32; 3],
    /// Its entity ID, as stored.
    pub entity_id: i32,
    /// Its entity groups, as stored.
    pub entity_group_ids: [i32; 8],
}
impl MapLayout {
    /// The parts of the kinds that place no map model ([`UnplacedPart`]), in part order.
    pub fn unplaced_parts(&self) -> Vec<UnplacedPart> {
        self.parts
            .iter()
            .filter(|p| {
                matches!(
                    p.kind,
                    PartKind::Enemy
                        | PartKind::Player
                        | PartKind::DummyObject
                        | PartKind::DummyAsset
                        | PartKind::DummyEnemy
                        | PartKind::ConnectCollision
                )
            })
            .map(|p| UnplacedPart {
                index: p.index,
                name: p.name.clone(),
                kind: p.kind.word(),
                model: p.model.and_then(|i| self.models.get(i)).map(|m| m.name.clone()),
                npc_param: p.npc_param,
                position: p.position,
                rotation_degrees: p.rotation_degrees,
                scale: p.scale,
                entity_id: p.entity_id,
                entity_group_ids: p.entity_group_ids,
            })
            .collect()
    }
}

/// The records of section `i`.
fn section_records(sections: &[Section], i: usize) -> Result<&[Vec<u8>]> {
    sections
        .get(i)
        .map(|s| s.records.as_slice())
        .ok_or_else(|| invalid("MSB", 0, format!("the layout has no section {i}")))
}

fn hex<S: serde::Serializer>(bytes: &[u8], s: S) -> std::result::Result<S::Ok, S::Error> {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for b in bytes {
        let _ = write!(text, "{b:02x}");
    }
    s.serialize_str(&text)
}

impl Generation {
    /// The word for a region type code, where SoulsFormats' `RegionType` names it (a lower-case word of `a-z`, `0-9` and
    /// `_`, as a marker's kind is). Sekiro's and Elden Ring's codes differ: 21 is Sekiro's muffling portal and Elden
    /// Ring's connection.
    pub fn region_type(&self, code: u32) -> Option<&'static str> {
        if self.name == Self::SEKIRO.name {
            Some(match code {
                0 => "logic",
                1 => "invasion_point",
                2 => "environment_map_point",
                4 => "sound",
                5 => "sfx",
                6 => "wind_sfx",
                8 => "spawn_point",
                11 => "patrol_route",
                13 => "warp_point",
                14 => "activation_area",
                15 => "event",
                17 => "environment_map_effect_box",
                18 => "wind_area",
                20 => "muffling_box",
                21 => "muffling_portal",
                23 => "sound_space_override",
                24 => "muffling_plane",
                25 => "parts_group_area",
                26 => "auto_draw_group_point",
                _ => return None,
            })
        } else {
            Some(match code {
                1 => "invasion_point",
                2 => "environment_map_point",
                4 => "sound",
                5 => "sfx",
                6 => "wind_sfx",
                8 => "spawn_point",
                9 => "message",
                17 => "environment_map_effect_box",
                18 => "wind_area",
                21 => "connection",
                22 => "patrol_route_22",
                26 => "buddy_summon_point",
                27 => "disable_tumbleweed",
                28 => "muffling_box",
                29 => "muffling_portal",
                30 => "sound_region",
                31 => "muffling_plane",
                32 => "patrol_route",
                33 => "map_point",
                35 => "weather_override",
                36 => "auto_draw_group_point",
                37 => "group_defeat_reward",
                38 => "map_point_discovery_override",
                39 => "map_point_participation_override",
                40 => "hitset",
                41 => "fast_travel_restriction",
                42 => "weather_create_asset_point",
                43 => "play_area",
                44 => "environment_map_output",
                46 => "mount_jump",
                48 => "dummy",
                49 => "fall_prevention_removal",
                50 => "navmesh_cutting",
                51 => "map_name_override",
                52 => "mount_jump_fall",
                53 => "horse_ride_override",
                54 => "locked_mount_jump",
                55 => "locked_mount_jump_fall",
                _ => return None,
            })
        }
    }

    /// The word for an event type code, where SoulsFormats' `EventType` names it (as [`Generation::region_type`]).
    pub fn event_type(&self, code: u32) -> Option<&'static str> {
        if self.name == Self::SEKIRO.name {
            Some(match code {
                4 => "treasure",
                5 => "generator",
                7 => "obj_act",
                9 => "map_offset",
                14 => "patrol_info",
                15 => "platoon_info",
                17 => "resource_item_info",
                18 => "grass_lod_param",
                20 => "skit_info",
                21 => "placement_group",
                22 => "parts_group",
                23 => "talk",
                24 => "auto_draw_group_collision",
                _ => return None,
            })
        } else {
            Some(match code {
                4 => "treasure",
                5 => "generator",
                7 => "obj_act",
                10 => "navmesh",
                12 => "pseudo_multiplayer",
                15 => "platoon_info",
                20 => "patrol_info",
                21 => "mount",
                23 => "sign_pool",
                24 => "retry_point",
                25 => "area_team",
                _ => return None,
            })
        }
    }
}

impl MapLayout {
    /// Every region of the layout, of every type, in file order, with its shape and the bytes of its type data. A
    /// record that does not fit the layout above is an error naming it.
    pub fn map_regions(&self) -> Result<Vec<Region>> {
        let mut out = Vec::new();
        for (index, data) in self.regions()?.records.iter().enumerate() {
            let r = View {
                data,
                format: "MSB region",
                be: false,
            };
            r.slice(0, 96)?;
            let name = text(&r, pointer(&r, 0, 96)?)?;
            let type_code = r.u32(8)?;
            let shape_code = r.u32(16)?;
            let shape_data = r.size(72)?;
            let shape_data_at = |delta: usize| -> Result<usize> {
                if shape_data < 96 {
                    return Err(invalid(r.format, 72, "a shape without data"));
                }
                if shape_data >= r.data.len() {
                    return Err(invalid(
                        r.format,
                        72,
                        format!(
                            "shape data at {shape_data:#x} is beyond the {} bytes of the record",
                            r.data.len()
                        ),
                    ));
                }
                r.at(shape_data, delta)
            };
            let shape = match shape_code {
                0xFFFF_FFFF => RegionShape::None,
                0 => RegionShape::Point,
                1 => RegionShape::Circle {
                    radius: shape_floats::<1>(&r, &shape_data_at)?[0],
                },
                2 => RegionShape::Sphere {
                    radius: shape_floats::<1>(&r, &shape_data_at)?[0],
                },
                3 => {
                    let [radius, height] = shape_floats::<2>(&r, &shape_data_at)?;
                    RegionShape::Cylinder { radius, height }
                }
                4 => {
                    let [width, depth] = shape_floats::<2>(&r, &shape_data_at)?;
                    RegionShape::Rectangle { width, depth }
                }
                5 => {
                    let [width, depth, height] = shape_floats::<3>(&r, &shape_data_at)?;
                    RegionShape::Box { width, depth, height }
                }
                6 => RegionShape::Composite {
                    children: (0usize..8)
                        .map(|i| {
                            Ok((
                                r.u32(shape_data_at(i.saturating_mul(8))?)? as i32,
                                r.u32(shape_data_at(i.saturating_mul(8).saturating_add(4))?)? as i32,
                            ))
                        })
                        .collect::<Result<_>>()?,
                },
                code => RegionShape::Unknown { code },
            };
            let entity = r.size(80)?;
            if entity < 96 {
                return Err(invalid(r.format, 80, "no entity block"));
            }
            let type_data = match r.size(88)? {
                0 => Vec::new(),
                at if at >= 96 => r.slice(at, r.data.len().saturating_sub(at))?.to_vec(),
                _ => return Err(invalid(r.format, 88, "type data inside the header")),
            };
            out.push(Region {
                index,
                name,
                type_code,
                type_name: self.generation.region_type(type_code),
                shape,
                position: vector(&r, 20)?,
                rotation_degrees: vector(&r, 32)?,
                entity_id: r.u32_at(entity, 4)? as i32,
                type_data,
            });
        }
        Ok(out)
    }

    /// Every event of the layout, of every type, in file order, with the bytes of its type data (a MapOffset event's
    /// among them: [`MapLayout::map_offsets`] reads those).
    pub fn map_events(&self) -> Result<Vec<Event>> {
        let regions = self.region_names()?;
        let mut out = Vec::new();
        for (index, data) in section_records(&self.sections, 1)?.iter().enumerate() {
            let r = View {
                data,
                format: "MSB event",
                be: false,
            };
            r.slice(0, 40)?;
            let name = text(&r, pointer(&r, 0, 40)?)?;
            let type_code = r.u32(12)?;
            let base = pointer(&r, 24, 40)?;
            let type_data = match r.size(32)? {
                0 => Vec::new(),
                at if at >= 40 => r.slice(at, r.data.len().saturating_sub(at))?.to_vec(),
                _ => return Err(invalid(r.format, 32, "type data inside the header")),
            };
            let named =
                |i: i32, names: &dyn Fn(usize) -> Option<String>| usize::try_from(i).ok().and_then(names);
            out.push(Event {
                index,
                name,
                event_id: r.u32(8)? as i32,
                type_code,
                type_name: self.generation.event_type(type_code),
                part: named(r.u32(base)? as i32, &|i| {
                    self.parts.get(i).map(|p| p.name.clone())
                }),
                region: named(r.u32_at(base, 4)? as i32, &|i| regions.get(i).cloned()),
                entity_id: r.u32_at(base, 8)? as i32,
                type_data,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod region_tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    /// A region record: a 96-byte header, then the name, the entity block, the shape data and the type data.
    fn region(
        name: &str,
        type_code: u32,
        shape: u32,
        floats: &[f32],
        entity: u32,
        type_data: &[u8],
    ) -> Vec<u8> {
        let mut tail = utf16(name);
        let entity_at = 96 + tail.len();
        tail.extend([0u8; 4]);
        tail.extend(entity.to_le_bytes());
        let shape_at = 96 + tail.len();
        tail.extend(floats.iter().flat_map(|f| f.to_le_bytes()));
        let type_at = 96 + tail.len();
        tail.extend(type_data);
        let mut r = vec![0u8; 96];
        r[0..8].copy_from_slice(&96u64.to_le_bytes());
        r[8..12].copy_from_slice(&type_code.to_le_bytes());
        r[16..20].copy_from_slice(&shape.to_le_bytes());
        for (i, v) in [1.0f32, 2.0, 3.0, 10.0, 20.0, 30.0].iter().enumerate() {
            r[20 + 4 * i..24 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        let slot =
            |r: &mut [u8], at: usize, v: usize| r[at..at + 8].copy_from_slice(&(v as u64).to_le_bytes());
        slot(&mut r, 72, if floats.is_empty() { 0 } else { shape_at });
        slot(&mut r, 80, entity_at);
        slot(&mut r, 88, if type_data.is_empty() { 0 } else { type_at });
        r.extend(tail);
        r
    }

    fn layout(generation: &'static Generation, records: Vec<Vec<u8>>) -> MapLayout {
        let mut sections = generation
            .sections
            .iter()
            .map(|name| Section {
                name: (*name).to_owned(),
                version: 0,
                records: Vec::new(),
            })
            .collect::<Vec<_>>();
        sections[2].records = records;
        MapLayout {
            generation,
            models: Vec::new(),
            parts: Vec::new(),
            map_offsets: Vec::new(),
            sections,
        }
    }

    #[test]
    fn drawn_points_use_the_same_part_kinds_and_ground_frame_as_the_area_opener() {
        let mut layout = layout(&Generation::SEKIRO, vec![]);
        let part = |index, kind| Part {
            index,
            name: format!("part{index}"),
            kind,
            model: None,
            hit_filter: None,
            npc_param: None,
            position: [index as f32, 2.0, 3.0],
            rotation_degrees: [0.0; 3],
            scale: [1.0; 3],
            entity_id: 0,
            entity_group_ids: [0; 8],
            shadows: ShadowFlags::default(),
        };
        layout.parts = vec![
            part(0, PartKind::MapPiece),
            part(1, PartKind::Object),
            part(2, PartKind::Asset),
            part(3, PartKind::Enemy),
            part(4, PartKind::Collision),
            part(5, PartKind::Player),
        ];
        assert_eq!(
            drawn_part_points(&layout).unwrap(),
            vec![[0.0, 3.0], [1.0, 3.0], [2.0, 3.0]]
        );
        layout.parts[0].position[0] = f32::NAN;
        assert!(drawn_part_points(&layout).is_err());
    }

    /// A region of any type is read with its shape's dimensions, its stored place and entity, and its type's bytes; the
    /// type is named by the generation that reads it.
    #[test]
    fn a_regions_shape_place_and_type_data_are_read_as_stored() {
        let layout = layout(
            &Generation::ELDEN_RING,
            vec![
                region("Wind", 6, 5, &[4.0, 5.0, 6.0], 1042, &[7, 8, 9]),
                region("Spot", 21, 0, &[], 0, &[]),
                region("Odd", 77, 3, &[2.0, 9.0], 5, &[]),
            ],
        );
        let regions = layout.map_regions().unwrap();
        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].name, "Wind");
        assert_eq!(regions[0].type_name, Some("wind_sfx"));
        assert_eq!(
            regions[0].shape,
            RegionShape::Box {
                width: 4.0,
                depth: 5.0,
                height: 6.0
            }
        );
        assert_eq!(regions[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(regions[0].rotation_degrees, [10.0, 20.0, 30.0]);
        assert_eq!(
            (regions[0].entity_id, regions[0].type_data.clone()),
            (1042, vec![7, 8, 9])
        );
        assert_eq!(
            (regions[1].shape.clone(), regions[1].type_name),
            (RegionShape::Point, Some("connection"))
        );
        assert!(regions[1].type_data.is_empty());
        // a type the generation does not name keeps its code
        assert_eq!((regions[2].type_code, regions[2].type_name), (77, None));
        assert_eq!(
            regions[2].shape,
            RegionShape::Cylinder {
                radius: 2.0,
                height: 9.0
            }
        );
    }

    #[test]
    fn enemy_initializer_is_separate_from_the_npc_row_in_both_generations() {
        for generation in [&Generation::SEKIRO, &Generation::ELDEN_RING] {
            let mut layout = layout(generation, Vec::new());
            let mut record = vec![0u8; 160];
            record[104..112].copy_from_slice(&128u64.to_le_bytes());
            record[140..144].copy_from_slice(&10002020i32.to_le_bytes());
            record[152..156].copy_from_slice(&9000i32.to_le_bytes());
            layout.sections[5].records.push(record);
            layout.parts.push(Part {
                index: 0,
                name: "human".into(),
                kind: PartKind::Enemy,
                model: None,
                hit_filter: None,
                npc_param: Some(10002020),
                position: [0.; 3],
                rotation_degrees: [0.; 3],
                scale: [1.; 3],
                entity_id: -1,
                entity_group_ids: [-1; 8],
                shadows: Default::default(),
            });
            assert_eq!(layout.character_init(0).unwrap(), Some(9000));
            layout.sections[5].records[0][152..156].copy_from_slice(&(-1i32).to_le_bytes());
            assert_eq!(layout.character_init(0).unwrap(), None);
            layout.parts[0].kind = PartKind::MapPiece;
            assert_eq!(layout.character_init(0).unwrap(), None);
        }
    }

    /// The same code names another type in the other generation.
    #[test]
    fn a_type_code_is_named_by_the_generation_that_reads_it() {
        assert_eq!(Generation::SEKIRO.region_type(21), Some("muffling_portal"));
        assert_eq!(Generation::ELDEN_RING.region_type(21), Some("connection"));
        assert_eq!(Generation::SEKIRO.event_type(9), Some("map_offset"));
        assert_eq!(Generation::ELDEN_RING.event_type(9), None);
        // every word is a valid marker kind
        for generation in Generation::ALL {
            for code in 0..64 {
                for word in [generation.region_type(code), generation.event_type(code)]
                    .into_iter()
                    .flatten()
                {
                    assert!(
                        word.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                        "{word}"
                    );
                }
            }
        }
    }

    /// A shape that needs data and has none is an error naming the field; a record shorter than its header is one too.
    #[test]
    fn a_region_that_does_not_fit_is_an_error() {
        let mut bad = region("Wind", 6, 5, &[4.0, 5.0, 6.0], 1, &[]);
        bad[72..80].copy_from_slice(&0u64.to_le_bytes());
        assert!(layout(&Generation::ELDEN_RING, vec![bad]).map_regions().is_err());
        assert!(
            layout(&Generation::ELDEN_RING, vec![vec![0; 40]])
                .map_regions()
                .is_err()
        );
    }

    /// A section chain whose next pointer lies beyond the file, or near the top of the address range, is an error at
    /// the pointer, not an overflow.
    #[test]
    fn a_section_chain_pointer_beyond_the_file_is_an_error() {
        for next in [u64::MAX - 1, u64::MAX - 3, 1 << 40] {
            let mut data = b"MSB \x01\0\0\0\x10\0\0\0\0\0\x01\xff".to_vec();
            data.extend([0u8; 4]);
            data.extend(1u32.to_le_bytes());
            data.extend(0u64.to_le_bytes());
            data.extend(next.to_le_bytes());
            let err = MapLayout::read(&data).unwrap_err().to_string();
            assert!(err.starts_with("MSB at 0x20"), "{err}");
        }
    }

    /// A shape-data pointer near the top of the address range is an error at the pointer.
    #[test]
    fn a_shape_pointer_near_the_address_limit_is_an_error() {
        for shape in [5, 3, 6] {
            let mut bad = region("Wind", 6, shape, &[4.0, 5.0, 6.0], 1, &[]);
            bad[72..80].copy_from_slice(&(u64::MAX - 2).to_le_bytes());
            let err = layout(&Generation::ELDEN_RING, vec![bad])
                .map_regions()
                .unwrap_err();
            assert!(err.to_string().starts_with("MSB region at 0x48"), "{err}");
        }
    }

    /// Type data is serialised as hexadecimal text, not a list of numbers.
    #[test]
    fn type_data_serialises_as_hex() {
        let layout = layout(
            &Generation::SEKIRO,
            vec![region("Snd", 4, 2, &[3.0], 9, &[0xab, 0x01])],
        );
        let json = serde_json::to_value(&layout.map_regions().unwrap()[0]).unwrap();
        assert_eq!(json["type_data"], "ab01");
        assert_eq!(json["shape"], "sphere");
        assert_eq!(json["radius"], 3.0);
        assert_eq!(json["type_name"], "sound");
    }
}
