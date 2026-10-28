//! FXR effect files (`f000800042.fxr`, in the `sfxbnd_*.ffxbnd.dcx` binders): what an effect is made of, and the
//! lights in it.
//!
//! An FXR holds a state machine (states with conditions that move between them), a tree of nodes, each with the
//! configs its states choose from and the actions a node performs, and, in the actions, parameters stored as
//! properties (a constant or a keyframed function) and plain 32-bit field words. This reader keeps all of that as
//! stored: an [`Action`] is its type number, its field words, its properties and its field lists, and what a type
//! number means is the game's (the community tools name 80 of them). What it types is the lights: a
//! [`PointLight`] (action 609) and a [`SpotLight`] (action 11000), with the position in the tree that a placement
//! needs ([`Fxr::lights`]). `LightShaft` (action 10003) is read as an ordinary action: no file of either installed
//! game holds one.
//!
//! Read: version 5, header minor 0, the layout of every file in Sekiro's and Elden Ring's archives. Refused by their
//! header: version 4 (Dark Souls III, whose header lacks the three id lists) and minor 1 (Nightreign), neither of
//! which any installed file has.
//!
//! The sections of a file lie one after another, each starting at the 16-aligned end of the one before, and the last
//! ends at the file's end; the records of each section except the fields and the modifiers are named by their
//! parents exactly once, in order, so the reader requires the tiling. Field words and modifier lists may be shared
//! (a constant a compiler found twice is stored once and named by both properties; some words no record names).
//!
//! Sources: the layout is SoulsFormatsNEXT's `FXR3` (`Container`, `Effect`, `Action`, `Property`), the names of the
//! light parameters, their types and their defaults those of the `fxr` project (EvenTorset/fxr, Unlicense, which
//! calls containers nodes and effects configs; this module uses the project's names), and the property arithmetic
//! (which fields a keyframed function stores) is checked on every property of every file of both installs.

use crate::bytes::{Budget, View};
use crate::{Binder, Result, invalid};
use std::collections::BTreeSet;

const FORMAT: &str = "FXR";
/// The header of a version-5 file: 112 bytes and the four lists of the version-5 extension.
const HEADER: usize = 144;
const STATE_MAP: usize = 16;
const STATE: usize = 16;
const CONDITION: usize = 96;
const NODE: usize = 48;
const CONFIG: usize = 32;
const ACTION: usize = 64;
const PROPERTY: usize = 40;
const MODIFIER: usize = 32;
const COND_PROPERTY: usize = 24;
const FIELD_LIST: usize = 16;
const FIELD: usize = 4;
/// How deep a tree of nodes may nest (the installed files reach 9).
const MAX_DEPTH: usize = 64;
/// The action type of a node's state-to-config map.
pub const STATE_CONFIG_MAP: i16 = 199;
/// The action type of an effect that spawns another effect (the effect's id is the first field word).
pub const SFX_REFERENCE: i16 = 132;
/// The action type of a point light.
pub const POINT_LIGHT: i16 = 609;
/// The action type of a spot light.
pub const SPOT_LIGHT: i16 = 11000;
/// The action type of a light shaft (not typed here).
pub const LIGHT_SHAFT: i16 = 10003;
/// The action type of a static translation and rotation of a node (six float field words: offset, then rotation in degrees).
pub const STATIC_NODE_TRANSFORM: i16 = 35;

/// The name of the member of an `sfxbnd` binder that holds effect `id`: `f000800042.fxr`.
pub fn member_name(id: i32) -> String {
    format!("f{id:09}.fxr")
}

/// The effect id a binder member name spells (`f000800042.fxr`, in any directory and letter case), if it spells one.
pub fn member_id(name: &str) -> Option<i32> {
    let leaf = name.rsplit(['\\', '/']).next()?.to_ascii_lowercase();
    let digits = leaf.strip_prefix('f')?.strip_suffix(".fxr")?;
    if digits.len() != 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Every effect of a binder: the id its member name spells and the member's bytes read, in member order. Members that
/// are not `f%09d.fxr` are skipped; a member that does not read is an `Err` beside its id.
pub fn read_binder(binder: &Binder) -> Vec<(i32, Result<Fxr>)> {
    binder
        .files
        .iter()
        .filter_map(|f| {
            let id = member_id(f.name.as_deref()?)?;
            Some((id, Fxr::read(&f.bytes)))
        })
        .collect()
}

/// An operand of a state condition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operand {
    /// A literal number.
    Literal(f32),
    /// An external value the game sets (the number is the external value's index).
    External(i32),
    /// The time of day.
    TimeOfDay,
    /// The time spent in the state.
    StateTime,
}

/// How a condition compares its operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// `left != right`.
    NotEqual,
    /// `left == right`.
    Equal,
    /// `left >= right`.
    GreaterOrEqual,
    /// `left > right`.
    Greater,
}

/// One condition of a state: while the state holds, the comparison is tested; when it fails the effect moves to state
/// [`Condition::else_state`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Condition {
    /// How the operands compare.
    pub comparison: Comparison,
    /// The two bits above the comparison in the stored operator word (SoulsFormatsNEXT: "unknown modifier").
    pub modifier: u8,
    /// The state the effect moves to when the comparison fails, as stored.
    pub else_state: i32,
    /// The left operand.
    pub left: Operand,
    /// The right operand.
    pub right: Operand,
}

/// A state of the machine: its conditions, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    /// The state's conditions.
    pub conditions: Vec<Condition>,
}

/// How many components a property's value has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    /// One number.
    Scalar,
    /// Two numbers.
    Vector2,
    /// Three numbers.
    Vector3,
    /// Four numbers (a colour with alpha, or a vector of four).
    Vector4,
}

impl ValueKind {
    /// The number of components.
    pub fn components(self) -> usize {
        match self {
            ValueKind::Scalar => 1,
            ValueKind::Vector2 => 2,
            ValueKind::Vector3 => 3,
            ValueKind::Vector4 => 4,
        }
    }
}

/// The function a property evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    /// Every component is 0 (no field words).
    Zero,
    /// Every component is 1 (no field words).
    One,
    /// The components stored in the fields.
    Constant,
    /// Keyframes held until the next (a step function).
    Stepped,
    /// Keyframes joined by straight lines.
    Linear,
    /// Keyframes with Bezier tangents.
    Bezier,
    /// Keyframes with Hermite tangents.
    Hermite,
    /// Armored Core 6's per-component Hermite function: kept as stored, not interpreted (no installed file has one).
    ComponentHermite,
}

/// The keyframes of a stepped, linear, Bezier or Hermite property.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    /// The smallest value of each component over the keyframes (the first block of the stored fields, which equals
    /// the minimum in every property of the installed files).
    pub minimum: Vec<f32>,
    /// The largest value of each component (the second block).
    pub maximum: Vec<f32>,
    /// The position (time in seconds or other axis) of each keyframe, ascending.
    pub positions: Vec<f32>,
    /// The value at each keyframe, `components` numbers each.
    pub values: Vec<Vec<f32>>,
    /// For Bezier and Hermite: the two tangent (or control point) values at each keyframe, else `None`.
    pub tangents: Option<Tangents>,
}

/// The two blocks of tangent values a Bezier or Hermite property stores after its values, `components` numbers per
/// keyframe each (the `fxr` project calls them `p1`, `p2` for Bezier and `t1`, `t2` for Hermite).
#[derive(Clone, Debug, PartialEq)]
pub struct Tangents {
    /// The first block.
    pub first: Vec<Vec<f32>>,
    /// The second block.
    pub second: Vec<Vec<f32>>,
}

/// A modifier of a property (a random variation, an external-value dependence, ...): kept as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    /// The first type word as stored.
    pub type_a: u16,
    /// The second type word as stored.
    pub type_b: u32,
    /// The modifier's own field words.
    pub fields: Vec<u32>,
    /// The properties the modifier is conditioned on.
    pub properties: Vec<Property>,
}

/// A property: a value that is constant or a function of time.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    /// How many components the value has.
    pub kind: ValueKind,
    /// The function that gives the value.
    pub function: Function,
    /// Whether the function loops.
    pub looped: bool,
    /// The second type word as stored (`kind | function << 2 | loop << 4` in all but 0.1% of the installed
    /// properties).
    pub type_b: u32,
    /// The field words as stored.
    pub fields: Vec<u32>,
    /// The modifiers; empty for a property that belongs to a modifier.
    pub modifiers: Vec<Modifier>,
}

