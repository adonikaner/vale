//! What a stroke leaves behind, measured on a real tile.
//!
//! A brush is easy to get right in the middle of a chunk and easy to get wrong
//! on its edge, because the vertex on a chunk boundary exists **twice**: it is
//! row 8 of one chunk and row 0 of the next, stored in two `MCVT` arrays that
//! nothing keeps in step. A stroke that moves one copy and not the other, or
//! moves them by different amounts, tears the ground along the boundary — and
//! nothing about the file is invalid afterwards, so only a picture or a check
//! like this one says so.

use crate::adt::{heights, AdtFile};
use crate::ops::{Brush, Edit, Falloff, Mode, Shape};
use vale_assets::world::adt::{ALPHA_LEN, ALPHA_SIDE};

/// The tile these run on, and where on it.
const TILE: (&str, u32, u32) = ("Azeroth", 32, 48);

fn tile() -> Option<AdtFile> {
    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("Data")
            .to_string_lossy()
            .into_owned()
    });
    let mut assets = vale_assets::Assets::open(&root).ok()?;
    let path = vale_assets::adt_path(TILE.0, TILE.1, TILE.2);
    let bytes = assets.read(&path).ok()?;
    AdtFile::parse(&bytes).ok()
}

/// Every vertex a pair of adjacent chunks share, as `(chunk, index, chunk,
/// index)`.
///
/// Two kinds. Along x, chunk `(ix, iy)`'s row 8 is chunk `(ix, iy + 1)`'s row 0,
/// because `index_y` grows along decreasing world x. Along y, column 8 is the
/// next chunk's column 0.
fn shared() -> Vec<(usize, usize, usize, usize)> {
    let mut pairs = Vec::new();
    for iy in 0..16usize {
        for ix in 0..16usize {
            let here = iy * 16 + ix;
            if iy + 1 < 16 {
                let below = (iy + 1) * 16 + ix;
                for col in 0..9 {
                    pairs.push((
                        here,
                        heights::outer(8, col).unwrap(),
                        below,
                        heights::outer(0, col).unwrap(),
                    ));
                }
            }
            if ix + 1 < 16 {
                let across = iy * 16 + ix + 1;
                for row in 0..9 {
                    pairs.push((
                        here,
                        heights::outer(row, 8).unwrap(),
                        across,
                        heights::outer(row, 0).unwrap(),
                    ));
                }
            }
        }
    }
    pairs
}

/// The largest disagreement between the two copies of a shared vertex.
fn worst_seam(tile: &AdtFile) -> (f32, usize, usize) {
    let fields: Vec<Vec<f32>> = (0..tile.chunks.len())
        .map(|i| heights::heights(tile.chunk(i).unwrap()))
        .collect();
    let mut worst = (0.0f32, 0usize, 0usize);
    for (a, ai, b, bi) in shared() {
        let gap = (fields[a][ai] - fields[b][bi]).abs();
        if gap > worst.0 {
            worst = (gap, a, b);
        }
    }
    worst
}

/// A stroke centred exactly on the boundary between two chunks leaves the two
/// copies of every shared vertex agreeing.
///
/// The tolerance is not zero and cannot be. A chunk's heights are stored
/// relative to its own `position.z`, and two adjacent chunks have different
/// ones, so the same world height is two different `f32` values either side of
/// the seam and the round trip through `+ base` and `- base` rounds differently
/// on each. What that costs is bounded by an ulp of the larger sum, which at
/// Elwynn's heights is about 3e-5 yards.
#[test]
fn a_stroke_on_a_chunk_boundary_leaves_no_seam() {
    let Some(mut tile) = tile() else {
        eprintln!("no archives — the stroke checks did not run");
        return;
    };
    let before = worst_seam(&tile);

    // The corner where chunks 70, 71, 86 and 87 meet, which is a boundary in
    // both directions at once.
    let origin = tile.chunk(70).unwrap().head().position();
    let at = [
        origin[0] - vale_assets::world::adt::CHUNK_SIZE,
        origin[1] - vale_assets::world::adt::CHUNK_SIZE,
    ];

    let brush = Brush {
        radius: 20.0,
        strength: 8.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        scale: 25.0,
        mode: Mode::Raise,
        ..Brush::default()
    };
    for _ in 0..60 {
        brush.stroke(&mut tile, at, 1.0 / 60.0, None);
    }

    let after = worst_seam(&tile);
    assert!(
        after.0 < 1e-3,
        "a stroke on the corner of chunks 70/71/86/87 left the two copies of a \
         shared vertex {} yards apart, between chunks {} and {} (it was {} before)",
        after.0,
        after.1,
        after.2,
        before.0
    );
}

/// **A smooth pulls every chunk under the brush toward one height.**
///
/// The reported fault was *"smooth is broken across tiles"*, and it was broken
/// across chunks too: the target was the mean of the chunk being written, so each
/// of the four chunks under one brush converged somewhere different and the
/// boundary between any two became a step. Worst at a tile seam, where the two
/// sides are in different files and nothing can notice.
///
/// What is measured is the symptom rather than the mechanism: the gap between two
/// neighbouring chunks' own mean heights under the brush. Smoothing should close
/// it, because both are being pulled to the same place. Under the fault it stays
/// where it was, because each chunk is being pulled to its own.
#[test]
fn a_smooth_converges_on_one_height_across_chunks() {
    let Some(mut tile) = tile() else {
        return;
    };
    // The corner where 70, 71, 86 and 87 meet, and a brush wide enough to cover
    // a good part of all four.
    let origin = tile.chunk(70).unwrap().head().position();
    let at = [
        origin[0] - vale_assets::world::adt::CHUNK_SIZE,
        origin[1] - vale_assets::world::adt::CHUNK_SIZE,
    ];
    let brush = Brush {
        radius: 60.0,
        strength: 6.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        scale: 25.0,
        mode: Mode::Smooth,
        ..Brush::default()
    };

    // One chunk's mean height inside the brush, which is what a step between two
    // chunks is a difference of.
    let mean = |tile: &AdtFile, index: usize| -> Option<f32> {
        let chunk = tile.chunk(index)?;
        let origin = chunk.head().position();
        let (mut sum, mut count) = (0.0, 0);
        for (i, h) in heights::heights(chunk).iter().enumerate() {
            let (dx, dy) = heights::vertex_offset(i);
            let (x, y) = (origin[0] - dx, origin[1] - dy);
            if ((x - at[0]).powi(2) + (y - at[1]).powi(2)).sqrt() <= brush.radius {
                sum += h;
                count += 1;
            }
        }
        (count > 0).then(|| sum / count as f32)
    };
    let gap = |tile: &AdtFile| (mean(tile, 70).unwrap() - mean(tile, 71).unwrap()).abs();

    let before = gap(&tile);
    // Enough of a stroke to converge, and the level recomputed each step the way
    // the tool does it: the ground is moving, so what it is moving toward moves
    // with it.
    for _ in 0..120 {
        let (sum, weight) = brush.weighed(&tile, at);
        let level = (weight > 0.0).then(|| (sum / weight) as f32);
        brush.stroke(&mut tile, at, 1.0 / 60.0, level);
    }
    let after = gap(&tile);

    assert!(
        before > 0.5,
        "the fixture's chunks 70 and 71 differ by only {before} yards, which is \
         too flat for this to measure anything"
    );
    assert!(
        after < before * 0.25,
        "smoothing left chunks 70 and 71 {after} yards apart, from {before}: \
         each chunk converged on its own height"
    );
}

