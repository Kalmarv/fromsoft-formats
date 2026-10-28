//! Elden Ring's legacy maps (the dungeons, catacombs, caves, the sky map and the colosseums) placed in the overworld's
//! frame: the regulation's `WorldMapLegacyConvParam` ([`ConvParam`]) and the rules that turn its rows into one transform
//! per map ([`Placements`]).
//!
//! Each row says that the point `src_position` of the legacy map `src` is the point `dst_position` of the map `dst`
//! (another legacy map, or an overworld tile). The rows carry no rotation, so a map's frame is its destination's
//! translated by `dst_position - src_position`. An overworld tile's frame has its origin at the tile's centre (the
//! collision of every tile spans +-128 m about it): the tile `mAA_X_Z_00` has its origin at `(256 X, 256 Z)` in the area's
//! global frame, a tile of level `L` (`_01`, `_02`, and the alternates `_11`, `_12`) at the centre of the `2^L` by `2^L`
//! small tiles it covers, so that [`tile_origin`] gives every tile a place in one frame per area (`m60`, `m61`).
//!
//! The rules (adapted from the open-source map editor Smithbox's `MapConnections_ER`, with one refinement the collision
//! of the maps themselves asks for):
//!
//! 1. A row between two legacy maps whose reference points are the same numbers says the two maps share an origin; maps
//!    joined so form one group, which has one place.
//! 2. A row with `is_base_point` whose destination is in `m60` or `m61` places its source, and the group of its source,
//!    in that tile at `dst_position - src_position`. The first such row (in file order) of a group decides. A map may
//!    have several (one per tile it is visible from): they agree to within 1.3 m where they describe the same place,
//!    and two maps have rows that do not ([`RESTRICTED`]), which are chosen by name.
//! 3. A row whose destination is another legacy map places that map from the source's place when the source is
//!    placed and the destination is not: `origin(dst) = origin(src) + src_position - dst_position`, whatever the row's
//!    `is_base_point`. The rows are swept in file order until a sweep places nothing.
//! 4. Maps the param does not place: [`Manual`].
//!
//! A legacy map's `_99` sibling (`m14_00_00_99`, its sky map) takes its base map's place.

use crate::bytes::View;
use crate::mapid::MapId;
use crate::param::Param;
use crate::{Result, invalid};
use std::collections::BTreeMap;

const FORMAT: &str = "WorldMapLegacyConvParam";
/// The param type [`ConvParam::from_param`] reads.
pub const PARAM_TYPE: &str = "WORLD_MAP_LEGACY_CONV_PARAM_ST";
/// The metres between two overworld tiles' origins at level 0.
const TILE_EDGE: f64 = 256.;

/// Where a game's `WorldMapLegacyConvParam` keeps its fields ([`crate::Game::legacy_conv`]): the layout revision and
/// row size the reader is written for. Every field offset is the SoulsFormats container rule's over Paramdex's
/// `WorldMapLegacyConvParam.xml` (Elden Ring, FormatVersion 203, data version 2): see [`ConvParam::from_param`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConvLayout {
    /// The layout revision ([`Param::data_version`]).
    pub data_version: u16,
    /// The row size in bytes.
    pub row_size: usize,
}

/// One row of `WorldMapLegacyConvParam`: the point `src_position` of `src` is the point `dst_position` of `dst`.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct ConvRow {
    /// The row's ID.
    pub id: i32,
    /// `disableParam_NT`: the row is left out of the NT (Japanese Nintendo) package. Kept as stored; the rules do not
    /// read it.
    pub disabled_in_nt: bool,
    /// The source map, area and grid numbers (`m14_00_00`), the variant 0.
    pub src: MapId,
    /// The source's reference point, in the source map's frame, metres.
    pub src_position: [f32; 3],
    /// The destination map (`m60_35_45`), the variant 0.
    pub dst: MapId,
    /// The same point in the destination map's frame, metres.
    pub dst_position: [f32; 3],
    /// `isBasePoint`: the game's note on the field says every source map has at least one base row.
    pub is_base_point: bool,
}

/// `WorldMapLegacyConvParam`'s rows, in file order.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ConvParam {
    /// The rows.
    pub rows: Vec<ConvRow>,
}

