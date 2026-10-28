//! Elden Ring's `regulation.bin` as a file: AES-256 in CBC mode under the game's regulation key, the first 16 bytes of the
//! file the initialisation vector, around a DCX envelope (zstd from 1.12.1 on, zlib before) of the param binder,
//! zero-padded to whole AES blocks. The crate stores no key: the caller passes the 32 bytes ([`crate::Keys::regulation`]).
//!
//! [`decrypt`] and [`encrypt`] are inverses on the file's bytes, and all 29 regulations of the installed game and of
//! the Smithbox collection (1.00.0 to 1.17.3) encrypt back to the same bytes with the vector they were stored with.
//! [`pack`] builds a file from a binder.
use aes::{
    Aes256,
    cipher::{Array, BlockCipherEncrypt, KeyInit},
};

use crate::dcx::{self, Encoder, Format};
use crate::{Binder, Result, invalid};

const FORMAT: &str = "regulation";

/// The initialisation vector a `regulation.bin` starts with.
pub fn iv(file: &[u8]) -> Result<[u8; 16]> {
    file.first_chunk::<16>()
        .copied()
        .ok_or_else(|| invalid(FORMAT, 0, "shorter than its initialisation vector"))
}

/// The plain bytes of `file`: the DCX envelope and the zero bytes that pad it to whole blocks. The vector is not part of
/// the result; [`iv`] reads it.
pub fn decrypt(key: &[u8; 32], file: &[u8]) -> Result<Vec<u8>> {
    crate::game_params::decrypt_cbc(key, file)
}

/// `plain` (zero-padded to whole blocks) encrypted under `key` in CBC mode, preceded by `iv`: the bytes of a
/// `regulation.bin`.
pub fn encrypt(key: &[u8; 32], iv: &[u8; 16], plain: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256::new(&Array::from(*key));
    let mut out = Vec::with_capacity(plain.len().saturating_add(32));
    out.extend_from_slice(iv);
    let mut previous = *iv;
    let (blocks, rest) = plain.as_chunks::<16>();
    let mut seal = |block: &[u8; 16]| {
        let mut mixed = [0u8; 16];
        for ((m, b), p) in mixed.iter_mut().zip(block).zip(previous) {
            *m = b ^ p;
        }
        let mut b = Array::from(mixed);
        cipher.encrypt_block(&mut b);
        previous = b.into();
        out.extend_from_slice(&previous);
    };
    for block in blocks {
        seal(block);
    }
    if !rest.is_empty() {
        let mut last = [0u8; 16];
        for (l, r) in last.iter_mut().zip(rest) {
            *l = *r;
        }
        seal(&last);
    }
    Ok(out)
}

/// A `regulation.bin` holding `binder`: its BND4 bytes ([`Binder::write`]) in a DCX envelope of `format` (zstd for the
/// current game, [`Format::ZSTD`]), padded and encrypted with `iv` under `key`. The compressor is not the tool's, so the
/// bytes differ from the shipped file's while it decrypts and decodes to the same binder.
pub fn pack(
    key: &[u8; 32],
    iv: &[u8; 16],
    binder: &Binder,
    format: &Format,
    encoder: &Encoder<'_>,
) -> Result<Vec<u8>> {
    let bytes = binder.write(encoder)?;
    let envelope = dcx::encode(format, &bytes, encoder)?;
    encrypt(key, iv, &envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binder::{BinderFile, Layout};
    use crate::dcx::Decoder;

    #[test]
    fn encrypting_what_was_decrypted_gives_the_file_back_and_pads_to_blocks() {
        let key = [9u8; 32];
        let iv = [4u8; 16];
        let plain: Vec<u8> = (0..64u8).collect();
        let file = encrypt(&key, &iv, &plain).unwrap();
        assert_eq!(file.len(), 16 + 64);
        assert_eq!(super::iv(&file).unwrap(), iv);
        assert_eq!(decrypt(&key, &file).unwrap(), plain);
        assert_eq!(encrypt(&key, &iv, &decrypt(&key, &file).unwrap()).unwrap(), file);
        // bytes short of a block are zero-padded
        let odd = encrypt(&key, &iv, &plain[..50]).unwrap();
        let mut padded = plain[..50].to_vec();
        padded.resize(64, 0);
        assert_eq!(decrypt(&key, &odd).unwrap(), padded);
        assert!(super::iv(&file[..8]).is_err());
    }

    #[test]
    fn a_packed_regulation_decrypts_and_decodes_to_its_binder() {
        let key = [3u8; 32];
        let iv = [8u8; 16];
        let binder = Binder {
            version: *b"11730034",
            format: 0x2e,
            files: vec![BinderFile {
                id: Some(0),
                name: Some("N:\\GR\\data\\Param\\param\\GameParam\\NpcParam.param".to_owned()),
                flags: 2,
                bytes: vec![5; 300],
                wrap: None,
            }],
            layout: Layout::default(),
        };
        let file = pack(&key, &iv, &binder, &Format::ZSTD, &Encoder { oodle: None }).unwrap();
        assert_eq!(super::iv(&file).unwrap(), iv);
        let decoder = Decoder {
            oodle: None,
            max_output: 1 << 20,
        };
        let read = crate::game_params::regulation_binder(&key, &file, &decoder).unwrap();
        assert_eq!(read.regulation_number(), Some(11_730_034));
        assert_eq!(read.files[0].bytes, vec![5; 300]);
    }
}
