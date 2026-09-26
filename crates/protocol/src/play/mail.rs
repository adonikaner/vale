//! **The mailbox** — what is in it, what one letter holds, and sending one.
//!
//! ```text
//! CMSG_GET_MAIL_LIST         mailbox        the inbox, whole
//!   -> SMSG_MAIL_LIST_RESULT                one header per letter, up to 254
//! CMSG_ITEM_TEXT_QUERY       textId, mailId the body of one letter
//!   -> SMSG_ITEM_TEXT_QUERY_RESPONSE
//! CMSG_MAIL_MARK_AS_READ     mailbox, id    …which is sent by reading it
//! CMSG_MAIL_TAKE_MONEY       mailbox, id    the coins
//! CMSG_MAIL_TAKE_ITEM        mailbox, id    …and the parcel
//! CMSG_MAIL_CREATE_TEXT_ITEM mailbox, id    …and the letter itself, as an item
//! CMSG_MAIL_RETURN_TO_SENDER mailbox, id
//! CMSG_MAIL_DELETE           mailbox, id
//!   -> SMSG_SEND_MAIL_RESULT                all six answer with this one packet
//! CMSG_SEND_MAIL             a whole letter
//!   -> SMSG_SEND_MAIL_RESULT
//! MSG_QUERY_NEXT_MAIL_TIME   (no body)      is anything waiting?
//!   -> MSG_QUERY_NEXT_MAIL_TIME             a float, and 0 means yes
//! SMSG_RECEIVED_MAIL                        …something just arrived
//! ```
//!
//! ## Opening a mailbox is not a packet
//!
//! `GAMEOBJECT_TYPE_MAILBOX` has an **empty** arm in vmangos'
//! `GameObject::Use`, so `CMSG_GAMEOBJ_USE` on one produces nothing at all. The
//! client opens the window itself: it stores the mailbox's guid and raises
//! `MAIL_SHOW`, and `CheckInbox()` is what puts `CMSG_GET_MAIL_LIST` on the
//! wire — **at most once every 60 seconds**, raising `MAIL_INBOX_UPDATE` on the list it
//! already holds when it refuses. That rate limit is the client's own and the
//! server neither knows nor enforces it.
//!
//! So the mailbox guid is *held* for the length of the window and stamped into
//! all nine outbound packets, which is why every body here takes one.
//!
//! ## The sender field is a union and its tag decides its width
//!
//! `SMSG_MAIL_LIST_RESULT` writes a `u64` player guid for [`MailKind::Normal`],
//! a `u32` entry for the three that come from something rather than somebody,
//! and **nothing at all** for [`MailKind::Item`]. A reader that assumed one
//! width is wrong for the rest of the packet, not merely for that field — and
//! the client's own parser is exactly this three-way split, with
//! `MAIL_ITEM` falling through untouched.
//!
//! **`MAIL_ITEM` also breaks the client**, which is worth knowing before
//! anybody "fixes" it here: `GetInboxHeaderInfo`'s switch on the type
//! covers 0..4 and type 5 skips the push entirely, so the reference itself
//! returns twelve values where it claims thirteen. vmangos never sends one
//! (its own comment reads `item entry (?) sender = "Unknown", NYI`), and this
//! reader treats it the way the client does: the field is absent and the sender
//! is unknown.
//!
//! ## The flag word is what the panel is made of
//!
//! Five bits, and four of them decide what the buttons on an open letter say —
//! see [`MailFlags`]. The two that are not obvious: `COPIED` is why the "keep
//! this letter" button greys out and is half of why an emptied letter deletes
//! itself on close, and `COD_PAYMENT` turns the subject into
//! `format(COD_PAYMENT, subject)` before it is ever shown.
//!
//! ## Everything is answered by one packet, including the failures
//!
//! `SMSG_SEND_MAIL_RESULT` carries a mail id, **which of six actions** it is
//! about and **which of nine results** happened, and its tail is conditional on
//! both: an equip error appends the inventory code, and a successful take
//! appends the item's guid and count. A reader that always read the tail is
//! four or eight bytes past the end on every other combination.

use crate::bytes::{Reader, Writer};

