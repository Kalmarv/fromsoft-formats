//! MTD material definitions (`.mtd`, Sekiro): a shader, a description, typed parameters and texture slots, in nested
//! blocks whose counts and markers are all checked against the file, with Shift-JIS strings decoded strictly.
use primase_foundations::bytes::Endian;

use crate::{Result, invalid};

/// A parameter value.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Value {
    /// A boolean.
    Bool(bool),
    /// One or two integers.
    Int(Vec<i32>),
    /// One to four floats.
    Float(Vec<f32>),
}
/// A named parameter of a material definition.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Parameter {
    /// The parameter's name.
    pub name: String,
    /// Its value.
    pub value: Value,
}
/// A texture slot of a material definition.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Texture {
    /// The slot's kind, as the file names it.
    pub kind: String,
    /// The UV set it uses.
    pub uv_number: i32,
    /// Its index in the shader's data.
    pub shader_data_index: i32,
    /// The texture's path (empty in the older block version, which stores none).
    pub path: String,
    /// The extra floats the newer block version stores after the path.
    pub extra: Vec<f32>,
}
/// A block's declared length against the bytes it was read to consume.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BlockLength {
    /// The offset where the block's contents start.
    pub start: usize,
    /// The block's kind code.
    pub kind: u32,
    /// The length the file declares.
    pub declared: usize,
    /// The length the structure actually spans.
    pub consumed: usize,
}
/// Non-zero bytes in a marker's alignment padding. Every MTD of Elden Ring and Sekiro has zero padding everywhere but
/// one Elden Ring file (`c9757_crystal.mtd`), whose padding holds 19 runs of leftover bytes, mostly ASCII fragments.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PaddingAnomaly {
    /// The offset of the padding's first byte.
    pub offset: usize,
    /// The padding bytes as the file holds them (one to three).
    pub bytes: Vec<u8>,
}
/// A read material definition.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MaterialDefinition {
    /// The shader it names.
    pub shader: String,
    /// Its description string.
    pub description: String,
    /// Its parameters, in file order.
    pub parameters: Vec<Parameter>,
    /// Its texture slots, in file order.
    pub textures: Vec<Texture>,
    /// Every block's declared and consumed length, in the order blocks end.
    pub block_lengths: Vec<BlockLength>,
    /// Every marker whose alignment padding is not all zero, in file order (empty on every MTD of both games but one).
    pub padding: Vec<PaddingAnomaly>,
}
struct Cursor<'a> {
    /// The file, read in order (little-endian).
    c: primase_foundations::bytes::Cursor<'a>,
    stack: Vec<(usize, usize, u32)>,
    blocks: Vec<BlockLength>,
    padding: Vec<PaddingAnomaly>,
}
impl Cursor<'_> {
    fn bytes(&mut self, what: &str, n: usize) -> Result<&[u8]> {
        let (at, left) = (self.c.position(), self.c.remaining());
        self.c.take(n).map_err(|_| {
            invalid(
                "MTD",
                at,
                format!("{what} needs {n} bytes, {left} are left in the file"),
            )
        })
    }
    fn u32(&mut self, what: &str) -> Result<u32> {
        let (at, left) = (self.c.position(), self.c.remaining());
        self.c.u32(Endian::Little).map_err(|_| {
            invalid(
                "MTD",
                at,
                format!("{what} needs a 4-byte word, {left} bytes are left in the file"),
            )
        })
    }
    fn expect(&mut self, n: u32) -> Result<()> {
        let at = self.c.position();
        let found = self.u32(&format!("a field of value {n}"))?;
        if found != n {
            return Err(invalid("MTD", at, format!("expected {n}, found {found}")));
        }
        Ok(())
    }
    fn count(&mut self, stride: usize) -> Result<usize> {
        let (at, left) = (self.c.position(), self.c.remaining());
        self.c.count(Endian::Little, stride).map_err(|_| {
            invalid(
                "MTD",
                at,
                format!(
                    "a count of records of {stride} bytes each, which the {} bytes after the count in the file cannot hold",
                    left.saturating_sub(4)
                ),
            )
        })
    }
    fn marker(&mut self, expected: u8) -> Result<()> {
        let at = self.c.position();
        if self.bytes("a marker byte", 1)?.first() != Some(&expected) {
            return Err(invalid("MTD", at, "marker"));
        }
        // the bytes up to the next multiple of four
        let padding = self.c.position().wrapping_neg() % 4;
        let offset = self.c.position();
        let bytes = self.bytes("alignment padding", padding)?;
        if bytes.iter().any(|b| *b != 0) {
            let bytes = bytes.to_vec();
            self.padding.push(PaddingAnomaly { offset, bytes });
        }
        Ok(())
    }
    fn string(&mut self, marker: u8) -> Result<String> {
        let n = self.count(1)?;
        let at = self.c.position();
        let s = encoding_rs::SHIFT_JIS
            .decode_without_bom_handling_and_without_replacement(self.bytes("a string", n)?)
            .ok_or_else(|| invalid("MTD", at, "malformed Shift-JIS string"))?
            .into_owned();
        self.marker(marker)?;
        Ok(s)
    }
    fn block(&mut self, kind: u32, versions: &[u32], marker: u8) -> Result<u32> {
        self.expect(0)?;
        let len = self.u32("a block length")? as usize;
        self.stack.push((self.c.position(), len, kind));
        self.expect(kind)?;
        let version = self.u32("a block version")?;
        if !versions.contains(&version) {
            return Err(invalid(
                "MTD",
                self.c.position().saturating_sub(4),
                "block version",
            ));
        }
        self.marker(marker)?;
        Ok(version)
    }
    fn end(&mut self) -> Result<()> {
        let (start, declared, kind) = self
            .stack
            .pop()
            .ok_or_else(|| invalid("MTD", self.c.position(), "block nesting"))?;
        self.blocks.push(BlockLength {
            start,
            kind,
            declared,
            consumed: self.c.position().saturating_sub(start),
        });
        Ok(())
    }
    fn float(&mut self) -> Result<f32> {
        let at = self.c.position();
        let x = f32::from_bits(self.u32("a float parameter")?);
        if !x.is_finite() {
            return Err(invalid("MTD", at, "non-finite parameter"));
        }
        Ok(x)
    }
}
impl MaterialDefinition {
    /// Read a decompressed MTD file. The structure is authoritative: typed counts and markers determine where blocks
    /// end, always within the file, and the declared block lengths are kept in [`MaterialDefinition::block_lengths`]
    /// as diagnostics (the game's own files can overstate them). Non-zero alignment padding after a marker is read past and
    /// recorded in [`MaterialDefinition::padding`]. Bytes after the last block are an error.
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut c = Cursor {
            c: primase_foundations::bytes::Cursor::new(data),
            stack: vec![],
            blocks: vec![],
            padding: vec![],
        };
        c.block(0, &[3], 1)?;
        c.block(1, &[2], 0xb0)?;
        if c.string(0x34)? != "MTD " {
            return Err(invalid("MTD", c.c.position(), "magic"));
        }
        c.expect(1000)?;
        c.end()?;
        c.marker(1)?;
        c.block(2, &[4], 0xa3)?;
        let shader = c.string(0xa3)?;
        let description = c.string(3)?;
        c.expect(1)?;
        c.block(3, &[4], 0xa3)?;
        c.expect(0)?;
        c.marker(3)?;
        let mut parameters = Vec::new();
        for _ in 0..c.count(20)? {
            c.block(4, &[4], 0xa3)?;
            let name = c.string(0xa3)?;
            let kind = c.string(4)?;
            c.expect(1)?;
            let (block, marker, n) = match kind.as_str() {
                "bool" => (0x1000, 0xc0, 1),
                "int" => (0x1001, 0xc5, 1),
                "int2" => (0x1001, 0xc5, 2),
                "float" => (0x1002, 0xca, 1),
                "float2" => (0x1002, 0xca, 2),
                "float3" => (0x1002, 0xca, 3),
                "float4" => (0x1002, 0xca, 4),
                _ => return Err(invalid("MTD", c.c.position(), format!("parameter type {kind}"))),
            };
            c.block(block, &[1], marker)?;
            c.expect(n)?;
            let value = if block == 0x1000 {
                match c.bytes("a boolean value", 1)?.first() {
                    Some(0) => Value::Bool(false),
                    Some(1) => Value::Bool(true),
                    _ => return Err(invalid("MTD", c.c.position().saturating_sub(1), "boolean")),
                }
            } else if block == 0x1001 {
                Value::Int(
                    (0..n)
                        .map(|_| c.u32("an integer parameter").map(|x| x as i32))
                        .collect::<Result<_>>()?,
                )
            } else {
                Value::Float((0..n).map(|_| c.float()).collect::<Result<_>>()?)
            };
            c.end()?;
            c.marker(4)?;
            c.expect(0)?;
            c.end()?;
            parameters.push(Parameter { name, value });
        }
        c.marker(3)?;
        let mut textures = Vec::new();
        for _ in 0..c.count(20)? {
            let version = c.block(0x2000, &[3, 5], 0xa3)?;
            let kind = c.string(0x35)?;
            let uv_number = c.u32("a UV number")? as i32;
            c.marker(0x35)?;
            let shader_data_index = c.u32("a shader data index")? as i32;
            let (path, extra) = if version == 5 {
                c.expect(0xa3)?;
                let path = c.string(0xba)?;
                let n = c.count(4)?;
                (path, (0..n).map(|_| c.float()).collect::<Result<Vec<_>>>()?)
            } else {
                (String::new(), vec![])
            };
            c.end()?;
            textures.push(Texture {
                kind,
                uv_number,
                shader_data_index,
                path,
                extra,
            });
        }
        c.marker(4)?;
        c.expect(0)?;
        c.end()?;
        c.marker(4)?;
        c.expect(0)?;
        c.end()?;
        c.marker(4)?;
        c.expect(0)?;
        c.end()?;
        if c.c.position() != data.len() {
            return Err(invalid("MTD", c.c.position(), "trailing bytes"));
        }
        Ok(Self {
            shader,
            description,
            parameters,
            textures,
            block_lengths: c.blocks,
            padding: c.padding,
        })
    }
}

#[cfg(test)]
mod short_tests {
    use super::*;

    /// A read past the end says what it needed and how much was left, at the position of the read.
    #[test]
    fn a_truncated_file_says_what_it_needed() {
        let e = MaterialDefinition::read(&[0, 0]).unwrap_err().to_string();
        assert_eq!(
            e,
            "MTD at 0x0: a field of value 0 needs a 4-byte word, 2 bytes are left in the file"
        );
        let e = MaterialDefinition::read(&[]).unwrap_err().to_string();
        assert!(e.contains("0 bytes are left"), "{e}");
    }
}
