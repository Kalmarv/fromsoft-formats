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
    /// The light's name: UTF-16 text in the name table, found through the record's length at record offset 16.
    pub name: String,
    /// Its shape, from the `u32` type at record offset 24.
    pub kind: Kind,
    /// Diffuse colour: the three bytes at record offset 29 (red, green, blue; no transfer function applied).
    pub diffuse: [u8; 3],
    /// Diffuse intensity: the `f32` at record offset 32, as stored (its mapping to a physical unit is the caller's).
    pub diffuse_power: f32,
    /// Specular colour: the three bytes at record offset 36 (red, green, blue).
    pub specular: [u8; 3],
    /// Specular intensity: the `f32` at record offset 40, as stored.
    pub specular_power: f32,
    /// Whether the light casts shadows: the byte at record offset 39 (0 or 1; any other value is refused).
    pub cast_shadows: bool,
    /// The cone angle: the `f32` at record offset 44, as stored (unit not established; meaningful for spot lights).
    pub cone_angle: f32,
    /// Its position: three `f32` at record offset 56, as stored.
    pub position: [f32; 3],
    /// Its rotation: three `f32` at record offset 68, as stored (angle unit and order are not established).
    pub rotation: [f32; 3],
    /// Its radius: the `f32` at record offset 88, as stored.
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
            base: 0,
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
        let names = v.sub(60, names_length, "BTL names")?;
        let budget = Budget::new("BTL", bytes.len());
        let mut lights = Vec::with_capacity(count);
        for i in 0..count {
            let record_at = v.element(start, i, 232)?;
            let r = v.sub(record_at, 232, "BTL light")?;
            let n = r.size(16)?;
            if !n.is_multiple_of(2) {
                return Err(r.err(16, "unaligned UTF-16 name"));
            }
            let (name, _) = names.utf16z(n, &budget)?;
            let kind = match r.u32(24)? {
                0 => Kind::Point,
                1 => Kind::Spot,
                2 => Kind::Directional,
                _ => return Err(r.err(24, "unknown light type")),
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

    /// A bank of `count` lights, each valid, with the light `bad` carrying `value` in its type word.
    fn bank_with_bad_type(count: usize, bad: usize, value: u32) -> Vec<u8> {
        let mut b = vec![0u8; 60];
        b[0..4].copy_from_slice(&2u32.to_le_bytes());
        b[4..8].copy_from_slice(&16u32.to_le_bytes());
        b[8..12].copy_from_slice(&(count as u32).to_le_bytes());
        b[12..16].copy_from_slice(&4u32.to_le_bytes());
        b[20..24].copy_from_slice(&232u32.to_le_bytes());
        b.extend([0u8; 4]);
        for i in 0..count {
            let mut light = vec![0u8; 232];
            let kind = if i == bad { value } else { 1 };
            light[24..28].copy_from_slice(&kind.to_le_bytes());
            b.extend(light);
        }
        b
    }

    /// A refusal inside a light names the field's position in the file, not in the 232-byte record.
    #[test]
    fn a_hostile_light_deep_in_the_file_is_refused_at_its_file_offset() {
        let ok = bank_with_bad_type(5, 9, 0);
        assert_eq!(LightBank::read(&ok).unwrap().lights.len(), 5);
        let bad = bank_with_bad_type(5, 3, 9);
        let e = LightBank::read(&bad).unwrap_err();
        let record = 64 + 3 * 232;
        assert_eq!(
            e.to_string(),
            format!("BTL light at {:#x}: unknown light type", record + 24)
        );
        assert_eq!(bad.get(record + 24), Some(&9));
    }
}
