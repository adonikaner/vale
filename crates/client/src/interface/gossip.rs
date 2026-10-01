//! The gossip window, the text behind it, and the `"npc"` unit token every
//! interaction panel is written against.
//!
//! This is the client's side of [`vale_protocol::play::gossip`]'s menu packets.
//! It keeps the same two kinds of state as the quest module. The window is one
//! page of dialogue, which arrives once and is replaced by the next one. The
//! text cache is a per-id population that fills one round trip later, as item
//! and quest templates do: `SMSG_GOSSIP_MESSAGE` carries a text id and
//! `CMSG_NPC_TEXT_QUERY` fetches the words.
//!
//! ## The `"npc"` unit token
//!
//! `GossipFrameUpdate` calls `UnitName("npc")` and `SetPortraitTexture(…,
//! "npc")`, and `QuestFrame` and `MerchantFrame` do the same. The token names
//! the unit the character is talking to, whichever window that conversation is
//! showing. [`NpcUnit`] is that guid. It is derived every frame from the
//! windows and not written by each of them: three writers with open/close
//! ordering between them can leave a stale guid behind, and a derivation
//! cannot.
//!
//! ## Presses, closing, and coded options
//!
//! `SelectGossipOption` sends the server's own option index, which the wire
//! carries per line; this client echoes it and does not recompute it.
//! `CloseGossip` sends nothing: 1.12 has no gossip-close opcode, and the server
//! learns of the close when the next hello arrives. The close is therefore a
//! local clear and a `GOSSIP_CLOSED`. An option whose `coded` flag is set needs
//! a text-entry box this client does not have, so pressing it does nothing: a
//! select sent without the string would be misread by the server.
//!
//! ## Walking out of range closes the interaction windows
//!
//! The server checks its `INTERACTION_DISTANCE` at every hello and sends
//! nothing afterwards; no packet announces that the player left.
//! [`out_of_range`] is the local close the 1.12.1 client makes: past five
//! yards, or when the NPC despawns, each of the gossip menu, the vendor and the
//! quest page that is open is closed, with its own event and no other. It is in
//! this module because this module owns the one guid all three windows share.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use vale_protocol::play::gossip::{GossipMenu, MISSING_TEXT};
use vale_protocol::socket::session::{NpcVerb, QuestVerb};

use super::events::{GossipClosed, GossipShow};
use crate::world::session::Session;

/// One of the three menu packets, handed on from
/// [`super::action::drain_events`]. The vendor family is
/// [`super::merchant::MerchantAnswer`]'s.
#[derive(Message, Debug, Clone)]
pub enum GossipAnswer {
    Show(Box<GossipMenu>),
    Closed,
    Text { text_id: u32, text: String },
}

/// The guid of the unit the character is talking to, which the `"npc"` token
/// names, or `None` between conversations. Derived by [`point_the_token`]; read
/// by [`super::api::Units`].
#[derive(Resource, Default)]
pub struct NpcUnit(pub Option<u64>);

/// The open gossip window, and the words behind every menu this session.
#[derive(Resource, Default)]
pub struct GossipWindow {
    menu: Option<GossipMenu>,
    /// The `npc_text` table, one entry per id ever seen. Never invalidated:
    /// the table is static data on the server's side.
    texts: HashMap<u32, String>,
    /// Ids asked about, so an id the server never answers is asked once.
    asked: HashSet<u32>,
}

impl GossipWindow {
    pub fn menu(&self) -> Option<&GossipMenu> {
        self.menu.as_ref()
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.menu.as_ref()?.guid)
    }

    /// The answer to `GetGossipText`. Empty while the words are still in
    /// flight, as the 1.12.1 client's first frame is, and the client's own
    /// `"Missing gossip text!"` for an id that answered blank.
    pub fn text(&self) -> String {
        let Some(menu) = &self.menu else {
            return String::new();
        };
        match self.texts.get(&menu.text_id) {
            Some(text) if text.is_empty() => MISSING_TEXT.to_string(),
            Some(text) => text.clone(),
            None => String::new(),
        }
    }
}

/// Registers the gossip messages, resources and systems. It does not
/// substitute the `$` variables.
///
/// An `npc_text` row carries `$B`, `$N`, `$C` and `$R` verbatim, because the
/// server writes the column out unchanged. This module cannot resolve them:
/// `$N` is the character's own name and there is no world borrow here. They are
/// substituted once, at the read, by [`crate::interface::messages::substitute`],
/// which the quest pages use too. A second copy of that rule could drift from
/// the first.
pub struct GossipPlugin;