fn float(w: u32) -> f32 {
    f32::from_bits(w)
}

impl Property {
    /// The number of components of the value.
    pub fn components(&self) -> usize {
        self.kind.components()
    }

    /// The value of a zero, one or constant property.
    pub fn constant(&self) -> Option<Vec<f32>> {
        let c = self.components();
        match self.function {
            Function::Zero => Some(vec![0.0; c]),
            Function::One => Some(vec![1.0; c]),
            Function::Constant => Some(self.fields.iter().copied().map(float).collect()),
            _ => None,
        }
    }

    /// The keyframes of a stepped, linear, Bezier or Hermite property; `None` for the other functions. The field
    /// count of every such property is checked when the file is read, so the keyframes always decode.
    pub fn keyframes(&self) -> Option<Keyframes> {
        let c = self.components();
        let tangents = match self.function {
            Function::Stepped | Function::Linear => false,
            Function::Bezier | Function::Hermite => true,
            _ => return None,
        };
        let n = usize::try_from(*self.fields.first()?).ok()?;
        let block = |from: usize, len: usize| -> Option<Vec<f32>> {
            let to = from.checked_add(len)?;
            Some(self.fields.get(from..to)?.iter().copied().map(float).collect())
        };
        let rows = |from: usize| -> Option<Vec<Vec<f32>>> {
            (0..n)
                .map(|i| block(from.checked_add(i.checked_mul(c)?)?, c))
                .collect()
        };
        let header = c.checked_mul(2)?.checked_add(1)?;
        let values_at = header.checked_add(n)?;
        let step = n.checked_mul(c)?;
        Some(Keyframes {
            minimum: block(1, c)?,
            maximum: block(c.checked_add(1)?, c)?,
            positions: block(header, n)?,
            values: rows(values_at)?,
            tangents: if tangents {
                Some(Tangents {
                    first: rows(values_at.checked_add(step)?)?,
                    second: rows(values_at.checked_add(step.checked_mul(2)?)?)?,
                })
            } else {
                None
            },
        })
    }

    /// The value the property settles on: a constant's, or the last keyframe's value of a function that does not
    /// loop; `None` for a looping function.
    pub fn settled(&self) -> Option<Vec<f32>> {
        if let Some(v) = self.constant() {
            return Some(v);
        }
        if self.looped {
            return None;
        }
        self.keyframes()?.values.pop()
    }
}

/// An action of a node or a config: a type number and the parameters the type reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    /// The action's type (609 point light, 11000 spot light, 35 static node transform, ...).
    pub id: i16,
    /// The first flag byte as stored (0 or 1: 1 with the second 0 in 17% of the installed actions).
    pub unknown_02: u8,
    /// The second flag byte as stored.
    pub unknown_03: u8,
    /// The word after the flags as stored.
    pub unknown_04: i32,
    /// The first list of field words.
    pub fields1: Vec<u32>,
    /// The second list of field words.
    pub fields2: Vec<u32>,
    /// The first list of properties.
    pub properties1: Vec<Property>,
    /// The second list of properties.
    pub properties2: Vec<Property>,
    /// The lists of field words after the properties (the state-config map holds its config indices here).
    pub field_lists: Vec<Vec<u32>>,
}

/// A config of a node: the actions that run while the config is chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The config's id (1004 for the usual one).
    pub id: i16,
    /// The config's actions.
    pub actions: Vec<Action>,
}

/// A node of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The node's id (2000 for the root, 2200 for an ordinary node, ...).
    pub id: i16,
    /// The node's own actions.
    pub actions: Vec<Action>,
    /// The configs the node's states choose among.
    pub configs: Vec<Config>,
    /// The child nodes.
    pub children: Vec<Node>,
}

impl Node {
    /// The config index the node's state-config map gives each state, if it has the map.
    pub fn state_configs(&self) -> Option<&[u32]> {
        let map = self.actions.iter().find(|a| a.id == STATE_CONFIG_MAP)?;
        map.field_lists.first().map(Vec::as_slice)
    }

    /// The config the node runs in `state`: the one its state-config map gives (none for the map's -1 or an index
    /// the node lacks), else its only config.
    pub fn config_in_state(&self, state: usize) -> Option<&Config> {
        match self.state_configs() {
            Some(map) => self.configs.get(usize::try_from(*map.get(state)?).ok()?),
            None if self.configs.len() == 1 => self.configs.first(),
            None => None,
        }
    }
}

/// A static translation and rotation of a node (action 35): its children and actions are placed in the frame it makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticTransform {
    /// The translation, as stored.
    pub offset: [f32; 3],
    /// The rotation about each axis in degrees, as stored. The fxr project documents the order as Z, then X, then Y, with
    /// the translation after all rotations.
    pub rotation_degrees: [f32; 3],
}

impl Config {
    /// The config's static node transform (action 35, six float field words), if it has one; a config with more than
    /// one, or one with other counts or a non-finite word, is an error.
    pub fn static_transform(&self) -> Result<Option<StaticTransform>> {
        let mut found = None;
        for a in self.actions.iter().filter(|a| a.id == STATIC_NODE_TRANSFORM) {
            let counts = (
                a.fields1.len(),
                a.fields2.len(),
                a.properties1.len(),
                a.properties2.len(),
                a.field_lists.len(),
            );
            let bad = |detail: String| invalid(FORMAT, 0, format!("static node transform: {detail}"));
            if counts != (6, 0, 0, 0, 0) || found.is_some() {
                return Err(bad(format!("counts {counts:?}, or a second one in the config")));
            }
            let w = Words {
                id: a.id,
                list: &a.fields1,
            };
            found = Some(StaticTransform {
                offset: [w.float(0)?, w.float(1)?, w.float(2)?],
                rotation_degrees: [w.float(3)?, w.float(4)?, w.float(5)?],
            });
        }
        Ok(found)
    }
}

/// A whole effect.
#[derive(Clone, Debug, PartialEq)]
pub struct Fxr {
    /// The effect's id (the number in its file name).
    pub id: i32,
    /// The state machine's states.
    pub states: Vec<State>,
    /// The root node.
    pub root: Node,
    /// The first id list of the header: the effects this one spawns (the targets of its `SFXReference` actions).
    pub references: Vec<i32>,
    /// The second id list of the header (external values).
    pub external_values: Vec<i32>,
    /// The third id list of the header.
    pub blood_enablers: Vec<i32>,
}

#[derive(Clone, Copy)]
enum Section {
    States,
    Conditions,
    Nodes,
    Configs,
    Actions,
    Properties,
    Modifiers,
    CondProperties,
    FieldLists,
    Fields,
}

impl Section {
    const ALL: [Section; 10] = [
        Section::States,
        Section::Conditions,
        Section::Nodes,
        Section::Configs,
        Section::Actions,
        Section::Properties,
        Section::Modifiers,
        Section::CondProperties,
        Section::FieldLists,
        Section::Fields,
    ];
    fn index(self) -> usize {
        self as usize
    }
    fn size(self) -> usize {
        match self {
            Section::States => STATE,
            Section::Conditions => CONDITION,
            Section::Nodes => NODE,
            Section::Configs => CONFIG,
            Section::Actions => ACTION,
            Section::Properties => PROPERTY,
            Section::Modifiers => MODIFIER,
            Section::CondProperties => COND_PROPERTY,
            Section::FieldLists => FIELD_LIST,
            Section::Fields => FIELD,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Section::States => "states",
            Section::Conditions => "conditions",
            Section::Nodes => "nodes",
            Section::Configs => "configs",
            Section::Actions => "actions",
            Section::Properties => "properties",
            Section::Modifiers => "modifiers",
            Section::CondProperties => "conditional properties",
            Section::FieldLists => "field lists",
            Section::Fields => "fields",
        }
    }
    /// Whether the parents name the records of the section exactly once, in order.
    fn tiled(self) -> bool {
        !matches!(self, Section::Fields | Section::Modifiers)
    }
}

/// A section of the file: where its records lie and how many.
#[derive(Clone, Copy, Default)]
struct Extent {
    offset: usize,
    count: usize,
}

struct Reader<'a> {
    v: View<'a>,
    extents: [Extent; 10],
    claims: [Vec<(usize, usize)>; 10],
    budget: Budget,
}

