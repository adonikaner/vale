//! The texture brush: what the ground is painted with.
//!
//! ## It is the first tool whose edit changes the size of the file
//!
//! The height brush moves 580 bytes about inside a region that stays 580 bytes
//! long. Painting with a texture a chunk does not carry adds a `MCLY` record and
//! a blend map, which moves every region after them and every offset that names
//! one. That is `vale_edit`'s problem and it is solved there — see
//! `vale_edit::adt::alpha`, and the round trip over five real tiles that says
//! the writer can do it.
//!
//! What is left here is the half a person touches, and it has one decision in it
//! that the height brush did not have to make.
//!
//! ## A stroke reaches the screen two different ways, and which one decides the cost
//!
//! ```text
//! the alpha changed        write the chunk's cell into the tile's atlas
//! the texture set changed  read the tile again
//! ```
//!
//! A chunk's blend maps are texels in one 1024x1024 atlas per tile, so moving
//! them is a write into an image that is already on the GPU: 16 KB for a chunk,
//! at every level of the mip chain, which
//! `vale_assets::world::adt::write_alpha_atlas_cell` does in one call because
//! a cell is self-contained at every level the chain reaches.
//!
//! Which *textures* a chunk names is not in the atlas at all. It is part of the
//! `TerrainMaterial` its draw group was built with — four image handles and a
//! layer count — so a chunk given a fourth texture has to have its tile read
//! again. That is a third of a second in a debug build, and it happens **once
//! per texture per chunk**: the first stroke that introduces a texture pays for
//! it and every stroke after it is live. [`Edit::changes_the_texture_set`] is
//! what tells the two apart and it is asked per edit, not per stroke.
//!
//! ## The catalogue is the archives' own listing
//!
//! `Tileset\` is 1,700-odd paths in the MPQ listing and this offers all of them,
//! filtered by a search box. Not the tile's own `MTEX`: that is the list of what
//! is already there, which is the one list a person painting does not need.
//!
//! ## What a texture edit does not carry with it
//!
//! `MCLY`'s `effectId` is the ground foliage a layer grows — the tufts and
//! shrubs `render::foliage` plants — and a new layer is written with none. So
//! painting grass over dirt does not plant grass; that is a second field on the
//! same record and a second panel, and it is left for later rather
//! than guessed at here.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_assets::world::adt::{self as rules, ALPHA_LEN, ALPHA_SIDE, CHUNKS_PER_SIDE};
use vale_client::assets::GameAssets;
use vale_client::render::terrain::{TerrainTile, TileAlpha};
use vale_edit::ops::{Falloff, PaintBrush, Working};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the texture brush may be, in yards, and how hard.
///
/// The radius has the same real ceiling the height brush's does and for the same
/// reason — a stroke reaches only the tiles this session has open — and the
/// strength has none worth the name: it is a fraction of the remaining distance
/// a second and one step is clamped to all of it, so a large number is simply
/// "immediately". Both were guesses before (100 yards and 10), and a guess that
/// refuses work the tool can do is the wrong kind.
pub const RADIUS: std::ops::RangeInclusive<f32> = 0.5..=(crate::OPEN_BLOCK / 2.0);
pub const RATE: std::ops::RangeInclusive<f32> = 0.0..=1_000.0;

/// The texture tool's settings.
#[derive(Resource, Debug, Clone)]
pub struct Textures {
    pub brush: PaintBrush,
    /// Every `Tileset\` path the archives name, sorted. Read once, lazily — see
    /// [`read_catalogue`].
    pub catalogue: Vec<String>,
    /// What the panel's search box holds. Held here rather than in the panel so
    /// that it survives the tool being switched away from and back.
    pub search: String,
    /// **Which folder the picker is showing**, as a lower-case path prefix
    /// ending in a separator.
    ///
    /// The same arrangement the model picker has and for the same reason — see
    /// [`crate::tools::place`]. 621 tilesets in one scroller means every texture
    /// is found by typing, and the archives' own directories are the game's own
    /// grouping: `Tileset\Elwynn\` is the set a zone was painted with.
    pub folder: String,
    /// **The chunk the panel is about, when it is not the one under the
    /// pointer.**
    ///
    /// The layer list has to be *reachable*: a person who wants to take a layer
    /// off has to move the pointer to the button, and the moment they do, the
    /// pointer is over something else and the panel is showing a different
    /// chunk. It was reported exactly that way — "deleting one requires moving
    /// the pointer, at which point whatever you were trying to delete has
    /// disappeared. It's circular."
    ///
    /// Latching on the pointer leaving the viewport does not fix it either,
    /// because reaching the panel sweeps the pointer across every chunk between
    /// here and the right-hand edge.
    ///
    /// So it is pinned, two ways, and neither of them needs the pointer to move:
    /// [`PIN`] pins whatever is under it, and a stroke that is **refused** pins
    /// the chunk that refused it — which is the case the list exists for, and
    /// means the answer to "why did it stop" is already on screen.
    pub pinned: Option<((u32, u32), usize)>,
}

