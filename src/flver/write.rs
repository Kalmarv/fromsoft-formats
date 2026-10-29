//! The FLVER writer: [`Model::write`] with the [`Layout`] of the file.
//!
//! A [`Model`] holds what a game reads from a FLVER; the file stores more, which [`Layout`] holds: the header's bounding
//! box and face counts, the meshes' bounding boxes, the skeleton set of version `0x2001a`, the index width each face set
//! states, and the bytes of the material extension (GX) area, which in Sekiro's and Elden Ring's files holds lists that
//! several materials share and lists that no material names. [`Layout::read`] takes them from a file; [`Layout::for_model`]
//! makes them for a model that was built or edited here. A model read from a file and written with that file's layout
//! writes back to the file's bytes: every one of Sekiro's 14,064 and Elden Ring's 20,557 distinct FLVERs does.
//!
//! The file, in order: the 128-byte header; the tables of dummies, materials, bones, meshes, face sets, vertex buffers,
//! vertex layouts and textures; the skeleton set (version `0x2001a`: a 32-byte record, then its two lists of 16-byte
//! bones); then, each section padded to 16 bytes, the layouts' members, the meshes' bounding boxes, bone palettes, face
//! set lists and vertex buffer lists, the GX area, the strings (each material's name and definition, then its textures'
//! paths and slot names; then the bones' names) and the data: for each mesh its face sets' indices and its vertex buffers
//! (the vertex buffers first in a SpeedTree model), each padded to 16 bytes. A mesh without a bone palette points the
//! palette offset at the start of the palettes. A SpeedTree model (the header word at +0x68 is 5 and the one at +0x6a
//! 0x8000) differs in three more places: each texture path is followed by one more zero unit, the data starts at a multiple
//! of 0x800, and the file is padded to one.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use super::{Material, Mesh, Model, count, encode, is_speedtree, table, vertex};
use crate::{Result, bytes::View, invalid, put::Out};

const FORMAT: &str = "FLVER";

/// The versions the reader and the writer take.
fn known_version(version: u32) -> bool {
    matches!(version, 0x20013 | 0x20014 | 0x20016 | 0x2001a)
}

/// One bone of a skeleton set: a bone's links in the set, and the model bone it stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkeletonBone {
    /// The parent's index in the list, -1 for none.
    pub parent: i16,
    /// The first child's index in the list, -1 for none.
    pub child: i16,
    /// The next sibling's index in the list, -1 for none.
    pub next_sibling: i16,
    /// The previous sibling's index in the list, -1 for none.
    pub previous_sibling: i16,
    /// The index in [`Model::bones`] of the bone it stands for.
    pub node: i32,
}

/// The skeleton set of a version `0x2001a` FLVER: two lists of bones. The first is the model's bone tree on every FLVER of
/// both installs; the second is the same on most and a reordering of the bones on the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkeletonSet {
    /// The base skeleton.
    pub base: Vec<SkeletonBone>,
    /// The second list.
    pub all: Vec<SkeletonBone>,
}

/// The bytes of the material extension (GX) area of a file, from the 16-aligned end of the vertex buffer lists to the first
/// string, and where each material's list is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GxArea {
    /// The area as stored: GX lists (items, then a terminator), the lists no material names, and the zero padding after
    /// them.
    pub bytes: Vec<u8>,
    /// For each material of the model, the offset in [`GxArea::bytes`] of its list, or `None` for a material that names
    /// none.
    pub offsets: Vec<Option<usize>>,
}

/// What a FLVER file stores that its [`Model`] does not hold (see the module's note).
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The model's bounding box in the header: the minimum, then the maximum.
    pub bounds: [[f32; 3]; 2],
    /// The two face counts of the header: the faces not counting motion-blur face sets or degenerate faces, and all faces.
    pub face_counts: [u32; 2],
    /// The header byte at +0x48: 0 in every file but the SpeedTree models, whose face sets state their widths all the
    /// same and which hold 16 here.
    pub index_width: u8,
    /// The header byte at +0x5d: 2 in version `0x2001a`, else 0, in every file of both installs.
    pub header_5d: u8,
    /// The header word at +0x68: 4 in every file of version `0x20014` and later but the SpeedTree models, whose 5 also
    /// shortens the GX terminators' stored length; 0 in version `0x20013`.
    pub header_68: u16,
    /// For each mesh, the words of its bounding box as stored (six, nine in version `0x2001a`), `None` for a mesh without
    /// one. The words are float bit patterns: an empty mesh stores infinity and NaN.
    pub mesh_bounds: Vec<Option<Vec<u32>>>,
    /// For each face set, mesh by mesh, the index width its record states: 16 or 32, or 0 for the header's.
    pub face_set_widths: Vec<u32>,
    /// Whether each mesh's data holds its vertex buffers before its face sets (the SpeedTree models).
    pub vertex_buffers_first: bool,
    /// The skeleton set of version `0x2001a`.
    pub skeletons: Option<SkeletonSet>,
    /// The GX area as stored; `None` writes the lists of the model's materials once each, without any other.
    pub gx: Option<GxArea>,
}

