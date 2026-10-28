//! The instruction schema of the games' event scripts: for each instruction (a bank and a number, the `2003:66` of an
//! event script) its name and its arguments, each with a name, a stored type and a role (an event flag, an entity id, a
//! part of a map id). [`crate::emevd`] reads a script's instructions as raw argument bytes; [`decode`] turns one into
//! typed arguments against this schema, and says by name why it refuses one.
//!
//! Names, types and enum names are the community's (the EMEDF files of DarkScript3, as Smithbox and soulstruct ship
//! them); the layouts are checked against every instruction of every stock script of both games (`tests/emedf_census.rs`:
//! none is refused by length). The roles are this crate's own, assigned from the argument names and checked on the same
//! scripts.
//!
//! **Layout.** Arguments are stored one after the other at their natural alignment (a `u32` at a multiple of four, a `u16`
//! at a multiple of two, a byte anywhere) and the block is padded to a multiple of four bytes. An instruction flagged
//! `vararg` ends with a `u32` that repeats: the event initialisers `2000:0` and `2000:6` carry one word per argument of
//! the event they start.
//!
//! **Substitution.** An event's [`Parameter`]s overwrite bytes of its instructions' arguments with bytes of its own
//! arguments when the event starts; an argument any of whose bytes a parameter overwrites is [`TypedArg::substituted`],
//! and its stored value is a placeholder.

use std::collections::BTreeMap;

use crate::Game;
use crate::emevd::{Event, Instruction, Parameter};
use crate::mapid::MapId;

mod elden;
mod sekiro;

/// How an argument is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArgKind {
    /// An unsigned byte.
    U8,
    /// A signed byte.
    I8,
    /// An unsigned little-endian 16-bit integer.
    U16,
    /// A signed little-endian 16-bit integer.
    I16,
    /// An unsigned little-endian 32-bit integer.
    U32,
    /// A signed little-endian 32-bit integer.
    I32,
    /// A little-endian 32-bit float.
    F32,
    /// A 32-bit offset into the script's string table (the community's string argument; no stock script uses one).
    Text,
}

impl ArgKind {
    /// The stored size in bytes, which is also the alignment.
    pub const fn size(self) -> usize {
        match self {
            ArgKind::U8 | ArgKind::I8 => 1,
            ArgKind::U16 | ArgKind::I16 => 2,
            ArgKind::U32 | ArgKind::I32 | ArgKind::F32 | ArgKind::Text => 4,
        }
    }
}

/// What an argument names in the world, as far as the schema says (assigned from the argument's name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// Nothing the crate reads: a condition group, a state, a count, a time, a message or a parameter row.
    Other,
    /// An event flag id (`Target Event Flag ID`, the ends and limits of a flag range, a response flag).
    Flag,
    /// The first flag of a run of flags (`Base Event Flag ID`), with [`Role::FlagBitCount`] beside it.
    FlagBase,
    /// How many bits a run of flags uses (`Number of Used Flag Bits`).
    FlagBitCount,
    /// An entity id: a part, a part group, a region or an event of a map's layout, or the player's.
    Entity,
    /// A map id packed as the decimal number `AABBCCDD` (`60444500` is `m60_44_45_00`); see [`MapId::from_decimal`].
    MapWord,
    /// The area byte of a map id given as separate bytes (file order: area, block, region, index).
    MapArea,
    /// The block byte (the grid X of a tile) of a map id given as separate bytes.
    MapBlock,
    /// The region byte (the grid Z of a tile) of a map id given as separate bytes.
    MapRegion,
    /// The index byte (the variant) of a map id given as separate bytes.
    MapIndex,
    /// The id of the event an initialiser starts or ends.
    EventId,
    /// The slot of the event an initialiser starts or ends.
    EventSlot,
}

/// One argument of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgDef {
    /// The community's name for it.
    pub name: &'static str,
    /// How it is stored.
    pub kind: ArgKind,
    /// What it names.
    pub role: Role,
    /// The community's name of the enumeration its value comes from, if it has one.
    pub enum_name: Option<&'static str>,
}

/// One instruction of the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionDef {
    /// The instruction's bank (the `2003` of `2003:66`).
    pub bank: i32,
    /// Its number within the bank (the `66`).
    pub id: i32,
    /// The community's name for it.
    pub name: &'static str,
    /// Whether the last argument repeats, zero or more times.
    pub vararg: bool,
    /// The arguments, in stored order; for a `vararg` instruction the last one is the repeating one.
    pub args: &'static [ArgDef],
}

