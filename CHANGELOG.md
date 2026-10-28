# Changelog

## Unreleased

- The references from map data to effects: `msb::Part::sfx_param_relative_id` (an Elden Ring asset's type data +0x1c, a
  signed 16-bit word; the only word of the type data that is -1 or the last digits of an existing row key: 409,246 of the
  421,253 parts of `AEGnnn_mmm` models store -1, 11,533 name a row of `AssetModelSfxParam`, 474 name an absent one),
  `MapLayout::sfx_region` (an SFX region's effect id and start-disabled flag: 1,872 Sekiro regions in 9 maps, 54 start
  disabled; 1,523 Elden Ring regions in 104 maps, 17 disabled) and `param::ModelSfxParam` with
  `game_params::read_model_sfx_param` (Sekiro's `ModelSfxParam`, 202 rows, and Elden Ring's `AssetModelSfxParam`, 1,293
  rows: eight slots of an effect id and the id of the model's dummy polygon it attaches to; compared field by field with the
  Smithbox paramdefs' reading, `tests/model_sfx_census.rs`). `ModelSfxParam::rows_with` is plural because Sekiro's
  param stores row 1230000 twice, with different slots.
- `fxr`: `StaticTransform::matrix` (rotations about Z, then X, then Y, then the translation: the fxr project's order,
  not checked against the game), `PlacedLight::local_matrix`, and `Fxr::spawns` (every `SFXReference` action with
  its place and the same `active_in_state` / `local_matrix` as a light: 543 Sekiro and 5,228 Elden Ring actions, all
  among a node's own actions, counted by an independent walker).

## 0.7.0 (2026-10-27)

- `fxr`: FXR effect files, version 5 and header minor 0, the layout of all 15,068 `.fxr` files of both installs (2,913 in
  Sekiro's 8 `sfxbnd` binders, 12,155 in Elden Ring's 220; none refused). `Fxr::read` keeps the state machine, the tree of
  nodes with their configs and actions, the actions' field words, properties (zero, one, constant, stepped, linear,
  Bezier and Hermite functions, with `Property::constant`, `keyframes` and `settled`), modifiers and field lists as stored,
  and requires the layout every file has: the sections tile the file and the records of the tiled sections are named by
  their parents exactly once. `Fxr::lights` types the 834 + 4,999 point lights (action 609, three layouts by the stored
  counts: Dark Souls III's in five Sekiro files, Sekiro's and Elden Ring's) and 13 + 17 spot lights (action 11000)
  with their tree position and the config their node chooses in state 0. Light shafts (action 10003) have no instance
  in either install and are read as plain actions. Version 4 (Dark Souls III) and minor 1 (Nightreign) are refused by
  their header. Every file's action counts and every light's parameters agree with the fxr library
  (EvenTorset/fxr 2b6b52f, Unlicense, from which the parameter names and types are taken), and the totals agree with
  an independent walker (`tests/fxr_census.rs`). `Node::config_in_state`, `Config::static_transform` (action 35, six floats: offset and
  rotation in degrees; 20,878 configs of Sekiro and 81,215 of Elden Ring have one) and `PlacedLight::static_transforms` give the
  transforms a light sits under; every node with several configs has a state-config map in both installs.
- `collision::Soup::bounds` and `Soup::simd`: the bounding-volume trees and simd trees of the compressed meshes
  `read_map` and `read_bodies` read, decoded and checked by `havok-formats` (a mesh whose trees do not hold together is
  refused), with the triangles and quads counted against the boxes of their leaves. `tests/bvtree_census.rs` reads
  every map `h`/`l` binder and every Sekiro object and Elden Ring asset collision binder and asserts the tree, leaf and
  box counts of both games against the census. Depends on the `havok-formats` commit that adds `mesh_trees`.
- Breaking: depends on `havok-formats` 0.6.0 (the bounding-volume and simd trees of compressed meshes), whose
  `CompressedMesh` and `MeshSection` gain fields; `Soup` gains `bounds` and `simd`, so a struct literal of it needs the
  fields.

## 0.6.0 (2026-10-27)

- `collision::read_map` reads extern mesh bodies (`hknpExternMeshShape`, every body of the map collision `f`
  binders) as posed triangles with no HitMtrl ID (-1), counted in `Soup::extern_triangles`, instead of listing them in
  `unread_bodies`; a triangle that stores a material other than -1 is refused. Every `f` binder of both installs
  reads: 1,145 Sekiro files (12,688 bodies, 537,436 triangles) and 1,850 Elden Ring files (264,858 bodies,
  1,707,650 triangles), `tests/extern_mesh_census.rs`. Depends on `havok-formats` at the commit that adds
  `extern_mesh`.
- `paramdef`: typed reading of any param by its paramdef. `ParamDef::from_xml` reads Paramdex's XML (the crate holds no
  definition: Paramdex states no licence, so the caller supplies the text), `ParamDef::layout(regulation)` lays the fields
  out with the SoulsFormats container rule and the regulation's `FirstVersion`/`RemovedVersion`, and
  `ParamDefs::bind(&Param, regulation)` picks the definition of the param's type whose layout size is the row size (a
  tie goes to the file's data version), then reads every row strictly (text decodes, no undeclared bit is set) into a
  `TypedParam` of `TypedRow`s with `get(name)` and `fields()`. All 194 params of the Elden Ring 1.17.3 install, the 135 params of
  Sekiro's 64-bit form and all 5,295 params of Smithbox's 28 other Elden Ring regulations (4,352,472 rows in all) bind
  with Smithbox's definitions, and a second decoder (Python) agrees on every value of every param.
- `Binder::regulation_number` (the eight ASCII digits of a regulation's BND4 version, `11730034`; `None` for Sekiro's
  `07D7R6`) and `game_params::regulation_binder` (the binder of any `regulation.bin`, with the key).
- Overworld: `overworld` reads Elden Ring's `WorldMapLegacyConvParam` (`ConvParam::from_param`, `Game::legacy_conv`,
  strict: padding, map numbers and coordinates are checked, 196 rows of the installed regulation) and turns its rows into
  a transform per legacy map (`Placements::from_conv`, `game_params::read_legacy_placements`): the map's origin in the
  frame of an overworld tile (`Placement`, re-expressed in any tile of the area with `origin_in_frame_of`, or in the
  area's frame with `global_origin`). `tile_origin` gives every tile of every level its origin in that frame. The rules are
  Smithbox's (first base row to a tile, chained rows between legacy maps, two restricted destinations, the ending
  cutscenes' map, the sky map, three colosseums) with one refinement: maps whose two reference points in a row between
  them are the same numbers share one origin (`Source::SharedOrigin`), which moves two maps from where the base-row rule
  puts them (`m12_07_00_00` 11.2 m, `m34_14_00_00` 2.0 m; the first is exact against the connecting collision of
  `m12_02_00_00`). 100 maps are placed, 79 of them in the base game's install; 15 maps the param names (areas 37, 38, 39
  and 70) have no rule. `MapId::from_name` reads a map name (`m14_00_00_00`) as the identity `MapId::name` writes.
