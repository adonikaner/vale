//! The texture brush: the tool that paints textures on the ground.
//!
//! ## A texture edit can change the size of the file
//!
//! The height brush moves 580 bytes inside a region that stays 580 bytes long.
//! Painting with a texture a chunk does not carry adds a `MCLY` record and a
//! blend map. That moves every region after them and every offset that names
//! one. `vale_edit` handles this: see `vale_edit::adt::alpha` and the round
//! trip over five real tiles that tests the writer.
//!
//! This module is the interactive half. It makes one decision the height brush
//! does not have to make: how an edit reaches the screen.
//!
//! ## The two ways an edit reaches the screen, and what each costs
//!
//! ```text
//! the alpha changed        write the chunk's cell into the tile's atlas
//! the texture set changed  read the tile again
//! ```
//!
//! A chunk's blend maps are texels in one 1024x1024 atlas per tile, so changing
//! them is a write into an image that is already on the GPU: 16 KB for a chunk,
//! at every level of the mip chain.
//! `vale_assets::world::adt::write_alpha_atlas_cell` does it in one call,
//! because a cell is self-contained at every level the chain reaches.
//!
//! Which textures a chunk names is not in the atlas. It is part of the
//! `TerrainMaterial` its draw group was built with (four image handles and a
//! layer count), so a chunk given a fourth texture must have its tile read
//! again. That takes a third of a second in a debug build and happens once per
//! texture per chunk: the first stroke that introduces a texture pays for it
//! and every stroke after it is live. [`Edit::changes_the_texture_set`]
//! distinguishes the two cases. It is asked per edit, not per stroke.
//!
//! ## What a stroke can be told
//!
//! The brush is `vale_edit::ops::PaintBrush`, and the panel sets all of it:
//!
//! ```text
//! Paint or Erase     put the texture down, or take it away so what is under
//!                    it shows. Shift held is the other one
//! opacity            how visible the texture is where a held stroke ends up
//! density, grain     under 100%, paint in patches of that size
//! existing only      never add the texture to a chunk that lacks it
//! reuse hidden       on a full chunk, take the layer that shows under 2%
//! ```
//!
//! The last two and the Full chunks guide are the panel's answers to the
//! four-texture limit, grouped under that heading. The stroke reports what it
//! could not do on the status line, a refusal before anything else.
//!
//! ## The catalogue is the archives' listing of `Tileset\`
//!
//! The MPQ listing has about 1,700 paths under `Tileset\`. The picker offers
//! all of them, filtered by a search box. It does not offer the tile's own
//! `MTEX`, which lists only the textures the tile already has.
//!
//! ## What a new layer grows
//!
//! `MCLY`'s `effectId` is the ground foliage a layer grows: the tufts and
//! shrubs `render::foliage` plants. A new layer is written with the brush's
//! `effect_id`. The panel sets that from [`usual_effect`] when a texture is
//! chosen: the id the open tiles most often pair with the texture. Zero plants
//! nothing. [`set_layer_effect`] changes the id on a layer already there.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_assets::world::adt::{self as rules, ALPHA_LEN, ALPHA_SIDE, CHUNKS_PER_SIDE};
use vale_client::assets::GameAssets;
use vale_client::render::terrain::{TerrainTile, TileAlpha};
use vale_edit::ops::{Falloff, PaintBrush, Working};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// The range of the texture brush's radius, in yards, and of its strength.
///
/// The radius has the same ceiling as the height brush's, for the same reason:
/// a stroke reaches only the tiles this session has open. The strength has no
/// meaningful ceiling. It is a fraction of the remaining distance per second,
/// and one step is clamped to all of that distance, so a large number means
/// "immediately". Both limits were guesses before (100 yards and 10), and they
/// refused work the tool can do.
pub const RADIUS: std::ops::RangeInclusive<f32> = 0.5..=(crate::OPEN_BLOCK / 2.0);
pub const RATE: std::ops::RangeInclusive<f32> = 0.0..=1_000.0;

