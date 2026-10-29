//! The writers against every stock file of both installs and the 29 `regulation.bin` files we hold: every DCX envelope
//! at any depth ([`dcx::frame`], [`dcx::encode`]), every BND4 binder and BHF4/BDF4 pair ([`Binder::write_stored`],
//! [`Binder::write`]), every TPF ([`Package::write`]), FMG ([`Fmg::write`]) and PARAM ([`Param::write`]), and the
//! encryption of `regulation.bin` ([`regulation`]).
//!
//! What is asserted:
//! - **byte-equal**: a DCX header and payload framed again from the parsed fields; a binder written from the members as
//!   stored; a TPF, an FMG (all but the 3 that hold unreferenced zero bytes between their strings) and a PARAM written
//!   from what the reader returns; a regulation encrypted again with its vector.
//! - **parse-equal, the difference stated**: a DCX envelope compressed again decodes to the same bytes under the same
//!   header, but its payload is not the shipped one (the compressors are not the tools'); a binder written with its
//!   nested envelopes compressed again reads back to the same binder; the 3 FMGs are written without their gaps.
//! - **refused**: a Kraken envelope with either size word one more or one less, with its payload one byte short or one byte
//!   over, is refused by [`Decoder::decode`].
//!
//! The counts are those of an independent census, two Python models that rebuilt the same files byte for byte: the
//! archives' DCX headers, 61,581 BND4 and 2,072 BHF4 binders, 288,921 TPF packages, 4,472 FMGs and 235 PARAMs, and the 29
//! regulations' 5,489 params. `[run] gate = 'smoke'` re-encodes and probes a seeded sample (one entry in sixteen),
//! `'full'` every one.
//!
//! Needs `installs.*`, `keys.*`, `tools.oodle6`, `oracles.uxm.*/dictionary.txt` and, for the regulations,
//! `oracles.smithbox.elden/regulations`; a test whose input is not configured does nothing.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod inputs;

use std::collections::BTreeMap;
use std::fmt::Display;

use fromsoft_formats::dcx::{self, Compression, Decoder, Encoder, Envelope};
use fromsoft_formats::fmg::Fmg;
use fromsoft_formats::param::Param;
use fromsoft_formats::regulation;
use fromsoft_formats::tpf::Package;
use fromsoft_formats::{Binder, Game, Keys, NameDictionary};
use inputs::{Depth, Game as Data, OracleSet, Tool, depth, install, keys, oodle, oracle, seed};
mod common;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The sample's stride in smoke mode.
const STRIDE: u64 = 16;

struct Census<'a> {
    decoder: &'a Decoder<'a>,
    encoder: Encoder<'a>,
    smoke: bool,
    seed: u64,
    counts: BTreeMap<String, usize>,
    failures: Vec<String>,
    /// The envelopes, params and FMGs that were not written back to the same bytes, by label and reason.
    notes: Vec<String>,
}

/// Whether `written` is `original` with up to 15 zero bytes of archive padding after it (`pad`) or exactly it.
fn same(written: &[u8], original: &[u8], pad: bool) -> bool {
    if !pad {
        return written == original;
    }
    original.starts_with(written)
        && original.len().saturating_sub(written.len()) < 16
        && original[written.len()..].iter().all(|&b| b == 0)
}

fn same_binder(a: &Binder, b: &Binder) -> bool {
    a.version == b.version
        && a.format == b.format
        && a.layout == b.layout
        && a.files.len() == b.files.len()
        && a.files.iter().zip(&b.files).all(|(x, y)| {
            (x.id, &x.name, x.flags, &x.bytes, x.wrap) == (y.id, &y.name, y.flags, &y.bytes, y.wrap)
        })
}

