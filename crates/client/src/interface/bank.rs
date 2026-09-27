//! **The bank window** — which banker it is open at, the one slot it sells,
//! and the right-click that moves an item across the counter.
//!
//! The client's half of [`vale_protocol::play::bank`]. The split every
//! panel in this directory keeps: the *rule* — what the next slot costs, and
//! when six is full — is [`vale_assets::tables::bank`], and the *contents*
//! are not this module's at all: the bank's twenty-four squares and six bags
//! are update fields, read by [`vale_protocol::play::items::Inventory`]
//! beside the backpack and diffed by [`super::items`],
//! which is what raises `PLAYERBANKSLOTS_CHANGED`. What is left here is the
//! three things that need a banker.
//!
//! ```text
//! that it is open      SMSG_SHOW_BANK, a guid — BANKFRAME_OPENED
//! buying a slot        CMSG_BUY_BANK_SLOT; the answer is a refusal or nothing
//! a click across       CMSG_AUTOBANK_ITEM / CMSG_AUTOSTORE_BANK_ITEM, chosen by
//!                      which side of the counter the square is on
//! ```
//!
//! ## The window can be drawn with no banker, and that is the reference
//!
//! `BankFrame_OnEvent`'s `BANKFRAME_OPENED` arm is `ShowUIPanel(this)` and
//! every square then reads `GetInventoryItemTexture("player", id)` off the
//! same fields the paper doll reads. Nothing in the packet is a list. So the
//! panel probe opens it with its contents intact, and what this window
//! *gates* is only the verbs: a purchase or a move sent with no banker in
//! reach is refused by the server (`CheckBanker`, `CanUseBank`), and the
//! refusal is the game's own sentence.
//!
//! ## A purchase that succeeds says nothing
//!
//! `HandleBuyBankSlotOpcode` sends `SMSG_BUY_BANK_SLOT_RESULT` for its three
//! refusals and, on success, writes the third byte of `PLAYER_BYTES_2` and
//! takes the money. So [`announce`] says a refusal's sentence and does
//! nothing else, and the panel's own `PLAYERBANKBAGSLOTS_CHANGED` comes off
//! the byte, from the inventory diff, a round trip later.
//!
//! ## The right-click is the same gesture as selling
//!
//! `ContainerFrameItemButton_OnClick`'s right button is a plain
//! `UseContainerItem(bag, slot)` and the C side decides what a use *is*;
//! with a vendor open it is a sale and with the bank open it is a deposit.
//! `BankFrameItemButtonGeneric_OnClick` sends the same verb with
//! `BANK_CONTAINER`, which is a withdrawal. Both land in
//! [`super::items`]'s use body, which asks [`BankWindow::is_open`]
//! ahead of the merchant test and sends [`vale_protocol::socket::session::Command::BankItem`];
//! the direction is the server's own fork on the source position.
//!
//! ## Walking away closes it, on the client's own tape
//!
//! There is no bank-close opcode and the server never says the conversation
//! ended, exactly as [`super::gossip`] states for its windows.
//! `CloseBankFrame()` is a local clear and a `BANKFRAME_CLOSED`, and
//! [`super::gossip::out_of_range`] takes this window down with the others.

use bevy::prelude::*;

use vale_protocol::play::bank::BankSlotResult;
use vale_protocol::socket::session::NpcVerb;

use super::events::{BankframeClosed, BankframeOpened};
use crate::world::session::Session;

/// One of the two bank packets, handed on from [`crate::world::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub enum BankAnswer {
    /// `SMSG_SHOW_BANK` — the banker.
    Show(u64),
    /// `SMSG_BUY_BANK_SLOT_RESULT` — a refusal, or a code this client does not
    /// know. See the module note on why success is not here.
    SlotResult(Option<BankSlotResult>),
}

/// A press the interface made.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankPress {
    /// `PurchaseSlot()` — the confirm popup's Yes.
    PurchaseSlot,
    /// `CloseBankFrame()` — local, like the gossip close.
    Close,
}

/// The open bank, or nothing.
#[derive(Resource, Default)]
pub struct BankWindow {
    /// The banker. Every verb names it again.
    banker: Option<u64>,
}

impl BankWindow {
    pub fn is_open(&self) -> bool {
        self.banker.is_some()
    }

    pub fn guid(&self) -> Option<u64> {
        self.banker
    }

