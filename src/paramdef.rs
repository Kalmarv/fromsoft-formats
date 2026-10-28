//! Typed reading of params by their definitions: a `ParamDef` (Paramdex's XML dialect, read from text the caller
//! supplies), the row layout it gives at a regulation version ([`ParamDef::layout`]) and the typed values of every row
//! of a [`Param`] ([`ParamDefs::bind`]).
//!
//! The file carries only the param type and the size of a row; what the bytes of a row are is the definition's. This
//! crate holds no definition: Paramdex states no licence, so the definitions are data the caller reads from its own copy
//! (Paramdex, or Smithbox's version-aware copy of it) and passes in.
//!
//! # The layout rule
//!
//! A definition's fields follow each other with no padding but their own. A field of the regulation's version is laid
//! out in order: a scalar takes its size, `fixstr`, `fixstrW` and `dummy8` arrays take their length (two bytes a
//! character for `fixstrW`). A bit field (`u8 flag:1`, `u16 mode:3`, `dummy8 pad:7`) lives in a container of its type's
//! width (8, 16 or 32 bits, `dummy8` counting as `u8`): it takes the next free bits of the open container, low bits
//! first, and opens a new container (at the next byte) when no container is open, the type's width differs or the
//! bits do not fit. Any other field closes the container. Bits of a container no field declares must be zero.
//!
//! A field with a `FirstVersion` exists from that regulation number on, one with a `RemovedVersion` until it, exclusive
//! (the numbers are those of [`crate::Binder::regulation_number`]). The size of the layout at the regulation must equal
//! the file's row size: [`ParamDefs::bind`] refuses a def that does not fit.
use std::collections::BTreeMap;

use crate::param::Param;
use crate::{Result, invalid, xml};

const FORMAT: &str = "PARAMDEF";
/// The newest `XmlVersion` of a definition file this reader knows (the layout rules did not change between 1 and 3).
const XML_VERSION_MAX: u32 = 3;

/// The type of a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub enum FieldType {
    /// `s8`.
    S8,
    /// `u8`.
    U8,
    /// `s16`.
    S16,
    /// `u16`.
    U16,
    /// `s32`.
    S32,
    /// `u32`.
    U32,
    /// `b32`: four bytes holding a boolean, read as the signed integer.
    B32,
    /// `f32`.
    F32,
    /// `angle32`: a `f32` holding an angle.
    Angle32,
    /// `f64`.
    F64,
    /// `fixstr`: a fixed number of Shift-JIS bytes, the text ending at the first NUL.
    FixStr,
    /// `fixstrW`: a fixed number of UTF-16 code units, the text ending at the first NUL.
    FixStrW,
    /// `dummy8`: a byte or an array of bytes, or a bit field of a byte container.
    Dummy8,
}

impl FieldType {
    fn parse(name: &str) -> Option<FieldType> {
        Some(match name {
            "s8" => FieldType::S8,
            "u8" => FieldType::U8,
            "s16" => FieldType::S16,
            "u16" => FieldType::U16,
            "s32" => FieldType::S32,
            "u32" => FieldType::U32,
            "b32" => FieldType::B32,
            "f32" => FieldType::F32,
            "angle32" => FieldType::Angle32,
            "f64" => FieldType::F64,
            "fixstr" => FieldType::FixStr,
            "fixstrW" => FieldType::FixStrW,
            "dummy8" => FieldType::Dummy8,
            _ => return None,
        })
    }

    /// The type's name in a definition (`u8`, `fixstrW`).
    pub fn name(self) -> &'static str {
        match self {
            FieldType::S8 => "s8",
            FieldType::U8 => "u8",
            FieldType::S16 => "s16",
            FieldType::U16 => "u16",
            FieldType::S32 => "s32",
            FieldType::U32 => "u32",
            FieldType::B32 => "b32",
            FieldType::F32 => "f32",
            FieldType::Angle32 => "angle32",
            FieldType::F64 => "f64",
            FieldType::FixStr => "fixstr",
            FieldType::FixStrW => "fixstrW",
            FieldType::Dummy8 => "dummy8",
        }
    }

    /// The bytes of one element (a character for the string types).
    fn size(self) -> usize {
        match self {
            FieldType::S8 | FieldType::U8 | FieldType::Dummy8 | FieldType::FixStr => 1,
            FieldType::S16 | FieldType::U16 | FieldType::FixStrW => 2,
            FieldType::S32 | FieldType::U32 | FieldType::B32 | FieldType::F32 | FieldType::Angle32 => 4,
            FieldType::F64 => 8,
        }
    }

    /// The width in bits of the container a bit field of this type lives in; `None` where the type has no bit fields.
    fn container_bits(self) -> Option<u8> {
        match self {
            FieldType::S8 | FieldType::U8 | FieldType::Dummy8 => Some(8),
            FieldType::S16 | FieldType::U16 => Some(16),
            FieldType::S32 | FieldType::U32 => Some(32),
            _ => None,
        }
    }

    fn is_signed(self) -> bool {
        matches!(self, FieldType::S8 | FieldType::S16 | FieldType::S32)
    }

    fn takes_length(self) -> bool {
        matches!(self, FieldType::FixStr | FieldType::FixStrW | FieldType::Dummy8)
    }
}

