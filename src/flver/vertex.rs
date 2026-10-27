use super::Member;
use crate::{Result, bytes::View, invalid};
use half::f16;
use primase_foundations::vertex::Format;

/// Decoded attributes in file coordinates. Repeated attributes retain stream order.
#[derive(Clone, Debug, Default)]
pub struct Vertex {
    /// The position (semantic 0), in the file's coordinates.
    pub position: [f32; 3],
    /// The normal (semantic 3).
    pub normal: [f32; 3],
    /// The normal's fourth component as stored; for a static mesh it names the bone the vertex follows.
    pub normal_w: i32,
    /// The bone weights (semantic 1).
    pub bone_weights: [f32; 4],
    /// The bone indices (semantic 2): into the mesh's palette ([`super::Mesh::palette`]) where it has one, else bones.
    pub bone_indices: [u16; 4],
    /// The UV members in stream order, each as one four-lane attribute the way the game's vertex program receives it
    /// (`TEXCOORDn.xyzw`; a lane its format has no value for is zero). A `Short2`, `Byte4A`, `Byte4B` or `Short2toFloat2`
    /// member is two int16 lanes over the UV factor (2048) and a `Float2` or `Float3` member its floats; a `Float4` or
    /// `UVPair` member gives two such attributes. A `UByte4Norm` (`0x13`) member is four normalised unsigned bytes
    /// (`b / 255`), which the game's programs read as a float `TEXCOORDn` and pass on unscaled.
    ///
    /// A `Half4` (`0x2e`) member is not a texture coordinate: it is a point in the model's space, xyz as IEEE half
    /// floats and w zero, the pivot vegetation sways about. The game's programs read it as a float `TEXCOORDn xyzw` and
    /// transform it like a position, so it is decoded as half floats, not as int16 over the UV factor, and its lanes
    /// are coordinates in metres, outside [0, 1]. Only meshes of pivot materials (and grass and tree assets) store one;
    /// a SpeedTree mesh's member is in [`Vertex::speedtree`] instead. Members are kept in stream order because the
    /// programs number their `TEXCOORDn` inputs by that order.
    pub uvs: Vec<[f32; 4]>,
    /// The four stored bytes of each `UByte4Norm` UV member and the index in [`Vertex::uvs`] of its attribute, for layer
    /// weights (the same values as that attribute's lanes before the `/ 255`).
    pub layer_bytes: Vec<(usize, [u8; 4])>,
    /// The tangents (semantic 6), in stream order.
    pub tangents: Vec<[f32; 4]>,
    /// Source semantic-6 member indices, parallel to tangents. Sparse indices stay sparse.
    pub tangent_indices: Vec<u8>,
    /// The bitangent (semantic 7), where the layout stores one.
    pub bitangent: [f32; 4],
    /// The vertex colours (semantic 10), in stream order.
    pub colors: Vec<[f32; 4]>,
    /// A SpeedTree vertex's stored members in layout order, each as its format gives it to a vertex program (four
    /// lanes, unused ones zero): half floats as floats, normalized unsigned bytes as `x / 255`, floats as stored.
    /// What each member stands for is the game's shader's, not the layout's semantic: the position, normal, one
    /// tangent and one UV set are set from them ([`super::speedtree_layout`]), a billboard card's are not.
    pub speedtree: Vec<[f32; 4]>,
}

pub(super) fn size(kind: u32) -> Result<usize> {
    Ok(match kind {
        0x10..=0x13 | 0x15 | 0x2d | 0x2f => 4,
        1 | 0x16 | 0x18 | 0x1a | 0x2e => 8,
        2 => 12,
        3 => 16,
        _ => return Err(invalid("FLVER", 0, format!("unsupported element {kind:#x}"))),
    })
}

/// `w` as an integer, when it is one that fits an `i32`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "the value is checked integral and within the i32 range before it is converted"
)]
fn exact_i32(w: f32) -> Option<i32> {
    (w.fract() == 0. && (-2_147_483_648.0..2_147_483_648.0).contains(&w)).then_some(w as i32)
}

