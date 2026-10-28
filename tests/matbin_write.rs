//! The MATBIN writer on every material binary of the Elden Ring install: `write` returns the stored bytes of each.
//!
//! The counts were measured with an independent Python implementation of the layout (a parser and a writer of its own,
//! run over the same files): 20,619 materials (15,089 in `allmaterial`, 2,026 in `allmaterial_dlc01`, 3,283 in
//! `allmaterial_dlc02`, 221 in `speedtree`) with 640,949 parameters and 152,138 samplers, 2,996 of them with a tail
//! (2,235,891 bytes), 20,599 float3 values stored with five floats and one value stored shorter than its type.
mod common;
mod inputs;

use fromsoft_formats::{
    Binder, Game,
    dcx::Decoder,
    matbin::{Material, Value},
};
use inputs::{Tool, oodle};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn elden_ring_material_binaries_write_back_to_their_bytes() -> Result<()> {
    let (Some(archive), Some(dll)) = (common::archive(Game::EldenRing)?, oodle(Tool::Oodle6)) else {
        return Ok(());
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let (mut materials, mut params, mut samplers, mut tails, mut tail_bytes) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut extended, mut truncated) = (0usize, 0usize);
    for path in [
        "/material/allmaterial.matbinbnd.dcx",
        "/material/allmaterial_dlc01.matbinbnd.dcx",
        "/material/allmaterial_dlc02.matbinbnd.dcx",
        "/material/speedtree.matbinbnd.dcx",
    ] {
        let binder = Binder::read(&archive.read(path, &decoder)?, &decoder)?;
        for file in binder.files.iter().filter(|f| f.bytes.starts_with(b"MAB\0")) {
            let name = file.name.clone().unwrap_or_default();
            let m = Material::read(&file.bytes).map_err(|e| format!("{name}: {e}"))?;
            let written = m.write().map_err(|e| format!("{name}: {e}"))?;
            if written != file.bytes {
                return Err(format!("{name}: write() differs from the stored bytes").into());
            }
            materials += 1;
            params += m.params.len();
            samplers += m.samplers.len();
            tails += usize::from(!m.unreferenced_tail.is_empty());
            tail_bytes += m.unreferenced_tail.len();
            extended += m
                .params
                .iter()
                .filter(|p| matches!(p.value, Value::Float3(_, Some(_))))
                .count();
            truncated += m
                .params
                .iter()
                .filter(|p| matches!(p.value, Value::Truncated { .. }))
                .count();
        }
    }
    println!(
        "materials {materials}, parameters {params}, samplers {samplers}, tails {tails} ({tail_bytes} bytes), \
         float3 stored with five floats {extended}, stored shorter than their type {truncated}"
    );
    assert_eq!(
        (
            materials, params, samplers, tails, tail_bytes, extended, truncated
        ),
        (20619, 640_949, 152_138, 2996, 2_235_891, 20_599, 1)
    );
    Ok(())
}