impl Default for Textures {
    fn default() -> Textures {
        Textures {
            brush: PaintBrush::default(),
            catalogue: Vec::new(),
            search: String::new(),
            // **[`ROOT`] and not the empty string**, which a derived default
            // would give: an empty prefix matches every path, so the picker
            // would open on one folder called `tileset` with everything in it
            // and a click nobody can avoid.
            folder: ROOT.to_string(),
            pinned: None,
        }
    }
}

/// The key that pins the chunk under the pointer, and unpins it.
///
/// **A key and not a button**, which is the whole point: a button would have to
/// be reached, and reaching it is what moves the pointer off the chunk. Space is
/// free — the camera flies on `WASD`, `Q` and `E`.
const PIN: KeyCode = KeyCode::Space;

/// Where the tileset picker starts. Everything the catalogue holds is under it
/// — it comes from `list_prefix("Tileset\\")` — so a level above would be one
/// folder with everything in it.
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
                    // **After the pick**, for the reason the height brush is:
                    // the stroke is aimed by where the pointer met the ground,
                    // and a frame earlier is a stroke that trails the cursor.
                    .after(crate::pick::aim),
            )
            .add_systems(Update, (draw_brush, draw_pin));
    }
}

/// Whether a stroke is being held, and whether the press that began it was the
/// world's.
///
/// The same two bools the doodad tool keeps and for the same reason: **a drag
/// belongs to where it began**, so the gate is asked on the press and not on
/// every frame. Without that a stroke that wandered over the inspector stopped
/// dead in the middle and started again on the way back.
#[derive(Resource, Default)]
struct Held {
    painting: bool,
    armed: bool,
    /// **The stroke's own copy of the texels it is working on**, per tile.
    ///
    /// A stroke cannot accumulate in the numbers the file holds: a 4-bit chunk
    /// has sixteen levels, so any step smaller than half a quantum is a step
    /// that rounds back to where it started. See `vale_edit::ops::Working`,
    /// which is the whole argument — and note that it is cleared when the button
    /// comes up, because a copy kept past the stroke that made it disagrees with
    /// the file the moment anything else writes.
    working: bevy::platform::collections::HashMap<(u32, u32), Working>,
}

