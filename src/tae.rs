//! TAE files (`.tae`): the time-action event tables of a character's animation binder. One file holds the animations of
//! a character by number (`3000`, `20010`) and, for each, the timed events the game runs while it plays: a type, a start
//! and an end in seconds, and the event's parameters as bytes. What a type or a byte means is the game's event
//! templates, which the file does not carry (community definitions name them: [`template`] reads those a caller
//! supplies; this reader names none): an event is its type number, its times and its parameter bytes as stored.
//!
//! Read: the 64-bit little-endian layout of version `0x1000D`, Sekiro's and Elden Ring's (SoulsFormatsNEXT's `TAE` reads
//! it as format `SDT`); the other versions (Demon's Souls, Dark Souls, Bloodborne, Dark Souls III) are refused by their
//! version. The header's constants, the animation groups table (the runs of consecutive animation numbers, an index over
//! the animations) and each animation's times table (the distinct start and end times of its events, ascending) are
//! checked against the rest of the file; the event groups (which events of an animation the file ties together) are
//! kept as [`Animation::event_groups`].
//!
//! The parameter bytes of an event run to the start of the next event's record; the last event's to the animation's
//! event groups, else to the next animation's header block, else to the end of the file, so they include the zero padding
//! that aligns the records to 16 bytes.
//!
//! Numbering: an animation binder may hold several event files, one per animation group, in consecutive member slots
//! from the binder's first event slot (DSAnimStudio): the player's `c0000.anibnd` holds `a50.tae` in slot 5,000,050 with
//! the animations of group 50 (`a050_…`). A file stores its animations' numbers within its group; an animation's number
//! is the group times a million plus that ([`Tae::number`]), the numbering of clip names ([`clip_animation_id`]) and of
//! [`Header::ImportOther`], which may name an animation of another event file of the binder ([`resolve`]). A file read
//! on its own ([`Tae::read`]) is group 0, where the stored numbers are the animations' own: every character binder but
//! the player's holds a single event file, in its first event slot.

use crate::bytes::{Budget, View};
use crate::{Binder, BinderFile, Result, invalid};

pub mod template;

const FORMAT: &str = "TAE";

/// The only version read: Sekiro's and Elden Ring's.
pub const VERSION: u32 = 0x1000D;

/// The span of numbers one animation group holds: an animation's number is its group times this plus its number within
/// the group.
pub const GROUP_SPAN: i64 = 1_000_000;

/// The binder member ID whose presence puts an animation binder's event slots at 5,000,000 to 5,099,999 (3,000,000 to
/// 3,099,999 without it; DSAnimStudio).
const VERSIONED_BINDER_MARK: i32 = 9_999_999;

/// Seconds from which SoulsFormatsNEXT treats an event's end as "until the animation ends" (it reads a stored `f32::MAX`
/// as this placeholder, and Elden Ring stores it as such).
pub const OPEN_END: f32 = 100.0;

/// What an animation's mini header states (the block the animation record's file offset names).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Header {
    /// The block is empty: no file name offset.
    None,
    /// The standard header: the animation plays its own motion data.
    Standard {
        /// "Makes the animation loop by default" (SoulsFormatsNEXT).
        loops: bool,
        /// The animation takes its motion data from the animation numbered `import_hkx_source`.
        imports_hkx: bool,
        /// "Allow this animation to be loaded from delayload anibnds" (SoulsFormatsNEXT).
        allow_delay_load: bool,
        /// The animation number [`Header::Standard::imports_hkx`] names.
        import_hkx_source: i32,
    },
    /// The animation takes both its motion data and its events from the animation numbered `from`.
    ImportOther {
        /// The number of the animation it takes both from, group and all ([`Tae::number`]): it may be an animation of
        /// another event file of the binder.
        from: i32,
        /// A stored word whose meaning is not established.
        unknown: i32,
    },
}

/// One timed event.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Event {
    /// The event's type number, as stored.
    pub kind: i32,
    /// The word after the type, as stored (SoulsFormatsNEXT: "unknown, used in SOTFS and SDT").
    pub unk04: i32,
    /// Seconds into the animation.
    pub start: f32,
    /// Seconds into the animation, as stored: `f32::MAX` or at least [`OPEN_END`] for an event with no end of its own
    /// ([`Event::open_ended`]).
    pub end: f32,
    /// The parameter bytes as stored, with the padding that follows them.
    pub parameters: Vec<u8>,
}

impl Event {
    /// Whether the event runs until its animation ends (an end of [`OPEN_END`] seconds or more).
    pub fn open_ended(&self) -> bool {
        self.end >= OPEN_END
    }
}

/// A group of events of one animation that the file ties together: the events, and a number of the group's own.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct EventGroup {
    /// The number stored in the group's type slot, as stored. It is not always the type of the group's events (they are
    /// of one type in the groups that have any, but the number differs on 74,920 of 247,075 groups in Sekiro's files and
    /// 289,341 of 943,080 in Elden Ring's).
    pub group_type: u64,
    /// The indices of the group's events in [`Animation::events`], in the order the group lists them. A group may be
    /// empty (2,509 of Sekiro's and 49,359 of Elden Ring's are).
    pub events: Vec<usize>,
}

/// One animation's record.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Animation {
    /// The animation's number within its file's group, as stored (`3000`); [`Tae::number`] is its number.
    pub id: i64,
    /// Where the animation gets its motion data and events.
    pub header: Header,
    /// The motion file name the mini header names, where it states one (not all do).
    pub file_name: Option<String>,
    /// The timed events, in file order.
    pub events: Vec<Event>,
    /// The event groups, in file order (none in 3,422 of Sekiro's 15,769 animations and 6,050 of Elden Ring's 51,151).
    pub event_groups: Vec<EventGroup>,
}