- Breaking: depends on `havok-formats` 0.5.0 (float tracks read in spline animations, rotation control points that are
  not finite kept as stored data, `extern_mesh`), whose `SplineAnimation` and `Unreadable` gain fields. `Soup` gains
  `extern_triangles`, so a struct literal of it needs the field.

## 0.5.0 (2026-10-27)

- DCX: a Kraken envelope's Oodle payload is walked block by block before it is decoded
  (`primase_foundations::compress::oodle_framing::walk`), and an envelope whose blocks do not consume exactly the
  payload is refused with `Error::Invalid` naming the offset in the envelope and the block: bytes left after the last
  quantum, a header or body that runs past the payload, an unknown decoder type, the checksum bit. The Oodle library
  itself decodes a payload with bytes added after its last block. All 144,194 Kraken payloads of Sekiro and Elden Ring
  pass; DFLT and ZSTD envelopes are unchanged.
- Depends on `primase-foundations` 0.2.1.
- MTD: non-zero alignment padding after a marker is read past and recorded in `MaterialDefinition::padding`
  (`PaddingAnomaly { offset, bytes }`) instead of refusing the file; every other check is unchanged. It is empty on
  every MTD of Sekiro and Elden Ring but `c9757_crystal.mtd` (19 runs). Breaking: a struct literal of
  `MaterialDefinition` needs the field.

## 0.4.0 (2026-10-27)

- TPF: the texture-flags byte may be `0x80`, as on every texture of Elden Ring's environment-probe packages (221,796
  textures); `Texture::flags` keeps it.
