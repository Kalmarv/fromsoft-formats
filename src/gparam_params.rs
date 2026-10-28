//! The params that decide which draw-param file (graphics environment) Elden Ring uses, read from rows as stored
//! ([`crate::param::Param`]): `GparamRefSettingsParam` (a map uses the files of another map), `WeatherParam` (a weather
//! names the file of the global weather set `m00_00_<id>_weatheroutdoor`) and `WeatherLotParam` (a lot is a weighted
//! choice among weathers). The offsets are those of the Paramdex definitions of the three param types, one layout
//! each; a param of another type or row size is refused.
//!
//! What the stock params show (`tests/weather_census.rs`): every one of `GparamRefSettingsParam`'s 70 rows is keyed by a
//! map's decimal number (see [`MapId::from_decimal`]); all 82 distinct `WeatherParam` file ids name a
//! `m00_00_<id>_weatheroutdoor` file the archive holds (the archive holds two more, 98 and 150, that no row names); the
//! 272 weighted entries of the 164 `WeatherLotParam` rows all name a `WeatherParam` row.
use crate::mapid::MapId;
use crate::param::Param;
use crate::{Result, invalid};

const FORMAT: &str = "gparam params";

/// `GparamRefSettingsParam`'s param type and row size.
const REF_TYPE: &str = "GPARAM_REF_SETTINGS_PARAM_ST";
const REF_ROW: usize = 32;
/// `WeatherParam`'s.
const WEATHER_TYPE: &str = "WEATHER_PARAM_ST";
const WEATHER_ROW: usize = 68;
/// `WeatherLotParam`'s, and the weathers a row chooses among.
const LOT_TYPE: &str = "WEATHER_LOT_PARAM_ST";
const LOT_ROW: usize = 82;
/// The weather choices of a lot row.
pub const LOT_ENTRIES: usize = 16;

fn refuse(param: &Param, expected: &str, size: usize) -> Result<()> {
    if param.param_type != expected || param.row_size != size {
        return Err(invalid(
            FORMAT,
            0,
            format!(
                "{} with rows of {} bytes, the read is {expected} with rows of {size}",
                param.param_type, param.row_size
            ),
        ));
    }
    Ok(())
}

fn bytes<const N: usize>(data: &[u8], at: usize) -> Result<[u8; N]> {
    data.get(at..at.saturating_add(N))
        .and_then(|b| <[u8; N]>::try_from(b).ok())
        .ok_or_else(|| invalid(FORMAT, at, "field beyond the row"))
}

/// One row of `GparamRefSettingsParam`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GparamRef {
    /// The row's ID: the decimal number of the map that borrows the files (`30010000` is `m30_01_00_00`).
    pub id: i32,
    /// `RefTargetMapId`: the decimal number of the map whose files it uses; -1 for none.
    pub target: i32,
}

impl GparamRef {
    /// The map that borrows the files.
    pub fn map(&self) -> Option<MapId> {
        MapId::from_decimal(self.id)
    }

    /// The map whose files it uses.
    pub fn target_map(&self) -> Option<MapId> {
        MapId::from_decimal(self.target)
    }
}

/// `GparamRefSettingsParam`: which map's draw-param files a map uses when it has none of its own.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct GparamRefSettings {
    /// The param's layout revision.
    pub data_version: u16,
    /// Every row, in file order.
    pub rows: Vec<GparamRef>,
}

