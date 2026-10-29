//! DCX envelopes with DFLT (zlib), KRAK (Oodle Kraken) and ZSTD payloads. Only the header variants listed at
//! [`Envelope::parse`] are accepted; any other is an error naming the field.
use crate::{
    Result,
    bytes::View,
    put::{Out, narrow},
    invalid,
};
use primase_foundations::compress::{
    Decompressor, Error as CompressError, Zlib, Zstd,
    oodle::{Codec, Level, Oodle},
    oodle_framing,
};
use std::io::Write;
/// Where the payload starts in an envelope: after the 76-byte header every variant [`Envelope::parse`] accepts shares.
const PAYLOAD_START: usize = 76;
/// The four-character compression tag a DCX envelope names at offset 40 (`KRAK`, `DFLT`, `ZSTD`), or `None` for bytes
/// that do not start with a DCX header. The tag is not validated here; [`Envelope::parse`] does that.
pub fn compression(data: &[u8]) -> Option<[u8; 4]> {
    if !data.starts_with(b"DCX\0") {
        return None;
    }
    data.get(40..44)?.try_into().ok()
}
/// How an envelope's payload is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Oodle Kraken: needs the game's library.
    Kraken,
    /// zlib.
    Deflate,
    /// One zstd frame: Elden Ring's `regulation.bin` once decrypted.
    Zstd,
}

/// What an envelope's header says besides the two sizes: everything a writer needs to frame a payload the way the
/// game's tools did. The variants that occur are the associated constants; [`Envelope::parse`] lists those accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// How the payload is compressed.
    pub compression: Compression,
    /// The version word at offset 4: `0x10000` or `0x11000`.
    pub version: u32,
    /// The word at offset 16, the size of the `DCP` section: `0x24` (only DFLT at version `0x10000`) or `0x44`.
    pub dcp_size: u32,
    /// The compression level byte at offset 48: 6 on every shipped Kraken envelope, 9 on every shipped zlib one, 21 on
    /// Elden Ring's current `regulation.bin`.
    pub level: u8,
    /// The byte at offset 56: 0, or 15 on the zlib envelopes of Elden Ring's older `regulation.bin` files.
    pub flag: u8,
}

impl Format {
    /// Kraken at level 6: every Kraken envelope of both games (27,074 in Sekiro's archives, 117,121 in Elden Ring's).
    pub const KRAKEN: Format = Format {
        compression: Compression::Kraken,
        version: 0x11000,
        dcp_size: 0x44,
        level: 6,
        flag: 0,
    };
    /// zlib at level 9 in the framing of the older tool output: version `0x10000` and a `DCP` section of 0x24 bytes
    /// (12 envelopes of Sekiro's archives and 19 of Elden Ring's).
    pub const DEFLATE_V1: Format = Format {
        compression: Compression::Deflate,
        version: 0x10000,
        dcp_size: 0x24,
        level: 9,
        flag: 0,
    };
    /// zlib at level 9 in the newer framing, version `0x11000` (one envelope of Sekiro's archives).
    pub const DEFLATE: Format = Format {
        compression: Compression::Deflate,
        version: 0x11000,
        dcp_size: 0x44,
        level: 9,
        flag: 0,
    };
    /// zlib as the Elden Ring `regulation.bin` files 1.00.0 to 1.10 frame it: level 9 and the flag byte 15.
    pub const DEFLATE_REGULATION: Format = Format {
        compression: Compression::Deflate,
        version: 0x11000,
        dcp_size: 0x44,
        level: 9,
        flag: 15,
    };
    /// zstd at level 21, as Elden Ring's `regulation.bin` from 1.12.1 on is framed.
    pub const ZSTD: Format = Format {
        compression: Compression::Zstd,
        version: 0x11000,
        dcp_size: 0x44,
        level: 21,
        flag: 0,
    };

    /// The four-character tag the header names the codec with.
    fn tag(&self) -> &'static [u8; 4] {
        match self.compression {
            Compression::Kraken => b"KRAK",
            Compression::Deflate => b"DFLT",
            Compression::Zstd => b"ZSTD",
        }
    }

    /// Refuses a combination [`Envelope::parse`] would refuse: a writer produces only what the reader reads.
    pub fn validate(&self) -> Result<()> {
        let ok = match self.compression {
            Compression::Kraken => {
                self.version == 0x11000
                    && self.dcp_size == 0x44
                    && [6, 9].contains(&self.level)
                    && self.flag == 0
            }
            Compression::Deflate => matches!(
                (self.version, self.dcp_size, self.level, self.flag),
                (0x10000, 0x24 | 0x44, 9, 0) | (0x11000, 0x44, 8 | 9, 0) | (0x11000, 0x44, 9, 15)
            ),
            Compression::Zstd => self.version == 0x11000 && self.dcp_size == 0x44 && self.flag == 0,
        };
        if ok {
            Ok(())
        } else {
            Err(invalid(
                "DCX",
                4,
                format!("unsupported envelope variant {self:?}"),
            ))
        }
    }
}

