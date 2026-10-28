//! Grass meshes (`.grass`): the triangle mesh a map piece's model carries to say where the game grows grass and of which
//! kind, found in `.mapbnd` and `.geombnd` binders next to the piece's FLVER (entry id 1200; Sekiro's 772 files, Elden
//! Ring's 1,925). A file holds no blade: the game scatters the grass itself over the mesh's triangles, from each
//! vertex's six densities, one for each of the six grass types a part's grass configuration names
//! (`msb::PartGrass`: six `GrassTypeParam` rows). The mesh comes with a binary tree of volumes over its triangles, for
//! culling.
//!
//! The layout is little-endian: a 40-byte header of ten words (`1`, `0x28`, then for each of the four tables its record
//! size and, but for the first, its count: volumes `0x14`, vertices `0x24`, faces `0x18`, boxes `0x18`, the box count equal
//! to the volume count), then the volumes, the vertices, the faces and the boxes, exactly filling the file.
//! The community's reader names a volume's first four words a child range and a face range; the files show a different
//! first half: a volume's first two words are its first and its second child (0 for none), not a range.
use crate::{
    Result,
    bytes::View,
    edge::{float, vector},
    invalid,
};
use std::ops::Range;

const FORMAT: &str = "GRASS";
const HEADER: usize = 0x28;
const VOLUME: usize = 0x14;
const VERTEX: usize = 0x24;
const FACE: usize = 0x18;
const BOX: usize = 0x18;

/// One volume of the tree over the triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    /// The index of its first child volume, if it has one (a child comes after its parent).
    pub first_child: Option<usize>,
    /// The index of its second child, if it has one.
    pub second_child: Option<usize>,
    /// The faces under it, as a range of [`Grass::faces`]. A volume with two children holds exactly theirs (the two ranges
    /// abut and fill it: 7,165 of Sekiro's volumes and 43,954 of Elden Ring's); a volume with one child holds more than
    /// that child does (242 and 19); a leaf holds the faces of its range.
    pub faces: Range<usize>,
    /// The grass types its faces' vertices grow, a bit for each of the six: bit n is set when a vertex of one of its faces
    /// has a density above 0 for type n. (The community's reader leaves this word unnamed.)
    pub grass_types: u32,
    /// The box that holds the vertices of its faces: its least and its greatest corner.
    pub bounds: [[f32; 3]; 2],
}

/// A point of the mesh with the density of each grass type there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// The position, in the model's space.
    pub position: [f32; 3],
    /// The densities of the six grass types (0 to 1 in the installs).
    pub densities: [f32; 6],
}

/// A triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    /// The stored normal. Its direction is against the right-hand cross product of the edges from the first vertex
    /// (cosine below -0.999 on 1,064,757 of Sekiro's 1,065,442 faces and 7,797,867 of Elden Ring's 7,799,267; 123 and 469
    /// are less aligned and the zero-area faces have no cross product), but it is not a unit vector: its length is 1 (to
    /// 0.001) on 88% of Sekiro's faces and 94% of Elden Ring's, from 0.1 to 5 on others and 0 on 532 and 674 faces. What
    /// the length stands for is not known.
    pub normal: [f32; 3],
    /// The indices of its three vertices in [`Grass::vertices`].
    pub vertices: [usize; 3],
}

/// A read grass mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Grass {
    /// The tree of volumes; the first is the root, which holds every face.
    pub volumes: Vec<Volume>,
    /// The mesh's vertices.
    pub vertices: Vec<Vertex>,
    /// The mesh's faces.
    pub faces: Vec<Face>,
}

fn count(v: &View<'_>, o: usize) -> Result<usize> {
    let n = v.u32(o)?;
    if n > i32::MAX as u32 {
        return Err(v.err(o, "negative count"));
    }
    usize::try_from(n).map_err(|_| v.err(o, "count"))
}

