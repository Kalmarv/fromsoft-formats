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
) -> Result<Option<fromsoft_formats::Archive>, Box<dyn std::error::Error>> {
    use crate::inputs::{Game as Data, install, keys};
    use fromsoft_formats::{Archive, Game, Keys};
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    Ok(match (install(data), keys(data)) {
        (Some(dir), Some(keys)) => Some(Archive::open_game(
            dir,
            game,
            &Keys::from_folder(keys, game, None)?,
        )?),
        _ => None,
    })
}

/// A float a reference dump writes as its u32 bit pattern in hex.
pub fn bits(v: &serde_json::Value) -> Result<u32, Box<dyn std::error::Error>> {
    Ok(u32::from_str_radix(v.as_str().ok_or("float bits")?, 16)?)
}

/// Three floats a reference dump writes as bit patterns.
pub fn bits3(v: &serde_json::Value) -> Result<[u32; 3], Box<dyn std::error::Error>> {
    let a = v.as_array().ok_or("vector")?;
    Ok([bits(&a[0])?, bits(&a[1])?, bits(&a[2])?])
}

/// Calls `f(label, bytes)` for each leaf file of the archive entries whose dictionary name ends with one of `suffixes`:
/// a BND4 is entered (its members decoded, at any depth up to eight) and anything else is a leaf. `label` is the entry's
/// name, then `|` and the member names down to the leaf. Entries are visited in path-hash order. Returns how many entries
/// were visited.
pub fn each_leaf(
    archive: &fromsoft_formats::Archive,
    names: &fromsoft_formats::NameDictionary,
    suffixes: &[&str],
    decoder: &fromsoft_formats::dcx::Decoder<'_>,
    mut f: impl FnMut(&str, &[u8]) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<usize, Box<dyn std::error::Error>> {
    fn leaves(
        bytes: &[u8],
        label: &str,
        decoder: &fromsoft_formats::dcx::Decoder<'_>,
        f: &mut impl FnMut(&str, &[u8]) -> Result<(), Box<dyn std::error::Error>>,
        depth: usize,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if bytes.starts_with(b"BND4") && depth < 8 {
            let binder = fromsoft_formats::Binder::read(bytes, decoder)?;
            for (i, file) in binder.files.iter().enumerate() {
                let name = file.name.clone().unwrap_or_else(|| format!("#{i}"));
                leaves(&file.bytes, &format!("{label}|{name}"), decoder, f, depth + 1)?;
            }
            Ok(())
        } else {
            f(label, bytes)
        }
    }
    let mut visited = 0;
    for (hash, paths) in &names.names {
        if !archive.entries.contains_key(hash) {
            continue;
        }
        let Some(path) = paths.iter().find(|p| suffixes.iter().any(|s| p.ends_with(s))) else {
            continue;
        };
        let bytes = archive.read_hash(*hash, decoder)?;
        visited += 1;
        leaves(&bytes, path, decoder, &mut f, 0)?;
    }
    Ok(visited)
}
