//! The observed Sekiro and Elden Ring `EquipParamWeapon` and `WepAbsorpPosParam` row layouts.
//!
//! Offsets follow the community Paramdex SDT/ER definitions at the installed row sizes (632/664 and 128/96 bytes).
//! The public records retain all eight trace slots and every stored dummy value, including zero and negative values.
//! They choose no equipped weapon, grip, animation or attachment policy.
use crate::bytes::View;
use crate::param::Row;
use crate::{Game, Result};

/// Number of trace records in a weapon row.
pub const TRACE_SLOTS: usize = 8;

/// Byte locations of the supported `EquipParamWeapon` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeaponLayout {
    pub size: usize,
    pub model: usize,
    pub motion_category: usize,
    pub hold_row: usize,
    pub sp_attack: Option<usize>,
    pub traces: usize,
}

/// Byte locations of the supported `WepAbsorpPosParam` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldLayout {
    pub size: usize,
    pub first: usize,
    pub models: usize,
    pub left_both: usize,
    pub shown: Option<usize>,
}

/// The observed layouts; no spelling, equipment or visibility choice is made.
pub fn layouts(game: Game) -> (WeaponLayout, HoldLayout) {
    match game {
        Game::EldenRing => (
            WeaponLayout {
                size: 664,
                model: 188,
                motion_category: 231,
                hold_row: 372,
                sp_attack: Some(238),
                traces: 272,
            },
            HoldLayout {
                size: 96,
                first: 8,
                models: 4,
                left_both: 52,
                shown: Some(60),
            },
        ),
        Game::Sekiro => (
            WeaponLayout {
                size: 632,
                model: 184,
                motion_category: 227,
                hold_row: 368,
                sp_attack: None,
                traces: 268,
            },
            HoldLayout {
                size: 128,
                first: 4,
                models: 6,
                left_both: 70,
                shown: None,
            },
        ),
    }
}

/// One stored `traceSfxId`, `traceDmyIdHead`, `traceDmyIdTail` triple.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Trace {
    pub effect: i32,
    pub head: i32,
    pub tail: i32,
}

/// Supported fields of one weapon row; even trace slots that name no effect are retained.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeaponRow {
    pub row: i32,
    pub model: u16,
    pub motion_category: u8,
    pub hold_row: i32,
    pub sp_attack_category: Option<u16>,
    pub traces: [Trace; TRACE_SLOTS],
}

fn view<'a>(row: &'a Row, size: usize, format: &'static str) -> Result<View<'a>> {
    let view = View {
        data: &row.data,
        format,
        be: false,
        base: 0,
    };
    if row.data.len() != size {
        return Err(view.err(
            0,
            format!(
                "row {} is {} bytes; the layout read is for {size}",
                row.id,
                row.data.len()
            ),
        ));
    }
    Ok(view)
}

