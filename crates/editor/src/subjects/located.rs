//! Where a spawn named by guid stands and what it is, for the lists that name
//! spawns on any map: a game event's spawns and a pool's members.
//!
//! The spawn tools read the open map's spawns whole. A list in the Spawning
//! workspace names spawns on every map, so each spawn it draws is asked for
//! here by kind and guid ([`SpawnRef`]), and every one asked for since the
//! last read is read in one statement per kind
//! (`vale_mangos::creature::located_query`,
//! `vale_mangos::gameobject::located_query`). The answers are kept until an
//! apply lands (`EditSession::database_writes`). A spawn the project creates
//! is in no database and is answered as not found.

use crate::server::fresh::{Fresh, FreshTask};
use crate::session::EditSession;
use crate::tools::spawns::spawn::{GoTo, SpawnKind, SpawnRef};
use crate::ui::shell::asks::ShellAsk;
use bevy::prelude::*;
use std::collections::{BTreeSet, HashMap};
use vale_mangos::schema::RowValue;

/// Where one spawn stands and what it is.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Located {
    pub(crate) map: u32,
    /// In the world's own axes.
    pub(crate) at: Vec3,
    pub(crate) entry: u32,
    /// The template's name at its highest patch, or empty.
    pub(crate) name: String,
}

impl Located {
    /// What a list calls the spawn: its template and entry.
    pub(crate) fn label(&self) -> String {
        match self.name.is_empty() {
            true => format!("entry {}", self.entry),
            false => format!("{} ({})", self.name, self.entry),
        }
    }
}

/// The most guids one statement names.
const BATCH: usize = 2000;

/// The places read so far, and the ones asked for since.
#[derive(Resource, Default)]
pub(crate) struct Locations {
    read: Fresh<HashMap<SpawnRef, Option<Located>>>,
    task: FreshTask<Vec<(SpawnRef, Option<Located>)>>,
    wanted: BTreeSet<SpawnRef>,
}

impl Locations {
    /// Where a spawn stands: `None` while it is being read, which asking
    /// starts, and `Some(None)` for a guid no spawn row holds.
    pub(crate) fn find(&mut self, spawn: SpawnRef) -> Option<Option<&Located>> {
        if !self.read.contains_key(&spawn) {
            self.wanted.insert(spawn);
            return None;
        }
        Some(self.read.get(&spawn).and_then(Option::as_ref))
    }

    /// The shell's ask that selects a spawn under its tool and puts the
    /// camera over it, opening its map when it is on another and is read.
    pub(crate) fn go_to(&mut self, spawn: SpawnRef) -> ShellAsk {
        let map = self.find(spawn).flatten().map(|place| (place.map, place.at));
        ShellAsk::ShowSpawn { spawn, go: GoTo { guid: spawn.guid, map } }
    }
}

/// Read every spawn asked for since the last read, one statement per kind,
/// and forget them all when an apply has moved the tables.
fn read_the_places(
    mut places: ResMut<Locations>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let Some(session) = session else { return };
    let writes = session.database_writes;
    let places = &mut *places;
    if places.task.renew(writes) | places.read.renew(writes) {
        places.wanted.clear();
    }
    match places.task.poll() {
        Some(Ok(answered)) => {
            for (spawn, place) in answered {
                places.wanted.remove(&spawn);
                places.read.insert(spawn, place);
            }
        }
        Some(Err(e)) => warn!("spawn places: {e}"),
        None => {}
    }
    if places.wanted.is_empty() || places.task.is_busy() {
        return;
    }
    let asked: Vec<SpawnRef> = places
        .wanted
        .iter()
        .copied()
        .filter(|spawn| !places.read.contains_key(spawn))
        .take(BATCH)
        .collect();
    if asked.is_empty() {
        places.wanted.clear();
        return;
    }
    places.task.start(writes, || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let mut out: Vec<(SpawnRef, Option<Located>)> = asked.iter().map(|spawn| (*spawn, None)).collect();
            for kind in [SpawnKind::Creature, SpawnKind::GameObject] {
                let guids: Vec<u64> = asked.iter().filter(|spawn| spawn.kind == kind).map(|spawn| spawn.guid).collect();
                let query = match kind {
                    SpawnKind::Creature => vale_mangos::creature::located_query(&guids),
                    SpawnKind::GameObject => vale_mangos::gameobject::located_query(&guids),
                };
                let Some(query) = query else { continue };
                for row in db.rows(&query)? {
                    let Some(guid) = row.integer("guid") else { continue };
                    let number = |column: &str| row.number(column).unwrap_or(0.0) as f32;
                    let place = Located {
                        map: row.integer("map").unwrap_or(0).max(0) as u32,
                        at: Vec3::new(number("position_x"), number("position_y"), number("position_z")),
                        entry: row.integer("id").unwrap_or(0).max(0) as u32,
                        name: row.text("name").unwrap_or("").to_string(),
                    };
                    let spawn = SpawnRef { kind, guid: guid.max(0) as u64 };
                    if let Some(slot) = out.iter_mut().find(|(asked, _)| *asked == spawn) {
                        slot.1 = Some(place);
                    }
                }
            }
            Ok(out)
        }))
    });
}

/// `--go-to-spawn`: the ask a Go To button pushes, pushed once the spawn's
/// place is read, so the spawn's map is known.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut places: ResMut<Locations>,
    mut asks: ResMut<crate::ui::shell::asks::ShellAsks>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(spawn) = args.subjects.go_to_spawn else {
        *done = true;
        return;
    };
    if places.find(spawn).is_some() {
        asks.push(places.go_to(spawn));
        *done = true;
    }
}

pub(crate) struct LocatedPlugin;

impl Plugin for LocatedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Locations>()
            .init_resource::<crate::ui::shell::asks::ShellAsks>()
            .add_systems(Update, (on_the_command_line, read_the_places));
    }
}
