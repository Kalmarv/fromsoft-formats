//! The reading machinery of the generated typed actions ([`super::actions`]): which list and word a named parameter is
//! at, its decoding (a property, a boolean, an integer, a float or a vector of floats), and the words and properties the
//! named parameters do not cover.

use super::{Action, FORMAT, Function, Property, Result, ValueKind, float, invalid};

/// One of an action's two field-word lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    /// The first field-word list.
    F1,
    /// The second field-word list.
    F2,
}

/// How a field word is read when a game stores as a word what another stores as a property.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordKind {
    /// A 32-bit integer, taken as its numeric value.
    Int,
    /// A float.
    Float,
    /// A boolean (0 or 1).
    Bool,
}

/// Where a parameter is in the lists of one layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The property at this index of the first property list.
    P1(usize),
    /// The property at this index of the second property list.
    P2(usize),
    /// The word at this index of the first field list.
    F1(usize),
    /// The word at this index of the second field list.
    F2(usize),
    /// A property of another layout, stored here as one word.
    Word(List, usize, WordKind),
}

/// How many entries each list of an action holds as stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stored {
    /// Properties in the first list.
    pub properties1: usize,
    /// Properties in the second list.
    pub properties2: usize,
    /// Params in the first field list.
    pub fields1: usize,
    /// Params in the second field list.
    pub fields2: usize,
}

/// What a typed action does not name: the entries past the named ones in each list (a file written for a newer game
/// than the project's names know), the lists of field words after the properties, and the action's three flag words.
#[derive(Clone, Debug, PartialEq)]
pub struct Extra {
    /// The list lengths as stored.
    pub stored: Stored,
    /// The properties of the first list past the named ones.
    pub properties1: Vec<Property>,
    /// The properties of the second list past the named ones.
    pub properties2: Vec<Property>,
    /// The words of the first field list past the named ones.
    pub fields1: Vec<u32>,
    /// The words of the second field list past the named ones.
    pub fields2: Vec<u32>,
    /// The lists of field words that follow the properties (the state-config map's indices, a random selector's weights).
    pub field_lists: Vec<Vec<u32>>,
    /// The two flag bytes and the word after them, as stored (`Action::unknown_02`, `unknown_03`, `unknown_04`).
    pub flags: (u8, u8, i32),
}

impl Extra {
    /// Whether the action holds anything the typed struct does not name.
    pub fn is_empty(&self) -> bool {
        self.properties1.is_empty()
            && self.properties2.is_empty()
            && self.fields1.is_empty()
            && self.fields2.is_empty()
            && self.field_lists.is_empty()
    }
}

pub(super) fn wrong_type(a: &Action, expected: i16) -> crate::Error {
    invalid(FORMAT, 0, format!("action {} read as action {expected}", a.id))
}

pub(super) fn unsupported(a: &Action) -> crate::Error {
    invalid(
        FORMAT,
        0,
        format!(
            "action {}: no typed reading (no installed file holds this type)",
            a.id
        ),
    )
}

/// A view of an action's lists for the generated readers.
pub struct Params<'a> {
    a: &'a Action,
}

