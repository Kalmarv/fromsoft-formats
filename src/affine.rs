//! Affine arithmetic on the places files store: column matrices in f64 ([`Matrix`]) and in f32 as a pose's columns
//! ([`pose_columns`]: a Havok body's position and unit quaternion).
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation
    )
)]

/// An affine map in f64: the images of the x, y and z axes, then the translation.
pub type Matrix = [[f64; 3]; 4];
/// The direction `p` through `m` (no translation).
pub fn affine_vector(m: &Matrix, p: [f64; 3]) -> [f64; 3] {
    let [[a0, a1, a2], [b0, b1, b2], [c0, c1, c2], _] = *m;
    let [x, y, z] = p;
    [
        a0 * x + b0 * y + c0 * z,
        a1 * x + b1 * y + c1 * z,
        a2 * x + b2 * y + c2 * z,
    ]
}
/// The point `p` through `m`.
pub fn affine_point(m: &Matrix, p: [f64; 3]) -> [f64; 3] {
    let [vx, vy, vz] = affine_vector(m, p);
    let [_, _, _, [tx, ty, tz]] = *m;
    [vx + tx, vy + ty, vz + tz]
}
/// `local`, then `parent`: the map of a child placed in its parent's space.
pub fn compose(parent: &Matrix, local: &Matrix) -> Matrix {
    let [x, y, z, t] = *local;
    [
        affine_vector(parent, x),
        affine_vector(parent, y),
        affine_vector(parent, z),
        affine_point(parent, t),
    ]
}

/// A body pose as the columns of its affine map (rotation by the unit quaternion `q`, then translation by `p`).
pub fn pose_columns(p: [f32; 3], q: [f32; 4]) -> [[f32; 3]; 4] {
    let [x, y, z, w] = q;
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let xw = x * w;
    let yw = y * w;
    let zw = z * w;
    [
        [1. - 2. * (yy + zz), 2. * (xy + zw), 2. * (xz - yw)],
        [2. * (xy - zw), 1. - 2. * (zz + xx), 2. * (yz + xw)],
        [2. * (xz + yw), 2. * (yz - xw), 1. - 2. * (yy + xx)],
        p,
    ]
}

/// `v` through the affine map with columns `c`.
pub fn posed(c: &[[f32; 3]; 4], v: [f32; 3]) -> [f32; 3] {
    let [[a0, a1, a2], [b0, b1, b2], [c0, c1, c2], [t0, t1, t2]] = *c;
    let [x, y, z] = v;
    [
        x * a0 + y * b0 + z * c0 + t0,
        x * a1 + y * b1 + z * c1 + t1,
        x * a2 + y * b2 + z * c2 + t2,
    ]
}

/// `v` through the pose of position `p` (its `xyz`) and orientation `q`.
pub fn pose(v: [f32; 3], p: [f32; 4], q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, _] = p;
    posed(&pose_columns([x, y, z], q), v)
}