/// **Who sent it**, which is a tag over a union rather than a label.
///
/// The numbering is vmangos' `MailMessageType` and the client's own switch
/// agrees with it exactly: 0 is a player, 2 an auction house (whose name
/// comes out of `AuctionHouse.dbc`), 3 a creature, 4 a game object, 5 an item.
/// **1 is not used by either side** and the client pushes nil for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MailKind {
    /// From another character: the sender field is a **`u64` guid**.
    #[default]
    Normal,
    /// The auction house. The sender field is a `u32` `AuctionHouse.dbc` row.
    Auction,
    /// A creature — most often a quest reward posted on. `u32` entry.
    Creature,
    /// A game object. `u32` entry.
    GameObject,
    /// An item. **No sender field at all**; see the module note.
    Item,
    /// Anything else, kept rather than refused: an unknown value is carried
    /// through, not dropped. The sender field is read as a `u32`, which is what
    /// the client's own parser does with 1.
    Other(u8),
}

impl MailKind {
    pub fn of(byte: u8) -> MailKind {
        match byte {
            0 => MailKind::Normal,
            2 => MailKind::Auction,
            3 => MailKind::Creature,
            4 => MailKind::GameObject,
            5 => MailKind::Item,
            other => MailKind::Other(other),
        }
    }

    /// The byte back, for a test and for anything that has to re-state it.
    pub fn byte(self) -> u8 {
        match self {
            MailKind::Normal => 0,
            MailKind::Auction => 2,
            MailKind::Creature => 3,
            MailKind::GameObject => 4,
            MailKind::Item => 5,
            MailKind::Other(byte) => byte,
        }
    }

    /// **Can this letter be replied to?** — `GetInboxHeaderInfo`'s twelfth
    /// answer, and half of the reason the Reply button is grey on most mail.
    ///
    /// Only a letter from a player, and only when it is not a GM's: the client
    /// tests the type against zero and then the GM flag, in that order.
    pub fn from_a_player(self) -> bool {
        matches!(self, MailKind::Normal)
    }
}

/// **What has happened to a letter**, as the `checked` word carries it.
///
/// A newtype rather than five `bool`s because the word is round-tripped: the
/// server sends it, the client tests four bits of it, and nothing here has any
/// business inventing a sixth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MailFlags(pub u32);

impl MailFlags {
    /// `MAIL_CHECK_MASK_READ` — the sender and subject draw grey.
    pub const READ: u32 = 0x01;
    /// `MAIL_CHECK_MASK_RETURNED` — it has already come back once, so it
    /// cannot be returned again and the button says Delete.
    pub const RETURNED: u32 = 0x02;
    /// `MAIL_CHECK_MASK_COPIED` — the body has been made into an item, so the
    /// letter itself is spent.
    pub const COPIED: u32 = 0x04;
    /// `MAIL_CHECK_MASK_COD_PAYMENT` — cash on delivery. The subject is
    /// rewritten through `COD_PAYMENT` before it is drawn.
    pub const COD_PAYMENT: u32 = 0x08;
    /// `MAIL_CHECK_MASK_HAS_BODY`.
    pub const HAS_BODY: u32 = 0x10;

    pub fn read(self) -> bool {
        self.0 & Self::READ != 0
    }
    pub fn returned(self) -> bool {
        self.0 & Self::RETURNED != 0
    }
    pub fn copied(self) -> bool {
        self.0 & Self::COPIED != 0
    }
    pub fn cod_payment(self) -> bool {
        self.0 & Self::COD_PAYMENT != 0
    }
    pub fn has_body(self) -> bool {
        self.0 & Self::HAS_BODY != 0
    }
}

/// **The one parcel a 1.12 letter may carry.** Later expansions send twelve;
/// `MAX_MAIL_ITEMS` is 1 here, and the eight fields are written even when there
/// is nothing to write — as eight zeroes, which is what `entry == 0` means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MailAttachment {
    pub entry: u32,
    /// `PERM_ENCHANTMENT_SLOT` only. The other slots are not in this packet.
    pub enchant: u32,
    /// **Signed on the wire**, and vmangos says so in its own comment. Kept as
    /// written because nothing here interprets it.
    pub random_property: i32,
    pub suffix_factor: u32,
    /// A **byte**, sitting between two `u32` runs — the same shape that made
    /// the trainer row's level field worth a comment. Read it wide and every
    /// field after it in every letter after it is three bytes out.
    pub count: u8,
    pub charges: u32,
    pub max_durability: u32,
    pub durability: u32,
}

