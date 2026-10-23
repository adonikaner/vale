//! **Making a character, and unmaking one** — the character screen's two
//! writes, each one packet out and one byte back.
//!
//! ```text
//! CMSG_CHAR_CREATE   cstring name, u8 race, class, gender,
//!                    skin, face, hairStyle, hairColour, facialHair, outfitId
//! SMSG_CHAR_CREATE   u8 code — a ResponseCodes value; 46 is success
//!
//! CMSG_CHAR_DELETE   u64 guid — plain, not packed
//! SMSG_CHAR_DELETE   u8 code — the same array eleven entries on; 57 is success
//! ```
//!
//! The two live in one file because they are one shape: the same socket, the
//! same 84-entry key array, and the same reason for not being a handler (below).
//! Where they differ is what silence means, and
//! [`crate::socket::world::delete_character`] is the one that has to have an
//! opinion about it. The character list itself, `SMSG_CHAR_ENUM`, is parsed
//! here too, by [`parse_char_enum`] into [`CharListEntry`].
//!
//! Ten bytes and a name, and **every one of them is a decision the client made
//! with no help from the server**: which races there are, which classes each may
//! be, how many faces a dwarf has. That half is
//! [`vale_assets::tables::charcreate`](../../vale_assets/charcreate/index.html);
//! this is only the sentence that carries it.
//!
//! ## The exchange is not a handler
//!
//! The bodies and the reply readers are here; the exchange itself,
//! [`crate::socket::world::create_character`] and
//! [`crate::socket::world::delete_character`], is beside the session it writes
//! to. Every other packet in this crate goes socket → frame → parse → apply,
//! through the one dispatch in [`crate::socket::handler`]. This one does not, for the same
//! reason [`crate::socket::world::WorldSession::char_enum`] does not: it happens
//! **before there is a world**, on the authenticated socket the character screen
//! is holding, and the client has nothing to do between the send and the reply.
//! A request and its answer, in one call, with the intervening packets recorded
//! rather than dropped silently.
//!
//! ## `outfitId` is 0 and that is the server's own reading
//!
//! vmangos' `HandleCharCreateOpcode` reads the byte and never uses it; the
//! starting gear comes from `playercreateinfo_item` keyed by race and class. The
//! reference sends 0 for it, which is what this sends.
//!
//! ## What the reply can say
//!
//! Two windows of `ResponseCodes` and not one — `CHAR_CREATE_*` at 45..=55 and
//! `CHAR_NAME_*` at 69..=82, because a name the server will not take is refused
//! by the name checker rather than by the creator. Both are keys in
//! `Interface\GlueXML\GlueStrings.lua` and both are looked up the same way; see
//! [`crate::codes::response_key`], which is the client's own flat array.

use crate::bytes::{Reader, Writer};
use crate::codes::{response_key, CHAR_CREATE_SUCCESS, CHAR_DELETE_SUCCESS};

/// `CMSG_CHAR_CREATE`'s body.
///
/// **The appearance travels as one `[u8; 5]` rather than as five arguments**,
/// in `PLAYER_BYTES` order — skin, face, hair style, hair colour, facial hair —
/// which is the same order [`CharListEntry::appearance`] comes
/// back in. Five small integers in a row is the shape that transposes silently,
/// and a transposed skin and face is a character who looks like somebody else
/// rather than a packet the server refuses; keeping the array whole means the
/// two ends cannot disagree about the order without the *type* disagreeing.
///
/// The name travels as the player typed it: **1.12 capitalises server-side**
/// (`ObjectMgr::NormalizePlayerName`), so a client that title-cased it here
/// would be doing the same work twice and would differ from the reference the
/// first time a locale disagreed about what an initial is.
pub fn char_create_body(
    name: &str,
    race: u8,
    class: u8,
    gender: u8,
    appearance: [u8; 5],
) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.u8(race).u8(class).u8(gender);
    for byte in appearance {
        w.u8(byte);
    }
    // `outfitId`, read and ignored by the server — see the module comment.
    w.u8(0);
    w.buf
}

