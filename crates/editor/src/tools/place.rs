//! Placing a new doodad or building: a model on the cursor, placed with a click.
//!
//! ## One tool for doodads and buildings
//!
//! A doodad and a building are different records in different lists with
//! different rules. Those differences live in [`super::doodads`] and
//! [`super::wmos`]. Placing either takes the same steps: choose a model, see
//! it where it would go, click. So the arming, the preview, the turn and the
//! scale are implemented here once, and only the twenty lines that write the
//! record know which list they write to.
//!
//! ## The preview is the model, drawn translucent
//!
//! A marker or a box would be cheaper, but it does not show what a person
//! placing scenery needs to see: whether this tree sits right on this slope,
//! its size against the ground, its trunk against the rock behind it. An
//! outline loses all of that.
//!
//! Drawing the real model costs nothing extra because both parts already
//! exist. `ModelCache::lookup` and `WmoCache::lookup` return the same batches
//! the world is drawn from, and `Materials::with_opacity` is the client's own
//! translucency, the one used for a stealthed rogue and a spirit at the
//! graveyard. It also handles opaque batches: an opaque batch cannot blend,
//! because the blend mode is in the pipeline key, so it is forced to mode 2
//! and its real mode is stored. A ghost material written in this file would
//! have to repeat that.
//!
//! `render::ships` is the precedent for the spawn: a building's batches under
//! an entity whose transform is written every frame, with no `MODF` row behind
//! it.
//!
//! ## The ghost carries no placement marker
//!
//! Its batches carry no `Doodad` and no `WmoPart`, so no other tool sees it:
//! the doodad pick casts rays against `Doodad` boxes, the WMO pick against
//! `WmoPart`, and the reconcilers walk both. A ghost with either marker would
//! be selectable and movable, and, once `rehome` started enforcing one row per
//! id, a candidate for being collapsed into a real placement.
//!
//! ## Duplicate arms this tool instead of copying
//!
//! "Duplicate" does not drop a second copy on top of the first, which the user
//! would then have to find and drag off. It arms this tool with the selected
//! placement's model, rotation and scale, so the copy is on the cursor and
//! lands where the user clicks. Duplicating uses the same gesture as placing.
//!
//! ## Model paths are resolved in [`Placing::arm`]
//!
//! The two ways a path reaches this tool do not agree on the model's
//! extension. The picker's paths come from the archives' own listing and are
//! `.m2`: `vale catalogue` reports 5,816 `.m2` under `World\` and no `.mdx`. A
//! duplicate's path comes from the tile's `MMDX`, and `MMDX` holds `.mdx`, the
//! pre-1.0 extension no archive holds. Every reader in this project swaps it,
//! and `vale_assets::world::m2::model_path` is the one implementation of the
//! swap. This tool did not swap it.
//!
//! Nothing reported the failure. `ModelCache` failed the path, so the preview
//! never appeared; `resolve_doodads` failed it too and dropped the placement,
//! so the click drew nothing; and the record written into the file was valid,
//! so when anything caused the tile to be read again (a drag, an undo, a
//! playtest) every copy appeared at once. The bug report read "when
//! duplicating, the preview does not show, nor do the duplicated placements,
//! until you move something in the world", which matches that sequence.
//!
//! So [`Placing::arm`] resolves the path, and it is the only entry point. The
//! swap depends on the kind because it is the model rule: a `.wmo` path is
//! already what the archive holds, and passing one through `model_path` would
//! ask for `stormwind.wmo.m2`.
//!
//! ## Scattering: each drop is rolled inside stated bounds
//!
//! A forest placed one identical tree at a time looks stamped, and turning and
//! sizing each tree by hand is slow. With [`Scatter`] on, every drop rolls the
//! next one: a new turn about up, a size drawn between two bounds, and a tilt
//! off the vertical up to a stated angle. These are the three values `MDDF`
//! carries and the ghost shows. The roll happens after the click, so the model
//! on the cursor is always what the next click writes, and the "Roll now"
//! button re-draws it without placing anything. A building takes the turn
//! only: `MODF` has no scale, and a tilted building is a mistake nine times in
//! ten.
//!
//! The random numbers come from a 64-bit xorshift in this file, seeded from
//! the clock on first use. Three numbers a second do not justify a dependency
//! and the crates it would pull in, and nothing here needs to be reproducible.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_assets::world::adt::PlacedModel;
use vale_client::assets::GameAssets;
use vale_client::render::axes;
use vale_client::render::doodads::{PendingDoodads, PlacedDoodad};
use vale_client::render::models::{Lookup, Materials, ModelCache};
use vale_client::render::terrain::TerrainTile;
use vale_client::render::wmos::{self as wmo_render, PendingWmos, WmoCache};
use vale_edit::adt::place::{Building, Doodad as Placement, MINTED_BASE};
use vale_edit::ops::{Edit, Placements};
use bevy::camera::primitives::Aabb;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How opaque the preview is.
///
/// Low enough to read as not yet placed against any ground, high enough to
/// judge the model's silhouette and colour. At 0.2 the ghost is too faint to
/// judge. At 0.8 it cannot be told apart from a placed model, and the user
/// needs to see whether they have clicked.
const GHOST: f32 = 0.45;

/// Degrees a turn key turns the ghost, and the factor shift applies.
const TURN: f32 = 15.0;

