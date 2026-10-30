# Changelog

## 0.12.1 (2026-10-30)

- `msb`: wind SFX regions (type 6). `MapLayout::wind_sfx_region` reads a region's type data as `WindSfxRegion { effect_id, wind_area, unknown_float }`, with each layout's offsets (Elden Ring: effect +0, area index +4, float +8; Sekiro: effect +0, index +20, float +24); other lengths and stored words are refused by name, and a region of another type gives `None`. Its census test agrees with an independent reading of every layout of both installs: Elden Ring 72 regions in 11 maps, Sekiro 106 in 9.

## 0.12.0 (2026-10-30)

- `param`: Elden Ring's texture-switch params, typed. `PartsDrawParam` (`PartsDraw`: `tex_lv01_BorderDist`,
  `tex_lv01_PlayDist` and `lodType` at 40, 44 and 80; revision 5, 144-byte rows), `AssetGeometryParam`
  (`AssetGeometry::draw_param`, `refDrawParamId` at 8; revision 6, 320-byte rows as the 1.17.3 regulation stores them,
  which Paramdex's definition, stating revision 4, lays out) and `DrawDistScaleParam` (`LoadBalancerDrawDistScaleParam`,
  `Lv00` .. `Lv20`; revision 0, 128-byte rows). Any other type, revision or row size is refused. eldenring.exe 1.17.3's
  getter of the texture switch reads the border at 0x28. `tests/paramdef_census.rs` checks every row against the
  Smithbox-typed values: 385 `PartsDrawParam`, 24,106 asset geometry and 154 draw-distance scale rows of regulation
  11730034 are equal.

## 0.11.0 (2026-10-29)

- `gparam`: `Gparam::time`, the header's time axis (`TimeAxis`, version 5 and later): the offset flag (byte 13), the
  offset (the float at 64) and the period (the float at 80), which the reader used to skip. `TimeAxis::sampling` gives
  the offset and period Elden Ring's own reader of the file takes from them: the offset and 24 hours when the flag is
  set, no offset and the stored period when it is clear. The game samples a field's values at the hour, wrapped by the
  period, less the offset, so a file of offset 6.5 keys 6:30 at time 0. Elden Ring's 448 files all set the flag with a
  period of 24 and an offset of 6.5 (407), 7 (33) or 4.5 (8); Sekiro's 248 version 5 files store an offset of 0 (69 with
  a period of 24). `tests/gparam_census.rs` asserts those counts. Breaking: `Gparam` has a new public field.

## 0.10.2 (2026-10-29)