impl Census<'_> {
    fn bump(&mut self, key: &str) {
        self.add(key, 1);
    }

    fn add(&mut self, key: &str, n: usize) {
        *self.counts.entry(key.to_owned()).or_default() += n;
    }

    fn fail(&mut self, label: &str, what: impl Display) {
        self.failures.push(format!("{label}: {what}"));
    }

    fn sampled(&self, key: u64) -> bool {
        !self.smoke || key.wrapping_add(self.seed).wrapping_mul(0x9E37_79B9_7F4A_7C15).is_multiple_of(STRIDE)
    }

    /// One DCX envelope at `label`: framed again, decoded, compressed again (Kraken only in the sample), and for a
    /// Kraken envelope in the sample probed with the malformed variants. Returns the decoded bytes.
    fn envelope(&mut self, label: &str, stored: &[u8], key: u64) -> Option<Vec<u8>> {
        let env = match Envelope::parse(stored, usize::MAX) {
            Ok(env) => env,
            Err(e) => {
                self.bump("dcx refused");
                self.notes.push(format!("dcx refused {label}: {e}"));
                return None;
            }
        };
        let codec = format!("{:?}", env.compression);
        self.bump("dcx envelopes");
        self.bump(&format!("dcx {codec}"));
        let end = 76 + env.payload.len();
        match dcx::frame(&env.format, env.decoded_size, env.payload) {
            Ok(framed) if stored.get(..end) == Some(&framed[..]) => self.bump("dcx framed byte-equal"),
            Ok(_) => self.fail(label, "frame writes other bytes than the envelope's"),
            Err(e) => self.fail(label, format!("frame: {e}")),
        }
        let decoded = match self.decoder.decode(stored) {
            Ok(decoded) => decoded,
            Err(e) => {
                self.fail(label, format!("decode: {e}"));
                return None;
            }
        };
        // every zlib and zstd envelope but the regulations' (35 to 54 MB, slow to compress at level 9 and 21), and the
        // sample of the rest
        if self.sampled(key) || (env.compression != Compression::Kraken && env.decoded_size <= 1 << 24) {
            self.re_encode(label, &env, &decoded);
        }
        if env.compression == Compression::Kraken && self.sampled(key) {
            self.refusals(label, &stored[..end]);
        }
        Some(decoded)
    }

    fn re_encode(&mut self, label: &str, env: &Envelope<'_>, decoded: &[u8]) {
        let codec = format!("{:?}", env.compression);
        let again = match dcx::encode(&env.format, decoded, &self.encoder) {
            Ok(again) => again,
            Err(e) => {
                self.fail(label, format!("encode: {e}"));
                return;
            }
        };
        let parsed = match Envelope::parse(&again, usize::MAX) {
            Ok(parsed) => parsed,
            Err(e) => {
                self.fail(label, format!("the envelope it wrote is refused: {e}"));
                return;
            }
        };
        if parsed.format != env.format || parsed.decoded_size != env.decoded_size {
            self.fail(label, "the envelope it wrote has another header");
        }
        match self.decoder.decode(&again) {
            Ok(back) if back == decoded => self.bump(&format!("dcx {codec} re-encoded decodes equal")),
            Ok(_) => self.fail(label, "the envelope it wrote decodes to other bytes"),
            Err(e) => self.fail(label, format!("the envelope it wrote does not decode: {e}")),
        }
        if parsed.payload == env.payload {
            self.bump(&format!("dcx {codec} re-encoded payload byte-equal"));
        }
        self.add(&format!("dcx {codec} stored payload bytes"), env.payload.len());
        self.add(
            &format!("dcx {codec} re-encoded payload bytes"),
            parsed.payload.len(),
        );
    }

    /// A good envelope, and the same with either size word one more or one less, its payload one byte short and one
    /// byte over: only the first may decode.
    fn refusals(&mut self, label: &str, envelope: &[u8]) {
        // a zero byte after the payload is alignment, so a payload size one over takes it
        let mut base = envelope.to_vec();
        base.push(0);
        if let Err(e) = self.decoder.decode(&base) {
            self.fail(
                label,
                format!("the envelope with one alignment byte is refused: {e}"),
            );
            return;
        }
        let word = |b: &[u8], at: usize| u32::from_be_bytes(b[at..at + 4].try_into().unwrap());
        let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
        for (at, name) in [(28, "decoded size"), (32, "payload size")] {
            let was = word(&base, at);
            for (now, how) in [(was.wrapping_sub(1), "-1"), (was.wrapping_add(1), "+1")] {
                let mut bad = base.clone();
                bad[at..at + 4].copy_from_slice(&now.to_be_bytes());
                cases.push((format!("{name} {how}"), bad));
            }
        }
        let mut short = base.clone();
        short.truncate(base.len() - 2);
        cases.push(("payload one byte short".to_owned(), short));
        let mut over = base;
        over.push(1);
        cases.push(("payload one byte over".to_owned(), over));
        for (what, bad) in cases {
            if self.decoder.decode(&bad).is_ok() {
                self.fail(label, format!("accepted with the {what}"));
            } else {
                self.bump("dcx malformed Kraken envelopes refused");
            }
        }
    }

    /// Any file at `label`: an envelope is decoded and looked through, a binder is walked, and TPFs, FMGs and PARAMs
    /// are written again.
    fn content(&mut self, label: &str, bytes: &[u8], key: u64) {
        if bytes.starts_with(b"DCX\0") {
            if let Some(decoded) = self.envelope(label, bytes, key) {
                self.content(label, &decoded, key);
            }
        } else if bytes.starts_with(b"BND4") {
            self.binder(label, bytes, None, key);
        } else if bytes.starts_with(b"TPF\0") {
            self.tpf(label, bytes);
        } else {
            let lower = label.to_ascii_lowercase();
            let lower = lower.strip_suffix(".dcx").unwrap_or(&lower);
            if lower.ends_with(".fmg") {
                self.fmg(label, bytes);
            } else if lower.ends_with(".param") {
                self.param(label, bytes);
            }
        }
    }

    /// A BND4 binder (`payload` none) or a BHF4 index with its BDF4 payload (an archive entry's bytes, which may end in
    /// zero padding).
    fn binder(&mut self, label: &str, bytes: &[u8], payload: Option<&[u8]>, key: u64) {
        let kind = if payload.is_some() { "bhf4" } else { "bnd4" };
        let stored = match payload {
            Some(payload) => Binder::read_split_stored(bytes, payload),
            None => Binder::read_stored(bytes),
        };
        let stored = match stored {
            Ok(stored) => stored,
            Err(e) => {
                self.fail(label, format!("{kind} read as stored: {e}"));
                return;
            }
        };
        self.bump(&format!("{kind} binders"));
        let written = match payload {
            Some(_) => stored.write_split_stored().map(|(i, p)| (i, Some(p))),
            None => stored.write_stored().map(|b| (b, None)),
        };
        match (written, payload) {
            (Ok((index, Some(pay))), Some(original)) => {
                if same(&index, bytes, true) && same(&pay, original, true) {
                    self.bump("bhf4 byte-equal");
                } else {
                    self.fail(label, "bhf4 written other bytes than the pair's");
                }
            }
            (Ok((file, None)), None) => {
                if same(&file, bytes, false) {
                    self.bump("bnd4 byte-equal");
                } else {
                    let first = (0..file.len().min(bytes.len())).find(|&i| file[i] != bytes[i]);
                    self.fail(
                        label,
                        format!("bnd4 written other bytes than the file's (first difference {first:?})"),
                    );
                }
            }
            (Ok(_), _) => self.fail(label, "write_stored returned the wrong shape"),
            (Err(e), _) => self.fail(label, format!("{kind} write: {e}")),
        }
        if self.sampled(key) {
            self.binder_round_trip(label, bytes, payload);
        }
        for (i, file) in stored.files.iter().enumerate() {
            let name = file.name.clone().unwrap_or_else(|| format!("#{i}"));
            self.content(&format!("{label}|{name}"), &file.bytes, key);
        }
    }

    /// The binder read with its nested envelopes decoded, written with them compressed again and read once more: the
    /// same binder.
    fn binder_round_trip(&mut self, label: &str, bytes: &[u8], payload: Option<&[u8]>) {
        let (decoder, encoder) = (self.decoder, self.encoder);
        let result = (|| -> fromsoft_formats::Result<bool> {
            Ok(match payload {
                Some(payload) => {
                    let a = Binder::read_split(bytes, payload, decoder)?;
                    let (index, pay) = a.write_split(&encoder)?;
                    same_binder(&a, &Binder::read_split(&index, &pay, decoder)?)
                }
                None => {
                    let a = Binder::read(bytes, decoder)?;
                    let again = a.write(&encoder)?;
                    same_binder(&a, &Binder::read(&again, decoder)?)
                }
            })
        })();
        match result {
            Ok(true) => self.bump("binders re-encoded read back equal"),
            Ok(false) => self.fail(
                label,
                "the binder written with its envelopes compressed again reads back different",
            ),
            Err(e) => self.fail(label, format!("binder round trip: {e}")),
        }
    }

    fn tpf(&mut self, label: &str, bytes: &[u8]) {
        self.bump("tpf packages");
        match Package::read(bytes) {
            Ok(package) => match package.write() {
                Ok(written) if written == bytes => self.bump("tpf byte-equal"),
                Ok(_) => self.fail(label, "tpf written other bytes than the file's"),
                Err(e) => self.fail(label, format!("tpf write: {e}")),
            },
            Err(e) => self.fail(label, format!("tpf read: {e}")),
        }
    }

    fn fmg(&mut self, label: &str, bytes: &[u8]) {
        self.bump("fmg files");
        let fmg = match Fmg::read(bytes) {
            Ok(fmg) => fmg,
            Err(e) => {
                self.fail(label, format!("fmg read: {e}"));
                return;
            }
        };
        match fmg.write() {
            Ok(written) if written == bytes => self.bump("fmg byte-equal"),
            Ok(written) => match Fmg::read(&written) {
                Ok(back) if back == fmg => {
                    self.bump("fmg normalised, read back equal");
                    self.notes.push(format!("fmg normalised {label}"));
                }
                _ => self.fail(label, "fmg written reads back different"),
            },
            Err(e) => self.fail(label, format!("fmg write: {e}")),
        }
    }

    fn param(&mut self, label: &str, bytes: &[u8]) {
        self.bump("param files");
        let (param, stored) = match Param::read_stored(bytes) {
            Ok(read) => read,
            Err(e) => {
                self.bump("param refused");
                self.notes.push(format!("param refused {label}: {e}"));
                return;
            }
        };
        match param.write(&stored) {
            Ok(written) if written == bytes => self.bump("param byte-equal"),
            Ok(_) => self.fail(label, "param written other bytes than the file's"),
            Err(e) => self.fail(label, format!("param write: {e}")),
        }
        // a param written with the facts the tools mostly write reads back to the same rows
        match param.write_derived().map(|w| Param::read(&w)) {
            Ok(Ok(back)) if back == param => self.bump("param derived read back equal"),
            other => self.fail(label, format!("param written with derived facts: {other:?}")),
        }
    }
}

