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
/// open-world tiles share `m60_00` and `m61_00` (no file carries a tile's coordinates), and an Elden Ring map with
/// no file of its own stem uses its family's `mAA_00` (m30's dungeons, m19, one m11 map). Empty for a name that is not
/// an area's.
fn draw_param_maps(game: Game, area: &str) -> Vec<String> {
    let (Some(first), Some(second)) = (area.get(..3), area.get(4..6)) else {
        return Vec::new();
    };
    match (game, first) {
        (Game::EldenRing, "m60" | "m61") => vec![format!("{first}_00")],
        (Game::EldenRing, _) if second != "00" => vec![format!("{first}_{second}"), format!("{first}_00")],
        _ => vec![format!("{first}_{second}")],
    }
}

/// The archive path of the draw-param file `stem` (`m11_01_0720`).
pub fn draw_param_path(stem: &str) -> String {
    format!("/param/drawparam/{stem}.gparam.dcx")
}

/// The draw-param files the archive holds for `area`'s map, by stem, in ID order: every `<map>_<0000..9999>` of the first
/// of its [`draw_param_maps`] that has any. A map none of whose stems has a file (Elden Ring's m31 and m32) has none.
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
        // neither (m31's files are not drawn from m31_05's family, and a missing family is not an error)
        assert!(draw_params_where(Game::EldenRing, "m32_00_00_00", has).is_empty());
        // Sekiro has no family fallback
        assert!(draw_params_where(Game::Sekiro, "m30_05_00_00", has).is_empty());
    }
}