/// The texture tool's settings.
#[derive(Resource, Debug, Clone)]
pub struct Textures {
    pub brush: PaintBrush,
    /// Every `Tileset\` path the archives name, sorted. Read once, lazily. See
    /// [`read_catalogue`].
    pub catalogue: Vec<String>,
    /// What the panel's search box holds. Held here rather than in the panel so
    /// that it survives the tool being switched away from and back.
    pub search: String,
    /// Which folder the picker is showing, as a lower-case path prefix ending
    /// in a separator.
    ///
    /// The model picker has the same arrangement for the same reason. See
    /// [`crate::tools::place`]. With 621 tilesets in one scroller, every
    /// texture has to be found by typing. The archives' directories are the
    /// game's own grouping: `Tileset\Elwynn\` is the set a zone was painted
    /// with.
    pub folder: String,
    /// The chunk the panel is about, when it is not the one under the pointer.
    ///
    /// The layer list has to be reachable. A person who wants to take a layer
    /// off has to move the pointer to the button. The pointer is then over a
    /// different chunk, and the panel shows that chunk instead. A user reported
    /// this: "deleting one requires moving the pointer, at which point whatever
    /// you were trying to delete has disappeared."
    ///
    /// Latching when the pointer leaves the viewport does not fix it, because
    /// reaching the panel moves the pointer across every chunk between its
    /// position and the right-hand edge.
    ///
    /// A chunk is pinned in two ways, and neither needs the pointer to move.
    /// [`PIN`] pins the chunk under the pointer. A stroke that is refused pins
    /// the chunk that refused it, which is the case the list exists for. The
    /// panel then already shows why the stroke stopped.
    pub pinned: Option<((u32, u32), usize)>,
}

impl Default for Textures {
    fn default() -> Textures {
        Textures {
            brush: PaintBrush::default(),
            catalogue: Vec::new(),
            search: String::new(),
            // [`ROOT`], not the empty string a derived default would give. An
            // empty prefix matches every path, so the picker would open on one
            // folder called `tileset` with everything in it, and every use
            // would begin with a click into that folder.
            folder: ROOT.to_string(),
            pinned: None,
        }
    }
}

/// The key that pins the chunk under the pointer, and unpins it.
///
/// It is a key and not a button because a button would have to be reached, and
/// reaching it moves the pointer off the chunk. Space is free: the camera flies
/// on `WASD`, `Q` and `E`.
const PIN: KeyCode = KeyCode::Space;

/// Where the tileset picker starts. Everything the catalogue holds is under
/// it, because the catalogue comes from `list_prefix("Tileset\\")`. A level
/// above would be one folder with everything in it.
pub const ROOT: &str = "tileset\\";

pub struct TextureToolPlugin;

impl Plugin for TextureToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Textures>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                (read_catalogue, stroke, live_paint)
                    .chain()
                    // After the pick, for the same reason as the height brush:
                    // the stroke is aimed by where the pointer met the ground,
                    // and a pick from the frame before makes the stroke trail
                    // the cursor.
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw_brush, draw_pin));
    }
}

/// Whether a stroke is being held, and whether the press that began it was
/// over the world.
///
/// The doodad tool keeps the same two bools for the same reason. A drag belongs
/// to where it began, so the gate is asked on the press and not on every frame.
/// Without that, a stroke that passed over the inspector stopped there and
/// started again when the pointer came back.
#[derive(Resource, Default)]
struct Held {
    painting: bool,
    armed: bool,
    /// The stroke's own copy of the texels it is working on, per tile.
    ///
    /// A stroke cannot accumulate in the numbers the file holds. A 4-bit chunk
    /// has sixteen levels, so any step smaller than half a quantum rounds back
    /// to where it started. See `vale_edit::ops::Working`. The copy is cleared
    /// when the button comes up, because a copy kept past the stroke that made
    /// it disagrees with the file as soon as anything else writes.
    working: bevy::platform::collections::HashMap<(u32, u32), Working>,
}

/// The list of tilesets, read once.
///
/// It is read lazily rather than at startup. The read walks the whole listing
/// of nineteen archives, and a session that never opens this tool should not
/// pay for it.
fn read_catalogue(mut textures: ResMut<Textures>, tool: Res<Tool>, assets: Res<GameAssets>) {
    if !matches!(*tool, Tool::Textures | Tool::Chunks) || !textures.catalogue.is_empty() {
        return;
    }
    let found = assets.with_archive(|chain| Ok(chain.list_prefix("Tileset\\")));
    let mut found: Vec<String> = match found {
        Ok(found) => found
            .into_iter()
            .filter(|path| path.ends_with(".blp"))
            // The `_s` files are the specular masks that sit beside every
            // tileset. See `vale_assets::world::adt`, which resolves them
            // from the base name. Offering them would list each texture
            // twice, once as itself and once as its gloss mask.
            .filter(|path| !path.trim_end_matches(".blp").ends_with("_s"))
            .collect(),
        Err(e) => {
            warn!("the tileset list could not be read: {e}");
            Vec::new()
        }
    };
    found.sort();
    found.dedup();
    // A marker so a failed read is not retried every frame for the rest of the
    // session: an empty catalogue with an empty archive behind it stays empty.
    if found.is_empty() {
        found.push(String::new());
    }
    textures.catalogue = found;
}

