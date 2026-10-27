//! BTL light-bank values: versions 16 (Sekiro) and 18 (Elden Ring) share the 232-byte light record. The file stores
//! RGB bytes; their transfer function and the mapping of intensity into target watts are the caller's to choose.
use crate::{
    Result,
    bytes::{Budget, View},
    edge::{float, vector},
    invalid,
};
/// A light's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Stored type 0.
    Point,
    /// Stored type 1.
    Spot,
    /// Stored type 2.
    Directional,
}
/// One 232-byte light record, as stored.
#[derive(Debug, Clone)]
pub struct Light {
    /// The light's name.
    pub name: String,
    /// Its shape.
    pub kind: Kind,
    /// Diffuse colour, RGB bytes.
    pub diffuse: [u8; 3],
    /// Diffuse intensity, as stored (its mapping to a physical unit is the caller's).
    pub diffuse_power: f32,
    /// Specular colour, RGB bytes.
    pub specular: [u8; 3],
    /// Specular intensity, as stored.
    pub specular_power: f32,
    /// Whether the light casts shadows.
    pub cast_shadows: bool,
    /// The cone angle, as stored.
    pub cone_angle: f32,
    /// Its position, as stored.
    pub position: [f32; 3],
    /// Its rotation, as stored (three floats).
    pub rotation: [f32; 3],
    /// Its radius, as stored.
    pub radius: f32,
    /// The record's remaining 4-byte words, as stored, by byte offset in the record; no meaning is assigned to them.
    pub auxiliary: std::collections::BTreeMap<usize, u32>,
}
/// A read light bank (`.btl`).
#[derive(Debug, Clone)]
pub struct LightBank {
    /// The stored version: one of [`VERSIONS`].
    pub version: u32,
    /// Its lights, in file order.
    pub lights: Vec<Light>,
}
/// The versions whose light record this reader knows.
pub const VERSIONS: [u32; 2] = [16, 18];
impl LightBank {
    /// Read a light bank. The version, the header, the record count against the file size, each name's offset and
    /// every field the reader holds to a fixed value are checked; a non-finite float is refused.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let v = View {
            data: bytes,
            format: "BTL",
            be: false,
        };
        v.expect(0, &2u32.to_le_bytes())?;
        let version = v.u32(4)?;
        if !VERSIONS.contains(&version) {
            return Err(invalid("BTL", 4, format!("unsupported version {version}")));
        }
        let count = v.u32(8)? as usize;
        let names_length = v.u32(12)? as usize;
        v.expect(16, &[0; 4])?;
        v.expect(20, &232u32.to_le_bytes())?;
        v.expect(24, &[0; 36])?;
        let start = 60usize
            .checked_add(names_length)
            .ok_or_else(|| invalid("BTL", 12, "name size overflow"))?;
        let end = count
            .checked_mul(232)
            .and_then(|n| start.checked_add(n))
            .ok_or_else(|| invalid("BTL", 8, "record size overflow"))?;
        if end != bytes.len() {
            return Err(invalid("BTL", 8, "record count does not cover file"));
        }
        let names = View {
            data: v.slice(60, names_length)?,
            format: "BTL names",
            be: false,
        };
        let budget = Budget::new("BTL", bytes.len());
        let mut lights = Vec::with_capacity(count);
        for i in 0..count {
            let record_at = v.element(start, i, 232)?;
            let r = View {
                data: v.slice(record_at, 232)?,
                format: "BTL light",
                be: false,
            };
            let n = r.size(16)?;
            if !n.is_multiple_of(2) {
                return Err(invalid("BTL", v.at(record_at, 16)?, "unaligned UTF-16 name"));
            }
            let (name, _) = names.utf16z(n, &budget)?;
            let kind = match r.u32(24)? {
                0 => Kind::Point,
                1 => Kind::Spot,
                2 => Kind::Directional,
                _ => return Err(invalid("BTL", v.at(record_at, 24)?, "unknown light type")),
            };
            r.boolean(28)?;
            for offset in [96, 140, 148, 168, 176, 180] {
                r.expect(offset, &[0; 4])?;
            }
            let diffuse = r.slice(29, 3)?.try_into().expect("three bytes");
            let specular = r.slice(36, 3)?.try_into().expect("three bytes");
            let mut auxiliary = std::collections::BTreeMap::new();
            for offset in (0..232).step_by(4) {
                if !matches!(
                    offset,
                    16 | 20 | 24 | 28 | 32 | 36 | 40 | 44 | 56 | 60 | 64 | 68 | 72 | 76 | 88
                ) {
                    auxiliary.insert(offset, r.u32(offset)?);
                }
            }
            lights.push(Light {
                name,
                kind,
                diffuse,
                diffuse_power: float(&r, 32)?,
                specular,
                specular_power: float(&r, 40)?,
                cast_shadows: r.boolean(39)?,
                cone_angle: float(&r, 44)?,
                position: vector(&r, 56)?,
                rotation: vector(&r, 68)?,
                radius: float(&r, 88)?,
                auxiliary,
            });
        }
        Ok(Self { version, lights })
    }
}

#[cfg(test)]
mod tests {
    use super::LightBank;

    /// A hand-made bank with no lights: the 60-byte header, version 16 or 18 read, any other refused.
    #[test]
    fn versions() {
        let bank = |version: u32| {
            let mut b = vec![0u8; 60];
            b[0..4].copy_from_slice(&2u32.to_le_bytes());
            b[4..8].copy_from_slice(&version.to_le_bytes());
            b[20..24].copy_from_slice(&232u32.to_le_bytes());
            b
        };
        for version in [16, 18] {
            let read = LightBank::read(&bank(version)).expect("known version");
            assert_eq!((read.version, read.lights.len()), (version, 0));
        }
        for version in [15, 17] {
            assert!(LightBank::read(&bank(version)).is_err());
        }
    }
}
