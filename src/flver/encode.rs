//! The inverse of [`super::Vertex`]'s decoding: a vertex's attributes written into the bytes of one vertex buffer, member by
//! member, in the encodings the reader decodes. Every encoding is exact on what the reader decodes (checked over every
//! value of the 8- and 16-bit encodings: a decoded value encodes back to the stored one), so a vertex that was read writes
//! back to its bytes; a value that does not fit its encoding (a normal component of 1.5 in a signed byte, a bone index of
//! 300 in a byte) is refused rather than clamped.
#![allow(
    clippy::cast_possible_truncation,
    clippy::arithmetic_side_effects,
    reason = "floats are rounded and range-checked before they are narrowed; offsets are sums of small in-memory sizes"
)]

use super::{Member, Vertex};
use half::f16;

/// Why a vertex cannot be written.
pub(super) type Failure = &'static str;

/// The reader's step for a signed-byte-style normalised value stored as `(byte - 127) * STEP`.
const BYTE_NORM_STEP: f32 = f32::from_bits(0x3c01_0204);

/// Where the next UV set, tangent and colour of a vertex are in its channels: a vertex buffer stores its layout's members
/// in order, and the vertex's channels hold the values of all its buffers in the order the buffers are read.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Cursors {
    pub uv: usize,
    pub tangent: usize,
    pub color: usize,
}

/// `x` rounded to the nearest integer, when that is a number in range.
fn round(x: f32) -> Result<i32, Failure> {
    let r = x.round();
    if r.is_finite() && (-2_147_483_648.0..2_147_483_648.0).contains(&r) {
        Ok(r as i32)
    } else {
        Err("a value that is not a finite number")
    }
}

fn unorm8(x: f32) -> Result<u8, Failure> {
    u8::try_from(round(x * 255.)?).map_err(|_| "a normalised value outside 0 to 1")
}

fn snorm8(x: f32) -> Result<u8, Failure> {
    // the byte holds a signed value over 127
    i8::try_from(round(x * 127.)?)
        .map(|b| b as u8)
        .map_err(|_| "a normalised value outside -1 to 1")
}

fn byte_norm(x: f32) -> Result<u8, Failure> {
    u8::try_from(round(x / BYTE_NORM_STEP + 127.)?)
        .map_err(|_| "a value outside the range of a byte-normalised component")
}

fn snorm16(x: f32) -> Result<i16, Failure> {
    i16::try_from(round(x * 32767.)?).map_err(|_| "a normalised value outside -1 to 1")
}

fn offset16(x: f32) -> Result<u16, Failure> {
    u16::try_from(round(x * 32767. + 32767.)?).map_err(|_| "a normalised value outside -1 to 1")
}

fn uv16(x: f32, factor: f32) -> Result<i16, Failure> {
    i16::try_from(round(x * factor)?)
        .map_err(|_| "a UV coordinate outside the range of a 16-bit fixed-point value")
}

fn byte(n: i32) -> Result<u8, Failure> {
    u8::try_from(n).map_err(|_| "a value that does not fit a byte")
}

fn put_f32(out: &mut Vec<u8>, x: f32) {
    out.extend_from_slice(&x.to_le_bytes());
}

fn put_f16(out: &mut Vec<u8>, x: f32) {
    out.extend_from_slice(&f16::from_f32(x).to_bits().to_le_bytes());
}

fn put_i16(out: &mut Vec<u8>, x: i16) {
    out.extend_from_slice(&x.to_le_bytes());
}

fn put_u16(out: &mut Vec<u8>, x: u16) {
    out.extend_from_slice(&x.to_le_bytes());
}

