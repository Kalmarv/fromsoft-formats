//! The community's names for the words of a material's GX items.
//!
//! A FLVER material carries extension items (`GxItem`: a four-byte id, a version and a payload of 32-bit words) that set
//! one property each: the emissive intensity (`GX03`), an albedo colour (`GX14`), the translucency scale (`GX05`), the
//! tessellation and displacement settings (`GX04`), a parallax offset (`GX09`), the water settings (`GX07`), a texture's
//! scroll speed (`GX20` to `GX32`). The file does not name the words. Smithbox's `GX_Item_Descriptors.json` does, and
//! [`ItemInfo`] holds its names for the items the installs store, from `tools/gen_gx_items.py`; the words it names fit
//! the values stored (ranges, integers where it says integer: `tests/gx_census.rs` holds every item of both installs to
//! it), which is all the evidence there is: the names are the community's and no value was seen to drive the game.
//!
//! [`GxItem::fields`] gives an item's words under those names; [`Material`]'s accessors (`emissive_intensity`,
//! `albedo_colour`, `translucency_scale`, `parallax`, `water`) read the properties the renderers lower. The items
//! the community names and neither install stores (the parallax-occlusion settings `GX02` and `GX12`, `GX01`, `GX08`,
//! `GX13`, `GX35` to `GX37`) are not here.
use super::{GxItem, Material, gx_items::ITEMS};
use crate::{Result, invalid};

const FORMAT: &str = "GX item";

/// How the descriptor types a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordKind {
    /// A word the descriptor does not name: kept as stored ([`Value::Raw`]).
    Unknown,
    /// An integer ([`Value::Int`]).
    Int,
    /// A float ([`Value::Float`]).
    Float,
    /// An integer the descriptor types as a choice (the tessellation mode: 0 or 1 in the installs).
    Choice,
    /// An integer the descriptor types as a switch (the displacement and decal switches: 0 or 1 in the installs).
    Flag,
}

/// The community's name for one word of an item and the type it gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WordInfo {
    /// The community's name, as written (`[Emissive] EmissiveIntensity`, `Albedo1 ScrollSpeedU`, `Unknown`).
    pub name: &'static str,
    /// The type it gives the word.
    pub kind: WordKind,
}

/// What the community names of an item: the words in order, from a (id, version) the installs store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemInfo {
    /// The item's id (`GX03`).
    pub id: [u8; 4],
    /// The item's version.
    pub version: u32,
    /// The group the descriptor files it under (`Tessellation`), if any.
    pub category: Option<&'static str>,
    /// Its words in order. An item may store fewer: Elden Ring's `GX14` stores the first four of seven.
    pub words: &'static [WordInfo],
}

/// The names of the item `id` in `version`, if the installs store it.
pub fn info(id: [u8; 4], version: u32) -> Option<&'static ItemInfo> {
    ITEMS.iter().find(|i| i.id == id && i.version == version)
}

/// Every item the installs store whose words the community names, in table order.
pub fn items() -> &'static [ItemInfo] {
    ITEMS
}

/// One word of an item.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    /// A word the descriptor does not name, as stored.
    Raw(u32),
    /// An integer.
    Int(i32),
    /// A float (finite).
    Float(f32),
}

/// One word of an item under its community name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// The community's name for the word ([`WordInfo::name`]).
    pub name: &'static str,
    /// Its value.
    pub value: Value,
}

