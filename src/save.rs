//! Elden Ring's PC character saves (`ER0000.sl2`): which slots hold a character and, for each, what the character
//! holds: its arm style, the active slot of each hand and quiver, and the item in every equipment slot.
//!
//! The file is a BND4 binder of twelve members. `USER_DATA000` to `USER_DATA009` are the ten character slots and
//! `USER_DATA010` is the profile. Each member begins with the MD5 of the rest of its bytes. The profile marks which slots
//! hold a character: one byte per slot, after its fixed head (an `i32`, the account's 64-bit id, 0x140 bytes) and a
//! length-prefixed menu block. A slot member is 0x10 bytes of MD5, then 0x280000 bytes.
//!
//! A slot is laid out in this order, which is the community save editor's (ER-Save-Editor 014107f's `SaveSlot::read`):
//!
//! 1. a version word, the map id (4 bytes), 0x18 bytes;
//! 2. 0x1400 item records (`GaItem`), each a handle and an item id. A weapon's record (item id type bits `0x0`) has 13
//!    bytes more and an armour piece's (`0x1`) 8; the empty record (item id 0) has none;
//! 3. the character's game data (0x1B0 bytes) and 0xD0 bytes;
//! 4. an equipment block of 22 words;
//! 5. `ChrAsm`, the seven words this reader names [`Hold`] (the arm style, then the active slot of the left hand, the
//!    right hand, the left and right arrows and the left and right bolts), and 22 words that are the item id in each
//!    equipment slot;
//! 6. 22 words that are the item record handle of each equipment slot.
//!
//! The 22 slot words, in both of the last two blocks, are the hands' three slots interleaved (left 1, right 1, left 2,
//! right 2, left 3, right 3), then arrows 1, bolts 1, arrows 2, bolts 2, two words this reader does not name, head,
//! chest, arms, legs, one word it does not name, the four talismans and one more word it does not name. An empty slot
//! holds the item id `0xFFFFFFFF` and the handle 0.
//!
//! The reader checks the layout against the file. Every slot member's MD5 must match its bytes. Every equipped handle
//! must be an item record of the slot, and the record must hold the item id the slot word holds; an armour piece's
//! record carries its type bits (`0x1`) and its slot word does not, so the comparison leaves out the top four bits.
//! A wrong record size or block offset would fail this check. The game reads the same seven `ChrAsm` words from its
//! player data: the arm style is word 0, and the active slot of slot kind `k` is word `1 + k`. The right hand's weapon
//! is the item in slot `2 x active + 1` of the 22, and the left hand's is in slot `2 x active` (the equipment readers
//! of the game's 2.7.2.0 executable).
use crate::bytes::View;
use crate::{Binder, Result, invalid};
use md5::{Digest, Md5};

const FORMAT: &str = "Elden Ring save";

/// The bytes of a slot member after its MD5.
pub const SLOT_SIZE: usize = 0x28_0000;
/// The item records of a slot.
pub const ITEM_RECORDS: usize = 0x1400;
/// The words of an equipment block (the item ids, or the handles, of every equipment slot).
pub const EQUIPMENT_WORDS: usize = 22;

/// How a character holds its weapons: the save's arm style word. The names are the game's own constants for the four
/// values, from the player's behaviour script (`HAND_LEFT` 0, `HAND_RIGHT` 1, `HAND_LEFT_BOTH` 2, `HAND_RIGHT_BOTH` 3).
/// The script picks a two-handed attack when the style is 2 or more, and the game's two-handed animation selector takes
/// the left hand's weapon for 2 and the right hand's otherwise (the player's script and the game's 2.7.2.0 executable).
/// Any other value is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
pub enum ArmStyle {
    /// 0 (`HAND_LEFT`).
    Left,
    /// 1 (`HAND_RIGHT`).
    Right,
    /// 2 (`HAND_LEFT_BOTH`): the left hand's weapon held in both hands.
    LeftBoth,
    /// 3 (`HAND_RIGHT_BOTH`): the right hand's weapon held in both hands.
    RightBoth,
}

impl ArmStyle {
    /// The style a stored word names, if any.
    pub fn from_word(word: u32) -> Option<Self> {
        match word {
            0 => Some(Self::Left),
            1 => Some(Self::Right),
            2 => Some(Self::LeftBoth),
            3 => Some(Self::RightBoth),
            _ => None,
        }
    }

    /// The word the save stores for the style.
    pub fn word(self) -> u32 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::LeftBoth => 2,
            Self::RightBoth => 3,
        }
    }
}