fn census<'a>(
    decoder: &'a Decoder<'a>,
    oodle: &'a primase_foundations::compress::oodle::Oodle,
) -> Census<'a> {
    Census {
        decoder,
        encoder: Encoder { oodle: Some(oodle) },
        smoke: depth() == Depth::Smoke,
        seed: seed(),
        counts: BTreeMap::new(),
        failures: Vec::new(),
        notes: Vec::new(),
    }
}

fn report(game: &str, census: &Census<'_>) {
    println!(
        "{game} writers ({} depth, seed {})",
        if census.smoke { "smoke" } else { "full" },
        census.seed
    );
    for (k, v) in &census.counts {
        println!("  {k}: {v}");
    }
    for note in &census.notes {
        println!("  note: {note}");
    }
    for f in &census.failures {
        println!("  FAIL {f}");
    }
}

fn walk(game: Game, data: Data) -> Result<Option<BTreeMap<String, usize>>> {
    let Some(archive) = common::archive(game)? else {
        return Ok(None);
    };
    let (Some(dictionary), Some(dll)) = (
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
        oodle(Tool::Oodle6),
    ) else {
        return Ok(None);
    };
    let names = NameDictionary::read_for(dictionary, game)?;
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 31,
    };
    let mut c = census(&decoder, &dll);

    // the BHD5 entries that are the index of a split binder, and the payload entry that belongs to each
    let mut payload_of = BTreeMap::new();
    for (&hash, found) in &names.names {
        let Some(name) = found.first() else { continue };
        if let Some(stem) = name.strip_suffix("bhd") {
            let payload = archive.hash(&format!("{stem}bdt"));
            if archive.entries.contains_key(&hash) && archive.entries.contains_key(&payload) {
                payload_of.insert(hash, payload);
            }
        }
    }
    let payloads: std::collections::BTreeSet<u64> = payload_of.values().copied().collect();

    for &hash in archive.entries.keys() {
        if payloads.contains(&hash) {
            continue;
        }
        if let Some(&payload) = payload_of.get(&hash) {
            let label = format!("#{hash:016x}");
            let (index, pay) = (
                archive.read_hash(hash, &decoder)?,
                archive.read_hash(payload, &decoder)?,
            );
            c.binder(&label, &index, Some(&pay), hash);
            continue;
        }
        let stored = archive.read_stored(hash)?;
        let label = names
            .names
            .get(&hash)
            .and_then(|n| n.first())
            .cloned()
            .unwrap_or_else(|| format!("#{hash:016x}"));
        c.content(&label, &stored, hash);
    }
    report(game.name(), &c);
    assert!(c.failures.is_empty(), "{} writer failures", c.failures.len());
    Ok(Some(c.counts))
}

