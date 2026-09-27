//! **The flight map** — the window a flight master opens, and the flight it
//! buys.
//!
//! The client's half of [`vale_protocol::play::taxi`], with the same split every
//! panel in this directory keeps: [`vale_assets::tables::taxi`] is the *rule* — which
//! nodes are drawn, where on the parchment, which route the lines take and what
//! it costs — because none of that needs a session, and what is left here is the
//! three things that do.
//!
//! ```text
//! which nodes    SMSG_SHOWTAXINODES, which arrives whole and is replaced whole
//! which side     the player's UNIT_FIELD_FACTIONTEMPLATE, which decides whether
//!                a node's mounts serve them at all
//! what a press   CMSG_ACTIVATETAXI or its EXPRESS twin, and the one reply
//! ```
//!
//! ## Nothing here flies the character, and nothing here spends the money
//!
//! Both are worth stating because both look like gaps. The flight is
//! `SMSG_MONSTER_MOVE` naming our own guid — the same packet Charge turned out
//! to be — so it reaches [`vale_protocol::state::movement::Mover::ride`] through the
//! ordinary movement arm and this module never sees it. And the cost comes off
//! `PLAYER_FIELD_COINAGE` because the server took it (`ActivateTaxiPathTo`
//! charges the first leg, `FlightPathMovementGenerator::Update` each further
//! one); the number in the tooltip is a quote this client computes for the
//! *display* and never a deduction.
//!
//! ## Four things close it, and this file used to think two of them did not
//!
//! The map comes down on its own close button, on Escape, on **walking away**
//! ([`super::gossip::out_of_range`], where it is the fifth window) and on
//! **taking a flight** (`SMSG_ACTIVATETAXIREPLY` code 0, the one edge that says
//! a departure is happening). Every one of them ends at the same place: the
//! `TAXIMAP_CLOSED` this module raises, which `TaxiFrame.lua` answers with
//! `HideUIPanel`.
//!
//! **The middle two are a correction.** This note used to argue that the taxi
//! map was deliberately not one of the windows walking away closes — a map
//! already holding its whole content does not care that the server has stopped
//! talking — and that taking a flight closed it anyway. Neither survived
//! contact: the window is a conversation with an NPC, it is the only thing
//! `UnitName("npc")` names while it is up, and nothing else was closing it. It
//! stayed open on the screen through the whole flight.

use bevy::prelude::*;

use vale_assets::tables::taxi::{Board, Take, TaxiMask, Team};
use vale_protocol::socket::session::TaxiVerb;
use vale_protocol::play::taxi::{TaxiMenu, TaxiReply};

use super::events::{TaximapClosed, TaximapOpened};
use crate::assets::GameAssets;
use crate::world::session::{LocalPlayer, Session, WorldEntity};

/// One of the four taxi packets, handed on from [`super::action::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum TaxiAnswer {
    /// `SMSG_SHOWTAXINODES` — the map.
    Show(TaxiMenu),
    /// `SMSG_TAXINODE_STATUS` — whether a master's own node is known.
    NodeStatus { guid: u64, known: bool },
    /// `SMSG_NEW_TAXI_PATH` — a flight point discovered.
    Discovered,
    /// `SMSG_ACTIVATETAXIREPLY`.
    Reply(TaxiReply),
}

/// The open flight map, or nothing.
#[derive(Resource, Default)]
pub struct TaxiWindow {
    open: Option<Open>,
    /// Which node the interface last called `TaxiNodeSetCurrent` for. Kept
    /// because the reference keeps it (it rebuilds that node's line list)
    /// even though every read here takes its index as an argument — see
    /// [`TaxiPress::SetCurrent`].
    examining: usize,
}

struct Open {
    /// The flight master, which every verb echoes back.
    guid: u64,
    menu: TaxiMenu,
    board: Board,
    /// The faction template the board was laid out for — the same second key
    /// [`super::trainer`] keeps for race and class, and for the same reason: it
    /// decides which nodes are drawn at all and it can arrive after the packet.
    built_for: Option<u32>,
}

