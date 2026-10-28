//! Writing binders: BND4 files and BHF4/BDF4 pairs in the layout of the games' own tools.
//!
//! The layout was found by rebuilding the 7,859 + 53,722 BND4 binders and 76 + 1,996 split pairs of both installs (and
//! the 29 binders of the `regulation.bin` files) from their parsed fields:
//!
//! ```text
//! BND4  0x00 "BND4", u8 unknown 04, u8 unknown 05, 3 zeros, u8 big-endian, u8 not-bit-big-endian, 0, u32 count,
//!       0x10 u64 64, 0x18 8 version bytes, 0x20 u64 record size, 0x28 u64 end of header (0 in a split index),
//!       0x30 u8 unicode, u8 format, u8 extended, 5 zeros, 0x38 u64 offset of the hash table (0 for none)
//!       0x40 the records, then the names (UTF-16 or ASCII, NUL-terminated, in member order, directly after the records),
//!       zeros up to a multiple of 8, the hash table, and the end of the header
//!       the members, each starting on a multiple of 16 unless it is empty (an empty member sits where the last ended);
//!       nothing follows the last member
//! hash table  u64 offset of the entries, u32 group count, bytes 10 08 08 00, the groups (u32 count, u32 first entry),
//!       the entries (u32 hash, u32 member index) by group, hash and index; the group count is the first prime from
//!       count / 7 up, a name's group its hash modulo that, and the hash of a name (backslashes read as slashes, a slash
//!       put before a name that has none, ASCII letters lowered) is 37 * hash + code unit over its UTF-16 units
//! BHF4  the same header with the end of header 0, records, names and hash table; the members are in the BDF4 file:
//!       0x00 "BDF4", the two unknown bytes, 3 zeros, the two byte-order bytes, 5 zeros, 0x10 u64 48, 0x18 the 8 version
//!       bytes, 16 zeros, and the members from 48. Both files end where their content does; the archives' entries for
//!       them are padded with zeros (to a multiple of 16 for most), which is the archive's and not the binder's.
//! ```
use std::borrow::Cow;

use primase_foundations::compress::{Compressor, Zlib};

use crate::binder::{Binder, BinderFile, normalised_format};
use crate::dcx::{self, Encoder};
use crate::emit::{Out, add, align_up, mul, narrow};
use crate::{Result, invalid};

/// How the members' bytes reach the file.
#[derive(Clone, Copy)]
enum Mode<'a> {
    /// The bytes are the stored bytes: written as they are.
    Stored,
    /// Each member is framed and compressed as its `wrap` and flags say.
    Encode(&'a Encoder<'a>),
}

/// One member as the file holds it.
struct Stored<'a> {
    /// The bytes in the file.
    bytes: Cow<'a, [u8]>,
    /// The size the record states as the decoded size.
    decoded: usize,
}

fn stored_member<'a>(
    format: &'static str,
    file: &'a BinderFile,
    index: usize,
    mode: Mode<'_>,
) -> Result<Stored<'a>> {
    if file.flags & !3 != 0 {
        return Err(invalid(
            format,
            0,
            format!("member {index} has flags {:#x}", file.flags),
        ));
    }
    match mode {
        Mode::Stored => {
            if file.flags & 1 != 0 {
                return Err(invalid(
                    format,
                    0,
                    format!(
                        "member {index} is flagged compressed and was read as stored: its decoded size is not kept"
                    ),
                ));
            }
            Ok(Stored {
                decoded: file.bytes.len(),
                bytes: Cow::Borrowed(&file.bytes),
            })
        }
        Mode::Encode(encoder) => {
            if file.flags & 1 != 0 {
                Ok(Stored {
                    bytes: Cow::Owned(Zlib.compress(&file.bytes)?),
                    decoded: file.bytes.len(),
                })
            } else if let Some(wrap) = &file.wrap {
                let framed = dcx::encode(wrap, &file.bytes, encoder)?;
                // an unflagged member states its stored size as its decoded size, as the shipped binders do
                Ok(Stored {
                    decoded: framed.len(),
                    bytes: Cow::Owned(framed),
                })
            } else {
                Ok(Stored {
                    decoded: file.bytes.len(),
                    bytes: Cow::Borrowed(&file.bytes),
                })
            }
        }
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    let mut d = 2usize;
    while d.checked_mul(d).is_some_and(|square| square <= n) {
        if n.checked_rem(d) == Some(0) {
            return false;
        }
        d = d.saturating_add(1);
    }
    true
}

