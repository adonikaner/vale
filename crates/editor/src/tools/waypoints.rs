//! The path a creature walks, drawn on the ground and edited there.
//!
//! This is the second subject in this editor whose document is a row in
//! vmangos' database, and the first whose edit is a set of rows. A
//! `creature_movement` row is one point of one path; a path is all of them
//! under one key, in order, and the tool edits the whole path as one unit. See
//! [`vale_mangos::path`] for the reason and the server source it comes from.
//!
//! ## It is a mode of the creature tool, not a tool of its own
//!
//! A path has no existence apart from the creature that walks it: the table is
//! keyed by a spawn's guid or a template's entry, and there is no way to point
//! at one in the world without first pointing at the creature. So there is no
//! entry on the rail. [`crate::tools::creatures`] selects a creature, the
//! **Waypoints** button opens its path here, and this takes the pointer while
//! it is open.
//!
//! Once open, the window follows the selection, as the template window beside
//! it does: clicking another creature shows that creature's path. So
//! [`Waypoints::showing`] is whether the window is open and [`Waypoints::open`]
//! is which path is in it, and they change at different moments.
//!
//! ## Pointer gestures: select, drag and add
//!
//! ```text
//! click a node          select it
//! drag a selected node  move it, at whatever height the ground under it is
//! click the ground      add a node there, while Add is armed
//! ```
//!
//! All three act only on presses over the world, never on presses over the
//! interface. `crate::ui::over_the_world` is the guard and is required: without
//! it, Add took every click in its own window. The button that turns Add off
//! added a point and stayed armed, and a click on a row of the point list added
//! a point instead of selecting it.
//!
//! The first two are [`crate::tools::creatures`]' own rule and are here for its
//! reason: without the select-then-drag split, every click on a node moved it a
//! fraction of a yard, because the pointer travels between the press and the
//! release of an ordinary click.
//!
//! The third is armed rather than modal-by-modifier, because a held modifier is
//! not something a scripted run can press. It still needs a pointer, so
//! `--waypoint-add <n>` is the scripted stand-in: it rings the creature with
//! points on the terrain, which is what makes the store, the undo, the SQL and
//! the apply checkable with nobody at the keyboard.
//!
//! ## What is drawn is the project's path, not the database's
//!
//! [`Waypoints::path`] is the database's reading with the project's store laid
//! over it, exactly as `creatures::Spawn::with_edits` does for a spawn's
//! columns and for the same reason: without it a node dragged across a field
//! would move nothing on screen until it had been applied and read back.
//!
//! ## A node takes the ground's height by default
//!
//! [`Waypoints::follow_ground`] is on by default: a node added or dragged takes
//! the height of the terrain under the pointer. That is right for the creatures
//! this tool is for and wrong for two groups: anything flying, and anything
//! indoors, whose floor is a building rather than the terrain. So it is a
//! switch, and with it off a drag keeps the node's own height and moves it in
//! the horizontal plane only.
//!
//! `vale waypoints <map>` is the check that has no window: it reports every
//! node's distance from the terrain as a distribution, and says why that is a
//! measurement rather than a fault.

use super::Tool;
use crate::marks::{Look, Marks};
use crate::session::{EditSession, Gesture};
use vale_client::world::camera::WorldCamera;
use vale_mangos::path::{self, Node, Path, Walk, Which};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};

/// What the pointer is doing with the open creature's path.
///
/// [`Default`] is written out rather than derived because of one field: the
/// module comment says a point takes the ground's height by default, and a
/// derived `Default` set `follow_ground` to false, so the switch on the window
/// was drawn unticked.
#[derive(Resource, Debug)]
pub struct Waypoints {
    /// Whether the window is open. Off by default, which is the ordinary
    /// state.
    ///
    /// Separate from [`Self::open`], which is which path is loaded, because
    /// the two change at different moments: the window is opened and shut by
    /// its button, and what it holds follows whichever creature is selected.
    /// An earlier version had only [`Self::open`], so the window stayed on the
    /// creature it was opened for while the template window beside it followed
    /// the selection, and the two panels showed different creatures for the
    /// same click.
    pub showing: bool,
    /// The creature the window is about. This is not the same as the table its
    /// path is in; see [`PathSubject`].
    pub subject: Option<PathSubject>,
    /// Whose path is loaded, and out of which table. `None` when the window is
    /// shut, when it is open with no creature selected, and until the read that
    /// decides which table has come back.
    pub open: Option<(Which, u64)>,
    /// What the database holds for it, read once when it is opened.
    ///
    /// Kept beside the working copy so the panel can say what an edit is
    /// against, and so a path can be reverted to it without a round trip.
    pub from_database: Option<Path>,
    /// The read, while it is running.
    task: Option<Task<Result<Path, String>>>,
    /// The `EditSession::database_writes` [`Self::from_database`] was read
    /// at. The path is read again when it moves; see `crate::server::fresh`.
    read_at: Option<u64>,
    /// Whether [`Self::from_database`] is an empty spawn path standing in for
    /// the database's because the project gives the spawn a path of its own.
    /// It is read again when the project stops claiming that path, after a
    /// discard or a project switch, or the database's path stays hidden.
    stands_in: bool,
    /// Why there is nothing, when there is nothing.
    pub trouble: Option<String>,
    /// Which node's form is open, as an index into the drawn path.
    pub selected: Option<usize>,
    /// Which node is under the pointer, as an index into the drawn path.
    pub hovered: Option<usize>,
    /// A held drag, once it has travelled far enough to be one.
    pub drag: Option<NodeDrag>,
    /// Whether a click on the ground adds a node. Off by default: with it
    /// on, the pointer cannot be used to select a creature.
    pub adding: bool,
    /// Whether a node placed or moved takes the ground's height — see the
    /// module comment.
    pub follow_ground: bool,
    /// A fly-to asked for by the panel, consumed by [`fly`].
    pub fly_to: Option<Vec3>,
    /// What the spawn's `movement_type` is, so the panel can say whether the
    /// path is walked at all. Read with the path.
    pub movement_type: u32,
    /// Whether `--waypoint-add` has finished, or was never asked for.
    ///
    /// `crate::server::creatures::on_the_command_line` waits on it. Without
    /// that, `--apply-creatures` fires on the first frame there is a session
    /// and this fires several seconds later, when the map query and the path
    /// query have both come back — so the apply ran against an empty store and
    /// reported "nothing to apply" while the edit it was meant to apply landed
    /// afterwards. Measured: 04:42:49 against 04:42:54.
    pub scripted_done: bool,
}

