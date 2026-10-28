use fromsoft_formats::mtd::MaterialDefinition;
fn word(b: &mut Vec<u8>, x: u32) {
    b.extend(x.to_le_bytes());
}
fn marker(b: &mut Vec<u8>, x: u8) {
    b.push(x);
    b.resize(b.len().next_multiple_of(4), 0);
}
fn string(b: &mut Vec<u8>, s: &[u8], m: u8) {
    word(b, s.len() as u32);
    b.extend(s);
    marker(b, m);
}
fn begin(b: &mut Vec<u8>, kind: u32, version: u32, m: u8) -> usize {
    let p = b.len();
    word(b, 0);
    word(b, 0);
    word(b, kind);
    word(b, version);
    marker(b, m);
    p
}
fn end(b: &mut [u8], p: usize) {
    let n = (b.len() - p - 8) as u32;
    b[p + 4..p + 8].copy_from_slice(&n.to_le_bytes());
}
fn fixture() -> Vec<u8> {
    let mut b = vec![];
    let file = begin(&mut b, 0, 3, 1);
    let header = begin(&mut b, 1, 2, 0xb0);
    string(&mut b, b"MTD ", 0x34);
    word(&mut b, 1000);
    end(&mut b, header);
    marker(&mut b, 1);
    let data = begin(&mut b, 2, 4, 0xa3);
    string(&mut b, b"synthetic.sps", 0xa3);
    string(&mut b, &[0x93, 0xfa, 0x96, 0x7b], 3);
    word(&mut b, 1);
    let lists = begin(&mut b, 3, 4, 0xa3);
    word(&mut b, 0);
    marker(&mut b, 3);
    word(&mut b, 0);
    marker(&mut b, 3);
    word(&mut b, 0);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, lists);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, data);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, file);
    b
}
#[test]
fn strict_shift_jis_and_empty_lists() {
    let m = MaterialDefinition::read(&fixture()).unwrap();
    assert_eq!(m.description, "日本");
    assert_eq!(m.shader, "synthetic.sps");
    assert!(m.parameters.is_empty() && m.textures.is_empty());
}
#[test]
fn malformed_text_is_an_error_at_the_string() {
    let mut b = fixture();
    let p = b.windows(4).position(|x| x == [0x93, 0xfa, 0x96, 0x7b]).unwrap();
    b[p..p + 4].copy_from_slice(&[0x81, 0x30, 0x96, 0x7b]);
    let error = MaterialDefinition::read(&b).unwrap_err().to_string();
    assert!(
        error.contains("Shift-JIS") && error.contains(&format!("{p:#x}")),
        "{error}"
    );
}
#[test]
fn rejects_truncation_and_trailing_bytes_but_records_declared_lengths() {
    let b = fixture();
    for n in 0..b.len() {
        assert!(MaterialDefinition::read(&b[..n]).is_err());
    }
    let mut bad = b.clone();
    bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    let parsed = MaterialDefinition::read(&bad).unwrap();
    let outer = parsed.block_lengths.last().unwrap();
    assert_eq!(outer.declared, u32::MAX as usize);
    assert_eq!(outer.consumed, b.len() - 8);
    let mut bad = b;
    bad.push(0);
    assert!(MaterialDefinition::read(&bad).is_err());
}
#[test]
fn nonzero_alignment_padding_is_read_past_and_recorded() {
    let clean = fixture();
    let clean_read = MaterialDefinition::read(&clean).unwrap();
    assert!(clean_read.padding.is_empty());
    // "synthetic.sps" is 13 bytes: its marker is followed by two padding bytes before the next word
    let name = clean.windows(13).position(|w| w == b"synthetic.sps").unwrap();
    let pad = (name + 14).next_multiple_of(4) - (name + 14);
    assert_eq!(pad, 2);
    let mut odd = clean.clone();
    odd[name + 14..name + 16].copy_from_slice(&[0x72, 0x34]);
    let read = MaterialDefinition::read(&odd).unwrap();
    assert_eq!(
        read.padding,
        [fromsoft_formats::mtd::PaddingAnomaly {
            offset: name + 14,
            bytes: vec![0x72, 0x34]
        }]
    );
    // everything else reads exactly as in the clean file
    assert_eq!(read.shader, clean_read.shader);
    assert_eq!(read.description, clean_read.description);
    assert_eq!(read.block_lengths, clean_read.block_lengths);
    // a second anomaly is recorded after the first, in file order
    let desc = odd
        .windows(4)
        .position(|w| w == [0x93, 0xfa, 0x96, 0x7b])
        .unwrap()
        + 5;
    let len = desc.next_multiple_of(4) - desc;
    assert!(len > 0, "the description's marker is followed by padding");
    odd[desc] = 0x01;
    let read = MaterialDefinition::read(&odd).unwrap();
    assert_eq!(read.padding.len(), 2);
    assert_eq!(read.padding[1].offset, desc);
    assert_eq!(read.padding[1].bytes, [&[0x01][..], &vec![0; len - 1]].concat());
}

