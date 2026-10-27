//! A game install's BHD5/BDT archives: [`Install`] and [`Keys`] say where they are and how to open them, [`Archive`]
//! indexes them and reads entries by archive path, and [`NameDictionary`] maps path hashes back to paths from a path
//! list the caller supplies. With [`game_params`](crate::game_params) (which reads Elden Ring's `regulation.bin`), this is
//! the only module that reads files from disk.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reader I/O: reads the install's archives"
)]
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation
    )
)]
pub use crate::bhd::{PathHash, path_hash, path_hash64};
use crate::{
    Game, Result,
    bhd::{Entry, Header, RecordLayout, unwrap_rsa_with},
    dcx::Decoder,
    game::ArchivePair,
    invalid,
};
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    num::NonZeroUsize,
    path::{Path, PathBuf},
};

/// Caller-provided UTF-8 dictionary, one path per line; blank/comment lines ignored. Paths are keyed by a game's
/// path hash. Hash collisions retain every distinct normalized candidate, never overwrite.
#[derive(Debug, Default, Clone)]
pub struct NameDictionary {
    /// The paths of each path hash, normalised (lowercase, `/` separators, a leading `/`) and sorted.
    pub names: BTreeMap<u64, Vec<String>>,
}
impl NameDictionary {
    /// A dictionary of Sekiro paths.
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        Self::read_for(path, Game::Sekiro)
    }
    /// A dictionary of `game`'s paths, keyed by its path hash.
    pub fn read_for(path: impl AsRef<Path>, game: Game) -> Result<Self> {
        Self::parse_for(&std::fs::read_to_string(path)?, game)
    }
    /// Sekiro paths from text.
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_for(text, Game::Sekiro)
    }
    /// `game`'s paths from text. A non-ASCII path is refused; the error's offset is the byte offset in `text` of its first non-ASCII byte.
    pub fn parse_for(text: &str, game: Game) -> Result<Self> {
        let hash = game.archives().hash;
        let mut names: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        let mut next_line = 0usize;
        for raw in text.split_inclusive('\n') {
            let line_at = next_line;
            next_line = next_line.saturating_add(raw.len());
            let line = raw.trim_end_matches(['\n', '\r']);
            let trimmed = line.trim();
            let s = trimmed.trim_start_matches('\u{feff}');
            if s.is_empty() || s.starts_with('#') {
                continue;
            }
            if let Some(bad) = s.bytes().position(|b| !b.is_ascii()) {
                // `s` is `line` less its edges: the bytes cut from the front come before the first bad byte
                let lead = (line.len().saturating_sub(line.trim_start().len()))
                    .saturating_add(trimmed.len().saturating_sub(s.len()));
                return Err(invalid(
                    "dictionary",
                    line_at.saturating_add(lead).saturating_add(bad),
                    format!("archive path {s:?} holds a non-ASCII byte; paths are ASCII"),
                ));
            }
            let mut s = crate::names::fold(s).replace('\\', "/");
            if !s.starts_with('/') {
                s.insert(0, '/');
            }
            let list = names.entry(hash.hash(&s)).or_default();
            if !list.contains(&s) {
                list.push(s);
                list.sort();
            }
        }
        Ok(Self { names })
    }
}

