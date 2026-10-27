//! EDGE files (hang edges) as Sekiro stores them: grapple points, hang segments and wall-hug segments.
use crate::{Result, bytes::View, invalid};
/// What an edge record marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A grapple point (stored type 1).
    Grapple,
    /// A hang segment (stored type 2).
    Hang,
    /// A wall-hug segment (stored type 3).
    Hug,
}
/// One 64-byte EDGE record.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Its three stored points, as stored.
    pub points: [[f32; 3]; 3],
    /// The stored float at offset 44; its meaning is not established here.
    pub parameter: f32,
    /// The stored ID at offset 48.
    pub id: i32,
    /// What the record marks.
    pub kind: Kind,
    /// The stored byte at offset 53.
    pub variation: u8,
    /// The stored byte at offset 54.
    pub flags: u8,
}
/// A read EDGE file.
#[derive(Debug, Clone)]
pub struct Edges {
    /// The file's stored ID (header offset 8).
    pub id: i32,
    /// Its records, in file order.
    pub edges: Vec<Edge>,
}
impl Edges {
    /// Read an EDGE file: the version, the exact record count and size, and every unused field are checked; a
    /// non-finite float or unknown type is refused.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let v = View {
            data: bytes,
            format: "EDGE",
            be: false,
        };
        v.expect(0, &4u32.to_le_bytes())?;
        let count = v.u32(4)? as usize;
        let length = count
            .checked_mul(64)
            .and_then(|n| n.checked_add(16))
            .ok_or_else(|| invalid("EDGE", 4, "size overflow"))?;
        if length != bytes.len() {
            return Err(invalid("EDGE", 4, "record count does not cover file"));
        }
        v.expect(12, &[0; 4])?;
        let mut edges = Vec::with_capacity(count);
        for i in 0..count {
            let start = v.element(16, i, 64)?;
            let r = View {
                data: v.slice(start, 64)?,
                format: "EDGE record",
                be: false,
            };
            r.expect(12, &1f32.to_le_bytes())?;
            r.expect(28, &1f32.to_le_bytes())?;
            r.expect(55, &[0; 9])?;
            let kind = match r.u8(52)? {
                1 => Kind::Grapple,
                2 => Kind::Hang,
                3 => Kind::Hug,
                _ => return Err(invalid("EDGE", v.at(start, 52)?, "unknown edge type")),
            };
            edges.push(Edge {
                points: [vector(&r, 0)?, vector(&r, 16)?, vector(&r, 32)?],
                parameter: float(&r, 44)?,
                id: r.u32(48)? as i32,
                kind,
                variation: r.u8(53)?,
                flags: r.u8(54)?,
            });
        }
        Ok(Self {
            id: v.u32(8)? as i32,
            edges,
        })
    }
}
pub(crate) fn float(v: &View<'_>, offset: usize) -> Result<f32> {
    let value = f32::from_bits(v.u32(offset)?);
    if !value.is_finite() {
        return Err(invalid(v.format, offset, "nonfinite float"));
    }
    Ok(value)
}
pub(crate) fn vector(v: &View<'_>, offset: usize) -> Result<[f32; 3]> {
    Ok([
        float(v, offset)?,
        float(v, v.at(offset, 4)?)?,
        float(v, v.at(offset, 8)?)?,
    ])
}
