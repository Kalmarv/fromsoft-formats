//! Elden Ring's weather assets, read from params' rows as stored ([`crate::param::Param`]): which assets of a sky map
//! each weather shows (`WeatherAssetReplaceParam`, [`WeatherAssetParam`]), the assets a weather change or the time spent
//! in a weather creates (`WeatherAssetCreateParam`, [`WeatherAssetCreateParam`]), and each map's creation limit, which
//! says which rows with limits apply on it (`MapDefaultInfoParam`'s `WeatherCreateAssetLimitId`,
//! [`WeatherAssetLimits`]). The offsets are those of the Paramdex definitions of the three param types, checked
//! against the install's rows (one layout each: type, layout revision and row size); a param of another type, revision
//! or row size is refused, and so is a row whose padding is not zero.
//!
//! What the install's params show (`tests/weather_assets_census.rs`): `WeatherAssetReplaceParam`'s 1,007 rows name 27
//! sky maps; `WeatherAssetCreateParam` has 8 rows creating four assets (AEG096_200 and AEG096_201 by weather,
//! AEG050_448 and AEG050_449 by the later addition's weather 82); `MapDefaultInfoParam` has 875 rows.
use crate::param::Param;
use crate::{Result, invalid};

const FORMAT: &str = "weather assets";

/// `WeatherAssetReplaceParam`'s param type.
pub const WEATHER_ASSET_REPLACE_PARAM_TYPE: &str = "WEATHER_ASSET_REPLACE_PARAM_ST";
/// The layout read (the install's): version 2, rows of 64 bytes holding `mapId` (u32) at 0, `TransitionSrcWeather`
/// (s16) at 4, `isFireAsh` (u8) at 8, `AssetId0`..`AssetId7` (s32) at 16..48 and `CreateAssetLimitId0`..`3` (s8) at
/// 56..60; the other bytes are padding and the reserved `reserved2` (u32 at 12, whose stated maximum is 0).
const REPLACE_VERSION: u16 = 2;
const REPLACE_ROW: usize = 64;
/// The padding and reserved bytes of a replace row: zero in every row the reader accepts.
const REPLACE_PADDING: [(usize, usize); 4] = [(6, 8), (9, 16), (48, 56), (60, 64)];

/// `WeatherAssetCreateParam`'s param type.
pub const WEATHER_ASSET_CREATE_PARAM_TYPE: &str = "WEATHER_ASSET_CREATE_PARAM_ST";
/// The layout read (the install's): version 1, rows of 64 bytes holding `AssetId` (u32) at 0, `SlotNo` (u32) at 4,
/// `CreateConditionType` (u8) at 8, `TransitionSrcWeather`, `TransitionDstWeather` and `ElapsedTimeCheckweather` (s16)
/// at 12, 14 and 16, `ElapsedTime`, `CreateDelayTime` (f32) at 20 and 24, `CreateProbability` (u32) at 28, `LifeTime`,
/// `FadeTime`, `EnableCreateTimeMin` and `EnableCreateTimeMax` (f32) at 32..48, `CreatePoint0`..`3` (s16) at 48..56 and
/// `CreateAssetLimitId0`..`3` (s8) at 56..60; the other bytes are padding and the reserved `Reserved2`.
const CREATE_VERSION: u16 = 1;
const CREATE_ROW: usize = 64;
/// The padding and reserved bytes of a create row.
const CREATE_PADDING: [(usize, usize); 3] = [(9, 12), (18, 20), (60, 64)];

/// `MapDefaultInfoParam`'s param type, of which [`WeatherAssetLimits`] reads one field.
pub const MAP_DEFAULT_INFO_PARAM_TYPE: &str = "MAP_DEFAULT_INFO_PARAM_ST";
/// The layout read (the install's): version 6, rows of 64 bytes, `WeatherCreateAssetLimitId` (s8) at 12.
const MAP_DEFAULT_INFO_VERSION: u16 = 6;
const MAP_DEFAULT_INFO_ROW: usize = 64;
const WEATHER_CREATE_ASSET_LIMIT: usize = 12;

/// The creation limit slots of a replace or create row (`CreateAssetLimitId0`..`3`, at 56).
const LIMITS_AT: usize = 56;
const LIMITS: usize = 4;