impl Default for Waypoints {
    fn default() -> Waypoints {
        Waypoints {
            showing: false,
            subject: None,
            open: None,
            from_database: None,
            task: None,
            read_at: None,
            stands_in: false,
            trouble: None,
            selected: None,
            hovered: None,
            drag: None,
            adding: false,
            // On, as the module comment states, because that is right for the
            // creatures this tool is for. Turned off for anything flying or
            // indoors.
            follow_ground: true,
            fly_to: None,
            movement_type: 0,
            scripted_done: false,
        }
    }
}

/// The creature a path is being edited for, and the ids needed to find the
/// path the server would use for it.
///
/// Two ids rather than one because vmangos resolves a creature's path in two
/// steps. `WaypointManager::GetDefaultPath` takes `creature_movement` keyed by
/// the spawn's guid, and only if there is none falls back to
/// `creature_movement_template` keyed by the template's entry.
///
/// An earlier version read only the first table. A creature whose path is a
/// template path (Princess in Elwynn, entry 330, nine nodes under
/// `creature_movement_template`) opened a window reading "This creature has no
/// path" while walking its patrol in the game. This was reported twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathSubject {
    /// The `creature` row's guid, which keys `creature_movement`.
    pub guid: u64,
    /// The template's entry, which keys `creature_movement_template`.
    pub entry: u32,
    /// `creature.movement_type`, so the panel can say whether the path is
    /// walked at all.
    pub movement_type: u32,
}

/// A node being dragged.
#[derive(Debug, Clone, Copy)]
pub struct NodeDrag {
    pub index: usize,
    /// Where it was when the button went down, so a status line can say how far
    /// it moved and an escape could put it back.
    pub from: Vec3,
    /// Where the pointer was, in pixels, for the travel threshold.
    pub pressed_at: Vec2,
    /// The offset between the node and the point the pointer met the ground,
    /// measured on the first frame the drag actually moves.
    pub grab: Option<Vec3>,
    pub moving: bool,
}

/// How far the pointer must travel before a drag moves anything, in pixels.
///
/// `creatures::DRAG_PIXELS`' value and its reason: a click on a node is a
/// selection, and without a threshold every one of them nudged the node.
const DRAG_PIXELS: f32 = 6.0;

/// How near the pointer has to be to a node to pick it, in pixels.
const GRAB_PIXELS: f32 = 14.0;

/// The colour of a path's legs. The path colours are named constants because
/// the four are a set and are read against each other rather than on their own.
///
/// A soft teal rather than the bright cyan an earlier version used. Saturated
/// blue over the brown of a road is tiring to look at over an editing session,
/// and a path is on screen for the whole session.
const LEG: Color = Color::srgb(0.38, 0.76, 0.78);

/// The leg back to the first node, which the creature walks and which is not
/// part of the drawn sequence — dimmer, so the direction of travel reads.
const LEG_HOME: Color = Color::srgb(0.24, 0.45, 0.47);

/// A node with nothing special about it.
const NODE: Color = Color::srgb(0.46, 0.84, 0.86);

impl Waypoints {
    /// The path as it is drawn: the database's reading with the project's
    /// store over it.
    pub fn path(&self, session: Option<&EditSession>) -> Option<Path> {
        let (which, owner) = self.open?;
        let from_database = self
            .from_database
            .clone()
            .unwrap_or_else(|| Path::empty(which, owner));
        Some(match session {
            Some(session) => session.server_paths.over(&from_database),
            None => from_database,
        })
    }

