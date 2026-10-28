//! TPF texture packages (PC): each texture's name and flags with its DDS file, retained exactly as stored, all faces
//! and mipmaps included. Pixels are not decoded.
use crate::{
    Result,
    bytes::{Budget, View, coverage},
    invalid,
};
/// The pixel layout a texture's stored format code stands for. The DDS file inside the texture states the layout itself
/// (a DXGI format in its extended header, a FourCC or channel masks); the code is the packager's, and a layout may be
/// stored under several codes (`DXT1` as 0, 1 and 108) and in more than one DDS dialect (code 0 is a DXGI BC1 sRGB header
/// in 12,647 Elden Ring textures and a plain `DXT1` FourCC, which cannot say sRGB, in 3,389).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PixelFamily {
    /// BC1 (`DXT1`): codes 0, 1 and 108.
    Bc1,
    /// BC3 (`DXT5`): codes 5 and 110.
    Bc3,
    /// BC4 (`ATI1`, one channel): code 103.
    Bc4,
    /// BC5 (`ATI2`, two channels): code 104.
    Bc5,
    /// BC6H (high dynamic range, unsigned): code 115.
    Bc6h,
    /// BC7: codes 102, 106, 107 and 112.
    Bc7,
    /// Sixteen bits, one of alpha over five each of red, green and blue (`A1R5G5B5`; the DDS masks `0x7c00`, `0x3e0`,
    /// `0x1f`, `0x8000`): code 6.
    A1R5G5B5,
    /// Four bytes in the order blue, green, red, alpha (masks `0xff0000`, `0xff00`, `0xff`, `0xff000000`): code 9.
    B8G8R8A8,
    /// Four bytes in the order red, green, blue, alpha (masks `0xff`, `0xff00`, `0xff0000`, `0xff000000`): code 105.
    R8G8B8A8,
    /// Four bytes in the order blue, green, red and one unused (masks `0xff0000`, `0xff00`, `0xff`): code 10.
    B8G8R8X8,
    /// Four half-float channels (Direct3D 9's format 113, a FourCC of that number): code 22.
    Rgba16Float,
}

impl PixelFamily {
    /// The family a stored format code stands for, `None` for a code no texture of the two games' installs stores
    /// (the community's tables list 3, 8, 23 to 25, 29, 33, 36, 100 and 113 as well; none occurs in either install).
    pub const fn of_code(code: u8) -> Option<Self> {
        Some(match code {
            0 | 1 | 108 => Self::Bc1,
            5 | 110 => Self::Bc3,
            103 => Self::Bc4,
            104 => Self::Bc5,
            115 => Self::Bc6h,
            102 | 106 | 107 | 112 => Self::Bc7,
            6 => Self::A1R5G5B5,
            9 => Self::B8G8R8A8,
            105 => Self::R8G8B8A8,
            10 => Self::B8G8R8X8,
            22 => Self::Rgba16Float,
            _ => return None,
        })
    }

    /// Whether the layout is block-compressed.
    pub const fn is_compressed(self) -> bool {
        matches!(
            self,
            Self::Bc1 | Self::Bc3 | Self::Bc4 | Self::Bc5 | Self::Bc6h | Self::Bc7
        )
    }
}