/// Which list a placement is going into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Doodad = 0,
    Wmo = 1,
}

impl Kind {
    /// The file extensions the archives use for this kind.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Doodad => &[".m2", ".mdx"],
            Kind::Wmo => &[".wmo"],
        }
    }
}

/// Which half of a placement tool's panel is showing.
///
/// This is a mode of one tool, not a second tool. Choosing a model and moving
/// a placed one use the same list, the same records and the same panel, and a
/// person switches between them constantly while scattering scenery. A
/// separate rail entry would add a click between placing each tree and
/// adjusting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Pick what is there, move it, turn it.
    #[default]
    Select,
    /// Put something new in.
    Place,
}

/// The two modes, as a list for the panel.
pub const MODES: [(&str, Mode); 2] = [("Select", Mode::Select), ("Place", Mode::Place)];

/// How the next drop is rolled. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scatter {
    /// Whether anything is rolled at all.
    pub on: bool,
    /// A fresh turn about up, anywhere in the circle.
    pub turn: bool,
    /// The size, drawn between these two multipliers. Doodads only.
    pub size: (f32, f32),
    /// The most a drop leans off the vertical, in degrees. Doodads only, and
    /// zero is upright.
    pub tilt_max: f32,
}

impl Default for Scatter {
    fn default() -> Scatter {
        Scatter {
            on: false,
            turn: true,
            size: (0.9, 1.1),
            tilt_max: 0.0,
        }
    }
}

/// What is armed, and what it would look like.
#[derive(Resource)]
pub struct Placing {
    pub mode: Mode,
    /// The model on the cursor, or `None` when the tool is disarmed, which is
    /// its usual state.
    pub path: Option<String>,
    pub kind: Kind,
    /// Degrees about the world's up axis. This is the only axis set before a
    /// placement lands; the gizmo sets the other two afterwards.
    pub turn: f32,
    /// Degrees about the world's x and y axes: a lean off the vertical. Only
    /// [`Scatter`] writes it.
    pub tilt: [f32; 2],
    /// Whether a drop leans onto the slope under the cursor instead of taking
    /// the roll's lean. Doodads only; a building is never leaned. See
    /// `vale_assets::world::adt::lean_to_normal`, and [`Self::lean`], where
    /// the answer for this frame is kept.
    pub align: bool,
    /// The lean the ground under the cursor asks for this frame — the two
    /// world angles, at the current turn — written by [`ground_lean`] and
    /// read by the ghost and the drop, so the two cannot disagree.
    pub lean: [f32; 2],
    /// A multiplier, and doodads only — `MODF` carries no scale.
    pub scale: f32,
    /// How the next drop is rolled after each one.
    pub scatter: Scatter,
    /// The roll's state; zero until first used.
    seed: u64,
    /// Every model path the archives and the project name, per kind, read
    /// lazily and again when the project's files change.
    pub doodads: Vec<String>,
    pub wmos: Vec<String>,
    /// The `EditSession::files_revision` the two lists were read at, by kind.
    catalogue_for: [Option<u64>; 2],
    /// What the panel's search box holds, kept per kind so switching between
    /// them does not lose it.
    pub search: String,
    /// Which folder the picker is showing, one per kind, as a lower-case path
    /// prefix ending in a separator.
    ///
    /// One per kind because the two catalogues do not share a shape: a doodad
    /// is anywhere under `World\` and a building is always under `World\wmo\`,
    /// so a single field would reset the picker to the root on every tool
    /// switch. It is kept across a switch for the same reason as
    /// [`Self::search`]: the user is looking for one model, and a reset to the
    /// top loses their place.
    folders: [String; 2],
    /// The path of a model that failed to load, so the panel can report it.
    /// A missing preview looks the same as one still loading, and the two need
    /// different responses.
    pub trouble: Option<String>,
    /// The preview entity and what it is showing, so that changing the chosen
    /// model rebuilds it and re-choosing the same one does not.
    ghost: Option<Entity>,
    showing: Option<String>,
}

impl Default for Placing {
    fn default() -> Placing {
        Placing {
            mode: Mode::default(),
            path: None,
            kind: Kind::default(),
            turn: 0.0,
            tilt: [0.0; 2],
            align: false,
            lean: [0.0; 2],
            // One, not zero. A derived default would arm the tool at scale
            // zero, which writes a doodad into the file that the renderer
            // draws at no size, so the click appears to do nothing.
            scale: 1.0,
            scatter: Scatter::default(),
            seed: 0,
            doodads: Vec::new(),
            wmos: Vec::new(),
            catalogue_for: [None; 2],
            search: String::new(),
            // The building picker starts one level in. Every `.wmo` in the
            // game is under `World\wmo\`, so at the root the picker would
            // show one folder holding everything, and every user would have
            // to click into it first.
            folders: [ROOT.to_string(), format!("{ROOT}wmo{SEP}")],
            trouble: None,
            ghost: None,
            showing: None,
        }
    }
}

impl Placing {
    /// Whether a click in the world would place something.
    ///
    /// Both picks check this and ignore the click while it is true, because a
    /// click that places must not also select. See the module comment.
    pub fn armed(&self) -> bool {
        self.path.is_some()
    }