- BTL: version 15 (240-byte light records, two Elden Ring banks) reads; the record size is checked in the header.
- BHD5: an entry outside its hash bucket (`hash % bucket_count`) is refused.
- FLVER: `Material` gains `string_bytes` (+16, checked against the material's strings outside version 0x20014) and
  `index` (+24). Breaking: a struct literal of `Material` needs them, and `btl::VERSIONS` has three entries.
- Depends on `havok-formats` 0.4.0 (Sekiro's one SDK 2015.1 clip now reads; `tests/havok_animation.rs` names why it
  still has no skeleton) and `dx` 0.2.0.

## 0.3.0 (2026-10-27)


- Breaking: depends on havok-formats 0.3.0, whose compressed meshes apply each section's page to shared vertices (the
  Sekiro and Elden Ring `l` collision sets had wrong vertices before) and whose cloth particle indices are `u16`.
- Collision: `read_map` and `read_bodies` refuse a shape-keyed mesh whose tree stores a `bitsPerKey` other than the
  shape's `numShapeKeyBits` (the two agree in every compressed mesh of the Sekiro and Elden Ring collision).
- `Body::motion_type` is documented as 0 static, 1 keyframed, 2 dynamic (only 2 stores a positive mass), where it
  said that 2 is keyframed.
- Breaking: `Rules::read` takes the game. Elden Ring's `2000:6` names a slot before the function and none of its 10,480
  calls in the base game's scripts names Sekiro's part-state functions, so none is decided and they are counted by the
  function they name.
- Breaking: `GrassType::lod_range` is `i16` (five Sekiro rows hold -1), and `GrassParam` reads Sekiro's
  `GRASS_TYPE_PARAM_ST` (version 6, 244-byte rows, no second model name: `model1` is empty) beside Elden Ring's.
- Breaking: `msb::Model`, `msb::Part`, `msb::Region`, `msb::Event` and `msb::PartGparam` gain fields (`instances`;
  `draw` and `tile_load`; `extra_map`; `gparam_override` and `scene_sub_id`), `msb::Generation` gains the record header
  sizes and extra-block offsets, and `equipment::FACE_OR_HEAD_BY_PRESENCE` is gone (the protector's category now says
  whether a head slot is the face).
- Params: `Param::read` reads the files that end with up to eight zero bytes after their last part (161 Elden Ring and
  5 Sekiro params of the installs were refused for it): every param of Elden Ring's `regulation.bin` (194 of 194) and
  135 of Sekiro's 138 read, the three refused being the 32-bit-form banks, by their format flags. No gap is allowed
  between the parts.
- Equipment: `equipment::OutfitLayout::of(game)` and `resolve_outfit` read both games' `CHARACTER_INIT_PARAM` and
  `EQUIP_PARAM_PROTECTOR_ST` (Elden Ring: version 2 / 320 bytes and version 6 / 416, the protector's model, category and
  gender at +164, +212 and +213). The part prefix comes from the protector's category, a protector in a slot of another
  category is refused, and a slot naming a protector row that does not exist leaves only that slot unresolved.
  `npc_outfits` and `npc_outfit_catalog` read Elden Ring. On the install 789 of the 793 protector rows with a model and
  a category of head, body, arms or legs find their part binder.
- Maps: `elden::parent_tiles` (through `Game::parent_maps`) gives the alternate overworld tiles `_10` their alternate
  parents `_11` and `_12`, which place 3,578 of their far assets. Regions and events have each generation's header size
  (96 and 40 in Sekiro, 104 and 48 in Elden Ring); Elden Ring's end with a 32-byte extra block, which ends the type
  data and names a map (`Region::extra_map`, `Event::extra_map`). The new `mapid::MapId` reads the four-byte map
  identifiers in both orders they occur in. `MapLayout::read` checks every model's instance count against the parts
  that name it. The entity block's draw controls (`Part::draw`), a part's tile-load map (`Part::tile_load`) and a
  collision's draw-param selectors (`PartGparam::gparam_override`, `scene_sub_id`) are read.
- FMG: `Fmg::read` refuses repeated or overlapping ids, a group whose strings do not follow the previous group's, a
  string count the groups do not fill and, in version 2, a wrong size field or string-offset table.
- Draw params: `area::draw_params` gives an Elden Ring map with no draw-param file of its own two-block stem the files of
  its family's `mAA_00` (20 maps of m30 and one of m11 of the 106 outside the open world); the 27 of m31 and m32, which
  have none under either stem, get none.
- `Binder::member` finds a member by its file name, ASCII case folded.
- `game_params::param_binder` is public.

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