/// A validated DCX envelope: how its payload is compressed, the payload as stored and the size it decodes to.
#[derive(Debug, Clone, Copy)]
pub struct Envelope<'a> {
    /// How the payload is compressed.
    pub compression: Compression,
    /// The header's other variant fields, so that the same envelope can be written again ([`frame`], [`encode`]).
    pub format: Format,
    /// The compressed payload, borrowed from the input.
    pub payload: &'a [u8],
    /// The size it decodes to, as the header states it.
    pub decoded_size: usize,
}

impl<'a> Envelope<'a> {
    /// Reads the envelope's header and accepts only the variants it knows: `KRAK` (version `0x11000`, level
    /// 6 or 9), `DFLT` (version `0x10000` level 9, or `0x11000` level 8 or 9) and `ZSTD` (version `0x11000`). A decoded
    /// size over `max_output`, a payload that overruns the data or trailing bytes other than up to 15 zero bytes of
    /// alignment are refused, naming the offset.
    pub fn parse(data: &'a [u8], max_output: usize) -> Result<Self> {
        let v = View {
            data,
            format: "DCX",
            be: true,
            base: 0,
        };
        v.expect(0, b"DCX\0")?;
        for (p, value) in [
            (8, 0x18),
            (12, 0x24),
            (44, 0x20),
            (52, 0),
            (60, 0),
            (64, 0x10100),
            (72, 8),
        ] {
            if v.u32(p)? != value {
                return Err(invalid("DCX", p, "unsupported header field"));
            }
        }
        v.expect(24, b"DCS\0")?;
        v.expect(36, b"DCP\0")?;
        v.expect(68, b"DCA\0")?;
        v.expect(49, &[0, 0, 0])?;
        v.expect(57, &[0, 0, 0])?;
        let version = v.u32(4)?;
        let a = v.u32(16)?;
        let b = v.u32(20)?;
        let level = v.u8(48)?;
        let flag = v.u8(56)?;
        if b != if a == 0x24 { 0x2c } else { 0x4c } {
            return Err(invalid("DCX", 20, "header offsets"));
        }
        let decoded_size = v.u32(28)? as usize;
        let size = v.u32(32)? as usize;
        if decoded_size > max_output {
            return Err(invalid("DCX", 28, "output exceeds configured limit"));
        }
        let payload = v.slice(PAYLOAD_START, size)?;
        let end = v.at(PAYLOAD_START, size)?;
        if data.len().saturating_sub(end) >= 16
            || v.slice(end, data.len().saturating_sub(end))?
                .iter()
                .any(|&x| x != 0)
        {
            return Err(invalid(
                "DCX",
                end,
                "trailing data, expected at most 15 zero alignment bytes",
            ));
        }
        let compression = match v.slice(40, 4)? {
            b"KRAK" => {
                if version != 0x11000 || a != 0x44 || ![6, 9].contains(&level) || flag != 0 {
                    return Err(invalid("DCX", 4, "unknown KRAK variant"));
                }
                Compression::Kraken
            }
            b"DFLT" => {
                if !matches!(
                    (version, a, level, flag),
                    (0x10000, 0x24 | 0x44, 9, 0) | (0x11000, 0x44, 8 | 9, 0) | (0x11000, 0x44, 9, 15)
                ) {
                    return Err(invalid("DCX", 4, "unknown DFLT variant"));
                }
                Compression::Deflate
            }
            // SoulsFormats' `DCX.DecompressDCXZSTD`: KRAK's header with any level byte
            b"ZSTD" => {
                if version != 0x11000 || a != 0x44 || flag != 0 {
                    return Err(invalid("DCX", 4, "unknown ZSTD variant"));
                }
                Compression::Zstd
            }
            _ => return Err(invalid("DCX", 40, "unsupported compression")),
        };
        Ok(Self {
            compression,
            format: Format {
                compression,
                version,
                dcp_size: a,
                level,
                flag,
            },
            payload,
            decoded_size,
        })
    }
}

