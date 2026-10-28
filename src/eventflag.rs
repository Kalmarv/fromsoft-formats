//! Event flag numbers: how the games lay a flag's number out and where they keep the flag, and which map a number
//! belongs to by its digits.
//!
//! **Sekiro.** A flag is a decimal number `R AA G Z nnn` (at most eight digits): a region digit, a two-digit area, a block
//! group digit, a zone digit and a three-digit number inside the zone. The game keeps 32 flags in a word, so the flag
//! `nnn` is bit `31 - nnn % 32` of word `nnn / 32` of its zone. A flag whose area is 90 or more, or whose area and group
//! digits are both zero, is global: the game keeps those apart from the blocks'.
//!
//! **Elden Ring.** A flag is stored by its *category*, the number divided by 1000, in a block of 125 bytes (1000 bits):
//! the flag `nnn` is bit `7 - nnn % 8` of byte `nnn / 8`. The category is where the digits mean a map, by the convention
//! the stock scripts follow: eight digits `AA BB d nnn` are a block of a legacy dungeon (`m12_01`), ten digits `10 XX ZZ d
//! nnn` a tile of the open world (`m60_XX_ZZ`) and `20 XX ZZ d nnn` a tile of the Land of Shadow (`m61_XX_ZZ`); a shorter
//! number names no map.
//!
//! The addressing is the games' (read from their flag managers); the map conventions are what the stock scripts show
//! (`tests/emedf_census.rs`: of the flags that name a map in the scripts of Elden Ring's dungeons 3,616 of 3,775 are the
//! script's own block's, in its tiles' scripts 2,686 of 3,212 are the script's own tile's).

use crate::Game;
use crate::mapid::MapId;

/// Where the game stores a flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Address {
    /// Sekiro: the digits of the number and the flag's place in its zone.
    Sekiro {
        /// The region digit (`R` of `R AA G Z nnn`).
        region: u8,
        /// The area (`AA`).
        area: u8,
        /// The block group digit (`G`).
        group: u8,
        /// The zone digit (`Z`).
        zone: u8,
        /// The word of the zone that holds the flag (`nnn / 32`).
        word: u8,
        /// The bit of that word, counted from the least significant bit (`31 - nnn % 32`).
        bit: u8,
    },
    /// Elden Ring: the category and the flag's place in its block.
    EldenRing {
        /// The number divided by 1000.
        category: u32,
        /// The byte of the block that holds the flag (`nnn / 8`).
        byte: u8,
        /// The bit of that byte, counted from the least significant bit (`7 - nnn % 8`).
        bit: u8,
    },
}

/// Which map's flags a number belongs to, by its digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scope {
    /// No map: the digits name none.
    Global,
    /// A block (`m12_01`, region and variant unspecified) or a tile (`m60_44_36`, variant unspecified); the bytes the
    /// number does not give are the wildcard ([`MapId::covers`]).
    Map(MapId),
}