/// The RSA public keys of a game's archive pairs (big-endian modulus and exponent), by pair stem, and the key of its
/// encrypted params where the game has one ([`crate::game::ParamSource::Regulation`]).
#[derive(Clone)]
pub struct Keys {
    pairs: BTreeMap<&'static str, (Vec<u8>, Vec<u8>)>,
    regulation: Option<[u8; 32]>,
}
impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys")
            .field("pairs", &self.pairs.keys().collect::<Vec<_>>())
            .field("regulation", &self.regulation.is_some())
            .finish()
    }
}
impl Keys {
    /// No keys: an install opened with them fails at its first archive pair, naming it.
    pub fn none() -> Self {
        Self {
            pairs: BTreeMap::new(),
            regulation: None,
        }
    }
    /// The keys the caller supplies: a PKCS#1 `RSA PUBLIC KEY` PEM file per archive pair in `folder`, named after the
    /// last component of the pair's stem (`Data0.pem`, `sd.pem`), and, for a game whose params are encrypted, their key
    /// in the file the game names ([`crate::game::ParamSource::Regulation`]: 64 hexadecimal digits, refused unless its
    /// SHA-256 is `regulation_sha256` when that is given; with `None` it is not checked). A pair without a file has no
    /// key; opening an install that holds it then fails. Without the params' key file, reading the params fails,
    /// naming it.
    pub fn from_folder(
        folder: impl AsRef<Path>,
        game: Game,
        regulation_sha256: Option<&[u8; 32]>,
    ) -> Result<Self> {
        let folder = folder.as_ref();
        let mut pairs = BTreeMap::new();
        for pair in game.archives().pairs {
            let file = folder.join(format!("{}.pem", pair.name()));
            if file.is_file() {
                let key = crate::keys::pem_public_key(&std::fs::read_to_string(&file)?)
                    .map_err(|e| invalid("PEM", 0, format!("{}: {e}", file.display())))?;
                pairs.insert(pair.stem, key);
            }
        }
        let mut regulation = None;
        if let crate::game::ParamSource::Regulation { key_file, .. } = game.params() {
            let file = folder.join(key_file);
            if file.is_file() {
                let text = std::fs::read_to_string(&file)?;
                regulation = Some(
                    crate::keys::checked_key(text.trim(), regulation_sha256).map_err(|e| match e {
                        crate::Error::Invalid { offset, detail, .. } => {
                            invalid("key", offset, format!("{}: {detail}", file.display()))
                        }
                        other => other,
                    })?,
                );
            }
        }
        Ok(Self { pairs, regulation })
    }
    /// The key of the pair `stem`: its modulus and exponent.
    pub fn get(&self, stem: &str) -> Option<(&[u8], &[u8])> {
        self.pairs.get(stem).map(|(n, e)| (n.as_slice(), e.as_slice()))
    }
    /// The key of the game's encrypted params, when the keys folder supplied it.
    pub fn regulation(&self) -> Option<&[u8; 32]> {
        self.regulation.as_ref()
    }
}

/// A game's install: its folder, the game and the keys of its archives; what a reader opens.
#[derive(Clone, Debug)]
pub struct Install {
    /// The install folder holding the archives (and, for Elden Ring, `regulation.bin`).
    pub root: PathBuf,
    /// Which game it is.
    pub game: Game,
    /// The archive keys the caller supplies.
    pub keys: Keys,
    /// The threads opening its archives may use (the index files' RSA blocks): the caller states them; one by default.
    pub threads: NonZeroUsize,
}
impl Install {
    /// The install of `game` in the folder `root`, its archives opened with `keys` on one thread.
    pub fn new(root: impl Into<PathBuf>, game: Game, keys: Keys) -> Self {
        Self {
            root: root.into(),
            game,
            keys,
            threads: NonZeroUsize::MIN,
        }
    }
    /// The same install opened on `threads` ([`Install::threads`]).
    pub fn with_threads(mut self, threads: NonZeroUsize) -> Self {
        self.threads = threads;
        self
    }
    /// Its archives.
    pub fn open(&self) -> Result<Archive> {
        Archive::open_game_with(self.threads, &self.root, self.game, &self.keys)
    }
}