- `dcx::encode`: a Kraken payload is compressed one 256 KiB block at a time and the blocks are concatenated, so every
  block sets the restart bit, as every block of every shipped payload does (the library's one-call compression sets it on
  the first block only, and the later blocks of the old output continued the first one's matches). The library decodes
  the concatenation as a whole, and the sizes come out like the shipped ones (a 1.99 MB binder: 1,138,147 bytes against
  the shipped 1,167,321, where the old output was 969,044). Why: a loose `o114903.objbnd.dcx` re-encoded the old way made
  Sekiro exit right after the load-game press (2 of 2 launches) while the stock bytes of the same file load; the same
  binder re-encoded with every block restarting loaded and the whole route ran (1 launch). A 1,135,856-byte binder
  re-encoded the old way also loaded (1 launch), so the restart bit is not shown to be necessary for every file; the
  cause of the exit with the old output is not established. `tests/writers_census.rs` asserts that every Kraken block
  it re-encodes restarts.

## 0.10.1 (2026-10-29)

- `msb`: the running game does not draw a part by its draw groups against the collisions' display groups alone (the
  term `DrawInfo::draws_under` tests, whose documentation now says what was seen against it): a part's own display
  groups take part. `DrawInfo::{displays_under, shown_by}` (a draw group among the collision's draw groups, or a display
  group among its display groups: a reading that fits every picture so far, not a finding) and
  `MapLayout::zero_draw_census` (the parts under no draw group, how many carry display groups and how many of those a
  collision's display groups meet: Elden Ring 317,870, 235,081 and 9,198; Sekiro 654, 271 and 230).
- Tests: `tests/nav_census.rs` expects the triangle totals of havok-formats 0.9.0's triangulator, which keeps the
  corners straight only seen from above: Sekiro 602,658, Elden Ring 11,079,642 (0.10.0's test still had the totals
  from before that change).
- Tests: `tests/writers_census.rs` walks the archives on `[run] threads` threads, largest entries first (Elden Ring's
  walk on four threads: 43 minutes to 16), with the same counts on any thread count; `tests/common` opens the archives'
  indexes on the same threads.

## 0.10.0 (2026-10-29)

- `weather_assets`, Elden Ring's weather asset params, each read strictly to the install's layout (type, layout
  revision and row size; padding zero):
  - `WeatherAssetParam` (`WeatherAssetReplaceParam`). Per row: the sky map (`mapId`, `map_name()` `m60_00_00_99`), the
    weather (`TransitionSrcWeather`), `isFireAsh` as stored (0 and 1 in the base game's rows, 2 in the later addition's
    maps' rows), the asset ids of the used `AssetId` slots and the set `CreateAssetLimitId`s, with `applies_at(limit)`.
    Version 2, 64-byte rows, every asset slot an asset or -1.
  - `WeatherAssetCreateParam` (`WeatherAssetCreateParam`). Per row: the asset created, its slot, the condition as
    stored, the transition weathers, the elapsed-time weather and seconds, delay, probability, life and fade times,
    the in-game hours, the creation points and limits it sets. Version 1, 64-byte rows.
  - `WeatherAssetLimits` (`MapDefaultInfoParam`'s `WeatherCreateAssetLimitId` of each map). Version 6, 64-byte rows.
  - `asset_model_name(id)`: `AEG096_010` for 96010.
- `game_params::read_weather_assets`, `read_weather_asset_creates` and `read_weather_asset_limits` read them from an
  install. `tests/weather_assets_census.rs` covers every row of the install: every field matches the paramdef reading,
  and every asset a replace row names is placed by its sky map's layout (the 18 sky maps the base game holds).
- `shader`: Elden Ring's shader stems and sampler names split into their parts. `Profile::parse` reads
  `C[DetailBlend][S2]_cloth`-style stems (category, family brackets, tags, suffixes; the family, the multi-blend
  count of `[MbN]`, the overlay of `[Ov_*]`), `sampler_map` folds a sampler's map spelling (`AlbedoMap`, `AlbedoMap_0`,
  `GSBlendMap_AlbedoMap_0`) into a `MapType`. Held to all 20,619 material binaries of the install (975 stems, 692 of them
  in the grammar; 152,138 samplers) and soulstruct's per-shader UV-slot table to the UV attributes of 76,319 meshes and
  to the shaders' own pixel programs (`tools/gen_shader_uv_programs.py`: the vertex-input texture coordinates 3,698
  sampled textures of 688 programs read; 172 of the 177 tuple lengths equal the programs' declared UV slots, 811 of the
  980 compared textures read the UV the table names, the 169 that do not are all of 24 character, hair, eye, grass and test shaders).
- `flver::gx`: the community's names for the words of the GX items the installs store (`GxItem::fields`,
  `Material::{emissive_intensity, translucency_scale, albedo_colour, parallax, water}`).
- Breaking: `btl::Light` gains `flicker_interval`, `flicker_brightness`, `event_id`, `near_clip`,
  `shadow_model_cull_flip`, `sharpness`, `horizontal_angle`, `near_distance`, `enable_distance`, `volume_density` and
  `exposure_weight`; the `auxiliary` map no longer holds those words. `cone_angle` is documented as a spot's full vertical
  angle in degrees, `horizontal_angle` (offset 184) as the horizontal one and `near_distance` (offset 188) as the near plane,
  from a running Sekiro.