impl ConvParam {
    /// The rows of `param`, which must be [`PARAM_TYPE`] of `layout`'s data version and row size.
    ///
    /// A row is `u8` flags (bit 0 `disableParam_NT`, the rest and the next three bytes padding), `u8` area, grid X and
    /// grid Z of the source and a padding byte, three `f32` for its reference point, the same for the destination, and
    /// the `isBasePoint` bit with 7 padding bits and 11 padding bytes: 48 bytes. Padding must be zero, the area and
    /// grid numbers at most 99 (the field maxima), the coordinates finite, and a row ID unique.
    pub fn from_param(param: &Param, layout: &ConvLayout) -> Result<ConvParam> {
        if param.param_type != PARAM_TYPE
            || param.data_version != layout.data_version
            || param.row_size != layout.row_size
        {
            return Err(invalid(
                FORMAT,
                0,
                format!(
                    "{} version {} with rows of {} bytes, the param read is {PARAM_TYPE} version {} with rows of {}",
                    param.param_type,
                    param.data_version,
                    param.row_size,
                    layout.data_version,
                    layout.row_size
                ),
            ));
        }
        let mut rows = Vec::with_capacity(param.rows.len());
        let mut ids = std::collections::BTreeSet::new();
        for (index, row) in param.rows.iter().enumerate() {
            if !ids.insert(row.id) {
                return Err(invalid(FORMAT, index, format!("row id {} repeats", row.id)));
            }
            let v = View {
                data: &row.data,
                format: FORMAT,
                be: false,
                base: 0,
            };
            let map = |at: usize| -> Result<MapId> {
                let bytes = v.slice(at, 3)?;
                if bytes.iter().any(|&n| n > 99) {
                    return Err(v.err(
                        at,
                        format!("row {}: map numbers {bytes:?}, the fields' maximum is 99", row.id),
                    ));
                }
                let mut id = [0u8; 4];
                id.iter_mut().zip(bytes).for_each(|(to, &n)| *to = n);
                Ok(MapId::from_bytes(id))
            };
            let position = |at: usize| -> Result<[f32; 3]> {
                let mut out = [0f32; 3];
                for (k, c) in out.iter_mut().enumerate() {
                    let value = f32::from_bits(v.u32(v.element(at, k, 4)?)?);
                    if !value.is_finite() {
                        return Err(v.err(at, format!("row {}: a coordinate is not finite", row.id)));
                    }
                    *c = value;
                }
                Ok(out)
            };
            let flags = v.u8(0)?;
            let base = v.u8(36)?;
            if flags > 1 || base > 1 {
                return Err(v.err(
                    if flags > 1 { 0 } else { 36 },
                    format!(
                        "row {}: padding bits set in a flags byte ({flags:#04x}, {base:#04x})",
                        row.id
                    ),
                ));
            }
            // padding: bytes 1..4, 7, 23 and 37..48
            for (at, len) in [(1, 3), (7, 1), (23, 1), (37, 11)] {
                if v.slice(at, len)?.iter().any(|&b| b != 0) {
                    return Err(v.err(at, format!("row {}: padding is not zero", row.id)));
                }
            }
            rows.push(ConvRow {
                id: row.id,
                disabled_in_nt: flags == 1,
                src: map(4)?,
                src_position: position(8)?,
                dst: map(20)?,
                dst_position: position(24)?,
                is_base_point: base == 1,
            });
        }
        Ok(ConvParam { rows })
    }
}

/// The destinations two maps are restricted to. `m13_00_00_00` (Crumbling Farum Azula) has a base row to `m60_51_43_00`
/// (Bestial Sanctum) and one to `m60_54_53_00` (Forge of the Giants) whose places are 1,892 m apart; the layout of the
/// second tile holds a connection region to the map that its name gives as for the cutscene background-read settings, with
/// the origin as the reference point on both sides, so the first is where the map is. `m15_00_00_00` (Miquella's Haligtree)
/// is held to `m60_48_57_00` (Ordina), which has no effect on the installed regulation, whose base rows of the map are both to
/// that tile (627 m apart: the first decides). Neither map has a connecting collision to test the choice against: it
/// stands on the community map editors' choice and on the regions' names.
pub const RESTRICTED: [(MapId, MapId); 2] = [
    (
        MapId::from_bytes([13, 0, 0, 0]),
        MapId::from_bytes([60, 51, 43, 0]),
    ),
    (
        MapId::from_bytes([15, 0, 0, 0]),
        MapId::from_bytes([60, 48, 57, 0]),
    ),
];