impl<'a> Params<'a> {
    /// A view of `a`.
    pub fn new(a: &'a Action) -> Params<'a> {
        Params { a }
    }

    fn err(&self, name: &str, detail: impl std::fmt::Display) -> crate::Error {
        invalid(
            FORMAT,
            0,
            format!("action {} parameter {name}: {detail}", self.a.id),
        )
    }

    fn word_at(&self, list: List, i: usize) -> Option<u32> {
        match list {
            List::F1 => self.a.fields1.get(i).copied(),
            List::F2 => self.a.fields2.get(i).copied(),
        }
    }

    fn word(&self, at: Option<Place>) -> Option<u32> {
        match at? {
            Place::F1(i) => self.word_at(List::F1, i),
            Place::F2(i) => self.word_at(List::F2, i),
            Place::P1(_) | Place::P2(_) | Place::Word(..) => None,
        }
    }

    /// Which of a type's layouts the action is stored in. `counts` holds, per layout (Sekiro's, Elden Ring's, Dark Souls
    /// III's), how many properties and words each of the four lists names; the layouts are tried in the order the game's,
    /// the other game's, Dark Souls III's. The first whose two property lists equal the stored ones in length is taken (the
    /// property lists of a layout are always full, and any field words past the named ones are kept as extras); failing
    /// that, the first the stored lists fit by being no longer (older files end their field lists early).
    pub fn layout(&self, preferred: usize, counts: &[[usize; 4]]) -> Result<usize> {
        let a = self.a;
        let stored = [
            a.properties1.len(),
            a.properties2.len(),
            a.fields1.len(),
            a.fields2.len(),
        ];
        let [p1, p2, f1, f2] = stored;
        let other = usize::from(preferred == 0);
        let candidates = [preferred, other, counts.len().saturating_sub(1)];
        let exact = |l: usize| {
            counts
                .get(l)
                .is_some_and(|c| *c.first().unwrap_or(&0) == p1 && c.get(1).copied() == Some(p2))
        };
        let fits = |l: usize| {
            counts.get(l).is_some_and(|c| {
                let [c1, c2, c3, c4] = *c;
                p1 <= c1 && p2 <= c2 && f1 <= c3 && f2 <= c4
            })
        };
        candidates
            .into_iter()
            .find(|&l| exact(l))
            .or_else(|| candidates.into_iter().find(|&l| fits(l)))
            .ok_or_else(|| {
                invalid(
                    FORMAT,
                    0,
                    format!(
                        "action {}: stored lists {stored:?} fit none of the layouts {counts:?}",
                        a.id
                    ),
                )
            })
    }

    /// The property at `at`, or the constant `default` where the lists are too short to hold it.
    pub fn property(&self, at: Option<Place>, default: &[f32], name: &str) -> Result<Property> {
        let found = match at {
            Some(Place::P1(i)) => self.a.properties1.get(i),
            Some(Place::P2(i)) => self.a.properties2.get(i),
            Some(Place::Word(list, i, kind)) => {
                let Some(w) = self.word_at(list, i) else {
                    return Ok(Property::constant_of(default));
                };
                let v = match kind {
                    WordKind::Int => w as i32 as f32,
                    WordKind::Float => {
                        let f = float(w);
                        if !f.is_finite() {
                            return Err(self.err(name, format!("{w:#x} is not a finite float")));
                        }
                        f
                    }
                    WordKind::Bool if w <= 1 => w as f32,
                    WordKind::Bool => return Err(self.err(name, format!("{w:#x} is not a boolean"))),
                };
                return Ok(Property::constant_of(&[v]));
            }
            _ => None,
        };
        match found {
            Some(p) if p.components() == default.len() => Ok(p.clone()),
            Some(p) => Err(self.err(
                name,
                format!("{} components stored, {} named", p.components(), default.len()),
            )),
            None => Ok(Property::constant_of(default)),
        }
    }

    /// A one-word boolean (0 or 1), or `default`.
    pub fn boolean(&self, at: Option<Place>, default: bool, name: &str) -> Result<bool> {
        match self.word(at) {
            None => Ok(default),
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            Some(w) => Err(self.err(name, format!("{w:#x} is not a boolean"))),
        }
    }

    /// A one-word integer, or `default`.
    pub fn int(&self, at: Option<Place>, default: i32, _name: &str) -> Result<i32> {
        Ok(self.word(at).map_or(default, |w| w as i32))
    }

    /// A one-word float, or `default`. A word that is not a finite float is an error.
    pub fn float(&self, at: Option<Place>, default: f32, name: &str) -> Result<f32> {
        match self.word(at) {
            None => Ok(default),
            Some(w) => {
                let f = float(w);
                if f.is_finite() {
                    Ok(f)
                } else {
                    Err(self.err(name, format!("{w:#x} is not a finite float")))
                }
            }
        }
    }

    fn vec<const N: usize>(&self, at: Option<Place>, default: [f32; N], name: &str) -> Result<[f32; N]> {
        let (list, i) = match at {
            Some(Place::F1(i)) => (List::F1, i),
            Some(Place::F2(i)) => (List::F2, i),
            _ => return Ok(default),
        };
        let mut out = default;
        let mut seen = 0usize;
        for (k, slot) in out.iter_mut().enumerate() {
            if let Some(w) = i.checked_add(k).and_then(|j| self.word_at(list, j)) {
                let f = float(w);
                if !f.is_finite() {
                    return Err(self.err(name, format!("component {k}: {w:#x} is not a finite float")));
                }
                *slot = f;
                seen = seen.saturating_add(1);
            }
        }
        match seen {
            0 => Ok(default),
            n if n == N => Ok(out),
            n => Err(self.err(name, format!("{n} of {N} components stored"))),
        }
    }

    /// A two-float vector, or `default`.
    #[allow(dead_code)]
    pub fn vec2(&self, at: Option<Place>, default: [f32; 2], name: &str) -> Result<[f32; 2]> {
        self.vec(at, default, name)
    }

    /// A three-float vector, or `default`.
    pub fn vec3(&self, at: Option<Place>, default: [f32; 3], name: &str) -> Result<[f32; 3]> {
        self.vec(at, default, name)
    }

    /// A four-float vector, or `default`.
    pub fn vec4(&self, at: Option<Place>, default: [f32; 4], name: &str) -> Result<[f32; 4]> {
        self.vec(at, default, name)
    }

    /// What lies past the named entries: `named` is the number of entries the game's names cover in each list
    /// (properties 1, properties 2, field words 1, field words 2).
    pub fn extra(&self, named: [usize; 4]) -> Extra {
        let a = self.a;
        Extra {
            stored: Stored {
                properties1: a.properties1.len(),
                properties2: a.properties2.len(),
                fields1: a.fields1.len(),
                fields2: a.fields2.len(),
            },
            properties1: a.properties1.iter().skip(named[0]).cloned().collect(),
            properties2: a.properties2.iter().skip(named[1]).cloned().collect(),
            fields1: a.fields1.iter().skip(named[2]).copied().collect(),
            fields2: a.fields2.iter().skip(named[3]).copied().collect(),
            field_lists: a.field_lists.clone(),
            flags: (a.unknown_02, a.unknown_03, a.unknown_04),
        }
    }
}

impl Property {
    /// The integer a zero, one or constant property of one component holds (a constant is rounded to the nearest
    /// integer: the files store an enum or an id in a property as a float); `None` for any other property.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the value is finite and under 2e9 in magnitude"
    )]
    pub fn as_integer(&self) -> Option<i32> {
        match (self.components(), self.constant()?.as_slice()) {
            (1, &[v]) if v.is_finite() && v.abs() < 2.0e9 => Some(v.round() as i32),
            _ => None,
        }
    }

    /// A constant property holding `values` (one to four components): what a typed action holds for a property its lists
    /// are too short to store.
    pub fn constant_of(values: &[f32]) -> Property {
        let (kind, components_less_one) = match values.len() {
            1 => (ValueKind::Scalar, 0u32),
            2 => (ValueKind::Vector2, 1),
            3 => (ValueKind::Vector3, 2),
            _ => (ValueKind::Vector4, 3),
        };
        Property {
            kind,
            function: Function::Constant,
            looped: false,
            // the second type word: components less one, then the function (2) shifted by two
            type_b: components_less_one | 0b1000,
            fields: values.iter().map(|v| v.to_bits()).collect(),
            modifiers: Vec::new(),
        }
    }
}

/// The value of a named parameter of a typed action.
#[derive(Clone, Debug, PartialEq)]
pub enum Param<'a> {
    /// A boolean field.
    Bool(bool),
    /// An integer field.
    Int(i32),
    /// A float field.
    Float(f32),
    /// A vector field of two to four floats.
    Vector(Vec<f32>),
    /// A property.
    Property(&'a Property),
}