/// The expected counts: those of the independent census, and `0` for the keys that must be absent.
fn expect(counts: &BTreeMap<String, usize>, wanted: &[(&str, usize)]) {
    for (key, want) in wanted {
        assert_eq!(counts.get(*key).copied().unwrap_or(0), *want, "{key}");
    }
}

#[test]
fn sekiro_archives() -> Result<()> {
    let Some(c) = walk(Game::Sekiro, Data::Sekiro)? else {
        return Ok(());
    };
    expect(
        &c,
        &[
            // the truncated entry `a28b84c8` of Data5 is the one envelope refused
            ("dcx refused", 1),
            ("dcx envelopes", 27_095),
            ("dcx framed byte-equal", 27_095),
            ("dcx Deflate", 22),
            ("bnd4 binders", 7_859),
            ("bnd4 byte-equal", 7_859),
            ("bhf4 binders", 76),
            ("bhf4 byte-equal", 76),
            ("tpf packages", 14_481),
            ("tpf byte-equal", 14_481),
            ("fmg files", 504),
            ("fmg byte-equal", 504),
            // the 32-bit form (3) and the graphics-quality params with more than 8 trailing zero bytes (12)
            ("param files", 193),
            ("param refused", 15),
            ("param byte-equal", 178),
            ("param derived read back equal", 178),
        ],
    );
    Ok(())
}