    /// Arm with a model, at a rotation and scale, and show the half of the panel
    /// that is about placing.
    ///
    /// The one entry point, used by the picker and by "Duplicate". See the
    /// module comment on why duplicating arms the tool instead of copying.
    pub fn arm(&mut self, kind: Kind, path: String, turn: f32, scale: f32) {
        self.kind = kind;
        // The path is resolved here and nowhere else. The module comment
        // describes what an unresolved `.mdx` caused.
        self.path = Some(match kind {
            Kind::Doodad => vale_assets::world::m2::model_path(&path),
            Kind::Wmo => path,
        });
        self.turn = turn;
        self.tilt = [0.0; 2];
        self.scale = scale;
        self.mode = Mode::Place;
        self.trouble = None;
    }

    /// Draw the next turn, size and tilt inside [`Scatter`]'s bounds, if it
    /// is on. Called after every drop and by the "Roll now" button.
    pub fn roll(&mut self) {
        if !self.scatter.on {
            return;
        }
        if self.seed == 0 {
            self.seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x9E37_79B9_7F4A_7C15)
                | 1;
        }
        if self.scatter.turn {
            self.turn = unit(&mut self.seed) * 360.0;
        }
        if self.kind == Kind::Doodad {
            let (low, high) = self.scatter.size;
            let (low, high) = (low.min(high), low.max(high));
            self.scale =
                (low + (high - low) * unit(&mut self.seed)).clamp(*SCALE.start(), *SCALE.end());
            let most = self.scatter.tilt_max.clamp(0.0, 90.0);
            // A lean of up to `most` in a random direction: an angle drawn
            // once and split between the two axes, so the corner of the
            // square is not twice as far as the side.
            let lean = unit(&mut self.seed) * most;
            let heading = unit(&mut self.seed) * std::f32::consts::TAU;
            self.tilt = [lean * heading.cos(), lean * heading.sin()];
        }
    }

    /// Clear the model on the cursor. `Escape` and a tool change call this.
    pub fn disarm(&mut self) {
        self.path = None;
        self.trouble = None;
    }

    /// The catalogue for a kind, which the panel lists.
    pub fn catalogue(&self, kind: Kind) -> &[String] {
        match kind {
            Kind::Doodad => &self.doodads,
            Kind::Wmo => &self.wmos,
        }
    }

    /// The folder the picker is showing for a kind, as a path prefix.
    pub fn folder(&self, kind: Kind) -> &str {
        &self.folders[kind as usize]
    }

    /// Set the folder the picker shows for a kind. A prefix with no trailing
    /// separator is given one, so that a folder cannot also match one whose
    /// name it is a prefix of.
    pub fn open_folder(&mut self, kind: Kind, prefix: &str) {
        let mut prefix = prefix.to_ascii_lowercase();
        if !prefix.is_empty() && !prefix.ends_with(SEP) {
            prefix.push(SEP);
        }
        self.folders[kind as usize] = prefix;
    }

    /// The folder a path is in, for opening the picker where the chosen model
    /// already is rather than at the top.
    pub fn folder_of(path: &str) -> String {
        match path.rfind(SEP) {
            Some(at) => path[..=at].to_ascii_lowercase(),
            None => ROOT.to_string(),
        }
    }
}

/// The next number in `0..1` from a 64-bit xorshift — see the module comment
/// on why it is not a crate.
fn unit(seed: &mut u64) -> f32 {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *seed = x;
    // The top 24 bits, which is what an `f32` can hold exactly.
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// The archives' own path separator, which is the one the picker's folders are
/// cut on. A backslash, given a name so that it reads as a separator and not as
/// an escape inside a condition.
pub const SEP: char = '\\';

/// Where the picker starts, and what its breadcrumb calls `World`.
///
/// Everything either catalogue holds is under it, since both are read from
/// `EditSession::listed` with the prefix `World\`, so a level above it would
/// be one folder with everything in it.
pub const ROOT: &str = "world\\";

pub struct PlaceToolPlugin;

impl Plugin for PlaceToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Placing>().add_systems(Update, scripted);
        app.add_systems(
            Update,
            (read_catalogue, aim, ground_lean, follow, commit)
                .chain()
                .after(crate::pick::aim)
                // Before both picks, so the click that places does not also
                // select whatever is behind it. A wrong order would produce
                // no error, so the order is stated here.
                .before(crate::tools::doodads::select)
                .before(crate::tools::wmos::select),
        );
    }
}

/// `--place` — open on the picker, and arm what it named.
///
/// Runs once, on the first frame the active tool is a placing tool. It is the
/// only way a scripted run reaches this half of the panel: the mode is set by
/// a click on a segmented control and the model by a click on a row, and
/// `--shot` can do neither. See [`crate::Args::place`], where the argument is.
fn scripted(
    mut placing: ResMut<Placing>,
    args: Res<crate::Args>,
    tool: Res<Tool>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(asked) = args.place.clone() else {
        *done = true;
        return;
    };
    let kind = match *tool {
        Tool::Doodads => Kind::Doodad,
        Tool::Wmos => Kind::Wmo,
        // Not a placement tool yet. `--place` with no `--tool` is a run that
        // wanted the doodad panel, so wait rather than giving up: the resource
        // is inserted before the first frame either way.
        _ => return,
    };
    *done = true;
    match asked {
        Some(path) => {
            // The picker opens on the model's own folder, where the next
            // model is most likely to be. A shot of the picker's rows also
            // needs this, because the root holds seventeen folders and no
            // models.
            placing.open_folder(kind, &Placing::folder_of(&path));
            placing.arm(kind, path, 0.0, 1.0);
        }
        // No model: the picker, showing its folders, with nothing on the cursor.
        None => {
            placing.kind = kind;
            placing.mode = Mode::Place;
        }
    }
}

