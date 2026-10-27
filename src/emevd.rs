//! Event scripts (`.emevd`, Sekiro's 64-bit little-endian layout): events, their instructions with raw argument
//! bytes, the parameter substitutions that fill instruction arguments from an event's own arguments, linked-file
//! offsets and the string table. Read only; nothing is interpreted here (see [`crate::states`] for one reading).
//!
//! Layout: a 0x90-byte header (magic, format bytes, version 0xCD, file size, then 16 i64: count and offset of events,
//! instructions, an unused table, layers, parameters and linked files, length and offset of arguments and strings),
//! then the tables. Every byte is accounted for: the tables, each instruction's arguments and the strings, with at most
//! 15 zero bytes of alignment between them.

use crate::bytes::{Budget, View, coverage};
use crate::{Result, invalid};

const FORMAT: &str = "EMEVD";
const HEADER: usize = 0x90;
/// Where the header's sixteen 64-bit fields start.
const HEADER_FIELDS: usize = HEADER - 16 * 8;
const EVENT: usize = 0x30;
const INSTRUCTION: usize = 0x20;
const LAYER: usize = 0x20;
const PARAMETER: usize = 0x20;

/// One event script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    /// The events, in file order.
    pub events: Vec<Event>,
    /// The linked-file offsets, as stored.
    pub linked_files: Vec<i64>,
    /// The string table, as stored.
    pub strings: Vec<u8>,
}

/// One event: an ID, how it behaves on rest, its instructions and its parameter substitutions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// The event's ID.
    pub id: i64,
    /// What the event does when the player rests: 0 default, 1 restart, 2 end (as stored).
    pub rest: u32,
    /// The event's instructions, in order.
    pub instructions: Vec<Instruction>,
    /// The substitutions that fill instruction arguments from the event's own arguments.
    pub parameters: Vec<Parameter>,
}

/// One instruction: a command number and its argument bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// The instruction's bank (the `2000` of `2000:6`).
    pub bank: i32,
    /// Its number within the bank (the `6` of `2000:6`).
    pub id: i32,
    /// Argument bytes as stored (before any parameter substitution).
    pub args: Vec<u8>,
    /// The layer value it carries, if it names a layer record.
    pub layer: Option<u32>,
}

impl Instruction {
    /// The arguments as little-endian i32s, when their length is a multiple of 4.
    pub fn args_i32(&self) -> Option<Vec<i32>> {
        let (words, rest) = self.args.as_chunks::<4>();
        rest.is_empty()
            .then(|| words.iter().map(|&w| i32::from_le_bytes(w)).collect())
    }
}

/// "When the event is initialised with arguments, copy `bytes` of them from `source` into instruction
/// `instruction`'s arguments at `target`."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    /// The index of the instruction within its event.
    pub instruction: usize,
    /// The byte offset in that instruction's arguments.
    pub target: usize,
    /// The byte offset in the event's own arguments.
    pub source: usize,
    /// How many bytes are copied.
    pub bytes: u32,
    /// A stored word whose meaning is not established.
    pub unk: i32,
}

fn i64_at(v: &View<'_>, o: usize) -> Result<i64> {
    Ok(v.u64(o)? as i64)
}

fn count(v: &View<'_>, o: usize) -> Result<usize> {
    let n = i64_at(v, o)?;
    usize::try_from(n).map_err(|_| invalid(FORMAT, o, "negative count"))
}

/// `n` records of `size` bytes at `offset`, inside the file.
fn table(v: &View<'_>, offset: usize, n: usize, size: usize) -> Result<(usize, usize)> {
    let len = n
        .checked_mul(size)
        .ok_or_else(|| invalid(FORMAT, offset, "table size overflow"))?;
    v.slice(offset, len)?;
    Ok((offset, len))
}

