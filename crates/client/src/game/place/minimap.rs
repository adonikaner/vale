//! **Where the little round map is looking** — the four facts the painter needs
//! and the widget cannot know.
//!
//! `crate::lua::widgets::minimap` holds what a *script* can read and write: the zoom, and
//! the ping. Everything else about the minimap is the world's answer, and the
//! draw walk runs with no borrow of the world at all — so it comes across as a
//! resource rather than through the widget.
//!
//! It is four fields and none of them is new work. The position and the facing
//! are the mover's own, already on [`WorldStatus`]; the map *directory* is the
//! one `Map.dbc` name the session resolved at login (`Azeroth`, `Kalimdor`),
//! which is also the key
//! `textures\Minimap\md5translate.trs` is written against; and the indoor flag
//! is the building test [`super::worldmap::track_area`] was already making every
//! frame to name the room.
//!
//! ## Why the indoor flag is worth carrying at all
//!
//! 1.12 keeps **two** zoom levels and two tables — `minimapZoom` and
//! `minimapInsideZoom`, registered side by side, each indexing its own radius
//! table — and the client picks between them on one indoor flag. Indoors the radii are yards outright and much tighter (150 down
//! to 25, against 233 down to 67), because a building's minimap art is the tile
//! it stands on and a wide view of a city is a blur. Dropping the flag would not
//! fail; it would draw an inn at four times the reference's scale and look
//! deliberate.

//! ## The dots and the two arrows
//!
//! What the map draws over its tiles is the world's answer too, and it comes
//! across the same way: [`MinimapView::blips`] is every unit and object the
//! reference's classifier would list (`vale_assets::look::blips`), each as an offset in yards from the character, and
//! [`MinimapView::markers`] is the two places the map points at — the flag a
//! guard's directions put up, and the body a ghost is walking back to. The
//! offsets are in yards rather than on the disc because the disc's radius is
//! the *widget's* zoom, which lives on the Lua side; the painter projects.

use bevy::prelude::*;

use vale_assets::look::blips::{self, Kind, Tracking};

use crate::world::session::{Session, WorldEntity, WorldStatus};

/// One dot: which cell, and where it is relative to the character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blip {
    pub kind: Kind,
    /// Yards north (`+x`) and west (`+y`) of the character.
    pub north: f32,
    pub west: f32,
}

/// The two places the map points at when they are off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// `SMSG_GOSSIP_POI` — a guard's directions, with the `POIIcons` cell the
    /// packet named. See [`super::worldmap::MapLandmarks`].
    Poi { icon: u32 },
    /// The character's own corpse, while they are a ghost.
    Corpse,
}

/// A marker: inside the disc it is drawn as its icon where it is, beyond it
/// as an arrow at the rim pointing the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub kind: MarkerKind,
    pub north: f32,
    pub west: f32,
}

