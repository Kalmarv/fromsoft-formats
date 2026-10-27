//! A split binder (a BHF4 index and its BDF4 payload) read keeping only the files a caller names: the others are
//! checked as a full read checks them and come back with no bytes.
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

/// A BHF4 index of one file `a` (id 7, 20 bytes) and the BDF4 payload that holds it.
fn split() -> (Vec<u8>, Vec<u8>) {
    let mut header = vec![0; 116];
    header[..4].copy_from_slice(b"BHF4");
    header[10] = 1;
    u32le(&mut header, 12, 1);
    u64le(&mut header, 16, 64);
    u64le(&mut header, 32, 28);
    u64le(&mut header, 40, 0);
    header[48] = 1;
    header[49] = 6u8.reverse_bits();
    header[64] = 2u8.reverse_bits();
    header[68..72].fill(255);
    u64le(&mut header, 72, 20);
    u32le(&mut header, 80, 48);
    u32le(&mut header, 84, 7);
    u32le(&mut header, 88, 92);
    header[92] = b'a';
    header.truncate(96);
    let mut payload = vec![0; 48];
    payload[..4].copy_from_slice(b"BDF4");
    payload[10] = 1;
    u64le(&mut payload, 16, 48);
    payload.extend(b"twenty payload bytes");
    (header, payload)
}

#[test]
fn read_split_where_keeps_the_named_files_and_checks_the_rest() {
    let (header, payload) = split();
    let full = Binder::read_split(&header, &payload, &decoder()).expect("split binder");
    assert_eq!(full.files[0].bytes, b"twenty payload bytes");
    let kept = Binder::read_split_where(&header, &payload, &decoder(), &|name| name == "a").expect("kept");
    assert_eq!(kept.files[0].bytes, b"twenty payload bytes");
    let left = Binder::read_split_where(&header, &payload, &decoder(), &|_| false).expect("left out");
    assert_eq!(left.files[0].name.as_deref(), Some("a"));
    assert_eq!(left.files[0].id, Some(7));
    assert!(left.files[0].bytes.is_empty());
    // a file left out is still checked: a payload cut anywhere is refused
    for n in 0..payload.len() {
        assert!(Binder::read_split_where(&header, &payload[..n], &decoder(), &|_| false).is_err());
    }
}
