//! What the real-data tests share: the configured archives, the Oodle codec, and the reference dumps' float bits.
#![allow(dead_code)]

/// The version-6 Oodle codec: the configured game library on Windows, oozextract elsewhere.
#[allow(unsafe_code)]
pub fn oodle(
    library: crate::inputs::OodleLibrary,
) -> primase_foundations::compress::Result<primase_foundations::compress::oodle::Oodle> {
    // SAFETY: the test configuration names a trusted native library.
    unsafe {
        primase_foundations::compress::oodle::Oodle::open(
            library.path(),
            primase_foundations::compress::oodle::Version::V6,
        )
    }
}

/// A configured game's archives (`installs.sekiro` with `keys.sekiro`, or `installs.elden` with `keys.elden`), or
/// `None` when the configuration lacks them.
pub fn archive(
    game: fromsoft_formats::Game,
) -> Result<Option<fromsoft_formats::Archive>, fromsoft_formats::Error> {
    use crate::inputs::{Game as Data, install, keys, threads};
    use fromsoft_formats::{Archive, Game, Keys};
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    Ok(match (install(data), keys(data)) {
        (Some(dir), Some(keys)) => Some(Archive::open_game_with(
            threads(),
            dir,
            game,
            &Keys::from_folder(keys, game, None)?,
        )?),
        _ => None,
    })
}

/// A float a reference dump writes as its u32 bit pattern in hex.
pub fn bits(v: &serde_json::Value) -> Result<u32, String> {
    u32::from_str_radix(v.as_str().ok_or("float bits")?, 16).map_err(|e| format!("float bits {v}: {e}"))
}

/// Three floats a reference dump writes as bit patterns.
pub fn bits3(v: &serde_json::Value) -> Result<[u32; 3], String> {
    let a = v.as_array().ok_or("vector")?;
    Ok([bits(&a[0])?, bits(&a[1])?, bits(&a[2])?])
}

/// A census's totals over some leaves, which add to its totals over others in any grouping: [`each_leaf`] keeps one
/// per archive entry and adds them up in hash order.
pub trait Tally: Default + Send {
    /// Adds `other`'s totals to these.
    fn add(&mut self, other: Self);
}

/// Adds the counts of `from` to those of `into`, key by key.
pub fn add_counts<K: Ord>(
    into: &mut std::collections::BTreeMap<K, usize>,
    from: std::collections::BTreeMap<K, usize>,
) {
    for (key, n) in from {
        *into.entry(key).or_default() += n;
    }
}