impl GparamRefSettings {
    /// The rows of `param`, which must be `GPARAM_REF_SETTINGS_PARAM_ST` with rows of 32 bytes.
    pub fn from_param(param: &Param) -> Result<GparamRefSettings> {
        refuse(param, REF_TYPE, REF_ROW)?;
        let rows = param
            .rows
            .iter()
            .map(|r| {
                Ok(GparamRef {
                    id: r.id,
                    target: i32::from_le_bytes(bytes(&r.data, 4)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(GparamRefSettings {
            data_version: param.data_version,
            rows,
        })
    }

    /// The map whose files `map` uses, if a row says so.
    pub fn target_of(&self, map: MapId) -> Option<MapId> {
        self.rows
            .iter()
            .find(|r| r.map() == Some(map))
            .and_then(GparamRef::target_map)
    }
}

/// One row of `WeatherParam`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherRow {
    /// The weather's ID.
    pub id: i32,
    /// `GparamId`: the number of the draw-param file the weather uses.
    pub gparam_id: u32,
}

impl WeatherRow {
    /// The archive path of the file the weather uses: `m00_00_<gparam id>_weatheroutdoor`.
    pub fn gparam_path(&self) -> String {
        weather_outdoor_path(self.gparam_id)
    }
}

/// The archive path of the weather file with this id (`/param/drawparam/m00_00_0020_weatheroutdoor.gparam.dcx`).
pub fn weather_outdoor_path(gparam_id: u32) -> String {
    format!("/param/drawparam/m00_00_{gparam_id:04}_weatheroutdoor.gparam.dcx")
}

/// `WeatherParam`: the weathers and the draw-param file each uses.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherParam {
    /// The param's layout revision.
    pub data_version: u16,
    /// Every row, in file order.
    pub rows: Vec<WeatherRow>,
}

impl WeatherParam {
    /// The rows of `param`, which must be `WEATHER_PARAM_ST` with rows of 68 bytes.
    pub fn from_param(param: &Param) -> Result<WeatherParam> {
        refuse(param, WEATHER_TYPE, WEATHER_ROW)?;
        let rows = param
            .rows
            .iter()
            .map(|r| {
                Ok(WeatherRow {
                    id: r.id,
                    gparam_id: u32::from_le_bytes(bytes(&r.data, 20)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WeatherParam {
            data_version: param.data_version,
            rows,
        })
    }

    /// The row with ID `id`.
    pub fn row(&self, id: i32) -> Option<&WeatherRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

/// One choice of a weather lot: a weather and its weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct LotEntry {
    /// `weatherTypeN`: a `WeatherParam` row ID (0 or -1 where unused).
    pub weather: i16,
    /// `lotteryWeightN`: its weight; 0 for an unused entry.
    pub weight: u16,
}

/// One row of `WeatherLotParam`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherLotRow {
    /// The lot's ID (the `Weather Lot Param ID` the event scripts' weather conditions name).
    pub id: i32,
    /// The sixteen choices, in stored order.
    pub entries: [LotEntry; LOT_ENTRIES],
    /// The five time-zone bytes at +68: a limit flag and the start and end as hour and minute. As stored.
    pub timezone: [u8; 5],
}

impl WeatherLotRow {
    /// The choices with a weight.
    pub fn weighted(&self) -> impl Iterator<Item = LotEntry> + '_ {
        self.entries.iter().copied().filter(|e| e.weight > 0)
    }
}

/// `WeatherLotParam`: the weighted choices of weather a lot makes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WeatherLotParam {
    /// The param's layout revision.
    pub data_version: u16,
    /// Every row, in file order.
    pub rows: Vec<WeatherLotRow>,
}

impl WeatherLotParam {
    /// The rows of `param`, which must be `WEATHER_LOT_PARAM_ST` with rows of 82 bytes.
    pub fn from_param(param: &Param) -> Result<WeatherLotParam> {
        refuse(param, LOT_TYPE, LOT_ROW)?;
        let rows = param
            .rows
            .iter()
            .map(|r| {
                let mut entries = [LotEntry {
                    weather: 0,
                    weight: 0,
                }; LOT_ENTRIES];
                for (i, entry) in entries.iter_mut().enumerate() {
                    let step = i.saturating_mul(2);
                    *entry = LotEntry {
                        weather: i16::from_le_bytes(bytes(&r.data, 4usize.saturating_add(step))?),
                        weight: u16::from_le_bytes(bytes(&r.data, 36usize.saturating_add(step))?),
                    };
                }
                Ok(WeatherLotRow {
                    id: r.id,
                    entries,
                    timezone: bytes(&r.data, 68)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WeatherLotParam {
            data_version: param.data_version,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::Row;

    fn param(param_type: &str, size: usize, rows: Vec<(i32, Vec<u8>)>) -> Param {
        Param {
            param_type: param_type.into(),
            data_version: 3,
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

    #[test]
    fn a_map_names_the_map_whose_files_it_uses() {
        let mut row = vec![0u8; 32];
        row[4..8].copy_from_slice(&30_000_000i32.to_le_bytes());
        let mut none = vec![0u8; 32];
        none[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        let refs =
            GparamRefSettings::from_param(&param(REF_TYPE, 32, vec![(31_050_000, row), (99_990_001, none)]))
                .unwrap();
        assert_eq!(
            refs.rows[0].map().unwrap().name().as_deref(),
            Some("m31_05_00_00")
        );
        assert_eq!(
            refs.target_of(MapId::from_name("m31_05_00_00").unwrap())
                .and_then(MapId::name)
                .as_deref(),
            Some("m30_00_00_00")
        );
        // -1 names no map, and a map without a row has no target
        assert_eq!(refs.rows[1].target_map(), None);
        assert_eq!(refs.target_of(MapId::from_name("m10_00_00_00").unwrap()), None);
    }

    #[test]
    fn a_weather_names_its_file() {
        let mut row = vec![0u8; 68];
        row[20..24].copy_from_slice(&621u32.to_le_bytes());
        let weather = WeatherParam::from_param(&param(WEATHER_TYPE, 68, vec![(1001, row)])).unwrap();
        assert_eq!(weather.row(1001).unwrap().gparam_id, 621);
        assert_eq!(
            weather.row(1001).unwrap().gparam_path(),
            "/param/drawparam/m00_00_0621_weatheroutdoor.gparam.dcx"
        );
        assert!(weather.row(1).is_none());
    }

    #[test]
    fn a_lot_chooses_among_weighted_weathers() {
        let mut row = vec![0u8; 82];
        row[4..6].copy_from_slice(&10i16.to_le_bytes());
        row[36..38].copy_from_slice(&700u16.to_le_bytes());
        row[6..8].copy_from_slice(&11i16.to_le_bytes());
        row[38..40].copy_from_slice(&300u16.to_le_bytes());
        // an entry with a weather and no weight is not a choice
        row[8..10].copy_from_slice(&20i16.to_le_bytes());
        row[68..73].copy_from_slice(&[1, 6, 30, 18, 0]);
        let lots = WeatherLotParam::from_param(&param(LOT_TYPE, 82, vec![(2000, row)])).unwrap();
        let lot = &lots.rows[0];
        assert_eq!(
            lot.weighted().map(|e| (e.weather, e.weight)).collect::<Vec<_>>(),
            [(10, 700), (11, 300)]
        );
        assert_eq!(lot.timezone, [1, 6, 30, 18, 0]);
    }

    #[test]
    fn another_param_type_or_row_size_is_refused() {
        let wrong = param("NPC_PARAM_ST", 68, Vec::new());
        assert!(WeatherParam::from_param(&wrong).is_err());
        let short = param(WEATHER_TYPE, 64, Vec::new());
        assert!(WeatherParam::from_param(&short).is_err());
        assert!(GparamRefSettings::from_param(&short).is_err());
        assert!(WeatherLotParam::from_param(&short).is_err());
    }
}