/// `header`, the index of `pair` whose BDT file is `bdt_size` bytes, added to `entries`. An entry that ends beyond the
/// BDT file, or whose hash another pair of `pairs` already lists, is refused at its record in the index (offsets are
/// those of the unwrapped index).
fn index_pair(
    entries: &mut BTreeMap<u64, IndexedEntry>,
    pairs: &[ArchivePair],
    pair: &ArchivePair,
    header: &Header,
    record: &RecordLayout,
    bdt_size: u64,
) -> Result<()> {
    for entry in &header.entries {
        let padded = u64::try_from(entry.padded_size).ok();
        if padded
            .and_then(|padded| entry.offset.checked_add(padded))
            .is_none_or(|end| end > bdt_size)
        {
            return Err(invalid(
                "BHD5",
                entry.index_offset.saturating_add(record.offset),
                format!(
                    "{}.bdt entry {:#x} at {:#x} of {} bytes ends beyond the {bdt_size} bytes of the file",
                    pair.stem, entry.hash, entry.offset, entry.padded_size
                ),
            ));
        }
        let indexed = IndexedEntry {
            shard: pair.shard,
            entry: entry.clone(),
        };
        if let Some(first) = entries.insert(entry.hash, indexed) {
            let first_pair = pairs
                .iter()
                .find(|p| p.shard == first.shard)
                .map_or("an earlier pair", |p| p.stem);
            return Err(invalid(
                "BHD5",
                entry.index_offset,
                format!(
                    "path hash {:#x} is listed by {} and by {first_pair}; each hash names one entry across the archives",
                    entry.hash, pair.stem
                ),
            ));
        }
    }
    Ok(())
}
/// An index entry and the archive pair it came from.
#[derive(Debug)]
pub struct IndexedEntry {
    /// The [`ArchivePair::shard`] of the pair holding the entry.
    pub shard: u8,
    /// The entry as the pair's BHD5 index lists it.
    pub entry: Entry,
}
/// A game's immutable archive indexes. BDT files are opened for each read, so reads need no shared seek cursor. Names
/// and Oodle loading remain caller-controlled.
#[derive(Debug)]
pub struct Archive {
    root: PathBuf,
    game: Game,
    /// Every entry of every pair, by path hash.
    pub entries: BTreeMap<u64, IndexedEntry>,
    /// The indexes of the pairs read, in the game's pair order.
    pub headers: Vec<Header>,
    /// Optional pairs the install does not hold (downloadable content not installed).
    pub absent: Vec<&'static str>,
}
impl Archive {
    /// An archive of `game` that holds nothing: for a reader that must read none of it.
    pub fn empty(game: Game) -> Self {
        Self {
            root: PathBuf::new(),
            game,
            entries: BTreeMap::new(),
            headers: Vec::new(),
            absent: Vec::new(),
        }
    }
    /// `game`'s archives under `root`, each index unwrapped with its pair's key in `keys`. An entry hash present in
    /// two pairs is an error.
    pub fn open_game(root: impl AsRef<Path>, game: Game, keys: &Keys) -> Result<Self> {
        Self::open_game_with(NonZeroUsize::MIN, root, game, keys)
    }
    /// [`Archive::open_game`] with each index file's RSA blocks unwrapped on `threads`: the same archive on any count.
    pub fn open_game_with(
        threads: NonZeroUsize,
        root: impl AsRef<Path>,
        game: Game,
        keys: &Keys,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let set = game.archives();
        let mut entries = BTreeMap::new();
        let mut headers = Vec::new();
        let mut absent = Vec::new();
        for pair in set.pairs {
            let bhd = root.join(format!("{}.bhd", pair.stem));
            if !pair.required && !bhd.exists() {
                absent.push(pair.stem);
                continue;
            }
            let (n, e) = keys
                .get(pair.stem)
                .ok_or_else(|| invalid("RSA", 0, format!("no key for {}", pair.stem)))?;
            let plain = unwrap_rsa_with(threads, &std::fs::read(&bhd)?, n, e)?;
            let header = Header::read(&plain, &set.record)?;
            let size = std::fs::metadata(root.join(format!("{}.bdt", pair.stem)))?.len();
            index_pair(&mut entries, set.pairs, pair, &header, &set.record, size)?;
            headers.push(header);
        }
        Ok(Self {
            root,
            game,
            entries,
            headers,
            absent,
        })
    }
    /// The game these archives belong to.
    pub fn game(&self) -> Game {
        self.game
    }
    /// The install path whose indexes and BDT members this handle reads.
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// The archive pair whose entries report `shard`.
    pub fn pair(&self, shard: u8) -> Option<&'static ArchivePair> {
        self.game.archives().pairs.iter().find(|p| p.shard == shard)
    }
    /// The game's hash of an archive path.
    pub fn hash(&self, path: &str) -> u64 {
        self.game.archives().hash.hash(path)
    }
    /// Whether the index lists `path`.
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&self.hash(path))
    }
    /// Read decrypted stored bytes, including BDT padding. Metadata logical_size
    /// lets consumers distinguish padding without losing reference-reader parity.
    pub fn read_stored(&self, hash: u64) -> Result<Vec<u8>> {
        let e = self.entries.get(&hash).ok_or(crate::Error::Missing(hash))?;
        let pair = self
            .pair(e.shard)
            .ok_or_else(|| invalid("BHD5", 0, "entry from an unknown archive"))?;
        let mut f = std::fs::File::open(self.root.join(format!("{}.bdt", pair.stem)))?;
        f.seek(SeekFrom::Start(e.entry.offset))?;
        let mut out = vec![0; e.entry.padded_size];
        f.read_exact(&mut out)?;
        e.entry.decrypt(&mut out)?;
        Ok(out)
    }
    /// Decode a DCX envelope when present; non-DCX bytes retain their stored padding.
    pub fn read_hash(&self, hash: u64, decoder: &Decoder<'_>) -> Result<Vec<u8>> {
        let bytes = self.read_stored(hash)?;
        decoder.decode_if_dcx(&bytes)
    }
    /// [`Archive::read_hash`] of the hash of the archive path `path`; [`crate::Error::Missing`] if the index lacks it.
    pub fn read(&self, path: &str, decoder: &Decoder<'_>) -> Result<Vec<u8>> {
        self.read_hash(self.hash(path), decoder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_ascii_path_is_refused_at_its_byte() {
        let text = "/a.txt\r\n# note\n  /b\u{e9}.txt\n";
        let e = NameDictionary::parse(text).unwrap_err();
        let crate::Error::Invalid { offset, detail, .. } = &e else {
            panic!("{e}")
        };
        assert_eq!(text.as_bytes().get(*offset), Some(&0xc3), "{e}");
        assert_eq!(*offset, text.find('\u{e9}').unwrap());
        assert!(detail.contains("non-ASCII"), "{e}");
        // a byte order mark at the start of the first line is not a path byte
        let bom = "\u{feff}/a.txt\n";
        assert_eq!(NameDictionary::parse(bom).unwrap().names.len(), 1);
        let e = NameDictionary::parse("\u{feff}/\u{e9}").unwrap_err();
        assert!(e.to_string().starts_with("dictionary at 0x4:"), "{e}");
    }

    fn entry(hash: u64, index_offset: usize, offset: u64, padded_size: usize) -> Entry {
        Entry {
            index_offset,
            hash,
            offset,
            padded_size,
            logical_size: 0,
            aes: None,
            sha: None,
        }
    }
    fn header(entries: Vec<Entry>) -> Header {
        Header {
            salt: Vec::new(),
            entries,
            bucket_count: 1,
            rsa_padding: 0,
        }
    }

    #[test]
    fn an_entry_beyond_the_bdt_and_a_repeated_hash_are_refused_at_their_records() {
        let pairs = [
            ArchivePair::required(0, "Data0"),
            ArchivePair::required(1, "Data1"),
        ];
        let record = RecordLayout::SEKIRO;
        let mut entries = BTreeMap::new();
        let ok = header(vec![entry(5, 36, 0, 100)]);
        index_pair(&mut entries, &pairs, &pairs[0], &ok, &record, 100).unwrap();
        // offset + size wrapping around u64, and plainly beyond the end
        for (offset, padded) in [(u64::MAX - 3, 16), (90, 16)] {
            let bad = header(vec![entry(6, 76, offset, padded)]);
            let e = index_pair(&mut entries, &pairs, &pairs[1], &bad, &record, 100)
                .unwrap_err()
                .to_string();
            assert!(e.starts_with("BHD5 at 0x54: Data1.bdt entry 0x6"), "{e}");
            assert!(e.contains("beyond the 100 bytes"), "{e}");
        }
        let again = header(vec![entry(5, 116, 0, 1)]);
        let e = index_pair(&mut entries, &pairs, &pairs[1], &again, &record, 100)
            .unwrap_err()
            .to_string();
        assert_eq!(
            e,
            "BHD5 at 0x74: path hash 0x5 is listed by Data1 and by Data0; each hash names one entry across the archives"
        );
    }
}
