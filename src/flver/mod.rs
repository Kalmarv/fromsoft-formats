//! FLVER2 models (versions `0x20013`, `0x20014`, `0x20016` and `0x2001a`: Sekiro's and Elden Ring's): bones, dummy
//! polygons, materials with their texture bindings, vertex layouts and the meshes' decoded vertices and face sets,
//! kept in the file's coordinates. Edge-compressed indices and the legacy text encoding are refused.
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation
    )
)]
mod vertex;
use crate::{
    Result,
    bytes::{Budget, View},
    invalid,
};
pub use vertex::Vertex;

/// One member of a vertex layout: an attribute a vertex stores, and where.
#[derive(Clone, Debug)]
pub struct Member {
    /// The member's first word as stored: 0 in an ordinary layout; in a SpeedTree layout its stream (low half) and a
    /// modifier (high half: 0x8000 for a member that is not stored, see `stored`).
    pub unknown: u32,
    /// Whether the mesh's vertex buffer stores the member. A SpeedTree layout also declares members of a stream the
    /// file does not hold (modifier 0x8000: the per-instance position and two basis vectors the game supplies at draw
    /// time); they are listed, never read.
    pub stored: bool,
    /// The member's offset in its stream (the vertex buffer for a stored member).
    pub offset: usize,
    /// The storage format code (`2` Float3, `3` Float4, `0x13` UByte4Norm, `0x2e` Half4, ...).
    pub kind: u32,
    /// What the member is (0 position, 1 bone weights, 2 bone indices, 3 normal, 5 UV, 6 tangent, 7 bitangent, 10
    /// colour).
    pub semantic: u32,
    /// Which member of that semantic this is (UV set, tangent number).
    pub index: u32,
}
/// A texture binding of a material.
#[derive(Clone, Debug)]
pub struct Texture {
    /// The texture's source path as stored.
    pub path: String,
    /// The slot it binds, as the material names it.
    pub kind: String,
    /// Its UV scale.
    pub scale: [f32; 2],
    /// How the texture repeats along u and v (`TilingTypeU`, `TilingTypeV`).
    pub tiling: [Tiling; 2],
    /// The three floats after the tiling types, whose meaning is not known.
    pub unknown: [f32; 3],
}
impl Default for Texture {
    fn default() -> Self {
        Texture {
            path: String::new(),
            kind: String::new(),
            scale: [1.; 2],
            tiling: [Tiling::None; 2],
            unknown: [0.; 3],
        }
    }
}
/// A texture's tiling type along one axis, as the FLVER texture record stores it (SoulsFormatsNEXT's `TilingType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub enum Tiling {
    /// None stated.
    #[default]
    None,
    /// Repeat.
    Repeat,
    /// Mirrored repeat.
    MirrorRepeat,
    /// Clamp to the edge.
    Clamp,
    /// Border colour.
    Border,
    /// Mirror once.
    MirrorOnce,
}
impl Tiling {
    fn read(byte: u8) -> Option<Tiling> {
        Some(match byte {
            0 => Tiling::None,
            1 => Tiling::Repeat,
            2 => Tiling::MirrorRepeat,
            3 => Tiling::Clamp,
            4 => Tiling::Border,
            5 => Tiling::MirrorOnce,
            _ => return None,
        })
    }
}
/// Raw, uninterpreted material extension record; payload bytes retain source order.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GxItem {
    /// The four-byte item ID.
    pub id: [u8; 4],
    /// The item's stored version.
    pub version: u32,
    /// The item's payload.
    pub data: Vec<u8>,
}
/// A material of a model: its name, the material definition it names and its texture bindings.
#[derive(Clone, Debug)]
pub struct Material {
    /// The material's name.
    pub name: String,
    /// The material definition it refers to (an MTD path in Sekiro, a MATBIN name in Elden Ring), as stored.
    pub mtd: String,
    /// Its texture bindings, in order.
    pub textures: Vec<Texture>,
    /// Raw, uninterpreted GX records in source order, excluding the terminator.
    pub gx: Vec<GxItem>,
}
/// One face set of a mesh: an index list with its flags.
#[derive(Clone, Debug)]
pub struct FaceSet {
    /// The flags as stored (bits 24 to 26 the level of detail, bit 31 motion blur; see [`FaceSet::kind`]).
    pub flags: u32,
    /// Whether the indices are a triangle strip (`0xffff` restarts) rather than a list.
    pub strip: bool,
    /// Whether back faces are culled.
    pub cull: bool,
    /// The vertex indices.
    pub indices: Vec<u32>,
}
impl FaceSet {
    /// File-specific ordering: odd strip faces reverse all three indices. Restarts
    /// reset parity; degenerate joints advance parity without emitting a face.
    pub fn triangulate(&self) -> Vec<u32> {
        if !self.strip {
            return self.indices.clone();
        }
        let mut out = Vec::new();
        let mut flip = false;
        for t in self.indices.windows(3) {
            let &[a, b, c] = t else {
                continue;
            };
            if t.contains(&0xffff) {
                flip = false;
                continue;
            }
            if a != b && a != c && b != c {
                if flip {
                    out.extend([c, b, a]);
                } else {
                    out.extend([a, b, c]);
                }
            }
            flip = !flip;
        }
        out
    }
    /// Not a motion-blur face set (bit 31) and of level of detail 0: bits 24 to 26 hold the level (Sekiro's face sets
    /// use 1 and 2; Elden Ring's also 3 and 4, the coarse stand-ins its assets carry for far views).
    pub fn is_lod0(&self) -> bool {
        self.flags & 0x8700_0000 == 0
    }
}
/// A mesh: a material, its vertices and the face sets that index them.
#[derive(Clone, Debug)]
pub struct Mesh {
    /// The stored `Dynamic` flag (the mesh is skinned).
    pub dynamic: bool,
    /// Its material's index in [`Model::materials`].
    pub material: usize,
    /// The bone (node) the mesh stands for, as stored (`NodeIndex`, -1 for none): the bone a static mesh's vertices
    /// follow when its layout stores no normal W to name one per vertex ([`Model::stores_normal_w`]).
    pub node: i32,
    /// The mesh's bone palette as stored (bone indices): empty when its vertices name bones directly.
    pub palette: Vec<u32>,
    /// The indices in [`Model::layouts`] of the layouts of its vertex buffers, in buffer order.
    pub layouts: Vec<usize>,
    /// Its vertices, decoded.
    pub vertices: Vec<Vertex>,
    /// Its face sets.
    pub faces: Vec<FaceSet>,
}
/// A bone of the model's skeleton.
#[derive(Clone, Debug)]
pub struct Bone {
    /// The bone's name as stored.
    pub name: String,
    /// Its translation relative to its parent, as stored.
    pub translation: [f32; 3],
    /// Its rotation relative to its parent, as stored (three angles).
    pub rotation: [f32; 3],
    /// Its scale, as stored.
    pub scale: [f32; 3],
    /// Its parent's index in [`Model::bones`], -1 for none.
    pub parent: i16,
}
/// A dummy polygon (SoulsFormatsNEXT: "used for hit detection, particle effect locations, and much more"): a named-by-number
/// point with a direction, in the model, that the game attaches things to (a weapon to a hand, an effect to a muzzle).
/// All as stored; nothing here says which number is which (the game's, not the file's).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Dummy {
    /// The point. In the space of the parent bone where there is one ([`Dummy::parent_bone`]), else in the model's; the
    /// file's frame.
    pub position: [f32; 3],
    /// The direction the point faces.
    pub forward: [f32; 3],
    /// Meaningful only where [`Dummy::use_upward_vector`] is set.
    pub upward: [f32; 3],
    /// "Indicates the type of dummy point this is (hitbox, sfx, etc)" (SoulsFormatsNEXT): the number the game's scripts
    /// and events name the point by.
    pub reference_id: i16,
    /// "Index of a bone that the dummy point is initially transformed to before binding to the attach bone"; -1 for none.
    pub parent_bone: i16,
    /// "Index of the bone that the dummy point follows physically"; -1 for none.
    pub attach_bone: i16,
    /// The four colour bytes in the file's order (alpha, red, green, blue; SoulsFormatsNEXT: "not certain about the
    /// ordering").
    pub color: [u8; 4],
    /// A stored flag whose meaning SoulsFormatsNEXT does not state.
    pub flag1: bool,
    /// Whether [`Dummy::upward`] is meaningful.
    pub use_upward_vector: bool,
    /// Two words SoulsFormatsNEXT calls unknown ("only used in Sekiro").
    pub unk30: i32,
    /// The word after it.
    pub unk34: i32,
}