impl GxItem {
    /// The community's names of this item, where the installs store it ([`info`]).
    pub fn info(&self) -> Option<&'static ItemInfo> {
        info(self.id, self.version)
    }

    /// The payload as 32-bit words; a payload that is not a whole number of words is refused.
    pub fn words(&self) -> Result<Vec<u32>> {
        let (words, rest) = self.data.as_chunks::<4>();
        if !rest.is_empty() {
            return Err(invalid(
                FORMAT,
                self.data.len(),
                format!("{} payload bytes are not whole words", self.data.len()),
            ));
        }
        Ok(words.iter().map(|w| u32::from_le_bytes(*w)).collect())
    }

    /// The words of the payload under the community's names, in order: `None` for an item the installs do not store or
    /// the community does not name. An item that stores fewer words than the descriptor lists gives the ones it stores;
    /// one that stores more, a float that is not finite, or a payload that is not whole words is refused.
    pub fn fields(&self) -> Result<Option<Vec<Field>>> {
        let Some(info) = self.info() else {
            return Ok(None);
        };
        let words = self.words()?;
        if words.len() > info.words.len() {
            return Err(invalid(
                FORMAT,
                self.data.len(),
                format!(
                    "{} words, the descriptor of {} v{} lists {}",
                    words.len(),
                    String::from_utf8_lossy(&self.id),
                    self.version,
                    info.words.len()
                ),
            ));
        }
        let mut out = Vec::with_capacity(words.len());
        for (at, (word, named)) in words.iter().zip(info.words).enumerate() {
            let value = match named.kind {
                WordKind::Unknown => Value::Raw(*word),
                WordKind::Int | WordKind::Choice | WordKind::Flag => {
                    Value::Int(i32::from_ne_bytes(word.to_ne_bytes()))
                }
                WordKind::Float => {
                    let f = f32::from_bits(*word);
                    if !f.is_finite() {
                        return Err(invalid(
                            FORMAT,
                            at.saturating_mul(4),
                            format!("{}: non-finite float", named.name),
                        ));
                    }
                    Value::Float(f)
                }
            };
            out.push(Field {
                name: named.name,
                value,
            });
        }
        Ok(Some(out))
    }
}

/// `GX14`: a colour the albedo is multiplied by, and a multiplier.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlbedoColour {
    /// Red, green and blue (0 to 1 in the installs).
    pub rgb: [f32; 3],
    /// The multiplier (0.79 to 1.36 in Elden Ring, 0.8 to 1 in Sekiro).
    pub multiplier: f32,
}

/// One layer of `GX09`: an offset and the height the parallax is taken about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParallaxLayer {
    /// The offset (0 in the installs).
    pub offset: f32,
    /// The middle height (0.5 in the installs).
    pub middle_height: f32,
}

/// `GX07`: the settings of a water surface. Every `scroll` is a speed in texture units per second as (U, V).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Water {
    /// The three normal maps' scroll speeds.
    pub normal_scroll: [[f32; 2]; 3],
    /// The two diffuse maps' scroll speeds.
    pub diffuse_scroll: [[f32; 2]; 2],
    /// The Fresnel term's minimum and maximum.
    pub fresnel: [f32; 2],
    /// The reflection offset.
    pub reflection_offset: f32,
    /// The refraction offset.
    pub refraction_offset: f32,
    /// The depth fog's colour as red, green, blue and alpha.
    pub depth_fog_colour: [f32; 4],
    /// The depth fog's density.
    pub depth_fog_density: f32,
    /// The surface's transparency.
    pub transparency: f32,
    /// The two fade ranges (`waterFadeRange` and `waterFadeRange2`).
    pub fade_range: [f32; 2],
}

impl Material {
    /// The first GX item of this id.
    pub fn gx_item(&self, id: [u8; 4]) -> Option<&GxItem> {
        self.gx.iter().find(|g| g.id == id)
    }

    /// The first `count` words of the material's item `id` as floats, `None` where it has no such item; an item of
    /// another version than `version`, one with fewer words, or a word that is not a finite float is refused.
    fn gx_floats(&self, id: [u8; 4], version: u32, count: usize) -> Result<Option<Vec<f32>>> {
        let Some(item) = self.gx_item(id) else {
            return Ok(None);
        };
        if item.version != version {
            return Err(invalid(
                FORMAT,
                0,
                format!(
                    "{} stored in version {}, the words are named for version {version}",
                    String::from_utf8_lossy(&id),
                    item.version
                ),
            ));
        }
        let words = item.words()?;
        let floats: Vec<f32> = words.iter().take(count).map(|w| f32::from_bits(*w)).collect();
        if floats.len() < count || floats.iter().any(|f| !f.is_finite()) {
            return Err(invalid(
                FORMAT,
                item.data.len(),
                format!(
                    "{} stores {} words, {count} finite floats are needed",
                    String::from_utf8_lossy(&id),
                    words.len()
                ),
            ));
        }
        Ok(Some(floats))
    }