/// `n` as an offset in the file, or a refusal naming `what`.
fn offset(out: &Out, n: usize, what: &str) -> Result<u32> {
    out.word(n, what)
}

impl Layout {
    /// The layout of the FLVER file `data`, which [`Model::read`] takes. Every offset is bounded by the file; a field the
    /// layout expects (the version, the zero words) but the file lacks is refused with its offset.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        v.expect(0, b"FLVER\0L\0")?;
        let version = v.u32(8)?;
        if !known_version(version) {
            return Err(v.err(8, format!("unsupported version {version:#x}")));
        }
        let data_start = count(&v, 12)?;
        let meta = View {
            data: v.slice(0, data_start)?,
            format: FORMAT,
            be: false,
            base: 0,
        };
        let float = |o: usize| -> Result<f32> { Ok(f32::from_bits(v.u32(o)?)) };
        let mut bounds = [[0f32; 3]; 2];
        for (k, slot) in bounds.iter_mut().flatten().enumerate() {
            *slot = float(v.element(40, k, 4)?)?;
        }
        let face_counts = [v.u32(64)?, v.u32(68)?];
        let (dummies, materials, bones, meshes, vbs, face_sets, layouts, textures) = (
            count(&v, 20)?,
            count(&v, 24)?,
            count(&v, 28)?,
            count(&v, 32)?,
            count(&v, 36)?,
            count(&v, 80)?,
            count(&v, 84)?,
            count(&v, 88)?,
        );
        let mut pos = 128;
        table(&v, &mut pos, dummies, 64)?;
        let materials_at = table(&v, &mut pos, materials, 32)?;
        let bones_at = table(&v, &mut pos, bones, 128)?;
        let meshes_at = table(&v, &mut pos, meshes, 48)?;
        let face_sets_at = table(&v, &mut pos, face_sets, 32)?;
        let vbs_at = table(&v, &mut pos, vbs, 32)?;
        let layouts_at = table(&v, &mut pos, layouts, 16)?;
        let textures_at = table(&v, &mut pos, textures, 32)?;

        // the skeleton set, and where the writer stands after it
        let mut skeletons = None;
        let mut p = pos;
        if version == 0x2001a {
            let s = table(&v, &mut pos, 1, 32)?;
            let lists = [
                (v.u16(s)? as usize, count(&v, v.at(s, 4)?)?),
                (v.u16(v.at(s, 2)?)? as usize, count(&v, v.at(s, 8)?)?),
            ];
            let read_list = |(n, at): (usize, usize)| -> Result<Vec<SkeletonBone>> {
                (0..n)
                    .map(|i| {
                        let o = meta.element(at, i, 16)?;
                        Ok(SkeletonBone {
                            parent: meta.u16(o)? as i16,
                            child: meta.u16_at(o, 2)? as i16,
                            next_sibling: meta.u16_at(o, 4)? as i16,
                            previous_sibling: meta.u16_at(o, 6)? as i16,
                            node: meta.u32_at(o, 8)? as i32,
                        })
                    })
                    .collect()
            };
            let [first, second] = lists;
            p = (first.1 + 16 * first.0).max(second.1 + 16 * second.0);
            skeletons = Some(SkeletonSet {
                base: read_list(first)?,
                all: read_list(second)?,
            });
        }

        // the meshes' bounding boxes, and the sizes of the sections that follow the layouts' members
        let words = if version == 0x2001a { 9 } else { 6 };
        let mut mesh_bounds = Vec::with_capacity(meshes);
        let (mut box_bytes, mut palettes, mut face_lists, mut buffer_lists) =
            (0usize, 0usize, 0usize, 0usize);
        for i in 0..meshes {
            let o = v.element(meshes_at, i, 48)?;
            let at = count(&v, v.at(o, 24)?)?;
            if at == 0 {
                mesh_bounds.push(None);
            } else {
                mesh_bounds.push(Some(
                    (0..words)
                        .map(|k| meta.u32(meta.element(at, k, 4)?))
                        .collect::<Result<Vec<u32>>>()?,
                ));
                box_bytes += 4 * words;
            }
            palettes += 4 * count(&v, v.at(o, 20)?)?;
            face_lists += 4 * count(&v, v.at(o, 32)?)?;
            buffer_lists += 4 * count(&v, v.at(o, 40)?)?;
        }
        p = p.next_multiple_of(16);
        for i in 0..layouts {
            p += 20 * count(&v, v.element(layouts_at, i, 16)?)?;
        }
        for section in [box_bytes, palettes, face_lists, buffer_lists] {
            p = p.next_multiple_of(16) + section;
        }
        let gx_start = p.next_multiple_of(16);

