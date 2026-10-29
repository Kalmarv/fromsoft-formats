//! TAE event templates: the names of event types and the typed layout of their parameter bytes, read from the XML of the
//! community's templates (DSAnimStudio's format, which Smithbox keeps for each game: `TAE.Template.SDT.xml` for Sekiro,
//! `TAE.Template.ER.xml` for Elden Ring). The XML is the caller's; the crate carries none.
//!
//! A template lists an event type's parameter fields in order, each a fixed-width scalar (`b` a boolean byte, `u8` `s8`
//! `u16` `s16` `u32` `s32` `u64` `s64` integers, `x8` `x16` `x32` integers the editor shows in hexadecimal, `f32` `f64`
//! floats) or a run of bytes (`aob` with a `length`), with an optional name, a value the field must hold (`assert`, 0 in
//! every field of the two games' templates), a default, a list of named values (`entry`) and the table a value refers to
//! (`ref`: `SpEffectParam`, `DecalParam`, ...). The fields follow each other with no alignment: over every event of every
//! event file of both installs, the 553,355 asserted fields of Sekiro's events and 2,050,123 of Elden Ring's hold their
//! asserted value (`tests/tae_template_census.rs`), which they would not if a field sat anywhere but where the packing puts
//! it.
//!
//! What the templates name is the community's: a field's name and type are an editor's reading, and the installs show
//! where it fails (a field the template types as a float that stores a negative integer: [`Value::Word`]; a value no
//! `entry` names; an event stored shorter than the template lists, with the fields it lacks in [`Decoded::missing`]).
//!
//! A second source, soulstruct's classes for the event types of the Dark Souls Remastered and III era, lists 113 types, 106
//! of them in Sekiro's template and 96 in Elden Ring's (none of the others occurs in either game's files). Where both give
//! an event type fields, no field of one straddles the edge of a field of the other except in Sekiro's event 112 (the
//! template's word at byte 6 against soulstruct's edge at byte 8, which Elden Ring's template and soulstruct both keep);
//! they type 5 words of Sekiro's events and 4 of Elden Ring's as a float where the other has an integer, and the stored
//! words side with the template for the jump table's argument A (floats) and Sekiro's event 130 (integers), and with
//! soulstruct for event 730 (integers: the words [`Value::Word`] keeps). soulstruct's classes are the older games' layouts,
//! so for 7 Sekiro and 9 Elden Ring event types some events hold a nonzero byte past the fields it lists, and none past the
//! template's (`tests/tae_template_census.rs`).
use crate::{Result, invalid, xml};
use std::collections::BTreeMap;

const FORMAT: &str = "TAE template";

/// How a field is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// `b`: a byte that is 0 or 1.
    Bool,
    /// `u8` and `x8`.
    U8,
    /// `s8`.
    S8,
    /// `u16` and `x16`.
    U16,
    /// `s16`.
    S16,
    /// `u32` and `x32`.
    U32,
    /// `s32`.
    S32,
    /// `u64`.
    U64,
    /// `s64`.
    S64,
    /// `f32`.
    F32,
    /// `f64`.
    F64,
    /// `aob`: this many bytes.
    Bytes(usize),
}

impl FieldKind {
    /// The tag a template writes the kind with (`x8`, `x16` and `x32` are read as `u8`, `u16` and `u32`); `aob` needs its
    /// `length` and is not here.
    fn from_tag(tag: &str) -> Option<Self> {
        Some(match tag {
            "b" => Self::Bool,
            "u8" | "x8" => Self::U8,
            "s8" => Self::S8,
            "u16" | "x16" => Self::U16,
            "s16" => Self::S16,
            "u32" | "x32" => Self::U32,
            "s32" => Self::S32,
            "u64" => Self::U64,
            "s64" => Self::S64,
            "f32" => Self::F32,
            "f64" => Self::F64,
            _ => return None,
        })
    }

