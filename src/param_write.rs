//! Writing PARAM files: the layout of the games' own tools, found by rebuilding with an independent model the 235 params
//! of both installs' archives (Sekiro's 190, Elden Ring's 45) and the 5,489 params of the 29 `regulation.bin` files.
//! [`Param::write`] writes a param in the layout its [`Stored::format`] names; Sekiro's and Elden Ring's, the one their
//! tools write:
//!
//! ```text
//! 0x00 u32 strings offset ([`Stored::strings_offset`]), u16 0, u16 unknown 06, u16 data version, u16 row count, u32 0
//! 0x10 u64 offset of the param type string, 20 zeros, u8 0 (little-endian), u8 0x85, u8 0x07, u8 0
//! 0x30 u64 offset of the row data, u64 0
//! 0x40 the row table, 24 bytes a row: u32 id, u32 0, u64 offset of its data, u64 offset of its name (0 for none)
//!      the rows' data one after the other, `row_size` each
//!      the param type, ASCII and a NUL
//!      the row names, UTF-16 and a NUL, in row order, directly after the type: the empty name once, shared by every
//!      row that has it, and every other name once for its own row
//!      zero bytes to the end of the file ([`Stored::padding`])
//! ```
use crate::param::{Format, Param, Stored};
use crate::put::{Out, add, mul, narrow};
use crate::{Result, invalid};

const FORMAT: &str = "PARAM";

impl Param {
    /// This param as a PARAM file with the header word and trailing zeros `stored` says ([`Stored::derived`] for a param
    /// that was not read from a file). A file read with [`Param::read_stored`] is written back to the same bytes.
    pub fn write(&self, stored: &Stored) -> Result<Vec<u8>> {
        let format = stored.format;
        let count: u16 = narrow(FORMAT, "the row count", self.rows.len())?;
        if !self.param_type.is_ascii() {
            return Err(invalid(FORMAT, 0x10, "the param type is not ASCII"));
        }
        if self.rows.iter().any(|r| r.data.len() != self.row_size) {
            return Err(invalid(FORMAT, 0, "a row's data is not row_size bytes"));
        }
        if count > 0 && self.row_size == 0 {
            return Err(invalid(FORMAT, 0, "rows of no bytes"));
        }
        let by_offset = format.flags_2d & 0x80 != 0;
        if !by_offset && self.param_type.len() > 0x20 {
            return Err(invalid(
                FORMAT,
                0x0c,
                "the param type is longer than the header's 32 bytes",
            ));
        }
        let int_offset = format.flags_2d & 0x03 == 0x03;
        let long_offset = !int_offset && format.flags_2d & 0x04 != 0;
        let header = if int_offset || long_offset { 0x40 } else { 0x30 };
        let entry = if format.flags_2d & 0x04 != 0 { 24 } else { 12 };
        let table_padding = if format.flags_2d == 0x01 { 0x20 } else { 0 };
        let data_start = add(
            FORMAT,
            add(FORMAT, header, mul(FORMAT, self.rows.len(), entry)?)?,
            table_padding,
        )?;
        let type_offset = add(FORMAT, data_start, mul(FORMAT, self.rows.len(), self.row_size)?)?;
        let names_start = if by_offset {
            add(FORMAT, type_offset, add(FORMAT, self.param_type.len(), 1)?)?
        } else {
            type_offset
        };

        // the names: the empty one once and shared, every other one for its row alone
        let mut names = Out::new(FORMAT, format.big_endian);
        let mut name_offsets = Vec::with_capacity(self.rows.len());
        let mut empty_at: Option<usize> = None;
        for row in &self.rows {
            match row.name.as_deref() {
                None => name_offsets.push(0usize),
                Some("") => {
                    let at = match empty_at {
                        Some(at) => at,
                        None => {
                            let at = add(FORMAT, names_start, names.len())?;
                            name(&mut names, format, "")?;
                            empty_at = Some(at);
                            at
                        }
                    };
                    name_offsets.push(at);
                }
                Some(text) => {
                    name_offsets.push(add(FORMAT, names_start, names.len())?);
                    name(&mut names, format, text)?;
                }
            }
        }

        let mut out = Out::new(FORMAT, format.big_endian);
        out.u32(stored.strings_offset);
        out.u16(if header == 0x30 {
            narrow(FORMAT, "the row data's offset", data_start)?
        } else {
            0
        });
        out.u16(self.unk06);
        out.u16(self.data_version);
        out.u16(count);
        if by_offset {
            out.u32(0);
            out.u64(narrow(FORMAT, "the param type's offset", type_offset)?);
            out.zeros(0x14);
        } else {
            let mut field = self.param_type.as_bytes().to_vec();
            if field.len() < 0x20 {
                field.push(0);
            }
            field.resize(0x20, stored.type_fill);
            out.bytes(&field);
        }
        out.u8(if format.big_endian { 0xff } else { 0 });
        out.u8(format.flags_2d);
        out.u8(format.flags_2e);
        out.u8(format.paramdef_format_version);
        if int_offset {
            out.u32(narrow(FORMAT, "the row data's offset", data_start)?);
            out.zeros(12);
        } else if long_offset {
            out.u64(narrow(FORMAT, "the row data's offset", data_start)?);
            out.u64(0);
        }
        for (i, (row, name_offset)) in self.rows.iter().zip(&name_offsets).enumerate() {
            let data_at = add(FORMAT, data_start, mul(FORMAT, i, self.row_size)?)?;
            out.u32(row.id as u32);
            if entry == 24 {
                out.u32(0);
                out.u64(narrow(FORMAT, "a row's offset", data_at)?);
                out.u64(narrow(FORMAT, "a name's offset", *name_offset)?);
            } else {
                out.u32(narrow(FORMAT, "a row's offset", data_at)?);
                out.u32(narrow(FORMAT, "a name's offset", *name_offset)?);
            }
        }
        out.zeros(table_padding);
        for row in &self.rows {
            out.bytes(&row.data);
        }
        if by_offset {
            out.bytes(self.param_type.as_bytes());
            out.u8(0);
        }
        out.bytes(&names.finish());
        out.zeros(stored.padding);
        Ok(out.finish())
    }