        // the first string
        let mut string_start = data_start;
        for i in 0..materials {
            let o = v.element(materials_at, i, 32)?;
            string_start = string_start.min(count(&v, o)?).min(count(&v, v.at(o, 4)?)?);
        }
        for i in 0..textures {
            let o = v.element(textures_at, i, 32)?;
            string_start = string_start.min(count(&v, o)?).min(count(&v, v.at(o, 4)?)?);
        }
        for i in 0..bones {
            string_start = string_start.min(count(&v, v.at(v.element(bones_at, i, 128)?, 12)?)?);
        }
        if gx_start > string_start {
            return Err(v.err(gx_start, "the GX area would start after the first string"));
        }
        let area = meta.slice(gx_start, string_start - gx_start)?.to_vec();
        let mut offsets = Vec::with_capacity(materials);
        for i in 0..materials {
            let o = v.element(materials_at, i, 32)?;
            let at = count(&v, v.at(o, 20)?)?;
            offsets.push(if at == 0 {
                None
            } else {
                let rel = at
                    .checked_sub(gx_start)
                    .filter(|&rel| rel < area.len())
                    .ok_or_else(|| v.err(v.at(o, 20).unwrap_or(o), "a GX list outside the GX area"))?;
                Some(rel)
            });
        }

        // the width each face set's record states
        let face_set_widths = (0..face_sets)
            .map(|i| v.u32(v.at(v.element(face_sets_at, i, 32)?, 24)?))
            .collect::<Result<Vec<u32>>>()?;

        // the order of the data: the first mesh that has both a face set and a vertex buffer says
        let mut vertex_buffers_first = is_speedtree(data);
        for i in 0..meshes {
            let o = v.element(meshes_at, i, 48)?;
            let (faces, buffers) = (count(&v, v.at(o, 32)?)?, count(&v, v.at(o, 40)?)?);
            if faces == 0 || buffers == 0 {
                continue;
            }
            // the first face set and vertex buffer the mesh names, by their lists
            let face = count(&v, count(&v, v.at(o, 36)?)?)?;
            let buffer = count(&v, count(&v, v.at(o, 44)?)?)?;
            let face_at = count(&v, v.at(v.element(face_sets_at, face, 32)?, 12)?)?;
            let buffer_at = count(&v, v.at(v.element(vbs_at, buffer, 32)?, 28)?)?;
            vertex_buffers_first = buffer_at < face_at;
            break;
        }
        Ok(Layout {
            bounds,
            face_counts,
            index_width: v.u8(72)?,
            header_5d: v.u8(93)?,
            header_68: v.u16(104)?,
            mesh_bounds,
            face_set_widths,
            vertex_buffers_first,
            skeletons,
            gx: Some(GxArea { bytes: area, offsets }),
        })
    }

    /// A layout for `model` made here, for a model that was built or edited: the bounding boxes are the extent of the
    /// vertices (an empty mesh has none), the face counts are those of its face sets, a face set states 32 bits when an
    /// index needs more than 16, the skeleton set is the model's bone tree twice, the header constants are the ones of the
    /// model's version, and the GX area holds each distinct list of the materials once. It differs from the layout of the
    /// files the game's tools wrote in the bounding boxes' ninth-word extension (zero here) and wherever a file keeps GX
    /// lists no material names.
    pub fn for_model(model: &Model) -> Layout {
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        let mut any = false;
        let words = if model.version == 0x2001a { 9 } else { 6 };
        let mut mesh_bounds = Vec::with_capacity(model.meshes.len());
        let (mut true_faces, mut total_faces) = (0u32, 0u32);
        let mut face_set_widths = Vec::new();
        for mesh in &model.meshes {
            let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
            for vertex in &mesh.vertices {
                for ((l, h), x) in lo.iter_mut().zip(hi.iter_mut()).zip(vertex.position) {
                    *l = l.min(x);
                    *h = h.max(x);
                }
            }
            if mesh.vertices.is_empty() {
                mesh_bounds.push(None);
            } else {
                any = true;
                for ((l, h), (a, b)) in low.iter_mut().zip(high.iter_mut()).zip(lo.iter().zip(&hi)) {
                    *l = l.min(*a);
                    *h = h.max(*b);
                }
                let mut bits: Vec<u32> = lo.iter().chain(&hi).map(|x| x.to_bits()).collect();
                bits.resize(words, 0);
                mesh_bounds.push(Some(bits));
            }
            let restarts = mesh.vertices.len() < 0xffff;
            for face in &mesh.faces {
                let (all, kept) = face_counts(face, restarts);
                total_faces = total_faces.saturating_add(all);
                true_faces = true_faces.saturating_add(kept);
                face_set_widths.push(if face.indices.iter().any(|&i| i > 0xffff) {
                    32
                } else {
                    16
                });
            }
        }
        let skeletons = (model.version == 0x2001a).then(|| {
            let tree: Vec<SkeletonBone> = model
                .bones
                .iter()
                .enumerate()
                .map(|(i, b)| SkeletonBone {
                    parent: b.parent,
                    child: b.child,
                    next_sibling: b.next_sibling,
                    previous_sibling: b.previous_sibling,
                    node: i32::try_from(i).unwrap_or(i32::MAX),
                })
                .collect();
            SkeletonSet {
                base: tree.clone(),
                all: tree,
            }
        });
        Layout {
            bounds: if any { [low, high] } else { [[0.; 3]; 2] },
            face_counts: [true_faces, total_faces],
            index_width: if model.speedtree { 16 } else { 0 },
            header_5d: u8::from(model.version == 0x2001a) * 2,
            header_68: match (model.speedtree, model.version) {
                (true, _) => 5,
                (false, 0x20013) => 0,
                _ => 4,
            },
            mesh_bounds,
            face_set_widths,
            vertex_buffers_first: model.speedtree,
            skeletons,
            gx: None,
        }
    }
}

