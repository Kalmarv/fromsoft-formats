//! Archive path classification and BHD/name-dictionary joins. No asset or variant is chosen.
use std::{
    collections::BTreeMap,
    path::PathBuf,
};

use crate::NameDictionary;
use crate::archive::IndexedEntry;
use serde::Serialize;

/// The kind named by an archive member's stored path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// A character binder.
    Character,
    /// An equipment-part binder.
    Part,
    /// A map's MSB (`/map/mapstudio/m11_01_00_00.msb.dcx`), named by its stem.
    Area,
}

/// A dictionary path whose hash exists in the archive index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ListingEntry {
    /// Stable identity and archive path (`/chr/c1010.chrbnd`, `/parts/bd_m_9120_l.partsbnd`).
    pub key: String,
    /// Kind parsed from the path.
    pub kind: AssetKind,
    /// The file stem the game names it by (`c1010`, `bd_m_9120_l`).
    pub name: String,
    /// Stored dictionary path.
    pub path: String,
    /// Path hash in the BHD index.
    pub hash: u64,
    /// Archive shard index.
    pub shard: u8,
    /// Logical size from the index; zero means unstated.
    pub logical_size: usize,
    /// Stored padded size from the index.
    pub padded_size: usize,
}

/// Named archive members, without reading binders, geometry or textures.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Listing {
    /// Install identity supplied by the caller.
    pub install: PathBuf,
    /// The dictionary used, when loaded from a file; `None` for an in-memory dictionary.
    pub dictionary: Option<PathBuf>,
    /// Number of character binders.
    pub characters: usize,
    /// Number of part binders.
    pub parts: usize,
    /// Sorted by kind, then name, then path.
    pub entries: Vec<ListingEntry>,
}

impl Listing {
    /// Build from BHD entries and a dictionary. A name not in the entries is ignored; each matching candidate name
    /// for one hash is listed.
    pub fn from_entries(
        install: PathBuf,
        dictionary: Option<PathBuf>,
        entries: &BTreeMap<u64, IndexedEntry>,
        names: &NameDictionary,
    ) -> Self {
        let mut catalog = Listing {
            install,
            dictionary,
            ..Listing::default()
        };
        for (&hash, indexed) in entries {
            let Some(candidates) = names.names.get(&hash) else {
                continue;
            };
            for path in candidates {
                let Some((kind, name)) = classify(path) else {
                    continue;
                };
                catalog.entries.push(ListingEntry {
                    key: path.clone(),
                    kind,
                    name: name.to_owned(),
                    path: path.clone(),
                    hash,
                    shard: indexed.shard,
                    logical_size: indexed.entry.logical_size,
                    padded_size: indexed.entry.padded_size,
                });
            }
        }
        catalog
            .entries
            .sort_by(|a, b| (a.kind, &a.name, &a.path).cmp(&(b.kind, &b.name, &b.path)));
        catalog.characters = catalog
            .entries
            .iter()
            .filter(|entry| entry.kind == AssetKind::Character)
            .count();
        catalog.parts = catalog
            .entries
            .iter()
            .filter(|entry| entry.kind == AssetKind::Part)
            .count();
        catalog
    }
}

