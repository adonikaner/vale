//! Putting a new thing in the world: a model on the cursor, and a click.
//!
//! ## One tool for two lists, because it is one gesture
//!
//! A doodad and a building are different records in different lists with
//! different rules — see [`super::doodads`] and [`super::wmos`], which is where
//! those differences live. **Placing one is the same act either way**: choose a
//! model, see it where it would go, click. So the arming, the preview, the turn
//! and the scale are here once, and only the twenty lines that write the record
//! know which list they are writing to.
//!
//! ## The preview is the model, drawn translucent
//!
//! A marker or a box would be cheaper and would answer the wrong question. What
//! a person placing scenery needs to know is whether *this tree* sits right on
//! *this slope* — its size against the ground, its trunk against the rock behind
//! it — and none of that survives being reduced to an outline.
//!
//! It costs nothing to do properly because both halves already exist:
//! `ModelCache::lookup` and `WmoCache::lookup` hand back the same batches the
//! world is drawn from, and `Materials::with_opacity` is the client's own
//! translucency — the one that dresses a stealthed rogue and a spirit at the
//! graveyard. It even carries the awkward half: an opaque batch cannot blend at
//! all, because the blend mode is in the pipeline key, so it is forced to mode 2
//! and its real mode stashed. A ghost written here would have got that wrong.
//!
//! `render::ships` is the precedent for the spawn — a building's batches under an
//! entity whose transform is written every frame, with no `MODF` row behind it.
//!
//! ## The ghost is not a placement and must not look like one
//!
//! Its batches carry **no** `Doodad` and no `WmoPart`, which is what keeps
//! every other tool blind to it: the doodad pick rays against `Doodad` boxes, the
//! WMO pick against `WmoPart`, the reconcilers walk both. A ghost wearing either
//! marker would be selectable, movable and — once `rehome` started enforcing one
//! row per id — a candidate for being collapsed into a real placement.
//!
//! ## Duplicating is arming, not copying
//!
//! "Duplicate" does not drop a second copy on top of the first, which is a thing
//! you then have to find and drag off. It arms *this* tool with the selected
//! placement's model, rotation and scale, so the copy is on the cursor and lands
//! where it is wanted. One gesture, and it is the gesture already learnt.
//!
//! ## A model path is resolved where it enters, and that is [`Placing::arm`]
//!
//! The two ways a path gets here do not agree about what a model is called.
//! The picker's come out of the archives' own listing and are `.m2` — measured:
//! `vale catalogue` reports 5,816 `.m2` under `World\` and not one `.mdx`.
//! **A duplicate's comes out of the tile's `MMDX`, and `MMDX` is `.mdx`**, which
//! is the pre-1.0 extension no archive holds. Every reader in this project swaps
//! it — `vale_assets::world::m2::model_path` is the one statement of that —
//! and this tool was the one that did not.
//!
//! What that looked like is worth keeping, because nothing anywhere reported it.
//! `ModelCache` failed the path, so the preview never appeared; `resolve_doodads`
//! failed it too and dropped the placement, so the click drew nothing; and the
//! record written into the file was perfectly good, so the moment anything made
//! the tile be read again — a drag, an undo, a playtest — every copy appeared at
//! once. It was reported as *"when duplicating, the preview does not show, nor do
//! the duplicated placements, until you move something in the world"*, which is
//! that sequence exactly.
//!
//! So [`Placing::arm`] resolves, and it is the only door in. Per kind, because
//! the swap is the *model* rule: a `.wmo` is already what the archive holds, and
//! putting one through `model_path` would ask for `stormwind.wmo.m2`.
//!
//! ## Scattering: every drop is different, inside stated bounds
//!
//! A forest placed one identical tree at a time looks stamped, and turning and
//! sizing each one by hand is the slow half of placing. With [`Scatter`] on,
//! every drop **rolls the next one**: a fresh turn about up, a size drawn
//! between two bounds, and a tilt off the vertical up to a stated angle — the
//! three numbers `MDDF` carries and the ghost shows. The roll happens after
//! the click, so what is on the cursor is always what the next click writes,
//! and **Roll now** re-draws it without placing anything. A building takes the
//! turn only: `MODF` has no scale, and a tilted building is a mistake nine
//! times in ten.
//!
//! The randomness is a 64-bit xorshift of this file's own, seeded from the
//! clock on first use. A dependency for three numbers a second is not worth
//! the tree it would pull in, and nothing here needs to be reproducible.

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
/// Low enough to read as "not there yet" against any ground, high enough to
/// judge the model's silhouette and its colour. A ghost at 0.2 is a smudge and
/// one at 0.8 is indistinguishable from a placement that has already landed,
/// which is the failure that matters — a person needs to know whether they have
/// clicked.
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
    /// What the archives call one of these.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Doodad => &[".m2", ".mdx"],
            Kind::Wmo => &[".wmo"],
        }
    }
}