fn align16(n: usize) -> Option<usize> {
    n.checked_add(15).map(|m| m & !15)
}

impl<'a> Reader<'a> {
    fn extent(&self, s: Section) -> Extent {
        self.extents.get(s.index()).copied().unwrap_or_default()
    }

    fn i32(&self, o: usize) -> Result<i32> {
        Ok(self.v.u32(o)? as i32)
    }

    fn zero(&self, o: usize, n: usize) -> Result<()> {
        if self.v.slice(o, n)?.iter().any(|&b| b != 0) {
            return Err(self.v.err(o, "a reserved word is not zero"));
        }
        Ok(())
    }

    /// A count word at `o`, with the zero word after it.
    fn count(&self, o: usize) -> Result<usize> {
        let n = usize::try_from(self.v.u32(o)?).map_err(|_| self.v.err(o, "count overflow"))?;
        Ok(n)
    }

    /// A pointer word at `o` (an offset in the low word, the high word zero).
    fn pointer(&self, o: usize) -> Result<usize> {
        self.zero(self.v.at(o, 4)?, 4)?;
        let p = self.v.u32(o)? as usize;
        if p > self.v.data.len() {
            return Err(self.v.err(
                o,
                format!(
                    "offset {p:#x} is beyond the {} bytes of the file",
                    self.v.data.len()
                ),
            ));
        }
        Ok(p)
    }

    /// `count` records of `section` named by a parent at `offset`: within the section, on a record boundary.
    fn claim(&mut self, section: Section, at: usize, offset: usize, count: usize) -> Result<()> {
        if count == 0 {
            return Ok(());
        }
        let e = self.extent(section);
        let size = section.size();
        let bytes = count
            .checked_mul(size)
            .ok_or_else(|| self.v.err(at, "size overflow"))?;
        let end = offset
            .checked_add(bytes)
            .ok_or_else(|| self.v.err(at, "size overflow"))?;
        let section_end = e
            .count
            .checked_mul(size)
            .and_then(|b| e.offset.checked_add(b))
            .ok_or_else(|| self.v.err(at, "size overflow"))?;
        let inside = offset >= e.offset && end <= section_end;
        let aligned = offset
            .checked_sub(e.offset)
            .is_some_and(|d| d.checked_rem(size) == Some(0));
        if !inside || (!aligned && section.tiled()) {
            return Err(self.v.err(
                at,
                format!(
                    "{count} {} at {offset:#x} lie outside the section ({:#x} to {section_end:#x})",
                    section.name(),
                    e.offset
                ),
            ));
        }
        if section.tiled()
            && let Some(list) = self.claims.get_mut(section.index())
        {
            list.push((offset, count));
        }
        Ok(())
    }

    /// The records of every tiled section were each named exactly once.
    fn finish(&self) -> Result<()> {
        for s in Section::ALL.into_iter().filter(|s| s.tiled()) {
            let e = self.extent(s);
            let mut spans = self.claims.get(s.index()).cloned().unwrap_or_default();
            spans.sort_unstable();
            let mut next = e.offset;
            for (offset, count) in spans {
                if offset != next {
                    return Err(self.v.err(
                        offset,
                        format!("the {} are not named in order: {next:#x} expected", s.name()),
                    ));
                }
                next = offset.saturating_add(count.saturating_mul(s.size()));
            }
            let end = e.offset.saturating_add(e.count.saturating_mul(s.size()));
            if next != end {
                return Err(self
                    .v
                    .err(next, format!("{} {} are named by no record", s.name(), e.count)));
            }
        }
        Ok(())
    }

    /// `count` field words at `offset` (inside the fields section).
    fn words(&mut self, at: usize, offset: usize, count: usize) -> Result<Vec<u32>> {
        self.claim(Section::Fields, at, offset, count)?;
        self.budget.charge(offset, count.saturating_mul(FIELD))?;
        (0..count)
            .map(|i| self.v.u32(self.v.element(offset, i, FIELD)?))
            .collect()
    }

    fn operand(&mut self, at: usize) -> Result<Operand> {
        // type word, flag bytes, has-field word, field offset
        let kind = i16::from_le_bytes(
            self.v
                .slice(at, 2)?
                .try_into()
                .map_err(|_| self.v.err(at, "short input"))?,
        );
        self.v.expect_at(at, 2, &[0, 1])?;
        self.zero(self.v.at(at, 4)?, 4)?;
        let has = self.v.u32_at(at, 8)?;
        self.zero(self.v.at(at, 12)?, 4)?;
        let field = self.v.u32_at(at, 16)? as usize;
        self.zero(self.v.at(at, 20)?, 20)?;
        let want = matches!(kind, -4 | -3);
        if has != u32::from(want) {
            return Err(self
                .v
                .err(at, format!("operand type {kind} with has-field word {has}")));
        }
        let word = if want {
            Some(
                *self
                    .words(at, field, 1)?
                    .first()
                    .ok_or_else(|| self.v.err(at, "no field"))?,
            )
        } else {
            None
        };
        Ok(match (kind, word) {
            (-4, Some(w)) => Operand::Literal(float(w)),
            (-3, Some(w)) => Operand::External(w as i32),
            (-2, None) => Operand::TimeOfDay,
            (-1, None) => Operand::StateTime,
            _ => return Err(self.v.err(at, format!("operand type {kind}"))),
        })
    }

    fn condition(&mut self, at: usize) -> Result<Condition> {
        let op = self.v.u16(at)?;
        if op > 0xF {
            return Err(self.v.err(at, format!("operator word {op:#x}")));
        }
        self.v.expect_at(at, 2, &[0, 1])?;
        self.zero(self.v.at(at, 4)?, 4)?;
        let else_state = self.i32(self.v.at(at, 8)?)?;
        self.zero(self.v.at(at, 12)?, 4)?;
        let left = self.operand(self.v.at(at, 16)?)?;
        let right = self.operand(self.v.at(at, 56)?)?;
        Ok(Condition {
            comparison: match op & 3 {
                0 => Comparison::NotEqual,
                1 => Comparison::Equal,
                2 => Comparison::GreaterOrEqual,
                _ => Comparison::Greater,
            },
            modifier: u8::try_from(op >> 2).map_err(|_| self.v.err(at, "operator word"))?,
            else_state,
            left,
            right,
        })
    }

    fn property(&mut self, at: usize, conditional: bool) -> Result<Property> {
        let word = self.v.u16(at)?;
        self.v.expect_at(at, 2, &[0, 1])?;
        let type_b = self.v.u32_at(at, 4)?;
        let count = self.count(self.v.at(at, 8)?)?;
        self.zero(self.v.at(at, 12)?, 4)?;
        let fields_at = self.v.u32_at(at, 16)? as usize;
        self.zero(self.v.at(at, 20)?, 4)?;
        if word & !0x10F3 != 0 {
            return Err(self.v.err(at, format!("property type word {word:#x}")));
        }
        let kind = match word & 3 {
            0 => ValueKind::Scalar,
            1 => ValueKind::Vector2,
            2 => ValueKind::Vector3,
            _ => ValueKind::Vector4,
        };
        let function = match (word >> 4) & 15 {
            0 => Function::Zero,
            1 => Function::One,
            2 => Function::Constant,
            3 => Function::Stepped,
            4 => Function::Linear,
            5 => Function::Bezier,
            6 => Function::Hermite,
            7 => Function::ComponentHermite,
            f => return Err(self.v.err(at, format!("property function {f}"))),
        };
        let looped = word & 0x1000 != 0;
        let c = kind.components();
        // the field words a function stores
        let fields = self.words(at, fields_at, count)?;
        let expected = match function {
            Function::Zero | Function::One => Some(0),
            Function::Constant => Some(c),
            Function::Stepped | Function::Linear | Function::Bezier | Function::Hermite => {
                let n = fields.first().map_or(0, |&n| n as usize);
                let tangents = matches!(function, Function::Bezier | Function::Hermite);
                let per_key = c.saturating_mul(if tangents { 3 } else { 1 });
                n.checked_mul(per_key)
                    .and_then(|v| v.checked_add(n))
                    .and_then(|v| v.checked_add(c.saturating_mul(2)))
                    .and_then(|v| v.checked_add(1))
            }
            Function::ComponentHermite => None,
        };
        if let Some(expected) = expected
            && expected != count
        {
            return Err(self.v.err(
                at,
                format!("a {function:?} property of {c} components stores {expected} words, not {count}"),
            ));
        }
        let modifiers = if conditional {
            Vec::new()
        } else {
            let modifiers_at = self.v.u32_at(at, 24)? as usize;
            self.zero(self.v.at(at, 28)?, 4)?;
            let n = self.count(self.v.at(at, 32)?)?;
            self.zero(self.v.at(at, 36)?, 4)?;
            self.modifiers(at, modifiers_at, n)?
        };
        Ok(Property {
            kind,
            function,
            looped,
            type_b,
            fields,
            modifiers,
        })
    }