/// Why the server would not make the character — or delete it, since both
/// refusals are a byte out of the same array and neither carries anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateRefusal {
    /// The wire byte, kept whether or not it names anything — unknown values are
    /// data, not errors.
    pub code: u8,
    /// Its `GlueStrings.lua` key, or `None` for a byte past the client's own
    /// array. A `None` here shows **no dialog at all**, which is the client's
    /// behaviour for a key the shipped file does not carry.
    pub key: Option<&'static str>,
}

/// The reply body, which is one byte.
///
/// **An empty body is a refusal rather than a success.** Nothing in vmangos
/// sends `SMSG_CHAR_CREATE` without a code, but reading "no bytes" as success
/// would put a character on the list that does not exist, and the failure would
/// then be the character screen's rather than this function's.
pub fn read_create_result(body: &[u8]) -> Result<(), CreateRefusal> {
    let Some(&code) = body.first() else {
        return Err(CreateRefusal {
            code: 0,
            key: Some("CHAR_CREATE_ERROR"),
        });
    };
    if code == CHAR_CREATE_SUCCESS {
        return Ok(());
    }
    Err(CreateRefusal {
        code,
        key: response_key(code),
    })
}

/// `CMSG_CHAR_DELETE`'s body: the guid, and nothing else.
///
/// **Plain, not packed.** `HandleCharDeleteOpcode` is `recv_data >> guid` into
/// an `ObjectGuid`, whose stream operator reads eight bytes outright — this is
/// the character screen rather than the world, and the packed form only ever
/// appears in world traffic. Sending a packed guid here is a short body the
/// server reads garbage out of.
pub fn char_delete_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// What a delete the server never answered reports — see
/// [`crate::socket::world::delete_character`].
///
/// `code: 0` is not a `ResponseCodes` value and is not pretending to be one:
/// nothing came back, so there is no byte. The key is the one the *answered*
/// failure uses, because from the character screen the two are the same event.
pub const SILENTLY_REFUSED: CreateRefusal = CreateRefusal {
    code: 0,
    key: Some("CHAR_DELETE_FAILED"),
};

/// The reply body, which is one byte.
///
/// **An empty body is a refusal**, on the same argument
/// [`read_create_result`] makes: reading "no bytes" as success would take a
/// character off the list that is still on the server, and the next
/// `CMSG_CHAR_ENUM` would put it back — a list that flickers rather than a
/// failure that says so.
pub fn read_delete_result(body: &[u8]) -> Result<(), CreateRefusal> {
    let Some(&code) = body.first() else {
        return Err(SILENTLY_REFUSED);
    };
    if code == CHAR_DELETE_SUCCESS {
        return Ok(());
    }
    Err(CreateRefusal {
        code,
        key: response_key(code),
    })
}