/// The list of tilesets, read once.
///
/// Lazily rather than at startup: it walks the whole listing of nineteen
/// archives, and a session that never opens this tool should not pay for it.
fn read_catalogue(mut textures: ResMut<Textures>, tool: Res<Tool>, assets: Res<GameAssets>) {
    if *tool != Tool::Textures || !textures.catalogue.is_empty() {
        return;
    }
    let found = assets.with_archive(|chain| Ok(chain.list_prefix("Tileset\\")));
    let mut found: Vec<String> = match found {
        Ok(found) => found
            .into_iter()
            .filter(|path| path.ends_with(".blp"))
            // The `_s` files are the specular masks that sit beside every
            // tileset — see `vale_assets::world::adt`, which resolves them
            // from the base name. Offering them would be offering each texture
            // twice, once as itself and once as its gloss.
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
    // **Pin, from the keyboard**, so that choosing a chunk does not move the
    // pointer off it — see [`Textures::pinned`]. Pressing it again on the same
    // chunk lets go.
    if keys.just_pressed(PIN) && !wants.wants_keyboard_input() && in_world {
        let under = cursor.tile.zip(cursor.chunk);
        textures.pinned = match (textures.pinned, under) {
            (Some(had), Some(now)) if had == now => None,
            (_, now) => now,
        };
    }
    // **The same three numbers on the wheel the height brush has** — see
    // [`crate::tools::Wheel`], which is the one list of what each modifier puts
    // on it and is what `camera::fly` reads to decline the same notch. Two
    // brushes that answered different modifiers would be two things to learn for
    // one gesture.
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
        session.status = "pick a texture first".into();
        return;
    }
    let Some(at) = cursor.ground else { return };

    if !held.painting {
        held.painting = true;
        session
            .history
            .begin(format!("Paint {}", leaf(&textures.brush.texture)));
    }

    let brush = textures.brush.clone();
    let seconds = time.delta_secs();
    let mut full: Vec<((u32, u32), usize)> = Vec::new();
    // **Every tile the circle reaches**, on the height brush's own argument: a
    // stroke that stopped at a tile border would leave the paint ending in a
    // straight line exactly there, with nothing about either file wrong.
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
        let edits = painted.edits;
        if edits.is_empty() {
            continue;
        }
        // **Which route to the screen each edit takes, decided per edit.** A
        // stroke that gives one chunk a new texture and moves the alpha on three
        // others rebuilds the tile once and patches the other three, rather than
        // rebuilding on every frame it is held.
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

    // **What the stroke could not do, said out loud.** A chunk already carrying
    // four textures cannot take a fifth, and roughly half the ground in the
    // shipped tiles is already at four — so a brush that refused in silence
    // stops dead part-way across a hillside with nothing anywhere saying why.
    // The inspector's *Chunk under the pointer* section is where the way out is.
    session.status = match full.len() {
        0 => format!("painting {}", leaf(&brush.texture)),
        1 => "1 chunk is full: four textures is the limit".to_string(),
        n => format!("{n} chunks are full: four textures is the limit"),
    };
    // **…and the first of them is put in front of the person painting.** This is
    // the case the layer list exists for: the stroke stopped, and the panel is
    // already showing which chunk stopped it and what its four textures are.
    //
    // It replaces a pin that was already there, deliberate or not. The rule is
    // *the panel shows what just stopped you*, and a pin held from a minute ago
    // while the brush is refusing somewhere else is the panel answering a
    // question nobody is asking.
    if let Some(&(coord, chunk)) = full.first() {
        textures.pinned = Some((coord, chunk));
    }
}

/// Write the repainted chunks' blend maps into the atlases that are already
/// drawn.
///
/// The paint counterpart of `super::terrain::live_ground`, and it is cheaper per
/// chunk than that one is: a cell is 64x64 texels wherever it is in the tile, so
/// this writes 16 KB and a mip chain rather than comparing against 81,840
/// vertices.
///
/// **What it costs is the re-upload**, which is the whole atlas: writing to an
/// `Image` marks the asset changed and Bevy re-prepares all 5.3 MiB of it. That
/// is per frame of a stroke and per tile the stroke is over, and it is the one
/// number in this tool nothing has measured.
///
/// A tile whose atlas is not in `Assets<Image>` cannot be patched — the client
/// builds them that way, and so does a tile that arrived before
/// `LiveEdits` was set — and is marked stale instead, exactly as `live_ground`
/// falls back.
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
        // **Every entity with this coordinate, not the first.** While a tile is
        // being replaced there are two — the outgoing one, which is what is on
        // screen, and the incoming one, which is hidden — and `find` picks
        // whichever the query happens to yield. Patching only the hidden one is
        // a stroke that stops appearing half way through and starts again when
        // the swap lands. See `super::terrain::swap`.
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
            // Drawn, but its atlas is not in `Assets<Image>` — the client builds
            // them that way, and so does a tile that arrived before `LiveEdits`
            // was set. Nothing to patch, so it is read again.
            session.stale.insert(coord);
        }
    }
}

/// One chunk's atlas cell, from the file's blend maps and the atlas's own
/// shadow.
///
/// **The alpha channel is read back rather than recomputed.** It is the chunk's
/// baked `MCSH`, which a paint stroke does not touch, and the copy already in
/// the atlas is by construction the one the tile was built with — so taking it
/// from there is both cheaper than decoding `MCSH` again and immune to
/// disagreeing with it.
///
/// The blend maps take the edge fix on the way out, because that is what a
/// *renderer* reads: the file's last row and column hold nothing the reference
/// client ever samples, and `vale_edit` deliberately decodes without it so
/// that a tile written back is written back as it was.
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

/// The tiles a brush's circle reaches — `super::terrain::tiles_in_range`, which
/// is shared because the two brushes ask about the same circle and one of them
/// getting it wrong for a large radius is a stroke with a cross of untouched
/// ground through it.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    super::terrain::tiles_in_range(radius, at)
}