impl WeaponLayout {
    /// Decode the supported fields of a row of exactly this layout's width.
    pub fn read(self, row: &Row) -> Result<WeaponRow> {
        let v = view(row, self.size, "EquipParamWeapon")?;
        let traces = (0..TRACE_SLOTS)
            .map(|slot| {
                let at = v.element(self.traces, slot, 12)?;
                Ok(Trace {
                    effect: i32::from_le_bytes(v.u32(at)?.to_le_bytes()),
                    head: i32::from_le_bytes(v.u32(v.at(at, 4)?)?.to_le_bytes()),
                    tail: i32::from_le_bytes(v.u32(v.at(at, 8)?)?.to_le_bytes()),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let traces = traces
            .try_into()
            .map_err(|_| v.err(self.traces, "trace slot count"))?;
        Ok(WeaponRow {
            row: row.id,
            model: v.u16(self.model)?,
            motion_category: v.u8(self.motion_category)?,
            hold_row: i32::from_le_bytes(v.u32(self.hold_row)?.to_le_bytes()),
            sp_attack_category: self.sp_attack.map(|at| v.u16(at)).transpose()?,
            traces,
        })
    }
}

/// Stored `dispPosType` meaning. Unknown values survive as their original byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum DisplayPosition {
    RightHand,
    LeftHand,
    Body,
    Other(u8),
}

impl DisplayPosition {
    pub fn of(value: u8) -> Self {
        match value {
            0 => Self::RightHand,
            1 => Self::LeftHand,
            3 => Self::Body,
            other => Self::Other(other),
        }
    }
}

/// One hold field, preserving its signed dummy value. No socket fallback or visibility is inferred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Hold {
    pub dummy: i16,
    pub shown: Option<DisplayPosition>,
}

/// Six authored ways to hold one model, in model order in the returned table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Holds {
    pub right: Hold,
    pub left: Hold,
    pub both: Hold,
    pub left_hang: Hold,
    pub right_hang: Hold,
    pub left_both: Hold,
}

impl HoldLayout {
    /// Decode every model and way, including nonpositive dummy values and unnamed display-position bytes.
    pub fn read(self, row: &Row) -> Result<Vec<Holds>> {
        let v = view(row, self.size, "WepAbsorpPosParam")?;
        v.slice(self.first, v.element(0, self.models, 10)?)?;
        v.slice(self.left_both, v.element(0, self.models, 2)?)?;
        if let Some(first) = self.shown {
            v.slice(first, v.element(0, self.models, 6)?)?;
        }
        (0..self.models)
            .map(|model| {
                let at = v.element(self.first, model, 10)?;
                let hold = |offset, way| -> Result<Hold> {
                    let shown = self
                        .shown
                        .map(|first| {
                            v.u8(v.at(v.element(first, model, 6)?, way)?)
                                .map(DisplayPosition::of)
                        })
                        .transpose()?;
                    Ok(Hold {
                        dummy: i16::from_le_bytes(v.u16(offset)?.to_le_bytes()),
                        shown,
                    })
                };
                Ok(Holds {
                    right: hold(at, 0)?,
                    left: hold(v.at(at, 2)?, 1)?,
                    both: hold(v.at(at, 4)?, 2)?,
                    left_hang: hold(v.at(at, 6)?, 5)?,
                    right_hang: hold(v.at(at, 8)?, 4)?,
                    left_both: hold(v.element(self.left_both, model, 2)?, 3)?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(size: usize) -> Row {
        Row {
            id: 42,
            name: None,
            data: vec![0; size],
        }
    }

    #[test]
    fn both_weapon_layouts_retain_inactive_trace_slots_and_refuse_truncation() {
        for game in [Game::Sekiro, Game::EldenRing] {
            let (layout, _) = layouts(game);
            let mut row = row(layout.size);
            row.data[layout.model..layout.model + 2].copy_from_slice(&550u16.to_le_bytes());
            row.data[layout.motion_category] = 29;
            row.data[layout.hold_row..layout.hold_row + 4].copy_from_slice(&123i32.to_le_bytes());
            if let Some(at) = layout.sp_attack {
                row.data[at..at + 2].copy_from_slice(&112u16.to_le_bytes());
            }
            row.data[layout.traces..layout.traces + 4].copy_from_slice(&(-1i32).to_le_bytes());
            row.data[layout.traces + 4..layout.traces + 8].copy_from_slice(&300i32.to_le_bytes());
            let decoded = layout.read(&row).unwrap();
            assert_eq!(
                (
                    decoded.row,
                    decoded.model,
                    decoded.motion_category,
                    decoded.hold_row
                ),
                (42, 550, 29, 123)
            );
            assert_eq!(
                decoded.sp_attack_category,
                if game == Game::EldenRing { Some(112) } else { None }
            );
            assert_eq!(
                decoded.traces[0],
                Trace {
                    effect: -1,
                    head: 300,
                    tail: 0
                }
            );
            assert_eq!(decoded.traces.len(), 8);
            row.data.pop();
            assert!(layout.read(&row).is_err());
        }
    }

    #[test]
    fn hold_layouts_keep_nonpositive_dummies_and_all_display_meanings() {
        for game in [Game::Sekiro, Game::EldenRing] {
            let (_, layout) = layouts(game);
            let mut row = row(layout.size);
            row.data[layout.first..layout.first + 2].copy_from_slice(&(-1i16).to_le_bytes());
            row.data[layout.first + 6..layout.first + 8].copy_from_slice(&2040i16.to_le_bytes());
            row.data[layout.left_both..layout.left_both + 2].copy_from_slice(&24i16.to_le_bytes());
            if let Some(at) = layout.shown {
                row.data[at..at + 6].copy_from_slice(&[0, 1, 3, 7, 3, 1]);
            }
            let decoded = layout.read(&row).unwrap();
            assert_eq!(decoded.len(), layout.models);
            assert_eq!(decoded[0].right.dummy, -1);
            assert_eq!(decoded[0].left.dummy, 0);
            assert_eq!(decoded[0].left_hang.dummy, 2040);
            assert_eq!(decoded[0].left_both.dummy, 24);
            if game == Game::EldenRing {
                assert_eq!(decoded[0].right.shown, Some(DisplayPosition::RightHand));
                assert_eq!(decoded[0].left.shown, Some(DisplayPosition::LeftHand));
                assert_eq!(decoded[0].both.shown, Some(DisplayPosition::Body));
                assert_eq!(decoded[0].left_both.shown, Some(DisplayPosition::Other(7)));
                assert_eq!(decoded[0].left_hang.shown, Some(DisplayPosition::LeftHand));
            } else {
                assert_eq!(decoded[0].right.shown, None);
            }
            row.data.pop();
            assert!(layout.read(&row).is_err());
        }
    }

    #[test]
    fn malformed_layout_counts_are_refused_before_allocating() {
        let (_, mut layout) = layouts(Game::EldenRing);
        let row = row(layout.size);
        layout.models = usize::MAX;
        assert!(layout.read(&row).is_err());
    }
}
