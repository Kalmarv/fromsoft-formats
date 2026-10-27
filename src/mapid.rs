//! Map identifiers as the games pack them in four bytes: the area, the two grid coordinates and the variant of a map
//! name `mAA_XX_ZZ_dd` (`m60_44_36_10`, `m10_00_00_00`).
//!
//! The same four bytes are read in two orders. A map word an MSB stores as a little-endian `i32` (the extra block of a
//! region or event, a part's tile-load config) has the area in its top byte ([`MapId::from_msb_word`]). The four bytes
//! a connect collision stores, and the words event scripts pass as map arguments, hold the area in the first byte in
//! file order ([`MapId::from_bytes`]; an event argument is read as a little-endian `u32`, so [`MapId::from_bytes`] of
//! its `to_le_bytes`). The area, X and Z bytes hold the name's numbers (60, 44, 36); the variant holds the name's two
//! digits as hexadecimal digits (`_10` is the byte `0x10`). A byte of `0xff` stands for any.
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

    /// Whether any field is the wildcard.
    pub const fn has_wildcard(self) -> bool {
        self.area == Self::WILDCARD
            || self.x == Self::WILDCARD
            || self.z == Self::WILDCARD
            || self.variant == Self::WILDCARD
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