/// One letter's header — everything `SMSG_MAIL_LIST_RESULT` says about it.
///
/// **The body is not in here** and cannot be: it arrives by
/// `CMSG_ITEM_TEXT_QUERY` against [`MailHeader::item_text_id`], one round trip
/// per letter, exactly the way an item template does.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MailHeader {
    /// The server's own id, and what all six verbs name a letter by. **Not** an
    /// index: the interface's row numbers are positions in this list and the
    /// two must never be confused.
    pub id: u32,
    pub kind: MailKind,
    /// The sender's guid, for [`MailKind::Normal`] only.
    pub sender_guid: u64,
    /// …and its entry, for the three kinds that are not a person.
    pub sender_entry: u32,
    pub subject: String,
    /// The body's key, or 0 for a letter with no words in it.
    pub item_text_id: u32,
    /// `Package.dbc` — the wrapping, which is the icon a parcel draws with.
    /// vmangos writes a hard 0 here.
    pub package: u32,
    /// `Stationery.dbc` — the paper, which decides both the letter's icon and
    /// the background the body is drawn on. **61 is the GM stationery**, and
    /// that single value is what makes a letter a GM's.
    pub stationery: u32,
    /// The parcel, or `None` when the eight item fields were all zero.
    pub item: Option<MailAttachment>,
    /// Copper enclosed.
    pub money: u32,
    /// …and copper wanted before the parcel may be taken.
    pub cod: u32,
    pub flags: MailFlags,
    /// **Days**, not seconds — the server divides by 86400 on the way out, and
    /// `InboxFrame_Update` multiplies back up again for anything under one day.
    pub days_left: f32,
    /// `MailTemplate.dbc`, for a letter whose words are in the client's own
    /// files rather than in the database. Read when `item_text_id` is 0.
    pub template_id: u32,
}

impl MailHeader {
    /// **Is this a GM's letter?** — stationery 61, tested by value.
    ///
    /// The client compares the stationery id against 61, and nothing else in
    /// the packet says so.
    pub fn is_gm(&self) -> bool {
        self.stationery == GM_STATIONERY
    }

    /// **Delete, or return to sender?** — `InboxItemCanDelete(index)`, whose
    /// answer is the *word on the button* rather than a permission.
    ///
    /// The client's tests, in order: already returned or paid on delivery, or not from
    /// a player at all, or holding neither a parcel nor coin. Anything else — a
    /// player's letter that still has something in it — is returnable, and the
    /// button says so.
    pub fn can_delete(&self) -> bool {
        self.flags.returned()
            || self.flags.cod_payment()
            || !self.kind.from_a_player()
            || (self.item.is_none() && self.money == 0)
    }

    /// **Is the letter itself takeable as an item?** — `GetInboxText`'s third
    /// answer.
    ///
    /// It needs words to keep (a body id or a template), must not have been
    /// copied already, and **must not be a GM's** — which is the one clause
    /// that is not deducible from the flags.
    pub fn text_is_takeable(&self) -> bool {
        (self.item_text_id != 0 || self.template_id != 0) && !self.flags.copied() && !self.is_gm()
    }
}

/// `MAIL_STATIONERY_GM`, and the only stationery id either side tests by value.
pub const GM_STATIONERY: u32 = 61;

/// **The base postage**, in copper — a constant 30 in the client.
///
/// The whole of `GetSendMailPrice()` is this plus the selected stationery's own
/// item price, and *that* half is skipped when the character already carries
/// one. The second half needs an item template and so cannot live here; see
/// `crate::play::mail`'s caller in the client.
pub const POSTAGE: u32 = 30;

/// **What the server has an answer about** — vmangos' `MailResponseType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailAction {
    Send,
    MoneyTaken,
    ItemTaken,
    ReturnedToSender,
    Deleted,
    MadePermanent,
    /// Anything else, kept rather than refused.
    Other(u32),
}

impl MailAction {
    pub fn of(word: u32) -> MailAction {
        match word {
            0 => MailAction::Send,
            1 => MailAction::MoneyTaken,
            2 => MailAction::ItemTaken,
            3 => MailAction::ReturnedToSender,
            4 => MailAction::Deleted,
            5 => MailAction::MadePermanent,
            other => MailAction::Other(other),
        }
    }
}

/// …and what it says — vmangos' `MailResponseResult`.
///
/// **The gap between 6 and 14 is real**, not a transcription slip: the two high
/// codes are later additions the 1.12 server still sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailResult {
    Ok,
    EquipError,
    CannotSendToSelf,
    NotEnoughMoney,
    RecipientNotFound,
    NotYourTeam,
    InternalError,
    DisabledForTrialAccount,
    RecipientCapReached,
    Other(u32),
}

