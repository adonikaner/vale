//! **The trade window** — handing something to another player, and the ten
//! verbs `TradeFrame.lua` and the `TRADE` popup are written against.
//!
//! The client's half of [`vale_protocol::play::trade`]. The split every
//! window in this crate keeps: the wire is there, the panel is
//! [`crate::lua::panels::trade`], and what is left here is the state one
//! trade is — who, whether the window is up, both offers, and the two accept
//! flags — plus the events that tell the interface it moved.
//!
//! ```text
//! SMSG_TRADE_STATUS BEGIN_TRADE <guid>   -> CMSG_BEGIN_TRADE at once (CMSG_IGNORE_TRADE under BlockTrades)
//! SMSG_TRADE_STATUS OPEN_WINDOW          -> TRADE_SHOW             TradeFrame opens on both sides
//! ClickTradeButton(i) / SetTradeMoney    -> TRADE_PLAYER_ITEM_CHANGED <i> / PLAYER_TRADE_MONEY, and the packet
//! SMSG_TRADE_STATUS_EXTENDED (theirs)    -> TRADE_TARGET_ITEM_CHANGED <i> x7, TRADE_MONEY_CHANGED
//! SMSG_TRADE_STATUS_EXTENDED (mine)      -> the same for our side, which only a spell aimed at it sends
//! SMSG_TRADE_STATUS TRADE_ACCEPT         -> TRADE_ACCEPT_UPDATE <mine> 1
//! SMSG_TRADE_STATUS BACK_TO_TRADE        -> TRADE_ACCEPT_UPDATE 0 0
//! SMSG_TRADE_STATUS <anything else>      -> TRADE_CLOSED, and its sentence
//! ```
//!
//! ## There is no "Trade with X?" popup, and the request is answered at once
//!
//! `StaticPopupDialogs["TRADE"]` exists in the shipped files and the real
//! client never shows it: a `BEGIN_TRADE` is answered with `CMSG_BEGIN_TRADE`
//! the moment it lands, and the window opens on both sides together — which
//! is what a player at a 5875 sees, and what was reported when this client
//! put the popup up instead. The one refusal is the `BlockTrades` CVar, the
//! options panel's own row, which answers `CMSG_IGNORE_TRADE` and says
//! `ERR_TRADE_BLOCKED_S`. `TRADE_REQUEST` and `TRADE_REQUEST_CANCEL` are
//! therefore declared and never raised.
//!
//! ## Our own offer is drawn here, because the server does not echo it
//!
//! `TradeData::SetItem` and `SetMoney` call `Update()` with its default
//! `for_trader = true`, which sends the offer to the *partner* only; the
//! echo with `theirs = 0` is sent by `SetSpell` alone, for the enchant. So a
//! client that waited for an echo drew its own squares empty while the
//! partner saw them filled — the second report. `ClickTradeButton` and
//! `SetTradeMoney` therefore write our side of the window locally, off the
//! bag square the cursor came from, and raise the two events themselves; an
//! echo that does arrive replaces it, since it is the server's own view.
//!
//! The partner's side is never predicted, and neither is either accept
//! flag beyond our own: `TradeFrame_SetAcceptState` is what disables the
//! Trade button, and the server never restates the presser's own flag (it
//! tells the *partner*), so `AcceptTrade()` raises `TRADE_ACCEPT_UPDATE`
//! here with the partner's flag as last heard.
//!
//! ## The `"NPC"` token names the partner while the window is up
//!
//! `TradeFrame_Update` draws the far side with `UnitName("NPC")` and
//! `SetPortraitTexture(…, "NPC")` — the same token every conversation window
//! uses — so [`crate::game::npc::gossip`]'s derivation of that token takes
//! [`TradeWindow::partner`] after the five NPC windows, and a trade partner
//! is a person with a face on the plate.
//!
//! ## `CloseTrade` is the window hiding, not a verb
//!
//! `TradeFrame_OnHide` calls it, and `TRADE_CLOSED` hides the frame, so a
//! close that came *from* the server would answer the server's own close with
//! a `CMSG_CANCEL_TRADE` about a trade that no longer exists — harmless
//! (`TradeCancel` on a null trade is a no-op) and noisy. So the window is
//! cleared here first and the packet goes only while one is still open.