    /// Whether the project changes this path at all.
    pub fn edited(&self, session: Option<&EditSession>) -> bool {
        let Some((which, owner)) = self.open else {
            return false;
        };
        session.is_some_and(|session| session.server_paths.touches(which, owner))
    }

    /// What the open creature's movement type does with a path.
    pub fn walk(&self) -> Walk {
        Walk::of(self.movement_type)
    }

    /// Whether the path being edited is the template's, which is shared by
    /// every spawn of this creature that has none of its own.
    ///
    /// The window states this before anything else, for the same reason the
    /// creature panel shows "this spawn" over "its template": an edit to each
    /// reaches a different set of creatures, and the path does not show which.
    pub fn editing_the_template(&self) -> bool {
        matches!(self.open, Some((Which::Template, _)))
    }

    /// Open a creature's path, dropping whatever was open.
    ///
    /// Which table it comes out of is not decided here: it is what the read
    /// finds, following `GetDefaultPath`'s own order. See [`PathSubject`].
    pub fn open_for(&mut self, guid: u64, entry: u32, movement_type: u32) {
        if self.subject.map(|subject| subject.guid) == Some(guid) {
            return;
        }
        self.subject = Some(PathSubject {
            guid,
            entry,
            movement_type,
        });
        self.open = None;
        self.movement_type = movement_type;
        self.from_database = None;
        self.selected = None;
        self.hovered = None;
        self.drag = None;
        self.trouble = None;
        self.adding = false;
    }

    /// Forget what the database held, so the open path is read again.
    ///
    /// The counterpart of `Creatures::forget`, called from the same four places
    /// for the same reason: after an apply or a revert the rows have changed,
    /// and a reading taken before it disagrees on screen with the current rows.
    /// Without it the window kept showing the path as it was when the creature
    /// was clicked.
    ///
    /// The selection is kept. It is an index into a path that is about to be
    /// read again as very nearly the same path, and dropping it would close the
    /// node form while it is being edited.
    pub fn forget(&mut self) {
        self.from_database = None;
    }

    /// Shut the window and drop the open path.
    pub fn close(&mut self) {
        *self = Waypoints {
            showing: false,
            subject: None,
            follow_ground: self.follow_ground,
            // Not a fact about the creature that was open: a window shut after
            // a scripted add must not put the apply back to waiting for one.
            scripted_done: self.scripted_done,
            ..Waypoints::default()
        };
    }

    /// Write a path into the project's store, under one undo entry.
    ///
    /// Every edit in this file goes through here, which is what makes each of
    /// them undoable and each of them mark the project unsaved. The subject is
    /// the path rather than the operation, so a drag's sixty frames fold into
    /// one entry — see `EditSession::set_server_path`.
    fn write(&self, session: &mut EditSession, path: &Path, now: f64, label: &'static str) {
        let subject = format!("{} {} path", path.which.table(), path.owner);
        session.set_server_path(
            path.which,
            path.owner,
            Some(path),
            Some(Gesture {
                label,
                subject: &subject,
                now,
            }),
        );
    }
}

/// The height a node should take at a point, by the ground switch.
fn height_for(follow_ground: bool, ground: Vec3, keep: f32) -> f32 {
    match follow_ground {
        true => ground.z,
        false => keep,
    }
}

pub struct WaypointToolPlugin;

impl Plugin for WaypointToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Waypoints>()
            .add_systems(
                Update,
                // After the pick, like every tool here, and after the creature
                // tool's own press so that a click cannot be taken by both.
                (
                    follow_the_selection,
                    read_the_path,
                    aim,
                    press,
                    drag,
                    draw,
                    fly,
                )
                    .chain()
                    // After the creature tool's own press, which is what moves the
                    // selection this follows.
                    .after(super::creatures::press),
            )
            .add_systems(
                Update,
                // `--waypoint-add`, which is one shot and waits for the read it
                // needs rather than being ordered against it.
                on_the_command_line.after(read_the_path),
            );
    }
}

/// Point the window at whichever creature is selected.
///
/// The template window beside this one draws `creatures.chosen_edited`, the
/// current selection, so clicking another creature changes what it shows.
/// This window used to hold the creature it was opened for, so the two panels
/// showed different creatures for the same click.
///
/// [`Waypoints::open_for`] is a no-op for the creature already loaded, so this
/// runs every frame and costs a comparison; when the selection does move it
/// drops the reading, the chosen node and the drag, and the next frame reads
/// the new path.
///
/// With the window open and nothing selected the path is dropped, so the
/// window disappears rather than going on showing a path belonging to a
/// creature that is no longer chosen. That is the template window's behaviour
/// too.
fn follow_the_selection(
    mut waypoints: ResMut<Waypoints>,
    creatures: Res<super::creatures::Creatures>,
    state: Res<crate::playtest::Playtest>,
) {
    if !waypoints.showing || !state.editing() {
        return;
    }
    let chosen = creatures
        .selected
        .and_then(|guid| creatures.spawns.iter().find(|spawn| spawn.guid == guid));
    match chosen {
        Some(spawn) => waypoints.open_for(spawn.guid, spawn.entry, spawn.movement_type),
        None => {
            waypoints.subject = None;
            // Not `close()`: the window is still open, it simply has nothing to
            // show, and shutting it would mean a click on empty ground closed a
            // panel the person had deliberately opened.
            waypoints.open = None;
            waypoints.from_database = None;
            waypoints.selected = None;
            waypoints.drag = None;
        }
    }
}