/// One field of a definition.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Field {
    /// The field's internal name.
    pub name: String,
    /// The field's type.
    pub ty: FieldType,
    /// The width in bits of a bit field.
    pub bits: Option<u8>,
    /// The length of a `fixstr`, `fixstrW` or `dummy8` array (characters or bytes).
    pub array: Option<usize>,
    /// The default value the definition states.
    pub default: Option<f64>,
    /// The first regulation number the field exists in.
    pub first_version: Option<u32>,
    /// The first regulation number the field no longer exists in.
    pub removed_version: Option<u32>,
    /// The name of the enumeration the definition gives the field (`Enum`), as editors show its values.
    pub enum_name: Option<String>,
}

impl Field {
    /// Whether the field exists at `regulation`.
    fn exists_at(&self, regulation: u32) -> bool {
        self.first_version.is_none_or(|first| regulation >= first)
            && self.removed_version.is_none_or(|removed| regulation < removed)
    }

    fn parse(element: &xml::Element) -> Result<Field> {
        let spec = element
            .attribute("Def")
            .ok_or_else(|| invalid(FORMAT, 0, "a Field without a Def"))?;
        if let Some((name, _)) = element
            .attributes
            .iter()
            .find(|(n, _)| !matches!(n.as_str(), "Def" | "FirstVersion" | "RemovedVersion"))
        {
            return Err(invalid(
                FORMAT,
                0,
                format!("{spec}: attribute {name} is not read"),
            ));
        }
        let version = |name: &str| -> Result<Option<u32>> {
            element
                .attribute(name)
                .map(|v| {
                    v.trim().parse::<u32>().map_err(|_| {
                        invalid(
                            FORMAT,
                            0,
                            format!("{spec}: {name} {v:?} is not a regulation number"),
                        )
                    })
                })
                .transpose()
        };
        let mut field = Field::parse_def(spec)?;
        field.first_version = version("FirstVersion")?;
        field.removed_version = version("RemovedVersion")?;
        field.enum_name = element
            .child("Enum")
            .map(|e| e.text.clone())
            .filter(|e| !e.is_empty());
        Ok(field)
    }

    /// `type name[:bits][ [length]][ = default]`. A bracket that does not hold a decimal length is part of the name
    /// (Paramdex's Sekiro `HitMaterialSpecialSettingParam` has a field `u8 GroundMaterialType [0,1,2,3]`).
    fn parse_def(spec: &str) -> Result<Field> {
        let bad = |why: &str| invalid(FORMAT, 0, format!("field {spec:?}: {why}"));
        let (declaration, default) = match spec.split_once('=') {
            Some((d, v)) => (
                d,
                Some(
                    v.trim()
                        .parse::<f64>()
                        .map_err(|_| bad("the default is not a number"))?,
                ),
            ),
            None => (spec, None),
        };
        let declaration = declaration.trim();
        let (type_name, rest) = declaration
            .split_once(char::is_whitespace)
            .ok_or_else(|| bad("no name"))?;
        let ty = FieldType::parse(type_name).ok_or_else(|| bad("unknown type"))?;
        let mut name = rest.trim();
        let mut array = None;
        let mut bits = None;
        if let Some(open) = name.rfind('[')
            && let Some(inner) = name
                .get(open.saturating_add(1)..)
                .and_then(|s| s.strip_suffix(']'))
            && let Ok(n) = inner.trim().parse::<usize>()
        {
            array = Some(n);
            name = name.get(..open).unwrap_or("").trim_end();
        }
        if let Some((n, b)) = name.rsplit_once(':') {
            bits = Some(
                b.trim()
                    .parse::<u8>()
                    .map_err(|_| bad("the bit width is not a number"))?,
            );
            name = n.trim_end();
        }
        if name.is_empty() {
            return Err(bad("no name"));
        }
        if let Some(width) = bits {
            let limit = ty
                .container_bits()
                .ok_or_else(|| bad("this type has no bit fields"))?;
            if width == 0 || width > limit {
                return Err(bad("the bit width does not fit the type"));
            }
            if array.is_some() {
                return Err(bad("both a bit width and a length"));
            }
        } else if ty.takes_length() {
            match array {
                Some(0) => return Err(bad("a length of 0")),
                None if ty != FieldType::Dummy8 => return Err(bad("this type needs a length")),
                None => array = Some(1),
                _ => {}
            }
        } else if array.is_some() {
            return Err(bad("this type has no array form"));
        }
        Ok(Field {
            name: name.to_owned(),
            ty,
            bits,
            array,
            default,
            first_version: None,
            removed_version: None,
            enum_name: None,
        })
    }
}

/// A paramdef: the fields of one layout of a param type's rows.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ParamDef {
    /// The param type the definition describes (`NPC_PARAM_ST`); params name it in their header.
    pub param_type: String,
    /// The definition's layout revision. A file's own revision can differ ([`TypedParam::data_version_matches`]).
    pub data_version: u16,
    /// The definition's `FormatVersion` (a property of the definition file; no param stores it).
    pub format_version: u16,
    /// The fields, in layout order.
    pub fields: Vec<Field>,
}