impl TaxiWindow {
    pub fn board(&self) -> Option<&Board> {
        self.open.as_ref().map(|open| &open.board)
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.open.as_ref()?.guid)
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Which node's route the interface last asked to be current — one-based,
    /// 0 for none.
    pub fn examining(&self) -> usize {
        self.examining
    }

    /// **The map art for the open window** — `SetTaxiMap`'s whole answer. `None`
    /// with nothing open, which draws the frame's own art and no parchment.
    pub fn map_art(&self) -> Option<String> {
        Some(vale_assets::tables::taxi::map_art(self.open.as_ref()?.board.map))
    }

    /// Take the window down, answering whether one was up.
    ///
    /// `pub(super)` because walking away closes it too, from
    /// [`super::gossip::out_of_range`] — see that function, which is where the
    /// argument for the taxi map being one of the windows it reaches is.
    pub(super) fn close(&mut self) -> bool {
        self.examining = 0;
        self.open.take().is_some()
    }
}

/// A press the interface made.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaxiPress {
    /// `TakeTaxiNode(i)` — one-based into the drawn nodes.
    Take(usize),
    /// `TaxiNodeSetCurrent(i)` — recorded and otherwise inert; see
    /// [`TaxiWindow::examining`].
    SetCurrent(usize),
    /// `CloseTaxiMap()` — local, like every other close in this directory:
    /// there is no opcode.
    Close,
}

pub struct TaxiPlugin;

impl Plugin for TaxiPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TaxiAnswer>()
            .add_message::<TaxiPress>()
            .init_resource::<TaxiWindow>()
            .add_systems(
                Update,
                (ask_status, announce, relayout, presses, act)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// **Ask every flight master in view whether we have been to their node.**
///
/// The answer is the green `!` over their head and nothing else, which is why
/// this is here rather than beside the window: it is asked about a *unit*, once,
/// on sight, and a client that never asks simply never marks one.
///
/// `UNIT_NPC_FLAG_FLIGHTMASTER` is the gate, and it is the same gate the
/// client's own handler applies at the other end (it tests bit 3 of
/// `UNIT_NPC_FLAGS` before it will put a mark up at all) — so a query about
/// anything else is a packet whose answer would be discarded. vmangos declines
/// it too, from the other side: `SendTaxiStatus` returns without replying when
/// `GetNearestTaxiNode` finds nothing.
///
/// **Once per guid and never again**, where the quest mark's twin re-asks every
/// time the log moves. It can be, because this answer *is* announced: a
/// discovery sends `SMSG_NEW_TAXI_PATH` and then a fresh `SMSG_TAXINODE_STATUS`
/// carrying 1 for the same master, in the same breath (`SendLearnNewTaxiNode`).
/// So the mark takes itself down at exactly the moment the message goes up, and
/// there is nothing here to poll.
///
/// The list is built under one lock and the sends happen outside it, which is
/// the rule `super::quest::ask_status` keeps and for the same reason: a `send`
/// under the world lock is network latency held against every reader of it.
fn ask_status(session: Res<Session>, units: Query<&WorldEntity>) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let ask: Vec<u64> = {
        let world = active.live.world();
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        units
            .iter()
            .filter(|unit| is_flight_master(unit))
            .map(|unit| unit.guid)
            .filter(|guid| world.wants_taxi_status(*guid))
            .collect()
    };
    for guid in ask {
        send(&session, TaxiVerb::NodeStatus(guid));
    }
}

