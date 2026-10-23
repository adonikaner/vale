//! Graveyards: the safe places of `WorldSafeLocs.dbc` on the open map, drawn
//! in the world, picked, dragged, made and removed, and the server rows that
//! say which zones each serves.
//!
//! A safe place is a client table row: a map, a position and a name. The
//! client reads it to draw the spirit healer's place on the world map; the
//! server reads the same file from `DataDir` to know where a released spirit
//! appears. Which place serves which zone is the server's alone:
//!
//! ```text
//! game_graveyard_zone     a link from a safe place to a zone or area, for one
//!                         side or both
//! world_safe_locs_facing  the way a spirit faces on appearing
//! ```
//!
//! What an edit to the client table is, is `vale_edit::dbc::places`; what the
//! two server tables hold is `vale_mangos::graveyard`. This file is the
//! pointer, the drawing, and the reads and writes of the server rows, on
//! `crate::tools::places::triggers`' terms. The panel is `crate::ui::tools::places::graveyards`.
//!
//! ## What the pointer does
//!
//! ```text
//! click a place               select it
//! drag a place                move it across the horizontal plane
//! Ctrl + drag                 move it up and down instead
//! New place armed + click     make a safe place on the ground there
//! Link zone armed + click     link the selected place to the zone clicked
//! Delete                      remove the selected place
//! Escape                      disarm, then drop the selection
//! ```
//!
//! ## Where an edit reaches the game
//!
//! The client reads the table from the project's copy on the next playtest,
//! and the server reads its copy from `DataDir\5875\dbc\`, which Client tables
//! on the Server panel writes. Links are live on `.reload game_graveyard_zone`;
//! a facing needs a restart.

use super::flightpaths::MARKER_RADIUS;
use super::triggers::{create_row, remove_row, shown};
use crate::tools::points::{self, MapPoint, PointTool, Points, HANDLE_PIXELS};
use crate::tools::Tool;
use crate::marks::{Look, Marks};
use crate::server::store::Gesture;
use crate::session::EditSession;
use vale_assets::tables::area::Areas;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::dbc::places::{self, SafeLoc};
use vale_edit::dbc::DbcFile;
use vale_mangos::graveyard::{self, Link};
use vale_mangos::row::{Assignment, Edits, Life};
use bevy::prelude::*;
use crate::server::fresh::FreshReads;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What a click on empty ground does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Armed {
    #[default]
    Nothing,
    /// Make a safe place on the ground.
    NewPlace,
    /// Link the selected place to the zone clicked.
    LinkZone,
}

/// The two server tables as the database holds them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Held {
    pub(crate) links: Vec<Link>,
    pub(crate) facings: HashMap<u32, f32>,
}

/// One link as the panel draws it: the project's edits over the database.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShownLink {
    pub(crate) link: Link,
    /// The row as the database holds it, for an edit typed back to that value
    /// to be cleared. `None` for a row the project creates.
    pub(crate) in_database: Option<Link>,
    pub(crate) life: Life,
}

/// The graveyard tool's state.
#[derive(Resource, Debug, Default)]
pub(crate) struct Graveyards {
    /// The safe places on the open map, and what the pointer is doing with
    /// them.
    pub(crate) points: Points<SafeLoc>,
    pub(crate) armed: Armed,
    /// The zone under the pointer, for Link zone: the area's zone, or the
    /// area itself when it is a zone.
    pub(crate) pointed_zone: Option<u32>,
    /// `AreaTable` as this session has it, for zone names, and the table
    /// revision it was read at; see `crate::tools::ground::areas::Areas::follow_table`.
    pub(crate) areas: Option<Arc<Areas>>,
    areas_at: Option<u64>,
    /// The two server tables, read at once and kept until an apply moves
    /// `EditSession::database_writes`. See [`Self::held`].
    reads: FreshReads<(), Held>,
}

/// How far from the camera a place is drawn and picked, in yards.
const MARKER_RANGE: f32 = 3000.0;

/// The name a new place is given, to be replaced.
pub(crate) const NEW_NAME: &str = "New graveyard";

