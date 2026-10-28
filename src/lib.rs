//! Strict readers (and for DCX, BND4, TPF, FMG, PARAM and `regulation.bin` writers) of FromSoftware's data (Sekiro: Shadows Die Twice and Elden Ring): the installed BHD5/BDT archives
//! ([`Archive`], with keys the caller supplies), DCX envelopes ([`dcx`]), BND4 binders ([`Binder`]) and the formats
//! they hold: FLVER models, map layouts (MSB), light banks (BTL), hang edges (EDGE), event scripts (EMEVD), text
//! (FMG), draw params (GPARAM), material definitions (MTD, MATBIN), shader metaparams, params (PARAM), animation
//! events (TAE), texture packages (TPF), collision (Havok, CLM2), effects and their lights (FXR), and the instruction
//! schema of the event scripts (EMEDF).
//!
//! Every read is checked: data that is shorter than it says, refers outside itself or holds a value the reader does
//! not know is an [`Error`] naming the format and the offset, rather than a panic or a silently skipped field. What a
//! reader keeps is what the file stores; what a file leaves to the game (which file is in force, what a flag means) is
//! the caller's to decide.
//!
//! The crate distributes no key, dictionary, native library or game file, and stores no key beyond the [`Keys`] value
//! the caller passes. The caller supplies the archive keys of its own install (kept as PEM files and read with
//! [`Keys::from_folder`]), the Oodle library for Kraken-compressed files (a [`dcx::Decoder`]) and, to turn path hashes
//! back into paths, a path list ([`NameDictionary`]).
//!
//! # Reading a file
//!
//! Every reader takes the bytes of one file, decompressed, and returns either the file's contents or an error that
//! says what it refused and where:
//!
//! ```
//! use fromsoft_formats::{Error, flver::Model};
//!
//! let err = Model::read(b"not a FLVER file").unwrap_err();
//! assert!(matches!(&err, Error::Invalid { format: "FLVER", offset: 0, .. }));
//! println!("{err}");
//! ```
//!
//! # Reading an install
//!
//! An [`Install`] names a game folder and the caller's keys; [`Install::open`] indexes its archives, and
//! [`Archive::read`] returns the decompressed bytes of an entry by archive path. Binders are opened with
//! [`Binder::read`], which also decodes the DCX envelopes of its members:
//!
//! ```no_run
//! # #![allow(unsafe_code)]
//! use fromsoft_formats::{Binder, Game, Install, Keys, dcx::Decoder};
//! use primase_foundations::compress::oodle::{Oodle, Version};
//! use std::path::Path;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // One PKCS#1 `RSA PUBLIC KEY` PEM per archive (`Data1.pem` ... `Data5.pem`), which the caller holds.
//! let keys = Keys::from_folder("keys", Game::Sekiro, None)?;
//! let archive = Install::new("D:/Games/Sekiro", Game::Sekiro, keys).open()?;
//!
//! // Kraken-compressed files need Oodle: the game's own library on Windows, a portable decoder elsewhere.
//! // SAFETY: the library is the game's own.
//! let oodle = unsafe { Oodle::open(Some(Path::new("D:/Games/Sekiro/oo2core_6_win64.dll")), Version::V6) }?;
//! let decoder = Decoder { oodle: Some(&oodle), max_output: 1 << 30 };
//!
//! let binder = Binder::read(&archive.read("/param/gameparam/gameparam.parambnd.dcx", &decoder)?, &decoder)?;
//! for file in &binder.files {
//!     println!("{:?} {} bytes", file.name, file.bytes.len());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! `examples/list_binder.rs` is this example as a program.
#![deny(clippy::disallowed_methods, clippy::disallowed_types, missing_docs)]
// every public item is documented
// Every reader takes the bytes of a stored file: no index or slice that can panic, no arithmetic on offsets or sizes that
// can wrap, no cast that can truncate (the tests, which build their inputs by hand, are outside the rule).
#![cfg_attr(
    not(test),
    deny(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation
    )
)]

pub mod affine;
pub mod archive;
pub mod area;
pub mod bhd;
pub mod binder;
pub mod blf;

mod binder_write;
pub mod btl;
mod bytes;
pub mod clm2;
pub mod collision;
pub mod dcx;
pub mod edge;
mod elden;
pub mod emedf;
pub mod emevd;
mod emevd_write;
mod emit;
pub mod equipment;
pub mod eventflag;
pub mod flver;
pub mod fmg;
mod fmg_write;
pub mod fxr;
pub mod game;
pub mod game_params;
pub mod gparam;
pub mod grass;

pub mod gparam_events;
pub mod gparam_params;
mod keys;
pub mod mapid;
pub mod matbin;
mod matbin_write;
pub mod messages;
pub mod metaparam;
pub mod msb;
pub mod mtd;
mod mtd_write;
pub mod names;
pub mod overworld;
pub mod param;
mod param_write;
pub mod paramdef;
pub mod regulation;
mod sekiro;
pub mod shader;
pub mod states;
pub mod tae;
pub mod tpf;
pub mod weather_assets;

mod tpf_write;
mod xml;

pub use archive::{Archive, Install, Keys, NameDictionary};
pub use binder::{Binder, BinderFile};
pub use game::Game;

/// A refusal: the data read is not what its format allows, or could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `format`'s data at `offset` holds what the reader refuses, or lacks what it needs.
    #[error("{format} at {offset:#x}: {detail}")]
    Invalid {
        /// The format that refused the data (`"FLVER"`, `"BND4"`).
        format: &'static str,
        /// The byte offset of the refused field in the data the caller passed, also for a field inside a record (for a few
        /// table-level refusals, an index; see the format's reader).
        offset: usize,
        /// What was expected or found.
        detail: String,
    },
    /// A Havok object the reader holds is not readable.
    #[error("{0}")]
    Havok(#[from] havok_formats::Error),
    /// A file could not be read from disk.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    /// A compressed payload did not decompress.
    #[error("compression: {0}")]
    Compression(#[from] primase_foundations::compress::Error),
    /// The archives hold no entry of this path hash.
    #[error("entry hash {0:08x} not found")]
    Missing(u64),
}

impl Error {
    /// [`Error::Invalid`]: `format`'s data at `offset` is refused for `detail`.
    pub fn invalid(format: &'static str, offset: usize, detail: impl ToString) -> Self {
        Error::Invalid {
            format,
            offset,
            detail: detail.to_string(),
        }
    }
}

/// A read's result.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn invalid(format: &'static str, offset: usize, detail: impl ToString) -> Error {
    Error::invalid(format, offset, detail)
}