/// Marks the preview, on the root and on every batch of it.
///
/// The batches carry it because the bounds walk looks for it and the root has
/// no mesh of its own; the root carries it because the root holds the
/// transform. Nothing else in this crate names it. See the module comment on
/// why a ghost must not carry `Doodad` or `WmoPart`.
#[derive(Component)]
struct Ghost;

/// Read a model list the first time it is wanted, and again after the
/// project's files change.
///
/// It is read lazily, like the tileset list, because it walks the listing of
/// nineteen archives and a session that never places anything should not pay
/// for that.
fn read_catalogue(
    mut placing: ResMut<Placing>,
    tool: Res<Tool>,
    assets: Res<GameAssets>,
    session: Option<Res<crate::session::EditSession>>,
) {
    let kind = match *tool {
        Tool::Doodads => Kind::Doodad,
        Tool::Wmos => Kind::Wmo,
        _ => return,
    };
    let Some(session) = session else { return };
    // Read again when the project's files change, so a model the project
    // adds is offered. See `EditSession::listed`.
    let at = match kind {
        Kind::Doodad => 0,
        Kind::Wmo => 1,
    };
    if placing.catalogue_for[at] == Some(session.files_revision) && !placing.catalogue(kind).is_empty() {
        return;
    }
    placing.catalogue_for[at] = Some(session.files_revision);
    let mut found: Vec<String> = session
        .listed(&assets, "World\\")
        .into_iter()
        .filter(|path| kind.extensions().iter().any(|end| path.ends_with(end)))
        // A `.wmo` that ends in `_NNN` is a group, not a building. A city is
        // one root and three hundred of those, and offering them would bury
        // the building under its own rooms.
        .filter(|path| kind != Kind::Wmo || !is_group(path))
        // Apply the same swap [`Placing::arm`] makes, so the two ways into
        // this tool agree whatever the listing holds. The archives hold
        // no `.mdx` (`vale catalogue`), so this changes nothing for them; a
        // patch archive or a project file that added one would otherwise put
        // an unopenable row in the list.
        .map(|path| match kind {
            Kind::Doodad => vale_assets::world::m2::model_path(&path),
            Kind::Wmo => path,
        })
        .collect();
    found.sort();
    found.dedup();
    // A marker, so a failed read is not retried every frame for the rest of the
    // session — the same guard the tileset list has.
    if found.is_empty() {
        found.push(String::new());
    }
    match kind {
        Kind::Doodad => placing.doodads = found,
        Kind::Wmo => placing.wmos = found,
    }
}

/// Whether a `.wmo` path names one of a building's groups rather than its root.
///
/// `stormwind.wmo` is the building; `stormwind_042.wmo` is one of its rooms.
fn is_group(path: &str) -> bool {
    let stem = path.trim_end_matches(".wmo");
    let tail = &stem[stem.len().saturating_sub(3)..];
    stem.len() > 4
        && tail.len() == 3
        && tail.bytes().all(|b| b.is_ascii_digit())
        && stem.as_bytes()[stem.len() - 4] == b'_'
}

/// Turn and scale the thing on the cursor, and put it away on `Escape`.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut placing: ResMut<Placing>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
) {
    // Disarm when the tool changes or editing stops. A model left on the
    // cursor after a switch to the terrain brush would be placed by the next
    // click of somebody who thinks they are painting.
    if !matches!(*tool, Tool::Doodads | Tool::Wmos) || !state.editing() {
        placing.disarm();
        return;
    }
    if placing.mode != Mode::Place {
        placing.disarm();
        return;
    }
    if !placing.armed() || wants.wants_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        placing.disarm();
        return;
    }

    let far = if keys.pressed(KeyCode::ShiftLeft) {
        3.0
    } else {
        1.0
    };
    if keys.just_pressed(KeyCode::Comma) {
        placing.turn = (placing.turn - TURN * far).rem_euclid(360.0);
    }
    if keys.just_pressed(KeyCode::Period) {
        placing.turn = (placing.turn + TURN * far).rem_euclid(360.0);
    }
    // The wheel with Control held scales the model, the same gesture both
    // brushes use for their own size. A building has no scale to change.
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    if in_world
        && placing.kind == Kind::Doodad
        && keys.pressed(KeyCode::ControlLeft)
        && scroll.delta.y != 0.0
    {
        let scaled = placing.scale * (1.0 + scroll.delta.y * 0.05);
        placing.scale = scaled.clamp(*SCALE.start(), *SCALE.end());
    }
}

/// What a placed doodad may be scaled to — `MDDF`'s own `u16` range, as a
/// multiplier. The same limits [`super::doodads`] edits within.
const SCALE: std::ops::RangeInclusive<f32> = (1.0 / 1024.0)..=(u16::MAX as f32 / 1024.0);