/// Calls `visit(totals, label, bytes)` for each leaf file of the archive entries whose dictionary name ends with one of
/// `suffixes`: a BND4 is entered (its members decoded, at any depth up to eight), a split binder (a BHF4 index named
/// `...bhd`, its payload the entry named `...bdt`) is opened and entered, and anything else is a leaf. `label` is the
/// entry's name, then `|` and the member names down to the leaf. The entries are read in one pass over the BDTs
/// ([`fromsoft_formats::Archive::read_each`], the split binders with [`fromsoft_formats::Archive::read_each_binder`])
/// and visited on [`crate::inputs::threads`] threads, each into totals of its own, which are added up in path-hash
/// order. Returns how many entries were visited and the totals; the first error in hash order (or a panic in `visit`)
/// is the error.
pub fn each_leaf<A: Tally>(
    archive: &fromsoft_formats::Archive,
    names: &fromsoft_formats::NameDictionary,
    suffixes: &[&str],
    decoder: &fromsoft_formats::dcx::Decoder<'_>,
    visit: impl Fn(&mut A, &str, &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Sync,
) -> Result<(usize, A), String> {
    type Visit<'a, A> =
        &'a dyn Fn(&mut A, &str, &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn members<A>(
        binder: &fromsoft_formats::Binder,
        label: &str,
        decoder: &fromsoft_formats::dcx::Decoder<'_>,
        totals: &mut A,
        visit: Visit<'_, A>,
        depth: usize,
    ) -> Result<(), String> {
        for (i, file) in binder.files.iter().enumerate() {
            let name = file.name.clone().unwrap_or_else(|| format!("#{i}"));
            leaves(
                &file.bytes,
                &format!("{label}|{name}"),
                decoder,
                totals,
                visit,
                depth + 1,
            )?;
        }
        Ok(())
    }
    fn leaves<A>(
        bytes: &[u8],
        label: &str,
        decoder: &fromsoft_formats::dcx::Decoder<'_>,
        totals: &mut A,
        visit: Visit<'_, A>,
        depth: usize,
    ) -> Result<(), String> {
        if bytes.starts_with(b"BHF4") {
            Err("a split binder not named ...bhd".to_owned())
        } else if bytes.starts_with(b"BND4") && depth < 8 {
            let binder = fromsoft_formats::Binder::read(bytes, decoder).map_err(|e| e.to_string())?;
            members(&binder, label, decoder, totals, visit, depth)
        } else {
            visit(totals, label, bytes).map_err(|e| e.to_string())
        }
    }
    // the entries the walk enters, each by the first of its names with one of the suffixes
    let wanted: std::collections::BTreeMap<u64, &str> = names
        .names
        .iter()
        .filter(|(hash, _)| archive.entries.contains_key(hash))
        .filter_map(|(&hash, paths)| {
            let path = paths.iter().find(|p| suffixes.iter().any(|s| p.ends_with(s)))?;
            Some((hash, path.as_str()))
        })
        .collect();
    let (split, whole): (Vec<_>, Vec<_>) = wanted
        .iter()
        .map(|(&hash, &path)| (hash, path))
        .partition(|(_, path)| path.ends_with("bhd"));
    let threads = crate::inputs::threads();
    let read_whole = archive
        .read_each(whole.iter().map(|w| w.1), decoder, threads, |path, bytes| {
            let mut totals = A::default();
            let result = bytes
                .map_err(|e| e.to_string())
                .and_then(|bytes| leaves(bytes, path, decoder, &mut totals, &visit, 0));
            (totals, result)
        })
        .map_err(|p| p.to_string())?;
    let read_split = archive
        .read_each_binder(split.iter().map(|w| w.1), decoder, threads, |path, binder| {
            let mut totals = A::default();
            let result = binder
                .map_err(|e| e.to_string())
                .and_then(|binder| members(&binder, path, decoder, &mut totals, &visit, 0));
            (totals, result)
        })
        .map_err(|p| p.to_string())?;
    let walked: std::collections::BTreeMap<u64, (A, Result<(), String>)> = whole
        .iter()
        .map(|w| w.0)
        .zip(read_whole)
        .chain(split.iter().map(|w| w.0).zip(read_split))
        .collect();
    let mut totals = A::default();
    for (entry, result) in walked.into_values() {
        result?;
        totals.add(entry);
    }
    Ok((wanted.len(), totals))
}

/// `work(hash, bytes)` for every entry of `archive` whose path hash `keep` accepts, by hash: the bytes (or the error)
/// [`fromsoft_formats::Archive::read_hash`] gives, read in one pass over the BDTs ([`fromsoft_formats::Archive::scan`])
/// with the decoding and `work` on [`crate::inputs::threads`] threads. For a census whose result per entry is small:
/// the caller merges them in the order a walk of the index gives. A panic in `work` is returned as the error.
pub fn scan_map<T: Send>(
    archive: &fromsoft_formats::Archive,
    decoder: &fromsoft_formats::dcx::Decoder<'_>,
    keep: impl Fn(u64) -> bool,
    work: impl Fn(u64, fromsoft_formats::Result<&[u8]>) -> T + Sync,
) -> Result<std::collections::BTreeMap<u64, T>, Box<dyn std::error::Error>> {
    let out = std::sync::Mutex::new(std::collections::BTreeMap::new());
    archive
        .scan(decoder, crate::inputs::threads(), keep, |hash, bytes| {
            let t = work(hash, bytes);
            out.lock().expect("only an insert holds the lock").insert(hash, t);
        })
        .map_err(|p| p.to_string())?;
    Ok(out.into_inner().map_err(|e| e.to_string())?)
}
