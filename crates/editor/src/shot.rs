//! `--shot <file>`: a picture of the editor, and then quit.
//!
//! ## A layout nobody can photograph is a layout nobody checks
//!
//! Every other decision in this crate has an instrument. The brush has
//! `vale_edit`'s tests, the playtest has `--playtest`, the tile writer has a
//! byte-for-byte round trip. The **panels** had nothing: the only way to see
//! whether a rail was the right width, whether a label column lined up, or
//! whether a heading's rule ran off the edge of a panel was to open the window
//! and look, which means the person who can check it is the person sitting in
//! front of it and nobody else.
//!
//! This is the client's own `--shot` at editor scale and for the same reason —
//! see `vale_client::run`, which has the argument written out at length. It
//! is deliberately **not** shared with it: the client's is welded to a
//! screenshot resource, a frame verdict and a dozen `diagnostics` counters that
//! an editor has no use for, and copying twenty-five lines of Bevy's own public
//! API is cheaper than a seam through a crate that must not know this one
//! exists.
//!
//! ## The delay is the whole of the flag
//!
//! A window is up on the first frame and the world is not: the archives open on
//! first use, the 3x3 streams a tile at a time, and the ground the camera is
//! dropped onto is not known until the tile under it has parsed. `--after`
//! defaults to enough seconds for all of that on a debug build, which is what
//! makes the picture reproducible rather than a race.

use bevy::prelude::*;

/// What was asked for, and whether it has been taken.
#[derive(Resource)]
pub struct Shot {
    /// Where to write it. `None` is every ordinary run.
    pub path: Option<String>,
    /// Seconds to wait first.
    pub after: f32,
    taken: bool,
}

impl Shot {
    pub fn new(path: Option<String>, after: Option<f32>) -> Shot {
        Shot {
            path,
            // Long enough for the archives, the nine tiles and the drop onto the
            // ground, on a debug build. The same number `vale-client --shot`
            // defaults to, for the same reasons.
            after: after.unwrap_or(20.0),
            taken: false,
        }
    }
}

/// **What the frames before the shutter cost**, so a scripted run reports a
/// number rather than a picture alone.
///
/// The editor had no frame instrument a script could read at all, which stopped
/// mattering the moment `--reach` existed: the block is 9 tiles or 49 depending
/// on a flag, and "is the 7x7 affordable" is not a question a screenshot
/// answers. This is the shape the client's own `--shot` prints — a mean with a
/// median and a p95 beside it — and for the reason written down there: the
/// frame-time spread on one machine is wide enough that a single number from a
/// short sample has produced flat contradictions.
///
/// **The window is the last [`SAMPLE`] seconds before the shutter**, not the
/// whole run: everything before that is the archives opening and the block
/// streaming in, which is real work and is not what a frame costs once the
/// world is up.
#[derive(Resource, Default)]
struct Frames(Vec<f32>);

/// How many seconds before the shot are counted. Five is about 300 frames on a
/// debug build, which is enough for a p95 to mean something.
const SAMPLE: f32 = 5.0;

/// `mean / median / p95`, in milliseconds, or `None` for a run too short to
/// have filled the window.
fn verdict(mut samples: Vec<f32>) -> Option<(f32, f32, f32)> {
    if samples.len() < 30 {
        return None;
    }
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    samples.sort_by(|a, b| a.total_cmp(b));
    let at = |fraction: f32| samples[((samples.len() - 1) as f32 * fraction) as usize];
    Some((mean * 1000.0, at(0.5) * 1000.0, at(0.95) * 1000.0))
}

/// Keep the last [`SAMPLE`] seconds of frame times.
///
/// Guarded on the shot being asked for at all, so an ordinary session pays
/// nothing: `path` is `None` in every run but a scripted one.
fn count(shot: Res<Shot>, time: Res<Time>, mut frames: ResMut<Frames>) {
    if shot.path.is_none() || shot.taken {
        return;
    }
    frames.0.push(time.delta_secs());
    // The window in frames is not known in advance, so it is trimmed by the
    // time it holds rather than by a count.
    let mut held: f32 = 0.0;
    let keep = frames
        .0
        .iter()
        .rev()
        .take_while(|delta| {
            held += **delta;
            held <= SAMPLE
        })
        .count()
        .max(1);
    let drop = frames.0.len() - keep;
    frames.0.drain(..drop);
}

pub struct ShotPlugin;

impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Frames>()
            // **`count` before `take`**, so the frame the shutter fires on is
            // in the window it reports rather than the first one after it.
            .add_systems(Update, (count, take).chain());
    }
}

/// Take it, once, and leave.
///
/// **Two frames and not one**: the save is an observer on a spawned entity and
/// the file is written after the render world has read the surface, so quitting
/// on the same frame quits before the picture exists. The flag is set on the
/// frame the shot is asked for and the exit is written on the next.
fn take(
    mut commands: Commands,
    mut shot: ResMut<Shot>,
    time: Res<Time>,
    frames: Res<Frames>,
    reach: Res<vale_client::render::terrain::TerrainReach>,
    tiles: Query<&vale_client::render::terrain::TerrainTile>,
    mut quit: MessageWriter<AppExit>,
    mut asked_at: Local<Option<f32>>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};

    let Some(path) = shot.path.clone() else {
        return;
    };
    let now = time.elapsed_secs();
    if let Some(asked) = *asked_at {
        if now - asked > 0.5 {
            quit.write(AppExit::Success);
        }
        return;
    }
    if shot.taken || now < shot.after {
        return;
    }
    shot.taken = true;
    *asked_at = Some(now);
    info!("--shot: {:.0}s in the editor — {path}", shot.after);
    let block = reach.tiles() * 2 + 1;
    match verdict(frames.0.clone()) {
        Some((mean, median, p95)) => info!(
            "--shot: {mean:.1} ms mean, {median:.1} median, {p95:.1} p95 over {} frames — {block}x{block} reach, {} tiles resident",
            frames.0.len(),
            tiles.iter().count(),
        ),
        None => info!("--shot: too few frames before the shutter to report a frame time"),
    }
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}