/// A TAE file.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Tae {
    /// The file's own number.
    pub id: i32,
    /// "What set of events this TAE uses" (SoulsFormatsNEXT): the event bank the file's types are numbered in.
    pub event_bank: i64,
    /// The skeleton file name the file states, if any.
    pub skeleton_name: Option<String>,
    /// The SIB file name the file states, if any.
    pub sib_name: Option<String>,
    /// The animation group the file holds, from its slot in its animation binder ([`Tae::read_member`]); 0 for a file
    /// read on its own.
    pub group: i64,
    /// The animations, in file order.
    pub animations: Vec<Animation>,
}

impl AsRef<Tae> for Tae {
    fn as_ref(&self) -> &Tae {
        self
    }
}

fn size(v: &View<'_>, o: usize) -> Result<usize> {
    v.size(o)
}

/// The header words that hold the same value in every file: Sekiro's and Elden Ring's (all 326 and 1,071 event files of
/// the installs). The words that vary are read where they are used: the bank (0x30), the file number (0x50, repeated at
/// 0x90 and 0x94), the animation count (0x54, repeated at 0x70), the tables' offsets (0x58, 0x60), the offset of the first
/// animation's body (0x78, 0 where there is none) and the two name offsets (0xb0, 0xb8).
fn header_constants(v: &View<'_>, id: i32, count: usize, first_body: Option<usize>) -> Result<()> {
    let words = |at: usize, want: &[u64]| -> Result<()> {
        for (i, &want) in want.iter().enumerate() {
            let o = v.element(at, i, 8)?;
            let got = v.u64(o)?;
            if got != want {
                return Err(v.err(o, format!("header word {got:#x}, the format holds {want:#x}")));
            }
        }
        Ok(())
    };
    if v.u32(0x0c)? as usize != v.data.len() {
        return Err(v.err(0x0c, "the file size the header states is not the file's"));
    }
    words(0x10, &[0x40, 1, 0x50, 0x80])?;
    words(0x38, &[0])?;
    v.expect(0x40, &[1, 0, 1, 2, 2, 1, 1, 1])?;
    words(0x48, &[1])?;
    words(
        0x68,
        &[
            0xa0,
            count as u64,
            first_body.map_or(0, |body| body as u64),
            1,
            0x90,
        ],
    )?;
    words(
        0x90,
        &[u64::from(id as u32) | u64::from(id as u32) << 32, 0x50, 0, 0xb0],
    )?;
    words(0xc0, &[0, 0])
}

/// The animation groups table (the group header at the offset at 0x60: the number of groups and the offset of the
/// entries, which follow the header at +16 unless the file has no animations) holds one entry (the first and last number, and the offset of the table record
/// of the first) for each maximal run of consecutive animation numbers; the numbers rise strictly through the file's table.
fn animation_runs(v: &View<'_>, table: usize, bodies: &[(i64, usize)]) -> Result<()> {
    let mut runs: Vec<(i64, i64, usize)> = Vec::new();
    for (i, &(id, _)) in bodies.iter().enumerate() {
        // the last run ends at the number before this one
        if let Some((_, last, _)) = runs.last_mut() {
            if id <= *last {
                return Err(v.err(
                    v.element(table, i, 16)?,
                    format!("animation {id} does not rise above the one before it"),
                ));
            }
            if Some(id) == last.checked_add(1) {
                *last = id;
                continue;
            }
        }
        runs.push((id, id, i));
    }
    let header = size(v, 0x60)?;
    let entries = size(v, v.at(header, 8)?)?;
    // a file without animations states no entries, and its offset is not the entries' place
    if !runs.is_empty() && entries != v.at(header, 16)? {
        return Err(v.err(
            v.at(header, 8)?,
            "the group entries do not follow the group header",
        ));
    }
    if size(v, header)? != runs.len() {
        return Err(v.err(
            header,
            format!(
                "{} animation groups, {} runs of consecutive animations",
                size(v, header)?,
                runs.len()
            ),
        ));
    }
    for (k, &(first, last, record)) in runs.iter().enumerate() {
        let at = v.element(entries, k, 16)?;
        let stored = (
            v.u32(at)? as i32,
            v.u32(v.at(at, 4)?)? as i32,
            size(v, v.at(at, 8)?)?,
        );
        if i64::from(stored.0) != first
            || i64::from(stored.1) != last
            || stored.2 != v.element(table, record, 16)?
        {
            return Err(v.err(
                at,
                format!(
                    "animation group {k} is {}..{} at {:#x}, the run is {first}..{last} at {:#x}",
                    stored.0,
                    stored.1,
                    stored.2,
                    v.element(table, record, 16)?
                ),
            ));
        }
    }
    Ok(())
}

fn float(v: &View<'_>, o: usize) -> Result<f32> {
    let n = f32::from_bits(v.u32(o)?);
    if n.is_nan() {
        return Err(invalid(FORMAT, o, "a time that is not a number"));
    }
    Ok(n)
}

/// The number of the animation a clip named `a000_003000` stands for: its group (the digits after `a`) times a million
/// plus its number (the digits after the underscore), `3000` for that one and `50003000` for `a050_003000`. This is the
/// numbering community tools use for these files; a program that finds the numbers disagree with a file's counts the
/// animations that match none.
pub fn clip_animation_id(stem: &str) -> Option<i64> {
    let rest = stem.strip_prefix('a')?;
    let (group, number) = rest.split_once('_')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(group) || !digits(number) {
        return None;
    }
    group
        .parse::<i64>()
        .ok()?
        .checked_mul(1_000_000)?
        .checked_add(number.parse::<i64>().ok()?)
}