impl Plugin for GossipPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GossipAnswer>()
            .add_message::<GossipPress>()
            .init_resource::<GossipWindow>()
            .init_resource::<NpcUnit>()
            .add_systems(
                Update,
                (announce, presses, act, point_the_token, out_of_range)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// A press the interface made. It is recorded as a message, like every write.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GossipPress {
    /// `SelectGossipOption(n)` — one-based into the *options* half.
    Option(usize),
    /// `SelectGossipAvailableQuest(n)` / `SelectGossipActiveQuest(n)` —
    /// one-based into each half of the quest list, split by icon exactly as a
    /// questgiver greeting's is.
    Available(usize),
    Active(usize),
    /// `CloseGossip()` — local; there is no opcode. See the module note.
    Close,
}

/// Applies each server answer to the window and raises the interface's event.
fn announce(
    mut answers: MessageReader<GossipAnswer>,
    mut window: ResMut<GossipWindow>,
    session: Res<Session>,
    mut shown: MessageWriter<GossipShow>,
    mut closed: MessageWriter<GossipClosed>,
) {
    for answer in answers.read() {
        match answer {
            GossipAnswer::Show(menu) => {
                // The words arrive in a separate packet. They are requested
                // once per id; a menu whose text is already cached opens whole.
                if !window.texts.contains_key(&menu.text_id)
                    && window.asked.insert(menu.text_id)
                {
                    if let Some(active) = session.active.as_ref() {
                        active.live.npc(NpcVerb::TextQuery {
                            text_id: menu.text_id,
                            guid: menu.guid,
                        });
                    }
                }
                window.menu = Some((**menu).clone());
                shown.write(GossipShow);
            }
            // `SMSG_GOSSIP_COMPLETE` closes this window and only this one. The
            // 1.12.1 client clears the gossip guid, which raises
            // `GOSSIP_CLOSED` and nothing else. The merchant and the quest page
            // each have their own guid and their own clear, and this packet
            // affects neither. Raising all three events here would close two
            // windows the 1.12.1 client leaves open.
            GossipAnswer::Closed => {
                if window.menu.take().is_some() {
                    closed.write(GossipClosed);
                }
            }
            GossipAnswer::Text { text_id, text } => {
                window.texts.insert(*text_id, text.clone());
                // The open menu's words have arrived: raise `GOSSIP_SHOW` again
                // so `GossipFrameUpdate` re-reads `GetGossipText`. A second
                // update of an already-correct panel costs little.
                if window.menu.as_ref().is_some_and(|m| m.text_id == *text_id) {
                    shown.write(GossipShow);
                }
            }
        }
    }
}

/// Drains the presses the interface made.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<GossipPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_gossip_presses() {
        out.write(press);
    }
}

/// Sends each press to the server, or applies it locally.
fn act(
    mut presses: MessageReader<GossipPress>,
    mut window: ResMut<GossipWindow>,
    session: Res<Session>,
    mut closed: MessageWriter<GossipClosed>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for press in presses.read() {
        let Some(menu) = window.menu.as_ref() else {
            continue;
        };
        let guid = menu.guid;
        match press {
            GossipPress::Option(index) => {
                let Some(option) = index.checked_sub(1).and_then(|i| menu.options.get(i))
                else {
                    continue;
                };
                // A coded option is refused and not sent without its string.
                // See the module note.
                if option.coded {
                    continue;
                }
                active.live.npc(NpcVerb::GossipSelect {
                    guid,
                    // The server's own index, echoed. Never this list's.
                    option: option.index,
                });
            }
            GossipPress::Available(index) | GossipPress::Active(index) => {
                // The quest list is one array split into two lists, by the same
                // rule as a questgiver greeting. See
                // [`super::quest::is_active_offer`].
                let active_half = matches!(press, GossipPress::Active(_));
                let Some(offer) = index.checked_sub(1).and_then(|i| {
                    menu.quests
                        .iter()
                        .filter(|q| super::quest::is_active_offer(q.icon) == active_half)
                        .nth(i)
                }) else {
                    continue;
                };
                let quest_id = offer.quest_id;
                // The icon-0 case is the same too: `SelectGossipAvailableQuest`
                // takes it exactly as the questgiver panel's does.
                let handin = active_half || super::quest::offer_is_immediate_handin(offer.icon);
                active.live.quest(match handin {
                    true => QuestVerb::Complete { guid, quest_id },
                    false => QuestVerb::Details { guid, quest_id },
                });
            }
            GossipPress::Close => {
                if window.menu.take().is_some() {
                    closed.write(GossipClosed);
                }
            }
        }
    }
}