/// Run the brush while the left button is held.
#[allow(clippy::too_many_arguments)]
fn stroke(
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    mut textures: ResMut<Textures>,
    mut held: ResMut<Held>,
    cursor: Res<Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
) {
    if !state.editing() || *tool != Tool::Textures {
        return;
    }
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // The pin is set from the keyboard, so that choosing a chunk does not move
    // the pointer off it. See [`Textures::pinned`]. Pressing the key again on
    // the same chunk unpins it.
    if keys.just_pressed(PIN) && !wants.wants_keyboard_input() && in_world {
        let under = cursor.tile.zip(cursor.chunk);
        textures.pinned = match (textures.pinned, under) {
            (Some(had), Some(now)) if had == now => None,
            (_, now) => now,
        };
    }
    // The wheel sets the same three numbers it sets on the height brush. See
    // [`crate::tools::Wheel`], which is the one list of what each modifier puts
    // on the wheel and is what `camera::fly` reads to decline the same notch.
    // Both brushes answer the same modifiers, so the gesture is learned once.
    if in_world && scroll.delta.y != 0.0 {
        let step = 1.0 + scroll.delta.y * 0.1;
        match crate::tools::Wheel::held(&keys) {
            Some(crate::tools::Wheel::Radius) => {
                textures.brush.radius =
                    (textures.brush.radius * step).clamp(*RADIUS.start(), *RADIUS.end());
            }
            Some(crate::tools::Wheel::Strength) => {
                textures.brush.strength =
                    (textures.brush.strength.max(0.01) * step).clamp(*RATE.start(), *RATE.end());
            }
            Some(crate::tools::Wheel::Core) => {
                textures.brush.core = (textures.brush.core + scroll.delta.y * 0.05)
                    .clamp(*crate::tools::CORE.start(), *crate::tools::CORE.end());
            }
            None => {}
        }
    }

    let Some(session) = session.as_mut() else {
        return;
    };
    if buttons.just_released(MouseButton::Left) {
        session.history.end();
        held.painting = false;
        held.armed = false;
        held.working.clear();
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        held.armed = in_world;
    }
    if !buttons.pressed(MouseButton::Left) || !held.armed {
        return;
    }
    if textures.brush.texture.is_empty() {
        session.status = "select a texture first".into();
        return;
    }
    let Some(at) = cursor.ground else { return };

    // Shift held is the other of the panel's two modes for as long as it is
    // held, as it is on the hole and water brushes: a stroke that went too far
    // is taken back without a trip to the panel.
    let mut brush = textures.brush.clone();
    if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        brush.erase = !brush.erase;
    }
    if !held.painting {
        held.painting = true;
        session.history.begin(match brush.erase {
            true => format!("Erase {}", leaf(&brush.texture)),
            false => format!("Paint {}", leaf(&brush.texture)),
        });
    }

    let seconds = time.delta_secs();
    let mut full: Vec<((u32, u32), usize)> = Vec::new();
    // What else the stroke could not do, or did in place of what was asked,
    // as counts for the status line.
    let (mut base, mut absent, mut reused) = (0usize, 0usize, 0usize);
    // Every tile the circle reaches, for the same reason as the height brush:
    // a stroke that stopped at a tile border would leave the paint ending in a
    // straight line there, with neither file being wrong.
    for coord in tiles_under(brush.radius, at) {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let painted = brush.stroke(
            tile,
            held.working.entry(coord).or_default(),
            [at.x, at.y],
            seconds,
        );
        full.extend(painted.full.iter().map(|&chunk| (coord, chunk)));
        base += painted.base.len();
        absent += painted.absent.len();
        reused += painted.reused.len();
        let edits = painted.edits;
        if edits.is_empty() {
            continue;
        }
        // The route to the screen is decided per edit. A stroke that gives one
        // chunk a new texture and moves the alpha on three others rebuilds the
        // tile once and patches the other three, rather than rebuilding on
        // every frame it is held.
        let mut rebuild = false;
        let mut touched: Vec<usize> = Vec::new();
        for edit in &edits {
            match edit.changes_the_texture_set() {
                true => rebuild = true,
                false => touched.extend(edit.painted()),
            }
        }
        session.history.record(&key, edits);
        match rebuild {
            true => {
                session.publish(coord);
                session.stale.insert(coord);
                session.unsaved.insert(coord);
            }
            false => {
                for chunk in touched {
                    session.repainted(coord, chunk);
                }
            }
        }
    }

    // The status line reports what the stroke could not do. A chunk already
    // carrying four textures cannot take a fifth, and roughly half the ground
    // in the shipped tiles is already at four. A brush that refused without a
    // message would stop part-way across a hillside with nothing saying why.
    // The inspector's "Chunk under the pointer" section holds the controls
    // that free a layer.
    //
    // One line, and a refusal comes before a report of what was done.
    let chunks = |n: usize| match n {
        1 => "1 chunk".to_string(),
        n => format!("{n} chunks"),
    };
    session.status = match (full.len(), base, absent, reused) {
        (1, ..) => "1 chunk is full: four textures is the limit".to_string(),
        (n @ 2.., ..) => format!("{n} chunks are full: four textures is the limit"),
        (0, n @ 1.., ..) => format!(
            "cannot erase the base texture of {}: replace the base in the Chunk list",
            chunks(n)
        ),
        (0, 0, n @ 1.., _) => format!(
            "no {1} layer on {0} under the brush: \"Paint existing layers only\" is on",
            chunks(n),
            leaf(&brush.texture)
        ),
        (0, 0, 0, n @ 1..) => format!("a hidden layer was reused on {}", chunks(n)),
        _ => match brush.erase {
            true => format!("erasing {}", leaf(&brush.texture)),
            false => format!("painting {}", leaf(&brush.texture)),
        },
    };
    // The first full chunk is pinned. This is the case the layer list exists
    // for: the stroke stopped, and the panel shows which chunk stopped it and
    // what its four textures are.
    //
    // It replaces a pin that was already there, deliberate or not. The rule is
    // that the panel shows the chunk that last refused the stroke. A pin held
    // from a minute ago, while the brush is refusing somewhere else, would
    // show a chunk that is not the one refusing.
    if let Some(&(coord, chunk)) = full.first() {
        textures.pinned = Some((coord, chunk));
    }
}

