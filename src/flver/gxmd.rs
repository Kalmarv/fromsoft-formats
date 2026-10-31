//! Version-123 GXMD item framing, as stored in FLVER material extension payloads.
//!
//! The framing is described by RunDevelopment's CC0 investigation:
//! <https://github.com/JKAnderson/SoulsFormats/issues/14>. Records carry an ID, a kind and
//! one, two, three or five words; kind zero is a boolean flag on a preceding value.
//! This module assigns no shader-specific meaning to IDs and preserves every payload word.
use std::collections::BTreeSet;

use super::GxItem;
use crate::{Result, bytes::View};

/// One framed GXMD record, in the payload's order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The stored value ID, also used by a flag referring to a preceding value.
    pub id: u32,
    /// The stored kind: zero for a flag; 1, 2, 3 or 11 for value records.
    pub kind: u32,
    /// The payload words without floating-point conversion, including a flag's boolean word.
    pub words: Vec<u32>,
}

/// The records of a recognized GXMD payload, retaining source order and raw value bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// Values and flags in their stored order.
    pub records: Vec<Record>,
}

impl GxItem {
    /// Read a version-123 GXMD payload. `None` means another ID/version or an unclassified
    /// record kind: the original item remains available and no partial interpretation is returned.
    /// Malformed recognized framing, duplicate values, and orphan or duplicate flags are errors.
    pub fn gxmd(&self) -> Result<Option<Metadata>> {
        if self.id != *b"GXMD" || self.version != 123 {
            return Ok(None);
        }
        read(&self.data)
    }
}

/// Read the payload of a version-123 GXMD item. Unknown kinds return `None`, not a partial
/// record list. Values remain raw words: their finiteness and shader meanings belong to a consumer.
pub fn read(data: &[u8]) -> Result<Option<Metadata>> {
    let view = View {
        data,
        format: "GXMD",
        be: false,
        base: 0,
    };
    if !data.len().is_multiple_of(4) || data.is_empty() {
        return Err(view.err(0, "unaligned or absent item count"));
    }
    let count = view.u32(0)?;
    let mut at = 4;
    let mut records = Vec::new();
    let mut values = BTreeSet::new();
    let mut flags = BTreeSet::new();
    for _ in 0..count {
        let header = view
            .sub(at, 8, "GXMD")
            .map_err(|_| view.err(at, "truncated item header"))?;
        let id = header.u32(0)?;
        let kind = header.u32(4)?;
        at = view.at(at, 8)?;
        let bytes = match kind {
            0 | 1 => 4,
            2 => 8,
            3 => 12,
            11 => 20,
            _ => return Ok(None),
        };
        let payload = view
            .slice(at, bytes)
            .map_err(|_| view.err(at, "truncated item value"))?;
        let words: Vec<u32> = payload
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        let first = view.u32(at)?;
        at = view.at(at, bytes)?;
        if kind == 0 {
            if first > 1 || !values.contains(&id) || !flags.insert(id) {
                return Err(view.err(at, "invalid, orphan or duplicate flag"));
            }
        } else if !values.insert(id) {
            return Err(view.err(at, "duplicate value ID"));
        }
        records.push(Record { id, kind, words });
    }
    if at != data.len() {
        return Err(view.err(at, "trailing data"));
    }
    Ok(Some(Metadata { records }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn every_recognized_kind_preserves_order_bits_and_flags() {
        let bytes = payload(&[
            6,
            91,
            2,
            0,
            0x7fc01234,
            237,
            1,
            0.5f32.to_bits(),
            237,
            0,
            1,
            92,
            3,
            1,
            2,
            3,
            93,
            11,
            4,
            5,
            6,
            7,
            8,
            94,
            1,
            9,
        ]);
        let item = GxItem {
            id: *b"GXMD",
            version: 123,
            data: bytes.clone(),
        };
        let result = item.gxmd().unwrap().unwrap();
        assert_eq!(
            result.records.iter().map(|r| (r.id, r.kind)).collect::<Vec<_>>(),
            [(91, 2), (237, 1), (237, 0), (92, 3), (93, 11), (94, 1)]
        );
        assert_eq!(result.records[0].words, [0, 0x7fc01234]);
        assert_eq!(result.records[1].words, [0.5f32.to_bits()]);
        assert_eq!(result.records[4].words, [4, 5, 6, 7, 8]);
        assert_eq!(item.data, bytes);
        for end in 0..bytes.len() {
            assert!(read(&bytes[..end]).is_err(), "truncation at {end}");
        }
    }

    #[test]
    fn ambiguity_is_refused_and_unknown_items_remain_uninterpreted() {
        for words in [
            vec![1, 1, 0, 1],
            vec![2, 1, 1, 0, 1, 1, 0],
            vec![2, 1, 1, 0, 1, 0, 2],
            vec![3, 1, 1, 0, 1, 0, 0, 1, 0, 1],
            vec![0, 0],
        ] {
            assert!(read(&payload(&words)).is_err(), "{words:?}");
        }
        assert!(read(&payload(&[1, 1, 99])).unwrap().is_none());
        let mut item = GxItem {
            id: *b"GXMD",
            version: 124,
            data: Vec::new(),
        };
        assert!(item.gxmd().unwrap().is_none());
        item.id = *b"GX01";
        item.version = 123;
        assert!(item.gxmd().unwrap().is_none());
        assert_eq!(read(&payload(&[0])).unwrap().unwrap().records, []);
    }
}
