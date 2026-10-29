//! The TAE writer: [`Tae::write`].
//!
//! The stored layout, in file order: the 0xd0-byte header; the skeleton and SIB names (UTF-16, each padded to 16 bytes, none
//! written for a name the file does not state); the animation table (an animation number and the offset of its body, 16
//! bytes each); the animation groups table (a count and the offset of the entries, then one 16-byte entry for each run of
//! consecutive animation numbers: its first and last number and the offset of the table record of the first); every
//! animation's 48-byte body; and then, animation by animation, its mini header block (48 bytes, and the motion file name
//! padded to 16 bytes where it states one), its times table (the distinct start and end times, ascending, padded to 16
//! bytes), the event headers (24 bytes each: the offsets of the start time, the end time and the event data), the event
//! data (a 16-byte record and the parameter bytes, which carry the padding that aligns the next record to 16 bytes), the
//! event group headers (32 bytes each) and the event group data (the group's type word and a zero word, then the offsets
//! of its events' headers, padded to 16 bytes). Event type 943 stores 0 for the offset of its parameters, every other
//! event the offset after its record.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use std::collections::BTreeMap;

use crate::{
    Result,
    put::Out,
    tae::{Animation, Header, Tae, VERSION},
};

const FORMAT: &str = "TAE";
/// The event type whose parameter offset is stored as 0.
const NO_PARAMETER_OFFSET_TYPE: i32 = 943;

/// A NUL-terminated UTF-16 name, padded to 16 bytes; nothing for an empty one.
fn name(out: &mut Out, text: Option<&str>) -> Result<()> {
    let Some(text) = text.filter(|t| !t.is_empty()) else {
        return Ok(());
    };
    if text.contains('\0') {
        return Err(out.err(format!("{text:?} holds a NUL, which ends a stored name")));
    }
    for unit in text.encode_utf16() {
        out.u16(unit);
    }
    out.u16(0);
    out.align(16);
    Ok(())
}

/// `n` as the signed 32-bit word the groups table stores animation numbers in.
fn to_i32(out: &Out, n: i64) -> Result<i32> {
    i32::try_from(n).map_err(|_| {
        out.err(format!(
            "animation {n} does not fit the 32 bits the groups table stores"
        ))
    })
}

/// The `i`th of `offsets`, or a refusal that names `what`.
fn nth(out: &Out, offsets: &[usize], i: usize, what: &str) -> Result<usize> {
    offsets.get(i).copied().ok_or_else(|| {
        out.err(format!(
            "{what} {i} is outside the {} the file holds",
            offsets.len()
        ))
    })
}

impl Tae {
    /// Write the file as a `.tae` of version `0x1000D` (Sekiro's and Elden Ring's), in the layout described at the top of
    /// this module. A file that [`Tae::read`] returned writes back to the bytes it was read from, for every event file
    /// of both installs the reader accepts: the writer sorts nothing and lays the tables out in the order the game's
    /// tools do. A name that is `Some("")` is written as no name (the reader returns it for an animation without events
    /// and without a motion file name).
    ///
    /// Refused: animations whose numbers do not rise strictly (the groups table holds the runs of consecutive numbers) or
    /// do not fit the 32 bits it stores them in, an animation whose mini header is [`Header::None`] (the format has one
    /// for every animation), a time that is not a number, an event group naming an event the animation lacks, a name
    /// with a NUL in it, and anything that does not fit a 32-bit offset or count.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut out = Out::new(FORMAT, false);
        let count = self.animations.len();
        let mut previous: Option<i64> = None;
        for a in &self.animations {
            if previous.is_some_and(|p| a.id <= p) {
                return Err(out.err(format!(
                    "animation {} does not rise above the one before it",
                    a.id
                )));
            }
            to_i32(&out, a.id)?;
            previous = Some(a.id);
        }
        let count_word = out.word(count, "animation count")?;
        out.bytes(b"TAE \0\0\0\xff");
        out.u32(VERSION);
        out.u32(0);
        for word in [0x40u64, 1, 0x50, 0x80] {
            out.u64(word);
        }
        out.i64(self.event_bank);
        out.u64(0);
        out.bytes(&[1, 0, 1, 2, 2, 1, 1, 1]);
        out.u64(1);
        out.i32(self.id);
        out.u32(count_word);
        out.u64(0);
        out.u64(0);
        out.u64(0xa0);
        out.u64(out.wide(count, "animation count")?);
        out.u64(0);
        out.u64(1);
        out.u64(0x90);
        out.u32(self.id as u32);
        out.u32(self.id as u32);
        out.u64(0x50);
        out.u64(0);
        out.u64(0xb0);
        out.zeros(4 * 8);
        let header_end = out.len();
        out.patch_u64(0xb0, out.wide(header_end, "name offset")?)?;
        name(&mut out, self.skeleton_name.as_deref())?;
        out.patch_u64(0xb8, out.wide(out.len(), "name offset")?)?;
        name(&mut out, self.sib_name.as_deref())?;

