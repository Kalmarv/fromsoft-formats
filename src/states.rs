//! Part states from a block's event script: which parts the game enables under which event flags.
//!
//! A block's script initialises common functions once per entity or entity group, through `2000:6`
//! (InitializeCommonEvent) with exactly seven i32 arguments: the function, the entity, four flags and the four states
//! packed as bytes. The functions 20004110 (map pieces, `2012:1`), 20004112 (objects, `2005:3`) and 20004114
//! (collisions, `2011:1`) disable the entity, then enable it unless one of the (flag, state) pairs holds (flag 0: an
//! unused pair), and restart when a flag changes; so a part is present while every pair fails. The polarity was checked
//! on Sekiro's castle (m11_01): the walls with the charcoal layer are the group hidden while flag 8302 is off, and a
//! live save with 8301, 8302 and 8306 off shows the intact walls.
//!
//! These are Sekiro's functions and Sekiro's layout of the call: in Elden Ring's scripts `2000:6` takes a slot number
//! (below 100) before the function, and the numbers 20004110, 20004112 and 20004114 are no part-state functions, so
//! no call of an Elden Ring script is decided here and the calls are counted by the function they name.
//!
//! Calls that cannot be decided
//! statically (another argument count, or arguments filled by parameter substitution), other common functions that
//! touch parts, and state instructions with literal entities in story events are not interpreted: the parts they name
//! keep their default presence and are counted in the report.

use std::collections::{BTreeMap, BTreeSet};

use crate::Game;
use crate::emevd::Script;
use crate::msb::{Part, PartKind};

/// The common functions that switch a part, by the part kind they address.
const STATE_FUNCTIONS: [(i32, PartKind); 3] = [
    (20004110, PartKind::MapPiece),
    (20004112, PartKind::Object),
    (20004114, PartKind::Collision),
];
/// Instructions that switch a part directly: `2012:1` map piece, `2005:3` object, `2011:1` collision state.
const DIRECT: [(i32, i32); 3] = [(2012, 1), (2005, 3), (2011, 1)];

/// One decided rule: the entity is disabled while any of the pairs holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The ID of the event that makes the call.
    pub event: i64,
    /// The common function called (`20004110`, `20004112` or `20004114`).
    pub function: i32,
    /// The entity or entity group it switches.
    pub entity: i32,
    /// (flag, state): disabled while the flag has this state (true: on).
    pub disabled_when: Vec<(i32, bool)>,
}

/// A script's part-state rules, and what it leaves open.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    rules: BTreeMap<(PartKindKey, i32), Vec<Rule>>,
    /// Every ID an uninterpreted call or instruction names.
    open: BTreeSet<i32>,
    /// Decided rules per function.
    pub decided: BTreeMap<i32, usize>,
    /// Part-state calls not interpreted: wrong argument count or arguments filled by substitution.
    pub undecidable_calls: usize,
    /// Calls of other common functions per function (their arguments may name parts).
    pub other_calls: BTreeMap<i32, usize>,
    /// Direct state instructions (story events); those with a literal entity name it as uninterpreted.
    pub direct_instructions: usize,
}

/// `PartKind` as a map key.
type PartKindKey = u8;
fn key(kind: PartKind) -> PartKindKey {
    match kind {
        PartKind::MapPiece => 0,
        PartKind::Object => 1,
        PartKind::Collision => 2,
        _ => 255,
    }
}