/// Write `v`'s attribute for the stored member `m` at the end of `out`, advancing `cursors` past the channel entries the
/// member takes.
pub(super) fn member(
    out: &mut Vec<u8>,
    v: &Vertex,
    m: &Member,
    cursors: &mut Cursors,
    uv_factor: f32,
) -> Result<(), Failure> {
    let uv = |i: usize| {
        v.uvs
            .get(i)
            .copied()
            .ok_or("fewer UV sets than the layout stores")
    };
    let tangent = |i: usize| {
        v.tangents
            .get(i)
            .copied()
            .ok_or("fewer tangents than the layout stores")
    };
    let color = |i: usize| {
        v.colors
            .get(i)
            .copied()
            .ok_or("fewer colours than the layout stores")
    };
    match (m.semantic, m.kind) {
        (0, 2 | 3) => {
            v.position.iter().for_each(|&x| put_f32(out, x));
            if m.kind == 3 {
                put_f32(out, 0.);
            }
        }
        (1, 0x10) => {
            for w in v.bone_weights {
                out.push(snorm8(w)?);
            }
        }
        (1, 0x13) => {
            for w in v.bone_weights {
                out.push(unorm8(w)?);
            }
        }
        (1, 0x16 | 0x1a) => {
            for w in v.bone_weights {
                put_i16(out, snorm16(w)?);
            }
        }
        (2, 0x11 | 0x2f) => {
            for i in v.bone_indices {
                out.push(u8::try_from(i).map_err(|_| "a bone index that does not fit a byte")?);
            }
        }
        (2, 0x18) => {
            for i in v.bone_indices {
                put_u16(out, i);
            }
        }
        (3, 2) => v.normal.iter().for_each(|&x| put_f32(out, x)),
        (3, 3) => {
            v.normal.iter().for_each(|&x| put_f32(out, x));
            put_f32(out, v.normal_w as f32);
        }
        (3, 0x10 | 0x11 | 0x13 | 0x2f) => {
            for x in v.normal {
                out.push(byte_norm(x)?);
            }
            out.push(byte(v.normal_w)?);
        }
        (3, 0x12) => {
            // the bytes are stored W, Z, Y, X
            let [x, y, z] = v.normal;
            out.push(byte(v.normal_w)?);
            out.push(snorm8(z)?);
            out.push(snorm8(y)?);
            out.push(snorm8(x)?);
        }
        (3, 0x1a) => {
            for x in v.normal {
                put_i16(out, snorm16(x)?);
            }
            put_i16(
                out,
                i16::try_from(v.normal_w).map_err(|_| "a normal W that does not fit 16 bits")?,
            );
        }
        (3, 0x2e) => {
            for x in v.normal {
                put_u16(out, offset16(x)?);
            }
            // the word is the signed 16-bit value as stored
            put_i16(
                out,
                i16::try_from(v.normal_w).map_err(|_| "a normal W that does not fit 16 bits")?,
            );
        }
        (5, 1) => {
            let a = uv(cursors.uv)?;
            cursors.uv += 1;
            put_f32(out, a[0]);
            put_f32(out, a[1]);
        }
        (5, 2) => {
            let a = uv(cursors.uv)?;
            cursors.uv += 1;
            a.iter().take(3).for_each(|&x| put_f32(out, x));
        }
        (5, 3) => {
            let (a, b) = (uv(cursors.uv)?, uv(cursors.uv + 1)?);
            cursors.uv += 2;
            put_f32(out, a[0]);
            put_f32(out, a[1]);
            put_f32(out, b[0]);
            put_f32(out, b[1]);
        }
        (5, 0x10..=0x12 | 0x15) => {
            let a = uv(cursors.uv)?;
            cursors.uv += 1;
            put_i16(out, uv16(a[0], uv_factor)?);
            put_i16(out, uv16(a[1], uv_factor)?);
        }
        (5, 0x16) => {
            let (a, b) = (uv(cursors.uv)?, uv(cursors.uv + 1)?);
            cursors.uv += 2;
            for x in [a[0], a[1], b[0], b[1]] {
                put_i16(out, uv16(x, uv_factor)?);
            }
        }
        (5, 0x13) => {
            let a = uv(cursors.uv)?;
            cursors.uv += 1;
            for x in a {
                out.push(unorm8(x)?);
            }
        }
        (5, 0x2e) => {
            let a = uv(cursors.uv)?;
            cursors.uv += 1;
            a.iter().for_each(|&x| put_f16(out, x));
        }
        (6, 3) => {
            let t = tangent(cursors.tangent)?;
            cursors.tangent += 1;
            t.iter().for_each(|&x| put_f32(out, x));
        }
        (6, 0x10 | 0x11 | 0x13 | 0x2f) => {
            let t = tangent(cursors.tangent)?;
            cursors.tangent += 1;
            for x in t {
                out.push(byte_norm(x)?);
            }
        }
        (6, 0x1a) => {
            let t = tangent(cursors.tangent)?;
            cursors.tangent += 1;
            for x in t {
                put_i16(out, snorm16(x)?);
            }
        }
        (7, 0x10 | 0x11 | 0x13 | 0x2f) => {
            for x in v.bitangent {
                out.push(byte_norm(x)?);
            }
        }
        (10, 3) => {
            let c = color(cursors.color)?;
            cursors.color += 1;
            c.iter().for_each(|&x| put_f32(out, x));
        }
        (10, 0x10 | 0x13) => {
            let c = color(cursors.color)?;
            cursors.color += 1;
            for x in c {
                out.push(unorm8(x)?);
            }
        }
        _ => return Err("a member of a semantic and encoding the reader does not decode"),
    }
    Ok(())
}