/// Read the open creature's path, on a task.
///
/// Re-read when the selection changes and when an apply has put something
/// different in the database, on `creatures::read_the_map`'s rule — never on
/// the project's edit counter, which moves on every keystroke.
fn read_the_path(
    mut waypoints: ResMut<Waypoints>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    state: Res<crate::playtest::Playtest>,
) {
    if let Some(task) = waypoints.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            waypoints.task = None;
            let subject = waypoints.subject;
            match done {
                Ok(path) => {
                    info!(
                        "waypoints: {} node(s) read from {}",
                        path.nodes.len(),
                        path.which.table()
                    );
                    waypoints.open = Some((path.which, path.owner));
                    waypoints.from_database = Some(path);
                    waypoints.stands_in = false;
                    waypoints.trouble = None;
                }
                Err(e) => {
                    warn!("waypoints: {e}");
                    waypoints.trouble = Some(e);
                    // An empty spawn path rather than none, so the panel can
                    // still offer to build one.
                    if let Some(subject) = subject {
                        waypoints.open = Some((Which::Spawn, subject.guid));
                        waypoints.from_database = Some(Path::empty(Which::Spawn, subject.guid));
                    }
                }
            }
            // A path this project has given the spawn takes precedence over
            // whatever the database answers. Without this, re-opening the
            // window on a creature whose own path exists only as an unapplied
            // edit read the template's path from the database and drew that,
            // so the project's edit was not visible until it had been applied.
            if let (Some(subject), Some(session)) = (subject, session.as_deref()) {
                if session.server_paths.touches(Which::Spawn, subject.guid)
                    && waypoints.open != Some((Which::Spawn, subject.guid))
                {
                    waypoints.open = Some((Which::Spawn, subject.guid));
                    waypoints.from_database = Some(Path::empty(Which::Spawn, subject.guid));
                    waypoints.stands_in = true;
                }
            }
        }
        return;
    }
    // Read again when the database has been written since the path was read,
    // or when the empty path standing in for a claimed one has lost its claim.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let claim_gone = match (waypoints.subject, session.as_deref()) {
        (Some(subject), Some(session)) => {
            waypoints.stands_in && !session.server_paths.touches(Which::Spawn, subject.guid)
        }
        _ => false,
    };
    if waypoints.from_database.is_some() && (waypoints.read_at != Some(writes) || claim_gone) {
        waypoints.from_database = None;
        waypoints.stands_in = false;
    }
    if !state.editing() || waypoints.from_database.is_some() {
        return;
    }
    waypoints.read_at = Some(writes);
    let Some(subject) = waypoints.subject else {
        return;
    };
    if session.is_none() {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        waypoints.trouble = Some(vale_mangos::conn::Where::absent());
        waypoints.open = Some((Which::Spawn, subject.guid));
        waypoints.from_database = Some(Path::empty(Which::Spawn, subject.guid));
        return;
    };
    waypoints.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        // `GetDefaultPath`'s order: the spawn's own path first, the template's
        // only when there is none. Reading only one table made a creature
        // walking a template patrol report having no path; see [`PathSubject`].
        let read = |db: &mut vale_mangos::conn::Db, which: Which, owner: u64| {
            let rows = db.rows(&path::path_query(which, owner))?;
            let mut nodes: Vec<(u32, Node)> = rows.iter().filter_map(path::node_from_row).collect();
            // Sorted by the point number the table stores, not by the order
            // the rows arrived: a path read out of order draws a creature
            // walking a tangle, and nothing on screen says why.
            nodes.sort_by_key(|(point, _)| *point);
            Ok::<Path, String>(Path {
                which,
                owner,
                nodes: nodes.into_iter().map(|(_, node)| node).collect(),
            })
        };
        let own = read(&mut db, Which::Spawn, subject.guid)?;
        if !own.nodes.is_empty() {
            return Ok(own);
        }
        let shared = read(&mut db, Which::Template, u64::from(subject.entry))?;
        // Neither answered: the creature has no path anywhere, and a new one
        // should be its own rather than one shared by every spawn of its kind.
        // That is the narrower edit, and it is what somebody building a patrol
        // for one guard means.
        Ok(match shared.nodes.is_empty() {
            true => own,
            false => shared,
        })
    }));
}