/// Fold what the server said into the window, and tell the interface.
#[allow(clippy::too_many_arguments)]
fn announce(
    mut answers: MessageReader<TaxiAnswer>,
    mut window: ResMut<TaxiWindow>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    // **One parameter for all thirteen**, because which surface each takes is
    // the message table's own column and not this file's: six of the twelve
    // refusals are yellow and six are red, and they were all red here.
    mut say: super::messages::Announce,
    mut opened: MessageWriter<TaximapOpened>,
    mut closed: MessageWriter<TaximapClosed>,
) {
    for answer in answers.read() {
        match answer {
            TaxiAnswer::Show(menu) => {
                let template = player.single().ok().and_then(|e| e.faction);
                let Some(board) = lay_out(&assets, menu, template) else {
                    // No tables and no board: opening the window anyway would
                    // put an empty parchment up and `DrawOneHopLines` would
                    // close it again with `ERR_TAXINOPATHS`, which is a worse
                    // report than nothing happening.
                    continue;
                };
                window.examining = 0;
                window.open = Some(Open {
                    guid: menu.guid,
                    menu: *menu,
                    board,
                    built_for: template,
                });
                opened.write(TaximapOpened);
            }
            // **The status byte has no panel behind it, and it is not a blip
            // either** — this comment used to say the reference used it for a
            // flight master's minimap icon, and that was wrong. Its handler
            // puts `Interface\Buttons\TalkToMeGreen.mdx` over the
            // unit's head when the byte is zero and takes it down when it is
            // not, which is the green `!` on a master you have never been to.
            //
            // Nothing to do here: the answer is kept on the object manager by
            // the packet handler itself, because a mark is *state* that has to
            // outlive the event — see `render::questmarks`.
            TaxiAnswer::NodeStatus { .. } => {}
            // **A discovery has no body and no map with it.** The mask that
            // moved arrives on the next `SMSG_SHOWTAXINODES`, which the server
            // sends in the same breath, so there is nothing to fold in — what
            // it is, is the message. `ERR_NEWTAXIPATH` is "New flight path
            // discovered!" and it is *yellow*: the client's own message table
            // gives it type 1, which is `UI_INFO_MESSAGE`, and it names
            // `TaxiNodeDiscovered` for the chime. Both are columns of record
            // 242 — see [`vale_assets::interface::messages`].
            TaxiAnswer::Discovered => say.key("ERR_NEWTAXIPATH"),
            TaxiAnswer::Reply(reply) => {
                // **`Ok` is a departure, and a departure closes the map.**
                // `SMSG_ACTIVATETAXIREPLY` code 0 is sent immediately before
                // `SendDoFlight`, so it is the one edge that says the flight is
                // happening — every other code is a refusal that leaves the map
                // up to be pressed again, which is what the twelve sentences
                // below are for.
                //
                // **This is a reading**, stated as one: the *event* is
                // certain (`TAXIMAP_CLOSED` is id 307, and `TaxiFrame.lua`
                // answers it with `HideUIPanel`), and what raises it in the
                // reference is not known — it is not `TakeTaxiNode`, which
                // sends the packet and nothing else, and `UIParent`'s `PLAYER_CONTROL_LOST` path
                // explicitly *skips* `CloseAllWindows` while `UnitOnTaxi`.
                if departs(*reply) && window.close() {
                    closed.write(TaximapClosed);
                }
                if let Some(key) = reply.key() {
                    say.key(key);
                }
            }
        }
    }
}

/// **Is this reply a departure?** — the one code that closes the map.
///
/// A function rather than a comparison at the call site so that the rule can be
/// held against the whole thirteen-code table: `SMSG_ACTIVATETAXIREPLY` code 0
/// is sent immediately before `SendDoFlight`, and every other code is a refusal
/// with a `GlobalStrings.lua` sentence attached — which is only useful with the
/// map still up to press again.
fn departs(reply: TaxiReply) -> bool {
    reply == TaxiReply::Ok
}

/// **Lay the map out again when the character's side lands.**
///
/// `UNIT_FIELD_FACTIONTEMPLATE` decides which nodes [`vale_assets::tables::taxi`]
/// will draw at all, and it arrives in an update block that can follow the taxi
/// packet — the same gap [`super::trainer::relayout`] exists for. Without this,
/// a window opened in it is laid out with no team and shows the *other* side's
/// nodes as well.
fn relayout(
    mut window: ResMut<TaxiWindow>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut opened: MessageWriter<TaximapOpened>,
) {
    let Some(open) = window.open.as_mut() else {
        return;
    };
    let Some(template) = player.single().ok().and_then(|e| e.faction) else {
        return;
    };
    if open.built_for == Some(template) {
        return;
    }
    let menu = open.menu;
    let Some(board) = lay_out(&assets, &menu, Some(template)) else {
        return;
    };
    open.built_for = Some(template);
    open.board = board;
    // `TAXIMAP_OPENED` is what re-draws the buttons; there is no update event in
    // 1.12 and the panel is already showing, so re-raising the open is the
    // reference's own only door.
    opened.write(TaximapOpened);
}