/// Write the repainted chunks' blend maps into the atlases that are already
/// drawn.
///
/// This is the paint counterpart of `super::terrain::live_ground`, and it is
/// cheaper per chunk. A cell is 64x64 texels wherever it is in the tile, so
/// this writes 16 KB and a mip chain rather than comparing against 81,840
/// vertices.
///
/// The cost is the re-upload, which is the whole atlas: writing to an `Image`
/// marks the asset changed and Bevy re-prepares all 5.3 MiB of it. That happens
/// per frame of a stroke and per tile the stroke is over. It is the one cost in
/// this tool that has not been measured.
///
/// A tile whose atlas is not in `Assets<Image>` cannot be patched. The client
/// builds them that way, and so does a tile that arrived before `LiveEdits`
/// was set. Such a tile is marked stale instead, which is the same fallback
/// `live_ground` has.
fn live_paint(
    mut session: Option<ResMut<EditSession>>,
    tiles: Query<(&TerrainTile, &TileAlpha)>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if session.repaint.is_empty() {
        return;
    }
    let repaint: Vec<((u32, u32), Vec<usize>)> = session
        .repaint
        .drain()
        .map(|(coord, chunks)| (coord, chunks.into_iter().collect::<Vec<usize>>()))
        .collect();

    for (coord, chunks) in repaint {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        // Every entity with this coordinate, not the first. While a tile is
        // being replaced there are two: the outgoing one, which is on screen,
        // and the incoming one, which is hidden. `find` picks whichever the
        // query yields first. Patching only the hidden one makes a stroke stop
        // appearing half way through and start again when the swap lands. See
        // `super::terrain::swap`.
        let drawn: Vec<Handle<Image>> = tiles
            .iter()
            .filter(|(drawn, _)| drawn.coord == coord)
            .map(|(_, atlas)| atlas.0.clone())
            .collect();
        if drawn.is_empty() {
            // Off screen or still loading. Its bytes are already right, so
            // whatever streams it next draws the paint.
            continue;
        }
        let mut reached = false;
        for handle in drawn {
            let Some(mut atlas) = images.get_mut(&handle) else {
                continue;
            };
            let Some(data) = atlas.data.as_mut() else {
                continue;
            };
            reached = true;
            for &index in &chunks {
                let Some(chunk) = tile.chunk(index) else {
                    continue;
                };
                let cell = cell_of(chunk, index, data);
                rules::write_alpha_atlas_cell(data, index, &cell);
            }
        }
        if !reached {
            // Drawn, but its atlas is not in `Assets<Image>`. The client builds
            // them that way, and so does a tile that arrived before `LiveEdits`
            // was set. There is nothing to patch, so the tile is read again.
            session.stale.insert(coord);
        }
    }
}