/// `n` rounded up to a multiple of `to` (a power of two here).
fn align_up(n: usize, to: usize) -> usize {
    n.div_ceil(to).saturating_mul(to)
}

impl InstructionDef {
    /// The arguments stored once: all of them, except the repeating one of a `vararg` instruction.
    pub fn fixed(&self) -> &'static [ArgDef] {
        match (self.vararg, self.args.split_last()) {
            (true, Some((_, fixed))) => fixed,
            _ => self.args,
        }
    }

    /// The repeating argument of a `vararg` instruction.
    pub fn repeating(&self) -> Option<&'static ArgDef> {
        if self.vararg { self.args.last() } else { None }
    }

    /// The byte offset of each of [`InstructionDef::fixed`], and the size of the fixed part padded to four bytes.
    pub fn layout(&self) -> (Vec<usize>, usize) {
        let mut at = 0usize;
        let mut offsets = Vec::with_capacity(self.args.len());
        for arg in self.fixed() {
            let start = align_up(at, arg.kind.size());
            offsets.push(start);
            at = start.saturating_add(arg.kind.size());
        }
        (offsets, align_up(at, 4))
    }
}

/// The instructions of one game, sorted by bank and number.
#[derive(Clone, Copy, Debug)]
pub struct Schema {
    instructions: &'static [InstructionDef],
}

/// The schema of `game`'s event scripts.
pub fn schema(game: Game) -> Schema {
    Schema {
        instructions: match game {
            Game::Sekiro => sekiro::INSTRUCTIONS,
            Game::EldenRing => elden::INSTRUCTIONS,
        },
    }
}

impl Schema {
    /// Every instruction, by bank and number.
    pub fn instructions(self) -> &'static [InstructionDef] {
        self.instructions
    }

    /// The instruction `bank`:`id`, if the schema has it.
    pub fn get(self, bank: i32, id: i32) -> Option<&'static InstructionDef> {
        self.instructions
            .binary_search_by(|d| (d.bank, d.id).cmp(&(bank, id)))
            .ok()
            .and_then(|i| self.instructions.get(i))
    }
}

/// An argument's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// An unsigned byte.
    U8(u8),
    /// A signed byte.
    I8(i8),
    /// An unsigned 16-bit integer.
    U16(u16),
    /// A signed 16-bit integer.
    I16(i16),
    /// An unsigned 32-bit integer.
    U32(u32),
    /// A signed 32-bit integer.
    I32(i32),
    /// A float.
    F32(f32),
    /// A string-table offset.
    Text(u32),
}

impl Value {
    /// The integer value; `None` for a float.
    pub fn as_i64(self) -> Option<i64> {
        Some(match self {
            Value::U8(v) => i64::from(v),
            Value::I8(v) => i64::from(v),
            Value::U16(v) => i64::from(v),
            Value::I16(v) => i64::from(v),
            Value::U32(v) | Value::Text(v) => i64::from(v),
            Value::I32(v) => i64::from(v),
            Value::F32(_) => return None,
        })
    }

    /// The value as a byte, the way a map id stores its parts: a signed byte reads as its bit pattern (`-1` is `0xff`,
    /// the wildcard), a wider integer only when it fits.
    pub fn as_byte(self) -> Option<u8> {
        match self {
            Value::U8(v) => Some(v),
            Value::I8(v) => Some(u8::from_ne_bytes(v.to_ne_bytes())),
            Value::F32(_) => None,
            other => other.as_i64().and_then(|v| u8::try_from(v).ok()),
        }
    }

    /// The value as a 32-bit word with the same bits as were stored (a signed value reads as its bit pattern).
    pub fn as_word(self) -> Option<u32> {
        match self {
            Value::U32(v) | Value::Text(v) => Some(v),
            Value::I32(v) => Some(u32::from_ne_bytes(v.to_ne_bytes())),
            _ => None,
        }
    }
}

/// An argument read against its definition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypedArg {
    /// Its definition (for a word of a repeating argument, the repeating argument's).
    pub def: &'static ArgDef,
    /// Its byte offset in the instruction's arguments.
    pub offset: usize,
    /// Its stored value.
    pub value: Value,
    /// Whether an event parameter overwrites any of its bytes: the stored value is then a placeholder.
    pub substituted: bool,
}

