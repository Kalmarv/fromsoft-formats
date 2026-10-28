//! The files an effect draws with: the textures (`sfx/tex/s00001_a.tpf`), mesh-particle models (`sfx/model/s20050.flver`) and
//! model animations (`sfx/hkx/s88670.anibnd`) that sit in the `sfxbnd` binders beside the effects, and Elden Ring's
//! resource lists (`sfx/ResourceList/f000000001.ffxreslist`), the text list of the files one effect needs.
//!
//! A texture is named by a number in a texture parameter of an action (a property's constant, stored as a float, or a
//! field word); the member that holds it is that number with a suffix that depends on what the texture is for. Sekiro's
//! members have no suffix for any kind (`s04012.tpf` is both an albedo and a normal map when an action asks for them),
//! Elden Ring's carry `_a` (albedo), `_n` (normal map), `_m` (the specular parameter map; `_3m` and `_1m` also occur) and `_d`
//! (the detail map). **Observed**: every texture id a constant property or field word names resolves in the installs but a
//! few (see the census in `tests/fxr_actions_census.rs`).

use super::{FORMAT, Result, invalid};
use crate::{Binder, Game};
use std::collections::BTreeMap;

/// What a texture parameter of an action is for (the fxr project's `textureType`: `a`, `n`, `3m`, `d`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TextureKind {
    /// The colour texture (`a`).
    Albedo,
    /// A normal map (`n`).
    Normal,
    /// The specular parameter map (`3m`).
    Specular,
    /// A detail map (`d`).
    Detail,
}

impl TextureKind {
    /// The kind the fxr project's `textureType` letters name.
    pub fn from_letter(letter: &str) -> Option<TextureKind> {
        Some(match letter {
            "a" => TextureKind::Albedo,
            "n" => TextureKind::Normal,
            "3m" => TextureKind::Specular,
            "d" => TextureKind::Detail,
            _ => return None,
        })
    }

    /// The suffix of the member name for this kind in `game`'s binders (without the underscore).
    pub fn suffix(self, game: Game) -> &'static str {
        match (game, self) {
            (Game::Sekiro, _) => "",
            (Game::EldenRing, TextureKind::Albedo) => "a",
            (Game::EldenRing, TextureKind::Normal) => "n",
            (Game::EldenRing, TextureKind::Specular) => "m",
            (Game::EldenRing, TextureKind::Detail) => "d",
        }
    }
}

/// What kind of file a parameter of an action refers to by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceKind {
    /// A texture of the kind.
    Texture(TextureKind),
    /// A model (`sfx/model/s%05d.flver`).
    Model,
    /// A model animation (`sfx/hkx/s%05d.anibnd`).
    Animation,
    /// A sound.
    Sound,
}

/// A parameter of a typed action that names a file by number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRef {
    /// What the file is.
    pub kind: ResourceKind,
    /// The parameter, under the fxr project's name.
    pub param: &'static str,
    /// The number; `None` when the parameter is a property that is not a constant (an animated texture number).
    pub id: Option<i32>,
}

/// The member name of texture `id` of `kind` in `game`: `s00001_a.tpf`.
pub fn texture_member(game: Game, kind: TextureKind, id: i32) -> String {
    match kind.suffix(game) {
        "" => format!("s{id:05}.tpf"),
        suffix => format!("s{id:05}_{suffix}.tpf"),
    }
}

/// The member name of mesh-particle model `id`: `s20050.flver`.
pub fn model_member(id: i32) -> String {
    format!("s{id:05}.flver")
}

/// The member name of model animation `id`: `s88670.anibnd`.
pub fn animation_member(id: i32) -> String {
    format!("s{id:05}.anibnd")
}

