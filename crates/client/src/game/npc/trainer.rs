//! **The training window** — what an NPC will teach, and buying one line of it.
//!
//! The client's half of [`vale_protocol::play::trainer`]. The split is the one
//! every panel in this directory keeps: [`vale_assets::tables::trainer`] is the
//! *rule* — which header a service goes under, in what order, and which of the
//! three filters hides it — because none of that needs a session; what is left
//! here is the three things that do.
//!
//! ```text
//! which services   SMSG_TRAINER_LIST, which arrives whole and is replaced whole
//! which character  race and class off UNIT_FIELD_BYTES_0, which decide the
//!                  skill line every row is grouped by
//! what a press does  CMSG_TRAINER_BUY_SPELL, and the two answers to it
//! ```
//!
//! ## The panel is a load-on-demand addon, and it is loaded eagerly
//!
//! 1.12 ships the trainer window as `Interface\AddOns\Blizzard_TrainerUI\`
//! rather than in `FrameXML`, and `UIParent.lua` reaches it through
//! `UIParentLoadAddOn("Blizzard_TrainerUI")` on `TRAINER_SHOW`. This client
//! loads it beside `FrameXML` at login and answers `LoadAddOn` for that one
//! name — see `crate::lua::host`. **A stated departure**: the reference defers
//! the load to save memory, and nothing else about the addon is different.
//!
//! ## Buying is gated twice, and the near gate is the client's own
//!
//! The client tests the service's state byte against zero *before* it builds
//! the packet and returns silently when the row is not green — so a red row's
//! Train button sends nothing at all. The server refuses it too
//! (`TRAIN_FAIL_NOT_ENOUGH_SKILL`), which is what makes the near gate safe to
//! keep: it can only ever suppress a packet the server was going to throw away.
//!
//! ## What a purchase changes is read locally
//!
//! `SMSG_TRAINER_BUY_SUCCEEDED` names the service and nothing else; the spell
//! itself arrives as an ordinary `SMSG_LEARNED_SPELL`, and **no packet resends
//! the list** — there is no `push $0x1b0` anywhere in the trainer module, so
//! the reference does not re-ask either. The row is marked `used` here and
//! `TRAINER_UPDATE` is raised, which is what turns it grey. That is a
//! **reading**: the effect matches the game, but the reference's own handler
//! for that opcode was not followed.

use bevy::prelude::*;

use vale_assets::tables::trainer::{Board, Kind, Service, State, Tables};
use vale_protocol::socket::session::NpcVerb;
use vale_protocol::play::trainer::{TrainFailure, TrainerList, TrainerState, TrainerType};

use super::super::events::{TrainerClosed, TrainerShow, TrainerUpdate};
use crate::assets::GameAssets;
use crate::world::session::{LocalPlayer, Session, WorldEntity};

/// One of the three trainer packets, handed on from [`super::super::combat::action::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum TrainerAnswer {
    Show(Box<TrainerList>),
    /// `SMSG_TRAINER_BUY_SUCCEEDED` — by the **service** spell id.
    Bought { spell: u32 },
    BuyFailed {
        spell: u32,
        reason: Option<TrainFailure>,
    },
}

/// The open trainer, or nothing.
#[derive(Resource, Default)]
pub struct TrainerWindow {
    open: Option<Open>,
}

/// What is on screen: the packet as it arrived, and the rows it was laid into.
struct Open {
    guid: u64,
    greeting: String,
    /// The decoded services, kept because a purchase amends one and the board
    /// is then rebuilt from them.
    services: Vec<Service>,
    kind: Kind,
    /// The race and class the board was laid out for — the same second key
    /// [`super::super::combat::spellbook`] keeps, and for the same reason: they decide every
    /// row's group and they can arrive after the packet does.
    built_for: (u8, u8),
    board: Board,
}

impl TrainerWindow {
    pub fn board(&self) -> Option<&Board> {
        self.open.as_ref().map(|open| &open.board)
    }

    /// …and mutably, for the four filter verbs, which are the only writes the
    /// interface makes that do not go on the wire.
    pub fn board_mut(&mut self) -> Option<&mut Board> {
        self.open.as_mut().map(|open| &mut open.board)
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.guid)
    }

    /// `GetTrainerGreetingText()` — empty when nothing is open, which is what
    /// the panel draws before its first `TRAINER_SHOW`.
    pub fn greeting(&self) -> String {
        self.open
            .as_ref()
            .map_or_else(String::new, |open| open.greeting.clone())
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Take the window down, answering whether one was up — the walk-away
    /// close's door, called from [`super::gossip::out_of_range`] like the
    /// vendor's.
    pub(crate) fn close(&mut self) -> bool {
        self.open.take().is_some()
    }
}