/// `param` is `param_type` of the layout read (`version`, rows of `size` bytes), or the read is refused.
fn refuse(param: &Param, param_type: &str, version: u16, size: usize) -> Result<()> {
    if param.param_type != param_type || (param.data_version, param.row_size) != (version, size) {
        return Err(invalid(
            FORMAT,
            0,
            format!(
                "{} version {} with rows of {} bytes, the read is {param_type} version {version} with rows of {size}",
                param.param_type, param.data_version, param.row_size
            ),
        ));
    }
    Ok(())
}

/// A row's bytes as the layout's `N`, with every padding byte zero.
fn row<'a, const N: usize>(id: i32, data: &'a [u8], padding: &[(usize, usize)]) -> Result<&'a [u8; N]> {
    let d: &[u8; N] = data
        .try_into()
        .map_err(|_| invalid(FORMAT, 0, format!("row {id} is not a row of the layout read")))?;
    for &(start, end) in padding {
        let bytes = d.get(start..end).unwrap_or_default();
        if let Some((i, b)) = bytes.iter().enumerate().find(|(_, b)| **b != 0) {
            let at = start.saturating_add(i);
            return Err(invalid(
                FORMAT,
                at,
                format!("row {id}: padding byte {at} is {b:#04x}"),
            ));
        }
    }
    Ok(d)
}

/// The `N` bytes of a row at `at`.
fn bytes<const N: usize>(d: &[u8], at: usize) -> Result<[u8; N]> {
    d.get(at..at.saturating_add(N))
        .and_then(|b| <[u8; N]>::try_from(b).ok())
        .ok_or_else(|| invalid(FORMAT, at, "field beyond the row"))
}

/// The creation limits a row sets (its `CreateAssetLimitId` slots that are not -1), in slot order.
fn limits(d: &[u8]) -> Result<Vec<i8>> {
    Ok(bytes::<LIMITS>(d, LIMITS_AT)?
        .into_iter()
        .map(|b| i8::from_le_bytes([b]))
        .filter(|&l| l != -1)
        .collect())
}

/// Whether a row with the creation limits `limits` applies on a map whose limit is `limit`: by the fields' definition
/// only on a map whose `MapDefaultInfoParam` creation limit is one of them; with none set, on every map.
fn applies(limits: &[i8], limit: i8) -> bool {
    limits.is_empty() || limits.contains(&limit)
}

/// The asset model `AEGnnn_mmm` an asset id `nnn * 1000 + mmm` names (the params' convention: `AEG999_999` is 999999).
pub fn asset_model_name(id: i32) -> String {
    format!("AEG{:03}_{:03}", id / 1000, id % 1000)
}

/// One row of the weather asset replace param: on the sky map `map`, under the weather `weather`, in the fire-ash state
/// `fire_ash`, on maps of one of the creation limits `limits` (or any map), the assets `assets` are the ones shown.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherAssetRow {
    /// The row's ID.
    pub id: i32,
    /// `mapId`: the sky map as eight decimal digits, `AABBCCDD` for `mAA_BB_CC_DD` (60000099: `m60_00_00_99`).
    pub map: u32,
    /// `TransitionSrcWeather`: the weather, a `WeatherParam` row.
    pub weather: i16,
    /// `isFireAsh` as stored: 0 and 1 (before and after the map's fire-ash state, as the field's definition describes
    /// it) in the base game's rows; 2 in the rows of the later addition's maps, a value the definition does not give.
    pub fire_ash: u8,
    /// The asset ids its `AssetId` slots name (positive), in slot order; an unused slot holds -1 and may come before a
    /// used one.
    pub assets: Vec<i32>,
    /// The ids its `CreateAssetLimitId` slots set (not -1), in slot order: by the field's definition the row applies
    /// only on a map whose `MapDefaultInfoParam` creation limit is one of them; none set, on every map.
    pub limits: Vec<i8>,
}

impl WeatherAssetRow {
    /// The sky map's name: `m60_00_00_99` for 60000099 (from the number's last eight digits).
    pub fn map_name(&self) -> String {
        format!(
            "m{:02}_{:02}_{:02}_{:02}",
            self.map / 1_000_000 % 100,
            self.map / 10_000 % 100,
            self.map / 100 % 100,
            self.map % 100
        )
    }