/// …and it leaves no spike: no vertex ends up further from its own neighbours
/// than the ground was already varying by.
///
/// A single vertex moved when the ones around it were not is what a hole in the
/// brush's chunk coverage looks like, and it is invisible in a count.
#[test]
fn a_stroke_leaves_no_vertex_stranded() {
    let Some(mut tile) = tile() else {
        return;
    };
    let origin = tile.chunk(70).unwrap().head().position();
    let at = [
        origin[0] - vale_assets::world::adt::CHUNK_SIZE,
        origin[1] - vale_assets::world::adt::CHUNK_SIZE,
    ];
    let brush = Brush {
        radius: 20.0,
        strength: 8.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        scale: 25.0,
        mode: Mode::Raise,
        ..Brush::default()
    };
    let step = |tile: &AdtFile| {
        // The largest gap between an outer vertex and the one next to it, over
        // the four chunks around the stroke.
        let mut worst = 0.0f32;
        for chunk in [70usize, 71, 86, 87] {
            let field = heights::heights(tile.chunk(chunk).unwrap());
            for row in 0..9 {
                for col in 0..8 {
                    let a = field[heights::outer(row, col).unwrap()];
                    let b = field[heights::outer(row, col + 1).unwrap()];
                    worst = worst.max((a - b).abs());
                }
            }
            for row in 0..8 {
                for col in 0..9 {
                    let a = field[heights::outer(row, col).unwrap()];
                    let b = field[heights::outer(row + 1, col).unwrap()];
                    worst = worst.max((a - b).abs());
                }
            }
        }
        worst
    };

    let before = step(&tile);
    for _ in 0..60 {
        brush.stroke(&mut tile, at, 1.0 / 60.0, None);
    }
    let after = step(&tile);
    // A smooth falloff over a 20-yard radius moves neighbouring vertices, 4.17
    // yards apart, by nearly the same amount, so a stroke adds very little to
    // the field's own roughness. Doubling it would mean a vertex was left
    // behind by its neighbours.
    assert!(
        after < before.max(1.0) * 2.0,
        "a stroke took the largest step between neighbouring vertices from \
         {before} to {after} yards"
    );
}

/// A stroke that reaches past the edge of the tile stops there, and says so.
///
/// Recorded rather than asserted away: a brush currently edits one tile, so a
/// stroke on a tile border leaves the neighbour untouched and the ground is
/// discontinuous across it. This is the check that will fail when that is
/// fixed, which is the point of it.
#[test]
fn a_stroke_on_a_tile_border_edits_one_side_of_it() {
    let Some(mut tile) = tile() else {
        return;
    };
    // Chunk 0's own corner is the tile's corner.
    let origin = tile.chunk(0).unwrap().head().position();
    let brush = Brush::default();
    let edits = brush.stroke(&mut tile, [origin[0], origin[1]], 1.0 / 60.0, None);
    let touched: Vec<usize> = edits.iter().filter_map(crate::ops::Edit::chunk).collect();
    assert!(
        !touched.is_empty(),
        "a stroke on the tile's own corner reaches its first chunk"
    );
    assert!(
        touched.iter().all(|&c| c < 256),
        "and reaches nothing outside this tile, which is the limitation"
    );
}

/// **A paint stroke makes its texture win where it lands and changes nothing
/// where it does not.**
///
/// Both halves matter and the second is the one that goes wrong quietly: the
/// blend maps of a chunk are 4,096 texels and a brush whose falloff or whose
/// texel-to-world mapping is off paints the whole chunk, or paints a mirrored
/// copy of itself, and the file is valid either way.
#[test]
fn a_paint_stroke_lands_where_the_pointer_is() {
    use crate::adt::alpha;
    use crate::ops::PaintBrush;
    use vale_assets::world::adt::ALPHA_SIDE;

    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let origin = tile.chunk(which).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];

    let was = alpha::paint(tile.chunk(which).unwrap());
    let brush = PaintBrush {
        radius: 8.0,
        strength: 4.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        texture: "Tileset\\Generic\\Black.blp".into(),
        effect_id: 0,
        ..PaintBrush::default()
    };
    // Four seconds of stroke, a tenth of a second at a time, which is what a
    // held button produces.
    let mut working = crate::ops::Working::default();
    for _ in 0..40 {
        brush.stroke(&mut tile, &mut working, at, 0.1);
    }

    let now = alpha::paint(tile.chunk(which).unwrap());
    assert_eq!(now.len(), was.len() + 1, "the texture became a new layer");
    let painted = now.maps.last().expect("the new layer");

    // Under the pointer it is opaque…
    let centre = ALPHA_SIDE / 2 * ALPHA_SIDE + ALPHA_SIDE / 2;
    assert!(painted[centre] > 240, "at the pointer: {}", painted[centre]);
    // …and in the chunk's far corner, well outside an 8-yard radius, nothing
    // was touched at all.
    assert_eq!(painted[0], 0, "the corner nearest the origin");
    assert_eq!(painted[ALPHA_SIDE - 1], 0, "along one edge");
    assert_eq!(painted[ALPHA_LEN - 1], 0, "the far corner");

    // The layers **under** it are left where they were. They are hidden by an
    // opaque layer over them, so moving them would change nothing visible and
    // would destroy the paint a person sees again the moment they take the top
    // layer off.
    for (i, map) in now.maps.iter().enumerate().take(was.len()).skip(1) {
        assert_eq!(map, &was.maps[i], "layer {i} was under the stroke and moved");
    }

    // …and the other half of the rule: painting with a texture that is **below**
    // an opaque one has to clear what is over it, or the brush does nothing at
    // all on that half of the chunk. The base layer is the extreme case — it can
    // only be revealed.
    let base = tile.texture_names()[was.layers[0].texture_id as usize].clone();
    let scrub = PaintBrush {
        texture: base,
        ..brush
    };
    // **A second stroke gets a second working copy**, which is what `Working`
    // being a stroke's own state means: the first one's texels are the file's
    // now, and carrying them over would paint from where the last stroke was.
    let mut working = crate::ops::Working::default();
    for _ in 0..40 {
        scrub.stroke(&mut tile, &mut working, at, 0.1);
    }
    let now = alpha::paint(tile.chunk(which).unwrap());
    for (i, map) in now.maps.iter().enumerate().skip(1) {
        assert!(
            map[centre] < 16,
            "layer {i} still covers the pointer at {}",
            map[centre]
        );
    }
}

/// **A stroke is undone exactly**, including the layer it added and the texture
/// name it appended.
///
/// The check the whole history rests on for this tool. A paint edit is the only
/// one that changes the length of a region, so it is the only one whose inverse
/// has to put bytes back that have *moved*.
#[test]
fn a_paint_stroke_inverts() {
    use crate::adt::alpha;
    use crate::ops::PaintBrush;

    let Some(tile) = tile() else { return };
    let before = tile.clone();
    let mut tile = tile;
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let origin = tile.chunk(which).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];

    let brush = PaintBrush {
        radius: 8.0,
        strength: 4.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        texture: "Tileset\\Generic\\Black.blp".into(),
        effect_id: 0,
        ..PaintBrush::default()
    };
    // The first step is the one that adds the layer; the rest only move texels.
    let mut working = crate::ops::Working::default();
    let first = brush.stroke(&mut tile, &mut working, at, 0.1);
    assert!(first.full.is_empty(), "the chunk was chosen because it has room");
    assert!(
        first.edits.iter().any(|edit| edit.changes_the_texture_set()),
        "the first step gives the chunk a layer it did not have"
    );
    let mut edits = first.edits;
    edits.extend(brush.stroke(&mut tile, &mut working, at, 0.1).edits);
    assert_ne!(alpha::paint(tile.chunk(which).unwrap()), alpha::paint(before.chunk(which).unwrap()));

    for edit in edits.iter().rev() {
        edit.revert(&mut tile);
    }
    assert_eq!(
        alpha::paint(tile.chunk(which).unwrap()),
        alpha::paint(before.chunk(which).unwrap()),
        "the chunk's paint came back"
    );
    assert_eq!(
        tile.textures, before.textures,
        "…and so did the tile's texture list"
    );
    assert_eq!(tile.write(), before.write(), "the whole tile came back");
}

