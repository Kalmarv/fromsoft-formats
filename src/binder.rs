//! BND4 file tables, names, hash buckets and file payloads.
use crate::{
    Result,
    bytes::{Budget, View, coverage},
    dcx::{Decoder, Envelope},
    invalid,
};
use primase_foundations::compress::{DEFLATE_MAX_RATIO, Decompressor, Zlib};
/// How many times over the file's bytes a binder's records may decode to (compressed files hold fewer bytes than they
/// make; the real binders of the games decode to a small multiple of their size).
const DECODED_TIMES: usize = 64;
/// One member of a binder.
#[derive(Debug, Clone)]
pub struct BinderFile {
    /// The member's ID, where the binder stores IDs.
    pub id: Option<i32>,
    /// The member's name, where the binder stores names (UTF-16 or ASCII; stored text, not normalised).
    pub name: Option<String>,
    /// The member's two flag bits as stored (bit 0: stored compressed).
    pub flags: u8,
    /// The member's bytes, decompressed; a nested DCX envelope is decoded too. Empty when read by
    /// [`Binder::split_names`].
    pub bytes: Vec<u8>,
}
/// A BND4 binder (or a BHF4 index with its BDF4 payload) read whole.
#[derive(Debug, Clone)]
pub struct Binder {
    /// The 8 version bytes of the header.
    pub version: [u8; 8],
    /// The record format flags, normalised to the layout the reader checks.
    pub format: u8,
    /// The members, in file-table order.
    pub files: Vec<BinderFile>,
}
impl Binder {
    /// Read a BND4 binder held whole in `data`: its file table and members, with compressed members and nested DCX
    /// envelopes decoded by `decoder`. The members' bytes copied or decoded are bounded by the file's size (a table
    /// that names one range over and over is refused), and every byte of the file must be accounted for, apart from up to 15 zero bytes of padding between parts.
    pub fn read(data: &[u8], decoder: &Decoder<'_>) -> Result<Self> {
        Self::read_inner(data, None, decoder, false, None)
    }
    /// [`Binder::read`] keeping only the files whose name `keep` accepts: the others are validated as the full read
    /// validates them (their bytes are within the file and the file is covered as it demands) and left out, with no
    /// bytes, neither copied nor decoded (a binder of hundreds of shader bundles holds the few a map names, and decoding
    /// all of them is what reading the bundles spent its time and memory on). A file without a name is always kept.
    pub fn read_where(data: &[u8], decoder: &Decoder<'_>, keep: &dyn Fn(&str) -> bool) -> Result<Self> {
        Self::read_inner(data, None, decoder, false, Some(keep))
    }
    /// Read a BHF4 index `header` and its matching BDF4 payload file `payload` (the `.edgebhd`/`.edgebdt` form).
    pub fn read_split(header: &[u8], payload: &[u8], decoder: &Decoder<'_>) -> Result<Self> {
        Self::read_inner(header, Some(payload), decoder, false, None)
    }
    /// [`Binder::read_split`] keeping only the files whose name `keep` accepts: the others are validated (their bytes
    /// are within the payload and cover it as the full read demands) and left out, with no bytes (a payload of
    /// hundreds of texture packages holds the few a map binds, and copying and decoding all of them is what a map's
    /// read spent its memory on).
    pub fn read_split_where(
        header: &[u8],
        payload: &[u8],
        decoder: &Decoder<'_>,
        keep: &dyn Fn(&str) -> bool,
    ) -> Result<Self> {
        Self::read_inner(header, Some(payload), decoder, false, Some(keep))
    }
    /// The regulation number the header's version bytes spell: eight ASCII digits (`11730034` for Elden Ring's
    /// 1.17.3 `regulation.bin`), the number a paramdef's `FirstVersion` and `RemovedVersion` are in. `None` where the
    /// version is not eight digits (Sekiro's `gameparam.parambnd.dcx` holds `07D7R6`).
    pub fn regulation_number(&self) -> Option<u32> {
        if !self.version.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(&self.version).ok()?.parse().ok()
    }