/// One chunk's atlas cell, from the file's blend maps and the atlas's own
/// shadow.
///
/// The alpha channel is read back rather than recomputed. It is the chunk's
/// baked `MCSH`, which a paint stroke does not touch. The copy already in the
/// atlas is the one the tile was built with, so taking it from there is cheaper
/// than decoding `MCSH` again and cannot disagree with it.
///
/// The blend maps take the edge fix here, because the cell is what a renderer
/// reads: the file's last row and column hold nothing the reference client
/// ever samples. `vale_edit` decodes without the fix, so that a tile written
/// back is written back as it was.
fn cell_of(chunk: &vale_edit::adt::MapChunk, index: usize, atlas: &[u8]) -> Vec<u8> {
    let paint = vale_edit::adt::alpha::paint(chunk);
    let mut maps = paint.maps;
    for map in maps.iter_mut() {
        rules::fix_alpha_edge(map);
    }
    let mut cell = rules::alpha_atlas_cell(&maps, &[]);

    let side = rules::ATLAS_SIDE;
    let (cell_x, cell_y) = (index % CHUNKS_PER_SIDE, index / CHUNKS_PER_SIDE);
    for texel in 0..ALPHA_LEN {
        let (tx, ty) = (texel % ALPHA_SIDE, texel / ALPHA_SIDE);
        let at = ((cell_y * ALPHA_SIDE + ty) * side + cell_x * ALPHA_SIDE + tx) * 4 + 3;
        cell[texel * 4 + 3] = atlas.get(at).copied().unwrap_or(0);
    }
    cell
}

/// The tiles a brush's circle reaches. This is
/// `super::terrain::tiles_in_range`, which is shared because the two brushes
/// ask about the same circle. A wrong answer for a large radius leaves a stroke
/// with a cross of untouched ground through it.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    super::terrain::tiles_in_range(radius, at)
}

/// The pinned chunk's square, on the ground.
///
/// Without it a pin cannot be seen: the panel says "chunk 137" and nothing on
/// screen shows which of the 2,304 squares in view that is. The square is drawn
/// along the chunk's own edges at the edited heights, through the same height
/// lookup the brush ring uses, so it lies on the ground rather than flat across
/// it.
fn draw_pin(
    mut gizmos: Gizmos,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    textures: Res<Textures>,
) {
    if *tool != Tool::Textures {
        return;
    }
    let (Some(session), Some((coord, chunk))) = (session, textures.pinned) else {
        return;
    };
    let Some(origin) = session
        .tiles
        .get(&coord)
        .and_then(|tile| tile.chunk(chunk))
        .map(|chunk| chunk.head().position())
    else {
        return;
    };

    /// Points sampled along each edge. Enough that a chunk's square follows a
    /// slope rather than cutting through it.
    const ALONG: usize = 9;
    /// Yards above the ground, so the line is not buried in the mesh.
    const LIFT: f32 = 0.2;
    let colour = Color::srgb(1.0, 0.85, 0.35);
    let side = rules::CHUNK_SIZE;

    let at = |t: f32, edge: usize| -> Option<Vec3> {
        // The origin is the chunk's maximum corner and the square runs down from
        // it in both axes, which is the convention `vale_assets` uses.
        let (x, y) = match edge {
            0 => (origin[0], origin[1] - t * side),
            1 => (origin[0] - t * side, origin[1] - side),
            2 => (origin[0] - side, origin[1] - side + t * side),
            _ => (origin[0] - side + t * side, origin[1]),
        };
        let coord = vale_assets::tile_for_position(x, y);
        let z = session
            .tiles
            .get(&coord)
            .and_then(|tile| vale_edit::adt::heights::height_at(tile, x, y))?;
        Some(vale_client::render::axes::to_bevy([x, y, z + LIFT]))
    };
    for edge in 0..4 {
        for step in 0..ALONG {
            let (a, b) = (step as f32 / ALONG as f32, (step + 1) as f32 / ALONG as f32);
            if let (Some(from), Some(to)) = (at(a, edge), at(b, edge)) {
                gizmos.line(from, to, colour);
            }
        }
    }
}

/// The brush ring, on the ground it is about to paint.
///
/// It is drawn through the height tool's own [`super::terrain::ring`], which
/// follows the edited heights rather than lying flat. Both brushes use the same
/// ring, so they cannot disagree about where the pointer is.
fn draw_brush(
    mut gizmos: Gizmos,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    textures: Res<Textures>,
    cursor: Res<Cursor>,
) {
    if *tool != Tool::Textures {
        return;
    }
    let (Some(session), Some(at)) = (session, cursor.ground) else {
        return;
    };
    // The eraser's ring is red, so the mode is read at the pointer and not
    // only on the panel. Shift is not asked here: the ring is drawn from the
    // panel's mode.
    let colour = match textures.brush.erase {
        true => Color::srgb(1.0, 0.45, 0.4),
        false => Color::srgb(0.85, 0.6, 1.0),
    };
    super::terrain::rings(
        &mut gizmos,
        &session,
        at,
        textures.brush.radius,
        textures.brush.core,
        textures.brush.shape,
        colour,
    );
}

