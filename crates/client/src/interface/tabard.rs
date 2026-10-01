//! The tabard designer's half of the session: two packets in, two requests
//! out, and four events.
//!
//! The designer's state is the board on the Lua host; see
//! [`crate::lua::panels::tabard`]. [`vale_protocol::play::guild`] has the two
//! packets. This file is the systems between the two:
//!
//! ```text
//! track          the guild's saved emblem, the character's rank and money
//! apply_answers  MSG_TABARDVENDOR_ACTIVATE and the answer to a save
//! walk_away      the window, when the designer is out of reach
//! act            what the interface pressed
//! preview        the design, for the model in the window
//! ```
//!
//! ## The window is a conversation with an NPC
//!
//! `MSG_TABARDVENDOR_ACTIVATE` opens it, the `"npc"` token names the
//! designer while it is open, and walking away closes it; see
//! [`TabardWindow`] and [`super::gossip`]. Closing sends nothing.
//!
//! ## When the Accept button is enabled
//!
//! `TabardFrame_UpdateButtons` runs on `TABARD_CANSAVE_CHANGED` and
//! `TABARD_SAVE_PENDING` and reads `TabardModel:CanSaveTabardNow()`, which
//! is true while the guild's record is held and no save is waiting.
//! [`track`] raises the first event when that answer changes.
//! `TABARD_SAVE_PENDING` is raised when a save is sent and when the server
//! refuses one.
//!
//! ## A saved emblem arrives as a guild query answer
//!
//! The server answers a save with a result word, and after a success sends
//! the saver the guild's query response, which carries the new emblem. That
//! answer updates the guild board, and [`track`] copies the emblem here. A
//! guild query is also sent on success, for a server that does not send the
//! response unasked.
//!
//! ## The preview
//!
//! `TabardModel` is a `<PlayerModel>` showing the character. While the
//! window is open [`TabardPreview`] holds the design, and
//! [`crate::render::paperdoll`] draws the character with it.
//!
//! Not done: the 1.12.1 client refuses to open the designer for a
//! shapeshifted character and prints `ERR_EMBLEMERROR_NOTABARDGEOSET`.

use bevy::prelude::*;

use super::events::{CloseTabardFrame, OpenTabardFrame, TabardCansaveChanged, TabardSavePending};
use crate::lua::panels::tabard::Press;
use crate::world::session::Session;
use vale_protocol::play::guild::{emblem_result, Emblem};
use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::socket::session::GuildVerb;

/// One of the designer's two packets, forwarded by
/// [`crate::world::incoming::drain_events`].
#[derive(Message, Debug, Clone, Copy)]
pub enum TabardAnswer {
    /// `MSG_TABARDVENDOR_ACTIVATE`: the designer's guid.
    Vendor(u64),
    /// `MSG_SAVE_GUILD_EMBLEM`: one of [`emblem_result`].
    Result(u32),
}

/// The tabard designer the window is open at, or nothing. Read by
/// [`super::gossip`] for the `"npc"` token.
#[derive(Resource, Default)]
pub struct TabardWindow(Option<u64>);

impl TabardWindow {
    pub fn guid(&self) -> Option<u64> {
        self.0
    }

    /// Take the window down, answering whether one was up.
    pub(crate) fn close(&mut self) -> bool {
        self.0.take().is_some()
    }
}

/// The design the open window holds, for the model that previews it. `None`
/// while the window is shut.
#[derive(Resource, Default, PartialEq, Eq)]
pub struct TabardPreview(pub Option<Emblem>);

pub struct TabardPlugin;