/// Closes the interaction windows when the player walks out of range.
///
/// The measurement is the client's own; nothing on the wire announces the
/// walk-away. The server checks `INTERACTION_DISTANCE` at every hello and sends
/// nothing afterwards. The 1.12.1 client closes its windows locally when the
/// range is exceeded, and this system does the same for every window at once,
/// through the one guid the `"npc"` token derives.
///
/// It has the same form as `death::corpse_range`, the other client-side range
/// check. No edge latch is needed: closing empties the windows, the token
/// derives to `None` next frame, and the check stops.
///
/// Three cases are distinguished:
/// * the reach is measured and over the limit: close;
/// * the guid resolves but the unit is unplaced this frame: hold, or the window
///   would close on the frame the entity arrives;
/// * the guid resolves to no unit at all: close, because the NPC despawned
///   during the conversation.
///
/// The check covers five windows and not only the first three. The trainer was
/// the fourth and the flight map is the fifth, both for the same reason:
/// `CanInteractWithNPC` gates every `CMSG_TRAINER_LIST` and every
/// `CMSG_TAXIQUERYAVAILABLENODES`, and nothing announces the walk-away.
///
/// The taxi map is included although it holds its whole content once opened and
/// `TaxiFrame` is `toplevel` with its own close. The window is a conversation
/// with an NPC: it is the only thing `UnitName("npc")` names while it is open,
/// and a press in it sends a packet the server refuses from out of range. While
/// the map was excluded from this check it stayed open when the player walked
/// away.
#[allow(clippy::too_many_arguments)]
pub(super) fn out_of_range(
    units: super::api::Units,
    npc: Res<NpcUnit>,
    mut gossip: ResMut<GossipWindow>,
    mut merchant: ResMut<super::merchant::MerchantWindow>,
    mut trainer: ResMut<super::trainer::TrainerWindow>,
    mut taxi: ResMut<super::taxi::TaxiWindow>,
    mut stable: ResMut<super::stable::StableWindow>,
    mut bank: ResMut<super::bank::BankWindow>,
    mut quests: ResMut<super::quest::Quests>,
    mut gossip_closed: MessageWriter<GossipClosed>,
    mut merchant_closed: MessageWriter<super::events::MerchantClosed>,
    mut trainer_closed: MessageWriter<super::events::TrainerClosed>,
    mut taxi_closed: MessageWriter<super::events::TaximapClosed>,
    mut stable_closed: MessageWriter<super::events::PetStableClosed>,
    mut bank_closed: MessageWriter<super::events::BankframeClosed>,
    mut quest_finished: MessageWriter<super::events::QuestFinished>,
) {
    let Some(giver) = npc.0 else {
        return;
    };
    // An item can be a quest giver and has no position, and this check is about
    // distance. `HandleQuestgiverQueryQuestOpcode` accepts an item guid as well
    // as a creature's, so `UseContainerItem` on a "This Item Begins a Quest"
    // item opens a real quest page whose giver is an item in a bag. Without
    // this test the walk-away rule below reads "the guid resolves to no unit at
    // all", which is permanently true for an item, and raises `QUEST_FINISHED`
    // on the next frame. `QuestFrame_OnEvent` answers that with an
    // unconditional `HideUIPanel`, so the page opened and closed within one
    // frame. Measured: the page arrives, `QuestFrame` reports visible, and then
    // two `QUEST_FINISHED`s close it.
    if !vale_protocol::state::objects::guid_is_in_the_world(giver) {
        return;
    }
    if !npc_gone(&units) {
        return;
    }
    // Only the events for the windows that were open are raised. Raising all of
    // them unconditionally would raise `QUEST_FINISHED` at a plain vendor, and
    // `QuestFrame_OnEvent` answers that with an unconditional `HideUIPanel`.
    if gossip.menu.take().is_some() {
        gossip_closed.write(GossipClosed);
    }
    if merchant.close() {
        merchant_closed.write(super::events::MerchantClosed);
    }
    if trainer.close() {
        trainer_closed.write(super::events::TrainerClosed);
    }
    if taxi.close() {
        taxi_closed.write(super::events::TaximapClosed);
    }
    if stable.close() {
        stable_closed.write(super::events::PetStableClosed);
    }
    if bank.close() {
        bank_closed.write(super::events::BankframeClosed);
    }
    if quests.drop_page() {
        quest_finished.write(super::events::QuestFinished);
    }
}