/// A number of at most two decimal digits as a byte.
fn digits(n: u32) -> u8 {
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// Where `game` keeps the flag `flag`. `None` for a number the game's layout does not hold (Sekiro's flags have at most
/// eight digits).
pub fn address(game: Game, flag: u32) -> Option<Address> {
    let n = flag % 1000;
    match game {
        Game::Sekiro => {
            if flag > 99_999_999 {
                return None;
            }
            Some(Address::Sekiro {
                region: digits(flag / 10_000_000 % 10),
                area: digits(flag / 100_000 % 100),
                group: digits(flag / 10_000 % 10),
                zone: digits(flag / 1000 % 10),
                word: digits(n / 32),
                bit: digits(31u32.saturating_sub(n % 32)),
            })
        }
        Game::EldenRing => Some(Address::EldenRing {
            category: flag / 1000,
            byte: digits(n / 8),
            bit: digits(7u32.saturating_sub(n % 8)),
        }),
    }
}

/// Which map's flags `flag` belongs to in `game`. `None` for a number that is no flag of the game (Sekiro's above
/// 99,999,999).
pub fn scope(game: Game, flag: u32) -> Option<Scope> {
    match game {
        Game::Sekiro => {
            if flag > 99_999_999 {
                return None;
            }
            let (area, group) = (flag / 100_000 % 100, flag / 10_000 % 10);
            if area >= 90 || area.saturating_add(group) == 0 {
                return Some(Scope::Global);
            }
            Some(Scope::Map(MapId::from_bytes([
                digits(area),
                digits(group),
                MapId::WILDCARD,
                MapId::WILDCARD,
            ])))
        }
        Game::EldenRing => {
            let tile = |area: u8| {
                Scope::Map(MapId::from_bytes([
                    area,
                    digits(flag / 1_000_000 % 100),
                    digits(flag / 10_000 % 100),
                    MapId::WILDCARD,
                ]))
            };
            Some(match flag {
                1_000_000_000..=1_099_999_999 => tile(60),
                2_000_000_000..=2_099_999_999 => tile(61),
                10_000_000..=99_999_999 => Scope::Map(MapId::from_bytes([
                    digits(flag / 1_000_000),
                    digits(flag / 10_000 % 100),
                    MapId::WILDCARD,
                    MapId::WILDCARD,
                ])),
                _ => Scope::Global,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sekiro_flag_is_its_digits_and_a_bit_of_a_word() {
        // 11110100: region 1, area 11, group 1, zone 0, number 100 = word 3, bit 31 - 4
        assert_eq!(
            address(Game::Sekiro, 11_110_100),
            Some(Address::Sekiro {
                region: 1,
                area: 11,
                group: 1,
                zone: 0,
                word: 3,
                bit: 27,
            })
        );
        // the number inside a zone: 0 is the top bit of word 0, 999 the bottom bit of word 31
        let at = |flag| match address(Game::Sekiro, flag) {
            Some(Address::Sekiro { word, bit, .. }) => (word, bit),
            other => panic!("{other:?}"),
        };
        assert_eq!(at(11_110_000), (0, 31));
        assert_eq!(at(11_110_031), (0, 0));
        assert_eq!(at(11_110_032), (1, 31));
        assert_eq!(at(11_110_999), (31, 24));
        assert_eq!(address(Game::Sekiro, 100_000_000), None);
    }

    #[test]
    fn a_sekiro_flag_is_global_without_an_area_or_past_the_areas() {
        // the part-state flags of the stock scripts
        for flag in [8301, 8302, 8303, 8304, 8306, 8310] {
            assert_eq!(scope(Game::Sekiro, flag), Some(Scope::Global), "{flag}");
        }
        // an area of 90 or more
        assert_eq!(scope(Game::Sekiro, 19_000_001), Some(Scope::Global));
        assert_eq!(
            scope(Game::Sekiro, 11_110_100),
            Some(Scope::Map(MapId::from_bytes([11, 1, 0xff, 0xff])))
        );
        assert_eq!(scope(Game::Sekiro, 100_000_000), None);
    }

    #[test]
    fn an_elden_ring_flag_is_a_bit_of_a_block_of_1000() {
        assert_eq!(
            address(Game::EldenRing, 1_042_360_800),
            Some(Address::EldenRing {
                category: 1_042_360,
                byte: 100,
                bit: 7,
            })
        );
        assert_eq!(
            address(Game::EldenRing, 12_010_805),
            Some(Address::EldenRing {
                category: 12_010,
                byte: 100,
                bit: 2,
            })
        );
        assert_eq!(
            address(Game::EldenRing, 999),
            Some(Address::EldenRing {
                category: 0,
                byte: 124,
                bit: 0,
            })
        );
    }

    #[test]
    fn an_elden_ring_flag_names_a_block_or_a_tile_by_its_digits() {
        let m = |a, b, c, d| Some(Scope::Map(MapId::from_bytes([a, b, c, d])));
        assert_eq!(scope(Game::EldenRing, 12_010_800), m(12, 1, 0xff, 0xff));
        assert_eq!(scope(Game::EldenRing, 1_042_360_800), m(60, 42, 36, 0xff));
        assert_eq!(scope(Game::EldenRing, 2_045_410_000), m(61, 45, 41, 0xff));
        assert_eq!(scope(Game::EldenRing, 9_000), Some(Scope::Global));
        assert_eq!(scope(Game::EldenRing, 131_328), Some(Scope::Global));
        assert_eq!(scope(Game::EldenRing, 9_999_999), Some(Scope::Global));
        // ten digits with another prefix name no tile
        assert_eq!(scope(Game::EldenRing, 3_000_000_000), Some(Scope::Global));
        // and a tile's flag covers the tile's variants, a block's the block's
        let Some(Scope::Map(tile)) = scope(Game::EldenRing, 1_042_360_800) else {
            panic!("a tile");
        };
        assert!(tile.covers(MapId::from_name("m60_42_36_00").unwrap()));
        assert!(tile.covers(MapId::from_name("m60_42_36_10").unwrap()));
        assert!(!tile.covers(MapId::from_name("m60_42_37_00").unwrap()));
    }
}