    fn modifiers(&mut self, at: usize, offset: usize, count: usize) -> Result<Vec<Modifier>> {
        self.claim(Section::Modifiers, at, offset, count)?;
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let m = self.v.element(offset, i, MODIFIER)?;
            let type_a = self.v.u16(m)?;
            self.v.expect_at(m, 2, &[0, 1])?;
            let type_b = self.v.u32_at(m, 4)?;
            let field_count = self.count(self.v.at(m, 8)?)?;
            let property_count = self.count(self.v.at(m, 12)?)?;
            let fields_at = self.v.u32_at(m, 16)? as usize;
            self.zero(self.v.at(m, 20)?, 4)?;
            let properties_at = self.v.u32_at(m, 24)? as usize;
            self.zero(self.v.at(m, 28)?, 4)?;
            let fields = self.words(m, fields_at, field_count)?;
            self.claim(Section::CondProperties, m, properties_at, property_count)?;
            let mut properties = Vec::with_capacity(property_count);
            for j in 0..property_count {
                properties.push(self.property(self.v.element(properties_at, j, COND_PROPERTY)?, true)?);
            }
            out.push(Modifier {
                type_a,
                type_b,
                fields,
                properties,
            });
        }
        Ok(out)
    }

    fn action(&mut self, at: usize) -> Result<Action> {
        let id = self.v.u16(at)? as i16;
        let unknown_02 = self.v.u8_at(at, 2)?;
        let unknown_03 = self.v.u8_at(at, 3)?;
        let unknown_04 = self.i32(self.v.at(at, 4)?)?;
        let fields1 = self.count(self.v.at(at, 8)?)?;
        let lists = self.count(self.v.at(at, 12)?)?;
        let properties1 = self.count(self.v.at(at, 16)?)?;
        let fields2 = self.count(self.v.at(at, 20)?)?;
        self.zero(self.v.at(at, 24)?, 4)?;
        let properties2 = self.count(self.v.at(at, 28)?)?;
        let fields_at = self.v.u32_at(at, 32)? as usize;
        self.zero(self.v.at(at, 36)?, 4)?;
        let lists_at = self.v.u32_at(at, 40)? as usize;
        self.zero(self.v.at(at, 44)?, 4)?;
        let properties_at = self.v.u32_at(at, 48)? as usize;
        self.zero(self.v.at(at, 52)?, 12)?;
        let total_properties = properties1
            .checked_add(properties2)
            .ok_or_else(|| self.v.err(at, "size overflow"))?;
        self.claim(Section::Properties, at, properties_at, total_properties)?;
        let mut props = Vec::with_capacity(total_properties);
        for i in 0..total_properties {
            props.push(self.property(self.v.element(properties_at, i, PROPERTY)?, false)?);
        }
        let properties2 = props.split_off(properties1.min(props.len()));
        self.claim(Section::FieldLists, at, lists_at, lists)?;
        let mut field_lists = Vec::with_capacity(lists);
        for i in 0..lists {
            let l = self.v.element(lists_at, i, FIELD_LIST)?;
            let offset = self.v.u32(l)? as usize;
            self.zero(self.v.at(l, 4)?, 4)?;
            let n = self.count(self.v.at(l, 8)?)?;
            self.zero(self.v.at(l, 12)?, 4)?;
            field_lists.push(self.words(l, offset, n)?);
        }
        let total_fields = fields1
            .checked_add(fields2)
            .ok_or_else(|| self.v.err(at, "size overflow"))?;
        let mut words = self.words(at, fields_at, total_fields)?;
        let fields2 = words.split_off(fields1.min(words.len()));
        Ok(Action {
            id,
            unknown_02,
            unknown_03,
            unknown_04,
            fields1: words,
            fields2,
            properties1: props,
            properties2,
            field_lists,
        })
    }

    fn actions(&mut self, at: usize, offset: usize, count: usize) -> Result<Vec<Action>> {
        self.claim(Section::Actions, at, offset, count)?;
        (0..count)
            .map(|i| {
                let a = self.v.element(offset, i, ACTION)?;
                self.action(a)
            })
            .collect()
    }

    fn node(&mut self, at: usize, depth: usize) -> Result<Node> {
        if depth > MAX_DEPTH {
            return Err(self.v.err(at, format!("nodes nest deeper than {MAX_DEPTH}")));
        }
        let id = self.v.u16(at)? as i16;
        self.v.expect_at(at, 2, &[0, 1])?;
        self.zero(self.v.at(at, 4)?, 4)?;
        let configs = self.count(self.v.at(at, 8)?)?;
        let actions = self.count(self.v.at(at, 12)?)?;
        let children = self.count(self.v.at(at, 16)?)?;
        self.zero(self.v.at(at, 20)?, 4)?;
        let configs_at = self.pointer(self.v.at(at, 24)?)?;
        let actions_at = self.pointer(self.v.at(at, 32)?)?;
        let children_at = self.pointer(self.v.at(at, 40)?)?;
        self.claim(Section::Nodes, at, children_at, children)?;
        let mut kids = Vec::with_capacity(children);
        for i in 0..children {
            kids.push(self.node(self.v.element(children_at, i, NODE)?, depth.saturating_add(1))?);
        }
        self.claim(Section::Configs, at, configs_at, configs)?;
        let mut cfgs = Vec::with_capacity(configs);
        for i in 0..configs {
            let c = self.v.element(configs_at, i, CONFIG)?;
            let config_id = self.v.u16(c)? as i16;
            self.v.expect_at(c, 2, &[0, 1])?;
            self.zero(self.v.at(c, 4)?, 8)?;
            let n = self.count(self.v.at(c, 12)?)?;
            self.zero(self.v.at(c, 16)?, 8)?;
            let actions_at = self.pointer(self.v.at(c, 24)?)?;
            cfgs.push(Config {
                id: config_id,
                actions: self.actions(c, actions_at, n)?,
            });
        }
        let own = self.actions(at, actions_at, actions)?;
        Ok(Node {
            id,
            actions: own,
            configs: cfgs,
            children: kids,
        })
    }
}

