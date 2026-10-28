//! The output side the writers share: a byte buffer that appends integers and strings in either byte order and pads
//! with zeros, and the checked arithmetic and narrowing of the sizes and offsets a file stores.
use crate::{Result, invalid};

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

    /// The bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Appends one byte.
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    /// Appends a 16-bit word.
    pub fn u16(&mut self, v: u16) {
        self.buf.extend(if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        });
    }

    /// Appends a 32-bit word.
    pub fn u32(&mut self, v: u32) {
        self.buf.extend(if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        });
    }

    /// Appends a signed 32-bit word.
    pub fn i32(&mut self, v: i32) {
        self.buf.extend(if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        });
    }

    /// Appends a 64-bit word.
    pub fn u64(&mut self, v: u64) {
        self.buf.extend(if self.big {
            v.to_be_bytes()
        } else {
            v.to_le_bytes()
        });
    }

    /// Appends `b`.
    pub fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }

    /// Appends `n` zero bytes.
    pub fn zeros(&mut self, n: usize) {
        self.buf.resize(self.buf.len().saturating_add(n), 0);
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
            return Err(invalid(
                self.format,
                self.buf.len(),
                format!("{text:?} is not ASCII, which this format's names are"),
            ));
        }
        self.buf.extend_from_slice(text.as_bytes());
        self.buf.push(0);
        Ok(())
    }

    /// The finished bytes.
    pub fn finish(self) -> Vec<u8> {
        self.buf
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
}
