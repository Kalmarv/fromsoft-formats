//! Where a FLVER keeps its vertex and index bytes. [`Model::read`](super::Model::read) decodes them; this says where
//! they sit, which is what a reader needs to tie a running game's buffers to the file.
//!
//! Sekiro uploads a model's data block (the bytes from the header's data offset, for its data length) as one GPU buffer,
//! as stored, and draws each face set from it with the index buffer at offset 0 and the vertex buffers at their offsets:
//! a draw's first index times the index width is a face set's [`FaceSetBytes::offset`], and a bound vertex buffer's
//! offset is a [`VertexBufferBytes::offset`] (seen in captured frames of Sekiro 1.06, where every indexed draw of a
//! model drew one whole face set).
use super::{count, count_at, table};
use crate::{Result, bytes::View};

/// The index bytes of a face set, in file order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceSetBytes {
    /// The offset of its first index from the start of the data block.
    pub offset: usize,
    /// How many indices it holds.
    pub count: usize,
    /// The width of an index in bits: 16 or 32 (the header's when the record states 0).
    pub width: u32,
}

/// The bytes of a vertex buffer, in file order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexBufferBytes {
    /// The offset of its first vertex from the start of the data block.
    pub offset: usize,
    /// The bytes of a vertex.
    pub stride: usize,
    /// How many vertices it holds.
    pub count: usize,
    /// Its layout's index in [`Model::layouts`](super::Model::layouts).
    pub layout: usize,
    /// Its position among the buffers of the mesh that uses it (the record's first word).
    pub ordinal: u32,
}

/// The face sets and vertex buffers a mesh uses, as indices into [`Storage::face_sets`] and
/// [`Storage::vertex_buffers`], in the order the mesh lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshBytes {
    /// Its face sets.
    pub face_sets: Vec<usize>,
    /// Its vertex buffers, in buffer order.
    pub vertex_buffers: Vec<usize>,
}

/// Where a FLVER file keeps its vertex and index data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Storage {
    /// The data block's offset in the file.
    pub data_offset: usize,
    /// The data block's length.
    pub data_len: usize,
    /// Every face set, in file order (the order of [`Mesh::faces`](super::Mesh::faces) is the mesh's list).
    pub face_sets: Vec<FaceSetBytes>,
    /// Every vertex buffer, in file order.
    pub vertex_buffers: Vec<VertexBufferBytes>,
    /// Every mesh, in file order.
    pub meshes: Vec<MeshBytes>,
}

impl Storage {
    /// The storage of the FLVER file `data`. The records' offsets and lengths are bounded by the data block; the rest of
    /// the file is checked by [`Model::read`](super::Model::read), not here.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: "FLVER",
            be: false,
            base: 0,
        };
        v.expect(0, b"FLVER\0L\0")?;
        let data_offset = count(&v, 12)?;
        let data_len = count(&v, 16)?;
        v.slice(data_offset, data_len)?;
        let header_width = u32::from(v.u8(72)?);
        let (dummies, materials, bones, meshes, buffers, face_sets) = (
            count(&v, 20)?,
            count(&v, 24)?,
            count(&v, 28)?,
            count(&v, 32)?,
            count(&v, 36)?,
            count(&v, 80)?,
        );
        let mut pos = 128;
        table(&v, &mut pos, dummies, 64)?;
        table(&v, &mut pos, materials, 32)?;
        table(&v, &mut pos, bones, 128)?;
        let meshes_at = table(&v, &mut pos, meshes, 48)?;
        let face_sets_at = table(&v, &mut pos, face_sets, 32)?;
        let buffers_at = table(&v, &mut pos, buffers, 32)?;
        let within = |at: usize, offset: usize, len: usize| -> Result<()> {
            if offset.checked_add(len).is_none_or(|end| end > data_len) {
                return Err(v.err(at, "bytes outside the data block"));
            }
            Ok(())
        };
        let mut sets = Vec::with_capacity(face_sets);
        for i in 0..face_sets {
            let o = v.element(face_sets_at, i, 32)?;
            let width = match v.u32_at(o, 24)? {
                0 => header_width,
                w => w,
            };
            let (n, offset, len) = (count_at(&v, o, 8)?, count_at(&v, o, 12)?, count_at(&v, o, 16)?);
            within(o, offset, len)?;
            sets.push(FaceSetBytes {
                offset,
                count: n,
                width,
            });
        }
        let mut vbs = Vec::with_capacity(buffers);
        for i in 0..buffers {
            let o = v.element(buffers_at, i, 32)?;
            let (stride, n, len, offset) = (
                count_at(&v, o, 8)?,
                count_at(&v, o, 12)?,
                count_at(&v, o, 24)?,
                count_at(&v, o, 28)?,
            );
            within(o, offset, len)?;
            vbs.push(VertexBufferBytes {
                offset,
                stride,
                count: n,
                layout: count_at(&v, o, 4)?,
                ordinal: v.u32(o)?,
            });
        }
        let list = |o: usize, n_at: usize, at: usize, limit: usize| -> Result<Vec<usize>> {
            let n = count_at(&v, o, n_at)?;
            let mut p = count_at(&v, o, at)?;
            let start = table(&v, &mut p, n, 4)?;
            (0..n)
                .map(|k| {
                    let r = count(&v, v.element(start, k, 4)?)?;
                    if r >= limit {
                        return Err(v.err(start, "index out of range"));
                    }
                    Ok(r)
                })
                .collect()
        };
        let mut mesh_bytes = Vec::with_capacity(meshes);
        for i in 0..meshes {
            let o = v.element(meshes_at, i, 48)?;
            mesh_bytes.push(MeshBytes {
                face_sets: list(o, 32, 36, face_sets)?,
                vertex_buffers: list(o, 40, 44, buffers)?,
            });
        }
        Ok(Storage {
            data_offset,
            data_len,
            face_sets: sets,
            vertex_buffers: vbs,
            meshes: mesh_bytes,
        })
    }

    /// The data block of `data`, the file this storage was read from: the bytes Sekiro uploads as one buffer.
    pub fn data_block<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        data.get(self.data_offset..self.data_offset.checked_add(self.data_len)?)
    }

    /// The face set (its index in [`Storage::face_sets`]) whose indices start `offset` bytes into the data block, and
    /// the vertex buffer that starts at `vertex_offset`: what a draw of an index buffer and vertex buffer bound at those
    /// offsets of the data block draws.
    pub fn at(&self, offset: usize, vertex_offset: usize) -> (Option<usize>, Option<usize>) {
        (
            self.face_sets.iter().position(|f| f.offset == offset),
            self.vertex_buffers.iter().position(|b| b.offset == vertex_offset),
        )
    }
}
