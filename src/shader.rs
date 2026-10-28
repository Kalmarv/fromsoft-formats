//! Elden Ring's shader names and sampler names, split into the parts they are made of.
//!
//! A material binary names its shader by a stem such as `C[DetailBlend][S2]_cloth` or `M[AMSN_V][Mb2][Ov_AN]_Edge`: a
//! category of letters (`C` a character, `CS` a cut-scene character, `P` a player part, `M` a map piece, `S` a sky or
//! scenery piece), the shader family in the first brackets, further bracketed tags (`[Mb2]` the number of blended
//! layers, `[Ov_AN]` an overlay layer and the maps it holds, `[S2]` a second skin layer, `[Snow]`), and suffixes after
//! underscores (`_cloth`, `_Edge`, `_Glow`, `_SSS`, `_Alpha`). [`Profile::parse`] splits a stem; it does not say what the
//! game draws a stem as, which is the shader's program and its material's parameters.
//!
//! A material's samplers are named `<shader>_snp_Texture2D_<n>_<map>[_<k>]` (`C_DetailBlend__snp_Texture2D_7_AlbedoMap`,
//! `M_AMSN_Mb2__snp_Texture2D_1_GSBlendMap_AlbedoMap_0`): [`sampler_map`] returns the map the name stands for, with the
//! few spellings the shaders use for one kind of map folded together (`AlbedoMap`, `AlbedoMap_0`, ...).
//!
//! The grammar and the map spellings follow the community's soulstruct (`eldenring/models/shaders.py`, GPL-3.0-or-later),
//! which states them as a regular expression; what the census over every material binary of Elden Ring's install shows of
//! them is in the tests (`tests/shader_census.rs`). Its per-shader tables of which texture coordinate set a sampler reads
//! are not taken: they are hypotheses, and the tests hold them against the models.

/// The shader families whose stems carry their name in the first brackets or in a tag.
const FAMILIES: [&str; 8] = [
    "AMSN",
    "DetailBlend",
    "Fur",
    "Face",
    "Shell",
    "Crystal",
    "Hair",
    "Eye",
];

/// A shader stem split into the parts of the grammar `category[base_type][tag]..._suffix_suffix`.
///
/// The stem of a material binary is [`crate::names::stem`] of its [`crate::matbin::Material::shader`] with the case kept
/// (`C[DetailBlend]`, not `c[detailblend]`): the grammar is case-sensitive.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Profile {
    /// The ASCII letters before the first bracket: `C`, `CS`, `P`, `M`, `S`.
    pub category: String,
    /// The text of the first brackets: the family, possibly with a variant (`DetailBlend`, `AMSN_V`, `VA_Frame`).
    pub base_type: String,
    /// The text of each further bracket, in order (`S2`, `Mb2`, `Ov_AN`, `3Mask`, `Snow_VtxMask`).
    pub tags: Vec<String>,
    /// The parts after the first underscore past the brackets, split at the underscores (`cloth`, `Edge`, `Glow[Int]`);
    /// empty when the stem ends at the last bracket.
    pub suffixes: Vec<String>,
}

/// The text inside the bracket `s` starts with and the text after it. A bracket holds at least one character and ends at
/// the first `]`.
fn bracket(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix('[')?;
    let end = inner.find(']')?;
    if end == 0 {
        return None;
    }
    Some((inner.get(..end)?, inner.get(end.saturating_add(1)..)?))
}

/// The stem of a shader path: its file name without the extension, the case kept (`C[DetailBlend]` of
/// `N:\GR\data\INTERROOT_win64\material\spx\C[DetailBlend].spx`). [`crate::names::stem`] is the case key of the same
/// name and cannot be parsed: the grammar is case-sensitive.
pub fn stem(path: &str) -> &str {
    let leaf = path.rsplit(['/', '\\']).next().unwrap_or(path);
    leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem)
}