impl Plugin for TabardPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TabardAnswer>()
            .init_resource::<TabardWindow>()
            .init_resource::<TabardPreview>()
            .add_systems(
                Update,
                (track, apply_answers, walk_away, act, preview)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// Copy the guild's saved emblem, the character's rank and its money to the
/// board, and raise `TABARD_CANSAVE_CHANGED` when the answer to
/// `CanSaveTabardNow` changes.
fn track(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    inventory: Res<super::items::Inventory>,
    mut could: Local<bool>,
    mut changed: MessageWriter<TabardCansaveChanged>,
) {
    let Some(host) = host else {
        *could = false;
        return;
    };
    let (emblem, rank) = {
        let guild = host.guild().borrow();
        (guild.own_emblem(), guild.rank())
    };
    let mut board = host.tabard().borrow_mut();
    board.guild = emblem;
    board.rank = rank;
    board.money = inventory.money;
    let can = board.can_save();
    if can != *could {
        *could = can;
        changed.write(TabardCansaveChanged);
    }
}

/// Fold the two packets into the board, the sentence and the events.
fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut answers: MessageReader<TabardAnswer>,
    mut window: ResMut<TabardWindow>,
    mut say: super::messages::Announce,
    mut open: MessageWriter<OpenTabardFrame>,
    mut pending: MessageWriter<TabardSavePending>,
) {
    let Some(host) = host else {
        answers.clear();
        return;
    };
    for answer in answers.read() {
        match *answer {
            TabardAnswer::Vendor(npc) => {
                host.tabard().borrow_mut().vendor = Some(npc);
                window.0 = Some(npc);
                open.write(OpenTabardFrame);
            }
            // A result outside the six is ignored, and the save stays
            // pending.
            TabardAnswer::Result(result) if result <= emblem_result::FAIL_NO_MESSAGE => {
                if let Some(key) = emblem_result::message(result) {
                    say.key(key);
                }
                host.tabard().borrow_mut().pending = false;
                if result == emblem_result::SUCCESS {
                    // The new emblem is the guild's query answer.
                    let guild = host.guild().borrow().guild_id();
                    if let (true, Some(active)) = (guild != 0, session.active.as_ref()) {
                        active.live.guild(GuildVerb::Query(guild));
                    }
                } else {
                    pending.write(TabardSavePending);
                }
            }
            TabardAnswer::Result(_) => {}
        }
    }
}

/// Close the window when the designer is out of reach or gone. The test is
/// the one every NPC window uses, and it applies only while the `"npc"` token
/// names this designer.
fn walk_away(
    units: super::api::Units,
    npc: Res<super::gossip::NpcUnit>,
    mut window: ResMut<TabardWindow>,
    mut closed: MessageWriter<CloseTabardFrame>,
) {
    let Some(guid) = window.guid() else { return };
    if npc.0 == Some(guid) && super::gossip::npc_gone(&units) && window.close() {
        closed.write(CloseTabardFrame);
    }
}

/// Act on what the interface pressed.
fn act(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut window: ResMut<TabardWindow>,
    mut say: super::messages::Announce,
    mut closed: MessageWriter<CloseTabardFrame>,
    mut pending: MessageWriter<TabardSavePending>,
) {
    let Some(mut host) = host else { return };
    // The window was closed by `walk_away`.
    if window.guid().is_none() && host.tabard().borrow().vendor.is_some() {
        host.tabard().borrow_mut().vendor = None;
    }
    for press in host.take_tabard_presses() {
        match press {
            Press::Refuse(key) => say.key(key),
            Press::Close => {
                host.tabard().borrow_mut().vendor = None;
                if window.close() {
                    closed.write(CloseTabardFrame);
                }
            }
            Press::Save(emblem) => {
                let (Some(npc), Some(active)) = (window.guid(), session.active.as_ref()) else {
                    // Nothing was sent, so nothing is waiting for an answer.
                    host.tabard().borrow_mut().pending = false;
                    continue;
                };
                active.live.guild(GuildVerb::SaveEmblem { npc, emblem });
                pending.write(TabardSavePending);
            }
        }
    }
}

/// Publish the design while the window is open. Written only on a change,
/// so a reader can use change detection.
fn preview(
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    window: Res<TabardWindow>,
    mut shown: ResMut<TabardPreview>,
) {
    let design = host
        .filter(|_| window.guid().is_some())
        .map(|host| host.tabard().borrow().design);
    if shown.0 != design {
        shown.0 = design;
    }
}

/// The arm [`crate::world::incoming`] calls for the designer's two events.
pub fn answer_of(event: &PlayerEvent) -> Option<TabardAnswer> {
    Some(match event {
        PlayerEvent::TabardVendor(npc) => TabardAnswer::Vendor(*npc),
        PlayerEvent::GuildEmblemResult(result) => TabardAnswer::Result(*result),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The designer's two events are forwarded and nothing else is.
    #[test]
    fn only_the_designers_events_become_answers() {
        assert!(matches!(
            answer_of(&PlayerEvent::TabardVendor(0x99)),
            Some(TabardAnswer::Vendor(0x99))
        ));
        assert!(matches!(
            answer_of(&PlayerEvent::GuildEmblemResult(4)),
            Some(TabardAnswer::Result(4))
        ));
        assert!(answer_of(&PlayerEvent::PetitionTurnInResult(0)).is_none());
    }

    /// The designer's window reports being closed once.
    #[test]
    fn the_window_closes_once() {
        let mut window = TabardWindow(Some(0x99));
        assert!(window.close());
        assert!(!window.close());
    }
}