impl Graveyards {
    /// The two server tables, once read.
    pub(crate) fn held(&self) -> Option<&Held> {
        self.reads.value()
    }

    /// Why the last read of the server tables answered nothing, when it
    /// answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.reads.trouble()
    }

    pub(crate) fn place(&self, id: u32) -> Option<&SafeLoc> {
        self.points.get(id)
    }

    pub(crate) fn stale(&mut self) {
        self.points.stale();
    }

    /// Read `AreaTable` again from the session's open copy when it changed.
    pub(crate) fn follow_table(&mut self, session: &EditSession) {
        let Some(open) = session.table(crate::tables::zones::area::TABLE) else {
            self.areas = None;
            self.areas_at = None;
            return;
        };
        if self.areas_at == Some(session.table_revision) {
            return;
        }
        self.areas = Areas::parse(&open.write()).map(Arc::new);
        self.areas_at = Some(session.table_revision);
    }

    /// A zone's or an area's name, with its id when the table has no name.
    pub(crate) fn zone_name(&self, id: u32) -> String {
        match self.areas.as_ref().and_then(|areas| areas.get(id)) {
            Some(area) if !area.name.is_empty() => area.name.clone(),
            _ => format!("area {id}"),
        }
    }

    /// The place's links as the project leaves them: the database's, then the
    /// ones the project creates. A creation the database holds because it was
    /// applied is listed once, as a creation.
    pub(crate) fn links_of(&self, edits: &Edits, id: u32) -> Vec<ShownLink> {
        let Some(held) = self.held() else {
            return Vec::new();
        };
        let created = |link: &Link| edits.row(graveyard::ZONE, &link.key()).is_some_and(|row| row.life == Life::Insert);
        let mut out: Vec<ShownLink> = held
            .links
            .iter()
            .filter(|link| link.safe_loc == id && !created(link))
            .filter_map(|link| {
                let (row, life) = shown(edits, graveyard::ZONE, &link.key(), Some(&link.assignments()))?;
                Some(ShownLink { link: Link::from_row(&row)?, in_database: Some(link.clone()), life })
            })
            .collect();
        let id_text = id.to_string();
        for (table, key, row) in edits.rows() {
            let ours = key.0.iter().any(|(column, value)| column == "id" && *value == id_text);
            if table != graveyard::ZONE || row.life != Life::Insert || !ours {
                continue;
            }
            if let Some((row, life)) = shown(edits, graveyard::ZONE, key, None) {
                if let Some(link) = Link::from_row(&row) {
                    out.push(ShownLink { link, in_database: None, life });
                }
            }
        }
        out
    }

    /// The place's facing as the project leaves it, in radians, and whether
    /// the database holds the row.
    pub(crate) fn facing(&self, edits: &Edits, id: u32) -> Option<(f32, bool)> {
        let held = self.held()?.facings.get(&id).copied();
        let assignments = held.map(facing_assignments);
        let (row, life) = shown(edits, graveyard::FACING, &graveyard::facing_key(id), assignments.as_deref())?;
        if life == Life::Delete {
            return None;
        }
        use vale_mangos::schema::RowValue;
        Some((row.number("orientation")? as f32, held.is_some()))
    }
}

fn facing_assignments(orientation: f32) -> Vec<Assignment> {
    vec![Assignment { column: "orientation", value: vale_mangos::sql::float(orientation) }]
}

/// Link a place to a zone for both sides. Refused when the place already
/// serves it.
pub(crate) fn add_link(session: &mut EditSession, graveyards: &Graveyards, id: u32, zone: u32, now: f64) -> Result<String, String> {
    let shown = graveyards.links_of(&session.rows.edits, id);
    let existing = shown.iter().find(|shown| shown.link.zone == zone);
    if existing.is_some_and(|shown| shown.life != Life::Delete) {
        return Err(format!("graveyard {id} is already linked to {}", graveyards.zone_name(zone)));
    }
    let link = existing.map(|shown| shown.link.clone()).unwrap_or_else(|| Link::new(id, zone));
    let key = link.key();
    let label = format!("{} {}", graveyard::ZONE, key.text());
    let gesture = Gesture { label: "Link graveyard", subject: &label, now };
    let held = existing.is_some_and(|shown| shown.in_database.is_some());
    create_row(session, graveyard::ZONE, &key, held, &link.assignments(), gesture);
    Ok(format!("graveyard {id} linked to {} ({zone})", graveyards.zone_name(zone)))
}