/// The first chunk of a tile with a layer to spare.
///
/// **Asked rather than assumed.** Most of a shipped tile's chunks already carry
/// the four the client draws, so a test that picked one by index picked one that
/// could take nothing and passed by doing nothing.
fn room_for_a_layer(tile: &AdtFile) -> Option<usize> {
    use crate::adt::alpha;
    (0..tile.chunks.len()).find(|&i| {
        let paint = alpha::paint(tile.chunk(i).expect("a chunk"));
        !paint.is_empty() && paint.len() < alpha::MAX_LAYERS
    })
}

/// **A stack of moves always returns the tile to where it started, even when the
/// caller's idea of the current value is wrong.**
///
/// This is the invariant the whole history rests on and the one that was broken:
/// a change is only reversible if its `before` is the state the file was
/// actually in, and every caller that supplies one is supplying its own cached
/// copy. The test therefore does what a stale caller does — hands
/// `Edit::move_doodad` an `after` computed from a record the file no longer
/// holds — and checks that reverting in order still arrives back at the start.
///
/// Built by hand rather than through the editor because the editor needs a
/// window; what is being checked is the arithmetic of the stack, which is here.
#[test]
fn a_stack_of_moves_returns_to_where_it_started() {
    use crate::ops::Edit;
    use crate::undo::History;
    use crate::TileKey;

    let Some(mut tile) = tile() else { return };
    if tile.doodad_list().is_empty() {
        return;
    }
    let start = tile.doodad_list();
    let key = TileKey::new(TILE.0, TILE.1, TILE.2);
    let mut history = History::new();

    // Ten moves of the first placement, each computed from a **stale** record —
    // the one the tile had two steps ago rather than the one it has now, which
    // is exactly what a panel that missed an undo is holding.
    let mut stale = start[0];
    for step in 0..10 {
        stale.position[0] += 1.0;
        let mut after = stale;
        after.position[1] += step as f32;
        let Some(edit) = Edit::move_doodad(&tile, 0, after) else {
            continue;
        };
        edit.apply(&mut tile);
        history.begin(format!("Move {step}"));
        history.record(&key, [edit]);
        history.end();
        // Whatever the caller thought, the tile now holds `after`.
        assert_eq!(tile.doodad_list()[0], after, "step {step}");
    }
    assert_eq!(history.depth_done(), 10);
    assert_ne!(tile.doodad_list()[0], start[0], "something moved");

    // …and every one of them comes back off, in order, to exactly the start.
    while let Some(change) = history.undo() {
        change.revert(&key, &mut tile);
    }
    assert_eq!(tile.doodad_list(), start, "the tile is where it began");

    // …and redoing them all puts it back where the last move left it.
    let ended = {
        let mut forward = History::new();
        std::mem::swap(&mut forward, &mut history);
        forward
    };
    let mut history = ended;
    let mut last = None;
    while let Some(change) = history.redo() {
        change.apply(&key, &mut tile);
        last = Some(change);
    }
    assert!(last.is_some(), "there was something to redo");
    assert_ne!(tile.doodad_list()[0], start[0]);
}

/// **A move of a placement the tile does not have is not an edit at all.** A
/// caller holding an index into a list that has since been renumbered would
/// otherwise record a change whose inverse writes a record into whatever is at
/// that position now.
#[test]
fn a_move_of_an_index_the_tile_does_not_have_is_declined() {
    use crate::ops::Edit;

    let Some(tile) = tile() else { return };
    let list = tile.doodad_list();
    assert!(Edit::move_doodad(&tile, list.len(), list[0]).is_none());
    // …and a move that moves nothing is not an edit either, which is what keeps
    // a panel redrawn every frame off the stack.
    assert!(Edit::move_doodad(&tile, 0, list[0]).is_none());
}

/// **A chunk that already carries four textures refuses a fifth, and says so.**
///
/// This is the reported symptom — *"you can be painting a texture and suddenly
/// run into a chunk where it just stops"* — and the cause is not a bug in the
/// brush: four is what the terrain shader draws and what 5875 draws. What was
/// wrong is that the refusal was silent, so the tool could not tell anybody.
///
/// The counts are why it matters rather than being a corner case. Measured over
/// the shipped tiles, chunks already at four: **144 of 256** on `Azeroth_32_48`,
/// **194 of 256** on `Azeroth_34_51`, **120 of 256** on `Kalimdor_30_41`. Half
/// to three quarters of the ground.
#[test]
fn a_full_chunk_refuses_a_fifth_texture_and_reports_it() {
    use crate::adt::alpha;
    use crate::ops::PaintBrush;

    let Some(mut tile) = tile() else { return };
    let full = (0..tile.chunks.len())
        .find(|&i| !alpha::paint(tile.chunk(i).expect("a chunk")).has_room());
    let Some(full) = full else {
        return;
    };
    let origin = tile.chunk(full).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let before = alpha::paint(tile.chunk(full).unwrap());

    let brush = PaintBrush {
        radius: 4.0,
        strength: 4.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        texture: "Tileset\\Generic\\Black.blp".into(),
        effect_id: 0,
        ..PaintBrush::default()
    };
    let mut working = crate::ops::Working::default();
    let painted = brush.stroke(&mut tile, &mut working, at, 0.1);
    assert_eq!(painted.full, vec![full], "the full chunk is named");
    assert!(
        painted.edits.is_empty() || painted.edits.iter().all(|e| e.painted() != Some(full)),
        "and nothing was written to it"
    );
    assert_eq!(alpha::paint(tile.chunk(full).unwrap()), before);
    // …and `MTEX` did not grow for a texture no chunk took, which is the other
    // half of refusing properly.
    assert!(
        !tile
            .texture_names()
            .iter()
            .any(|name| name.eq_ignore_ascii_case("Tileset\\Generic\\Black.blp"))
            || painted.edits.iter().any(|e| e.changes_the_texture_set()),
        "a refused texture is not named"
    );

    // Take a layer off and the same stroke lands.
    let mut paint = alpha::paint(tile.chunk(full).unwrap());
    assert!(paint.remove_layer(paint.len() - 1));
    alpha::set_paint(tile.chunk_mut(full).unwrap(), &paint);
    let painted = brush.stroke(&mut tile, &mut crate::ops::Working::default(), at, 0.1);
    assert!(painted.full.is_empty(), "the slot is free now");
    assert!(painted.edits.iter().any(|e| e.painted() == Some(full)));
}