pub struct TrainerPlugin;

impl Plugin for TrainerPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TrainerAnswer>()
            .add_message::<TrainerPress>()
            .init_resource::<TrainerWindow>()
            .add_systems(
                Update,
                (announce, relayout, presses, act)
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// A press the interface made.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum TrainerPress {
    /// `BuyTrainerService(i)` — one-based, and **0 buys every green row**
    /// (the client loops the whole list). The interface never passes 0,
    /// but a `/script` can and the reference obeys it.
    Buy(usize),
    /// `CloseTrainer()` — local, like the gossip close: there is no opcode.
    Close,
    /// `SetTrainerServiceTypeFilter(word, on [, exclusive])`.
    TypeFilter {
        word: String,
        on: bool,
        exclusive: bool,
    },
    /// …and its `"all"` form.
    AllTypeFilters,
    /// `SetTrainerSkillLineFilter(group, on)`.
    LineFilter { group: usize, on: bool },
    /// `ExpandTrainerSkillLine(i)` / `CollapseTrainerSkillLine(i)`, **0 being
    /// every line**.
    Expand { row: usize, expanded: bool },
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<TrainerAnswer>,
    mut window: ResMut<TrainerWindow>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut say: super::super::messages::Announce,
    mut shown: MessageWriter<TrainerShow>,
    mut updated: MessageWriter<TrainerUpdate>,
) {
    for answer in answers.read() {
        match answer {
            TrainerAnswer::Show(list) => {
                let who = player.single().ok().and_then(|e| e.race_class).unwrap_or((0, 0));
                let services: Vec<Service> = list.services.iter().map(service_of).collect();
                let kind = kind_of(list.kind);
                let Some(board) = lay_out(&assets, kind, &services, who) else {
                    continue;
                };
                window.open = Some(Open {
                    guid: list.guid,
                    greeting: list.greeting.clone(),
                    services,
                    kind,
                    built_for: who,
                    board,
                });
                shown.write(TrainerShow);
            }
            // **The row goes grey here, not on a second packet** — see the
            // module note, where that reading is stated.
            TrainerAnswer::Bought { spell } => {
                let Some(open) = window.open.as_mut() else {
                    continue;
                };
                let Some(service) = open.services.iter_mut().find(|s| s.spell == *spell) else {
                    continue;
                };
                service.state = State::Used;
                // **Amended in place, not laid out again** — see
                // `Board::mark_used`. Rebuilding the board here reset the type
                // filter, the skill-line filter and every collapsed line on
                // every purchase, because those three live on the board and
                // `Board::build` starts them at their defaults. The copy in
                // `open.services` is what a later `relayout` reads, so both
                // move.
                if !open.board.mark_used(*spell) {
                    continue;
                }
                updated.write(TrainerUpdate);
            }
            TrainerAnswer::BuyFailed { reason, .. } => {
                if let Some(key) = reason.and_then(TrainFailure::key) {
                    say.key(key);
                }
            }
        }
    }
}

/// **Lay the rows out again when the character's identity lands.**
///
/// Race and class decide every row's skill line and they arrive in an update
/// block that can follow the trainer packet — the same race the spellbook has
/// with its own `built_for`. Without this, a window opened in that gap is laid
/// out for race 0 class 0, every service resolves to no skill line, and the
/// panel comes up **empty** while every check reports success.
fn relayout(
    mut window: ResMut<TrainerWindow>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut updated: MessageWriter<TrainerUpdate>,
) {
    let Some(open) = window.open.as_mut() else {
        return;
    };
    let Some(who) = player.single().ok().and_then(|e| e.race_class) else {
        return;
    };
    if who == open.built_for {
        return;
    }
    let (services, kind) = (open.services.clone(), open.kind);
    let Some(board) = lay_out(&assets, kind, &services, who) else {
        return;
    };
    let selected = open.board.selected();
    open.built_for = who;
    open.board = board;
    open.board.select_spell(selected);
    updated.write(TrainerUpdate);
}

/// The rules, over the shipped tables — `None` while the archives are still
/// opening, which holds the window rather than laying it out from nothing.
fn lay_out(
    assets: &GameAssets,
    kind: Kind,
    services: &[Service],
    (race, class): (u8, u8),
) -> Option<Board> {
    let tables = assets.display_tables().ok()?;
    let spells = tables.spellbook()?;
    Some(Board::build(
        kind,
        services,
        &Tables {
            spells,
            skills: tables.skills(),
            race,
            class,
        },
    ))
}

