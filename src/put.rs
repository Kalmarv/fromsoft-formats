//! The output side the writers share ([`Out`]): a byte buffer that appends integers, floats and strings in either byte
//! order, pads and aligns with zeros and patches placeholders once their value is known, and the checked arithmetic and
//! narrowing of the sizes and offsets a file stores. Every refusal names the format and the offset the writer had
//! reached.

//! The byte sink the writers of MTD, MATBIN, EMEVD, TAE, MSB and FLVER share ([`Out`]): little-endian puts,
//! zero alignment, placeholders patched once their value is known, and counts and offsets that must fit the 32-bit
//! words a file stores them in.
use crate::{Error, Result, invalid};

/// A growing output buffer of one format, little- or big-endian.
#[derive(Debug)]
pub(crate) struct Out {
    buf: Vec<u8>,
    big: bool,
    format: &'static str,
}

impl Out {
    /// An empty buffer for `format` (named in the errors), big-endian where `big`.
    pub fn new(format: &'static str, big: bool) -> Self {
        Self {
            buf: Vec::new(),
            big,
            format,
        }
    }

    /// The number of bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// The bytes written so far.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    /// The finished bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    /// A refusal at the current offset.
    pub fn err(&self, detail: impl ToString) -> Error {
        invalid(self.format, self.buf.len(), detail)
    }

    /// A refusal at offset `at`.
    pub fn err_at(&self, at: usize, detail: impl ToString) -> Error {
        invalid(self.format, at, detail)
    }

    /// `n` as the 32-bit word a file stores it in, or a refusal that names `what`.
    pub fn word(&self, n: usize, what: &str) -> Result<u32> {
        u32::try_from(n).map_err(|_| self.err(format!("{what} {n} does not fit a 32-bit word")))
    }

    /// `n` as a signed 32-bit word, or a refusal that names `what`.
    pub fn signed(&self, n: usize, what: &str) -> Result<i32> {
        i32::try_from(n).map_err(|_| self.err(format!("{what} {n} does not fit a signed 32-bit word")))
    }

    /// `n` as an unsigned 64-bit word, or a refusal that names `what`.
    pub fn wide(&self, n: usize, what: &str) -> Result<u64> {
        u64::try_from(n).map_err(|_| self.err(format!("{what} {n} does not fit a 64-bit word")))
    }

    /// Appends one byte.
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// Appends a 16-bit word.
    pub fn u16(&mut self, v: u16) {
        let b = if self.big { v.to_be_bytes() } else { v.to_le_bytes() };
        self.buf.extend_from_slice(&b);
    }

    /// Appends a signed 16-bit word.
    pub fn i16(&mut self, v: i16) {
        self.u16(u16::from_ne_bytes(v.to_ne_bytes()));
    }

    /// Appends a 32-bit word.
    pub fn u32(&mut self, v: u32) {
        let b = self.word_bytes(v);
        self.buf.extend_from_slice(&b);
    }

    /// Appends a signed 32-bit word.
    pub fn i32(&mut self, v: i32) {
        self.u32(u32::from_ne_bytes(v.to_ne_bytes()));
    }

    /// Appends a 64-bit word.
    pub fn u64(&mut self, v: u64) {
        let b = self.wide_bytes(v);
        self.buf.extend_from_slice(&b);
    }

    /// Appends a signed 64-bit word.
    pub fn i64(&mut self, v: i64) {
        self.u64(u64::from_ne_bytes(v.to_ne_bytes()));
    }

    /// Appends a 32-bit float, its bits as stored.
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    /// Appends `b`.
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    /// Appends `n` zero bytes.
    pub fn zeros(&mut self, n: usize) {
        self.buf.resize(self.buf.len().saturating_add(n), 0);
    }

    /// Zero bytes up to the next multiple of `align` (nothing when already on one; `align` 0 or 1 does nothing).
    pub fn align(&mut self, align: usize) {
        if align > 1 {
            let target = self.buf.len().next_multiple_of(align);
            self.buf.resize(target, 0);
        }
    }

    /// Appends `text` as UTF-16 (in the buffer's byte order) and a 16-bit NUL.
    pub fn utf16z(&mut self, text: &str) {
        for unit in text.encode_utf16() {
            self.u16(unit);
        }
        self.u16(0);
    }

