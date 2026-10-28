//! What a property's stored function evaluates to, and the typed form of a property's modifiers.
//!
//! The four keyframed functions (stepped, linear, Bezier, Hermite). Stepped, linear and Bezier follow the fxr project's
//! evaluation (EvenTorset/fxr, Unlicense: **sourced**; a Bezier segment is the cubic Hermite through the two keys with the
//! stored tangent values, which is also what Sekiro 1.06's code computes). The Hermite function is the game's own, read
//! from the decompiled executable (see `hermite_ease`: **inferred**, code read and not run), where the project has only an
//! approximation (the files store tangent angles, not tangents). A looping function takes its argument modulo its last
//! keyframe's position in the project and in the executable. The project's evaluation is compared with this one at probe
//! points over a sample of the installed properties (`tests/fxr_actions_oracle.rs`).

use super::{FORMAT, Function, Modifier, Property, Result, float, invalid};

/// A property modifier, typed. The kinds are the ones the installed files hold (type word A decoded as the fxr project
/// does: `(A >> 12 & 3) << 4 | A >> 4 & 15`; the low two bits are the value's component count minus one).
#[derive(Clone, Debug, PartialEq)]
pub enum ModifierKind {
    /// The value varies by up to `max` either way (`p - max` to `p + max`), per component, from a random stream chosen
    /// by `seeds` (one per component).
    RandomDelta {
        /// The seed of each component's stream.
        seeds: Vec<i32>,
        /// The largest difference of each component.
        max: Vec<f32>,
    },
    /// A random value between `min` and `max` is added.
    RandomRange {
        /// The seed of each component's stream.
        seeds: Vec<i32>,
        /// The smallest addition of each component.
        min: Vec<f32>,
        /// The largest addition of each component.
        max: Vec<f32>,
    },
    /// The value varies by up to the fraction `max` of itself either way (`p - p*max` to `p + p*max`).
    RandomFraction {
        /// The seed of each component's stream.
        seeds: Vec<i32>,
        /// The largest fraction of each component.
        max: Vec<f32>,
    },
    /// The value is multiplied by `factor` (a function of the external value's own argument) for an external value.
    ExternalValue1 {
        /// The external value's index.
        external_value: i32,
        /// The factor, a property of the modifier.
        factor: Property,
    },
    /// Like [`ModifierKind::ExternalValue1`] but fixed when the effect is created.
    ExternalValue2 {
        /// The external value's index.
        external_value: i32,
        /// The factor, a property of the modifier.
        factor: Property,
    },
}

impl Modifier {
    /// The number of components of the value the modifier applies to.
    pub fn components(&self) -> usize {
        usize::from(self.type_a & 3).saturating_add(1)
    }

    /// The modifier's kind number as the fxr project decodes type word A.
    pub fn type_number(&self) -> u16 {
        (((self.type_a >> 12) & 3) << 4) | ((self.type_a >> 4) & 15)
    }

