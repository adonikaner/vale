//! **The mailbox** — the inbox the character can read at one, and the letter
//! they can put in it.
//!
//! The client's half of [`vale_protocol::play::mail`]. Three things live
//! here and each is a different kind of state:
//!
//! ```text
//! the mailbox   which one is open, which is entirely the client's — see below
//! the inbox     SMSG_MAIL_LIST_RESULT, which arrives whole and replaces whole
//! the draft     what is in the Send tab, which no packet states until it is sent
//! ```
//!
//! ## Opening one crosses no wire, and that is the part to get right
//!
//! vmangos' `GameObject::Use` has an **empty** arm for
//! `GAMEOBJECT_TYPE_MAILBOX`, so the use packet this client already sends at
//! every clickable game object reaches a body that does nothing. The window is
//! the client's own: it stores the guid and raises `MAIL_SHOW`, and
//! only then does `CheckInbox()` ask for the list.
//!
//! So [`Mailbox::guid`] is what "the mail window is open" *means* here, and it
//! is stamped into all nine outbound packets, because the server re-checks that
//! the character is standing at that mailbox on every one of them
//! (`WorldSession::CheckMailBox`).
//!
//! ## The inbox is asked for once a minute and never patched
//!
//! `CheckInbox()` is rate-limited by the client at 60 seconds and raises
//! `MAIL_INBOX_UPDATE` on the list
//! it already has when it refuses. That is faithfully reproduced, and it is not
//! decoration: `MailFrame_OnEvent` calls `CheckInbox()` on every `MAIL_SHOW`,
//! so a player opening and closing a mailbox ten times would otherwise send ten
//! full inbox requests.
//!
//! **Nothing patches the local list from a result code.** The six verbs are all
//! answered by `SMSG_SEND_MAIL_RESULT`, which names an id and an outcome and
//! nothing else — so a client that edited its own copy from those would drift
//! the first time anything else touched the mailbox. What happens instead is
//! what the reference does: a successful verb clears the rate limit and asks
//! for the list again.
//!
//! **The one local edit is the read flag**, and it is local because the *packet*
//! is: `GetInboxText` sends `CMSG_MAIL_MARK_AS_READ` and the server
//! answers with nothing at all, so the row would stay yellow until the next
//! full list if the flag were not set here.
//!
//! ## A letter's words and a letter's sender are both second round trips
//!
//! The body arrives by `CMSG_ITEM_TEXT_QUERY` against the header's own
//! `itemTextId` — one query per letter, cached for the session like an item
//! template — and the sender is a guid that `CMSG_NAME_QUERY` has to answer for,
//! which is [`ObjectManager::want_social_guid`]'s queue, the same one the
//! friends list uses.
//!
//! **A sender that is not a player is answered as nothing**, which draws the
//! interface's own `UNKNOWN`. The reference resolves the other three kinds out
//! of `AuctionHouse.dbc` and its creature and game-object caches; this client
//! reads none of the three for a letter, and that is a **stated absence**
//! rather than an oversight — an auction house is a subsystem this client does
//! not have, and a creature entry with no creature in view has no query door
//! here. The cost is the word `Unknown` where the reference says "Auction
//! House".
//!
//! ## Three reads answer inside the call that made them
//!
//! `SetSendMailMoney`, `SetSendMailCOD` and `SelectStationery` are writes whose
//! result the interface reads back **in the next line of the same Lua body** —
//! `StaticPopupDialogs["SEND_MONEY"].OnAccept` is
//! `if ( SetSendMailMoney(...) ) then SendMailFrame_SendMail(); end`, and
//! `StationeryPopupButton_OnClick` calls `GetSelectedStationeryTexture()` two
//! lines after `SelectStationery(index)`. Recorded and drained a system later,
//! all three are a frame late and the first of them never sends the letter at
//! all. They are atomics for the reason
//! [`super::quest::Quests::selected`] is one: pure interface state, no packet,
//! and a `Resource` has to be `Sync`.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use vale_protocol::play::mail::{
    MailAction, MailFlags, MailHeader, MailResponse, MailResult, OutgoingMail, POSTAGE,
};
use vale_protocol::socket::session::MailVerb;
use vale_protocol::state::query::ItemInfo;

use super::super::combat::cursor::{Cursor, Held, Place};
use super::super::events::{
    CloseInboxItem, MailClosed, MailFailed, MailInboxUpdate, MailSendInfoUpdate, MailSendSuccess,
    MailShow, SendMailCodChanged, SendMailMoneyChanged, UpdatePendingMail,
};
use crate::world::session::{LocalPlayer, Session, WorldEntity};

/// One of the five mail packets, handed on from [`super::super::incoming`].
///
/// **The list is boxed** on the same terms the trainer's is: a mailbox holds up
/// to 254 headers of 60-odd bytes each and every other variant here is a word.
#[derive(Message, Debug, Clone)]
pub enum MailAnswer {
    List(Box<Vec<MailHeader>>),
    Result(MailResponse),
    Received,
    NextTime(f32),
    Text { id: u32, text: String },
}