/// The rules, over the shipped tables — `None` while the archives are still
/// opening, or for a chain with no taxi tables at all.
fn lay_out(assets: &GameAssets, menu: &TaxiMenu, template: Option<u32>) -> Option<Board> {
    let tables = assets.display_tables().ok()?;
    let taxi = tables.taxi()?;
    let team = template
        .and_then(|template| tables.faction_group(template))
        .and_then(Team::of_group);
    let board = Board::build(taxi, menu.current, &TaxiMask(menu.mask), team);
    (!board.rows.is_empty()).then_some(board)
}

/// Drain what the interface pressed.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<TaxiPress>) {
    let Some(mut host) = host else { return };
    for press in host.take_taxi_presses() {
        out.write(press);
    }
}

/// …and act on it: one verb that leaves, one close that does not.
fn act(
    mut presses: MessageReader<TaxiPress>,
    mut window: ResMut<TaxiWindow>,
    assets: Res<GameAssets>,
    session: Res<Session>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut say: super::messages::Announce,
    mut closed: MessageWriter<TaximapClosed>,
) {
    for press in presses.read() {
        match press {
            TaxiPress::SetCurrent(row) => window.examining = *row,
            TaxiPress::Close => {
                if window.close() {
                    closed.write(TaximapClosed);
                }
            }
            TaxiPress::Take(row) => {
                // **The near gate is the client's own**: a
                // character already in the air is refused here rather than by
                // the server, which would answer `ERR_TAXIPLAYERALREADYMOUNTED`
                // to the same effect one round trip later.
                if player.single().is_ok_and(on_taxi) {
                    say.key("ERR_TAXIPLAYERALREADYMOUNTED");
                    continue;
                }
                let Some(open) = window.open.as_ref() else {
                    continue;
                };
                let Ok(tables) = assets.display_tables() else {
                    continue;
                };
                let Some(taxi) = tables.taxi() else { continue };
                let guid = open.guid;
                match open.board.take(taxi, *row) {
                    Take::Direct { from, to } => {
                        send(&session, TaxiVerb::Activate { guid, from, to });
                    }
                    Take::Express { nodes, cost } => {
                        send(&session, TaxiVerb::ActivateExpress { guid, cost, nodes });
                    }
                    Take::Refused(key) => say.key(key),
                    Take::Nothing => {}
                }
            }
        }
    }
}

fn send(session: &Session, verb: TaxiVerb) {
    if let Some(active) = session.active.as_ref() {
        active.live.taxi(verb);
    }
}

/// Is this unit in the air on a flight path?
///
/// The whole of what `UnitOnTaxi(unit)` answers, and the one piece of taxi state
/// that is not in this module's own resource: it is on the unit, it is set by
/// the server, and it is true of a character who logged out mid-flight and back
/// in — which no window of ours would have known about. The **mover** reads the
/// same bit off the same field for the same reason; see
/// `vale_protocol::state::movement::Restraint::on_taxi`, which is what stops that
/// character walking around on the ground.
pub fn on_taxi(unit: &WorldEntity) -> bool {
    unit.unit_flags & vale_protocol::state::objects::UNIT_FLAG_TAXI_FLIGHT != 0
}

/// `UNIT_NPC_FLAGS`' flight-master bit — **a different field from the one
/// above**, which is the reason both are named here rather than left as
/// literals: `UNIT_FIELD_FLAGS` says what a unit is *doing* and
/// `UNIT_NPC_FLAGS` what it will *talk about*, and 0x8 means something in each.
pub const UNIT_NPC_FLAG_FLIGHTMASTER: u32 = 0x0008;