impl MailResult {
    pub fn of(word: u32) -> MailResult {
        match word {
            0 => MailResult::Ok,
            1 => MailResult::EquipError,
            2 => MailResult::CannotSendToSelf,
            3 => MailResult::NotEnoughMoney,
            4 => MailResult::RecipientNotFound,
            5 => MailResult::NotYourTeam,
            6 => MailResult::InternalError,
            14 => MailResult::DisabledForTrialAccount,
            15 => MailResult::RecipientCapReached,
            other => MailResult::Other(other),
        }
    }

    /// **The `GlobalStrings.lua` key the reference shows for this code** — taken
    /// from its own handler rather than reasoned from the code's name.
    ///
    /// The client switches on the result word into nine arms, and every arm but
    /// the first shows one error row:
    ///
    /// ```text
    ///  0 Ok                       -- not a refusal; see below
    ///  1 EquipError               the inventory result's own message
    ///  2 CannotSendToSelf         356 ERR_MAIL_TO_SELF
    ///  3 NotEnoughMoney            37 ERR_NOT_ENOUGH_MONEY
    ///  4 RecipientNotFound        357 ERR_MAIL_TARGET_NOT_FOUND
    ///  5 NotYourTeam              255 ERR_PLAYER_WRONG_FACTION
    ///  6..13                      358 ERR_MAIL_DATABASE_ERROR
    /// 14 DisabledForTrialAccount  446 ERR_RESTRICTED_ACCOUNT
    /// 15 RecipientCapReached      452 ERR_MAIL_REACHED_CAP
    /// ```
    ///
    /// **Two of these were guessed here and both guesses were wrong**, which is
    /// why the table above is quoted rather than described.
    /// [`MailResult::NotYourTeam`] was answered `None` on the reasoning that
    /// 5875 ships no *mail* faction string — true, and the client borrows the
    /// **player** one anyway; and everything from 6 to 13 was answered `None`
    /// where the reference says `ERR_MAIL_DATABASE_ERROR`, which is the line a
    /// player actually sees when a send fails for a reason nobody named.
    ///
    /// [`MailResult::EquipError`] answers `None` here and that one is real: the
    /// message is the *inventory* result's, which is a second table this
    /// function has no access to — see [`MailResponse::equip_error`], which
    /// carries the code for a caller that does.
    pub fn key(self) -> Option<&'static str> {
        Some(match self {
            MailResult::Ok | MailResult::EquipError => return None,
            MailResult::CannotSendToSelf => "ERR_MAIL_TO_SELF",
            MailResult::NotEnoughMoney => "ERR_NOT_ENOUGH_MONEY",
            MailResult::RecipientNotFound => "ERR_MAIL_TARGET_NOT_FOUND",
            MailResult::NotYourTeam => "ERR_PLAYER_WRONG_FACTION",
            MailResult::DisabledForTrialAccount => "ERR_RESTRICTED_ACCOUNT",
            MailResult::RecipientCapReached => "ERR_MAIL_REACHED_CAP",
            // **6..13 and anything past 15**, which the client folds into one
            // arm; everything above 15 goes to the same place.
            MailResult::InternalError | MailResult::Other(_) => "ERR_MAIL_DATABASE_ERROR",
        })
    }

    /// **What a *successful send* says** — error row 359, shown before the
    /// draft is cleared.
    ///
    /// Not part of [`Self::key`] because it is not a refusal and because it is
    /// conditional on the *action* as well as the result: only
    /// [`MailAction::Send`] says it. `ERR_MAIL_SENT` is `UI_INFO_MESSAGE`, so
    /// it is the yellow line in the middle of the screen rather than a chat
    /// line.
    pub const SENT: &'static str = "ERR_MAIL_SENT";

    /// **Every key this enum can name**, so a test can cross them against the
    /// game's own `GlobalStrings.lua` rather than against a memory of it — the
    /// check that would have caught `ERR_MAIL_WRONG_FACTION`, which does not
    /// exist and was written here from the shape of its neighbours.
    pub const KEYS: [&'static str; 7] = [
        "ERR_MAIL_TO_SELF",
        "ERR_NOT_ENOUGH_MONEY",
        "ERR_MAIL_TARGET_NOT_FOUND",
        "ERR_PLAYER_WRONG_FACTION",
        "ERR_RESTRICTED_ACCOUNT",
        "ERR_MAIL_REACHED_CAP",
        "ERR_MAIL_DATABASE_ERROR",
    ];
}