impl Vertex {
    /// A SpeedTree vertex's stored members ([`Vertex::speedtree`]): `Float4` (3), `Float2` (1), `Half4` (0x2e),
    /// `Half2` (0x2d) and `UByte4Norm` (0x13), the formats Elden Ring's SpeedTree layouts store; others are refused.
    pub(super) fn read_speedtree(&mut self, data: &[u8], base: usize, members: &[Member]) -> Result<()> {
        let v = View {
            data,
            format: "FLVER SpeedTree vertex",
            be: false,
            base,
        };
        let mut at = 0;
        for m in members.iter().filter(|m| m.stored) {
            let n = size(m.kind)?;
            let raw = v.slice(at, n)?;
            let mut lanes = [0f32; 4];
            match m.kind {
                1 | 3 => {
                    for (k, lane) in lanes.iter_mut().take(n / 4).enumerate() {
                        *lane = f32::from_bits(v.u32(v.element(at, k, 4)?)?);
                    }
                }
                0x2d | 0x2e => {
                    for (k, lane) in lanes.iter_mut().take(n / 2).enumerate() {
                        *lane = f16::from_bits(v.u16(v.element(at, k, 2)?)?).to_f32();
                    }
                }
                0x13 => {
                    for (lane, &b) in lanes.iter_mut().zip(raw) {
                        *lane = f32::from(b) / 255.;
                    }
                }
                other => {
                    return Err(v.err(at, format!("SpeedTree member type {other:#x}")));
                }
            }
            if lanes.iter().any(|x| !x.is_finite()) {
                return Err(v.err(at, "non-finite SpeedTree member"));
            }
            self.speedtree.push(lanes);
            at = v.at(at, n)?;
        }
        Ok(())
    }

    /// The stored SpeedTree member `index`.
    fn speedtree_member(&self, index: usize) -> Result<[f32; 4]> {
        self.speedtree.get(index).copied().ok_or_else(|| {
            invalid(
                "FLVER SpeedTree vertex",
                index,
                format!(
                    "the layout names stored member {index}, the vertex stores {}",
                    self.speedtree.len()
                ),
            )
        })
    }

    /// A SpeedTree vertex's attributes from its stored members (indices into [`Vertex::speedtree`], the roles
    /// [`super::SpeedTreeLayout::Geometry`] names): the position and UV0 as stored, and the normal and tangent
    /// ([`Vertex::speedtree_frame`]).
    pub(super) fn speedtree_attributes(
        &mut self,
        position: usize,
        uv: usize,
        normal: usize,
        tangent: usize,
    ) -> Result<()> {
        let [px, py, pz, _] = self.speedtree_member(position)?;
        let [tu, tv, ..] = self.speedtree_member(uv)?;
        self.position = [px, py, pz];
        self.uvs = vec![[tu, tv, 0., 0.]];
        self.speedtree_frame(normal, tangent)
    }

    /// A SpeedTree vertex's normal and tangent from the stored members of indices `normal` and `tangent`, unpacked
    /// as its programs unpack them (`x * 2 - 1`), the tangent in the ordinary programs' convention. A SpeedTree pixel
    /// program bends the normal by its normal map's `x` along the stored tangent `T` and its `y` along `-w (N × T)`,
    /// `w` the 1 its vertex program passes; an ordinary one bends it by `x` along `w (N × T')` and `y` along `T'`.
    /// `T' = T × N` with `w` 1 is the same frame where `T` is perpendicular to `N` (the stored pairs are within a
    /// byte's precision of it).
    pub(super) fn speedtree_frame(&mut self, normal: usize, tangent: usize) -> Result<()> {
        let unpack = |[x, y, z, _]: [f32; 4]| [x * 2. - 1., y * 2. - 1., z * 2. - 1.];
        let n = unpack(self.speedtree_member(normal)?);
        let s = unpack(self.speedtree_member(tangent)?);
        let ([nx, ny, nz], [sx, sy, sz]) = (n, s);
        self.normal = n;
        self.tangents = vec![[sy * nz - sz * ny, sz * nx - sx * nz, sx * ny - sy * nx, 1.]];
        self.tangent_indices = vec![0];
        Ok(())
    }

