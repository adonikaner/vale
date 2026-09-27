//! **The C functions `MailFrame.lua` calls** — sixteen reads and fourteen
//! writes, over three panels that share one packet family.
//!
//! ```text
//! the inbox      GetInboxNumItems()          GetInboxHeaderInfo(i)  — thirteen
//!                GetInboxText(i)             GetInboxItem(i)
//!                GetInboxInvoiceInfo(i)      InboxItemCanDelete(i)
//! the letter     TakeInboxMoney(i)  TakeInboxItem(i)  TakeInboxTextItem(i)
//!                ReturnInboxItem(i) DeleteInboxItem(i)
//! the draft      GetSendMailItem()  GetSendMailPrice()  GetSendMailMoney()
//!                GetSendMailCOD()   SetSendMailMoney(n) SetSendMailCOD(n)
//!                ClickSendMailItemButton()   ClearSendMail()
//!                SendMail(name, subject, body)
//! the paper      GetNumStationeries()  GetStationeryInfo(i)  SelectStationery(i)
//!                GetSelectedStationeryTexture()
//!                GetNumPackages()  GetPackageInfo(i)  SelectPackage(i)
//! the box        CheckInbox()  CloseMail()  HasNewMail()
//! ```
//!
//! The split every panel keeps: the frame is the game's own `MailFrame.xml`,
//! the wire is [`vale_protocol::play::mail`], the paper table's own rule is
//! [`vale_assets::tables::stationery`], the window is
//! [`crate::interface::mail`], and this file is registration and arguments.
//!
//! ## `GetInboxHeaderInfo` returns thirteen and the last two are not
//! independent
//!
//! ```lua
//! packageIcon, stationeryIcon, sender, subject, money, CODAmount, daysLeft,
//!   hasItem, wasRead, wasReturned, textCreated, canReply, isGM
//! ```
//!
//! `canReply` is "from a player, and not a GM's" and `isGM` is "stationery 61";
//! the client tests them in that order and **pushes `isGM` only when `canReply`
//! is nil**, so the reference itself returns twelve values in the ordinary case
//! while claiming thirteen. That quirk is not reproduced — thirteen are always
//! pushed here — because nothing in `MailFrame.lua` can tell the difference: it
//! reads `isGM` only in the branch where the sender is not a player.
//!
//! **`sender` is nil-able and that is what the interface tests.**
//! `InboxFrame_Update` is `if ( not sender ) then sender = UNKNOWN`, so an
//! empty string here would draw a nameless row instead of "Unknown" — the same
//! shape `GetRewardSpell`'s zero-versus-nil had.
//!
//! ## Three of the writes answer inside their own call
//!
//! `SetSendMailMoney`, `SetSendMailCOD` and `SelectStationery` — see
//! [`crate::interface::mail`]'s module note, which carries the two Lua bodies
//! that read them back on the next line. They are registered here among the
//! reads for the same reason `SelectQuestLogEntry` and `SelectTrainerService`
//! are.
//!
//! ## The invoice is answered as "not one", and it is a stated absence
//!
//! `GetInboxInvoiceInfo` describes an auction-house receipt, which the
//! reference parses out of the letter's own body text into a client-side
//! structure, set while the body is read. This client has no
//! auction house, so the fourth answer of `GetInboxText` is always nil,
//! `OpenMailInvoiceFrame` stays hidden, and this function answers nothing. What
//! that costs on a server with an auction house is a receipt drawn as an
//! ordinary letter — the words are still there, the arithmetic panel is not.

use super::super::api::{one_or_nil, Answers};