/// An envelope of `format` around `payload`, which decodes to `decoded_size` bytes: the 76-byte header and the payload
/// as given, nothing compressed and no padding after it. The header of every envelope [`Envelope::parse`] reads from the
/// installs is exactly what this writes from the fields the reader returns.
pub fn frame(format: &Format, decoded_size: usize, payload: &[u8]) -> Result<Vec<u8>> {
    format.validate()?;
    let mut out = Out::new("DCX", true);
    out.bytes(b"DCX\0");
    out.u32(format.version);
    out.u32(0x18);
    out.u32(0x24);
    out.u32(format.dcp_size);
    out.u32(if format.dcp_size == 0x24 { 0x2c } else { 0x4c });
    out.bytes(b"DCS\0");
    out.u32(narrow("DCX", "the decoded size", decoded_size)?);
    out.u32(narrow("DCX", "the payload size", payload.len())?);
    out.bytes(b"DCP\0");
    out.bytes(format.tag());
    out.u32(0x20);
    out.u8(format.level);
    out.zeros(3);
    out.u32(0);
    out.u8(format.flag);
    out.zeros(3);
    out.u32(0);
    out.u32(0x10100);
    out.bytes(b"DCA\0");
    out.u32(8);
    debug_assert_eq!(out.len(), PAYLOAD_START);
    out.bytes(payload);
    Ok(out.finish())
}

/// What compresses a payload: the Oodle library, needed for Kraken envelopes only. zlib and zstd need nothing from the
/// caller.
#[derive(Debug, Clone, Copy)]
pub struct Encoder<'a> {
    /// The Oodle codec the caller opened; `None` makes a Kraken envelope an error.
    pub oodle: Option<&'a Oodle>,
}

/// `decoded` compressed as `format` says and framed ([`frame`]).
///
/// The compressors are not the ones that made the shipped files, so the payload differs from theirs while decoding to
/// the same bytes (and the header equals theirs): Kraken goes through the game's library at the level the header
/// states (`format.level`, an Oodle compression level, 6 in every shipped Kraken envelope but those that say 9; one
/// outside the library's -4 to 9 is refused; the sizes at 6 come out within about 1 per cent of the shipped ones); zlib
/// is flate2's pure-Rust deflate at `format.level` (the real zlib at level 9 reproduces 48 of the 58 shipped zlib
/// payloads, this implementation none); zstd is libzstd's streaming encoder at `format.level` with the frame
/// parameters of the shipped `regulation.bin` (no content size, no checksum, the 64 MiB window of level 21).
pub fn encode(format: &Format, decoded: &[u8], encoder: &Encoder<'_>) -> Result<Vec<u8>> {
    format.validate()?;
    let payload = match format.compression {
        Compression::Kraken => {
            let level = Level::new(i32::from(format.level)).ok_or_else(|| {
                invalid(
                    "DCX",
                    48,
                    format!("Oodle level {} is outside the library's -4 to 9", format.level),
                )
            })?;
            encoder
                .oodle
                .ok_or_else(|| invalid("DCX", 40, "Oodle required"))?
                .compress_at(Codec::KRAKEN, level, decoded)?
        }
        Compression::Deflate => {
            let mut z = flate2::write::ZlibEncoder::new(
                Vec::new(),
                flate2::Compression::new(u32::from(format.level)),
            );
            z.write_all(decoded)?;
            z.finish()?
        }
        Compression::Zstd => {
            let mut z = zstd::stream::write::Encoder::new(Vec::new(), i32::from(format.level))?;
            z.include_checksum(false)?;
            z.include_contentsize(false)?;
            z.write_all(decoded)?;
            z.finish()?
        }
    };
    frame(format, decoded.len(), &payload)
}

