//! The MATBIN writer on hand-built materials laid out as the game's files are.
use fromsoft_formats::matbin::{Material, Param, Sampler, Value};

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}
fn put64(b: &mut [u8], at: usize, x: usize) {
    b[at..at + 8].copy_from_slice(&(x as u64).to_le_bytes());
}

/// A material laid out as the game's files are: the records, then every string and value packed in file order (each
/// parameter's name and value, each sampler's type and path, the shader, the source) and `tail`. Its parameters hold
/// every value type, a float3 stored with five floats and a float2 stored as one float; the first is a bool, so every
/// string after it starts at an odd offset and the last item ends at one.
fn packed(tail: &[u8]) -> Vec<u8> {
    let (one, half) = (1f32.to_le_bytes(), 0.5f32.to_le_bytes());
    let values: Vec<(u32, Vec<u8>)> = vec![
        (0, vec![1]),
        (4, 7i32.to_le_bytes().to_vec()),
        (5, [3i32, -4].iter().flat_map(|x| x.to_le_bytes()).collect()),
        (8, half.to_vec()),
        (9, [one, half].concat()),
        (10, [one, one, one].concat()),
        (10, [one, half, one, one, one].concat()),
        (11, [one; 4].concat()),
        (12, [half; 5].concat()),
        (9, half.to_vec()),
    ];
    let params = values.len();
    let mut b = vec![0u8; 0x38 + params * 0x28 + 0x30];
    b[..4].copy_from_slice(b"MAB\0");
    b[4] = 2;
    b[0x18..0x1c].copy_from_slice(&7u32.to_le_bytes());
    b[0x1c..0x20].copy_from_slice(&(params as u32).to_le_bytes());
    b[0x20..0x24].copy_from_slice(&1u32.to_le_bytes());
    let push = |b: &mut Vec<u8>, bytes: &[u8]| {
        let at = b.len();
        b.extend_from_slice(bytes);
        at
    };
    for (i, (code, value)) in values.iter().enumerate() {
        let r = 0x38 + i * 0x28;
        let name = push(&mut b, &utf16(&format!("p{i}")));
        let at = push(&mut b, value);
        put64(&mut b, r, name);
        put64(&mut b, r + 8, at);
        b[r + 0x10..r + 0x14].copy_from_slice(&(i as u32 + 10).to_le_bytes());
        b[r + 0x14..r + 0x18].copy_from_slice(&code.to_le_bytes());
    }
    let s = 0x38 + params * 0x28;
    let kind = push(&mut b, &utf16("M_snp_Texture2D_0_AlbedoMap_0"));
    let path = push(&mut b, &utf16("N:\\a.tif"));
    put64(&mut b, s, kind);
    put64(&mut b, s + 8, path);
    b[s + 0x10..s + 0x14].copy_from_slice(&5u32.to_le_bytes());
    b[s + 0x14..s + 0x18].copy_from_slice(&2f32.to_le_bytes());
    b[s + 0x18..s + 0x1c].copy_from_slice(&3f32.to_le_bytes());
    let shader = push(&mut b, &utf16("x.spx"));
    let source = push(&mut b, &utf16("x.matxml"));
    put64(&mut b, 8, shader);
    put64(&mut b, 16, source);
    b.extend_from_slice(tail);
    b
}

#[test]
fn a_packed_material_writes_back_to_its_bytes() {
    let m = Material::read(&packed(&[])).unwrap();
    // the fixture holds what it says it does
    assert_eq!(m.params[0].value, Value::Bool(true));
    assert_eq!(m.params[6].value, Value::Float3([1., 0.5, 1.], Some([1., 1.])));
    assert_eq!(
        m.params[9].value,
        Value::Truncated {
            code: 9,
            bytes: 0.5f32.to_le_bytes().to_vec()
        }
    );
    assert_eq!((m.samplers[0].key, m.samplers[0].unknown), (5, [2., 3.]));
    // with and without a tail; a tail that starts with a zero byte after the odd end keeps it
    for tail in [
        vec![],
        utf16("old revision"),
        vec![0, 1, 2],
        vec![0],
        vec![9, 0, 0],
    ] {
        let bytes = packed(&tail);
        let m = Material::read(&bytes).unwrap();
        assert_eq!(m.unreferenced_tail, tail);
        assert_eq!(m.write().unwrap(), bytes, "tail {tail:?}");
    }
}

#[test]
fn an_edited_material_writes_and_reads_back_equal() {
    let mut m = Material::read(&packed(&[7, 7])).unwrap();
    m.params[1].value = Value::Int(-9);
    m.params[5].value = Value::Float3([2., 3., 4.], Some([5., 6.]));
    m.params[6].value = Value::Float3([2., 3., 4.], None);
    m.shader = "a_longer_shader_path.spx".into();
    m.samplers.push(Sampler {
        kind: "K".into(),
        path: String::new(),
        key: 9,
        unknown: [0., 1.],
    });
    m.params.push(Param {
        name: "added".into(),
        key: 99,
        value: Value::Bool(false),
    });
    let again = Material::read(&m.write().unwrap()).unwrap();
    assert_eq!(again, m);
}

#[test]
fn what_the_format_cannot_store_is_refused() {
    let base = Material::read(&packed(&[])).unwrap();
    let mut m = base.clone();
    // as many bytes as the type declares is the typed variant's size
    m.params[3].value = Value::Truncated {
        code: 9,
        bytes: vec![0; 8],
    };
    assert!(m.write().is_err());
    m.params[3].value = Value::Truncated {
        code: 13,
        bytes: vec![0],
    };
    assert!(m.write().unwrap_err().to_string().contains("parameter type 13"));
    let mut m = base;
    m.shader = "a\0b".into();
    assert!(m.write().unwrap_err().to_string().contains("NUL"));
}