/// The hash table's group count for `n` members: the first prime from `n / 7` up (2 for fewer than 21 members).
fn group_count(n: usize) -> usize {
    let mut k = n.checked_div(7).unwrap_or(0);
    while !is_prime(k) {
        k = k.saturating_add(1);
    }
    k
}

/// The hash a binder's hash table keys a member name by: backslashes read as slashes, a slash put first where the name
/// has none, ASCII capitals lowered, then `37 * hash + unit` over the UTF-16 units. Every one of the 536,561 names of
/// the installs' hashed binders has the hash it stores under this rule.
pub(crate) fn name_hash(name: &str) -> u32 {
    let mut hash = 0u32;
    let mut feed = |unit: u16| {
        let unit = if (0x41..=0x5a).contains(&unit) {
            unit.wrapping_add(32)
        } else {
            unit
        };
        hash = hash.wrapping_mul(37).wrapping_add(u32::from(unit));
    };
    if !name.starts_with(['/', '\\']) {
        feed(u16::from(b'/'));
    }
    for unit in name.encode_utf16() {
        feed(if unit == u16::from(b'\\') {
            u16::from(b'/')
        } else {
            unit
        });
    }
    hash
}

/// The hash table of the members named `names`, as it is laid out at `hashptr`.
fn hash_table(format: &'static str, big: bool, hashptr: usize, names: &[&str]) -> Result<Vec<u8>> {
    let groups = group_count(names.len());
    let groups32: u32 = narrow(format, "the hash group count", groups)?;
    let mut entries = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        entries.push((name_hash(name), narrow::<u32>(format, "a member index", i)?));
    }
    entries.sort_unstable_by_key(|&(hash, index)| (hash.checked_rem(groups32).unwrap_or(0), hash, index));
    let mut counts = vec![0u32; groups];
    for (hash, _) in &entries {
        let group = usize::try_from(hash.checked_rem(groups32).unwrap_or(0)).unwrap_or(0);
        if let Some(count) = counts.get_mut(group) {
            *count = count.saturating_add(1);
        }
    }
    let entries_at = add(format, add(format, hashptr, 16)?, mul(format, groups, 8)?)?;
    let mut out = Out::new(format, big);
    out.u64(narrow(format, "the hash entries' offset", entries_at)?);
    out.u32(groups32);
    out.bytes(&[0x10, 0x08, 0x08, 0x00]);
    let mut first = 0u32;
    for count in &counts {
        out.u32(*count);
        out.u32(first);
        first = first.saturating_add(*count);
    }
    for (hash, index) in &entries {
        out.u32(*hash);
        out.u32(*index);
    }
    Ok(out.finish())
}

impl Binder {
    /// This binder as a BND4 file. Members that were nested DCX envelopes ([`BinderFile::wrap`]) are framed again and
    /// compressed with `encoder`; members with the compression flag are zlib-compressed. The layout is the games' own:
    /// a binder read with [`Binder::read_stored`] and written with [`Binder::write_stored`] is byte-identical to the
    /// file, and one read with [`Binder::read`] and written with this reads back to the same binder, byte-identical
    /// where the compressors reproduce the shipped payloads (see [`dcx::encode`]).
    pub fn write(&self, encoder: &Encoder<'_>) -> Result<Vec<u8>> {
        Ok(self.write_files(false, Mode::Encode(encoder))?.0)
    }

    /// This binder as a BND4 file, its members' bytes written as they are: the inverse of [`Binder::read_stored`].
    pub fn write_stored(&self) -> Result<Vec<u8>> {
        Ok(self.write_files(false, Mode::Stored)?.0)
    }

    /// This binder as a BHF4 index and its BDF4 payload, members encoded as [`Binder::write`] does.
    pub fn write_split(&self, encoder: &Encoder<'_>) -> Result<(Vec<u8>, Vec<u8>)> {
        self.write_files(true, Mode::Encode(encoder))
    }

    /// This binder as a BHF4 index and its BDF4 payload, its members' bytes written as they are: the inverse of
    /// [`Binder::read_split_stored`].
    pub fn write_split_stored(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        self.write_files(true, Mode::Stored)
    }