/// Keep the preview where the cursor is, building it when the model changes.
#[allow(clippy::too_many_arguments)]
fn follow(
    mut placing: ResMut<Placing>,
    cursor: Res<Cursor>,
    mut models: ResMut<ModelCache>,
    mut buildings: ResMut<WmoCache>,
    mut materials: Materials,
    mut commands: Commands,
    mut ghosts: Query<&mut Transform, With<Ghost>>,
) {
    // Nothing armed, or nowhere to stand: the preview goes away rather than
    // hanging in the air where the pointer last met the world.
    //
    // The surface and not the ground (see [`crate::pick`]), so a lamp dropped
    // on a bridge or a crate on a dock stands on it. The lean still comes from
    // the ground, because it is the terrain's slope: see [`ground_lean`].
    let (Some(path), Some(at)) = (placing.path.clone(), cursor.surface) else {
        if let Some(ghost) = placing.ghost.take() {
            commands.entity(ghost).despawn();
            placing.showing = None;
        }
        return;
    };

    if placing.showing.as_deref() != Some(path.as_str()) {
        if let Some(ghost) = placing.ghost.take() {
            commands.entity(ghost).despawn();
        }
        placing.showing = None;
        // The cache is asked every frame until it answers `Ready`. Both caches
        // load on a task and answer `Loading` meanwhile, the same way the
        // world's own placements load, so the preview appears as soon after a
        // model is chosen as the world would draw it.
        let (draws, failed) = match placing.kind {
            Kind::Doodad => match models.lookup(&path) {
                Lookup::Ready(assets) => (Some(assets.draws.clone()), false),
                Lookup::Failed => (None, true),
                Lookup::Loading => (None, false),
            },
            Kind::Wmo => match buildings.lookup(&path) {
                wmo_render::Lookup::Ready(ready) => (Some(ready.draws().to_vec()), false),
                wmo_render::Lookup::Failed => (None, true),
                _ => (None, false),
            },
        };
        // Record a model that fails to load. Without this the tool is armed,
        // the cursor shows nothing, and a click writes a record for a file the
        // archives do not hold. That is how the `.mdx` bug in the module
        // comment behaved, and this makes a repeat of it visible.
        placing.trouble = failed.then(|| path.clone());
        let Some(draws) = draws else { return };

        let ghost = commands
            .spawn((Ghost, Transform::default(), Visibility::default()))
            .id();
        for draw in &draws {
            // The client's own translucency, which also handles opaque
            // batches. See the module comment.
            let material = materials
                .with_opacity(&draw.material, GHOST)
                .unwrap_or_else(|| draw.material.clone());
            commands.spawn((
                Ghost,
                Mesh3d(draw.mesh.clone()),
                MeshMaterial3d(material),
                ChildOf(ghost),
            ));
        }
        placing.ghost = Some(ghost);
        placing.showing = Some(path);
    }

    let Some(ghost) = placing.ghost else { return };
    let Ok(mut transform) = ghosts.get_mut(ghost) else {
        return;
    };
    *transform = transform_of(
        placing.kind,
        at,
        placing.turn,
        placing.tilt_now(),
        placing.scale,
    );
}

impl Placing {
    /// The lean the next drop is written with: the ground's when
    /// [`Self::align`] is on, the roll's otherwise.
    pub fn tilt_now(&self) -> [f32; 2] {
        match self.align && self.kind == Kind::Doodad {
            true => self.lean,
            false => self.tilt,
        }
    }
}

/// Read the slope under the cursor into [`Placing::lean`], at the current
/// turn, on every frame the tool is armed. It is a separate system because
/// [`follow`] has the caches and not the session, and the lean comes from the
/// session's ground.
///
/// It reads `Cursor::ground` where [`follow`] reads `Cursor::surface`, on
/// purpose. Where a model stands is whatever is under the pointer. The lean is
/// the terrain's slope, which is the only slope with a known normal: a hull
/// gives a height, not a normal. So a doodad dropped on a roof with
/// [`Placing::align`] on stands on the roof and leans with the hillside under
/// it. That result is wrong and visible; the setting is meant for scattering
/// rocks on a hillside, where it is right.
fn ground_lean(
    mut placing: ResMut<Placing>,
    session: Option<Res<EditSession>>,
    cursor: Res<Cursor>,
) {
    if !placing.align || placing.path.is_none() {
        return;
    }
    let lean = match (session.as_deref(), cursor.ground) {
        (Some(session), Some(at)) => super::doodads::ground_normal(session, at.x, at.y)
            .map(|normal| {
                let world = vale_assets::world::adt::lean_to_normal(normal, placing.turn);
                [world[0], world[1]]
            })
            .unwrap_or([0.0; 2]),
        _ => [0.0; 2],
    };
    if placing.lean != lean {
        placing.lean = lean;
    }
}

/// Where the preview stands, and where the record will say it stands.
///
/// Both use this one function, so the preview matches the placement. It goes
/// through `placement_matrix` like everything else instead of composing a
/// rotation here, because `placement_matrix` is the one implementation of the
/// `MDDF` frame, including the 180° term.
fn transform_of(kind: Kind, at: Vec3, turn: f32, tilt: [f32; 2], scale: f32) -> Transform {
    let rotation =
        vale_assets::world::adt::placement_euler_from_world(world_angles(kind, turn, tilt));
    let scale = match kind {
        Kind::Doodad => scale,
        Kind::Wmo => 1.0,
    };
    let matrix = vale_assets::world::adt::placement_matrix(at.to_array(), rotation, scale);
    Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)))
}

