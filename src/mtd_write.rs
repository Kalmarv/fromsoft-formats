//! The MTD writer: [`MaterialDefinition::write`] and [`MaterialDefinition::write_canonical`].
//!
//! An MTD is a tree of blocks (a zero word, the block's length, its kind and version, a marker byte and the alignment
//! padding after it). The reader keeps the lengths the file declares for its blocks ([`MaterialDefinition::block_lengths`]:
//! the game's own files overstate them: 83 of Sekiro's 3,170 files and 92 of Elden Ring's 2,602 do) and the non-zero bytes in
//! the alignment padding ([`MaterialDefinition::padding`]), so a definition that was read writes back to the bytes it was
//! read from.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use crate::{
    Result,
    mtd::{BlockLength, MaterialDefinition, PaddingAnomaly, Texture, Value},
    put::Out,
};

const FORMAT: &str = "MTD";

/// A parameter's type name, the kind and marker of its value block, and how many values it holds.
fn parameter_type(value: &Value) -> Option<(&'static str, u32, u8, u32)> {
    Some(match value {
        Value::Bool(_) => ("bool", 0x1000, 0xc0, 1),
        Value::Int(v) => match v.len() {
            1 => ("int", 0x1001, 0xc5, 1),
            2 => ("int2", 0x1001, 0xc5, 2),
            _ => return None,
        },
        Value::Float(v) => match v.len() {
            1 => ("float", 0x1002, 0xca, 1),
            2 => ("float2", 0x1002, 0xca, 2),
            3 => ("float3", 0x1002, 0xca, 3),
            4 => ("float4", 0x1002, 0xca, 4),
            _ => return None,
        },
    })
}

struct Writer<'a> {
    out: Out,
    /// The blocks being written: the offset of each one's length word and its kind.
    open: Vec<(usize, u32)>,
    /// The lengths a read recorded, in the order blocks end; `None` writes each block the length it spans.
    recorded: Option<&'a [BlockLength]>,
    ended: usize,
    /// The non-zero padding a read recorded, in file order, and how many of them have been written back.
    padding: &'a [PaddingAnomaly],
    padded: usize,
}