#[derive(Debug, Clone)]
pub struct CharListEntry {
    pub guid: u64,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u8,
    /// `skin, face, hairStyle, hairColour, facialHair`: the five bytes that
    /// `PLAYER_BYTES` and `PLAYER_BYTES_2` carry once the character is in the
    /// world.
    ///
    /// Read rather than skipped, because character select draws the character.
    /// The model comes from the race and the clothing from `equipment`, but the
    /// face and hairstyle come only from these bytes; without them every
    /// character on the list is drawn with skin 0, face 0 and no hair. Kept as
    /// the five raw bytes rather than as
    /// `vale_assets::look::character::Appearance` because this crate does not
    /// depend on `vale_assets`; the client builds the `Appearance` in one line.
    pub appearance: [u8; 5],
    pub zone: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `characters.character_flags`, unchanged. See [`CharListEntry::GHOST`].
    pub flags: u32,
    /// The character's equipment, as `(ItemDisplayInfo id, InventoryType)` per
    /// slot, with zeroes where the slot is empty.
    ///
    /// This is the only place in the protocol where equipment arrives with its
    /// display ids already resolved: vmangos `Player::BuildEnumData` writes
    /// `proto->DisplayInfoID` and `proto->InventoryType` directly, while
    /// `PLAYER_VISIBLE_ITEM_n_0` in the world carries an item entry that needs
    /// a `CMSG_ITEM_QUERY_SINGLE` round trip. Character select can therefore
    /// dress the character on the frame the list arrives.
    ///
    /// It does not carry the item's class, subclass or sheath type, the three
    /// fields that decide where a put-away weapon hangs. Those are in the item
    /// template, `Item.dbc` is not in the 1.12 archives, and no packet on this
    /// screen can fetch them.
    ///
    /// Character select does not need them, and their absence is not why a
    /// character-select model would have empty hands. The 1.12.1 client draws
    /// no weapon on character select as put away: both weapons are attached at
    /// hand points, and the only item property it uses is
    /// `inventoryType == 14` (shield), which this list carries. It skips the
    /// ranged slot (index 17). See
    /// `vale_assets::tables::item::Weapon::from_char_enum`.
    ///
    /// The index is the equipment slot, and the code depends on it: 15 is the
    /// main hand and 16 the off hand, and no inventory type distinguishes them
    /// (a one-hand sword is `INVTYPE_WEAPON` in either). So this stays a
    /// positional list including its empty slots rather than being compacted.
    pub equipment: Vec<(u32, u8)>,
    /// The id of the character's guild, 0 for none.
    pub guild: u32,
    /// That guild's emblem, which a guild tabard on the character-select
    /// model is painted with. The list carries the id only;
    /// [`crate::socket::world::WorldSession::char_enum`] asks for the emblem with
    /// `CMSG_GUILD_QUERY`, which the server answers before a character is in
    /// the world. `None` for a character in no guild and for a guild the
    /// server did not answer for.
    pub emblem: Option<crate::play::guild::Emblem>,
}

impl CharListEntry {
    /// `CHARACTER_FLAG_GHOST`: the character is dead, and the row reads
    /// `<name> (Ghost)`. `GetCharacterInfo`'s eighth return value.
    pub const GHOST: u32 = 0x0000_2000;
    /// `CHARACTER_FLAG_HIDE_HELM` / `_HIDE_CLOAK`: the interface options that
    /// hide the helm and the cloak. The server stores them, and they decide
    /// what the character-select model wears.
    pub const HIDE_HELM: u32 = 0x0000_0400;
    pub const HIDE_CLOAK: u32 = 0x0000_0800;

    pub fn is_ghost(&self) -> bool {
        self.flags & Self::GHOST != 0
    }
    pub fn hides_helm(&self) -> bool {
        self.flags & Self::HIDE_HELM != 0
    }
    pub fn hides_cloak(&self) -> bool {
        self.flags & Self::HIDE_CLOAK != 0
    }
}

/// Number of equipment slots serialised per character in `SMSG_CHAR_ENUM`.
///
/// vmangos `Player::BuildEnumData` loops `slot < INVENTORY_SLOT_BAG_START + 1`,
/// i.e. 19 equipment slots plus the first bag.
///
/// Each slot is five bytes, `u32 displayId` then `u8 inventoryType`. 1.12 has
/// no per-slot enchantment field; 2.4 adds one. Reading one overruns the packet
/// by 80 bytes per character and misaligns every entry after the first, which
/// `a_second_character_parses_after_the_first` tests.
const CHAR_ENUM_EQUIPMENT_SLOTS: usize = 20;