/// **Strength decides how fast a stroke paints**, which it did not.
///
/// The fault it guards against is subtle enough to have shipped: a 4-bit chunk
/// stores sixteen levels, so a step smaller than half a quantum is a step that
/// rounds back to where it started. Snapping *away* from the start fixes that
/// and breaks this — every step becomes a whole quantum, so the weakest possible
/// brush advances a fifteenth of the range per frame and the setting controls
/// nothing. Measured from the window at the time: a quarter of a second to full
/// opacity at any strength.
///
/// So the check is a ratio and not a threshold: a tenth of the strength over the
/// same time has to leave visibly less paint down.
#[test]
fn strength_decides_how_fast_a_stroke_paints() {
    use crate::adt::alpha;
    use crate::ops::{PaintBrush, Working};
    use vale_assets::world::adt::ALPHA_SIDE;

    let Some(base) = tile() else { return };
    let Some(which) = room_for_a_layer(&base) else {
        return;
    };
    let origin = base.chunk(which).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let centre = ALPHA_SIDE / 2 * ALPHA_SIDE + ALPHA_SIDE / 2;

    // Half a second of stroke at sixty frames a second, which is what holding
    // the button down for half a second is.
    let paint_with = |strength: f32| -> u8 {
        let mut tile = base.clone();
        let mut working = Working::default();
        let brush = PaintBrush {
            radius: 8.0,
            strength,
            falloff: Falloff::Smooth,
            shape: Shape::Circle,
            core: 0.0,
            texture: "Tileset\\Generic\\Black.blp".into(),
            effect_id: 0,
            ..PaintBrush::default()
        };
        for _ in 0..30 {
            brush.stroke(&mut tile, &mut working, at, 1.0 / 60.0);
        }
        let now = alpha::paint(tile.chunk(which).unwrap());
        now.maps.last().map(|map| map[centre]).unwrap_or(0)
    };

    let weak = paint_with(0.2);
    let strong = paint_with(6.0);
    assert!(
        strong > weak,
        "strength did nothing: {weak} at 0.2 against {strong} at 6.0"
    );
    assert!(strong > 200, "a strong brush covers in half a second: {strong}");
    assert!(
        weak < 128,
        "a weak one does not: {weak} — and this is the assertion that was false"
    );
}

/// …and a **weak** brush still gets there if it is held long enough, which is
/// the property the snap-away rule existed to protect and which the working copy
/// now provides instead.
#[test]
fn a_weak_brush_still_reaches_opaque_if_it_is_held() {
    use crate::adt::alpha;
    use crate::ops::{PaintBrush, Working};
    use vale_assets::world::adt::ALPHA_SIDE;

    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let origin = tile.chunk(which).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let mut working = Working::default();
    let brush = PaintBrush {
        radius: 8.0,
        strength: 0.2,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        texture: "Tileset\\Generic\\Black.blp".into(),
        effect_id: 0,
        ..PaintBrush::default()
    };
    // Twenty seconds of a very weak brush.
    for _ in 0..1200 {
        brush.stroke(&mut tile, &mut working, at, 1.0 / 60.0);
    }
    let now = alpha::paint(tile.chunk(which).unwrap());
    let centre = ALPHA_SIDE / 2 * ALPHA_SIDE + ALPHA_SIDE / 2;
    let value = now.maps.last().map(|map| map[centre]).unwrap_or(0);
    assert!(value > 240, "held long enough it reaches opaque: {value}");
}

/// **A chunk with no textures at all takes the first one as its base.**
///
/// Reported as "tiles with no textures don't seem to be able to add one", and
/// it was an early return: a chunk with no `MCLY` answered `Nothing` and could
/// never be given anything. Not a corner case — `development` has **543** such
/// chunks and `development 2,2` is 256 of 256.
///
/// The base is the one layer with no blend map, so there is no partial version
/// of it: the whole chunk becomes the texture however little of it the brush
/// covered, and [`Painted::based`] is what says so.
#[test]
fn a_chunk_with_no_textures_takes_the_first_one_as_its_base() {
    use crate::adt::alpha;
    use crate::adt::AdtFile;
    use crate::ops::{PaintBrush, Working};

    // A tile from the map that has them. Skipped on a machine with no archives,
    // like every other check here.
    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("Data")
            .to_string_lossy()
            .into_owned()
    });
    let Ok(mut assets) = vale_assets::Assets::open(&root) else {
        return;
    };
    let Ok(bytes) = assets.read(&vale_assets::adt_path("development", 2, 2)) else {
        return;
    };
    let mut tile = AdtFile::parse(&bytes).expect("a shipped tile parses");
    let blank = (0..tile.chunks.len())
        .find(|&i| alpha::paint(tile.chunk(i).expect("a chunk")).is_empty());
    let Some(blank) = blank else {
        return;
    };

    let origin = tile.chunk(blank).unwrap().head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let brush = PaintBrush {
        radius: 8.0,
        strength: 6.0,
        falloff: Falloff::Smooth,
        shape: Shape::Circle,
        core: 0.0,
        texture: "Tileset\\Generic\\Black.blp".into(),
        effect_id: 0,
        ..PaintBrush::default()
    };
    let painted = brush.stroke(&mut tile, &mut Working::default(), at, 0.1);
    assert_eq!(painted.based, vec![blank], "the chunk is reported as based");
    assert_eq!(painted.edits.len(), 1);
    assert!(
        painted.edits[0].changes_the_texture_set(),
        "a base is a texture set change, so the tile is read again"
    );

    let now = alpha::paint(tile.chunk(blank).unwrap());
    assert_eq!(now.len(), 1, "one layer, and it is the base");
    assert_eq!(
        tile.texture_names()[now.layers[0].texture_id as usize],
        "Tileset\\Generic\\Black.blp"
    );
    // The base has no blend map in the file — that is what being the base means.
    assert!(tile
        .chunk(blank)
        .and_then(|c| c.region(crate::adt::Region::Alpha))
        .is_some_and(|sub| sub.data.is_empty()));
    assert_eq!(tile.chunk(blank).unwrap().head().layer_count(), 1);

    // …and a second texture now blends over it in the ordinary way.
    let over = PaintBrush {
        texture: "Tileset\\Generic\\White.blp".into(),
        ..brush
    };
    let mut working = Working::default();
    for _ in 0..20 {
        over.stroke(&mut tile, &mut working, at, 0.1);
    }
    let now = alpha::paint(tile.chunk(blank).unwrap());
    assert_eq!(now.len(), 2, "the second one is a layer over the base");
    assert!(now.maps[1][ALPHA_SIDE / 2 * ALPHA_SIDE + ALPHA_SIDE / 2] > 200);
}


/// **The chunk under the pointer is always taken, and a small radius takes only
/// it.**
///
/// The whole of [`AreaBrush::covers`]' contract, and both halves are the kind of
/// thing that is quietly wrong. A rule of "the circle must overlap the square"
/// makes a click near a chunk corner do nothing, which reads as a brush that
/// ignores you; a rule of "centres within the radius" alone does the same at the
/// far corner of a chunk. The answer is both, and this says so.
#[test]
fn an_area_brush_always_takes_the_chunk_under_the_pointer() {
    use vale_assets::world::adt::CHUNK_SIZE;
    use crate::ops::AreaBrush;

    let Some(tile) = tile() else { return };
    {
        let path = TILE.0;
        let origin = tile.chunk(70).expect("chunk 70").head().position();
        // Right at the chunk's far corner, which is the worst case for a
        // centres-only rule: the centre is most of a chunk away.
        let corner = [origin[0] - CHUNK_SIZE + 0.1, origin[1] - CHUNK_SIZE + 0.1];

        let tight = AreaBrush { radius: 1.0, area: 5 };
        assert_eq!(
            tight.covers(&tile, corner),
            vec![70],
            "{path}: a tight brush at a corner is exactly the chunk under it"
        );

        // …and a radius that reaches the neighbours takes them as well, with the
        // chunk under the pointer still in the list.
        let wide = AreaBrush { radius: CHUNK_SIZE * 1.5, area: 5 };
        let covered = wide.covers(&tile, corner);
        assert!(covered.contains(&70), "{path}");
        assert!(covered.len() > 1, "{path}: a wide brush took only one chunk");

        // **A chunk already carrying the area yields no edit**, which is what
        // keeps a held stroke from one entry on the history per frame.
        let mut painted = tile.clone();
        let brush = AreaBrush { radius: 1.0, area: 5 };
        assert_eq!(brush.stroke(&mut painted, corner).len(), 1, "{path}");
        assert!(
            brush.stroke(&mut painted, corner).is_empty(),
            "{path}: painting the same area twice made a second edit"
        );
    }
}