/// The arm [`super::super::incoming`] calls, kept here so the five packet kinds
/// and the five window writes are one file apart rather than two.
pub fn answer_of(
    event: &vale_protocol::play::spells::PlayerEvent,
) -> Option<MailAnswer> {
    use vale_protocol::play::spells::PlayerEvent as E;
    Some(match event {
        E::MailList(list) => MailAnswer::List(list.clone()),
        E::MailResult(response) => MailAnswer::Result(*response),
        E::MailReceived => MailAnswer::Received,
        E::MailNextTime(seconds) => MailAnswer::NextTime(*seconds),
        E::ItemText { id, text } => MailAnswer::Text {
            id: *id,
            text: text.clone(),
        },
        _ => return None,
    })
}

/// **A letter attached to the draft**, and the two halves of what that means.
///
/// The guid is what goes on the wire; everything else is a copy taken at the
/// attach, for the same reason [`Held`]'s is — the bag square it came out of is
/// still full and stays so until the server answers, and re-reading it every
/// frame would empty the slot half-way through its own send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    /// Where it came from, so the cursor can be given it back.
    pub from: Place,
    /// The item object's own guid — the only field `CMSG_SEND_MAIL` sees.
    pub guid: u64,
    pub entry: u32,
    pub count: u32,
}

/// What is in the Send tab.
#[derive(Default)]
struct Draft {
    /// `Stationery.dbc` id, and **0 is what stops the letter being sent at
    /// all** — the client returns before it builds the packet. Answered
    /// immediately; see the module note.
    stationery: AtomicU32,
    /// `Package.dbc` id. The shipped table has one row and vmangos ignores the
    /// field; carried because the packet has a slot for it.
    package: AtomicU32,
    money: AtomicU32,
    cod: AtomicU32,
    item: Option<Attached>,
}

/// **Everything the mail window is made of.**
///
/// **`Default` is hand-written for one field**, and the test below is why:
/// [`Mailbox::pending`] is a countdown whose *zero* means "there is mail
/// waiting", so a derived default lights the envelope on the minimap the
/// moment the client starts. The reference initialises it to `-1.0` for
/// exactly that reason.
#[derive(Resource)]
pub struct Mailbox {
    /// The open mailbox's guid, or `None` for "the window is shut". See the
    /// module note: this is the whole of what "open" means.
    guid: Option<u64>,
    /// The inbox as the last `SMSG_MAIL_LIST_RESULT` stated it.
    inbox: Vec<MailHeader>,
    /// Bodies by `itemTextId`, cached for the session — a letter's words do not
    /// change while it is in the box.
    bodies: HashMap<u32, String>,
    /// …and the ids already asked about, so a text the server never answers is
    /// not asked for once a frame for ever. The same shape
    /// [`super::quest::Quests::asked`] has.
    asked_bodies: HashSet<u32>,
    /// **Which letter's words the interface last asked for**, by mail id.
    ///
    /// Written by a *read* — `GetInboxText` is where the reference sends both
    /// the text query and the mark-as-read — and drained by [`ask`]. 0 is
    /// nothing wanted.
    wanted_body: AtomicU32,
    /// Sender names, learned from the object manager's name cache.
    names: HashMap<u64, String>,
    /// Item templates for the parcels and for the stationery items, copied out
    /// of the session's cache on the same terms
    /// [`crate::game::character::items::Inventory`] copies its own: every read
    /// of this happens inside a scoped Lua call, where taking the world lock
    /// per `GetInboxItem` would put network latency on a hover.
    templates: HashMap<u32, ItemInfo>,
    /// When the inbox may next be asked for, on the app clock. `None` is "now".
    next_check: Option<f64>,
    /// **Seconds until the next letter**, counted down every frame and clamped
    /// at zero — see [`vale_protocol::play::mail::parse_next_mail_time`].
    /// Negative is "nothing waiting", which is what `-1.0` starts at.
    pending: f32,
    /// A letter is on the wire: the Send button is disabled until the result
    /// lands, which is `MAIL_FAILED`'s whole job.
    sending: AtomicBool,
    draft: Draft,
}

impl Mailbox {
    /// **Is the window open?** — which is the same question as "is a mailbox
    /// held", and the gate every verb takes.
    pub fn is_open(&self) -> bool {
        self.guid.is_some()
    }

    pub fn guid(&self) -> Option<u64> {
        self.guid
    }

    /// The inbox, in the order the server listed it. The interface's row
    /// numbers are positions in this, one-based.
    pub fn inbox(&self) -> &[MailHeader] {
        &self.inbox
    }

    /// One letter by the interface's **one-based** row.
    pub fn letter(&self, row: usize) -> Option<&MailHeader> {
        self.inbox.get(row.checked_sub(1)?)
    }

