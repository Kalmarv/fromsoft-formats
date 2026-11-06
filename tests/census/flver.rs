//! The FLVER census: every model of an install's archives read by ours and checked against the reference census of the
//! same files, SoulsFormats' of Sekiro (`oracles.soulsformats.sekiro/census`) and SoulsFormatsNEXT's of Elden Ring
//! (`oracles.soulsformats.elden/census`): each file's bytes, version, meshes, materials, vertex counts, a hash of the
//! decoded vertices and each face set's flags and indices. A FLVER the reference refuses (a record with `error`) must
//! be refused by ours too, except a SpeedTree FLVER: the reference refuses every one of Elden Ring's (an index out of
//! range), ours reads their stored members; they are counted. Beside the comparison it counts what the reference does
//! not compare: the bones' links, boxes and flags, the dummies' attachment, and the header and face-set words whose
//! meaning is not known.
use super::Report;
use crate::{common, inputs};
use fromsoft_formats::{
    Binder, Game, NameDictionary,
    bhd::PathHash,
    dcx::Decoder,
    flver::{Layout, Model},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The census of `game`'s FLVER models, or `None` when the configuration lacks the install, its keys, the Oodle library,
/// the reference census or the name dictionary.
pub fn census(game: Game) -> Result<Option<Report>> {
    // Elden Ring's census of the install with Shadow of the Erdtree (`census` was made before the DLC)
    let (data, census) = match game {
        Game::Sekiro => (
            inputs::Game::Sekiro,
            inputs::oracle(inputs::OracleSet::SoulsFormats, inputs::Game::Sekiro, "census"),
        ),
        Game::EldenRing => (
            inputs::Game::Elden,
            inputs::oracle(
                inputs::OracleSet::SoulsFormats,
                inputs::Game::Elden,
                "census-sote",
            ),
        ),
    };
    let (Some(dll), Some(oracle), Some(names)) = (
        inputs::oodle(inputs::Tool::Oodle6),
        census,
        inputs::oracle(inputs::OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let Some(archive) = common::archive(game)? else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let names = NameDictionary::read_for(names, game)?;
    // the reference writes a hash at its game's width
    let width = match game.archives().hash {
        PathHash::Base37 => 8,
        PathHash::Base133 => 16,
    };
    let mut expected = BTreeMap::new();
    for line in std::fs::read_to_string(oracle.join("flver/models.jsonl"))?.lines() {
        let value: serde_json::Value = serde_json::from_str(line)?;
        expected.insert(value["id"].as_str().ok_or("id")?.to_owned(), value);
    }
    // shard, hash, the reference's message: the entries it cannot decode
    let mut rejected = BTreeMap::new();
    for line in std::fs::read_to_string(oracle.join("errors.tsv"))?.lines() {
        let f: Vec<_> = line.splitn(3, '\t').collect();
        let [shard, hash, message] = f[..] else {
            return Err(format!("errors.tsv row: {line}").into());
        };
        rejected.insert(format!("{shard}/{hash}"), message.to_owned());
    }
    let check = Check {
        decoder: &decoder,
        expected: &expected,
        next: game == Game::EldenRing,
    };
    // What one entry's check gives: the ids of the FLVERs it checked and what it found (kept when the entry is then
    // refused, as a walk entry by entry keeps them), its refusal, and a split binder's index with the path of its payload
    // (read in a second pass).
    struct Checked {
        seen: Vec<String>,
        report: Report,
        refused: Option<String>,
        index: Option<(String, Vec<u8>)>,
    }
    let id_of = |hash: u64| format!("{}/{hash:0width$x}", archive.entries[&hash].shard);
    let name_of = |hash: u64| names.names.get(&hash).and_then(|x| x.first());
    // every entry read in one pass and checked on the scan's threads, then merged in the index's order
    let mut checked = common::scan_map(
        &archive,
        &decoder,
        |hash| !name_of(hash).is_some_and(|n| n.ends_with("bdt")),
        |hash, bytes| {
            let id = id_of(hash);
            let mut c = Checked {
                seen: Vec::new(),
                report: Report::default(),
                refused: None,
                index: None,
            };
            let result = (|| -> Result<()> {
                let bytes = bytes?;
                if bytes.starts_with(b"BHF4") {
                    let name = name_of(hash).ok_or("unnamed split binder")?;
                    c.index = Some((format!("{}bdt", &name[..name.len() - 3]), bytes.to_vec()));
                } else {
                    check.visit(bytes, &id, &mut c.seen, &mut c.report, 0)?;
                }
                Ok(())
            })();
            c.refused = result.err().map(|e| e.to_string());
            c
        },
    )?;
    // the split binders: each payload read in one more pass and read with the index the scan kept, its members checked
    let split: Vec<(u64, String, Vec<u8>)> = checked
        .iter_mut()
        .filter_map(|(&hash, c)| c.index.take().map(|(payload, index)| (hash, payload, index)))
        .collect();
    let by_payload: BTreeMap<&str, (u64, &[u8])> = split
        .iter()
        .map(|(hash, payload, index)| (payload.as_str(), (*hash, index.as_slice())))
        .collect();
    let members = archive.read_each(
        split.iter().map(|(_, payload, _)| payload),
        &decoder,
        inputs::threads(),
        |payload, bytes| {
            let (hash, index) = by_payload[payload];
            let id = id_of(hash);
            let mut c = Checked {
                seen: Vec::new(),
                report: Report::default(),
                refused: None,
                index: None,
            };
            let result = (|| -> Result<()> {
                let binder = Binder::read_split(index, bytes?, &decoder)?;
                for (i, f) in binder.files.iter().enumerate() {
                    check.visit(&f.bytes, &format!("{id}/s{i}"), &mut c.seen, &mut c.report, 0)?;
                }
                c.report.count("split binders", 1);
                Ok(())
            })();
            c.refused = result.err().map(|e| e.to_string());
            c
        },
    )?;
    for ((hash, ..), c) in split.iter().zip(members) {
        *checked
            .get_mut(hash)
            .ok_or("a split binder the scan did not give")? = c;
    }
    let mut report = Report::default();
    let mut reached = BTreeMap::new();
    for (hash, c) in checked {
        let id = id_of(hash);
        for seen in &c.seen {
            match expected.remove(seen) {
                Some(record) => {
                    reached.insert(seen.clone(), record);
                }
                None => report.failures.push(format!("{seen}: checked twice")),
            }
        }
        report.add(c.report);
        if let Some(e) = c.refused {
            // both decoders reject it: agreement on the entry, the reasons side by side
            match rejected.remove(&id) {
                Some(theirs) => report
                    .agreed_refusals
                    .push(format!("{id}: ours {e}; reference {theirs}")),
                None => report.failures.push(format!("{id}: {e}")),
            }
        }
    }
    // An entry a later archive lists again with the same bytes (Elden Ring's DLC repeats Data3's `/chr/c5550.chrbnd.dcx`)
    // is read once, from the archive that lists it first; the reference reads both copies. Each record of the copy not
    // read must be the record of the copy read, but for its id.
    for r in &archive.repeated {
        let (kept, again) = (
            format!("{}/{:0width$x}", r.kept, r.hash),
            format!("{}/{:0width$x}", r.again, r.hash),
        );
        let copies: Vec<String> = expected
            .keys()
            .filter(|id| {
                id.strip_prefix(&again)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            })
            .cloned()
            .collect();
        for id in copies {
            let read = format!("{kept}{}", &id[again.len()..]);
            let without_id = |v: &serde_json::Value| {
                let mut v = v.clone();
                v.as_object_mut().map(|o| o.remove("id"));
                v
            };
            match (expected.remove(&id), reached.get(&read)) {
                (Some(theirs), Some(ours)) if without_id(&theirs) == without_id(ours) => {
                    report.count(
                        "models of an entry repeated by a later archive, matched through the first",
                        1,
                    );
                }
                _ => report.failures.push(format!(
                    "{id}: repeats {read}, whose reference record differs or was not reached"
                )),
            }
        }
    }
    for id in expected.keys() {
        report
            .failures
            .push(format!("{id}: in the reference census, never reached"));
    }
    for (id, theirs) in rejected {
        report.failures.push(format!(
            "{id}: the reference refuses it ({theirs}), ours does not"
        ));
    }
    Ok(Some(report))
}

/// One census's check of a FLVER against its reference record.
struct Check<'a> {
    decoder: &'a Decoder<'a>,
    /// The reference's record of each FLVER, by id.
    expected: &'a BTreeMap<String, serde_json::Value>,
    /// The reference is SoulsFormatsNEXT's (Elden Ring) rather than SoulsFormats' (Sekiro).
    next: bool,
}

/// The packed normal and tangent components of one model, against the reference's convention ([`reference_normal`]).
#[derive(Default)]
struct Normals {
    components: u64,
    one_ulp: u64,
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02X}")).collect()
}

/// The header and face-set words of `bytes` (a FLVER `model` was read from) that the reader keeps for the writer: the
/// default index width at 0x48, the bytes at 0x5d and 0x68 by the model's kind, and the index width each face set states.
fn layout_values(bytes: &[u8], model: &Model, report: &mut Report) -> Result<()> {
    let layout = Layout::read(bytes)?;
    let kind = if model.speedtree {
        "SpeedTree".to_owned()
    } else {
        format!("version {:#x}", model.version)
    };
    report.value("version", model.version);
    report.value("header 0x48 default index width", layout.index_width);
    report.value(&format!("header 0x5d ({kind})"), layout.header_5d);
    report.value(&format!("header 0x68 ({kind})"), layout.header_68);
    for width in layout.face_set_widths {
        report.value("face set index width", width);
    }
    Ok(())
}

/// Counts the fields of `model`'s bones and face sets that the reference census does not compare: the links, the bounding
/// box and the flags of the bones, and the word at +6 of the face sets (each value's records).
fn tally(model: &Model, report: &mut Report) {
    let mut add = |key: &str, n: bool| report.flag(key, n);
    let mesh_nodes: BTreeSet<usize> = model
        .meshes
        .iter()
        .filter_map(|m| usize::try_from(m.node).ok())
        .collect();
    let dummy_nodes: BTreeSet<usize> = model
        .dummies
        .iter()
        .flat_map(|d| [d.parent_bone, d.attach_bone])
        .filter_map(|b| usize::try_from(b).ok())
        .collect();
    for (i, bone) in model.bones.iter().enumerate() {
        add("bones", true);
        let valid = bone.bounds_min.iter().zip(&bone.bounds_max).all(|(a, b)| a <= b);
        let sentinel = bone.bounds_min == [f32::MAX; 3] && bone.bounds_max == [-f32::MAX; 3];
        add("box valid", valid);
        add("box sentinel", sentinel);
        add("box neither", !valid && !sentinel);
        let children: Vec<usize> = model
            .bones
            .iter()
            .enumerate()
            .filter(|(_, b)| usize::try_from(b.parent) == Ok(i))
            .map(|(j, _)| j)
            .collect();
        add("bones with children", !children.is_empty());
        add(
            "first child is not the lowest-indexed child",
            children
                .first()
                .is_some_and(|&lowest| usize::try_from(bone.child) != Ok(lowest)),
        );
        add("flags above the low four bits", bone.flags & !0xf != 0);
        let (unused, dummy) = (bone.flags & 1 != 0, bone.flags & 2 != 0);
        let (named_by_mesh, named_by_dummy) = (mesh_nodes.contains(&i), dummy_nodes.contains(&i));
        let leaf_unnamed = children.is_empty() && !named_by_mesh && !named_by_dummy;
        add("bit 1 set", unused);
        add(
            "bit 1 set on a bone that has children or is named",
            unused && !leaf_unnamed,
        );
        add("childless unnamed bones", leaf_unnamed);
        add("childless unnamed bones without bit 1", leaf_unnamed && !unused);
        add("bit 2 set", dummy);
        add("bit 2 set on a bone no dummy names", dummy && !named_by_dummy);
        add("bones a dummy names", named_by_dummy);
        add("bones a dummy names without bit 2", named_by_dummy && !dummy);
        add("bit 4 set", bone.flags & 4 != 0);
        add("bit 8 set", bone.flags & 8 != 0);
        add("bit 8 set on a mesh's node", bone.flags & 8 != 0 && named_by_mesh);
    }
    // the dummies' `flag1` against the attach bone, in models of one bone and in the others
    let single = model.bones.len() == 1;
    for dummy in &model.dummies {
        let attached = dummy.attach_bone != -1;
        add("dummies", true);
        add("dummies with flag1", dummy.flag1);
        add("dummies with an attach bone", attached);
        if single {
            add("dummies in single-bone models", true);
            add(
                "single-bone dummies with an attach bone and flag1",
                attached && dummy.flag1,
            );
            add(
                "single-bone dummies with an attach bone and no flag1",
                attached && !dummy.flag1,
            );
            add(
                "single-bone dummies with no attach bone and flag1",
                !attached && dummy.flag1,
            );
            add(
                "single-bone dummies with no attach bone and no flag1",
                !attached && !dummy.flag1,
            );
        } else {
            add(
                "other dummies where flag1 follows the attach bone",
                attached == dummy.flag1,
            );
            add(
                "other dummies with flag1 and no attach bone",
                !attached && dummy.flag1,
            );
            add(
                "other dummies with an attach bone and no flag1",
                attached && !dummy.flag1,
            );
        }
    }
    for _ in model.meshes.iter().flat_map(|m| &m.faces) {
        add("face sets", true);
    }
    for bone in &model.bones {
        report.value("bone flags", bone.flags);
    }
    for face in model.meshes.iter().flat_map(|m| &m.faces) {
        report.value("face set word +6", face.unknown_06);
    }
}
/// A vertex's UVs in the reference's packing. The reader keeps each UV member as the one four-lane attribute the game's
/// vertex program receives (`Vertex::uvs`). SoulsFormatsNEXT splits two kinds of member into two UVs of two lanes: a
/// `UByte4Norm` (`0x13`) member, four unorm bytes `(b0, b1, 0)` and `(b2, b3, 0)`, and a kind `0x2e` member, whose four
/// 16-bit words it reads as int16 over the UV factor, likewise two UVs. The reader's values are what the game's programs
/// receive (the `0x13` member as unorm bytes, the `0x2e` member as half floats, both a float `TEXCOORDn xyzw` used
/// unscaled; the `0x2e` member is a pivot point in the model's space, not a texture coordinate; see `Vertex::uvs`), so
/// this projects them back onto the reference's packing of the same stored bytes:
/// the unorm lanes as they are, the half lanes through their 16-bit words. SoulsFormats' census of Sekiro has one UV
/// per member.
fn reference_uvs(
    v: &fromsoft_formats::flver::Vertex,
    kinds: &[u32],
    next: bool,
    sekiro_words: bool,
) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    for (i, uv) in v.uvs.iter().enumerate() {
        if next && kinds.get(i) == Some(&0x13) {
            out.push([uv[0], uv[1], 0.]);
            out.push([uv[2], uv[3], 0.]);
        } else if next && kinds.get(i) == Some(&0x2e) {
            let word = |x: f32| f32::from(half_bits(x) as i16) / 2048.;
            out.push([word(uv[0]), word(uv[1]), 0.]);
            out.push([word(uv[2]), word(uv[3]), 0.]);
        } else if !next && kinds.get(i) == Some(&0x2e) {
            // SoulsFormats' Short4toFloat4B: three int16 over the UV factor, then an int16 it asserts is zero
            assert_eq!(half_bits(uv[3]), 0, "reference asserts the fourth word is zero");
            let word = |x: f32| {
                if sekiro_words {
                    f32::from(half_bits(x) as i16) / 2048.
                } else {
                    x
                }
            };
            out.push([word(uv[0]), word(uv[1]), word(uv[2])]);
        } else if !next && kinds.get(i) == Some(&0x13) {
            // SoulsFormats reads the four bytes as an int16 pair over the UV factor
            let raw = v
                .layer_bytes
                .iter()
                .find(|(at, _)| *at == i)
                .map(|(_, r)| *r)
                .unwrap_or_default();
            out.push([
                f32::from(i16::from_le_bytes([raw[0], raw[1]])) / 2048.,
                f32::from(i16::from_le_bytes([raw[2], raw[3]])) / 2048.,
                0.,
            ]);
        } else {
            out.push([uv[0], uv[1], uv[2]]);
        }
    }
    out
}
/// The 16 bits of the half float `x` (a value the reader decoded from a half, so exactly representable).
fn half_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let magnitude = bits & 0x7fff_ffff;
    if magnitude == 0 {
        return sign;
    }
    let exponent = (magnitude >> 23) as i32 - 127;
    if exponent < -14 {
        // a subnormal half: the mantissa over 2^-24
        return sign | (f32::from_bits(magnitude) * 2f32.powi(24)).round() as u16;
    }
    sign | ((((exponent + 15) as u32) << 10) | ((magnitude & 0x7f_ffff) >> 13)) as u16
}
/// The kind of the layout member each of a mesh's UVs came from, in the reader's push order.
fn uv_member_kinds(model: &Model, layouts: &[usize]) -> Vec<u32> {
    let mut kinds = Vec::new();
    for &l in layouts {
        for member in &model.layouts[l] {
            if member.semantic == 5 {
                let n = if matches!(member.kind, 3 | 0x16) { 2 } else { 1 };
                kinds.extend(std::iter::repeat_n(member.kind, n));
            }
        }
    }
    kinds
}
/// The independent decoder divides by 127; the game multiplies by its rounded
/// reciprocal. Recover the unique encoded integer only after checking the
/// consumer result bit-for-bit. Project to the reference's value convention for
/// its digest, and separately count every one-ULP deviation. This never changes
/// production decoding or accepts a different packed attribute.
fn reference_normal(value: f32, normals: &mut Normals) -> Result<f32> {
    let integer = (value * 127.).round();
    if !(-127. ..=128.).contains(&integer)
        || (integer * f32::from_bits(0x3c01_0204)).to_bits() != value.to_bits()
    {
        return Err("normal is not a consumer-decoded packed byte".into());
    }
    let reference = integer / 127.;
    let ulp = value.to_bits().abs_diff(reference.to_bits());
    if ulp > 1 {
        return Err("packed normal differs from reference by more than one ULP".into());
    }
    normals.components += 1;
    normals.one_ulp += u64::from(ulp != 0);
    Ok(reference)
}