/// One row of the inbox, as `GetInboxHeaderInfo` answers it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InboxRow {
    /// `Package.dbc`'s icon, and **nil is the ordinary case**: vmangos writes 0
    /// in the package field for every letter it sends, so this is the wrapping
    /// on a parcel the interface would draw instead of the paper.
    pub package_icon: Option<String>,
    /// `Stationery.dbc`'s row's own *item*'s icon — see
    /// [`vale_assets::tables::stationery`], where the two columns are told
    /// apart. `None` until that item's template answers.
    pub stationery_icon: Option<String>,
    /// **`None` is what draws `UNKNOWN`** — see the module note.
    pub sender: Option<String>,
    /// Already rewritten through `COD_PAYMENT` when the letter is one; see
    /// [`InboxRow::subject`]'s writer in the `Live` impl.
    pub subject: String,
    pub money: u32,
    pub cod: u32,
    /// **Days**, and the interface formats under one day as a countdown.
    pub days_left: f32,
    pub has_item: bool,
    pub was_read: bool,
    pub was_returned: bool,
    pub text_created: bool,
    pub can_reply: bool,
    pub is_gm: bool,
}

/// What `GetInboxText` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InboxText {
    /// The words, or `None` while the query is in flight — which draws an empty
    /// letter for one round trip rather than a stale one.
    pub body: Option<String>,
    /// The stationery's background stem, without the `1`/`2` the interface
    /// appends.
    pub texture: Option<String>,
    /// Whether the letter itself can be kept as an item.
    pub takeable: bool,
}

/// One item line — the parcel in a letter, or the one attached to the draft.
///
/// The same five answers `GetInboxItem` and `GetSendMailItem` both give, which
/// is why they are one shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailItemLine {
    pub name: String,
    pub texture: Option<String>,
    pub count: u32,
    pub quality: u32,
    /// `canUse` — which greys the button red. Always true here: what decides it
    /// is the item's class and race masks against the character's, and this
    /// client does not test those for something it has not been given yet. The
    /// same stated approximation the quest reward buttons make.
    pub usable: bool,
}

/// One row of the stationery popup, as `GetStationeryInfo` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StationeryLine {
    /// The row's own **id**, which is what `SelectStationery` is given here —
    /// the interface passes a position and this file converts, because the
    /// offered list moves when the bags do.
    pub id: u32,
    /// The item's name, or empty while its template is in flight.
    pub name: String,
    /// The item's icon.
    pub texture: Option<String>,
    /// **Copper, or `None` for free** — `StationeryPopupFrame_Update` tests
    /// `if ( cost )` and draws a zero money frame otherwise, so nil and 0 are
    /// two different rows on screen.
    pub cost: Option<u32>,
}

