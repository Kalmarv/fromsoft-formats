//! The named archive paths shared by readers and catalogues. These spell names; they do not select files, test
//! residency or change the case of an input. The game-specific lookup order remains at the call site.

macro_rules! path {
    ($(#[$doc:meta])* $name:ident($($arg:ident: $ty:ty),+) => $template:literal) => {
        $(#[$doc])*
        pub fn $name($($arg: $ty),+) -> String {
            format!($template, $($arg = $arg),+)
        }
    };
}

path!(/// A character's model binder.
    character(name: &str) => "/chr/{name}.chrbnd.dcx");
path!(/// A character's animation binder, including an authored companion stem.
    character_animation(name: &str) => "/chr/{name}.anibnd.dcx");
path!(/// A character's behavior/skeleton binder.
    character_behavior(name: &str) => "/chr/{name}.behbnd.dcx");
path!(/// A character's unsuffixed texture binder.
    character_texture(name: &str) => "/chr/{name}.texbnd.dcx");
path!(/// A character's texture binder at the named authored form (`h` or `l`).
    character_texture_form(name: &str, form: &str) => "/chr/{name}_{form}.texbnd.dcx");
path!(/// A character part's binder.
    part(name: &str) => "/parts/{name}.partsbnd.dcx");
path!(/// A placed object's binder in the object namespace.
    object(name: &str) => "/obj/{name}.objbnd.dcx");
path!(/// A map's layout, using the archive's canonical lower-case prefix.
    map_layout(area: &str) => "/map/mapstudio/{area}.msb.dcx");
path!(/// An area's event script, or the shared `common` script.
    event(area: &str) => "/event/{area}.emevd.dcx");
path!(/// A draw-param file by its complete stem, including any authored layer suffix.
    draw_param(stem: &str) => "/param/drawparam/{stem}.gparam.dcx");
path!(/// A renderer shader binder by its folder stem.
    shader(folder: &str) => "/shader/{folder}.shaderbnd.dcx");
path!(/// An effect binder by its namespace (`m10`, `c4130`, `commoneffects`, ...).
    sfx(namespace: &str) => "/sfx/sfxbnd_{namespace}.ffxbnd.dcx");
path!(/// An area's split texture-binder header.
    map_texture_header(area: &str, index: usize) => "/map/{area}/{area}_{index:04}.tpfbhd");
path!(/// The data file paired with an area's split texture-binder header.
    map_texture_data(area: &str, index: usize) => "/map/{area}/{area}_{index:04}.tpfbdt");
path!(/// A map's unbundled environment package at an authored state.
    map_environment(area: &str, state: u8) => "/map/{area}/{area}_envmap_{state:02}.tpf.dcx");
path!(/// An Elden Ring map's bundled environment package at an authored state, tier and part.
    map_environment_bundle(family: &str, area: &str, state: u8, tier: &str, part: u8)
        => "/map/{family}/{area}/{area}_envmap_{state:02}_{tier}_{part:02}.tpfbnd.dcx");
path!(/// One map-piece binder inside a map's archive folder.
    map_model(folder: &str, name: &str) => "{folder}/{name}.mapbnd.dcx");
path!(/// A map's light bank zero.
    light_bank(folder: &str, area: &str) => "{folder}/{area}_0000.btl.dcx");
path!(/// A map's hang-edge binder header.
    edge_header(folder: &str, area: &str) => "{folder}/{area}.edgebhd");
path!(/// A map's hang-edge binder data file.
    edge_data(folder: &str, area: &str) => "{folder}/{area}.edgebdt");
path!(/// A validated map-collision stem's binder header.
    collision_header(stem: &str) => "{stem}.hkxbhd");
path!(/// A validated map-collision stem's binder data file.
    collision_data(stem: &str) => "{stem}.hkxbdt");
path!(/// An Elden Ring asset's model binder, with its already validated group and lower-case model name.
    asset_model(group: &str, name: &str) => "/asset/aeg/{group}/{name}.geombnd.dcx");
path!(/// An Elden Ring asset's collision binder at a named authored resolution suffix.
    asset_collision(group: &str, name: &str, suffix: &str) => "/asset/aeg/{group}/{name}_{suffix}.geomhkxbnd.dcx");
path!(/// A low-resolution texture package beside a package stem (the stem includes its folder).
    low_texture_package(stem: &str) => "{stem}_l.tpf.dcx");

/// The common effect namespace, shared by supported area and clip lookup orders.
pub const COMMON_EFFECTS: &str = "/sfx/sfxbnd_commoneffects.ffxbnd.dcx";
/// The second downloadable-content common effect namespace.
pub const COMMON_EFFECTS_DLC02: &str = "/sfx/sfxbnd_commoneffects_dlc02.ffxbnd.dcx";

/// A validated map's collision binder stem (`set` is its authored `h` or `f` namespace).
pub fn map_collision_stem(folder: &str, area: &str, set: char) -> String {
    format!("{folder}/{set}{}", area.strip_prefix('m').unwrap_or(area))
}

/// The collision member an MSB `h<n>` model names, preserving the existing case-insensitive archive comparison.
/// `None` means this is not a map-collision model name.
pub fn map_collision_member(area: &str, model: &str) -> Option<String> {
    let model = crate::names::fold(model);
    let suffix = model.strip_prefix('h')?;
    Some(crate::names::fold(&format!(
        "h{}_{suffix}.hkx",
        area.strip_prefix('m').unwrap_or(area)
    )))
}
