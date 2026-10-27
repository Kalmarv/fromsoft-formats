//! Lists the members of one binder inside an installed game's archives.
//!
//! ```text
//! cargo run --example list_binder -- <sekiro|elden> <game folder> <keys folder> <oodle library | -> <archive path>
//! cargo run --example list_binder -- sekiro "D:/Games/Sekiro" ./keys "D:/Games/Sekiro/oo2core_6_win64.dll" /param/gameparam/gameparam.parambnd.dcx
//! ```
//!
//! The keys folder holds one PKCS#1 `RSA PUBLIC KEY` PEM per archive (`Data1.pem` ... `Data5.pem` for Sekiro;
//! `Data0.pem` ... `Data3.pem`, `DLC.pem`, `sd.pem` for Elden Ring). Pass `-` for the Oodle library off Windows, where
//! a portable decoder is used instead. (In Git Bash on Windows, set `MSYS_NO_PATHCONV=1` so that the archive path is not
//! rewritten as a file path.)
#![allow(unsafe_code, reason = "opens the Oodle library the caller names")]

use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
use primase_foundations::compress::oodle::{Oodle, Version};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [game, root, keys, oodle, path] = args.as_slice() else {
        return Err("usage: list_binder <sekiro|elden> <game folder> <keys folder> <oodle library | -> <archive path>".into());
    };
    let game = match game.as_str() {
        "sekiro" => Game::Sekiro,
        "elden" => Game::EldenRing,
        other => return Err(format!("unknown game {other:?}").into()),
    };

    // The caller supplies the keys, the game's own Oodle library and the install.
    let keys = Keys::from_folder(keys, game, None)?;
    let archive = Install::new(root, game, keys).open()?;
    // SAFETY: the library is the game's own, named by the caller.
    let oodle = unsafe { Oodle::open((oodle != "-").then(|| Path::new(oodle)), Version::V6) }?;
    let decoder = Decoder {
        oodle: Some(&oodle),
        max_output: 1 << 30,
    };

    let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
    for file in &binder.files {
        println!(
            "{:>8}  {:>10} bytes  {}",
            file.id.map_or_else(|| "-".to_owned(), |id| id.to_string()),
            file.bytes.len(),
            file.name.as_deref().unwrap_or("(unnamed)")
        );
    }
    Ok(())
}