- `tae::template`: the community's event templates (DSAnimStudio's XML, supplied by the caller) read and applied:
  `Templates::parse`, `EventTemplate::decode`. Held to a second source, soulstruct's event classes (106 of Sekiro's and 96 of
  Elden Ring's templated types are also listed there; the two agree on every field boundary but one, and on whether a
  4-byte word is a float but for five Sekiro and four Elden Ring words: `tests/tae_template_census.rs`).
- `tpf::PixelFamily`: the layout a stored format code stands for (`Texture::pixel_family`). Codes 9 and 105 are the same
  four bytes in the opposite channel order.
- `collision::hit_class_name`: the community's names of the HitMtrlParam classes.
- `blf::LoadList` reads the `.blf` load lists of divided binders; `Binder::division_markers` is the rule that names a
  character's division binders.
- `grass::Grass` reads `.grass` meshes.
- Tests: Smithbox's file dictionaries as a census input (`tests/dictionary_census.rs`), the shared walker
  `each_leaf` opens split binders.
- Writers for DCX, BND4/BHF4, TPF, FMG, PARAM and `regulation.bin`, each rebuilt from its parsed fields over every stock
  file of both installs (and the 29 `regulation.bin` files we hold): `dcx::frame`, `dcx::encode` (with `dcx::Encoder`),
  `Binder::write`, `write_stored`, `write_split`, `write_split_stored`, `Package::write`, `Fmg::write`, `Param::write`
  (with `param::Stored`), `Param::write_derived`, and the `regulation` module (`decrypt`, `encrypt`, `iv`, `pack`). The
  structure is byte-equal wherever the bytes are the format's own: DCX headers and payloads framed again, binders written
  from their members as stored, TPFs, PARAMs, FMGs (all but 3 of Elden Ring's 3,968, which hold unreferenced zero bytes
  between their strings), and regulations encrypted again. What a compressor decides is not: a Kraken, zlib or zstd
  payload compressed again decodes to the same bytes under the same header but is not the shipped payload (the real zlib
  at level 9 reproduces 48 of 58 shipped zlib payloads, this crate's pure-Rust deflate none). `dcx::encode` writes a
  Kraken payload at the level the header states (6 or 9; `Oodle::compress_at`), which needs a `primase-foundations`
  with `oodle::Level`; a level outside -4 to 9 is refused.
- Breaking: `BinderFile` has `wrap: Option<dcx::Format>` (how a member that held a DCX envelope was framed), `Binder` has
  `layout: binder::Layout` (byte orders, name encoding, the format byte as stored, the hash-table flag and two unknown
  header bytes), `dcx::Envelope` has `format: dcx::Format` (the header's variant fields), `tpf::Package` has
  `names_padding`, and `fmg::Fmg` has `blank` (the ids a group lists with no string), `narrow` and `extra_padding`.
  `Param::read_stored` returns the two header facts the rows do not say (`param::Stored`).
- `Binder::read_stored` and `Binder::read_split_stored`: a read that keeps each member's bytes as stored (nested
  envelopes and compressed members untouched), which needs no decoder.
- Depends on `flate2` and `zstd` (libzstd) for the compressors.
- Tests: `tests/writers_census.rs` runs the writers over both installs and the regulations; `dcx` unit tests refuse each
  size word one more and one less and a payload one byte short and one byte over.
- `emedf`: the instruction schema of the event scripts of both games (Sekiro 405 instructions, Elden Ring 361): each
  instruction's name and its arguments' names, stored types, enum names and a role (event flag, entity id, a part of a map
  id, the event an initialiser starts), generated by `tools/gen_emedf.py` from the community's EMEDF files (names, types and
  enum names only). `decode` / `decode_event` read an instruction against the schema, with the event's parameter
  substitutions marked per argument, and refuse by name (not in the schema, argument bytes differ from the layout, a
  repeating argument that is not whole words); `Typed::map`, `Typed::initialisation` and `argument_uses` read the map an
  instruction names, the event it starts and what each of an event's own arguments is used as. Over all 42,885 Sekiro and
  107,639 Elden Ring stock instructions every one is typed except 4 calls of `11:3`, which the community schema lacks; each
  of the 6,920 + 34,483 parameter substitutions lands on exactly one argument of its typed instruction.