impl Profile {
    /// Split `stem` into category, brackets and suffixes; `None` for a stem outside the grammar: one with no letters and a
    /// bracket first (`GXFlver_ColDif`, `SpeedTree_5010f_w32187`, `MultiBlend2_et1`, `Test_C[AMSN]_Translucency`), a
    /// character after the last bracket that is not an underscore (`C[c2110]rope`), a lone underscore at the end, or a
    /// bracket left open or empty.
    pub fn parse(stem: &str) -> Option<Self> {
        let letters = stem.bytes().take_while(u8::is_ascii_alphabetic).count();
        if letters == 0 {
            return None;
        }
        let category = stem.get(..letters)?;
        let (base_type, mut rest) = bracket(stem.get(letters..)?)?;
        let mut tags = Vec::new();
        while let Some((tag, after)) = bracket(rest) {
            tags.push(tag.to_owned());
            rest = after;
        }
        let suffixes = if rest.is_empty() {
            Vec::new()
        } else {
            let suffix = rest.strip_prefix('_').filter(|s| !s.is_empty())?;
            suffix.split('_').map(str::to_owned).collect()
        };
        Some(Self {
            category: category.to_owned(),
            base_type: base_type.to_owned(),
            tags,
            suffixes,
        })
    }

    /// [`Profile::parse`] of the stem of a shader path ([`stem`]).
    pub fn of_shader(path: &str) -> Option<Self> {
        Self::parse(stem(path))
    }

    /// The stem this profile was split from.
    pub fn stem(&self) -> String {
        let mut s = format!("{}[{}]", self.category, self.base_type);
        for tag in &self.tags {
            s.push('[');
            s.push_str(tag);
            s.push(']');
        }
        for suffix in &self.suffixes {
            s.push('_');
            s.push_str(suffix);
        }
        s
    }

    /// The shader family: the last tag that names one of `AMSN`, `DetailBlend`, `Fur`, `Face`, `Shell`, `Crystal`, `Hair`,
    /// `Eye` (`CS[VA_Frame][Fur]` is `Fur`), else the first brackets when they are one of these, else the one these
    /// begin (`AMSN_V` is `AMSN`), else the first brackets as written (`ChrCustomize`, `Water`, `c3200`).
    pub fn family(&self) -> &str {
        if let Some(tag) = self.tags.iter().rev().find(|t| FAMILIES.contains(&t.as_str())) {
            return tag;
        }
        if FAMILIES.contains(&self.base_type.as_str()) {
            return &self.base_type;
        }
        FAMILIES
            .iter()
            .find(|family| self.base_type.starts_with(**family))
            .copied()
            .unwrap_or(self.base_type.as_str())
    }

    /// How many layers a multi-blend stem blends: the number after `Mb` in the first tag that has one (`Mb2`, `Mb3_ColorAdjust`,
    /// `Mb4_Ch2`), and 1 for a stem with no such tag.
    pub fn multi_blend_count(&self) -> u32 {
        self.tags
            .iter()
            .find_map(|tag| {
                let digits = tag.strip_prefix("Mb")?;
                let n = digits.bytes().take_while(u8::is_ascii_digit).count();
                digits.get(..n).filter(|d| !d.is_empty())?.parse().ok()
            })
            .unwrap_or(1)
    }

    /// The text after `Ov_` in the first overlay tag (`AN` of `[Ov_AN]`, `detail_AN`, `N_AlbedoMask_MetallicMask`).
    pub fn overlay(&self) -> Option<&str> {
        self.tags.iter().find_map(|tag| tag.strip_prefix("Ov_"))
    }

    /// Whether `tag` is one of the tags, as written.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Whether `suffix` is one of the suffixes, as written.
    pub fn has_suffix(&self, suffix: &str) -> bool {
        self.suffixes.iter().any(|s| s == suffix)
    }

    /// Whether the category is a character's: `C`, `CS` (a cut-scene character) or `P` (a player part).
    pub fn is_character(&self) -> bool {
        matches!(self.category.as_str(), "C" | "CS" | "P")
    }

    /// Whether the category is a map piece's (`M`).
    pub fn is_map(&self) -> bool {
        self.category == "M"
    }

    /// Whether the family is one character's own shader: `c` and three digits at the start of the family (`c2200`).
    pub fn is_specific_character(&self) -> bool {
        let family = self.family().as_bytes();
        family.first() == Some(&b'c') && family.get(1..4).is_some_and(|d| d.iter().all(u8::is_ascii_digit))
    }
}

