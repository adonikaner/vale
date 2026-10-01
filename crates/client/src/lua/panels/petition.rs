//! The guild charter's C functions: the registrar window and the petition
//! window.
//!
//! ```text
//! GetGuildCharterCost()        copper, for the registrar's money frame
//! BuyGuildCharter(name)        buy a charter for a guild of that name
//! TurnInGuildCharter()         found the guild with the charter carried
//! GetTabardInfo()              ask the same NPC for the tabard designer
//! CloseGuildRegistrar()        the registrar window was hidden
//!
//! GetPetitionInfo()            type, title, body, maxSignatures,
//!                              originator, isOriginator
//! GetNumPetitionNames()        how many have signed
//! GetPetitionNameInfo(i)       one signer's name
//! CanSignPetition()            whether the Sign button is enabled
//! SignPetition()  OfferPetition()  RenamePetition(name)
//! ClosePetition()              the petition window was hidden
//! ```
//!
//! [`vale_protocol::play::petition`] has the wire formats and
//! [`crate::interface::petition`] is the systems that fill this board and
//! send what it queues.
//!
//! ## The state is held, and the requests are queued
//!
//! `PetitionFrame_Update` reads six functions in one handler, on an event
//! raised after the board was written. The board is one [`Charter`] behind an
//! `Rc<RefCell<_>>`, as the guild's is, and a press pushes a [`Press`] that
//! [`crate::interface::petition`] turns into a packet. A press carries no
//! guid: the registrar, the charter item and the selected player are the
//! session's to name.
//!
//! ## A charter is three answers
//!
//! `SMSG_PETITION_SHOW_SIGNATURES` names the charter, its owner and its
//! signers by guid. The guild name and the signature limit are the answer to
//! `CMSG_PETITION_QUERY`, kept by petition id in [`Charter::records`]. The
//! names are the answers to `CMSG_NAME_QUERY`. The 1.12.1 client raises
//! `PETITION_SHOW` when the petition's record is held and no name is still
//! being asked for; [`Charter::ready`] is that test.
//!
//! ## What the reads answer with nothing open
//!
//! `GetPetitionInfo` answers `nil, nil, nil, 0, nil, nil`. The fourth value
//! is a number because `PetitionFrame_Update` compares it with the signature
//! count.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use vale_protocol::play::petition::{CharterOffer, PetitionQuery, Signatures};

/// The functions this file registers, sorted. Checked against [`register`]
/// by `every_verb_is_registered`.
pub const VERBS: [&str; 13] = [
    "BuyGuildCharter",
    "CanSignPetition",
    "CloseGuildRegistrar",
    "ClosePetition",
    "GetGuildCharterCost",
    "GetNumPetitionNames",
    "GetPetitionInfo",
    "GetPetitionNameInfo",
    "GetTabardInfo",
    "OfferPetition",
    "RenamePetition",
    "SignPetition",
    "TurnInGuildCharter",
];

/// The most bytes a guild name may have. `GuildRegistrarFrameEditBox` and the
/// `RENAME_GUILD` popup both allow 24 letters.
pub const MAX_NAME_BYTES: usize = 24;

/// The `flags` bit of a petition record that marks a guild charter.
/// `GetPetitionInfo` answers `"charter"` for a record with it and
/// `"petition"` for one without.
pub const FLAG_CHARTER: u32 = 1;

/// A press the interface made, for [`crate::interface::petition`] to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// `BuyGuildCharter(name)`, with a name that passed [`name_fault`].
    Buy(String),
    /// `TurnInGuildCharter()`.
    TurnIn,
    /// `SignPetition(n)`. The byte is sent as given; the interface passes
    /// none and the client sends 1.
    Sign(u8),
    /// `OfferPetition()`: ask the selected player to sign.
    Offer,
    /// `RenamePetition(name)`, with a name that passed [`name_fault`].
    Rename(String),
    /// `ClosePetition()`: the petition window was hidden.
    ClosePetition,
    /// `CloseGuildRegistrar()`: the registrar window was hidden.
    CloseRegistrar,
    /// `GetTabardInfo()`: the registrar's third button.
    TabardInfo,
    /// A refusal the client states itself, as a `GlobalStrings.lua` key.
    Refuse(&'static str),
}

/// The charter the petition window shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Open {
    /// `SMSG_PETITION_SHOW_SIGNATURES`.
    pub shown: Signatures,
    /// Whether the character signed this charter while it was open. A window
    /// closed without signing declines the charter; one closed after signing
    /// does not.
    pub signed: bool,
}

