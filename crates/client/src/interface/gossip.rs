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
//! [`close_when_npc_gone`] is the local close the 1.12.1 client makes: past
//! five yards, or when the NPC despawns, the window is closed with its own
//! event and no other. Each window implements [`NpcWindow`] and registers the
//! close for itself; each window that names the token implements [`NpcToken`]
//! and registers its claim. This module owns the token and the two traits.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use vale_protocol::play::gossip::{GossipMenu, MISSING_TEXT};
use vale_protocol::play::gossip::NpcVerb;
use vale_protocol::play::quest::QuestVerb;

use super::events::{GossipClosed, GossipShow};
use crate::world::session::Session;

/// One of the three menu packets, handed on from
/// [`crate::world::incoming::drain_events`]. The vendor family is
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
            .init_resource::<TokenClaims>()
            .add_systems(
                Update,
                (
                    (announce, presses, act).chain().before(NpcSet::Claim),
                    point_the_token.after(NpcSet::Claim).before(NpcSet::Close),
                )
                    .in_set(super::GameSet),
            );
        claims_the_token::<GossipWindow>(app);
        closes_on_walk_away::<GossipWindow>(app);
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
    let Some(host) = host else { return };
    for press in crate::lua::panels::gossip::take(&host) {
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

/// A window that is a conversation with one NPC or object, closed when the
/// character walks away from it.
///
/// Each window implements this in its own module and registers
/// [`close_when_npc_gone`] for itself with [`closes_on_walk_away`], so a new
/// window is no edit to this file.
pub(crate) trait NpcWindow: Resource<Mutability = bevy::ecs::component::Mutable> {
    /// The message raised when the walk-away closes an open window.
    type Closed: Message;
    const CLOSED: Self::Closed;

    /// The guid the window is talking to, or `None` while it is shut.
    fn guid(&self) -> Option<u64>;

    /// Shut the window, and answer whether it was open.
    fn close(&mut self) -> bool;

    /// The distance the walk-away rule compares with `INTERACTION_DISTANCE`,
    /// or `None` while the character or the guid's object is unplaced. For a
    /// unit this is the distance minus both combat reaches
    /// ([`super::api::Units::reach_to_guid`]).
    fn reach(units: &super::api::Units, guid: u64) -> Option<f32> {
        units.reach_to_guid(super::api::UnitId::Player, guid)
    }
}

/// A window that names the `"npc"` token while it is open.
///
/// Each one claims the token with [`claims_the_token`]; [`point_the_token`]
/// picks the claim with the lowest [`NpcToken::ORDER`].
pub(crate) trait NpcToken: Resource {
    /// The window's priority for the token: lower wins. Each window has its
    /// own number; [`claims_the_token`] refuses a second window with the same
    /// one.
    const ORDER: u8;

    /// The guid the window names, or `None` while it is shut.
    fn npc(&self) -> Option<u64>;
}

/// The guid each open window claims for the `"npc"` token, by
/// [`NpcToken::ORDER`].
#[derive(Resource, Default)]
pub(crate) struct TokenClaims {
    claims: std::collections::BTreeMap<u8, u64>,
    /// The orders registered at build, so two windows cannot share one.
    orders: HashSet<u8>,
}

/// The parts of the frame the NPC windows' shared systems run in: every
/// window's claim on the token, then [`point_the_token`], then every window's
/// walk-away close.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NpcSet {
    Claim,
    Close,
}

/// Register `W`'s claim on the `"npc"` token.
pub(crate) fn claims_the_token<W: NpcToken>(app: &mut App) {
    let fresh = app
        .world_mut()
        .get_resource_or_init::<TokenClaims>()
        .orders
        .insert(W::ORDER);
    assert!(fresh, "two windows claim the npc token at order {}", W::ORDER);
    app.add_systems(
        Update,
        claim_the_token::<W>
            .in_set(NpcSet::Claim)
            .in_set(super::GameSet),
    );
}

/// Register `W`'s walk-away close.
pub(crate) fn closes_on_walk_away<W: NpcWindow>(app: &mut App) {
    app.add_systems(
        Update,
        close_when_npc_gone::<W>
            .in_set(NpcSet::Close)
            .in_set(super::GameSet),
    );
}

/// Record the guid `W` names this frame. Change-gated, because
/// [`point_the_token`] runs every frame either way.
fn claim_the_token<W: NpcToken>(window: Res<W>, mut claims: ResMut<TokenClaims>) {
    let now = window.npc();
    if claims.claims.get(&W::ORDER).copied() == now {
        return;
    }
    match now {
        Some(guid) => claims.claims.insert(W::ORDER, guid),
        None => claims.claims.remove(&W::ORDER),
    };
}