    /// `GX03`: the emissive intensity (0.7 to 3.32 in Sekiro's, 0.6 and 1 in Elden Ring's items).
    pub fn emissive_intensity(&self) -> Result<Option<f32>> {
        Ok(self.gx_floats(*b"GX03", 100, 1)?.and_then(|f| f.first().copied()))
    }

    /// `GX05`: the translucency scale (1 or 10 in Sekiro's items, always 1 in Elden Ring's).
    pub fn translucency_scale(&self) -> Result<Option<f32>> {
        Ok(self.gx_floats(*b"GX05", 100, 1)?.and_then(|f| f.first().copied()))
    }

    /// `GX14`: the albedo colour and its multiplier.
    pub fn albedo_colour(&self) -> Result<Option<AlbedoColour>> {
        Ok(self.gx_floats(*b"GX14", 101, 4)?.and_then(|f| {
            let [r, g, b, multiplier]: [f32; 4] = f.try_into().ok()?;
            Some(AlbedoColour {
                rgb: [r, g, b],
                multiplier,
            })
        }))
    }

    /// `GX09`: the two parallax layers.
    pub fn parallax(&self) -> Result<Option<[ParallaxLayer; 2]>> {
        Ok(self.gx_floats(*b"GX09", 100, 4)?.and_then(|f| {
            let [o1, h1, o2, h2]: [f32; 4] = f.try_into().ok()?;
            Some([
                ParallaxLayer {
                    offset: o1,
                    middle_height: h1,
                },
                ParallaxLayer {
                    offset: o2,
                    middle_height: h2,
                },
            ])
        }))
    }