impl Script {
    /// Read a decompressed `.emevd`. Any other format, a field outside its known values, an overlap or an unaccounted
    /// byte is an error at its offset.
    pub fn read(data: &[u8]) -> Result<Self> {
        let v = View {
            data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        v.expect(0, b"EVD\0\x00\xff\x01\xff")?;
        if v.u32(8)? != 0xCD {
            return Err(invalid(FORMAT, 8, "version is not 0xCD (Sekiro)"));
        }
        if v.u32(12)? as usize != data.len() {
            return Err(invalid(FORMAT, 12, "file size differs from the data"));
        }
        // the offset of the header's `i`th 64-bit field
        let h = |i: usize| HEADER_FIELDS.saturating_add(i.saturating_mul(8));
        let (n_events, events_at) = (count(&v, h(0))?, v.size(h(1))?);
        let (n_instructions, instructions_at) = (count(&v, h(2))?, v.size(h(3))?);
        if count(&v, h(4))? != 0 {
            return Err(invalid(FORMAT, h(4), "unknown table is not empty"));
        }
        let (n_layers, layers_at) = (count(&v, h(6))?, v.size(h(7))?);
        let (n_parameters, parameters_at) = (count(&v, h(8))?, v.size(h(9))?);
        let (n_linked, linked_at) = (count(&v, h(10))?, v.size(h(11))?);
        let (args_len, args_at) = (count(&v, h(12))?, v.size(h(13))?);
        let (strings_len, strings_at) = (count(&v, h(14))?, v.size(h(15))?);

        let budget = Budget::new(FORMAT, data.len());
        let mut spans = vec![(0, HEADER)];
        spans.push(table(&v, events_at, n_events, EVENT)?);
        spans.push(table(&v, instructions_at, n_instructions, INSTRUCTION)?);
        spans.push(table(&v, layers_at, n_layers, LAYER)?);
        spans.push(table(&v, parameters_at, n_parameters, PARAMETER)?);
        spans.push(table(&v, linked_at, n_linked, 8)?);
        spans.push((strings_at, strings_len));
        v.slice(args_at, args_len)?;

        let mut layers = Vec::with_capacity(n_layers);
        for i in 0..n_layers {
            let o = v.element(layers_at, i, LAYER)?;
            if v.u32(o)? != 2
                || i64_at(&v, v.at(o, 8)?)? != 0
                || i64_at(&v, v.at(o, 16)?)? != -1
                || i64_at(&v, v.at(o, 24)?)? != 1
            {
                return Err(invalid(FORMAT, o, "layer record outside its known values"));
            }
            layers.push(v.u32(v.at(o, 4)?)?);
        }

        let (mut next_instruction, mut next_parameter) = (0usize, 0usize);
        let mut events = Vec::with_capacity(n_events);
        for i in 0..n_events {
            let o = v.element(events_at, i, EVENT)?;
            let id = i64_at(&v, o)?;
            let n_ins = count(&v, v.at(o, 8)?)?;
            let ins_at = v.size(v.at(o, 16)?)?;
            let n_par = count(&v, v.at(o, 24)?)?;
            let par_at = v.size(v.at(o, 32)?)?;
            let rest = v.u32(v.at(o, 40)?)?;
            if rest > 2 || v.u32(v.at(o, 44)?)? != 0 {
                return Err(invalid(
                    FORMAT,
                    v.at(o, 40)?,
                    "rest behaviour or padding outside its known values",
                ));
            }
            // counts are checked against what the tables have left before anything is sized by them
            if n_ins > n_instructions.saturating_sub(next_instruction)
                || n_par > n_parameters.saturating_sub(next_parameter)
            {
                return Err(invalid(
                    FORMAT,
                    v.at(o, 8)?,
                    "more instructions or parameters than the tables hold",
                ));
            }
            // events own consecutive runs of the instruction and parameter tables, in order
            if (n_ins > 0 && ins_at != v.element(0, next_instruction, INSTRUCTION)?)
                || (n_par > 0 && par_at != v.element(0, next_parameter, PARAMETER)?)
            {
                return Err(invalid(
                    FORMAT,
                    o,
                    "event's instructions or parameters are not the next in their table",
                ));
            }
            let mut instructions = Vec::with_capacity(n_ins);
            for k in 0..n_ins {
                let p = v.element(instructions_at, next_instruction.saturating_add(k), INSTRUCTION)?;
                let bank = v.u32(p)? as i32;
                let id = v.u32(v.at(p, 4)?)? as i32;
                let len = count(&v, v.at(p, 8)?)?;
                let at_slot = v.at(p, 16)?;
                let at = i64_at(&v, at_slot)?;
                let layer_slot = v.at(p, 24)?;
                let layer_at = i64_at(&v, layer_slot)?;
                let args = if len == 0 {
                    Vec::new()
                } else {
                    let at = usize::try_from(at).map_err(|_| invalid(FORMAT, at_slot, "argument offset"))?;
                    if at.checked_add(len).is_none_or(|end| end > args_len) {
                        return Err(invalid(FORMAT, at_slot, "arguments outside the argument block"));
                    }
                    let arg_start = v.at(args_at, at)?;
                    spans.push((arg_start, len));
                    budget.charge(v.at(p, 8)?, len)?;
                    v.slice(arg_start, len)?.to_vec()
                };
                let layer = match layer_at {
                    -1 => None,
                    at => match usize::try_from(at) {
                        Ok(at) if at.is_multiple_of(LAYER) => at
                            .checked_div(LAYER)
                            .and_then(|index| layers.get(index))
                            .copied()
                            .map(Some)
                            .ok_or_else(|| invalid(FORMAT, layer_slot, "layer offset"))?,
                        _ => return Err(invalid(FORMAT, layer_slot, "layer offset")),
                    },
                };
                instructions.push(Instruction {
                    bank,
                    id,
                    args,
                    layer,
                });
            }
            let mut parameters = Vec::with_capacity(n_par);
            for k in 0..n_par {
                let p = v.element(parameters_at, next_parameter.saturating_add(k), PARAMETER)?;
                let par = Parameter {
                    instruction: count(&v, p)?,
                    target: count(&v, v.at(p, 8)?)?,
                    source: count(&v, v.at(p, 16)?)?,
                    bytes: v.u32(v.at(p, 24)?)?,
                    unk: v.u32(v.at(p, 28)?)? as i32,
                };
                let fits = instructions.get(par.instruction).is_some_and(|ins| {
                    par.target
                        .checked_add(par.bytes as usize)
                        .is_some_and(|end| end <= ins.args.len())
                });
                if par.bytes == 0 || !fits {
                    return Err(invalid(
                        FORMAT,
                        p,
                        "parameter outside its instruction's arguments",
                    ));
                }
                parameters.push(par);
            }
            next_instruction = next_instruction.saturating_add(n_ins);
            next_parameter = next_parameter.saturating_add(n_par);
            events.push(Event {
                id,
                rest,
                instructions,
                parameters,
            });
        }
        if next_instruction != n_instructions || next_parameter != n_parameters {
            return Err(invalid(
                FORMAT,
                h(2),
                "instructions or parameters owned by no event",
            ));
        }
        let linked_files = (0..n_linked)
            .map(|i| i64_at(&v, v.element(linked_at, i, 8)?))
            .collect::<Result<Vec<_>>>()?;
        budget.charge(h(14), strings_len)?;
        let strings = v.slice(strings_at, strings_len)?.to_vec();
        coverage(&v, spans, data.len(), 15)?;
        Ok(Script {
            events,
            linked_files,
            strings,
        })
    }
}
