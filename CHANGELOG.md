# Changelog

## Unreleased

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
  and 70) have no rule.

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