/// `SMSG_SEND_MAIL_RESULT`, whole — and its tail is conditional on both words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailResponse {
    /// 0 for a send, since a letter that has not been made has no id.
    pub mail_id: u32,
    pub action: MailAction,
    pub result: MailResult,
    /// `InventoryResult`, present **only** on [`MailResult::EquipError`].
    pub equip_error: Option<u32>,
    /// The item's low guid and its count, present **only** on a successful
    /// [`MailAction::ItemTaken`].
    pub item: Option<(u32, u32)>,
}

// --- parsing ----------------------------------------------------------------

/// How many letters one packet may name. The count is a byte and vmangos stops
/// at 254 itself, so this is the wire's own ceiling rather than a guard.
const MAX_MAIL: usize = 254;

/// `SMSG_MAIL_LIST_RESULT` — a byte count and then that many headers.
///
/// **A short tail stops the walk rather than failing the packet**, which is
/// this crate's rule for a damaged body: a mailbox that reads nine of its ten
/// letters is a mailbox, and returning `None` would draw an empty one.
pub fn parse_mail_list(body: &[u8]) -> Option<Vec<MailHeader>> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let count = usize::from(r.u8()).min(MAX_MAIL);
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        match read_header(&mut r) {
            Some(header) => out.push(header),
            None => break,
        }
    }
    Some(out)
}

/// One header, or `None` when the body runs out inside it.
fn read_header(r: &mut Reader) -> Option<MailHeader> {
    if !r.has(4 + 1) {
        return None;
    }
    let id = r.u32();
    let kind = MailKind::of(r.u8());
    // **The union**, and the widths are the client's own three-way split — see
    // the module note. `MailKind::Item` reads nothing.
    let (sender_guid, sender_entry) = match kind {
        MailKind::Normal => {
            if !r.has(8) {
                return None;
            }
            (r.u64(), 0)
        }
        MailKind::Item => (0, 0),
        _ => {
            if !r.has(4) {
                return None;
            }
            (0, r.u32())
        }
    };
    let subject = r.cstring();
    // itemTextId, package, stationery; the eight item fields; money, cod,
    // checked; the days float; the template id.
    if !r.has(4 * 3 + (4 * 4 + 1 + 4 * 3) + 4 * 3 + 4 + 4) {
        return None;
    }
    let item_text_id = r.u32();
    let package = r.u32();
    let stationery = r.u32();
    let attachment = MailAttachment {
        entry: r.u32(),
        enchant: r.u32(),
        random_property: r.u32() as i32,
        suffix_factor: r.u32(),
        count: r.u8(),
        charges: r.u32(),
        max_durability: r.u32(),
        durability: r.u32(),
    };
    Some(MailHeader {
        id,
        kind,
        sender_guid,
        sender_entry,
        subject,
        item_text_id,
        package,
        stationery,
        // **Zero entry is no parcel**, which is the shape the server writes
        // rather than an omitted block: eight zero fields are always present.
        item: (attachment.entry != 0).then_some(attachment),
        money: r.u32(),
        cod: r.u32(),
        flags: MailFlags(r.u32()),
        days_left: r.f32(),
        template_id: r.u32(),
    })
}

/// `SMSG_SEND_MAIL_RESULT` — see [`MailResponse`] for why the tail is read
/// conditionally.
pub fn parse_mail_result(body: &[u8]) -> Option<MailResponse> {
    let mut r = Reader::new(body);
    if !r.has(4 * 3) {
        return None;
    }
    let mail_id = r.u32();
    let action = MailAction::of(r.u32());
    let result = MailResult::of(r.u32());
    let mut equip_error = None;
    let mut item = None;
    if result == MailResult::EquipError {
        if !r.has(4) {
            return None;
        }
        equip_error = Some(r.u32());
    } else if action == MailAction::ItemTaken {
        if !r.has(8) {
            return None;
        }
        item = Some((r.u32(), r.u32()));
    }
    Some(MailResponse {
        mail_id,
        action,
        result,
        equip_error,
        item,
    })
}