    /// The first member whose file name (what follows its last `/` or `\`) is `leaf`, ASCII case folded; `None` where no
    /// member has it. A member without a name is never found.
    pub fn member(&self, leaf: &str) -> Option<&BinderFile> {
        let want = crate::names::fold(leaf);
        self.files.iter().find(|f| {
            f.name
                .as_deref()
                .and_then(|n| n.rsplit(['/', '\\']).next())
                .is_some_and(|n| crate::names::fold(n) == want)
        })
    }
    /// The names of the binders a divided binder names as the rest of its character's: the stems (lower case) of its
    /// `.txt` and `.blf` members whose file name starts with `prefix` (compared lower case), in the binder's order, each once.
    /// A character's animations are kept in other binders (Sekiro's player `c0000.anibnd` holds the markers
    /// `c0000_a000_hi.txt` ... for `c0000_a000_hi.anibnd`; Elden Ring's characters `c2010_div00.blf`, `c2010_div01.blf`
    /// for `c2010_div00.anibnd` and `c2010_div01.anibnd`, the player's `.txt` as Sekiro's); whether a marker names a
    /// binder the archive holds is the caller's to check (two of Elden Ring's player markers name none). The
    /// `.blf` lists are read by [`crate::blf::LoadList`].
    pub fn division_markers(&self, prefix: &str) -> Vec<String> {
        let prefix = crate::names::fold(prefix);
        let mut out: Vec<String> = Vec::new();
        for file in &self.files {
            let Some(name) = file.name.as_deref() else {
                continue;
            };
            let leaf = crate::names::fold(name.rsplit(['/', '\\']).next().unwrap_or(name));
            let Some(stem) = leaf.strip_suffix(".txt").or_else(|| leaf.strip_suffix(".blf")) else {
                continue;
            };
            if stem.starts_with(&prefix) && !out.iter().any(|o| o == stem) {
                out.push(stem.to_owned());
            }
        }
        out
    }
    /// Validate a split index and list names without touching its payload. Full
    /// payload bounds and decompression are checked by `read_split` when loaded.
    pub fn split_names(header: &[u8]) -> Result<Vec<String>> {
        Ok(Self::read_inner(
            header,
            None,
            &Decoder {
                oodle: None,
                max_output: 0,
            },
            true,
            None,
        )?
        .files
        .into_iter()
        .filter_map(|f| f.name)
        .collect())
    }
    fn read_inner(
        data: &[u8],
        payload: Option<&[u8]>,
        decoder: &Decoder<'_>,
        index_only: bool,
        keep: Option<&dyn Fn(&str) -> bool>,
    ) -> Result<Self> {
        let split = payload.is_some() || index_only;
        let mut v = View {
            data,
            format: if split { "BHF4" } else { "BND4" },
            be: false,
            base: 0,
        };
        v.expect(0, if split { b"BHF4" } else { b"BND4" })?;
        v.boolean(4)?;
        v.boolean(5)?;
        v.expect(6, &[0, 0, 0])?;
        v.be = v.boolean(9)?;
        let bitbe = !v.boolean(10)?;
        v.expect(11, &[0])?;
        if v.u64(16)? != 64 {
            return Err(invalid("BND4", 16, "header size"));
        }
        let n = v.u32(12)? as usize;
        let version = v
            .slice(24, 8)?
            .try_into()
            .map_err(|_| invalid("BND4", 24, "version"))?;
        let record = v.size(32)?;
        let headerend = if split {
            v.expect(40, &[0; 8])?;
            data.len()
        } else {
            v.size(40)?
        };
        let pv = View {
            data: payload.unwrap_or(data),
            format: if split { "BDF4" } else { "BND4" },
            be: v.be,
            base: 0,
        };
        // what the records copy out of the file, and what they decode: each record is charged for the bytes it names, so
        // records that name one range over and over are refused before they are copied
        let budget = Budget::new("BND4", data.len().saturating_add(pv.data.len()));
        let decoded_budget = Budget::times("BND4", data.len().saturating_add(pv.data.len()), DECODED_TIMES);
        let mut payload_spans = Vec::new();
        if split && !index_only {
            pv.expect(0, b"BDF4")?;
            pv.boolean(4)?;
            pv.boolean(5)?;
            pv.expect(6, &[0; 3])?;
            if pv.boolean(9)? != v.be {
                return Err(invalid("BDF4", 9, "endian mismatch"));
            }
            pv.boolean(10)?;
            pv.expect(11, &[0; 5])?;
            if ![48, 64].contains(&pv.u64(16)?) {
                return Err(invalid("BDF4", 16, "header size"));
            }
            pv.expect(32, &[0; 16])?;
            payload_spans.push((0, 48));
        }
        v.slice(0, headerend)?;
        let unicode = v.boolean(48)?;
        let raw = v.u8(49)?;
        let format = if bitbe || raw & 1 != 0 && raw & 128 == 0 {
            raw
        } else {
            raw.reverse_bits()
        };
        if format & 0xc1 != 0 {
            return Err(invalid("BND4", 49, "unsupported format flags"));
        }
        let ids = format & 2 != 0;
        let names = format & 12 != 0;
        let long = format & 16 != 0;
        let compression = format & 32 != 0;
        let expected: usize = [
            16,
            if long { 8 } else { 4 },
            if compression { 8 } else { 0 },
            if ids { 4 } else { 0 },
            if names { 4 } else { 0 },
            if format == 4 { 8 } else { 0 },
        ]
        .into_iter()
        .sum();
        if record != expected {
            return Err(invalid("BND4", 32, "record size mismatch"));
        }
        let ext = v.u8(50)?;
        if ![0, 1, 4, 128].contains(&ext) || (split && ![0, 4].contains(&ext)) {
            return Err(invalid("BND4", 50, "extended flag"));
        }
        v.expect(51, &[0; 5])?;
        let hashptr = v.size(56)?;
        let tablebytes = n
            .checked_mul(record)
            .ok_or_else(|| invalid("BND4", 12, "count overflow"))?;
        v.slice(64, tablebytes)?;
        let mut spans = vec![(0, v.at(64, tablebytes)?)];
        let mut files = Vec::new();
        for i in 0..n {
            let p = v.element(64, i, record)?;
            let f = v.u8(p)?;
            let flags = if bitbe { f } else { f.reverse_bits() };
            if flags & !3 != 0 {
                return Err(invalid("BND4", p, "unknown file flags"));
            }
            v.expect(v.at(p, 1)?, &[0, 0, 0, 255, 255, 255, 255])?;
            let size = v.size(v.at(p, 8)?)?;
            let mut q = v.at(p, 16)?;
            let decoded = if compression {
                let s = v.size(q)?;
                q = v.at(q, 8)?;
                Some(s)
            } else {
                None
            };
            let off = if long {
                let x = v.size(q)?;
                q = v.at(q, 8)?;
                x
            } else {
                let x = v.u32(q)? as usize;
                q = v.at(q, 4)?;
                x
            };
            let mut id = if ids {
                let x = v.u32(q)? as i32;
                q = v.at(q, 4)?;
                Some(x)
            } else {
                None
            };
            let name = if names {
                let o = v.u32(q)? as usize;
                q = v.at(q, 4)?;
                let (text, end) = if unicode {
                    v.utf16z(o, &budget)?
                } else {
                    v.asciiz(o, &budget)?
                };
                if end > headerend {
                    return Err(invalid("BND4", o, "name outside header"));
                }
                spans.push((o, end.saturating_sub(o)));
                Some(text)
            } else {
                None
            };
            if format == 4 {
                id = Some(v.u32(q)? as i32);
                v.expect(v.at(q, 4)?, &[0; 4])?;
            }
            if index_only {
                if off < 48 {
                    return Err(invalid("BHF4", off, "file overlaps payload header"));
                }
                files.push(BinderFile {
                    id,
                    name,
                    flags,
                    bytes: Vec::new(),
                });
                continue;
            }
            let bytes = pv.slice(off, size)?;
            if off < if split { 48 } else { headerend } {
                return Err(invalid("BND4", off, "file overlaps header"));
            }
            budget.charge(off, size)?;
            if size > 0 {
                if split {
                    payload_spans.push((off, size));
                } else {
                    spans.push((off, size));
                }
            }
            // a file the caller does not keep is validated above and not copied or decoded
            if let (Some(keep), Some(name)) = (keep, name.as_deref())
                && !keep(name)
            {
                files.push(BinderFile {
                    id,
                    name: Some(name.to_owned()),
                    flags,
                    bytes: Vec::new(),
                });
                continue;
            }
            let bytes = if flags & 1 != 0 {
                let want = decoded.ok_or_else(|| invalid("BND4", p, "compressed file lacks output size"))?;
                if want > decoder.max_output {
                    return Err(invalid("BND4", p, "output limit"));
                }
                // a deflate stream cannot make more than this of what it holds
                if want > size.saturating_mul(DEFLATE_MAX_RATIO) {
                    return Err(invalid(
                        "BND4",
                        p,
                        "decoded size beyond what the stored bytes can inflate to",
                    ));
                }
                decoded_budget.charge(p, want)?;
                let b = if bytes.starts_with(b"DCX\0") {
                    decoder.decode(bytes)?
                } else {
                    Zlib.decompress(bytes, want)?
                };
                if b.len() != want {
                    return Err(invalid("BND4", p, "decoded size mismatch"));
                }
                b
            } else {
                bytes.to_vec()
            };
            let bytes = if bytes.starts_with(b"DCX\0") {
                decoded_budget.charge(off, Envelope::parse(&bytes, decoder.max_output)?.decoded_size)?;
                decoder.decode(&bytes)?
            } else {
                bytes
            };
            files.push(BinderFile {
                id,
                name,
                flags,
                bytes,
            });
        }
        if ext == 4 {
            v.slice(hashptr, 16)?;
            v.expect(v.at(hashptr, 12)?, &[16, 8, 8, 0])?;
            let hashes = v.size(hashptr)?;
            let hash_bytes = n
                .checked_mul(8)
                .ok_or_else(|| invalid("BND4", hashptr, "hash size overflow"))?;
            v.slice(hashes, hash_bytes)?;
            let buckets = v.u32(v.at(hashptr, 8)?)? as usize;
            let bucket_bytes = buckets
                .checked_mul(8)
                .ok_or_else(|| invalid("BND4", hashptr, "bucket overflow"))?;
            let bucket_table = v.at(hashptr, 16)?;
            v.slice(bucket_table, bucket_bytes)?;
            spans.push((hashptr, v.at(16, bucket_bytes)?));
            let mut total = 0usize;
            let mut seen = std::collections::BTreeSet::new();
            for b in 0..buckets {
                let p = v.element(bucket_table, b, 8)?;
                let count = v.u32(p)? as usize;
                let first = v.u32(v.at(p, 4)?)? as usize;
                if first != total || count > n.saturating_sub(total) {
                    return Err(invalid("BND4", p, "noncontiguous hash groups"));
                }
                for j in 0..count {
                    let h = v.element(hashes, first.saturating_add(j), 8)?;
                    let hash = v.u32(h)?;
                    let index = v.u32(v.at(h, 4)?)? as usize;
                    if index >= n || !seen.insert(index) || (hash as usize).checked_rem(buckets) != Some(b) {
                        return Err(invalid("BND4", h, "invalid hash index/bucket"));
                    }
                }
                total = total
                    .checked_add(count)
                    .ok_or_else(|| invalid("BND4", p, "hash overflow"))?;
            }
            if total != n {
                return Err(invalid("BND4", hashptr, "hash count mismatch"));
            }
            spans.push((hashes, hash_bytes));
            if !split && v.at(hashes, hash_bytes)? != headerend {
                return Err(invalid("BND4", 40, "header end mismatch"));
            }
        } else if hashptr != 0 {
            return Err(invalid("BND4", 56, "unexpected hash table"));
        }
        coverage(&v, spans, data.len(), 15)?;
        if split && !index_only {
            coverage(&pv, payload_spans, pv.data.len(), 15)?;
        }
        Ok(Self {
            version,
            format,
            files,
        })
    }
}

