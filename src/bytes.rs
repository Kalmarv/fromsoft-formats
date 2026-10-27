use primase_foundations::bytes::{BoundsError, Endian};

use crate::{Result, invalid};
pub(crate) struct View<'a> {
    pub data: &'a [u8],
    pub format: &'static str,
    pub be: bool,
    /// The file offset of `data[0]`: 0 for a view of the whole file, the record's offset for a view of one record, so
    /// that every error offset is a position in the file the caller passed.
    pub base: usize,
}
impl<'a> View<'a> {
    /// [`crate::Error::Invalid`] at `o` (an offset into `data`) of this view's format, as a file offset.
    pub fn err(&self, o: usize, detail: impl ToString) -> crate::Error {
        invalid(self.format, self.base.saturating_add(o), detail)
    }
    /// A view of `len` bytes at `o`, of `format`, whose errors are file offsets.
    pub fn sub(&self, o: usize, len: usize, format: &'static str) -> Result<View<'a>> {
        Ok(View {
            data: self.slice(o, len)?,
            format,
            be: self.be,
            base: self.base.saturating_add(o),
        })
    }
    /// Its byte order: big-endian where the file says so (`be`), else little.
    fn endian(&self) -> Endian {
        if self.be { Endian::Big } else { Endian::Little }
    }
    /// A read at `o` that runs past the data, as this format's error.
    fn short(&self, o: usize, width: usize, e: BoundsError) -> crate::Error {
        match e {
            BoundsError::LengthOverflow { .. } => self.err(o, "size overflow"),
            BoundsError::OutOfBounds { .. } => self.err(
                o,
                format!(
                    "short input: {width} bytes expected, {} left in the data",
                    self.data.len().saturating_sub(o)
                ),
            ),
        }
    }
    /// The offset `delta` bytes after `base`, or an error at `base` when the sum does not fit an offset.
    pub fn at(&self, base: usize, delta: usize) -> Result<usize> {
        base.checked_add(delta)
            .ok_or_else(|| self.err(base, "offset overflow"))
    }
    /// The offset of element `i` of a table of `size`-byte elements at `base`, or an error at `base` when it does not
    /// fit an offset.
    pub fn element(&self, base: usize, i: usize, size: usize) -> Result<usize> {
        i.checked_mul(size)
            .and_then(|delta| base.checked_add(delta))
            .ok_or_else(|| self.err(base, "offset overflow"))
    }
    /// [`View::u16`] at `delta` bytes after `base`.
    pub fn u16_at(&self, base: usize, delta: usize) -> Result<u16> {
        self.u16(self.at(base, delta)?)
    }
    /// [`View::boolean`] at `delta` bytes after `base`.
    pub fn boolean_at(&self, base: usize, delta: usize) -> Result<bool> {
        self.boolean(self.at(base, delta)?)
    }
    /// [`View::expect`] at `delta` bytes after `base`.
    pub fn expect_at(&self, base: usize, delta: usize, b: &[u8]) -> Result<()> {
        self.expect(self.at(base, delta)?, b)
    }
    /// [`View::u8`] at `delta` bytes after `base`.
    pub fn u8_at(&self, base: usize, delta: usize) -> Result<u8> {
        self.u8(self.at(base, delta)?)
    }
    /// [`View::u32`] at `delta` bytes after `base`.
    pub fn u32_at(&self, base: usize, delta: usize) -> Result<u32> {
        self.u32(self.at(base, delta)?)
    }
    pub fn slice(&self, o: usize, n: usize) -> Result<&'a [u8]> {
        primase_foundations::bytes::View::new(self.data)
            .slice(o, n)
            .map_err(|e| self.short(o, n, e))
    }
    pub fn u8(&self, o: usize) -> Result<u8> {
        primase_foundations::bytes::View::new(self.data)
            .u8(o)
            .map_err(|e| self.short(o, 1, e))
    }
    pub fn u16(&self, o: usize) -> Result<u16> {
        primase_foundations::bytes::View::new(self.data)
            .u16(o, self.endian())
            .map_err(|e| self.short(o, 2, e))
    }
    pub fn u32(&self, o: usize) -> Result<u32> {
        primase_foundations::bytes::View::new(self.data)
            .u32(o, self.endian())
            .map_err(|e| self.short(o, 4, e))
    }
    pub fn u64(&self, o: usize) -> Result<u64> {
        primase_foundations::bytes::View::new(self.data)
            .u64(o, self.endian())
            .map_err(|e| self.short(o, 8, e))
    }
    pub fn size(&self, o: usize) -> Result<usize> {
        usize::try_from(self.u64(o)?).map_err(|_| self.err(o, "size overflow"))
    }
    pub fn expect(&self, o: usize, b: &[u8]) -> Result<()> {
        if self.slice(o, b.len())? != b {
            Err(self.err(o, "unexpected field"))
        } else {
            Ok(())
        }
    }
    pub fn boolean(&self, o: usize) -> Result<bool> {
        match self.u8(o)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.err(o, "invalid boolean")),
        }
    }
}
/// How many times over a reader may copy or scan the bytes of its input, in what the records name: a file whose records
/// name one range, or one string, over and over holds `n` copies of it for 32 bytes a record. Legitimate files name each
/// byte about once; four times is the allowance for what the format shares.
pub(crate) const SPEND_TIMES: usize = 4;