    /// The bytes the field takes.
    pub const fn width(self) -> usize {
        match self {
            Self::Bool | Self::U8 | Self::S8 => 1,
            Self::U16 | Self::S16 => 2,
            Self::U32 | Self::S32 | Self::F32 => 4,
            Self::U64 | Self::S64 | Self::F64 => 8,
            Self::Bytes(n) => n,
        }
    }
}

/// One parameter field of an event type.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// How it is stored.
    pub kind: FieldKind,
    /// The community's name for it, if the template gives one.
    pub name: Option<String>,
    /// The value the template asserts the field always holds (0 in every asserted field of the two games' templates).
    pub assert: Option<f64>,
    /// The value the editor starts a new event's field with (-1 on eight fields).
    pub default: Option<i64>,
    /// The values the template names, in the template's order.
    pub entries: Vec<(i64, String)>,
    /// The name of a list of sounds or particles the value is looked up in (`aliasEnum`: `Sound`, `Particle`).
    pub alias_enum: Option<String>,
    /// The name of a project-defined value list the value is looked up in (`projectEnum`: `SP_EFFECT_TYPE`).
    pub project_enum: Option<String>,
    /// The param table a value of the field is a row id of (`ref`: `SpEffectParam`, `DecalParam`, `LockCamParam`,
    /// `MaterialEx`, `AiSoundParam`, `Bullet`).
    pub reference: Option<String>,
}

impl Field {
    /// Whether a record that stores no bytes for the field loses nothing: the field is asserted zero, or the template names
    /// it padding (a name that starts with `pad`, in any case).
    pub fn is_padding(&self) -> bool {
        self.assert == Some(0.)
            || self
                .name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().starts_with("pad"))
    }

    /// The template's name for `value`, if it names it.
    pub fn entry(&self, value: i64) -> Option<&str> {
        self.entries
            .iter()
            .find(|(v, _)| *v == value)
            .map(|(_, name)| name.as_str())
    }
}

/// An event type's template.
#[derive(Clone, Debug, PartialEq)]
pub struct EventTemplate {
    /// The event type number ([`super::Event::kind`]).
    pub kind: i32,
    /// The community's name for the event type; none where the template only says `Event <number>`.
    pub name: Option<String>,
    /// The parameter fields in order.
    pub fields: Vec<Field>,
}

/// A stored field.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A boolean byte.
    Bool(bool),
    /// A signed integer, or an unsigned one that fits.
    Int(i64),
    /// An unsigned 64-bit integer that does not fit [`Value::Int`].
    Unsigned(u64),
    /// A finite float.
    Float(f64),
    /// The stored word of a 32-bit float field that is not a finite float (the template types the field as a float and it
    /// stores something else: -1, a negative integer).
    Word(u32),
    /// A run of bytes.
    Bytes(Vec<u8>),
}

/// One field read from an event's parameter bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Argument<'t> {
    /// The field's position in the template.
    pub index: usize,
    /// The template's field.
    pub field: &'t Field,
    /// The stored value.
    pub value: Value,
}

/// An event's parameter bytes read through its template.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded<'t> {
    /// The fields the bytes hold, in order.
    pub arguments: Vec<Argument<'t>>,
    /// The fields the bytes end before (a field cut short counts): none for an event stored whole. 4,627 of Sekiro's
    /// events and 268 of Elden Ring's are stored shorter than their template; [`Field::is_padding`] says which of the fields
    /// they lack matter.
    pub missing: Vec<&'t Field>,
    /// Whether the bytes after the template's last field (the padding to the next record) hold a non-zero byte: false for
    /// every event of both installs.
    pub trailing_nonzero: bool,
}

impl EventTemplate {
    /// The bytes the fields take together.
    pub fn size(&self) -> usize {
        self.fields
            .iter()
            .fold(0usize, |n, f| n.saturating_add(f.kind.width()))
    }

