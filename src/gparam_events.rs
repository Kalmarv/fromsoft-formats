//! The event instructions that switch which draw-param file (graphics environment) an area uses, read from a script
//! through [`crate::emedf`]. Sekiro's `2003:88` ("Set Area Gparam Sub ID") names an area's block, the sub id of the file to
//! use there and the seconds to blend over; Elden Ring's `2012:11` ("Activate Gparam Override") names a sub id and the
//! seconds to blend over, and `2012:12` ("Deactivate Gparam Override") ends the override. What the sub id does to the
//! game's choice of file, and which file an override names, is the game's: this module reads the calls, nothing more.
//!
//! What the stock scripts show (`tests/gparam_census.rs`): Sekiro's 43 calls name sub ids 0, 10, 20, 30, 40 and 50, and 35
//! of them name an area that has a file of exactly that number (`m11_00_0020` for sub id 20); Elden Ring's 11 activations
//! name 0, 1, 2, 3, 4 and 500, and of the 10 that belong to a map 6 name a file that exists (0 in the open-world tiles,
//! 500 in `m14_00`: `m14_00_0500`); the other 4 name 1 in `m11_10` (whose only file is 0000) and 4, 2 and 3 in three
//! expansion maps the base archives hold no draw-param file of.

use crate::Game;
use crate::emedf::{self, Typed, Value};
use crate::emevd::Script;

/// What a call switches. A field is `None` when an event parameter fills it in when the event starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Switch {
    /// Sekiro `2003:88`: the sub id of the draw-param file of the area (`area`, `block`).
    AreaSubId {
        /// The area (`11` of `m11_01`).
        area: Option<u8>,
        /// The block (`1`).
        block: Option<u8>,
        /// The sub id: the last two digits of the draw-param file id (0, 10, 20 ...).
        sub: Option<u8>,
        /// The seconds the change takes.
        seconds: Option<f32>,
    },
    /// Elden Ring `2012:11`: an override of the draw-param file of the area the event belongs to.
    Override {
        /// The instruction's `Gparam Sub ID`.
        id: Option<i32>,
        /// The seconds the change takes.
        seconds: Option<f32>,
    },
    /// Elden Ring `2012:12`: the override ends.
    ClearOverride {
        /// The seconds the change takes.
        seconds: Option<f32>,
    },
}

/// One call of such an instruction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Call {
    /// The id of the event that makes the call.
    pub event: i64,
    /// The index of the instruction in the event.
    pub instruction: usize,
    /// What it switches.
    pub switch: Switch,
}

/// The value of argument `index` of `typed`, unless an event parameter fills it in.
fn literal(typed: &Typed, index: usize) -> Option<Value> {
    typed.args.get(index).filter(|a| !a.substituted).map(|a| a.value)
}

fn seconds(value: Option<Value>) -> Option<f32> {
    match value {
        Some(Value::F32(v)) => Some(v),
        _ => None,
    }
}

/// Every call of the draw-param switching instructions of `script`, in script order. Instructions the schema of `game`
/// does not type are skipped.
pub fn calls(game: Game, script: &Script) -> Vec<Call> {
    let mut out = Vec::new();
    for event in &script.events {
        for (k, typed) in emedf::decode_event(game, event).into_iter().enumerate() {
            let Ok(typed) = typed else {
                continue;
            };
            let switch = match (game, typed.def.bank, typed.def.id) {
                (Game::Sekiro, 2003, 88) => Switch::AreaSubId {
                    area: literal(&typed, 0).and_then(Value::as_byte),
                    block: literal(&typed, 1).and_then(Value::as_byte),
                    sub: literal(&typed, 2).and_then(Value::as_byte),
                    seconds: seconds(literal(&typed, 3)),
                },
                (Game::EldenRing, 2012, 11) => Switch::Override {
                    id: literal(&typed, 0)
                        .and_then(Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok()),
                    seconds: seconds(literal(&typed, 1)),
                },
                (Game::EldenRing, 2012, 12) => Switch::ClearOverride {
                    seconds: seconds(literal(&typed, 0)),
                },
                _ => continue,
            };
            out.push(Call {
                event: event.id,
                instruction: k,
                switch,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emevd::{Event, Instruction, Parameter};

    fn script(instructions: Vec<Instruction>, parameters: Vec<Parameter>) -> Script {
        Script {
            events: vec![Event {
                id: 11_100_000,
                rest: 0,
                instructions,
                parameters,
            }],
            linked_files: Vec::new(),
            strings: Vec::new(),
        }
    }

    fn call(bank: i32, id: i32, args: Vec<u8>) -> Instruction {
        Instruction {
            bank,
            id,
            args,
            layer: None,
        }
    }

    #[test]
    fn sekiros_sub_id_call_names_the_block_the_sub_id_and_the_seconds() {
        let mut args = Vec::new();
        args.extend(11u32.to_le_bytes());
        args.extend(1u32.to_le_bytes());
        args.extend([20, 0, 0, 0]);
        args.extend(5.0f32.to_le_bytes());
        let s = script(vec![call(2003, 88, args)], Vec::new());
        assert_eq!(
            calls(Game::Sekiro, &s),
            [Call {
                event: 11_100_000,
                instruction: 0,
                switch: Switch::AreaSubId {
                    area: Some(11),
                    block: Some(1),
                    sub: Some(20),
                    seconds: Some(5.0),
                },
            }]
        );
        // the same bytes are no call in Elden Ring's schema, where 2003:88 is another instruction (or none)
        assert!(calls(Game::EldenRing, &s).is_empty());
    }

    #[test]
    fn a_parameter_filled_field_is_none() {
        let mut args = Vec::new();
        args.extend(11u32.to_le_bytes());
        args.extend(1u32.to_le_bytes());
        args.extend([0, 0, 0, 0]);
        args.extend(0.0f32.to_le_bytes());
        // the sub id byte (offset 8) is filled by the event's first argument
        let p = Parameter {
            instruction: 0,
            target: 8,
            source: 0,
            bytes: 1,
            unk: 0,
        };
        let s = script(vec![call(2003, 88, args)], vec![p]);
        let found = calls(Game::Sekiro, &s);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].switch,
            Switch::AreaSubId {
                area: Some(11),
                block: Some(1),
                sub: None,
                seconds: Some(0.0),
            }
        );
    }

    #[test]
    fn elden_rings_override_calls_name_a_sub_id_and_the_seconds() {
        let mut on = Vec::new();
        on.extend(500i32.to_le_bytes());
        on.extend(3.0f32.to_le_bytes());
        let off = 10.0f32.to_le_bytes().to_vec();
        let s = script(
            vec![call(2012, 11, on), call(1000, 0, Vec::new()), call(2012, 12, off)],
            Vec::new(),
        );
        let found = calls(Game::EldenRing, &s);
        assert_eq!(
            found
                .iter()
                .map(|c| (c.instruction, c.switch))
                .collect::<Vec<_>>(),
            [
                (
                    0,
                    Switch::Override {
                        id: Some(500),
                        seconds: Some(3.0),
                    }
                ),
                (2, Switch::ClearOverride { seconds: Some(10.0) }),
            ]
        );
    }
}
