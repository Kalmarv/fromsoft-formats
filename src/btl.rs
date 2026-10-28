//! BTL light-bank values: versions 16 (Sekiro) and 18 (Elden Ring) share the 232-byte light record; version 15 (two
//! Elden Ring banks) has a 240-byte record that reads as version 18 up to byte 232. The file stores RGB bytes; their
//! transfer function and the mapping of intensity into target watts are the caller's to choose.
//!
//! Eleven further words carry names: the community's editor names them (flicker, event id, near clip, sharpness, width,
//! start cutoff, enable distance, volume density, the shadow cull flip) and a running Sekiro showed what four of them are
//! (the horizontal cone angle, the spot's near plane, the sharpness divisor and the exposure weight). A field's
//! documentation says which of the two its name rests on; the others' values fit their names in every light of both
//! installs (`tests/btl_census.rs`) and no more.
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
/// One light record (232 bytes in versions 16 and 18, 240 in version 15), as stored.
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
    /// The cone angle: the `f32` at record offset 44. Read back from a running Sekiro, a spot light's is its full vertical
    /// angle in degrees (the lighting shader's scale for the cone's vertical axis is `cot(angle / 2)`, to 1.5e-7 on five
    /// lights); [`Light::horizontal_angle`] is the other axis's.
    pub cone_angle: f32,
    /// Its position: three `f32` at record offset 56, as stored.
    pub position: [f32; 3],
    /// Its rotation: three `f32` at record offset 68, as stored (angle unit and order are not established).
    pub rotation: [f32; 3],
    /// Its radius: the `f32` at record offset 88, as stored.
    pub radius: f32,
    /// The flicker interval (shortest, longest) in seconds: the `f32` words at record offsets 116 and 120 (the community
    /// editor's names; `[0, 0]` on all of Elden Ring's lights, `[0.2, 0.8]` or `[0.2, 1]` on 89 of Sekiro's 735 and
    /// `[0, 0]` on the rest).
    pub flicker_interval: [f32; 2],
    /// The factor the light's brightness takes while it flickers: the `f32` at record offset 124 (1 on all of Elden Ring's
    /// lights and 646 of Sekiro's, 0.6 on the other 89).
    pub flicker_brightness: f32,
    /// The map event the light is tied to: the `i32` at record offset 128 (the community editor says only Dark Souls II
    /// uses it; -1 on every light of both installs).
    pub event_id: i32,
    /// The `f32` at record offset 156, the community editor's "near clip" (1 on every light of both installs). The
    /// near plane of a spot light is [`Light::near_distance`], not this.
    pub near_clip: f32,
    /// The byte at record offset 163, the community editor's "shadow model cull flip" (0 or 1; any other value is refused).
    pub shadow_model_cull_flip: bool,
    /// The `f32` at record offset 164, the community editor's "sharpness". Read back from a running Sekiro, the light's
    /// diffuse record carries `1 / sharpness` as its fourth value, which the shader's wrapped-diffuse term and its
    /// distance term take (to 2e-7 on five lights).
    pub sharpness: f32,
    /// A spot light's full horizontal cone angle in degrees: the `f32` at record offset 184 (the community editor's
    /// "width"). Read back from a running Sekiro as the horizontal axis's `cot(angle / 2)`; the loader uses
    /// [`Light::cone_angle`] for both axes where this is 0 (code read, not run). 0 on 668 of Sekiro's lights and 11,007
    /// of Elden Ring's.
    pub horizontal_angle: f32,
    /// A spot light's near plane: the `f32` at record offset 188 (the community editor's "light start cutoff"). Read back
    /// from a running Sekiro as the depth row's `far / (far - near)` of the spot record (to 1e-7 on five lights).
    pub near_distance: f32,
    /// The distance at which the light fades in, 0 for always: the `f32` at record offset 196 (the community editor's
    /// "enable dist"; 0 to 1,000 in the installs, 40 on 4,928 of Elden Ring's lights).
    pub enable_distance: f32,
    /// The community editor's "volume density": the `f32` at record offset 208 (0 to 10 in the installs; 0 on 8,009 of
    /// Elden Ring's lights, 1 on 2,221).
    pub volume_density: f32,
    /// The weight of the exposure term: the `f32` at record offset 224 (the community editor leaves it unnamed). The
    /// renderer's light packer multiplies the light's colours by `2^(-weight * exposure)` where it is not 0 (read back from
    /// a running Sekiro: a light of weight 1 reads 1.081215 times its colour, one of weight 0 exactly 1; code read in both
    /// games). 1 on 20 of Sekiro's lights, 0 on every Elden Ring light.
    pub exposure_weight: f32,
    /// The record's remaining 4-byte words, as stored, by byte offset in the record (version 15 has two more, at 232 and 236,
    /// zero in every shipped light); no meaning is assigned to them. The word at 160 holds three unnamed bytes and the
    /// byte [`Light::shadow_model_cull_flip`] is the last of.
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
pub const VERSIONS: [u32; 3] = [15, 16, 18];
/// The light record size a version stores in the header at offset 20: 240 for version 15, 232 for the others.
const fn record_size(version: u32) -> u32 {
    if version == 15 { 240 } else { 232 }
}
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
        let stored_size = record_size(version);
        v.expect(20, &stored_size.to_le_bytes())?;
        let size = stored_size as usize;
        v.expect(24, &[0; 36])?;
        let start = 60usize
            .checked_add(names_length)
            .ok_or_else(|| invalid("BTL", 12, "name size overflow"))?;
        let end = count
            .checked_mul(size)
            .and_then(|n| start.checked_add(n))
            .ok_or_else(|| invalid("BTL", 8, "record size overflow"))?;
        if end != bytes.len() {
            return Err(invalid("BTL", 8, "record count does not cover file"));
        }
        let names = v.sub(60, names_length, "BTL names")?;
        let budget = Budget::new("BTL", bytes.len());
        let mut lights = Vec::with_capacity(count);
        for i in 0..count {
            let record_at = v.element(start, i, size)?;
            let r = v.sub(record_at, size, "BTL light")?;
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
            for offset in (0..size).step_by(4) {
                if !matches!(
                    offset,
                    16 | 20
                        | 24
                        | 28
                        | 32
                        | 36
                        | 40
                        | 44
                        | 56
                        | 60
                        | 64
                        | 68
                        | 72
                        | 76
                        | 88
                        | 116
                        | 120
                        | 124
                        | 128
                        | 156
                        | 164
                        | 184
                        | 188
                        | 196
                        | 208
                        | 224
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
                flicker_interval: [float(&r, 116)?, float(&r, 120)?],
                flicker_brightness: float(&r, 124)?,
                event_id: r.u32(128)? as i32,
                near_clip: float(&r, 156)?,
                shadow_model_cull_flip: r.boolean(163)?,
                sharpness: float(&r, 164)?,
                horizontal_angle: float(&r, 184)?,
                near_distance: float(&r, 188)?,
                enable_distance: float(&r, 196)?,
                volume_density: float(&r, 208)?,
                exposure_weight: float(&r, 224)?,
                auxiliary,
            });
        }
        Ok(Self { version, lights })
    }
}