pub(crate) fn remove_link(session: &mut EditSession, shown: &ShownLink, now: f64) {
    let key = shown.link.key();
    let label = format!("{} {}", graveyard::ZONE, key.text());
    let gesture = Gesture { label: "Unlink graveyard", subject: &label, now };
    remove_row(session, graveyard::ZONE, &key, shown.in_database.is_some(), gesture);
}

/// Set which side a link serves: 0 both, 67 Horde, 469 Alliance.
pub(crate) fn set_faction(session: &mut EditSession, shown: &ShownLink, faction: u32, now: f64) {
    let key = shown.link.key();
    let label = format!("{} {} faction", graveyard::ZONE, key.text());
    let gesture = Gesture { label: "Edit graveyard link", subject: &label, now };
    let held = shown.in_database.as_ref().map(Link::assignments);
    crate::subjects::services::write_column(session, graveyard::ZONE, &key, held.as_deref(), "faction", faction.to_string(), gesture);
}

/// Set the way a spirit faces at the place, in radians.
pub(crate) fn set_facing(session: &mut EditSession, graveyards: &Graveyards, id: u32, radians: f32, now: f64) {
    let held = graveyards.held().and_then(|held| held.facings.get(&id)).copied();
    let key = graveyard::facing_key(id);
    let label = format!("{} {}", graveyard::FACING, key.text());
    let gesture = Gesture { label: "Edit graveyard facing", subject: &label, now };
    let value = vale_mangos::sql::float(radians);
    let claimed = session.rows.edits.row(graveyard::FACING, &key).is_some();
    match (held, claimed) {
        (None, false) => create_row(session, graveyard::FACING, &key, false, &facing_assignments(radians), gesture),
        (held, _) => {
            let assignments = held.map(facing_assignments);
            crate::subjects::services::write_column(session, graveyard::FACING, &key, assignments.as_deref(), "orientation", value, gesture);
        }
    }
}

/// Write a place's position as one gesture.
pub(crate) fn move_place(session: &mut EditSession, place: &SafeLoc, to: Vec3, now: f64) {
    use vale_assets::tables::safeloc::fields as sf;
    crate::tables::fields::set_fields(
        session,
        places::SAFE_LOCS,
        place.record,
        &[(sf::X, to.x.to_bits()), (sf::Y, to.y.to_bits()), (sf::Z, to.z.to_bits())],
        "Move graveyard",
        &format!("WorldSafeLocs {} position", place.id),
        now,
    );
}

/// Make a safe place on the ground at `at`. Answers the status line.
pub(crate) fn new_place(session: &mut EditSession, graveyards: &mut Graveyards, at: Vec3) -> String {
    match places::new_safe_loc(&mut session.tables, graveyards.points.map, at.to_array(), NEW_NAME) {
        Ok(done) => {
            let made = done.made;
            super::flightpaths::record(session, "Add graveyard", done);
            graveyards.stale();
            graveyards.points.selected = made;
            format!("graveyard {} made; link it to a zone for the server to use it", made.unwrap_or(0))
        }
        Err(e) => e.to_string(),
    }
}