    /// Read `bytes`, an event's parameter bytes ([`super::Event::parameters`]), through the fields. A boolean that is neither
    /// 0 nor 1, a float64 that is not finite and a field that holds other than its asserted value are refused, with the
    /// field's offset in `bytes`; a 32-bit float field that stores a word that is not a finite float is kept as that word
    /// ([`Value::Word`]).
    pub fn decode<'t>(&'t self, bytes: &[u8]) -> Result<Decoded<'t>> {
        let mut arguments = Vec::with_capacity(self.fields.len());
        let mut at = 0usize;
        for (index, field) in self.fields.iter().enumerate() {
            let end = at.saturating_add(field.kind.width());
            let Some(raw) = bytes.get(at..end) else {
                return Ok(Decoded {
                    arguments,
                    missing: self.fields.iter().skip(index).collect(),
                    trailing_nonzero: false,
                });
            };
            let value = read(field, raw, at)?;
            if let Some(want) = field.assert {
                let stored = match &value {
                    Value::Bool(b) => Some(f64::from(u8::from(*b))),
                    Value::Int(i) => Some(*i as f64),
                    Value::Unsigned(u) => Some(*u as f64),
                    Value::Float(f) => Some(*f),
                    Value::Word(_) | Value::Bytes(_) => None,
                };
                if stored != Some(want) {
                    return Err(invalid(
                        FORMAT,
                        at,
                        format!(
                            "event {} field {index}: stored {value:?}, the template asserts {want}",
                            self.kind
                        ),
                    ));
                }
            }
            arguments.push(Argument { index, field, value });
            at = end;
        }
        let trailing_nonzero = bytes.get(at..).is_some_and(|rest| rest.iter().any(|b| *b != 0));
        Ok(Decoded {
            arguments,
            missing: Vec::new(),
            trailing_nonzero,
        })
    }
}

/// The value of `field` in `raw` (exactly its width), at `at` in the event's bytes.
fn read(field: &Field, raw: &[u8], at: usize) -> Result<Value> {
    fn le<const N: usize>(raw: &[u8]) -> Option<[u8; N]> {
        raw.get(..N)?.try_into().ok()
    }
    let bad = || invalid(FORMAT, at, "field narrower than its kind");
    Ok(match field.kind {
        FieldKind::Bool => match raw.first().copied() {
            Some(0) => Value::Bool(false),
            Some(1) => Value::Bool(true),
            Some(b) => return Err(invalid(FORMAT, at, format!("boolean byte {b} is not 0 or 1"))),
            None => return Err(bad()),
        },
        FieldKind::U8 => Value::Int(i64::from(*raw.first().ok_or_else(bad)?)),
        FieldKind::S8 => Value::Int(i64::from(i8::from_le_bytes(le::<1>(raw).ok_or_else(bad)?))),
        FieldKind::U16 => Value::Int(i64::from(u16::from_le_bytes(le::<2>(raw).ok_or_else(bad)?))),
        FieldKind::S16 => Value::Int(i64::from(i16::from_le_bytes(le::<2>(raw).ok_or_else(bad)?))),
        FieldKind::U32 => Value::Int(i64::from(u32::from_le_bytes(le::<4>(raw).ok_or_else(bad)?))),
        FieldKind::S32 => Value::Int(i64::from(i32::from_le_bytes(le::<4>(raw).ok_or_else(bad)?))),
        FieldKind::S64 => Value::Int(i64::from_le_bytes(le::<8>(raw).ok_or_else(bad)?)),
        FieldKind::U64 => {
            let u = u64::from_le_bytes(le::<8>(raw).ok_or_else(bad)?);
            i64::try_from(u).map_or(Value::Unsigned(u), Value::Int)
        }
        FieldKind::F32 => {
            let bits = u32::from_le_bytes(le::<4>(raw).ok_or_else(bad)?);
            let f = f32::from_bits(bits);
            if f.is_finite() {
                Value::Float(f64::from(f))
            } else {
                Value::Word(bits)
            }
        }
        FieldKind::F64 => {
            let f = f64::from_le_bytes(le::<8>(raw).ok_or_else(bad)?);
            if !f.is_finite() {
                return Err(invalid(FORMAT, at, "a float64 field that is not finite"));
            }
            Value::Float(f)
        }
        FieldKind::Bytes(_) => Value::Bytes(raw.to_vec()),
    })
}