impl ParamDef {
    /// Read a paramdef from the text of its XML file.
    pub fn from_xml(text: &str) -> Result<ParamDef> {
        let root = xml::parse(text)?;
        if root.name != "PARAMDEF" {
            return Err(invalid(
                FORMAT,
                0,
                format!("the root element is {}, not PARAMDEF", root.name),
            ));
        }
        match root.attribute("XmlVersion").map(str::parse::<u32>) {
            Some(Ok(1..=XML_VERSION_MAX)) => {}
            other => {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!(
                        "XmlVersion {:?}, the ones read are 1 to {XML_VERSION_MAX}",
                        other.map(|v| v.map_err(|e| e.to_string()))
                    ),
                ));
            }
        }
        let text_of = |name: &str| -> Result<&str> {
            root.child(name)
                .map(|c| c.text.as_str())
                .ok_or_else(|| invalid(FORMAT, 0, format!("no {name} element")))
        };
        let number = |name: &str| -> Result<u16> {
            text_of(name)?
                .parse::<u16>()
                .map_err(|_| invalid(FORMAT, 0, format!("{name} is not a number")))
        };
        let flag = |name: &str| -> Result<bool> {
            match text_of(name)? {
                "True" | "true" => Ok(true),
                "False" | "false" => Ok(false),
                other => Err(invalid(FORMAT, 0, format!("{name} {other:?} is not a boolean"))),
            }
        };
        if flag("BigEndian")? {
            return Err(invalid(FORMAT, 0, "big-endian definitions are not read"));
        }
        let param_type = text_of("ParamType")?.to_owned();
        if param_type.is_empty() {
            return Err(invalid(FORMAT, 0, "an empty ParamType"));
        }
        let fields = root
            .child("Fields")
            .ok_or_else(|| invalid(FORMAT, 0, "no Fields element"))?
            .children
            .iter()
            .filter(|c| c.name == "Field")
            .map(Field::parse)
            .collect::<Result<Vec<_>>>()?;
        Ok(ParamDef {
            data_version: number("DataVersion")?,
            format_version: number("FormatVersion")?,
            param_type,
            fields,
        })
    }

    /// Whether any field exists only from or until a regulation: the layout then depends on the regulation number.
    pub fn version_aware(&self) -> bool {
        self.fields
            .iter()
            .any(|f| f.first_version.is_some() || f.removed_version.is_some())
    }

    /// The row layout at `regulation` (the number [`crate::Binder::regulation_number`] reads; `None` where the binder
    /// has none, which only a definition with no version-gated field accepts).
    pub fn layout(&self, regulation: Option<u32>) -> Result<Layout> {
        if regulation.is_none() && self.version_aware() {
            return Err(invalid(
                FORMAT,
                0,
                format!(
                    "{} has version-gated fields and no regulation number was given",
                    self.param_type
                ),
            ));
        }
        let mut columns = Vec::with_capacity(self.fields.len());
        let mut containers: Vec<Container> = Vec::new();
        // The bits taken of the open container, its width and where it starts.
        let mut open: Option<(u8, u8, usize)> = None;
        let mut offset = 0usize;
        let overflow = || invalid(FORMAT, 0, format!("{}: the layout overflows", self.param_type));
        for (index, field) in self.fields.iter().enumerate() {
            if regulation.is_some_and(|r| !field.exists_at(r)) {
                continue;
            }
            if let (Some(width), Some(limit)) = (field.bits, field.ty.container_bits()) {
                let fits = open.is_some_and(|(used, container, _)| {
                    container == limit
                        && u16::from(used).saturating_add(u16::from(width)) <= u16::from(container)
                });
                if !fits {
                    if let Some((used, container, start)) = open.take() {
                        containers.push(Container {
                            offset: start,
                            bits: container,
                            used,
                        });
                        offset = offset
                            .checked_add(usize::from(container / 8))
                            .ok_or_else(overflow)?;
                    }
                    open = Some((0, limit, offset));
                }
                let (used, container, start) = open.ok_or_else(overflow)?;
                columns.push(Column {
                    field: index,
                    name: field.name.clone(),
                    ty: field.ty,
                    offset: start,
                    bits: Some(BitRange {
                        shift: used,
                        width,
                        container,
                    }),
                    array: 1,
                });
                open = Some((used.saturating_add(width), container, start));
                continue;
            }
            if let Some((used, container, start)) = open.take() {
                containers.push(Container {
                    offset: start,
                    bits: container,
                    used,
                });
                offset = offset
                    .checked_add(usize::from(container / 8))
                    .ok_or_else(overflow)?;
            }
            let array = field.array.unwrap_or(1);
            columns.push(Column {
                field: index,
                name: field.name.clone(),
                ty: field.ty,
                offset,
                bits: None,
                array,
            });
            let size = field.ty.size().checked_mul(array).ok_or_else(overflow)?;
            offset = offset.checked_add(size).ok_or_else(overflow)?;
        }
        if let Some((used, container, start)) = open.take() {
            containers.push(Container {
                offset: start,
                bits: container,
                used,
            });
            offset = offset
                .checked_add(usize::from(container / 8))
                .ok_or_else(overflow)?;
        }
        Ok(Layout {
            size: offset,
            columns,
            containers,
        })
    }
}