/// The pinned chunk's square, on the ground.
///
/// **Without it a pin is invisible**: the panel says *chunk 137* and there is
/// nothing on screen saying which of the 2,304 squares in view that is. Drawn
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
        // it in both axes — `vale_assets`' own convention.
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
/// Through the height tool's own [`super::terrain::ring`], which follows the
/// edited heights rather than lying flat — the same ring, so the two brushes
/// cannot come to disagree about where the pointer is.
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
    let colour = Color::srgb(0.85, 0.6, 1.0);
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
    /// What this layer actually shows, 0 to 1 — see
    /// `vale_edit::adt::alpha::Paint::coverage`, and note that it is not the
    /// layer's own alpha.
    pub coverage: f32,
    /// What it grows: the `GroundEffectTexture` row, zero for nothing.
    pub effect_id: u32,
    /// **How it crawls**: `(direction 0..7, speed 0..7, on)` — `MCLY`'s
    /// texture animation. See
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
/// **The answer to "why did the brush stop here".** Four textures is the limit
/// and about half the shipped chunks are already at four, so a person painting
/// needs to see which four and which of them is doing nothing — which is what
/// the coverage column is for: a layer at 0% can be taken off and the picture
/// does not change.
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

/// **What the shipped ground grows on this texture**: the `effectId` most
/// often paired with it across the open tiles' layers, ignoring zero, or
/// `None` when no open layer of it grows anything.
///
/// The one place the answer can come from, since neither table names a
/// texture: `GroundEffectTexture` is keyed by an id that only `MCLY` carries.
/// A brush that took its id from here plants what the zone plants; a person
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

/// **Change what one layer grows**, on [`swap_layer`]'s terms — one entry on
/// the history, folded by `now` so a dragged number field is one press of
/// undo. The tile is read again, since the foliage is built from the layer.
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
        format!("Grow effect {effect_id} on layer {layer}"),
        format!("effect {} {chunk} {layer}", key.vpath()),
        now,
    );
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = format!("layer {layer} of chunk {chunk} now grows effect {effect_id}");
}

/// **Change how one layer crawls** — `MCLY`'s texture animation, the direction
/// and speed that make lava move. On [`set_layer_effect`]'s terms: one entry on
/// the history, folded by `now` so that a dragged number is one press of undo.
///
/// The tile is read again rather than patched, and the reason is not the paint:
/// the velocity reaches the shader as part of the **draw group's** own uniform,
/// and `Adt::to_mesh` groups chunks by their textures *and* their scroll — so a
/// chunk that starts crawling has to leave the group of the still ones it was
/// folded in with. See `vale_assets::world::adt::TerrainDraw::scrolls`.
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
        format!("Animate layer {layer}"),
        format!("animate {} {chunk} {layer}", key.vpath()),
        now,
    );
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = match on {
        true => format!(
            "layer {layer} of chunk {chunk} crawls {}° at speed {rate}",
            turn * 45
        ),
        false => format!("layer {layer} of chunk {chunk} is still"),
    };
}

/// Take one layer off the chunk under the pointer.
///
/// The one way out of the four-layer limit, and it is deliberately a button a
/// person presses rather than something the brush does on their behalf: dropping
/// a layer throws away whatever blend it was carrying, and which of the four is
/// expendable is a judgement the coverage column informs and does not make.
///
/// It changes the chunk's texture *set*, so the tile is read again rather than
/// patched — see this module's own comment on the two routes.
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

/// **Swap what one layer draws, keeping its blend.**
///
/// The way through the four-layer limit that throws nothing away: a layer
/// painted in the right shape with the wrong tileset keeps its shape and
/// changes its texture. `vale_edit::adt::alpha::Paint::set_layer_texture`
/// is the operation; what is here is naming the texture in the tile's own
/// `MTEX` and recording the undo entry, on [`set_base`]'s own terms — and
/// like it, the tile is read again, since the chunk's texture *set* changed.
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
        .begin(format!("Swap layer {layer} to {}", leaf(path)));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    session.status = format!("layer {layer} of chunk {chunk} now draws {}", leaf(path));
}