impl Fxr {
    /// Read the file `data` (a `.fxr` member of an `sfxbnd` binder).
    pub fn read(data: &[u8]) -> Result<Fxr> {
        let v = View {
            data,
            format: FORMAT,
            be: false,
            base: 0,
        };
        v.expect(0, b"FXR\0")?;
        let minor = v.u16(4)?;
        if minor != 0 {
            return Err(v.err(
                4,
                format!("header minor {minor}: only minor 0 is read (minor 1 is Nightreign's)"),
            ));
        }
        let version = v.u16(6)?;
        if version != 5 {
            return Err(v.err(
                6,
                format!("version {version}: only version 5 (Sekiro, Elden Ring) is read (4 is Dark Souls III's, which has no header lists)"),
            ));
        }
        let mut r = Reader {
            v,
            extents: [Extent::default(); 10],
            claims: Default::default(),
            budget: Budget::new(FORMAT, data.len()),
        };
        r.v.expect(8, &[1, 0, 0, 0])?;
        let id = r.i32(12)?;
        let state_map = r.v.u32(16)? as usize;
        r.v.expect(20, &[1, 0, 0, 0])?;
        // sections 2 to 11, in the order of the header: states, conditions, nodes, configs, actions, properties,
        // modifiers, conditional properties, field lists, fields
        for (i, section) in Section::ALL.into_iter().enumerate() {
            let at = 24usize.saturating_add(i.saturating_mul(8));
            let offset = r.v.u32(at)? as usize;
            let count = r.v.u32(r.v.at(at, 4)?)? as usize;
            if let Some(e) = r.extents.get_mut(section.index()) {
                *e = Extent { offset, count };
            }
        }
        r.v.expect(104, &[1, 0, 0, 0, 0, 0, 0, 0])?;
        let list = |r: &Reader<'_>, at: usize| -> Result<Vec<i32>> {
            let offset = r.v.u32(at)? as usize;
            let count = r.count(r.v.at(at, 4)?)?;
            (0..count).map(|i| r.i32(r.v.element(offset, i, 4)?)).collect()
        };
        let references = list(&r, 112)?;
        let external_values = list(&r, 120)?;
        let blood_enablers = list(&r, 128)?;
        if r.v.u32(136)? != 0 || r.v.u32(140)? != 0 {
            return Err(r.v.err(136, "the fourth header list is not empty"));
        }
        // the layout: state map, then each section at the 16-aligned end of the one before, then the lists
        if state_map != HEADER {
            return Err(r.v.err(
                16,
                format!("state map at {state_map:#x}, after the header at {HEADER:#x} expected"),
            ));
        }
        let mut pos =
            align16(HEADER.saturating_add(STATE_MAP)).ok_or_else(|| r.v.err(16, "size overflow"))?;
        for s in Section::ALL {
            let e = r.extent(s);
            if e.count == 0 {
                continue;
            }
            if e.offset != pos {
                return Err(r.v.err(
                    24,
                    format!(
                        "the {} start at {:#x}, not at {pos:#x} where the section before ends",
                        s.name(),
                        e.offset
                    ),
                ));
            }
            let bytes = e
                .count
                .checked_mul(s.size())
                .ok_or_else(|| r.v.err(24, "size overflow"))?;
            r.v.slice(e.offset, bytes)?;
            pos = align16(e.offset.saturating_add(bytes)).ok_or_else(|| r.v.err(24, "size overflow"))?;
        }
        for (at, ids) in [
            (112, &references),
            (120, &external_values),
            (128, &blood_enablers),
        ] {
            if ids.is_empty() {
                continue;
            }
            let offset = r.v.u32(at)? as usize;
            if offset != pos {
                return Err(r.v.err(at, format!("an id list at {offset:#x}, not at {pos:#x}")));
            }
            pos = align16(offset.saturating_add(ids.len().saturating_mul(4)))
                .ok_or_else(|| r.v.err(at, "size overflow"))?;
        }
        if pos != data.len() {
            return Err(r.v.err(
                pos.min(data.len()),
                format!("the sections end at {pos:#x}, the file at {:#x}", data.len()),
            ));
        }
        // the state map, its states and their conditions
        let states = r.extent(Section::States);
        r.zero(state_map, 4)?;
        if r.count(r.v.at(state_map, 4)?)? != states.count
            || r.v.u32(r.v.at(state_map, 8)?)? as usize != states.offset
        {
            return Err(r
                .v
                .err(state_map, "the state map does not name the states section"));
        }
        r.zero(r.v.at(state_map, 12)?, 4)?;
        r.claim(Section::States, state_map, states.offset, states.count)?;
        let mut state_list = Vec::with_capacity(states.count);
        for i in 0..states.count {
            let s = r.v.element(states.offset, i, STATE)?;
            r.zero(s, 4)?;
            let n = r.count(r.v.at(s, 4)?)?;
            let conditions_at = r.v.u32_at(s, 8)? as usize;
            r.zero(r.v.at(s, 12)?, 4)?;
            r.claim(Section::Conditions, s, conditions_at, n)?;
            let mut conditions = Vec::with_capacity(n);
            for j in 0..n {
                conditions.push(r.condition(r.v.element(conditions_at, j, CONDITION)?)?);
            }
            state_list.push(State { conditions });
        }
        // the root node is the one container no node names
        let nodes = r.extent(Section::Nodes);
        if nodes.count == 0 {
            return Err(r.v.err(24, "no root node"));
        }
        r.claim(Section::Nodes, 24, nodes.offset, 1)?;
        let root = r.node(nodes.offset, 0)?;
        r.finish()?;
        Ok(Fxr {
            id,
            states: state_list,
            root,
            references,
            external_values,
            blood_enablers,
        })
    }

    /// Every action of the tree, node by node (a node's own actions, then each config's, then the children's).
    pub fn actions(&self) -> Vec<&Action> {
        fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Action>) {
            out.extend(node.actions.iter());
            for c in &node.configs {
                out.extend(c.actions.iter());
            }
            for child in &node.children {
                walk(child, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// The ids of the effects this one spawns, from its `SFXReference` actions (equal to [`Fxr::references`] in every
    /// installed file).
    pub fn spawned(&self) -> BTreeSet<i32> {
        self.actions()
            .into_iter()
            .filter(|a| a.id == SFX_REFERENCE)
            .filter_map(|a| a.fields1.first().map(|&w| w as i32))
            .collect()
    }

    /// Every point and spot light of the tree with its place in it, in tree order.
    pub fn lights(&self) -> Result<Vec<PlacedLight<'_>>> {
        fn walk<'a>(
            node: &'a Node,
            ancestors: &mut Vec<&'a Node>,
            out: &mut Vec<PlacedLight<'a>>,
        ) -> Result<()> {
            ancestors.push(node);
            for (index, config) in node.configs.iter().enumerate() {
                for action in &config.actions {
                    if let Some(light) = Light::from_action(action)? {
                        out.push(PlacedLight {
                            light,
                            action,
                            ancestors: ancestors.clone(),
                            config,
                            config_index: index,
                        });
                    }
                }
            }
            for child in &node.children {
                walk(child, ancestors, out)?;
            }
            ancestors.pop();
            Ok(())
        }
        // a light is an action of a config; one among a node's own actions would have no config to choose
        fn own(node: &Node, count: &mut usize) {
            *count = count.saturating_add(node.actions.iter().filter(|a| Light::kind(a.id)).count());
            for c in &node.children {
                own(c, count);
            }
        }
        let mut stray = 0;
        own(&self.root, &mut stray);
        if stray != 0 {
            return Err(invalid(
                FORMAT,
                0,
                format!("{stray} lights among a node's own actions, not in a config"),
            ));
        }
        let mut out = Vec::new();
        walk(&self.root, &mut Vec::new(), &mut out)?;
        Ok(out)
    }
}

/// A light and where it sits.
#[derive(Clone, Debug)]
pub struct PlacedLight<'a> {
    /// The typed light.
    pub light: Light,
    /// The light's action as stored.
    pub action: &'a Action,
    /// The nodes from the root down to the one whose config holds the light.
    pub ancestors: Vec<&'a Node>,
    /// The config that holds the light.
    pub config: &'a Config,
    /// The config's index in its node.
    pub config_index: usize,
}

impl PlacedLight<'_> {
    /// The node whose config holds the light.
    pub fn node(&self) -> Option<&Node> {
        self.ancestors.last().copied()
    }

    /// Whether the node's state-config map chooses the light's config in `state`; a node with one config and no map
    /// always runs it.
    pub fn active_in_state(&self, state: usize) -> bool {
        let Some(node) = self.node() else { return false };
        match node.state_configs() {
            Some(map) => map.get(state).is_some_and(|&c| c as usize == self.config_index),
            None => node.configs.len() == 1,
        }
    }

    /// The static transforms from the root down to the light: of the config each ancestor runs in `state`, and of the
    /// light's own config (which is the holder's, whether or not `state` chooses it). A node that runs no config in
    /// `state` (it has several and no map, or the map gives none) contributes nothing.
    pub fn static_transforms(&self, state: usize) -> Result<Vec<StaticTransform>> {
        let mut out = Vec::new();
        for (i, node) in self.ancestors.iter().enumerate() {
            let last = i.saturating_add(1) == self.ancestors.len();
            let config = if last {
                Some(self.config)
            } else {
                node.config_in_state(state)
            };
            if let Some(t) = config.map(Config::static_transform).transpose()?.flatten() {
                out.push(t);
            }
        }
        Ok(out)
    }
}

/// A typed light.
#[derive(Clone, Debug, PartialEq)]
pub enum Light {
    /// A point light (action 609).
    Point(PointLight),
    /// A spot light (action 11000).
    Spot(SpotLight),
}

/// Which layout of the light action a file stores. Elden Ring's lists are Sekiro's with words appended; the five
/// Sekiro files that hold Dark Souls III's layout (no multiplier properties, one more field word at index 12) are
/// `Legacy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revision {
    /// Dark Souls III's layout, found in five Sekiro point lights.
    Legacy,
    /// Sekiro's and Elden Ring's.
    Current,
}