/// The kinds of map the shaders' samplers are named for, with the spellings folded: `AlbedoMap` is [`MapType::Albedo`],
/// `BlendEdgeTexture` and `BlendEdge` both [`MapType::BlendEdge`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub enum MapType {
    /// `AlbedoMap`.
    Albedo,
    /// `NormalMap`, and a sampler named just `Normal`.
    Normal,
    /// `MetallicMap`.
    Metallic,
    /// `ShininessMap`.
    Shininess,
    /// `EmissiveMap`.
    Emissive,
    /// `VectorMap`.
    Vector,
    /// `ReflectanceMap`.
    Reflectance,
    /// `Mask1Map`: a one-channel mask.
    Mask1,
    /// `Mask3Map`: a three-channel mask.
    Mask3,
    /// `3Mask`.
    ThreeMask,
    /// `DistortionDepth`.
    DistortionDepth,
    /// `BlendMaskTexture`.
    BlendMask,
    /// `BlendEdgeTexture` and `BlendEdge`.
    BlendEdge,
    /// `UniqueOpacityMask`.
    UniqueOpacityMask,
    /// `Shadowmap`.
    Shadowmap,
    /// `DisplacementMap`.
    Displacement,
}

/// The map names of the table, as a sampler spells them, with what each stands for. A name that merely ends in one of
/// these (`DetailNormalMap`) stands for the first row it ends with.
const MAP_NAMES: [(&str, MapType); 18] = [
    ("AlbedoMap", MapType::Albedo),
    ("NormalMap", MapType::Normal),
    ("MetallicMap", MapType::Metallic),
    ("ShininessMap", MapType::Shininess),
    ("EmissiveMap", MapType::Emissive),
    ("VectorMap", MapType::Vector),
    ("ReflectanceMap", MapType::Reflectance),
    ("Mask1Map", MapType::Mask1),
    ("Mask3Map", MapType::Mask3),
    ("3Mask", MapType::ThreeMask),
    ("DistortionDepth", MapType::DistortionDepth),
    ("BlendMaskTexture", MapType::BlendMask),
    ("BlendEdgeTexture", MapType::BlendEdge),
    ("UniqueOpacityMask", MapType::UniqueOpacityMask),
    ("BlendEdge", MapType::BlendEdge),
    ("Normal", MapType::Normal),
    ("Shadowmap", MapType::Shadowmap),
    ("DisplacementMap", MapType::Displacement),
];

/// The names of the effect maps a sampler may end with when it carries no `_snp_Texture2D_<n>_` part: the label the
/// sampler stands for under, and the name it ends with.
const EFFECT_ENDINGS: [(&str, &str); 4] = [
    ("Fire_Mask", "__Fire___Mask"),
    ("Fire_Pattern", "__Fire___Pattern"),
    ("Fire_LightEdge", "__Fire___Light_Edge"),
    ("Fire_BlackEdge", "__Fire___Black_Edge"),
];

impl MapType {
    /// The map's short name: `Albedo`, `Normal`, `Mask1`, `3Mask`, `BlendEdge`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Albedo => "Albedo",
            Self::Normal => "Normal",
            Self::Metallic => "Metallic",
            Self::Shininess => "Shininess",
            Self::Emissive => "Emissive",
            Self::Vector => "Vector",
            Self::Reflectance => "Reflectance",
            Self::Mask1 => "Mask1",
            Self::Mask3 => "Mask3",
            Self::ThreeMask => "3Mask",
            Self::DistortionDepth => "DistortionDepth",
            Self::BlendMask => "BlendMask",
            Self::BlendEdge => "BlendEdge",
            Self::UniqueOpacityMask => "UniqueOpacityMask",
            Self::Shadowmap => "Shadowmap",
            Self::Displacement => "Displacement",
        }
    }
}

