//! The game's params, read as stored ([`crate::param::Param`]) from where the game keeps them
//! ([`crate::Game::params`]: Sekiro's `param/gameparam/gameparam.parambnd.dcx`, Elden Ring's `regulation.bin`), and
//! the map parts that name an NPC param row: which rows point at which character model. An Elden Ring install needs its
//! `regulation.key` in the [`crate::Keys`] (the caller supplies it).
#![allow(
    clippy::disallowed_methods,
    reason = "reader I/O: reads the game's regulation file beside its executable"
)]
use aes::{
    Aes256,
    cipher::{Array, BlockCipherDecrypt, KeyInit},
};

use crate::game::ParamSource;
use crate::msb::{MapLayout, PartKind};
use crate::param::{NpcParam, Param};
use crate::{Archive, Binder, Install, NameDictionary, Result, dcx::Decoder, invalid};

const FORMAT: &str = "FromSoftware params";

/// `data` decrypted with AES-256 in CBC mode under `key`, its first 16 bytes the initialisation vector. The rest must
/// be whole blocks (the game's files are); no padding is removed (the DCX envelope inside states its own size).
pub(crate) fn decrypt_cbc(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>> {
    let (iv, body) = data
        .split_first_chunk::<16>()
        .ok_or_else(|| invalid(FORMAT, 0, "shorter than its initialisation vector"))?;
    let (blocks, rest) = body.as_chunks::<16>();
    if !rest.is_empty() {
        return Err(invalid(
            FORMAT,
            16,
            format!("{} bytes are not whole AES blocks", body.len()),
        ));
    }
    let cipher = Aes256::new(&Array::from(*key));
    let mut previous = *iv;
    let mut out = Vec::with_capacity(body.len());
    for block in blocks {
        let mut b = Array::from(*block);
        cipher.decrypt_block(&mut b);
        out.extend(b.iter().zip(previous).map(|(x, p)| x ^ p));
        previous = *block;
    }
    Ok(out)
}

/// The param binder inside the bytes of a `regulation.bin`, decrypted with the regulation `key` (a regulation file
/// of any version of the game, not only the installed one).
pub fn regulation_binder(key: &[u8; 32], bytes: &[u8], decoder: &Decoder<'_>) -> Result<Binder> {
    Binder::read(&decoder.decode_if_dcx(&decrypt_cbc(key, bytes)?)?, decoder)
}

/// The game's param binder, from where `install`'s game keeps it: every param file of the game, as stored (a member is
/// read with [`Param::read`]).
pub fn param_binder(install: &Install, archive: &Archive, decoder: &Decoder<'_>) -> Result<Binder> {
    match install.game.params() {
        ParamSource::Archived(path) => {
            if !archive.contains(path) {
                return Err(invalid(FORMAT, 0, format!("{path} is not in the archives")));
            }
            Binder::read(&archive.read(path, decoder)?, decoder)
        }
        ParamSource::Regulation { file, key_file } => {
            let key = install.keys.regulation().ok_or_else(|| {
                invalid(
                    FORMAT,
                    0,
                    format!(
                        "{} keeps its params in {file}, whose key the keys folder did not supply ({key_file})",
                        install.game.name()
                    ),
                )
            })?;
            let path = install.root.join(file);
            let bytes =
                std::fs::read(&path).map_err(|e| invalid(FORMAT, 0, format!("{}: {e}", path.display())))?;
            regulation_binder(key, &bytes, decoder)
        }
    }
}

/// The param file `leaf` (`NpcParam.param`) of the install's game params, strictly read.
pub fn read_param(install: &Install, archive: &Archive, decoder: &Decoder<'_>, leaf: &str) -> Result<Param> {
    let binder = param_binder(install, archive, decoder)?;
    let file = binder
        .member(leaf)
        .ok_or_else(|| invalid(FORMAT, 0, format!("the param binder holds no {leaf}")))?;
    Param::read(&file.bytes)
}

/// The NPC param of the install's game params: every row's id and display mask bits, with no row chosen for any
/// model.
pub fn read_npc_param(install: &Install, archive: &Archive, decoder: &Decoder<'_>) -> Result<NpcParam> {
    NpcParam::from_param(
        &read_param(install, archive, decoder, "NpcParam.param")?,
        &install.game.npc_param(),
    )
}

/// The model effect param of the install's game params (Sekiro's `ModelSfxParam`, Elden Ring's `AssetModelSfxParam`):
/// the effects a placed object or asset spawns, and the dummy polygons they attach to.
pub fn read_model_sfx_param(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<crate::param::ModelSfxParam> {
    let leaf = match install.game {
        crate::Game::Sekiro => "ModelSfxParam.param",
        crate::Game::EldenRing => "AssetModelSfxParam.param",
    };
    crate::param::ModelSfxParam::from_param(&read_param(install, archive, decoder, leaf)?)
}

/// Elden Ring's weather asset replace param: which assets of each sky map each weather shows. Refused for a game that
/// has no such param (Sekiro: its param binder holds none).
pub fn read_weather_assets(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<crate::weather_assets::WeatherAssetParam> {
    crate::weather_assets::WeatherAssetParam::from_param(&read_param(
        install,
        archive,
        decoder,
        "WeatherAssetReplaceParam.param",
    )?)
}

/// Elden Ring's weather asset create param: the assets a weather change or the time spent in a weather creates.
/// Refused for a game that has no such param (Sekiro).
pub fn read_weather_asset_creates(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<crate::weather_assets::WeatherAssetCreateParam> {
    crate::weather_assets::WeatherAssetCreateParam::from_param(&read_param(
        install,
        archive,
        decoder,
        "WeatherAssetCreateParam.param",
    )?)
}

/// Elden Ring's weather asset creation limit of each map (`MapDefaultInfoParam`), which says which of the weather
/// asset rows with limits apply on it. Refused for a game that has no such param (Sekiro).
pub fn read_weather_asset_limits(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<crate::weather_assets::WeatherAssetLimits> {
    crate::weather_assets::WeatherAssetLimits::from_param(&read_param(
        install,
        archive,
        decoder,
        "MapDefaultInfoParam.param",
    )?)
}

/// The descriptive NPC fields, with the game's explicit layout and no row choice.
pub fn read_npc_stats(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<Vec<crate::param::NpcStats>> {
    crate::param::NpcStats::from_param(
        &read_param(install, archive, decoder, "NpcParam.param")?,
        &install.game.npc_stats(),
    )
}

/// The legacy maps' places in the overworld ([`crate::overworld::Placements`]): the install's `WorldMapLegacyConvParam`
/// read and put through the placement rules. Refused for a game that has no such param (Sekiro).
pub fn read_legacy_placements(
    install: &Install,
    archive: &Archive,
    decoder: &Decoder<'_>,
) -> Result<crate::overworld::Placements> {
    let layout = install.game.legacy_conv().ok_or_else(|| {
        invalid(
            FORMAT,
            0,
            format!("{} has no WorldMapLegacyConvParam", install.game.name()),
        )
    })?;
    let param = read_param(install, archive, decoder, "WorldMapLegacyConvParam.param")?;
    Ok(crate::overworld::Placements::from_conv(
        &crate::overworld::ConvParam::from_param(&param, &layout)?,
    ))
}

/// A map part that names an NPC param row: an Enemy or DummyEnemy part, its model and its `NPCParamID`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct NpcPlacement {
    /// The map (`m10_00_00_00`).
    pub map: String,
    /// The part's name in the map.
    pub part: String,
    /// The part's model name (`c3000`).
    pub model: String,
    /// Whether the part is a DummyEnemy rather than an Enemy.
    pub dummy: bool,
    /// The row it names, as stored (0 or -1 where it names none).
    pub row: i32,
    /// Equipment/appearance row, independently stored by the MSB enemy, never inferred from `row`.
    pub character_init: Option<i32>,
}

/// Every Enemy and DummyEnemy part of the maps `dictionary` names that the archives hold
/// (`/map/mapstudio/<map>.msb.dcx`), in map then file order: which NPC rows point at which model. The archives index
/// files by a hash of their path and keep no names, so the maps read are the ones the dictionary names (a community path
/// list such as UXM's `SekiroDictionary.txt` or `EldenRingDictionary.txt`); a map it does not name is not read.
pub fn npc_placements(
    archive: &Archive,
    decoder: &Decoder<'_>,
    dictionary: &NameDictionary,
) -> Result<Vec<NpcPlacement>> {
    let mut maps: Vec<&str> = dictionary
        .names
        .values()
        .flatten()
        .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
        .filter(|m| !m.contains('/'))
        .collect();
    maps.sort_unstable();
    maps.dedup();
    let mut out = Vec::new();
    for map in maps {
        let path = format!("/map/mapstudio/{map}.msb.dcx");
        if !archive.contains(&path) {
            continue;
        }
        let layout = MapLayout::read(&archive.read(&path, decoder)?)?;
        for p in &layout.parts {
            let dummy = match p.kind {
                PartKind::Enemy => false,
                PartKind::DummyEnemy => true,
                _ => continue,
            };
            out.push(NpcPlacement {
                map: map.to_owned(),
                part: p.name.clone(),
                model: match p.model {
                    Some(m) => layout
                        .models
                        .get(m)
                        .map(|model| model.name.clone())
                        .ok_or_else(|| {
                            invalid(
                                "MSB",
                                0,
                                format!("part {} names model {m} of {}", p.name, layout.models.len()),
                            )
                        })?,
                    None => String::new(),
                },
                dummy,
                row: p.npc_param.unwrap_or(-1),
                character_init: layout.character_init(p.index)?,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cbc_takes_the_first_block_as_the_vector() {
        use aes::cipher::BlockCipherEncrypt;
        let key = [7u8; 32];
        let iv = [3u8; 16];
        let plain: Vec<u8> = (0..48).collect();
        // encrypt: each block XORed with the previous ciphertext (the vector first), then enciphered
        let cipher = Aes256::new(&Array::from(key));
        let mut data = iv.to_vec();
        let mut previous = iv;
        for block in plain.as_chunks::<16>().0 {
            let mut b = Array::from(std::array::from_fn::<u8, 16, _>(|i| block[i] ^ previous[i]));
            cipher.encrypt_block(&mut b);
            previous = b.into();
            data.extend(previous);
        }
        assert_eq!(decrypt_cbc(&key, &data).unwrap(), plain);
        assert!(decrypt_cbc(&key, &data[..20]).is_err());
        assert!(decrypt_cbc(&key, &data[..8]).is_err());
    }
}