/// The textures, models and animations of one `sfxbnd` binder, by the number in their member names. The members are the
/// binder's own; the index holds positions into [`Binder::files`].
#[derive(Clone, Debug, Default)]
pub struct Resources {
    /// Texture members by (id, suffix without its underscore; empty for none).
    textures: BTreeMap<(i32, String), usize>,
    /// Model members by id.
    models: BTreeMap<i32, usize>,
    /// Animation members by id.
    animations: BTreeMap<i32, usize>,
    /// Resource lists by effect id.
    lists: BTreeMap<i32, usize>,
}

/// The leaf of a member name (`N:\...\sfx\tex\s00001_a.tpf` is `s00001_a.tpf`) and the folder it is in (`tex`).
fn leaf_and_folder(name: &str) -> Option<(&str, &str)> {
    let mut parts = name.rsplit(['\\', '/']);
    let leaf = parts.next()?;
    let folder = parts.next()?;
    Some((leaf, folder))
}

/// The number after the leading `s` of a member name and the text between it and the extension: `("00001", "_a")`.
fn split_stem(leaf: &str) -> Option<(i32, &str, &str)> {
    let (stem, extension) = leaf.rsplit_once('.')?;
    let rest = stem.strip_prefix(['s', 'S'])?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let id = rest.get(..digits)?.parse().ok()?;
    Some((id, rest.get(digits..)?, extension))
}

impl Resources {
    /// Indexes the members of `binder` that are textures, models, animations or resource lists. Other members (the
    /// effects themselves) are skipped; two members that would take the same key are an error.
    pub fn index(binder: &Binder) -> Result<Resources> {
        let mut r = Resources::default();
        for (i, file) in binder.files.iter().enumerate() {
            let Some(name) = file.name.as_deref() else {
                continue;
            };
            let Some((leaf, folder)) = leaf_and_folder(name) else {
                continue;
            };
            let folder = folder.to_ascii_lowercase();
            let lower = leaf.to_ascii_lowercase();
            let dup = |what: &str| invalid(FORMAT, 0, format!("two {what} members named {leaf}"));
            match (folder.as_str(), lower.rsplit_once('.').map(|(_, e)| e)) {
                ("tex", Some("tpf")) => {
                    let Some((id, suffix, _)) = split_stem(&lower) else {
                        continue;
                    };
                    let suffix = suffix.strip_prefix('_').unwrap_or(suffix).to_owned();
                    if r.textures.insert((id, suffix), i).is_some() {
                        return Err(dup("texture"));
                    }
                }
                ("model", Some("flver")) => {
                    let Some((id, _, _)) = split_stem(&lower) else {
                        continue;
                    };
                    if r.models.insert(id, i).is_some() {
                        return Err(dup("model"));
                    }
                }
                ("hkx", Some("anibnd")) => {
                    let Some((id, _, _)) = split_stem(&lower) else {
                        continue;
                    };
                    if r.animations.insert(id, i).is_some() {
                        return Err(dup("animation"));
                    }
                }
                ("resourcelist", Some("ffxreslist")) => {
                    let Some(id) = super::member_id(&lower.replace(".ffxreslist", ".fxr")) else {
                        continue;
                    };
                    if r.lists.insert(id, i).is_some() {
                        return Err(dup("resource list"));
                    }
                }
                _ => {}
            }
        }
        Ok(r)
    }

    /// The position in [`Binder::files`] of texture `id` of `kind` for `game`.
    pub fn texture(&self, game: Game, kind: TextureKind, id: i32) -> Option<usize> {
        self.textures.get(&(id, kind.suffix(game).to_owned())).copied()
    }

    /// Every texture member with its id and suffix.
    pub fn textures(&self) -> impl Iterator<Item = (i32, &str, usize)> {
        self.textures
            .iter()
            .map(|((id, suffix), &i)| (*id, suffix.as_str(), i))
    }

    /// The position of model `id`.
    pub fn model(&self, id: i32) -> Option<usize> {
        self.models.get(&id).copied()
    }

    /// The position of animation `id`.
    pub fn animation(&self, id: i32) -> Option<usize> {
        self.animations.get(&id).copied()
    }

