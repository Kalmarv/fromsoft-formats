//! Observed ER MaterialExParam v4 rows and TAE 198 structural records.
//!
//! The row layout and byte-4 event link are the installed-data observations used by the source reader.
//! Unknown event words retain their unsigned and finite-float views; no transition envelope is inferred.
use crate::bytes::View;
use crate::param::Param;
use crate::{Game, Result, invalid};
use std::collections::BTreeMap;

/// One supported material parameter row, with every original byte retained.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct MaterialExRow {
    /// PARAM row number.
    pub id: i32,
    /// Stored UTF-16LE parameter name.
    pub parameter: String,
    /// Stored material identifier, including negative values.
    pub material_id: i32,
    /// Five stored lanes: RGBA and intensity, with no application rule.
    pub rgba_i: [f32; 5],
    /// Complete original row, including unnamed fields.
    pub bytes: Vec<u8>,
}

/// Decode only the observed MATERIAL_EX_PARAM_ST v4, 96-byte layout. Duplicate IDs are refused.
pub fn read_rows(table: &Param) -> Result<BTreeMap<i32, MaterialExRow>> {
    const FORMAT: &str = "MaterialExParam";
    if table.param_type != "MATERIAL_EX_PARAM_ST" || table.data_version != 4 || table.row_size != 96 {
        return Err(invalid(
            FORMAT,
            0,
            format!(
                "unobserved layout: {} v{} / {} bytes; only ER v4 / 96 is decoded",
                table.param_type, table.data_version, table.row_size
            ),
        ));
    }
    let mut out = BTreeMap::new();
    for row in &table.rows {
        let error = |at, reason: String| invalid(FORMAT, at, format!("row {}: {reason}", row.id));
        if row.data.len() != 96 {
            return Err(error(0, "row length is not 96".into()));
        }
        let view = View {
            data: &row.data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        let units: Vec<_> = view
            .slice(0, 64)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .take_while(|unit| *unit != 0)
            .collect();
        let parameter = String::from_utf16(&units).map_err(|reason| error(0, reason.to_string()))?;
        if parameter.trim().is_empty() {
            return Err(error(0, "empty parameter name".into()));
        }
        let mut rgba_i = [0.; 5];
        for (index, lane) in rgba_i.iter_mut().enumerate() {
            *lane = f32::from_bits(view.u32(view.element(68, index, 4)?)?);
        }
        if rgba_i.iter().any(|value| !value.is_finite()) {
            return Err(error(68, "non-finite stored lane".into()));
        }
        let parsed = MaterialExRow {
            id: row.id,
            parameter,
            material_id: i32::from_le_bytes(view.u32(64)?.to_le_bytes()),
            rgba_i,
            bytes: row.data.clone(),
        };
        if out.insert(row.id, parsed).is_some() {
            return Err(error(0, "duplicate row id".into()));
        }
    }
    Ok(out)
}

/// One complete event word; these are structural views, not named application semantics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventWord {
    /// Byte position of the word.
    pub offset: usize,
    /// Unsigned little-endian view.
    pub unsigned: u32,
    /// Float view, present only if finite.
    pub finite_float: Option<f32>,
}

/// Structural TAE 198 record, with the supported row link separated from unknown words.
#[derive(Clone, Debug, PartialEq)]
pub struct Event198 {
    /// All complete words, in byte order. The enclosing TAE retains trailing bytes.
    pub words: Vec<EventWord>,
    /// Byte-4 signed row link, only for the observed ER envelope.
    pub row_id: Option<i32>,
    /// Why the row link was not decoded, without discarding the structural views.
    pub refusal: Option<String>,
}
impl Event198 {
    /// Inspect ER's 64-byte record or its zero-padded 72-byte alias. Sekiro remains explicitly unproven.
    pub fn read(game: Game, bytes: &[u8]) -> Self {
        let words = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .zip((0..bytes.len()).step_by(4))
            .map(|(word, offset)| {
                let value = f32::from_le_bytes(*word);
                EventWord {
                    offset,
                    unsigned: u32::from_le_bytes(*word),
                    finite_float: value.is_finite().then_some(value),
                }
            })
            .collect();
        let refusal = if game != Game::EldenRing {
            Some("Sekiro event 198 row layout and envelope are unproven; raw event retained".into())
        } else if bytes.len() != 64
            && !(bytes.len() == 72
                && bytes
                    .get(64..)
                    .is_some_and(|tail| tail.iter().all(|byte| *byte == 0)))
        {
            Some(format!(
                "event 198 has {} bytes, not an observed 64-byte record or zero-padded 72-byte alias",
                bytes.len()
            ))
        } else {
            None
        };
        let row_id = if refusal.is_none() {
            bytes
                .get(4..8)
                .and_then(|word| word.try_into().ok())
                .map(i32::from_le_bytes)
        } else {
            None
        };
        Self {
            words,
            row_id,
            refusal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Hand-worked row: UTF-16LE logical name, id -1, stored RGBA zero and I one.
    #[test]
    fn verified_row_layout_keeps_all_bytes_and_refuses_other_revisions() {
        let mut bytes = vec![0; 96];
        for (at, unit) in "C[PixelKill]_Int".encode_utf16().enumerate() {
            bytes[at * 2..at * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        bytes[64..68].copy_from_slice(&(-1i32).to_le_bytes());
        bytes[84..88].copy_from_slice(&1f32.to_le_bytes());
        let mut table = Param {
            param_type: "MATERIAL_EX_PARAM_ST".into(),
            data_version: 4,
            unk06: 1,
            row_size: 96,
            rows: vec![crate::param::Row {
                id: 10300,
                name: None,
                data: bytes.clone(),
            }],
        };
        let rows = read_rows(&table).unwrap();
        let row = &rows[&10300];
        assert_eq!(row.parameter, "C[PixelKill]_Int");
        assert_eq!(row.material_id, -1);
        assert_eq!(row.rgba_i, [0., 0., 0., 0., 1.]);
        assert_eq!(row.bytes, bytes);
        table.data_version = 3;
        assert!(read_rows(&table).is_err());
        table.data_version = 4;
        table.rows[0].data[68..72].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(read_rows(&table).is_err());
    }
    #[test]
    fn event_word_views_keep_the_observed_link_and_unknown_envelope() {
        let mut bytes = vec![0; 64];
        bytes[4..8].copy_from_slice(&10300i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&0.5f32.to_le_bytes());
        let event = Event198::read(Game::EldenRing, &bytes);
        assert!(event.refusal.is_none());
        assert_eq!(event.row_id, Some(10300));
        assert!(
            event
                .words
                .iter()
                .any(|word| word.offset == 8 && word.finite_float == Some(0.5))
        );
        assert!(
            Event198::read(Game::Sekiro, &bytes)
                .refusal
                .unwrap()
                .contains("unproven")
        );
        assert!(Event198::read(Game::EldenRing, &bytes[..7]).refusal.is_some());
        bytes.extend_from_slice(&[0; 8]);
        assert!(Event198::read(Game::EldenRing, &bytes).refusal.is_none());
        bytes[71] = 1;
        assert!(Event198::read(Game::EldenRing, &bytes).refusal.is_some());
    }
}