/// One row of the chunk-under-the-pointer list, as the panel needs it.
#[derive(Debug, Clone)]
pub struct Layer {
    pub texture: String,
    /// How much of the chunk this layer shows, 0 to 1. It is not the layer's
    /// own alpha. See `vale_edit::adt::alpha::Paint::coverage`.
    pub coverage: f32,
    /// What it grows: the `GroundEffectTexture` row, zero for nothing.
    pub effect_id: u32,
    /// How its texture moves: `(direction 0..7, speed 0..7, on)`, which is
    /// `MCLY`'s texture animation. See
    /// `vale_assets::world::adt::layer_flags::ANIMATION_ROTATION`.
    pub animation: (u32, u32, bool),
}

/// Which chunk the layer list is about: the pinned one, or the one under the
/// pointer.
pub fn shown(textures: &Textures, cursor: &Cursor) -> Option<((u32, u32), usize)> {
    textures.pinned.or_else(|| cursor.tile.zip(cursor.chunk))
}

/// What a chunk is painted with.
///
/// This list shows why the brush stopped on a chunk. Four textures is the limit
/// and about half the shipped chunks are already at four, so a person painting
/// needs to see which four, and which of them shows nothing. The coverage
/// column gives the second: a layer at 0% can be taken off and the picture does
/// not change.
///
/// `None` for a tile that is not open.
pub fn layers_of(session: &EditSession, coord: (u32, u32), chunk: usize) -> Option<Vec<Layer>> {
    let tile = session.tiles.get(&coord)?;
    let paint = vale_edit::adt::alpha::paint(tile.chunk(chunk)?);
    let names = tile.texture_names();
    let coverage = paint.coverage();
    Some(
        paint
            .layers
            .iter()
            .enumerate()
            .map(|(i, layer)| Layer {
                texture: names
                    .get(layer.texture_id as usize)
                    .cloned()
                    .unwrap_or_default(),
                coverage: coverage.get(i).copied().unwrap_or(0.0),
                effect_id: layer.effect_id,
                animation: paint.layer_animation(i).unwrap_or((0, 0, false)),
            })
            .collect(),
    )
}

/// What the shipped ground grows on this texture: the `effectId` most often
/// paired with it across the open tiles' layers, ignoring zero, or `None` when
/// no open layer of it grows anything.
///
/// The open layers are the only source for this, since neither table names a
/// texture: `GroundEffectTexture` is keyed by an id that only `MCLY` carries.
/// A brush that takes its id from here plants what the zone plants. A person
/// who wants something else has the number beside it.
pub fn usual_effect(session: &EditSession, path: &str) -> Option<u32> {
    let mut seen: bevy::platform::collections::HashMap<u32, usize> = Default::default();
    for tile in session.tiles.values() {
        let Some(texture_id) = tile
            .texture_names()
            .iter()
            .position(|name| name.eq_ignore_ascii_case(path))
        else {
            continue;
        };
        for index in 0..tile.chunks.len() {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            for layer in vale_edit::adt::alpha::paint(chunk).layers {
                if layer.texture_id as usize == texture_id && layer.effect_id != 0 {
                    *seen.entry(layer.effect_id).or_default() += 1;
                }
            }
        }
    }
    seen.into_iter()
        .max_by_key(|&(id, count)| (count, std::cmp::Reverse(id)))
        .map(|(id, _)| id)
}

/// Change what one layer grows. It records as [`swap_layer`] does: one entry
/// on the history. The entry is folded by `now`, so a dragged number field is
/// one press of undo. The tile is read again, since the foliage is built from
/// the layer.
pub fn set_layer_effect(
    session: &mut EditSession,
    coord: (u32, u32),
    chunk: usize,
    layer: usize,
    effect_id: u32,
    now: f64,
) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    if !paint.set_layer_effect(layer, effect_id) {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session.history.begin_gesture(
        format!("Set ground effect {effect_id} on layer {layer}"),
        format!("effect {} {chunk} {layer}", key.vpath()),
        now,
    );
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = format!("layer {layer} of chunk {chunk} now uses ground effect {effect_id}");
}

/// Change how one layer's texture moves. This is `MCLY`'s texture animation,
/// the direction and speed that make lava move. It records as
/// [`set_layer_effect`] does: one entry on the history, folded by `now` so
/// that a dragged number is one press of undo.
///
/// The tile is read again rather than patched, and the reason is not the paint.
/// The velocity reaches the shader as part of the draw group's own uniform, and
/// `Adt::to_mesh` groups chunks by their textures and their scroll. A chunk
/// that starts moving therefore has to leave the group of still chunks it was
/// grouped with. See `vale_assets::world::adt::TerrainDraw::scrolls`.
pub fn set_layer_animation(
    session: &mut EditSession,
    coord: (u32, u32),
    chunk: usize,
    layer: usize,
    turn: u32,
    rate: u32,
    on: bool,
    now: f64,
) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    if !paint.set_layer_animation(layer, turn, rate, on) {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session.history.begin_gesture(
        format!("Set texture animation on layer {layer}"),
        format!("animate {} {chunk} {layer}", key.vpath()),
        now,
    );
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = match on {
        true => format!(
            "layer {layer} of chunk {chunk} scrolls toward {}°, speed {rate}",
            turn * 45
        ),
        false => format!("texture animation off on layer {layer} of chunk {chunk}"),
    };
}