/// An instruction read against the schema.
#[derive(Clone, Debug, PartialEq)]
pub struct Typed {
    /// The instruction's definition.
    pub def: &'static InstructionDef,
    /// Its arguments in stored order, the words of a repeating argument one by one.
    pub args: Vec<TypedArg>,
}

/// An event initialiser's call (`2000:0`, `2000:6`) read as what it starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Initialisation {
    /// The slot, where the instruction has one (Sekiro's `2000:6` has none).
    pub slot: Option<i32>,
    /// The id of the event it starts.
    pub event: u32,
    /// The words it passes as the event's arguments.
    pub words: Vec<u32>,
}

impl Typed {
    /// The arguments no event parameter overwrites.
    pub fn literal(&self) -> impl Iterator<Item = &TypedArg> {
        self.args.iter().filter(|a| !a.substituted)
    }

    /// The arguments of `role`, substituted or not.
    pub fn with_role(&self, role: Role) -> impl Iterator<Item = &TypedArg> {
        self.args.iter().filter(move |a| a.def.role == role)
    }

    /// The map this instruction names, when it names one in full: a [`Role::MapWord`] argument read as a decimal map
    /// number, or the separate area, block, region and index bytes (the schema may give only the first two, as Sekiro's
    /// instructions do; the others are then zero). `None` when it names none, a part is substituted or a value is not
    /// a map's.
    pub fn map(&self) -> Option<MapId> {
        if let Some(word) = self.with_role(Role::MapWord).next() {
            let value = word.value.as_i64().and_then(|v| i32::try_from(v).ok())?;
            return (!word.substituted).then(|| MapId::from_decimal(value)).flatten();
        }
        let part = |role: Role| -> Option<Option<u8>> {
            match self.with_role(role).next() {
                None => Some(None),
                Some(arg) if arg.substituted => None,
                Some(arg) => arg.value.as_byte().map(Some),
            }
        };
        let (area, block) = (part(Role::MapArea)??, part(Role::MapBlock)??);
        let region = part(Role::MapRegion)?.unwrap_or(0);
        let index = part(Role::MapIndex)?.unwrap_or(0);
        Some(MapId::from_bytes([area, block, region, index]))
    }

    /// What an event initialiser starts. `None` for any other instruction, and when the event id is substituted.
    pub fn initialisation(&self) -> Option<Initialisation> {
        let event = self.with_role(Role::EventId).next()?;
        if event.substituted {
            return None;
        }
        let slot = self
            .with_role(Role::EventSlot)
            .next()
            .and_then(|a| a.value.as_i64())
            .and_then(|v| i32::try_from(v).ok());
        let words = match self.def.repeating() {
            Some(repeating) => self
                .args
                .iter()
                .filter(|a| std::ptr::eq(a.def, repeating))
                .filter_map(|a| a.value.as_word())
                .collect(),
            None => Vec::new(),
        };
        Some(Initialisation {
            slot,
            event: event.value.as_word()?,
            words,
        })
    }
}

/// Why an instruction was not typed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The schema has no instruction of this bank and number.
    #[error("instruction {bank}:{id} is not in the schema")]
    Unknown {
        /// The instruction's bank.
        bank: i32,
        /// Its number.
        id: i32,
    },
    /// The argument bytes are not as long as the layout needs.
    #[error("instruction {bank}:{id} stores {found} argument bytes, its layout needs {expected}")]
    Length {
        /// The instruction's bank.
        bank: i32,
        /// Its number.
        id: i32,
        /// The bytes stored.
        found: usize,
        /// The bytes the layout needs.
        expected: usize,
    },
    /// A repeating argument's bytes are not at least the fixed part followed by whole words.
    #[error("instruction {bank}:{id} stores {found} argument bytes, {fixed} then whole words are needed")]
    RepeatLength {
        /// The instruction's bank.
        bank: i32,
        /// Its number.
        id: i32,
        /// The bytes stored.
        found: usize,
        /// The bytes of the fixed part.
        fixed: usize,
    },
}

impl Refusal {
    /// The reason in a few words, for counting refusals.
    pub fn reason(&self) -> &'static str {
        match self {
            Refusal::Unknown { .. } => "not in the schema",
            Refusal::Length { .. } => "argument bytes differ from the layout",
            Refusal::RepeatLength { .. } => "repeating argument is not whole words after the fixed part",
        }
    }
}