/// A place the param does not state, which the rules supply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Manual {
    /// The map is only ever loaded from another map's place and shares its origin (`m11_71_00_00`, the ending cutscenes'
    /// map, from `m11_05_00_00`, Leyndell, Ashen Capital).
    SameOrigin(MapId),
    /// The map lies in a tile's frame at a fixed offset: the whole-world sky map `m60_00_00_00` (and its `_99`), whose
    /// origin is the centre of the level-2 tile `m60_10_08_02`, and the colosseums in the overworld tiles that hold them.
    Tile,
    /// The map lies at a fixed offset from another legacy map (the Royal Colosseum `m45_00_00_00` from Leyndell).
    Offset(MapId),
}

/// How a map's place was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Source {
    /// A base row, by its ID, with the overworld tile as its destination.
    BaseRow(i32),
    /// The map shares its origin with `with`, another legacy map whose base row `row` places both (rows between them
    /// give the same reference point in each).
    SharedOrigin {
        /// The base row that places the group.
        row: i32,
        /// The map of the group that row's source is.
        with: MapId,
    },
    /// A row by its ID from the map `via`, itself placed.
    Chained {
        /// The row.
        row: i32,
        /// The placed map the row's other end is.
        via: MapId,
    },
    /// A rule the param does not carry ([`Manual`]).
    Manual(Manual),
}

/// A legacy map's place: its origin is `offset` metres from the origin of the tile `tile` (in the tile's own frame, Y up),
/// and the map has no rotation relative to it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Placement {
    /// The overworld tile (`m60_35_45_00`), whose frame `offset` is in.
    pub tile: MapId,
    /// The map's origin in the tile's frame.
    pub offset: [f64; 3],
    /// How it was found.
    pub source: Source,
}

impl Placement {
    /// The map's origin in the frame of `other`, a tile of the same area (any level: `m60_35_46_00`, `m60_17_23_01`,
    /// `m60_35_46_10`); `None` for a map that is not a tile of that area.
    pub fn origin_in_frame_of(&self, other: MapId) -> Option<[f64; 3]> {
        let (area, here) = tile_origin(self.tile)?;
        let (other_area, there) = tile_origin(other)?;
        (area == other_area).then_some([
            self.offset[0] + here[0] - there[0],
            self.offset[1],
            self.offset[2] + here[1] - there[1],
        ])
    }

    /// The map's origin in the area's global frame (the frame of [`tile_origin`]), with the area (60 or 61).
    pub fn global_origin(&self) -> Option<(u8, [f64; 3])> {
        let (area, at) = tile_origin(self.tile)?;
        Some((
            area,
            [self.offset[0] + at[0], self.offset[1], self.offset[2] + at[1]],
        ))
    }
}

/// Where the frame origin of an overworld tile is in its area's global frame (metres east and north: X and Z), with the
/// area (`60` or `61`): the centre of the tile for level 0, of the `2^L` by `2^L` tiles it covers for the larger ones.
/// The variant byte is the name's two digits: the tens digit 0 (the base tile) or 1 (the alternate beside it), the units
/// digit the level 0, 1 or 2. `None` for any other map, and for the area's grid cell 0, 0 (the sky map).
pub fn tile_origin(tile: MapId) -> Option<(u8, [f64; 2])> {
    if !matches!(tile.area, 60 | 61) || tile.x == 0 || tile.z == 0 {
        return None;
    }
    let (alternate, level) = (tile.variant >> 4, tile.variant & 15);
    if alternate > 1 || level > 2 {
        return None;
    }
    // a level-L tile covers 2^L small tiles a side, the first at index * 2^L; its origin is the centre of their origins
    let span = match level {
        0 => 1.,
        1 => 2.,
        _ => 4.,
    };
    let origin = |index: u8| (f64::from(index) * span + (span - 1.) / 2.) * TILE_EDGE;
    Some((tile.area, [origin(tile.x), origin(tile.z)]))
}

/// The map every placement of `map` is taken from: `m14_00_00_00` for its sky map `m14_00_00_99`.
fn base_map(map: MapId) -> MapId {
    if map.variant == 0x99 {
        MapId { variant: 0, ..map }
    } else {
        map
    }
}