    /// The position of effect `id`'s resource list.
    pub fn list(&self, id: i32) -> Option<usize> {
        self.lists.get(&id).copied()
    }

    /// How many textures, models, animations and resource lists the binder holds.
    pub fn counts(&self) -> [usize; 4] {
        [
            self.textures.len(),
            self.models.len(),
            self.animations.len(),
            self.lists.len(),
        ]
    }
}

/// A resource an effect's list names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listed {
    /// A texture: its number and suffix (`s00001_a.tif` is `(1, "a")`; no suffix is empty).
    Texture(i32, String),
    /// A model (`s83050.sib` names the model `s83050.flver`).
    Model(i32),
    /// A model animation (`s88670.hkt` names `s88670.anibnd`).
    Animation(i32),
    /// The skeleton of a model animation (`s82120_skeleton.hkt`: the skeleton inside `s82120.anibnd`).
    Skeleton(i32),
    /// A file named as an asset's (`AET999_645_a.tif`: the texture of a map asset, which an effect shares), by its name.
    Asset(String),
}

/// Elden Ring's resource list of an effect (`f000000001.ffxreslist`): a text of file names, one a line, the way the effect's
/// authoring tool refers to the files (`.tif` for a texture, `.sib` for a model, `.hkt` for an animation and `_skeleton.hkt` for its skeleton). A line that is
/// none of these is an error.
pub fn read_resource_list(bytes: &[u8]) -> Result<Vec<Listed>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| invalid(FORMAT, 0, format!("resource list is not UTF-8: {e}")))?;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let bad = || invalid(FORMAT, 0, format!("resource list line {line:?}"));
        let lower = line.to_ascii_lowercase();
        let Some((id, suffix, extension)) = split_stem(&lower) else {
            // a map asset's texture or model: `AET999_645_a.tif`, `AEG999_645.sib`
            if lower.starts_with("aet") || lower.starts_with("aeg") {
                out.push(Listed::Asset(line.to_owned()));
                continue;
            }
            return Err(bad());
        };
        out.push(match extension {
            "tif" => Listed::Texture(id, suffix.strip_prefix('_').unwrap_or(suffix).to_owned()),
            "sib" if suffix.is_empty() => Listed::Model(id),
            "hkt" if suffix.is_empty() => Listed::Animation(id),
            "hkt" if suffix == "_skeleton" => Listed::Skeleton(id),
            _ => return Err(bad()),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_names_follow_the_games_suffixes() {
        assert_eq!(
            texture_member(Game::EldenRing, TextureKind::Albedo, 1),
            "s00001_a.tpf"
        );
        assert_eq!(
            texture_member(Game::EldenRing, TextureKind::Specular, 31411),
            "s31411_m.tpf"
        );
        assert_eq!(
            texture_member(Game::Sekiro, TextureKind::Normal, 4012),
            "s04012.tpf"
        );
        assert_eq!(model_member(83050), "s83050.flver");
        assert_eq!(animation_member(88670), "s88670.anibnd");
    }

    #[test]
    fn resource_lists_read_by_extension() {
        let list = read_resource_list(
            b"s00001_a.tif\r\ns83050.sib\r\n\r\ns88670.hkt\r\ns60012_n.tif\r\ns82120_skeleton.hkt\r\n",
        )
        .unwrap();
        assert_eq!(
            list,
            [
                Listed::Texture(1, "a".into()),
                Listed::Model(83050),
                Listed::Animation(88670),
                Listed::Texture(60012, "n".into()),
                Listed::Skeleton(82120)
            ]
        );
        assert!(read_resource_list(b"s00001_a.png\r\n").is_err());
        assert!(read_resource_list(b"x.tif\r\n").is_err());
        assert_eq!(
            read_resource_list(b"AET999_645_a.tif\r\n").unwrap(),
            [Listed::Asset("AET999_645_a.tif".into())]
        );
    }
}