impl Grass {
    /// Read a `.grass` file. The header's constants, the table sizes against the file, every float being finite, every
    /// face vertex index and child index being in range, a child coming after its parent and being the child of one, a
    /// child's faces being within its parent's and the box count equalling the volume count are checked.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        v.expect(0, &1u32.to_le_bytes())?;
        v.expect(4, &(HEADER as u32).to_le_bytes())?;
        v.expect(8, &(VOLUME as u32).to_le_bytes())?;
        v.expect(16, &(VERTEX as u32).to_le_bytes())?;
        v.expect(24, &(FACE as u32).to_le_bytes())?;
        v.expect(32, &(BOX as u32).to_le_bytes())?;
        let volumes = count(&v, 12)?;
        let vertices = count(&v, 20)?;
        let faces = count(&v, 28)?;
        if count(&v, 36)? != volumes {
            return Err(v.err(36, "box count is not the volume count"));
        }
        let size = |n: usize, each: usize, at: usize| {
            n.checked_mul(each)
                .ok_or_else(|| v.err(at, "table size overflow"))
        };
        let volumes_at = HEADER;
        let vertices_at = v.at(volumes_at, size(volumes, VOLUME, 12)?)?;
        let faces_at = v.at(vertices_at, size(vertices, VERTEX, 20)?)?;
        let boxes_at = v.at(faces_at, size(faces, FACE, 28)?)?;
        let end = v.at(boxes_at, size(volumes, BOX, 36)?)?;
        if end != data.len() {
            return Err(v.err(
                end.min(data.len()),
                format!("the tables end at {end}, the file at {}", data.len()),
            ));
        }
        let mut out_vertices = Vec::with_capacity(vertices);
        for i in 0..vertices {
            let at = v.element(vertices_at, i, VERTEX)?;
            let mut densities = [0f32; 6];
            for (k, d) in densities.iter_mut().enumerate() {
                *d = float(&v, v.element(v.at(at, 12)?, k, 4)?)?;
            }
            out_vertices.push(Vertex {
                position: vector(&v, at)?,
                densities,
            });
        }
        let mut out_faces = Vec::with_capacity(faces);
        for i in 0..faces {
            let at = v.element(faces_at, i, FACE)?;
            let mut indices = [0usize; 3];
            for (k, index) in indices.iter_mut().enumerate() {
                let o = v.element(v.at(at, 12)?, k, 4)?;
                let n = count(&v, o)?;
                if n >= vertices {
                    return Err(v.err(o, format!("vertex {n} of {vertices}")));
                }
                *index = n;
            }
            out_faces.push(Face {
                normal: vector(&v, at)?,
                vertices: indices,
            });
        }
        let mut out_volumes: Vec<Volume> = Vec::with_capacity(volumes);
        let mut has_parent = vec![false; volumes];
        for i in 0..volumes {
            let at = v.element(volumes_at, i, VOLUME)?;
            let child = |delta: usize, has_parent: &mut Vec<bool>| -> Result<Option<usize>> {
                let o = v.at(at, delta)?;
                let c = count(&v, o)?;
                if c == 0 {
                    return Ok(None);
                }
                if c <= i || c >= volumes {
                    return Err(v.err(o, format!("child {c} of volume {i} of {volumes}")));
                }
                match has_parent.get_mut(c) {
                    Some(seen) if !*seen => *seen = true,
                    _ => return Err(v.err(o, format!("volume {c} is the child of two volumes"))),
                }
                Ok(Some(c))
            };
            let first_child = child(0, &mut has_parent)?;
            let second_child = child(4, &mut has_parent)?;
            let (start, stop) = (count(&v, v.at(at, 8)?)?, count(&v, v.at(at, 12)?)?);
            if start > stop || stop > faces {
                return Err(v.err(v.at(at, 8)?, format!("faces {start} to {stop} of {faces}")));
            }
            let b = v.element(boxes_at, i, BOX)?;
            let (lo, hi) = (vector(&v, b)?, vector(&v, v.at(b, 12)?)?);
            if lo.iter().zip(&hi).any(|(l, h)| l > h) {
                return Err(v.err(b, "a box whose least corner is past its greatest"));
            }
            out_volumes.push(Volume {
                first_child,
                second_child,
                faces: start..stop,
                grass_types: v.u32(v.at(at, 16)?)?,
                bounds: [lo, hi],
            });
        }
        for (i, volume) in out_volumes.iter().enumerate() {
            for c in [volume.first_child, volume.second_child].into_iter().flatten() {
                let inside = out_volumes.get(c).is_some_and(|child| {
                    child.faces.start >= volume.faces.start && child.faces.end <= volume.faces.end
                });
                if !inside {
                    return Err(v.err(
                        v.element(volumes_at, i, VOLUME)?,
                        format!("volume {c} holds faces its parent {i} does not"),
                    ));
                }
            }
        }
        if has_parent.iter().skip(1).any(|p| !p) {
            return Err(invalid(FORMAT, volumes_at, "a volume no volume has as a child"));
        }
        if let Some(root) = out_volumes.first()
            && root.faces != (0..faces)
        {
            return Err(invalid(
                FORMAT,
                volumes_at,
                "the root volume does not hold every face",
            ));
        }
        Ok(Self {
            volumes: out_volumes,
            vertices: out_vertices,
            faces: out_faces,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Grass;

    fn put_u32(b: &mut Vec<u8>, w: u32) {
        b.extend(w.to_le_bytes());
    }
    fn put_f32(b: &mut Vec<u8>, f: f32) {
        b.extend(f.to_le_bytes());
    }

    /// A mesh of one triangle with a root volume over it, `edit`ed.
    fn file(edit: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
        let mut b = Vec::new();
        for w in [1, 0x28, 0x14, 1, 0x24, 3, 0x18, 1, 0x18, 1] {
            put_u32(&mut b, w);
        }
        // the root volume: no children, the faces 0..1, grass type 0
        for w in [0, 0, 0, 1, 1] {
            put_u32(&mut b, w);
        }
        // three vertices, each the same density of type 0
        for (x, y) in [(0., 0.), (1., 0.), (0., 1.)] {
            for f in [x, y, 0., 1., 0., 0., 0., 0., 0.] {
                put_f32(&mut b, f);
            }
        }
        // the face: the normal against (1,0,0) x (0,1,0), vertices 0 1 2
        for f in [0., 0., -1.] {
            put_f32(&mut b, f);
        }
        for w in [0, 1, 2] {
            put_u32(&mut b, w);
        }
        for f in [0., 0., 0., 1., 1., 0.] {
            put_f32(&mut b, f);
        }
        edit(&mut b);
        b
    }

    #[test]
    fn a_mesh_reads_with_its_tree() {
        let g = Grass::read(&file(|_| {})).expect("a valid mesh");
        assert_eq!((g.volumes.len(), g.vertices.len(), g.faces.len()), (1, 3, 1));
        assert_eq!(g.volumes[0].faces, 0..1);
        assert_eq!(g.volumes[0].grass_types, 1);
        assert_eq!(g.vertices[1].position, [1., 0., 0.]);
        assert_eq!(g.vertices[0].densities, [1., 0., 0., 0., 0., 0.]);
        assert_eq!(g.faces[0].vertices, [0, 1, 2]);
        assert_eq!(g.volumes[0].bounds, [[0., 0., 0.], [1., 1., 0.]]);
    }

    #[test]
    fn what_breaks_the_layout_is_refused() {
        // a byte too many, a byte too few
        assert!(Grass::read(&file(|b| b.push(0))).is_err());
        assert!(
            Grass::read(&file(|b| {
                b.pop();
            }))
            .is_err()
        );
        // a header constant, the box count
        assert!(Grass::read(&file(|b| b[0] = 2)).is_err());
        assert!(Grass::read(&file(|b| b[36] = 2)).is_err());
        // a vertex index past the vertices; a non-finite density
        let faces_at = 40 + 20 + 3 * 36;
        assert!(Grass::read(&file(|b| b[faces_at + 12] = 3)).is_err());
        assert!(
            Grass::read(&file(
                |b| b[40 + 20 + 12..40 + 20 + 16].copy_from_slice(&f32::NAN.to_le_bytes())
            ))
            .is_err()
        );
        // a child that is not after its parent, a face range past the faces, a root that holds fewer faces
        assert!(Grass::read(&file(|b| b[40] = 1)).is_err());
        assert!(Grass::read(&file(|b| b[40 + 12] = 2)).is_err());
        assert!(Grass::read(&file(|b| b[40 + 8] = 1)).is_err());
        // a box whose corners are the wrong way round
        let box_at = faces_at + 24;
        assert!(
            Grass::read(&file(
                |b| b[box_at + 12..box_at + 16].copy_from_slice(&(-1f32).to_le_bytes())
            ))
            .is_err()
        );
    }
}