/// Take one layer off the chunk under the pointer.
///
/// This is the one way out of the four-layer limit. It is a button a person
/// presses rather than something the brush does for them: dropping a layer
/// discards the blend it was carrying, and the coverage column informs the
/// choice of which of the four to drop but does not make it.
///
/// It changes the chunk's texture set, so the tile is read again rather than
/// patched. See this module's own comment on the two routes.
pub fn drop_layer(session: &mut EditSession, coord: (u32, u32), chunk: usize, layer: usize) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    let name = tile
        .texture_names()
        .get(
            paint
                .layers
                .get(layer)
                .map(|l| l.texture_id as usize)
                .unwrap_or(usize::MAX),
        )
        .cloned()
        .unwrap_or_default();
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    if !paint.remove_layer(layer) {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session.history.begin(format!("Remove {}", leaf(&name)));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = format!("removed {} from chunk {chunk}", leaf(&name));
}

/// Swap what one layer draws, keeping its blend.
///
/// This gets past the four-layer limit without discarding anything: a layer
/// painted in the right shape with the wrong tileset keeps its shape and
/// changes its texture. `vale_edit::adt::alpha::Paint::set_layer_texture`
/// is the operation. This function names the texture in the tile's own `MTEX`
/// and records the undo entry, as [`set_base`] does. As there, the tile is
/// read again, since the chunk's texture set changed.
pub fn swap_layer(
    session: &mut EditSession,
    coord: (u32, u32),
    chunk: usize,
    layer: usize,
    path: &str,
) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    let texture_id = tile.name_texture(path);
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    if !paint.set_layer_texture(layer, texture_id) {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session
        .history
        .begin(format!("Replace layer {layer} texture with {}", leaf(path)));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = format!("layer {layer} of chunk {chunk} now uses {}", leaf(path));
}

/// Replace the texture a chunk is painted with underneath every layer.
///
/// The base is the one part of a chunk's paint no brush can reach: painting
/// adds a layer over it, and a chunk already at the four-layer limit cannot do
/// even that. Ground authored on the wrong tileset therefore could not be
/// corrected. A user asked "is there a way to clear or change the base texture
/// for a tile?", and there was not.
///
/// It replaces rather than removes, which keeps it well defined: every blend
/// map and every layer above the base stays exactly as it was, and the one
/// texture id underneath them changes. Removing a base would mean inventing
/// what the chunk looks like where the layer above it is transparent.
///
/// `vale_edit::adt::alpha::Paint::set_base` is the operation. This function
/// names the texture in the tile's own `MTEX` and records the undo entry.
pub fn set_base(session: &mut EditSession, coord: (u32, u32), chunk: usize, path: &str) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    // The texture is named on the tile first. `MTEX` is the tile's list and the
    // layer holds an index into it, so a texture the tile does not list has to
    // be added before a chunk can point at it.
    let texture_id = tile.name_texture(path);
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    if !paint.set_base(texture_id) {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session.history.begin(format!("Set base texture {}", leaf(path)));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
}

/// Take every layer off but the base, leaving flat ground.
///
/// With [`set_base`], this is how a chunk's paint is cleared. Dropping layers
/// one at a time also works, but it is three presses and the index of each
/// remaining layer shifts after every one.
pub fn clear_paint(session: &mut EditSession, coord: (u32, u32), chunk: usize) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
        Some(chunk) => chunk,
        None => return,
    });
    if paint.clear_to_base() == 0 {
        return;
    }
    let Some(open) = tile.chunk_mut(chunk) else {
        return;
    };
    vale_edit::adt::alpha::set_paint(open, &paint);
    let edit = vale_edit::ops::Edit::Paint {
        chunk,
        before: Box::new(before),
        after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
    };
    session.history.begin("Clear paint".to_string());
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
}