/// Will this unit sell a flight?
pub fn is_flight_master(unit: &WorldEntity) -> bool {
    unit.npc_flags & UNIT_NPC_FLAG_FLIGHTMASTER != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flag, both ways — a constant transcribed from vmangos with one
    /// reader is exactly the shape that gets a bit wrong and never shows it.
    #[test]
    fn the_taxi_flag_is_the_one_the_server_sets() {
        let mut unit = WorldEntity::default();
        assert!(!on_taxi(&unit));
        unit.unit_flags = vale_protocol::state::objects::UNIT_FLAG_TAXI_FLIGHT;
        assert!(on_taxi(&unit));
        // …and it is not confused with its neighbours: `IN_COMBAT` is 0x80000
        // and `REMOVE_CLIENT_CONTROL` — which the server always pairs with this
        // one — is 0x4.
        unit.unit_flags = 0x0008_0000 | 0x4;
        assert!(!on_taxi(&unit));
    }

    /// **The two flag fields are two fields**, and 0x8 means something in each:
    /// `UNIT_FIELD_FLAGS`' 0x8 is not the flight-master bit and
    /// `UNIT_NPC_FLAGS`' 0x8 is not a state of flight. A client that crossed
    /// them would query taxi status about anything wearing the wrong bit and
    /// put a green `!` over it.
    #[test]
    fn the_flight_master_bit_is_the_npc_field_and_not_the_unit_one() {
        let mut unit = WorldEntity::default();
        assert!(!is_flight_master(&unit));
        unit.unit_flags = UNIT_NPC_FLAG_FLIGHTMASTER;
        assert!(!is_flight_master(&unit), "the wrong field answered");
        unit.unit_flags = 0;
        unit.npc_flags = UNIT_NPC_FLAG_FLIGHTMASTER;
        assert!(is_flight_master(&unit));
        assert!(!on_taxi(&unit), "…and it is not read as being in flight");
        // A gossip-and-vendor NPC is not one; a master who also talks is.
        unit.npc_flags = 0x0001 | 0x0004;
        assert!(!is_flight_master(&unit));
        unit.npc_flags = 0x0001 | UNIT_NPC_FLAG_FLIGHTMASTER;
        assert!(is_flight_master(&unit));
    }

    /// **A discovery says the game's own line, in yellow.**
    ///
    /// `ERR_NEWTAXIPATH`, `UI_INFO_MESSAGE` and `TaxiNodeDiscovered` — three
    /// columns of one record, none of which could have been guessed from the
    /// packet: `SMSG_NEW_TAXI_PATH` has **no body at all**, so the key, the
    /// colour and the chime are all the client's own table's.
    #[test]
    fn a_discovery_says_the_games_own_line_in_yellow_and_makes_its_own_noise() {
        use bevy::ecs::message::Messages as MessageQueue;
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(crate::interface::messages::UiStrings(Some(std::sync::Arc::new(
            vale_assets::interface::strings::Strings::parse(
                br#"ERR_NEWTAXIPATH = "New flight path discovered!";"#,
            ),
        ))));
        world.init_resource::<MessageQueue<crate::interface::events::UiInfoMessage>>();
        world.init_resource::<MessageQueue<crate::interface::events::UiErrorMessage>>();
        world.init_resource::<MessageQueue<crate::interface::events::ChatMessageReceived>>();
        world.init_resource::<MessageQueue<crate::interface::messages::MessageSound>>();
        world
            .run_system_once(|mut say: crate::interface::messages::Announce| {
                say.key("ERR_NEWTAXIPATH");
            })
            .expect("the system runs");

        let queue = world.resource::<MessageQueue<crate::interface::events::UiInfoMessage>>();
        let mut cursor = queue.get_cursor();
        let said: Vec<String> = cursor.read(queue).map(|m| m.0.clone()).collect();
        assert_eq!(said, vec!["New flight path discovered!"]);
        // **And not through the red one**, which is the whole point of there
        // being two: `UIErrorsFrame` picks (1, 1, 0) for this name and
        // (1, 0.1, 0.1) for the other.
        assert_eq!(
            world
                .resource::<MessageQueue<crate::interface::events::UiErrorMessage>>()
                .len(),
            0
        );
        // …and it is not a chat line either, which is the third surface the
        // table names and the one 122 of its records take.
        assert_eq!(
            world
                .resource::<MessageQueue<crate::interface::events::ChatMessageReceived>>()
                .len(),
            0
        );
        // **And the chime the table names**, which nothing in this client
        // played before the table existed.
        let sounds = world.resource::<MessageQueue<crate::interface::messages::MessageSound>>();
        let mut cursor = sounds.get_cursor();
        let played: Vec<&str> = cursor.read(sounds).map(|s| s.0).collect();
        assert_eq!(played, vec!["TaxiNodeDiscovered"]);
    }

    /// The window's own state, without a world: what is open, what art it wants
    /// and what a close leaves behind.
    #[test]
    fn a_closed_window_answers_nothing_and_a_close_is_idempotent() {
        let mut window = TaxiWindow::default();
        assert!(!window.is_open());
        assert_eq!(window.map_art(), None);
        assert_eq!(window.guid(), None);
        assert!(!window.close(), "nothing was open");
        window.open = Some(Open {
            guid: 9,
            menu: TaxiMenu {
                guid: 9,
                current: 2,
                mask: [0; 8],
            },
            board: Board {
                rows: Vec::new(),
                current: 2,
                map: 1,
            },
            built_for: None,
        });
        window.examining = 3;
        assert_eq!(
            window.map_art().as_deref(),
            Some(r"Interface\TaxiFrame\TAXIMAP1.blp"),
            "the art is the continent's, not the node's"
        );
        assert!(window.close());
        assert_eq!(window.examining(), 0, "the examined row goes with it");
    }

    /// An open window for the tests below.
    fn open_window(guid: u64) -> TaxiWindow {
        TaxiWindow {
            open: Some(Open {
                guid,
                menu: TaxiMenu { guid, current: 2, mask: [0; 8] },
                board: Board { rows: Vec::new(), current: 2, map: 1 },
                built_for: None,
            }),
            examining: 0,
        }
    }

    /// **The flight master is who `"npc"` means while the map is up**, which is
    /// the whole of the portrait and the name on it: `TaxiFrame_OnEvent` opens
    /// with `TaxiMerchant:SetText(UnitName("npc"))` and
    /// `SetPortraitTexture(TaxiPortrait, "npc")`, and a window nothing names
    /// draws a blank plate and an empty frame.
    #[test]
    fn an_open_flight_map_names_its_master() {
        assert_eq!(TaxiWindow::default().guid(), None);
        assert_eq!(open_window(4242).guid(), Some(4242));
    }

    /// **A departure closes the map, and every one of the twelve refusals
    /// leaves it up.**
    ///
    /// Held against the whole table rather than against a couple of samples,
    /// because the two properties are the same property seen twice: the code
    /// that closes the window is exactly the code with no sentence to read in
    /// it. `SMSG_ACTIVATETAXIREPLY` 0 is sent immediately before
    /// `SendDoFlight`; 1..12 are `ERR_TAXI*` keys.
    #[test]
    fn only_an_accepted_flight_closes_the_map() {
        let mut closed = 0;
        for code in 0..=12u32 {
            let reply = TaxiReply::of(code).expect("a code the table has");
            assert_eq!(
                departs(reply),
                code == 0,
                "{reply:?} (code {code}) got the departure wrong"
            );
            assert_eq!(
                reply.key().is_none(),
                code == 0,
                "{reply:?} (code {code}): a refusal must have a sentence and a departure must not"
            );
            if departs(reply) {
                let mut window = open_window(9);
                assert!(window.close(), "the departure left the map up");
                assert!(!window.is_open());
                closed += 1;
            }
        }
        assert_eq!(closed, 1, "exactly one of the thirteen is a departure");
    }
}