/// Which node the pointer is over, by its distance in pixels on screen.
///
/// In pixels rather than in yards, for `creatures::aim`'s reason: a node twenty
/// yards off is a speck and a node underfoot fills the screen, and a grab
/// radius in world units is either unusable close up or impossible far away.
fn aim(
    mut waypoints: ResMut<Waypoints>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Creatures || waypoints.open.is_none() {
        waypoints.hovered = None;
        return;
    }
    // A pointer over a panel is not over the world, and a node behind the
    // waypoint window must not be highlighted under it. `creatures::aim` makes
    // the same check for the same reason.
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        waypoints.hovered = None;
        return;
    }
    // A held drag keeps what it grabbed.
    if waypoints.drag.is_some() {
        return;
    }
    let Some(path) = waypoints.path(session.as_deref()) else {
        return;
    };
    let Ok((camera, camera_at)) = camera.single() else {
        return;
    };
    let Some(pointer) = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position())
    else {
        waypoints.hovered = None;
        return;
    };

    let mut best: Option<(f32, usize)> = None;
    for (index, node) in path.nodes.iter().enumerate() {
        let at = vale_client::render::axes::to_bevy([node.x, node.y, node.z]);
        let Ok(on_screen) = camera.world_to_viewport(camera_at, at) else {
            continue;
        };
        let away = on_screen.distance(pointer);
        if away > GRAB_PIXELS {
            continue;
        }
        // Nearest on screen wins, which for two nodes stacked in the picture is
        // the one the pointer is actually over rather than the one nearer the
        // camera — a path that doubles back has both.
        if best.is_none_or(|(had, _)| away < had) {
            best = Some((away, index));
        }
    }
    waypoints.hovered = best.map(|(_, index)| index);
}

/// Choose a node, arm a drag, or add a node.
///
/// Three outcomes from one button, and which one is decided before the press is
/// looked at: with Add armed a click on the ground appends, otherwise a click
/// on a node selects it and a click on an already-selected node arms a drag.
fn press(
    mut waypoints: ResMut<Waypoints>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
) {
    if !state.editing() || *tool != Tool::Creatures || waypoints.open.is_none() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // Only a press over the world counts. Without this check, Add took every
    // click in the window: the button that turns it off added a point and
    // stayed on, and clicking a row in the point list added a point instead of
    // selecting it. Every other tool here checks `crate::ui::over_the_world`
    // too.
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(mut path) = waypoints.path(Some(session)) else {
        return;
    };

    if waypoints.adding {
        // A click on the world appends a node, or inserts one after the
        // selected node when there is one, so the middle of a path can be
        // repaired rather than only its end extended.
        //
        // The surface and not the ground, so a patrol route laid over a bridge
        // or through a building's upper floor sits on it rather than under it —
        // see [`crate::pick`]. A node's z is what the server walks the creature
        // at, so a node dropped to the terrain under a bridge is a creature
        // walking through the river.
        let Some(ground) = cursor.surface else { return };
        let at = match waypoints.selected {
            Some(index) => index + 1,
            None => path.nodes.len(),
        };
        path.insert(at, Node::at(ground.x, ground.y, ground.z));
        waypoints.write(session, &path, time.elapsed_secs_f64(), "Add waypoint");
        waypoints.selected = Some(at);
        session.status = format!("added point {} of {}", at + 1, path.nodes.len());
        return;
    }

    let Some(index) = waypoints.hovered else {
        return;
    };
    if waypoints.selected != Some(index) {
        waypoints.selected = Some(index);
        // No drag starts here: the press that selects a node does not also
        // move it. See the module comment.
        return;
    }
    let from = path
        .nodes
        .get(index)
        .map(|node| Vec3::new(node.x, node.y, node.z))
        .unwrap_or_default();
    let at = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position())
        .unwrap_or_default();
    waypoints.drag = Some(NodeDrag {
        index,
        from,
        pressed_at: at,
        grab: None,
        moving: false,
    });
}

/// Move the held node, once the pointer has travelled far enough.
///
/// The whole path is written on every frame of the drag, under one gesture, for
/// `creatures::drag`'s reason: the drawing reads the project's store over the
/// database's, so a drag that only wrote at the end would move nothing until
/// the button came up. It costs one undo entry and one store entry however many
/// frames it takes, because the gesture names the path.
fn drag(
    mut waypoints: ResMut<Waypoints>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    windows: Query<&Window>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        waypoints.drag = None;
        return;
    }
    let Some(held) = waypoints.drag else { return };
    let Some(session) = session.as_mut() else {
        return;
    };

    if !buttons.pressed(MouseButton::Left) {
        waypoints.drag = None;
        if held.moving {
            if let Some(node) = waypoints
                .path(Some(session))
                .and_then(|path| path.nodes.get(held.index).cloned())
            {
                let now = Vec3::new(node.x, node.y, node.z);
                session.status = format!(
                    "moved point {} by {:.1} yards",
                    held.index + 1,
                    now.distance(held.from)
                );
            }
        }
        return;
    }

    if !held.moving {
        let at = windows
            .single()
            .ok()
            .and_then(|window| window.cursor_position())
            .unwrap_or(held.pressed_at);
        if at.distance(held.pressed_at) < DRAG_PIXELS {
            return;
        }
        waypoints.drag = Some(NodeDrag {
            moving: true,
            ..held
        });
        return;
    }

    let Some(ground) = cursor.surface else { return };
    let grab = match held.grab {
        Some(grab) => grab,
        None => {
            // Measured on the first frame the drag moves, which is also the
            // frame nothing is written: without it the node jumps under the
            // cursor the moment the threshold is crossed.
            let grab = held.from - ground;
            waypoints.drag = Some(NodeDrag {
                grab: Some(grab),
                ..held
            });
            return;
        }
    };
    let Some(mut path) = waypoints.path(Some(session)) else {
        return;
    };
    let Some(node) = path.nodes.get_mut(held.index) else {
        waypoints.drag = None;
        return;
    };
    node.x = ground.x + grab.x;
    node.y = ground.y + grab.y;
    node.z = height_for(waypoints.follow_ground, ground, node.z);
    let path = path.clone();
    waypoints.write(session, &path, time.elapsed_secs_f64(), "Move waypoint");
}