/// Replace the base texture of every chunk of a tile, as [`set_base`] does for
/// one chunk. This is how a tile's base is changed.
///
/// It records one undo entry for the whole tile rather than 256, because it is
/// one action and undoing it a chunk at a time would be 256 presses.
pub fn set_tile_base(session: &mut EditSession, coord: (u32, u32), path: &str) -> usize {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return 0;
    };
    let texture_id = tile.name_texture(path);
    let mut edits = Vec::new();
    for chunk in 0..vale_edit::adt::blank::CHUNKS {
        let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
        let mut paint = vale_edit::adt::alpha::paint(match tile.chunk(chunk) {
            Some(chunk) => chunk,
            None => continue,
        });
        if !paint.set_base(texture_id) {
            continue;
        }
        let Some(open) = tile.chunk_mut(chunk) else {
            continue;
        };
        vale_edit::adt::alpha::set_paint(open, &paint);
        edits.push(vale_edit::ops::Edit::Paint {
            chunk,
            before: Box::new(before),
            after: Box::new(vale_edit::ops::ChunkPaint::capture(tile, chunk)),
        });
    }
    let changed = edits.len();
    if changed == 0 {
        return 0;
    }
    session.history.begin(format!("Set tile base texture {}", leaf(path)));
    session.history.record(&key, edits);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    changed
}

/// An archive path as a row in a list: the folder it is in, and its file name.
///
/// The row has two components and not one. The tilesets are grouped by zone
/// (`Tileset\Elwynn\`, `Tileset\Barrens\`) and the folder is most of what
/// identifies one, because `grassbase.blp` appears in a dozen of them. The rest
/// of the directory is in the whole path, which is shown on hover.
///
/// The names come back from the archive listing in lower case, because that is
/// how the listing is keyed. The archives answer either spelling, so a stroke
/// writes a lower-case path into `MTEX` where Blizzard's own tools wrote mixed
/// case. Nothing reads it case-sensitively.
pub fn leaf(path: &str) -> &str {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let rest = &path[..path.len() - file.len()];
    let folder = rest.trim_end_matches(['\\', '/']);
    let folder = folder.rsplit(['\\', '/']).next().unwrap_or("");
    match folder.is_empty() {
        true => file,
        // One slice of the original, so this borrows rather than allocating on
        // every row of a list redrawn every frame.
        false => &path[path.len() - file.len() - folder.len() - 1..],
    }
}

/// The five falloffs, as a list the panel can offer. It is the height brush's
/// own list, named again here so the panel does not reach into the other tool's
/// file for it.
pub const FALLOFFS: [(&str, Falloff); 5] = super::terrain::FALLOFFS;

/// The three footprints, as a list the panel can offer. This and [`FALLOFFS`]
/// are the height brush's own lists and not a second pair: a texture brush
/// whose edge did not match the height brush's would make the two tools
/// disagree about where the pointer is.
pub const SHAPES: [(&str, vale_edit::ops::Shape); 3] = super::terrain::SHAPES;

#[cfg(test)]
mod tests {
    use super::*;

    /// A brush in the middle of a tile names one tile, and one on a corner names
    /// four. The height brush has the same property. It keeps a stroke from
    /// ending in a straight line at a border.
    #[test]
    fn a_brush_names_every_tile_its_circle_reaches() {
        let middle = vale_assets::world::adt::TILE_SIZE * 0.5;
        let origin = vale_assets::world::adt::MAP_ORIGIN;
        // The centre of tile (32, 32), which is the middle of the map.
        let at = Vec3::new(
            origin - 32.0 * 533.33333 - middle,
            origin - 32.0 * 533.33333 - middle,
            0.0,
        );
        assert_eq!(tiles_under(10.0, at).len(), 1);
        // The tile's far corner, where four tiles meet.
        let corner = Vec3::new(at.x + middle, at.y + middle, 0.0);
        assert_eq!(tiles_under(10.0, corner).len(), 4);
    }

    /// A row names the folder and the file. `grassbase.blp` on its own is in a
    /// dozen tilesets, so the zone folder is most of what identifies one.
    #[test]
    fn a_row_names_the_folder_and_the_file() {
        assert_eq!(
            leaf("tileset\\elwynn\\grassbase.blp"),
            "elwynn\\grassbase.blp"
        );
        assert_eq!(leaf("tileset/elwynn/grassbase.blp"), "elwynn/grassbase.blp");
        // A path with no folder above it is returned unchanged.
        assert_eq!(leaf("grassbase.blp"), "grassbase.blp");
        assert_eq!(leaf(""), "");
    }

    /// The catalogue is the archives' list without the gloss masks. Every
    /// tileset ships beside a `_s` variant holding its specular, and offering
    /// both would list each texture twice under names that differ by two
    /// characters.
    #[test]
    fn the_gloss_masks_are_not_in_the_catalogue() {
        let gloss = |path: &str| path.trim_end_matches(".blp").ends_with("_s");
        assert!(gloss("Tileset\\Elwynn\\ElwynnGrass_s.blp"));
        assert!(!gloss("Tileset\\Elwynn\\ElwynnGrass.blp"));
    }
}