/// Which half of a placement tool's panel is showing.
///
/// **A mode and not a second tool.** Choosing a model and moving one already
/// placed are the same subject — the same list, the same records, the same
/// panel — and a person switches between them constantly while scattering
/// scenery. A separate rail entry would put a click between every tree and the
/// nudge that straightens it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Pick what is there, move it, turn it.
    #[default]
    Select,
    /// Put something new in.
    Place,
}

/// The three, as a list a panel can offer.
pub const MODES: [(&str, Mode); 2] = [("Select", Mode::Select), ("Place", Mode::Place)];

/// How the next drop is rolled — see the module comment.
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
    /// The model on the cursor, or `None` — which is the tool switched off and
    /// is the state it spends nearly all of its time in.
    pub path: Option<String>,
    pub kind: Kind,
    /// Degrees about the world's up, which is the one axis worth having before a
    /// placement lands: the other two are the gizmo's job afterwards.
    pub turn: f32,
    /// …and the other two, in degrees about the world's x and y, which only
    /// [`Scatter`] writes: a lean off the vertical.
    pub tilt: [f32; 2],
    /// **Whether a drop leans onto the slope under the cursor** instead of
    /// taking the roll's lean. Doodads only; a building is never leaned. See
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
    /// Every model path the archives name, per kind, read once and lazily.
    pub doodads: Vec<String>,
    pub wmos: Vec<String>,
    /// What the panel's search box holds, kept per kind so switching between
    /// them does not lose it.
    pub search: String,
    /// **Which folder the picker is showing**, one per kind, as a lower-case
    /// path prefix ending in a separator.
    ///
    /// One per kind because the two catalogues do not share a shape: a doodad
    /// is anywhere under `World\` and a building is always under `World\wmo\`,
    /// so a single field would drop somebody at the root every time they
    /// switched tool. Kept across a switch for the same reason [`Self::search`]
    /// is — the list is being used to find one thing, and going back to the top
    /// is the one thing nobody wants.
    folders: [String; 2],
    /// **A model that will not load, said out loud.** A preview that is simply
    /// absent is indistinguishable from one that has not arrived yet, and the
    /// two want different things done about them.
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
            // **One, not zero.** A derived default would arm the tool at a scale
            // of nothing, which places a doodad the file says is there and the
            // renderer draws at no size — visible as "the click did nothing".
            scale: 1.0,
            scatter: Scatter::default(),
            seed: 0,
            doodads: Vec::new(),
            wmos: Vec::new(),
            search: String::new(),
            // **The buildings start one level in.** Every `.wmo` in the game
            // is under `World\wmo\`, so the root would be one folder with
            // everything in it and a click nobody can avoid.
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
    /// **Asked by both picks**, which decline while it is true — see the module
    /// comment: a click that places must not also select.
    pub fn armed(&self) -> bool {
        self.path.is_some()
    }

    /// Arm with a model, at a rotation and scale, and show the half of the panel
    /// that is about placing.
    ///
    /// The one entry point, used by the picker and by *Duplicate* — see the
    /// module comment on why duplicating arms rather than copies.
    pub fn arm(&mut self, kind: Kind, path: String, turn: f32, scale: f32) {
        self.kind = kind;
        // **Resolved here and nowhere else** — see the module comment, where
        // what an unresolved `.mdx` looked like is written down.
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
    /// is on. Called after every drop and by **Roll now**.
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

    /// …and put it away, which is what `Escape` and a tool change do.
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

    /// …and go somewhere else in it. A prefix with no trailing separator is
    /// given one, so that a folder cannot also match one whose name it is a
    /// prefix of.
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
/// cut on. A backslash, and named so that it reads as a separator rather than as
/// an escape in the middle of a condition.
pub const SEP: char = '\\';

/// Where the picker starts, and what its breadcrumb calls `World`.
///
/// Everything either catalogue holds is under it — both come from
/// `list_prefix("World\")` — so a level above it would be one folder with
/// everything in it.
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
                // **Before both picks**, which is what keeps the click that
                // places from also selecting whatever is behind it. Stated
                // because nothing would report it.
                .before(crate::tools::doodads::select)
                .before(crate::tools::wmos::select),
        );
    }
}