    fn write_files(&self, split: bool, mode: Mode<'_>) -> Result<(Vec<u8>, Vec<u8>)> {
        let fmt: &'static str = if split { "BHF4" } else { "BND4" };
        let layout = &self.layout;
        let format = self.format;
        if format & 0xc1 != 0 {
            return Err(invalid(fmt, 49, "unsupported format flags"));
        }
        if normalised_format(layout.format_raw, layout.bit_big_endian) != format {
            return Err(invalid(
                fmt,
                49,
                "the format byte as stored and the normalised format do not agree",
            ));
        }
        if ![0, 1, 4, 128].contains(&layout.extended) || (split && ![0, 4].contains(&layout.extended)) {
            return Err(invalid(fmt, 50, "extended flag"));
        }
        let ids = format & 2 != 0;
        let names = format & 12 != 0;
        let long = format & 16 != 0;
        let compression = format & 32 != 0;
        let record: usize = [
            16,
            if long { 8 } else { 4 },
            if compression { 8 } else { 0 },
            if ids { 4 } else { 0 },
            if names { 4 } else { 0 },
            if format == 4 { 8 } else { 0 },
        ]
        .into_iter()
        .sum();
        let big = layout.big_endian;
        let n = self.files.len();
        let count: u32 = narrow(fmt, "the member count", n)?;

        let mut stored = Vec::with_capacity(n);
        for (i, file) in self.files.iter().enumerate() {
            stored.push(stored_member(fmt, file, i, mode)?);
        }
        let mut member_ids = Vec::new();
        if ids || format == 4 {
            for (i, file) in self.files.iter().enumerate() {
                member_ids.push(file.id.ok_or_else(|| {
                    invalid(
                        fmt,
                        0,
                        format!("member {i} has no ID, which this binder's format stores"),
                    )
                })?);
            }
        }
        let mut name_texts: Vec<&str> = Vec::new();
        if names {
            for (i, file) in self.files.iter().enumerate() {
                name_texts.push(file.name.as_deref().ok_or_else(|| {
                    invalid(
                        fmt,
                        0,
                        format!("member {i} has no name, which this binder's format stores"),
                    )
                })?);
            }
        }

        let table_end = add(fmt, 64, mul(fmt, n, record)?)?;
        let mut name_blob = Out::new(fmt, big);
        let mut name_offsets = Vec::with_capacity(name_texts.len());
        for text in &name_texts {
            name_offsets.push(add(fmt, table_end, name_blob.len())?);
            if layout.unicode {
                name_blob.utf16z(text);
            } else {
                name_blob.asciiz(text)?;
            }
        }
        let name_bytes = name_blob.finish();
        let names_end = add(fmt, table_end, name_bytes.len())?;
        let (hashptr, hash_bytes) = if layout.extended == 4 {
            if !names {
                return Err(invalid(
                    fmt,
                    50,
                    "a name hash table needs a format that stores names",
                ));
            }
            let hashptr = align_up(fmt, names_end, 8)?;
            (hashptr, hash_table(fmt, big, hashptr, &name_texts)?)
        } else {
            (0, Vec::new())
        };
        let header_end = if hashptr == 0 {
            names_end
        } else {
            add(fmt, hashptr, hash_bytes.len())?
        };

        // where each member starts: on a multiple of 16, unless it is empty
        let mut cursor = if split { 48 } else { header_end };
        let mut offsets = Vec::with_capacity(n);
        for member in &stored {
            if !member.bytes.is_empty() {
                cursor = align_up(fmt, cursor, 16)?;
            }
            offsets.push(cursor);
            cursor = add(fmt, cursor, member.bytes.len())?;
        }

        let mut index = Out::new(fmt, big);
        index.bytes(if split { b"BHF4" } else { b"BND4" });
        index.u8(u8::from(layout.unknown_04));
        index.u8(u8::from(layout.unknown_05));
        index.zeros(3);
        index.u8(u8::from(layout.big_endian));
        index.u8(u8::from(!layout.bit_big_endian));
        index.u8(0);
        index.u32(count);
        index.u64(64);
        index.bytes(&self.version);
        index.u64(narrow(fmt, "the record size", record)?);
        index.u64(if split {
            0
        } else {
            narrow(fmt, "the end of the header", header_end)?
        });
        index.u8(u8::from(layout.unicode));
        index.u8(layout.format_raw);
        index.u8(layout.extended);
        index.zeros(5);
        index.u64(narrow(fmt, "the hash table's offset", hashptr)?);
        for (i, ((file, member), offset)) in self.files.iter().zip(&stored).zip(&offsets).enumerate() {
            index.u8(if layout.bit_big_endian {
                file.flags
            } else {
                file.flags.reverse_bits()
            });
            index.bytes(&[0, 0, 0, 255, 255, 255, 255]);
            index.u64(narrow(fmt, "a member's size", member.bytes.len())?);
            if compression {
                index.u64(narrow(fmt, "a member's decoded size", member.decoded)?);
            }
            if long {
                index.u64(narrow(fmt, "a member's offset", *offset)?);
            } else {
                index.u32(narrow(fmt, "a member's offset", *offset)?);
            }
            let id = member_ids.get(i).copied();
            if ids {
                index.i32(id.unwrap_or(0));
            }
            if names {
                let at = name_offsets.get(i).copied().unwrap_or(0);
                index.u32(narrow(fmt, "a name's offset", at)?);
            }
            if format == 4 {
                index.i32(id.unwrap_or(0));
                index.u32(0);
            }
        }
        index.bytes(&name_bytes);
        if hashptr != 0 {
            index.zeros(hashptr.saturating_sub(index.len()));
            index.bytes(&hash_bytes);
        }

        if !split {
            for (member, offset) in stored.iter().zip(&offsets) {
                index.zeros(offset.saturating_sub(index.len()));
                index.bytes(&member.bytes);
            }
            return Ok((index.finish(), Vec::new()));
        }

        let mut payload = Out::new("BDF4", big);
        payload.bytes(b"BDF4");
        payload.u8(u8::from(layout.unknown_04));
        payload.u8(u8::from(layout.unknown_05));
        payload.zeros(3);
        payload.u8(u8::from(layout.big_endian));
        payload.u8(u8::from(!layout.bit_big_endian));
        payload.zeros(5);
        payload.u64(48);
        payload.bytes(&self.version);
        payload.zeros(16);
        for (member, offset) in stored.iter().zip(&offsets) {
            payload.zeros(offset.saturating_sub(payload.len()));
            payload.bytes(&member.bytes);
        }
        Ok((index.finish(), payload.finish()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binder::Layout;
    use crate::dcx::{Decoder, Format};

    fn file(id: i32, name: &str, bytes: &[u8]) -> BinderFile {
        BinderFile {
            id: Some(id),
            name: Some(name.to_owned()),
            flags: 2,
            bytes: bytes.to_vec(),
            wrap: None,
        }
    }

    fn binder(files: Vec<BinderFile>) -> Binder {
        Binder {
            version: *b"07D7R6\0\0",
            format: 0x2e,
            files,
            layout: Layout::default(),
        }
    }

    fn decoder() -> Decoder<'static> {
        Decoder {
            oodle: None,
            max_output: 1 << 20,
        }
    }

    #[test]
    fn the_name_hash_and_group_count_match_the_installs() {
        // hashes computed by the independent Python model that rebuilt the installs' 61,581 hashed binders byte for byte
        for (name, hash) in [
            (
                r"N:\NTC\data\Target\INTERROOT_win64\chr\c9800\c9800.flver",
                0x4ef7_e8e5,
            ),
            (
                r"N:\GR\data\INTERROOT_win64\asset\aeg\aeg221\aeg221_365_h.hkx",
                0xbd70_5bca,
            ),
            ("a", 0x72c),
            ("/Param/NpcParam.param", 0x725f_3790),
            ("\u{30a2}.fmg", 0x312e_a4f1),
        ] {
            assert_eq!(name_hash(name), hash, "{name}");
        }
        // the slash form and the backslash form of a name, and its case, hash alike
        assert_eq!(name_hash(r"A\B"), name_hash("/a/b"));
        for (n, groups) in [
            (0, 2),
            (1, 2),
            (7, 2),
            (14, 2),
            (21, 3),
            (28, 5),
            (35, 5),
            (42, 7),
            (100, 17),
            (350, 53),
        ] {
            assert_eq!(group_count(n), groups, "{n} members");
        }
    }

    #[test]
    fn a_binder_written_stored_reads_back_stored_and_rewrites_the_same_bytes() {
        let b = binder(vec![
            file(200, r"N:\x\a.flver", &[1; 100]),
            // empty: sits where the last member ended, which is not on a multiple of 16
            file(9_999_999, r"N:\x\ver.txt", &[]),
            file(300, r"N:\x\b.hkx", &[2; 33]),
            file(1100, r"N:\x\c.flver", &[3; 16]),
        ]);
        let bytes = b.write_stored().unwrap();
        let again = Binder::read_stored(&bytes).unwrap();
        assert_eq!(again.layout, b.layout);
        assert_eq!((again.version, again.format), (b.version, b.format));
        for (a, e) in again.files.iter().zip(&b.files) {
            assert_eq!(
                (a.id, &a.name, a.flags, &a.bytes),
                (e.id, &e.name, e.flags, &e.bytes)
            );
        }
        assert_eq!(again.write_stored().unwrap(), bytes);
        // every member but the empty one starts on a multiple of 16, nothing follows the last member
        let off = |i: usize| {
            let at = 64 + i * 36 + 24;
            u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
        };
        assert_eq!(off(0) % 16, 0);
        assert_eq!(
            off(1),
            off(0) + 100,
            "an empty member starts where the last ended"
        );
        assert_eq!(off(2), off(0) + 112);
        assert_eq!(off(3), off(2) + 48);
        assert_eq!(bytes.len(), off(3) + 16);
        // and the decoding read agrees with the stored one
        let decoded = Binder::read(&bytes, &decoder()).unwrap();
        assert_eq!(decoded.files[2].bytes, vec![2; 33]);
    }

    #[test]
    fn nested_envelopes_are_framed_again_and_a_big_endian_ascii_binder_round_trips() {
        let mut b = binder(vec![file(1, "one.txt", &[7; 200]), file(2, "two.txt", &[8; 10])]);
        b.files[0].wrap = Some(Format::DEFLATE_V1);
        let bytes = b.write(&Encoder { oodle: None }).unwrap();
        let read = Binder::read(&bytes, &decoder()).unwrap();
        assert_eq!(read.files[0].bytes, vec![7; 200]);
        assert_eq!(read.files[0].wrap, Some(Format::DEFLATE_V1));
        assert_eq!(read.files[1].wrap, None);
        // the stored read keeps the envelope, and writes the file again as it is
        let stored = Binder::read_stored(&bytes).unwrap();
        assert!(stored.files[0].bytes.starts_with(b"DCX\0"));
        assert_eq!(stored.write_stored().unwrap(), bytes);
        // ASCII names, big-endian words, bit-big-endian flags, no hash table
        let mut odd = binder(vec![file(5, "five", &[5; 20]), file(6, "six", &[6; 20])]);
        odd.layout = Layout {
            big_endian: true,
            bit_big_endian: true,
            unicode: false,
            format_raw: 0x2e,
            extended: 0,
            unknown_04: true,
            unknown_05: false,
        };
        let bytes = odd.write_stored().unwrap();
        let again = Binder::read_stored(&bytes).unwrap();
        assert_eq!(again.layout, odd.layout);
        assert_eq!(again.write_stored().unwrap(), bytes);
        assert_eq!(again.files[1].name.as_deref(), Some("six"));
    }

    #[test]
    fn a_split_binder_is_an_index_and_a_payload_that_end_where_their_content_does() {
        let b = binder(vec![file(0, "a", &[1; 21]), file(1, "b", &[2; 5])]);
        let (index, payload) = b.write_split_stored().unwrap();
        assert_eq!(&index[..4], b"BHF4");
        assert_eq!(&payload[..4], b"BDF4");
        assert_eq!(&payload[24..32], &b.version);
        // the members: the first at 48, the second on the next multiple of 16, nothing after the last
        assert_eq!(payload.len(), 80 + 5);
        let again = Binder::read_split_stored(&index, &payload).unwrap();
        assert_eq!(
            again.write_split_stored().unwrap(),
            (index.clone(), payload.clone())
        );
        // an archive pads its entries with zeros (up to 15 for a 16-byte grain): the readers take them
        let (mut padded_index, mut padded_payload) = (index.clone(), payload.clone());
        padded_index.resize(index.len() + 8, 0);
        padded_payload.resize(payload.len() + 11, 0);
        let again = Binder::read_split_stored(&padded_index, &padded_payload).unwrap();
        assert_eq!(again.write_split_stored().unwrap(), (index, payload));
        let decoded = Binder::read_split(&padded_index, &padded_payload, &decoder()).unwrap();
        assert_eq!(decoded.files[0].bytes, vec![1; 21]);
    }

    #[test]
    fn a_binder_the_format_cannot_hold_is_refused() {
        let mut b = binder(vec![file(1, "a", &[1])]);
        b.files[0].name = None;
        assert!(b.write_stored().is_err(), "a named format needs the name");
        let mut b = binder(vec![file(1, "a", &[1])]);
        b.files[0].id = None;
        assert!(b.write_stored().is_err(), "an ID format needs the ID");
        let mut b = binder(vec![file(1, "a", &[1])]);
        b.layout.format_raw = 0x2e;
        assert!(
            b.write_stored().is_err(),
            "the stored byte must be the normalised format in the stored bit order"
        );
        let mut b = binder(vec![file(1, "caf\u{e9}", &[1])]);
        b.layout.unicode = false;
        assert!(b.write_stored().is_err(), "ASCII names only");
        let mut b = binder(vec![file(1, "a", &[1])]);
        b.files[0].flags = 4;
        assert!(b.write_stored().is_err());
        b.files[0].flags = 3;
        assert!(
            b.write_stored().is_err(),
            "a member flagged compressed has no stored size to write"
        );
    }
}