/// What the minimap is centred on this frame — see the module comment.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct MinimapView {
    /// `false` before the character is in the world, which is what makes the
    /// widget draw nothing rather than the middle of the map grid. The two glue
    /// screens and `--audit` are both this state.
    pub in_world: bool,
    /// The `Map.dbc` directory — `Azeroth`, `Kalimdor`, `PVPZone01`. The key
    /// `md5translate.trs` is written against, and empty before there is a map.
    pub directory: String,
    /// The character's world position: north (+x) and west (+y).
    pub position: (f32, f32),
    /// …and which way they are looking, in the server's own radians.
    pub facing: f32,
    /// Whether the character is inside a building, which chooses between the
    /// client's two radius tables — see the module comment.
    pub indoors: bool,
    /// The dots — see the module comment. Rebuilt every frame there is a world,
    /// as the reference rebuilds its five lists on every update.
    pub blips: Vec<Blip>,
    /// …and the two places the map points at.
    pub markers: Vec<Marker>,
}

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapView>()
            // **After the area poll**, which is what writes the indoor flag: a
            // frame's worth of disagreement between the two would zoom the map
            // in and out on every doorway.
            .add_systems(
                Update,
                (track.after(super::worldmap::track_area), collect)
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// Copy the four facts across, on the frame they change.
///
/// **Written unconditionally rather than on a change**, unlike every other
/// resource in this directory: the position moves every frame a character
/// walks, so a change test here would be a comparison that almost always
/// says yes for the cost of a comparison. It is four fields and a `String`
/// that is only cloned when the map id moves.
fn track(
    session: Res<Session>,
    status: Res<WorldStatus>,
    place: Res<super::worldmap::WorldMapState>,
    mut view: ResMut<MinimapView>,
) {
    let live = session.active.is_some() && status.in_world;
    if !live {
        if view.in_world {
            *view = MinimapView::default();
        }
        return;
    }
    if view.directory != status.map_name {
        view.directory = status.map_name.clone();
    }
    view.in_world = true;
    view.position = (status.position.x, status.position.y);
    view.facing = status.orientation;
    view.indoors = place.indoors;
}

/// A cap on the dots, so a city under Track Humanoids costs a bounded walk in
/// the painter. The reference's lists grow without one; two hundred is more
/// than fit on the disc.
const MOST_BLIPS: usize = 200;

/// **The dots and the markers**, off the world the frame is drawing.
///
/// The reference's classifier is walked over every entity in view;
/// the rule itself is [`vale_assets::look::blips`], and what is here is only
/// the gathering of its inputs: the character's own tracking fields, each unit's
/// flags and cached creature type, each object's lock, and the group roster.
///
/// **A group member's dot comes off the roster, not off the object** — the
/// reference appends its party list separately and skips a grouped player in
/// the object walk. Where the member is in view
/// their drawn position is used; where they are not,
/// `SMSG_PARTY_MEMBER_STATS`' two `int16`s are, if the zone puts them on this
/// map.
fn collect(
    mut view: ResMut<MinimapView>,
    units: Query<(&WorldEntity, Option<&Transform>)>,
    party: Res<crate::game::session::party::Party>,
    landmarks: Res<super::worldmap::MapLandmarks>,
    dying: Res<crate::game::character::death::Dying>,
    status: Res<WorldStatus>,
    assets: Res<crate::assets::GameAssets>,
) {
    view.blips.clear();
    view.markers.clear();
    if !view.in_world {
        return;
    }
    let here = view.position;
    let Some((me, tracking)) = units
        .iter()
        .find(|(unit, _)| unit.is_self)
        .map(|(unit, _)| (unit.guid, unit.tracking))
    else {
        return;
    };
    let tracking = tracking
        .map(|(creatures, resources, stealthed)| Tracking { creatures, resources, stealthed })
        .unwrap_or_default();
    let tables = assets.display_tables().ok();
    let offset = |transform: &Transform| {
        let at = crate::render::axes::to_wow(transform.translation);
        (at[0] - here.0, at[1] - here.1)
    };

    for (unit, transform) in units.iter() {
        if view.blips.len() >= MOST_BLIPS {
            break;
        }
        if unit.is_self || party.holds(unit.guid) {
            continue;
        }
        let Some(transform) = transform else {
            continue;
        };
        let kind = if unit.object_state.is_some() {
            // A game object — `GAMEOBJECT_STATE` is the one field nothing else
            // carries — and its lock against the resource mask.
            let Some(tables) = tables.as_ref() else {
                continue;
            };
            blips::resource_kind(&tracking, tables.locks().keys(unit.object_lock))
        } else {
            let owner = unit.charmed_by.or(unit.summoned_by);
            blips::unit_kind(
                &tracking,
                &blips::Unit {
                    creature_type: unit.creature_type,
                    vis_flags: unit.vis_flags,
                    hunters_marked: unit.hunters_marked,
                    alive: !unit.dead,
                    mine: owner == Some(me),
                    quest_turn_in: unit.quest_turn_in,
                },
            )
        };
        if let Some(kind) = kind {
            let (north, west) = offset(transform);
            view.blips.push(Blip { kind, north, west });
        }
    }

    // The group, off the roster. Each member once: the drawn position when
    // they are in view, the stats packet's when they are not.
    for index in 0..party.count() {
        let Some(member) = party.member(index) else {
            continue;
        };
        if member.guid == me {
            continue;
        }
        let seen = units
            .iter()
            .find(|(unit, transform)| unit.guid == member.guid && transform.is_some())
            .and_then(|(_, transform)| transform.map(offset));
        let far = || {
            let stats = member.stats.as_ref()?;
            let (x, y) = stats.position?;
            let zone = u32::from(stats.zone?);
            let map = tables.as_ref()?.areas()?.get(zone)?.map;
            (map == status.map_id).then(|| (f32::from(x) - here.0, f32::from(y) - here.1))
        };
        if let Some((north, west)) = seen.or_else(far) {
            view.blips.push(Blip { kind: Kind::Party, north, west });
        }
    }

    // The two markers: the guard's flag, on the map it was named on, and the
    // body, while there is one to walk back to.
    if let Some(poi) = landmarks.gossip.as_ref().filter(|_| landmarks.map == status.map_id) {
        view.markers.push(Marker {
            kind: MarkerKind::Poi { icon: poi.icon },
            north: poi.position.0 - here.0,
            west: poi.position.1 - here.1,
        });
    }
    if dying.ghost {
        if let Some(corpse) = dying.corpse.as_ref().filter(|c| c.corpse_map_id == status.map_id) {
            view.markers.push(Marker {
                kind: MarkerKind::Corpse,
                north: corpse.x - here.0,
                west: corpse.y - here.1,
            });
        }
    }
}