/// The value of an argument of `kind` at `offset` in `bytes`; `None` past the end.
fn read_value(bytes: &[u8], offset: usize, kind: ArgKind) -> Option<Value> {
    let end = offset.checked_add(kind.size())?;
    let b = bytes.get(offset..end)?;
    Some(match kind {
        ArgKind::U8 => Value::U8(*b.first()?),
        ArgKind::I8 => Value::I8(i8::from_le_bytes([*b.first()?])),
        ArgKind::U16 => Value::U16(u16::from_le_bytes(b.try_into().ok()?)),
        ArgKind::I16 => Value::I16(i16::from_le_bytes(b.try_into().ok()?)),
        ArgKind::U32 => Value::U32(u32::from_le_bytes(b.try_into().ok()?)),
        ArgKind::I32 => Value::I32(i32::from_le_bytes(b.try_into().ok()?)),
        ArgKind::F32 => Value::F32(f32::from_le_bytes(b.try_into().ok()?)),
        ArgKind::Text => Value::Text(u32::from_le_bytes(b.try_into().ok()?)),
    })
}

/// Read `instruction` against `game`'s schema. `parameters` are the parameters of its event that overwrite this
/// instruction's arguments (those whose [`Parameter::instruction`] is its index); [`decode_event`] picks them out.
pub fn decode(game: Game, instruction: &Instruction, parameters: &[Parameter]) -> Result<Typed, Refusal> {
    let (bank, id) = (instruction.bank, instruction.id);
    let def = schema(game).get(bank, id).ok_or(Refusal::Unknown { bank, id })?;
    let (offsets, fixed) = def.layout();
    let found = instruction.args.len();
    if def.vararg {
        if found < fixed || found.saturating_sub(fixed) % 4 != 0 {
            return Err(Refusal::RepeatLength {
                bank,
                id,
                found,
                fixed,
            });
        }
    } else if found != fixed {
        return Err(Refusal::Length {
            bank,
            id,
            found,
            expected: fixed,
        });
    }
    let overwritten = |offset: usize, size: usize| {
        let end = offset.saturating_add(size);
        parameters.iter().any(|p| {
            let p_end = p.target.saturating_add(p.bytes as usize);
            p.target < end && offset < p_end
        })
    };
    let mut args = Vec::with_capacity(def.args.len());
    for (arg, &offset) in def.fixed().iter().zip(&offsets) {
        let value = read_value(&instruction.args, offset, arg.kind).ok_or(Refusal::Length {
            bank,
            id,
            found,
            expected: fixed,
        })?;
        args.push(TypedArg {
            def: arg,
            offset,
            value,
            substituted: overwritten(offset, arg.kind.size()),
        });
    }
    if let Some(repeating) = def.repeating() {
        let mut offset = fixed;
        while offset < found {
            let value =
                read_value(&instruction.args, offset, repeating.kind).ok_or(Refusal::RepeatLength {
                    bank,
                    id,
                    found,
                    fixed,
                })?;
            args.push(TypedArg {
                def: repeating,
                offset,
                value,
                substituted: overwritten(offset, repeating.kind.size()),
            });
            offset = offset.saturating_add(repeating.kind.size());
        }
    }
    Ok(Typed { def, args })
}

/// Read every instruction of `event`, in order, with the event's parameters taken into account.
pub fn decode_event(game: Game, event: &Event) -> Vec<Result<Typed, Refusal>> {
    let mut by_instruction: BTreeMap<usize, Vec<Parameter>> = BTreeMap::new();
    for parameter in &event.parameters {
        by_instruction
            .entry(parameter.instruction)
            .or_default()
            .push(*parameter);
    }
    event
        .instructions
        .iter()
        .enumerate()
        .map(|(k, instruction)| {
            let own = by_instruction.get(&k).map_or(&[][..], Vec::as_slice);
            decode(game, instruction, own)
        })
        .collect()
}

/// What one of an event's own arguments is used as: a parameter copies `bytes` bytes of the event's arguments at `source`
/// into an instruction's arguments, and if the bytes land on one argument of the schema, that argument says what they are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArgumentUse {
    /// The byte offset in the event's own arguments.
    pub source: usize,
    /// How many bytes are copied.
    pub bytes: u32,
    /// The index of the instruction that receives them.
    pub instruction: usize,
    /// The argument of the schema they replace, when the copy is exactly that argument's bytes.
    pub target: Option<&'static ArgDef>,
}

