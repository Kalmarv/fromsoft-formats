//! Writing PARAM files: the layout of the games' own tools, found by rebuilding with an independent model the 235 params
//! of both installs' archives (Sekiro's 190, Elden Ring's 45) and the 5,489 params of the 29 `regulation.bin` files.
//! [`Param::read`] accepts all of them but 24 graphics-quality params of the archives, whose trailing zeros run past
//! its limit of 8 bytes.
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
use crate::emit::{Out, add, mul, narrow};
use crate::param::{Param, Stored};
use crate::{Result, invalid};

const FORMAT: &str = "PARAM";

impl Param {
    /// This param as a PARAM file with the header word and trailing zeros `stored` says ([`Stored::derived`] for a param
    /// that was not read from a file). A file read with [`Param::read_stored`] is written back to the same bytes.
    pub fn write(&self, stored: &Stored) -> Result<Vec<u8>> {
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
        let data_start = add(FORMAT, 0x40, mul(FORMAT, self.rows.len(), 24)?)?;
        let type_offset = add(FORMAT, data_start, mul(FORMAT, self.rows.len(), self.row_size)?)?;
        let type_end = add(FORMAT, type_offset, add(FORMAT, self.param_type.len(), 1)?)?;

        // the names: the empty one once and shared, every other one for its row alone
        let mut names = Out::new(FORMAT, false);
        let mut name_offsets = Vec::with_capacity(self.rows.len());
        let mut empty_at: Option<usize> = None;
        for row in &self.rows {
            match row.name.as_deref() {
                None => name_offsets.push(0usize),
                Some("") => {
                    let at = match empty_at {
                        Some(at) => at,
                        None => {
                            let at = add(FORMAT, type_end, names.len())?;
                            names.u16(0);
                            empty_at = Some(at);
                            at
                        }
                    };
                    name_offsets.push(at);
                }
                Some(name) => {
                    name_offsets.push(add(FORMAT, type_end, names.len())?);
                    names.utf16z(name);
                }
            }
        }

        let mut out = Out::new(FORMAT, false);
        out.u32(stored.strings_offset);
        out.u16(0);
        out.u16(self.unk06);
        out.u16(self.data_version);
        out.u16(count);
        out.u32(0);
        out.u64(narrow(FORMAT, "the param type's offset", type_offset)?);
        out.zeros(0x14);
        out.u8(0);
        out.u8(0x85);
        out.u8(0x07);
        out.u8(0);
        out.u64(narrow(FORMAT, "the row data's offset", data_start)?);
        out.u64(0);
        for (i, (row, name_offset)) in self.rows.iter().zip(&name_offsets).enumerate() {
            out.u32(row.id as u32);
            out.u32(0);
            let data_at = add(FORMAT, data_start, mul(FORMAT, i, self.row_size)?)?;
            out.u64(narrow(FORMAT, "a row's offset", data_at)?);
            out.u64(narrow(FORMAT, "a name's offset", *name_offset)?);
        }
        for row in &self.rows {
            out.bytes(&row.data);
        }
        out.bytes(self.param_type.as_bytes());
        out.u8(0);
        out.bytes(&names.finish());
        out.zeros(stored.padding);
        Ok(out.finish())
    }

    /// [`Param::write`] with [`Stored::derived`]: for a param built here rather than read from a file.
    pub fn write_derived(&self) -> Result<Vec<u8>> {
        self.write(&Stored::derived(self))
    }
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
                },
            ),
            // the one shared empty name
            (
                vec![row(1, Some(""), 1), row(5, Some(""), 2), row(6, None, 3)],
                Stored {
                    strings_offset: 0,
                    padding: 2,
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
}
