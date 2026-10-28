//! The constraint data of every character, part, object, asset and map tagfile of the installs, read into the typed atoms
//! of `havok_formats::constraint`: every `hknpConstraintCinfo` of both games reads, with each atom the type of its slot.
//!
//! The counts are those of a census over the same files (a probe that walked the type tables and counted the class of
//! every `constraintData` and the value of each atom's members, written before the reader): each tagfile is counted as
//! often as a binder repeats it. The census also found: every rotation of a stored transform orthonormal to 1.2e-6 and
//! right-handed, every limit's minimum at most its maximum, no float that is not finite, and every integer member constant
//! per class (the flags, the axes, the factor 48), which the second half of each test asserts through the reader.
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and `oracles.uxm.{sekiro,elden}/dictionary.txt`;
//! a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod hkwalk;
mod inputs;

use std::collections::BTreeMap;

use fromsoft_formats::Game;
use havok_formats::constraint::{ConstraintData, Transform, read_constraint_data};
use hkwalk::{Member, Result, Set};

#[derive(Debug, Default, PartialEq)]
struct Tally {
    /// Constraint data objects by class (a breakable one counted as itself and, wrapped, as `breakable/<class>`).
    classes: BTreeMap<String, usize>,
    /// The worst error of a rotation column's length or two columns' dot product, over every stored transform.
    worst_orthonormality: f64,
    /// Transforms whose rotation has a negative determinant.
    left_handed: usize,
    /// Limits whose minimum exceeds their maximum.
    inverted_limits: usize,
    /// Atoms that differ from the constants of their class (the flags, axes and factor the census found constant).
    off_constant: usize,
    /// Position motors found behind the motor pointers of ragdoll and hinge atoms, and null motor pointers.
    motors: usize,
    null_motors: usize,
}

impl Tally {
    fn merge(&mut self, o: Self) {
        for (class, n) in o.classes {
            *self.classes.entry(class).or_default() += n;
        }
        self.worst_orthonormality = self.worst_orthonormality.max(o.worst_orthonormality);
        self.left_handed += o.left_handed;
        self.inverted_limits += o.inverted_limits;
        self.off_constant += o.off_constant;
        self.motors += o.motors;
        self.null_motors += o.null_motors;
    }

    fn transform(&mut self, t: &Transform) {
        let c = t.columns.map(|c| c.map(f64::from));
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let error = [
            dot(c[0], c[0]) - 1.0,
            dot(c[1], c[1]) - 1.0,
            dot(c[2], c[2]) - 1.0,
            dot(c[0], c[1]),
            dot(c[0], c[2]),
            dot(c[1], c[2]),
        ]
        .into_iter()
        .fold(0.0f64, |worst, e| worst.max(e.abs()));
        self.worst_orthonormality = self.worst_orthonormality.max(error);
        let cross = [
            c[1][1] * c[2][2] - c[1][2] * c[2][1],
            c[1][2] * c[2][0] - c[1][0] * c[2][2],
            c[1][0] * c[2][1] - c[1][1] * c[2][0],
        ];
        if dot(c[0], cross) <= 0.0 {
            self.left_handed += 1;
        }
    }

    fn limit(&mut self, min: f32, max: f32) {
        if min > max {
            self.inverted_limits += 1;
        }
    }

    fn data(&mut self, data: &ConstraintData) {
        match data {
            ConstraintData::Ragdoll(r) => {
                self.transform(&r.transforms.transform_a);
                self.transform(&r.transforms.transform_b);
                self.limit(r.twist.min_angle, r.twist.max_angle);
                self.limit(r.cone.min_angle, r.cone.max_angle);
                self.limit(r.planes.min_angle, r.planes.max_angle);
                for m in r.motors.motors {
                    if m.is_some() {
                        self.motors += 1;
                    } else {
                        self.null_motors += 1;
                    }
                }
                // enabled: friction, twist, cone and planes; not the motors; axes and measurement modes per slot
                let same = r.friction.enabled
                    && r.twist.enabled
                    && r.cone.enabled
                    && r.planes.enabled
                    && !r.motors.enabled
                    && r.friction.num_friction_axes == 3
                    && (r.twist.twist_axis, r.twist.ref_axis) == (0, 1)
                    && (
                        r.cone.twist_axis_in_a,
                        r.cone.ref_axis_in_b,
                        r.cone.angle_measurement_mode,
                    ) == (0, 0, 0)
                    && (
                        r.planes.twist_axis_in_a,
                        r.planes.ref_axis_in_b,
                        r.planes.angle_measurement_mode,
                    ) == (0, 1, 1)
                    && r.ball_socket.velocity_stabilization_factor == 48;
                self.off_constant += usize::from(!same);
            }
            ConstraintData::LimitedHinge(h) => {
                self.transform(&h.transforms.transform_a);
                self.transform(&h.transforms.transform_b);
                self.limit(h.limit.min_angle, h.limit.max_angle);
                if h.motor.motor.is_some() {
                    self.motors += 1;
                } else {
                    self.null_motors += 1;
                }
                let same = h.friction.enabled
                    && h.limit.enabled
                    && !h.motor.enabled
                    && h.friction.num_friction_axes == 1
                    && (h.limit.limit_axis, h.limit.cosine_axis) == (0, 1)
                    && h.ball_socket.velocity_stabilization_factor == 48;
                self.off_constant += usize::from(!same);
            }
            ConstraintData::Hinge(h) => {
                self.transform(&h.transforms.transform_a);
                self.transform(&h.transforms.transform_b);
                self.off_constant += usize::from(h.ball_socket.velocity_stabilization_factor != 48);
            }
            ConstraintData::Fixed(f) => {
                self.transform(&f.transforms.transform_a);
                self.transform(&f.transforms.transform_b);
                self.off_constant += usize::from(f.ball_socket.velocity_stabilization_factor != 48);
            }
            ConstraintData::BallAndSocket(b) => {
                self.off_constant += usize::from(b.ball_socket.velocity_stabilization_factor != 48);
            }
            ConstraintData::Wheel(w) => {
                self.transform(&w.suspension_base.transform_a);
                self.transform(&w.suspension_base.transform_b);
                self.limit(w.suspension_min, w.suspension_max);
            }
            ConstraintData::Breakable { constraint, .. } => self.data(constraint),
        }
    }
}