/// What a sampler's name says it holds.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub enum Map {
    /// One of the kinds of map the table names.
    Known(MapType),
    /// A map the table does not name, as the sampler spells it without the trailing index (`FlowMap`, `Fire___Mask`,
    /// `Generic__Cloud`), or an effect map the name ends with (`Fire_Mask`, `Fire_Pattern`, `Fire_LightEdge`,
    /// `Fire_BlackEdge`).
    Other(String),
}

/// `s` without a final underscore and digits (`AlbedoMap_0` is `AlbedoMap`), as the shortest non-empty start of `s` that
/// leaves nothing, or an underscore and digits only.
fn without_index(s: &str) -> Option<&str> {
    s.char_indices().find_map(|(at, c)| {
        let end = at.saturating_add(c.len_utf8());
        let rest = s.get(end..)?;
        let index_only = rest
            .strip_prefix('_')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        (rest.is_empty() || index_only).then(|| s.get(..end)).flatten()
    })
}

/// The map name in the part of a sampler's name after `_snp_Texture2D_<n>_`: an optional `GSBlendMap_`, an optional `RGB_`,
/// leading underscores dropped, the name, an optional index.
fn map_name(tail: &str) -> Option<&str> {
    let mut starts = Vec::with_capacity(4);
    for (gs, rgb) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut s = tail;
        if gs {
            s = match s.strip_prefix("GSBlendMap_") {
                Some(r) => r,
                None => continue,
            };
        }
        if rgb {
            s = match s.strip_prefix("RGB_") {
                Some(r) => r,
                None => continue,
            };
        }
        starts.push(s);
    }
    for s in starts {
        let underscores = s.bytes().take_while(|b| *b == b'_').count();
        for dropped in (0..=underscores).rev() {
            if let Some(name) = s.get(dropped..).and_then(without_index) {
                return Some(name);
            }
        }
    }
    None
}

/// The part of `sampler` after the last `_snp_Texture2D_<n>_` that leaves a map name.
fn after_slot(sampler: &str) -> Option<&str> {
    const MARK: &str = "_snp_Texture2D_";
    let mut marks: Vec<usize> = sampler.match_indices(MARK).map(|(at, _)| at).collect();
    marks.reverse();
    marks.into_iter().find_map(|at| {
        if at == 0 {
            return None;
        }
        let after = sampler.get(at.saturating_add(MARK.len())..)?;
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let tail = after.get(digits..)?.strip_prefix('_')?;
        map_name(tail)
    })
}

