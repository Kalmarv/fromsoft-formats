//! The files a map keeps besides its layout, where the games keep them: its light bank (`<folder>/<map>_0000.btl.dcx`),
//! its hang-edge binder (`<folder>/<map>.edgebhd` and `.edgebdt`) and the draw-param files of its map
//! (`/param/drawparam/<mNN_NN>_<id>.gparam.dcx`). `folder` is the map's archive folder ([`crate::Game::map_folder`]).
use crate::btl::LightBank;
use crate::dcx::Decoder;
use crate::edge::Edges;
use crate::{Archive, Binder, Error, Game, Result};

/// The map's light bank zero, or `None` where the archives hold none.
pub fn light_bank(
    archive: &Archive,
    decoder: &Decoder<'_>,
    folder: &str,
    map: &str,
) -> Result<Option<LightBank>> {
    match archive.read(&format!("{folder}/{map}_0000.btl.dcx"), decoder) {
        Err(Error::Missing(_)) => Ok(None),
        Err(e) => Err(e),
        Ok(bytes) => LightBank::read(&bytes).map(Some),
    }
}

/// Every EDGE file of the map's edge binder, in binder order, by its file name (`m11_01_00_00_000.edge`), or `None`
/// where the archives hold no edge binder. Members that are not EDGE files are left out.
pub fn edge_files(
    archive: &Archive,
    decoder: &Decoder<'_>,
    folder: &str,
    map: &str,
) -> Result<Option<Vec<(String, Edges)>>> {
    let stem = format!("{folder}/{map}");
    let header = match archive.read(&format!("{stem}.edgebhd"), decoder) {
        Err(Error::Missing(_)) => return Ok(None),
        Err(e) => return Err(e),
        Ok(header) => header,
    };
    let binder = Binder::read_split(
        &header,
        &archive.read(&format!("{stem}.edgebdt"), decoder)?,
        decoder,
    )?;
    let mut files = Vec::new();
    for file in binder.files {
        let Some(name) = file.name.as_deref().filter(|s| {
            let lower = crate::names::fold(s);
            lower.strip_suffix(".dcx").unwrap_or(&lower).ends_with(".edge")
        }) else {
            continue;
        };
        let name = name.rsplit(['/', '\\']).next().unwrap_or(name).to_owned();
        files.push((name, Edges::read(&decoder.decode_if_dcx(&file.bytes)?)?));
    }
    Ok(Some(files))
}

/// The stems the draw-param files of an area's map can share, first choice first: `m11_01` for `m11_01_00_00`; Elden Ring's
/// open-world tiles share `m60_00` and `m61_00` (no file carries a tile's coordinates), an Elden Ring map with
/// no file of its own stem uses its family's `mAA_00` (m30's dungeons, m19, one m11 map), and the caves and tunnels (`m31`,
/// `m32`) use the catacombs' `m30_00`. The game's own table says so for these (`GparamRefSettingsParam`: a row per map,
/// keyed by its decimal map number, names the map whose files it uses; 70 rows, every one of them agrees with this rule,
/// `tests/weather_census.rs`). Empty for a name that is not an area's.
fn draw_param_maps(game: Game, area: &str) -> Vec<String> {
    let (Some(first), Some(second)) = (area.get(..3), area.get(4..6)) else {
        return Vec::new();
    };
    match (game, first) {
        (Game::EldenRing, "m60" | "m61") => vec![format!("{first}_00")],
        (Game::EldenRing, "m31" | "m32") => vec![format!("{first}_{second}"), "m30_00".to_owned()],
        (Game::EldenRing, _) if second != "00" => vec![format!("{first}_{second}"), format!("{first}_00")],
        _ => vec![format!("{first}_{second}")],
    }
}

/// The four-digit id of a draw-param file (`0720` of `m11_01_0720`; every file of both installs has a 0 for its first digit).
/// Its hundreds digit is the *slot* (0 to 9: the first
/// scene byte of a collision part chooses it, and the area's lighting states fill the slots of `m10_00`) and its last
/// two digits the *sub id* (0, 10, 20 ... : an event sets it for an area, a collision's scene config or override names
/// it in the open world). Across the installs all 1,304 Sekiro collisions and 3,209 of the 3,266 Elden Ring collisions
/// name a file that exists by these two numbers (`tests/gparam_census.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DrawParamId(u16);

impl DrawParamId {
    /// The id of slot `slot` (0 to 9) and sub id `sub` (0 to 99).
    pub fn new(slot: u8, sub: u8) -> Option<DrawParamId> {
        if slot > 9 || sub > 99 {
            return None;
        }
        u16::from(slot)
            .checked_mul(100)?
            .checked_add(u16::from(sub))
            .map(DrawParamId)
    }

    /// The id with this number (0 to 999: every file of both installs is below 1000, the thousands digit being 0).
    pub fn from_number(number: u16) -> Option<DrawParamId> {
        (number <= 999).then_some(DrawParamId(number))
    }

    /// The four-digit number.
    pub fn number(self) -> u16 {
        self.0
    }

    /// The slot, the hundreds digit.
    pub fn slot(self) -> u8 {
        u8::try_from(self.0 / 100).unwrap_or(u8::MAX)
    }

    /// The sub id, the last two digits.
    pub fn sub(self) -> u8 {
        u8::try_from(self.0 % 100).unwrap_or(u8::MAX)
    }