/// Each parameter of `event` with the argument it fills, for the instructions the schema types. A parameter into an
/// instruction the schema refuses has no target.
pub fn argument_uses(game: Game, event: &Event) -> Vec<ArgumentUse> {
    let typed = decode_event(game, event);
    event
        .parameters
        .iter()
        .map(|p| {
            let target = typed
                .get(p.instruction)
                .and_then(|t| t.as_ref().ok())
                .and_then(|t| {
                    t.args
                        .iter()
                        .find(|a| a.offset == p.target && a.def.kind.size() == p.bytes as usize)
                })
                .map(|a| a.def);
            ArgumentUse {
                source: p.source,
                bytes: p.bytes,
                instruction: p.instruction,
                target,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instruction(bank: i32, id: i32, args: Vec<u8>) -> Instruction {
        Instruction {
            bank,
            id,
            args,
            layer: None,
        }
    }

    fn words(w: &[i32]) -> Vec<u8> {
        w.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    #[test]
    fn both_schemas_are_sorted_and_hold_the_instructions_the_states_reader_uses() {
        for game in Game::ALL {
            let s = schema(game);
            assert!(
                s.instructions()
                    .windows(2)
                    .all(|w| (w[0].bank, w[0].id) < (w[1].bank, w[1].id)),
                "{} schema is not sorted by bank and number",
                game.name()
            );
            assert_eq!(s.get(2000, 6).map(|d| d.vararg), Some(true));
            assert_eq!(s.get(3, 0).map(|d| d.vararg), Some(false));
            assert!(s.get(9999, 9999).is_none());
        }
        // the four instructions the part-state reader uses are in both
        for game in Game::ALL {
            for (bank, id) in [(2000, 6), (2005, 3), (2011, 1), (2012, 1)] {
                assert!(schema(game).get(bank, id).is_some(), "{bank}:{id}");
            }
        }
    }

    #[test]
    fn a_layout_follows_natural_alignment_and_pads_to_four() {
        let def = schema(Game::EldenRing).get(3, 8).expect("3:8");
        let (offsets, size) = def.layout();
        assert_eq!(offsets.len(), def.fixed().len());
        assert_eq!(offsets.first(), Some(&0));
        assert_eq!(size % 4, 0);
        // every kind aligns to its own size
        for game in Game::ALL {
            for d in schema(game).instructions() {
                let (offsets, size) = d.layout();
                for (arg, off) in d.fixed().iter().zip(&offsets) {
                    assert_eq!(off % arg.kind.size(), 0, "{}:{} {}", d.bank, d.id, arg.name);
                }
                assert_eq!(size % 4, 0);
            }
        }
    }

    #[test]
    fn a_set_event_flag_call_reads_its_flag() {
        // Elden Ring 2003:66 Set Event Flag: flag type (u8), flag id (u32), state (u8)
        let def = schema(Game::EldenRing).get(2003, 66).expect("2003:66");
        let (offsets, size) = def.layout();
        let mut bytes = vec![0u8; size];
        bytes[offsets[0]] = 0;
        bytes[offsets[1]..offsets[1] + 4].copy_from_slice(&1_042_360_800u32.to_le_bytes());
        bytes[offsets[2]] = 1;
        let typed = decode(Game::EldenRing, &instruction(2003, 66, bytes), &[]).expect("typed");
        let flags: Vec<_> = typed.with_role(Role::Flag).map(|a| a.value.as_i64()).collect();
        assert_eq!(flags, vec![Some(1_042_360_800)]);
        assert!(typed.literal().count() == typed.args.len());
    }

    #[test]
    fn a_length_that_is_not_the_layouts_is_refused_by_name() {
        let def = schema(Game::EldenRing).get(2003, 66).expect("2003:66");
        let (_, size) = def.layout();
        let err = decode(Game::EldenRing, &instruction(2003, 66, vec![0; size + 4]), &[]).unwrap_err();
        assert_eq!(err.reason(), "argument bytes differ from the layout");
        let err = decode(Game::EldenRing, &instruction(9999, 1, Vec::new()), &[]).unwrap_err();
        assert_eq!(err, Refusal::Unknown { bank: 9999, id: 1 });
        // a repeating argument must be whole words after the fixed part
        let err = decode(Game::Sekiro, &instruction(2000, 0, vec![0; 9]), &[]).unwrap_err();
        assert_eq!(
            err.reason(),
            "repeating argument is not whole words after the fixed part"
        );
    }

    #[test]
    fn an_initialiser_reads_its_event_and_the_words_it_passes() {
        // Sekiro 2000:0: slot, event, then the event's arguments
        let call = instruction(2000, 0, words(&[3, 11_100_230, 7, -1]));
        let typed = decode(Game::Sekiro, &call, &[]).expect("typed");
        assert_eq!(
            typed.initialisation(),
            Some(Initialisation {
                slot: Some(3),
                event: 11_100_230,
                words: vec![7, u32::MAX],
            })
        );
        // Sekiro 2000:6 has no slot
        let common = instruction(2000, 6, words(&[20_004_110, 1_117_204, 8302]));
        let init = decode(Game::Sekiro, &common, &[])
            .expect("typed")
            .initialisation();
        assert_eq!(
            init,
            Some(Initialisation {
                slot: None,
                event: 20_004_110,
                words: vec![1_117_204, 8302],
            })
        );
        // Elden Ring's 2000:6 names a slot first
        let common = instruction(2000, 6, words(&[7, 20_004_110, 1_117_204]));
        let init = decode(Game::EldenRing, &common, &[])
            .expect("typed")
            .initialisation();
        assert_eq!(
            init.map(|i| (i.slot, i.event, i.words)),
            Some((Some(7), 20_004_110, vec![1_117_204]))
        );
    }

    #[test]
    fn a_parameter_marks_the_arguments_it_overwrites() {
        let call = instruction(2000, 0, words(&[0, 11_100_230, 5]));
        let p = Parameter {
            instruction: 0,
            target: 4,
            source: 0,
            bytes: 4,
            unk: 0,
        };
        let typed = decode(Game::Sekiro, &call, &[p]).expect("typed");
        let substituted: Vec<bool> = typed.args.iter().map(|a| a.substituted).collect();
        assert_eq!(substituted, vec![false, true, false]);
        // a substituted event id is not an initialisation of a known event
        assert_eq!(typed.initialisation(), None);
        // a copy of two bytes into the middle of a word still marks the word
        let p = Parameter {
            target: 9,
            bytes: 2,
            ..p
        };
        let typed = decode(Game::Sekiro, &call, &[p]).expect("typed");
        let substituted: Vec<bool> = typed.args.iter().map(|a| a.substituted).collect();
        assert_eq!(substituted, vec![false, false, true]);
    }

    #[test]
    fn a_player_in_map_call_names_its_map_in_file_order() {
        // Elden Ring 3:8: condition group, state, then area, block, region, index
        let def = schema(Game::EldenRing).get(3, 8).expect("3:8");
        let (offsets, size) = def.layout();
        let mut bytes = vec![0u8; size];
        let area = def
            .fixed()
            .iter()
            .position(|a| a.role == Role::MapArea)
            .expect("area");
        let block = def
            .fixed()
            .iter()
            .position(|a| a.role == Role::MapBlock)
            .expect("block");
        let region = def
            .fixed()
            .iter()
            .position(|a| a.role == Role::MapRegion)
            .expect("region");
        let index = def
            .fixed()
            .iter()
            .position(|a| a.role == Role::MapIndex)
            .expect("index");
        bytes[offsets[area]] = 60;
        bytes[offsets[block]] = 44;
        bytes[offsets[region]] = 36;
        bytes[offsets[index]] = 0x10;
        let typed = decode(Game::EldenRing, &instruction(3, 8, bytes), &[]).expect("typed");
        assert_eq!(
            typed.map().and_then(|m| m.name()).as_deref(),
            Some("m60_44_36_10")
        );
    }

    #[test]
    fn a_map_number_argument_reads_as_decimal_digits() {
        // Elden Ring 2002:11: cutscene, playback, area entity, map, player entity, then two unknowns
        let def = schema(Game::EldenRing).get(2002, 11).expect("2002:11");
        let (offsets, size) = def.layout();
        let at = def
            .fixed()
            .iter()
            .position(|a| a.role == Role::MapWord)
            .expect("map word");
        let mut bytes = vec![0u8; size];
        bytes[offsets[at]..offsets[at] + 4].copy_from_slice(&11_050_000i32.to_le_bytes());
        let typed = decode(Game::EldenRing, &instruction(2002, 11, bytes), &[]).expect("typed");
        assert_eq!(
            typed.map().and_then(|m| m.name()).as_deref(),
            Some("m11_05_00_00")
        );
    }
}
