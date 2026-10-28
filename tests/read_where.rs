//! A BND4 binder read keeping only the files a caller names: the others are checked as a full read checks them and come
//! back with no bytes, so a binder of many members decodes and copies only the ones wanted.
use fromsoft_formats::{Binder, dcx::Decoder};

fn u32le(b: &mut [u8], p: usize, x: u32) {
    b[p..p + 4].copy_from_slice(&x.to_le_bytes());
}
fn u64le(b: &mut [u8], p: usize, x: u64) {
    b[p..p + 8].copy_from_slice(&x.to_le_bytes());
}
fn decoder() -> Decoder<'static> {
    Decoder {
        oodle: None,
        max_output: 1024,
    }
}

/// A BND4 of the files `(name, payload)`: ids 10, 11, ..., stored plainly, ASCII names.
fn binder(files: &[(&str, &[u8])]) -> Vec<u8> {
    let n = files.len();
    let name_at = 64 + n * 28;
    let names: usize = files.iter().map(|(name, _)| name.len() + 1).sum();
    let header_end = name_at + names;
    let payloads: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
    let mut b = vec![0u8; header_end + payloads];
    b[..4].copy_from_slice(b"BND4");
    b[10] = 1;
    u32le(&mut b, 12, n as u32);
    u64le(&mut b, 16, 64);
    u64le(&mut b, 32, 28);
    u64le(&mut b, 40, header_end as u64);
    b[49] = 6u8.reverse_bits();
    let (mut name_p, mut data_p) = (name_at, header_end);
    for (i, (name, bytes)) in files.iter().enumerate() {
        let p = 64 + i * 28;
        b[p + 4..p + 8].fill(255);
        u64le(&mut b, p + 8, bytes.len() as u64);
        u32le(&mut b, p + 16, data_p as u32);
        u32le(&mut b, p + 20, 10 + i as u32);
        u32le(&mut b, p + 24, name_p as u32);
        b[name_p..name_p + name.len()].copy_from_slice(name.as_bytes());
        b[data_p..data_p + bytes.len()].copy_from_slice(bytes);
        name_p += name.len() + 1;
        data_p += bytes.len();
    }
    b
}

#[test]
fn read_where_keeps_the_named_files_and_checks_the_rest() {
    let bytes = binder(&[
        ("a.bin", b"first payload"),
        ("b.bin", b"second"),
        ("c.bin", b"third one"),
    ]);
    let full = Binder::read(&bytes, &decoder()).expect("binder");
    assert_eq!(full.files.len(), 3);
    assert_eq!(full.files[1].bytes, b"second");
    let kept = Binder::read_where(&bytes, &decoder(), &|name| name == "b.bin").expect("kept");
    assert_eq!(kept.files.len(), 3);
    for (kept, full) in kept.files.iter().zip(&full.files) {
        assert_eq!(
            (kept.id, &kept.name, kept.flags),
            (full.id, &full.name, full.flags)
        );
    }
    assert!(kept.files[0].bytes.is_empty() && kept.files[2].bytes.is_empty());
    assert_eq!(kept.files[1].bytes, full.files[1].bytes);
    let all = Binder::read_where(&bytes, &decoder(), &|_| true).expect("all");
    assert_eq!(
        all.files.iter().map(|f| &f.bytes).collect::<Vec<_>>(),
        full.files.iter().map(|f| &f.bytes).collect::<Vec<_>>()
    );
    // a file left out is still checked: a binder cut anywhere is refused, as the full read refuses it
    for n in 0..bytes.len() {
        assert_eq!(
            Binder::read_where(&bytes[..n], &decoder(), &|_| false).is_err(),
            Binder::read(&bytes[..n], &decoder()).is_err(),
            "cut at {n}"
        );
    }
}