/// **The reads this module registers**, for the count that measures the gap.
///
/// **The three writes that answer inside their own call are in this list**, for
/// the reason `SelectQuestLogEntry` is in the quest panel's: they are
/// registered among the reads because they must be, and this array is checked
/// against what the scope really holds rather than against what the file is
/// about. See `api`'s own `the_list_and_the_registration_are_the_same_set`.
pub const READS: [&str; 20] = [
    "GetInboxHeaderInfo",
    "GetInboxInvoiceInfo",
    "GetInboxItem",
    "GetInboxNumItems",
    "GetInboxText",
    "GetNumPackages",
    "GetNumStationeries",
    "GetPackageInfo",
    "GetSelectedStationeryTexture",
    "GetSendMailCOD",
    "GetSendMailItem",
    "GetSendMailMoney",
    "GetSendMailPrice",
    "GetStationeryInfo",
    "HasNewMail",
    "InboxItemCanDelete",
    "SelectPackage",
    "SelectStationery",
    "SetSendMailCOD",
    "SetSendMailMoney",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "GetInboxNumItems",
        scope.create_function(move |_, ()| Ok(answers.mail_count()))?,
    )?;
    globals.set(
        "HasNewMail",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.mail_has_new())))?,
    )?;
    globals.set(
        "GetInboxHeaderInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let row = index(n).and_then(|row| answers.mail_row(row)).unwrap_or_default();
            // Thirteen, always — see the module note on the reference's own
            // twelve.
            Ok((
                row.package_icon,
                row.stationery_icon,
                row.sender,
                row.subject,
                row.money,
                row.cod,
                row.days_left,
                one_or_nil(row.has_item),
                one_or_nil(row.was_read),
                one_or_nil(row.was_returned),
                one_or_nil(row.text_created),
                one_or_nil(row.can_reply),
                one_or_nil(row.is_gm),
            ))
        })?,
    )?;
    globals.set(
        "GetInboxText",
        scope.create_function(move |_, n: Option<i64>| {
            let Some(row) = index(n) else {
                return Ok((None, None, mlua::Value::Nil, mlua::Value::Nil));
            };
            // **The read with a side effect** — the reference sends both
            // `CMSG_ITEM_TEXT_QUERY` and `CMSG_MAIL_MARK_AS_READ` from inside
            // this function, and there is nowhere else the
            // interface gives it the chance: `InboxFrame_OnClick` sets a Lua
            // field and calls `OpenMail_Update`, which calls this.
            answers.mail_open(row);
            let text = answers.mail_text(row);
            Ok((
                text.body,
                text.texture,
                one_or_nil(text.takeable),
                // **isInvoice — always nil**; see the module note.
                mlua::Value::Nil,
            ))
        })?,
    )?;
    globals.set(
        "GetInboxItem",
        scope.create_function(move |_, n: Option<i64>| {
            let line = index(n).and_then(|row| answers.mail_item(row));
            Ok(item_answer(line))
        })?,
    )?;
    // **Seven nils, and seven of them rather than one** — `OpenMail_Update`
    // assigns all seven at once and then tests `if ( playerName )`, so a single
    // nil would leave the other six holding whatever the last call left there.
    globals.set(
        "GetInboxInvoiceInfo",
        scope.create_function(move |_, _n: Option<i64>| {
            Ok((
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Nil,
            ))
        })?,
    )?;
    globals.set(
        "InboxItemCanDelete",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(one_or_nil(
                index(n).is_some_and(|row| answers.mail_can_delete(row)),
            ))
        })?,
    )?;

    // --- the draft ---

    globals.set(
        "GetSendMailItem",
        scope.create_function(move |_, ()| Ok(item_answer(answers.mail_send_item())))?,
    )?;
    globals.set(
        "GetSendMailPrice",
        scope.create_function(move |_, ()| Ok(answers.mail_send_price()))?,
    )?;
    globals.set(
        "GetSendMailMoney",
        scope.create_function(move |_, ()| Ok(answers.mail_send_money()))?,
    )?;
    globals.set(
        "GetSendMailCOD",
        scope.create_function(move |_, ()| Ok(answers.mail_send_cod()))?,
    )?;
    // **Both setters answer inside the call**, and the money one's answer is
    // what decides whether the letter is sent at all:
    // `if ( SetSendMailMoney(...) ) then SendMailFrame_SendMail(); end`.
    globals.set(
        "SetSendMailMoney",
        scope.create_function(move |_, copper: Option<i64>| {
            Ok(one_or_nil(answers.mail_set_money(copper_of(copper))))
        })?,
    )?;
    globals.set(
        "SetSendMailCOD",
        scope.create_function(move |_, copper: Option<i64>| {
            // **The client's returns nothing at all**, unlike its money twin —
            // `SendMailMailButton_OnClick` calls it and ignores the result.
            answers.mail_set_cod(copper_of(copper));
            Ok(())
        })?,
    )?;

    // --- the paper ---

    globals.set(
        "GetNumStationeries",
        scope.create_function(move |_, ()| Ok(answers.mail_stationery().len()))?,
    )?;
    globals.set(
        "GetStationeryInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let row = index(n)
                .and_then(|row| answers.mail_stationery().into_iter().nth(row - 1))
                .unwrap_or_default();
            Ok((row.name, row.texture, row.cost))
        })?,
    )?;
    globals.set(
        "SelectStationery",
        scope.create_function(move |_, n: Option<i64>| {
            // The interface's index is a position in the *offered* list; the
            // window stores the row's own id. See [`StationeryLine::id`].
            if let Some(id) = index(n)
                .and_then(|row| answers.mail_stationery().into_iter().nth(row - 1))
                .map(|row| row.id)
            {
                answers.mail_select_stationery(id);
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "GetSelectedStationeryTexture",
        scope.create_function(move |_, ()| Ok(answers.mail_selected_stationery()))?,
    )?;
    // **The packages are answered as none**, which is the shipped table's own
    // state seen from the wire: `Package.dbc` has one row, vmangos writes 0 into
    // the package field of every letter it sends, and nothing in 1.12 selects
    // one. `StationeryPopupFrame` is the only consumer and it never opens on
    // packages. Registered rather than left absent because a missing name aborts
    // the `OnLoad` it appears in.
    globals.set(
        "GetNumPackages",
        scope.create_function(move |_, ()| Ok(0_usize))?,
    )?;
    globals.set(
        "GetPackageInfo",
        scope.create_function(move |_, _n: Option<i64>| {
            Ok((mlua::Value::Nil, mlua::Value::Nil, mlua::Value::Nil))
        })?,
    )?;
    globals.set(
        "SelectPackage",
        scope.create_function(move |_, _n: Option<i64>| Ok(()))?,
    )?;
    Ok(())
}

/// The five values both item reads push. **A nil name is what the interface
/// tests** — `OpenMail_Update` is `if ( name ) then … else … end` and
/// `SendMailFrame_Update` the same, so an empty string draws an enabled button
/// over nothing.
fn item_answer(
    line: Option<MailItemLine>,
) -> (Option<String>, Option<String>, u32, u32, mlua::Value) {
    match line {
        // **`stackCount` is 0 rather than nil for an empty slot**:
        // `SendMailFrame_Update`'s next line is `if ( stackCount <= 1 )`, which
        // is an arithmetic error on nil and takes the whole draft panel down.
        None => (None, None, 0, 0, mlua::Value::Nil),
        Some(line) => (
            Some(line.name),
            line.texture,
            line.count,
            line.quality,
            one_or_nil(line.usable),
        ),
    }
}

/// A one-based Lua index, or `None` for anything else.
fn index(n: Option<i64>) -> Option<usize> {
    usize::try_from(n?).ok().filter(|row| *row > 0)
}

/// A copper amount off the money input frame. **Negative and absent are both
/// zero**, which is the safe direction: the alternative is a wrapped `u32` of
/// four billion copper going out on the wire.
fn copper_of(n: Option<i64>) -> u32 {
    u32::try_from(n.unwrap_or(0)).unwrap_or(0)
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::mail::MailPress>>>;

/// Register the ten writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::mail::MailPress as P;
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = std::rc::Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(press) = $body {
                    queue.borrow_mut().push(press);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    push!("CheckInbox", (), |_a| Some(P::CheckInbox));
    push!("CloseMail", (), |_a| Some(P::Close));
    push!("ClearSendMail", (), |_a| Some(P::ClearSend));
    push!("ClickSendMailItemButton", (), |_a| Some(P::ClickItem));
    // **Three strings, and every one of them may be empty.** The window's own
    // gate is in [`crate::interface::mail`] rather than here: this file does not
    // know whether a stationery is selected.
    push!(
        "SendMail",
        (Option<String>, Option<String>, Option<String>),
        |args| {
            let (to, subject, body) = args;
            Some(P::Send {
                to: to.unwrap_or_default(),
                subject: subject.unwrap_or_default(),
                body: body.unwrap_or_default(),
            })
        }
    );
    push!("TakeInboxMoney", Option<i64>, |n| index(n).map(P::TakeMoney));
    push!("TakeInboxItem", Option<i64>, |n| index(n).map(P::TakeItem));
    push!("TakeInboxTextItem", Option<i64>, |n| index(n).map(P::TakeText));
    push!("ReturnInboxItem", Option<i64>, |n| index(n).map(P::Return));
    push!("DeleteInboxItem", Option<i64>, |n| index(n).map(P::Delete));
    Ok(())
}

/// **What the interface may ask about the mailbox.**
///
/// Here rather than in [`super::super::api`] for the reason every other panel's
/// is: a read's four pieces — this declaration, the answer below it, the
/// registration further up this file and the name in [`READS`] — belong in the
/// file the subject is named after.
///
/// Every index that crosses this boundary is the interface's own, **one-based**,
/// and it is a position in the inbox rather than a mail id. The two are
/// deliberately never mixed: the wire names a letter by id and the panel names
/// it by row, and the conversion happens once, in
/// [`crate::interface::mail::Mailbox::letter`].
pub trait MailAnswers {
    /// `GetInboxNumItems()`.
    fn mail_count(&self) -> usize;
    /// One row of the inbox, or `None` past the end.
    fn mail_row(&self, row: usize) -> Option<InboxRow>;
    /// `GetInboxText(i)`'s first three answers.
    fn mail_text(&self, row: usize) -> InboxText;
    /// **…and its side effect** — see [`install`]: the reference sends the text
    /// query and the mark-as-read from inside that read.
    fn mail_open(&self, row: usize);
    /// `GetInboxItem(i)` — the parcel, or `None` for a letter with none.
    fn mail_item(&self, row: usize) -> Option<MailItemLine>;
    /// `InboxItemCanDelete(i)` — the *word on the button*, not a permission.
    fn mail_can_delete(&self, row: usize) -> bool;
    /// `HasNewMail()`.
    fn mail_has_new(&self) -> bool;
    /// `GetNumStationeries()` / `GetStationeryInfo(i)` — which of the five rows
    /// this character is offered; see
    /// [`vale_assets::tables::stationery::MailTables::offered`].
    fn mail_stationery(&self) -> Vec<StationeryLine>;
    /// `GetSelectedStationeryTexture()` — the background stem.
    fn mail_selected_stationery(&self) -> Option<String>;
    /// `SelectStationery(i)` — **a write, and it takes effect at once**; by the
    /// row's own id rather than by its position.
    fn mail_select_stationery(&self, id: u32);
    /// `GetSendMailItem()`.
    fn mail_send_item(&self) -> Option<MailItemLine>;
    /// `GetSendMailPrice()` — 30 copper plus the paper.
    fn mail_send_price(&self) -> u32;
    /// `GetSendMailMoney()` / `GetSendMailCOD()`.
    fn mail_send_money(&self) -> u32;
    fn mail_send_cod(&self) -> u32;
    /// `SetSendMailMoney(n)` — **a write whose answer decides whether the letter
    /// is sent**; `false` is "you do not have that much".
    fn mail_set_money(&self, copper: u32) -> bool;
    /// `SetSendMailCOD(n)` — a write, and one that silently does nothing when
    /// there is no parcel to collect on.
    fn mail_set_cod(&self, copper: u32);
    /// `GameTooltip:SetInboxItem(i)` — the **entry** of the parcel in one
    /// letter, for the plate. By entry rather than by link because 1.12 ships
    /// no `GetInboxItemLink`, which is the same asymmetry the buyback tab has.
    fn mail_item_entry(&self, row: usize) -> Option<u32>;
    /// …and `GameTooltip:SetSendMailItem()`, which takes no argument because
    /// there is only ever one thing attached.
    fn mail_send_entry(&self) -> Option<u32>;
}

impl MailAnswers for super::super::api::Live<'_, '_, '_> {
    fn mail_count(&self) -> usize {
        self.mail.inbox().len()
    }

    fn mail_row(&self, row: usize) -> Option<InboxRow> {
        let letter = self.mail.letter(row)?;
        let tables = self.tables.as_ref();
        let stationery = tables.and_then(|t| t.mail().stationery_by_id(letter.stationery));
        Some(InboxRow {
            package_icon: tables
                .and_then(|t| t.mail().package_icon(letter.package))
                .map(str::to_string),
            // **The row's own item's icon, not the row's texture column** — see
            // [`vale_assets::tables::stationery`], where the two are told
            // apart, and note that this is `None` for one round trip after the
            // window opens rather than an empty string.
            stationery_icon: stationery
                .and_then(|row| self.mail.template(row.item))
                .and_then(|item| tables.and_then(|t| t.item_icon(item.display_id))),
            sender: self.mail.sender_name(letter).map(str::to_string),
            // **A COD letter's subject is rewritten before it is drawn** —
            // the client builds `format(COD_PAYMENT, subject)`, and without it a
            // parcel wanting payment reads exactly like one that does not.
            subject: if letter.flags.cod_payment() {
                self.strings
                    .and_then(|strings| strings.get("COD_PAYMENT"))
                    .map_or_else(
                        || letter.subject.clone(),
                        |line| line.replacen("%s", &letter.subject, 1),
                    )
            } else {
                letter.subject.clone()
            },
            money: letter.money,
            cod: letter.cod,
            days_left: letter.days_left,
            has_item: letter.item.is_some(),
            was_read: letter.flags.read(),
            was_returned: letter.flags.returned(),
            text_created: letter.flags.copied(),
            can_reply: letter.kind.from_a_player() && !letter.is_gm(),
            is_gm: letter.is_gm(),
        })
    }

    fn mail_text(&self, row: usize) -> InboxText {
        let Some(letter) = self.mail.letter(row) else {
            return InboxText::default();
        };
        // **A templated letter's words are in the client's own files**, which is
        // `GetInboxText`'s second source: a letter with no
        // `itemTextId` falls through to `MailTemplate.dbc`.
        let templated = (letter.item_text_id == 0 && letter.template_id != 0)
            .then(|| {
                self.tables
                    .as_ref()
                    .and_then(|t| t.mail().template(letter.template_id))
                    .map(|row| row.body.clone())
            })
            .flatten();
        InboxText {
            body: templated.or_else(|| self.mail.body(letter).map(str::to_string)),
            texture: self
                .tables
                .as_ref()
                .and_then(|t| t.mail().stationery_by_id(letter.stationery))
                .map(|row| row.texture.clone()),
            takeable: letter.text_is_takeable(),
        }
    }

    fn mail_open(&self, row: usize) {
        if let Some(letter) = self.mail.letter(row) {
            self.mail.want_body(letter.id);
        }
    }

    fn mail_item(&self, row: usize) -> Option<MailItemLine> {
        let attachment = self.mail.letter(row)?.item?;
        Some(self.mail_line(attachment.entry, u32::from(attachment.count)))
    }

    fn mail_can_delete(&self, row: usize) -> bool {
        self.mail.letter(row).is_some_and(|letter| letter.can_delete())
    }

    fn mail_has_new(&self) -> bool {
        self.mail.has_new_mail()
    }

    fn mail_stationery(&self) -> Vec<StationeryLine> {
        let Some(tables) = self.tables.as_ref() else {
            return Vec::new();
        };
        let carried = |entry: u32| self.inventory.carried.find_entry(entry).is_some();
        tables
            .mail()
            .offered(carried)
            .into_iter()
            .map(|row| {
                let template = self.mail.template(row.item);
                StationeryLine {
                    id: row.id,
                    name: template.map(|item| item.name.clone()).unwrap_or_default(),
                    texture: template.and_then(|item| tables.item_icon(item.display_id)),
                    // **nil is free paper and 0 is a free *purchase***, which
                    // are two different rows on screen —
                    // `StationeryPopupFrame_Update` tests `if ( cost )`. The
                    // one this client answers nil for is paper the character
                    // already holds.
                    cost: (!carried(row.item))
                        .then(|| template.map(|item| item.buy_price))
                        .flatten()
                        .filter(|price| *price > 0),
                }
            })
            .collect()
    }

    fn mail_selected_stationery(&self) -> Option<String> {
        let id = self.mail.stationery();
        self.tables
            .as_ref()?
            .mail()
            .stationery_by_id(id)
            .map(|row| row.texture.clone())
    }

    fn mail_select_stationery(&self, id: u32) {
        self.mail.select_stationery(id);
    }

    fn mail_send_item(&self) -> Option<MailItemLine> {
        let attached = self.mail.attached()?;
        Some(self.mail_line(attached.entry, attached.count))
    }

    fn mail_send_price(&self) -> u32 {
        let id = self.mail.stationery();
        let price = (|| {
            let row = self.tables.as_ref()?.mail().stationery_by_id(id)?;
            // **Carrying the paper makes it free** — the client skips the
            // lookup entirely when the row's own item guid is set.
            if self.inventory.carried.find_entry(row.item).is_some() {
                return None;
            }
            Some(self.mail.template(row.item)?.buy_price)
        })();
        crate::interface::mail::postage(price)
    }

    fn mail_send_money(&self) -> u32 {
        self.mail.money()
    }

    fn mail_send_cod(&self) -> u32 {
        self.mail.cod()
    }

    fn mail_set_money(&self, copper: u32) -> bool {
        self.mail.set_money(copper, self.inventory.money)
    }

    fn mail_set_cod(&self, copper: u32) {
        self.mail.set_cod(copper);
    }

    fn mail_item_entry(&self, row: usize) -> Option<u32> {
        Some(self.mail.letter(row)?.item?.entry)
    }

    fn mail_send_entry(&self) -> Option<u32> {
        Some(self.mail.attached()?.entry)
    }
}

impl super::super::api::Live<'_, '_, '_> {
    /// One item line off a template that may not have arrived — the shape both
    /// item reads share.
    ///
    /// **An unresolved entry still draws a button**, with the interface's own
    /// `UNKNOWN` on it and the placeholder icon: a nil name here would make
    /// `OpenMail_Update` hide the parcel a letter demonstrably has.
    fn mail_line(&self, entry: u32, count: u32) -> MailItemLine {
        let template = self.mail.template(entry);
        MailItemLine {
            name: template.map(|item| item.name.clone()).unwrap_or_else(|| {
                self.strings
                    .and_then(|strings| strings.get("UNKNOWN"))
                    .map_or_else(|| "Unknown".to_string(), str::to_string)
            }),
            texture: template
                .and_then(|item| {
                    self.tables
                        .as_ref()
                        .and_then(|t| t.item_icon(item.display_id))
                })
                .or_else(|| Some(super::loot::UNKNOWN_ICON.to_string())),
            count,
            quality: template.map_or(0, |item| item.quality),
            usable: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// One-based in, and everything else is nothing.
    #[test]
    fn a_lua_index_is_one_based_and_anything_else_is_nothing() {
        assert_eq!(index(Some(1)), Some(1));
        assert_eq!(index(Some(0)), None);
        assert_eq!(index(Some(-3)), None);
        assert_eq!(index(None), None);
    }

    /// **A negative amount is zero rather than four billion copper**, which is
    /// the whole reason the conversion is a function.
    #[test]
    fn a_negative_amount_is_zero_rather_than_a_wrap() {
        assert_eq!(copper_of(Some(500)), 500);
        assert_eq!(copper_of(Some(-1)), 0);
        assert_eq!(copper_of(None), 0);
        assert_eq!(copper_of(Some(i64::from(u32::MAX) + 1)), 0);
    }

    /// **An empty slot answers a nil name and a zero count** — the first
    /// because the interface tests it, the second because it does arithmetic on
    /// it on the very next line.
    #[test]
    fn an_empty_item_slot_is_a_nil_name_and_a_number() {
        let (name, texture, count, quality, usable) = item_answer(None);
        assert_eq!(name, None);
        assert_eq!(texture, None);
        assert_eq!(count, 0);
        assert_eq!(quality, 0);
        assert_eq!(usable, mlua::Value::Nil);
    }

    /// An empty mailbox answers numbers rather than nils, which is what
    /// `InboxFrame_Update`'s `index <= numItems` comparison needs.
    #[test]
    fn an_empty_mailbox_answers_numbers() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetInboxNumItems()"), "Integer(0)");
        assert_eq!(eval(&world, "return GetNumStationeries()"), "Integer(0)");
        assert_eq!(eval(&world, "return GetSendMailPrice()"), "Integer(30)");
        assert_eq!(eval(&world, "return GetSendMailMoney()"), "Integer(0)");
        // …and a row past the end is an empty row rather than a raise.
        assert_eq!(
            eval(&world, "local a = GetInboxHeaderInfo(3); if a then return 1 else return 0 end"),
            "Integer(0)",
            "a nil package icon, not an empty string"
        );
    }

    /// **`GetInboxText` on nothing still answers four values**, because
    /// `OpenMail_Update` assigns all four in one statement.
    #[test]
    fn the_body_read_answers_four_values_even_for_nothing() {
        let world = Stub::default();
        assert_eq!(
            eval(
                &world,
                "local a, b, c, d = GetInboxText(1); if a or b or c or d then return 1 else return 0 end"
            ),
            "Integer(0)"
        );
    }
}