/// A slot's `ChrAsm` head: the arm style and the active slot of each hand and quiver, as stored. A hand's active slot is
/// 0, 1 or 2 (a larger one is refused). An arrow or bolt slot is 0 or 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Hold {
    /// Word 0.
    pub arm_style: ArmStyle,
    /// Word 1.
    pub left_hand: u32,
    /// Word 2.
    pub right_hand: u32,
    /// Word 3.
    pub left_arrow: u32,
    /// Word 4.
    pub right_arrow: u32,
    /// Word 5.
    pub left_bolt: u32,
    /// Word 6.
    pub right_bolt: u32,
}

/// The item id in each equipment slot, as the save stores it, with `None` for an empty slot (`0xFFFFFFFF`). A weapon's id
/// is its `EquipParamWeapon` row plus its upgrade level (the Uchigatana `9000000`; an unarmed hand holds `110000`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Equipped {
    /// The left hand's three slots.
    pub left_hand: [Option<u32>; 3],
    /// The right hand's three slots.
    pub right_hand: [Option<u32>; 3],
    /// The two arrow slots.
    pub arrows: [Option<u32>; 2],
    /// The two bolt slots.
    pub bolts: [Option<u32>; 2],
    /// Head armour.
    pub head: Option<u32>,
    /// Chest armour.
    pub chest: Option<u32>,
    /// Arm armour.
    pub arms: Option<u32>,
    /// Leg armour.
    pub legs: Option<u32>,
    /// The four talismans.
    pub talismans: [Option<u32>; 4],
}

/// One character slot.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Slot {
    /// The slot's number, 0 to 9 (`USER_DATA000` to `USER_DATA009`).
    pub index: usize,
    /// The slot's version word.
    pub version: u32,
    /// How the character holds its weapons.
    pub hold: Hold,
    /// What it has equipped.
    pub equipped: Equipped,
}

impl Slot {
    /// The item id of the weapon in the active slot of the right hand.
    pub fn right_weapon(&self) -> Option<u32> {
        active(&self.equipped.right_hand, self.hold.right_hand)
    }

    /// The item id of the weapon in the active slot of the left hand.
    pub fn left_weapon(&self) -> Option<u32> {
        active(&self.equipped.left_hand, self.hold.left_hand)
    }
}

fn active(slots: &[Option<u32>; 3], index: u32) -> Option<u32> {
    usize::try_from(index)
        .ok()
        .and_then(|i| slots.get(i).copied().flatten())
}

/// A save file: its occupied character slots, in slot order.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Save {
    /// The slots the profile marks as holding a character.
    pub slots: Vec<Slot>,
}

impl Save {
    /// Read a save file as stored on disk. The profile and every occupied slot are read and checked. A slot the profile
    /// does not mark is not read.
    pub fn read(data: &[u8]) -> Result<Self> {
        let binder = Binder::read_stored(data)?;
        let member = |name: &str| -> Result<&[u8]> {
            binder
                .files
                .iter()
                .find(|f| f.name.as_deref() == Some(name))
                .map(|f| f.bytes.as_slice())
                .ok_or_else(|| invalid(FORMAT, 0, format!("no member {name}")))
        };
        let profile = checked(member("USER_DATA010")?, "USER_DATA010")?;
        let occupied = occupied(&profile)?;
        let mut slots = Vec::new();
        for (index, used) in occupied.iter().enumerate() {
            if *used {
                let name = format!("USER_DATA{index:03}");
                let body = checked(member(&name)?, &name)?;
                slots.push(read_slot(index, &body)?);
            }
        }
        Ok(Self { slots })
    }

    /// The slot numbered `index`, if it holds a character.
    pub fn slot(&self, index: usize) -> Option<&Slot> {
        self.slots.iter().find(|s| s.index == index)
    }
}

/// A member's bytes after its MD5, refused when the MD5 is not that of those bytes.
fn checked<'a>(member: &'a [u8], name: &str) -> Result<View<'a>> {
    let whole = View {
        data: member,
        format: FORMAT,
        be: false,
        base: 0,
    };
    let stored = whole.slice(0, 16)?;
    let body = whole.sub(16, member.len().saturating_sub(16), FORMAT)?;
    if Md5::digest(body.data).as_slice() != stored {
        return Err(invalid(
            FORMAT,
            0,
            format!("{name}: its MD5 is not that of its bytes"),
        ));
    }
    Ok(body)
}

/// The profile's ten slot bytes: after an `i32`, the account id (8 bytes), 0x140 bytes and the menu block (a word, its
/// length, its bytes). Each is 0 or 1.
fn occupied(profile: &View<'_>) -> Result<[bool; 10]> {
    let menu = 4 + 8 + 0x140;
    let length = usize::try_from(profile.u32_at(menu, 4)?).map_err(|_| profile.err(menu, "size overflow"))?;
    let flags = profile.at(profile.at(menu, 8)?, length)?;
    let mut out = [false; 10];
    for (i, used) in out.iter_mut().enumerate() {
        *used = profile.boolean(profile.at(flags, i)?)?;
    }
    Ok(out)
}

