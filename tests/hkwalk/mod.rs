//! The Havok tagfiles of an install's collision and character binders, handed to a visitor on a few threads: the walk
//! the install-wide collision tests share (`bvtree_census.rs` has its own, older copy).
//!
//! The binders are those the game's name dictionary names, as in the census that found the rules: the map `h` and `l`
//! binders (split `.hkxbhd`/`.hkxbdt` pairs) and Sekiro's `obj/*.objbnd` or Elden Ring's `asset/aeg/*.geomhkxbnd` for the
//! compressed-mesh collision ([`Set::Mesh`]), the map `f` binders for the extern meshes ([`Set::Extern`]), and the
//! `chr/*.chrbnd` and `parts/*.partsbnd` binders for characters ([`Set::Characters`]: ragdolls and cloth).
#![allow(dead_code, reason = "each test uses part of the walker")]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::{Archive, Binder, Game, NameDictionary};
use havok_formats::{Compendium, Tagfile};

use crate::common;
use crate::inputs::{Game as Data, OracleSet, Tool, oodle, oracle};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Which kind of binder a path is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Set {
    /// Map `h` and `l` binders, Sekiro objects, Elden Ring assets: the compressed meshes.
    Mesh,
    /// Map `f` binders: extern meshes.
    Extern,
    /// Character and parts binders.
    Characters,
}

/// The set of binder `path`, and whether it is a split map binder.
pub fn set_of(game: Game, path: &str) -> Option<(Set, bool)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let map = path.starts_with("/map/") && path.ends_with(".hkxbhd");
    if map && (name.starts_with('h') || name.starts_with('l')) {
        return Some((Set::Mesh, true));
    }
    if map && name.starts_with('f') {
        return Some((Set::Extern, true));
    }
    let object = match game {
        Game::Sekiro => path.starts_with("/obj/") && path.ends_with(".objbnd.dcx"),
        Game::EldenRing => path.starts_with("/asset/aeg/") && path.ends_with(".geomhkxbnd.dcx"),
    };
    if object {
        return Some((Set::Mesh, false));
    }
    let character = (path.starts_with("/chr/") && path.ends_with(".chrbnd.dcx"))
        || (path.starts_with("/parts/") && path.ends_with(".partsbnd.dcx"));
    character.then_some((Set::Characters, false))
}

/// One tagfile of a binder.
pub struct Member<'a> {
    /// The binder's path in the archive.
    pub binder: &'a str,
    /// The member's name (lower case).
    pub name: &'a str,
    /// The tagfile's bytes, out of their DCX envelope.
    pub bytes: &'a [u8],
    /// The type compendium the file reads with (none: it has its own types).
    pub compendium: Option<&'a Compendium>,
    /// The file.
    pub tag: &'a Tagfile<'a>,
}

/// An install's archive and the binders to walk.
pub struct Install {
    pub game: Game,
    archive: Archive,
    dll: primase_foundations::compress::oodle::Oodle,
    binders: Vec<(String, bool)>,
}

/// The configured install of `game` and its binders of the `wanted` sets, or `None` when the configuration lacks the
/// archive, the Oodle library or the name dictionary.
pub fn open(game: Game, wanted: &[Set]) -> Result<Option<Install>> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (Some(archive), Some(dll), Some(dictionary)) = (
        common::archive(game).map_err(|e| e.to_string())?,
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll).map_err(|e| e.to_string())?;
    let names = NameDictionary::read_for(dictionary, game).map_err(|e| e.to_string())?;
    let mut binders: Vec<(String, bool)> = names
        .names
        .iter()
        .filter(|(hash, _)| archive.entries.contains_key(*hash))
        .flat_map(|(_, paths)| paths)
        .filter_map(|p| {
            set_of(game, p)
                .filter(|(set, _)| wanted.contains(set))
                .map(|(_, map)| (p.clone(), map))
        })
        .collect();
    binders.sort();
    binders.dedup();
    Ok(Some(Install {
        game,
        archive,
        dll,
        binders,
    }))
}

impl Install {
    /// How many binders there are.
    pub fn binder_count(&self) -> usize {
        self.binders.len()
    }