/// What a reader may still copy out of its input (see [`SPEND_TIMES`]), charged before the copy is made.
pub(crate) struct Budget {
    format: &'static str,
    left: std::cell::Cell<usize>,
}
impl Budget {
    /// An allowance of [`SPEND_TIMES`] times `input` bytes.
    pub fn new(format: &'static str, input: usize) -> Self {
        Self::times(format, input, SPEND_TIMES)
    }
    /// An allowance of `times` times `input` bytes: for what a reader decodes, which a compressed file holds in fewer
    /// bytes than it makes.
    pub fn times(format: &'static str, input: usize, times: usize) -> Self {
        Self {
            format,
            left: std::cell::Cell::new(input.saturating_mul(times)),
        }
    }
    /// `n` bytes taken from the input at `offset`, or an error when the records have named more than the allowance.
    pub fn charge(&self, offset: usize, n: usize) -> Result<()> {
        match self.left.get().checked_sub(n) {
            Some(left) => {
                self.left.set(left);
                Ok(())
            }
            None => Err(invalid(
                self.format,
                offset,
                "records name more bytes than the file holds, several times over",
            )),
        }
    }
}
impl View<'_> {
    /// The NUL-terminated UTF-16 string at `o` and the offset after its terminator, charged to `budget` once its end is
    /// found and before it is built.
    pub fn utf16z(&self, o: usize, budget: &Budget) -> Result<(String, usize)> {
        let mut end = o;
        while self.u16(end)? != 0 {
            end = end.checked_add(2).ok_or_else(|| self.err(o, "size overflow"))?;
        }
        let after = self.at(end, 2)?;
        budget.charge(self.base.saturating_add(o), after.saturating_sub(o))?;
        let units = (o..end)
            .step_by(2)
            .map(|at| self.u16(at))
            .collect::<Result<Vec<u16>>>()?;
        let text = String::from_utf16(&units).map_err(|e| self.err(o, e))?;
        Ok((text, after))
    }
    /// The NUL-terminated ASCII string at `o` and the offset after its terminator, charged like [`View::utf16z`]; a
    /// byte over 0x7f is an error (Shift-JIS names are not read).
    pub fn asciiz(&self, o: usize, budget: &Budget) -> Result<(String, usize)> {
        let mut end = o;
        while self.u8(end)? != 0 {
            end = end.checked_add(1).ok_or_else(|| self.err(o, "size overflow"))?;
        }
        let after = self.at(end, 1)?;
        budget.charge(self.base.saturating_add(o), after.saturating_sub(o))?;
        let raw = self.slice(o, end.saturating_sub(o))?;
        if !raw.is_ascii() {
            return Err(self.err(o, "non-ASCII Shift-JIS name unsupported"));
        }
        let text = String::from_utf8(raw.to_vec()).map_err(|_| self.err(o, "name encoding"))?;
        Ok((text, after))
    }
}
/// Check that `spans` and zero bytes account for the whole of `v` up to `end`: zero padding of at most `padding`
/// bytes may separate the spans and follow the last one.
pub(crate) fn coverage(v: &View<'_>, spans: Vec<(usize, usize)>, end: usize, padding: usize) -> Result<()> {
    coverage_split(v, spans, end, padding, padding)
}

/// [`coverage`] with separate allowances: at most `gap` zero bytes between spans (and before the first), at most `tail`
/// after the last.
pub(crate) fn coverage_split(
    v: &View<'_>,
    mut spans: Vec<(usize, usize)>,
    end: usize,
    gap: usize,
    tail: usize,
) -> Result<()> {
    spans.sort_unstable();
    spans.dedup();
    let mut pos = 0;
    for (start, len) in spans {
        v.slice(start, len)?;
        if start < pos {
            return Err(v.err(start, "overlapping sections"));
        }
        if start.saturating_sub(pos) > gap || v.slice(pos, start.saturating_sub(pos))?.iter().any(|&b| b != 0)
        {
            return Err(v.err(pos, "unaccounted bytes"));
        }
        pos = start
            .checked_add(len)
            .ok_or_else(|| v.err(start, "size overflow"))?;
    }
    if pos > end
        || end.saturating_sub(pos) > tail
        || v.slice(pos, end.saturating_sub(pos))?.iter().any(|&b| b != 0)
    {
        return Err(v.err(pos, "unaccounted trailing bytes"));
    }
    Ok(())
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    #[test]
    fn wrapper_retains_bounds_and_error_context() {
        let view = View {
            data: &[1, 2, 3, 4],
            format: "bounds fixture",
            be: false,
            base: 0,
        };
        assert!(view.slice(4, 0).unwrap().is_empty());
        assert_eq!(view.u32(0).unwrap(), 0x04030201);
        for (offset, length, detail) in [
            (4, 1, "short input: 1 bytes expected, 0 left in the data"),
            (5, 0, "short input: 0 bytes expected, 0 left in the data"),
            (usize::MAX, 1, "size overflow"),
        ] {
            assert_eq!(
                view.slice(offset, length).unwrap_err().to_string(),
                invalid(view.format, offset, detail).to_string()
            );
        }
    }
}

#[cfg(test)]
mod base_tests {
    use super::*;

    #[test]
    fn a_sub_view_reports_file_offsets() {
        let data = [0u8; 64];
        let file = View {
            data: &data,
            format: "file",
            be: false,
            base: 0,
        };
        let record = file.sub(40, 8, "record").unwrap();
        assert_eq!(record.err(3, "x").to_string(), "record at 0x2b: x");
        assert_eq!(
            record.u32(6).unwrap_err().to_string(),
            "record at 0x2e: short input: 4 bytes expected, 2 left in the data"
        );
        assert_eq!(
            record.expect(2, &[1]).unwrap_err().to_string(),
            "record at 0x2a: unexpected field"
        );
        let inner = record.sub(4, 4, "inner").unwrap();
        assert!(inner.boolean(0).is_ok());
        assert_eq!(inner.err(1, "y").to_string(), "inner at 0x2d: y");
    }
}