/// The three world-axis angles a drop is written with: the turn about up,
/// and the lean for a doodad. A building is never leaned — see the module
/// comment.
fn world_angles(kind: Kind, turn: f32, tilt: [f32; 2]) -> [f32; 3] {
    match kind {
        Kind::Doodad => [tilt[0], tilt[1], turn],
        Kind::Wmo => [0.0, 0.0, turn],
    }
}

/// Write the record.
#[allow(clippy::too_many_arguments)]
fn commit(
    mut placing: ResMut<Placing>,
    mut session: Option<ResMut<EditSession>>,
    mut doodads: ResMut<super::doodads::Selection>,
    mut wmos: ResMut<super::wmos::Selection>,
    cursor: Res<Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    parts: Query<(&GlobalTransform, &Aabb), With<Ghost>>,
    mut tiles: Query<(&TerrainTile, &mut PendingDoodads, &mut PendingWmos)>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !placing.armed() || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let (Some(path), Some(at)) = (placing.path.clone(), cursor.surface) else {
        return;
    };
    let coord = vale_assets::tile_for_position(at.x, at.y);
    if !session.tiles.contains_key(&coord) {
        session.status = format!("tile {},{} is not open", coord.0, coord.1);
        return;
    }

    // Take an id from the minted range, not the largest existing id plus one.
    // See `vale_edit::adt::place::MINTED_BASE`. A tool only sees the tiles it
    // has open, and one id naming two objects now loses data instead of only
    // being untidy.
    let unique_id = session
        .tiles
        .values()
        .filter_map(|tile| tile.highest_minted_id())
        .max()
        .map(|had| had.saturating_add(1))
        .unwrap_or(MINTED_BASE);

    let position = vale_assets::world::adt::placement_from_world(at.to_array());
    let rotation = vale_assets::world::adt::placement_euler_from_world(world_angles(
        placing.kind,
        placing.turn,
        placing.tilt_now(),
    ));
    let kind = placing.kind;
    let radius = ghost_radius(&parts);

    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = Placements::capture(tile);
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();

    let index = match kind {
        Kind::Doodad => {
            let record = Placement {
                name_id: tile.name_model(&path),
                unique_id,
                position,
                rotation,
                scale: (placing.scale * 1024.0)
                    .round()
                    .clamp(1.0, f32::from(u16::MAX)) as u16,
                flags: 0,
            };
            tile.add_doodad(record, radius)
        }
        Kind::Wmo => {
            // The box comes from the preview, the only thing that knows the
            // building's size. A `MODF` row carries its own box, so a new row
            // must be given one. It is the same union `wmos::refit` takes, for
            // the same reason: the mover reads this box before the `.wmo` has
            // loaded.
            let (lower, upper) = ghost_bounds(&parts).unwrap_or((position, position));
            let record = Building {
                name_id: tile.name_building(&path),
                unique_id,
                position,
                rotation,
                bounds_lower: lower,
                bounds_upper: upper,
                flags: 0,
                doodad_set: 0,
                name_set: 0,
                padding: 0,
            };
            tile.add_building(record)
        }
    };

    let edit = Edit::Placements {
        before: Box::new(before),
        after: Box::new(Placements::capture(tile)),
    };
    session.history.begin(format!("Place {leaf}"));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.status = format!("placed {leaf} in tile {},{}", coord.0, coord.1);

    // Hand the new placement to the pass that spawns placements instead of
    // re-reading the tile.
    //
    // Adding a row changes the tile's lists, and every other edit that does
    // that requests the tile again. Since `terrain::swap` the re-read is
    // invisible, but it still takes a third of a second. Placing is repeated
    // quickly, so with a re-read three fast clicks would show nothing for a
    // moment and then three trees at once.
    //
    // Spawning it directly loses nothing. `PendingDoodads` and `PendingWmos`
    // are the same lists the tile loader fills, drained by the same passes, so
    // the result is what a re-read would produce, including the collision
    // hull, the lamps and a `Doodad`/`WmoPart` marker. The marker makes the new
    // placement selectable immediately.
    //
    // A tile that is not on screen has no lists to push to. Its file is
    // already correct, and whatever streams it next draws the placement.
    let placed = PlacedModel {
        path: path.clone(),
        unique_id,
        position: at.to_array(),
        matrix: vale_assets::world::adt::placement_matrix(
            at.to_array(),
            rotation,
            match kind {
                Kind::Doodad => placing.scale,
                Kind::Wmo => 1.0,
            },
        ),
        scale: placing.scale,
        doodad_set: 0,
        name_set: 0,
    };
    for (tile, mut doodads, mut buildings) in &mut tiles {
        if tile.coord != coord {
            continue;
        }
        match kind {
            // `on_ground` with `shadowed` false: the bit is the tile's baked
            // `MCSH` under the origin and only the loader has the decoded chunks
            // to read it from. It picks the per-instance sun scale, so a doodad
            // placed under a tree is lit as though it were in the open until the
            // tile is next read. Only the shading is off; the model is correct.
            Kind::Doodad => doodads
                .outdoor
                .push(PlacedDoodad::on_ground(placed.clone(), false)),
            Kind::Wmo => buildings.0.push(placed.clone()),
        }
    }

    // Select the new placement, so the gizmo is already on it and the user
    // does not have to find it again to turn it. The index is what `add_*`
    // returned, and `resync` keeps it correct from the next frame.
    match kind {
        Kind::Doodad => {
            wmos.only(None);
            doodads.only(
                session
                    .tiles
                    .get(&coord)
                    .and_then(|tile| tile.doodad_at(index))
                    .map(|record| super::doodads::Selected {
                        tile: coord,
                        index,
                        unique_id,
                        path: path.clone(),
                        record,
                    }),
            );
        }
        Kind::Wmo => {
            doodads.only(None);
            wmos.only(
                session
                    .tiles
                    .get(&coord)
                    .and_then(|tile| tile.building_at(index))
                    .map(|record| super::wmos::Selected {
                        tile: coord,
                        index,
                        unique_id,
                        path: path.clone(),
                        record,
                    }),
            );
        }
    }

    // The tool stays armed. Placing many copies is the common case, such as a
    // copse of trees or a row of crates, and disarming after every click would
    // need one trip to the panel per tree. `Escape` disarms it, and so does a
    // tool change: see [`aim`].
    //
    // The next drop is rolled now, so the ghost shows what the next click
    // writes. With scatter off this changes nothing.
    placing.roll();
}