    /// Whether the row applies on a map whose creation limit is `limit` (-1 for a map that sets none).
    pub fn applies_at(&self, limit: i8) -> bool {
        applies(&self.limits, limit)
    }
}

/// `WeatherAssetReplaceParam`: which assets of a sky map each weather shows.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherAssetParam {
    /// Every row, in file order.
    pub rows: Vec<WeatherAssetRow>,
}

impl WeatherAssetParam {
    /// The rows of `param`, which must be [`WEATHER_ASSET_REPLACE_PARAM_TYPE`] of the install's layout (version 2,
    /// 64-byte rows) with every padding byte zero and every asset slot an asset id or -1 (any other is refused).
    pub fn from_param(param: &Param) -> Result<WeatherAssetParam> {
        refuse(
            param,
            WEATHER_ASSET_REPLACE_PARAM_TYPE,
            REPLACE_VERSION,
            REPLACE_ROW,
        )?;
        let rows = param
            .rows
            .iter()
            .map(|r| {
                let d = row::<REPLACE_ROW>(r.id, &r.data, &REPLACE_PADDING)?;
                let mut assets = Vec::new();
                for k in 0..8usize {
                    let at = k.saturating_mul(4).saturating_add(16);
                    match i32::from_le_bytes(bytes(d, at)?) {
                        -1 => {}
                        id if id > 0 => assets.push(id),
                        id => {
                            return Err(invalid(
                                FORMAT,
                                at,
                                format!("row {}: AssetId{k} is {id}, neither an asset nor -1", r.id),
                            ));
                        }
                    }
                }
                let [fire_ash] = bytes::<1>(d, 8)?;
                Ok(WeatherAssetRow {
                    id: r.id,
                    map: u32::from_le_bytes(bytes(d, 0)?),
                    weather: i16::from_le_bytes(bytes(d, 4)?),
                    fire_ash,
                    assets,
                    limits: limits(d)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WeatherAssetParam { rows })
    }
}

/// One row of the weather asset create param: the asset `asset` created in the slot `slot` when the row's condition
/// holds, for its life time, at one of its creation points.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WeatherAssetCreate {
    /// The row's ID.
    pub id: i32,
    /// `AssetId`: the asset created, `nnn * 1000 + mmm` for `AEGnnn_mmm` ([`asset_model_name`]).
    pub asset: u32,
    /// `SlotNo`: by the field's definition one asset at a time is created in a slot (0 to 4).
    pub slot: u32,
    /// `CreateConditionType` as stored. By the definition there are two conditions: a weather transition (the
    /// transition weathers are read only for it) and the time spent in a weather (the elapsed time is read only for it).
    /// The install's rows: 0 in the six that set a transition, 1 in the two that set an elapsed time; the definition
    /// does not number them.
    pub condition: u8,
    /// `TransitionSrcWeather` and `TransitionDstWeather`: the weather changed from and to (`WeatherParam` rows).
    pub transition: (i16, i16),
    /// `ElapsedTimeCheckweather`: the weather whose time is counted.
    pub elapsed_weather: i16,
    /// `ElapsedTime`: the real-time seconds spent in it.
    pub elapsed_time: f32,
    /// `CreateDelayTime`: seconds between the decision and the creation; 0 or less, at once.
    pub delay: f32,
    /// `CreateProbability`: the percent chance of creation when the condition holds.
    pub probability: u32,
    /// `LifeTime`: the created asset's life in real-time seconds.
    pub life_time: f32,
    /// `FadeTime`: the fade in and out in seconds; 0 or less, none.
    pub fade_time: f32,
    /// `EnableCreateTimeMin` and `EnableCreateTimeMax`: the in-game hours (0 to 24) it may be created between; by the
    /// definition -1 in either means at any hour.
    pub hours: (f32, f32),
    /// The creation points its `CreatePoint` slots set (not -1), in slot order: the last three digits of the layouts'
    /// weather asset creation points.
    pub points: Vec<i16>,
    /// The creation limits its `CreateAssetLimitId` slots set (not -1), as [`WeatherAssetRow::limits`].
    pub limits: Vec<i8>,
}

impl WeatherAssetCreate {
    /// Whether the row applies on a map whose creation limit is `limit` (-1 for a map that sets none).
    pub fn applies_at(&self, limit: i8) -> bool {
        applies(&self.limits, limit)
    }
}

/// `WeatherAssetCreateParam`: the assets a weather change or the time spent in a weather creates.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct WeatherAssetCreateParam {
    /// Every row, in file order.
    pub rows: Vec<WeatherAssetCreate>,
}

impl WeatherAssetCreateParam {
    /// The rows of `param`, which must be [`WEATHER_ASSET_CREATE_PARAM_TYPE`] of the install's layout (version 1,
    /// 64-byte rows) with every padding byte zero (any other is refused).
    pub fn from_param(param: &Param) -> Result<WeatherAssetCreateParam> {
        refuse(param, WEATHER_ASSET_CREATE_PARAM_TYPE, CREATE_VERSION, CREATE_ROW)?;
        let rows = param
            .rows
            .iter()
            .map(|r| {
                let d = row::<CREATE_ROW>(r.id, &r.data, &CREATE_PADDING)?;
                let f32_at = |at: usize| bytes(d, at).map(f32::from_le_bytes);
                let i16_at = |at: usize| bytes(d, at).map(i16::from_le_bytes);
                let mut points = Vec::new();
                for k in 0..4usize {
                    match i16_at(k.saturating_mul(2).saturating_add(48))? {
                        -1 => {}
                        p => points.push(p),
                    }
                }
                let [condition] = bytes::<1>(d, 8)?;
                Ok(WeatherAssetCreate {
                    id: r.id,
                    asset: u32::from_le_bytes(bytes(d, 0)?),
                    slot: u32::from_le_bytes(bytes(d, 4)?),
                    condition,
                    transition: (i16_at(12)?, i16_at(14)?),
                    elapsed_weather: i16_at(16)?,
                    elapsed_time: f32_at(20)?,
                    delay: f32_at(24)?,
                    probability: u32::from_le_bytes(bytes(d, 28)?),
                    life_time: f32_at(32)?,
                    fade_time: f32_at(36)?,
                    hours: (f32_at(40)?, f32_at(44)?),
                    points,
                    limits: limits(d)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WeatherAssetCreateParam { rows })
    }
}

/// Each map's weather asset creation limit: `MapDefaultInfoParam`'s `WeatherCreateAssetLimitId` by row id, the row id
/// being the map as eight decimal digits (`60423600` for `m60_42_36_00`); -1 where the map sets none.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherAssetLimits {
    /// The limit of each row's map.
    pub maps: std::collections::BTreeMap<i32, i8>,
}

impl WeatherAssetLimits {
    /// The limits of `param`, which must be [`MAP_DEFAULT_INFO_PARAM_TYPE`] of the install's layout (version 6, 64-byte
    /// rows) with no row id twice (any other is refused).
    pub fn from_param(param: &Param) -> Result<WeatherAssetLimits> {
        refuse(
            param,
            MAP_DEFAULT_INFO_PARAM_TYPE,
            MAP_DEFAULT_INFO_VERSION,
            MAP_DEFAULT_INFO_ROW,
        )?;
        let mut maps = std::collections::BTreeMap::new();
        for r in &param.rows {
            let [limit] = bytes::<1>(&r.data, WEATHER_CREATE_ASSET_LIMIT)?;
            if maps.insert(r.id, i8::from_le_bytes([limit])).is_some() {
                return Err(invalid(FORMAT, 0, format!("row {} stored twice", r.id)));
            }
        }
        Ok(WeatherAssetLimits { maps })
    }

    /// The creation limit of the map whose eight-digit number is `map`: `None` when the param has no row for it.
    pub fn of(&self, map: i32) -> Option<i8> {
        self.maps.get(&map).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::Row;

    /// A param of `param_type`, layout revision `version` and `size`-byte rows holding `rows` (id, data).
    fn param(param_type: &str, version: u16, size: usize, rows: Vec<(i32, Vec<u8>)>) -> Param {
        Param {
            param_type: param_type.into(),
            data_version: version,
            unk06: 1,
            row_size: size,
            rows: rows
                .into_iter()
                .map(|(id, data)| Row {
                    id,
                    name: Some(String::new()),
                    data,
                })
                .collect(),
        }
    }

    /// A weather asset row of `map`, `weather`, `fire_ash`, the asset slots `assets` and the limit slots `limits`.
    fn weather_row(map: u32, weather: i16, fire_ash: u8, assets: [i32; 8], limits: [i8; 4]) -> Vec<u8> {
        let mut d = vec![0u8; 64];
        d[0..4].copy_from_slice(&map.to_le_bytes());
        d[4..6].copy_from_slice(&weather.to_le_bytes());
        d[8] = fire_ash;
        for (k, a) in assets.iter().enumerate() {
            d[16 + 4 * k..20 + 4 * k].copy_from_slice(&a.to_le_bytes());
        }
        for (k, l) in limits.iter().enumerate() {
            d[56 + k] = *l as u8;
        }
        d
    }

    /// A row reads its map, weather, fire-ash state, the assets of its used slots (an unused slot between them
    /// skipped) and its set limits; the map's name and where the row applies follow.
    #[test]
    fn a_weather_asset_row_reads_its_map_weather_assets_and_limits() {
        let rows = vec![
            (
                60001000,
                weather_row(
                    60000099,
                    0,
                    0,
                    [96010, -1, 96400, -1, -1, -1, -1, -1],
                    [0, -1, -1, -1],
                ),
            ),
            (
                1000100,
                weather_row(10000099, 20, 1, [96603, 96611, -1, -1, -1, -1, -1, -1], [-1; 4]),
            ),
        ];
        let p = WeatherAssetParam::from_param(&param(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, 64, rows)).unwrap();
        let [open, legacy] = [&p.rows[0], &p.rows[1]];
        assert_eq!(
            open,
            &WeatherAssetRow {
                id: 60001000,
                map: 60000099,
                weather: 0,
                fire_ash: 0,
                assets: vec![96010, 96400],
                limits: vec![0],
            }
        );
        assert_eq!(open.map_name(), "m60_00_00_99");
        assert_eq!(legacy.map_name(), "m10_00_00_99");
        assert_eq!((legacy.weather, legacy.fire_ash), (20, 1));
        assert_eq!(asset_model_name(open.assets[1]), "AEG096_400");
        assert_eq!(
            (open.applies_at(0), open.applies_at(1), open.applies_at(-1)),
            (true, false, false)
        );
        assert!(legacy.applies_at(-1) && legacy.applies_at(4));
    }

    /// Another type or layout, a padding byte set and an asset slot that is neither an asset nor -1 are refused.
    #[test]
    fn a_weather_asset_param_outside_the_layout_is_refused() {
        let good = weather_row(60000099, 0, 0, [96010, -1, -1, -1, -1, -1, -1, -1], [-1; 4]);
        let read = |t: &str, v: u16, row: Vec<u8>| {
            let size = row.len();
            WeatherAssetParam::from_param(&param(t, v, size, vec![(1, row)]))
        };
        assert!(read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, good.clone()).is_ok());
        assert!(read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 3, good.clone()).is_err());
        assert!(read("NPC_PARAM_ST", 2, good.clone()).is_err());
        assert!(read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, vec![0; 68]).is_err());
        for at in [6, 9, 12, 15, 48, 55, 60, 63] {
            let mut bad = good.clone();
            bad[at] = 1;
            let e = read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, bad)
                .unwrap_err()
                .to_string();
            assert!(e.contains(&format!("padding byte {at} ")), "{e}");
        }
        for id in [0, -2] {
            let bad = weather_row(60000099, 0, 0, [96010, id, -1, -1, -1, -1, -1, -1], [-1; 4]);
            let e = read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, bad)
                .unwrap_err()
                .to_string();
            assert!(e.contains(&format!("AssetId1 is {id}")), "{e}");
        }
        // a row whose data is not the param's row size is not a row of the layout
        let short = param(WEATHER_ASSET_REPLACE_PARAM_TYPE, 2, 64, vec![(1, vec![0; 60])]);
        assert!(WeatherAssetParam::from_param(&short).is_err());
    }

    /// A create row of the install's layout: `AssetId` 96200, slot 0, condition 0, the transition 21 to 1, elapsed
    /// weather 0 and time 0, no delay (-1), 100 percent, a life of 600 s, a fade of 3.5 s, between 7 and 16 o'clock, at
    /// the creation point 1, for the creation limit 70.
    fn create_row() -> Vec<u8> {
        let mut d = vec![0u8; 64];
        d[0..4].copy_from_slice(&96200u32.to_le_bytes());
        d[12..14].copy_from_slice(&21i16.to_le_bytes());
        d[14..16].copy_from_slice(&1i16.to_le_bytes());
        d[24..28].copy_from_slice(&(-1.0f32).to_le_bytes());
        d[28..32].copy_from_slice(&100u32.to_le_bytes());
        d[32..36].copy_from_slice(&600.0f32.to_le_bytes());
        d[36..40].copy_from_slice(&3.5f32.to_le_bytes());
        d[40..44].copy_from_slice(&7.0f32.to_le_bytes());
        d[44..48].copy_from_slice(&16.0f32.to_le_bytes());
        d[48..50].copy_from_slice(&1i16.to_le_bytes());
        for at in [50, 52, 54] {
            d[at..at + 2].copy_from_slice(&(-1i16).to_le_bytes());
        }
        d[56] = 70;
        d[57..60].fill(0xff);
        d
    }

    /// A create row reads every field at its offset, the creation points and limits it sets; another layout and a
    /// padding byte set are refused.
    #[test]
    fn a_weather_asset_create_row_reads_its_asset_condition_and_times() {
        let p = WeatherAssetCreateParam::from_param(&param(
            WEATHER_ASSET_CREATE_PARAM_TYPE,
            1,
            64,
            vec![(1, create_row())],
        ))
        .unwrap();
        assert_eq!(
            p.rows,
            [WeatherAssetCreate {
                id: 1,
                asset: 96200,
                slot: 0,
                condition: 0,
                transition: (21, 1),
                elapsed_weather: 0,
                elapsed_time: 0.0,
                delay: -1.0,
                probability: 100,
                life_time: 600.0,
                fade_time: 3.5,
                hours: (7.0, 16.0),
                points: vec![1],
                limits: vec![70],
            }]
        );
        assert_eq!(asset_model_name(p.rows[0].asset as i32), "AEG096_200");
        assert!(p.rows[0].applies_at(70) && !p.rows[0].applies_at(-1));
        let read = |t: &str, v: u16, row: Vec<u8>| {
            let size = row.len();
            WeatherAssetCreateParam::from_param(&param(t, v, size, vec![(1, row)]))
        };
        assert!(read(WEATHER_ASSET_CREATE_PARAM_TYPE, 2, create_row()).is_err());
        assert!(read(WEATHER_ASSET_REPLACE_PARAM_TYPE, 1, create_row()).is_err());
        assert!(read(WEATHER_ASSET_CREATE_PARAM_TYPE, 1, vec![0; 68]).is_err());
        for at in [9, 11, 18, 19, 60, 63] {
            let mut bad = create_row();
            bad[at] = 1;
            let e = read(WEATHER_ASSET_CREATE_PARAM_TYPE, 1, bad)
                .unwrap_err()
                .to_string();
            assert!(e.contains(&format!("padding byte {at} ")), "{e}");
        }
    }

    /// The map default param's creation limit per map, read at its offset; a row id twice and other layouts refused.
    #[test]
    fn map_default_rows_give_each_maps_weather_asset_limit() {
        let row = |limit: i8| {
            let mut d = vec![0u8; 64];
            d[12] = limit as u8;
            d
        };
        let limits = WeatherAssetLimits::from_param(&param(
            MAP_DEFAULT_INFO_PARAM_TYPE,
            6,
            64,
            vec![(60423600, row(0)), (10000000, row(-1))],
        ))
        .unwrap();
        assert_eq!(
            (limits.of(60423600), limits.of(10000000), limits.of(60000000)),
            (Some(0), Some(-1), None)
        );
        let twice = param(MAP_DEFAULT_INFO_PARAM_TYPE, 6, 64, vec![(1, row(0)), (1, row(1))]);
        assert!(WeatherAssetLimits::from_param(&twice).is_err());
        let other = param(MAP_DEFAULT_INFO_PARAM_TYPE, 5, 64, vec![(1, row(0))]);
        assert!(WeatherAssetLimits::from_param(&other).is_err());
    }
}
