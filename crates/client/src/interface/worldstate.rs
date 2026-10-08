//! World states, the client's side: the table the server keeps per zone, the
//! rows of `WorldStateUI.dbc` the frame above the minimap lists, and the event
//! that makes it redraw.
//!
//! The wire is [`vale_protocol::play::worldstate`]; the rule that chooses the
//! rows and formats their text is [`vale_assets::tables::worldstate`]. This
//! module holds the state between the two and raises `UPDATE_WORLD_STATES`:
//!
//! * `SMSG_INIT_WORLD_STATES` replaces the whole table, records the map and
//!   zone, rebuilds the list of rows, and raises the event once.
//! * `SMSG_UPDATE_WORLD_STATE` changes one value and raises the event. The list
//!   is not rebuilt, because the values play no part in it.
//! * Joining or leaving LocalDefense changes which outdoor rows are listed. The
//!   1.12.1 client rebuilds the list and raises the event when that condition
//!   changes, once a map is known. [`apply`] checks the channel board every
//!   frame; the board holds ten slots, so the check is ten comparisons.
//!
//! `WorldStateFrame.lua` reads the list through `GetNumWorldStateUI` and
//! `GetWorldStateUIInfo`, which [`crate::lua::panels::worldstate`] answers from
//! [`WorldStates`]. Leaving the world empties the table, so the next login's
//! frame does not draw the last zone's lines before its own packet arrives.
//!
//! `WorldStateUI.dbc` is read here on first need rather than through
//! `DisplayTables`, because nothing else reads it.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::worldstate::{WorldStateInfo, WorldStateUi, OUTDOOR_CHANNEL_FLAGS};
use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::play::worldstate::WorldStatesInit;

use crate::interface::events::{PlayerLeavingWorld, UpdateWorldStates};

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone)]
pub enum WorldStateAnswer {
    /// `SMSG_INIT_WORLD_STATES`.
    Init(Box<WorldStatesInit>),
    /// `SMSG_UPDATE_WORLD_STATE`.
    Update { state: u32, value: i32 },
}

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<WorldStateAnswer> {
    match event {
        PlayerEvent::WorldStatesInit(init) => Some(WorldStateAnswer::Init(init.clone())),
        PlayerEvent::WorldStateUpdate { state, value } => {
            Some(WorldStateAnswer::Update { state: *state, value: *value })
        }
        _ => None,
    }
}

/// The world state table and the rows listed from it.
#[derive(Resource, Default)]
pub struct WorldStates {
    /// The map and zone the last `SMSG_INIT_WORLD_STATES` named; `None` before
    /// the first one of a session.
    place: Option<(u32, u32)>,
    values: HashMap<u32, i32>,
    /// Whether the character is in LocalDefense, as of the last check.
    outdoor: bool,
    /// The listed `WorldStateUI.dbc` row ids, in file order.
    listed: Vec<u32>,
    table: Option<Arc<WorldStateUi>>,
}

impl WorldStates {
    /// The value of a world state; 0 for one the server never sent.
    pub fn value(&self, state: u32) -> i32 {
        self.values.get(&state).copied().unwrap_or(0)
    }

    /// `GetNumWorldStateUI()`.
    pub fn count(&self) -> usize {
        self.listed.len()
    }

    /// `GetWorldStateUIInfo(index)`, 1-based. `None` for an index outside the
    /// list, which the client answers with a single 0.
    pub fn info(&self, index: usize) -> Option<WorldStateInfo> {
        let id = *self.listed.get(index.checked_sub(1)?)?;
        let row = self.table.as_ref()?.get(id)?;
        Some(row.info(|state| self.value(state)))
    }

    /// Rebuild the list for the current place. Nothing is listed before the
    /// first `SMSG_INIT_WORLD_STATES` or without the table.
    fn relist(&mut self) {
        self.listed = match (self.place, self.table.as_ref()) {
            (Some((map, zone)), Some(table)) => table.listed(map, zone, self.outdoor),
            _ => Vec::new(),
        };
    }

    fn init(&mut self, init: &WorldStatesInit) {
        self.place = Some((init.map, init.zone));
        self.values = init.states.iter().copied().collect();
        self.relist();
    }
}

pub struct WorldStatePlugin;

impl Plugin for WorldStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<WorldStateAnswer>()
            .init_resource::<WorldStates>()
            .add_systems(Update, apply.in_set(super::GameSet));
    }
}