/// A read FLVER model.
#[derive(Clone, Debug)]
pub struct Model {
    /// The skeleton's bones, in file order.
    pub bones: Vec<Bone>,
    /// The model's dummy polygons, in file order.
    pub dummies: Vec<Dummy>,
    /// The FLVER version, one of `0x20013`, `0x20014`, `0x20016`, `0x2001a`.
    pub version: u32,
    /// A SpeedTree model ([`is_speedtree`]): its vertices carry their stored members in [`Vertex::speedtree`], and
    /// the attributes its programs read from them ([`speedtree_layout`]).
    pub speedtree: bool,
    /// The vertex layouts, each the members of one vertex buffer format.
    pub layouts: Vec<Vec<Member>>,
    /// The materials.
    pub materials: Vec<Material>,
    /// The meshes.
    pub meshes: Vec<Mesh>,
}
fn count(v: &View<'_>, o: usize) -> Result<usize> {
    let n = v.u32(o)?;
    if n > i32::MAX as u32 {
        return Err(invalid(v.format, o, "negative count or offset"));
    }
    Ok(n as usize)
}
fn table(v: &View<'_>, pos: &mut usize, n: usize, stride: usize) -> Result<usize> {
    let len = n
        .checked_mul(stride)
        .ok_or_else(|| invalid(v.format, *pos, "table overflow"))?;
    v.slice(*pos, len)?;
    let start = *pos;
    *pos = v.at(start, len)?;
    Ok(start)
}
fn string(v: &View<'_>, o: usize, budget: &Budget) -> Result<String> {
    Ok(v.utf16z(o, budget)?.0)
}
fn count_at(v: &View<'_>, o: usize, delta: usize) -> Result<usize> {
    count(v, v.at(o, delta)?)
}
fn float_at(v: &View<'_>, o: usize, delta: usize) -> Result<f32> {
    float(v, v.at(o, delta)?)
}
fn float(v: &View<'_>, o: usize) -> Result<f32> {
    let n = f32::from_bits(v.u32(o)?);
    if !n.is_finite() {
        return Err(invalid(v.format, o, "non-finite value"));
    }
    Ok(n)
}
fn refs(v: &View<'_>, o: usize, n: usize, limit: usize, budget: &Budget) -> Result<Vec<usize>> {
    let bytes = n
        .checked_mul(4)
        .ok_or_else(|| invalid(v.format, o, "array overflow"))?;
    v.slice(o, bytes)?;
    budget.charge(o, bytes)?;
    (0..n)
        .map(|i| {
            let at = v.element(o, i, 4)?;
            let r = count(v, at)?;
            if r >= limit {
                Err(invalid(v.format, at, "index out of range"))
            } else {
                Ok(r)
            }
        })
        .collect()
}
impl Model {
    /// The bytes the model's meshes hold: each vertex as it sits in memory with the lengths of its channels, and each
    /// face set's indices. A copy of the model costs about this much (the bones, layouts and materials are small beside
    /// it).
    pub fn resident_bytes(&self) -> u64 {
        let channel = std::mem::size_of::<[f32; 4]>();
        self.meshes
            .iter()
            .map(|mesh| {
                let vertices: usize = mesh
                    .vertices
                    .iter()
                    .map(|v| {
                        [
                            std::mem::size_of::<Vertex>(),
                            [v.uvs.len(), v.tangents.len(), v.colors.len(), v.speedtree.len()]
                                .into_iter()
                                .fold(0usize, usize::saturating_add)
                                .saturating_mul(channel),
                            v.tangent_indices.len(),
                            v.layer_bytes
                                .len()
                                .saturating_mul(std::mem::size_of::<(usize, [u8; 4])>()),
                        ]
                        .into_iter()
                        .fold(0usize, usize::saturating_add)
                    })
                    .sum();
                let indices: usize = mesh
                    .faces
                    .iter()
                    .map(|f| f.indices.len().saturating_mul(std::mem::size_of::<u32>()))
                    .sum();
                u64::try_from(vertices.saturating_add(indices)).unwrap_or(u64::MAX)
            })
            .sum()
    }

