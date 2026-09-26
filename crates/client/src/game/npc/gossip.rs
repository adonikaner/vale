//! **Talking to an NPC** — the gossip window, the text behind it, and the
//! `"npc"` unit token every interaction panel is written against.
//!
//! The client's half of [`vale_protocol::play::gossip`]'s menu side. The same two
//! kinds of state the quest module keeps: the **window** is a page of dialogue
//! that arrived once and is replaced by the next one, and the **text cache** is
//! a per-id population that fills a round trip behind, exactly as item and
//! quest templates do — `SMSG_GOSSIP_MESSAGE` carries a text *id* and the words
//! are `CMSG_NPC_TEXT_QUERY`'s to fetch.
//!
//! ## `"npc"` is a unit token, and this module is where it points
//!
//! `GossipFrameUpdate` calls `UnitName("npc")` and `SetPortraitTexture(…,
//! "npc")`, and `QuestFrame` and `MerchantFrame` do the same — the token names
//! *whoever the character is talking to*, whichever window that conversation is
//! showing. [`NpcUnit`] is that guid, **derived every frame** from the three
//! windows rather than written by each of them: three writers with
//! open/close ordering between them is exactly the shape that leaves a stale
//! guid behind, and a derivation cannot.
//!
//! ## A press is a record, a close is local, and a coded option is refused
//!
//! `SelectGossipOption` sends the **server's own** option index, which the wire
//! carries per line and this client echoes rather than recomputes. `CloseGossip`
//! sends nothing at all — 1.12 has no gossip-close opcode; the server learns
//! when the next hello arrives — so the close is a local clear and a
//! `GOSSIP_CLOSED`. And an option whose `coded` flag is set wants a text-entry
//! box this client does not have, so its press does nothing rather than sending
//! an answerless select: the server would read the missing string as garbage.
//!
//! ## …and walking away closes all three windows, on the client's own tape
//!
//! The server gates every hello on its `INTERACTION_DISTANCE` and never speaks
//! again — no packet announces that the player left. [`out_of_range`] is the
//! reference's local close: past five yards (or the NPC despawning outright),
//! whichever of the gossip menu, the vendor and the quest page are up come
//! down, each with its own event and only its own. It lives here because this
//! module already owns the one guid all three windows share.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use vale_protocol::play::gossip::{GossipMenu, MISSING_TEXT};
use vale_protocol::socket::session::{NpcVerb, QuestVerb};

use super::super::events::{GossipClosed, GossipShow};
use crate::world::session::Session;

/// One of the three menu packets, handed on from
/// [`super::super::combat::action::drain_events`]. The vendor family is
/// [`super::merchant::MerchantAnswer`]'s.
#[derive(Message, Debug, Clone)]
pub enum GossipAnswer {
    Show(Box<GossipMenu>),
    Closed,
    Text { text_id: u32, text: String },
}

/// **Whoever the character is talking to** — the guid the `"npc"` token names,
/// or `None` between conversations. Derived by [`point_the_token`]; read by
/// [`super::super::api::Units`].
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

    /// **What `GetGossipText` answers.** Empty while the words are still in
    /// flight — the reference's own wordless first frame — and the client's
    /// own `"Missing gossip text!"` for an id that answered blank.
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

/// **The `$` variables are somebody else's job.**
///
/// An `npc_text` row carries `$B`, `$N`, `$C` and `$R` verbatim — the server
/// writes the column out unchanged — and this module cannot resolve them: `$N`
/// is the character's own name and there is no world borrow here. They are
/// substituted once, at the read, by [`crate::game::messages::substitute`],
/// which the quest pages go through too. Two copies of that rule is two places
/// for it to drift.
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
                    .in_set(super::super::GameSet),
            );
    }
}

/// A press the interface made — recorded, like every write.
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