/// Where a bit field sits in its container.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct BitRange {
    /// The first bit (0 is the container's lowest).
    pub shift: u8,
    /// The number of bits.
    pub width: u8,
    /// The container's width in bits: 8, 16 or 32.
    pub container: u8,
}

/// A container of bit fields: where it is, its width and how many of its low bits some field declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Container {
    offset: usize,
    bits: u8,
    used: u8,
}

/// One field of a layout: where its value is in a row.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Column {
    /// The index of the field in [`ParamDef::fields`].
    pub field: usize,
    /// The field's name.
    pub name: String,
    /// The field's type.
    pub ty: FieldType,
    /// The byte offset of the field, or of its container for a bit field.
    pub offset: usize,
    /// The bits a bit field takes.
    pub bits: Option<BitRange>,
    /// The elements of an array field (1 for the others).
    pub array: usize,
}

/// A typed value of a field.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub enum Value {
    /// A signed integer (`s8`, `s16`, `s32`, `b32`, and signed bit fields, sign-extended).
    Signed(i64),
    /// An unsigned integer (`u8`, `u16`, `u32`, bit fields of unsigned types and of `dummy8`).
    Unsigned(u64),
    /// A `f32` or `angle32`.
    Float(f32),
    /// A `f64`.
    Double(f64),
    /// A `fixstr` or `fixstrW`, up to its first NUL.
    Text(String),
    /// A `dummy8` array.
    Bytes(Vec<u8>),
}

impl Value {
    /// The integer value, where this is an integer that fits.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Signed(v) => Some(*v),
            Value::Unsigned(v) => i64::try_from(*v).ok(),
            _ => None,
        }
    }

    /// The numeric value as a `f64` (an integer value converts exactly up to 2^53).
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Signed(v) => Some(*v as f64),
            Value::Unsigned(v) => Some(*v as f64),
            Value::Float(v) => Some(f64::from(*v)),
            Value::Double(v) => Some(*v),
            _ => None,
        }
    }

    /// The text of a string field.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl Column {
    fn bytes<'r>(&self, row: &'r [u8], len: usize) -> Result<&'r [u8]> {
        self.offset
            .checked_add(len)
            .and_then(|end| row.get(self.offset..end))
            .ok_or_else(|| {
                invalid(
                    FORMAT,
                    self.offset,
                    format!("{} lies beyond a row of {} bytes", self.name, row.len()),
                )
            })
    }

    /// The field's value in `row`, which must be at least the layout's size.
    pub fn read(&self, row: &[u8]) -> Result<Value> {
        if let Some(bits) = self.bits {
            let raw = match bits.container {
                8 => u64::from(self.bytes(row, 1)?.first().copied().unwrap_or(0)),
                16 => u64::from(u16::from_le_bytes(array(self.bytes(row, 2)?))),
                _ => u64::from(u32::from_le_bytes(array(self.bytes(row, 4)?))),
            };
            let mask = if bits.width >= 64 {
                u64::MAX
            } else {
                (1u64 << bits.width).saturating_sub(1)
            };
            let value = (raw >> bits.shift) & mask;
            return Ok(if self.ty.is_signed() {
                // sign-extend from the field's top bit
                let top = 1u64 << bits.width.saturating_sub(1);
                #[allow(clippy::cast_possible_wrap)]
                let signed = if value & top != 0 {
                    (value | !mask) as i64
                } else {
                    value as i64
                };
                Value::Signed(signed)
            } else {
                Value::Unsigned(value)
            });
        }
        Ok(match self.ty {
            FieldType::S8 => Value::Signed(i64::from(i8::from_le_bytes(array(self.bytes(row, 1)?)))),
            FieldType::U8 => Value::Unsigned(u64::from(u8::from_le_bytes(array(self.bytes(row, 1)?)))),
            FieldType::S16 => Value::Signed(i64::from(i16::from_le_bytes(array(self.bytes(row, 2)?)))),
            FieldType::U16 => Value::Unsigned(u64::from(u16::from_le_bytes(array(self.bytes(row, 2)?)))),
            FieldType::S32 | FieldType::B32 => {
                Value::Signed(i64::from(i32::from_le_bytes(array(self.bytes(row, 4)?))))
            }
            FieldType::U32 => Value::Unsigned(u64::from(u32::from_le_bytes(array(self.bytes(row, 4)?)))),
            FieldType::F32 | FieldType::Angle32 => {
                Value::Float(f32::from_le_bytes(array(self.bytes(row, 4)?)))
            }
            FieldType::F64 => Value::Double(f64::from_le_bytes(array(self.bytes(row, 8)?))),
            FieldType::Dummy8 => Value::Bytes(self.bytes(row, self.array)?.to_vec()),
            FieldType::FixStr => {
                let raw = self.bytes(row, self.array)?;
                let text = raw.split(|&b| b == 0).next().unwrap_or(&[]);
                let decoded = encoding_rs::SHIFT_JIS
                    .decode_without_bom_handling_and_without_replacement(text)
                    .ok_or_else(|| invalid(FORMAT, self.offset, format!("{}: not Shift-JIS", self.name)))?;
                Value::Text(decoded.into_owned())
            }
            FieldType::FixStrW => {
                let len = self
                    .array
                    .checked_mul(2)
                    .ok_or_else(|| invalid(FORMAT, self.offset, "string length overflows"))?;
                let units: Vec<u16> = self
                    .bytes(row, len)?
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&c| u16::from_le_bytes(c))
                    .take_while(|&u| u != 0)
                    .collect();
                Value::Text(
                    String::from_utf16(&units)
                        .map_err(|_| invalid(FORMAT, self.offset, format!("{}: not UTF-16", self.name)))?,
                )
            }
        })
    }

    /// Whether the field is padding (`dummy8`).
    pub fn is_padding(&self) -> bool {
        self.ty == FieldType::Dummy8
    }
}