/// Parse `SMSG_CHAR_ENUM`. Only the pet block is skipped, and it must still be
/// consumed exactly or the next character is misread.
///
/// The appearance and the equipment are read, not skipped: the five appearance
/// bytes and the twenty `(display id, inventory type)` pairs are the only data
/// about how a character on this screen looks. An earlier version skipped them,
/// and character select showed an empty model. Reading them costs nothing
/// extra, because the parser has to step over them anyway.
pub(crate) fn parse_char_enum(body: &[u8]) -> Vec<CharListEntry> {
    let mut r = Reader::new(body);
    let count = r.u8();
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let guid = {
            let lo = r.u32() as u64;
            let hi = r.u32() as u64;
            (hi << 32) | lo
        };
        let name = r.cstring();
        let race = r.u8();
        let class = r.u8();
        let gender = r.u8();
        // Skin, face, hair style, hair colour, facial hair: the order in which
        // vmangos `Player::BuildEnumData` writes them.
        let appearance = [r.u8(), r.u8(), r.u8(), r.u8(), r.u8()];
        let level = r.u8();
        let zone = r.u32();
        let map = r.u32();
        let x = r.f32();
        let y = r.f32();
        let z = r.f32();
        let guild = r.u32();
        let flags = r.u32();
        let _first_login = r.u8();
        let _pet_display_id = r.u32();
        let _pet_level = r.u32();
        let _pet_family = r.u32();
        let equipment = (0..CHAR_ENUM_EQUIPMENT_SLOTS)
            .map(|_| (r.u32(), r.u8()))
            .collect();

        out.push(CharListEntry {
            guid,
            name,
            race,
            class,
            gender,
            appearance,
            level,
            zone,
            map,
            x,
            y,
            z,
            flags,
            equipment,
            guild,
            emblem: None,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One character, byte for byte as vmangos `Player::BuildEnumData` writes
    /// one.
    fn character_bytes(w: &mut Writer, guid: u32, name: &str, flags: u32) {
        w.u32(guid).u32(0); // guid
        w.buf.extend_from_slice(name.as_bytes());
        w.u8(0);
        w.u8(1).u8(8).u8(1); // human, warlock, female
        w.u8(3).u8(5).u8(9).u8(2).u8(4); // skin, face, hair, colour, facial hair
        w.u8(60);
        w.u32(12).u32(0); // zone, map
        w.f32(-8900.0).f32(-130.0).f32(80.0);
        w.u32(0); // guild
        w.u32(flags);
        w.u8(0); // first login
        w.u32(0).u32(0).u32(0); // pet
        for slot in 0..CHAR_ENUM_EQUIPMENT_SLOTS {
            // Slot 0 is the head, slot 4 the chest; the rest are empty. Mostly
            // empty slots are the common case, and the parser must handle them.
            match slot {
                0 => w.u32(21_549).u8(1),
                4 => w.u32(31_051).u8(5),
                _ => w.u32(0).u8(0),
            };
        }
    }

    /// The three blocks this parser once skipped: the five appearance bytes,
    /// the flag word and the twenty equipment slots.
    ///
    /// Character select draws all three, so a misread shows as the wrong face,
    /// no equipment, or a missing `(Ghost)`. The body is built in the test
    /// rather than captured, because a capture would fix one account's
    /// characters, and the test is about the layout.
    #[test]
    fn a_character_row_carries_its_appearance_its_flags_and_its_wardrobe() {
        let mut w = Writer::new();
        w.u8(1); // one character
        character_bytes(
            &mut w,
            7,
            "Alden",
            CharListEntry::GHOST | CharListEntry::HIDE_HELM,
        );

        let rows = parse_char_enum(&w.buf);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "Alden");
        assert_eq!(row.appearance, [3, 5, 9, 2, 4], "skin, face, hair, colour, beard");
        assert!(row.is_ghost() && row.hides_helm() && !row.hides_cloak());
        assert_eq!(row.equipment.len(), CHAR_ENUM_EQUIPMENT_SLOTS);
        assert_eq!(row.equipment[0], (21_549, 1));
        assert_eq!(row.equipment[4], (31_051, 5));
        assert_eq!(row.equipment[19], (0, 0), "the last slot is the first bag");
    }

    /// A block-size error shows in the second character of the list.
    ///
    /// A per-character body read one byte long or short misaligns every entry
    /// after the first. The usual case is the 2.4 enchantment field, 80 bytes
    /// per character, which makes the rest of the list unreadable. An account
    /// with one character does not show the error.
    #[test]
    fn a_second_character_parses_after_the_first() {
        let mut w = Writer::new();
        w.u8(2);
        character_bytes(&mut w, 7, "Alden", CharListEntry::GHOST);
        character_bytes(&mut w, 9, "Bram", 0);

        let rows = parse_char_enum(&w.buf);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, "Bram");
        assert_eq!(rows[1].guid, 9);
        assert!(!rows[1].is_ghost(), "the first row's flags did not bleed");
        assert_eq!(rows[1].appearance, [3, 5, 9, 2, 4]);
        assert_eq!(rows[1].equipment[4], (31_051, 5));
    }

    /// **The body is a name and ten bytes, in vmangos' own read order** —
    /// `name >> race >> class >> gender >> skin >> face >> hairStyle >>
    /// hairColor >> facialHair >> outfitId`. A transposition here compiles and
    /// creates a character who looks like somebody else, so the order is what
    /// the test is about.
    #[test]
    fn the_body_is_the_name_then_the_nine_choices_and_a_zero() {
        let body = char_create_body("Hollowell", 4, 8, 1, [3, 5, 9, 2, 4]);
        assert_eq!(&body[..9], b"Hollowell");
        assert_eq!(body[9], 0, "the name is a cstring");
        assert_eq!(&body[10..], &[4, 8, 1, 3, 5, 9, 2, 4, 0]);
        assert_eq!(body.len(), 19);
    }

    /// **46 is the whole of success**, and everything else names a key —
    /// including the two windows that are not adjacent.
    #[test]
    fn the_reply_is_one_byte_and_the_refusals_name_their_own_key() {
        assert_eq!(read_create_result(&[CHAR_CREATE_SUCCESS]), Ok(()));
        assert_eq!(
            read_create_result(&[49]),
            Err(CreateRefusal {
                code: 49,
                key: Some("CHAR_CREATE_NAME_IN_USE")
            })
        );
        // …and a name refused by the *name* checker, thirteen codes further on.
        assert_eq!(
            read_create_result(&[70]),
            Err(CreateRefusal {
                code: 70,
                key: Some("CHAR_NAME_TOO_SHORT")
            })
        );
        // A byte past the client's own array is data: kept, and shown as
        // nothing.
        assert_eq!(
            read_create_result(&[200]),
            Err(CreateRefusal {
                code: 200,
                key: None
            })
        );
        // …and no body at all is not a success.
        assert!(read_create_result(&[]).is_err());
    }

    /// **Eight bytes, little-endian, and not packed** — see
    /// [`char_delete_body`], where the reason is that `ObjectGuid`'s stream
    /// operator reads a whole `uint64` and the packed form is world traffic.
    #[test]
    fn the_delete_body_is_the_guid_and_nothing_else() {
        assert_eq!(
            char_delete_body(0x0000_0000_0000_1234),
            vec![0x34, 0x12, 0, 0, 0, 0, 0, 0]
        );
    }

    /// **57 is the whole of success**, and the one refusal vmangos actually
    /// sends names its own key. The three it does *not* send are the next test.
    #[test]
    fn the_delete_reply_is_one_byte_out_of_the_same_array() {
        assert_eq!(read_delete_result(&[CHAR_DELETE_SUCCESS]), Ok(()));
        // The guild-leader refusal, which is the only code that ever arrives.
        assert_eq!(
            read_delete_result(&[58]),
            Err(CreateRefusal {
                code: 58,
                key: Some("CHAR_DELETE_FAILED")
            })
        );
        // …and the success byte is not the *create* success byte, which is the
        // transposition this pair of constants exists to make impossible.
        assert_ne!(CHAR_DELETE_SUCCESS, CHAR_CREATE_SUCCESS);
        assert!(read_delete_result(&[CHAR_CREATE_SUCCESS]).is_err());
    }

    /// **A delete the server never answers is a refusal, not a dead socket.**
    /// Three of `HandleCharDeleteOpcode`'s five paths `return` without sending
    /// anything; the character screen has to survive all three.
    #[test]
    fn a_delete_that_is_never_answered_reports_the_answered_failures_key() {
        assert_eq!(read_delete_result(&[]), Err(SILENTLY_REFUSED));
        assert_eq!(SILENTLY_REFUSED.key, Some("CHAR_DELETE_FAILED"));
        // Not a `ResponseCodes` value, and deliberately: nothing came back.
        assert_eq!(SILENTLY_REFUSED.code, 0);
        assert_ne!(response_key(0), SILENTLY_REFUSED.key);
    }
}