/// Remove a place, and the server rows that name it. Answers the status line.
pub(crate) fn remove_place(session: &mut EditSession, graveyards: &mut Graveyards, id: u32, now: f64) -> String {
    let edits = session.rows.edits.clone();
    let links = graveyards.links_of(&edits, id);
    let facing = graveyards.facing(&edits, id);
    let done = match places::remove_safe_loc(&mut session.tables, id) {
        Ok(done) => done,
        Err(e) => return e.to_string(),
    };
    super::flightpaths::record(session, "Remove graveyard", done);
    let subject = format!("WorldSafeLocs {id} removed");
    session.as_one("Remove graveyard", &subject, |session| {
        for shown in links.iter().filter(|shown| shown.life != Life::Delete) {
            remove_link(session, shown, now);
        }
        if let Some((_, held)) = facing {
            let key = graveyard::facing_key(id);
            let gesture = Gesture { label: "Remove graveyard", subject: &subject, now };
            remove_row(session, graveyard::FACING, &key, held, gesture);
        }
    });
    graveyards.points.selected = None;
    graveyards.points.confirm_remove = None;
    graveyards.stale();
    format!("graveyard {id} removed")
}

/// The area id of the chunk under a point, where its tile is open.
pub(crate) fn area_at(session: &EditSession, at: Vec3) -> Option<u32> {
    let coord = vale_assets::tile_for_position(at.x, at.y);
    let tile = session.tiles.get(&coord)?;
    let chunk = vale_edit::adt::heights::chunk_at(tile, at.x, at.y)?;
    vale_edit::adt::area::area(tile, chunk)
}

/// The zone an area is in: its parent, or itself when it is a zone.
pub(crate) fn zone_of(areas: Option<&Areas>, area: u32) -> u32 {
    areas.and_then(|areas| areas.zone_of(area)).map_or(area, |zone| zone.id)
}

pub(crate) struct GraveyardToolPlugin;

impl Plugin for GraveyardToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Graveyards>()
            .add_systems(
                Update,
                (
                    points::collect::<Graveyards>,
                    read_the_rows,
                    point_at_zone,
                    points::aim::<Graveyards>,
                    press,
                    points::drag::<Graveyards>,
                    keys,
                )
                    .chain()
                    .after(crate::pick::aim),
            )
            .add_systems(Update, draw.after(keys));
    }
}

fn active(tool: &Tool, state: &crate::playtest::Playtest) -> bool {
    *tool == Tool::Graveyards && state.editing()
}

impl MapPoint for SafeLoc {
    fn id(&self) -> u32 {
        self.id
    }

    fn position(&self) -> Vec3 {
        Vec3::from(self.at)
    }
}

impl PointTool for Graveyards {
    type Point = SafeLoc;
    const TOOL: Tool = Tool::Graveyards;
    const TABLE: &'static str = places::SAFE_LOCS;
    const RANGE: f32 = MARKER_RANGE;
    /// Wider than a trigger's centre: a graveyard is picked by its centre
    /// alone, with no volume to fall back on.
    const PICK_PIXELS: f32 = HANDLE_PIXELS * 1.5;
    const NOUN: &'static str = "graveyard";

    fn points(&self) -> &Points<SafeLoc> {
        &self.points
    }

    fn points_mut(&mut self) -> &mut Points<SafeLoc> {
        &mut self.points
    }

    fn rebuild(&mut self, table: &DbcFile, map: u32) {
        self.points.list = places::safe_locs(table).into_iter().filter(|place| place.map == map).collect();
    }

    fn follow(&mut self, session: &EditSession) {
        self.follow_table(session);
    }

    fn set_position(session: &mut EditSession, place: &SafeLoc, to: Vec3, now: f64) {
        move_place(session, place, to, now);
    }

    fn remove(&mut self, session: &mut EditSession, id: u32, now: f64) -> String {
        remove_place(session, self, id, now)
    }
}

/// Read the two server tables while the tool is open, and forget them when an
/// apply has moved the database.
fn read_the_rows(
    mut graveyards: ResMut<Graveyards>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(session) = session else { return };
    let writes = session.database_writes;
    graveyards.reads.renew(writes);
    match graveyards.reads.poll() {
        Some(Ok(())) => {
            if let Some(held) = graveyards.held() {
                info!("graveyards: {} link(s), {} facing(s)", held.links.len(), held.facings.len());
            }
        }
        Some(Err(e)) => warn!("graveyards: {e}"),
        None => {}
    }
    let patch = crate::server::settings::server_patch(&settings);
    graveyards.reads.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            use vale_mangos::schema::RowValue;
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let links = db.rows(&graveyard::links_query(patch))?.iter().filter_map(Link::from_row).collect();
            let facings = db
                .rows(&graveyard::facings_query())?
                .iter()
                .filter_map(|row| Some((row.integer("id")? as u32, row.number("orientation")? as f32)))
                .collect();
            Ok(Held { links, facings })
        }))
    });
}