/// The first `N` bytes of `bytes` (the caller sliced exactly `N`; any shortfall reads as zeros).
fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    for (slot, b) in out.iter_mut().zip(bytes) {
        *slot = *b;
    }
    out
}

/// The layout of a row at one regulation version: its size and where each field is.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The bytes of a row.
    pub size: usize,
    /// The fields present at the version, in order.
    pub columns: Vec<Column>,
    containers: Vec<Container>,
}

impl Layout {
    /// The first column named `name`.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// The first container of `row` holding a set bit that no field declares: the byte offset and the undeclared bits
    /// that are set.
    pub fn undeclared_bits(&self, row: &[u8]) -> Option<(usize, u32)> {
        self.containers.iter().find_map(|c| {
            let width = usize::from(c.bits / 8);
            let bytes = c
                .offset
                .checked_add(width)
                .and_then(|end| row.get(c.offset..end))?;
            let word = u32::from_le_bytes(array(bytes));
            let rest = word.checked_shr(u32::from(c.used)).unwrap_or(0);
            (rest != 0).then(|| (c.offset, rest.checked_shl(u32::from(c.used)).unwrap_or(0)))
        })
    }
}

/// A set of paramdefs, by param type. A type can have several (revisions of one layout, or one file per game version);
/// [`ParamDefs::bind`] picks the one a param fits.
#[derive(Clone, Debug, Default)]
pub struct ParamDefs {
    by_type: BTreeMap<String, Vec<ParamDef>>,
}

impl ParamDefs {
    /// An empty set.
    pub fn new() -> ParamDefs {
        ParamDefs::default()
    }

    /// Add a definition.
    pub fn insert(&mut self, def: ParamDef) {
        self.by_type.entry(def.param_type.clone()).or_default().push(def);
    }

    /// Read every definition of `texts` (the text of each XML file) into a set.
    pub fn from_xml<'a>(texts: impl IntoIterator<Item = &'a str>) -> Result<ParamDefs> {
        let mut defs = ParamDefs::new();
        for text in texts {
            defs.insert(ParamDef::from_xml(text)?);
        }
        Ok(defs)
    }

    /// The definitions of `param_type`.
    pub fn for_type(&self, param_type: &str) -> &[ParamDef] {
        self.by_type.get(param_type).map_or(&[], Vec::as_slice)
    }

    /// How many definitions the set holds.
    pub fn len(&self) -> usize {
        self.by_type.values().map(Vec::len).sum()
    }

    /// Whether the set holds none.
    pub fn is_empty(&self) -> bool {
        self.by_type.is_empty()
    }

    /// Type `param`'s rows by the definition of its param type that fits it at `regulation` (the binder's regulation
    /// number, [`crate::Binder::regulation_number`]): the definitions of the type whose layout size is the file's row
    /// size (any of them, for a param with no rows); when several fit, the one whose data version is the file's.
    /// Every row is then read strictly: a text field must decode, and no bit that the definition does not declare may
    /// be set.
    pub fn bind<'a>(&'a self, param: &'a Param, regulation: Option<u32>) -> Result<TypedParam<'a>> {
        let candidates = self.for_type(&param.param_type);
        if candidates.is_empty() {
            return Err(invalid(
                FORMAT,
                0,
                format!("no definition for {}", param.param_type),
            ));
        }
        let mut fits = Vec::new();
        let mut sizes = Vec::new();
        for def in candidates {
            let layout = def.layout(regulation)?;
            sizes.push(layout.size);
            if param.rows.is_empty() || layout.size == param.row_size {
                fits.push((def, layout));
            }
        }
        let candidates_fitting = fits.len();
        if candidates_fitting > 1 {
            let by_version = fits
                .iter()
                .filter(|(def, _)| def.data_version == param.data_version)
                .count();
            if by_version == 1 {
                fits.retain(|(def, _)| def.data_version == param.data_version);
            }
        }
        let (def, layout) = match fits.len() {
            1 => fits.swap_remove(0),
            0 => {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!(
                        "layout size: {} has rows of {} bytes, its definitions give {:?} at regulation {:?}",
                        param.param_type, param.row_size, sizes, regulation
                    ),
                ));
            }
            n => {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!(
                        "ambiguous: {n} definitions of {} fit rows of {} bytes and data version {}",
                        param.param_type, param.row_size, param.data_version
                    ),
                ));
            }
        };
        for row in &param.rows {
            if let Some((offset, bits)) = layout.undeclared_bits(&row.data) {
                return Err(invalid(
                    FORMAT,
                    offset,
                    format!(
                        "undeclared bits: row {} of {} has bits {bits:#x} set that {} does not declare",
                        row.id, param.param_type, def.param_type
                    ),
                ));
            }
            for column in &layout.columns {
                column.read(&row.data).map_err(|e| {
                    invalid(
                        FORMAT,
                        0,
                        format!("text: row {} of {}: {e}", row.id, param.param_type),
                    )
                })?;
            }
        }
        Ok(TypedParam {
            param,
            def,
            layout,
            regulation,
        })
    }
}