impl Tae {
    /// Read the file `data` (already decoded from its DCX envelope, if it had one).
    pub fn read(data: &[u8]) -> Result<Tae> {
        let v = View {
            data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        v.expect(0, b"TAE ")?;
        if v.u8(4)? != 0 {
            return Err(invalid(FORMAT, 4, "big-endian files are not read"));
        }
        if v.u8(7)? != 0xFF {
            return Err(invalid(FORMAT, 7, "32-bit files are not read"));
        }
        let version = v.u32(8)?;
        if version != VERSION {
            return Err(invalid(
                FORMAT,
                8,
                format!("version {version:#x}, the one read is {VERSION:#x} (Sekiro, Elden Ring)"),
            ));
        }
        let budget = Budget::new(FORMAT, data.len());
        let event_bank = v.u64(48)? as i64;
        let id = v.u32(80)? as i32;
        let count = v.u32(84)? as usize;
        let table = size(&v, 88)?;
        let name = |at: usize| -> Result<Option<String>> {
            match size(&v, at)? {
                0 => Ok(None),
                // 15 of Sekiro's weapon-part files (table at 0xd0) point both names at the table: no name
                offset if offset == table => Ok(None),
                offset => Ok(Some(v.utf16z(offset, &budget)?.0)),
            }
        };
        let skeleton_name = name(176)?;
        let sib_name = name(184)?;
        // each record names its animation's body: a number and an offset
        let bytes = count
            .checked_mul(16)
            .ok_or_else(|| invalid(FORMAT, 84, "size overflow"))?;
        v.slice(table, bytes)?;
        let mut bodies = Vec::with_capacity(count);
        for i in 0..count {
            let at = v.element(table, i, 16)?;
            let slot = v.at(at, 8)?;
            let body = size(&v, slot)?;
            if body >= data.len() {
                return Err(invalid(
                    FORMAT,
                    slot,
                    format!(
                        "animation body at {body:#x} is beyond the {} bytes of the file",
                        data.len()
                    ),
                ));
            }
            bodies.push((v.u64(at)? as i64, body));
        }
        header_constants(&v, id, count, bodies.first().map(|&(_, body)| body))?;
        animation_runs(&v, table, &bodies)?;
        let mut animations = Vec::with_capacity(count);
        for (i, &(animation_id, body)) in bodies.iter().enumerate() {
            // where this animation's event parameters stop at the latest: the next animation's header block
            let next_block = bodies
                .get(i.saturating_add(1))
                .map(|&(_, b)| v.at(b, 24).and_then(|at| size(&v, at)))
                .transpose()?;
            animations.push(read_animation(&v, &budget, animation_id, body, next_block)?);
        }
        Ok(Tae {
            id,
            event_bank,
            skeleton_name,
            sib_name,
            group: 0,
            animations,
        })
    }

    /// Read the event file `member` of the animation binder `binder` (the member's bytes as the binder decoded them),
    /// its group the member's slot past the binder's first event slot. A member outside the binder's event slots is
    /// refused.
    pub fn read_member(binder: &Binder, member: &BinderFile) -> Result<Tae> {
        let (first, last) = if binder.files.iter().any(|f| f.id == Some(VERSIONED_BINDER_MARK)) {
            (5_000_000, 5_099_999)
        } else {
            (3_000_000, 3_099_999)
        };
        let group = member
            .id
            .filter(|slot| (first..=last).contains(slot))
            .and_then(|slot| slot.checked_sub(first))
            .ok_or_else(|| {
                invalid(
                    FORMAT,
                    0,
                    format!(
                        "event file {} in binder slot {:?}, outside the binder's event slots {first} to {last}",
                        member.name.as_deref().unwrap_or("?"),
                        member.id,
                    ),
                )
            })?;
        let mut tae = Tae::read(&member.bytes)?;
        tae.group = i64::from(group);
        Ok(tae)
    }

    /// The number of `animation`, one of this file's: the file's group times [`GROUP_SPAN`] plus its stored number.
    pub fn number(&self, animation: &Animation) -> i64 {
        self.group.saturating_mul(GROUP_SPAN).saturating_add(animation.id)
    }

    /// The animation numbered `number` ([`Tae::number`]).
    pub fn animation(&self, number: i64) -> Option<&Animation> {
        self.animations.iter().find(|a| self.number(a) == number)
    }

    /// [`resolve`] within this file alone.
    pub fn resolved_animation(&self, number: i64) -> Result<Option<&Animation>> {
        Ok(resolve(std::slice::from_ref(self), number)?.map(|(_, a)| a))
    }
}

/// The record whose events the animation numbered `number` plays, among the event files `files` of a binder, with the
/// index of the file it is in: the first file with an animation of that number, then each import
/// ([`Header::ImportOther`]) followed to the animation it names in any of the files; a negative number names none, and
/// the animation plays its own record (DSAnimStudio). `None` where no file has the animation; an import naming an
/// animation no file has, or a cycle of imports, is an error. Motion-only imports keep their own events; only
/// ImportOther inherits events as well as motion.
pub fn resolve<T: AsRef<Tae>>(files: &[T], number: i64) -> Result<Option<(usize, &Animation)>> {
    let find = |n: i64| {
        files
            .iter()
            .enumerate()
            .find_map(|(i, f)| f.as_ref().animation(n).map(|a| (i, a)))
    };
    let Some(mut found) = find(number) else {
        return Ok(None);
    };
    let mut seen = std::collections::BTreeSet::new();
    while let Header::ImportOther { from, .. } = &found.1.header
        && *from >= 0
    {
        if !seen.insert((found.0, found.1.id)) {
            return Err(invalid(
                FORMAT,
                0,
                format!("animation {number}: cyclic event import"),
            ));
        }
        found = find(i64::from(*from)).ok_or_else(|| {
            invalid(
                FORMAT,
                0,
                format!("animation {number}: event import {from} absent"),
            )
        })?;
    }
    Ok(Some(found))
}

fn read_animation(
    v: &View<'_>,
    budget: &Budget,
    id: i64,
    body: usize,
    next_block: Option<usize>,
) -> Result<Animation> {
    let headers = size(v, body)?;
    let groups = size(v, v.at(body, 8)?)?;
    let times = size(v, v.at(body, 16)?)?;
    let block = size(v, v.at(body, 24)?)?;
    let count_at = v.at(body, 32)?;
    let count = v.u32(count_at)? as usize;
    let bytes = count
        .checked_mul(24)
        .ok_or_else(|| invalid(FORMAT, count_at, "size overflow"))?;
    v.slice(headers, bytes)?;
    // each event: where its start and end times and its data are
    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let at = v.element(headers, i, 24)?;
        let (start, end, data) = (size(v, at)?, size(v, v.at(at, 8)?)?, size(v, v.at(at, 16)?)?);
        let parameters = v.at(data, 16)?;
        // the record's parameter offset is the next word after it, or 0
        let stored_at = v.at(data, 8)?;
        let stored = size(v, stored_at)?;
        if stored != 0 && stored != parameters {
            return Err(invalid(
                FORMAT,
                stored_at,
                format!("event {i} of animation {id} names parameters at {stored:#x}, not after its record"),
            ));
        }
        records.push((start, end, data, parameters));
    }
    let limit = if groups != 0 {
        groups
    } else {
        next_block.unwrap_or(v.data.len())
    };
    let mut events = Vec::with_capacity(count);
    for (i, &(start, end, data, parameters)) in records.iter().enumerate() {
        // the parameters run to the next record's start, which follows them in the file
        let stop = match records.get(i.saturating_add(1)) {
            Some(&(_, _, next, _)) => next,
            None => limit,
        };
        let length = stop.checked_sub(parameters).ok_or_else(|| {
            invalid(
                FORMAT,
                data,
                format!("event {i} of animation {id}: its parameters would end before they start (events not in file order)"),
            )
        })?;
        budget.charge(parameters, length)?;
        events.push(Event {
            kind: v.u32(data)? as i32,
            unk04: v.u32(v.at(data, 4)?)? as i32,
            start: float(v, start)?,
            end: float(v, end)?,
            parameters: v.slice(parameters, length)?.to_vec(),
        });
    }
    check_times(v, id, body, times, &events)?;
    let event_groups = read_event_groups(v, budget, id, body, groups, headers, events.len())?;
    let (header, file_name) = read_block(v, budget, block, times);
    Ok(Animation {
        id,
        header,
        file_name,
        events,
        event_groups,
    })
}