/// The zone under the pointer, for Link zone.
fn point_at_zone(
    mut graveyards: ResMut<Graveyards>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    session: Option<Res<EditSession>>,
) {
    graveyards.pointed_zone = match (active(&tool, &state), cursor.ground, session.as_deref()) {
        (true, Some(at), Some(session)) => area_at(session, at).map(|area| zone_of(graveyards.areas.as_deref(), area)),
        _ => None,
    };
}

#[allow(clippy::too_many_arguments)]
fn press(
    mut graveyards: ResMut<Graveyards>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    match (graveyards.armed, graveyards.points.hovered) {
        (Armed::NewPlace, _) => {
            if let Some(at) = cursor.surface {
                graveyards.armed = Armed::Nothing;
                let line = new_place(session, &mut graveyards, at);
                session.status = line;
            }
        }
        (Armed::LinkZone, _) => {
            let (Some(zone), Some(id)) = (graveyards.pointed_zone, graveyards.points.selected) else {
                session.status = "no open ground with an area under the pointer".to_string();
                return;
            };
            graveyards.armed = Armed::Nothing;
            let line = match add_link(session, &graveyards, id, zone, time.elapsed_secs_f64()) {
                Ok(line) | Err(line) => line,
            };
            session.status = line;
        }
        (Armed::Nothing, Some(id)) => graveyards.points.grab(id, &windows),
        (Armed::Nothing, None) => {}
    }
}

fn keys(
    mut graveyards: ResMut<Graveyards>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || wants.wants_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if graveyards.armed != Armed::Nothing {
            graveyards.armed = Armed::Nothing;
        } else if graveyards.points.confirm_remove.is_some() {
            graveyards.points.confirm_remove = None;
        } else {
            graveyards.points.selected = None;
        }
    }
    if !keys.just_pressed(KeyCode::Delete) {
        return;
    }
    if let Some(session) = session.as_mut() {
        points::delete_selected(&mut *graveyards, session, time.elapsed_secs_f64());
    }
}