#[cfg(test)]
mod tests {
    use super::LightBank;

    /// A hand-made bank with no lights: the 60-byte header, version 15, 16 or 18 read with the record size each stores,
    /// any other version, or a version with the other family's record size, refused.
    #[test]
    fn versions() {
        let bank = |version: u32, size: u32| {
            let mut b = vec![0u8; 60];
            b[0..4].copy_from_slice(&2u32.to_le_bytes());
            b[4..8].copy_from_slice(&version.to_le_bytes());
            b[20..24].copy_from_slice(&size.to_le_bytes());
            b
        };
        for (version, size) in [(15, 240), (16, 232), (18, 232)] {
            let read = LightBank::read(&bank(version, size)).expect("known version");
            assert_eq!((read.version, read.lights.len()), (version, 0));
        }
        for (version, size) in [(15, 232), (16, 240), (18, 240), (17, 232), (17, 240)] {
            assert!(
                LightBank::read(&bank(version, size)).is_err(),
                "{version} with {size}"
            );
        }
    }

    /// A version 15 bank holds 240-byte records: the record count is measured with 240, a 232-byte stride does not
    /// cover the file, and the two words past byte 232 are read as auxiliary words.
    #[test]
    fn version_15_records_are_240_bytes() {
        let bank = |count: usize, record: usize| {
            let mut b = vec![0u8; 60];
            b[0..4].copy_from_slice(&2u32.to_le_bytes());
            b[4..8].copy_from_slice(&15u32.to_le_bytes());
            b[8..12].copy_from_slice(&(count as u32).to_le_bytes());
            b[12..16].copy_from_slice(&4u32.to_le_bytes());
            b[20..24].copy_from_slice(&240u32.to_le_bytes());
            b.extend([0u8; 4]);
            for i in 0..count {
                let mut light = vec![0u8; 240];
                light[16..20].copy_from_slice(&2u32.to_le_bytes());
                light[232..240].copy_from_slice(&[i as u8 + 1, 0, 0, 0, 7, 0, 0, 0]);
                light.truncate(record);
                b.extend(light);
            }
            b
        };
        let read = LightBank::read(&bank(2, 240)).expect("two 240-byte lights");
        assert_eq!((read.version, read.lights.len()), (15, 2));
        let aux = |i: usize| (read.lights[i].auxiliary[&232], read.lights[i].auxiliary[&236]);
        assert_eq!((aux(0), aux(1)), ((1, 7), (2, 7)));
        assert!(read.lights[0].auxiliary.keys().all(|&k| k < 240));
        assert!(LightBank::read(&bank(2, 232)).is_err());
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

#[cfg(test)]
mod named_words {
    use super::LightBank;

    /// A one-light bank of `version` 16 with `record` as the light.
    fn bank(record: &[u8]) -> Vec<u8> {
        let mut b = vec![0u8; 60];
        b[0..4].copy_from_slice(&2u32.to_le_bytes());
        b[4..8].copy_from_slice(&16u32.to_le_bytes());
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[12..16].copy_from_slice(&4u32.to_le_bytes());
        b[20..24].copy_from_slice(&232u32.to_le_bytes());
        b.extend([0u8; 4]);
        b.extend(record);
        b
    }

    fn put(record: &mut [u8], at: usize, word: u32) {
        record[at..at + 4].copy_from_slice(&word.to_le_bytes());
    }

    /// The words the community names are read under their names, with their units' values, and leave the auxiliary
    /// words; the words nobody names stay there.
    #[test]
    fn the_named_words_are_read_and_are_not_auxiliary() {
        let mut r = vec![0u8; 232];
        for (at, v) in [
            (116, 0.2f32),
            (120, 0.8),
            (124, 0.6),
            (156, 1.0),
            (164, 3.0),
            (184, 60.0),
            (188, 2.0),
            (196, 40.0),
            (208, 1.5),
            (224, 1.0),
            (152, 1.0),
        ] {
            put(&mut r, at, v.to_bits());
        }
        put(&mut r, 128, (-1i32) as u32);
        r[163] = 1;
        let light = LightBank::read(&bank(&r)).expect("a valid bank").lights.remove(0);
        assert_eq!(light.flicker_interval, [0.2, 0.8]);
        assert_eq!(
            (light.flicker_brightness, light.event_id, light.near_clip),
            (0.6, -1, 1.0)
        );
        assert!(light.shadow_model_cull_flip);
        assert_eq!(
            (light.sharpness, light.horizontal_angle, light.near_distance),
            (3.0, 60.0, 2.0)
        );
        assert_eq!(
            (light.enable_distance, light.volume_density, light.exposure_weight),
            (40.0, 1.5, 1.0)
        );
        for named in [116, 120, 124, 128, 156, 164, 184, 188, 196, 208, 224] {
            assert!(!light.auxiliary.contains_key(&named), "{named}");
        }
        // the word at 152 and the word holding the cull flip's byte are not named
        assert_eq!(light.auxiliary.get(&152), Some(&1.0f32.to_bits()));
        assert_eq!(light.auxiliary.get(&160), Some(&0x0100_0000));
    }

    /// A cull flip that is neither 0 nor 1 and a named float that is not finite are refused at their offsets.
    #[test]
    fn a_bad_named_word_is_refused() {
        let mut r = vec![0u8; 232];
        r[163] = 2;
        let e = LightBank::read(&bank(&r)).unwrap_err().to_string();
        assert!(
            e.ends_with("invalid boolean") && e.contains(&format!("{:#x}", 64 + 163)),
            "{e}"
        );
        let mut r = vec![0u8; 232];
        put(&mut r, 184, f32::INFINITY.to_bits());
        let e = LightBank::read(&bank(&r)).unwrap_err().to_string();
        assert!(
            e.ends_with("nonfinite float") && e.contains(&format!("{:#x}", 64 + 184)),
            "{e}"
        );
    }
}