/// A face set's faces as the header counts them: all of them, and those a motion-blur face set or a degenerate triangle
/// does not make. A strip counts the triangles of consecutive indices, skipping those that touch the restart marker where
/// the mesh uses one.
fn face_counts(face: &super::FaceSet, restarts: bool) -> (u32, u32) {
    let triangles = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    if !face.strip {
        let n = triangles(face.indices.len() / 3);
        return (n, n);
    }
    let (mut all, mut kept) = (0u32, 0u32);
    for w in face.indices.windows(3) {
        let &[a, b, c] = w else { continue };
        if restarts && (a == 0xffff || b == 0xffff || c == 0xffff) {
            continue;
        }
        all = all.saturating_add(1);
        if face.flags & 0x8000_0000 == 0 && a != b && b != c && c != a {
            kept = kept.saturating_add(1);
        }
    }
    (all, kept)
}

/// A NUL-terminated UTF-16 string.
fn string(out: &mut Out, text: &str) -> Result<()> {
    if text.contains('\0') {
        return Err(out.err(format!("{text:?} holds a NUL, which ends a stored string")));
    }
    for unit in text.encode_utf16() {
        out.u16(unit);
    }
    out.u16(0);
    Ok(())
}

/// A GX list's items as stored, without the terminator.
fn gx_items(material: &Material) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for item in &material.gx {
        let length = u32::try_from(item.data.len().saturating_add(12))
            .map_err(|_| invalid(FORMAT, 0, "a GX item longer than 32 bits"))?;
        bytes.extend_from_slice(&item.id);
        bytes.extend_from_slice(&item.version.to_le_bytes());
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(&item.data);
    }
    Ok(bytes)
}

/// How many channel entries (UV sets, tangents, colours) a stored member takes from a vertex.
fn takes(member: &super::Member) -> (usize, usize, usize) {
    match member.semantic {
        5 => (if matches!(member.kind, 3 | 0x16) { 2 } else { 1 }, 0, 0),
        6 => (0, 1, 0),
        10 => (0, 0, 1),
        _ => (0, 0, 0),
    }
}

