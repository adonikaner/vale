//! The guild charter's half of the session: seven packets in, nine requests
//! out, and four events.
//!
//! The charter's state is the board on the Lua host; see
//! [`crate::lua::panels::petition`]. [`vale_protocol::play::petition`] has the
//! wire formats. This file is the systems between the two:
//!
//! ```text
//! track          the character's guid and whether it is in a guild
//! apply_answers  the seven packets: the board, the sentences, the events
//! resolve        names from the name cache, and PETITION_SHOW when the open
//!                charter is whole
//! act            what the interface pressed
//! walk_away      the registrar window, when the registrar is out of reach
//! ```
//!
//! ## Two windows
//!
//! The registrar window is a conversation with an NPC. It opens on
//! `SMSG_PETITION_SHOWLIST`, the `"npc"` token names the registrar while it
//! is open, and walking away closes it; see [`RegistrarWindow`] and
//! [`super::gossip`].
//!
//! The petition window is a charter item. It opens on
//! `SMSG_PETITION_SHOW_SIGNATURES`, which the server sends in answer to a
//! right-click on the charter and to a player who was offered one. It has no
//! NPC and no range.
//!
//! ## When `PETITION_SHOW` is raised
//!
//! The signatures packet carries guids. The 1.12.1 client raises
//! `PETITION_SHOW` once the petition's record is held and every signer has a
//! name, so the window never draws a blank line for a signer. [`resolve`]
//! raises it on the frame that becomes true, and again when a signature is
//! added or the charter is renamed.
//!
//! ## Closing without signing declines
//!
//! Hiding the petition window sends `MSG_PETITION_DECLINE`, unless the
//! character owns the charter or signed it while it was open. The server
//! forwards the decline to the owner, who reads `ERR_PETITION_DECLINED_S`.
//!
//! ## Where each sentence goes
//!
//! Every sentence is said through [`super::messages::Announce`], which puts
//! a key in the chat frame or the error frame as the client's message table
//! states for that key. The refusals the client makes itself (a bad name,
//! too little money, no charter carried, an offer to the wrong target) are
//! keys of the same table. A sentence that names a player waits in
//! [`Waiting`] until the name query answers.

use bevy::prelude::*;

use super::events::{GuildRegistrarClosed, GuildRegistrarShow, PetitionClosed, PetitionShow};
use crate::lua::panels::petition::{Open, Press};
use crate::world::session::{Session, WorldEntity};
use vale_protocol::play::petition::{result, PetitionQuery, ShowList, SignResult, Signatures};
use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::socket::session::{GuildVerb, PetitionVerb};

/// `UNIT_NPC_FLAG_PETITIONER` and `UNIT_NPC_FLAG_TABARDDESIGNER`. The 1.12.1
/// client opens the registrar window only for an NPC that carries both.
const REGISTRAR_FLAGS: u32 = vale_assets::look::cursor::npc_flags::PETITIONER
    | vale_assets::look::cursor::npc_flags::TABARDDESIGNER;

/// What the session thread said about a charter, forwarded by
/// [`crate::world::incoming::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum PetitionAnswer {
    ShowList(ShowList),
    Signatures(Signatures),
    Query(PetitionQuery),
    SignResult(SignResult),
    /// `SMSG_TURN_IN_PETITION_RESULTS`.
    TurnIn(u32),
    /// The guid of the player who declined to sign.
    Declined(u64),
    Renamed { item: u64, name: String },
}

/// The guild registrar the registrar window is open at, or nothing. Read by
/// [`super::gossip`] for the `"npc"` token.
#[derive(Resource, Default)]
pub struct RegistrarWindow(Option<u64>);

impl RegistrarWindow {
    pub fn guid(&self) -> Option<u64> {
        self.0
    }

    /// Take the window down, answering whether one was up.
    pub(crate) fn close(&mut self) -> bool {
        self.0.take().is_some()
    }
}

