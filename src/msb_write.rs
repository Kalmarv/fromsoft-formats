//! The map layout writer: [`MapLayout::write`].
//!
//! An MSB is a header of 16 bytes and a chain of sections. Each section stores its revision, its entry count (the records and
//! one more slot), the offset of its name, a table of the records' offsets whose last slot points at the next section (0
//! after the last), the name (UTF-16, padded to 8 bytes) and the records (each a multiple of 8 bytes long in the game's files). A record is
//! position independent (the offsets inside it count from its first byte), so [`Section::records`] holds the bytes of each
//! as stored and the writer only has to lay the sections out again.
//!
//! The decoded [`Part`]s are views of the part records. [`MapLayout::write`] lays the fields of the parts that sit at a fixed
//! place in the record over the records it writes (the placement, the entity ID and groups, the shadow and draw controls, a
//! collision's hit filter, an enemy's NPC row, an asset's effect id, a part's tile-load map), so that a layout edited
//! through [`MapLayout::parts`] is written as edited. What changes the size of a record (a part's name) or what it
//! points at (its model, its kind) is not written from the view and is refused where the view disagrees with the record.
//! Anything else is edited in the records themselves.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use crate::{
    Result,
    emit::Out,
    invalid,
    mapid::MapId,
    msb::{FlagByte, Generation, MapLayout, Part, PartKind},
};

const FORMAT: &str = "MSB";
/// The file's first 16 bytes.
const MAGIC: &[u8; 16] = b"MSB \x01\0\0\0\x10\0\0\0\0\0\x01\xff";
/// Where the part records keep their placement: the position, the rotation in degrees and the scale (three floats each).
const POSITION: usize = 32;
const ROTATION: usize = 44;
const SCALE: usize = 56;
/// Where the part records keep the pointers to their entity block and their type data.
const ENTITY_POINTER: usize = 96;
const TYPE_DATA_POINTER: usize = 104;

/// Overwrite `bytes` at `at` in `record`, a part's, or refuse naming `what`.
fn put(record: &mut [u8], at: usize, bytes: &[u8], what: &str) -> Result<()> {
    let slot = at
        .checked_add(bytes.len())
        .and_then(|end| record.get_mut(at..end))
        .ok_or_else(|| invalid("MSB part", at, format!("{what} lies outside the record")))?;
    slot.copy_from_slice(bytes);
    Ok(())
}

/// The 64-bit pointer at `at` in `record`.
fn pointer(record: &[u8], at: usize, what: &str) -> Result<usize> {
    let bytes: [u8; 8] = at
        .checked_add(8)
        .and_then(|end| record.get(at..end))
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| invalid("MSB part", at, format!("{what} lies outside the record")))?;
    usize::try_from(u64::from_le_bytes(bytes))
        .map_err(|_| invalid("MSB part", at, format!("{what} does not fit")))
}