/// What the flicker of a light does (the fields shared by both light types).
#[derive(Clone, Debug, PartialEq)]
pub struct Flicker {
    /// Whether the light jitters and flickers.
    pub enabled: bool,
    /// The jitter acceleration.
    pub acceleration: f32,
    /// The jitter along X, Y and Z.
    pub jitter: [f32; 3],
    /// The shortest and longest interval between flickers, in seconds.
    pub interval: [f32; 2],
    /// The brightness a flicker dims to.
    pub brightness: f32,
}

/// A point light (action 609).
#[derive(Clone, Debug, PartialEq)]
pub struct PointLight {
    /// The layout the action stores.
    pub revision: Revision,
    /// The diffuse colour (red, green, blue, alpha; unrestricted above 1).
    pub diffuse_color: Property,
    /// The specular colour, used when [`PointLight::separate_specular`] is set.
    pub specular_color: Property,
    /// The distance the light reaches, in metres.
    pub radius: Property,
    /// The diffuse brightness multiplier; absent in the legacy layout.
    pub diffuse_multiplier: Option<Property>,
    /// The specular brightness multiplier; absent in the legacy layout.
    pub specular_multiplier: Option<Property>,
    /// The flicker.
    pub flicker: Flicker,
    /// Whether the light casts shadows.
    pub shadows: bool,
    /// Whether the specular colour is its own (else the diffuse colour is both).
    pub separate_specular: bool,
    /// The fade-out time in frames (30 to the second).
    pub fade_out_frames: i32,
    /// How dark the shadows are.
    pub shadow_darkness: f32,
    /// The distance beyond which the light is not drawn (0: no limit).
    pub max_view_distance: f32,
    /// The density of the fake volumetric fog around the light; absent in the legacy layout.
    pub volume_density: Option<f32>,
    /// Whether the asymmetry parameter shapes the fog; absent where the list is too short.
    pub phase_function: Option<bool>,
    /// The fog's asymmetry parameter; absent where the list is too short.
    pub asymmetry_param: Option<f32>,
    /// The fog's adaptation exponent; absent where the list is too short.
    pub adaptation_exponent: Option<f32>,
}

/// A spot light (action 11000).
#[derive(Clone, Debug, PartialEq)]
pub struct SpotLight {
    /// The diffuse colour.
    pub diffuse_color: Property,
    /// The specular colour, used when [`SpotLight::separate_specular`] is set.
    pub specular_color: Property,
    /// The diffuse brightness multiplier.
    pub diffuse_multiplier: Property,
    /// The specular brightness multiplier.
    pub specular_multiplier: Property,
    /// The near plane distance.
    pub near: Property,
    /// The far plane distance.
    pub far: Property,
    /// The cone's half-extent along X at the far plane.
    pub radius_x: Property,
    /// The cone's half-extent along Y at the far plane.
    pub radius_y: Property,
    /// The flicker.
    pub flicker: Flicker,
    /// Whether the light casts shadows.
    pub shadows: bool,
    /// Whether the specular colour is its own.
    pub separate_specular: bool,
    /// How dark the shadows are.
    pub shadow_darkness: f32,
    /// The fade-out time in frames (30 to the second).
    pub fade_out_frames: i32,
    /// The density of the fake volumetric fog.
    pub volume_density: f32,
    /// Whether the asymmetry parameter shapes the fog; absent where the list is too short.
    pub phase_function: Option<bool>,
    /// The fog's asymmetry parameter; absent where the list is too short.
    pub asymmetry_param: Option<f32>,
    /// The fog's adaptation exponent; absent where the list is too short.
    pub adaptation_exponent: Option<f32>,
}

/// A light action's words, typed by position.
struct Words<'a> {
    id: i16,
    list: &'a [u32],
}

impl Words<'_> {
    fn err(&self, index: usize, detail: impl std::fmt::Display) -> crate::Error {
        invalid(
            FORMAT,
            0,
            format!("action {} field word {index}: {detail}", self.id),
        )
    }
    fn word(&self, index: usize) -> Option<u32> {
        self.list.get(index).copied()
    }
    fn float(&self, index: usize) -> Result<f32> {
        let w = self.word(index).ok_or_else(|| self.err(index, "absent"))?;
        let f = float(w);
        if !f.is_finite() {
            return Err(self.err(index, format!("{w:#x} is not a finite float")));
        }
        Ok(f)
    }
    fn boolean(&self, index: usize) -> Result<bool> {
        match self.word(index).ok_or_else(|| self.err(index, "absent"))? {
            0 => Ok(false),
            1 => Ok(true),
            w => Err(self.err(index, format!("{w} is not a boolean"))),
        }
    }
    fn int(&self, index: usize) -> Result<i32> {
        Ok(self.word(index).ok_or_else(|| self.err(index, "absent"))? as i32)
    }
    fn opt_float(&self, index: usize) -> Result<Option<f32>> {
        self.word(index).map(|_| self.float(index)).transpose()
    }
    fn opt_boolean(&self, index: usize) -> Result<Option<bool>> {
        self.word(index).map(|_| self.boolean(index)).transpose()
    }
}

fn flicker(w: &Words<'_>, at: usize) -> Result<Flicker> {
    // from `at`: enabled, acceleration, an unnamed float, jitter x y z, interval min max, brightness
    let k = |i: usize| at.saturating_add(i);
    Ok(Flicker {
        enabled: w.boolean(k(0))?,
        acceleration: w.float(k(1))?,
        jitter: [w.float(k(3))?, w.float(k(4))?, w.float(k(5))?],
        interval: [w.float(k(6))?, w.float(k(7))?],
        brightness: w.float(k(8))?,
    })
}

fn property_at(a: &Action, list: &[Property], index: usize, what: &str) -> Result<Property> {
    list.get(index).cloned().ok_or_else(|| {
        invalid(
            FORMAT,
            0,
            format!("action {}: no {what} property ({index} of {})", a.id, list.len()),
        )
    })
}

impl Light {
    fn kind(id: i16) -> bool {
        id == POINT_LIGHT || id == SPOT_LIGHT
    }

    /// The typed light an action holds: `None` for any other action, an error for a point or spot light whose lists
    /// have counts no installed file has.
    pub fn from_action(a: &Action) -> Result<Option<Light>> {
        match a.id {
            POINT_LIGHT => PointLight::from_action(a).map(|l| Some(Light::Point(l))),
            SPOT_LIGHT => SpotLight::from_action(a).map(|l| Some(Light::Spot(l))),
            _ => Ok(None),
        }
    }
}

impl PointLight {
    /// The point light an action 609 holds.
    pub fn from_action(a: &Action) -> Result<PointLight> {
        let counts = (
            a.properties1.len(),
            a.properties2.len(),
            a.fields1.len(),
            a.fields2.len(),
        );
        let revision = match counts {
            (10, 2, 2, 25) => Revision::Legacy,
            (10, 5, 2, 25..=33) => Revision::Current,
            _ => {
                return Err(invalid(
                    FORMAT,
                    0,
                    format!(
                        "point light with {counts:?} properties and fields: no installed file has these counts"
                    ),
                ));
            }
        };
        let w = Words {
            id: a.id,
            list: &a.fields2,
        };
        let legacy = revision == Revision::Legacy;
        // the legacy list has one word more at index 12, which moves the words after it by one
        let shift = usize::from(legacy);
        let at = |i: usize| i.saturating_add(shift);
        Ok(PointLight {
            revision,
            diffuse_color: property_at(a, &a.properties1, 0, "diffuse color")?,
            specular_color: property_at(a, &a.properties1, 1, "specular color")?,
            radius: property_at(a, &a.properties1, 2, "radius")?,
            diffuse_multiplier: (!legacy)
                .then(|| property_at(a, &a.properties2, 3, "diffuse multiplier"))
                .transpose()?,
            specular_multiplier: (!legacy)
                .then(|| property_at(a, &a.properties2, 4, "specular multiplier"))
                .transpose()?,
            flicker: flicker(&w, 1)?,
            shadows: w.boolean(10)?,
            separate_specular: w.boolean(11)?,
            fade_out_frames: w.int(at(12))?,
            shadow_darkness: w.float(at(13))?,
            max_view_distance: w.float(if legacy { 24 } else { 23 })?,
            volume_density: if legacy { None } else { w.opt_float(24)? },
            phase_function: if legacy { None } else { w.opt_boolean(26)? },
            asymmetry_param: if legacy { None } else { w.opt_float(27)? },
            adaptation_exponent: if legacy { None } else { w.opt_float(28)? },
        })
    }
}

