//! TAE files (`.tae`): the time-action event tables of a character's animation binder. One file holds the animations of
//! a character by number (`3000`, `20010`) and, for each, the timed events the game runs while it plays: a type, a start
//! and an end in seconds, and the event's parameters as bytes. What a type or a byte means is the game's event
//! templates, which the file does not carry (community definitions name them; this reader names none): an event is its
//! type number, its times and its parameter bytes as stored.
//!
//! Read: the 64-bit little-endian layout of version `0x1000D`, Sekiro's and Elden Ring's (SoulsFormatsNEXT's `TAE` reads
//! it as format `SDT`); the other versions (Demon's Souls, Dark Souls, Bloodborne, Dark Souls III) are refused by their
//! version. Not read: the animation groups table (an index over the animations), and the event groups (which events the
//! file ties together).
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
    let (header, file_name) = read_block(v, budget, block, times);
    Ok(Animation {
        id,
        header,
        file_name,
        events,
    })
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

    /// A one-animation file: animation 3000, two events (type 0 from 0 to 0.5 s and type 5 from 0.5 s to the end), the
    /// mini header naming `a000_003000.hkt`.
    fn file() -> Vec<u8> {
        let mut b = vec![0u8; 0xd0];
        put(&mut b, 0, b"TAE ");
        b[7] = 0xff;
        u32le(&mut b, 8, VERSION);
        u64le(&mut b, 48, 7); // event bank
        u32le(&mut b, 80, 1010); // the file's number
        u32le(&mut b, 84, 1); // one animation
        u64le(&mut b, 88, 0xd0); // the table
        // the table: animation 3000, its body at 0xe0
        u64le(&mut b, 0xd0, 3000);
        u64le(&mut b, 0xd8, 0xe0);
        // the body: events at 0x170, no groups, times at 0x160, the block at 0x110
        u64le(&mut b, 0xe0, 0x170);
        u64le(&mut b, 0xe0 + 8, 0);
        u64le(&mut b, 0xe0 + 16, 0x160);
        u64le(&mut b, 0xe0 + 24, 0x110);
        u32le(&mut b, 0xe0 + 32, 2);
        u32le(&mut b, 0xe0 + 40, 3);
        // the block: standard type, its record at 0x120
        u32le(&mut b, 0x110, 0);
        u32le(&mut b, 0x118, 0x120);
        // the record: the name at 0x140, loops, no import, source -1
        u32le(&mut b, 0x120, 0x140);
        put(&mut b, 0x128, &[1]);
        u32le(&mut b, 0x12c, u32::MAX);
        let name: Vec<u8> = "a000_003000.hkt\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        put(&mut b, 0x140, &name);
        // the times: 0, 0.5 and the placeholder for no end
        f32le(&mut b, 0x160, 0.0);
        f32le(&mut b, 0x164, 0.5);
        f32le(&mut b, 0x168, f32::MAX);
        // the headers: (start, end, data) of each event
        for (i, (start, end, data)) in [(0x160u64, 0x164u64, 0x1a0u64), (0x164, 0x168, 0x1c0)]
            .into_iter()
            .enumerate()
        {
            u64le(&mut b, 0x170 + i * 24, start);
            u64le(&mut b, 0x170 + i * 24 + 8, end);
            u64le(&mut b, 0x170 + i * 24 + 16, data);
        }
        // the events: type, word, parameters offset, 16 parameter bytes
        for (data, kind, fill) in [(0x1a0usize, 0u32, 0xaau8), (0x1c0, 5, 0xbb)] {
            u32le(&mut b, data, kind);
            u32le(&mut b, data + 4, 9);
            u64le(&mut b, data + 8, data as u64 + 16);
            put(&mut b, data + 16, &[fill; 16]);
        }
        b
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
        // the last event's parameters run to the end of the file
        assert_eq!(a.events[1].parameters, vec![0xbb; 16]);
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

    #[test]
    fn events_out_of_file_order_are_refused() {
        let mut b = file();
        // the second event's record before the first's parameters
        u64le(&mut b, 0x170 + 24 + 16, 0x190);
        put(&mut b, 0x190 + 8, &(0x190u64 + 16).to_le_bytes());
        assert!(Tae::read(&b).is_err());
    }

    fn member(id: i32, bytes: Vec<u8>) -> BinderFile {
        BinderFile {
            id: Some(id),
            name: Some(format!("{id}.tae")),
            flags: 0,
            bytes,
        }
    }

    #[test]
    fn a_member_is_the_group_its_slot_makes_and_its_animations_are_numbered_in_it() {
        // a binder holding member 9,999,999 has its event slots from 5,000,000
        let binder = Binder {
            version: [0; 8],
            format: 0,
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
            let mut b = file();
            u64le(&mut b, 0xd8, body);
            let e = Tae::read(&b).unwrap_err().to_string();
            assert!(
                e.starts_with("TAE at 0xd8: animation body at ") && e.contains("is beyond the"),
                "{e}"
            );
            // a later animation's body is read for the end of the one before it
            for body in [u64::MAX - 3, u64::MAX - 40] {
                let mut b = file();
                u32le(&mut b, 84, 2);
                u64le(&mut b, 88, 0x200);
                u64le(&mut b, 0x200, 3000);
                u64le(&mut b, 0x208, 0xe0);
                u64le(&mut b, 0x210, 3001);
                u64le(&mut b, 0x218, body);
                let e = Tae::read(&b).unwrap_err().to_string();
                assert!(e.starts_with("TAE at 0x218: animation body at "), "{e}");
            }
        }
    }
}