/// What decodes an envelope: the optional Oodle decoder (needed for Kraken envelopes only) and the largest decoded size
/// in bytes the caller allows, so that a hostile size field cannot demand an allocation.
#[derive(Debug, Clone, Copy)]
pub struct Decoder<'a> {
    /// The Oodle decoder the caller opened (`primase_foundations::compress::oodle::Oodle`); `None` makes a Kraken
    /// envelope an error.
    pub oodle: Option<&'a Oodle>,
    /// The largest decoded size in bytes an envelope may state.
    pub max_output: usize,
}
/// Refuses an Oodle `payload` whose blocks and quanta do not consume exactly its bytes for `decoded_size`, naming the
/// offset in the envelope.
fn check_framing(payload: &[u8], decoded_size: usize) -> Result<()> {
    match oodle_framing::walk(payload, decoded_size) {
        Ok(_) => Ok(()),
        Err(CompressError::Invalid { offset, detail, .. }) => Err(invalid(
            "DCX",
            PAYLOAD_START.saturating_add(offset),
            format!("Oodle payload: {detail}"),
        )),
        Err(e) => Err(e.into()),
    }
}
impl Decoder<'_> {
    /// Decode the DCX envelope `data`; anything else is an error.
    pub fn decode(&self, data: &[u8]) -> Result<Vec<u8>> {
        let envelope = Envelope::parse(data, self.max_output)?;
        match envelope.compression {
            Compression::Kraken => {
                // The library accepts bytes after the last block, so the framing is checked before it decodes.
                check_framing(envelope.payload, envelope.decoded_size)?;
                Ok(self
                    .oodle
                    .ok_or_else(|| invalid("DCX", 40, "Oodle required"))?
                    .decompress(envelope.payload, envelope.decoded_size)?)
            }
            Compression::Deflate => Ok(Zlib.decompress(envelope.payload, envelope.decoded_size)?),
            Compression::Zstd => Ok(Zstd.decompress(envelope.payload, envelope.decoded_size)?),
        }
    }
    /// Decode `data` if it starts with a DCX envelope, else return it unchanged.
    pub fn decode_if_dcx(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.starts_with(b"DCX\0") {
            self.decode(data)
        } else {
            Ok(data.to_vec())
        }
    }
}
#[cfg(test)]
mod compression_tests {
    use super::{Compression, Decoder, Encoder, Envelope, Format, encode, frame};

    /// A KRAK envelope (the layout of a 0x11000 `DCX` with the 0x44 `DCP` block, level 9) around `payload`.
    fn krak(decoded: u32, payload: &[u8]) -> Vec<u8> {
        envelope(b"KRAK", decoded, payload)
    }

    /// An envelope of `kind` laid out as [`krak`]'s.
    fn envelope(kind: &[u8; 4], decoded: u32, payload: &[u8]) -> Vec<u8> {
        let mut b = b"DCX\0".to_vec();
        for (at, word) in [(4, 0x11000u32), (8, 0x18), (12, 0x24), (16, 0x44), (20, 0x4c)] {
            b.resize(at, 0);
            b.extend(word.to_be_bytes());
        }
        b.extend(*b"DCS\0");
        b.extend(decoded.to_be_bytes());
        b.extend((payload.len() as u32).to_be_bytes());
        b.extend(*b"DCP\0");
        b.extend(*kind);
        b.extend(0x20u32.to_be_bytes());
        b.extend([9, 0, 0, 0]);
        b.extend(0u32.to_be_bytes());
        b.extend([0, 0, 0, 0]);
        b.extend(0u32.to_be_bytes());
        b.extend(0x10100u32.to_be_bytes());
        b.extend(*b"DCA\0");
        b.extend(8u32.to_be_bytes());
        b.extend(payload);
        b
    }

    #[test]
    fn an_envelope_names_its_payload_and_decoded_size() {
        let bytes = krak(1000, &[1, 2, 3]);
        let e = Envelope::parse(&bytes, usize::MAX).unwrap();
        assert_eq!(e.compression, Compression::Kraken);
        assert_eq!((e.decoded_size, e.payload), (1000, &[1, 2, 3][..]));
        // a decoded size over the ceiling, and trailing bytes that are not alignment zeros, are refused
        assert!(Envelope::parse(&bytes, 999).is_err());
        let mut dirty = bytes;
        dirty.push(1);
        assert!(Envelope::parse(&dirty, usize::MAX).is_err());
    }

    /// An Oodle payload for 10 decoded bytes: one Kraken block (`8C 06`) holding one quantum whose three-byte header
    /// (compressed size minus one, 18 bits) states a four-byte body. Only the framing is real; the body is not decodable.
    const FRAMED: [u8; 9] = [0x8C, 0x06, 0, 0, 3, 1, 2, 3, 4];

