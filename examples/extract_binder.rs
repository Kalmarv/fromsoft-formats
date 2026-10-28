//! Writes the members of one binder inside an installed game's archives to a folder, one file per member under its own
//! name (its last path component; members without a name are written as `<id>`).
//!
//! ```text
//! cargo run --example extract_binder -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <archive path> <out folder>
//! ```
//!
//! Arguments as `list_binder`'s, plus the folder written to (created if missing). The folder holds game data: keep it out
//! of any repository. An archive path of the form `file:<path>` reads a binder from a local file instead (a binder
//! extracted earlier that holds binders, as a shader binder holds its bundles); the game folder and keys are then unused.
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, path, out] = args.as_slice() else {
        return Err("usage: extract_binder <sekiro|elden> <game folder> <keys folder> <oodle library | -> <archive path> <out folder>".into());
    };
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };
    let bytes = match path.strip_prefix("file:") {
        Some(local) => std::fs::read(local)?,
        None => {
            let keys = Keys::from_folder(keys, game, None)?;
            Install::new(root, game, keys).open()?.read(path, &decoder)?
        }
    };
    let binder = Binder::read(&bytes, &decoder)?;
    std::fs::create_dir_all(out)?;
    for file in &binder.files {
        let name = match file.name.as_deref() {
            Some(n) => n.rsplit(['\\', '/']).next().unwrap_or(n).to_owned(),
            None => file.id.map_or_else(|| "unnamed".to_owned(), |id| id.to_string()),
        };
        std::fs::write(Path::new(out).join(&name), &file.bytes)?;
    }
    println!("{} members written", binder.files.len());
    Ok(())
}