impl Model {
    /// Write the model as a FLVER file, with the file's [`Layout`]. A model that [`Model::read`] returned, written with the
    /// [`Layout::read`] of the same bytes, writes back to them, for every FLVER of both installs the reader takes. Every
    /// vertex attribute is written in the encoding its layout member states and read back by the reader; a value that does
    /// not fit its encoding, a vertex that lacks an attribute its layout stores, a mesh whose buffers disagree with each
    /// other, and anything that does not fit a 32-bit offset or count are refused, naming the mesh.
    ///
    /// The vertices' derived fields are not written: a SpeedTree vertex is written from [`Vertex::speedtree`] (its position,
    /// normal, tangent and UV are derived from those), and [`Vertex::layer_bytes`] only repeats the UV lanes it names.
    ///
    /// [`Vertex::speedtree`]: super::Vertex::speedtree
    /// [`Vertex::layer_bytes`]: super::Vertex::layer_bytes
    pub fn write(&self, layout: &Layout) -> Result<Vec<u8>> {
        if !known_version(self.version) {
            return Err(invalid(
                FORMAT,
                8,
                format!("unsupported version {:#x}", self.version),
            ));
        }
        let mismatch = |what: &str| invalid(FORMAT, 0, format!("the layout does not fit the model: {what}"));
        if layout.mesh_bounds.len() != self.meshes.len() {
            return Err(mismatch("one set of bounding box words per mesh"));
        }
        let total_faces: usize = self.meshes.iter().map(|m| m.faces.len()).sum();
        if layout.face_set_widths.len() != total_faces {
            return Err(mismatch("one index width per face set"));
        }
        if let Some(gx) = &layout.gx
            && gx.offsets.len() != self.materials.len()
        {
            return Err(mismatch("one GX list offset per material"));
        }
        let total_buffers: usize = self.meshes.iter().map(|m| m.layouts.len()).sum();
        let total_textures: usize = self.materials.iter().map(|m| m.textures.len()).sum();
        for (i, mesh) in self.meshes.iter().enumerate() {
            if mesh.material >= self.materials.len() {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!("mesh {i} names material {}", mesh.material),
                ));
            }
            if mesh.layouts.iter().any(|&l| l >= self.layouts.len()) {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!("mesh {i} names a vertex layout the model lacks"),
                ));
            }
            let empty = mesh.faces.is_empty() && mesh.layouts.is_empty();
            if !empty && !(1..=3).contains(&mesh.layouts.len()) {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!("mesh {i} has {} vertex buffers", mesh.layouts.len()),
                ));
            }
        }

        let mut out = Out::new(FORMAT, false);
        let speedtree = self.speedtree;
        out.bytes(b"FLVER\0L\0");
        out.u32(self.version);
        out.u32(0);
        out.u32(0);
        for n in [
            self.dummies.len(),
            self.materials.len(),
            self.bones.len(),
            self.meshes.len(),
            total_buffers,
        ] {
            let n = offset(&out, n, "table size")?;
            out.u32(n);
        }
        layout.bounds.iter().flatten().for_each(|&x| out.f32(x));
        out.u32(layout.face_counts[0]);
        out.u32(layout.face_counts[1]);
        out.u8(layout.index_width);
        out.u8(1);
        out.zeros(2);
        out.u32(0);
        for n in [total_faces, self.layouts.len(), total_textures] {
            let n = offset(&out, n, "table size")?;
            out.u32(n);
        }
        out.u8(0);
        out.u8(layout.header_5d);
        out.zeros(2);
        out.zeros(8);
        out.u16(layout.header_68);
        out.u16(if speedtree { 0x8000 } else { 0 });
        out.zeros(20);

        for d in &self.dummies {
            d.position.iter().for_each(|&x| out.f32(x));
            out.bytes(&d.color);
            d.forward.iter().for_each(|&x| out.f32(x));
            out.u16(d.reference_id as u16);
            out.u16(d.parent_bone as u16);
            d.upward.iter().for_each(|&x| out.f32(x));
            out.u16(d.attach_bone as u16);
            out.u8(u8::from(d.flag1));
            out.u8(u8::from(d.use_upward_vector));
            out.i32(d.unk30);
            out.i32(d.unk34);
            out.zeros(8);
        }
        let materials_at = out.len();
        let mut first_texture = 0usize;
        for m in &self.materials {
            out.u32(0);
            out.u32(0);
            out.u32(offset(&out, m.textures.len(), "texture count")?);
            out.u32(offset(&out, first_texture, "texture index")?);
            out.u32(m.string_bytes);
            out.u32(0);
            out.i32(m.index);
            out.u32(0);
            first_texture += m.textures.len();
        }
        let bones_at = out.len();
        for b in &self.bones {
            b.translation.iter().for_each(|&x| out.f32(x));
            out.u32(0);
            b.rotation.iter().for_each(|&x| out.f32(x));
            out.u16(b.parent as u16);
            out.u16(b.child as u16);
            b.scale.iter().for_each(|&x| out.f32(x));
            out.u16(b.next_sibling as u16);
            out.u16(b.previous_sibling as u16);
            b.bounds_min.iter().for_each(|&x| out.f32(x));
            out.u32(b.flags);
            b.bounds_max.iter().for_each(|&x| out.f32(x));
            out.zeros(52);
        }
        let meshes_at = out.len();
        for m in &self.meshes {
            out.u8(u8::from(m.dynamic));
            out.zeros(3);
            out.u32(offset(&out, m.material, "material index")?);
            out.zeros(8);
            out.i32(m.node);
            out.u32(offset(&out, m.palette.len(), "palette size")?);
            out.u32(0);
            out.u32(0);
            out.u32(offset(&out, m.faces.len(), "face set count")?);
            out.u32(0);
            out.u32(offset(&out, m.layouts.len(), "vertex buffer count")?);
            out.u32(0);
        }
        let face_sets_at = out.len();
        let mut widths = layout.face_set_widths.iter();
        let mut face_widths = Vec::with_capacity(total_faces);
        for m in &self.meshes {
            for f in &m.faces {
                let width = *widths
                    .next()
                    .ok_or_else(|| mismatch("one index width per face set"))?;
                if !matches!(width, 0 | 16 | 32) {
                    return Err(invalid(FORMAT, 0, format!("a face set index width of {width}")));
                }
                let effective = if width == 0 {
                    u32::from(layout.index_width)
                } else {
                    width
                };
                if !matches!(effective, 16 | 32) {
                    return Err(invalid(FORMAT, 0, "a face set with no index width"));
                }
                face_widths.push(effective);
                out.u32(f.flags);
                out.u8(u8::from(f.strip));
                out.u8(u8::from(f.cull));
                out.u16(f.unknown_06);
                out.u32(offset(&out, f.indices.len(), "index count")?);
                out.u32(0);
                out.u32(offset(
                    &out,
                    f.indices.len() * (effective as usize / 8),
                    "index bytes",
                )?);
                out.u32(0);
                out.u32(width);
                out.u32(0);
            }
        }
        let buffers_at = out.len();
        for m in &self.meshes {
            for (ordinal, &l) in m.layouts.iter().enumerate() {
                let members = self.layouts.get(l).map(Vec::as_slice).unwrap_or_default();
                let stride: usize = members
                    .iter()
                    .filter(|x| x.stored)
                    .map(|x| vertex::size(x.kind))
                    .sum::<Result<usize>>()?;
                out.u32(offset(&out, ordinal, "buffer ordinal")?);
                out.u32(offset(&out, l, "layout index")?);
                out.u32(offset(&out, stride, "vertex size")?);
                out.u32(offset(&out, m.vertices.len(), "vertex count")?);
                out.zeros(8);
                out.u32(offset(&out, stride * m.vertices.len(), "buffer bytes")?);
                out.u32(0);
            }
        }
        let layouts_at = out.len();
        for members in &self.layouts {
            out.u32(offset(&out, members.len(), "member count")?);
            out.zeros(8);
            out.u32(0);
        }
        let textures_at = out.len();
        for m in &self.materials {
            for t in &m.textures {
                out.u32(0);
                out.u32(0);
                t.scale.iter().for_each(|&x| out.f32(x));
                let [u, w] = t.tiling;
                out.u8(u as u8);
                out.u8(w as u8);
                out.zeros(2);
                t.unknown.iter().for_each(|&x| out.f32(x));
            }
        }
        if self.version == 0x2001a {
            let set = layout.skeletons.clone().unwrap_or_default();
            let record = out.len();
            out.u16(
                u16::try_from(set.base.len())
                    .map_err(|_| invalid(FORMAT, 0, "more skeleton bones than 16 bits"))?,
            );
            out.u16(
                u16::try_from(set.all.len())
                    .map_err(|_| invalid(FORMAT, 0, "more skeleton bones than 16 bits"))?,
            );
            out.zeros(28);
            for (slot, list) in [(4, &set.base), (8, &set.all)] {
                out.patch_u32(record + slot, offset(&out, out.len(), "skeleton offset")?)?;
                for b in list {
                    out.i16(b.parent);
                    out.i16(b.child);
                    out.i16(b.next_sibling);
                    out.i16(b.previous_sibling);
                    out.i32(b.node);
                    out.u32(0);
                }
            }
        }

        out.align(16);
        for (l, members) in self.layouts.iter().enumerate() {
            out.patch_u32(
                layouts_at + 16 * l + 12,
                offset(&out, out.len(), "member offset")?,
            )?;
            for x in members {
                out.u32(x.unknown);
                out.u32(offset(&out, x.offset, "member offset")?);
                out.u32(x.kind);
                out.u32(x.semantic);
                out.u32(x.index);
            }
        }
        out.align(16);
        let words = if self.version == 0x2001a { 9 } else { 6 };
        for (i, bounds) in layout.mesh_bounds.iter().enumerate() {
            let Some(bounds) = bounds else { continue };
            if bounds.len() != words {
                return Err(mismatch("six bounding box words (nine in version 0x2001a)"));
            }
            out.patch_u32(
                meshes_at + 48 * i + 24,
                offset(&out, out.len(), "bounding box offset")?,
            )?;
            bounds.iter().for_each(|&w| out.u32(w));
        }
        out.align(16);
        let palettes_at = out.len();
        for (i, m) in self.meshes.iter().enumerate() {
            let at = if m.palette.is_empty() {
                palettes_at
            } else {
                out.len()
            };
            out.patch_u32(meshes_at + 48 * i + 28, offset(&out, at, "palette offset")?)?;
            m.palette.iter().for_each(|&b| out.u32(b));
        }
        out.align(16);
        let mut next_face = 0usize;
        for (i, m) in self.meshes.iter().enumerate() {
            out.patch_u32(
                meshes_at + 48 * i + 36,
                offset(&out, out.len(), "face set list offset")?,
            )?;
            for k in 0..m.faces.len() {
                out.u32(offset(&out, next_face + k, "face set index")?);
            }
            next_face += m.faces.len();
        }
        out.align(16);
        let mut next_buffer = 0usize;
        for (i, m) in self.meshes.iter().enumerate() {
            out.patch_u32(
                meshes_at + 48 * i + 44,
                offset(&out, out.len(), "buffer list offset")?,
            )?;
            for k in 0..m.layouts.len() {
                out.u32(offset(&out, next_buffer + k, "buffer index")?);
            }
            next_buffer += m.layouts.len();
        }
        out.align(16);

        // the GX area: the stored one where every material's list is in it as it stands, else each distinct list once
        let area_at = out.len();
        let mut lists: Vec<Option<usize>> = vec![None; self.materials.len()];
        let items: Vec<Vec<u8>> = self.materials.iter().map(gx_items).collect::<Result<_>>()?;
        let stored = layout.gx.as_ref().filter(|gx| {
            gx.offsets.iter().zip(&items).all(|(at, bytes)| match at {
                Some(at) => gx.bytes.get(*at..at.saturating_add(bytes.len())) == Some(bytes.as_slice()),
                None => bytes.is_empty(),
            })
        });
        if let Some(gx) = stored {
            out.bytes(&gx.bytes);
            for (slot, at) in lists.iter_mut().zip(&gx.offsets) {
                *slot = at.map(|at| area_at + at);
            }
        } else {
            let mut written: Vec<(&Vec<u8>, usize)> = Vec::new();
            for (slot, bytes) in lists.iter_mut().zip(&items) {
                if bytes.is_empty() {
                    continue;
                }
                let at = match written.iter().find(|(b, _)| *b == bytes) {
                    Some(&(_, at)) => at,
                    None => {
                        let at = out.len();
                        out.bytes(bytes);
                        out.u32(0x7fff_ffff);
                        out.u32(100);
                        // the SpeedTree models state the terminator's length without its 12 bytes of header
                        out.u32(if layout.header_68 == 5 { 0 } else { 12 });
                        written.push((bytes, at));
                        at
                    }
                };
                *slot = Some(at);
            }
        }
        for (i, at) in lists.iter().enumerate() {
            if let Some(at) = at {
                out.patch_u32(materials_at + 32 * i + 20, offset(&out, *at, "GX list offset")?)?;
            }
        }
        out.align(16);

        // the strings
        let mut texture = 0usize;
        for (i, m) in self.materials.iter().enumerate() {
            out.patch_u32(materials_at + 32 * i, offset(&out, out.len(), "string offset")?)?;
            string(&mut out, &m.name)?;
            out.patch_u32(
                materials_at + 32 * i + 4,
                offset(&out, out.len(), "string offset")?,
            )?;
            string(&mut out, &m.mtd)?;
            for t in &m.textures {
                out.patch_u32(
                    textures_at + 32 * texture,
                    offset(&out, out.len(), "string offset")?,
                )?;
                string(&mut out, &t.path)?;
                if speedtree {
                    out.u16(0);
                }
                out.patch_u32(
                    textures_at + 32 * texture + 4,
                    offset(&out, out.len(), "string offset")?,
                )?;
                string(&mut out, &t.kind)?;
                texture += 1;
            }
        }
        out.align(16);
        for (i, b) in self.bones.iter().enumerate() {
            out.patch_u32(bones_at + 128 * i + 12, offset(&out, out.len(), "string offset")?)?;
            string(&mut out, &b.name)?;
        }
        out.align(16);
        if speedtree {
            out.align(0x800);
        }
        let data_start = out.len();
        out.patch_u32(12, offset(&out, data_start, "data offset")?)?;

        // the data
        let mut face_index = 0usize;
        let mut buffer_index = 0usize;
        for (mi, m) in self.meshes.iter().enumerate() {
            let first_face = face_index;
            let first_buffer = buffer_index;
            face_index += m.faces.len();
            buffer_index += m.layouts.len();
            let faces = |out: &mut Out| -> Result<()> {
                for (k, f) in m.faces.iter().enumerate() {
                    out.align(16);
                    let width = face_widths.get(first_face + k).copied().unwrap_or(16);
                    out.patch_u32(
                        face_sets_at + 32 * (first_face + k) + 12,
                        offset(out, out.len() - data_start, "index offset")?,
                    )?;
                    for &i in &f.indices {
                        if width == 16 {
                            out.u16(u16::try_from(i).map_err(|_| {
                                invalid(
                                    FORMAT,
                                    0,
                                    format!("mesh {mi}: index {i} does not fit a 16-bit face set"),
                                )
                            })?);
                        } else {
                            out.u32(i);
                        }
                    }
                }
                Ok(())
            };
            let buffers = |out: &mut Out| -> Result<()> {
                write_buffers(out, self, mi, m, first_buffer, buffers_at, data_start)
            };
            if layout.vertex_buffers_first {
                buffers(&mut out)?;
                faces(&mut out)?;
            } else {
                faces(&mut out)?;
                buffers(&mut out)?;
            }
        }
        out.align(16);
        let data_len = out.len() - data_start;
        out.patch_u32(16, offset(&out, data_len, "data size")?)?;
        if speedtree {
            out.align(0x800);
        }
        Ok(out.finish())
    }
}