/// The little-endian word at `at` in `record`.
fn word(record: &[u8], at: usize) -> Option<u32> {
    let bytes: [u8; 4] = record.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// The UTF-16 string at `at` in `record`.
fn text(record: &[u8], at: usize) -> Option<String> {
    let mut units = Vec::new();
    let mut o = at;
    loop {
        let unit = u16::from_le_bytes(record.get(o..o.checked_add(2)?)?.try_into().ok()?);
        if unit == 0 {
            return String::from_utf16(&units).ok();
        }
        units.push(unit);
        o = o.checked_add(2)?;
    }
}

fn floats(v: [f32; 3]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// The record of `part` with the part's fields laid over it.
fn sync(generation: &Generation, record: &[u8], part: &Part) -> Result<Vec<u8>> {
    let refuse = |what: &str| {
        invalid(
            "MSB part",
            0,
            format!("part {} ({}): {what}", part.index, part.name),
        )
    };
    // what the view must still agree with
    if pointer(record, 0, "the name")
        .ok()
        .and_then(|at| text(record, at))
        .as_deref()
        != Some(part.name.as_str())
    {
        return Err(refuse(
            "the name differs from the record's, which a view cannot change",
        ));
    }
    let code = word(record, generation.part_kind);
    let kind = generation
        .part_kinds
        .iter()
        .find(|(c, _)| Some(*c) == code)
        .map(|(_, k)| *k);
    if kind != Some(part.kind) {
        return Err(refuse("the kind differs from the record's"));
    }
    let model = word(record, generation.part_model).and_then(|m| (m != u32::MAX).then_some(m as usize));
    if model != part.model {
        return Err(refuse("the model differs from the record's"));
    }
    let mut r = record.to_vec();
    put(&mut r, POSITION, &floats(part.position), "the position")?;
    put(&mut r, ROTATION, &floats(part.rotation_degrees), "the rotation")?;
    put(&mut r, SCALE, &floats(part.scale), "the scale")?;
    let entity = pointer(&r, ENTITY_POINTER, "the entity block")?;
    let at = |delta: usize| entity.saturating_add(delta);
    put(&mut r, at(0), &part.entity_id.to_le_bytes(), "the entity id")?;
    for (k, group) in part.entity_group_ids.iter().enumerate() {
        put(&mut r, at(0x1c + 4 * k), &group.to_le_bytes(), "an entity group")?;
    }
    let s = part.shadows;
    put(&mut r, at(0x0c), &[u8::from(s.cast)], "the shadow cast flag")?;
    put(
        &mut r,
        at(0x11),
        &[u8::from(s.receive)],
        "the shadow receive flag",
    )?;
    put(&mut r, at(0x12), &[u8::from(s.only)], "the shadow only flag")?;
    let point_light = match generation.point_light_shadow {
        FlagByte::Bool => u8::from(s.point_light_cast),
        FlagByte::ZeroOrMinusOne => {
            if s.point_light_cast {
                0xff
            } else {
                0
            }
        }
    };
    put(&mut r, at(0x0a), &[point_light], "the point-light shadow byte")?;
    put(&mut r, at(0x0d), &[s.static_cast], "the static shadow byte")?;
    put(&mut r, at(0x0e), &[s.cascade3_cast], "the cascade 3 shadow byte")?;
    let d = part.draw;
    if generation.name == Generation::SEKIRO.name {
        put(
            &mut r,
            at(8),
            &[d.lod_param_id.unwrap_or(255)],
            "the LOD param row",
        )?;
    } else {
        put(
            &mut r,
            at(4),
            &[u8::from(d.use_parts_draw_param)],
            "the parts draw param flag",
        )?;
        put(
            &mut r,
            at(8),
            &d.parts_draw_param_id.to_le_bytes(),
            "the parts draw param row",
        )?;
        put(
            &mut r,
            at(0x14),
            &[u8::from(d.draw_only_in_reflection)],
            "a draw flag",
        )?;
        put(
            &mut r,
            at(0x15),
            &[u8::from(d.enable_on_above_shadow)],
            "a draw flag",
        )?;
        put(
            &mut r,
            at(0x16),
            &[u8::from(d.disable_point_light_effect)],
            "a draw flag",
        )?;
        put(&mut r, at(0x3f), &[u8::from(d.disable_rtao)], "a draw flag")?;
    }
    // the type data
    let type_data = |r: &[u8]| pointer(r, TYPE_DATA_POINTER, "the type data");
    match part.hit_filter {
        Some(filter) if part.kind == PartKind::Collision => {
            let data = type_data(&r)?;
            put(&mut r, data, &[filter], "the hit filter")?;
        }
        Some(_) => return Err(refuse("a hit filter on a part that is not a collision")),
        None => {}
    }
    match part.npc_param {
        Some(row) if matches!(part.kind, PartKind::Enemy | PartKind::DummyEnemy) => {
            let data = type_data(&r)?;
            put(
                &mut r,
                data.saturating_add(12),
                &row.to_le_bytes(),
                "the NPC param row",
            )?;
        }
        Some(_) => return Err(refuse("an NPC param row on a part that is not an enemy")),
        None => {}
    }
    match part.sfx_param_relative_id {
        Some(id) if part.kind == PartKind::Asset && generation.name != Generation::SEKIRO.name => {
            let data = type_data(&r)?;
            put(
                &mut r,
                data.saturating_add(0x1c),
                &id.to_le_bytes(),
                "the effect id",
            )?;
        }
        Some(_) => return Err(refuse("an effect id on a part that is not an Elden Ring asset")),
        None => {}
    }
    if let Some(slot) = generation.tile_load {
        let config = pointer(&r, slot, "the tile-load config")?;
        match (config, part.tile_load) {
            (0, None) => {}
            (0, Some(_)) => {
                return Err(refuse(
                    "a tile-load map on a part that stores no tile-load config",
                ));
            }
            (at, map) => {
                let word = map.map_or(u32::MAX, |m: MapId| {
                    u32::from_be_bytes([m.area, m.x, m.z, m.variant])
                });
                put(&mut r, at, &word.to_le_bytes(), "the tile-load map")?;
            }
        }
    }
    Ok(r)
}

impl MapLayout {
    /// Write the layout as an `.msb` file. A layout that [`MapLayout::read`] returned writes back to the bytes it was read
    /// from, for every map of both installs (Sekiro's 10, Elden Ring's 887 distinct files of 935). The fields of
    /// [`MapLayout::parts`] are laid over the part records as the module's note says; the records of every other
    /// section are written as they are in [`MapLayout::sections`].
    ///
    /// Refused: sections that are not the generation's chain, a part whose
    /// view names another record than it has (its index outside the section, two parts for one record) or disagrees with
    /// its record in what the view cannot change, and a layout whose bytes [`MapLayout::read`] would refuse.
    pub fn write(&self) -> Result<Vec<u8>> {
        let generation = self.generation;
        let names_match = self.sections.len() == generation.sections.len()
            && self
                .sections
                .iter()
                .zip(generation.sections)
                .all(|(s, name)| s.name == *name);
        if !names_match {
            return Err(invalid(
                FORMAT,
                0,
                "the sections are not the generation's section chain",
            ));
        }
        let parts_section = self
            .sections
            .get(5)
            .ok_or_else(|| invalid(FORMAT, 0, "a layout without a parts section"))?;
        let mut parts_records: Vec<Vec<u8>> = parts_section.records.clone();
        let mut seen = vec![false; parts_records.len()];
        for part in &self.parts {
            let slot = seen.get_mut(part.index).ok_or_else(|| {
                invalid(
                    FORMAT,
                    0,
                    format!("part {} names a record the section lacks", part.index),
                )
            })?;
            if std::mem::replace(slot, true) {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!("two parts name record {}", part.index),
                ));
            }
            let record = parts_section.records.get(part.index).ok_or_else(|| {
                invalid(
                    FORMAT,
                    0,
                    format!("part {} names a record the section lacks", part.index),
                )
            })?;
            let synced = sync(generation, record, part)?;
            if let Some(slot) = parts_records.get_mut(part.index) {
                *slot = synced;
            }
        }

        let mut out = Out::new(FORMAT);
        out.bytes(MAGIC);
        for (si, section) in self.sections.iter().enumerate() {
            let records = if si == 5 { &parts_records } else { &section.records };
            let start = out.len();
            let slots = records.len() + 1;
            let name: Vec<u8> = section
                .name
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            let name_at = start + 16 + 8 * slots;
            let first = (name_at + name.len()).next_multiple_of(8);
            out.u32(section.version);
            out.u32(out.word(slots, "section slots")?);
            out.u64(out.wide(name_at, "section name offset")?);
            let mut at = first;
            for record in records {
                out.u64(out.wide(at, "record offset")?);
                at += record.len();
            }
            // the next section starts where this one's records end
            out.u64(if si + 1 == self.sections.len() {
                0
            } else {
                out.wide(at, "next section offset")?
            });
            out.bytes(&name);
            out.align(8);
            for record in records {
                out.bytes(record);
            }
        }
        let bytes = out.into_bytes();
        MapLayout::read(&bytes)?;
        Ok(bytes)
    }
}