use vale_protocol::play::spells::CastTarget;
use bevy::prelude::*;

use vale_protocol::play::trade::{
    TradeItem, TradeOffer, TradeStatus, TradeStatusPacket, TRADE_SLOT_COUNT,
};
use vale_protocol::socket::session::TradeVerb;

use super::super::events::{
    PlayerTradeMoney, TradeAcceptUpdate, TradeClosed, TradeMoneyChanged,
    TradePlayerItemChanged, TradeShow, TradeTargetItemChanged,
};
use crate::world::session::Session;

/// One of the two trade packets, handed on from [`super::super::incoming`].
#[derive(Message, Debug, Clone)]
pub enum TradeAnswer {
    Status(TradeStatusPacket),
    Offer(Box<TradeOffer>),
}

/// A press the interface made — the ten verbs, less the two refusals nobody
/// in `Interface\FrameXML\` calls.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TradePress {
    /// `InitiateTrade(unit)` — the popup menu's Trade line, by token.
    Initiate(String),
    /// `BeginTrade()` — the `TRADE` popup's Yes.
    Begin,
    /// `CancelTrade()` — its No.
    Cancel,
    /// `AcceptTrade()` — the Trade button.
    Accept,
    /// `CancelTradeAccept()` — the Cancel button while accepted.
    Unaccept,
    /// `CloseTrade()` — `TradeFrame_OnHide`.
    Close,
    /// `ClickTradeButton(id)` — a square on our side, 1..7.
    ClickSlot(u8),
    /// `ClickTargetTradeButton(id)` — a square on **their** side, which is the
    /// enchant door and nothing else. See the `act` arm.
    ClickTheirSlot,
    /// `SetTradeMoney(copper)` — the money box, on every change.
    SetMoney(u32),
}

/// **The enchant square was clicked with a spell waiting** — the trade
/// window's own door onto a cast.
///
/// The third of the three [`crate::game::combat::action::Asked`] already
/// carries, and it is written here rather than acted on for the same reason the
/// other two are: every cast in this client goes out through one place, and this
/// module has no bar, no cooldowns and no error frame.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSlotPicked {
    /// Always `TRADE_SLOT_NONTRADED` — the server refuses every other square
    /// with `SPELL_FAILED_ITEM_NOT_READY`, so the number is carried rather than
    /// assumed at the far end.
    pub trade_slot: u8,
}

/// What one open trade is.
struct Open {
    partner: u64,
    mine: TradeOffer,
    theirs: TradeOffer,
    my_accept: bool,
    their_accept: bool,
    /// The last `CMSG_SET_TRADE_GOLD` sent, so the money box's
    /// `OnValueChanged` — which fires on every keystroke and on the frame's
    /// own `OnShow` — does not send the same number twice.
    money_sent: u32,
}

/// The trade, or the request for one, or nothing.
#[derive(Resource, Default)]
pub struct TradeWindow {
    /// `BEGIN_TRADE` arrived: who asked, until the popup answers or the
    /// window opens.
    request: Option<u64>,
    /// We asked: who, until the window opens or a refusal lands.
    asked: Option<u64>,
    open: Option<Open>,
}

impl TradeWindow {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Who the window is with — the `"NPC"` token's answer while it is up.
    pub fn partner(&self) -> Option<u64> {
        Some(self.open.as_ref()?.partner)
    }

    /// One side's offer, or `None` with no window.
    pub fn offer(&self, theirs: bool) -> Option<&TradeOffer> {
        let open = self.open.as_ref()?;
        Some(if theirs { &open.theirs } else { &open.mine })
    }

    /// Our own accept flag, as the interface last heard it.
    pub fn my_accept(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.my_accept)
    }

    pub fn their_accept(&self) -> bool {
        self.open.as_ref().is_some_and(|open| open.their_accept)
    }

    /// The window, for a test: open with a partner and nothing in it.
    #[cfg(test)]
    pub(crate) fn open_for_test(&mut self, partner: u64) {
        self.open = Some(Open::with(partner));
    }
}

