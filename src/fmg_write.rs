//! Writing FMG text files: the layout of the games' own tools, found by rebuilding the 4,472 FMGs of both installs
//! (504 of Sekiro's, 3,968 of Elden Ring's) from their strings, blank ids and trailing padding.
//!
//! The ids (those with a string and the blank ones) in ascending order are cut into groups of consecutive ids; each
//! group has the index of its first string slot, its first and its last id; the strings follow the slot table in id
//! order, each as UTF-16 and a NUL, with no sharing and no gaps; the file is padded with zeros to a multiple of 4 bytes
//! and then by [`Fmg::extra_padding`]. The layout of [`crate::fmg`] describes the header.
use crate::fmg::Fmg;
use crate::put::{Out, add, mul, narrow};
use crate::{Result, invalid};

const FORMAT: &str = "FromSoftware FMG";

impl Fmg {
    /// This text file as FMG bytes (little-endian, Unicode): version 2 with 64-bit offsets, or version 1 for a
    /// [`Fmg::narrow`] one. A file read with [`Fmg::read`] is written back to the same bytes, apart from the 3 of
    /// Elden Ring's 3,968 that hold unreferenced zero bytes between their strings (they are written without them).
    /// An id may not be both in [`Fmg::entries`] and in [`Fmg::blank`].
    pub fn write(&self) -> Result<Vec<u8>> {
        if let Some(id) = self.blank.iter().find(|id| self.entries.contains_key(*id)) {
            return Err(invalid(
                FORMAT,
                0,
                format!("id {id} is both blank and given a string"),
            ));
        }
        // every id of the file in ascending order, with its string or none
        let mut ids: Vec<(i32, Option<&str>)> = self
            .entries
            .iter()
            .map(|(id, text)| (*id, Some(text.as_str())))
            .chain(self.blank.iter().map(|id| (*id, None)))
            .collect();
        ids.sort_unstable_by_key(|(id, _)| *id);

        // groups of consecutive ids: (index of the first slot, first id, last id)
        let mut groups: Vec<(usize, i32, i32)> = Vec::new();
        for (slot, (id, _)) in ids.iter().enumerate() {
            let continues = groups
                .last()
                .is_some_and(|(_, _, last)| last.checked_add(1) == Some(*id));
            if continues {
                if let Some(group) = groups.last_mut() {
                    group.2 = *id;
                }
            } else {
                groups.push((slot, *id, *id));
            }
        }

        let wide = !self.narrow;
        let (group_size, first_group, slot_size) = if wide { (16, 0x28, 8) } else { (12, 0x1c, 4) };
        let offsets_at = add(FORMAT, first_group, mul(FORMAT, groups.len(), group_size)?)?;
        let strings_at = add(FORMAT, offsets_at, mul(FORMAT, ids.len(), slot_size)?)?;

        // the string area, and where each string starts in the file
        let mut text = Out::new(FORMAT, false);
        let mut slots = Vec::with_capacity(ids.len());
        for (_, string) in &ids {
            match string {
                None => slots.push(0usize),
                Some(s) => {
                    slots.push(add(FORMAT, strings_at, text.len())?);
                    text.utf16z(s);
                }
            }
        }
        let text = text.finish();
        let strings_end = add(FORMAT, strings_at, text.len())?;
        let aligned = add(FORMAT, strings_end, strings_end.wrapping_neg() & 3)?;
        let total = add(FORMAT, aligned, self.extra_padding)?;

        let mut out = Out::new(FORMAT, false);
        out.u8(0);
        out.u8(0);
        out.u8(if wide { 2 } else { 1 });
        out.u8(0);
        out.u32(narrow(FORMAT, "the file size", total)?);
        out.u8(1);
        out.zeros(3);
        out.u32(narrow(FORMAT, "the group count", groups.len())?);
        out.u32(narrow(FORMAT, "the string count", ids.len())?);
        if wide {
            out.u32(0xff);
            out.u64(narrow(FORMAT, "the offset table's offset", offsets_at)?);
            out.u64(0);
        } else {
            out.u32(narrow(FORMAT, "the offset table's offset", offsets_at)?);
            out.u32(0);
        }
        for (index, first, last) in &groups {
            out.u32(narrow(FORMAT, "a group's first slot", *index)?);
            out.i32(*first);
            out.i32(*last);
            if wide {
                out.u32(0);
            }
        }
        for slot in &slots {
            if wide {
                out.u64(narrow(FORMAT, "a string's offset", *slot)?);
            } else {
                out.u32(narrow(FORMAT, "a string's offset", *slot)?);
            }
        }
        out.bytes(&text);
        out.zeros(total.saturating_sub(out.len()));
        Ok(out.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn fmg(entries: &[(i32, &str)], blank: &[i32], narrow: bool, extra_padding: usize) -> Fmg {
        Fmg {
            entries: entries
                .iter()
                .map(|(id, s)| (*id, (*s).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            blank: blank.iter().copied().collect::<BTreeSet<_>>(),
            narrow,
            extra_padding,
        }
    }

    #[test]
    fn a_one_string_file_is_the_bytes_the_layout_gives() {
        let mut expected = vec![0, 0, 2, 0];
        expected.extend(0x48u32.to_le_bytes()); // the file's size, padded to 4
        expected.extend([1, 0, 0, 0]);
        expected.extend(1u32.to_le_bytes()); // groups
        expected.extend(1u32.to_le_bytes()); // strings
        expected.extend(0xffu32.to_le_bytes());
        expected.extend(0x38u64.to_le_bytes()); // the offset table follows the one group
        expected.extend(0u64.to_le_bytes());
        for word in [0u32, 10, 10, 0] {
            expected.extend(word.to_le_bytes()); // index 0, ids 10 to 10
        }
        expected.extend(0x40u64.to_le_bytes()); // the string follows the one slot
        expected.extend([0x61, 0, 0x62, 0, 0, 0]); // "ab"
        expected.extend([0, 0]); // padding to 4
        assert_eq!(fmg(&[(10, "ab")], &[], false, 0).write().unwrap(), expected);
        assert_eq!(Fmg::read(&expected).unwrap(), fmg(&[(10, "ab")], &[], false, 0));
    }

    #[test]
    fn groups_follow_consecutive_ids_and_blank_ids_hold_a_group_together() {
        let with_blank = fmg(&[(1, "a"), (3, "b")], &[2], false, 0);
        let without = fmg(&[(1, "a"), (3, "b")], &[], false, 0);
        let group_count = |bytes: &[u8]| u32::from_le_bytes(bytes[0x0c..0x10].try_into().unwrap());
        assert_eq!(group_count(&with_blank.write().unwrap()), 1);
        assert_eq!(group_count(&without.write().unwrap()), 2);
        for f in [with_blank, without] {
            let bytes = f.write().unwrap();
            assert_eq!(Fmg::read(&bytes).unwrap(), f);
            assert_eq!(Fmg::read(&bytes).unwrap().write().unwrap(), bytes);
        }
    }

    #[test]
    fn both_layouts_negative_ids_and_trailing_padding_round_trip() {
        for narrow in [false, true] {
            for extra in [0, 4, 16] {
                let f = fmg(
                    &[(-5, "\u{30a2}\u{30a4}"), (-4, ""), (7, "x"), (i32::MAX, "end")],
                    &[8],
                    narrow,
                    extra,
                );
                let bytes = f.write().unwrap();
                assert_eq!(bytes.len() % 4, 0);
                assert_eq!(Fmg::read(&bytes).unwrap(), f, "narrow {narrow}, extra {extra}");
                assert_eq!(Fmg::read(&bytes).unwrap().write().unwrap(), bytes);
            }
        }
        // no ids at all is a header
        let empty = Fmg::default();
        assert_eq!(Fmg::read(&empty.write().unwrap()).unwrap(), empty);
    }

    #[test]
    fn an_id_that_is_both_blank_and_given_a_string_is_refused() {
        assert!(fmg(&[(1, "a")], &[1], false, 0).write().is_err());
    }
}