    /// Read a FLVER from its decompressed bytes. Every table, count and offset is bounded by the file, the bytes the
    /// records name are bounded by its size (records that name one range over and over are refused), and every vertex
    /// buffer and face set must be used by a mesh; an unknown version, vertex format or flag is an error naming the
    /// offset.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: "FLVER",
            be: false,
        };
        v.expect(0, b"FLVER\0L\0")?;
        let version = v.u32(8)?;
        if !matches!(version, 0x20013 | 0x20014 | 0x20016 | 0x2001a) {
            return Err(invalid(v.format, 8, format!("unsupported version {version:#x}")));
        }
        let data_start = count(&v, 12)?;
        let data_len = count(&v, 16)?;
        v.slice(data_start, data_len)?;
        // Elden Ring's files may be zero-padded after the data block to the next 2048-byte boundary.
        let end = v.at(data_start, data_len)?;
        if !(end == data.len()
            || (data.len() == end.next_multiple_of(0x800)
                && v.slice(end, data.len().saturating_sub(end))?
                    .iter()
                    .all(|&b| b == 0)))
        {
            return Err(invalid(v.format, 16, "data length mismatch"));
        }
        let meta = View {
            data: v.slice(0, data_start)?,
            format: v.format,
            be: false,
        };
        // names, GX records, palettes, index lists and vertex buffers are copied or decoded from the bytes the records
        // name: records that name one range over and over are refused before it is copied
        let budget = Budget::new(v.format, data.len());
        for i in 0..6 {
            float(&v, v.element(40, i, 4)?)?;
        }
        if !v.boolean(73)? {
            return Err(invalid(v.format, 73, "legacy text encoding unsupported"));
        }
        v.boolean(74)?;
        v.expect(75, &[0])?;
        v.expect(94, &[0; 10])?;
        v.expect(108, &[0; 20])?;
        let mut pos = 128;
        let dummy_count = count(&v, 20)?;
        let mat_count = count(&v, 24)?;
        let bone_count = count(&v, 28)?;
        let mesh_count = count(&v, 32)?;
        let vb_count = count(&v, 36)?;
        let face_count = count(&v, 80)?;
        let layout_count = count(&v, 84)?;
        let texture_count = count(&v, 88)?;
        let dummy = table(&v, &mut pos, dummy_count, 64)?;
        let mat = table(&v, &mut pos, mat_count, 32)?;
        let bone = table(&v, &mut pos, bone_count, 128)?;
        let mesh = table(&v, &mut pos, mesh_count, 48)?;
        let face = table(&v, &mut pos, face_count, 32)?;
        let vb = table(&v, &mut pos, vb_count, 32)?;
        let layout = table(&v, &mut pos, layout_count, 16)?;
        let texture = table(&v, &mut pos, texture_count, 32)?;
        if version == 0x2001a {
            let s = table(&v, &mut pos, 1, 32)?;
            v.expect_at(s, 12, &[0; 20])?;
            for i in 0..2 {
                let n = v.u16(v.element(s, i, 2)?)? as usize;
                let mut p = count(&v, v.element(v.at(s, 4)?, i, 4)?)?;
                let a = table(&meta, &mut p, n, 16)?;
                for j in 0..n {
                    meta.expect(v.at(v.element(a, j, 16)?, 12)?, &[0; 4])?;
                }
            }
        }
        if pos > data_start {
            return Err(invalid(v.format, pos, "tables overlap data"));
        }
        let mut dummies = Vec::new();
        for i in 0..dummy_count {
            let o = v.element(dummy, i, 64)?;
            let flag1 = v.boolean_at(o, 46)?;
            let use_upward_vector = v.boolean_at(o, 47)?;
            v.expect_at(o, 56, &[0; 8])?;
            let bytes = v.slice(v.at(o, 12)?, 4)?;
            // as stored: a dummy is data the model's meshes do not depend on, so a value that is not finite does not
            // refuse the model (whoever places the dummy checks it)
            let raw = |delta: usize| -> Result<f32> { Ok(f32::from_bits(v.u32_at(o, delta)?)) };
            dummies.push(Dummy {
                position: [raw(0)?, raw(4)?, raw(8)?],
                color: bytes
                    .try_into()
                    .map_err(|_| invalid(v.format, o, "dummy colour bytes"))?,
                forward: [raw(16)?, raw(20)?, raw(24)?],
                reference_id: v.u16_at(o, 28)? as i16,
                parent_bone: v.u16_at(o, 30)? as i16,
                upward: [raw(32)?, raw(36)?, raw(40)?],
                attach_bone: v.u16_at(o, 44)? as i16,
                flag1,
                use_upward_vector,
                unk30: v.u32_at(o, 48)? as i32,
                unk34: v.u32_at(o, 52)? as i32,
            });
        }
        let mut bones = Vec::new();
        for i in 0..bone_count {
            let o = v.element(bone, i, 128)?;
            let name = string(&meta, count_at(&v, o, 12)?, &budget)?;
            v.expect_at(o, 76, &[0; 52])?;
            bones.push(Bone {
                name,
                translation: [float(&v, o)?, float_at(&v, o, 4)?, float_at(&v, o, 8)?],
                rotation: [float_at(&v, o, 16)?, float_at(&v, o, 20)?, float_at(&v, o, 24)?],
                scale: [float_at(&v, o, 32)?, float_at(&v, o, 36)?, float_at(&v, o, 40)?],
                parent: v.u16_at(o, 28)? as i16,
            });
        }
        let speedtree = is_speedtree(data);
        let mut layouts = Vec::new();
        for i in 0..layout_count {
            let o = v.element(layout, i, 16)?;
            v.expect_at(o, 4, &[0; 8])?;
            let n = count(&v, o)?;
            let mut p = count_at(&v, o, 12)?;
            let a = table(&meta, &mut p, n, 20)?;
            let mut members = Vec::new();
            // each stream's members are contiguous in it: an ordinary layout's one stream, a SpeedTree layout's stored
            // stream and each stream it declares but does not store
            let mut offsets = std::collections::BTreeMap::<(bool, u16), usize>::new();
            for j in 0..n {
                let r = v.element(a, j, 20)?;
                let word = v.u32(r)?;
                let (stored, stream) = if speedtree {
                    match word >> 16 {
                        0 => (true, (word & 0xffff) as u16),
                        0x8000 => (false, (word & 0xffff) as u16),
                        _ => return Err(invalid(v.format, r, "SpeedTree member modifier")),
                    }
                } else {
                    (true, 0)
                };
                let kind = v.u32_at(r, 8)?;
                let offset = offsets.entry((stored, stream)).or_default();
                if count_at(&v, r, 4)? != *offset {
                    return Err(invalid(v.format, v.at(r, 4)?, "noncontiguous vertex layout"));
                }
                members.push(Member {
                    unknown: word,
                    stored,
                    offset: *offset,
                    kind,
                    semantic: v.u32_at(r, 12)?,
                    index: v.u32_at(r, 16)?,
                });
                *offset = offset.saturating_add(vertex::size(kind)?);
            }
            layouts.push(members);
        }
        let mut textures = Vec::new();
        for i in 0..texture_count {
            let o = v.element(texture, i, 32)?;
            v.expect_at(o, 18, &[0; 2])?;
            let tiling = |at: usize| -> Result<Tiling> {
                Tiling::read(v.u8(at)?).ok_or_else(|| invalid(v.format, at, "texture tiling type"))
            };
            textures.push(Some(Texture {
                path: string(&meta, count(&v, o)?, &budget)?,
                kind: string(&meta, count_at(&v, o, 4)?, &budget)?,
                scale: [float_at(&v, o, 8)?, float_at(&v, o, 12)?],
                tiling: [tiling(v.at(o, 16)?)?, tiling(v.at(o, 17)?)?],
                unknown: [float_at(&v, o, 20)?, float_at(&v, o, 24)?, float_at(&v, o, 28)?],
            }));
        }
        let gx_padding_only = v.u16(0x68)? == 5;
        let mut materials = Vec::new();
        for i in 0..mat_count {
            let o = v.element(mat, i, 32)?;
            v.expect_at(o, 28, &[0; 4])?;
            let n = count_at(&v, o, 8)?;
            let first = count_at(&v, o, 12)?;
            let mut bound = Vec::new();
            for j in first
                ..first
                    .checked_add(n)
                    .ok_or_else(|| invalid(v.format, o, "texture range overflow"))?
            {
                bound.push(
                    textures
                        .get_mut(j)
                        .and_then(Option::take)
                        .ok_or_else(|| invalid(v.format, o, "missing or shared texture"))?,
                );
            }
            let mut gx = count_at(&v, o, 20)?;
            let mut gx_items = Vec::new();
            if gx != 0 {
                loop {
                    let id = meta.u32(gx)?;
                    let len = count_at(&meta, gx, 8)?;
                    if matches!(id, 0x7fff_ffff | 0xffff_ffff) {
                        // the terminator's length counts its zero padding and its 12-byte header, except in a file
                        // whose header word at 0x68 has 5 in its low half (Elden Ring's SpeedTree models), where it
                        // counts the padding only (as SoulsFormatsNEXT reads it)
                        let padding = if gx_padding_only {
                            len
                        } else {
                            len.checked_sub(12)
                                .ok_or_else(|| invalid(v.format, gx, "short GX record"))?
                        };
                        if v.u32_at(gx, 4)? != 100 || v.slice(v.at(gx, 12)?, padding)?.iter().any(|&b| b != 0)
                        {
                            return Err(invalid(v.format, gx, "GX terminator"));
                        }
                        break;
                    }
                    if len < 12 {
                        return Err(invalid(v.format, gx, "short GX record"));
                    }
                    meta.slice(gx, len)?;
                    budget.charge(gx, len)?;
                    gx_items.push(GxItem {
                        id: meta.slice(gx, 4)?.try_into().expect("four-byte checked slice"),
                        version: meta.u32_at(gx, 4)?,
                        data: meta.slice(meta.at(gx, 12)?, len.saturating_sub(12))?.to_vec(),
                    });
                    gx = meta.at(gx, len)?;
                }
            }
            materials.push(Material {
                name: string(&meta, count(&v, o)?, &budget)?,
                mtd: string(&meta, count_at(&v, o, 4)?, &budget)?,
                textures: bound,
                gx: gx_items,
            });
        }
        if textures.iter().any(Option::is_some) {
            return Err(invalid(v.format, texture, "unclaimed textures"));
        }
        let mut faces = Vec::new();
        for i in 0..face_count {
            let o = v.element(face, i, 32)?;
            v.expect_at(o, 20, &[0; 4])?;
            v.expect_at(o, 28, &[0; 4])?;
            let flags = v.u32(o)?;
            let n = count_at(&v, o, 8)?;
            let width = match v.u32_at(o, 24)? {
                0 => v.u8(72)? as u32,
                w => w,
            };
            if !matches!(width, 16 | 32) || flags & 0x4000_0000 != 0 {
                return Err(invalid(v.format, o, "unsupported compressed indices"));
            }
            let start = data_start
                .checked_add(count_at(&v, o, 12)?)
                .ok_or_else(|| invalid(v.format, o, "index offset overflow"))?;
            if count_at(&v, o, 16)?
                != n.checked_mul(width as usize / 8)
                    .ok_or_else(|| invalid(v.format, o, "index length overflow"))?
            {
                return Err(invalid(v.format, v.at(o, 16)?, "index length mismatch"));
            }
            v.slice(
                start,
                n.checked_mul(width as usize / 8)
                    .ok_or_else(|| invalid(v.format, o, "index size overflow"))?,
            )?;
            budget.charge(start, n.saturating_mul(width as usize / 8))?;
            let indices = (0..n)
                .map(|j| {
                    if width == 16 {
                        v.element(start, j, 2).and_then(|at| v.u16(at)).map(u32::from)
                    } else {
                        v.element(start, j, 4).and_then(|at| v.u32(at))
                    }
                })
                .collect::<Result<_>>()?;
            faces.push(Some(FaceSet {
                flags,
                strip: v.boolean_at(o, 4)?,
                cull: v.boolean_at(o, 5)?,
                indices,
            }));
        }
        let mut used_vb = vec![false; vb_count];
        let mut meshes = Vec::new();
        for i in 0..mesh_count {
            let o = v.element(mesh, i, 48)?;
            let dynamic = v.boolean(o)?;
            v.expect_at(o, 1, &[0; 3])?;
            v.expect_at(o, 8, &[0; 8])?;
            let material = count_at(&v, o, 4)?;
            if material >= materials.len() {
                return Err(invalid(v.format, v.at(o, 4)?, "material index"));
            }
            let node = v.u32_at(o, 16)? as i32;
            let palette_len = count_at(&v, o, 20)?;
            let mut at = count_at(&v, o, 28)?;
            let start = table(&meta, &mut at, palette_len, 4)?;
            budget.charge(start, palette_len.saturating_mul(4))?;
            let palette = (0..palette_len)
                .map(|i| meta.element(start, i, 4).and_then(|at| meta.u32(at)))
                .collect::<Result<Vec<u32>>>()?;
            let face_refs = refs(
                &meta,
                count_at(&v, o, 36)?,
                count_at(&v, o, 32)?,
                face_count,
                &budget,
            )?;
            let buffers = refs(
                &meta,
                count_at(&v, o, 44)?,
                count_at(&v, o, 40)?,
                vb_count,
                &budget,
            )?;
            // A mesh with neither face sets nor vertex buffers is empty: its bounds are the exporter's sentinel
            // (minimum -inf, maximum NaN in Elden Ring's files) and are read but not required to be finite.
            let empty = face_refs.is_empty() && buffers.is_empty();
            let bounds = count_at(&v, o, 24)?;
            if bounds != 0 {
                for j in 0..if version == 0x2001a { 9 } else { 6 } {
                    if empty {
                        meta.u32(meta.element(bounds, j, 4)?)?;
                    } else {
                        float(&meta, meta.element(bounds, j, 4)?)?;
                    }
                }
            }
            if empty {
                meshes.push(Mesh {
                    dynamic,
                    material,
                    node,
                    palette,
                    layouts: Vec::new(),
                    vertices: Vec::new(),
                    faces: Vec::new(),
                });
                continue;
            }
            if buffers.is_empty() || buffers.len() > 3 {
                return Err(invalid(v.format, v.at(o, 40)?, "vertex buffer count"));
            }
            let &[first_buffer, ..] = buffers.as_slice() else {
                return Err(invalid(v.format, v.at(o, 40)?, "vertex buffer count"));
            };
            let n = count_at(&v, v.element(vb, first_buffer, 32)?, 12)?;
            // Validate every buffer extent before allocating decoded vertices.
            let mut streams = Vec::new();
            for (ordinal, &b) in buffers.iter().enumerate() {
                match used_vb.get_mut(b) {
                    Some(used) if !*used => *used = true,
                    _ => return Err(invalid(v.format, o, "shared vertex buffer")),
                }
                let r = v.element(vb, b, 32)?;
                v.expect_at(r, 16, &[0; 8])?;
                if v.u32(r)? != u32::try_from(ordinal).unwrap_or(u32::MAX) || count_at(&v, r, 12)? != n {
                    return Err(invalid(v.format, r, "vertex buffer identity/count"));
                }
                let layout_slot = v.at(r, 4)?;
                let l = count(&v, layout_slot)?;
                let members = layouts
                    .get(l)
                    .ok_or_else(|| invalid(v.format, layout_slot, "layout index"))?;
                let stride = count_at(&v, r, 8)?;
                let expected = members
                    .iter()
                    .filter(|m| m.stored)
                    .try_fold(0usize, |a, m| vertex::size(m.kind).map(|s| a.saturating_add(s)))?;
                if stride != expected || stride == 0 {
                    return Err(invalid(v.format, v.at(r, 8)?, "vertex stride"));
                }
                if count_at(&v, r, 24)?
                    != n.checked_mul(stride)
                        .ok_or_else(|| invalid(v.format, r, "vertex length overflow"))?
                {
                    return Err(invalid(v.format, v.at(r, 24)?, "vertex length mismatch"));
                }
                let start = data_start
                    .checked_add(count_at(&v, r, 28)?)
                    .ok_or_else(|| invalid(v.format, r, "buffer offset overflow"))?;
                v.slice(
                    start,
                    n.checked_mul(stride)
                        .ok_or_else(|| invalid(v.format, r, "buffer size overflow"))?,
                )?;
                budget.charge(start, n.saturating_mul(stride))?;
                streams.push((l, stride, start));
            }
            let mut vertices = vec![Vertex::default(); n];
            for &(l, stride, start) in &streams {
                let members = layouts.get(l).map(Vec::as_slice).unwrap_or_default();
                for (j, vert) in vertices.iter_mut().enumerate() {
                    let data = v.slice(v.element(start, j, stride)?, stride)?;
                    if speedtree {
                        vert.read_speedtree(data, members)?;
                    } else {
                        vert.read(data, members, 2048.)?;
                    }
                }
            }
            if speedtree {
                // one stored stream per mesh: the members' indices are its layout's
                let &[(l, ..)] = &streams[..] else {
                    return Err(invalid(
                        v.format,
                        o,
                        "a SpeedTree mesh with other than one vertex buffer",
                    ));
                };
                if let SpeedTreeLayout::Geometry {
                    position,
                    uv,
                    normal,
                    tangent,
                } = speedtree_layout(layouts.get(l).map(Vec::as_slice).unwrap_or_default())?
                {
                    for vert in &mut vertices {
                        vert.speedtree_attributes(position, uv, normal, tangent)?;
                    }
                }
            }
            let mut mesh_faces = Vec::new();
            for f in face_refs {
                let fs = faces
                    .get_mut(f)
                    .and_then(Option::take)
                    .ok_or_else(|| invalid(v.format, o, "shared face set"))?;
                if fs
                    .indices
                    .iter()
                    .any(|&r| usize::try_from(r).is_ok_and(|r| r >= n) && !(fs.strip && r == 0xffff))
                {
                    return Err(invalid(v.format, o, "vertex index out of range"));
                }
                mesh_faces.push(fs);
            }
            meshes.push(Mesh {
                dynamic,
                material,
                node,
                palette,
                layouts: streams.iter().map(|s| s.0).collect(),
                vertices,
                faces: mesh_faces,
            });
        }
        if faces.iter().any(Option::is_some) || used_vb.contains(&false) {
            return Err(invalid(v.format, mesh, "unclaimed face set or vertex buffer"));
        }
        Ok(Self {
            bones,
            dummies,
            version,
            speedtree,
            layouts,
            materials,
            meshes,
        })
    }
}