- `mapid`: `MapId::from_decimal` / `to_decimal` for the decimal map number Elden Ring's cutscene-and-warp instructions
  take (`11050000` is `m11_05_00_00`; 64 of the 65 literals in the stock scripts name an installed map), and
  `MapId::covers` for wildcard matching.
- `eventflag`: how the games address a flag (`address`: Sekiro's decimal fields and the word and bit in its zone, Elden
  Ring's category of 1000 and the byte and bit in its block) and which map a flag number names by its digits (`scope`).
- `msb`: `group_numbers`, `DrawInfo::{draw_group_numbers, display_group_numbers, draws_under}` and
  `MapLayout::group_census` (a layout's map pieces, objects and assets against its collisions' display groups).
- `area::DrawParamId` (a draw-param file's slot and sub id), `PartGparam::{draw_param_slot, draw_param_id}` (the file a
  collision's scene config names: it exists for all 1,304 Sekiro and 3,209 of the 3,266 Elden Ring collisions) and `gparam_events` (the calls of the event instructions that switch an area's draw-param file).
- `gparam_params`: Elden Ring's params that decide the draw-param file: `GparamRefSettings` (a map uses another map's files:
  70 rows keyed by the decimal map number), `WeatherParam` (a weather's file id: all 82 distinct ids name a
  `m00_00_<id>_weatheroutdoor` file) and `WeatherLotParam` (the weighted weathers of a lot).
- `area::draw_params`: Elden Ring's caves and tunnels (`m31`, `m32`) use the catacombs' `m30_00` files, as the game's
  `GparamRefSettings` rows say (they used to have none); every row of that table agrees with the rule.
- Writers for MTD, MATBIN, EMEVD, TAE, MSB and FLVER, each proven by writing back every stock file of both installs
  (`tests/*_write.rs`; the expected counts come from an independent Python parser and writer run over the same files):
  `mtd::MaterialDefinition::write` (keeping the declared block lengths and the leftover padding a read recorded) and
  `write_canonical`, `matbin::Material::write`, `emevd::Script::write`, `tae::Tae::write`, `msb::MapLayout::write` and
  `flver::Model::write` with its `flver::Layout` (`Layout::read`, `Layout::for_model`).
- Breaking: `mtd::Texture` has `version` (the texture block's version, 5 or 3: Sekiro's definitions hold version-5 blocks
  with an empty path and no floats, which a version-3 block also reads as). `matbin::Material::unreferenced_tail` holds
  every byte after the last referenced item, so the zero byte that realigns an item ending at an odd offset (722 of
  Elden Ring's 20,619 materials) is its first byte rather than skipped.
- `emevd::Script::write` stores -1 for an event's empty parameter list; 6 events of 4 of Sekiro's 30 files store the
  running offset there instead (all other files of both installs are byte-equal).
- `msb::MapLayout::write` lays the parts' fixed fields (placement, entity ID and groups, shadow and draw controls, hit
  filter, NPC row, effect id, tile-load map) over the part records, so a layout edited through `parts` is written as
  edited; a name, kind or model a view disagrees with the record on is refused.