    /// [`Param::write`] with [`Stored::derived`]: for a param built here rather than read from a file.
    pub fn write_derived(&self) -> Result<Vec<u8>> {
        self.write(&Stored::derived(self))
    }
}

/// A row name as the format stores it: UTF-16 and a NUL with [`Format::flags_2e`] 0x01, else Shift-JIS and a NUL.
fn name(out: &mut Out, format: Format, text: &str) -> Result<()> {
    if format.flags_2e & 0x01 != 0 {
        out.utf16z(text);
    } else {
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(text);
        if unmappable {
            return Err(invalid(
                FORMAT,
                0,
                format!("the row name {text:?} has no Shift-JIS form"),
            ));
        }
        out.bytes(&bytes);
        out.u8(0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::Row;

    fn row(id: i32, name: Option<&str>, byte: u8) -> Row {
        Row {
            id,
            name: name.map(str::to_owned),
            data: vec![byte; 12],
        }
    }

    fn param(rows: Vec<Row>) -> Param {
        Param {
            param_type: "TEST_PARAM_ST".to_owned(),
            data_version: 3,
            unk06: 1,
            row_size: 12,
            rows,
        }
    }

    #[test]
    fn a_param_reads_back_to_itself_and_rewrites_the_same_bytes() {
        let variants = [
            // no names at all, and the 8 zero bytes the tools append then
            (
                vec![row(1, None, 1), row(-2, None, 2)],
                Stored {
                    strings_offset: 0,
                    padding: 8,
                    format: Format::SEKIRO,
                    type_fill: 0,
                },
            ),
            // the one shared empty name
            (
                vec![row(1, Some(""), 1), row(5, Some(""), 2), row(6, None, 3)],
                Stored {
                    strings_offset: 0,
                    padding: 2,
                    format: Format::SEKIRO,
                    type_fill: 0,
                },
            ),
            // real names, each its own, an empty one among them, and padding the tools left
            (
                vec![
                    row(1, Some("Low"), 1),
                    row(2, Some(""), 2),
                    row(3, Some("Low"), 3),
                    row(4, Some("\u{30a2}"), 4),
                ],
                Stored {
                    strings_offset: 0,
                    padding: 4,
                    format: Format::SEKIRO,
                    type_fill: 0,
                },
            ),
        ];
        for (rows, mut stored) in variants {
            let p = param(rows);
            stored.strings_offset = Stored::derived(&p).strings_offset;
            let bytes = p.write(&stored).unwrap();
            let (again, again_stored) = Param::read_stored(&bytes).unwrap();
            assert_eq!(again, p);
            assert_eq!(again_stored, stored);
            assert_eq!(again.write(&again_stored).unwrap(), bytes);
        }
        // no rows at all: the reader reads a row size of 0
        let empty = Param {
            row_size: 0,
            ..param(vec![])
        };
        let bytes = empty.write_derived().unwrap();
        assert_eq!(Param::read(&bytes).unwrap(), empty);
    }

    #[test]
    fn the_strings_offset_is_data_and_the_derived_one_rounds_the_type_offset_up() {
        let p = param(vec![row(1, Some(""), 1)]);
        // header 0x40, one 24-byte entry, 12 bytes of row data: the type string at 0x64, which rounds up to 0x70
        let derived = Stored::derived(&p);
        assert_eq!((derived.strings_offset, derived.padding), (0x70, 2));
        let bytes = p.write(&derived).unwrap();
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 0x70);
        let kept = p
            .write(&Stored {
                strings_offset: 0x64,
                ..derived
            })
            .unwrap();
        assert_eq!(Param::read_stored(&kept).unwrap().1.strings_offset, 0x64);
        assert_eq!(kept.len(), bytes.len());
        // a param with no names gets no padding by default
        assert_eq!(Stored::derived(&param(vec![row(1, None, 1)])).padding, 0);
    }

    #[test]
    fn a_param_the_format_cannot_hold_is_refused() {
        let mut p = param(vec![row(1, None, 1)]);
        p.rows[0].data.pop();
        assert!(p.write_derived().is_err(), "a short row");
        let mut p = param(vec![row(1, None, 1)]);
        p.param_type = "T\u{e9}ST".to_owned();
        assert!(p.write_derived().is_err(), "a type that is not ASCII");
        let mut p = param(vec![]);
        p.rows = (0..70_000).map(|i| row(i, None, 0)).collect();
        assert!(p.write_derived().is_err(), "more rows than the 16-bit count");
    }

    /// Every layout of the format writes, reads back to the same param and facts, and rewrites the same bytes: either
    /// byte order; the data offset in a u16 (with and without the 32 zero bytes of flags 0x01), a 32-bit or a 64-bit
    /// word; the type by offset or in the header (filled with zeros or spaces); UTF-16 or Shift-JIS names.
    #[test]
    fn every_layout_reads_back_to_itself_and_rewrites_the_same_bytes() {
        let rows = || {
            vec![
                row(10, Some("\u{9b54}\u{6cd5}"), 1),
                row(-3, None, 2),
                row(7, Some(""), 3),
                row(8, Some("Low"), 4),
            ]
        };
        for big_endian in [false, true] {
            for (flags_2d, type_fill) in [
                (0x85, 0),
                (0x83, 0),
                (0x03, 0x20),
                (0x03, 0),
                (0x05, 0),
                (0x00, 0),
                (0x01, 0x20),
                (0x80, 0),
            ] {
                for flags_2e in [0x07, 0x00] {
                    let format = Format {
                        big_endian,
                        flags_2d,
                        flags_2e,
                        paramdef_format_version: 3,
                    };
                    let p = param(rows());
                    let stored = Stored {
                        strings_offset: 0x1234,
                        padding: 6,
                        format,
                        type_fill,
                    };
                    let bytes = p.write(&stored).unwrap();
                    let (again, again_stored) =
                        Param::read_stored(&bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
                    assert_eq!(again, p, "{format:?}");
                    assert_eq!(again_stored, stored, "{format:?}");
                    assert_eq!(again.write(&again_stored).unwrap(), bytes, "{format:?}");
                }
            }
        }
    }

    /// The 32-bit form laid out here by hand from SoulsFormats' description: header with the type in its 32 bytes,
    /// data offset word at 0x30, 12-byte row entries, Shift-JIS names.
    #[test]
    fn the_32_bit_form_reads_as_laid_out() {
        let mut b = vec![0u8; 0x40];
        let data_start = 0x40 + 2 * 12;
        let names = data_start + 2 * 4;
        b[0..4].copy_from_slice(&(names as u32).to_le_bytes());
        b[6..8].copy_from_slice(&2u16.to_le_bytes());
        b[8..10].copy_from_slice(&5u16.to_le_bytes());
        b[10..12].copy_from_slice(&2u16.to_le_bytes());
        b[0x0c..0x0c + 9].copy_from_slice(b"AI_BANK_X");
        b[0x0c + 10..0x2c].fill(0x20);
        b[0x2d] = 0x03;
        b[0x30..0x34].copy_from_slice(&(data_start as u32).to_le_bytes());
        b.resize(data_start, 0);
        let sjis = [0x96u8, 0xbc, 0]; // "名"
        for (i, (id, name)) in [(100i32, names), (200, names + sjis.len())]
            .into_iter()
            .enumerate()
        {
            let at = 0x40 + i * 12;
            b[at..at + 4].copy_from_slice(&id.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&((data_start + 4 * i) as u32).to_le_bytes());
            b[at + 8..at + 12].copy_from_slice(&(name as u32).to_le_bytes());
        }
        b.extend([1, 2, 3, 4, 5, 6, 7, 8]);
        b.extend(sjis);
        b.extend(b"ab\0");
        let (p, stored) = Param::read_stored(&b).unwrap();
        assert_eq!(p.param_type, "AI_BANK_X");
        assert_eq!((p.row_size, p.data_version, p.unk06), (4, 5, 2));
        assert_eq!(p.rows[0].name.as_deref(), Some("\u{540d}"));
        assert_eq!(p.rows[1].name.as_deref(), Some("ab"));
        assert_eq!(p.rows[1].data, [5, 6, 7, 8]);
        assert_eq!(
            (stored.format.flags_2d, stored.type_fill, stored.padding),
            (0x03, 0x20, 0)
        );
        assert_eq!(p.write(&stored).unwrap(), b);
    }
}