/// Draw the path: a dome on each node and a tube along each leg.
///
/// ## Why the hidden part of the path is drawn faint
///
/// Every part is [`Look::Ghosted`]: solid where nothing is in front of it, and
/// faint where a rise or a building is. Three earlier versions drew gizmo
/// lines and failed. A depth-tested path vanished behind every rise. Drawing
/// each leg twice, solid in place and dashed in front, flickered, because two
/// coincident translucent lines are visible as both and the blend order of two
/// gizmo groups is not stable from frame to frame. Choosing the pass per leg by
/// whether its nodes were under the terrain answered a different question from
/// whether anything is in front of the leg. A ghost pass is drawn only where the
/// solid pass is not, per pixel, so neither problem arises; see
/// [`crate::marks`].
fn draw(
    mut marks: ResMut<Marks>,
    waypoints: Res<Waypoints>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
) {
    if !state.editing() || *tool != Tool::Creatures {
        return;
    }
    let Some(path) = waypoints.path(session.as_deref()) else {
        return;
    };
    let Ok(camera) = camera.single() else { return };
    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));

    let point = |node: &Node| vale_client::render::axes::to_bevy([node.x, node.y, node.z]);

    // Closed when the generator repeats, which both of them do: the leg
    // from the last node back to the first is one the creature really walks,
    // and a path drawn open reads as ending where it does not.
    let legs = path.nodes.len();
    for index in 0..legs {
        let next = (index + 1) % legs;
        if next == index {
            break;
        }
        // The leg home is dimmer, so the direction of travel is readable in a
        // picture: it is the only one that is not part of the drawn sequence.
        let home = next == 0;
        let (from, to) = (point(&path.nodes[index]), point(&path.nodes[next]));
        let colour = match home {
            true => LEG_HOME,
            false => LEG,
        };
        marks.wide_line(from, to, 1.6, colour, Look::Ghosted);
    }

    for (index, node) in path.nodes.iter().enumerate() {
        let chosen = waypoints.selected == Some(index);
        let under = waypoints.hovered == Some(index);
        let at = point(node);
        let here = Vec3::new(node.x, node.y, node.z);
        // The same screen-proportional radius the creature markers use, so a
        // node stays grabbable at any distance.
        // One and a half times the radius an earlier version used: a node is
        // a thing to aim at, and these are drawn at every distance from
        // underfoot to the far side of a zone.
        let radius = (here.distance(eye) * 0.015).clamp(0.5, 4.5);
        let colour = match (chosen, under, node.waittime > 0) {
            (true, _, _) => Color::srgb(1.0, 0.82, 0.25),
            (_, true, _) => Color::WHITE,
            // A node with a wait is where the creature actually stops, which is
            // the thing a person is looking for when they open this at all.
            (_, _, true) => Color::srgb(1.0, 0.70, 0.30),
            _ => NODE,
        };
        // A dome standing on the node, so a node on a slope reads as standing
        // on it. The one being aimed at is larger.
        let size = match chosen || under {
            true => radius * 0.9,
            false => radius * 0.7,
        };
        marks.dome(at, size, colour, Look::Ghosted);
        if chosen {
            // Which way it faces on arrival, when that is a thing that happens
            // — `Node::faces` is the rule, and it needs a wait as well as an
            // orientation.
            if node.faces() {
                let facing = Vec3::new(node.orientation.cos(), node.orientation.sin(), 0.0);
                let lift = Vec3::Z * (size * 0.5);
                let from = vale_client::render::axes::to_bevy((here + lift).to_array());
                let nose = vale_client::render::axes::to_bevy(
                    (here + lift + facing * (radius * 3.0)).to_array(),
                );
                let thickness = marks.line_radius(from) * 1.6;
                marks.arrow(from, nose, thickness, Color::srgb(1.0, 0.95, 0.6), Look::Ghosted);
            }
            if node.wander_distance > 0.1 {
                marks.flat_ring(
                    at,
                    node.wander_distance,
                    Color::srgba(1.0, 0.82, 0.25, 0.5),
                    Look::Ghosted,
                );
            }
        }
    }
}