/// The times table (the offset at body +16, its length the u64 at body +40) holds the distinct start and end times of the
/// animation's events, by bit pattern, in ascending order.
fn check_times(v: &View<'_>, id: i64, body: usize, times: usize, events: &[Event]) -> Result<()> {
    let mut expected: Vec<f32> = events.iter().flat_map(|e| [e.start, e.end]).collect();
    expected.sort_by(f32::total_cmp);
    expected.dedup_by(|a, b| a.to_bits() == b.to_bits());
    let count_at = v.at(body, 40)?;
    if size(v, count_at)? != expected.len() {
        return Err(invalid(
            FORMAT,
            count_at,
            format!(
                "animation {id}: {} times stored, its events have {} distinct start and end times",
                size(v, count_at)?,
                expected.len()
            ),
        ));
    }
    for (i, time) in expected.iter().enumerate() {
        let at = v.element(times, i, 4)?;
        if v.u32(at)? != time.to_bits() {
            return Err(invalid(
                FORMAT,
                at,
                format!(
                    "animation {id}: time {i} of the times table is not the animation's next distinct event time"
                ),
            ));
        }
    }
    Ok(())
}

/// The event groups of the animation whose body is at `body` (the group array at `array`, its length the u32 at body
/// +36): each group is 32 bytes, a member count, the offset of that many u32 offsets of event headers of the animation,
/// the offset of the group's type number (a u64) and a zero word.
fn read_event_groups(
    v: &View<'_>,
    budget: &Budget,
    id: i64,
    body: usize,
    array: usize,
    headers: usize,
    events: usize,
) -> Result<Vec<EventGroup>> {
    let count_at = v.at(body, 36)?;
    let n = v.u32(count_at)? as usize;
    if n == 0 {
        return Ok(Vec::new());
    }
    if array == 0 {
        return Err(invalid(
            FORMAT,
            v.at(body, 8)?,
            format!("animation {id}: {n} event groups and no group array"),
        ));
    }
    let bytes = n
        .checked_mul(32)
        .ok_or_else(|| invalid(FORMAT, count_at, "size overflow"))?;
    v.slice(array, bytes)?;
    budget.charge(array, bytes)?;
    let mut groups = Vec::with_capacity(n);
    for k in 0..n {
        let at = v.element(array, k, 32)?;
        let members = size(v, at)?;
        let values = size(v, v.at(at, 8)?)?;
        let kind = size(v, v.at(at, 16)?)?;
        v.expect_at(at, 24, &[0; 8])?;
        let bytes = members
            .checked_mul(4)
            .ok_or_else(|| invalid(FORMAT, at, "size overflow"))?;
        v.slice(values, bytes)?;
        budget.charge(values, bytes)?;
        let mut list = Vec::with_capacity(members);
        for j in 0..members {
            let word_at = v.element(values, j, 4)?;
            let event = (v.u32(word_at)? as usize)
                .checked_sub(headers)
                .filter(|delta| delta % 24 == 0 && delta / 24 < events)
                .map(|delta| delta / 24)
                .ok_or_else(|| {
                    invalid(
                        FORMAT,
                        word_at,
                        format!("animation {id}: event group {k} names a place that is not one of its event headers"),
                    )
                })?;
            list.push(event);
        }
        groups.push(EventGroup {
            group_type: v.u64(kind)?,
            events: list,
        });
    }
    Ok(groups)
}