impl Check<'_> {
    /// Checks the FLVER `bytes` (or each FLVER a BND4 of them holds) against its reference record, adding the ids it
    /// checked to `seen` and what it found to `report`.
    fn visit(
        &self,
        bytes: &[u8],
        id: &str,
        seen: &mut Vec<String>,
        report: &mut Report,
        depth: usize,
    ) -> Result<()> {
        if depth > 8 {
            return Err("nested binder limit".into());
        }
        if bytes.starts_with(b"BND4") {
            let b = Binder::read(bytes, self.decoder)?;
            for (i, f) in b.files.iter().enumerate() {
                self.visit(&f.bytes, &format!("{id}/b{i}"), seen, report, depth + 1)?;
            }
            return Ok(());
        }
        if !bytes.starts_with(b"FLVER\0") {
            return Ok(());
        }
        let want = self
            .expected
            .get(id)
            .ok_or_else(|| format!("unexpected FLVER {id}"))?;
        seen.push(id.to_owned());
        assert_eq!(sha(bytes), want["sha256"].as_str().ok_or("sha")?, "{id}");
        if let Some(theirs) = want.get("error") {
            return match Model::read(bytes) {
                Ok(m) if m.speedtree => {
                    report.count("SpeedTree models the reference refuses, read", 1);
                    tally(&m, report);
                    layout_values(bytes, &m, report)
                }
                Ok(_) => Err(format!("{id}: read, the reference refuses it: {theirs}").into()),
                Err(e) => {
                    report
                        .agreed_refusals
                        .push(format!("FLVER {id}: ours {e}; reference {theirs}"));
                    Ok(())
                }
            };
        }
        let model = Model::read(bytes)?;
        tally(&model, report);
        layout_values(bytes, &model, report)?;
        assert_eq!(
            u64::from(model.version),
            want["version"].as_u64().ok_or("version")?
        );
        let meshes = want["meshes"].as_array().ok_or("meshes")?;
        assert_eq!(model.meshes.len(), meshes.len(), "{id}");
        let mut normals = Normals::default();
        for (mesh_index, (m, w)) in model.meshes.iter().zip(meshes).enumerate() {
            let uv_kinds = uv_member_kinds(&model, &m.layouts);
            for &layout in &m.layouts {
                for member in &model.layouts[layout] {
                    if matches!(member.semantic, 3 | 6) && member.kind != 0x11 {
                        return Err("reference normal projection only covers uint4 attributes".into());
                    }
                }
            }
            assert_eq!(m.material as u64, w["material"].as_u64().ok_or("material")?);
            assert_eq!(m.vertices.len() as u64, w["vertices"].as_u64().ok_or("vertices")?);
            let build = |sekiro_words: bool, normals: &mut Normals| -> Result<Vec<u8>> {
                let mut stream = Vec::new();
                for v in &m.vertices {
                    for x in v.position {
                        stream.extend(x.to_le_bytes());
                    }
                    for x in v.normal {
                        stream.extend(reference_normal(x, normals)?.to_le_bytes());
                    }
                    stream.extend(v.normal_w.to_le_bytes());
                    let uvs = reference_uvs(v, &uv_kinds, self.next, sekiro_words);
                    stream.extend((uvs.len() as u32).to_le_bytes());
                    for x in uvs.iter().flatten() {
                        stream.extend(x.to_le_bytes());
                    }
                    stream.extend((v.tangents.len() as u32).to_le_bytes());
                    for x in v.tangents.iter().flatten() {
                        stream.extend(reference_normal(*x, normals)?.to_le_bytes());
                    }
                    stream.extend((v.colors.len() as u32).to_le_bytes());
                    for x in v.colors.iter().flatten() {
                        stream.extend(x.to_le_bytes());
                    }
                }
                Ok(stream)
            };
            let want_hash = w["vertex_hash"].as_str().ok_or("vertex hash")?;
            let stored = sha(&build(false, &mut normals)?);
            if stored != want_hash {
                // SoulsFormats reads a Sekiro `0x2e` member as int16 words over the UV factor; ours reads halves (see
                // `reference_uvs`). A mesh counts as that named divergence only when reading its `0x2e` members the
                // reference's way reproduces the reference's hash exactly; any other difference is a failure.
                let sekiro_half4 = !self.next && uv_kinds.contains(&0x2e);
                if sekiro_half4 && sha(&build(true, &mut Normals::default())?) == want_hash {
                    report.count("Sekiro 0x2e meshes the reference reads as int16 words", 1);
                    for v in &m.vertices {
                        for (i, uv) in v.uvs.iter().enumerate() {
                            if uv_kinds.get(i) == Some(&0x2e) {
                                for x in &uv[..3] {
                                    report.count("Sekiro 0x2e x/y/z lanes read as halves", 1);
                                    report.flag("Sekiro 0x2e x/y/z lanes in [0, 1]", (0. ..=1.).contains(x));
                                    report.maximum("Sekiro 0x2e x/y/z lane magnitude", f64::from(x.abs()));
                                }
                            }
                        }
                    }
                } else {
                    report.failures.push(format!(
                        "{id} mesh {mesh_index}: vertex hash differs, uv member kinds {uv_kinds:x?}"
                    ));
                }
            }
            let faces = w["faces"].as_array().ok_or("faces")?;
            assert_eq!(m.faces.len(), faces.len());
            for (f, expected) in m.faces.iter().zip(faces) {
                assert_eq!(u64::from(f.flags), expected["flags"].as_u64().ok_or("flags")?);
                assert_eq!(f.strip, expected["strip"].as_bool().ok_or("strip")?);
                assert_eq!(f.cull, expected["cull"].as_bool().ok_or("cull")?);
                assert_eq!(
                    sha(&f.indices.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<_>>()),
                    expected["hash"].as_str().ok_or("index hash")?,
                    "{id}"
                );
                assert_eq!(
                    sha(&f
                        .triangulate()
                        .iter()
                        .flat_map(|i| i.to_le_bytes())
                        .collect::<Vec<_>>()),
                    expected["triangulated"].as_str().ok_or("triangle hash")?,
                    "{id}"
                );
            }
            report.count("meshes", 1);
        }
        report.count("packed normal/tangent components", normals.components);
        report.count(
            "packed normal/tangent components one ULP from the reference's convention",
            normals.one_ulp,
        );
        report.count("models", 1);
        Ok(())
    }
}