/// Mesh `mi`'s vertex buffers: each padded to 16 bytes, its offset (from the data start) patched into buffer `first + k`'s
/// record, and its vertices encoded member by member.
fn write_buffers(
    out: &mut Out,
    model: &Model,
    mi: usize,
    mesh: &Mesh,
    first: usize,
    buffers_at: usize,
    data_start: usize,
) -> Result<()> {
    let fail = |out: &Out, why: &str| out.err(format!("mesh {mi}: {why}"));
    let mut cursors = encode::Cursors::default();
    for (k, &l) in mesh.layouts.iter().enumerate() {
        let members = model.layouts.get(l).map(Vec::as_slice).unwrap_or_default();
        out.align(16);
        out.patch_u32(
            buffers_at + 32 * (first + k) + 28,
            offset(out, out.len() - data_start, "buffer offset")?,
        )?;
        let stride: usize = members
            .iter()
            .filter(|x| x.stored)
            .map(|x| vertex::size(x.kind))
            .sum::<Result<usize>>()?;
        for (vi, v) in mesh.vertices.iter().enumerate() {
            let mut local = cursors;
            let mut bytes = Vec::with_capacity(stride);
            let mut stored = 0usize;
            for member in members {
                if !member.stored {
                    continue;
                }
                // the reader takes a SpeedTree model's stored members in order, whatever offsets its layout states
                if !model.speedtree && bytes.len() != member.offset {
                    return Err(fail(out, "a member does not sit where its layout puts it"));
                }
                let written = if model.speedtree {
                    let r = encode::speedtree_member(&mut bytes, v, member, stored);
                    stored += 1;
                    r
                } else {
                    encode::member(&mut bytes, v, member, &mut local, 2048.)
                };
                written.map_err(|why| fail(out, &format!("vertex {vi}: {why}")))?;
            }
            if bytes.len() != stride {
                return Err(fail(out, "a vertex does not fill its stride"));
            }
            out.bytes(&bytes);
        }
        // the next buffer's members continue the channels where this one's stopped
        for member in members.iter().filter(|x| x.stored) {
            let (uv, tangent, color) = takes(member);
            cursors.uv += uv;
            cursors.tangent += tangent;
            cursors.color += color;
        }
    }
    Ok(())
}