#[cfg(test)]
mod division_tests {
    use super::{Binder, BinderFile};

    fn binder(names: &[Option<&str>]) -> Binder {
        Binder {
            version: *b"07D7R6\0\0",
            format: 0x74,
            files: names
                .iter()
                .map(|n| BinderFile {
                    id: None,
                    name: n.map(str::to_owned),
                    flags: 0,
                    bytes: Vec::new(),
                })
                .collect(),
        }
    }

    /// Markers are the `.txt` and `.blf` members that start with the character's prefix, by stem, in order, once each; a
    /// marker of another character, a member without a name and the other members are not.
    #[test]
    fn division_markers_name_the_binders_of_a_character() {
        let b = binder(&[
            Some(r"N:\GR\data\Model\chr\c2010\hkx\skeleton.hkx"),
            Some(r"N:\GR\data\Model\chr\c2010\blf\c2010_div01.blf"),
            None,
            Some(r"N:\GR\data\Model\chr\c2010\blf\C2010_DIV00.BLF"),
            Some("c2010_div01.txt"),
            Some("delayload_c0000_c1020.txt"),
            Some("c2010.tae"),
            Some("c2010_a000_hi.hkx"),
        ]);
        assert_eq!(b.division_markers("c2010_"), ["c2010_div01", "c2010_div00"]);
        assert_eq!(b.division_markers("C2010_"), ["c2010_div01", "c2010_div00"]);
        assert!(b.division_markers("c0000_").is_empty());
        assert!(binder(&[]).division_markers("c2010_").is_empty());
    }
}