impl SpotLight {
    /// The spot light an action 11000 holds.
    pub fn from_action(a: &Action) -> Result<SpotLight> {
        let counts = (
            a.properties1.len(),
            a.properties2.len(),
            a.fields1.len(),
            a.fields2.len(),
        );
        if !matches!(counts, (11, 0, 21..=28, 0)) {
            return Err(invalid(
                FORMAT,
                0,
                format!(
                    "spot light with {counts:?} properties and fields: no installed file has these counts"
                ),
            ));
        }
        let w = Words {
            id: a.id,
            list: &a.fields1,
        };
        Ok(SpotLight {
            diffuse_color: property_at(a, &a.properties1, 0, "diffuse color")?,
            specular_color: property_at(a, &a.properties1, 1, "specular color")?,
            diffuse_multiplier: property_at(a, &a.properties1, 2, "diffuse multiplier")?,
            specular_multiplier: property_at(a, &a.properties1, 3, "specular multiplier")?,
            near: property_at(a, &a.properties1, 4, "near")?,
            far: property_at(a, &a.properties1, 5, "far")?,
            radius_x: property_at(a, &a.properties1, 6, "radius x")?,
            radius_y: property_at(a, &a.properties1, 7, "radius y")?,
            flicker: flicker(&w, 1)?,
            shadows: w.boolean(10)?,
            separate_specular: w.boolean(11)?,
            shadow_darkness: w.float(12)?,
            fade_out_frames: w.int(15)?,
            volume_density: w.float(19)?,
            phase_function: w.opt_boolean(21)?,
            asymmetry_param: w.opt_float(22)?,
            adaptation_exponent: w.opt_float(23)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put16(b: &mut Vec<u8>, v: u16) {
        b.extend(v.to_le_bytes());
    }
    fn put32(b: &mut Vec<u8>, v: u32) {
        b.extend(v.to_le_bytes());
    }
    fn zeros(b: &mut Vec<u8>, words: usize) {
        b.extend(std::iter::repeat_n(0u8, words * 4));
    }
    fn pad16(b: &mut Vec<u8>) {
        while !b.len().is_multiple_of(16) {
            b.push(0);
        }
    }
    fn f(v: f32) -> u32 {
        v.to_bits()
    }

    /// A property record: kind, function, its field words' count and their offset (no modifiers).
    fn property(b: &mut Vec<u8>, kind: u16, function: u16, count: u32, offset: u32) {
        put16(b, kind | (function << 4));
        b.extend([0, 1]);
        put32(b, u32::from(kind) | (u32::from(function) << 2));
        put32(b, count);
        put32(b, 0);
        put32(b, offset);
        put32(b, 0);
        zeros(b, 4);
    }

    /// A version-5 file with one state, a root node and a child node whose one config holds a point light in the
    /// current layout: the colours and radius constant, the diffuse multiplier a two-keyframe linear function
    /// (`linear_words` words stored for it; 7 is right), the rest zero, one or constant.
    fn fixture(shadows: u32, linear_words: u32) -> Vec<u8> {
        let (nodes, configs, actions, properties) = (2usize, 1usize, 1usize, 15usize);
        let linear = 7usize;
        // the properties' words: diffuse 4, specular 4, radius 1, three scalars, 0.75, the linear 7, the specular
        // multiplier 1; then the action's fields1 (2) and fields2 (29)
        let prop_words = 4 + 4 + 1 + 3 + 1 + linear + 1;
        let fields_n = prop_words + 2 + 29;
        let mut at = 160usize;
        let mut next = |bytes: usize| {
            let o = at;
            at = (at + bytes + 15) & !15;
            o
        };
        let states_off = next(STATE);
        let nodes_off = next(nodes * NODE);
        let configs_off = next(configs * CONFIG);
        let actions_off = next(actions * ACTION);
        let props_off = next(properties * PROPERTY);
        let fields_off = next(fields_n * FIELD);
        let end = at;
        let mut b = Vec::new();
        b.extend(b"FXR\0");
        put16(&mut b, 0);
        put16(&mut b, 5);
        put32(&mut b, 1);
        put32(&mut b, 800_000_042);
        put32(&mut b, 144);
        put32(&mut b, 1);
        let section = |b: &mut Vec<u8>, off: usize, n: usize| {
            put32(b, if n == 0 { 0 } else { u32::try_from(off).unwrap() });
            put32(b, u32::try_from(n).unwrap());
        };
        section(&mut b, states_off, 1);
        section(&mut b, 0, 0); // conditions
        section(&mut b, nodes_off, nodes);
        section(&mut b, configs_off, configs);
        section(&mut b, actions_off, actions);
        section(&mut b, props_off, properties);
        section(&mut b, 0, 0); // modifiers
        section(&mut b, 0, 0); // conditional properties
        section(&mut b, 0, 0); // field lists
        section(&mut b, fields_off, fields_n);
        put32(&mut b, 1);
        put32(&mut b, 0);
        zeros(&mut b, 8); // the three id lists and the unused fourth
        assert_eq!(b.len(), 144);
        // the state map and its one state
        for w in [0, 1, u32::try_from(states_off).unwrap(), 0] {
            put32(&mut b, w);
        }
        pad16(&mut b);
        zeros(&mut b, 4);
        pad16(&mut b);
        // nodes: the root, then its child
        for (id, configs_n, children_n, children_at) in
            [(2000u16, 0u32, 1u32, nodes_off + NODE), (2200, 1, 0, 0)]
        {
            put16(&mut b, id);
            b.extend([0, 1]);
            put32(&mut b, 0);
            put32(&mut b, configs_n);
            put32(&mut b, 0); // actions
            put32(&mut b, children_n);
            put32(&mut b, 0);
            put32(
                &mut b,
                if configs_n == 0 {
                    0
                } else {
                    u32::try_from(configs_off).unwrap()
                },
            );
            put32(&mut b, 0);
            put32(&mut b, 0);
            put32(&mut b, 0);
            put32(
                &mut b,
                if children_n == 0 {
                    0
                } else {
                    u32::try_from(children_at).unwrap()
                },
            );
            put32(&mut b, 0);
        }
        pad16(&mut b);
        // the config
        put16(&mut b, 1004);
        b.extend([0, 1]);
        zeros(&mut b, 2);
        put32(&mut b, 1);
        zeros(&mut b, 2);
        put32(&mut b, u32::try_from(actions_off).unwrap());
        put32(&mut b, 0);
        pad16(&mut b);
        // the action
        put16(&mut b, 609);
        b.extend([0, 1]);
        put32(&mut b, 0);
        put32(&mut b, 2); // fields1
        put32(&mut b, 0); // field lists
        put32(&mut b, 10); // properties1
        put32(&mut b, 29); // fields2
        put32(&mut b, 0);
        put32(&mut b, 5); // properties2
        put32(&mut b, u32::try_from(fields_off + prop_words * FIELD).unwrap());
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, u32::try_from(props_off).unwrap());
        zeros(&mut b, 3);
        pad16(&mut b);
        // the properties; each constant names the next words of the fields section
        let mut word = fields_off;
        fn constant(b: &mut Vec<u8>, word: &mut usize, kind: u16, n: u32) {
            property(b, kind, 2, n, u32::try_from(*word).unwrap());
            *word += n as usize * FIELD;
        }
        constant(&mut b, &mut word, 3, 4); // diffuse
        constant(&mut b, &mut word, 3, 4); // specular
        constant(&mut b, &mut word, 0, 1); // radius
        for _ in 0..4 {
            property(&mut b, 0, 0, 0, 0); // zero
        }
        for _ in 0..3 {
            constant(&mut b, &mut word, 0, 1);
        }
        constant(&mut b, &mut word, 0, 1); // properties2: 0.75
        property(&mut b, 0, 1, 0, 0); // one
        property(&mut b, 0, 1, 0, 0); // one
        property(&mut b, 0, 4, linear_words, u32::try_from(word).unwrap()); // diffuse multiplier, linear
        word += linear * FIELD;
        constant(&mut b, &mut word, 0, 1); // specular multiplier
        pad16(&mut b);
        // the fields: the properties' words in the order above, then fields1 and fields2
        let mut words: Vec<u32> = vec![
            f(1.0),
            f(0.5),
            f(0.25),
            f(1.0),
            f(0.9),
            f(0.8),
            f(0.7),
            f(1.0),
            f(3.0),
        ];
        words.extend([f(10.0), f(10.0), f(10.0), f(0.75)]);
        words.extend([2, f(0.0), f(6.4), f(0.0), f(0.2), f(0.0), f(6.4)]);
        words.push(f(2.0));
        assert_eq!(words.len(), prop_words);
        words.extend([1, f(0.0)]);
        let mut fields2 = vec![0u32; 29];
        fields2[2] = f(1.0);
        fields2[8] = f(1.0);
        fields2[9] = f(0.5);
        fields2[10] = shadows;
        fields2[12] = 6;
        fields2[13] = f(0.8);
        fields2[26] = 1;
        words.extend(fields2);
        for w in &words {
            put32(&mut b, *w);
        }
        pad16(&mut b);
        assert_eq!(b.len(), end);
        b
    }

    #[test]
    fn a_point_light_in_a_hand_built_file_reads_as_stored() {
        let file = Fxr::read(&fixture(1, 7)).unwrap();
        assert_eq!(file.id, 800_000_042);
        assert_eq!(file.states.len(), 1);
        assert_eq!((file.root.id, file.root.children.len()), (2000, 1));
        let lights = file.lights().unwrap();
        assert_eq!(lights.len(), 1);
        let l = &lights[0];
        assert_eq!((l.ancestors.len(), l.config.id, l.config_index), (2, 1004, 0));
        assert!(l.active_in_state(0));
        let Light::Point(p) = &l.light else {
            panic!("{:?}", l.light)
        };
        assert_eq!(p.revision, Revision::Current);
        assert_eq!(p.diffuse_color.constant(), Some(vec![1.0, 0.5, 0.25, 1.0]));
        assert_eq!(p.radius.constant(), Some(vec![3.0]));
        let multiplier = p.diffuse_multiplier.as_ref().unwrap();
        let k = multiplier.keyframes().unwrap();
        assert_eq!(
            (k.positions, k.values),
            (vec![0.0, 0.2], vec![vec![0.0], vec![6.4]])
        );
        assert_eq!(multiplier.settled(), Some(vec![6.4]));
        assert_eq!(
            p.specular_multiplier.as_ref().unwrap().constant(),
            Some(vec![2.0])
        );
        assert!(p.shadows && !p.separate_specular);
        assert_eq!(p.fade_out_frames, 6);
        assert_eq!(p.phase_function, Some(true));
        assert_eq!(member_name(800_000_042), "f800000042.fxr");
        assert_eq!(
            member_id("N:\\GR\\data\\sfx\\effect\\F800000042.FXR"),
            Some(800_000_042)
        );
        assert_eq!(member_id("f80000042.fxr"), None);
    }

    #[test]
    fn refusals_name_their_reason() {
        let good = fixture(0, 7);
        let reason = |b: &[u8]| Fxr::read(b).unwrap_err().to_string();
        let mut v4 = good.clone();
        v4[6] = 4;
        assert!(reason(&v4).contains("version 4"), "{}", reason(&v4));
        let mut minor = good.clone();
        minor[4] = 1;
        assert!(reason(&minor).contains("minor 1"), "{}", reason(&minor));
        let short = &good[..good.len() - 16];
        assert!(reason(short).contains("short input"), "{}", reason(short));
        let mut long = good.clone();
        long.extend([0u8; 16]);
        assert!(reason(&long).contains("the sections end"), "{}", reason(&long));
        // a linear property that stores the wrong number of words
        assert!(
            reason(&fixture(0, 6)).contains("stores"),
            "{}",
            reason(&fixture(0, 6))
        );
        // a flag that is not a boolean is refused when the light is typed, not when the file is read
        let flag = Fxr::read(&fixture(2, 7)).unwrap();
        assert!(flag.lights().unwrap_err().to_string().contains("not a boolean"));
        // a node no record names: the first node record starts after the states section
        let mut orphan = good.clone();
        orphan[176 + 16] = 0;
        assert!(
            reason(&orphan).contains("named by no record"),
            "{}",
            reason(&orphan)
        );
    }

    #[test]
    fn a_property_stores_the_words_its_function_needs() {
        let mut p = Property {
            kind: ValueKind::Vector2,
            function: Function::Hermite,
            looped: false,
            type_b: 0,
            // two keyframes of two components: minimum, maximum, positions, values, then two tangent blocks
            fields: vec![
                2,
                f(1.0),
                f(2.0),
                f(3.0),
                f(4.0),
                f(0.0),
                f(1.0),
                f(10.0),
                f(20.0),
                f(30.0),
                f(40.0),
            ],
            modifiers: Vec::new(),
        };
        p.fields.extend([5, 6, 7, 8, 9, 10, 11, 12].map(|n| f(n as f32)));
        let k = p.keyframes().unwrap();
        assert_eq!(k.minimum, vec![1.0, 2.0]);
        assert_eq!(k.maximum, vec![3.0, 4.0]);
        assert_eq!(k.positions, vec![0.0, 1.0]);
        assert_eq!(k.values, vec![vec![10.0, 20.0], vec![30.0, 40.0]]);
        let t = k.tangents.unwrap();
        assert_eq!(t.first, vec![vec![5.0, 6.0], vec![7.0, 8.0]]);
        assert_eq!(t.second, vec![vec![9.0, 10.0], vec![11.0, 12.0]]);
        assert_eq!(p.settled(), Some(vec![30.0, 40.0]));
        p.looped = true;
        assert_eq!(p.settled(), None);
    }

    #[test]
    fn a_node_runs_the_config_its_map_gives_and_a_config_reads_its_transform() {
        let transform = |offset: [f32; 3], rotation: [f32; 3]| Action {
            id: STATIC_NODE_TRANSFORM,
            unknown_02: 0,
            unknown_03: 1,
            unknown_04: 0,
            fields1: offset.iter().chain(&rotation).map(|v| v.to_bits()).collect(),
            fields2: Vec::new(),
            properties1: Vec::new(),
            properties2: Vec::new(),
            field_lists: Vec::new(),
        };
        let config = |id: i16, actions: Vec<Action>| Config { id, actions };
        let map = |indices: &[u32]| Action {
            id: STATE_CONFIG_MAP,
            unknown_02: 0,
            unknown_03: 1,
            unknown_04: 0,
            fields1: Vec::new(),
            fields2: Vec::new(),
            properties1: Vec::new(),
            properties2: Vec::new(),
            field_lists: vec![indices.to_vec()],
        };
        let node = Node {
            id: 2200,
            actions: vec![map(&[1, u32::MAX])],
            configs: vec![
                config(1, Vec::new()),
                config(2, vec![transform([0.0, 1.5, 0.0], [0.0, 90.0, 0.0])]),
            ],
            children: Vec::new(),
        };
        assert_eq!(node.config_in_state(0).map(|c| c.id), Some(2));
        assert_eq!(node.config_in_state(1).map(|c| c.id), None);
        assert_eq!(node.config_in_state(2).map(|c| c.id), None);
        assert_eq!(
            node.configs[1].static_transform().unwrap(),
            Some(StaticTransform {
                offset: [0.0, 1.5, 0.0],
                rotation_degrees: [0.0, 90.0, 0.0],
            })
        );
        assert_eq!(node.configs[0].static_transform().unwrap(), None);
        // two in one config, or one that is not six floats, is refused
        let two = config(
            3,
            vec![transform([0.0; 3], [0.0; 3]), transform([0.0; 3], [0.0; 3])],
        );
        assert!(two.static_transform().is_err());
        let mut short = transform([0.0; 3], [0.0; 3]);
        short.fields1.pop();
        assert!(config(4, vec![short]).static_transform().is_err());
        let single = Node {
            id: 2200,
            actions: Vec::new(),
            configs: vec![config(1, Vec::new())],
            children: Vec::new(),
        };
        assert_eq!(single.config_in_state(5).map(|c| c.id), Some(1));
    }
}