/// Write a SpeedTree vertex's stored member `m`, the `k`th of the stored members, from [`Vertex::speedtree`].
pub(super) fn speedtree_member(out: &mut Vec<u8>, v: &Vertex, m: &Member, k: usize) -> Result<(), Failure> {
    let lanes = v
        .speedtree
        .get(k)
        .copied()
        .ok_or("fewer stored members than the layout declares")?;
    match m.kind {
        1 => lanes.iter().take(2).for_each(|&x| put_f32(out, x)),
        3 => lanes.iter().for_each(|&x| put_f32(out, x)),
        0x2d => lanes.iter().take(2).for_each(|&x| put_f16(out, x)),
        0x2e => lanes.iter().for_each(|&x| put_f16(out, x)),
        0x13 => {
            for x in lanes {
                out.push(unorm8(x)?);
            }
        }
        _ => return Err("a SpeedTree member of an encoding the reader does not decode"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every stored byte of the byte-normalised, unsigned-normalised and signed-normalised encodings, and every 16-bit
    /// value of the 16-bit ones, decodes (as the reader does) to a float that encodes back to the stored value.
    #[test]
    fn the_encodings_invert_the_readers_decoding_on_every_stored_value() {
        for b in 0..=255u8 {
            let norm = (f32::from(b) - 127.) * BYTE_NORM_STEP;
            assert_eq!(byte_norm(norm).unwrap(), b);
            assert_eq!(unorm8(f32::from(b) / 255.).unwrap(), b);
            assert_eq!(snorm8(f32::from(b as i8) / 127.).unwrap(), b);
        }
        for s in i16::MIN..=i16::MAX {
            assert_eq!(snorm16(f32::from(s) / 32767.).unwrap(), s);
            assert_eq!(uv16(f32::from(s) / 2048., 2048.).unwrap(), s);
        }
        for u in 0..=u16::MAX {
            assert_eq!(offset16((f32::from(u) - 32767.) / 32767.).unwrap(), u);
        }
    }

    #[test]
    fn a_value_that_does_not_fit_is_refused_not_clamped() {
        assert!(snorm8(1.5).is_err());
        assert!(unorm8(-0.5).is_err());
        assert!(byte_norm(f32::NAN).is_err());
        assert!(uv16(100., 2048.).is_err());
        assert!(byte(300).is_err());
    }
}
