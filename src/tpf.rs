//! TPF texture packages (PC): each texture's name and flags with its DDS file, retained exactly as stored, all faces
//! and mipmaps included. Pixels are not decoded.
use crate::{
    Result,
    bytes::{Budget, View, coverage},
    invalid,
};
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
    /// The texture's flags as stored (0 or 1).
    pub flags: u8,
    /// Its optional extra data.
    pub extra: Option<Extra>,
    /// The DDS file, complete, starting with `DDS `.
    pub dds: Vec<u8>,
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
            if texture_flags > 1 {
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