/// The wire's row, decoded — see `vale_assets::tables::trainer`'s module note on why
/// the two crates each name these.
fn service_of(service: &vale_protocol::play::trainer::TrainerService) -> Service {
    Service {
        spell: service.spell,
        state: match service.state {
            TrainerState::Available => State::Available,
            TrainerState::Unavailable => State::Unavailable,
            TrainerState::Used => State::Used,
        },
        cost: service.cost,
        point_cost: service.point_cost,
        req_level: service.req_level,
        req_skill: service.req_skill,
        req_skill_value: service.req_skill_value,
        req_spells: service.req_spells,
    }
}

fn kind_of(kind: TrainerType) -> Kind {
    match kind {
        TrainerType::Class => Kind::Class,
        TrainerType::Talent => Kind::Talent,
        TrainerType::Tradeskill => Kind::Tradeskill,
        TrainerType::Pet => Kind::Pet,
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<TrainerPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_trainer_presses() {
        out.write(press);
    }
}

/// …and act on it: four filters that stay here, one verb that leaves.
fn act(
    mut presses: MessageReader<TrainerPress>,
    mut window: ResMut<TrainerWindow>,
    session: Res<Session>,
    mut closed: MessageWriter<TrainerClosed>,
    mut updated: MessageWriter<TrainerUpdate>,
) {
    for press in presses.read() {
        // **Every filter raises `TRAINER_UPDATE`**, which is what the client
        // does: each of the three setters ends in the rebuild and then the
        // event. Without it the panel keeps drawing the rows it
        // last read and the dropdown's tick is the only thing that moves.
        let mut changed = true;
        match press {
            TrainerPress::Buy(row) => {
                changed = false;
                let Some(open) = window.open.as_ref() else {
                    continue;
                };
                let Some(active) = session.active.as_ref() else {
                    continue;
                };
                // 0 is every green row — see [`TrainerPress::Buy`].
                let rows: Vec<u32> = match row {
                    0 => (1..=open.board.visible())
                        .filter_map(|i| open.board.row(i))
                        .filter(|r| r.state == Some(State::Available))
                        .map(|r| r.spell)
                        .collect(),
                    // **The near gate**: a row that is not green sends nothing.
                    _ => open
                        .board
                        .row(*row)
                        .filter(|r| r.state == Some(State::Available))
                        .map(|r| vec![r.spell])
                        .unwrap_or_default(),
                };
                for spell in rows {
                    active.live.npc(NpcVerb::TrainerBuy {
                        trainer: open.guid,
                        spell,
                    });
                }
            }
            TrainerPress::Close => {
                changed = false;
                if window.close() {
                    closed.write(TrainerClosed);
                }
            }
            TrainerPress::TypeFilter {
                word,
                on,
                exclusive,
            } => {
                changed = window
                    .board_mut()
                    .is_some_and(|board| board.set_type_filter(word, *on, *exclusive));
            }
            TrainerPress::AllTypeFilters => {
                if let Some(board) = window.board_mut() {
                    board.set_all_type_filters();
                }
            }
            TrainerPress::LineFilter { group, on } => {
                changed = window
                    .board_mut()
                    .is_some_and(|board| board.set_line_filter(*group, *on));
            }
            TrainerPress::Expand { row, expanded } => {
                changed = window
                    .board_mut()
                    .is_some_and(|board| board.set_expanded(*row, *expanded));
            }
        }
        if changed {
            updated.write(TrainerUpdate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire's three states become the rules' three, by the byte the filter
    /// mask is indexed with. Two enums with the same shape in two crates is
    /// exactly where a silent transposition lives.
    #[test]
    fn the_two_state_enums_agree_by_the_byte() {
        for byte in [0u8, 1, 2] {
            let wire = TrainerState::of(byte);
            let rule = service_of(&vale_protocol::play::trainer::TrainerService {
                spell: 1,
                state: wire,
                cost: 0,
                point_cost: (0, 0),
                req_level: 0,
                req_skill: 0,
                req_skill_value: 0,
                req_spells: [0; 3],
            })
            .state;
            assert_eq!(usize::from(wire.byte()), rule.index());
        }
    }

    /// …and the four trainer types, which decide the whole grouping.
    #[test]
    fn the_two_kind_enums_agree() {
        assert_eq!(kind_of(TrainerType::Class), Kind::Class);
        assert_eq!(kind_of(TrainerType::Talent), Kind::Talent);
        assert_eq!(kind_of(TrainerType::Tradeskill), Kind::Tradeskill);
        assert_eq!(kind_of(TrainerType::Pet), Kind::Pet);
        // The default the wire degrades to is the plain grouping either side.
        assert_eq!(kind_of(TrainerType::of(99)), Kind::Class);
    }
}