/// `--place` — open on the picker, and arm what it named.
///
/// **Once, on the first frame the tool is one that places.** It is the only way
/// a scripted run reaches this half of the panel at all: the mode is a click on
/// a segmented control and the model is a click on a row, and neither is
/// something `--shot` can do. See [`crate::Args::place`], where the argument is.
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
            // …and the picker opens on the model's own folder, which is where
            // the next one is most likely to be — and is what a shot of the
            // picker's rows needs, since the root is seventeen folders and
            // no models.
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

/// The preview, on the root **and on every batch of it**.
///
/// On the batches because that is what the bounds walk needs to find, and the
/// root has no mesh of its own; on the root because that is what carries the
/// transform. Nothing else in this crate names it, which is the point — see the
/// module comment on why a ghost must not wear `Doodad` or `WmoPart`.
#[derive(Component)]
struct Ghost;

/// Read the model lists, once each, the first time one is wanted.
///
/// Lazily for the reason the tileset list is: it walks the listing of nineteen
/// archives, and a session that never places anything should not pay for it.
fn read_catalogue(mut placing: ResMut<Placing>, tool: Res<Tool>, assets: Res<GameAssets>) {
    let kind = match *tool {
        Tool::Doodads => Kind::Doodad,
        Tool::Wmos => Kind::Wmo,
        _ => return,
    };
    if !placing.catalogue(kind).is_empty() {
        return;
    }
    let found = assets.with_archive(|chain| Ok(chain.list_prefix("World\\")));
    let mut found: Vec<String> = match found {
        Ok(found) => found
            .into_iter()
            .filter(|path| kind.extensions().iter().any(|end| path.ends_with(end)))
            // **A `.wmo` that ends in `_NNN` is a group, not a building.** A
            // city is one root and three hundred of those, and offering them
            // would bury the thing somebody is looking for under its own rooms.
            .filter(|path| kind != Kind::Wmo || !is_group(path))
            // …and through the same swap [`Placing::arm`] makes, so the two
            // doors into this tool agree whatever the listing happens to hold.
            // Today it holds no `.mdx` at all — `vale catalogue` — so this
            // changes nothing; a patch archive that added one would otherwise
            // put an unopenable row in the list.
            .map(|path| match kind {
                Kind::Doodad => vale_assets::world::m2::model_path(&path),
                Kind::Wmo => path,
            })
            .collect(),
        Err(e) => {
            warn!("the model list could not be read: {e}");
            Vec::new()
        }
    };
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
    // **Disarmed by anything that changes the subject.** A model left on the
    // cursor after switching to the terrain brush is a click away from being
    // placed by somebody who thinks they are painting.
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
    // The wheel scales, under control, which is the gesture both brushes use for
    // their own size. A building has no scale to change.
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
    // The surface and not the ground — see [`crate::pick`] — so a lamp dropped
    // on a bridge or a crate on a dock stands on it. What is still the
    // *ground's* is the lean, because that is the terrain's slope by definition:
    // see [`ground_lean`].
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
        // **Asked every frame until it is `Ready`.** Both caches load on a task
        // and answer `Loading` meanwhile, exactly as the world's own placements
        // are loaded — so the preview appears a moment after a model is chosen,
        // which is the same moment the world would have taken.
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
        // **A model that will not load says so.** Without this the tool is
        // armed, the cursor shows nothing, and a click writes a record for a
        // file the archives do not hold — which is the shape of the `.mdx` bug
        // in the module comment and is worth being unable to have silently
        // twice.
        placing.trouble = failed.then(|| path.clone());
        let Some(draws) = draws else { return };

        let ghost = commands
            .spawn((Ghost, Transform::default(), Visibility::default()))
            .id();
        for draw in &draws {
            // The client's own translucency, which also handles the half a ghost
            // written here would have got wrong — see the module comment.
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

/// **Read the slope under the cursor** into [`Placing::lean`], at the current
/// turn, on every frame the tool is armed. Its own system because [`follow`]
/// has the caches and not the session, and the lean is the session's ground.
///
/// **It reads `Cursor::ground` where [`follow`] reads `Cursor::surface`**, and
/// the two are deliberately different. Where a model *stands* is whatever is
/// under the pointer; the lean is the **terrain's** slope, which is the only
/// slope there is a number for — a hull answers a height, not a normal. So a
/// doodad dropped on a roof with [`Placing::align`] on stands on the roof and
/// leans with the hillside under it. That is wrong and it is visible; the
/// setting exists for scattering rocks on a hillside, which is where it is
/// right.
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
/// **One function, used by both**, so that what is previewed is what is placed.
/// It goes through `placement_matrix` like everything else rather than composing
/// a rotation here, because that is the one statement of the `MDDF` frame — the
/// 180° term included.
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

    // **An id from the minted range and not "the largest plus one"** — see
    // `vale_edit::adt::place::MINTED_BASE`. A tool can only see the tiles it
    // has open, and one id naming two objects is now destructive rather than
    // merely untidy.
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
            // **The box comes from the preview**, which is the only thing that
            // knows how big this building is — a `MODF` row carries its own box
            // and a brand new row has to be given one. It is the same union
            // `wmos::refit` takes, and for the same reason: the mover reads this
            // box before the `.wmo` has loaded.
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

    // **Handed to the pass that spawns placements, not left to a re-read.**
    //
    // Adding a row changes the tile's lists, and every other edit that does that
    // asks for the tile again — which since `terrain::swap` is invisible but is
    // still a third of a second. That is the wrong trade *here*, because placing
    // is a repeated gesture: scattering a copse would be click, click, click,
    // nothing, nothing, then three trees at once.
    //
    // Nothing is lost by spawning it directly. `PendingDoodads` and
    // `PendingWmos` are the same lists the tile loader fills, drained by the
    // same passes, so what appears is what a re-read would have produced —
    // including its collision hull, its lamps and a `Doodad`/`WmoPart` marker,
    // which is what makes the thing just placed immediately selectable.
    //
    // A tile that is not on screen has no lists to push to, and then the file is
    // already right and whatever streams it next draws it.
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
            // tile is next read — which is a shade, not a wrong picture.
            Kind::Doodad => doodads
                .outdoor
                .push(PlacedDoodad::on_ground(placed.clone(), false)),
            Kind::Wmo => buildings.0.push(placed.clone()),
        }
    }

    // **Selected, so the gizmo is already on it.** Placing something and then
    // having to find it again to turn it is two gestures where one will do; the
    // index is what `add_*` just answered and `resync` keeps it honest from the
    // next frame.
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

    // **It stays armed.** Scattering is the common case — a copse of trees, a
    // row of crates — and disarming after every click would be one round trip to
    // the panel per tree. `Escape` is how it stops, and switching subject does
    // it too: see [`aim`].
    //
    // …and the next drop is rolled now, so the ghost shows what the next click
    // writes. Nothing changes with scatter off.
    placing.roll();
}