    /// `GX07`: the water settings.
    pub fn water(&self) -> Result<Option<Water>> {
        Ok(self.gx_floats(*b"GX07", 104, 22)?.and_then(|f| {
            let at = |i: usize| f.get(i).copied();
            Some(Water {
                normal_scroll: [[at(0)?, at(1)?], [at(2)?, at(3)?], [at(20)?, at(21)?]],
                diffuse_scroll: [[at(4)?, at(5)?], [at(6)?, at(7)?]],
                fresnel: [at(8)?, at(9)?],
                reflection_offset: at(10)?,
                refraction_offset: at(11)?,
                depth_fog_colour: [at(12)?, at(13)?, at(14)?, at(15)?],
                depth_fog_density: at(16)?,
                transparency: at(17)?,
                fade_range: [at(18)?, at(19)?],
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::{Value, info, items};
    use crate::flver::{GxItem, Material};

    fn item(id: &[u8; 4], version: u32, words: &[u32]) -> GxItem {
        GxItem {
            id: *id,
            version,
            data: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
        }
    }

    fn material(gx: Vec<GxItem>) -> Material {
        Material {
            name: String::new(),
            mtd: String::new(),
            textures: Vec::new(),
            string_bytes: 0,
            index: 0,
            gx,
        }
    }

    #[test]
    fn the_table_is_sorted_and_every_word_has_a_name() {
        for i in items() {
            assert!(!i.words.is_empty());
            assert!(i.words.iter().all(|w| !w.name.is_empty()));
        }
        let keys: Vec<_> = items().iter().map(|i| (i.id, i.version)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(keys, sorted, "ids in order, one entry for each id and version");
        assert_eq!(info(*b"GX03", 100).map(|i| i.words.len()), Some(4));
        assert_eq!(info(*b"GX03", 101), None);
        assert_eq!(info(*b"GXMD", 123), None);
    }

    #[test]
    fn fields_carry_the_community_names() {
        let emissive = item(b"GX03", 100, &[1.5f32.to_bits(), 0, 0, 0]);
        let fields = emissive.fields().unwrap().unwrap();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0].name, "[Emissive] EmissiveIntensity");
        assert_eq!(fields[0].value, Value::Float(1.5));
        assert_eq!(fields[1].value, Value::Raw(0));
        // GX04's first words are switches and a choice
        let tess = item(
            b"GX04",
            100,
            &[1, 1, 5f32.to_bits(), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        let f = tess.fields().unwrap().unwrap();
        assert_eq!((f[0].value, f[1].value), (Value::Int(1), Value::Int(1)));
        // fewer words than the descriptor lists are fine (Elden Ring's GX14 stores four of seven)
        let short = item(
            b"GX14",
            101,
            &[1f32.to_bits(), 1f32.to_bits(), 1f32.to_bits(), 1f32.to_bits()],
        );
        assert_eq!(short.fields().unwrap().unwrap().len(), 4);
        // more are refused, as is a payload that is not whole words and a float that is not finite
        assert!(item(b"GX03", 100, &[0; 5]).fields().is_err());
        let mut odd = item(b"GX03", 100, &[0; 4]);
        odd.data.push(0);
        assert!(odd.fields().is_err() && odd.words().is_err());
        assert!(
            item(b"GX03", 100, &[f32::NAN.to_bits(), 0, 0, 0])
                .fields()
                .is_err()
        );
        // an item nobody names is None, not an error
        assert!(item(b"GXMD", 123, &[1, 2, 3, 4]).fields().unwrap().is_none());
    }

    #[test]
    fn a_material_reads_its_properties() {
        let m = material(vec![
            item(b"GX03", 100, &[3.32f32.to_bits(), 0, 0, 0]),
            item(b"GX05", 100, &[10f32.to_bits(), 0, 0, 0]),
            item(
                b"GX14",
                101,
                &[
                    0.25f32.to_bits(),
                    0.5f32.to_bits(),
                    1f32.to_bits(),
                    0.8f32.to_bits(),
                ],
            ),
        ]);
        assert_eq!(m.emissive_intensity().unwrap(), Some(3.32));
        assert_eq!(m.translucency_scale().unwrap(), Some(10.));
        let c = m.albedo_colour().unwrap().unwrap();
        assert_eq!((c.rgb, c.multiplier), ([0.25, 0.5, 1.], 0.8));
        assert_eq!(m.parallax().unwrap(), None);
        assert_eq!(m.water().unwrap(), None);
        // an item of another version than the names are for, or with too few words, is refused
        assert!(
            material(vec![item(b"GX03", 101, &[0; 4])])
                .emissive_intensity()
                .is_err()
        );
        assert!(
            material(vec![item(b"GX14", 101, &[0; 3])])
                .albedo_colour()
                .is_err()
        );
    }

    #[test]
    fn water_words_are_named_in_order() {
        let words: Vec<u32> = (0..29).map(|i| (i as f32 + 1.).to_bits()).collect();
        let water = material(vec![item(b"GX07", 104, &words)])
            .water()
            .unwrap()
            .unwrap();
        assert_eq!(water.normal_scroll, [[1., 2.], [3., 4.], [21., 22.]]);
        assert_eq!(water.diffuse_scroll, [[5., 6.], [7., 8.]]);
        assert_eq!(water.fresnel, [9., 10.]);
        assert_eq!((water.reflection_offset, water.refraction_offset), (11., 12.));
        assert_eq!(water.depth_fog_colour, [13., 14., 15., 16.]);
        assert_eq!((water.depth_fog_density, water.transparency), (17., 18.));
        assert_eq!(water.fade_range, [19., 20.]);
    }
}