/// Closes one window when the character walks out of its range.
///
/// The measurement is the client's own; nothing on the wire announces the
/// walk-away. The server checks `INTERACTION_DISTANCE` at every hello and sends
/// nothing afterwards. The 1.12.1 client closes its windows locally when the
/// range is exceeded, and this system does the same for each window, against
/// the guid that window is talking to.
///
/// It has the same form as `death::corpse_range`, the other client-side range
/// check. No edge latch is needed: closing empties the window, its guid reads
/// `None` next frame, and the check stops. Only the window that was open raises
/// its event: raising `QUEST_FINISHED` at a plain vendor would make
/// `QuestFrame_OnEvent` run its unconditional `HideUIPanel`.
///
/// Three cases are distinguished:
/// * the reach is measured and over the limit: close;
/// * the guid resolves but the object is unplaced this frame: hold, or the
///   window would close on the frame the entity arrives;
/// * the guid resolves to nothing at all: close, because the NPC despawned
///   during the conversation.
///
/// The taxi map is one of these windows although it holds its whole content
/// once opened and `TaxiFrame` is `toplevel` with its own close. The window is
/// a conversation with an NPC: it is the only thing `UnitName("npc")` names
/// while it is open, and a press in it sends a packet the server refuses from
/// out of range. `CanInteractWithNPC` gates every `CMSG_TRAINER_LIST` and every
/// `CMSG_TAXIQUERYAVAILABLENODES`, and nothing announces the walk-away.
pub(crate) fn close_when_npc_gone<W: NpcWindow>(
    units: super::api::Units,
    mut window: ResMut<W>,
    mut closed: MessageWriter<W::Closed>,
) {
    let Some(guid) = window.guid() else {
        return;
    };
    // An item can be a quest giver and has no position, and this check is about
    // distance. `HandleQuestgiverQueryQuestOpcode` accepts an item guid as well
    // as a creature's, so `UseContainerItem` on a "This Item Begins a Quest"
    // item opens a real quest page whose giver is an item in a bag. Without
    // this test the walk-away rule below reads "the guid resolves to nothing
    // at all", which is permanently true for an item, and raises
    // `QUEST_FINISHED` on the next frame. `QuestFrame_OnEvent` answers that
    // with an unconditional `HideUIPanel`, so the page opened and closed within
    // one frame.
    if !vale_protocol::state::objects::guid_is_in_the_world(guid) {
        return;
    }
    let gone = match W::reach(&units, guid) {
        Some(reach) => reach > vale_protocol::play::gossip::INTERACTION_DISTANCE,
        None => !units.in_view(guid),
    };
    if gone && window.close() {
        closed.write(W::CLOSED);
    }
}

/// The distance between the centres of the character and the object `guid`
/// names, or `None` while either is unplaced or absent.
///
/// The popups that outlive their gossip window (the innkeeper's bind, the pet
/// untrainer's question) and the mailbox measure this rather than
/// [`super::api::Units::reach_to_guid`]: their range is from centre to
/// centre.
pub(crate) fn reach_to(units: &super::api::Units, guid: u64) -> Option<f32> {
    let (_, here) = units.placed(super::api::UnitId::Player)?;
    let (_, _, there) = units.all.iter().find(|(_, unit, _)| unit.guid == guid)?;
    Some(here.translation.distance(there?.translation))
}

/// How far the innkeeper and the pet untrainer may be while their question is
/// up: `CheckBinderDist()` and `CheckPetUntrainerDist()`.
///
/// The server's own interaction distance, not a reading of the client's: every
/// gossip verb goes through `GetNPCIfCanInteractWith`, which is
/// `IsWithinDistInMap(pl, INTERACTION_DISTANCE)` at 5.0 yards plus the two
/// units' bounding radii. The client's own number was not measured, so this
/// is the distance at which the request would be refused rather than the one
/// at which the client takes the box away; they are the same rule in
/// practice, because a popup that stayed up past it would send a packet the
/// server drops.
pub(crate) const POPUP_RANGE: f32 = 10.0;

/// Points the `"npc"` token at the unit the character is talking to: the
/// claim with the lowest [`NpcToken::ORDER`].
///
/// The guid is derived and not written; see the module note. The orders put
/// the most recent kind of conversation first: a quest page replaces the
/// gossip menu that opened it on the same NPC.
fn point_the_token(claims: Res<TokenClaims>, mut npc: ResMut<NpcUnit>) {
    let pointed = claims.claims.values().next().copied();
    // Change-gated: a `ResMut` dereferenced every frame is a resource marked
    // changed every frame, and `Units` reads this.
    if npc.0 != pointed {
        npc.0 = pointed;
    }
}

impl NpcWindow for GossipWindow {
    type Closed = GossipClosed;
    const CLOSED: GossipClosed = GossipClosed;

    fn guid(&self) -> Option<u64> {
        GossipWindow::guid(self)
    }

    fn close(&mut self) -> bool {
        self.menu.take().is_some()
    }
}

/// Second, after the quest page, which replaces the menu that opened it.
impl NpcToken for GossipWindow {
    const ORDER: u8 = 1;

    fn npc(&self) -> Option<u64> {
        GossipWindow::guid(self)
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