/// What a texture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureKind {
    /// A 2D image (stored kind 0).
    Image2D,
    /// A cube map (stored kind 1).
    Cube,
    /// A volume texture (stored kind 2).
    Volume,
}
/// Opaque optional metadata; words retain their bits rather than assuming floats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extra {
    /// The stored ID.
    pub id: u32,
    /// The stored words.
    pub words: Vec<u32>,
}
/// One texture of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    /// The texture's name as stored.
    pub name: String,
    /// The stored format code.
    pub format: u8,
    /// What the texture is.
    pub kind: TextureKind,
    /// The stored mipmap count.
    pub mipmaps: u8,
    /// The texture's flags byte as stored: 0 or 1, or 0x80 on Elden Ring's environment-probe packages (every one of the
    /// 221,796 textures of the `*_envmap_*_middle_*.tpfbnd.dcx` binders). The reader keeps the byte and assigns it no meaning.
    pub flags: u8,
    /// Its optional extra data.
    pub extra: Option<Extra>,
    /// The DDS file, complete, starting with `DDS `.
    pub dds: Vec<u8>,
}
impl Texture {
    /// The pixel layout of the stored format code ([`PixelFamily::of_code`]).
    pub fn pixel_family(&self) -> Option<PixelFamily> {
        PixelFamily::of_code(self.format)
    }
}
/// A read texture package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The package's flags as stored.
    pub flags: u8,
    /// The names' encoding as stored (1: UTF-16, else ASCII).
    pub encoding: u8,
    /// The textures, in file order.
    pub textures: Vec<Texture>,
}
impl Package {
    /// Read a decompressed `.tpf`. Counts, offsets and sizes are bounded by the file, every payload must be a DDS file, and
    /// every byte must be accounted for (up to 15 zero bytes at the end).
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: "TPF",
            be: false,
            base: 0,
        };
        v.expect(0, b"TPF\0")?;
        v.expect(12, &[0])?;
        v.expect(15, &[0])?;
        let data_size = v.u32(4)? as usize;
        let count = v.u32(8)? as usize;
        let flags = v.u8(13)?;
        let encoding = v.u8(14)?;
        if flags > 3 || encoding > 2 {
            return Err(invalid("TPF", 13, "unknown flags or encoding"));
        }
        v.slice(
            16,
            count
                .checked_mul(20)
                .ok_or_else(|| invalid("TPF", 8, "count overflow"))?,
        )?;
        let mut p = 16;
        let mut descriptors = Vec::new();
        for _ in 0..count {
            let offset = v.u32(p)? as usize;
            let size = v.u32(v.at(p, 4)?)? as usize;
            let format = v.u8(v.at(p, 8)?)?;
            let kind = match v.u8(v.at(p, 9)?)? {
                0 => TextureKind::Image2D,
                1 => TextureKind::Cube,
                2 => TextureKind::Volume,
                _ => return Err(invalid("TPF", v.at(p, 9)?, "unknown texture kind")),
            };
            let mipmaps = v.u8(v.at(p, 10)?)?;
            let texture_flags = v.u8(v.at(p, 11)?)?;
            if !matches!(texture_flags, 0 | 1 | 0x80) {
                return Err(invalid(
                    "TPF",
                    v.at(p, 11)?,
                    "compressed/unknown PC texture flags",
                ));
            }
            let name = v.u32(v.at(p, 12)?)? as usize;
            let has_extra = v.u32(v.at(p, 16)?)?;
            let flag_at = v.at(p, 16)?;
            p = v.at(p, 20)?;
            let extra = match has_extra {
                0 => None,
                1 => {
                    let id = v.u32(p)?;
                    let len_at = v.at(p, 4)?;
                    let len = v.u32(len_at)? as usize;
                    if !len.is_multiple_of(4) {
                        return Err(invalid("TPF", len_at, "extra word size"));
                    }
                    let words_at = v.at(p, 8)?;
                    v.slice(words_at, len)?;
                    let mut words = Vec::new();
                    for i in (0..len).step_by(4) {
                        words.push(v.u32(v.at(words_at, i)?)?);
                    }
                    p = v.at(words_at, len)?;
                    Some(Extra { id, words })
                }
                _ => return Err(invalid("TPF", flag_at, "extra flag")),
            };
            descriptors.push((offset, size, format, kind, mipmaps, texture_flags, name, extra));
        }
        let budget = Budget::new("TPF", data.len());
        let mut spans = vec![(0, p)];
        let mut names_end = p;
        let mut payload_start = None::<usize>;
        let mut payload_end = 0;
        let mut textures = Vec::new();
        for (offset, size, format, kind, mipmaps, flags, name, extra) in descriptors {
            let (text, end) = if encoding == 1 {
                v.utf16z(name, &budget)?
            } else {
                v.asciiz(name, &budget)?
            };
            spans.push((name, end.saturating_sub(name)));
            names_end = names_end.max(end);
            budget.charge(offset, size)?;
            let dds = v.slice(offset, size)?.to_vec();
            if !dds.starts_with(b"DDS ") {
                return Err(invalid("TPF", offset, "PC texture is not DDS"));
            }
            spans.push((offset, size));
            payload_start = Some(payload_start.map_or(offset, |first| first.min(offset)));
            payload_end = payload_end.max(v.at(offset, size)?);
            textures.push(Texture {
                name: text,
                format,
                kind,
                mipmaps,
                flags,
                extra,
                dds,
            });
        }
        let logical_end = payload_end.max(names_end);
        // PC producers measure data size either before payload alignment or at
        // the first DDS. Padding between sections is zero but has no fixed width.
        let first = payload_start.unwrap_or(names_end);
        if first < names_end
            || (data_size != logical_end.saturating_sub(names_end)
                && data_size != logical_end.saturating_sub(first))
        {
            return Err(invalid("TPF", 4, "data length mismatch"));
        }
        if data.len().saturating_sub(logical_end) > 15
            || v.slice(logical_end, data.len().saturating_sub(logical_end))?
                .iter()
                .any(|b| *b != 0)
        {
            return Err(invalid("TPF", logical_end, "trailing bytes"));
        }
        coverage(&v, spans, logical_end, usize::MAX)?;
        Ok(Self {
            flags,
            encoding,
            textures,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Package, PixelFamily};

    #[test]
    fn the_codes_the_installs_store_have_a_family() {
        let stored = [
            0u8, 1, 5, 6, 9, 10, 22, 102, 103, 104, 105, 106, 107, 108, 110, 112, 115,
        ];
        assert!(stored.iter().all(|c| PixelFamily::of_code(*c).is_some()));
        for absent in [2u8, 3, 8, 23, 24, 25, 29, 33, 36, 100, 113, 255] {
            assert_eq!(PixelFamily::of_code(absent), None, "{absent}");
        }
        assert_eq!(PixelFamily::of_code(108), Some(PixelFamily::Bc1));
        assert_eq!(PixelFamily::of_code(9), Some(PixelFamily::B8G8R8A8));
        assert_eq!(PixelFamily::of_code(105), Some(PixelFamily::R8G8B8A8));
        assert!(PixelFamily::Bc7.is_compressed() && !PixelFamily::R8G8B8A8.is_compressed());
    }

    /// A package of one ASCII-named texture whose descriptor carries `texture_flags`; its "DDS" is a stub, since only the
    /// magic is checked.
    fn package(texture_flags: u8) -> Vec<u8> {
        let mut b = vec![0u8; 16];
        b[..4].copy_from_slice(b"TPF\0");
        b[4..8].copy_from_slice(&8u32.to_le_bytes());
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[13] = 3;
        let mut descriptor = [0u8; 20];
        descriptor[..4].copy_from_slice(&38u32.to_le_bytes());
        descriptor[4..8].copy_from_slice(&8u32.to_le_bytes());
        descriptor[8] = 105;
        descriptor[11] = texture_flags;
        descriptor[12..16].copy_from_slice(&36u32.to_le_bytes());
        b.extend(descriptor);
        b.extend(b"a\0");
        b.extend(b"DDS \0\0\0\0");
        b
    }

    #[test]
    fn the_texture_flags_byte_is_0_1_or_0x80_and_kept() {
        for flags in [0, 1, 0x80] {
            let read = Package::read(&package(flags)).expect("known flags byte");
            assert_eq!((read.textures.len(), read.textures[0].flags), (1, flags));
        }
        for flags in [2, 0x40, 0x81, 0xff] {
            let e = Package::read(&package(flags)).unwrap_err().to_string();
            assert_eq!(
                e, "TPF at 0x1b: compressed/unknown PC texture flags",
                "{flags:#x}"
            );
        }
    }
}