/// Whether a FLVER stores SpeedTree vegetation: the high half of its header word at 0x68 is 0x8000. Its layouts' members
/// name their stream and whether the file stores them ([`Member::stored`]), with offsets per stream; [`Model::read`]
/// reads the stored ones into [`Vertex::speedtree`] (the reference reader, SoulsFormatsNEXT ee1dd61, refuses
/// such models) and the vertex's attributes from them ([`speedtree_layout`]).
pub fn is_speedtree(data: &[u8]) -> bool {
    data.starts_with(b"FLVER\0L\0") && data.get(0x6a..0x6c) == Some(&[0x00, 0x80][..])
}

/// What a SpeedTree layout's stored members are to the game's SpeedTree vertex programs ([`speedtree_layout`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedTreeLayout {
    /// Branches, fronds and leaves: the indices, among the stored members, of the rest position (`xyz`), UV0 (`xy`),
    /// the normal and the SpeedTree tangent (unsigned bytes, unpacked as `x * 2 - 1`).
    Geometry {
        /// The index of the position member among the stored members.
        position: usize,
        /// The index of the UV0 member.
        uv: usize,
        /// The index of the normal member.
        normal: usize,
        /// The index of the tangent member.
        tangent: usize,
    },
    /// A billboard card (`Float4` UV2, `Float2` UV3): its program turns the card to the camera and picks its image
    /// in the atlas by the view's angle, so its vertices have no position of their own.
    Billboard,
}

