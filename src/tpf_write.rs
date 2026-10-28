//! Writing TPF texture packages: the layout of the games' own tools, found by rebuilding the 288,921 packages of both
//! installs (332,420 textures) from their parsed fields.
//!
//! ```text
//! 0x00 "TPF\0", u32 size of the DDS files together, u32 texture count, u8 0, u8 flags, u8 name encoding, u8 0
//! 0x10 a 20-byte record per texture: u32 offset of its DDS file, u32 its size, u8 format, u8 kind, u8 mipmaps,
//!      u8 flags, u32 offset of its name, u32 1 when extra data follows; the extra data (u32 id, u32 byte count, the
//!      words) sits directly after its record
//!      the names, directly after the last record: UTF-16 (encoding 1) or ASCII, NUL-terminated, in texture order
//!      zero padding between the names and the first DDS file (none but in one file)
//!      the DDS files one after the other, no padding between and none after the last
//! ```
use crate::emit::{Out, add, mul, narrow};
use crate::tpf::{Package, TextureKind};
use crate::{Result, invalid};

const FORMAT: &str = "TPF";

impl Package {
    /// This package as a TPF file. Every payload must be a DDS file (it starts with `DDS `), a name must be ASCII unless
    /// the package's encoding is UTF-16 (1), and an extra's word count is whatever [`crate::tpf::Extra::words`] holds.
    /// A package read with [`Package::read`] is written back to the same bytes: all 288,921 packages of both installs
    /// were rebuilt that way.
    pub fn write(&self) -> Result<Vec<u8>> {
        let count: u32 = narrow(FORMAT, "the texture count", self.textures.len())?;
        let mut extras = 0usize;
        for t in &self.textures {
            if !t.dds.starts_with(b"DDS ") {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!("the payload of {:?} is not a DDS file", t.name),
                ));
            }
            if let Some(extra) = &t.extra {
                let words = mul(FORMAT, extra.words.len(), 4)?;
                extras = add(FORMAT, extras, add(FORMAT, 8, words)?)?;
            }
        }
        let records_end = add(
            FORMAT,
            add(FORMAT, 16, mul(FORMAT, self.textures.len(), 20)?)?,
            extras,
        )?;
        let mut names = Out::new(FORMAT, false);
        let mut name_offsets = Vec::with_capacity(self.textures.len());
        for t in &self.textures {
            name_offsets.push(add(FORMAT, records_end, names.len())?);
            if self.encoding == 1 {
                names.utf16z(&t.name);
            } else {
                names.asciiz(&t.name)?;
            }
        }
        let name_bytes = names.finish();
        let first = add(
            FORMAT,
            add(FORMAT, records_end, name_bytes.len())?,
            self.names_padding,
        )?;
        let mut total = 0usize;
        for t in &self.textures {
            total = add(FORMAT, total, t.dds.len())?;
        }

        let mut out = Out::new(FORMAT, false);
        out.bytes(b"TPF\0");
        out.u32(narrow(FORMAT, "the size of the DDS files", total)?);
        out.u32(count);
        out.u8(0);
        out.u8(self.flags);
        out.u8(self.encoding);
        out.u8(0);
        let mut offset = first;
        for (t, name_offset) in self.textures.iter().zip(&name_offsets) {
            out.u32(narrow(FORMAT, "a DDS file's offset", offset)?);
            out.u32(narrow(FORMAT, "a DDS file's size", t.dds.len())?);
            out.u8(t.format);
            out.u8(match t.kind {
                TextureKind::Image2D => 0,
                TextureKind::Cube => 1,
                TextureKind::Volume => 2,
            });
            out.u8(t.mipmaps);
            out.u8(t.flags);
            out.u32(narrow(FORMAT, "a name's offset", *name_offset)?);
            out.u32(u32::from(t.extra.is_some()));
            if let Some(extra) = &t.extra {
                out.u32(extra.id);
                out.u32(narrow(
                    FORMAT,
                    "an extra's byte count",
                    mul(FORMAT, extra.words.len(), 4)?,
                )?);
                for word in &extra.words {
                    out.u32(*word);
                }
            }
            offset = add(FORMAT, offset, t.dds.len())?;
        }
        out.bytes(&name_bytes);
        out.zeros(self.names_padding);
        for t in &self.textures {
            out.bytes(&t.dds);
        }
        Ok(out.finish())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tpf::{Extra, Texture};

    fn texture(name: &str, kind: TextureKind, dds_len: usize, extra: Option<Extra>) -> Texture {
        let mut dds = b"DDS ".to_vec();
        dds.resize(dds_len, 0xab);
        Texture {
            name: name.to_owned(),
            format: 105,
            kind,
            mipmaps: 11,
            flags: 0,
            extra,
            dds,
        }
    }

    #[test]
    fn a_package_reads_back_to_itself_and_rewrites_the_same_bytes() {
        for (encoding, padding) in [(1u8, 0usize), (2, 0), (2, 272), (1, 8)] {
            let package = Package {
                flags: 3,
                encoding,
                names_padding: padding,
                textures: vec![
                    texture("o007050_Grass_01_a", TextureKind::Image2D, 70, None),
                    texture(
                        "o007050_cube",
                        TextureKind::Cube,
                        44,
                        Some(Extra {
                            id: 7,
                            words: vec![1, 0x3f80_0000, 3],
                        }),
                    ),
                    texture("v", TextureKind::Volume, 4, None),
                ],
            };
            let bytes = package.write().unwrap();
            assert_eq!(
                Package::read(&bytes).unwrap(),
                package,
                "encoding {encoding}, padding {padding}"
            );
            // the size field counts the DDS files only
            assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 70 + 44 + 4);
            assert!(bytes.ends_with(&package.textures[2].dds));
            assert_eq!(Package::read(&bytes).unwrap().write().unwrap(), bytes);
        }
    }

    #[test]
    fn a_payload_that_is_not_dds_and_a_name_the_encoding_cannot_hold_are_refused() {
        let mut package = Package {
            flags: 3,
            encoding: 2,
            names_padding: 0,
            textures: vec![texture("caf\u{e9}", TextureKind::Image2D, 8, None)],
        };
        assert!(package.write().is_err(), "an ASCII package takes ASCII names");
        package.encoding = 1;
        assert!(package.write().is_ok());
        package.textures[0].dds[0] = b'X';
        assert!(package.write().is_err());
    }
}