/// Whether the unit the `"npc"` token names is out of reach: farther than
/// `INTERACTION_DISTANCE`, or despawned. A unit that resolves but is unplaced
/// this frame is not gone.
///
/// [`out_of_range`] closes its seven windows on it. The guild registrar and
/// the tabard designer close theirs on it from their own modules, because
/// their state is theirs; see [`super::petition`] and [`super::tabard`].
pub(super) fn npc_gone(units: &super::api::Units) -> bool {
    use super::api::UnitId;
    match units.reach(UnitId::Player, UnitId::Npc) {
        Some(reach) => reach > vale_protocol::play::gossip::INTERACTION_DISTANCE,
        None => units.resolve(UnitId::Npc).is_none(),
    }
}

/// Points the `"npc"` token at the unit the character is talking to.
///
/// The guid is derived and not written; see the module note. The priority puts
/// the most recent kind of conversation first: a quest page replaces the gossip
/// menu that opened it on the same NPC. A vendor window is a conversation too.
fn point_the_token(
    gossip: Res<GossipWindow>,
    merchant: Res<super::merchant::MerchantWindow>,
    trainer: Res<super::trainer::TrainerWindow>,
    taxi: Res<super::taxi::TaxiWindow>,
    stable: Res<super::stable::StableWindow>,
    bank: Res<super::bank::BankWindow>,
    quests: Res<super::quest::Quests>,
    trade: Res<super::trade::TradeWindow>,
    registrar: Res<super::petition::RegistrarWindow>,
    tabard: Res<super::tabard::TabardWindow>,
    mut npc: ResMut<NpcUnit>,
) {
    // The trainer is a conversation too: `ClassTrainerFrame_Update` calls
    // `UnitName("npc")` and `SetPortraitTexture(…, "npc")` on every redraw, so
    // a window whose guid is not named here draws an empty plate and no face.
    // The flight master is one for the same reason: `TaxiFrame_OnEvent` opens
    // with `TaxiMerchant:SetText(UnitName("npc"))` and
    // `SetPortraitTexture(TaxiPortrait, "npc")`, so a window not named here
    // draws a blank plate and an empty portrait frame.
    let pointed = quests
        .page()
        .guid()
        .or_else(|| gossip.guid())
        .or_else(|| merchant.guid())
        .or_else(|| trainer.guid())
        .or_else(|| taxi.guid())
        // The stable master: the first line of `PetStable_Update` is
        // `SetPortraitTexture(PetStableFramePortrait, "npc")`, so a window not
        // named here draws the plate with an empty face.
        .or_else(|| stable.guid())
        // The banker: the open branch of `BankFrame_OnEvent` calls
        // `BankFrameTitleText:SetText(UnitName("npc"))` and
        // `SetPortraitTexture(BankPortraitTexture, "npc")`.
        .or_else(|| bank.guid())
        // The guild registrar: `GuildRegistrar_OnShow` calls
        // `SetPortraitTexture(GuildRegistrarFramePortrait, "NPC")` and
        // `UnitName("NPC")`. The tabard designer: `TabardFrame_OnEvent` does
        // the same with `TabardFramePortrait`.
        .or_else(|| registrar.guid())
        .or_else(|| tabard.guid())
        // The trade partner is not an NPC and takes the token anyway:
        // `TradeFrame_Update` draws the far side with `UnitName("NPC")` and
        // `SetPortraitTexture(…, "NPC")`. See [`super::trade`].
        .or_else(|| trade.partner());
    // Change-gated: a `ResMut` dereferenced every frame is a resource marked
    // changed every frame, and `Units` reads this.
    if npc.0 != pointed {
        npc.0 = pointed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::gossip::GossipOption;

    fn menu() -> GossipMenu {
        GossipMenu {
            guid: 9,
            text_id: 1234,
            options: vec![
                GossipOption {
                    index: 0,
                    icon: 1,
                    coded: false,
                    text: "Browse".into(),
                },
                GossipOption {
                    index: 3,
                    icon: 0,
                    coded: true,
                    text: "Speak the password".into(),
                },
            ],
            quests: Vec::new(),
        }
    }

    /// The text is empty while in flight, the words once they land, and the
    /// client's own literal for an id that answered blank.
    #[test]
    fn the_text_is_pending_then_words_then_the_missing_literal() {
        let mut window = GossipWindow {
            menu: Some(menu()),
            ..GossipWindow::default()
        };
        assert_eq!(window.text(), "", "in flight");
        window.texts.insert(1234, "Well met, $N.$BStay a while.".into());
        assert_eq!(
            window.text(),
            "Well met, $N.$BStay a while.",
            "raw out of here — the $ variables are substituted at the read"
        );
        window.texts.insert(1234, String::new());
        assert_eq!(window.text(), MISSING_TEXT, "the client's own fallback");
        window.menu = None;
        assert_eq!(window.text(), "", "no window, no words");
    }
}