/// `SMSG_RECEIVED_MAIL` — one word, and vmangos always writes 0.
///
/// **The packet's arrival is the whole of its meaning.** It is what turns the
/// envelope on the minimap on, by way of a fresh `MSG_QUERY_NEXT_MAIL_TIME`.
pub fn parse_received_mail(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `MSG_QUERY_NEXT_MAIL_TIME` — **seconds until the next letter is
/// deliverable**, and the sign is the answer.
///
/// vmangos sends `0.0` when something is already waiting and `-86400.0` when
/// nothing is. The client keeps the value and counts it down every frame,
/// clamping at zero; `HasNewMail()` is
/// `abs(value) <= epsilon`, i.e. **exactly zero**, which is why a negative
/// number is "nothing" rather than "overdue".
pub fn parse_next_mail_time(body: &[u8]) -> Option<f32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.f32())
}

/// `SMSG_ITEM_TEXT_QUERY_RESPONSE` — the id back, and the words.
pub fn parse_item_text(body: &[u8]) -> Option<(u32, String)> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let id = r.u32();
    Some((id, r.cstring()))
}

// --- what we send -----------------------------------------------------------

/// `CMSG_GET_MAIL_LIST` — the mailbox, and nothing else.
pub fn get_mail_list_body(mailbox: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(mailbox);
    w.buf
}

/// **The five verbs that name one letter**: mark as read, take the money, take
/// the parcel, return it, delete it. Identical bodies, which is why they are
/// one function.
pub fn mail_id_body(mailbox: u64, mail_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(mailbox);
    w.u32(mail_id);
    w.buf
}

/// `CMSG_MAIL_CREATE_TEXT_ITEM` — the sixth, and the one with a tail.
///
/// The trailing word is the mail template id, which the server reads and
/// **throws away** (`read_skip<uint32>`, "Mail store own 100% correct value
/// anyway"). It is sent because 1.12's client sends it; a body without it is
/// four bytes short and the server's own read runs off the end.
pub fn create_text_item_body(mailbox: u64, mail_id: u32, template_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(mailbox);
    w.u32(mail_id);
    w.u32(template_id);
    w.buf
}

/// The constant the client builds into `CMSG_ITEM_TEXT_QUERY`'s third word.
/// vmangos names it `unk` and does nothing with it.
const ITEM_TEXT_QUERY_TAIL: u32 = 0x7000_0000;

/// `CMSG_ITEM_TEXT_QUERY` — the body of a letter, by its text id.
///
/// The third word is a constant the client `or`s together and the server
/// ignores. Sent as the client sends it.
pub fn item_text_query_body(item_text_id: u32, mail_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(item_text_id);
    w.u32(mail_id);
    w.u32(ITEM_TEXT_QUERY_TAIL);
    w.buf
}

/// **A letter on its way out** — every field `CMSG_SEND_MAIL` carries that the
/// caller decides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutgoingMail {
    pub mailbox: u64,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// `Stationery.dbc` — vmangos calls it `unk1` and stores it on the letter,
    /// so it really does come back in the list. **Non-zero is checked by the
    /// client before it will send at all**.
    pub stationery: u32,
    /// `Package.dbc`, which vmangos calls `unk2` and ignores. The client sends
    /// its selected package here and 0 when nothing is attached.
    pub package: u32,
    /// The parcel's own object guid, or 0.
    pub item: u64,
    /// Copper enclosed — mutually exclusive with [`Self::cod`], which is the
    /// client's own gate rather than the server's.
    pub money: u32,
    /// …and copper wanted on delivery, which needs an item to be attached to.
    pub cod: u32,
}

/// `CMSG_SEND_MAIL`, in the order the client writes it.
///
/// **The nine bytes at the end are the client's**, not padding this could drop:
/// vmangos reads a `uint64` and a `uint8` after the COD for every build past
/// 1.9.4, so a body that stopped at the COD leaves the server's own `>>`
/// operators reading off the end.
pub fn send_mail_body(mail: &OutgoingMail) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(mail.mailbox);
    w.cstring(&mail.to);
    w.cstring(&mail.subject);
    w.cstring(&mail.body);
    w.u32(mail.stationery);
    w.u32(mail.package);
    w.u64(mail.item);
    w.u32(mail.money);
    w.u32(mail.cod);
    w.u64(0);
    w.u8(0);
    w.buf
}