/// **A grade leaves the ground at the height it asked for, in world terms.**
///
/// The one thing that could go wrong here and did: `heights::heights` adds the
/// chunk's own `position.z` and `set_heights` takes it off again, so a caller
/// that subtracts it as well writes `wanted — z` where `wanted` belongs. The
/// file is valid afterwards and every chunk is a different amount wrong, because
/// `position.z` differs from chunk to chunk — which on screen is ground driven
/// hundreds of yards below where it was, coming apart along every boundary.
///
/// So the assertion is against the height the grade *asked* for and not against
/// the height before it, which is the only form of it that fails.
#[test]
fn a_grade_leaves_the_height_it_asked_for() {
    use crate::ops::grade::{End, Grade};

    let Some(mut tile) = tile() else {
        return;
    };
    // A level line across the middle of the tile, at a height well away from
    // anything the ground already has — so an answer that is off by a base is
    // off by hundreds of yards rather than by a plausible amount.
    let origin = tile.chunk(0).unwrap().head().position();
    let mid = [origin[0] - 260.0, origin[1] - 260.0];
    let level = 500.0;
    let grade = Grade {
        from: End {
            at: [mid[0] - 60.0, mid[1]],
            height: level,
        },
        to: End {
            at: [mid[0] + 60.0, mid[1]],
            height: level,
        },
        width: 20.0,
        falloff: 0.0,
        strength: 1.0,
    };
    let edits = grade.write_into(&mut tile);
    assert!(
        !edits.is_empty(),
        "a grade across the middle of a tile moves something"
    );

    let mut checked = 0usize;
    for index in 0..256 {
        let Some(chunk) = tile.chunk(index) else {
            continue;
        };
        let chunk_origin = chunk.head().position();
        for (i, height) in heights::heights(chunk).iter().enumerate() {
            let (dx, dy) = heights::vertex_offset(i);
            let (x, y) = (chunk_origin[0] - dx, chunk_origin[1] - dy);
            let Some((wanted, weight)) = grade.at(x, y) else {
                continue;
            };
            if weight < 0.999 {
                continue;
            }
            assert!(
                (height - wanted).abs() < 0.05,
                "chunk {index} vertex {i} is at {height}, not {wanted}",
            );
            checked += 1;
        }
    }
    assert!(checked > 100, "only {checked} vertices were under the grade");
}

/// **A grade records the shading it changed**, so an undo puts back the ground
/// *and* the light on it.
///
/// `heights::recompute_normals` rewrites `MCNR`, and a caller that runs it
/// without recording the result leaves an undo that restores the shape and not
/// the shading: flat ground lit as though the ramp were still cut into it. That
/// is what it was reported as.
#[test]
fn a_grade_records_the_shading_it_changed() {
    use crate::ops::grade::{End, Grade};
    use crate::ops::Edit;

    let Some(mut tile) = tile() else {
        return;
    };
    let before = tile.write();
    let origin = tile.chunk(0).unwrap().head().position();
    let mid = [origin[0] - 260.0, origin[1] - 260.0];
    let grade = Grade {
        from: End {
            at: [mid[0] - 60.0, mid[1]],
            height: 400.0,
        },
        to: End {
            at: [mid[0] + 60.0, mid[1]],
            height: 460.0,
        },
        width: 20.0,
        falloff: 10.0,
        strength: 1.0,
    };
    let edits = grade.write_into(&mut tile);
    assert!(
        edits.iter().any(|e| matches!(e, Edit::Normals { .. })),
        "a ramp changes the shading of the ground it is cut into"
    );

    // ...and the stack returns the file to where it started, byte for byte.
    for edit in edits.iter().rev() {
        edit.revert(&mut tile);
    }
    assert_eq!(
        tile.write(),
        before,
        "an undone grade writes back byte for byte"
    );
}

/// **A map-wide rename writes back byte for byte when it is undone**, on a real
/// tile with a real `MTEX`.
///
/// The property that matters is not that the name changed — that is
/// `swap_in_place` and it is checked where it lives — but that
/// [`Edit::Names`]'s two directions are exact. A rename to a *longer* path
/// moves every offset in `MMID` after it and grows the file, so an inverse that
/// only put the blob back would leave the offsets pointing into the middle of
/// names. This is what says the pair is the whole of it.
#[test]
fn a_renamed_path_is_undone_byte_for_byte() {
    let Some(mut tile) = tile() else {
        eprintln!("no archives; skipping");
        return;
    };
    let before_bytes = tile.write();
    let Some(first) = tile.texture_names().first().cloned() else {
        eprintln!("the tile names no texture; skipping");
        return;
    };

    let before = crate::ops::TileNames::capture(&tile);
    // Deliberately longer than what it replaces, so the blob grows.
    let moved = tile.rename_texture(&first, r"Tileset\Invented\AVeryMuchLongerName.blp");
    assert_eq!(moved, 1, "the tile's first texture is named once");
    let after = crate::ops::TileNames::capture(&tile);
    assert_ne!(before, after);
    assert!(
        tile.texture_names()
            .iter()
            .any(|name| name == r"Tileset\Invented\AVeryMuchLongerName.blp"),
        "the new name is in MTEX"
    );
    assert_ne!(tile.write(), before_bytes, "and the file moved");

    let edit = Edit::Names {
        before: Box::new(before),
        after: Box::new(after),
    };
    edit.revert(&mut tile);
    assert_eq!(
        tile.write(),
        before_bytes,
        "an undone rename writes back byte for byte"
    );
    edit.apply(&mut tile);
    assert_eq!(
        tile.texture_names().first().map(String::as_str),
        Some(r"Tileset\Invented\AVeryMuchLongerName.blp"),
        "and a redo puts it back"
    );

    // **The tile has to be re-read rather than patched**, which is the fork
    // that decides whether a renamed texture ever reaches the screen: what a
    // path names is baked into the draw group's material, and no atlas write
    // reaches it.
    assert!(edit.remeshes());
    assert_eq!(edit.chunk(), None);
}

/// A brush for the tests below: eight yards, strong, on `texture`.
fn paint_brush(texture: &str) -> crate::ops::PaintBrush {
    crate::ops::PaintBrush {
        radius: 8.0,
        strength: 4.0,
        texture: texture.into(),
        ..crate::ops::PaintBrush::default()
    }
}

/// Hold `brush` at the middle of chunk `which` for `seconds`.
fn hold(
    tile: &mut AdtFile,
    brush: &crate::ops::PaintBrush,
    which: usize,
    seconds: f32,
) -> crate::ops::Painted {
    let origin = tile.chunk(which).expect("a chunk").head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let mut working = crate::ops::Working::default();
    let mut all = crate::ops::Painted::default();
    for _ in 0..(seconds * 10.0) as usize {
        let step = brush.stroke(tile, &mut working, at, 0.1);
        all.edits.extend(step.edits);
        all.full.extend(step.full);
        all.base.extend(step.base);
        all.absent.extend(step.absent);
        all.reused.extend(step.reused);
    }
    all
}

/// The texel in the middle of a chunk's blend map, which is under the brush
/// [`hold`] holds.
fn centre_of(tile: &AdtFile, which: usize, layer: usize) -> u8 {
    use vale_assets::world::adt::ALPHA_SIDE;
    let paint = crate::adt::alpha::paint(tile.chunk(which).unwrap());
    paint.maps[layer][ALPHA_SIDE / 2 * ALPHA_SIDE + ALPHA_SIDE / 2]
}