impl Writer<'_> {
    fn word(&mut self, v: u32) {
        self.out.u32(v);
    }

    /// The marker byte and the alignment padding after it: zeros, or the bytes the read recorded at this place.
    fn marker(&mut self, marker: u8) -> Result<()> {
        self.out.u8(marker);
        let run = (4 - self.out.len() % 4) % 4;
        match self.padding.get(self.padded) {
            Some(odd) if odd.offset == self.out.len() => {
                if odd.bytes.len() != run {
                    return Err(self.out.err(format!(
                        "recorded padding of {} bytes at a marker that is followed by {run}",
                        odd.bytes.len()
                    )));
                }
                self.out.bytes(&odd.bytes);
                self.padded += 1;
            }
            _ => self.out.zeros(run),
        }
        Ok(())
    }

    /// A length-prefixed Shift-JIS string and its marker.
    fn string(&mut self, text: &str, marker: u8) -> Result<()> {
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(text);
        if unmappable {
            return Err(self.out.err(format!("{text:?} has a character Shift-JIS lacks")));
        }
        let n = self.out.word(bytes.len(), "string length")?;
        self.word(n);
        self.out.bytes(&bytes);
        self.marker(marker)
    }

    fn block(&mut self, kind: u32, version: u32, marker: u8) -> Result<()> {
        self.word(0);
        self.open.push((self.out.len(), kind));
        self.word(0);
        self.word(kind);
        self.word(version);
        self.marker(marker)
    }

    /// Close the innermost block, writing its length: the one the read recorded for it where there is one (it must
    /// describe a block of this kind and size), else the bytes it spans.
    fn end(&mut self) -> Result<()> {
        let (at, kind) = self
            .open
            .pop()
            .ok_or_else(|| self.out.err("a block closed that was never opened"))?;
        let consumed = self.out.len() - (at + 4);
        let declared = match self.recorded {
            None => consumed,
            Some(recorded) => {
                let record = recorded.get(self.ended).ok_or_else(|| {
                    self.out.err(format!(
                        "block {} ends, the recorded lengths hold {} blocks",
                        self.ended,
                        recorded.len()
                    ))
                })?;
                if record.kind != kind || record.consumed != consumed {
                    return Err(self.out.err(format!(
                        "block {} is kind {kind} spanning {consumed} bytes, the recorded one is kind {} spanning {}: clear \
                         `block_lengths` or write the canonical form",
                        self.ended, record.kind, record.consumed
                    )));
                }
                record.declared
            }
        };
        self.ended += 1;
        let declared = self.out.word(declared, "block length")?;
        self.out.patch_u32(at, declared)
    }

    fn texture(&mut self, t: &Texture) -> Result<()> {
        if !matches!(t.version, 3 | 5) {
            return Err(self.out.err(format!("texture block version {}", t.version)));
        }
        self.block(0x2000, t.version, 0xa3)?;
        self.string(&t.kind, 0x35)?;
        self.word(t.uv_number as u32);
        self.marker(0x35)?;
        self.word(t.shader_data_index as u32);
        if t.version == 5 {
            self.word(0xa3);
            self.string(&t.path, 0xba)?;
            let n = self.out.word(t.extra.len(), "texture float count")?;
            self.word(n);
            for &x in &t.extra {
                self.out.f32(x);
            }
        } else if !t.path.is_empty() || !t.extra.is_empty() {
            return Err(self
                .out
                .err("a version 3 texture block holds no path and no floats"));
        }
        self.end()
    }

    fn run(&mut self, m: &MaterialDefinition) -> Result<()> {
        self.block(0, 3, 1)?;
        self.block(1, 2, 0xb0)?;
        self.string("MTD ", 0x34)?;
        self.word(1000);
        self.end()?;
        self.marker(1)?;
        self.block(2, 4, 0xa3)?;
        self.string(&m.shader, 0xa3)?;
        self.string(&m.description, 3)?;
        self.word(1);
        self.block(3, 4, 0xa3)?;
        self.word(0);
        self.marker(3)?;
        let n = self.out.word(m.parameters.len(), "parameter count")?;
        self.word(n);
        for p in &m.parameters {
            let (name, block, marker, count) = parameter_type(&p.value).ok_or_else(|| {
                self.out
                    .err(format!("parameter {:?} holds a value of no stored type", p.name))
            })?;
            self.block(4, 4, 0xa3)?;
            self.string(&p.name, 0xa3)?;
            self.string(name, 4)?;
            self.word(1);
            self.block(block, 1, marker)?;
            self.word(count);
            match &p.value {
                Value::Bool(b) => self.out.u8(u8::from(*b)),
                Value::Int(v) => v.iter().for_each(|&x| self.out.i32(x)),
                Value::Float(v) => v.iter().for_each(|&x| self.out.f32(x)),
            }
            self.end()?;
            self.marker(4)?;
            self.word(0);
            self.end()?;
        }
        self.marker(3)?;
        let n = self.out.word(m.textures.len(), "texture count")?;
        self.word(n);
        for t in &m.textures {
            self.texture(t)?;
        }
        for _ in 0..3 {
            self.marker(4)?;
            self.word(0);
            self.end()?;
        }
        if !self.open.is_empty() {
            return Err(self.out.err("a block was never closed"));
        }
        if let Some(recorded) = self.recorded
            && self.ended != recorded.len()
        {
            return Err(self.out.err(format!(
                "{} blocks written, the recorded lengths hold {}",
                self.ended,
                recorded.len()
            )));
        }
        if let Some(odd) = self.padding.get(self.padded) {
            return Err(self.out.err_at(
                odd.offset,
                "recorded padding that falls at no marker of the written file: clear `padding` or write the canonical form",
            ));
        }
        Ok(())
    }
}

impl MaterialDefinition {
    /// Write the definition as an MTD file. A definition that [`MaterialDefinition::read`] returned writes back to the
    /// bytes it was read from: every block gets the length the file declared for it
    /// ([`MaterialDefinition::block_lengths`]) and the alignment padding keeps the non-zero bytes it held
    /// ([`MaterialDefinition::padding`]). Both records must still describe the structure being written (the same
    /// blocks, each of the size it was read with, markers at the same offsets): a definition edited past that is
    /// refused with the block or offset that no longer fits, to be written either with those two fields cleared (empty
    /// vectors write each block the length it spans and zero padding) or with [`MaterialDefinition::write_canonical`].
    /// A parameter or texture the format cannot store (an integer list of other than one or two values, a float list
    /// of other than one to four, a texture block version other than 3 and 5, text Shift-JIS lacks) is refused.
    pub fn write(&self) -> Result<Vec<u8>> {
        let recorded = (!self.block_lengths.is_empty()).then_some(self.block_lengths.as_slice());
        self.write_with(recorded, &self.padding)
    }

    /// Write the definition as an MTD file in canonical form: every block the length it spans and zero alignment
    /// padding, whatever [`MaterialDefinition::block_lengths`] and [`MaterialDefinition::padding`] hold. This is the
    /// form for a definition built or edited here; it differs from the stored bytes of the game's definitions that
    /// overstate their block lengths (83 of Sekiro's and 92 of Elden Ring's) or hold leftover padding bytes (1 of Elden Ring's).
    pub fn write_canonical(&self) -> Result<Vec<u8>> {
        self.write_with(None, &[])
    }

    fn write_with(&self, recorded: Option<&[BlockLength]>, padding: &[PaddingAnomaly]) -> Result<Vec<u8>> {
        let mut w = Writer {
            out: Out::new(FORMAT, false),
            open: Vec::new(),
            recorded,
            ended: 0,
            padding,
            padded: 0,
        };
        w.run(self)?;
        Ok(w.out.finish())
    }
}