/// Fold what the server said into the window, and tell the interface.
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
                // **The words are somebody else's packet.** Asked for once per
                // id; a menu whose text is already cached opens whole.
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
            // **`SMSG_GOSSIP_COMPLETE` closes this window and only this one.**
            // The client's handler clears the gossip module's own guid, which
            // raises `GOSSIP_CLOSED` and nothing else. The merchant and the
            // quest page have a guid and a clear apiece and neither is on this
            // path — so raising
            // all three here, which an earlier round planned to, would shut two
            // windows the reference leaves standing.
            GossipAnswer::Closed => {
                if window.menu.take().is_some() {
                    closed.write(GossipClosed);
                }
            }
            GossipAnswer::Text { text_id, text } => {
                window.texts.insert(*text_id, text.clone());
                // The open menu just got its words: raise `GOSSIP_SHOW` again
                // so `GossipFrameUpdate` re-reads `GetGossipText`. A second
                // update of an already-correct panel is the cheap direction.
                if window.menu.as_ref().is_some_and(|m| m.text_id == *text_id) {
                    shown.write(GossipShow);
                }
            }
        }
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<GossipPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_gossip_presses() {
        out.write(press);
    }
}

/// …and send it.
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
                // **A coded option is refused**, not half-sent — see the
                // module note.
                if option.coded {
                    continue;
                }
                active.live.npc(NpcVerb::GossipSelect {
                    guid,
                    // The server's own index, echoed — never this list's.
                    option: option.index,
                });
            }
            GossipPress::Available(index) | GossipPress::Active(index) => {
                // The same one-array-two-lists split a questgiver greeting has,
                // crossed by the same rule — see [`super::quest::is_active_offer`].
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
                // …and the same icon-0 branch, which `SelectGossipAvailableQuest`
                // takes exactly as the questgiver panel's does.
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

/// **Walking away is what closes a conversation**, and the measurement is the
/// client's own — nothing on the wire ever announces it. The server gates
/// every hello on `INTERACTION_DISTANCE` and then never speaks again; the
/// reference client closes its windows locally when the range is exceeded, and
/// this is that close, for all three windows at once through the one guid the
/// `"npc"` token already derives.
///
/// The same shape as `death::corpse_range`, the other client-side range check.
/// No edge latch is needed: closing empties the windows, the token derives to
/// `None` next frame, and the check stops itself.
///
/// Three absences, told apart deliberately:
/// * **reach measured and over the line** — close;
/// * **the guid resolves but is unplaced this frame** — hold, or the window
///   would flicker shut on the frame the entity arrives;
/// * **the guid resolves to nobody at all** — close: the NPC despawned under
///   the conversation, which is the walk-away seen from the other side.
///
/// **Five windows now, not three.** The trainer was the fourth and the flight
/// map is the fifth, both on the same tape and for the same reason:
/// `CanInteractWithNPC` gates every `CMSG_TRAINER_LIST` and every
/// `CMSG_TAXIQUERYAVAILABLENODES`, and nothing announces the walk-away.
///
/// **The taxi map used to be excluded and the argument was wrong.** It ran: a
/// map already holding its whole content does not care that the server has
/// stopped talking, and `TaxiFrame` is `toplevel` with its own close. Both
/// halves are true and neither is the point — the window is a *conversation
/// with an NPC*, it is the only thing `UnitName("npc")` is naming while it is
/// open, and a press in it sends a packet the server will refuse from across
/// the zone. Reported as "it stays open when you walk away", which it did.
#[allow(clippy::too_many_arguments)]
pub(super) fn out_of_range(
    units: super::super::api::Units,
    npc: Res<NpcUnit>,
    mut gossip: ResMut<GossipWindow>,
    mut merchant: ResMut<super::merchant::MerchantWindow>,
    mut trainer: ResMut<super::trainer::TrainerWindow>,
    mut taxi: ResMut<super::taxi::TaxiWindow>,
    mut stable: ResMut<super::stable::StableWindow>,
    mut bank: ResMut<super::bank::BankWindow>,
    mut quests: ResMut<super::quest::Quests>,
    mut gossip_closed: MessageWriter<GossipClosed>,
    mut merchant_closed: MessageWriter<super::super::events::MerchantClosed>,
    mut trainer_closed: MessageWriter<super::super::events::TrainerClosed>,
    mut taxi_closed: MessageWriter<super::super::events::TaximapClosed>,
    mut stable_closed: MessageWriter<super::super::events::PetStableClosed>,
    mut bank_closed: MessageWriter<super::super::events::BankframeClosed>,
    mut quest_finished: MessageWriter<super::super::events::QuestFinished>,
) {
    use super::super::api::UnitId;
    let Some(giver) = npc.0 else {
        return;
    };
    // **An item is a quest giver with no position, and this check is about
    // distance.** `HandleQuestgiverQueryQuestOpcode` takes an item guid as
    // happily as a creature's, so `UseContainerItem` on a "This Item Begins a
    // Quest" scrap opens a real quest page whose giver is a thing in a bag.
    // Without this test the walk-away rule below reads "the guid resolves to
    // nobody at all" — which is true and permanent for an item — and fires
    // `QUEST_FINISHED` on the very next frame. `QuestFrame_OnEvent` answers
    // that with an unconditional `HideUIPanel`, so the page opened and shut
    // again inside one frame and the whole feature looked like a dead packet.
    // Measured: the page arrives, `QuestFrame` reports visible, and then two
    // `QUEST_FINISHED`s close it.
    if !vale_protocol::state::objects::guid_is_in_the_world(giver) {
        return;
    }
    let gone = match units.reach(UnitId::Player, UnitId::Npc) {
        Some(reach) => reach > vale_protocol::play::gossip::INTERACTION_DISTANCE,
        // Unplaced is a hold; despawned is a close — see above.
        None => units.resolve(UnitId::Npc).is_none(),
    };
    if !gone {
        return;
    }
    // **Only the events for the windows that were up.** Firing all three
    // unconditionally would raise `QUEST_FINISHED` at a plain vendor, and
    // `QuestFrame_OnEvent` answers that with an unconditional `HideUIPanel`.
    if gossip.menu.take().is_some() {
        gossip_closed.write(GossipClosed);
    }
    if merchant.close() {
        merchant_closed.write(super::super::events::MerchantClosed);
    }
    if trainer.close() {
        trainer_closed.write(super::super::events::TrainerClosed);
    }
    if taxi.close() {
        taxi_closed.write(super::super::events::TaximapClosed);
    }
    if stable.close() {
        stable_closed.write(super::super::events::PetStableClosed);
    }
    if bank.close() {
        bank_closed.write(super::super::events::BankframeClosed);
    }
    if quests.drop_page() {
        quest_finished.write(super::super::events::QuestFinished);
    }
}

/// **Point the `"npc"` token at whoever the character is talking to.**
///
/// Derived, not written — see the module note. The priority is the freshest
/// kind of conversation: a quest page replaces the gossip menu that opened it
/// on the same NPC, and a vendor window is a conversation too.
fn point_the_token(
    gossip: Res<GossipWindow>,
    merchant: Res<super::merchant::MerchantWindow>,
    trainer: Res<super::trainer::TrainerWindow>,
    taxi: Res<super::taxi::TaxiWindow>,
    stable: Res<super::stable::StableWindow>,
    bank: Res<super::bank::BankWindow>,
    quests: Res<super::quest::Quests>,
    trade: Res<super::super::session::trade::TradeWindow>,
    mut npc: ResMut<NpcUnit>,
) {
    // **The trainer is a conversation too** — `ClassTrainerFrame_Update` calls
    // `UnitName("npc")` and `SetPortraitTexture(…, "npc")` on every redraw, so
    // a window whose guid this does not name draws an empty plate and no face.
    // **…and so is the flight master**, on exactly the same evidence:
    // `TaxiFrame_OnEvent` opens with `TaxiMerchant:SetText(UnitName("npc"))`
    // and `SetPortraitTexture(TaxiPortrait, "npc")`, so a window this does not
    // name draws a blank plate and an empty portrait frame — which is what was
    // reported.
    let pointed = quests
        .page()
        .guid()
        .or_else(|| gossip.guid())
        .or_else(|| merchant.guid())
        .or_else(|| trainer.guid())
        .or_else(|| taxi.guid())
        // **…and so is the stable master**: `PetStable_Update`'s own first line
        // is `SetPortraitTexture(PetStableFramePortrait, "npc")`, so a window
        // this does not name draws the plate with an empty face.
        .or_else(|| stable.guid())
        // **…and the banker**: `BankFrame_OnEvent`'s open arm is
        // `BankFrameTitleText:SetText(UnitName("npc"))` and
        // `SetPortraitTexture(BankPortraitTexture, "npc")`.
        .or_else(|| bank.guid())
        // **…and the trade partner**, who is not an NPC and takes the token
        // anyway: `TradeFrame_Update` draws the far side with `UnitName("NPC")`
        // and `SetPortraitTexture(…, "NPC")`. See
        // [`super::super::session::trade`].
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
