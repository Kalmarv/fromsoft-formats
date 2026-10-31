//! Elden Ring environment-package identities and the MSBE box fields they accompany.
//! These describe stored data; selecting quality, states and their blend is the caller's operation.
use crate::{
    Archive, Result,
    bytes::View,
    invalid,
    msb::{Generation, MapLayout, Region},
};
use serde::{Deserialize, Serialize};

/// A member's retained name and texture header, with no DDS payload allocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemberHeader {
    /// The BND member path.
    pub member: String,
    /// The TPF texture name.
    pub texture: String,
    /// Parsed group, state and coefficient identity.
    pub identity: TextureIdentity,
    /// The TPF format byte.
    pub format: u8,
    /// The TPF texture shape.
    pub kind: crate::tpf::TextureKind,
    /// The TPF stored mipmap count.
    pub mipmaps: u8,
}

/// Metadata of one authored environment package; heavy texture bytes are not retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PackageInventory {
    /// The package's archive identity.
    pub package: Package,
    /// Member headers, in binder/TPF order.
    pub members: Vec<MemberHeader>,
    /// Sum of the decompressed binder member lengths, for bounded-memory accounting.
    pub member_bytes: usize,
}

/// Inspect one binder at a time, retaining only member names and headers. The outer DCX must be decompressed,
/// but no DDS bytes are copied or kept in the returned catalog. No tier or state is selected.
pub fn inventory(
    archive: &Archive,
    decoder: &crate::dcx::Decoder<'_>,
    packages: &[Package],
) -> Result<Vec<PackageInventory>> {
    let mut out = Vec::new();
    for package in packages {
        let binder = crate::Binder::read(&archive.read(&package.path, decoder)?, decoder)?;
        let mut members = Vec::new();
        let mut member_bytes = 0usize;
        for member in binder.files {
            member_bytes = member_bytes
                .checked_add(member.bytes.len())
                .ok_or_else(|| invalid("environment catalog", 0, "member length overflow"))?;
            for texture in crate::tpf::Package::view(&member.bytes)?.textures {
                let identity = texture_identity(&texture.name, &package.area)?;
                if identity.state != package.state {
                    return Err(invalid("environment catalog", 0, "package/member state mismatch"));
                }
                members.push(MemberHeader {
                    member: member.name.clone(),
                    texture: texture.name,
                    identity,
                    format: texture.format,
                    kind: texture.kind,
                    mipmaps: texture.mipmaps,
                });
            }
        }
        out.push(PackageInventory {
            package: package.clone(),
            members,
            member_bytes,
        });
    }
    Ok(out)
}

/// Quality word stored in an environment binder's path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    /// The `high` package word.
    High,
    /// The `middle` package word.
    Middle,
    /// The `low` package word.
    Low,
}
impl Tier {
    /// The stored lower-case path word.
    pub fn word(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Middle => "middle",
            Self::Low => "low",
        }
    }
}

/// One installed package identity; a state number is not an hour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Package {
    /// Full map identity, such as `m10_00_00_00`.
    pub area: String,
    /// The two-decimal package state number.
    pub state: u8,
    /// The stored quality tier.
    pub tier: Tier,
    /// The final two-decimal binder part number.
    pub part: u8,
    /// The complete archive path.
    pub path: String,
}