impl Rules {
    /// The rules of one script of `game`. Sekiro's `2000:6` calls name the function first; Elden Ring's name a slot
    /// first, and none of its functions is a part-state function (every call is counted in
    /// [`Rules::other_calls`] by the function it names).
    pub fn read(script: &Script, game: Game) -> Self {
        let mut s = Rules::default();
        let (slot_words, state_functions): (usize, &[(i32, PartKind)]) = match game {
            Game::Sekiro => (0, &STATE_FUNCTIONS),
            Game::EldenRing => (1, &[]),
        };
        for e in &script.events {
            for (k, ins) in e.instructions.iter().enumerate() {
                let substituted = e.parameters.iter().any(|p| p.instruction == k);
                // the complete little-endian words; a trailing partial word makes a known call undecidable
                let (words, rest) = ins.args.as_chunks::<4>();
                let args: Vec<i32> = words.iter().map(|&w| i32::from_le_bytes(w)).collect();
                let aligned = rest.is_empty();
                if DIRECT.contains(&(ins.bank, ins.id)) {
                    s.direct_instructions = s.direct_instructions.saturating_add(1);
                    if !substituted && let Some(&id) = args.first() {
                        s.open.insert(id);
                    }
                    continue;
                }
                if (ins.bank, ins.id) != (2000, 6) {
                    continue;
                }
                let Some((&function, tail)) = args.get(slot_words..).and_then(<[i32]>::split_first) else {
                    continue;
                };
                let Some(&(_, kind)) = state_functions.iter().find(|(f, _)| *f == function) else {
                    {
                        let calls = s.other_calls.entry(function).or_default();
                        *calls = calls.saturating_add(1);
                    }
                    s.open.extend(tail.iter().copied().filter(|&x| x > 0));
                    continue;
                };
                let (true, false, &[entity, d0, d1, d2, d3, packed]) = (aligned, substituted, tail) else {
                    s.undecidable_calls = s.undecidable_calls.saturating_add(1);
                    if let Some(&id) = tail.first() {
                        s.open.insert(id);
                    }
                    continue;
                };
                let disabled_when = [d0, d1, d2, d3]
                    .into_iter()
                    .zip(packed.to_le_bytes())
                    .filter(|&(id, _)| id != 0)
                    .map(|(id, state)| (id, state != 0))
                    .collect();
                let decided = s.decided.entry(function).or_default();
                *decided = decided.saturating_add(1);
                s.rules.entry((key(kind), entity)).or_default().push(Rule {
                    event: e.id,
                    function,
                    entity,
                    disabled_when,
                });
            }
        }
        s
    }

    /// The rules deciding a part (through its entity ID or any of its groups), in script order per ID.
    pub fn rules_for(&self, part: &Part) -> Vec<&Rule> {
        part.entity_ids()
            .flat_map(|id| self.rules.get(&(key(part.kind), id)).into_iter().flatten())
            .collect()
    }

    /// Whether something uninterpreted names one of the part's IDs, so it keeps its default presence.
    pub fn left_open(&self, part: &Part) -> bool {
        part.entity_ids().any(|id| self.open.contains(&id))
    }

    /// Every flag a rule names, in flag order.
    pub fn flags(&self) -> BTreeSet<i32> {
        self.rules
            .values()
            .flatten()
            .flat_map(|r| r.disabled_when.iter().map(|&(f, _)| f))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emevd::{Event, Instruction};

    fn call(arguments: &[i32]) -> Script {
        Script {
            events: vec![Event {
                id: 50,
                rest: 0,
                instructions: vec![Instruction {
                    bank: 2000,
                    id: 6,
                    args: arguments.iter().flat_map(|x| x.to_le_bytes()).collect(),
                    layer: None,
                }],
                parameters: Vec::new(),
            }],
            linked_files: Vec::new(),
            strings: Vec::new(),
        }
    }

    /// Sekiro's `2000:6` names the function first; Elden Ring's names a slot first, and the part-state function numbers
    /// are Sekiro's alone.
    #[test]
    fn the_function_follows_the_slot_in_elden_rings_calls() {
        let sekiro = Rules::read(&call(&[20004110, 1117204, 8302, 0, 0, 0, 0x01]), Game::Sekiro);
        assert_eq!(sekiro.decided, BTreeMap::from([(20004110, 1)]));
        assert!(sekiro.other_calls.is_empty());
        assert_eq!(sekiro.flags(), BTreeSet::from([8302]));
        // the same words read as Elden Ring's: slot 20004110 is no slot, so the call is another function's
        let elden = Rules::read(
            &call(&[7, 20004110, 1117204, 8302, 0, 0, 0, 0x01]),
            Game::EldenRing,
        );
        assert!(elden.decided.is_empty() && elden.flags().is_empty());
        assert_eq!(elden.other_calls, BTreeMap::from([(20004110, 1)]));
        // the entities the call names are left open, not the slot
        let part = |id| Part {
            index: 0,
            name: String::new(),
            kind: PartKind::MapPiece,
            model: None,
            hit_filter: None,
            npc_param: None,
            position: [0.; 3],
            rotation_degrees: [0.; 3],
            scale: [1.; 3],
            entity_id: id,
            entity_group_ids: [-1; 8],
            shadows: Default::default(),
            draw: Default::default(),
            tile_load: None,
        };
        assert!(elden.left_open(&part(1117204)) && elden.left_open(&part(8302)));
        assert!(!elden.left_open(&part(7)));
        // an Elden Ring call of no arguments after the slot, or of none at all, names no function
        assert!(Rules::read(&call(&[7]), Game::EldenRing).other_calls.is_empty());
        assert!(Rules::read(&call(&[]), Game::EldenRing).other_calls.is_empty());
    }
}