/// A binder path's asset kind and stem, or `None` for any other archive member.
pub fn classify(path: &str) -> Option<(AssetKind, &str)> {
    // Sekiro's dictionary and BHD hashes name the stored envelope (`....chrbnd.dcx`); the reader's key omits the
    // envelope, so the catalog accepts both spellings and keeps the archive path as the stable key.
    let stem = path.strip_suffix(".dcx").unwrap_or(path);
    if let Some(rest) = stem.strip_prefix("/chr/") {
        let name = rest.strip_suffix(".chrbnd")?;
        if !name.is_empty() && !name.contains('/') {
            return Some((AssetKind::Character, name));
        }
    }
    if let Some(rest) = stem.strip_prefix("/parts/") {
        let name = rest.strip_suffix(".partsbnd")?;
        if !name.is_empty() && !name.contains('/') {
            return Some((AssetKind::Part, name));
        }
    }
    if let Some(rest) = stem.strip_prefix("/map/mapstudio/") {
        let name = rest.strip_suffix(".msb")?;
        if !name.is_empty() && !name.contains('/') {
            return Some((AssetKind::Area, name));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bhd::Entry;

    fn entry(hash: u64, logical: usize) -> (u64, IndexedEntry) {
        (
            hash,
            IndexedEntry {
                shard: 1,
                entry: Entry {
                    hash,
                    offset: 0,
                    padded_size: logical.next_multiple_of(16),
                    logical_size: logical,
                    aes: None,
                    sha: None,
                    index_offset: 0,
                },
            },
        )
    }

    #[test]
    fn only_named_character_and_part_binders_in_the_archive_are_listed() {
        let names = NameDictionary::parse(
            "# comment\n/chr/c1010.chrbnd.dcx\n/parts/bd_m_9120_l.partsbnd.dcx\n/map/m11_01.msb\n/chr/not_in_archive.chrbnd.dcx\n",
        )
        .expect("dictionary");
        // Hashes are what the dictionary computed; recompute from paths rather than relying on literals.
        let entries: BTreeMap<u64, IndexedEntry> = [
            ("/chr/c1010.chrbnd.dcx", 100usize),
            ("/parts/bd_m_9120_l.partsbnd.dcx", 200),
            ("/map/m11_01.msb", 300),
        ]
        .into_iter()
        .map(|(path, size)| entry(u64::from(crate::archive::path_hash(path)), size))
        .collect();
        let catalog = Listing::from_entries(PathBuf::from("install"), None, &entries, &names);
        assert_eq!(catalog.characters, 1);
        assert_eq!(catalog.parts, 1);
        assert_eq!(catalog.entries.len(), 2);
        assert_eq!(catalog.entries[0].kind, AssetKind::Character);
        assert_eq!(catalog.entries[0].name, "c1010");
        assert_eq!(catalog.entries[0].key, "/chr/c1010.chrbnd.dcx");
        assert_eq!(catalog.entries[1].kind, AssetKind::Part);
        assert_eq!(catalog.entries[1].logical_size, 200);
        assert!(
            catalog
                .entries
                .iter()
                .all(|entry| entry.path != "/map/m11_01.msb")
        );
    }

    #[test]
    fn entries_are_listed_in_stable_name_order() {
        let names = NameDictionary::parse("/chr/c1010.chrbnd.dcx\n/chr/c1011.chrbnd.dcx\n")
            .expect("names");
        // Both names hash to the same key only in a real collision; build a dictionary with one shared hash by
        // reusing the same normalized path twice is not allowed. Verify the stable order instead.
        let entries = BTreeMap::from([
            entry(
                u64::from(crate::archive::path_hash(
                    "/chr/c1010.chrbnd.dcx",
                )),
                10,
            ),
            entry(
                u64::from(crate::archive::path_hash(
                    "/chr/c1011.chrbnd.dcx",
                )),
                20,
            ),
        ]);
        let catalog = Listing::from_entries(PathBuf::from("install"), None, &entries, &names);
        assert_eq!(catalog.entries.len(), 2);
        assert_eq!(catalog.entries[0].name, "c1010");
        assert_eq!(catalog.entries[1].name, "c1011");
    }

    #[test]
    fn classify_rejects_other_members_and_nested_paths() {
        assert_eq!(
            classify("/chr/c1010.chrbnd"),
            Some((AssetKind::Character, "c1010"))
        );
        assert_eq!(
            classify("/chr/c1010.chrbnd.dcx"),
            Some((AssetKind::Character, "c1010"))
        );
        assert_eq!(
            classify("/parts/bd_m_9120_l.partsbnd.dcx"),
            Some((AssetKind::Part, "bd_m_9120_l"))
        );
        assert_eq!(
            classify("/parts/bd_m_9120_l.partsbnd"),
            Some((AssetKind::Part, "bd_m_9120_l"))
        );
        assert_eq!(
            classify("/map/mapstudio/m11_01_00_00.msb.dcx"),
            Some((AssetKind::Area, "m11_01_00_00"))
        );
        assert_eq!(classify("/map/mapstudio/m11_01_00_00.msb.dcx.bak"), None);
        assert_eq!(classify("/chr/nested/c1010.chrbnd"), None);
        assert_eq!(classify("/parts/bd_m_9120_l.tpf"), None);
        assert_eq!(classify("/map/m11_01.msb"), None);
    }
}
