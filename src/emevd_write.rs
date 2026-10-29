//! The event-script writer: [`Script::write`].
//!
//! The stored layout is the one the reader describes: the 0x90-byte header, the events (0x30 bytes each), the
//! instructions (0x20 each, the events' own runs in order), the layer records (0x20 each, one per distinct layer value in
//! the order the instructions first name them), the argument block (each instruction's arguments padded to four bytes,
//! the block padded to sixteen), the parameter substitutions (0x20 each), the linked-file offsets and the string table.
//! An event or instruction without arguments stores -1 for the offset of the list or bytes it does not have.
#![allow(
    clippy::arithmetic_side_effects,
    reason = "offsets and lengths are sums of in-memory sizes; every value that reaches the file goes through a checked conversion"
)]

use crate::{
    Result,
    emevd::{Event, Script},
    put::Out,
};

const FORMAT: &str = "EMEVD";
const HEADER: usize = 0x90;
/// Where the header's sixteen 64-bit fields start.
const HEADER_FIELDS: usize = 0x10;
const INSTRUCTION: usize = 0x20;
const LAYER: usize = 0x20;
const PARAMETER: usize = 0x20;

/// `n` as the signed 64-bit word the file stores counts and offsets in.
fn signed(out: &Out, n: usize, what: &str) -> Result<i64> {
    i64::try_from(n).map_err(|_| out.err(format!("{what} {n} does not fit a 64-bit word")))
}

impl Script {
    /// Write the script as an `.emevd` file in Sekiro's 64-bit layout, which Elden Ring's files share. A script that
    /// [`Script::read`] returned writes back to the bytes it was read from, with one difference: an event without
    /// parameters is written with the offset -1 for its (empty) list, where 6 events of 4 of Sekiro's 30 files store the
    /// running offset of the table instead (0 or the offset the next event's list would start at), a word the reader
    /// does not take from an event with no parameters. Every other file of both installs (Sekiro's other 26, all
    /// 446 distinct files of Elden Ring's 589) is written byte for byte.
    ///
    /// The checks of [`Script::read`] apply to what is written: a rest behaviour above 2, or a parameter substitution of
    /// no bytes or outside its instruction's arguments, is refused.
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut out = Out::new(FORMAT, false);
        out.bytes(b"EVD\0\x00\xff\x01\xff");
        out.u32(0xCD);
        out.u32(0);
        out.zeros(HEADER - 16);
        let n_instructions: usize = self.events.iter().map(|e| e.instructions.len()).sum();
        let n_parameters: usize = self.events.iter().map(|e| e.parameters.len()).sum();
        let mut layers: Vec<u32> = Vec::new();
        for layer in self
            .events
            .iter()
            .flat_map(|e| &e.instructions)
            .filter_map(|i| i.layer)
        {
            if !layers.contains(&layer) {
                layers.push(layer);
            }
        }

        let events_at = out.len();
        let (mut next_instruction, mut next_parameter) = (0usize, 0usize);
        for event in &self.events {
            self.check_event(&out, event)?;
            out.i64(event.id);
            out.i64(signed(&out, event.instructions.len(), "instruction count")?);
            out.i64(if event.instructions.is_empty() {
                -1
            } else {
                signed(&out, next_instruction * INSTRUCTION, "instruction offset")?
            });
            out.i64(signed(&out, event.parameters.len(), "parameter count")?);
            out.i64(if event.parameters.is_empty() {
                -1
            } else {
                signed(&out, next_parameter * PARAMETER, "parameter offset")?
            });
            out.u32(event.rest);
            out.u32(0);
            next_instruction += event.instructions.len();
            next_parameter += event.parameters.len();
        }

        let instructions_at = out.len();
        out.zeros(n_instructions * INSTRUCTION);
        let layers_at = out.len();
        for &layer in &layers {
            out.u32(2);
            out.u32(layer);
            out.i64(0);
            out.i64(-1);
            out.i64(1);
        }

        let arguments_at = out.len();
        for (k, instruction) in self.events.iter().flat_map(|e| &e.instructions).enumerate() {
            let record = instructions_at + k * INSTRUCTION;
            let at = if instruction.args.is_empty() {
                -1
            } else {
                signed(&out, out.len() - arguments_at, "argument offset")?
            };
            let layer = match instruction.layer {
                None => -1,
                Some(layer) => {
                    let index = layers
                        .iter()
                        .position(|&l| l == layer)
                        .ok_or_else(|| out.err("a layer value missing from the layer table"))?;
                    signed(&out, index * LAYER, "layer offset")?
                }
            };
            let len = signed(&out, instruction.args.len(), "argument length")?;
            out.patch(record, &instruction.bank.to_le_bytes())?;
            out.patch(record + 4, &instruction.id.to_le_bytes())?;
            out.patch(record + 8, &len.to_le_bytes())?;
            out.patch(record + 16, &at.to_le_bytes())?;
            out.patch(record + 24, &layer.to_le_bytes())?;
            out.bytes(&instruction.args);
            out.align(4);
        }
        out.zeros((16 - (out.len() - arguments_at) % 16) % 16);
        let arguments_len = out.len() - arguments_at;

        let parameters_at = out.len();
        for p in self.events.iter().flat_map(|e| &e.parameters) {
            out.i64(signed(&out, p.instruction, "parameter instruction")?);
            out.i64(signed(&out, p.target, "parameter target")?);
            out.i64(signed(&out, p.source, "parameter source")?);
            out.u32(p.bytes);
            out.i32(p.unk);
        }
        let linked_at = out.len();
        for &offset in &self.linked_files {
            out.i64(offset);
        }
        let strings_at = out.len();
        out.bytes(&self.strings);

        let size = out.word(out.len(), "file size")?;
        out.patch_u32(12, size)?;
        let fields = [
            self.events.len(),
            events_at,
            n_instructions,
            instructions_at,
            0,
            layers_at,
            layers.len(),
            layers_at,
            n_parameters,
            parameters_at,
            self.linked_files.len(),
            linked_at,
            arguments_len,
            arguments_at,
            self.strings.len(),
            strings_at,
        ];
        for (i, field) in fields.into_iter().enumerate() {
            let value = out.wide(field, "header field")?;
            out.patch_u64(HEADER_FIELDS + i * 8, value)?;
        }
        Ok(out.finish())
    }

    /// The reader's checks of one event's own fields.
    fn check_event(&self, out: &Out, event: &Event) -> Result<()> {
        if event.rest > 2 {
            return Err(out.err(format!("event {} rest behaviour {}", event.id, event.rest)));
        }
        for p in &event.parameters {
            let fits = event.instructions.get(p.instruction).is_some_and(|ins| {
                p.target
                    .checked_add(p.bytes as usize)
                    .is_some_and(|end| end <= ins.args.len())
            });
            if p.bytes == 0 || !fits {
                return Err(out.err(format!(
                    "event {}: a parameter substitution outside its instruction's arguments",
                    event.id
                )));
            }
        }
        Ok(())
    }
}