/// The roles of a SpeedTree layout's stored members, as Elden Ring's SpeedTree G-buffer vertex programs read them:
/// with the per-instance stream an identity instance (position 0, scale 1, basis vectors x and y, the level-of-detail
/// blend at the finest level) and the wind off, each program outputs `Half4` UV2's `xyz` as the position, `Half4`
/// UV3's `xy` as the texture coordinate, and the members at its last two `TEXCOORD` inputs as the normal and the
/// tangent: its layout's two `UByte4Norm` members, in order, for every mesh but three, whose programs read another
/// member ([`Model::set_speedtree_frame`] gives a mesh those). The billboard cards' layout is [`SpeedTreeLayout::Billboard`]; another layout is refused.
pub fn speedtree_layout(members: &[Member]) -> Result<SpeedTreeLayout> {
    let stored: Vec<&Member> = members.iter().filter(|m| m.stored).collect();
    let refused = || {
        invalid(
            "FLVER SpeedTree layout",
            0,
            "stored members of no known SpeedTree layout",
        )
    };
    let uv = |index: u32, kind: u32| {
        stored
            .iter()
            .position(|m| m.semantic == 5 && m.index == index && m.kind == kind)
    };
    if stored.len() == 2 && uv(2, 3) == Some(0) && uv(3, 1) == Some(1) {
        return Ok(SpeedTreeLayout::Billboard);
    }
    let (Some(position), Some(texture)) = (uv(2, 0x2e), uv(3, 0x2e)) else {
        return Err(refused());
    };
    let bytes: Vec<usize> = stored
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind == 0x13)
        .map(|(k, _)| k)
        .collect();
    let &[normal, tangent] = &bytes[..] else {
        return Err(refused());
    };
    if stored
        .iter()
        .any(|m| m.semantic != 5 || !matches!(m.kind, 0x13 | 0x2d | 0x2e))
    {
        return Err(refused());
    }
    Ok(SpeedTreeLayout::Geometry {
        position,
        uv: texture,
        normal,
        tangent,
    })
}