    /// The typed modifier; an error for a type or field count no installed file has.
    pub fn kind(&self) -> Result<ModifierKind> {
        let c = self.components();
        let bad = |detail: String| {
            invalid(
                FORMAT,
                0,
                format!("modifier type {}: {detail}", self.type_number()),
            )
        };
        let ints =
            |from: usize| -> Vec<i32> { self.fields.iter().skip(from).take(c).map(|&w| w as i32).collect() };
        let floats = |from: usize| -> Result<Vec<f32>> {
            self.fields
                .iter()
                .skip(from)
                .take(c)
                .map(|&w| {
                    let f = float(w);
                    if f.is_finite() {
                        Ok(f)
                    } else {
                        Err(bad(format!("{w:#x} is not a finite float")))
                    }
                })
                .collect()
        };
        let expect = |fields: usize, properties: usize| -> Result<()> {
            if self.fields.len() == fields && self.properties.len() == properties {
                Ok(())
            } else {
                Err(bad(format!(
                    "{} field words and {} properties, expected {fields} and {properties}",
                    self.fields.len(),
                    self.properties.len()
                )))
            }
        };
        match self.type_number() {
            21 => {
                expect(c.saturating_mul(2), 0)?;
                Ok(ModifierKind::RandomDelta {
                    seeds: ints(0),
                    max: floats(c)?,
                })
            }
            24 => {
                expect(c.saturating_mul(3), 0)?;
                Ok(ModifierKind::RandomRange {
                    seeds: ints(0),
                    min: floats(c)?,
                    max: floats(c.saturating_mul(2))?,
                })
            }
            53 => {
                expect(c.saturating_mul(2), 0)?;
                Ok(ModifierKind::RandomFraction {
                    seeds: ints(0),
                    max: floats(c)?,
                })
            }
            n @ (38 | 39) => {
                expect(1, 1)?;
                let (Some(&word), Some(factor)) = (self.fields.first(), self.properties.first()) else {
                    return Err(bad("an external-value modifier without its word or factor".into()));
                };
                let (external_value, factor) = (word as i32, factor.clone());
                Ok(if n == 38 {
                    ModifierKind::ExternalValue1 {
                        external_value,
                        factor,
                    }
                } else {
                    ModifierKind::ExternalValue2 {
                        external_value,
                        factor,
                    }
                })
            }
            n => Err(bad(format!("type {n} is not one the installed files hold"))),
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn cubic_bezier(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let k = 1.0 - t;
    k * (k * k * p0 + 3.0 * t * (k * p1 + t * p2)) + t * t * t * p3
}

/// The tangent angle at which a Hermite segment is a straight line: 45 degrees.
const QUARTER_PI: f32 = std::f32::consts::FRAC_PI_4;

/// `H(0, m0, 1, m1; t)`: the cubic Hermite curve from 0 to 1 with start tangent `m0` and end tangent `m1`.
fn hermite_unit(m0: f32, m1: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    (3.0 * t2 - 2.0 * t3) + (t3 - 2.0 * t2 + t) * m0 + (t3 - t2) * m1
}

/// How far a Hermite segment has gone (0 to 1) at its normalised position `u`, given the tangent angles `s1` and `s2` the
/// file stores at its start key (radians: 0 flat, a quarter turn a straight line, a half a quarter turn steepest).
///
/// This is the evaluation in Sekiro 1.06's executable (the scalar Hermite property function), read from its decompiled
/// image: two reference easings, `a(u) = 1 - sqrt(1 - H(0, 0.7, 1, 1.5; u)^2)` (slow start) and `b(u) = sqrt(1 - (H(0, 1.5, 1, 0.7; u) - 1)^2)`
/// (slow end), the start angle blends `a` (0) to the line `u` (a quarter turn) to `b` (half a quarter turn), the end angle blends `b`
/// to the line to `a`, and the result is `e1 + u * (e2 - e1)`. **Inferred** (code read, not run); it agrees with the fxr
/// project's independent approximation to 0.5% of the largest key value over 34,803 probe points
/// (`tests/fxr_actions_oracle.rs`).
fn hermite_ease(s1: f32, s2: f32, u: f32) -> f32 {
    let h1 = hermite_unit(0.7, 1.5, u);
    let h2 = hermite_unit(1.5, 0.7, u);
    let a = 1.0 - (1.0 - h1 * h1).max(0.0).sqrt();
    let b = (1.0 - (h2 - 1.0) * (h2 - 1.0)).max(0.0).sqrt();
    let e1 = if s1 <= QUARTER_PI {
        a + (s1 / QUARTER_PI) * (u - a)
    } else {
        u + ((s1 - QUARTER_PI) / QUARTER_PI) * (b - u)
    };
    let e2 = if s2 <= QUARTER_PI {
        b + (s2 / QUARTER_PI) * (u - b)
    } else {
        u + ((s2 - QUARTER_PI) / QUARTER_PI) * (a - u)
    };
    e1 + u * (e2 - e1)
}

fn component(rows: &[Vec<f32>], key: usize, c: usize) -> f32 {
    rows.get(key).and_then(|r| r.get(c)).copied().unwrap_or(0.0)
}

impl Property {
    /// The value of the stored function at argument `x` (seconds since the argument's start), without modifiers, one
    /// number per component. A zero, one or constant property ignores `x`; a keyframed one holds its first value before
    /// its first keyframe and its last after its last, or, if it loops, takes `x` modulo its last keyframe's position (NaN
    /// when that is 0: the project says the games do the same). **Sourced** (see the module).
    pub fn value_at(&self, x: f32) -> Vec<f32> {
        if let Some(v) = self.constant() {
            return v;
        }
        let components = self.components();
        let Some(k) = self.keyframes() else {
            return vec![0.0; components];
        };
        if k.positions.is_empty() {
            return vec![0.0; components];
        }
        if k.positions.len() == 1 {
            return k.values.first().cloned().unwrap_or_else(|| vec![0.0; components]);
        }
        let x = if self.looped {
            let duration = k.positions.iter().copied().fold(0.0f32, f32::max);
            if duration == 0.0 {
                // the game's comparisons on a not-a-number argument fail and give the first keyframe
                return k.values.first().cloned().unwrap_or_else(|| vec![0.0; components]);
            }
            x % duration
        } else {
            x
        };
        // the keyframes whose position does not go backwards (the project drops the others)
        let mut keep: Vec<usize> = Vec::with_capacity(k.positions.len());
        let mut furthest = f32::NEG_INFINITY;
        for (i, &p) in k.positions.iter().enumerate() {
            if p >= furthest {
                furthest = p;
                keep.push(i);
            }
        }
        let mut prev = None;
        let mut next = None;
        for &i in &keep {
            if k.positions.get(i).is_some_and(|&p| p <= x) {
                prev = Some(i);
            } else {
                next = Some(i);
                break;
            }
        }
        let value = |i: usize| k.values.get(i).cloned().unwrap_or_else(|| vec![0.0; components]);
        let first = keep.first().copied().unwrap_or(0);
        if self.function == Function::Stepped {
            return value(prev.unwrap_or(first));
        }
        let (a, b) = match (prev, next) {
            (None, Some(b)) => return value(b),
            (Some(a), None) => return value(a),
            (Some(a), Some(b)) => (a, b),
            (None, None) => return value(first),
        };
        let (pa, pb) = (
            k.positions.get(a).copied().unwrap_or(0.0),
            k.positions.get(b).copied().unwrap_or(1.0),
        );
        let t = (x - pa) / (pb - pa);
        (0..components)
            .map(|c| {
                let (v1, v2) = (component(&k.values, a, c), component(&k.values, b, c));
                match (&self.function, &k.tangents) {
                    (Function::Bezier, Some(tg)) => {
                        let (t1, t2) = (component(&tg.first, a, c), component(&tg.second, b, c));
                        cubic_bezier(v1, v1 + t1 / 3.0, v2 - t2 / 3.0, v2, t)
                    }
                    (Function::Hermite, Some(tg)) => {
                        // both tangent angles are the previous keyframe's
                        lerp(
                            v1,
                            v2,
                            hermite_ease(component(&tg.first, a, c), component(&tg.second, a, c), t),
                        )
                    }
                    _ => lerp(v1, v2, t),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(
        function: Function,
        positions: &[f32],
        values: &[f32],
        tangents: Option<(&[f32], &[f32])>,
    ) -> Property {
        let n = positions.len();
        let mut fields = vec![n as u32];
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        fields.extend([min.to_bits(), max.to_bits()]);
        fields.extend(positions.iter().map(|p| p.to_bits()));
        fields.extend(values.iter().map(|v| v.to_bits()));
        if let Some((a, b)) = tangents {
            fields.extend(a.iter().chain(b).map(|v| v.to_bits()));
        }
        Property {
            kind: super::super::ValueKind::Scalar,
            function,
            looped: false,
            type_b: 0,
            fields,
            modifiers: Vec::new(),
        }
    }

    #[test]
    fn linear_and_stepped_keyframes() {
        let lin = property(Function::Linear, &[0.0, 2.0, 4.0], &[0.0, 10.0, 0.0], None);
        assert_eq!(lin.value_at(-1.0), [0.0]);
        assert_eq!(lin.value_at(1.0), [5.0]);
        assert_eq!(lin.value_at(2.0), [10.0]);
        assert_eq!(lin.value_at(3.0), [5.0]);
        assert_eq!(lin.value_at(9.0), [0.0]);
        let step = property(Function::Stepped, &[0.0, 2.0, 4.0], &[1.0, 2.0, 3.0], None);
        assert_eq!(step.value_at(-1.0), [1.0]);
        assert_eq!(step.value_at(1.9), [1.0]);
        assert_eq!(step.value_at(2.0), [2.0]);
        assert_eq!(step.value_at(100.0), [3.0]);
    }

    #[test]
    fn a_looping_function_wraps_at_its_last_keyframe() {
        let mut lin = property(Function::Linear, &[0.0, 2.0], &[0.0, 10.0], None);
        lin.looped = true;
        assert_eq!(lin.value_at(1.0), [5.0]);
        assert_eq!(lin.value_at(3.0), [5.0]);
        assert_eq!(lin.value_at(2.0), [0.0]);
        let mut empty = property(Function::Linear, &[0.0, 0.0], &[1.0, 2.0], None);
        empty.looped = true;
        assert_eq!(empty.value_at(1.0), [1.0]);
    }

    #[test]
    fn constants_ignore_the_argument() {
        assert_eq!(Property::constant_of(&[0.25, 0.5]).value_at(7.0), [0.25, 0.5]);
        let mut one = Property::constant_of(&[3.0]);
        one.function = Function::One;
        one.fields.clear();
        assert_eq!(one.value_at(1.0), [1.0]);
    }

    #[test]
    fn a_bezier_with_flat_tangents_eases_through_the_middle() {
        // p1 = p2 = 0: control points at the end values, so cubic_bezier(0, 0, 1, 1, 0.5) = 0.5
        let b = property(
            Function::Bezier,
            &[0.0, 1.0],
            &[0.0, 1.0],
            Some((&[0.0, 0.0], &[0.0, 0.0])),
        );
        assert!((b.value_at(0.5)[0] - 0.5).abs() < 1e-6);
        assert!(b.value_at(0.25)[0] < 0.25);
        assert!(b.value_at(0.75)[0] > 0.75);
        assert_eq!(b.value_at(0.0), [0.0]);
        assert_eq!(b.value_at(1.0), [1.0]);
    }

    #[test]
    fn the_hermite_function_is_a_straight_line_at_forty_five_degrees_and_eases_off_it() {
        let a = std::f32::consts::FRAC_PI_4;
        let h = property(
            Function::Hermite,
            &[0.0, 2.0],
            &[10.0, 20.0],
            Some((&[a, a], &[a, a])),
        );
        for (x, want) in [(0.5, 12.5), (1.0, 15.0), (1.5, 17.5)] {
            assert!((h.value_at(x)[0] - want).abs() < 1e-4, "{x}: {:?}", h.value_at(x));
        }
        // flat tangents ease in and out, a slow start (0, 45 degrees) stays under the line at the start and ends on it
        let flat = property(
            Function::Hermite,
            &[0.0, 1.0],
            &[0.0, 1.0],
            Some((&[0.0, 0.0], &[0.0, 0.0])),
        );
        assert_eq!(flat.value_at(0.0), [0.0]);
        assert_eq!(flat.value_at(1.0), [1.0]);
        assert!(flat.value_at(0.1)[0] < 0.1);
        assert!(flat.value_at(0.9)[0] > 0.9);
        assert!((flat.value_at(0.5)[0] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn modifiers_decode_by_type_word_and_field_count() {
        let delta = Modifier {
            type_a: 0xD050,
            type_b: 0,
            fields: vec![7, 0.5f32.to_bits()],
            properties: Vec::new(),
        };
        assert_eq!(delta.type_number(), 21);
        assert_eq!(delta.components(), 1);
        assert_eq!(
            delta.kind().unwrap(),
            ModifierKind::RandomDelta {
                seeds: vec![7],
                max: vec![0.5]
            }
        );
        let fraction4 = Modifier {
            type_a: 0xF053,
            type_b: 0,
            fields: vec![
                1,
                2,
                3,
                4,
                0.1f32.to_bits(),
                0.2f32.to_bits(),
                0.3f32.to_bits(),
                0.4f32.to_bits(),
            ],
            properties: Vec::new(),
        };
        assert_eq!(fraction4.type_number(), 53);
        assert_eq!(fraction4.components(), 4);
        assert!(
            matches!(fraction4.kind().unwrap(), ModifierKind::RandomFraction { ref seeds, .. } if seeds == &[1, 2, 3, 4])
        );
        // a count that does not fit, and a type no installed file has
        let short = Modifier {
            type_a: 0xD050,
            type_b: 0,
            fields: vec![7],
            properties: Vec::new(),
        };
        assert!(short.kind().is_err());
        let odd = Modifier {
            type_a: 0x0000,
            type_b: 0,
            fields: Vec::new(),
            properties: Vec::new(),
        };
        assert!(odd.kind().is_err());
        let external = Modifier {
            type_a: 0xE060,
            type_b: 0,
            fields: vec![5],
            properties: vec![Property::constant_of(&[2.0])],
        };
        assert!(matches!(
            external.kind().unwrap(),
            ModifierKind::ExternalValue1 {
                external_value: 5,
                ..
            }
        ));
    }
}