/// Sentences that name a player whose name has not arrived: the key and the
/// player's guid. Said by [`resolve`] when the name does.
#[derive(Resource, Default)]
struct Waiting(Vec<(&'static str, u64)>);

/// Whether the open charter was whole on the last frame, so that
/// `PETITION_SHOW` is raised when it becomes whole and not on every frame it
/// stays so. Cleared to raise it again.
#[derive(Resource, Default)]
struct Shown(bool);

pub struct PetitionPlugin;

impl Plugin for PetitionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PetitionAnswer>()
            .init_resource::<RegistrarWindow>()
            .init_resource::<Waiting>()
            .init_resource::<Shown>()
            .add_systems(
                Update,
                (track, apply_answers, resolve, act, walk_away, mirror)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// Copy the character's guid and guild membership to the board. The guild
/// board already holds the membership; see [`super::guild`].
fn track(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    player: Query<&WorldEntity, With<crate::world::session::LocalPlayer>>,
) {
    let Some(host) = host else { return };
    let in_guild = host.guild().borrow().guild_id() != 0;
    let own = player.single().map_or(0, |me| me.guid);
    let mut board = host.petition().borrow_mut();
    board.own_guid = own;
    board.in_guild = in_guild;
}

/// Fold the server's seven answers into the board, the sentences and the
/// events.
#[allow(clippy::too_many_arguments)]
fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    units: Query<&WorldEntity>,
    mut answers: MessageReader<PetitionAnswer>,
    mut registrar: ResMut<RegistrarWindow>,
    mut waiting: ResMut<Waiting>,
    mut shown: ResMut<Shown>,
    mut say: super::messages::Announce,
    mut registrar_show: MessageWriter<GuildRegistrarShow>,
    mut registrar_closed: MessageWriter<GuildRegistrarClosed>,
    mut closed: MessageWriter<PetitionClosed>,
) {
    let Some(host) = host else {
        // Drained, so answers that arrive before the interface is up are not
        // replayed on the frame the host appears.
        answers.clear();
        return;
    };
    let ignored = |guid: u64| {
        host.social()
            .borrow()
            .ignores
            .iter()
            .any(|(ignored, _)| *ignored == guid)
    };
    let mut board = host.petition().borrow_mut();
    for answer in answers.read() {
        match answer {
            PetitionAnswer::ShowList(list) => {
                let offer = list.offers.first().copied();
                let flags = units
                    .iter()
                    .find(|unit| unit.guid == list.npc)
                    .map_or(0, |unit| unit.npc_flags);
                // An NPC that is only a petitioner, and an offer without its
                // first flag, open nothing.
                if flags & REGISTRAR_FLAGS != REGISTRAR_FLAGS
                    || !offer.is_some_and(|offer| offer.flags & 1 != 0)
                {
                    continue;
                }
                board.registrar = Some(list.npc);
                board.offer = offer;
                registrar.0 = Some(list.npc);
                registrar_show.write(GuildRegistrarShow);
            }
            PetitionAnswer::Signatures(list) => {
                // A charter offered by an ignored player is dropped.
                if ignored(list.owner) {
                    continue;
                }
                // A charter already open is closed first, the same charter
                // included, as hiding the window would close it.
                if let Some(open) = board.petition.take() {
                    decline(&session, &board, &open);
                    closed.write(PetitionClosed);
                }
                if !board.records.contains_key(&list.petition) {
                    if let Some(active) = session.active.as_ref() {
                        active.live.petition(PetitionVerb::Query {
                            petition: list.petition,
                            item: list.item,
                        });
                    }
                }
                board.petition = Some(Open {
                    shown: list.clone(),
                    signed: false,
                });
                shown.0 = false;
            }
            PetitionAnswer::Query(record) => {
                board.records.insert(record.petition, record.clone());
            }
            PetitionAnswer::SignResult(signed) if signed.player != board.own_guid => {
                // Somebody signed a charter. The result is not read: the
                // server sends this to the owner only for a signature it
                // took. It changes the window only when that charter is the
                // one open.
                let Some(open) = board.petition.as_mut() else {
                    continue;
                };
                if open.shown.item != signed.item || open.shown.signers.contains(&signed.player) {
                    continue;
                }
                open.shown.signers.push(signed.player);
                waiting.0.push(("ERR_PETITION_SIGNED_S", signed.player));
                shown.0 = false;
            }
            PetitionAnswer::SignResult(signed) => match signed.result {
                result::OK => {
                    say.key("ERR_PETITION_SIGNED");
                    if board.petition.take().is_some() {
                        closed.write(PetitionClosed);
                    }
                }
                result::ALREADY_SIGNED => say.key("ERR_PETITION_ALREADY_SIGNED"),
                result::ALREADY_IN_GUILD => say.key("ERR_PETITION_IN_GUILD"),
                result::CANT_SIGN_OWN => say.key("ERR_PETITION_CREATOR"),
                result::NOT_SERVER => say.key("ERR_PETITION_NOT_SAME_SERVER"),
                _ => {}
            },
            PetitionAnswer::TurnIn(outcome) => match *outcome {
                // The guild exists. The registrar window closes and nothing
                // is printed; the character's guild fields change.
                result::OK => {
                    board.registrar = None;
                    if registrar.close() {
                        registrar_closed.write(GuildRegistrarClosed);
                    }
                }
                result::ALREADY_IN_GUILD => say.key("ERR_PETITION_IN_GUILD"),
                result::NEED_MORE => say.key("ERR_PETITION_NOT_ENOUGH_SIGNATURES"),
                _ => {}
            },
            PetitionAnswer::Declined(player) => {
                waiting.0.push(("ERR_PETITION_DECLINED_S", *player));
            }
            PetitionAnswer::Renamed { item, name } => {
                // The packet names the charter item; the record is keyed by
                // petition id, which is known for the open charter only.
                let id = board
                    .petition
                    .as_ref()
                    .filter(|open| open.shown.item == *item)
                    .map(|open| open.shown.petition);
                if let Some(record) = id.and_then(|id| board.records.get_mut(&id)) {
                    record.name = name.clone();
                    shown.0 = false;
                }
            }
        }
    }
}

/// Send `MSG_PETITION_DECLINE` for a charter that is closing, unless the
/// character owns it, signed it, or never received its record.
fn decline(session: &Session, board: &crate::lua::panels::petition::Charter, open: &Open) {
    if open.signed
        || open.shown.owner == board.own_guid
        || !board.records.contains_key(&open.shown.petition)
    {
        return;
    }
    if let Some(active) = session.active.as_ref() {
        active.live.petition(PetitionVerb::Decline(open.shown.item));
    }
}

/// Copy the names the board is missing from the name cache, say the
/// sentences that were waiting for one, and raise `PETITION_SHOW` when the
/// open charter becomes whole.
///
/// The world lock is taken only while a name is missing.
fn resolve(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    mut waiting: ResMut<Waiting>,
    mut shown: ResMut<Shown>,
    mut say: super::messages::Announce,
    mut show: MessageWriter<PetitionShow>,
) {
    let Some(host) = host else {
        waiting.0.clear();
        shown.0 = false;
        return;
    };
    let mut board = host.petition().borrow_mut();
    let missing: Vec<u64> = board
        .named_guids()
        .into_iter()
        .chain(waiting.0.iter().map(|(_, guid)| *guid))
        .filter(|guid| !board.names.contains_key(guid))
        .collect();
    if !missing.is_empty() {
        if let Some(world) = session
            .active
            .as_ref()
            .and_then(|active| active.live.world().lock().ok())
        {
            for guid in missing {
                if let Some(info) = world.players.get(&guid) {
                    board.names.insert(guid, info.name.clone());
                }
            }
        }
    }
    waiting.0.retain(|(key, guid)| match board.names.get(guid) {
        Some(name) => {
            say.formatted(key, name);
            false
        }
        None => true,
    });
    let ready = board.ready();
    if ready && !shown.0 {
        show.write(PetitionShow);
    }
    shown.0 = ready;
}

/// Act on what the interface pressed.
#[allow(clippy::too_many_arguments)]
fn act(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    inventory: Res<super::items::Inventory>,
    units: super::api::Units,
    assets: Res<crate::assets::GameAssets>,
    mut registrar: ResMut<RegistrarWindow>,
    mut say: super::messages::Announce,
    mut registrar_closed: MessageWriter<GuildRegistrarClosed>,
    mut closed: MessageWriter<PetitionClosed>,
) {
    let Some(mut host) = host else { return };
    let presses = host.take_petition_presses();
    if presses.is_empty() {
        return;
    }
    let mut board = host.petition().borrow_mut();
    let send = |verb: PetitionVerb| {
        if let Some(active) = session.active.as_ref() {
            active.live.petition(verb);
        }
    };
    for press in presses {
        match press {
            Press::Refuse(key) => say.key(key),
            Press::Buy(name) => {
                let (Some(npc), Some(offer)) = (board.registrar, board.offer) else {
                    continue;
                };
                if board.in_guild {
                    say.key("ERR_ALREADY_IN_GUILD");
                } else if inventory.money < offer.cost {
                    say.key("ERR_NOT_ENOUGH_MONEY");
                } else {
                    send(PetitionVerb::Buy {
                        npc,
                        name,
                        index: offer.index,
                    });
                }
            }
            Press::TurnIn => match carried_charter(&inventory) {
                Some(item) => send(PetitionVerb::TurnIn(item)),
                None => say.key("ERR_NO_GUILD_CHARTER"),
            },
            Press::Sign(byte) => {
                if let Some(open) = &board.petition {
                    send(PetitionVerb::Sign {
                        item: open.shown.item,
                        byte,
                    });
                }
            }
            Press::Offer => {
                let Some(item) = board.petition.as_ref().map(|open| open.shown.item) else {
                    continue;
                };
                match offer_to(&units, &assets) {
                    Ok(target) => {
                        send(PetitionVerb::Offer {
                            item,
                            player: target.guid,
                        });
                        say.formatted("ERR_PETITION_OFFERED_S", &target.name);
                    }
                    Err(Refusal::Silent) => {}
                    Err(Refusal::Named(key, name)) => say.formatted(key, &name),
                    Err(Refusal::Said(key)) => say.key(key),
                }
            }
            Press::Rename(name) => {
                if let Some(open) = &board.petition {
                    send(PetitionVerb::Rename {
                        item: open.shown.item,
                        name,
                    });
                }
            }
            Press::ClosePetition => {
                if let Some(open) = board.petition.take() {
                    decline(&session, &board, &open);
                    closed.write(PetitionClosed);
                }
            }
            Press::CloseRegistrar => {
                board.registrar = None;
                if registrar.close() {
                    registrar_closed.write(GuildRegistrarClosed);
                }
            }
            Press::TabardInfo => {
                // The registrar's third button asks the same NPC for the
                // tabard designer and closes the registrar window.
                if let (Some(npc), Some(active)) = (board.registrar.take(), session.active.as_ref())
                {
                    active.live.guild(GuildVerb::TabardVendor(npc));
                }
                if registrar.close() {
                    registrar_closed.write(GuildRegistrarClosed);
                }
            }
        }
    }
}

/// Close the registrar window when the registrar is out of reach or gone.
/// The test is the one every NPC window uses, and it applies only while the
/// `"npc"` token names this registrar.
fn walk_away(
    units: super::api::Units,
    npc: Res<super::gossip::NpcUnit>,
    mut registrar: ResMut<RegistrarWindow>,
    mut closed: MessageWriter<GuildRegistrarClosed>,
) {
    let Some(guid) = registrar.guid() else { return };
    if npc.0 == Some(guid) && super::gossip::npc_gone(&units) && registrar.close() {
        closed.write(GuildRegistrarClosed);
    }
}

/// Keep the board's registrar in step with [`RegistrarWindow`], which
/// [`walk_away`] clears.
fn mirror(host: Option<NonSendMut<crate::lua::host::LuaHost>>, registrar: Res<RegistrarWindow>) {
    let Some(host) = host else { return };
    if registrar.guid().is_none() && host.petition().borrow().registrar.is_some() {
        host.petition().borrow_mut().registrar = None;
    }
}

/// The guid of the first guild charter in the bags: the backpack, then the
/// four bags in order. A charter is an item whose template has
/// `ITEM_FLAG_CHARTER`.
fn carried_charter(inventory: &super::items::Inventory) -> Option<u64> {
    use vale_protocol::state::query::item_flags::CHARTER;
    (0..=4).find_map(|bag| {
        inventory
            .carried
            .container(bag)?
            .iter()
            .flatten()
            .find(|item| {
                inventory
                    .template(item.entry)
                    .is_some_and(|template| template.flags & CHARTER != 0)
            })
            .map(|item| item.guid)
    })
}

/// Why `OfferPetition` sent nothing.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    /// The target is not a player.
    Silent,
    /// A key with no argument.
    Said(&'static str),
    /// A key that takes the target's name.
    Named(&'static str, String),
}

/// The player `OfferPetition` offers the charter to, in the order the 1.12.1
/// client tests: there is a target, it is a player, it is not the character,
/// it is on the character's side, and it is in no guild.
fn offer_to<'a>(
    units: &'a super::api::Units,
    assets: &crate::assets::GameAssets,
) -> Result<&'a WorldEntity, Refusal> {
    use super::api::UnitId;
    let target = units
        .get(UnitId::Target)
        .ok_or(Refusal::Said("ERR_OUT_OF_RANGE"))?;
    if target.kind != vale_protocol::state::update::ObjectType::Player {
        return Err(Refusal::Silent);
    }
    if target.is_self {
        return Err(Refusal::Said("ERR_PETITION_CREATOR"));
    }
    let group = |unit: &WorldEntity| {
        let tables = assets.display_tables().ok()?;
        tables.faction_group(unit.faction?)
    };
    if let Some(me) = units.get(UnitId::Player) {
        if group(me) != group(target) {
            return Err(Refusal::Said("ERR_GUILD_NOT_ALLIED"));
        }
    }
    if target.guild_id != 0 {
        return Err(Refusal::Named("ERR_ALREADY_IN_GUILD_S", target.name.clone()));
    }
    Ok(target)
}

/// The arm [`crate::world::incoming`] calls for the seven charter events.
pub fn answer_of(event: &PlayerEvent) -> Option<PetitionAnswer> {
    Some(match event {
        PlayerEvent::PetitionShowList(list) => PetitionAnswer::ShowList(list.clone()),
        PlayerEvent::PetitionSignatures(list) => PetitionAnswer::Signatures(list.clone()),
        PlayerEvent::PetitionQuery(record) => PetitionAnswer::Query(record.clone()),
        PlayerEvent::PetitionSignResult(signed) => PetitionAnswer::SignResult(*signed),
        PlayerEvent::PetitionTurnInResult(outcome) => PetitionAnswer::TurnIn(*outcome),
        PlayerEvent::PetitionDeclined(player) => PetitionAnswer::Declined(*player),
        PlayerEvent::PetitionRenamed { item, name } => PetitionAnswer::Renamed {
            item: *item,
            name: name.clone(),
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seven charter events are forwarded and nothing else is.
    #[test]
    fn only_charter_events_become_answers() {
        assert!(answer_of(&PlayerEvent::PetitionTurnInResult(0)).is_some());
        assert!(answer_of(&PlayerEvent::PetitionDeclined(7)).is_some());
        assert!(answer_of(&PlayerEvent::GuildDecline("Bram".into())).is_none());
    }

    /// The registrar window reports being closed once.
    #[test]
    fn the_registrar_window_closes_once() {
        let mut window = RegistrarWindow(Some(0x99));
        assert_eq!(window.guid(), Some(0x99));
        assert!(window.close());
        assert!(!window.close());
        assert_eq!(window.guid(), None);
    }
}