/// The representative of `map`'s group (the smallest map of it), with the path compressed.
fn find(parent: &mut BTreeMap<MapId, MapId>, map: MapId) -> MapId {
    let mut root = map;
    while let Some(&up) = parent.get(&root) {
        if up == root {
            break;
        }
        root = up;
    }
    let mut at = map;
    while let Some(&up) = parent.get(&at) {
        if up == root || up == at {
            break;
        }
        parent.insert(at, root);
        at = up;
    }
    root
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn widen(p: [f32; 3]) -> [f64; 3] {
    p.map(f64::from)
}

/// The sky map `m60_00_00_00` lies at the centre of the level-2 tile `m60_10_08_02`: the tile `m60_40_32_00` plus
/// `(384, 0, 384)`.
const SKY_MAP: (MapId, MapId, [f64; 3]) = (
    MapId::from_bytes([60, 0, 0, 0]),
    MapId::from_bytes([60, 40, 32, 0]),
    [384., 0., 384.],
);

/// The colosseums' places: the Royal Colosseum from Leyndell `m11_00_00_00`, the other two in the tiles that hold their
/// arenas (the colosseums lie in no row of the param).
const COLOSSEUMS: [(MapId, Manual, MapId, [f64; 3]); 3] = [
    (
        MapId::from_bytes([45, 0, 0, 0]),
        Manual::Offset(MapId::from_bytes([11, 0, 0, 0])),
        MapId::from_bytes([11, 0, 0, 0]),
        [-359.44, 32.74, -492.72],
    ),
    (
        MapId::from_bytes([45, 1, 0, 0]),
        Manual::Tile,
        MapId::from_bytes([60, 47, 42, 0]),
        [-2.34, 150.4, -43.36],
    ),
    (
        MapId::from_bytes([45, 2, 0, 0]),
        Manual::Tile,
        MapId::from_bytes([60, 42, 40, 0]),
        [-24.47, 208.82, -66.69],
    ),
];

/// Every legacy map's place in the overworld.
#[derive(Clone, Debug, PartialEq)]
pub struct Placements {
    /// By map (variant 0).
    pub maps: BTreeMap<MapId, Placement>,
    /// The maps the rows name that no rule places (their destinations are in an area the rules do not place).
    pub unplaced: Vec<MapId>,
}

impl Placements {
    /// The places of every legacy map `param` and the manual cases give.
    pub fn from_conv(param: &ConvParam) -> Placements {
        let mut maps = BTreeMap::new();
        // 1. maps whose two reference points are the same numbers share an origin
        let mut parent: BTreeMap<MapId, MapId> = BTreeMap::new();
        for row in param.rows.iter().filter(|r| !matches!(r.dst.area, 60 | 61)) {
            if row.src_position == row.dst_position {
                let (a, b) = (find(&mut parent, row.src), find(&mut parent, row.dst));
                if a != b {
                    parent.insert(a.max(b), a.min(b));
                }
            }
        }
        // 2. the first base row to an overworld tile of any map of a group places the group
        let mut groups: BTreeMap<MapId, (&ConvRow, Placement)> = BTreeMap::new();
        for row in &param.rows {
            if !row.is_base_point || !matches!(row.dst.area, 60 | 61) {
                continue;
            }
            if RESTRICTED
                .iter()
                .any(|&(src, dst)| src == row.src && dst != row.dst)
            {
                continue;
            }
            groups.entry(find(&mut parent, row.src)).or_insert((
                row,
                Placement {
                    tile: row.dst,
                    offset: add(widen(row.dst_position), widen(row.src_position).map(|c| -c)),
                    source: Source::BaseRow(row.id),
                },
            ));
        }
        let members: Vec<MapId> = param
            .rows
            .iter()
            .flat_map(|r| [r.src, r.dst])
            .filter(|m| !matches!(m.area, 60 | 61))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        for map in members {
            if let Some((row, placement)) = groups.get(&find(&mut parent, map)) {
                maps.insert(
                    map,
                    Placement {
                        source: if map == row.src {
                            placement.source
                        } else {
                            Source::SharedOrigin {
                                row: row.id,
                                with: row.src,
                            }
                        },
                        ..*placement
                    },
                );
            }
        }
        // 3. rows between legacy maps, swept until a sweep places nothing
        loop {
            let before = maps.len();
            for row in &param.rows {
                if matches!(row.dst.area, 60 | 61) || maps.contains_key(&row.dst) {
                    continue;
                }
                let Some(from) = maps.get(&row.src).copied() else {
                    continue;
                };
                maps.insert(
                    row.dst,
                    Placement {
                        tile: from.tile,
                        offset: add(
                            from.offset,
                            add(widen(row.src_position), widen(row.dst_position).map(|c| -c)),
                        ),
                        source: Source::Chained {
                            row: row.id,
                            via: row.src,
                        },
                    },
                );
            }
            if maps.len() == before {
                break;
            }
        }
        // 4. the cases no row states
        if let Some(leyndell_ashen) = maps.get(&MapId::from_bytes([11, 5, 0, 0])).copied() {
            maps.insert(
                MapId::from_bytes([11, 71, 0, 0]),
                Placement {
                    source: Source::Manual(Manual::SameOrigin(MapId::from_bytes([11, 5, 0, 0]))),
                    ..leyndell_ashen
                },
            );
        }
        maps.insert(
            SKY_MAP.0,
            Placement {
                tile: SKY_MAP.1,
                offset: SKY_MAP.2,
                source: Source::Manual(Manual::Tile),
            },
        );
        for (map, how, anchor, offset) in COLOSSEUMS {
            let placement = match how {
                Manual::Offset(from) => maps.get(&from).map(|p| Placement {
                    tile: p.tile,
                    offset: add(p.offset, offset),
                    source: Source::Manual(how),
                }),
                _ => Some(Placement {
                    tile: anchor,
                    offset,
                    source: Source::Manual(how),
                }),
            };
            if let Some(placement) = placement {
                maps.insert(map, placement);
            }
        }
        let mut unplaced: Vec<MapId> = param
            .rows
            .iter()
            .flat_map(|r| [r.src, r.dst])
            .filter(|m| !matches!(m.area, 60 | 61) && !maps.contains_key(m))
            .collect();
        unplaced.sort_unstable();
        unplaced.dedup();
        Placements { maps, unplaced }
    }

    /// The place of `map`: a legacy map's own, or for its `_99` sky map its base map's. `None` for a tile, and for a map
    /// the rules do not place.
    pub fn get(&self, map: MapId) -> Option<&Placement> {
        self.maps.get(&base_map(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(a: u8, x: u8, z: u8) -> MapId {
        MapId::from_bytes([a, x, z, 0])
    }

    fn row(id: i32, src: MapId, s: [f32; 3], dst: MapId, d: [f32; 3], base: bool) -> ConvRow {
        ConvRow {
            id,
            disabled_in_nt: false,
            src,
            src_position: s,
            dst,
            dst_position: d,
            is_base_point: base,
        }
    }

    /// The Academy: two base rows to two tiles that describe one place (256 m apart in Z), the first decides.
    #[test]
    fn the_first_base_row_decides_and_the_tiles_agree() {
        let academy = id(14, 0, 0);
        let conv = ConvParam {
            rows: vec![
                row(
                    133,
                    academy,
                    [0.; 3],
                    id(60, 35, 45),
                    [-112.058, 310.577, 194.278],
                    true,
                ),
                row(
                    134,
                    academy,
                    [0.; 3],
                    id(60, 35, 46),
                    [-112.058, 310.577, -61.722],
                    true,
                ),
            ],
        };
        let placements = Placements::from_conv(&conv);
        let p = placements.get(academy).unwrap();
        assert_eq!(p.tile, id(60, 35, 45));
        assert_eq!(p.source, Source::BaseRow(133));
        assert!((p.offset[0] + 112.058).abs() < 1e-3 && (p.offset[2] - 194.278).abs() < 1e-3);
        // the same place in the frame of the next tile
        let next = p.origin_in_frame_of(id(60, 35, 46)).unwrap();
        assert!((next[2] + 61.722).abs() < 1e-3, "{next:?}");
        assert!((next[1] - 310.577).abs() < 1e-3);
        // its sky map shares it
        assert_eq!(
            placements
                .get(MapId::from_bytes([14, 0, 0, 0x99]))
                .map(|p| p.tile),
            Some(id(60, 35, 45))
        );
    }

    #[test]
    fn a_row_between_legacy_maps_chains_whichever_way_the_file_orders_them() {
        // m10_01 hangs off m10_00 by a row that comes first in the file
        let conv = ConvParam {
            rows: vec![
                row(1, id(10, 0, 0), [-514., 28., 200.], id(10, 1, 0), [0.; 3], true),
                row(
                    2,
                    id(10, 0, 0),
                    [0.; 3],
                    id(60, 39, 39),
                    [387.383, 273.608, -282.331],
                    true,
                ),
            ],
        };
        let p = Placements::from_conv(&conv);
        let child = p.get(id(10, 1, 0)).unwrap();
        assert_eq!(child.tile, id(60, 39, 39));
        assert_eq!(
            child.source,
            Source::Chained {
                row: 1,
                via: id(10, 0, 0)
            }
        );
        assert!((child.offset[0] - (387.383 - 514.)).abs() < 1e-3);
        assert!((child.offset[1] - (273.608 + 28.)).abs() < 1e-3);
        assert!((child.offset[2] - (-282.331 + 200.)).abs() < 1e-3);
    }

    /// Two maps that give one reference point as both their ends share an origin, so the one without a good base row of its
    /// own takes the other's place: m12_07's own base row is 11 m from where its connecting collision meets m12_02's.
    #[test]
    fn maps_that_name_one_point_twice_share_an_origin() {
        let (siofra, nokron) = (id(12, 2, 0), id(12, 7, 0));
        let conv = ConvParam {
            rows: vec![
                row(
                    124,
                    siofra,
                    [928.988, -617.3, 1188.437],
                    nokron,
                    [928.988, -617.3, 1188.437],
                    true,
                ),
                row(
                    120,
                    siofra,
                    [1549.947, -301.49, 1910.793],
                    id(60, 48, 39),
                    [143.636, 116.941, 243.683],
                    true,
                ),
                row(
                    130,
                    nokron,
                    [599.083, -494.007, 975.002],
                    id(60, 45, 36),
                    [-40.917, -64.378, 79.001],
                    true,
                ),
            ],
        };
        let p = Placements::from_conv(&conv);
        let (a, b) = (p.get(siofra).unwrap(), p.get(nokron).unwrap());
        assert_eq!((a.tile, b.tile), (id(60, 48, 39), id(60, 48, 39)));
        assert_eq!(a.offset, b.offset);
        assert_eq!(a.source, Source::BaseRow(120));
        assert_eq!(
            b.source,
            Source::SharedOrigin {
                row: 120,
                with: siofra
            }
        );
        // the same rows with the shared point told apart place the second map by its own row
        let mut apart = conv.clone();
        apart.rows[0].dst_position[1] += 1.;
        let q = Placements::from_conv(&apart);
        assert_eq!(q.get(nokron).unwrap().tile, id(60, 45, 36));
    }

    #[test]
    fn restricted_maps_take_only_their_destination() {
        let farum = id(13, 0, 0);
        let conv = ConvParam {
            rows: vec![
                row(132, farum, [0.; 3], id(60, 54, 53), [0.; 3], true),
                row(
                    11,
                    farum,
                    [-2509.61, -874.01, -668.01],
                    id(60, 51, 43),
                    [-36.71, 344.39, 0.32],
                    true,
                ),
            ],
        };
        let p = Placements::from_conv(&conv);
        assert_eq!(p.get(farum).unwrap().tile, id(60, 51, 43));
        assert_eq!(p.get(farum).unwrap().source, Source::BaseRow(11));
    }

    #[test]
    fn manual_cases_follow_their_anchors() {
        let conv = ConvParam {
            rows: vec![
                row(1, id(11, 0, 0), [0.; 3], id(60, 45, 52), [0.; 3], true),
                row(2, id(11, 5, 0), [0.; 3], id(60, 45, 52), [10., 0., 0.], true),
            ],
        };
        let p = Placements::from_conv(&conv);
        assert_eq!(p.get(id(11, 71, 0)).unwrap().offset, [10., 0., 0.]);
        assert_eq!(p.get(id(45, 0, 0)).unwrap().tile, id(60, 45, 52));
        let sky = p.get(MapId::from_bytes([60, 0, 0, 0x99])).unwrap();
        assert_eq!(sky.global_origin(), Some((60, [10624., 0., 8576.])));
        assert_eq!(
            tile_origin(MapId::from_bytes([60, 10, 8, 2])),
            Some((60, [10624., 8576.]))
        );
        // without the colosseum's anchor there is no place for it
        let none = Placements::from_conv(&ConvParam { rows: Vec::new() });
        assert!(none.get(id(45, 0, 0)).is_none());
        assert!(none.get(id(45, 1, 0)).is_some());
    }

    #[test]
    fn tile_frames_nest() {
        // a level-1 tile's origin is where the average of its four small tiles' origins is; the tile `_10` shares the
        // base tile's origin
        let small = [(44u8, 36u8), (45, 36), (44, 37), (45, 37)];
        let (_, big) = tile_origin(MapId::from_bytes([60, 22, 18, 1])).unwrap();
        let mean = small.iter().fold([0., 0.], |acc, &(x, z)| {
            let (_, o) = tile_origin(id(60, x, z)).unwrap();
            [acc[0] + o[0] / 4., acc[1] + o[1] / 4.]
        });
        assert_eq!(big, mean);
        assert_eq!(
            tile_origin(MapId::from_bytes([60, 44, 36, 0x10])),
            tile_origin(id(60, 44, 36))
        );
        assert_eq!(
            tile_origin(MapId::from_bytes([60, 11, 9, 0x12])),
            tile_origin(MapId::from_bytes([60, 11, 9, 2]))
        );
        assert!(tile_origin(MapId::from_bytes([60, 44, 36, 0x99])).is_none());
        assert!(tile_origin(id(14, 0, 0)).is_none());
        assert!(tile_origin(id(60, 0, 0)).is_none());
    }

    fn param_of(rows: Vec<crate::param::Row>) -> Param {
        Param {
            param_type: PARAM_TYPE.to_owned(),
            data_version: 2,
            unk06: 0,
            row_size: 48,
            rows,
        }
    }

    fn data(src: [u8; 3], s: [f32; 3], dst: [u8; 3], d: [f32; 3], base: u8) -> Vec<u8> {
        let mut b = vec![0u8; 48];
        b[4..7].copy_from_slice(&src);
        b[20..23].copy_from_slice(&dst);
        for k in 0..3 {
            b[8 + 4 * k..12 + 4 * k].copy_from_slice(&s[k].to_le_bytes());
            b[24 + 4 * k..28 + 4 * k].copy_from_slice(&d[k].to_le_bytes());
        }
        b[36] = base;
        b
    }

    const LAYOUT: ConvLayout = ConvLayout {
        data_version: 2,
        row_size: 48,
    };

    #[test]
    fn a_row_reads_and_padding_is_checked() {
        let good = crate::param::Row {
            id: 133,
            name: None,
            data: data([14, 0, 0], [0.; 3], [60, 35, 45], [-112.058, 310.577, 194.278], 1),
        };
        let conv = ConvParam::from_param(&param_of(vec![good.clone()]), &LAYOUT).unwrap();
        assert_eq!(conv.rows.len(), 1);
        assert_eq!(conv.rows[0].dst, id(60, 35, 45));
        assert!(conv.rows[0].is_base_point && !conv.rows[0].disabled_in_nt);
        for (at, value) in [
            (1, 1u8),
            (7, 1),
            (23, 9),
            (37, 1),
            (47, 0x80),
            (0, 2),
            (36, 3),
            (4, 100),
            (21, 200),
        ] {
            let mut bad = good.clone();
            bad.data[at] = value;
            assert!(
                ConvParam::from_param(&param_of(vec![bad]), &LAYOUT).is_err(),
                "byte {at} = {value}"
            );
        }
        let mut nan = good.clone();
        nan.data[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(ConvParam::from_param(&param_of(vec![nan]), &LAYOUT).is_err());
        let again = crate::param::Row {
            id: 133,
            ..good.clone()
        };
        assert!(ConvParam::from_param(&param_of(vec![good.clone(), again]), &LAYOUT).is_err());
        let mut other = param_of(vec![good]);
        other.row_size = 44;
        assert!(ConvParam::from_param(&other, &LAYOUT).is_err());
    }
}