        let table = out.len();
        if count > 0 {
            out.patch_u64(0x58, out.wide(table, "table offset")?)?;
        }
        let mut records = Vec::with_capacity(count);
        for a in &self.animations {
            records.push(out.len());
            out.i64(a.id);
            out.u64(0);
        }
        let group_header = out.len();
        out.patch_u64(0x60, out.wide(group_header, "group header offset")?)?;
        // the runs of consecutive animation numbers: first, last, index of the first
        let mut runs: Vec<(i64, i64, usize)> = Vec::new();
        for (i, a) in self.animations.iter().enumerate() {
            match runs.last_mut() {
                Some((_, last, _)) if Some(a.id) == last.checked_add(1) => *last = a.id,
                _ => runs.push((a.id, a.id, i)),
            }
        }
        out.u64(out.wide(runs.len(), "group count")?);
        out.u64(if runs.is_empty() {
            0
        } else {
            out.wide(group_header + 16, "group entries offset")?
        });
        for &(first, last, index) in &runs {
            // the numbers were checked to fit above
            let (first, last) = (to_i32(&out, first)?, to_i32(&out, last)?);
            out.i32(first);
            out.i32(last);
            out.u64(out.wide(nth(&out, &records, index, "animation")?, "record offset")?);
        }
        let mut bodies = Vec::with_capacity(count);
        for _ in &self.animations {
            bodies.push(out.len());
            out.zeros(48);
        }
        if let Some(&first) = bodies.first() {
            out.patch_u64(0x78, out.wide(first, "body offset")?)?;
        }
        for (i, &body) in bodies.iter().enumerate() {
            out.patch_u64(
                nth(&out, &records, i, "animation")? + 8,
                out.wide(body, "body offset")?,
            )?;
        }
        for (a, &body) in self.animations.iter().zip(&bodies) {
            animation(&mut out, a, body)?;
        }
        let size = out.word(out.len(), "file size")?;
        out.patch_u32(12, size)?;
        Ok(out.finish())
    }
}

/// One animation's mini header block, times, events and event groups, and the fields of its body at `body`.
fn animation(out: &mut Out, a: &Animation, body: usize) -> Result<()> {
    let block = out.len();
    let kind = match &a.header {
        Header::None => {
            return Err(out.err(format!(
                "animation {} has no mini header, which the format stores for every animation",
                a.id
            )));
        }
        Header::Standard { .. } => 0u64,
        Header::ImportOther { .. } => 1,
    };
    out.u64(kind);
    out.u64(out.wide(block + 16, "record offset")?);
    out.u64(0);
    match &a.header {
        Header::Standard {
            loops,
            imports_hkx,
            allow_delay_load,
            import_hkx_source,
        } => {
            out.u8(u8::from(*loops));
            out.u8(u8::from(*imports_hkx));
            out.u8(u8::from(*allow_delay_load));
            out.u8(0);
            out.i32(*import_hkx_source);
        }
        Header::ImportOther { from, unknown } => {
            out.i32(*from);
            out.i32(*unknown);
        }
        Header::None => {}
    }
    out.zeros(16);
    // the motion file name's offset is where the name goes, which is where the times table starts when there is none
    out.patch_u64(block + 16, out.wide(out.len(), "name offset")?)?;
    name(out, a.file_name.as_deref())?;

    let mut times: Vec<f32> = a.events.iter().flat_map(|e| [e.start, e.end]).collect();
    if times.iter().any(|t| t.is_nan()) {
        return Err(out.err(format!("animation {}: a time that is not a number", a.id)));
    }
    times.sort_by(f32::total_cmp);
    times.dedup_by(|x, y| x.to_bits() == y.to_bits());
    let times_at = if times.is_empty() { 0 } else { out.len() };
    let mut time_offsets = BTreeMap::new();
    for t in &times {
        time_offsets.insert(t.to_bits(), out.len());
        out.f32(*t);
    }
    out.align(16);

    let headers = out.len();
    let headers_at = if a.events.is_empty() { 0 } else { headers };
    out.zeros(24 * a.events.len());
    let mut event_headers = Vec::with_capacity(a.events.len());
    for (j, e) in a.events.iter().enumerate() {
        let header = headers + 24 * j;
        event_headers.push(header);
        let data = out.len();
        let time = |t: f32| {
            time_offsets
                .get(&t.to_bits())
                .copied()
                .ok_or_else(|| out.err("a time missing from the times table"))
        };
        let (start, end) = (time(e.start)?, time(e.end)?);
        out.patch_u64(header, out.wide(start, "time offset")?)?;
        out.patch_u64(header + 8, out.wide(end, "time offset")?)?;
        out.patch_u64(header + 16, out.wide(data, "event offset")?)?;
        out.i32(e.kind);
        out.i32(e.unk04);
        out.u64(if e.kind == NO_PARAMETER_OFFSET_TYPE {
            0
        } else {
            out.wide(data + 16, "parameter offset")?
        });
        out.bytes(&e.parameters);
        out.align(16);
    }

    let groups = out.len();
    let groups_at = if a.event_groups.is_empty() { 0 } else { groups };
    out.zeros(32 * a.event_groups.len());
    for (k, group) in a.event_groups.iter().enumerate() {
        let kind_at = out.len();
        out.u64(group.group_type);
        out.u64(0);
        let values_at = out.len();
        for &event in &group.events {
            let header = nth(out, &event_headers, event, "event")?;
            let word = out.word(header, "event header offset")?;
            out.u32(word);
        }
        out.align(16);
        let record = groups + 32 * k;
        out.patch_u64(record, out.wide(group.events.len(), "group size")?)?;
        out.patch_u64(record + 8, out.wide(values_at, "group values offset")?)?;
        out.patch_u64(record + 16, out.wide(kind_at, "group type offset")?)?;
    }

    out.patch_u64(body, out.wide(headers_at, "event headers offset")?)?;
    out.patch_u64(body + 8, out.wide(groups_at, "event groups offset")?)?;
    out.patch_u64(body + 16, out.wide(times_at, "times offset")?)?;
    out.patch_u64(body + 24, out.wide(block, "mini header offset")?)?;
    out.patch_u32(body + 32, out.word(a.events.len(), "event count")?)?;
    out.patch_u32(body + 36, out.word(a.event_groups.len(), "event group count")?)?;
    out.patch_u32(body + 40, out.word(times.len(), "time count")?)?;
    Ok(())
}