/// The preview's own footprint, for the `MCRF` references a new doodad needs.
///
/// An `MDDF` row says nothing about how big the model is, which is what
/// `add_doodad`'s `radius` is a stand-in for, and the preview is the only
/// thing that knows the model's size.
fn ghost_radius(parts: &Query<(&GlobalTransform, &Aabb), With<Ghost>>) -> f32 {
    match ghost_bounds(parts) {
        Some((lower, upper)) => {
            let span = |axis: usize| (upper[axis] - lower[axis]).abs() / 2.0;
            span(0).max(span(2)).max(1.0)
        }
        None => 20.0,
    }
}

/// The union of the preview's drawn batches, in the file's own frame.
///
/// The same walk `wmos::refit` makes, over the ghost instead of over a
/// placement, so a building placed here starts with the box a moved one would be
/// re-fitted to.
fn ghost_bounds(
    parts: &Query<(&GlobalTransform, &Aabb), With<Ghost>>,
) -> Option<([f32; 3], [f32; 3])> {
    let mut lower = Vec3::splat(f32::INFINITY);
    let mut upper = Vec3::splat(f32::NEG_INFINITY);
    let mut found = false;
    for (transform, aabb) in parts.iter() {
        let centre = Vec3::from(aabb.center);
        let half = Vec3::from(aabb.half_extents);
        for corner in 0..8 {
            let offset = Vec3::new(
                if corner & 1 == 0 { -half.x } else { half.x },
                if corner & 2 == 0 { -half.y } else { half.y },
                if corner & 4 == 0 { -half.z } else { half.z },
            );
            let point = transform.affine().transform_point3(centre + offset);
            lower = lower.min(point);
            upper = upper.max(point);
            found = true;
        }
    }
    if !found {
        return None;
    }
    let a = vale_assets::world::adt::placement_from_world(axes::to_wow(lower));
    let b = vale_assets::world::adt::placement_from_world(axes::to_wow(upper));
    let mut low = [0.0f32; 3];
    let mut high = [0.0f32; 3];
    for axis in 0..3 {
        low[axis] = a[axis].min(b[axis]);
        high[axis] = a[axis].max(b[axis]);
    }
    Some((low, high))
}