/// The mini header block at `block` and the file name it names: `Header::None` and no name where the block states
/// neither or is malformed (the block is the part of the record SoulsFormatsNEXT itself reads heuristically).
fn read_block(v: &View<'_>, budget: &Budget, block: usize, times: usize) -> (Header, Option<String>) {
    let read = || -> Result<(Header, Option<String>)> {
        let kind = v.u32(block)?;
        let record = v.u32(v.at(block, 8)?)? as usize;
        if record == 0 {
            return Ok((Header::None, None));
        }
        let name_at = v.u32(record)? as usize;
        let header = match kind {
            0 => Header::Standard {
                loops: v.u8(v.at(record, 8)?)? != 0,
                imports_hkx: v.u8(v.at(record, 9)?)? != 0,
                allow_delay_load: v.u8(v.at(record, 10)?)? != 0,
                import_hkx_source: v.u32(v.at(record, 12)?)? as i32,
            },
            1 => Header::ImportOther {
                from: v.u32(v.at(record, 8)?)? as i32,
                unknown: v.u32(v.at(record, 12)?)? as i32,
            },
            other => return Err(invalid(FORMAT, block, format!("mini header type {other}"))),
        };
        // the name is the text at `name_at`, unless that is the times table or a value that reads as a time
        let name = if name_at < v.data.len() && name_at != times && v.u64(name_at).ok() != Some(1) {
            let as_time = f32::from_bits(v.u32(name_at)?);
            if (0.016667..=100.0).contains(&as_time) {
                None
            } else {
                Some(v.utf16z(name_at, budget)?.0)
            }
        } else {
            None
        };
        Ok((header, name))
    };
    read().unwrap_or((Header::None, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(buf: &mut Vec<u8>, at: usize, bytes: &[u8]) {
        if buf.len() < at + bytes.len() {
            buf.resize(at + bytes.len(), 0);
        }
        buf[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn u32le(buf: &mut Vec<u8>, at: usize, n: u32) {
        put(buf, at, &n.to_le_bytes());
    }
    fn u64le(buf: &mut Vec<u8>, at: usize, n: u64) {
        put(buf, at, &n.to_le_bytes());
    }
    fn f32le(buf: &mut Vec<u8>, at: usize, n: f32) {
        put(buf, at, &n.to_le_bytes());
    }

    /// Where the parts of [`file_of`] sit: the body of the animations (all share one), the mini header block, its record
    /// and the name, the times table, the event headers, the two events, the event group array with its values and type,
    /// and the group header.
    const BODY: usize = 0xd0;
    const BLOCK: usize = 0x100;
    const RECORD: usize = 0x110;
    const NAME: usize = 0x120;
    const TIMES: usize = 0x140;
    const HEADERS: usize = 0x150;
    const EVENT0: usize = 0x180;
    const EVENT1: usize = 0x1a0;
    const GROUPS: usize = 0x1c0;
    const GROUP_VALUES: usize = 0x1e0;
    const GROUP_TYPE: usize = 0x1e8;
    const GROUP_HEADER: usize = 0x1f0;

    /// A file whose animations have the numbers `ids`, all with the one body: two events (type 0 from 0 to 0.5 s and
    /// type 5 from 0.5 s to the end) tied by one event group of type number 7, the mini header naming `a000_003000.hkt`.
    /// The animation groups table has an entry for each maximal run of consecutive numbers, and the file is as long as its
    /// header says. With `event_groups` false the body has none and the last event's parameters run to the table.
    fn file_of(ids: &[i64], event_groups: bool) -> Vec<u8> {
        let mut runs: Vec<(i64, i64, usize)> = Vec::new();
        for (i, &id) in ids.iter().enumerate() {
            match runs.last_mut() {
                Some((_, last, _)) if id == *last + 1 => *last = id,
                _ => runs.push((id, id, i)),
            }
        }
        let entries = GROUP_HEADER + 16;
        let table = entries + 16 * runs.len();
        let mut b = vec![0u8; table + 16 * ids.len()];
        put(&mut b, 0, b"TAE ");
        b[7] = 0xff;
        u32le(&mut b, 8, VERSION);
        for (at, word) in [
            (0x10, 0x40),
            (0x18, 1),
            (0x20, 0x50),
            (0x28, 0x80),
            (0x30, 7), // the event bank
            (0x48, 1),
            (0x58, table as u64),
            (0x60, GROUP_HEADER as u64),
            (0x68, 0xa0),
            (0x70, ids.len() as u64),
            (0x78, if ids.is_empty() { 0 } else { BODY as u64 }),
            (0x80, 1),
            (0x88, 0x90),
            (0x90, 1010 | 1010 << 32), // the file's number, twice
            (0x98, 0x50),
            (0xa8, 0xb0),
        ] {
            u64le(&mut b, at, word);
        }
        put(&mut b, 0x40, &[1, 0, 1, 2, 2, 1, 1, 1]);
        u32le(&mut b, 0x50, 1010);
        u32le(&mut b, 0x54, ids.len() as u32);
        // the group header and its entries, the table
        u64le(&mut b, GROUP_HEADER, runs.len() as u64);
        u64le(&mut b, GROUP_HEADER + 8, entries as u64);
        for (k, &(first, last, record)) in runs.iter().enumerate() {
            put(&mut b, entries + 16 * k, &(first as i32).to_le_bytes());
            put(&mut b, entries + 16 * k + 4, &(last as i32).to_le_bytes());
            u64le(&mut b, entries + 16 * k + 8, (table + 16 * record) as u64);
        }
        for (i, &id) in ids.iter().enumerate() {
            u64le(&mut b, table + 16 * i, id as u64);
            u64le(&mut b, table + 16 * i + 8, BODY as u64);
        }
        // the body: events at HEADERS, the groups, times, the block, two events, one group, three distinct times
        u64le(&mut b, BODY, HEADERS as u64);
        u64le(&mut b, BODY + 8, if event_groups { GROUPS as u64 } else { 0 });
        u64le(&mut b, BODY + 16, TIMES as u64);
        u64le(&mut b, BODY + 24, BLOCK as u64);
        u32le(&mut b, BODY + 32, 2);
        u32le(&mut b, BODY + 36, u32::from(event_groups));
        u64le(&mut b, BODY + 40, 3);
        // the block: standard type, its record
        u32le(&mut b, BLOCK, 0);
        u64le(&mut b, BLOCK + 8, RECORD as u64);
        // the record: the name, loops, no import, source -1
        u32le(&mut b, RECORD, NAME as u32);
        put(&mut b, RECORD + 8, &[1]);
        u32le(&mut b, RECORD + 12, u32::MAX);
        let name: Vec<u8> = "a000_003000.hkt\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        put(&mut b, NAME, &name);
        // the times: 0, 0.5 and the placeholder for no end
        f32le(&mut b, TIMES, 0.0);
        f32le(&mut b, TIMES + 4, 0.5);
        f32le(&mut b, TIMES + 8, f32::MAX);
        // the headers: (start, end, data) of each event
        for (i, (start, end, data)) in [(TIMES, TIMES + 4, EVENT0), (TIMES + 4, TIMES + 8, EVENT1)]
            .into_iter()
            .enumerate()
        {
            u64le(&mut b, HEADERS + i * 24, start as u64);
            u64le(&mut b, HEADERS + i * 24 + 8, end as u64);
            u64le(&mut b, HEADERS + i * 24 + 16, data as u64);
        }
        // the events: type, word, parameters offset, 16 parameter bytes
        for (data, kind, fill) in [(EVENT0, 0u32, 0xaau8), (EVENT1, 5, 0xbb)] {
            u32le(&mut b, data, kind);
            u32le(&mut b, data + 4, 9);
            u64le(&mut b, data + 8, data as u64 + 16);
            put(&mut b, data + 16, &[fill; 16]);
        }
        // the event group: both events (by the offsets of their headers), its type number
        u64le(&mut b, GROUPS, 2);
        u64le(&mut b, GROUPS + 8, GROUP_VALUES as u64);
        u64le(&mut b, GROUPS + 16, GROUP_TYPE as u64);
        u32le(&mut b, GROUP_VALUES, HEADERS as u32);
        u32le(&mut b, GROUP_VALUES + 4, HEADERS as u32 + 24);
        u64le(&mut b, GROUP_TYPE, 7);
        let len = b.len() as u32;
        u32le(&mut b, 0x0c, len);
        b
    }

    fn file() -> Vec<u8> {
        file_of(&[3000], true)
    }

    /// The offset of the table record of animation `i` of a file whose numbers are one run.
    fn record(i: usize) -> usize {
        GROUP_HEADER + 32 + 16 * i
    }

    #[test]
    fn an_animations_events_come_with_their_times_and_parameter_bytes() {
        let tae = Tae::read(&file()).unwrap();
        assert_eq!((tae.id, tae.event_bank, tae.animations.len()), (1010, 7, 1));
        let a = tae.animation(3000).unwrap();
        assert_eq!(a.file_name.as_deref(), Some("a000_003000.hkt"));
        assert_eq!(
            a.header,
            Header::Standard {
                loops: true,
                imports_hkx: false,
                allow_delay_load: false,
                import_hkx_source: -1
            }
        );
        assert_eq!(a.events.len(), 2);
        assert_eq!(
            (a.events[0].kind, a.events[0].start, a.events[0].end),
            (0, 0.0, 0.5)
        );
        assert_eq!(a.events[0].parameters, vec![0xaa; 16]);
        assert_eq!(a.events[1].kind, 5);
        assert_eq!(a.events[1].unk04, 9);
        assert!(a.events[1].open_ended());
        assert!(!a.events[0].open_ended());
        // the last event's parameters run to the event group array
        assert_eq!(a.events[1].parameters, vec![0xbb; 16]);
        assert_eq!(
            a.event_groups,
            [EventGroup {
                group_type: 7,
                events: vec![0, 1]
            }]
        );
        // without the array the last event's parameters run to the end of the animation's data
        let tae = Tae::read(&file_of(&[3000], false)).unwrap();
        assert!(tae.animations[0].event_groups.is_empty());
        let data = file_of(&[3000], false);
        assert_eq!(
            tae.animations[0].events[1].parameters.len(),
            data.len() - (EVENT1 + 16)
        );
    }

    /// A name offset that is the animation table's offset names nothing (15 of Sekiro's weapon-part files state both so).
    #[test]
    fn a_name_that_points_at_the_animation_table_is_no_name() {
        let mut b = file();
        assert_eq!(
            (
                Tae::read(&b).unwrap().skeleton_name,
                Tae::read(&b).unwrap().sib_name
            ),
            (None, None)
        );
        let name: Vec<u8> = "skeleton\0".encode_utf16().flat_map(u16::to_le_bytes).collect();
        put(&mut b, NAME, &name);
        u64le(&mut b, 0xb0, NAME as u64);
        assert_eq!(Tae::read(&b).unwrap().skeleton_name.as_deref(), Some("skeleton"));
        let table = record(0);
        u64le(&mut b, 0xb0, table as u64);
        u64le(&mut b, 0xb8, table as u64);
        let tae = Tae::read(&b).unwrap();
        assert_eq!((tae.skeleton_name, tae.sib_name), (None, None));
    }

    #[test]
    fn other_versions_and_widths_are_refused_by_what_they_are() {
        let mut other = file();
        u32le(&mut other, 8, 0x1000c);
        assert!(Tae::read(&other).unwrap_err().to_string().contains("0x1000c"));
        let mut narrow = file();
        narrow[7] = 0;
        assert!(Tae::read(&narrow).unwrap_err().to_string().contains("32-bit"));
        let mut big = file();
        big[4] = 1;
        assert!(Tae::read(&big).is_err());
        assert!(Tae::read(&file()[..0x100]).is_err());
    }

    /// Every header word that holds one value in all of Sekiro's and Elden Ring's files is checked, and a file with
    /// none is a file without animations.
    #[test]
    fn a_header_word_that_differs_from_the_formats_constant_is_refused() {
        for (at, edit) in [
            (0x0c, 1u8),
            (0x10, 1),
            (0x18, 2),
            (0x20, 3),
            (0x28, 4),
            (0x38, 5),
            (0x40, 0),
            (0x47, 9),
            (0x48, 7),
            (0x49, 1),
            (0x68, 1),
            (0x70, 5),
            (0x78, 5),
            (0x80, 0),
            (0x88, 2),
            (0x90, 3),
            (0x94, 4),
            (0x98, 7),
            (0xa0, 1),
            (0xa8, 1),
            (0xc0, 1),
            (0xc8, 1),
        ] {
            let mut b = file();
            b[at] ^= edit.max(1);
            let e = Tae::read(&b).unwrap_err().to_string();
            // the error names the word or run of bytes the edit falls in
            let word = match at {
                0x40..=0x47 => 0x40,
                0x48..=0x4f => 0x48,
                0x94 => 0x90,
                at => at,
            };
            assert!(e.starts_with(&format!("TAE at {word:#x}")), "{at:#x}: {e}");
        }
        // a file without animations: no group entries, no first body
        let mut empty = file_of(&[], false);
        assert!(Tae::read(&empty).unwrap().animations.is_empty());
        // its group header's entries offset is not the place after the header
        u64le(&mut empty, GROUP_HEADER + 8, 0x118);
        assert!(Tae::read(&empty).unwrap().animations.is_empty());
    }

    /// The animation groups table has an entry for each maximal run of consecutive animation numbers, and the numbers rise.
    #[test]
    fn the_animation_groups_table_is_the_runs_of_consecutive_numbers() {
        let ids = [3000, 3001, 3002, 3010, 3011, 20_000];
        let b = file_of(&ids, true);
        assert_eq!(Tae::read(&b).unwrap().animations.len(), 6);
        let entries = GROUP_HEADER + 16;
        // one entry too few, a run end moved, an offset moved, entries not after the header
        let mut few = b.clone();
        few[GROUP_HEADER] = 2;
        assert!(
            Tae::read(&few)
                .unwrap_err()
                .to_string()
                .contains("2 animation groups, 3 runs")
        );
        let mut last = b.clone();
        last[entries + 4] = 0xb9;
        assert!(
            Tae::read(&last)
                .unwrap_err()
                .to_string()
                .contains("animation group 0")
        );
        let mut offset = b.clone();
        offset[entries + 16 + 8] ^= 0x10;
        assert!(
            Tae::read(&offset)
                .unwrap_err()
                .to_string()
                .contains("animation group 1")
        );
        let mut moved = b.clone();
        moved[GROUP_HEADER + 8] ^= 0x10;
        assert!(
            Tae::read(&moved)
                .unwrap_err()
                .to_string()
                .contains("do not follow")
        );
        // the numbers rise strictly
        for ids in [[3001, 3000], [3000, 3000]] {
            let e = Tae::read(&file_of(&ids, true)).unwrap_err().to_string();
            assert!(e.contains("does not rise above"), "{e}");
        }
    }

    /// The event groups name events of their animation by the offsets of their headers, and are 32 bytes with a zero last
    /// word; a count with no array is refused.
    #[test]
    fn a_damaged_event_group_is_refused() {
        let mut outside = file();
        u32le(&mut outside, GROUP_VALUES + 4, HEADERS as u32 + 48);
        assert!(
            Tae::read(&outside)
                .unwrap_err()
                .to_string()
                .contains("not one of its event headers")
        );
        let mut between = file();
        u32le(&mut between, GROUP_VALUES, HEADERS as u32 + 4);
        assert!(Tae::read(&between).is_err());
        let mut fourth = file();
        fourth[GROUPS + 24] = 1;
        assert!(Tae::read(&fourth).is_err());
        let mut missing = file();
        u64le(&mut missing, BODY + 8, 0);
        assert!(
            Tae::read(&missing)
                .unwrap_err()
                .to_string()
                .contains("no group array")
        );
        let mut beyond = file();
        u64le(&mut beyond, GROUPS, 1 << 40);
        assert!(Tae::read(&beyond).is_err());
        // an empty group is read
        let mut empty = file();
        u64le(&mut empty, GROUPS, 0);
        assert!(
            Tae::read(&empty).unwrap().animations[0].event_groups[0]
                .events
                .is_empty()
        );
    }

    /// The times table is the animation's distinct event times, ascending: its length is the u64 at body +40.
    #[test]
    fn the_times_table_is_the_distinct_event_times() {
        let mut count = file();
        u64le(&mut count, BODY + 40, 2);
        assert!(
            Tae::read(&count)
                .unwrap_err()
                .to_string()
                .contains("2 times stored")
        );
        // the second event ends at a time the table does not hold
        let mut value = file();
        f32le(&mut value, TIMES + 12, 0.75);
        u64le(&mut value, HEADERS + 24 + 8, TIMES as u64 + 12);
        let e = Tae::read(&value).unwrap_err().to_string();
        assert!(e.starts_with("TAE at 0x148") && e.contains("time 2"), "{e}");
        let mut order = file();
        f32le(&mut order, TIMES, 0.5);
        f32le(&mut order, TIMES + 4, 0.0);
        assert!(Tae::read(&order).is_err());
    }

    #[test]
    fn events_out_of_file_order_are_refused() {
        let mut b = file();
        // the second event's record where the first's is, so before the first's parameters
        u64le(&mut b, HEADERS + 24 + 16, EVENT0 as u64);
        assert!(Tae::read(&b).is_err());
    }

    fn member(id: i32, bytes: Vec<u8>) -> BinderFile {
        BinderFile {
            id: Some(id),
            name: Some(format!("{id}.tae")),
            flags: 0,
            bytes,
            wrap: None,
        }
    }

    #[test]
    fn a_member_is_the_group_its_slot_makes_and_its_animations_are_numbered_in_it() {
        // a binder holding member 9,999,999 has its event slots from 5,000,000
        let binder = Binder {
            version: [0; 8],
            format: 0,
            layout: crate::binder::Layout::default(),
            files: vec![member(5_000_050, file()), member(9_999_999, Vec::new())],
        };
        let tae = Tae::read_member(&binder, &binder.files[0]).unwrap();
        assert_eq!(tae.group, 50);
        assert_eq!(tae.number(&tae.animations[0]), 50_003_000);
        assert!(tae.animation(3000).is_none());
        assert_eq!(tae.animation(50_003_000).unwrap().id, 3000);
        // without it, from 3,000,000; a member outside the slots is refused, naming its slot
        let binder = Binder {
            version: [0; 8],
            format: 0,
            layout: crate::binder::Layout::default(),
            files: vec![member(3_000_000, file()), member(5_000_050, file())],
        };
        assert_eq!(Tae::read_member(&binder, &binder.files[0]).unwrap().group, 0);
        let e = Tae::read_member(&binder, &binder.files[1])
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("slot Some(5000050)") && e.contains("3000000 to 3099999"),
            "{e}"
        );
        // a file read on its own is group 0
        assert_eq!(Tae::read(&file()).unwrap().group, 0);
    }

    #[test]
    fn an_import_names_an_animation_of_any_event_file_of_the_binder() {
        let animation = |id, header| Animation {
            id,
            header,
            file_name: None,
            events: Vec::new(),
            event_groups: Vec::new(),
        };
        let tae = |group, animations| Tae {
            id: 0,
            event_bank: 0,
            skeleton_name: None,
            sib_name: None,
            group,
            animations,
        };
        let import = |from| Header::ImportOther { from, unknown: 0 };
        // group 50's 122301 imports 50_122_302 (its own file's 122302), which imports group 0's 3000
        let files = [
            tae(0, vec![animation(3000, Header::None)]),
            tae(
                50,
                vec![
                    animation(122_301, import(50_122_302)),
                    animation(122_302, import(3000)),
                ],
            ),
        ];
        let (file, found) = resolve(&files, 50_122_301).unwrap().unwrap();
        assert_eq!((file, found.id), (0, 3000));
        assert!(resolve(&files, 122_301).unwrap().is_none());
        // within the group-50 file alone the second import's target is absent
        let e = files[1].resolved_animation(50_122_301).unwrap_err().to_string();
        assert!(e.contains("event import 3000 absent"), "{e}");
        // a cycle across files is refused
        let cycle = [
            tae(0, vec![animation(3000, import(50_000_001))]),
            tae(50, vec![animation(1, import(3000))]),
        ];
        assert!(resolve(&cycle, 3000).unwrap_err().to_string().contains("cyclic"));
        // a negative import names none: the animation's own record
        let own = [tae(0, vec![animation(5010, import(-1))])];
        assert_eq!(resolve(&own, 5010).unwrap().unwrap().1.id, 5010);
    }

    #[test]
    fn a_clip_is_the_animation_its_group_and_number_make() {
        assert_eq!(clip_animation_id("a000_003000"), Some(3000));
        assert_eq!(clip_animation_id("a050_003000"), Some(50_003_000));
        assert_eq!(clip_animation_id("skeleton"), None);
        assert_eq!(clip_animation_id("a00x_003000"), None);
        assert_eq!(clip_animation_id("a000_"), None);
    }

    #[test]
    fn an_animation_body_beyond_the_file_is_an_error_at_the_table_slot() {
        for body in [u64::MAX - 3, u64::MAX - 40, 1 << 50] {
            // the first animation's body, then the second's (which the first's end is read from)
            for index in [0, 1] {
                let mut b = file_of(&[3000, 3001], true);
                let slot = record(index) + 8;
                u64le(&mut b, slot, body);
                let e = Tae::read(&b).unwrap_err().to_string();
                assert!(
                    e.starts_with(&format!("TAE at {slot:#x}: animation body at "))
                        && e.contains("is beyond the"),
                    "{e}"
                );
            }
        }
    }
}
