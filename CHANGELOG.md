# Changelog

## 0.2.0 (2026-10-27)

- Breaking: depends on havok-formats 0.2.0, whose packfile members are renamed (`RigidBody`, `MeshMaterial`,
  `Packfile`, `AnimationType::ReferencePose`) and which checks its collision-mesh census on every file.
- Breaking: `Tae` has a new public field, `group`.

- TAE: an animation binder's event files are numbered by their binder slot (`Tae::read_member`, the `group` field): the
  player's binder holds one file per animation group, storing numbers within the group. `Tae::number` gives an
  animation's number (group times a million plus the stored one), `Tae::animation` looks animations up by it, and
  `tae::resolve` follows an import to the animation it names in any event file of the binder.

## 0.1.0 (2026-10-27)

- First release: Sekiro and Elden Ring BHD5 archives, DCX, BND4 (whole and split), FLVER, MSB, TPF, MTD and MATBIN, params and more. The archive keys are the caller's to supply.