/// `--waypoint-add <n>`: ring the chosen creature with points.
///
/// The scripted stand-in for placing a point with the pointer — see
/// [`crate::Args::waypoint_add`]. It waits for the path to have been read,
/// because the edit it makes is written over whatever the database holds and a
/// read landing afterwards would be read into `from_database` under an edit
/// that had already been recorded against the empty one.
///
/// The radius is the creature's own `wander_distance` when it has one and ten
/// yards otherwise. Ten is about the size of a guard's beat and is far enough
/// that the points are distinguishable on screen at editing distance.
///
/// Every point is on the terrain at its own position, whatever the ground
/// switch says: a scripted run has no pointer, so there is no ground under one
/// to take, and a ring at the creature's own height would sink into any slope.
/// The height comes out of the session's open tiles rather than the
/// archives, which is `pick::session_ground`'s source and for its reason: a
/// tile this project has raised is the ground a point should land on.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    mut waypoints: ResMut<Waypoints>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    creatures: Res<super::creatures::Creatures>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(count) = args.waypoint_add else {
        *done = true;
        waypoints.scripted_done = true;
        return;
    };
    let (Some((which, owner)), Some(session)) = (waypoints.open, session.as_mut()) else {
        return;
    };
    // The read has to have landed first — see this function's own doc.
    if waypoints.from_database.is_none() {
        return;
    }
    let Some(spawn) = creatures.spawns.iter().find(|spawn| spawn.guid == owner) else {
        *done = true;
        waypoints.scripted_done = true;
        warn!("--waypoint-add: no creature with guid {owner} on this map");
        return;
    };
    // The tile under the creature must also be open. This system runs about
    // sixty milliseconds after the map query lands, well before that tile has
    // been opened and parsed. Without this wait every point took the fallback
    // height and the ring was a flat disc buried in the slope. Measured: five
    // points at the creature's own 66.757, one of them 5.2 yards under the
    // terrain. Waiting costs a few frames of a scripted run and nothing else.
    let home = vale_assets::tile_for_position(spawn.at.x, spawn.at.y);
    if !session.tiles.contains_key(&home) {
        return;
    }
    *done = true;

    let radius = match spawn.wander > 0.5 {
        true => spawn.wander,
        false => 10.0,
    };
    let spawn_at = spawn.at;
    let count = count.max(2);
    let mut nodes_out = Vec::with_capacity(count);
    for step in 0..count {
        let angle = std::f32::consts::TAU * step as f32 / count as f32;
        let x = spawn_at.x + radius * angle.cos();
        let y = spawn_at.y + radius * angle.sin();
        // The ground under each point, so the ring follows a slope rather than
        // being a flat disc buried in it. The tile under the creature is open
        // by now; a point that crosses into a neighbour this session has not
        // opened keeps the creature's own height, which is the nearest thing to
        // right that is available without a read.
        let coord = vale_assets::tile_for_position(x, y);
        let z = session
            .tiles
            .get(&coord)
            .and_then(|tile| vale_edit::adt::heights::height_at(tile, x, y))
            .unwrap_or(spawn_at.z);
        nodes_out.push(Node::at(x, y, z));
    }
    let path = Path {
        which,
        owner,
        nodes: nodes_out,
    };
    let nodes = path.nodes.len();
    waypoints.write(session, &path, time.elapsed_secs_f64(), "Add waypoints");
    waypoints.selected = Some(0);
    waypoints.scripted_done = true;
    info!("--waypoint-add: {nodes} point(s) around guid {owner}, radius {radius:.1} yards");
    session.status = format!("{nodes} point(s) added around {owner}");
}