/// The last component of a path: the file name.
pub fn leaf(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A building's root is offered and its group files are not.
    /// `stormwind.wmo` is one root and some three hundred group files, and
    /// listing the groups would bury every building under its own rooms.
    #[test]
    fn a_wmo_group_is_not_a_building() {
        assert!(!is_group(
            "world\\wmo\\azeroth\\buildings\\stormwind\\stormwind.wmo"
        ));
        assert!(is_group(
            "world\\wmo\\azeroth\\buildings\\stormwind\\stormwind_042.wmo"
        ));
        assert!(is_group("world\\wmo\\a\\b_000.wmo"));
        // A name that only ends in digits is not a group. A group name ends in
        // an underscore and exactly three digits.
        assert!(!is_group("world\\wmo\\a\\tower42.wmo"));
        assert!(!is_group("world\\wmo\\a\\b_42.wmo"));
        assert!(!is_group("world\\wmo\\a\\b_0000.wmo"));
    }

    /// A path is resolved when the tool is armed, whichever way it came in.
    ///
    /// The picker's paths are the archives' own and are `.m2`; a duplicate's
    /// path is the tile's `MMDX` string and is `.mdx`, which no archive holds.
    /// Arming an `.mdx` path gave no preview, drew no placement, and then
    /// produced every copy at once when anything read the tile again. See the
    /// module comment. This test guards against that bug returning.
    #[test]
    fn an_armed_path_is_what_the_archives_hold() {
        let mut placing = Placing::default();
        placing.arm(
            Kind::Doodad,
            r"World\Azeroth\Elwynn\PassiveDoodads\ElwynnTree\ElwynnTreeCanopy04.mdx".into(),
            0.0,
            1.0,
        );
        assert_eq!(
            placing.path.as_deref(),
            Some(r"World\Azeroth\Elwynn\PassiveDoodads\ElwynnTree\ElwynnTreeCanopy04.m2")
        );

        // An `.m2` path is left unchanged, so the picker's own rows are not
        // affected.
        placing.arm(Kind::Doodad, r"world\critter\rat\rat.m2".into(), 0.0, 1.0);
        assert_eq!(placing.path.as_deref(), Some(r"world\critter\rat\rat.m2"));

        // A building path does not take the model swap, which would ask the
        // archives for `stormwind.wmo.m2`.
        let building = r"world\wmo\azeroth\buildings\stormwind\stormwind.wmo";
        placing.arm(Kind::Wmo, building.into(), 0.0, 1.0);
        assert_eq!(placing.path.as_deref(), Some(building));
    }

    /// The picker's folder is a prefix with a separator on the end, and the two
    /// kinds keep their own.
    ///
    /// The separator matters: without it one folder's prefix also matches a
    /// folder whose name it begins, and the listing mixes in the second
    /// folder's contents.
    #[test]
    fn the_picker_keeps_a_folder_per_kind() {
        let mut placing = Placing::default();
        assert_eq!(placing.folder(Kind::Doodad), ROOT);
        assert_eq!(placing.folder(Kind::Wmo), r"world\wmo\");

        placing.open_folder(Kind::Doodad, r"World\Azeroth");
        assert_eq!(placing.folder(Kind::Doodad), r"world\azeroth\");
        // The other kind's folder is unchanged.
        assert_eq!(placing.folder(Kind::Wmo), r"world\wmo\");

        assert_eq!(
            Placing::folder_of(r"World\Critter\Rat\Rat.m2"),
            r"world\critter\rat\"
        );
        assert_eq!(Placing::folder_of("rat.m2"), ROOT);
    }

    /// The preview stands where the record will say it stands.
    #[test]
    fn the_preview_and_the_record_agree() {
        let at = Vec3::new(-9450.0, -50.0, 60.0);
        let turn = 37.0;
        let drawn = transform_of(Kind::Doodad, at, turn, [0.0; 2], 1.0);

        // What `commit` writes, put back through the renderer's own placement
        // matrix, the route a real placement takes on the next frame.
        let position = vale_assets::world::adt::placement_from_world(at.to_array());
        let rotation = vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, turn]);
        let world = vale_assets::world::adt::placement_to_world(position);
        let matrix = vale_assets::world::adt::placement_matrix(world, rotation, 1.0);
        let placed = Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)));

        assert!((drawn.translation - placed.translation).length() < 1e-2);
        assert!(drawn.rotation.angle_between(placed.rotation) < 1e-3);
    }

    /// A building ignores the scale, because `MODF` has no scale field.
    #[test]
    fn a_building_is_never_scaled() {
        let at = Vec3::new(0.0, 0.0, 0.0);
        let one = transform_of(Kind::Wmo, at, 0.0, [0.0; 2], 1.0);
        let big = transform_of(Kind::Wmo, at, 0.0, [0.0; 2], 8.0);
        assert!((one.scale - big.scale).length() < 1e-4);
        // A building is never leaned, whatever the roll says.
        assert_eq!(world_angles(Kind::Wmo, 30.0, [10.0, 5.0]), [0.0, 0.0, 30.0]);
        assert_eq!(
            world_angles(Kind::Doodad, 30.0, [10.0, 5.0]),
            [10.0, 5.0, 30.0]
        );
    }

    /// A roll stays inside the stated bounds, changes something, and does
    /// nothing with scatter off.
    #[test]
    fn a_roll_stays_inside_its_bounds() {
        let mut placing = Placing::default();
        placing.arm(Kind::Doodad, "world\\a.m2".into(), 12.0, 1.0);
        placing.roll();
        assert_eq!(
            (placing.turn, placing.scale, placing.tilt),
            (12.0, 1.0, [0.0; 2]),
            "off is off"
        );

        placing.scatter = Scatter {
            on: true,
            turn: true,
            size: (0.5, 2.0),
            tilt_max: 10.0,
        };
        let mut turns = std::collections::BTreeSet::new();
        for _ in 0..64 {
            placing.roll();
            assert!((0.0..360.0).contains(&placing.turn), "{}", placing.turn);
            assert!((0.5..=2.0).contains(&placing.scale), "{}", placing.scale);
            let lean = (placing.tilt[0].powi(2) + placing.tilt[1].powi(2)).sqrt();
            assert!(lean <= 10.0 + 1e-3, "{lean}");
            turns.insert(placing.turn.to_bits());
        }
        assert!(turns.len() > 32, "the turns vary");

        // A building takes the turn and nothing else.
        placing.arm(Kind::Wmo, "world\\wmo\\b.wmo".into(), 0.0, 1.0);
        placing.scatter.on = true;
        placing.roll();
        assert_eq!(placing.scale, 1.0);
        assert_eq!(placing.tilt, [0.0; 2]);
    }

    /// The generator is in `0..1` and is not stuck.
    #[test]
    fn the_roll_is_in_the_unit_interval() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..1000 {
            let x = unit(&mut seed);
            assert!((0.0..1.0).contains(&x));
            seen.insert(x.to_bits());
        }
        assert!(seen.len() > 990);
    }
}