/// The charter state the interface reads, shared between the interpreter and
/// the ECS.
#[derive(Default)]
pub struct Charter {
    /// The guild registrar the registrar window is open at.
    pub registrar: Option<u64>,
    /// What that registrar sells: the first entry of its
    /// `SMSG_PETITION_SHOWLIST`.
    pub offer: Option<CharterOffer>,
    /// The open charter.
    pub petition: Option<Open>,
    /// Every `SMSG_PETITION_QUERY_RESPONSE` received, by petition id.
    pub records: HashMap<u32, PetitionQuery>,
    /// The character's own guid and whether it is in a guild, which decide
    /// `CanSignPetition` and `isOriginator`.
    pub own_guid: u64,
    pub in_guild: bool,
    /// Player names by guid, for the owner and the signers. Written by
    /// [`crate::interface::petition`] from the name cache.
    pub names: HashMap<u64, String>,
}

impl Charter {
    /// The open charter's record, when its query has answered.
    pub fn record(&self) -> Option<&PetitionQuery> {
        self.records.get(&self.petition.as_ref()?.shown.petition)
    }

    /// The guids whose names the open charter shows: its owner and its
    /// signers.
    pub fn named_guids(&self) -> Vec<u64> {
        let Some(open) = &self.petition else {
            return Vec::new();
        };
        std::iter::once(open.shown.owner)
            .chain(open.shown.signers.iter().copied())
            .collect()
    }

    /// Whether the open charter can be drawn whole: its record is held and
    /// every signer has a name.
    pub fn ready(&self) -> bool {
        let Some(open) = &self.petition else {
            return false;
        };
        self.record().is_some()
            && open
                .shown
                .signers
                .iter()
                .all(|guid| self.names.contains_key(guid))
    }

    /// `CanSignPetition`. Refused for the charter's owner and for a player
    /// who has signed it. A guild charter is also refused to a member of a
    /// guild and when it holds as many signatures as it takes.
    fn can_sign(&self) -> bool {
        let (Some(open), Some(record)) = (&self.petition, self.record()) else {
            return false;
        };
        let charter = record.flags & FLAG_CHARTER != 0;
        let full = open.shown.signers.len() as u32 >= record.max_signatures;
        !(charter && (self.in_guild || full))
            && open.shown.owner != self.own_guid
            && !open.shown.signers.contains(&self.own_guid)
    }
}

/// The key of the refusal a guild name gets before it is sent, or `None` for
/// a name that is sent.
///
/// Two of the 1.12.1 client's checks are made here: an empty name and a name
/// over [`MAX_NAME_BYTES`]. Its checks on the characters themselves are not;
/// the server makes its own and answers a name it refuses with
/// `SMSG_GUILD_COMMAND_RESULT`.
pub fn name_fault(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("ERR_GUILD_ENTER_NAME")
    } else if name.len() > MAX_NAME_BYTES {
        Some("ERR_GUILD_NAME_INVALID")
    } else {
        None
    }
}

pub type Held = Rc<RefCell<Charter>>;
pub type Queue = Rc<RefCell<Vec<Press>>>;