fn class_name(data: &ConstraintData) -> &'static str {
    match data {
        ConstraintData::Ragdoll(_) => "ragdoll",
        ConstraintData::LimitedHinge(_) => "limited hinge",
        ConstraintData::Hinge(_) => "hinge",
        ConstraintData::BallAndSocket(_) => "ball and socket",
        ConstraintData::Fixed(_) => "fixed",
        ConstraintData::Wheel(_) => "wheel",
        ConstraintData::Breakable { .. } => "breakable",
    }
}

fn visit(t: &mut Tally, m: &Member<'_>) -> Result<()> {
    let tag = m.tag;
    for (i, item) in tag.items.iter().enumerate() {
        if i == 0 || tag.types.get(item.type_index).map(|ty| ty.name.as_str()) != Some("hknpConstraintCinfo")
        {
            continue;
        }
        let items = tag.item(i)?;
        for k in 0..item.count {
            let data = items.get(k)?.field("constraintData")?.dereference()?;
            let read = read_constraint_data(data)?;
            *t.classes.entry(class_name(&read).to_owned()).or_default() += 1;
            if let ConstraintData::Breakable { constraint, .. } = &read {
                *t.classes
                    .entry(format!("breakable/{}", class_name(constraint)))
                    .or_default() += 1;
            }
            t.data(&read);
        }
    }
    Ok(())
}

fn tally(game: Game) -> Result<Option<Tally>> {
    let Some(install) = hkwalk::open(game, &[Set::Mesh, Set::Extern, Set::Characters])? else {
        return Ok(None);
    };
    let t = install.run(visit, Tally::merge)?;
    eprintln!(
        "{} constraint atoms: {} binders, {t:?}",
        game.name(),
        install.binder_count()
    );
    Ok(Some(t))
}

fn counts(t: &Tally) -> Vec<(&str, usize)> {
    t.classes.iter().map(|(k, v)| (k.as_str(), *v)).collect()
}

#[test]
fn sekiros_constraint_atoms_read() -> Result<()> {
    let Some(t) = tally(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!(
        counts(&t),
        [
            ("breakable", 3),
            ("breakable/ragdoll", 3),
            ("hinge", 26),
            ("limited hinge", 439),
            ("ragdoll", 2_031),
        ]
    );
    assert!(t.worst_orthonormality < 2e-6, "{}", t.worst_orthonormality);
    assert_eq!((t.left_handed, t.inverted_limits, t.off_constant), (0, 0, 0));
    // the motor pointers the census counted: of the 2,031 unwrapped ragdoll joints 1,961 have all three set and 70 none, of
    // the 439 hinges 395 have theirs set and 44 null; the 3 wrapped joints have none
    assert_eq!((t.motors, t.null_motors), (1_961 * 3 + 395, 70 * 3 + 44 + 3 * 3));
    Ok(())
}

#[test]
fn elden_rings_constraint_atoms_read() -> Result<()> {
    let Some(t) = tally(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!(
        counts(&t),
        [
            ("ball and socket", 681),
            ("breakable", 169),
            ("breakable/ball and socket", 1),
            ("breakable/limited hinge", 16),
            ("breakable/ragdoll", 152),
            ("fixed", 36),
            ("hinge", 1),
            ("limited hinge", 1_260),
            ("ragdoll", 5_739),
            ("wheel", 48),
        ]
    );
    assert!(t.worst_orthonormality < 2e-6, "{}", t.worst_orthonormality);
    assert_eq!((t.left_handed, t.inverted_limits, t.off_constant), (0, 0, 0));
    // the motor pointers the census counted: 5,715 unwrapped ragdoll joints have all three set and 24 none, the 1,260
    // unwrapped hinges have theirs set; the 152 wrapped ragdoll joints and 16 wrapped hinges have none
    assert_eq!(
        (t.motors, t.null_motors),
        (5_715 * 3 + 1_260, 24 * 3 + 152 * 3 + 16)
    );
    Ok(())
}