- `flver::Layout` holds what a file stores that `Model` does not (header bounding box and face counts, the meshes'
  bounding boxes, the version-`0x2001a` skeleton set, each face set's stated index width, the GX area with the lists
  materials share and the ones no material names, the SpeedTree models' data order). SpeedTree models differ from the rest
  in an extra zero unit after each texture path, data at a multiple of 0x800 with the vertex buffers first, and a file
  padded to one; the writer follows them.
- `tae::Tae::check_templates` and `write_checked`: hold a file to the event templates (`tae::template`), refusing an
  event whose asserted field stores another value with the animation, event and field named (`tae::TemplateCheck`).
- `flver::Dummy::flag1` documented from a census of every dummy of both installs (20,268 in Sekiro's 14,459 models,
  88,340 in Elden Ring's 22,169): in a model of more than one bone it is set exactly when the attach bone is not -1
  except for 28 and 102 dummies that are set with none; in a model of one bone it does not follow the attach bone
  (Sekiro 34 set and 8 clear of 42 with attach bone 0, Elden Ring 29 and 2,157 of 2,186). `tests/flver_census.rs` counts
  these against an independent parser's census (the dummy fields as a separate Python reader of the FLVER header region
  gives them).

- Breaking: the variable-length lists of `flver::Vertex` (`uvs`, `layer_bytes`, `tangents`, `tangent_indices`, `colors`)
  are `flver::Channel`s, not `Vec`s: a list keeps its first few values in the vertex (`uvs` two, `tangents`, `colors` and
  `layer_bytes` one, `tangent_indices` four) and moves to the heap beyond that. A `Channel` dereferences to a slice
  and has `push`, `pop`, `truncate`, `clear`, `extend` and `iter_mut`, compares equal to the same values in a `Vec`, an
  array or a slice, prints and serializes as the slice does, and is built from a `Vec` or an array with
  `Channel::from(..)` (where a field was assigned `vec![..]`). A vertex used to own up to five heap blocks; reading
  Sekiro's models (16.5 million vertices in a tenth of the archives) now allocates one per vertex only for the lists
  longer than the room above, and `Model::resident_bytes` counts those. `Vertex::speedtree` stays a `Vec`.
- The unwrap of an archive index squares with a dedicated Montgomery square (each cross product once) instead of
  multiplying a number by itself: the same bytes (held to `BigUint::modpow` and to the product), about a fifth less time
  in `unwrap_rsa`.
- `Binder::read_where(data, decoder, keep)`: `Binder::read` keeping only the members whose name `keep` accepts. The others
  are validated as a full read validates them and come back with no bytes, neither copied nor decoded: a binder of
  hundreds of shader bundles holds the few a map names, and reading it whole decoded all of them (Elden Ring's area read
  spent 15 % of its CPU there).
- `examples/read_bench.rs`: a timed read of an install's archives by stage (index unwrap, BDT read and decryption, DCX
  decode, binder split, FLVER parse) with a digest of everything decoded and a census of the vertices' list lengths, for
  comparing changes to the read path.
- `fxr`: every action of every installed effect file typed. `fxr::actions` has one struct per action type that occurs in
  Sekiro's or Elden Ring's files (72 types: the emitters and their shapes and spreads, particle and node movement, the
  node and particle attributes and modifier, billboards, multi-texture billboards, point sprites, lines, model particles,
  tracers, distortions, radial blurs, GPU particles, lens flares, forces, water interaction, lights and the actions the fxr
  project leaves unnamed), generated by `tools/gen_fxr_actions.py` from the project's action data (EvenTorset/fxr,
  Unlicense). A struct holds every parameter the project names in Sekiro's, Elden Ring's or Dark Souls III's layout, each as
  a `Property`, a boolean, an integer, a float or a vector, with the project's default for a parameter the stored lists are
  too short to hold; `Extra` keeps what a struct does not name (stored counts, words and properties past the named ones,
  field lists, the flag bytes). `Typed::read(action, game)` reads any of them, `params()` names every parameter and
  `resources()` the files it refers to; `Fxr::typed(game)` is the whole node tree typed (`TypedNode`, `TypedConfig`).
  Which of a type's layouts an action is stored in is decided by its property lists' lengths (three Sekiro billboards keep
  Dark Souls III's layout, with one word past the project's names; 26 Elden Ring action-10500 records keep Sekiro's).
  Read over all 2,913 + 12,155 files: no action refused; the actions per type equal an independent walker's; the typed
  parameters equal the fxr library's `toJSON` over a covering sample (every action type and list-length layout, and
  every tenth file: 334 Sekiro and 1,246 Elden Ring files, 21,656 and 91,888 actions), once its display conventions are
  applied (the editor negates the x of a node's offset and rotation, shows turn intervals in 1/50 s and effect times in
  1/30 s, and inverts the follow-rotation flags); every enum value stored is one the project names.
