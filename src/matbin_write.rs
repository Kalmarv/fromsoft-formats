//! The MATBIN writer: [`Material::write`].
//!
//! The game's material binaries are laid out the same way in every file of the Elden Ring install (20,619): the
//! 0x38-byte header, the parameter records (0x28 bytes each), the sampler records (0x30 bytes each), then every string
//! and value with no padding between them, in this order: for each parameter its name and its value, for each sampler
//! its type name and its path, the shader path, the material's source path, and whatever the reader kept as the
//! unreferenced tail. The records hold the offsets of what follows them.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use crate::{
    Result,
    matbin::{Material, Value},
    put::Out,
};

const FORMAT: &str = "MATBIN";
const HEADER: usize = 0x38;
const PARAM: usize = 0x28;
const SAMPLER: usize = 0x30;

/// The bytes a value of type `code` declares, `None` for a code the format does not have.
fn declared(code: u32) -> Option<usize> {
    Some(match code {
        0 => 1,
        4 | 8 => 4,
        5 | 9 => 8,
        10 => 12,
        11 => 16,
        12 => 20,
        _ => return None,
    })
}

/// A value's type code.
fn code(value: &Value) -> u32 {
    match value {
        Value::Bool(_) => 0,
        Value::Int(_) => 4,
        Value::Int2(_) => 5,
        Value::Float(_) => 8,
        Value::Float2(_) => 9,
        Value::Float3(..) => 10,
        Value::Float4(_) => 11,
        Value::Float5(_) => 12,
        Value::Truncated { code, .. } => *code,
    }
}

impl Material {
    /// Write the material as a MATBIN file, in the layout every file of the game has (see the module's note): the
    /// strings and values packed after the records, then [`Material::unreferenced_tail`]. A material that
    /// [`Material::read`] returned writes back to the bytes it was read from, for every material of Elden Ring's
    /// install. A value the format cannot hold is refused: a [`Value::Truncated`] of a code the format lacks or with
    /// as many bytes as its code declares or more (that is the typed variant's size), and a string with a NUL in it.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut out = Out::new(FORMAT);
        let params = out.word(self.params.len(), "parameter count")?;
        let samplers = out.word(self.samplers.len(), "sampler count")?;
        out.bytes(b"MAB\0");
        out.u32(2);
        out.u64(0);
        out.u64(0);
        out.u32(self.key);
        out.u32(params);
        out.u32(samplers);
        out.zeros(0x14);
        out.zeros(self.params.len() * PARAM + self.samplers.len() * SAMPLER);
        for (i, p) in self.params.iter().enumerate() {
            let record = HEADER + i * PARAM;
            let name = string(&mut out, &p.name)?;
            let at = out.len();
            value(&mut out, &p.value)?;
            out.patch_u64(record, name)?;
            out.patch_u64(record + 8, out.wide(at, "value offset")?)?;
            out.patch_u32(record + 0x10, p.key)?;
            out.patch_u32(record + 0x14, code(&p.value))?;
        }
        let samplers_at = HEADER + self.params.len() * PARAM;
        for (i, s) in self.samplers.iter().enumerate() {
            let record = samplers_at + i * SAMPLER;
            let kind = string(&mut out, &s.kind)?;
            let path = string(&mut out, &s.path)?;
            out.patch_u64(record, kind)?;
            out.patch_u64(record + 8, path)?;
            out.patch_u32(record + 0x10, s.key)?;
            let [first, second] = s.unknown;
            out.patch_u32(record + 0x14, first.to_bits())?;
            out.patch_u32(record + 0x18, second.to_bits())?;
        }
        let shader = string(&mut out, &self.shader)?;
        let source = string(&mut out, &self.source)?;
        out.patch_u64(8, shader)?;
        out.patch_u64(16, source)?;
        out.bytes(&self.unreferenced_tail);
        Ok(out.into_bytes())
    }
}

/// A NUL-terminated UTF-16 string at the current offset; returns the offset it starts at.
fn string(out: &mut Out, text: &str) -> Result<u64> {
    if text.contains('\0') {
        return Err(out.err(format!("{text:?} holds a NUL, which ends a stored string")));
    }
    let at = out.wide(out.len(), "string offset")?;
    for unit in text.encode_utf16() {
        out.u16(unit);
    }
    out.u16(0);
    Ok(at)
}

/// A value's stored bytes.
fn value(out: &mut Out, value: &Value) -> Result<()> {
    match value {
        Value::Bool(b) => out.u8(u8::from(*b)),
        Value::Int(x) => out.i32(*x),
        Value::Int2(a) => a.iter().for_each(|&x| out.i32(x)),
        Value::Float(x) => out.f32(*x),
        Value::Float2(a) => a.iter().for_each(|&x| out.f32(x)),
        Value::Float3(a, extra) => {
            a.iter().for_each(|&x| out.f32(x));
            if let Some(more) = extra {
                more.iter().for_each(|&x| out.f32(x));
            }
        }
        Value::Float4(a) => a.iter().for_each(|&x| out.f32(x)),
        Value::Float5(a) => a.iter().for_each(|&x| out.f32(x)),
        Value::Truncated { code, bytes } => {
            let size = declared(*code).ok_or_else(|| out.err(format!("parameter type {code}")))?;
            if bytes.is_empty() || bytes.len() >= size {
                return Err(out.err(format!(
                    "a truncated value of type {code} holds {} bytes, none or as many as the type declares ({size})",
                    bytes.len()
                )));
            }
            out.bytes(bytes);
        }
    }
    Ok(())
}