impl Model {
    /// Whether mesh `mesh`'s layout stores a normal with a W (every normal member the reader decodes but `Float3`):
    /// the W a static vertex names its bone by. A mesh that does not exist stores none.
    pub fn stores_normal_w(&self, mesh: usize) -> bool {
        self.meshes.get(mesh).is_some_and(|m| {
            m.layouts
                .iter()
                .filter_map(|&l| self.layouts.get(l))
                .flatten()
                .any(|member| member.semantic == 3 && member.stored && member.kind != 2)
        })
    }

    /// The stored members of mesh `mesh`'s one SpeedTree layout, and its roles; `None` for an ordinary model.
    fn speedtree_mesh(&self, mesh: usize) -> Result<Option<(Vec<&Member>, SpeedTreeLayout)>> {
        if !self.speedtree {
            return Ok(None);
        }
        let m = self
            .meshes
            .get(mesh)
            .ok_or_else(|| invalid("FLVER SpeedTree mesh", mesh, "mesh index"))?;
        let Some(&l) = m.layouts.first() else {
            return Ok(None);
        };
        let members = self
            .layouts
            .get(l)
            .ok_or_else(|| invalid("FLVER SpeedTree mesh", mesh, format!("layout index {l}")))?;
        Ok(Some((
            members.iter().filter(|m| m.stored).collect(),
            speedtree_layout(members)?,
        )))
    }