    /// The words of one letter, or `None` while the query is in flight — and
    /// **an empty string for a letter that has none**, which is what a letter
    /// with `itemTextId == 0` and no template is.
    pub fn body(&self, letter: &MailHeader) -> Option<&str> {
        if letter.item_text_id == 0 {
            return Some("");
        }
        self.bodies.get(&letter.item_text_id).map(String::as_str)
    }

    /// A sender's name, for the one kind of letter this client resolves — see
    /// the module note on the other three.
    pub fn sender_name(&self, letter: &MailHeader) -> Option<&str> {
        if !letter.kind.from_a_player() {
            return None;
        }
        self.names.get(&letter.sender_guid).map(String::as_str)
    }

    /// An item template this window has copied in, or `None` while its query is
    /// in flight.
    pub fn template(&self, entry: u32) -> Option<&ItemInfo> {
        self.templates.get(&entry)
    }

    /// `HasNewMail()` — **exactly zero**, which is the client's own
    /// `abs(x) <= epsilon` test and not "less than or equal".
    pub fn has_new_mail(&self) -> bool {
        self.pending == 0.0
    }

    /// The draft's currently selected `Stationery.dbc` id, 0 for none.
    pub fn stationery(&self) -> u32 {
        self.draft.stationery.load(Ordering::Relaxed)
    }

    /// …and the two amounts, which are mutually exclusive by the client's own
    /// gate rather than the server's.
    pub fn money(&self) -> u32 {
        self.draft.money.load(Ordering::Relaxed)
    }

    pub fn cod(&self) -> u32 {
        self.draft.cod.load(Ordering::Relaxed)
    }

    /// What is attached to the draft, if anything.
    pub fn attached(&self) -> Option<&Attached> {
        self.draft.item.as_ref()
    }

    /// **`SetSendMailMoney(amount)`, answered inside the call** — see the
    /// module note. The client's version is a purse test and then a store.
    ///
    /// `purse` is the character's own copper. `false` is the refusal the
    /// reference reports with message 0x25 and returns nil for, which is what
    /// stops `StaticPopupDialogs["SEND_MONEY"]` from sending.
    pub fn set_money(&self, amount: u32, purse: u32) -> bool {
        if amount > purse {
            return false;
        }
        self.draft.money.store(amount, Ordering::Relaxed);
        true
    }

    /// **`SetSendMailCOD(amount)`**, which stores **only when an
    /// item is attached** and silently does nothing otherwise.
    pub fn set_cod(&self, amount: u32) -> bool {
        if self.draft.item.is_none() {
            return false;
        }
        self.draft.cod.store(amount, Ordering::Relaxed);
        true
    }

    /// **`SelectStationery(id)`**, by the row's own `Stationery.dbc` id rather
    /// than by its position in the offered list — the interface passes a
    /// position and the panel converts, because the offered list moves when the
    /// bags do.
    pub fn select_stationery(&self, id: u32) {
        self.draft.stationery.store(id, Ordering::Relaxed);
    }

    pub fn select_package(&self, id: u32) {
        self.draft.package.store(id, Ordering::Relaxed);
    }

    /// **Record that a letter's words were asked for** — the one read in this
    /// subject with a side effect, and the reference has the same one. See
    /// [`Mailbox::wanted_body`].
    pub fn want_body(&self, mail_id: u32) {
        self.wanted_body.store(mail_id, Ordering::Relaxed);
    }

    /// Is a letter on the wire? The Send button's own gate.
    pub fn sending(&self) -> bool {
        self.sending.load(Ordering::Relaxed)
    }

    /// **Take the window down**, answering whether one was up — the same door
    /// [`super::trainer::TrainerWindow::close`] has, and called from the same
    /// two places: the interface's `CloseMail()` and the walk-away.
    fn close(&mut self) -> bool {
        if self.guid.take().is_none() {
            return false;
        }
        // **The draft goes with it**, as in the reference: every piece of the
        // draft is zeroed on close. An amount left behind would
        // be attached to the *next* letter, at the next mailbox.
        self.draft.stationery.store(0, Ordering::Relaxed);
        self.draft.package.store(0, Ordering::Relaxed);
        self.draft.money.store(0, Ordering::Relaxed);
        self.draft.cod.store(0, Ordering::Relaxed);
        self.draft.item = None;
        self.sending.store(false, Ordering::Relaxed);
        self.inbox.clear();
        self.next_check = None;
        true
    }
}

impl Default for Mailbox {
    fn default() -> Mailbox {
        Mailbox {
            guid: None,
            inbox: Vec::new(),
            bodies: HashMap::new(),
            asked_bodies: HashSet::new(),
            wanted_body: AtomicU32::new(0),
            names: HashMap::new(),
            templates: HashMap::new(),
            next_check: None,
            // **Not zero** — see the type's own note.
            pending: NOTHING_WAITING,
            sending: AtomicBool::new(false),
            draft: Draft::default(),
        }
    }
}

/// What the delivery countdown holds when there is nothing on its way —
/// `0xbf800000`, which is the reference's own initial value and is negative for
/// the same reason the server's `-86400` is: `HasNewMail()` tests for *exactly*
/// zero.
const NOTHING_WAITING: f32 = -1.0;