/// **Replace what a chunk is painted with underneath everything.**
///
/// The base is the one part of a chunk's paint no brush could reach: painting
/// adds a layer *over* it, and a chunk already at the four-layer limit cannot
/// even do that. So ground authored on the wrong tileset had no way back — which
/// was reported from the window as *"is there a way to clear or change the base
/// texture for a tile?"*, and there was not.
///
/// **It replaces rather than removes**, which is what makes it well defined:
/// every blend map and every layer above the base stays exactly as it was, and
/// the one texture id underneath them changes. Removing a base would mean
/// inventing what the chunk looks like where the layer above it is transparent.
///
/// `vale_edit::adt::alpha::Paint::set_base` is the operation; what is here is
/// naming the texture in the tile's own `MTEX` and recording the undo entry.
pub fn set_base(session: &mut EditSession, coord: (u32, u32), chunk: usize, path: &str) {
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = vale_edit::ops::ChunkPaint::capture(tile, chunk);
    // **Named on the tile first.** `MTEX` is the tile's list and the layer holds
    // an index into it, so a texture the tile has never heard of has to be added
    // before a chunk can point at it.
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
    session.history.begin(format!("Base {}", leaf(path)));
    session.history.record(&key, [edit]);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
}

/// **Take every layer off but the base**, leaving flat ground.
///
/// The other half of *clear this chunk's paint*. Dropping layers one at a time
/// works and is three presses with an index that shifts under you each time.
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

/// …and the same over **every chunk of a tile**, which is what *change this
/// tile's base* means.
///
/// One undo entry for the whole tile rather than 256, because it is one action
/// and undoing it a chunk at a time would be 256 presses.
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
    session.history.begin(format!("Tile base {}", leaf(path)));
    session.history.record(&key, edits);
    session.history.end();
    session.publish(coord);
    session.stale.insert(coord);
    changed
}

/// An archive path as a row in a list: the folder it is in, and its file name.
///
/// **Two components and not one.** The tilesets are grouped by zone —
/// `Tileset\Elwynn\`, `Tileset\Barrens\` — and the folder is most of what
/// identifies one, because `grassbase.blp` appears in a dozen of them. The whole
/// path is a hover away, which is where the rest of the directory belongs.
///
/// The names come back from the archive listing in lower case, because that is
/// how the listing is keyed. The archives answer either spelling, so what a
/// stroke writes into `MTEX` is a lower-case path where Blizzard's own tools
/// wrote mixed case; nothing reads it case-sensitively.
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

/// The three falloffs, as a list the panel can offer. The height brush's own
/// list, named again here so the panel does not reach across into the other
/// tool's file for it.
pub const FALLOFFS: [(&str, Falloff); 5] = super::terrain::FALLOFFS;

/// …and the three footprints, likewise. **The height brush's own lists and not
/// a second pair**: a texture brush whose edge did not match the one that moves
/// the ground would be two tools that disagree about where the pointer is.
pub const SHAPES: [(&str, vale_edit::ops::Shape); 3] = super::terrain::SHAPES;

#[cfg(test)]
mod tests {
    use super::*;

    /// A brush in the middle of a tile names one tile, and one on a corner names
    /// four. The same property the height brush has, and the one that keeps a
    /// stroke from ending in a straight line at a border.
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
        // …and its far corner, where four tiles meet.
        let corner = Vec3::new(at.x + middle, at.y + middle, 0.0);
        assert_eq!(tiles_under(10.0, corner).len(), 4);
    }

    /// **A row names the folder and the file.** `grassbase.blp` on its own is
    /// in a dozen tilesets, so the zone folder is most of what identifies one.
    #[test]
    fn a_row_names_the_folder_and_the_file() {
        assert_eq!(
            leaf("tileset\\elwynn\\grassbase.blp"),
            "elwynn\\grassbase.blp"
        );
        assert_eq!(leaf("tileset/elwynn/grassbase.blp"), "elwynn/grassbase.blp");
        // …and a path with nothing above it is just itself.
        assert_eq!(leaf("grassbase.blp"), "grassbase.blp");
        assert_eq!(leaf(""), "");
    }

    /// **The catalogue is the archives' list without the gloss masks.** Every
    /// tileset ships beside a `_s` variant holding its specular, and offering
    /// both would be offering each texture twice under names that differ by two
    /// characters.
    #[test]
    fn the_gloss_masks_are_not_in_the_catalogue() {
        let gloss = |path: &str| path.trim_end_matches(".blp").ends_with("_s");
        assert!(gloss("Tileset\\Elwynn\\ElwynnGrass_s.blp"));
        assert!(!gloss("Tileset\\Elwynn\\ElwynnGrass.blp"));
    }
}