/// Register the thirteen functions.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    // The registrar's price, which `GuildRegistrar_ShowPurchaseFrame` hands
    // to `MoneyFrame_Update`. Zero before a registrar has stated one.
    let get = Rc::clone(held);
    globals.set(
        "GetGuildCharterCost",
        lua.create_function(move |_, ()| Ok(get.borrow().offer.map_or(0, |offer| offer.cost)))?,
    )?;

    // `(petitionType, title, bodyText, maxSignatures, originatorName,
    // isOriginator)`. The owner's name is nil until its name query answers.
    let get = Rc::clone(held);
    globals.set(
        "GetPetitionInfo",
        lua.create_function(move |lua, ()| {
            let board = get.borrow();
            let (Some(open), Some(record)) = (&board.petition, board.record()) else {
                return Ok(mlua::Variadic::from(vec![
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Number(0.0),
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                ]));
            };
            let text = |s: &str| lua.create_string(s).map(mlua::Value::String);
            let kind = if record.flags & FLAG_CHARTER != 0 {
                "charter"
            } else {
                "petition"
            };
            Ok(mlua::Variadic::from(vec![
                text(kind)?,
                text(&record.name)?,
                text(&record.body)?,
                mlua::Value::Number(f64::from(record.max_signatures)),
                match board.names.get(&open.shown.owner) {
                    Some(name) => text(name)?,
                    None => mlua::Value::Nil,
                },
                if open.shown.owner == board.own_guid {
                    mlua::Value::Number(1.0)
                } else {
                    mlua::Value::Nil
                },
            ]))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetNumPetitionNames",
        lua.create_function(move |_, ()| {
            Ok(get
                .borrow()
                .petition
                .as_ref()
                .map_or(0, |open| open.shown.signers.len()))
        })?,
    )?;

    // One-based. Nil for a line nobody has signed and for a signer whose
    // name has not arrived.
    let get = Rc::clone(held);
    globals.set(
        "GetPetitionNameInfo",
        lua.create_function(move |_, index: Option<usize>| {
            let board = get.borrow();
            Ok(index
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| board.petition.as_ref()?.shown.signers.get(i))
                .and_then(|guid| board.names.get(guid).cloned()))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "CanSignPetition",
        lua.create_function(move |_, ()| Ok(get.borrow().can_sign().then_some(1)))?,
    )?;

    // The signature is recorded here, so that hiding the window in the same
    // handler does not decline the charter.
    let get = Rc::clone(held);
    let send = Rc::clone(queue);
    globals.set(
        "SignPetition",
        lua.create_function(move |_, byte: Option<u8>| {
            if let Some(open) = get.borrow_mut().petition.as_mut() {
                open.signed = true;
                send.borrow_mut().push(Press::Sign(byte.unwrap_or(1)));
            }
            Ok(())
        })?,
    )?;

    let plain = |name: &str, press: fn() -> Press| -> mlua::Result<()> {
        let send = Rc::clone(queue);
        globals.set(
            name,
            lua.create_function(move |_, ()| {
                send.borrow_mut().push(press());
                Ok(())
            })?,
        )
    };
    plain("TurnInGuildCharter", || Press::TurnIn)?;
    plain("OfferPetition", || Press::Offer)?;
    plain("ClosePetition", || Press::ClosePetition)?;
    plain("CloseGuildRegistrar", || Press::CloseRegistrar)?;
    plain("GetTabardInfo", || Press::TabardInfo)?;

    // A request that carries a guild name. A name that fails `name_fault`
    // queues the refusal and answers nil; a name that passes answers 1.
    let named = |name: &str, press: fn(String) -> Press| -> mlua::Result<()> {
        let send = Rc::clone(queue);
        globals.set(
            name,
            lua.create_function(move |_, text: Option<String>| {
                let text = text.unwrap_or_default();
                Ok(match name_fault(&text) {
                    Some(key) => {
                        send.borrow_mut().push(Press::Refuse(key));
                        None
                    }
                    None => {
                        send.borrow_mut().push(press(text));
                        Some(1)
                    }
                })
            })?,
        )
    };
    named("BuyGuildCharter", Press::Buy)?;
    named("RenamePetition", Press::Rename)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered() -> (Held, Queue, mlua::Lua) {
        let lua = mlua::Lua::new();
        let held: Held = Rc::default();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &held, &queue).expect("registers");
        {
            let mut board = held.borrow_mut();
            board.own_guid = 0x10;
            board.names.insert(0x10, "Cade".into());
            board.names.insert(0x11, "Bram".into());
            board.records.insert(
                42,
                PetitionQuery {
                    petition: 42,
                    owner: 0x10,
                    name: "The Watch".into(),
                    body: String::new(),
                    flags: FLAG_CHARTER,
                    min_signatures: 9,
                    max_signatures: 9,
                },
            );
            board.petition = Some(Open {
                shown: Signatures {
                    item: 7,
                    owner: 0x10,
                    petition: 42,
                    signers: vec![0x11, 0x12],
                },
                signed: false,
            });
        }
        (held, queue, lua)
    }

    /// The list above and the registration are the same set, in both
    /// directions.
    #[test]
    fn every_verb_is_registered() {
        let names = |lua: &mlua::Lua| -> std::collections::BTreeSet<String> {
            lua.globals()
                .pairs::<String, mlua::Value>()
                .filter_map(Result::ok)
                .map(|(name, _)| name)
                .collect()
        };
        let lua = mlua::Lua::new();
        let before = names(&lua);
        register(&lua, &Rc::default(), &Rc::new(RefCell::new(Vec::new()))).expect("registers");
        let added: std::collections::BTreeSet<String> =
            names(&lua).difference(&before).cloned().collect();
        let listed: std::collections::BTreeSet<String> =
            VERBS.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(added, listed);
    }

    /// The six values are in the order `PetitionFrame_Update` assigns them,
    /// and the owner of the charter is its originator.
    #[test]
    fn the_petition_answers_six_values_to_its_owner() {
        let (_held, _queue, lua) = registered();
        let (kind, title, max, owner, mine): (String, String, i64, String, i64) = lua
            .load("local k, t, b, m, o, i = GetPetitionInfo(); return k, t, m, o, i")
            .eval()
            .expect("values");
        assert_eq!((kind.as_str(), title.as_str(), max), ("charter", "The Watch", 9));
        assert_eq!((owner.as_str(), mine), ("Cade", 1));
    }

    /// A signer whose name has not arrived answers nil, and the charter is
    /// not ready to draw until it has.
    #[test]
    fn a_charter_is_ready_when_every_signer_is_named() {
        let (held, _queue, lua) = registered();
        let (count, first, second): (i64, String, mlua::Value) = lua
            .load("return GetNumPetitionNames(), GetPetitionNameInfo(1), GetPetitionNameInfo(2)")
            .eval()
            .expect("values");
        assert_eq!((count, first.as_str()), (2, "Bram"));
        assert!(matches!(second, mlua::Value::Nil));
        assert!(!held.borrow().ready());
        held.borrow_mut().names.insert(0x12, "Adele".into());
        assert!(held.borrow().ready());
        held.borrow_mut().records.clear();
        assert!(!held.borrow().ready(), "no record");
    }

    /// The owner cannot sign, a member of a guild cannot, a player who has
    /// signed cannot sign again, and a full charter takes no signature.
    #[test]
    fn who_may_sign() {
        let (held, _queue, _lua) = registered();
        assert!(!held.borrow().can_sign(), "the owner");
        held.borrow_mut().own_guid = 0x13;
        assert!(held.borrow().can_sign());
        held.borrow_mut().in_guild = true;
        assert!(!held.borrow().can_sign(), "in a guild");
        held.borrow_mut().in_guild = false;
        held.borrow_mut().own_guid = 0x11;
        assert!(!held.borrow().can_sign(), "already signed");
        held.borrow_mut().own_guid = 0x13;
        held.borrow_mut().records.get_mut(&42).expect("record").max_signatures = 2;
        assert!(!held.borrow().can_sign(), "full");
        held.borrow_mut().petition = None;
        assert!(!held.borrow().can_sign(), "no charter open");
    }

    /// With no charter open `GetPetitionInfo` still answers six values, the
    /// fourth of them the number 0.
    #[test]
    fn a_shut_window_answers_six_values() {
        let (held, _queue, lua) = registered();
        held.borrow_mut().petition = None;
        let (values, max, names, cost): (i64, i64, i64, i64) = lua
            .load(
                r#"
                local k, t, b, m = GetPetitionInfo()
                return select('#', GetPetitionInfo()), m, GetNumPetitionNames(), GetGuildCharterCost()
                "#,
            )
            .eval()
            .expect("values");
        assert_eq!((values, max, names, cost), (6, 0, 0, 0));
    }

    /// A name is checked before it is queued, the signature byte defaults to
    /// 1, and signing marks the open charter as signed.
    #[test]
    fn a_press_is_queued_and_a_bad_name_is_refused() {
        let (held, queue, lua) = registered();
        let (empty, long, good): (mlua::Value, mlua::Value, i64) = lua
            .load(
                r#"
                return BuyGuildCharter(""), BuyGuildCharter("A name of twenty-five by."),
                    BuyGuildCharter("The Watch")
                "#,
            )
            .eval()
            .expect("values");
        assert!(matches!(empty, mlua::Value::Nil) && matches!(long, mlua::Value::Nil));
        assert_eq!(good, 1);
        lua.load("SignPetition(); RenamePetition('New'); GetTabardInfo()")
            .exec()
            .expect("runs");
        assert_eq!(
            queue.borrow().as_slice(),
            [
                Press::Refuse("ERR_GUILD_ENTER_NAME"),
                Press::Refuse("ERR_GUILD_NAME_INVALID"),
                Press::Buy("The Watch".into()),
                Press::Sign(1),
                Press::Rename("New".into()),
                Press::TabardInfo,
            ]
        );
        assert!(held.borrow().petition.as_ref().expect("open").signed);
    }
}