/// A press the interface made.
///
/// The three that answer inside their own call are **not** here — see the
/// module note; they are methods on [`Mailbox`].
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum MailPress {
    /// `CheckInbox()` — ask for the list, subject to the 60-second limit.
    CheckInbox,
    /// `CloseMail()` — local, like the gossip close.
    Close,
    /// `ClearSendMail()` — put the draft back the way it was found.
    ClearSend,
    /// `ClickSendMailItemButton()` — attach what is on the cursor, or take the
    /// attachment back onto it.
    ClickItem,
    /// `SendMail(name, subject, body)`.
    Send {
        to: String,
        subject: String,
        body: String,
    },
    /// `TakeInboxMoney(i)` — one-based rows throughout.
    TakeMoney(usize),
    /// `TakeInboxItem(i)` — **and the payer of a COD**, which the server takes
    /// on this packet.
    TakeItem(usize),
    /// `TakeInboxTextItem(i)` — keep the letter itself.
    TakeText(usize),
    /// `ReturnInboxItem(i)`.
    Return(usize),
    /// `DeleteInboxItem(i)`.
    Delete(usize),
}

pub struct MailPlugin;

impl Plugin for MailPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MailAnswer>()
            .add_message::<MailPress>()
            .init_resource::<Mailbox>()
            .add_systems(
                Update,
                (
                    open_at_a_mailbox,
                    announce,
                    tick_pending,
                    presses,
                    act,
                    ask,
                    learn,
                    out_of_range,
                )
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// **Click a mailbox, and the window opens here** — see the module note on why
/// there is no packet for it.
///
/// The use packet still goes out beside this, from [`super::object`], and
/// reaches the arm of `GameObject::Use` that does nothing. It is left alone
/// rather than special-cased: it costs one packet, it is what this client sends
/// at every other clickable object, and suppressing it would be a claim about
/// the 1.12.1 client's behaviour that has not been confirmed.
fn open_at_a_mailbox(
    mut clicked: MessageReader<super::object::ObjectUsed>,
    mut mailbox: ResMut<Mailbox>,
    mut shown: MessageWriter<MailShow>,
) {
    for used in clicked.read() {
        if used.kind != vale_assets::look::object::Kind::Mailbox {
            continue;
        }
        // **Re-clicking the same box does not re-open it**, which keeps the
        // rate limit meaningful: `MAIL_SHOW` ends in `CheckInbox()`.
        if mailbox.guid == Some(used.guid) {
            continue;
        }
        mailbox.close();
        mailbox.guid = Some(used.guid);
        shown.write(MailShow);
    }
}

/// Fold what the server said into the window, and tell the interface.
#[allow(clippy::too_many_arguments)]
fn announce(
    mut answers: MessageReader<MailAnswer>,
    mut mailbox: ResMut<Mailbox>,
    mut say: super::super::messages::Announce,
    mut inbox_updated: MessageWriter<MailInboxUpdate>,
    mut sent: MessageWriter<MailSendSuccess>,
    mut failed: MessageWriter<MailFailed>,
    mut money_changed: MessageWriter<SendMailMoneyChanged>,
    mut cod_changed: MessageWriter<SendMailCodChanged>,
    mut closed_item: MessageWriter<CloseInboxItem>,
    mut pending: MessageWriter<UpdatePendingMail>,
) {
    for answer in answers.read() {
        match answer {
            MailAnswer::List(list) => {
                mailbox.inbox = (**list).clone();
                inbox_updated.write(MailInboxUpdate);
            }
            // **One packet, seven verbs** — and the reference's own handler is
            // followed here arm for arm. See [`MailResult::key`].
            MailAnswer::Result(response) => {
                mailbox.sending.store(false, Ordering::Relaxed);
                let ok = response.result == MailResult::Ok;
                if !ok {
                    if let Some(key) = response.result.key() {
                        say.key(key);
                    }
                }
                match response.action {
                    MailAction::Send if ok => {
                        // **"Mail sent." — and it is yellow, centre screen.**
                        // Message 359, shown before the draft is cleared.
                        // `ERR_MAIL_SENT` is `UI_INFO_MESSAGE`, so the surface
                        // is the table's rather than this call's.
                        say.key(MailResult::SENT);
                        // **The whole draft, not the half that was spent** —
                        // the client zeroes the item, the money, the COD, the
                        // stationery *and* the package, and then raises three
                        // events. A stationery left selected is the shape the
                        // reference deliberately does not leave behind.
                        mailbox.draft.item = None;
                        mailbox.draft.money.store(0, Ordering::Relaxed);
                        mailbox.draft.cod.store(0, Ordering::Relaxed);
                        mailbox.draft.stationery.store(0, Ordering::Relaxed);
                        mailbox.draft.package.store(0, Ordering::Relaxed);
                        money_changed.write(SendMailMoneyChanged);
                        cod_changed.write(SendMailCodChanged);
                        sent.write(MailSendSuccess);
                    }
                    _ if ok => {
                        // Something changed about one letter and the wire says
                        // only which. Ask again rather than patch — see the
                        // module note — and close the open letter if it is gone.
                        if matches!(
                            response.action,
                            MailAction::Deleted | MailAction::ReturnedToSender
                        ) {
                            closed_item.write(CloseInboxItem(response.mail_id));
                        }
                        mailbox.next_check = None;
                    }
                    _ => {}
                }
                // **`MAIL_FAILED` on every result, a success included**, which
                // is the single most surprising line in the reference's handler:
                // the raise is past every arm of the switch and is not
                // conditional on anything. The name is misleading — its only
                // job is `SendMailMailButton:Enable()`, and
                // `SendMailMailButton_OnClick` ends in an unconditional
                // `this:Disable()`. Raised only on failure, as it was, the Send
                // button stays dead after the *first successful send* of a
                // session and nothing else ever turns it back on.
                failed.write(MailFailed);
            }
            // A letter has been delivered. The envelope on the minimap is
            // driven by the countdown, so ask for it again rather than guessing.
            MailAnswer::Received => {
                mailbox.pending = 0.0;
                pending.write(UpdatePendingMail);
                // …and, if a box is open, the list it is showing is stale.
                if mailbox.is_open() {
                    mailbox.next_check = None;
                }
            }
            MailAnswer::NextTime(seconds) => {
                mailbox.pending = *seconds;
                pending.write(UpdatePendingMail);
            }
            MailAnswer::Text { id, text } => {
                mailbox.bodies.insert(*id, text.clone());
                inbox_updated.write(MailInboxUpdate);
            }
        }
    }
}

/// **Count the delivery clock down**, which is the whole of what the minimap's
/// envelope is driven by — one subtraction a frame with a clamp at zero.
///
/// A negative value is "nothing waiting" and is left alone; without the clamp,
/// `HasNewMail()`'s exact-zero test would be crossed rather than landed on.
fn tick_pending(
    mut mailbox: ResMut<Mailbox>,
    time: Res<Time>,
    mut pending: MessageWriter<UpdatePendingMail>,
) {
    if mailbox.pending <= 0.0 {
        return;
    }
    mailbox.pending -= time.delta_secs();
    if mailbox.pending <= 0.0 {
        mailbox.pending = 0.0;
        pending.write(UpdatePendingMail);
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<MailPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_mail_presses() {
        out.write(press);
    }
}

/// …and act on it: two that stay here, and eight that leave.
#[allow(clippy::too_many_arguments)]
fn act(
    mut presses: MessageReader<MailPress>,
    mut mailbox: ResMut<Mailbox>,
    mut cursor: ResMut<Cursor>,
    inventory: Res<super::super::character::items::Inventory>,
    session: Res<Session>,
    time: Res<Time>,
    mut closed: MessageWriter<MailClosed>,
    mut info: MessageWriter<MailSendInfoUpdate>,
    mut money_changed: MessageWriter<SendMailMoneyChanged>,
    mut cod_changed: MessageWriter<SendMailCodChanged>,
    mut inbox_updated: MessageWriter<MailInboxUpdate>,
) {
    let now = time.elapsed_secs_f64();
    for press in presses.read() {
        let Some(mailbox_guid) = mailbox.guid else {
            // **Every verb needs a mailbox**, and `CloseMail` on a shut window
            // is what `MailFrame_OnEvent`'s own `MAIL_SHOW` arm sends when
            // `ShowUIPanel` declines. Nothing to do either way.
            continue;
        };
        let live = session.active.as_ref().map(|active| &active.live);
        match press {
            MailPress::CheckInbox => {
                // **The 60-second limit is the client's** — see the module
                // note. A refusal still redraws, because the panel called this
                // expecting the list to appear.
                if mailbox.next_check.is_some_and(|at| now < at) {
                    inbox_updated.write(MailInboxUpdate);
                    continue;
                }
                mailbox.next_check = Some(now + CHECK_INTERVAL);
                if let Some(live) = live {
                    live.mail(MailVerb::List(mailbox_guid));
                }
            }
            MailPress::Close => {
                // **Whatever was attached goes back to the bags**, which costs
                // nothing: the item never left them. The cursor is not given it
                // back — the window is gone and a pointer holding a letter's
                // parcel would be a drag nobody started.
                if mailbox.close() {
                    closed.write(MailClosed);
                }
            }
            MailPress::ClearSend => {
                mailbox.draft.item = None;
                mailbox.draft.money.store(0, Ordering::Relaxed);
                mailbox.draft.cod.store(0, Ordering::Relaxed);
                info.write(MailSendInfoUpdate);
                money_changed.write(SendMailMoneyChanged);
                cod_changed.write(SendMailCodChanged);
            }
            // **Attach, or take back** — the same one button both ways, which is
            // what `SendMailPackageButton_OnClick` is.
            MailPress::ClickItem => {
                match cursor.held.take() {
                    Some(Held::Item(item)) => {
                        // The guid is the only field the wire sees and the
                        // cursor does not carry one — it holds a *place*. See
                        // [`Attached`].
                        let guid = match item.from {
                            Place::Container { bag, slot } => inventory
                                .carried
                                .container_item(bag, usize::from(slot))
                                .map(|slot| slot.guid),
                            // **A worn item cannot be posted**, and the reference
                            // does not offer to: the send slot takes a bag
                            // square. Put it back on the cursor rather than
                            // swallowing the click.
                            Place::Inventory(_) => None,
                        };
                        match guid {
                            Some(guid) => {
                                mailbox.draft.item = Some(Attached {
                                    from: item.from,
                                    guid,
                                    entry: item.entry,
                                    count: item.split.map_or(item.count, u32::from),
                                });
                            }
                            None => cursor.held = Some(Held::Item(item)),
                        }
                    }
                    // A spell or a bar slot is not a parcel; hand it back.
                    Some(other) => cursor.held = Some(other),
                    // **Nothing on the cursor takes the attachment off**, which
                    // is the other half of the same button. The COD goes with
                    // it: `SetSendMailCOD` refuses without an item, so leaving
                    // one behind would send a COD on a letter with nothing in
                    // it.
                    None => {
                        if mailbox.draft.item.take().is_some() {
                            mailbox.draft.cod.store(0, Ordering::Relaxed);
                            cod_changed.write(SendMailCodChanged);
                        }
                    }
                }
                info.write(MailSendInfoUpdate);
            }
            MailPress::Send { to, subject, body } => {
                // **The client's own three gates**, all in `SendMail`: a
                // recipient, a stationery, and not both money and COD. Each of
                // them is a packet the server would refuse.
                let stationery = mailbox.stationery();
                let (money, cod) = (mailbox.money(), mailbox.cod());
                if to.is_empty() || stationery == 0 || (money != 0 && cod != 0) {
                    continue;
                }
                let Some(live) = live else { continue };
                mailbox.sending.store(true, Ordering::Relaxed);
                live.mail(MailVerb::Send(Box::new(OutgoingMail {
                    mailbox: mailbox_guid,
                    to: to.clone(),
                    subject: subject.clone(),
                    body: body.clone(),
                    stationery,
                    package: mailbox.draft.package.load(Ordering::Relaxed),
                    item: mailbox.draft.item.as_ref().map_or(0, |item| item.guid),
                    money,
                    cod,
                })));
            }
            // --- the five that name one letter ---
            MailPress::TakeMoney(row)
            | MailPress::TakeItem(row)
            | MailPress::TakeText(row)
            | MailPress::Return(row)
            | MailPress::Delete(row) => {
                let Some(letter) = mailbox.letter(*row) else {
                    continue;
                };
                let (mail_id, template_id) = (letter.id, letter.template_id);
                let Some(live) = live else { continue };
                live.mail(match press {
                    MailPress::TakeMoney(_) => MailVerb::TakeMoney {
                        mailbox: mailbox_guid,
                        mail_id,
                    },
                    MailPress::TakeItem(_) => MailVerb::TakeItem {
                        mailbox: mailbox_guid,
                        mail_id,
                    },
                    MailPress::TakeText(_) => MailVerb::TakeText {
                        mailbox: mailbox_guid,
                        mail_id,
                        template_id,
                    },
                    MailPress::Return(_) => MailVerb::Return {
                        mailbox: mailbox_guid,
                        mail_id,
                    },
                    _ => MailVerb::Delete {
                        mailbox: mailbox_guid,
                        mail_id,
                    },
                });
            }
        }
    }
}

/// **The client's own inbox rate limit**, in seconds.
const CHECK_INTERVAL: f64 = 60.0;

/// **Ask for what is missing**: a letter's words, a sender's name, and the item
/// templates the two panels draw with.
///
/// One system for three queues because all three take the same world lock, and
/// **the lock is only taken when something is missing** — the rule
/// `session::social::learn_names` states: after the first second of an open
/// window this is a scan of a short list against a mutex the session thread
/// writes forty times a second.
fn ask(
    mut mailbox: ResMut<Mailbox>,
    session: Res<Session>,
    mut inbox_updated: MessageWriter<MailInboxUpdate>,
) {
    // **The read that has a side effect**, drained here — see the module note.
    let wanted = mailbox.wanted_body.swap(0, Ordering::Relaxed);
    let letter = (wanted != 0)
        .then(|| mailbox.inbox.iter().find(|l| l.id == wanted).cloned())
        .flatten();
    let mut mark_read = None;
    if let Some(letter) = letter.as_ref() {
        // **The read flag is set locally**, because the packet has no answer —
        // see the module note.
        if !letter.flags.read() {
            mark_read = Some(letter.id);
        }
    }
    if let Some(mail_id) = mark_read {
        if let (Some(active), Some(guid)) = (session.active.as_ref(), mailbox.guid) {
            active.live.mail(MailVerb::MarkAsRead {
                mailbox: guid,
                mail_id,
            });
        }
        if let Some(row) = mailbox.inbox.iter_mut().find(|l| l.id == mail_id) {
            row.flags = MailFlags(row.flags.0 | MailFlags::READ);
            // **…and say so**, or the row stays yellow until something else
            // moves the list. The reference has no such gap: it sets the flag
            // inside `GetInboxText` and `InboxFrame_OnClick` redraws two lines
            // later, in the same call. Here the flag is set a system after the
            // read, so the redraw has to be asked for.
            inbox_updated.write(MailInboxUpdate);
        }
    }
    // **Only the letter that was opened.** Asking for every body would be one
    // packet per letter every time a mailbox is opened; the reference asks
    // lazily for exactly that reason, from inside `GetInboxText`.
    let Some(letter) = letter else {
        return;
    };
    if letter.item_text_id == 0
        || mailbox.bodies.contains_key(&letter.item_text_id)
        || !mailbox.asked_bodies.insert(letter.item_text_id)
    {
        return;
    }
    if let Some(active) = session.active.as_ref() {
        active.live.mail(MailVerb::TextQuery {
            item_text_id: letter.item_text_id,
            mail_id: letter.id,
        });
    }
}

/// **Copy in whatever the queries have answered** — the twin of
/// `session::social::learn_names`, and it takes the lock on the same terms.
///
/// Three populations, and the third is not on the wire at all: the five
/// stationery rows' own items, whose buy price is half of `GetSendMailPrice()`
/// and whose icon is the picture on every letter in the list. They are wanted
/// for as long as a mailbox is open, which is the only time anything reads them.
fn learn(
    mut mailbox: ResMut<Mailbox>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    mut inbox_updated: MessageWriter<MailInboxUpdate>,
) {
    if !mailbox.is_open() {
        return;
    }
    let stationery: Vec<u32> = assets
        .display_tables()
        .ok()
        .map(|tables| tables.mail().stationery().iter().map(|row| row.item).collect())
        .unwrap_or_default();
    let guids: Vec<u64> = mailbox
        .inbox
        .iter()
        .filter(|letter| letter.kind.from_a_player() && letter.sender_guid != 0)
        .map(|letter| letter.sender_guid)
        .filter(|guid| !mailbox.names.contains_key(guid))
        .collect();
    let entries: Vec<u32> = mailbox
        .inbox
        .iter()
        .filter_map(|letter| letter.item.map(|item| item.entry))
        .chain(mailbox.draft.item.as_ref().map(|item| item.entry))
        .chain(stationery)
        .filter(|entry| *entry != 0 && !mailbox.templates.contains_key(entry))
        .collect();
    // **The lock is only taken when something is missing**, which after the
    // first second of an open window is never.
    if guids.is_empty() && entries.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(mut world) = active.live.world().lock() else {
        return;
    };
    let mut moved = false;
    for guid in guids {
        match world.players.get(&guid) {
            Some(info) => {
                mailbox.names.insert(guid, info.name.clone());
                moved = true;
            }
            // …and if it has not answered yet, get in the queue the friends
            // list uses.
            None => world.want_social_guid(guid),
        }
    }
    for entry in entries {
        match world.items.get(&entry) {
            Some(info) => {
                mailbox.templates.insert(entry, info.clone());
                moved = true;
            }
            None => world.want_item(entry),
        }
    }
    if moved {
        inbox_updated.write(MailInboxUpdate);
    }
}

/// **Walk away and the window shuts** — the game object's own version of
/// `gossip::out_of_range`, and it exists for the same reason: the server checks
/// the range on every one of the nine packets
/// (`WorldSession::CheckMailBox` → `GetGameObjectIfCanInteractWith`), and
/// nothing announces the walk-away.
///
/// The reference does this by guid rather than by distance — it shuts
/// the window when the object the guid names leaves the client's world — and
/// both halves are here, because a mailbox in a city does not despawn and a
/// player walking out of the inn is the ordinary case.
fn out_of_range(
    mut mailbox: ResMut<Mailbox>,
    player: Query<&Transform, With<LocalPlayer>>,
    objects: Query<(&WorldEntity, &Transform)>,
    mut closed: MessageWriter<MailClosed>,
) {
    let Some(guid) = mailbox.guid else {
        return;
    };
    let Ok(me) = player.single() else {
        return;
    };
    let here = objects
        .iter()
        .find(|(entity, _)| entity.guid == guid)
        .map(|(_, transform)| transform.translation);
    // **Unplaced is a hold and despawned is a close** — the same three-way the
    // NPC windows make, for the same reason: an object streams in a frame after
    // its guid does.
    let gone = match here {
        Some(there) => {
            me.translation.distance(there)
                > vale_protocol::play::gossip::INTERACTION_DISTANCE
        }
        None => false,
    };
    if gone && mailbox.close() {
        closed.write(MailClosed);
    }
}

/// **The postage on the draft** — `GetSendMailPrice()`.
///
/// 30 copper, plus the selected stationery's own item price **unless the
/// character is already carrying one**. A free function because the rule is the
/// interesting part and it can be checked with no session: see the test below.
pub fn postage(item_price: Option<u32>) -> u32 {
    POSTAGE + item_price.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::mail::MailKind;

    fn letter(id: u32, guid: u64) -> MailHeader {
        MailHeader {
            id,
            kind: MailKind::Normal,
            sender_guid: guid,
            subject: "Hello".into(),
            stationery: 41,
            ..Default::default()
        }
    }

    /// **Money is refused above the purse and the refusal is what stops the
    /// send** — see the module note: this answer is read inside the same Lua
    /// body that asked for it.
    #[test]
    fn money_is_gated_on_the_purse_and_answers_at_once() {
        let mailbox = Mailbox::default();
        assert!(!mailbox.set_money(500, 100));
        assert_eq!(mailbox.money(), 0, "a refusal stores nothing");
        assert!(mailbox.set_money(100, 100), "exactly the purse is allowed");
        assert_eq!(mailbox.money(), 100);
    }

    /// **A COD with no parcel is refused**, which is the client's first check
    /// — and the reason is not tidiness: the server takes a COD
    /// when the *item* is taken, so a COD on an empty letter is money nobody
    /// can pay.
    #[test]
    fn a_cod_needs_something_to_be_collected_on() {
        let mut mailbox = Mailbox::default();
        assert!(!mailbox.set_cod(100));
        assert_eq!(mailbox.cod(), 0);
        mailbox.draft.item = Some(Attached {
            from: Place::Container { bag: 0, slot: 1 },
            guid: 0x99,
            entry: 6948,
            count: 1,
        });
        assert!(mailbox.set_cod(100));
        assert_eq!(mailbox.cod(), 100);
    }

    /// **Closing empties the draft**, as in the reference — an amount left
    /// behind would be attached to the next letter at the next mailbox.
    #[test]
    fn closing_the_window_empties_the_draft_and_the_inbox() {
        let mut mailbox = Mailbox::default();
        mailbox.guid = Some(0x77);
        mailbox.inbox.push(letter(1, 0xAA));
        mailbox.select_stationery(41);
        assert!(mailbox.set_money(50, 1000));
        assert!(mailbox.close());
        assert!(!mailbox.is_open());
        assert_eq!(mailbox.stationery(), 0);
        assert_eq!(mailbox.money(), 0);
        assert!(mailbox.inbox().is_empty());
        assert!(!mailbox.close(), "closing a shut window is not an event");
    }

    /// **`HasNewMail()` is exactly zero**, not "at most zero" — the client's
    /// own `abs(x) <= epsilon`, and the reason a `-86400` from the server reads
    /// as "nothing" rather than as "overdue".
    #[test]
    fn new_mail_is_the_countdown_at_exactly_zero() {
        let mut mailbox = Mailbox::default();
        assert!(!mailbox.has_new_mail(), "the default is nothing waiting");
        mailbox.pending = -86400.0;
        assert!(!mailbox.has_new_mail());
        mailbox.pending = 12.0;
        assert!(!mailbox.has_new_mail());
        mailbox.pending = 0.0;
        assert!(mailbox.has_new_mail());
    }

    /// A letter with no words is an empty body rather than a query that never
    /// answers — which is what stops `OpenMailBodyText` sitting blank for ever
    /// on an auction notice.
    #[test]
    fn a_letter_with_no_text_id_answers_at_once() {
        let mut mailbox = Mailbox::default();
        let mut plain = letter(1, 0xAA);
        assert_eq!(mailbox.body(&plain), Some(""));
        plain.item_text_id = 7;
        assert_eq!(mailbox.body(&plain), None, "in flight");
        mailbox.bodies.insert(7, "Dear friend".into());
        assert_eq!(mailbox.body(&plain), Some("Dear friend"));
    }

    /// **Only a player's letter gets a name**, and the other three kinds are
    /// answered as nothing — a stated absence; see the module note.
    #[test]
    fn only_a_players_letter_resolves_to_a_name() {
        let mut mailbox = Mailbox::default();
        mailbox.names.insert(0xAA, "Bram".into());
        let from_a_player = letter(1, 0xAA);
        assert_eq!(mailbox.sender_name(&from_a_player), Some("Bram"));
        let from_the_auction_house = MailHeader {
            kind: MailKind::Auction,
            sender_entry: 2,
            ..letter(2, 0)
        };
        assert_eq!(mailbox.sender_name(&from_the_auction_house), None);
    }

    /// The postage rule, which is a constant plus a conditional price.
    #[test]
    fn postage_is_thirty_plus_the_paper_you_do_not_own() {
        assert_eq!(postage(None), 30, "carrying the stationery is free paper");
        assert_eq!(postage(Some(120)), 150);
    }
}