/// A held stroke converges on the opacity asked for, from below and from
/// above.
#[test]
fn a_stroke_converges_on_its_opacity_from_either_side() {
    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let half = crate::ops::PaintBrush {
        opacity: 0.5,
        ..paint_brush("Tileset\\Generic\\Black.blp")
    };
    hold(&mut tile, &half, which, 6.0);
    let top = crate::adt::alpha::paint(tile.chunk(which).unwrap()).len() - 1;
    let from_below = centre_of(&tile, which, top);
    assert!((110..=145).contains(&from_below), "from below: {from_below}");

    hold(&mut tile, &paint_brush("Tileset\\Generic\\Black.blp"), which, 6.0);
    assert!(centre_of(&tile, which, top) > 230, "a full stroke is opaque");
    hold(&mut tile, &half, which, 6.0);
    let from_above = centre_of(&tile, which, top);
    assert!((110..=145).contains(&from_above), "from above: {from_above}");
}

/// An eraser takes the texture's own layer toward transparent and adds
/// nothing: a chunk that does not carry the texture is not changed, and the
/// layer stays on a chunk that does.
#[test]
fn an_eraser_takes_a_texture_away_and_never_adds_one() {
    use crate::adt::alpha;
    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let was = alpha::paint(tile.chunk(which).unwrap());
    let eraser = crate::ops::PaintBrush {
        erase: true,
        ..paint_brush("Tileset\\Generic\\Black.blp")
    };
    let nothing = hold(&mut tile, &eraser, which, 2.0);
    assert!(nothing.edits.is_empty(), "the chunk does not carry it");
    assert_eq!(alpha::paint(tile.chunk(which).unwrap()), was);

    hold(&mut tile, &paint_brush("Tileset\\Generic\\Black.blp"), which, 4.0);
    let top = alpha::paint(tile.chunk(which).unwrap()).len() - 1;
    assert!(centre_of(&tile, which, top) > 230);
    hold(&mut tile, &eraser, which, 6.0);
    assert!(centre_of(&tile, which, top) < 20, "erased under the brush");
    assert_eq!(alpha::paint(tile.chunk(which).unwrap()).len(), was.len() + 1);

    // The base has no blend map, and an eraser on it says so.
    let base = tile.texture_names()[was.layers[0].texture_id as usize].clone();
    let on_base = crate::ops::PaintBrush {
        erase: true,
        ..paint_brush(&base)
    };
    let refused = hold(&mut tile, &on_base, which, 0.5);
    assert!(refused.edits.is_empty());
    assert!(refused.base.contains(&which));
}

/// A stroke confined to existing layers leaves a chunk without the texture
/// alone, and says which it was.
#[test]
fn a_stroke_confined_to_existing_layers_adds_none() {
    use crate::adt::alpha;
    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let was = alpha::paint(tile.chunk(which).unwrap());
    let confined = crate::ops::PaintBrush {
        existing_only: true,
        ..paint_brush("Tileset\\Generic\\Black.blp")
    };
    let done = hold(&mut tile, &confined, which, 2.0);
    assert!(done.absent.contains(&which));
    assert_eq!(alpha::paint(tile.chunk(which).unwrap()), was);
}

/// At a density under 1 a held stroke paints some of the texels under it and
/// leaves others bare, and the value that decides is the place's own.
#[test]
fn a_sparse_stroke_paints_patches() {
    use crate::adt::alpha;
    use vale_assets::world::adt::ALPHA_SIDE;
    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    let sparse = crate::ops::PaintBrush {
        density: 0.5,
        falloff: Falloff::Flat,
        ..paint_brush("Tileset\\Generic\\Black.blp")
    };
    hold(&mut tile, &sparse, which, 6.0);
    let paint = alpha::paint(tile.chunk(which).unwrap());
    let map = paint.maps.last().unwrap();
    // The 16x16 texels round the middle are well inside an eight-yard brush.
    let (mut painted, mut bare) = (0, 0);
    for ty in ALPHA_SIDE / 2 - 8..ALPHA_SIDE / 2 + 8 {
        for tx in ALPHA_SIDE / 2 - 8..ALPHA_SIDE / 2 + 8 {
            match map[ty * ALPHA_SIDE + tx] {
                200.. => painted += 1,
                0..=40 => bare += 1,
                _ => {}
            }
        }
    }
    assert!(painted > 16, "some of it is painted: {painted}");
    assert!(bare > 16, "and some of it is not: {bare}");
    assert_eq!(crate::ops::speckle(1.0, 2.0, 2.0), crate::ops::speckle(1.0, 2.0, 2.0));
    for step in 0..50 {
        let value = crate::ops::speckle(step as f32 * 0.52, 3.0, 2.0);
        assert!((0.0..=1.0).contains(&value));
    }
}

/// A full chunk refuses a fifth texture, and with the switch on gives it the
/// layer that shows almost nothing.
#[test]
fn a_full_chunk_gives_up_a_layer_that_shows_nothing() {
    use crate::adt::alpha;
    let Some(mut tile) = tile() else { return };
    let Some(which) = room_for_a_layer(&tile) else {
        return;
    };
    // Fill the chunk with one-yard dots, each a layer showing about 0.3%.
    let dots = [
        "Tileset\\Generic\\Black.blp",
        "Tileset\\Generic\\Red.blp",
        "Tileset\\Generic\\Grey.blp",
    ];
    for dot in dots {
        if !alpha::paint(tile.chunk(which).unwrap()).has_room() {
            break;
        }
        let brush = crate::ops::PaintBrush {
            radius: 1.0,
            ..paint_brush(dot)
        };
        hold(&mut tile, &brush, which, 2.0);
    }
    let full = alpha::paint(tile.chunk(which).unwrap());
    assert_eq!(full.len(), alpha::MAX_LAYERS);

    let fifth = paint_brush("Tileset\\Generic\\Fifth.blp");
    let refused = hold(&mut tile, &fifth, which, 0.5);
    assert!(refused.full.contains(&which) && refused.edits.is_empty());

    let reusing = crate::ops::PaintBrush {
        reuse_hidden: true,
        ..fifth
    };
    let done = hold(&mut tile, &reusing, which, 4.0);
    assert!(done.reused.contains(&which));
    let now = alpha::paint(tile.chunk(which).unwrap());
    assert_eq!(now.len(), alpha::MAX_LAYERS, "no layer was added");
    let names = tile.texture_names();
    let layer = now
        .layers
        .iter()
        .position(|layer| names[layer.texture_id as usize].ends_with("Fifth.blp"))
        .expect("the texture took a layer");
    assert!(layer > 0, "the base is never the one given up");
    assert!(centre_of(&tile, which, layer) > 200, "and it was painted");
}