    /// The stem of this id's file for the map `map` (`m11_01`): `m11_01_0720`.
    pub fn stem(self, map: &str) -> String {
        format!("{map}_{:04}", self.0)
    }

    /// A stem of the plain `<map>_<four digits>` form read back: `m11_01_0720` is (`m11_01`, 0720). `None` for the
    /// layered names (`m00_00_0000_weatherbase`) and for anything else.
    pub fn from_stem(stem: &str) -> Option<(&str, DrawParamId)> {
        let (map, digits) = stem.rsplit_once('_')?;
        if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) || map.is_empty() {
            return None;
        }
        Some((map, DrawParamId::from_number(digits.parse().ok()?)?))
    }
}

/// The archive path of the draw-param file `stem` (`m11_01_0720`).
pub fn draw_param_path(stem: &str) -> String {
    format!("/param/drawparam/{stem}.gparam.dcx")
}

/// The draw-param files the archive holds for `area`'s map, by stem, in ID order: every `<map>_<0000..9999>` of the first
/// of the stems its map's files can share that has any (its own `mAA_BB`, then, in Elden Ring, the stem a family or the
/// open world shares: `m60_00`, `m61_00`, `mAA_00`, the catacombs' `m30_00`). A map none of whose stems has a file has
/// none.
pub fn draw_params(archive: &Archive, game: Game, area: &str) -> Vec<String> {
    draw_params_where(game, area, |path| archive.contains(path))
}

fn draw_params_where(game: Game, area: &str, has: impl Fn(&str) -> bool) -> Vec<String> {
    for map in draw_param_maps(game, area) {
        let stems: Vec<String> = (0..10_000)
            .map(|id| format!("{map}_{id:04}"))
            .filter(|stem| has(&draw_param_path(stem)))
            .collect();
        if !stems.is_empty() {
            return stems;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_param_id_is_a_slot_and_a_sub_id() {
        let id = DrawParamId::new(7, 20).unwrap();
        assert_eq!((id.number(), id.slot(), id.sub()), (720, 7, 20));
        assert_eq!(id.stem("m11_01"), "m11_01_0720");
        assert_eq!(DrawParamId::from_stem("m11_01_0720"), Some(("m11_01", id)));
        assert_eq!(DrawParamId::from_number(0).unwrap().stem("m60_00"), "m60_00_0000");
        assert_eq!(DrawParamId::from_number(999).unwrap().slot(), 9);
        // a slot is one digit, a sub id two, a number below 1000
        assert_eq!(DrawParamId::new(10, 0), None);
        assert_eq!(DrawParamId::new(0, 100), None);
        assert_eq!(DrawParamId::from_number(1000), None);
        for stem in [
            "m00_00_0000_weatherbase",
            "m11_01_720",
            "m11_01_07200",
            "_0720",
            "m11_01_07x0",
            "default",
        ] {
            assert_eq!(DrawParamId::from_stem(stem), None, "{stem}");
        }
    }

    fn maps(game: Game, area: &str) -> Vec<String> {
        draw_param_maps(game, area)
    }

    #[test]
    fn a_map_names_its_files_by_its_first_two_blocks_and_open_world_tiles_share_one_set() {
        assert_eq!(maps(Game::Sekiro, "m11_01_00_00"), ["m11_01"]);
        assert_eq!(maps(Game::EldenRing, "m10_00_00_00"), ["m10_00"]);
        assert_eq!(maps(Game::EldenRing, "m12_02_00_00"), ["m12_02", "m12_00"]);
        assert_eq!(maps(Game::EldenRing, "m60_44_36_00"), ["m60_00"]);
        assert_eq!(maps(Game::EldenRing, "m61_50_50_00"), ["m61_00"]);
        // the caves and tunnels use the catacombs' files
        assert_eq!(maps(Game::EldenRing, "m31_05_00_00"), ["m31_05", "m30_00"]);
        assert_eq!(maps(Game::EldenRing, "m32_12_00_00"), ["m32_12", "m30_00"]);
        assert_eq!(maps(Game::Sekiro, "m31_05_00_00"), ["m31_05"]);
        assert!(maps(Game::Sekiro, "m6").is_empty());
        assert_eq!(
            draw_param_path("m11_01_0720"),
            "/param/drawparam/m11_01_0720.gparam.dcx"
        );
    }

    #[test]
    fn an_elden_ring_map_without_files_of_its_own_uses_its_familys_and_one_without_either_has_none() {
        let held = ["m30_00_0000", "m30_00_0100", "m30_01_0000", "m31_00_0000"];
        let has = |path: &str| held.iter().any(|stem| path == draw_param_path(stem));
        // the own stem has files: the family's are not added
        assert_eq!(
            draw_params_where(Game::EldenRing, "m30_01_00_00", has),
            ["m30_01_0000"]
        );
        // the own stem has none: the family's `m30_00`
        assert_eq!(
            draw_params_where(Game::EldenRing, "m30_05_00_00", has),
            ["m30_00_0000", "m30_00_0100"]
        );
        // the caves and tunnels have none of their own and use the catacombs'
        assert_eq!(
            draw_params_where(Game::EldenRing, "m32_00_00_00", has),
            ["m30_00_0000", "m30_00_0100"]
        );
        // neither: a missing family is not an error
        assert!(draw_params_where(Game::EldenRing, "m35_05_00_00", has).is_empty());
        // Sekiro has no family fallback
        assert!(draw_params_where(Game::Sekiro, "m30_05_00_00", has).is_empty());
    }
}