/// `MSG_QUERY_NEXT_MAIL_TIME` — no body at all.
pub fn next_mail_time_body() -> Vec<u8> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: &mut Writer, id: u32, kind: u8, subject: &str, entry: u32) {
        w.u32(id);
        w.u8(kind);
        match kind {
            0 => {
                w.u64(0x0000_0000_0000_00AA);
            }
            5 => {}
            _ => {
                w.u32(0x1234);
            }
        }
        w.bytes(subject.as_bytes());
        w.u8(0);
        w.u32(77); // itemTextId
        w.u32(0); // package
        w.u32(41); // stationery
        w.u32(entry);
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u8(if entry == 0 { 0 } else { 5 });
        w.u32(0);
        w.u32(0);
        w.u32(0);
        w.u32(1234); // money
        w.u32(0); // cod
        w.u32(MailFlags::READ); // checked
        w.f32(29.5); // days
        w.u32(0); // template
    }

    /// **The union's width is decided by the tag, and getting it wrong loses
    /// every letter after it** — which is what makes a two-letter packet with
    /// two different kinds the check worth having.
    #[test]
    fn the_sender_field_is_as_wide_as_the_kind_says() {
        let mut w = Writer::new();
        w.u8(2);
        header(&mut w, 10, 0, "From a friend", 0);
        header(&mut w, 11, 3, "From a creature", 6948);
        let list = parse_mail_list(&w.buf).expect("a list");
        assert_eq!(list.len(), 2, "the second header survived the first's guid");
        assert_eq!(list[0].kind, MailKind::Normal);
        assert_eq!(list[0].sender_guid, 0xAA);
        assert_eq!(list[0].sender_entry, 0);
        assert_eq!(list[0].subject, "From a friend");
        assert_eq!(list[1].kind, MailKind::Creature);
        assert_eq!(list[1].sender_guid, 0);
        assert_eq!(list[1].sender_entry, 0x1234);
        assert_eq!(list[1].item.map(|i| (i.entry, i.count)), Some((6948, 5)));
        assert_eq!(list[1].money, 1234);
        assert!((list[1].days_left - 29.5).abs() < 1e-6);
    }

    /// **`MAIL_ITEM` writes no sender at all**, which is the third width and
    /// the one a reader is most likely to get wrong by assuming there are two.
    #[test]
    fn an_item_sender_occupies_no_bytes() {
        let mut w = Writer::new();
        w.u8(1);
        header(&mut w, 12, 5, "From an item", 0);
        let list = parse_mail_list(&w.buf).expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].kind, MailKind::Item);
        assert_eq!(list[0].subject, "From an item");
        assert_eq!(list[0].money, 1234);
    }

    /// A truncated tail keeps what was whole — the rule this crate applies to
    /// every damaged body.
    #[test]
    fn a_short_body_keeps_the_letters_that_were_complete() {
        let mut w = Writer::new();
        w.u8(2);
        header(&mut w, 10, 0, "Whole", 0);
        w.u32(11);
        w.u8(0);
        let list = parse_mail_list(&w.buf).expect("a list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, 10);
    }

    /// **The result's tail depends on both words**, and reading it
    /// unconditionally runs off the end of five of the six actions.
    #[test]
    fn the_result_tail_is_conditional_on_the_action_and_the_code() {
        let mut w = Writer::new();
        w.u32(9).u32(4).u32(0);
        let deleted = parse_mail_result(&w.buf).expect("a result");
        assert_eq!(deleted.action, MailAction::Deleted);
        assert_eq!(deleted.result, MailResult::Ok);
        assert_eq!(deleted.item, None);
        assert_eq!(deleted.equip_error, None);

        let mut w = Writer::new();
        w.u32(9).u32(2).u32(0).u32(0x55).u32(3);
        let taken = parse_mail_result(&w.buf).expect("a result");
        assert_eq!(taken.item, Some((0x55, 3)));

        let mut w = Writer::new();
        w.u32(9).u32(2).u32(1).u32(50);
        let refused = parse_mail_result(&w.buf).expect("a result");
        assert_eq!(refused.result, MailResult::EquipError);
        assert_eq!(refused.equip_error, Some(50));
        assert_eq!(refused.item, None, "the equip code is not an item pair");
    }

    /// **Zero is new mail and a negative number is none**, which is the
    /// opposite of what "next mail time" reads like.
    #[test]
    fn the_next_mail_time_is_new_mail_only_at_zero() {
        let mut w = Writer::new();
        w.f32(0.0);
        assert_eq!(parse_next_mail_time(&w.buf), Some(0.0));
        let mut w = Writer::new();
        w.f32(-86400.0);
        assert_eq!(parse_next_mail_time(&w.buf), Some(-86400.0));
    }

    /// The button's own word, in the four states that decide it.
    #[test]
    fn a_letter_says_delete_unless_a_player_sent_something_in_it() {
        let mut letter = MailHeader {
            kind: MailKind::Normal,
            money: 100,
            ..Default::default()
        };
        assert!(!letter.can_delete(), "coin from a player is returnable");
        letter.money = 0;
        assert!(letter.can_delete(), "an empty letter is deletable");
        letter.item = Some(MailAttachment {
            entry: 6948,
            ..Default::default()
        });
        assert!(!letter.can_delete());
        letter.flags = MailFlags(MailFlags::RETURNED);
        assert!(letter.can_delete(), "it has already come back once");
        letter.flags = MailFlags(MailFlags::COD_PAYMENT);
        assert!(letter.can_delete());
        letter.flags = MailFlags(0);
        letter.kind = MailKind::Creature;
        assert!(letter.can_delete(), "nothing to return it to");
    }

    /// **A GM's letter cannot be kept as an item**, which is the clause the
    /// flags do not carry.
    #[test]
    fn a_gm_letter_is_never_takeable_as_text() {
        let letter = MailHeader {
            item_text_id: 5,
            stationery: GM_STATIONERY,
            ..Default::default()
        };
        assert!(letter.is_gm());
        assert!(!letter.text_is_takeable());
        let ordinary = MailHeader {
            stationery: 41,
            ..letter.clone()
        };
        assert!(ordinary.text_is_takeable());
        let copied = MailHeader {
            flags: MailFlags(MailFlags::COPIED),
            ..ordinary.clone()
        };
        assert!(!copied.text_is_takeable());
    }

    /// **The nine bytes after the COD are not optional** — see
    /// [`send_mail_body`].
    #[test]
    fn a_sent_letter_carries_the_two_trailing_constants() {
        let body = send_mail_body(&OutgoingMail {
            mailbox: 0x1122_3344_5566_7788,
            to: "Bram".into(),
            subject: "Hello".into(),
            body: "Hi".into(),
            stationery: 41,
            package: 0,
            item: 0,
            money: 500,
            cod: 0,
        });
        let expected =
            8 + "Bram\0".len() + "Hello\0".len() + "Hi\0".len() + 4 + 4 + 8 + 4 + 4 + 8 + 1;
        assert_eq!(body.len(), expected);
        assert_eq!(body[body.len() - 9..], [0u8; 9]);
    }

    /// **The reference's own switch, arm by arm** — see [`MailResult::key`],
    /// where the client's table is quoted. This is the check that
    /// two of these were once *reasoned* rather than read, and both readings
    /// were wrong.
    #[test]
    fn every_result_says_what_the_clients_own_switch_says() {
        let key = |code: u32| MailResult::of(code).key();
        assert_eq!(key(0), None, "a success is not a refusal");
        assert_eq!(key(1), None, "the equip error's message is the inventory's");
        assert_eq!(key(2), Some("ERR_MAIL_TO_SELF"));
        assert_eq!(key(3), Some("ERR_NOT_ENOUGH_MONEY"));
        assert_eq!(key(4), Some("ERR_MAIL_TARGET_NOT_FOUND"));
        // **The player string, not a mail one** — 5875 ships no mail faction
        // line and the client borrows this one.
        assert_eq!(key(5), Some("ERR_PLAYER_WRONG_FACTION"));
        // …and 6 through 13 are one arm.
        for code in 6..=13 {
            assert_eq!(key(code), Some("ERR_MAIL_DATABASE_ERROR"), "code {code}");
        }
        assert_eq!(key(14), Some("ERR_RESTRICTED_ACCOUNT"));
        assert_eq!(key(15), Some("ERR_MAIL_REACHED_CAP"));
        // …and anything past the table's end falls into the same arm.
        assert_eq!(key(99), Some("ERR_MAIL_DATABASE_ERROR"));

        let named: Vec<&str> = (0..16).filter_map(key).collect();
        for listed in MailResult::KEYS {
            assert!(named.contains(&listed), "{listed} is listed and never produced");
        }
    }

    /// The five one-letter verbs really do share a body, and the sixth does
    /// not.
    #[test]
    fn the_letter_verbs_are_a_mailbox_and_an_id() {
        assert_eq!(mail_id_body(0x77, 9).len(), 12);
        assert_eq!(create_text_item_body(0x77, 9, 0).len(), 16);
        assert_eq!(get_mail_list_body(0x77).len(), 8);
        assert!(next_mail_time_body().is_empty());
    }
}