    /// Appends `text` as ASCII and a NUL; a character over 0x7f is an error at the current offset.
    pub fn asciiz(&mut self, text: &str) -> Result<()> {
        if !text.is_ascii() {
            return Err(self.err(format!("{text:?} is not ASCII, which this format's names are")));
        }
        self.buf.extend_from_slice(text.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    /// Overwrites the four bytes at `at` (a placeholder written earlier) in the buffer's byte order.
    pub fn patch_u32(&mut self, at: usize, v: u32) -> Result<()> {
        let b = self.word_bytes(v);
        self.patch(at, &b)
    }

    /// Overwrites the eight bytes at `at` (a placeholder written earlier) in the buffer's byte order.
    pub fn patch_u64(&mut self, at: usize, v: u64) -> Result<()> {
        let b = self.wide_bytes(v);
        self.patch(at, &b)
    }

    /// Overwrites the bytes at `at..at + v.len()`, which must already be written.
    pub fn patch(&mut self, at: usize, v: &[u8]) -> Result<()> {
        let slot = at
            .checked_add(v.len())
            .and_then(|end| self.buf.get_mut(at..end));
        match slot {
            Some(slot) => {
                slot.copy_from_slice(v);
                Ok(())
            }
            None => Err(self.err_at(at, "patch outside the bytes written")),
        }
    }

    fn word_bytes(&self, v: u32) -> [u8; 4] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }

    fn wide_bytes(&self, v: u64) -> [u8; 8] {
        if self.big { v.to_be_bytes() } else { v.to_le_bytes() }
    }
}

/// `v` rounded up to a multiple of `to`, or an error naming `format` where the result does not fit.
pub(crate) fn align_up(format: &'static str, v: usize, to: usize) -> Result<usize> {
    let rem = v.checked_rem(to).unwrap_or(0);
    if rem == 0 {
        return Ok(v);
    }
    v.checked_add(to.saturating_sub(rem)).ok_or_else(|| {
        invalid(
            format,
            0,
            format!("an offset of {v} rounded up to {to} does not fit"),
        )
    })
}

/// `v` in the integer type a file field stores it in, or an error saying which field `what` it does not fit.
pub(crate) fn narrow<T: TryFrom<usize>>(format: &'static str, what: &str, v: usize) -> Result<T> {
    T::try_from(v).map_err(|_| invalid(format, 0, format!("{what} of {v} does not fit its field")))
}

/// `a + b`, or an error naming `format` where a file offset or size would overflow.
pub(crate) fn add(format: &'static str, a: usize, b: usize) -> Result<usize> {
    a.checked_add(b)
        .ok_or_else(|| invalid(format, 0, format!("an offset of {a} plus {b} overflows")))
}

/// `a * b`, or an error naming `format` where a table size would overflow.
pub(crate) fn mul(format: &'static str, a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| invalid(format, 0, format!("a size of {a} times {b} overflows")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_padding_follow_the_buffers_byte_order() {
        for big in [false, true] {
            let mut out = Out::new("TEST", big);
            out.u8(1);
            out.u16(0x0203);
            out.zeros(1);
            out.u32(0x0405_0607);
            out.u64(0x0809_0a0b_0c0d_0e0f);
            out.i32(-2);
            out.utf16z("a");
            out.asciiz("b").unwrap();
            out.zeros(6);
            let b = out.finish();
            let word = |at: usize| {
                let w: [u8; 4] = b[at..at + 4].try_into().unwrap();
                if big {
                    u32::from_be_bytes(w)
                } else {
                    u32::from_le_bytes(w)
                }
            };
            assert_eq!(b[0], 1);
            assert_eq!(b[3], 0, "alignment pads with zeros");
            assert_eq!(word(4), 0x0405_0607);
            assert_eq!(word(16), (-2i32) as u32);
            assert_eq!(b.len() % 8, 0);
            let wide: [u8; 8] = b[8..16].try_into().unwrap();
            assert_eq!(
                if big {
                    u64::from_be_bytes(wide)
                } else {
                    u64::from_le_bytes(wide)
                },
                0x0809_0a0b_0c0d_0e0f
            );
        }
    }

    #[test]
    fn a_non_ascii_name_and_an_overflowing_size_are_refused() {
        let mut out = Out::new("TEST", false);
        out.u32(0);
        assert!(out.asciiz("caf\u{e9}").is_err());
        assert_eq!(align_up("TEST", 17, 16).unwrap(), 32);
        assert_eq!(align_up("TEST", 32, 16).unwrap(), 32);
        assert!(align_up("TEST", usize::MAX, 16).is_err());
        assert!(narrow::<u32>("TEST", "a size", usize::MAX).is_err());
        assert_eq!(narrow::<u8>("TEST", "a count", 255).unwrap(), 255);
    }

    #[test]
    fn puts_are_little_endian_and_patches_must_land_inside() {
        let mut o = Out::new("TEST", false);
        o.u32(0);
        o.u16(0x0201);
        o.align(4);
        o.u64(1);
        o.patch_u32(0, 0xdead_beef).unwrap();
        assert_eq!(
            o.as_slice(),
            [0xef, 0xbe, 0xad, 0xde, 1, 2, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
        );
        assert!(o.patch_u32(13, 1).is_err());
        assert!(o.patch_u64(usize::MAX, 1).is_err());
        assert!(o.word(1usize << 33, "x").is_err());
    }

    #[test]
    fn patches_and_floats_follow_the_buffers_byte_order() {
        let mut o = Out::new("TEST", true);
        o.u32(0);
        o.f32(1.0);
        o.i64(-1);
        o.patch_u32(0, 0x0102_0304).unwrap();
        assert_eq!(&o.as_slice()[..8], [1, 2, 3, 4, 0x3f, 0x80, 0, 0]);
        assert_eq!(&o.as_slice()[8..], [0xff; 8]);
    }
}