    /// Take the window down, answering whether one was up — the door
    /// [`super::gossip::out_of_range`] calls, like the stable's.
    pub(crate) fn close(&mut self) -> bool {
        self.banker.take().is_some()
    }
}

pub struct BankPlugin;

impl Plugin for BankPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BankAnswer>()
            .add_message::<BankPress>()
            .init_resource::<BankWindow>()
            .add_systems(
                Update,
                (announce, presses, act).chain().in_set(super::GameSet),
            );
    }
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<BankAnswer>,
    mut window: ResMut<BankWindow>,
    mut say: super::messages::Announce,
    mut opened: MessageWriter<BankframeOpened>,
) {
    for answer in answers.read() {
        match *answer {
            BankAnswer::Show(banker) => {
                // **A second show at the same banker is not a second open**:
                // `BankFrame_OnEvent` would `ShowUIPanel` a shown panel, which
                // is harmless, but the reference raises it once and so does
                // this.
                if window.banker != Some(banker) {
                    window.banker = Some(banker);
                    opened.write(BankframeOpened);
                }
            }
            // A refusal says its sentence; the window is left alone. The
            // success that never arrives is the module note.
            BankAnswer::SlotResult(result) => {
                if let Some(key) = result.and_then(BankSlotResult::key) {
                    say.key(key);
                }
            }
        }
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<BankPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_bank_presses() {
        out.write(press);
    }
}

/// …and act on it: one verb that leaves, and one close that does not.
fn act(
    mut presses: MessageReader<BankPress>,
    mut window: ResMut<BankWindow>,
    session: Res<Session>,
    mut closed: MessageWriter<BankframeClosed>,
) {
    for press in presses.read() {
        match press {
            BankPress::PurchaseSlot => {
                let (Some(banker), Some(active)) = (window.guid(), session.active.as_ref()) else {
                    continue;
                };
                active.live.npc(NpcVerb::BuyBankSlot(banker));
            }
            BankPress::Close => {
                if window.close() {
                    closed.write(BankframeClosed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::interface::events::UiErrorMessage;
    use crate::interface::messages::UiStrings;
    use std::sync::Arc;

    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(UiStrings(Some(Arc::new(
            vale_assets::interface::strings::Strings::parse(
                b"ERR_BANKSLOT_INSUFFICIENT_FUNDS = \"You can't afford that.\";",
            ),
        ))));
        app.add_message::<BankframeOpened>()
            .add_message::<BankframeClosed>()
            .init_resource::<Session>()
            .add_plugins(BankPlugin);
        crate::interface::messages::Announce::register(&mut app);
        app
    }

    fn drained<M: Message + Clone>(app: &mut App) -> Vec<M> {
        let messages = app.world().resource::<bevy::ecs::message::Messages<M>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).cloned().collect()
    }

    /// The show opens the window once, a repeat at the same banker is silent,
    /// and the close takes it down with its event.
    #[test]
    fn a_show_opens_once_and_a_close_shuts_it() {
        let mut app = app();
        app.world_mut().write_message(BankAnswer::Show(0x77));
        app.world_mut().write_message(BankAnswer::Show(0x77));
        app.update();
        assert!(app.world().resource::<BankWindow>().is_open());
        assert_eq!(app.world().resource::<BankWindow>().guid(), Some(0x77));
        assert_eq!(drained::<BankframeOpened>(&mut app).len(), 1);

        app.world_mut().write_message(BankPress::Close);
        app.update();
        assert!(!app.world().resource::<BankWindow>().is_open());
        assert_eq!(drained::<BankframeClosed>(&mut app).len(), 1);
        // …and a second close raises nothing more: the queue still holds
        // the one from the frame before, and no other.
        app.world_mut().write_message(BankPress::Close);
        app.update();
        assert_eq!(drained::<BankframeClosed>(&mut app).len(), 1);
    }

    /// A refusal is the game's own sentence, and leaves the window as it was.
    #[test]
    fn a_refusal_says_its_sentence_and_changes_nothing() {
        let mut app = app();
        app.world_mut().write_message(BankAnswer::Show(0x77));
        app.update();
        app.world_mut()
            .write_message(BankAnswer::SlotResult(Some(BankSlotResult::InsufficientFunds)));
        app.world_mut().write_message(BankAnswer::SlotResult(None));
        app.update();
        let said = drained::<UiErrorMessage>(&mut app);
        assert_eq!(said.len(), 1);
        assert!(app.world().resource::<BankWindow>().is_open());
    }
}