    /// The refusal `decode` gives an envelope around `payload`, which no Oodle library is given to decode: either the
    /// framing check's (offset, detail) or the error for the missing library.
    fn refusal(kind: &[u8; 4], decoded: u32, payload: &[u8]) -> (usize, String) {
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        match decoder.decode(&envelope(kind, decoded, payload)) {
            Err(crate::Error::Invalid {
                format: "DCX",
                offset,
                detail,
            }) => (offset, detail),
            other => panic!("expected a DCX refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_kraken_payload_must_end_where_its_blocks_end() {
        // exactly framed: the framing check passes and only the missing library stops the decode
        assert_eq!(refusal(b"KRAK", 10, &FRAMED), (40, "Oodle required".into()));
        // a byte appended: refused before decoding, at the offset in the envelope where the blocks end
        let mut long = FRAMED.to_vec();
        long.push(0);
        let (offset, detail) = refusal(b"KRAK", 10, &long);
        assert_eq!(offset, 76 + FRAMED.len());
        assert!(detail.contains("1 trailing bytes"), "{detail}");
        // a byte short: the last quantum's body runs past the payload
        let (offset, detail) = refusal(b"KRAK", 10, &FRAMED[..FRAMED.len() - 1]);
        assert!(offset >= 76, "{offset}");
        assert!(detail.starts_with("Oodle payload: "), "{detail}");
        assert_ne!(detail, "Oodle required");
        // a stored block (`CC 06`, raw bytes) is framed by the decoded size alone
        let mut stored = vec![0xCC, 0x06];
        stored.extend([7; 10]);
        assert_eq!(refusal(b"KRAK", 10, &stored), (40, "Oodle required".into()));
        stored.pop();
        assert_ne!(refusal(b"KRAK", 10, &stored).1, "Oodle required");
        // a header that is not a block header, and bytes for a stream that decodes to nothing
        assert_ne!(refusal(b"KRAK", 10, &[0, 0, 0, 0]).1, "Oodle required");
        assert_ne!(refusal(b"KRAK", 0, &[0x8C, 0x06]).1, "Oodle required");
    }

    #[test]
    fn other_payloads_are_not_walked_as_oodle() {
        // a DFLT or ZSTD envelope around Oodle-shaped bytes fails in its own codec, not in the Oodle framing check
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        for kind in [b"DFLT", b"ZSTD"] {
            assert!(matches!(
                decoder.decode(&envelope(kind, 10, &FRAMED)),
                Err(crate::Error::Compression(_))
            ));
        }
    }

    #[test]
    fn a_zstd_envelope_decodes_its_frame() {
        let text = b"NpcParam.param";
        // one zstd frame (RFC 8878): magic, a single-segment descriptor with a one-byte content size, and one raw block
        // (last, type 0, its size in the top bits of the three-byte header)
        let mut frame = vec![0x28, 0xb5, 0x2f, 0xfd, 0x20, text.len() as u8];
        frame.extend(&(((text.len() as u32) << 3) | 1).to_le_bytes()[..3]);
        frame.extend(text);
        let bytes = envelope(b"ZSTD", text.len() as u32, &frame);
        assert_eq!(
            Envelope::parse(&bytes, usize::MAX).unwrap().compression,
            Compression::Zstd
        );
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        assert_eq!(decoder.decode(&bytes).unwrap(), text);
        // a size the frame does not decode to is refused
        assert!(decoder.decode(&envelope(b"ZSTD", 3, &frame)).is_err());
    }

    #[test]
    fn the_envelope_names_its_compression() {
        let mut krak = b"DCX\0".to_vec();
        krak.resize(40, 0);
        krak.extend_from_slice(b"KRAK");
        assert_eq!(super::compression(&krak), Some(*b"KRAK"));
        assert_eq!(super::compression(&krak[..42]), None);
        assert_eq!(super::compression(b"BND4"), None);
    }

    #[test]
    fn frame_writes_the_header_the_reader_reads_for_every_variant() {
        let kraken9 = Format {
            level: 9,
            ..Format::KRAKEN
        };
        for format in [
            Format::KRAKEN,
            kraken9,
            Format::DEFLATE_V1,
            Format {
                dcp_size: 0x44,
                ..Format::DEFLATE_V1
            },
            Format::DEFLATE,
            Format {
                level: 8,
                ..Format::DEFLATE
            },
            Format::DEFLATE_REGULATION,
            Format::ZSTD,
        ] {
            let bytes = frame(&format, 1000, &[1, 2, 3]).unwrap();
            let e = Envelope::parse(&bytes, usize::MAX).unwrap();
            assert_eq!(
                (e.format, e.decoded_size, e.payload),
                (format, 1000, &[1, 2, 3][..])
            );
            assert_eq!(e.compression, format.compression);
            assert_eq!(bytes.len(), 79);
        }
        // the envelope this module's tests build by hand is what `frame` writes
        assert_eq!(frame(&kraken9, 1000, &[1, 2, 3]).unwrap(), krak(1000, &[1, 2, 3]));
        // a variant the reader refuses is not written
        for bad in [
            Format {
                level: 7,
                ..Format::KRAKEN
            },
            Format {
                version: 0x12000,
                ..Format::ZSTD
            },
            Format {
                flag: 1,
                ..Format::DEFLATE
            },
            Format {
                dcp_size: 0x24,
                ..Format::KRAKEN
            },
        ] {
            assert!(frame(&bad, 1, &[]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn encoded_zlib_and_zstd_envelopes_decode_to_their_input() {
        let decoded: Vec<u8> = (0..20_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        for format in [
            Format::DEFLATE_V1,
            Format::DEFLATE,
            Format::DEFLATE_REGULATION,
            Format::ZSTD,
        ] {
            let bytes = encode(&format, &decoded, &Encoder { oodle: None }).unwrap();
            let e = Envelope::parse(&bytes, usize::MAX).unwrap();
            assert_eq!((e.format, e.decoded_size), (format, decoded.len()));
            assert!(
                e.payload.len() < decoded.len() / 2,
                "{format:?} compresses this input"
            );
            assert_eq!(decoder.decode(&bytes).unwrap(), decoded, "{format:?}");
        }
        // the zstd frame states no content size, no checksum and no dictionary, as the shipped regulation frames
        let z = encode(&Format::ZSTD, &decoded, &Encoder { oodle: None }).unwrap();
        assert_eq!(&z[76..80], &[0x28, 0xb5, 0x2f, 0xfd]);
        assert_eq!(
            z[80] & 0xe7,
            0,
            "frame header descriptor: no size, checksum or dictionary id"
        );
        // Kraken needs the library
        let e = encode(&Format::KRAKEN, &decoded, &Encoder { oodle: None }).unwrap_err();
        assert_eq!(e.to_string(), "DCX at 0x28: Oodle required");
    }

    /// The error a decode gives, or `None` where it succeeds; a Kraken envelope whose framing passes ends at the missing
    /// library, which is "success" for this test.
    fn refused_by(decoder: &Decoder<'_>, bytes: &[u8]) -> Option<String> {
        match decoder.decode(bytes) {
            Ok(_) => None,
            Err(e) if e.to_string() == "DCX at 0x28: Oodle required" => None,
            Err(e) => Some(e.to_string()),
        }
    }

    /// Each size word one more and one less, a payload one byte short and one byte over: refused, for every codec.
    #[test]
    fn a_size_word_one_off_or_a_payload_one_byte_off_is_refused() {
        let decoded: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        // Kraken: one stored block (`CC 06`, raw bytes), which the framing walk checks without the library
        let mut stored = vec![0xCC, 0x06];
        stored.extend(&decoded);
        let cases = [
            (
                Format::DEFLATE,
                encode(&Format::DEFLATE, &decoded, &Encoder { oodle: None }).unwrap(),
            ),
            (
                Format::ZSTD,
                encode(&Format::ZSTD, &decoded, &Encoder { oodle: None }).unwrap(),
            ),
            (
                Format::KRAKEN,
                frame(&Format::KRAKEN, decoded.len(), &stored).unwrap(),
            ),
        ];
        for (format, mut good) in cases {
            // a zero byte after the payload is alignment, so a payload size one over takes it
            good.push(0);
            assert_eq!(refused_by(&decoder, &good), None, "{format:?} as framed");
            let word = |bytes: &[u8], at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
            let (dsize, csize) = (word(&good, 28), word(&good, 32));
            for (at, was) in [(28, dsize), (32, csize)] {
                for now in [was - 1, was + 1] {
                    let mut bad = good.clone();
                    bad[at..at + 4].copy_from_slice(&now.to_be_bytes());
                    assert!(
                        refused_by(&decoder, &bad).is_some(),
                        "{format:?}: the word at {at} is {now}, not {was}"
                    );
                }
            }
            // the payload cut by one byte (its size word kept, so the envelope ends early), and one byte over
            let mut short = good.clone();
            short.truncate(short.len() - 2);
            assert!(refused_by(&decoder, &short).is_some(), "{format:?} short");
            let mut over = good.clone();
            over.push(1);
            assert!(refused_by(&decoder, &over).is_some(), "{format:?} over");
        }
    }
}