/// One parameter block: its name, type name, value block kind and marker, value count and value bytes.
fn parameter(
    b: &mut Vec<u8>,
    name: &[u8],
    kind: &[u8],
    block: u32,
    block_marker: u8,
    count: u32,
    body: &[u8],
) {
    let p = begin(b, 4, 4, 0xa3);
    string(b, name, 0xa3);
    string(b, kind, 4);
    word(b, 1);
    let inner = begin(b, block, 1, block_marker);
    word(b, count);
    b.extend(body);
    end(b, inner);
    marker(b, 4);
    word(b, 0);
    end(b, p);
}
/// One texture block of `version` 3 or 5 (3 stores neither the path nor the floats).
fn texture(b: &mut Vec<u8>, version: u32, kind: &[u8], uv: u32, index: u32, path: &[u8], extra: &[f32]) {
    let p = begin(b, 0x2000, version, 0xa3);
    string(b, kind, 0x35);
    word(b, uv);
    marker(b, 0x35);
    word(b, index);
    if version == 5 {
        word(b, 0xa3);
        string(b, path, 0xba);
        word(b, extra.len() as u32);
        for x in extra {
            word(b, x.to_bits());
        }
    }
    end(b, p);
}
/// [`fixture`] with three parameters (a boolean, an integer pair, a float triple) and three textures (a version 5
/// block with a path and two floats, one with neither, and a version 3 block).
fn rich_fixture() -> Vec<u8> {
    let mut b = vec![];
    let file = begin(&mut b, 0, 3, 1);
    let header = begin(&mut b, 1, 2, 0xb0);
    string(&mut b, b"MTD ", 0x34);
    word(&mut b, 1000);
    end(&mut b, header);
    marker(&mut b, 1);
    let data = begin(&mut b, 2, 4, 0xa3);
    string(&mut b, b"synthetic.sps", 0xa3);
    string(&mut b, &[0x93, 0xfa, 0x96, 0x7b], 3);
    word(&mut b, 1);
    let lists = begin(&mut b, 3, 4, 0xa3);
    word(&mut b, 0);
    marker(&mut b, 3);
    word(&mut b, 3);
    parameter(&mut b, b"g_Flag", b"bool", 0x1000, 0xc0, 1, &[1]);
    let pair: Vec<u8> = [7i32, -2].iter().flat_map(|x| x.to_le_bytes()).collect();
    parameter(&mut b, b"g_Pair", b"int2", 0x1001, 0xc5, 2, &pair);
    let triple: Vec<u8> = [0.25f32, -1.5, 3.0]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .collect();
    parameter(&mut b, b"g_Colour", b"float3", 0x1002, 0xca, 3, &triple);
    marker(&mut b, 3);
    word(&mut b, 3);
    texture(&mut b, 5, b"g_DiffuseTexture", 1, 0, b"N:\\x\\a.tga", &[0.5, 2.0]);
    texture(&mut b, 5, b"g_Mask", 2, 1, b"", &[]);
    texture(&mut b, 3, b"g_Bump", 1, 2, b"", &[]);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, lists);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, data);
    marker(&mut b, 4);
    word(&mut b, 0);
    end(&mut b, file);
    b
}
#[test]
fn a_read_definition_writes_back_to_the_bytes_it_was_read_from() {
    // the empty and the full fixture, each as built and with the outer block's length overstated (as the game's own
    // files do) and one marker's padding holding leftover bytes
    for base in [fixture(), rich_fixture()] {
        let mut overstated = base.clone();
        let n = u32::from_le_bytes(overstated[4..8].try_into().unwrap()) + 20;
        overstated[4..8].copy_from_slice(&n.to_le_bytes());
        let name = base.windows(13).position(|w| w == b"synthetic.sps").unwrap();
        let mut odd = overstated.clone();
        odd[name + 14..name + 16].copy_from_slice(&[0x72, 0x34]);
        for bytes in [base, overstated, odd] {
            let m = MaterialDefinition::read(&bytes).unwrap();
            assert_eq!(m.write().unwrap(), bytes);
        }
    }
    let m = MaterialDefinition::read(&rich_fixture()).unwrap();
    assert_eq!(m.parameters.len(), 3);
    assert_eq!(
        m.textures.iter().map(|t| t.version).collect::<Vec<_>>(),
        [5, 5, 3]
    );
}
#[test]
fn the_canonical_form_spans_every_block_and_zeroes_the_padding() {
    let clean = rich_fixture();
    let name = clean.windows(13).position(|w| w == b"synthetic.sps").unwrap();
    let mut odd = clean.clone();
    let n = u32::from_le_bytes(odd[4..8].try_into().unwrap()) + 20;
    odd[4..8].copy_from_slice(&n.to_le_bytes());
    odd[name + 14..name + 16].copy_from_slice(&[0x72, 0x34]);
    let m = MaterialDefinition::read(&odd).unwrap();
    assert_ne!(m.write().unwrap(), clean);
    assert_eq!(m.write_canonical().unwrap(), clean);
    // a definition built here records nothing and writes the canonical form
    let mut built = MaterialDefinition::read(&clean).unwrap();
    built.block_lengths.clear();
    built.padding.clear();
    assert_eq!(built.write().unwrap(), clean);
}
#[test]
fn an_edit_that_changes_a_blocks_size_is_refused_until_the_records_are_cleared() {
    let mut m = MaterialDefinition::read(&rich_fixture()).unwrap();
    m.textures[0].path = "N:\\x\\a_longer_name.tga".into();
    let error = m.write().unwrap_err().to_string();
    assert!(error.contains("block_lengths"), "{error}");
    m.block_lengths.clear();
    m.write().unwrap();
    let again = MaterialDefinition::read(&m.write().unwrap()).unwrap();
    assert_eq!(again.textures[0].path, "N:\\x\\a_longer_name.tga");
    assert_eq!(again.parameters, m.parameters);
}
#[test]
fn what_the_format_cannot_store_is_refused() {
    let base = MaterialDefinition::read(&rich_fixture()).unwrap();
    let mut m = base.clone();
    m.parameters[2].value = fromsoft_formats::mtd::Value::Float(vec![1.0; 5]);
    assert!(m.write_canonical().is_err());
    let mut m = base.clone();
    m.parameters[1].value = fromsoft_formats::mtd::Value::Int(vec![]);
    assert!(m.write_canonical().is_err());
    let mut m = base.clone();
    m.textures[0].version = 4;
    assert!(m.write_canonical().unwrap_err().to_string().contains("version 4"));
    let mut m = base.clone();
    m.textures[2].path = "a path a version 3 block has no room for".into();
    assert!(m.write_canonical().is_err());
    let mut m = base;
    m.description = "\u{1F600}".into();
    assert!(m.write_canonical().unwrap_err().to_string().contains("Shift-JIS"));
}
