//! Map identifiers as the games pack them in four bytes: the area, the two grid coordinates and the variant of a map
//! name `mAA_XX_ZZ_dd` (`m60_44_36_10`, `m10_00_00_00`).
//!
//! The same four bytes are read in two orders. A map word an MSB stores as a little-endian `i32` (the extra block of a
//! region or event, a part's tile-load config) has the area in its top byte ([`MapId::from_msb_word`]). The four bytes
//! a connect collision stores, and the words event scripts pass as map arguments, hold the area in the first byte in
//! file order ([`MapId::from_bytes`]; an event argument is read as a little-endian `u32`, so [`MapId::from_bytes`] of
//! its `to_le_bytes`). The area, X and Z bytes hold the name's numbers (60, 44, 36); the variant holds the name's two
//! digits as hexadecimal digits (`_10` is the byte `0x10`). A byte of `0xff` stands for any.
//!
//! Elden Ring's event scripts also pass a map as one decimal number, the digits of the name run together
//! (`60444500` is `m60_44_45_00`): [`MapId::from_decimal`].
use std::fmt;

/// A map's identity, as four bytes: the area, the grid X and Z, and the variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize)]
pub struct MapId {
    /// The area (`60` of `m60_44_36_10`).
    pub area: u8,
    /// The grid X (`44`): the block number in a dungeon's name.
    pub x: u8,
    /// The grid Z (`36`).
    pub z: u8,
    /// The variant byte (`0x10` of `_10`): the name's two digits as hexadecimal digits.
    pub variant: u8,
}

impl MapId {
    /// The byte that stands for any value of its field.
    pub const WILDCARD: u8 = 0xff;

    /// The identity of four bytes in file order: area, X, Z, variant.
    pub const fn from_bytes(bytes: [u8; 4]) -> MapId {
        let [area, x, z, variant] = bytes;
        MapId { area, x, z, variant }
    }

    /// The identity of a map word an MSB stores as a little-endian `i32`: the variant in the low byte, then Z, then X,
    /// and the area in the top byte.
    pub const fn from_msb_word(word: u32) -> MapId {
        MapId::from_bytes(word.to_be_bytes())
    }

    /// The identity of a decimal map number: the four two-digit numbers of the name run together, `AABBCCDD` (the
    /// `Map ID` argument of Elden Ring's cutscene-and-warp instructions, `11050000` for `m11_05_00_00`). `None` for a
    /// negative number or one of more than eight digits.
    pub fn from_decimal(number: i32) -> Option<MapId> {
        let n = u32::try_from(number).ok()?;
        if n > 99_999_999 {
            return None;
        }
        let variant = n % 100;
        Some(MapId {
            area: u8::try_from(n / 1_000_000).ok()?,
            x: u8::try_from(n / 10_000 % 100).ok()?,
            z: u8::try_from(n / 100 % 100).ok()?,
            // the variant byte holds the two digits as hexadecimal digits
            variant: u8::try_from(((variant / 10) << 4) | (variant % 10)).ok()?,
        })
    }

    /// The decimal number of [`MapId::from_decimal`]: `None` for an identity with a wildcard, or one whose numbers or
    /// variant digits are not the two decimal digits a name holds (the same identities that have no [`MapId::name`]).
    pub fn to_decimal(self) -> Option<i32> {
        self.name()?;
        let variant = u32::from(self.variant >> 4)
            .checked_mul(10)?
            .checked_add(u32::from(self.variant & 15))?;
        let number = u32::from(self.area)
            .checked_mul(1_000_000)?
            .checked_add(u32::from(self.x).checked_mul(10_000)?)?
            .checked_add(u32::from(self.z).checked_mul(100)?)?
            .checked_add(variant)?;
        i32::try_from(number).ok()
    }

    /// Whether this identity, whose wildcard bytes stand for any value, includes `other`: every field is the wildcard
    /// or equals `other`'s.
    pub const fn covers(self, other: MapId) -> bool {
        (self.area == Self::WILDCARD || self.area == other.area)
            && (self.x == Self::WILDCARD || self.x == other.x)
            && (self.z == Self::WILDCARD || self.z == other.z)
            && (self.variant == Self::WILDCARD || self.variant == other.variant)
    }

    /// Whether any field is the wildcard.
    pub const fn has_wildcard(self) -> bool {
        self.area == Self::WILDCARD
            || self.x == Self::WILDCARD
            || self.z == Self::WILDCARD
            || self.variant == Self::WILDCARD
    }

    /// The identity of a map name `m60_44_36_10`, the inverse of [`MapId::name`]: `m`, then four groups of two decimal
    /// digits joined by `_`. `None` for anything else.
    pub fn from_name(name: &str) -> Option<MapId> {
        let mut groups = name.strip_prefix('m')?.split('_');
        let mut next = || -> Option<&str> {
            groups
                .next()
                .filter(|g| g.len() == 2 && g.bytes().all(|b| b.is_ascii_digit()))
        };
        let (area, x, z, variant) = (next()?, next()?, next()?, next()?);
        if groups.next().is_some() {
            return None;
        }
        Some(MapId {
            area: area.parse().ok()?,
            x: x.parse().ok()?,
            z: z.parse().ok()?,
            // the variant byte holds the two digits as hexadecimal digits
            variant: u8::from_str_radix(variant, 16).ok()?,
        })
    }