- `fxr::enums`: the project's value sets (blend mode, attach mode, orientation modes, emitter directions, node and config
  kinds, distortion, force volume and reflection modes, ...) as enums with `value`/`from_value`, generated by
  `tools/gen_fxr_enums.py`.
- `fxr::ModifierKind` (`Modifier::kind`): the property modifiers that occur (random delta, random range, random fraction,
  external value 1 and 2), decoded from the type word by field count, 359,000 over both installs. `Property::value_at`
  evaluates a stored function (zero, one, constant, stepped, linear, Bezier, Hermite) as the project does; its Hermite is the
  project's approximation (the files store tangent angles only). Compared with the library's `valueAt` at probe points
  over 9,865 keyframed properties.
- `fxr::resources`: the file names of an effect's textures (`s00001_a.tpf` in Elden Ring; Sekiro's carry no suffix),
  models and animations, an index of a binder's members by number (`Resources::index`) and Elden Ring's resource lists
  (`read_resource_list`, the text of `.ffxreslist` members).
- Depends on havok-formats 0.8.1.

## 0.9.0 (2026-10-28)

- Breaking: depends on havok-formats 0.8.0 (a breaking release: it reads the whole `hclClothData`), whose collision
  types appear in this crate's API.

## 0.8.0 (2026-10-27)

- Breaking: `msb::Region::index_in_type` (+12) and `Region::number` (+44), `msb::Event::index_in_type` (+16). The region index
  is the per-type counter in all 45,121 regions; the region number is unique within its map in all 945 maps (0 to n-1 in
  419) and is not the record index; the event index is the per-type counter in 9,209 of 9,211 events (the two others have
  type `0xffffffff`). `Event::event_id` (+8) is unique within its map in all 945 and equals the record index in 367 of 9,211.
- `MapLayout::draw_infos`, with `DrawInfo`, `DisplayInfo2` and `PartDrawInfo`: the block of a part's first payload slot
  (+80: eight display-group words, eight draw-group words, 32 collision-mask words and two four-byte fields; 388 bytes in
  Sekiro, 392 in Elden Ring) and of its second (+88, 72 bytes), as stored. A block of another length, or with data in its
  zero bytes, is refused. Read over all 55,352 Sekiro and 457,108 Elden Ring parts; the counts of non-zero arrays per kind
  equal an independent census's (`tests/map_census.rs`). What the groups do is not read.
- Breaking: `flver::FaceSet::unknown_06` (the word at +6, 1 on 85,996 of Sekiro's face sets and 149,048 of Elden Ring's) and
  the bone fields `flver::Bone::{child, next_sibling, previous_sibling, bounds_min, bounds_max, flags}` (+30, +44, +46,
  +48, +64, +60). `Model::read` now refuses a bone chain that does not visit exactly the bones naming the bone as parent
  (through `child` then `next_sibling`, `previous_sibling` inverting it): none of the installs' bones is refused.
- Breaking: `tae::Animation::event_groups` (`tae::EventGroup`: the group's type number and the indices of its events), and
  `Tae::read` now checks the header's constant words, the animation numbers (strictly rising), the animation groups table
  (one entry per maximal run of consecutive numbers), each animation's times table (its events' distinct start and end
  times, ascending) and the event groups (32 bytes, members that are the animation's event headers). None of the 326 Sekiro
  and 1,071 Elden Ring event files of the installs is refused.

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
- Breaking: depends on `havok-formats` 0.7.0 (the SHRED packfile and tagfile checks, the tagfile writer), whose
  physics views gain fields, and on `dx` 0.3.0.

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