/// A game's event templates, by event type number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Templates {
    events: BTreeMap<i32, EventTemplate>,
}

fn number(text: &str, what: &str) -> Result<i64> {
    let t = text.trim();
    let parsed = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => i64::from_str_radix(hex, 16),
        None => t.parse(),
    };
    parsed.map_err(|_| invalid(FORMAT, 0, format!("{what} {text:?} is not an integer")))
}

fn field(element: &xml::Element) -> Result<Field> {
    let tag = element.name.as_str();
    let kind = if tag == "aob" {
        let length = element
            .attribute("length")
            .ok_or_else(|| invalid(FORMAT, 0, "an aob field without a length"))?;
        FieldKind::Bytes(
            usize::try_from(number(length, "an aob length")?)
                .map_err(|_| invalid(FORMAT, 0, "a negative aob length"))?,
        )
    } else {
        FieldKind::from_tag(tag).ok_or_else(|| invalid(FORMAT, 0, format!("unknown field type {tag:?}")))?
    };
    let assert = element
        .attribute("assert")
        .map(|a| {
            a.trim()
                .parse::<f64>()
                .map_err(|_| invalid(FORMAT, 0, format!("assert {a:?} is not a number")))
        })
        .transpose()?;
    let default = element
        .attribute("default")
        .map(|d| number(d, "a default"))
        .transpose()?;
    let mut entries = Vec::new();
    for entry in element.children.iter().filter(|c| c.name == "entry") {
        let value = entry
            .attribute("value")
            .ok_or_else(|| invalid(FORMAT, 0, "an entry without a value"))?;
        entries.push((
            number(value, "an entry value")?,
            entry.attribute("name").unwrap_or_default().to_owned(),
        ));
    }
    if let Some(other) = element.children.iter().find(|c| c.name != "entry") {
        return Err(invalid(
            FORMAT,
            0,
            format!("{:?} inside a {tag:?} field", other.name),
        ));
    }
    let named = |a: &str| {
        element
            .attribute(a)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    };
    Ok(Field {
        kind,
        name: named("name"),
        assert,
        default,
        entries,
        alias_enum: named("aliasEnum"),
        project_enum: named("projectEnum"),
        reference: named("ref"),
    })
}

impl Templates {
    /// The templates of an `event_template` document. The tags `event`, `entry` and the field types above are read; any
    /// other tag, an `event` without a numeric `id`, a repeated `id`, or an `entry` or `assert` that is not a number is
    /// refused.
    pub fn parse(text: &str) -> Result<Self> {
        let root = xml::parse(text)?;
        if root.name != "event_template" {
            return Err(invalid(
                FORMAT,
                0,
                format!("the root is {:?}, not event_template", root.name),
            ));
        }
        let mut events = BTreeMap::new();
        for e in &root.children {
            if e.name != "event" {
                return Err(invalid(FORMAT, 0, format!("{:?} inside event_template", e.name)));
            }
            let id = e
                .attribute("id")
                .ok_or_else(|| invalid(FORMAT, 0, "an event without an id"))?;
            let kind = i32::try_from(number(id, "an event id")?)
                .map_err(|_| invalid(FORMAT, 0, format!("event id {id:?} out of range")))?;
            // one event spells its name attribute `Name`
            let name = e
                .attribute("name")
                .or_else(|| e.attribute("Name"))
                .map(str::trim)
                .filter(|n| {
                    !n.is_empty()
                        && !n
                            .strip_prefix("Event ")
                            .is_some_and(|rest| rest.trim().parse::<i64>().is_ok())
                })
                .map(str::to_owned);
            let fields = e.children.iter().map(field).collect::<Result<Vec<_>>>()?;
            if events
                .insert(kind, EventTemplate { kind, name, fields })
                .is_some()
            {
                return Err(invalid(FORMAT, 0, format!("event {kind} is defined twice")));
            }
        }
        Ok(Self { events })
    }