/// A doodad over ground a stroke raised is raised by as much as the ground
/// under it, one over ground it did not touch is not moved, and undoing the
/// edits puts the tile back.
#[test]
fn a_placement_follows_the_ground_under_it() {
    use crate::adt::heights;
    use crate::ops::follow;
    use vale_assets::world::adt::placement_to_world;

    let Some(mut tile) = tile() else { return };
    let before_bytes = tile.write();
    let doodads = tile.doodad_list();
    // A doodad whose origin is over this tile's own ground.
    let Some((index, doodad, ground)) = doodads.iter().enumerate().find_map(|(index, doodad)| {
        let [x, y, _] = placement_to_world(doodad.position);
        Some((index, *doodad, heights::height_at(&tile, x, y)?))
    }) else {
        return;
    };
    let [x, y, _] = placement_to_world(doodad.position);
    let was = follow::standing(&tile);
    assert!(was.doodads.iter().any(|(id, _)| *id == doodad.unique_id));

    let brush = Brush {
        radius: 12.0,
        strength: 10.0,
        mode: Mode::Raise,
        ..Brush::default()
    };
    let mut edits = Vec::new();
    for _ in 0..10 {
        edits.extend(brush.stroke(&mut tile, [x, y], 0.1, None));
    }
    let raised = heights::height_at(&tile, x, y).expect("still ground") - ground;
    assert!(raised > 1.0, "the ground rose: {raised}");
    assert_eq!(tile.doodad_at(index).unwrap().position, doodad.position, "not yet moved");

    let done = follow::follow(&mut tile, &was);
    assert!(done.doodads.contains(&index));
    let now = tile.doodad_at(index).unwrap();
    assert!(
        (now.position[1] - doodad.position[1] - raised).abs() < 1e-3,
        "the doodad rose {} and the ground {raised}",
        now.position[1] - doodad.position[1]
    );
    assert_eq!(now.position[0], doodad.position[0]);
    assert_eq!(now.position[2], doodad.position[2]);
    // Far from the stroke nothing moved: every moved doodad is near it.
    for &moved in &done.doodads {
        let [mx, my, _] = placement_to_world(doodads[moved].position);
        assert!((mx - x).hypot(my - y) < 60.0, "a far doodad moved");
    }
    // Against a reading taken now, nothing is left to move.
    let settled = follow::standing(&tile);
    assert!(follow::follow(&mut tile, &settled).edits.is_empty());

    // Everything inverts, in reverse order.
    for edit in done.edits.iter().rev().chain(edits.iter().rev()) {
        edit.revert(&mut tile);
    }
    assert_eq!(tile.write(), before_bytes);
}

/// A height map read straight back changes nothing; one with a pixel moved
/// moves that vertex in every chunk that has it, and undoing puts the tile
/// back.
#[test]
fn a_height_map_reads_back_as_the_tile_it_came_from() {
    use crate::adt::heights;
    use crate::ops::image::{self, HEIGHT_SIDE};

    let Some(mut tile) = tile() else { return };
    let bytes = tile.write();
    let map = image::export_heights(&tile).expect("a whole tile");
    assert_eq!(map.pixels.len(), HEIGHT_SIDE * HEIGHT_SIDE);
    assert!(map.high > map.low);
    assert!(image::import_heights(&mut tile, &map).is_empty(), "nothing to change");
    assert_eq!(tile.write(), bytes);

    // The pixel at column 16, row 16 is the corner four chunks share.
    let mut raised = map.clone();
    let at = 16 * HEIGHT_SIDE + 16;
    raised.pixels[at] = raised.pixels[at].saturating_add(2000).max(2000);
    let edits = image::import_heights(&mut tile, &raised);
    let moved: Vec<usize> = edits
        .iter()
        .filter_map(|edit| match edit {
            Edit::Heights { chunk, .. } => Some(*chunk),
            _ => None,
        })
        .collect();
    assert_eq!(moved, vec![0, 1, 16, 17], "the four chunks at that corner");
    let corner = |chunk: usize, row: usize, column: usize| {
        heights::heights(tile.chunk(chunk).unwrap())[heights::outer(row, column).unwrap()]
    };
    let height = corner(0, 8, 8);
    assert_eq!(height, corner(1, 8, 0));
    assert_eq!(height, corner(16, 0, 8));
    assert_eq!(height, corner(17, 0, 0));
    assert!(edits.iter().any(|edit| matches!(edit, Edit::Normals { .. })));
    // What was written reads back as the picture that wrote it.
    let again = image::export_heights(&tile).unwrap();
    assert!(image::import_heights(&mut tile, &again).is_empty());

    for edit in edits.iter().rev() {
        edit.revert(&mut tile);
    }
    assert_eq!(tile.write(), bytes);
}

/// A blend map read straight back changes nothing, and a channel written
/// lands on the layer it stands for and on no chunk without that layer.
#[test]
fn a_blend_map_reads_back_as_the_tile_it_came_from() {
    use crate::adt::alpha;
    use crate::ops::image::{self, BLEND_SIDE};
    use vale_assets::world::adt::ALPHA_SIDE;

    let Some(mut tile) = tile() else { return };
    let bytes = tile.write();
    let rgb = image::export_blend(&tile);
    assert_eq!(rgb.len(), BLEND_SIDE * BLEND_SIDE * 3);
    assert!(image::import_blend(&mut tile, &rgb).is_empty(), "nothing to change");
    assert_eq!(tile.write(), bytes);

    // Clear the red channel of one chunk that has a second layer.
    let Some(which) = (0..256).find(|&index| {
        let paint = alpha::paint(tile.chunk(index).unwrap());
        paint.len() >= 2 && paint.maps[1].iter().any(|&texel| texel > 0)
    }) else {
        return;
    };
    let mut cleared = rgb.clone();
    let (cx, cy) = (which % 16, which / 16);
    for ty in 0..ALPHA_SIDE {
        for tx in 0..ALPHA_SIDE {
            let at = (cy * ALPHA_SIDE + ty) * BLEND_SIDE + cx * ALPHA_SIDE + tx;
            cleared[at * 3] = 0;
        }
    }
    let edits = image::import_blend(&mut tile, &cleared);
    assert_eq!(edits.len(), 1);
    assert!(matches!(&edits[0], Edit::Paint { chunk, .. } if *chunk == which));
    let now = alpha::paint(tile.chunk(which).unwrap());
    assert!(now.maps[1].iter().all(|&texel| texel == 0));
    for edit in edits.iter().rev() {
        edit.revert(&mut tile);
    }
    assert_eq!(tile.write(), bytes);
}

/// A flatten that may only fill raises what is under the target and leaves
/// what is over it, and one that may only cut does the reverse.
#[test]
fn a_flatten_can_be_told_to_fill_or_to_cut() {
    use crate::adt::heights;
    use crate::ops::Only;
    let Some(tile) = tile() else { return };
    let origin = tile.chunk(70).unwrap().head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let before = heights::heights(tile.chunk(70).unwrap());
    let (low, high) = before.iter().fold((f32::MAX, f32::MIN), |(low, high), &h| {
        (low.min(h), high.max(h))
    });
    let middle = (low + high) / 2.0;
    let run = |only: Only| {
        let mut tile = tile.clone();
        let brush = Brush {
            radius: 60.0,
            strength: 50.0,
            falloff: Falloff::Flat,
            mode: Mode::Flatten { to: middle },
            only,
            ..Brush::default()
        };
        for _ in 0..30 {
            brush.stroke(&mut tile, at, 0.1, None);
        }
        heights::heights(tile.chunk(70).unwrap())
    };
    let filled = run(Only::Fill);
    let cut = run(Only::Cut);
    for (i, &was) in before.iter().enumerate() {
        assert!(filled[i] >= was - 1e-3, "a fill lowered vertex {i}");
        assert!(cut[i] <= was + 1e-3, "a cut raised vertex {i}");
        if was > middle + 0.01 {
            assert_eq!(filled[i], was, "a fill left what is over the target");
        }
        if was < middle - 0.01 {
            assert_eq!(cut[i], was, "a cut left what is under the target");
        }
    }
    assert!(filled.iter().all(|&h| h >= middle - 0.05), "the hollows are filled");
    assert!(cut.iter().all(|&h| h <= middle + 0.05), "the rises are cut");
}