#[test]
fn elden_ring_archives() -> Result<()> {
    let Some(c) = walk(Game::EldenRing, Data::Elden)? else {
        return Ok(());
    };
    expect(
        &c,
        &[
            ("dcx refused", 0),
            ("dcx envelopes", 117_141),
            ("dcx framed byte-equal", 117_141),
            ("dcx Deflate", 20),
            ("bnd4 binders", 53_722),
            ("bnd4 byte-equal", 53_722),
            ("bhf4 binders", 1_996),
            ("bhf4 byte-equal", 1_996),
            ("tpf packages", 274_440),
            ("tpf byte-equal", 274_440),
            ("fmg files", 3_968),
            ("fmg byte-equal", 3_965),
            // three files hold unreferenced zero bytes between their strings, which the writer leaves out
            ("fmg normalised, read back equal", 3),
            ("param files", 45),
            ("param refused", 12),
            ("param byte-equal", 33),
            ("param derived read back equal", 33),
        ],
    );
    Ok(())
}

/// Elden Ring's `regulation.bin` files: the installed one and the 28 of Smithbox's releases.
#[test]
fn regulations() -> Result<()> {
    let (Some(root), Some(folder), Some(dll), Some(releases)) = (
        install(Data::Elden),
        keys(Data::Elden),
        oodle(Tool::Oodle6),
        oracle(OracleSet::Smithbox, Data::Elden, "regulations"),
    ) else {
        return Ok(());
    };
    let keys = Keys::from_folder(folder, Game::EldenRing, None)?;
    let key = keys.regulation().unwrap();
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 31,
    };
    let mut c = census(&decoder, &dll);
    let mut files = vec![root.join("regulation.bin")];
    let mut folders: Vec<_> = std::fs::read_dir(&releases)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("regulation.bin").exists())
        .collect();
    folders.sort();
    files.extend(folders.iter().map(|f| f.join("regulation.bin")));
    for (i, file) in files.iter().enumerate() {
        let label = if i == 0 {
            "installed".to_owned()
        } else {
            file.parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let bytes = std::fs::read(file)?;
        c.bump("regulations");
        let iv = regulation::iv(&bytes)?;
        let plain = regulation::decrypt(key, &bytes)?;
        if regulation::encrypt(key, &iv, &plain)? == bytes {
            c.bump("regulations encrypted back byte-equal");
        } else {
            c.fail(&label, "encrypting what was decrypted gives other bytes");
        }
        // a regulation is sampled by its position, the installed one and one in four
        c.content(&label, &plain, if i % 4 == 0 { 0 } else { 1 });
        if i == 0 {
            // the whole file made again from its binder: it decrypts, decodes and reads back to the binder
            let binder = regulation::decrypt(key, &bytes).and_then(|p| {
                let decoded = decoder.decode(&p)?;
                Binder::read(&decoded, &decoder)
            })?;
            let made = regulation::pack(key, &iv, &binder, &dcx::Format::ZSTD, &c.encoder)?;
            let back = fromsoft_formats::game_params::regulation_binder(key, &made, &decoder)?;
            if same_binder(&binder, &back) {
                c.bump("regulation packed from its binder reads back equal");
            } else {
                c.fail(&label, "regulation packed from its binder reads back different");
            }
        }
    }
    report("regulations", &c);
    assert!(c.failures.is_empty(), "{} writer failures", c.failures.len());
    expect(
        &c.counts,
        &[
            ("regulations", 29),
            ("regulations encrypted back byte-equal", 29),
            // 16 zlib envelopes (1.00.0 to 1.10) and 13 zstd (1.12.1 on, with the installed 1.17.3)
            ("dcx envelopes", 29),
            ("dcx framed byte-equal", 29),
            ("bnd4 binders", 29),
            ("bnd4 byte-equal", 29),
            ("param files", 5_489),
            ("param byte-equal", 5_489),
            ("regulation packed from its binder reads back equal", 1),
        ],
    );
    Ok(())
}