    /// The input indices (`TEXCOORD<n>`, the members' UV indices) of the normal and the tangent mesh `mesh`'s layout
    /// holds ([`SpeedTreeLayout::Geometry`], what [`Model::read`] sets); `None` for an ordinary model's mesh and a
    /// billboard card's.
    pub fn speedtree_frame(&self, mesh: usize) -> Result<Option<[u32; 2]>> {
        Ok(match self.speedtree_mesh(mesh)? {
            Some((stored, SpeedTreeLayout::Geometry { normal, tangent, .. })) => {
                let index = |k: usize| {
                    stored
                        .get(k)
                        .map(|m| m.index)
                        .ok_or_else(|| invalid("FLVER SpeedTree mesh", mesh, format!("stored member {k}")))
                };
                Some([index(normal)?, index(tangent)?])
            }
            _ => None,
        })
    }

    /// Set mesh `mesh`'s vertices' normal and tangent from the stored members a program reads as them, at its inputs
    /// `TEXCOORD<normal>` and `TEXCOORD<tangent>`, unpacked as for the layout's pair. Where the program reads the
    /// layout's own pair this changes nothing. Refused for a mesh that is not a SpeedTree geometry mesh, and for an
    /// input its layout does not store.
    pub fn set_speedtree_frame(&mut self, mesh: usize, [normal, tangent]: [u32; 2]) -> Result<()> {
        let refused = |why: String| invalid("FLVER SpeedTree mesh", mesh, why);
        let (normal, tangent) = match self.speedtree_mesh(mesh)? {
            Some((stored, SpeedTreeLayout::Geometry { .. })) => {
                let at = |index: u32| {
                    stored
                        .iter()
                        .position(|m| m.semantic == 5 && m.index == index)
                        .ok_or_else(|| refused(format!("its layout stores no TEXCOORD{index}")))
                };
                (at(normal)?, at(tangent)?)
            }
            _ => return Err(refused("not a SpeedTree geometry mesh".into())),
        };
        let vertices = self
            .meshes
            .get_mut(mesh)
            .map(|m| m.vertices.as_mut_slice())
            .ok_or_else(|| refused("mesh index".into()))?;
        for v in vertices {
            v.speedtree_frame(normal, tangent)?;
        }
        Ok(())
    }
}
/// What a face set is for, from its flags: a level of detail (0 the finest) or the motion-blur variant of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaceSetKind {
    /// A level of detail; 0 is the finest.
    Lod(u8),
    /// The motion-blur variant of a level of detail.
    MotionBlur(u8),
}

impl FaceSet {
    /// Bits 24 to 26 of the flags are the level of detail (`0x0100_0000` level 1 ... `0x0400_0000` level 4),
    /// `0x8000_0000` the motion-blur variant; edge compression is refused when the model is read.
    pub fn kind(&self) -> FaceSetKind {
        let level = ((self.flags >> 24) & 7) as u8;
        if self.flags & 0x8000_0000 != 0 {
            FaceSetKind::MotionBlur(level)
        } else {
            FaceSetKind::Lod(level)
        }
    }
}