/// A tilted flatten converges on a plane through the pivot: level across the
/// bearing and rising along it by the tangent of the angle.
#[test]
fn a_tilted_flatten_makes_a_ramp_through_its_pivot() {
    use crate::adt::heights;
    let Some(mut tile) = tile() else { return };
    let origin = tile.chunk(70).unwrap().head().position();
    let at = [origin[0] - 16.0, origin[1] - 16.0];
    let to = origin[2];
    // Thirty degrees, rising to the north, which is world +x.
    let tilt = Brush::tilted(30.0, 0.0);
    assert!((tilt[0] - 30f32.to_radians().tan()).abs() < 1e-6);
    assert!(tilt[1].abs() < 1e-6);
    let east = Brush::tilted(45.0, 90.0);
    assert!(east[0].abs() < 1e-6 && (east[1] + 1.0).abs() < 1e-6, "east is -y");

    let brush = Brush {
        radius: 14.0,
        strength: 50.0,
        falloff: Falloff::Flat,
        mode: Mode::Flatten { to },
        tilt,
        pivot: at,
        ..Brush::default()
    };
    for _ in 0..40 {
        brush.stroke(&mut tile, at, 0.1, None);
    }
    for (x, y) in [(at[0], at[1]), (at[0] + 8.0, at[1]), (at[0] - 8.0, at[1] + 6.0)] {
        let expected = to + tilt[0] * (x - at[0]) + tilt[1] * (y - at[1]);
        let found = heights::height_at(&tile, x, y).expect("ground");
        assert!((found - expected).abs() < 0.2, "at {x},{y}: {found} against {expected}");
    }
}

/// A selection is marked by position, moves as one, levels to one height,
/// and a brush told to protect it leaves it where it is.
#[test]
fn a_selection_of_vertices_moves_together_and_can_be_kept_from_a_brush() {
    use crate::adt::heights;
    use crate::ops::vertices::{Mask, Selected};
    let Some(mut tile) = tile() else { return };
    let bytes = tile.write();
    // The corner four chunks share: chunk 0's last outer vertex.
    let origin = tile.chunk(17).unwrap().head().position();
    let at = [origin[0], origin[1]];
    let mut selected = Selected::default();
    assert_eq!(selected.mark(&tile, at, 1.0, Shape::Circle, true), 4, "one vertex, in four chunks");
    assert!(selected.holds(0, heights::outer(8, 8).unwrap()));
    assert!(selected.holds(17, heights::outer(0, 0).unwrap()));
    assert_eq!(selected.count(), 4);
    assert_eq!(selected.positions(&tile).len(), 4);

    let (sum, count) = selected.weighed(&tile);
    let was = (sum[2] / count as f64) as f32;
    let edits = selected.shift(&mut tile, 5.0);
    let corner = |tile: &AdtFile, chunk: usize, row: usize, column: usize| {
        heights::heights(tile.chunk(chunk).unwrap())[heights::outer(row, column).unwrap()]
    };
    assert!((corner(&tile, 0, 8, 8) - was - 5.0).abs() < 1e-4);
    assert_eq!(corner(&tile, 0, 8, 8), corner(&tile, 17, 0, 0), "still welded");
    assert!(edits.iter().any(|edit| matches!(edit, Edit::Normals { .. })));

    // A brush that keeps the selection moves the ground round it and not it.
    let held = corner(&tile, 0, 8, 8);
    let beside = corner(&tile, 17, 1, 1);
    let brush = Brush {
        radius: 20.0,
        strength: 10.0,
        ..Brush::default()
    };
    let mut more = Vec::new();
    for _ in 0..10 {
        more.extend(brush.stroke_masked(&mut tile, at, 0.1, None, Some((&selected, Mask::Protect))));
    }
    assert_eq!(corner(&tile, 0, 8, 8), held, "the kept vertex did not move");
    assert_eq!(corner(&tile, 17, 0, 0), held);
    assert!(corner(&tile, 17, 1, 1) > beside + 1.0, "the ground beside it rose");

    // Levelling puts every entry at one height.
    let wide = {
        let mut wide = Selected::default();
        wide.mark(&tile, at, 12.0, Shape::Square, true);
        wide
    };
    let levelled = wide.level(&mut tile, 40.0);
    assert!(wide.positions(&tile).iter().all(|position| position[2] == 40.0));

    // Taking vertices out empties the set, and everything undoes.
    let mut gone = wide.clone();
    gone.mark(&tile, at, 30.0, Shape::Square, false);
    assert!(gone.is_empty());
    for edit in levelled.iter().rev().chain(more.iter().rev()).chain(edits.iter().rev()) {
        edit.revert(&mut tile);
    }
    assert_eq!(tile.write(), bytes);
}

/// A selection tilts onto a plane through its centre, smooths toward its
/// neighbours, and a brush confined to it moves nothing outside it.
#[test]
fn a_selection_tilts_smooths_and_confines_a_brush() {
    use crate::adt::heights;
    use crate::ops::vertices::{plane, Mask, Selected};
    let Some(mut tile) = tile() else { return };
    let bytes = tile.write();
    let origin = tile.chunk(17).unwrap().head().position();
    let at = [origin[0], origin[1]];
    let mut selected = Selected::default();
    selected.mark(&tile, at, 15.0, Shape::Circle, true);

    // Ten degrees, rising north: every vertex on the plane, and the centre
    // where it was.
    let (sum, count) = selected.weighed(&tile);
    let centre = sum.map(|axis| (axis / count as f64) as f32);
    let slope = Brush::tilted(10.0, 0.0);
    assert!(slope[0] > 0.17 && slope[0] < 0.18 && slope[1].abs() < 1e-6);
    let tilted = selected.tilt(&mut tile, centre, slope);
    for position in selected.positions(&tile) {
        let wanted = plane(centre, slope, position);
        assert!((position[2] - wanted).abs() < 1e-3, "{position:?} against {wanted}");
    }
    let (after, _) = selected.weighed(&tile);
    assert!(((after[2] / count as f64) as f32 - centre[2]).abs() < 1e-2, "mean kept");

    // A spike in the middle, smoothed: it comes down toward its neighbours.
    let mut spike = Selected::default();
    spike.mark(&tile, at, 1.0, Shape::Circle, true);
    let raised = spike.shift(&mut tile, 30.0);
    let peak = spike.positions(&tile)[0][2];
    let plan = spike.smoothed(&tile, 1.0, |x, y| heights::height_at(&tile, x, y));
    let smoothed = plan.write(&mut tile);
    let now = spike.positions(&tile)[0][2];
    assert!(now < peak - 20.0, "{peak} to {now}");
    let welded: Vec<f32> = spike.positions(&tile).iter().map(|p| p[2]).collect();
    // A shared vertex's copies are placed from different chunk origins, so
    // they sample the ground a rounding apart.
    assert!(welded.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 1e-3), "still welded: {welded:?}");

    // A brush confined to the selection raises it and nothing round it.
    let outside = |tile: &AdtFile| heights::heights(tile.chunk(17).unwrap())[heights::outer(4, 4).unwrap()];
    let left = outside(&tile);
    let (inside_was, _) = selected.weighed(&tile);
    let brush = Brush {
        radius: 40.0,
        strength: 10.0,
        ..Brush::default()
    };
    let mut confined = Vec::new();
    for _ in 0..5 {
        confined.extend(brush.stroke_masked(&mut tile, at, 0.1, None, Some((&selected, Mask::Confine))));
    }
    assert_eq!(outside(&tile), left, "outside the selection, nothing moved");
    let (inside_now, _) = selected.weighed(&tile);
    assert!(inside_now[2] > inside_was[2] + 1.0, "the selection rose");

    for edit in confined
        .iter()
        .rev()
        .chain(smoothed.iter().rev())
        .chain(raised.iter().rev())
        .chain(tilted.iter().rev())
    {
        edit.revert(&mut tile);
    }
    assert_eq!(tile.write(), bytes);
}