/// Draw the places near the camera: a dome on the ground with a post and a
/// crossbar over it, and, where its facing is known, an arrow the way a
/// spirit faces. All [`Look::Ghosted`], since each is a thing to aim at.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut marks: ResMut<Marks>,
    graveyards: Res<Graveyards>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    session: Option<Res<EditSession>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active(&tool, &state) {
        return;
    }
    let (Some(eye), Some(session)) = (points::eye(&camera), session) else {
        return;
    };
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    let linked: HashSet<u32> = graveyards
        .held()
        .into_iter()
        .flat_map(|held| held.links.iter().map(|link| link.safe_loc))
        .collect();
    for place in &graveyards.points.list {
        let at = Vec3::from(place.at);
        if !graveyards.points.near(place, eye, MARKER_RANGE) {
            continue;
        }
        let colour = match graveyards.points.highlight(place.id) {
            Some(colour) => colour,
            // A place no zone links to is never used by the server.
            None if graveyards.held().is_some() && !linked.contains(&place.id) => Color::srgb(0.6, 0.6, 0.65),
            None => Color::srgb(0.55, 0.8, 1.0),
        };
        let radius = marks.visible_radius(bevy(at), MARKER_RADIUS);
        marks.dome(bevy(at), radius, colour, Look::Ghosted);
        let top = at + Vec3::Z * radius * 5.0;
        let post = radius * 0.12;
        marks.tube(bevy(at), bevy(top), post, colour, Look::Ghosted);
        // A cross at the top, as the world map draws a spirit healer.
        let arm = radius * 1.2;
        marks.tube(bevy(top - Vec3::X * arm), bevy(top + Vec3::X * arm), post, colour, Look::Ghosted);
        if let Some((facing, _)) = graveyards.facing(&session.rows.edits, place.id) {
            let ahead = at + Vec3::new(facing.cos(), facing.sin(), 0.0) * radius * 4.0;
            let lift = Vec3::Z * radius * 0.3;
            marks.arrow(bevy(at + lift), bevy(ahead + lift), post, colour, Look::Ghosted);
        }
    }
    let Some(pointer) = cursor.surface else { return };
    match graveyards.armed {
        Armed::NewPlace => {
            let radius = marks.visible_radius(bevy(pointer), MARKER_RADIUS);
            marks.dome(bevy(pointer), radius, Color::srgb(0.55, 0.8, 1.0), Look::Ghosted);
        }
        Armed::LinkZone => {
            if let Some(place) = graveyards.points.selected.and_then(|id| graveyards.place(id)) {
                marks.line(bevy(Vec3::from(place.at)), bevy(pointer), Color::srgb(0.5, 1.0, 0.5), Look::Ghosted);
            }
        }
        Armed::Nothing => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(links: Vec<Link>) -> Graveyards {
        let mut graveyards = Graveyards::default();
        graveyards.reads.insert((), Held { links, facings: HashMap::new() });
        graveyards
    }

    #[test]
    fn a_place_lists_its_own_links_and_the_ones_the_project_makes() {
        let graveyards = held(vec![Link::new(4, 12), Link::new(4, 40), Link::new(5, 12)]);
        let mut edits = Edits::default();
        assert_eq!(graveyards.links_of(&edits, 4).len(), 2);
        let made = Link::new(4, 85);
        let row = crate::subjects::services::creation(&made.assignments());
        edits.set_row_line(graveyard::ZONE, &made.key(), Some(&row.to_line()));
        let shown = graveyards.links_of(&edits, 4);
        assert_eq!(shown.len(), 3);
        assert!(shown.iter().any(|shown| shown.link.zone == 85 && shown.in_database.is_none() && shown.life == Life::Insert));
        assert_eq!(graveyards.links_of(&edits, 5).len(), 1);
    }

    #[test]
    fn a_removed_link_is_shown_as_removed() {
        let graveyards = held(vec![Link::new(4, 12)]);
        let mut edits = Edits::default();
        let removal = vale_mangos::row::RowEdit {
            life: Life::Delete,
            ..Default::default()
        };
        edits.set_row_line(graveyard::ZONE, &Link::new(4, 12).key(), Some(&removal.to_line()));
        let shown = graveyards.links_of(&edits, 4);
        assert_eq!(shown[0].life, Life::Delete);
    }

    /// A link the project created and applied is in the database's reading
    /// and still the project's creation: it is listed once, as the creation.
    #[test]
    fn an_applied_link_is_listed_once() {
        let made = Link::new(4, 85);
        let graveyards = held(vec![made.clone()]);
        let mut edits = Edits::default();
        let row = crate::subjects::services::creation(&made.assignments());
        edits.set_row_line(graveyard::ZONE, &made.key(), Some(&row.to_line()));
        let shown = graveyards.links_of(&edits, 4);
        assert_eq!(shown.len(), 1);
        assert_eq!((shown[0].in_database.is_none(), shown[0].life), (true, Life::Insert));
    }

    #[test]
    fn an_area_with_no_zone_is_its_own() {
        assert_eq!(zone_of(None, 1519), 1519);
    }

    #[test]
    fn a_link_belongs_to_the_place_its_id_column_names() {
        // The link key carries the place id under `id` only; a zone of the
        // same number is not this place's.
        let graveyards = held(Vec::new());
        let mut edits = Edits::default();
        let other = Link::new(12, 4);
        let row = crate::subjects::services::creation(&other.assignments());
        edits.set_row_line(graveyard::ZONE, &other.key(), Some(&row.to_line()));
        assert!(graveyards.links_of(&edits, 4).is_empty());
    }
}