    /// The template of event type `kind`.
    pub fn get(&self, kind: i32) -> Option<&EventTemplate> {
        self.events.get(&kind)
    }

    /// The event types the templates define, in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = &EventTemplate> {
        self.events.values()
    }

    /// How many event types the templates define.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the templates define no event type.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldKind, Templates, Value};

    const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<event_template game="SDT">
	<event id="7" Name="A &amp; B">
		<s32 name="v"><entry value="0" name="zero"/><entry value="-1" name="none"/></s32>
		<f32 name="v"/>
		<b/>
		<s64 name="wide"/>
	</event>
	<event id="8" name="Event 8"><u8 assert="0"/><s16 ref="SpEffectParam" default="-1"/><aob length="2"/></event>
	<event id="9" name=""/>
	<event id="10" name="Pad"><s32 name="a"/><s32 assert="0"/><u8 name="Padding1"/><s32 name="b"/></event>
</event_template>"#;

    fn bytes(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn templates_keep_names_kinds_and_entries() {
        let t = Templates::parse(XML).unwrap();
        assert_eq!(t.len(), 4);
        let e = t.get(7).unwrap();
        assert_eq!(e.name.as_deref(), Some("A & B"));
        assert_eq!(
            e.fields.iter().map(|f| f.kind).collect::<Vec<_>>(),
            [FieldKind::S32, FieldKind::F32, FieldKind::Bool, FieldKind::S64]
        );
        assert_eq!(e.size(), 17);
        assert_eq!(e.fields[0].entry(-1), Some("none"));
        assert_eq!(e.fields[0].entry(5), None);
        // a placeholder name or none is no name
        assert_eq!(t.get(8).unwrap().name, None);
        assert_eq!(t.get(9).unwrap().name, None);
        let f = &t.get(8).unwrap().fields;
        assert_eq!(
            (f[0].assert, f[1].reference.as_deref(), f[1].default),
            (Some(0.), Some("SpEffectParam"), Some(-1))
        );
        assert_eq!(f[2].kind, FieldKind::Bytes(2));
        assert_eq!(t.iter().map(|e| e.kind).collect::<Vec<_>>(), [7, 8, 9, 10]);
    }

    #[test]
    fn bad_templates_are_refused() {
        for bad in [
            "<other/>",
            "<event_template><x/></event_template>",
            "<event_template><event/></event_template>",
            r#"<event_template><event id="a"/></event_template>"#,
            r#"<event_template><event id="1"/><event id="1"/></event_template>"#,
            r#"<event_template><event id="1"><q8/></event></event_template>"#,
            r#"<event_template><event id="1"><u8 assert="x"/></event></event_template>"#,
            r#"<event_template><event id="1"><aob/></event></event_template>"#,
            r#"<event_template><event id="1"><u8><entry name="n"/></u8></event></event_template>"#,
            r#"<event_template><event id="1"><u8><foo/></u8></event></event_template>"#,
        ] {
            assert!(Templates::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn fields_are_read_in_order_at_their_widths() {
        let t = Templates::parse(XML).unwrap();
        let e = t.get(7).unwrap();
        let stored = bytes(&[
            &(-2i32).to_le_bytes(),
            &0.5f32.to_le_bytes(),
            &[1],
            &(-9i64).to_le_bytes(),
            &[0, 0, 0],
        ]);
        let d = e.decode(&stored).unwrap();
        assert_eq!(
            d.arguments.iter().map(|a| a.value.clone()).collect::<Vec<_>>(),
            [
                Value::Int(-2),
                Value::Float(0.5),
                Value::Bool(true),
                Value::Int(-9)
            ]
        );
        assert!(d.missing.is_empty() && !d.trailing_nonzero);
        // padding after the last field that is not zero is reported, not refused
        let mut loud = stored.clone();
        loud.push(7);
        assert!(e.decode(&loud).unwrap().trailing_nonzero);
        // a boolean that is neither 0 nor 1 is refused at its offset
        let mut bad = stored.clone();
        bad[8] = 2;
        let err = e.decode(&bad).unwrap_err().to_string();
        assert!(err.starts_with("TAE template at 0x8:"), "{err}");
    }

    #[test]
    fn a_float_field_that_stores_an_integer_keeps_its_word() {
        let t = Templates::parse(XML).unwrap();
        let e = t.get(7).unwrap();
        let stored = bytes(&[
            &0i32.to_le_bytes(),
            &u32::MAX.to_le_bytes(),
            &[0],
            &0i64.to_le_bytes(),
        ]);
        let d = e.decode(&stored).unwrap();
        assert_eq!(d.arguments[1].value, Value::Word(u32::MAX));
        // a NaN keeps its bits too
        let nan = 0x7fc0_0001u32;
        let stored = bytes(&[&0i32.to_le_bytes(), &nan.to_le_bytes(), &[0], &0i64.to_le_bytes()]);
        assert_eq!(e.decode(&stored).unwrap().arguments[1].value, Value::Word(nan));
    }

    #[test]
    fn a_short_event_keeps_the_fields_it_stores_and_names_the_ones_it_lacks() {
        let t = Templates::parse(XML).unwrap();
        let e = t.get(7).unwrap();
        let stored = bytes(&[&(-2i32).to_le_bytes()]);
        let d = e.decode(&stored).unwrap();
        assert_eq!(d.arguments.len(), 1);
        assert_eq!(d.missing.len(), 3);
        // a field cut short counts as missing
        let d = e.decode(&bytes(&[&0i32.to_le_bytes(), &[0, 0]])).unwrap();
        assert_eq!((d.arguments.len(), d.missing.len()), (1, 3));
        // the lacking fields that are asserted zero or padding are the ones a caller may ignore
        let p = t.get(10).unwrap();
        let d = p.decode(&1i32.to_le_bytes()).unwrap();
        let ignorable: Vec<bool> = d.missing.iter().map(|f| f.is_padding()).collect();
        assert_eq!(ignorable, [true, true, false]);
    }

    #[test]
    fn an_asserted_field_must_hold_its_value() {
        let t = Templates::parse(XML).unwrap();
        let e = t.get(8).unwrap();
        assert!(e.decode(&[0, 0, 0, 0, 0]).is_ok());
        let err = e.decode(&[1, 0, 0, 0, 0]).unwrap_err().to_string();
        assert!(
            err.contains("event 8 field 0") && err.contains("asserts 0"),
            "{err}"
        );
        // a run of bytes is kept as stored
        assert_eq!(
            e.decode(&[0, 5, 0, 9, 8]).unwrap().arguments[2].value,
            Value::Bytes(vec![9, 8])
        );
    }

    #[test]
    fn eight_byte_unsigned_values_keep_their_range() {
        let t =
            Templates::parse(r#"<event_template><event id="1"><u64/><u64/><x16/></event></event_template>"#)
                .unwrap();
        let stored = bytes(&[
            &1u64.to_le_bytes(),
            &u64::MAX.to_le_bytes(),
            &0xBEEFu16.to_le_bytes(),
        ]);
        let d = t.get(1).unwrap().decode(&stored).unwrap();
        let v: Vec<Value> = d.arguments.into_iter().map(|a| a.value).collect();
        assert_eq!(v, [Value::Int(1), Value::Unsigned(u64::MAX), Value::Int(0xBEEF)]);
    }
}