/// A param whose rows are typed by a definition ([`ParamDefs::bind`]).
#[derive(Clone, Debug)]
pub struct TypedParam<'a> {
    /// The param as stored.
    pub param: &'a Param,
    /// The definition that fits it.
    pub def: &'a ParamDef,
    /// The row layout at the regulation.
    pub layout: Layout,
    /// The regulation number the layout is for.
    pub regulation: Option<u32>,
}

impl<'a> TypedParam<'a> {
    /// Whether the definition's data version is the file's. They differ in 10 of the 194 params of the Elden Ring
    /// 1.17.3 regulation, where the layout (by size) still fits: the revision numbers are not a gate.
    pub fn data_version_matches(&self) -> bool {
        self.def.data_version == self.param.data_version
    }

    /// Every row, typed.
    pub fn rows(&self) -> impl Iterator<Item = TypedRow<'_>> {
        self.param.rows.iter().map(|r| TypedRow {
            id: r.id,
            name: r.name.as_deref(),
            data: &r.data,
            layout: &self.layout,
        })
    }

    /// The first row with `id`.
    pub fn row(&self, id: i32) -> Option<TypedRow<'_>> {
        self.rows().find(|r| r.id == id)
    }
}

/// One typed row.
#[derive(Clone, Copy, Debug)]
pub struct TypedRow<'a> {
    /// The row's ID.
    pub id: i32,
    /// The row's name as stored ([`crate::param::Row::name`]).
    pub name: Option<&'a str>,
    /// The row's bytes.
    pub data: &'a [u8],
    layout: &'a Layout,
}