/// Take the camera to a node the panel asked for.
///
/// `creatures::fly`'s two lines: the target is in the world's own axes and the
/// camera stops wanting the ground, so it holds the height it is given rather
/// than dropping to the terrain under a node that is meant to be in the air.
fn fly(mut waypoints: ResMut<Waypoints>, mut camera: ResMut<crate::camera::EditorCamera>) {
    let Some(at) = waypoints.fly_to.take() else {
        return;
    };
    camera.target = at;
    camera.wants_the_ground = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_path() -> Path {
        Path {
            which: Which::Spawn,
            owner: 12345,
            nodes: vec![
                Node::at(0.0, 0.0, 0.0),
                Node::at(10.0, 0.0, 0.0),
                Node::at(10.0, 10.0, 0.0),
            ],
        }
    }

    /// Opening a creature drops everything about the last one. A selection
    /// carried across would be an index into a path that is not there, and the
    /// stale `from_database` would draw the previous creature's path under the
    /// new one's name.
    #[test]
    fn opening_another_creature_forgets_the_last() {
        let mut waypoints = Waypoints::default();
        waypoints.open_for(1, 1, 2);
        waypoints.from_database = Some(a_path());
        waypoints.selected = Some(2);
        waypoints.hovered = Some(1);
        waypoints.adding = true;

        waypoints.open_for(2, 2, 2);
        assert_eq!(waypoints.subject.map(|open| open.guid), Some(2));
        // Which table answers is the read's to decide, so it is not set yet.
        assert!(waypoints.open.is_none());
        assert!(waypoints.from_database.is_none());
        assert_eq!(waypoints.selected, None);
        assert_eq!(waypoints.hovered, None);
        assert!(
            !waypoints.adding,
            "Add must not stay armed across creatures"
        );
    }

    /// Opening the creature already open changes nothing, so a panel redrawing
    /// every frame does not throw away a selection.
    #[test]
    fn opening_the_same_creature_keeps_the_selection() {
        let mut waypoints = Waypoints::default();
        waypoints.open_for(1, 1, 2);
        waypoints.from_database = Some(a_path());
        waypoints.selected = Some(2);
        waypoints.open_for(1, 1, 2);
        assert_eq!(waypoints.selected, Some(2));
        assert!(waypoints.from_database.is_some());
    }

    /// The window stays open across a change of creature. It is what the
    /// follow depends on: `showing` is whether the window is up and `open` is
    /// what is in it, and re-pointing the second must not close the first.
    #[test]
    fn following_a_new_creature_leaves_the_window_open() {
        let mut waypoints = Waypoints {
            showing: true,
            ..Waypoints::default()
        };
        waypoints.open_for(1, 1, 2);
        assert!(waypoints.showing);
        waypoints.open_for(2, 2, 3);
        assert!(waypoints.showing, "re-pointing the window must not shut it");
        assert_eq!(waypoints.subject.map(|open| open.guid), Some(2));
        assert_eq!(
            waypoints.walk(),
            Walk::Cyclic,
            "and the movement type follows too"
        );
    }

    /// Only `close`, which the window's button calls, shuts the window;
    /// `forget` does not.
    #[test]
    fn closing_is_the_only_thing_that_shuts_the_window() {
        let mut waypoints = Waypoints {
            showing: true,
            ..Waypoints::default()
        };
        waypoints.open_for(1, 1, 2);
        waypoints.forget();
        assert!(waypoints.showing, "re-reading must not shut it either");
        waypoints.close();
        assert!(!waypoints.showing);
        assert!(waypoints.open.is_none() && waypoints.subject.is_none());
    }

    /// The ground switch is on by default, as the module comment states. A
    /// derived `Default` set it off, and the window drew it unticked.
    #[test]
    fn a_point_takes_the_ground_unless_it_is_told_not_to() {
        assert!(Waypoints::default().follow_ground);
        // Nothing else is on: a fresh tool has no path open, nothing
        // selected, and Add disarmed.
        let fresh = Waypoints::default();
        assert!(fresh.subject.is_none() && fresh.selected.is_none() && !fresh.adding);
    }

    /// Closing keeps the ground switch and drops the rest. The switch is a
    /// preference about how this tool behaves rather than a fact about the
    /// creature that was open.
    #[test]
    fn closing_keeps_the_preference_and_nothing_else() {
        let mut waypoints = Waypoints {
            follow_ground: true,
            ..Waypoints::default()
        };
        waypoints.open_for(1, 1, 2);
        waypoints.selected = Some(0);
        waypoints.close();
        assert!(waypoints.follow_ground);
        assert!(waypoints.open.is_none());
        assert!(waypoints.selected.is_none());
    }

    /// The height a node takes follows the switch: the ground's with it on, its
    /// own with it off. That is the difference between a path a flying creature
    /// can keep and one dragged onto the floor of the valley under it.
    #[test]
    fn the_ground_switch_decides_the_height() {
        let ground = Vec3::new(5.0, 5.0, 40.0);
        assert_eq!(height_for(true, ground, 300.0), 40.0);
        assert_eq!(height_for(false, ground, 300.0), 300.0);
    }

    /// With no session the path drawn is the database's, and the tool is still
    /// usable — which is what a machine with no project open sees.
    #[test]
    fn the_drawn_path_falls_back_to_the_database() {
        let mut waypoints = Waypoints::default();
        assert!(
            waypoints.path(None).is_none(),
            "nothing open, nothing drawn"
        );
        waypoints.open_for(12345, 12345, 2);
        // Nothing is drawn until the read says which table answered. The
        // spawn's own path and its template's are different rows that reach
        // different sets of creatures, and drawing one before knowing which
        // would show a guess the person could edit.
        assert!(waypoints.path(None).is_none(), "the table is not known yet");
        waypoints.open = Some((Which::Spawn, 12345));
        assert_eq!(waypoints.path(None), Some(Path::empty(Which::Spawn, 12345)));
        waypoints.from_database = Some(a_path());
        assert_eq!(waypoints.path(None), Some(a_path()));
    }

    /// What the open creature's movement type does with the path, which is the
    /// question that decides whether an edit here does anything at all.
    #[test]
    fn the_panel_can_say_whether_the_path_is_walked() {
        let mut waypoints = Waypoints::default();
        waypoints.open_for(1, 1, 0);
        assert_eq!(waypoints.walk(), Walk::Never);
        waypoints.open_for(2, 2, 2);
        assert_eq!(waypoints.walk(), Walk::Waypoint);
        waypoints.open_for(3, 3, 3);
        assert_eq!(waypoints.walk(), Walk::Cyclic);
    }
}