/// Enumerate the supported two-decimal state/part namespace by archive membership, without reading payloads.
/// All quality tiers are retained; no package or time state is selected.
pub fn packages(archive: &Archive, area: &str) -> Result<Vec<Package>> {
    if area.len() != 12 || !area.is_ascii() || !area.starts_with('m') {
        return Err(invalid("environment package", 0, "expected full map identity"));
    }
    let mut out = Vec::new();
    for state in 0..100 {
        for tier in [Tier::High, Tier::Middle, Tier::Low] {
            for part in 0..100 {
                let path = format!(
                    "/map/{}/{area}/{area}_envmap_{state:02}_{}_{part:02}.tpfbnd.dcx",
                    area.get(..3)
                        .ok_or_else(|| invalid("environment package", 0, "invalid map family"))?,
                    tier.word()
                );
                if archive.contains(&path) {
                    out.push(Package {
                        area: area.into(),
                        state,
                        tier,
                        part,
                        path,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// A GIIV coefficient channel or a GILM reflection cube member identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TextureIdentity {
    /// The GIIV/GILM numeric group identity.
    pub group: u32,
    /// The member's explicit package state number.
    pub state: u8,
    /// X/Y/Z/W are 0/1/2/3; a reflection cube has no coefficient channel.
    pub channel: Option<usize>,
}
/// Parse the GIIV/GILM member grammar without choosing a package state.
pub fn texture_identity(name: &str, area: &str) -> Result<TextureIdentity> {
    let stem = name.strip_suffix(".dds").unwrap_or(name);
    let (rest, volume) = if let Some(s) = stem.strip_prefix(&format!("{area}_GIIV")) {
        (s, true)
    } else if let Some(s) = stem.strip_prefix(&format!("{area}_GILM")) {
        (s, false)
    } else {
        return Err(invalid(
            "environment texture",
            0,
            format!("unexpected identity {name}"),
        ));
    };
    let fields: Vec<_> = rest.split('_').collect();
    let [group_text, state_text, channel_text] = fields.as_slice() else {
        return Err(invalid(
            "environment texture",
            0,
            "expected group_state_channel identity",
        ));
    };
    if state_text.len() != 2
        || !state_text.bytes().all(|b| b.is_ascii_digit())
        || !group_text.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid(
            "environment texture",
            0,
            "expected group_state_channel identity",
        ));
    }
    let group = group_text
        .parse()
        .map_err(|_| invalid("environment texture", 0, "invalid group"))?;
    let state = state_text
        .parse()
        .map_err(|_| invalid("environment texture", 0, "invalid state"))?;
    let channel = if volume {
        Some(match *channel_text {
            "X" => 0,
            "Y" => 1,
            "Z" => 2,
            "W" => 3,
            _ => return Err(invalid("environment texture", 0, "invalid coefficient channel")),
        })
    } else if *channel_text == "rem" {
        None
    } else {
        return Err(invalid("environment texture", 0, "invalid reflection suffix"));
    };
    Ok(TextureIdentity {
        group,
        state,
        channel,
    })
}

/// Numeric identity in an authored Env_Box or Env_Point name; no association rule is applied here.
pub fn region_identity(name: &str, prefix: &str) -> Result<u32> {
    name.strip_prefix(prefix)
        .and_then(|s| s.split('_').next())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            invalid(
                "environment region",
                0,
                format!("invalid {prefix} identity {name}"),
            )
        })
}

/// Established MSBE environment-effect-box type fields. Uninterpreted bytes remain in Region::type_data.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BoxFields {
    /// The stored fade distance at type-data +4, in map metres.
    pub fade_distance: f32,
    /// The stored priority byte at type-data +9.
    pub priority: u8,
    /// Signed value at type data +0x0a; -1 leaves the authored default identity in effect.
    pub group_override: i16,
    /// The stored reflection multiplier at type-data +0x24.
    pub specular_scale: f32,
    /// The stored diffuse multiplier at type-data +0x28.
    pub diffuse_scale: f32,
}
/// Decode established MSBE effect-box fields; other generations/types return `None`.
pub fn box_fields(layout: &MapLayout, region: &Region) -> Result<Option<BoxFields>> {
    if layout.generation.name != Generation::ELDEN_RING.name
        || region.type_name != Some("environment_map_effect_box")
    {
        return Ok(None);
    }
    let view = View {
        data: &region.type_data,
        format: "MSBE environment box",
        be: false,
        base: 0,
    };
    let float = |at| -> Result<f32> {
        let value = f32::from_bits(view.u32(at)?);
        if value.is_finite() {
            Ok(value)
        } else {
            Err(view.err(at, "non-finite box field"))
        }
    };
    Ok(Some(BoxFields {
        fade_distance: float(4)?,
        priority: view.u8(9)?,
        group_override: view.u16(0x0a)? as i16,
        specular_scale: float(0x24)?,
        diffuse_scale: float(0x28)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn msbe_environment_box_fields_preserve_signed_override_and_scales() {
        let mut data = vec![0; 0x2c];
        for (at, value) in [(4, 12.5f32), (0x24, 0.75), (0x28, 1.25)] {
            data[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        data[9] = 7;
        data[10..12].copy_from_slice(&(-1i16).to_le_bytes());
        let mut region = Region {
            index: 0,
            name: "Env_Box902".into(),
            type_code: 0,
            index_in_type: 0,
            number: 0,
            type_name: Some("environment_map_effect_box"),
            shape: crate::msb::RegionShape::Box {
                width: 2.,
                height: 4.,
                depth: 6.,
            },
            position: [0.; 3],
            rotation_degrees: [0.; 3],
            entity_id: -1,
            type_data: data,
            extra_map: None,
        };
        let layout = MapLayout {
            generation: &Generation::ELDEN_RING,
            models: Vec::new(),
            parts: Vec::new(),
            map_offsets: Vec::new(),
            sections: Vec::new(),
        };
        let fields = box_fields(&layout, &region).expect("valid fields").expect("box");
        assert_eq!(
            fields,
            BoxFields {
                fade_distance: 12.5,
                priority: 7,
                group_override: -1,
                specular_scale: 0.75,
                diffuse_scale: 1.25
            }
        );
        region.type_data[10..12].copy_from_slice(&901i16.to_le_bytes());
        assert_eq!(
            box_fields(&layout, &region)
                .expect("valid fields")
                .expect("box")
                .group_override,
            901
        );
        region.type_data.truncate(0x28);
        assert!(box_fields(&layout, &region).is_err());
    }
    #[test]
    fn environment_member_identities_keep_states_and_channels() {
        assert_eq!(
            texture_identity("m10_00_00_00_GIIV0901_06_Z", "m10_00_00_00").expect("identity"),
            TextureIdentity {
                group: 901,
                state: 6,
                channel: Some(2)
            }
        );
        assert_eq!(
            texture_identity("m10_00_00_00_GILM0002_05_rem.dds", "m10_00_00_00")
                .expect("identity")
                .channel,
            None
        );
        assert!(texture_identity("m10_00_00_00_GIIV0000_05_Q", "m10_00_00_00").is_err());
        assert!(texture_identity("m10_00_00_00_GILM0000_5_rem", "m10_00_00_00").is_err());
        assert_eq!(region_identity("Env_Box002", "Env_Box").expect("identity"), 2);
    }
}