    fn push_tangent(&mut self, v: &View<'_>, at: usize, index: u32, tangent: [f32; 4]) -> Result<()> {
        let index = u8::try_from(index)
            .map_err(|_| v.err(at, format!("tangent semantic index {index} exceeds 255")))?;
        if self.tangent_indices.contains(&index) {
            return Err(v.err(at, format!("duplicate tangent semantic index {index}")));
        }
        self.tangent_indices.push(index);
        self.tangents.push(tangent);
        Ok(())
    }

    pub(super) fn read(
        &mut self,
        data: &[u8],
        base: usize,
        members: &[Member],
        uv_factor: f32,
    ) -> Result<()> {
        let v = View {
            data,
            format: "FLVER vertex",
            be: false,
            base,
        };
        for m in members {
            let o = m.offset;
            let raw = v.slice(o, size(m.kind)?)?;
            let decode = |f: Format| f.decode(raw).map_err(|e| v.err(o, e));
            let f = |i: usize| -> Result<f32> {
                let x = f32::from_bits(v.u32(v.element(o, i, 4)?)?);
                if !x.is_finite() {
                    return Err(v.err(o, "non-finite attribute"));
                }
                Ok(x)
            };
            let short = |i: usize| -> Result<f32> { Ok(f32::from(v.u16(v.element(o, i, 2)?)? as i16)) };
            // the element's first four bytes
            let quad = || -> Result<[u8; 4]> {
                raw.first_chunk::<4>()
                    .copied()
                    .ok_or_else(|| v.err(o, "element shorter than four bytes"))
            };
            let byte_norm = || -> Result<[f32; 4]> {
                Ok(quad()?.map(|b| (f32::from(b) - 127.) * f32::from_bits(0x3c01_0204)))
            };
            let short_norm = || -> Result<[f32; 4]> {
                Ok([
                    short(0)? / 32767.,
                    short(1)? / 32767.,
                    short(2)? / 32767.,
                    short(3)? / 32767.,
                ])
            };
            let half = |i: usize| -> Result<f32> { Ok(f16::from_bits(v.u16(v.element(o, i, 2)?)?).to_f32()) };
            match (m.semantic, m.kind) {
                (0, 2 | 3) => {
                    self.position = [f(0)?, f(1)?, f(2)?];
                    if m.kind == 3 && f(3)? != 0. {
                        return Err(v.err(o, "position W is not zero"));
                    }
                }
                (1, 0x10) => self.bone_weights = quad()?.map(|b| f32::from(b as i8) / 127.),
                (1, 0x13) => self.bone_weights = decode(Format::Unorm8x4)?,
                (1, 0x16 | 0x1a) => self.bone_weights = short_norm()?,
                (2, 0x11 | 0x2f) => self.bone_indices = quad()?.map(u16::from),
                (2, 0x18) => {
                    self.bone_indices = [
                        v.u16(o)?,
                        v.u16(v.at(o, 2)?)?,
                        v.u16(v.at(o, 4)?)?,
                        v.u16(v.at(o, 6)?)?,
                    ]
                }
                (3, 2 | 3) => {
                    self.normal = [f(0)?, f(1)?, f(2)?];
                    if m.kind == 3 {
                        self.normal_w = exact_i32(f(3)?).ok_or_else(|| v.err(o, "fractional normal W"))?;
                    }
                }
                (3, 0x10 | 0x11 | 0x13 | 0x2f) => {
                    let [x, y, z, _] = byte_norm()?;
                    let [_, _, _, w] = quad()?;
                    self.normal = [x, y, z];
                    self.normal_w = i32::from(w);
                }
                (3, 0x12) => {
                    let [w, z, y, x] = quad()?;
                    self.normal = [x, y, z].map(|b| f32::from(b as i8) / 127.);
                    self.normal_w = i32::from(w);
                }
                (3, 0x1a | 0x2e) => {
                    let component = |i: usize| -> Result<f32> {
                        if m.kind == 0x1a {
                            Ok(short(i)? / 32767.)
                        } else {
                            Ok((f32::from(v.u16(v.element(o, i, 2)?)?) - 32767.) / 32767.)
                        }
                    };
                    self.normal = [component(0)?, component(1)?, component(2)?];
                    self.normal_w = i32::from(v.u16(v.at(o, 6)?)? as i16);
                }
                (5, 1) => self.uvs.push([f(0)?, f(1)?, 0., 0.]),
                (5, 2) => self.uvs.push([f(0)?, f(1)?, f(2)?, 0.]),
                (5, 3) => {
                    self.uvs.push([f(0)?, f(1)?, 0., 0.]);
                    self.uvs.push([f(2)?, f(3)?, 0., 0.]);
                }
                (5, 0x10..=0x12 | 0x15 | 0x16) => {
                    self.uvs
                        .push([short(0)? / uv_factor, short(1)? / uv_factor, 0., 0.]);
                    if m.kind == 0x16 {
                        self.uvs
                            .push([short(2)? / uv_factor, short(3)? / uv_factor, 0., 0.]);
                    }
                }
                (5, 0x13) => {
                    let bytes = quad()?;
                    self.layer_bytes.push((self.uvs.len(), bytes));
                    self.uvs.push(bytes.map(|b| f32::from(b) / 255.));
                }
                (5, 0x2e) => {
                    let lanes = [half(0)?, half(1)?, half(2)?, half(3)?];
                    if lanes.iter().any(|x: &f32| !x.is_finite()) {
                        return Err(v.err(o, "non-finite half-float UV member"));
                    }
                    self.uvs.push(lanes);
                }
                (6, 3) => self.push_tangent(&v, o, m.index, decode(Format::Float4)?)?,
                (6, 0x10 | 0x11 | 0x13 | 0x2f) => self.push_tangent(&v, o, m.index, byte_norm()?)?,
                (6, 0x1a) => self.push_tangent(&v, o, m.index, short_norm()?)?,
                (7, 0x10 | 0x11 | 0x13 | 0x2f) => self.bitangent = byte_norm()?,
                (10, 3) => self.colors.push(decode(Format::Float4)?),
                (10, 0x10 | 0x13) => self.colors.push(decode(Format::Unorm8x4)?),
                _ => {
                    return Err(v.err(
                        o,
                        format!("unsupported semantic {} / type {:#x}", m.semantic, m.kind),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tangent_tests {
    use super::*;

    #[test]
    fn tangent_members_retain_the_layouts_sparse_semantic_indices() {
        let members: Vec<_> = [0, 1, 4]
            .into_iter()
            .enumerate()
            .map(|(i, index)| Member {
                unknown: 0,
                stored: true,
                offset: i * 16,
                kind: 3,
                semantic: 6,
                index,
            })
            .collect();
        let tangents = [
            [1.0f32, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, 0.0, -1.0, -1.0],
        ];
        let bytes: Vec<_> = tangents.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        let mut vertex = Vertex::default();
        vertex.read(&bytes, 0, &members, 2048.0).unwrap();
        assert_eq!(vertex.tangents, tangents);
        assert_eq!(vertex.tangent_indices, [0, 1, 4]);
    }

    /// A refusal inside a vertex names the member's position in the file: the vertex's offset plus the member's.
    #[test]
    fn a_refused_member_is_named_at_its_file_offset() {
        let members = [Member {
            unknown: 0,
            stored: true,
            offset: 4,
            kind: 3,
            semantic: 99,
            index: 0,
        }];
        let bytes = [0u8; 20];
        let e = Vertex::default()
            .read(&bytes, 1000, &members, 2048.0)
            .unwrap_err();
        assert!(e.to_string().starts_with("FLVER vertex at 0x3ec:"), "{e}");
        let tangents = [Member {
            semantic: 6,
            index: 300,
            ..members[0]
        }];
        let e = Vertex::default()
            .read(&bytes, 1000, &tangents, 2048.0)
            .unwrap_err();
        assert!(e.to_string().starts_with("FLVER vertex at 0x3ec:"), "{e}");
    }
}