impl Open {
    fn with(partner: u64) -> Open {
        Open {
            partner,
            mine: TradeOffer::default(),
            theirs: TradeOffer::default(),
            my_accept: false,
            their_accept: false,
            money_sent: 0,
        }
    }
}

pub struct TradePlugin;

impl Plugin for TradePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TradeAnswer>()
            .add_message::<TradePress>()
            .add_message::<TradeSlotPicked>()
            .init_resource::<TradeWindow>()
            .add_systems(
                Update,
                (answers, presses, act)
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// Fold what the server said into the window, and tell the interface.
#[allow(clippy::too_many_arguments)]
fn answers(
    mut incoming: MessageReader<TradeAnswer>,
    mut window: ResMut<TradeWindow>,
    session: Res<Session>,
    cvars: Res<super::super::cvars::CVars>,
    units: super::super::api::Units,
    mut say: super::super::messages::Announce,
    mut shown: MessageWriter<TradeShow>,
    mut closed: MessageWriter<TradeClosed>,
    mut accept: MessageWriter<TradeAcceptUpdate>,
    mut player_item: MessageWriter<TradePlayerItemChanged>,
    mut target_item: MessageWriter<TradeTargetItemChanged>,
    mut player_money: MessageWriter<PlayerTradeMoney>,
    mut target_money: MessageWriter<TradeMoneyChanged>,
) {
    for answer in incoming.read() {
        match answer {
            TradeAnswer::Status(packet) => {
                let Some(status) = packet.status else {
                    continue;
                };
                match status {
                    // **Answered at once** — see the module note. The
                    // `BlockTrades` CVar is the one refusal, and it is the
                    // asker's name the sentence wants.
                    TradeStatus::BeginTrade => {
                        let Some(who) = packet.partner else { continue };
                        window.request = Some(who);
                        let Some(active) = session.active.as_ref() else { continue };
                        if cvars.flag("BlockTrades") {
                            window.request = None;
                            active.live.trade(TradeVerb::Ignore);
                            let name = units.name_of_guid(who).unwrap_or_default();
                            say.formatted("ERR_TRADE_BLOCKED_S", &name);
                            continue;
                        }
                        active.live.trade(TradeVerb::Begin);
                    }
                    TradeStatus::OpenWindow => {
                        let partner = window
                            .request
                            .take()
                            .or_else(|| window.asked.take())
                            .unwrap_or(0);
                        window.open = Some(Open::with(partner));
                        shown.write(TradeShow);
                    }
                    TradeStatus::TradeAccept => {
                        if let Some(open) = window.open.as_mut() {
                            open.their_accept = true;
                            accept.write(TradeAcceptUpdate {
                                player: open.my_accept,
                                target: true,
                            });
                        }
                    }
                    // Both off — see the module note in
                    // [`vale_protocol::play::trade`].
                    TradeStatus::BackToTrade => {
                        if let Some(open) = window.open.as_mut() {
                            open.my_accept = false;
                            open.their_accept = false;
                            accept.write(TradeAcceptUpdate {
                                player: false,
                                target: false,
                            });
                        }
                    }
                    other => {
                        debug_assert!(other.ends_trade());
                        let partner = window
                            .open
                            .as_ref()
                            .map(|open| open.partner)
                            .or(window.request)
                            .or(window.asked);
                        if window.open.take().is_some() {
                            closed.write(TradeClosed);
                        }
                        window.request = None;
                        window.asked = None;
                        if let Some(key) = other.key() {
                            if key.ends_with("_S") {
                                let name = partner
                                    .and_then(|guid| units.name_of_guid(guid))
                                    .unwrap_or_default();
                                say.formatted(key, &name);
                            } else {
                                say.key(key);
                            }
                        }
                    }
                }
            }
            TradeAnswer::Offer(offer) => {
                let Some(open) = window.open.as_mut() else {
                    continue;
                };
                let side = if offer.theirs {
                    &mut open.theirs
                } else {
                    &mut open.mine
                };
                let before = std::mem::replace(side, (**offer).clone());
                for slot in 0..TRADE_SLOT_COUNT {
                    if before.items[slot] == side.items[slot] {
                        continue;
                    }
                    let id = slot as u8 + 1;
                    if offer.theirs {
                        target_item.write(TradeTargetItemChanged(id));
                    } else {
                        player_item.write(TradePlayerItemChanged(id));
                    }
                }
                if before.money != side.money {
                    if offer.theirs {
                        target_money.write(TradeMoneyChanged);
                    } else {
                        player_money.write(PlayerTradeMoney);
                    }
                }
            }
        }
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<TradePress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_trade_presses() {
        out.write(press);
    }
}

/// …and act on it: nine packets, one local clear, and our own side of the
/// window drawn as it is sent — see the module note.
#[allow(clippy::too_many_arguments)]
fn act(
    mut presses: MessageReader<TradePress>,
    mut window: ResMut<TradeWindow>,
    session: Res<Session>,
    units: super::super::api::Units,
    inventory: Res<super::super::character::items::Inventory>,
    mut cursor: ResMut<super::super::combat::cursor::Cursor>,
    mut accept: MessageWriter<TradeAcceptUpdate>,
    mut player_item: MessageWriter<TradePlayerItemChanged>,
    mut player_money: MessageWriter<PlayerTradeMoney>,
    // **The spell cursor and the cast an item started**, for the one square in
    // the window that is not a square for items — see the `ClickSlot` arm.
    targeting: Res<super::super::combat::action::SpellTargeting>,
    mut pending: ResMut<super::super::character::items::PendingItemCast>,
    mut enchant: MessageWriter<TradeSlotPicked>,
) {
    for press in presses.read() {
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        match press {
            TradePress::Initiate(token) => {
                let Some(guid) = super::super::api::UnitId::parse(token)
                    .and_then(|id| units.get(id))
                    .map(|unit| unit.guid)
                else {
                    continue;
                };
                window.asked = Some(guid);
                active.live.trade(TradeVerb::Initiate(guid));
            }
            TradePress::Begin => {
                if window.request.is_some() {
                    active.live.trade(TradeVerb::Begin);
                }
            }
            TradePress::Cancel => {
                // The popup's No, or `CancelTrade` typed. The server answers
                // both sides with `TRADE_CANCELED`, which is what closes a
                // window that is up; the request is cleared here because the
                // popup that held it is already gone.
                window.request = None;
                window.asked = None;
                active.live.trade(TradeVerb::Cancel);
            }
            TradePress::Accept => {
                if let Some(open) = window.open.as_mut() {
                    open.my_accept = true;
                    accept.write(TradeAcceptUpdate {
                        player: true,
                        target: open.their_accept,
                    });
                    active.live.trade(TradeVerb::Accept);
                }
            }
            TradePress::Unaccept => {
                if let Some(open) = window.open.as_mut() {
                    open.my_accept = false;
                    accept.write(TradeAcceptUpdate {
                        player: false,
                        target: open.their_accept,
                    });
                    active.live.trade(TradeVerb::Unaccept);
                }
            }
            // See the module note: cleared first, sent only if it was up.
            TradePress::Close => {
                if window.open.take().is_some() {
                    active.live.trade(TradeVerb::Cancel);
                }
            }
            TradePress::ClickSlot(id) => {
                let Some(open) = window.open.as_mut() else { continue };
                let Some(trade_slot) = id.checked_sub(1).filter(|s| usize::from(*s) < TRADE_SLOT_COUNT)
                else {
                    continue;
                };
                let square = usize::from(trade_slot);
                // **With something held, the square takes it**: the packet
                // names the bag square the cursor came from, and the cursor
                // lets go — the item stays in the bag, which is what the
                // reference draws too. With nothing held, a filled square is
                // emptied; an empty one is nothing. Our side of the window
                // is written here as well, because the server will not say
                // it back — see the module note.
                match cursor.item().cloned() {
                    Some(held) => {
                        cursor.held = None;
                        let Some((bag, slot)) = held.from.server() else { continue };
                        active.live.trade(TradeVerb::SetItem {
                            trade_slot,
                            bag,
                            slot,
                        });
                        let carried = match held.from {
                            super::super::combat::cursor::Place::Container { bag, slot } => {
                                inventory.carried.container_item(bag, usize::from(slot))
                            }
                            super::super::combat::cursor::Place::Inventory(id) => {
                                inventory.carried.inventory_slot(id)
                            }
                        };
                        open.mine.items[square] = Some(TradeItem {
                            entry: held.entry,
                            display_id: inventory
                                .template(held.entry)
                                .map_or(0, |t| t.display_id),
                            count: carried.map_or(held.count, |item| item.count),
                            durability: carried.map_or(0, |item| item.durability),
                            ..TradeItem::default()
                        });
                        player_item.write(TradePlayerItemChanged(*id));
                    }
                    None => {
                        if cursor.held.is_none() && open.mine.items[square].is_some() {
                            active.live.trade(TradeVerb::ClearItem { trade_slot });
                            open.mine.items[square] = None;
                            player_item.write(TradePlayerItemChanged(*id));
                        }
                    }
                }
            }
            // **The partner's square is where an enchant is aimed**, and it is
            // the only thing that square does. The reference reads the spell
            // cursor *before* it reads the button's number and answers it
            // whatever square was clicked — so the slot sent here is
            // `TRADE_SLOT_NONTRADED` rather than the one under the pointer, and
            // the server refuses every other one outright
            // (`SPELL_FAILED_ITEM_NOT_READY`).
            //
            // **The partner's square, not ours**, and that is the direction the
            // server states: `TradeHandler`'s completion applies *my* stored
            // spell to *his* non-traded item. The recipient puts their item in
            // their own square and the enchanter clicks it from the other side.
            //
            // Two doors reach it, the same two a bag square has: a spell from
            // the book waiting for an item, and a cast an *item* started — an
            // oil, a sharpening stone, a scroll. The second is the pending one,
            // and it is taken here for the same reason `character::items` takes
            // it: a question is answered once.
            //
            // Nothing visible happens on success. The server stores the spell
            // against the trade and answers `SPELL_FAILED_DONT_REPORT`; the
            // echo arrives as `SMSG_TRADE_STATUS_EXTENDED`'s `spell` field,
            // which is what fills the square's "Will receive %s." line.
            TradePress::ClickTheirSlot => {
                if window.open.is_none() || !targeting.wants_item() {
                    continue;
                }
                let trade_slot = vale_protocol::play::trade::TRADE_SLOT_NONTRADED;
                match pending.take() {
                    Some(from) => {
                        active.live.use_item(
                            from.bag,
                            from.slot,
                            Some(from.spell_index),
                            CastTarget::TradeSlot(trade_slot),
                        );
                    }
                    None => {
                        enchant.write(TradeSlotPicked { trade_slot });
                    }
                }
            }
            TradePress::SetMoney(copper) => {
                if let Some(open) = window.open.as_mut() {
                    if open.money_sent != *copper {
                        open.money_sent = *copper;
                        open.mine.money = *copper;
                        active.live.trade(TradeVerb::SetGold(*copper));
                        player_money.write(PlayerTradeMoney);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::trade::TradeItem;

    fn harness() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        crate::game::events::register(&mut app);
        app.init_resource::<Session>()
            .init_resource::<crate::game::cvars::CVars>()
            .init_resource::<crate::game::messages::UiStrings>()
            .init_resource::<crate::game::combat::target::Selection>()
            .init_resource::<crate::game::combat::target::Hovered>()
            .init_resource::<crate::game::npc::gossip::NpcUnit>()
            .init_resource::<crate::game::character::reputation::PlayerStanding>()
            .init_resource::<crate::game::session::party::Party>()
            .add_message::<TradeAnswer>()
            .init_resource::<TradeWindow>()
            .add_systems(Update, answers);
        crate::game::messages::Announce::register(&mut app);
        app
    }

    fn status(app: &mut App, code: u32, partner: Option<u64>) {
        app.world_mut().write_message(TradeAnswer::Status(TradeStatusPacket {
            code,
            status: TradeStatus::from_code(code),
            partner,
        }));
        app.update();
    }

    /// Everything raised so far, **taken**: `iter_current_update_messages`
    /// answers over a double buffer that the minimal plugins never swap, so a
    /// read that did not drain would see last frame's again.
    fn raised<T: Message + Clone>(app: &mut App) -> Vec<T> {
        app.world_mut()
            .resource_mut::<Messages<T>>()
            .drain()
            .collect()
    }

    /// **The whole state machine, from the asked side**: the request is
    /// remembered and answered rather than shown, the open is a window with
    /// the asker as partner, an accept from the far side is one flag,
    /// `BACK_TO_TRADE` is both off, and a completion is a close with its own
    /// sentence. (No session in the harness, so the answer is not sent and
    /// the partner is the request that was remembered.)
    #[test]
    fn a_request_opens_and_a_completion_closes() {
        let mut app = harness();
        status(&mut app, TradeStatus::BeginTrade as u32, Some(0x77));
        assert!(!app.world().resource::<TradeWindow>().is_open());

        status(&mut app, TradeStatus::OpenWindow as u32, None);
        assert_eq!(raised::<TradeShow>(&mut app).len(), 1);
        assert_eq!(app.world().resource::<TradeWindow>().partner(), Some(0x77));

        status(&mut app, TradeStatus::TradeAccept as u32, None);
        let flags = raised::<TradeAcceptUpdate>(&mut app);
        assert_eq!(flags.len(), 1);
        assert!(!flags[0].player);
        assert!(flags[0].target);

        status(&mut app, TradeStatus::BackToTrade as u32, None);
        let flags = raised::<TradeAcceptUpdate>(&mut app);
        assert!(!flags[0].player && !flags[0].target);

        status(&mut app, TradeStatus::TradeComplete as u32, None);
        assert_eq!(raised::<TradeClosed>(&mut app).len(), 1);
        assert!(!app.world().resource::<TradeWindow>().is_open());
    }

    /// **A refusal with no window up closes nothing**, and a refusal after
    /// one does — once.
    #[test]
    fn a_refusal_takes_down_whichever_is_up() {
        let mut app = harness();
        status(&mut app, TradeStatus::TargetTooFar as u32, None);
        assert!(raised::<TradeClosed>(&mut app).is_empty());

        app.world_mut().resource_mut::<TradeWindow>().open_for_test(0x77);
        status(&mut app, TradeStatus::TradeCanceled as u32, None);
        assert_eq!(raised::<TradeClosed>(&mut app).len(), 1);
        status(&mut app, TradeStatus::TradeCanceled as u32, None);
        assert!(raised::<TradeClosed>(&mut app).is_empty());
    }

    /// **An offer says which slots moved and whose money**, and the echo of
    /// our own offer is the player's side.
    #[test]
    fn an_offer_raises_one_event_per_slot_that_changed() {
        let mut app = harness();
        app.world_mut().resource_mut::<TradeWindow>().open_for_test(0x77);
        let mut offer = TradeOffer {
            theirs: true,
            money: 500,
            ..TradeOffer::default()
        };
        offer.items[2] = Some(TradeItem {
            entry: 2589,
            count: 20,
            ..TradeItem::default()
        });
        app.world_mut().write_message(TradeAnswer::Offer(Box::new(offer.clone())));
        app.update();
        assert_eq!(raised::<TradeTargetItemChanged>(&mut app), vec![TradeTargetItemChanged(3)]);
        assert_eq!(raised::<TradeMoneyChanged>(&mut app).len(), 1);
        assert!(raised::<TradePlayerItemChanged>(&mut app).is_empty());

        // The same offer again moves nothing.
        app.world_mut().write_message(TradeAnswer::Offer(Box::new(offer.clone())));
        app.update();
        assert!(raised::<TradeTargetItemChanged>(&mut app).is_empty());

        offer.theirs = false;
        app.world_mut().write_message(TradeAnswer::Offer(Box::new(offer)));
        app.update();
        assert_eq!(raised::<TradePlayerItemChanged>(&mut app), vec![TradePlayerItemChanged(3)]);
        assert_eq!(raised::<PlayerTradeMoney>(&mut app).len(), 1);
        let window = app.world().resource::<TradeWindow>();
        assert_eq!(window.offer(false).map(|o| o.money), Some(500));
        assert_eq!(window.offer(true).and_then(|o| o.items[2].as_ref()).map(|i| i.count), Some(20));
    }
}
