//! A census of what the install's placed-object binders hold besides their model: Sekiro's `/obj/*.objbnd.dcx`,
//! Elden Ring's `/asset/aeg/*/*.geombnd.dcx`. The area read's `object_animations` option reads the animation binder as
//! a `.anibnd` member of the model's binder (Sekiro: 233 of 1395, none beside the binder as a file of its own, which
//! this census still checks for Elden Ring), so this prints what is there instead of assuming:
//!
//! - `members <game> <extension> <count>`: the extensions of every member of every object binder, so an animation
//!   stored under another name shows;
//! - `nested <game> <count>` and `beside <game> <count>`: binders with an `.anibnd` member, and with an `.anibnd.dcx`
//!   binder beside them in the archive;
//! - `sample <game> <binder> <members of its animation binder>`: the first few of each, to read the names from.
//!
//! It asserts only that the binders read. Runs where the install, the dictionary and the Oodle library are configured;
//! otherwise it does nothing.
mod inputs;

use fromsoft_formats::{Archive, Binder, NameDictionary, dcx::Decoder};
use inputs::{Game, OracleSet, Tool, install, oodle, oracle};
use std::collections::BTreeMap;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[allow(unsafe_code)]
fn native(
    path: &inputs::OodleLibrary,
) -> primase_foundations::compress::Result<primase_foundations::compress::oodle::Oodle> {
    // SAFETY: caller explicitly supplies a trusted native version-6 decoder.
    unsafe {
        primase_foundations::compress::oodle::Oodle::open(
            path.path(),
            primase_foundations::compress::oodle::Version::V6,
        )
    }
}

fn leaf(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase()
}

fn census(
    game: Game,
    kind: fromsoft_formats::Game,
    label: &str,
    is_object_binder: impl Fn(&str) -> Option<&'static str>,
) -> Result<()> {
    let Some(dir) = install(game) else {
        return Ok(());
    };
    // each game opens with its own keys: Sekiro's are published, Elden Ring's are the install owner's folder
    let keys = match (kind, inputs::keys(game)) {
        (_, Some(folder)) => fromsoft_formats::Keys::from_folder(folder, kind, None)?,
        (_, None) => return Ok(()),
    };
    let Some(names) = oracle(OracleSet::Uxm, game, "dictionary.txt") else {
        return Ok(());
    };
    let Some(dll) = oodle(Tool::Oodle6) else {
        return Ok(());
    };
    let oodle = native(&dll)?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1024 * 1024 * 1024,
    };
    let archive = Archive::open_game(dir, kind, &keys)?;
    let names = NameDictionary::read_for(names, kind)?;
    // the object binders, in the dictionary's order
    let mut wanted = Vec::new();
    for (&hash, paths) in &names.names {
        if !archive.entries.contains_key(&hash) {
            continue;
        }
        for path in paths {
            if let Some(suffix) = is_object_binder(path) {
                wanted.push((path.as_str(), suffix));
            }
        }
    }
    // an animation binder as its member count and its first eight members' names
    let summary = |animation: &Binder| -> (usize, Vec<String>) {
        let list = animation
            .files
            .iter()
            .take(8)
            .map(|f| leaf(f.name.as_deref().unwrap_or("")))
            .collect();
        (animation.files.len(), list)
    };
    /// One object binder: whether it read, its members by extension, the animation binder nested in it, and the error
    /// that stopped it.
    #[derive(Default)]
    struct Object {
        read: bool,
        members: BTreeMap<String, usize>,
        nested: Option<(usize, Vec<String>)>,
        error: Option<String>,
    }
    let read = archive.read_each_binder(
        wanted.iter().map(|w| w.0),
        &decoder,
        inputs::threads(),
        |_, binder| {
            let mut o = Object::default();
            let result = (|| -> Result<()> {
                let binder = binder?;
                o.read = true;
                for f in &binder.files {
                    let name = leaf(f.name.as_deref().unwrap_or(""));
                    let extension = name.split_once('.').map_or("(none)", |(_, e)| e).to_owned();
                    *o.members.entry(extension).or_default() += 1;
                    if name.ends_with(".anibnd") || name.ends_with(".anibnd.dcx") {
                        o.nested = Some(summary(&Binder::read(
                            &decoder.decode_if_dcx(&f.bytes)?,
                            &decoder,
                        )?));
                    }
                }
                Ok(())
            })();
            o.error = result.err().map(|e| e.to_string());
            o
        },
    )?;
    // the animation binder beside each object binder that read and holds none, in a second pass
    let sibling =
        |(path, suffix): (&str, &str)| path.strip_suffix(suffix).map(|stem| format!("{stem}.anibnd.dcx"));
    let siblings: Vec<String> = wanted
        .iter()
        .zip(&read)
        .filter(|(_, o)| o.error.is_none() && o.nested.is_none())
        .filter_map(|(&w, _)| sibling(w).filter(|p| archive.contains(p)))
        .collect();
    let beside_read = archive.read_each_binder(&siblings, &decoder, inputs::threads(), |_, binder| {
        binder.map(|b| summary(&b)).map_err(|e| e.to_string())
    })?;
    let mut beside_read: BTreeMap<&String, _> = siblings.iter().zip(beside_read).collect();
    let mut members: BTreeMap<String, usize> = BTreeMap::new();
    let (mut binders, mut nested, mut beside, mut samples) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for (&(path, suffix), o) in wanted.iter().zip(read) {
        binders += usize::from(o.read);
        for (extension, n) in o.members {
            *members.entry(extension).or_default() += n;
        }
        if let Some(e) = o.error {
            failures.push(format!("{path}: {e}"));
            continue;
        }
        let mut animation = o.nested;
        if let Some(sibling) = sibling((path, suffix)).filter(|p| archive.contains(p)) {
            beside += 1;
            if animation.is_none() {
                match beside_read.remove(&sibling).ok_or("a sibling not read")? {
                    Ok(summary) => animation = Some(summary),
                    Err(e) => {
                        failures.push(format!("{path}: {e}"));
                        continue;
                    }
                }
            }
        }
        if let Some((count, list)) = animation {
            nested += 1;
            if samples < 6 {
                samples += 1;
                println!("sample {label} {path} {count} members: {list:?}");
            }
        }
    }
    println!("binders {label} {binders}");
    for (extension, count) in &members {
        println!("members {label} {extension} {count}");
    }
    println!("nested-or-beside {label} {nested} (beside {beside})");
    for f in &failures {
        println!("FAIL {f}");
    }
    assert!(failures.is_empty(), "{} unreadable binders", failures.len());
    Ok(())
}

#[test]
fn what_the_object_binders_hold() -> Result<()> {
    census(Game::Sekiro, fromsoft_formats::Game::Sekiro, "sekiro", |p| {
        (p.starts_with("/obj/") && p.ends_with(".objbnd.dcx")).then_some(".objbnd.dcx")
    })?;
    census(Game::Elden, fromsoft_formats::Game::EldenRing, "elden", |p| {
        (p.starts_with("/asset/aeg/") && p.ends_with(".geombnd.dcx")).then_some(".geombnd.dcx")
    })?;
    Ok(())
}