/// Whether a joined channel opens the outdoor rows: its `ChatChannels.dbc`
/// row carries [`OUTDOOR_CHANNEL_FLAGS`].
fn in_defense_channel(host: Option<&crate::lua::host::LuaHost>) -> bool {
    let Some(host) = host else {
        return false;
    };
    let board = host.channels().borrow();
    let Some(table) = board.table.as_ref() else {
        return false;
    };
    let joined = board.joined().any(|(_, slot)| {
        table
            .get(slot.zone_id)
            .is_some_and(|row| row.flags & OUTDOOR_CHANNEL_FLAGS == OUTDOOR_CHANNEL_FLAGS)
    });
    joined
}

/// Fold the packets into the table, follow the LocalDefense condition, and
/// raise `UPDATE_WORLD_STATES` once per frame in which anything changed.
fn apply(
    assets: Res<crate::assets::GameAssets>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut answers: MessageReader<WorldStateAnswer>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut states: ResMut<WorldStates>,
    mut raise: MessageWriter<UpdateWorldStates>,
) {
    if leaving.read().next().is_some() {
        let table = states.table.take();
        *states = WorldStates { table, ..Default::default() };
    }
    let mut changed = false;
    for answer in answers.read() {
        if states.table.is_none() {
            states.table = Some(Arc::new(load_table(&assets)));
        }
        match answer {
            WorldStateAnswer::Init(init) => states.init(init),
            WorldStateAnswer::Update { state, value } => {
                states.values.insert(*state, *value);
            }
        }
        changed = true;
    }
    let outdoor = in_defense_channel(host.as_deref());
    if outdoor != states.outdoor {
        states.outdoor = outdoor;
        if states.place.is_some() {
            states.relist();
            changed = true;
        }
    }
    if changed {
        raise.write(UpdateWorldStates);
    }
}

/// `WorldStateUI.dbc`, or an empty table when the archives do not have it, in
/// which case the frame lists nothing.
fn load_table(assets: &crate::assets::GameAssets) -> WorldStateUi {
    assets
        .with_archive(|archive| Ok(archive.read(&dbc_path("WorldStateUI")).ok()))
        .ok()
        .flatten()
        .and_then(|raw| WorldStateUi::parse(&raw).ok())
        .unwrap_or_else(|| {
            warn!("WorldStateUI.dbc did not load; the world state frame will list nothing");
            WorldStateUi::default()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::worldstate::{RowType, WorldStateRow};

    fn table() -> WorldStateUi {
        let row = |id, map, zone, row_type, state_variable, text: &str| WorldStateRow {
            id,
            map,
            zone,
            icon: String::new(),
            text: text.to_string(),
            tooltip: String::new(),
            state_variable,
            row_type: RowType::from_code(row_type),
            dynamic_icon: String::new(),
            dynamic_tooltip: String::new(),
            extended_ui: String::new(),
            extended_states: [0; 3],
        };
        WorldStateUi::from_rows(vec![
            row(2, 489, 0, 0, 2339, "%1581w/%1601w"),
            row(136, 0, 139, 1, 0, "Towers Controlled: %2327w"),
        ])
    }

    #[test]
    fn an_init_lists_the_place_and_an_index_reads_one_row() {
        let mut states = WorldStates { table: Some(Arc::new(table())), ..Default::default() };
        assert_eq!(states.count(), 0);
        states.init(&WorldStatesInit {
            map: 489,
            zone: 3277,
            states: vec![(2339, 1), (1581, 2), (1601, 3)],
        });
        assert_eq!(states.count(), 1);
        let info = states.info(1).unwrap();
        assert_eq!((info.state, info.text.as_str()), (1, "2/3"));
        assert_eq!(states.info(0), None);
        assert_eq!(states.info(2), None);
    }

    /// An init replaces the table: a value the new zone does not send reads 0.
    #[test]
    fn an_init_forgets_the_previous_zone() {
        let mut states = WorldStates { table: Some(Arc::new(table())), ..Default::default() };
        states.init(&WorldStatesInit { map: 489, zone: 3277, states: vec![(1581, 2)] });
        states.init(&WorldStatesInit { map: 0, zone: 139, states: vec![(2327, 4)] });
        assert_eq!(states.value(1581), 0);
        assert_eq!(states.count(), 0, "the outdoor rows wait for LocalDefense");
        states.outdoor = true;
        states.relist();
        assert_eq!(states.info(1).unwrap().text, "Towers Controlled: 4");
    }
}