    /// The map's name, `m60_44_36_10`: `None` for an identity with a wildcard, or one whose numbers or variant digits
    /// are not the two decimal digits a name holds.
    pub fn name(self) -> Option<String> {
        let digits = |b: u8| (b <= 99).then_some(b);
        let variant = (self.variant >> 4 <= 9 && self.variant & 15 <= 9).then_some(self.variant);
        Some(format!(
            "m{:02}_{:02}_{:02}_{:02x}",
            digits(self.area)?,
            digits(self.x)?,
            digits(self.z)?,
            variant?
        ))
    }
}

impl fmt::Display for MapId {
    /// The name, or the four bytes in hexadecimal where there is none (`3c2926ff`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(&name),
            None => write!(
                f,
                "{:02x}{:02x}{:02x}{:02x}",
                self.area, self.x, self.z, self.variant
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MapId;

    #[test]
    fn an_msb_word_has_the_area_in_its_top_byte() {
        let id = MapId::from_msb_word(0x3c2c_2410);
        assert_eq!(id, MapId::from_bytes([60, 44, 36, 0x10]));
        assert_eq!(id.name().as_deref(), Some("m60_44_36_10"));
        assert_eq!(
            MapId::from_msb_word(0x0a00_0000).name().as_deref(),
            Some("m10_00_00_00")
        );
        assert_eq!(MapId::from_msb_word(0x3c21_2900).to_string(), "m60_33_41_00");
    }

    #[test]
    fn an_event_argument_has_the_area_in_its_low_byte() {
        // a little-endian u32 whose bytes in file order are area, X, Z, variant
        let argument = u32::from_le_bytes([60, 44, 36, 0x10]);
        assert_eq!(argument, 0x1024_2c3c);
        assert_eq!(
            MapId::from_bytes(argument.to_le_bytes()).name().as_deref(),
            Some("m60_44_36_10")
        );
    }

    #[test]
    fn a_decimal_number_runs_the_name_digits_together() {
        let id = MapId::from_decimal(60_444_500).unwrap();
        assert_eq!(id, MapId::from_bytes([60, 44, 45, 0]));
        assert_eq!(id.name().as_deref(), Some("m60_44_45_00"));
        assert_eq!(
            MapId::from_decimal(11_050_000).unwrap().to_string(),
            "m11_05_00_00"
        );
        // the variant digits are stored as hexadecimal digits, as in a name
        assert_eq!(MapId::from_decimal(14_000_099).unwrap().variant, 0x99);
        assert_eq!(
            MapId::from_decimal(14_000_010).unwrap(),
            MapId::from_name("m14_00_00_10").unwrap()
        );
        // and read back
        for name in ["m60_44_36_10", "m10_00_00_00", "m14_00_00_99", "m61_51_43_00"] {
            let id = MapId::from_name(name).unwrap();
            assert_eq!(MapId::from_decimal(id.to_decimal().unwrap()), Some(id), "{name}");
        }
        assert_eq!(MapId::from_decimal(0), Some(MapId::from_bytes([0, 0, 0, 0])));
        // negative numbers and numbers of more than eight digits are no map
        assert_eq!(MapId::from_decimal(-1), None);
        assert_eq!(MapId::from_decimal(100_000_000), None);
        assert_eq!(MapId::from_decimal(i32::MAX), None);
        // a wildcard, or a variant that is not two decimal digits, has no decimal number
        assert_eq!(MapId::from_bytes([60, 255, 255, 255]).to_decimal(), None);
        assert_eq!(MapId::from_bytes([60, 44, 36, 0x1a]).to_decimal(), None);
    }

    #[test]
    fn a_name_reads_back_as_the_identity_it_names() {
        for name in [
            "m60_44_36_10",
            "m10_00_00_00",
            "m14_00_00_99",
            "m61_51_43_00",
            "m99_99_99_99",
        ] {
            let id = MapId::from_name(name).unwrap();
            assert_eq!(id.name().as_deref(), Some(name));
        }
        assert_eq!(
            MapId::from_name("m60_44_36_10"),
            Some(MapId::from_bytes([60, 44, 36, 0x10]))
        );
        for bad in [
            "",
            "m",
            "60_44_36_10",
            "m60_44_36",
            "m60_44_36_10_00",
            "m60_4_36_10",
            "m60_44_36_1a",
            "m60_-4_36_10",
            "m60_44_36_100",
            "M60_44_36_10",
            "m60_44_36_\u{ff11}0",
        ] {
            assert_eq!(MapId::from_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_wildcard_covers_any_value_of_its_field() {
        let any_variant = MapId::from_bytes([60, 44, 36, 0xff]);
        assert!(any_variant.covers(MapId::from_bytes([60, 44, 36, 0x10])));
        assert!(any_variant.covers(any_variant));
        assert!(!any_variant.covers(MapId::from_bytes([60, 44, 37, 0])));
        // an identity without a wildcard covers itself only
        let one = MapId::from_bytes([60, 44, 36, 0x10]);
        assert!(one.covers(one) && !one.covers(MapId::from_bytes([60, 44, 36, 0])));
    }

    #[test]
    fn a_wildcard_has_no_name() {
        let id = MapId::from_bytes([0x3c, 0x29, 0x26, 0xff]);
        assert!(id.has_wildcard());
        assert_eq!(id.name(), None);
        assert_eq!(id.to_string(), "3c2926ff");
        // a variant of non-decimal digits, or a number of three digits, is no name either
        assert_eq!(MapId::from_bytes([60, 44, 36, 0x1a]).name(), None);
        assert_eq!(MapId::from_bytes([60, 100, 36, 0]).name(), None);
        assert!(!MapId::from_bytes([60, 44, 36, 0x12]).has_wildcard());
    }
}