/// The preview's own footprint, for the `MCRF` references a new doodad needs.
///
/// An `MDDF` row says nothing about how big the model is, which is what
/// `add_doodad`'s `radius` is a stand-in for — and the preview is the one thing
/// in the room that knows.
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

/// The last part of a path, which is the only part that names anything.
pub fn leaf(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A building's root is offered and its rooms are not.** `stormwind.wmo`
    /// is one root and some three hundred group files, and listing those would
    /// bury every building in the game under its own corridors.
    #[test]
    fn a_wmo_group_is_not_a_building() {
        assert!(!is_group(
            "world\\wmo\\azeroth\\buildings\\stormwind\\stormwind.wmo"
        ));
        assert!(is_group(
            "world\\wmo\\azeroth\\buildings\\stormwind\\stormwind_042.wmo"
        ));
        assert!(is_group("world\\wmo\\a\\b_000.wmo"));
        // A name that merely ends in digits is not a group — the underscore and
        // exactly three digits are what says so.
        assert!(!is_group("world\\wmo\\a\\tower42.wmo"));
        assert!(!is_group("world\\wmo\\a\\b_42.wmo"));
        assert!(!is_group("world\\wmo\\a\\b_0000.wmo"));
    }

    /// **A path is resolved on the way in, whichever door it came through.**
    ///
    /// The picker's paths are the archives' own and are `.m2`; a *duplicate*'s
    /// is the tile's `MMDX` string and is `.mdx`, which no archive holds. Arming
    /// the second one gave no preview, drew no placement, and then produced
    /// every copy at once the moment anything read the tile again — see the
    /// module comment. This is the one line that has to hold for that not to
    /// come back.
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

        // …and `.m2` is left exactly as it is, so the picker's own rows are
        // untouched.
        placing.arm(Kind::Doodad, r"world\critter\rat\rat.m2".into(), 0.0, 1.0);
        assert_eq!(placing.path.as_deref(), Some(r"world\critter\rat\rat.m2"));

        // **A building is not a model and must not take the swap**, which would
        // ask the archives for `stormwind.wmo.m2`.
        let building = r"world\wmo\azeroth\buildings\stormwind\stormwind.wmo";
        placing.arm(Kind::Wmo, building.into(), 0.0, 1.0);
        assert_eq!(placing.path.as_deref(), Some(building));
    }

    /// The picker's folder is a prefix with a separator on the end, and the two
    /// kinds keep their own.
    ///
    /// The separator matters: without it one folder's prefix also matches a
    /// folder whose name it begins, which is a listing with a second folder's
    /// contents mixed into it.
    #[test]
    fn the_picker_keeps_a_folder_per_kind() {
        let mut placing = Placing::default();
        assert_eq!(placing.folder(Kind::Doodad), ROOT);
        assert_eq!(placing.folder(Kind::Wmo), r"world\wmo\");

        placing.open_folder(Kind::Doodad, r"World\Azeroth");
        assert_eq!(placing.folder(Kind::Doodad), r"world\azeroth\");
        // …and the other kind is where it was.
        assert_eq!(placing.folder(Kind::Wmo), r"world\wmo\");

        assert_eq!(
            Placing::folder_of(r"World\Critter\Rat\Rat.m2"),
            r"world\critter\rat\"
        );
        assert_eq!(Placing::folder_of("rat.m2"), ROOT);
    }

    /// The preview stands where the record will say it stands, which is the one
    /// property that makes a preview worth having.
    #[test]
    fn the_preview_and_the_record_agree() {
        let at = Vec3::new(-9450.0, -50.0, 60.0);
        let turn = 37.0;
        let drawn = transform_of(Kind::Doodad, at, turn, [0.0; 2], 1.0);

        // What `commit` writes, put back through the renderer's own placement
        // matrix — the route a real placement takes on the frame after.
        let position = vale_assets::world::adt::placement_from_world(at.to_array());
        let rotation = vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, turn]);
        let world = vale_assets::world::adt::placement_to_world(position);
        let matrix = vale_assets::world::adt::placement_matrix(world, rotation, 1.0);
        let placed = Transform::from_matrix(axes::to_bevy_affine(Mat4::from_cols_array(&matrix)));

        assert!((drawn.translation - placed.translation).length() < 1e-2);
        assert!(drawn.rotation.angle_between(placed.rotation) < 1e-3);
    }

    /// …and a building ignores the scale, because `MODF` has nowhere to put one.
    #[test]
    fn a_building_is_never_scaled() {
        let at = Vec3::new(0.0, 0.0, 0.0);
        let one = transform_of(Kind::Wmo, at, 0.0, [0.0; 2], 1.0);
        let big = transform_of(Kind::Wmo, at, 0.0, [0.0; 2], 8.0);
        assert!((one.scale - big.scale).length() < 1e-4);
        // …and never leaned, whatever the roll says.
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