/// The map a sampler's name stands for: the name after `_snp_Texture2D_<n>_` (without a leading `GSBlendMap_` or `RGB_`,
/// leading underscores or a trailing `_<k>` index), looked up among the table's spellings, a name that ends in one of them
/// standing for it (`DetailNormalMap` is a normal map); a name the table does not know is [`Map::Other`] with the name
/// as extracted. `None` where the sampler has no such part and does not end in one of the effect maps (the legacy
/// samplers `g_DiffuseTexture`, `g_BumpmapTexture2` of the `GXFlver_*` shaders).
pub fn sampler_map(sampler: &str) -> Option<Map> {
    if let Some(name) = after_slot(sampler) {
        let name = name.trim_start_matches('_');
        if name.is_empty() {
            return None;
        }
        if let Some((_, kind)) = MAP_NAMES.iter().find(|(spelling, _)| *spelling == name) {
            return Some(Map::Known(*kind));
        }
        if let Some((_, kind)) = MAP_NAMES.iter().find(|(spelling, _)| name.ends_with(*spelling)) {
            return Some(Map::Known(*kind));
        }
        return Some(Map::Other(name.to_owned()));
    }
    EFFECT_ENDINGS
        .iter()
        .find(|(_, ending)| sampler.ends_with(*ending))
        .map(|(label, _)| Map::Other((*label).to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{Map, MapType, Profile, sampler_map, stem, without_index};

    fn parts(stem: &str) -> (String, String, Vec<String>, Vec<String>) {
        let p = Profile::parse(stem).unwrap_or_else(|| panic!("{stem} parses"));
        (p.category, p.base_type, p.tags, p.suffixes)
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_stem_splits_into_category_base_tags_and_suffixes() {
        assert_eq!(
            parts("C[DetailBlend][S2]_cloth"),
            (
                "C".into(),
                "DetailBlend".into(),
                strings(&["S2"]),
                strings(&["cloth"])
            )
        );
        assert_eq!(
            parts("M[AMSN_V][Mb2][Ov_AN]_Edge"),
            (
                "M".into(),
                "AMSN_V".into(),
                strings(&["Mb2", "Ov_AN"]),
                strings(&["Edge"])
            )
        );
        assert_eq!(
            parts("CS[VA_Frame][Fur]_FurBlur"),
            (
                "CS".into(),
                "VA_Frame".into(),
                strings(&["Fur"]),
                strings(&["FurBlur"])
            )
        );
        // a bracket after the suffixes is part of the suffix, not a tag
        assert_eq!(parts("C[AMSN]_Glow[Int]").3, strings(&["Glow[Int]"]));
        assert_eq!(parts("M[A]"), ("M".into(), "A".into(), vec![], vec![]));
        assert_eq!(parts("M[a]_b__c").3, strings(&["b", "", "c"]));
    }

    #[test]
    fn the_stem_of_a_shader_path_keeps_its_case() {
        assert_eq!(
            stem(r"N:\GR\data\INTERROOT_win64\material\spx\C[DetailBlend].spx"),
            "C[DetailBlend]"
        );
        assert_eq!(stem("M[Sky9].spx"), "M[Sky9]");
        assert_eq!(stem("C[AMSN]"), "C[AMSN]");
        assert_eq!(
            Profile::of_shader("x/y/M[AMSN_V][Mb2].spx").map(|p| p.multi_blend_count()),
            Some(2)
        );
    }

    #[test]
    fn stems_outside_the_grammar_are_none() {
        for stem in [
            "",
            "[SketchUp]",
            "GXFlver_ColDif",
            "SpeedTree_5010f_w32187",
            "Test_C[AMSN]_Translucency",
            "C[c2110]rope",
            "C[AMSN]_",
            "C[AMSN",
            "C[]",
            "C[A]x",
            "M[directional_grass][Far_ColorAdjust]]",
            "M[CutScene]M[AMSN_V]",
            "É[A]",
        ] {
            assert_eq!(Profile::parse(stem), None, "{stem:?}");
        }
    }

    #[test]
    fn a_profile_rebuilds_its_stem() {
        for stem in [
            "C[DetailBlend][S2]_cloth",
            "M[AMSN_V][Ov_N][Mb3_ColorAdjust][3Mask_ch18]",
            "C[AMSN]_Glow[Int]",
            "M[a]_b__c",
            "P[ChrCustomize][Hair]",
        ] {
            assert_eq!(Profile::parse(stem).map(|p| p.stem()).as_deref(), Some(stem));
        }
    }

    #[test]
    fn the_family_is_the_last_known_tag_then_the_base_then_its_prefix() {
        let family = |stem: &str| Profile::parse(stem).map(|p| p.family().to_owned()).unwrap();
        assert_eq!(family("CS[VA_Frame][Fur]_FurBlur"), "Fur");
        assert_eq!(family("C[DetailBlend][S2]"), "DetailBlend");
        assert_eq!(family("M[AMSN_V][Ov_N]"), "AMSN");
        assert_eq!(family("C[Fur][Shell]"), "Shell");
        assert_eq!(family("P[ChrCustomize][Hair]"), "Hair");
        assert_eq!(family("P[ChrCustomize]"), "ChrCustomize");
        assert_eq!(family("M[Waterfall2]"), "Waterfall2");
        assert_eq!(family("C[c2200]_Body"), "c2200");
    }

    #[test]
    fn multi_blend_overlay_and_flags() {
        let p = |stem| Profile::parse(stem).unwrap();
        assert_eq!(p("M[AMSN_V][Mb2][Ov_N]").multi_blend_count(), 2);
        assert_eq!(
            p("M[AMSN_V][Ov_N][Mb3_ColorAdjust][3Mask_ch18]").multi_blend_count(),
            3
        );
        assert_eq!(p("M[AMSN_V][Mb4_Ch2][3Mask][Ov_AN]").multi_blend_count(), 4);
        assert_eq!(p("M[AMSN_V][Mb][Mb5]").multi_blend_count(), 5);
        assert_eq!(p("M[AMSN_V]").multi_blend_count(), 1);
        assert_eq!(p("M[AMSN_V][Mb2][Ov_AN]").overlay(), Some("AN"));
        assert_eq!(p("M[AMSN_V][Ov_detail_AN]").overlay(), Some("detail_AN"));
        assert_eq!(p("M[AMSN_V]").overlay(), None);
        assert!(p("C[DetailBlend]_cloth").has_suffix("cloth"));
        assert!(!p("M[AMSN_V][Snow_VtxMask]").has_tag("Snow"));
        assert!(p("M[AMSN_V][Snow]").has_tag("Snow"));
        assert!(p("CS[AMSN]").is_character() && p("P[Hair]").is_character() && !p("M[A]").is_character());
        assert!(p("M[A]").is_map());
        assert!(p("C[c2200]_Body").is_specific_character());
        assert!(!p("C[c22]_Body").is_specific_character());
        assert!(!p("C[AMSN]").is_specific_character());
    }

    #[test]
    fn an_index_is_only_dropped_at_the_end() {
        assert_eq!(without_index("AlbedoMap_0"), Some("AlbedoMap"));
        assert_eq!(without_index("AlbedoMap"), Some("AlbedoMap"));
        assert_eq!(without_index("Generic__1"), Some("Generic_"));
        assert_eq!(without_index("A_1_2"), Some("A_1"));
        assert_eq!(without_index("A_12"), Some("A"));
        assert_eq!(without_index("A_x"), Some("A_x"));
        assert_eq!(without_index("_1"), Some("_1"));
        assert_eq!(without_index(""), None);
    }

    #[test]
    fn a_sampler_stands_for_its_map() {
        let known = |s: &str| sampler_map(s).and_then(|m| if let Map::Known(k) = m { Some(k) } else { None });
        assert_eq!(
            known("C_DetailBlend__snp_Texture2D_7_AlbedoMap"),
            Some(MapType::Albedo)
        );
        assert_eq!(
            known("M_AMSN_Mb2__snp_Texture2D_1_GSBlendMap_AlbedoMap_0"),
            Some(MapType::Albedo)
        );
        assert_eq!(
            known("X_snp_Texture2D_3_RGB_BlendEdgeTexture"),
            Some(MapType::BlendEdge)
        );
        assert_eq!(known("X_snp_Texture2D_3__BlendEdge_1"), Some(MapType::BlendEdge));
        assert_eq!(
            known("X_snp_Texture2D_0_ReflectanceMap_0"),
            Some(MapType::Reflectance)
        );
        assert_eq!(known("X_snp_Texture2D_7_Normal"), Some(MapType::Normal));
        // a name that ends in a map's spelling stands for it
        assert_eq!(known("X_snp_Texture2D_2_DetailNormalMap"), Some(MapType::Normal));
        assert_eq!(
            sampler_map("C_Crystal__snp_Texture2D_2__DistortionDepth"),
            Some(Map::Known(MapType::DistortionDepth))
        );
        assert_eq!(
            sampler_map("X_snp_Texture2D_4_FlowMap"),
            Some(Map::Other("FlowMap".into()))
        );
        assert_eq!(
            sampler_map("X_snp_Texture2D_4__Fire___Pattern"),
            Some(Map::Other("Fire___Pattern".into()))
        );
        // the last slot part wins
        assert_eq!(
            known("A_snp_Texture2D_1_NormalMap_snp_Texture2D_2_AlbedoMap"),
            Some(MapType::Albedo)
        );
        // the effect endings count where there is no slot part
        assert_eq!(
            sampler_map("Fx__Fire___Light_Edge"),
            Some(Map::Other("Fire_LightEdge".into()))
        );
        for legacy in [
            "g_DiffuseTexture",
            "g_BumpmapTexture2",
            "",
            "_snp_Texture2D_1_AlbedoMap",
        ] {
            assert_eq!(sampler_map(legacy), None, "{legacy:?}");
        }
        assert_eq!(MapType::ThreeMask.name(), "3Mask");
    }
}