    /// Call `visit` on every tagfile of every binder, on up to eight threads; each thread folds into its own state,
    /// `merge` adds them up. A binder or a visit that fails is collected, and the failures are returned as one error.
    pub fn run<S: Default + Send>(
        &self,
        visit: impl Fn(&mut S, &Member<'_>) -> Result<()> + Sync,
        merge: impl Fn(&mut S, S),
    ) -> Result<S> {
        let next = AtomicUsize::new(0);
        let failures = Mutex::new(Vec::new());
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
        let states: Vec<S> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let decoder = Decoder {
                            oodle: Some(&self.dll),
                            max_output: 1 << 30,
                        };
                        let mut state = S::default();
                        loop {
                            let i = next.fetch_add(1, Ordering::Relaxed);
                            let Some((path, map)) = self.binders.get(i) else {
                                break;
                            };
                            if let Err(e) = self.one(path, *map, &decoder, &mut state, &visit) {
                                if let Ok(mut f) = failures.lock() {
                                    f.push(format!("{path}: {e}"));
                                }
                            }
                        }
                        state
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .expect("a walker thread panicked: its counts would be lost")
                })
                .collect()
        });
        let failures = failures.into_inner().unwrap_or_default();
        if !failures.is_empty() {
            return Err(format!("{} binders failed: {failures:#?}", failures.len()).into());
        }
        let mut total = S::default();
        for s in states {
            merge(&mut total, s);
        }
        Ok(total)
    }

    fn one<S>(
        &self,
        path: &str,
        map: bool,
        decoder: &Decoder<'_>,
        state: &mut S,
        visit: &impl Fn(&mut S, &Member<'_>) -> Result<()>,
    ) -> Result<()> {
        let header = self.archive.read(path, decoder)?;
        let binder = if map {
            let payload = self.archive.read(&path.replace(".hkxbhd", ".hkxbdt"), decoder)?;
            Binder::read_split(&header, &payload, decoder)?
        } else {
            Binder::read(&header, decoder)?
        };
        let mut compendia = Vec::new();
        let mut members = Vec::new();
        for f in &binder.files {
            let name = f.name.as_deref().unwrap_or("").to_ascii_lowercase();
            let hkx = name.ends_with(".hkx") || name.ends_with(".hkx.dcx");
            if !hkx && !name.ends_with(".compendium") && !name.ends_with(".compendium.dcx") {
                continue;
            }
            let bytes = decoder.decode_if_dcx(&f.bytes)?;
            if bytes.get(4..8) == Some(b"TCM0") {
                compendia.push(Compendium::read(&bytes)?);
            } else if hkx && bytes.get(4..8) == Some(b"TAG0") {
                members.push((name, bytes));
            }
        }
        for (name, bytes) in &members {
            // the member's own types, or the first compendium that has them
            let compendium = std::iter::once(None)
                .chain(compendia.iter().map(Some))
                .find(|c| Tagfile::read(bytes, *c).is_ok())
                .flatten();
            let tag = Tagfile::read(bytes, compendium).map_err(|e| format!("{name}: {e}"))?;
            visit(
                state,
                &Member {
                    binder: path,
                    name,
                    bytes,
                    compendium,
                    tag: &tag,
                },
            )
            .map_err(|e| format!("{name}: {e}"))?;
        }
        Ok(())
    }
}

/// Every shape of every physics body of a tagfile (the `hknpPhysicsSceneData` variants of its root).
pub fn shapes<'t, 'a>(tag: &'t Tagfile<'a>) -> havok_formats::Result<Vec<havok_formats::Value<'t, 'a>>> {
    let mut out = Vec::new();
    let variants = tag.root()?.field("namedVariants")?.array()?;
    for i in 0..variants.len() {
        let physics = variants.get(i)?.field("variant")?.dereference()?;
        if physics.type_name() != "hknpPhysicsSceneData" {
            continue;
        }
        let systems = physics.field("systemDatas")?.array()?;
        for j in 0..systems.len() {
            let bodies = systems.get(j)?.dereference()?.field("bodyCinfos")?.array()?;
            for k in 0..bodies.len() {
                out.push(bodies.get(k)?.field("shape")?.dereference()?);
            }
        }
    }
    Ok(out)
}

/// Whether `class` is a compressed mesh shape of the games.
pub fn is_compressed_mesh(class: &str) -> bool {
    matches!(
        class,
        "fsnpCustomParamCompressedMeshShape" | "hknpCompressedMeshShape"
    )
}