impl<'a> TypedRow<'a> {
    /// The value of the first field named `name`; `None` where the layout at the regulation has no such field.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.layout.column(name)?.read(self.data).ok()
    }

    /// Every field but the padding, with its name and value, in layout order.
    pub fn fields(&self) -> impl Iterator<Item = (&'a str, Value)> + use<'a> {
        let data = self.data;
        self.layout
            .columns
            .iter()
            .filter(|c| !c.is_padding())
            .filter_map(move |c| Some((c.name.as_str(), c.read(data).ok()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Binder;
    use crate::param::Row;

    fn xml(fields: &[&str]) -> String {
        let body: String = fields
            .iter()
            .map(|f| format!("    <Field Def=\"{f}\"/>\n"))
            .collect();
        format!(
            "<?xml version=\"1.0\"?>\n<PARAMDEF XmlVersion=\"2\">\n  <ParamType>T_ST</ParamType>\n  <DataVersion>3</DataVersion>\n  \
             <BigEndian>False</BigEndian>\n  <Unicode>True</Unicode>\n  <FormatVersion>203</FormatVersion>\n  <Fields>\n{body}  </Fields>\n</PARAMDEF>\n"
        )
    }

    fn def(fields: &[&str]) -> ParamDef {
        ParamDef::from_xml(&xml(fields)).unwrap()
    }

    fn param(row_size: usize, rows: &[(i32, Vec<u8>)]) -> Param {
        Param {
            param_type: "T_ST".into(),
            data_version: 3,
            unk06: 0,
            row_size,
            rows: rows
                .iter()
                .map(|(id, data)| Row {
                    id: *id,
                    name: None,
                    data: data.clone(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_container_rule_packs_bits_and_closes_on_other_fields() {
        let d = def(&[
            "u8 a:1",
            "dummy8 pad:7",
            "dummy8 gap[3]",
            "s32 id = -1",
            "u16 m:3",
            "u16 n:13",
            "u16 o:1",
            "u8 p:2",
            "u32 q:5",
            "u32 r:27",
            "u32 s:1",
            "f32 x",
        ]);
        assert_eq!(d.fields[3].default, Some(-1.0));
        let l = d.layout(None).unwrap();
        let at = |n: &str| {
            let c = l.column(n).unwrap();
            (c.offset, c.bits.map(|b| (b.shift, b.width)))
        };
        assert_eq!(at("a"), (0, Some((0, 1))));
        assert_eq!(at("pad"), (0, Some((1, 7))));
        assert_eq!(at("gap"), (1, None));
        assert_eq!(at("id"), (4, None));
        // m and n fill one u16 (3 + 13), o opens the next, p (a different width) another, q and r fill a u32
        assert_eq!(at("m"), (8, Some((0, 3))));
        assert_eq!(at("n"), (8, Some((3, 13))));
        assert_eq!(at("o"), (10, Some((0, 1))));
        assert_eq!(at("p"), (12, Some((0, 2))));
        assert_eq!(at("q"), (13, Some((0, 5))));
        assert_eq!(at("r"), (13, Some((5, 27))));
        assert_eq!(at("s"), (17, Some((0, 1))));
        assert_eq!(at("x"), (21, None));
        assert_eq!(l.size, 25);
    }

    #[test]
    fn a_bit_field_reads_its_bits_low_first_and_signed_types_extend() {
        let l = def(&["u8 a:1", "u8 b:3", "s8 c:4", "u16 w:12", "s16 v:4"])
            .layout(None)
            .unwrap();
        // 0b1010_0111: a = 1, b = 0b011, c (the same container: both are 8 bits wide) = 0b1010 = -6
        let row = [0b1010_0111u8, 0x34, 0xA2];
        let get = |n: &str| l.column(n).unwrap().read(&row).unwrap();
        assert_eq!(get("a"), Value::Unsigned(1));
        assert_eq!(get("b"), Value::Unsigned(0b011));
        assert_eq!(get("c"), Value::Signed(-6));
        assert_eq!(get("w"), Value::Unsigned(0x234));
        assert_eq!(get("v"), Value::Signed(-6));
        assert_eq!(l.size, 3);
    }

    #[test]
    fn scalars_and_strings_decode() {
        let d = def(&[
            "s8 a",
            "u8 b",
            "s16 c",
            "u16 d",
            "s32 e",
            "u32 f",
            "b32 g",
            "f32 h",
            "angle32 i",
            "f64 j",
            "fixstr k[6]",
            "fixstrW l[4]",
            "dummy8 m[2]",
        ]);
        let l = d.layout(None).unwrap();
        assert_eq!(l.size, 1 + 1 + 2 + 2 + 4 + 4 + 4 + 4 + 4 + 8 + 6 + 8 + 2);
        let mut row = vec![0xFFu8, 200, 0xFE, 0xFF, 0x2C, 0x01];
        row.extend((-70000i32).to_le_bytes());
        row.extend(4_000_000_000u32.to_le_bytes());
        row.extend(1i32.to_le_bytes());
        row.extend(1.5f32.to_le_bytes());
        row.extend(3.25f32.to_le_bytes());
        row.extend((-0.125f64).to_le_bytes());
        let text_at = row.len();
        // Shift-JIS "テス", a NUL, and a byte after it that is not text
        row.extend([0x83, 0x65, 0x83, 0x58, 0, 0x41]);
        let wide_at = row.len();
        for c in "aé".encode_utf16().chain([0, 0x41]) {
            row.extend(c.to_le_bytes());
        }
        row.extend([7, 9]);
        let get = |n: &str| l.column(n).unwrap().read(&row).unwrap();
        assert_eq!(get("a"), Value::Signed(-1));
        assert_eq!(get("b"), Value::Unsigned(200));
        assert_eq!(get("c"), Value::Signed(-2));
        assert_eq!(get("d"), Value::Unsigned(300));
        assert_eq!(get("e"), Value::Signed(-70000));
        assert_eq!(get("f"), Value::Unsigned(4_000_000_000));
        assert_eq!(get("g"), Value::Signed(1));
        assert_eq!(get("h"), Value::Float(1.5));
        assert_eq!(get("i"), Value::Float(3.25));
        assert_eq!(get("j"), Value::Double(-0.125));
        assert_eq!(get("k"), Value::Text("テス".into()));
        assert_eq!(get("l"), Value::Text("aé".into()));
        assert_eq!(get("m"), Value::Bytes(vec![7, 9]));
        assert_eq!(get("h").as_f64(), Some(1.5));
        assert_eq!(get("e").as_i64(), Some(-70000));
        assert_eq!(get("k").as_str(), Some("テス"));
        // text that is not Shift-JIS or not UTF-16 is refused, not replaced
        let mut bad = row.clone();
        bad[text_at] = 0x81;
        bad[text_at + 1] = 0x20;
        assert!(l.column("k").unwrap().read(&bad).is_err());
        let mut bad = row;
        bad[wide_at..wide_at + 2].copy_from_slice(&0xD800u16.to_le_bytes());
        assert!(l.column("l").unwrap().read(&bad).is_err());
    }

    #[test]
    fn fields_follow_the_regulation_they_exist_in() {
        let xml = xml(&["s32 old", "s32 added", "s32 gone"])
            .replace(
                "<Field Def=\"s32 added\"/>",
                "<Field Def=\"s32 added\" FirstVersion=\"200\"/>",
            )
            .replace(
                "<Field Def=\"s32 gone\"/>",
                "<Field Def=\"s32 gone\" RemovedVersion=\"300\"/>",
            );
        let d = ParamDef::from_xml(&xml).unwrap();
        assert!(d.version_aware());
        assert!(d.layout(None).is_err());
        let sizes: Vec<usize> = [199, 200, 299, 300]
            .into_iter()
            .map(|r| d.layout(Some(r)).unwrap().size)
            .collect();
        assert_eq!(sizes, [8, 12, 12, 8]);
        assert!(d.layout(Some(250)).unwrap().column("gone").is_some());
        assert!(d.layout(Some(300)).unwrap().column("gone").is_none());
        // a def with no gated field needs no regulation
        assert_eq!(def(&["s32 a"]).layout(None).unwrap().size, 4);
    }

    #[test]
    fn a_bracket_that_is_not_a_length_belongs_to_the_name() {
        let d = def(&["u8 GroundMaterialType [0,1,2,3]", "dummy8 pad[2]", "dummy8 one"]);
        assert_eq!(d.fields[0].name, "GroundMaterialType [0,1,2,3]");
        assert_eq!(d.layout(None).unwrap().size, 4);
    }

    #[test]
    fn definitions_that_cannot_be_laid_out_are_refused() {
        for bad in [
            "u8 a:9",
            "u8 a:0",
            "f32 a:3",
            "fixstr a",
            "fixstr a[0]",
            "u8 a[2]",
            "q32 a",
            "u8",
            "u8 a = x",
            "dummy8 a:3[2]",
        ] {
            assert!(ParamDef::from_xml(&xml(&[bad])).is_err(), "{bad}");
        }
        let good = xml(&["u8 a"]);
        for broken in [
            good.replace("XmlVersion=\"2\"", "XmlVersion=\"4\""),
            good.replace("False", "True"),
            good.replace("PARAMDEF", "PARAMDFE"),
            good.replace("<DataVersion>3</DataVersion>", ""),
            good.replace("<Field Def=\"u8 a\"/>", "<Field Def=\"u8 a\" Gated=\"1\"/>"),
        ] {
            assert!(ParamDef::from_xml(&broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn bind_picks_the_def_that_fits_and_refuses_the_rest() {
        let mut defs = ParamDefs::new();
        let mut small = def(&["s32 a"]);
        small.data_version = 2;
        defs.insert(small);
        defs.insert(def(&["s32 a", "u8 b:1", "dummy8 c:7", "dummy8 d[3]"]));
        assert_eq!(defs.len(), 2);
        // 8-byte rows: only the second def fits
        let p = param(
            8,
            &[
                (1, vec![1, 0, 0, 0, 1, 0, 0, 0]),
                (2, vec![255, 255, 255, 255, 0, 0, 0, 0]),
            ],
        );
        let typed = defs.bind(&p, None).unwrap();
        assert!(typed.data_version_matches());
        assert_eq!(typed.row(1).unwrap().get("b"), Some(Value::Unsigned(1)));
        assert_eq!(typed.row(2).unwrap().get("a"), Some(Value::Signed(-1)));
        let names: Vec<_> = typed.row(1).unwrap().fields().map(|(n, _)| n).collect();
        assert_eq!(names, ["a", "b"]);
        // 4-byte rows fit the older def, whose revision is not the file's
        let mut p4 = param(4, &[(1, vec![5, 0, 0, 0])]);
        let typed = defs.bind(&p4, None).unwrap();
        assert_eq!(typed.def.data_version, 2);
        assert!(!typed.data_version_matches());
        // two defs of one size: the file's data version decides
        let mut twin = def(&["u16 x", "u16 y"]);
        twin.data_version = 9;
        defs.insert(twin);
        p4.data_version = 9;
        assert_eq!(defs.bind(&p4, None).unwrap().def.data_version, 9);
        p4.data_version = 4;
        assert!(
            defs.bind(&p4, None)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        // no def of the type, or none of the row size
        let mut other = param(4, &[]);
        other.param_type = "OTHER_ST".into();
        assert!(
            defs.bind(&other, None)
                .unwrap_err()
                .to_string()
                .contains("no definition")
        );
        let wrong = param(6, &[(1, vec![0; 6])]);
        assert!(
            defs.bind(&wrong, None)
                .unwrap_err()
                .to_string()
                .contains("layout size")
        );
    }

    #[test]
    fn bind_refuses_bits_no_field_declares() {
        let defs = ParamDefs::from_xml([xml(&["u8 a:3", "s32 b"]).as_str()]).unwrap();
        let ok = param(5, &[(1, vec![0b101, 1, 0, 0, 0])]);
        assert!(defs.bind(&ok, None).is_ok());
        let stray = param(5, &[(1, vec![0b1000_0101, 1, 0, 0, 0])]);
        let why = defs.bind(&stray, None).unwrap_err().to_string();
        assert!(why.contains("undeclared bits"), "{why}");
        assert!(why.contains("0x80"), "{why}");
    }

    #[test]
    fn the_regulation_number_is_eight_digits() {
        let binder = |v: &[u8; 8]| Binder {
            version: *v,
            format: 0,
            files: Vec::new(),
            layout: crate::binder::Layout::default(),
        };
        assert_eq!(binder(b"11730034").regulation_number(), Some(11_730_034));
        assert_eq!(binder(b"07D7R6\0\0").regulation_number(), None);
        assert_eq!(binder(b"1173003 ").regulation_number(), None);
    }
}