/// The item id a slot word stores, `None` for an empty slot.
fn item(word: u32) -> Option<u32> {
    (word != u32::MAX).then_some(word)
}

fn read_slot(index: usize, body: &View<'_>) -> Result<Slot> {
    if body.data.len() != SLOT_SIZE {
        return Err(body.err(
            0,
            format!(
                "USER_DATA{index:03} is {:#x} bytes after its MD5, not {SLOT_SIZE:#x}",
                body.data.len()
            ),
        ));
    }
    let version = body.u32(0)?;
    // the item records: handle, item id, and 13 more bytes for a weapon, 8 for an armour piece
    let mut records: Vec<(u32, u32)> = Vec::new();
    let mut o = 0x20usize;
    for _ in 0..ITEM_RECORDS {
        let (handle, id) = (body.u32(o)?, body.u32_at(o, 4)?);
        o = body.at(o, 8)?;
        if id != 0 {
            match id & 0xF000_0000 {
                0 => o = body.at(o, 13)?,
                0x1000_0000 => o = body.at(o, 8)?,
                _ => {}
            }
            records.push((handle, id));
        }
    }
    let equip_index = body.at(o, 0x1B0 + 0xD0)?;
    let hold_at = body.at(equip_index, EQUIPMENT_WORDS * 4)?;
    let words = |at: usize, n: usize| -> Result<Vec<u32>> {
        (0..n).map(|i| body.u32(body.element(at, i, 4)?)).collect()
    };
    let head = words(hold_at, 7)?;
    let items_at = body.at(hold_at, 7 * 4)?;
    let items = words(items_at, EQUIPMENT_WORDS)?;
    let handles = words(body.at(items_at, EQUIPMENT_WORDS * 4)?, EQUIPMENT_WORDS)?;
    let word = |v: &[u32], i: usize| {
        v.get(i)
            .copied()
            .ok_or_else(|| body.err(hold_at, "short equipment block"))
    };
    for (i, (id, handle)) in items.iter().zip(&handles).enumerate() {
        let at = body.element(items_at, i, 4)?;
        let held = records.iter().find(|(h, _)| h == handle).map(|(_, id)| *id);
        let agrees = match (item(*id), *handle) {
            (None, 0) => true,
            (Some(id), handle) if handle != 0 => held.is_some_and(|r| r & 0x0FFF_FFFF == id & 0x0FFF_FFFF),
            _ => false,
        };
        if !agrees {
            return Err(body.err(
                at,
                format!(
                    "USER_DATA{index:03} equipment slot {i}: item id {id:#010x} with handle {handle:#010x}, which the slot's \
                     item records hold as {}",
                    held.map_or("nothing".to_owned(), |r| format!("{r:#010x}"))
                ),
            ));
        }
    }
    let style = word(&head, 0)?;
    let arm_style = ArmStyle::from_word(style)
        .ok_or_else(|| body.err(hold_at, format!("arm style {style}, not one of 0 to 3")))?;
    let hold = Hold {
        arm_style,
        left_hand: word(&head, 1)?,
        right_hand: word(&head, 2)?,
        left_arrow: word(&head, 3)?,
        right_arrow: word(&head, 4)?,
        left_bolt: word(&head, 5)?,
        right_bolt: word(&head, 6)?,
    };
    let limits = [
        (hold.left_hand, 2),
        (hold.right_hand, 2),
        (hold.left_arrow, 1),
        (hold.right_arrow, 1),
        (hold.left_bolt, 1),
        (hold.right_bolt, 1),
    ];
    for (k, (slot, most)) in (1..).zip(limits) {
        if slot > most {
            return Err(body.err(
                body.element(hold_at, k, 4)?,
                format!("active slot {slot}, above {most}"),
            ));
        }
    }
    let at = |i: usize| -> Result<Option<u32>> { Ok(item(word(&items, i)?)) };
    let equipped = Equipped {
        left_hand: [at(0)?, at(2)?, at(4)?],
        right_hand: [at(1)?, at(3)?, at(5)?],
        arrows: [at(6)?, at(8)?],
        bolts: [at(7)?, at(9)?],
        head: at(12)?,
        chest: at(13)?,
        arms: at(14)?,
        legs: at(15)?,
        talismans: [at(17)?, at(18)?, at(19)?, at(20)?],
    };
    Ok(Slot {
        index,
        version,
        hold,
        equipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binder::{BinderFile, Layout};

    const NONE: u32 = u32::MAX;
    const RIGHT_WEAPON: u32 = 0x8080_0001;
    const HEAD: u32 = 0x9080_0002;

    fn put(out: &mut Vec<u8>, words: &[u32]) {
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }

    /// A slot: one weapon record and one armour record among the 0x1400, the arm style `style`, the active right slot
    /// `right`, an Uchigatana in the right hand's first slot and head armour; the head's handle `head_handle`.
    fn slot(style: u32, right: u32, head_handle: u32) -> Vec<u8> {
        let mut s = Vec::new();
        put(&mut s, &[262]);
        s.extend_from_slice(&[0; 4 + 0x18]);
        put(&mut s, &[RIGHT_WEAPON, 9_000_000]);
        s.extend_from_slice(&[0; 13]);
        put(&mut s, &[HEAD, 0x100d_4670]);
        s.extend_from_slice(&[0; 8]);
        for _ in 2..ITEM_RECORDS {
            put(&mut s, &[0, 0]);
        }
        s.extend_from_slice(&[0; 0x1B0 + 0xD0]);
        s.extend_from_slice(&[0; EQUIPMENT_WORDS * 4]);
        put(&mut s, &[style, 0, right, 0, 0, 0, 0]);
        let mut items = [NONE; EQUIPMENT_WORDS];
        let mut handles = [0; EQUIPMENT_WORDS];
        (items[1], handles[1]) = (9_000_000, RIGHT_WEAPON);
        (items[12], handles[12]) = (0x000d_4670, head_handle);
        put(&mut s, &items);
        put(&mut s, &handles);
        s.resize(SLOT_SIZE, 0);
        s
    }

    fn member(index: usize, body: &[u8], md5_ok: bool) -> BinderFile {
        let mut bytes = Md5::digest(body).to_vec();
        if !md5_ok {
            bytes[0] ^= 1;
        }
        bytes.extend_from_slice(body);
        BinderFile {
            id: Some(0),
            name: Some(format!("USER_DATA{index:03}")),
            flags: 0b1010,
            bytes,
            wrap: None,
        }
    }

    /// A save whose slot 0 is `slot0` (its MD5 right when `md5_ok`), the other nine empty and unmarked.
    fn save(slot0: &[u8], md5_ok: bool) -> Vec<u8> {
        let mut files = vec![member(0, slot0, md5_ok)];
        for i in 1..10 {
            files.push(member(i, &vec![0; SLOT_SIZE], true));
        }
        let mut profile = vec![0; 4 + 8 + 0x140];
        put(&mut profile, &[7, 2]);
        profile.extend_from_slice(&[0xAA, 0xBB]);
        profile.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        profile.resize(0x6_0000, 0);
        files.push(member(10, &profile, true));
        Binder {
            version: *b"00000001",
            format: 4,
            files,
            layout: Layout {
                format_raw: 0x20,
                extended: 0,
                ..Layout::default()
            },
        }
        .write_stored()
        .unwrap()
    }

    #[test]
    fn a_slot_reads_its_hold_and_equipment() {
        let read = Save::read(&save(&slot(3, 0, HEAD), true)).unwrap();
        assert_eq!(read.slots.len(), 1);
        let s = read.slot(0).unwrap();
        assert_eq!(
            (s.version, s.hold.arm_style, s.hold.right_hand),
            (262, ArmStyle::RightBoth, 0)
        );
        assert_eq!((s.right_weapon(), s.left_weapon()), (Some(9_000_000), None));
        assert_eq!(s.equipped.head, Some(0x000d_4670));
        assert_eq!(s.equipped.right_hand, [Some(9_000_000), None, None]);
        assert!(read.slot(1).is_none());
    }

    #[test]
    fn the_active_slot_picks_the_weapon() {
        // the right hand's slot 1 is active and empty
        let read = Save::read(&save(&slot(1, 1, HEAD), true)).unwrap();
        let s = read.slot(0).unwrap();
        assert_eq!((s.hold.arm_style, s.right_weapon()), (ArmStyle::Right, None));
    }

    #[test]
    fn a_slot_whose_md5_is_wrong_is_refused_by_name() {
        let err = Save::read(&save(&slot(3, 0, HEAD), false))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("USER_DATA000: its MD5 is not that of its bytes"),
            "{err}"
        );
    }

    #[test]
    fn an_equipped_handle_the_records_do_not_hold_is_refused() {
        let err = Save::read(&save(&slot(3, 0, 0x9080_0009), true))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("equipment slot 12") && err.contains("hold as nothing"),
            "{err}"
        );
    }

    #[test]
    fn an_arm_style_or_active_slot_out_of_range_is_refused() {
        let err = Save::read(&save(&slot(4, 0, HEAD), true))
            .unwrap_err()
            .to_string();
        assert!(err.contains("arm style 4, not one of 0 to 3"), "{err}");
        let err = Save::read(&save(&slot(3, 3, HEAD), true))
            .unwrap_err()
            .to_string();
        assert!(err.contains("active slot 3, above 2"), "{err}");
    }
}
