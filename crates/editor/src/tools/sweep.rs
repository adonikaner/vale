//! A find-and-replace over every tile of a map.
//!
//! ## The one operation whose subject is the map rather than the tile
//!
//! Everything else in this directory edits what is open: a brush reaches the
//! 7x7 the camera is streaming, a layer swap reaches a chunk the pointer is
//! over. Retiring a tileset from a zone, or changing one tree across a forest,
//! is neither — the tiles are mostly not open and mostly not near the camera,
//! and there are hundreds of them.
//!
//! What makes it a small operation anyway is that **every index in an ADT is a
//! position in a name list**. `MCLY` indexes `MTEX`, `MDDF` indexes `MMID`,
//! `MODF` indexes `MWID`; so changing what a name *is*, in place, renumbers
//! nothing and touches no record. One `Edit::Names` per tile is the whole of
//! it, and a forest of two thousand trees changes model without a single
//! placement moving.
//!
//! ## It is spread over frames, not put on a thread
//!
//! The work wants `&mut EditSession` — the history, the open tiles, the
//! overlay — none of which crosses a thread, so `crate::jobs` is the wrong
//! shape for it. Instead [`Run`] holds a queue and [`advance`] takes
//! [`PER_FRAME`] tiles off it each frame, which keeps the window answering and
//! gives the panel something true to draw a bar from.
//!
//! ## Most tiles are rejected without being parsed
//!
//! A map is 687 tiles on Azeroth and about 2 MB of parse each. A sweep that
//! parsed all of them would be eleven seconds of work to change forty of them.
//!
//! So the queue is filtered on the **bytes**: a tile whose file does not
//! contain the path as a substring cannot name it, and the check is a scan of a
//! buffer already in hand. It is deliberately a cheap over-approximation — the
//! path could appear inside some other string — and a tile that passes it and
//! turns out not to name the path simply reports no change.
//!
//! **Case is the one trap in that.** The archives answer case-insensitively and
//! one texture is spelled three ways across a map, so the scan is
//! case-insensitive too; a case-sensitive one would skip the tiles that spell
//! it differently and report a swap that had missed half the zone.
//!
//! ## What it does not do
//!
//! It does not fold a duplicate. Renaming `A` to `B` on a tile that already
//! names `B` leaves two entries reading `B` — correct on screen, one slot of
//! `MTEX`'s wasted. Folding them would renumber the list and every record
//! indexing it, which is a different operation with its own way of going
//! wrong. See `vale_edit::adt::place::AdtFile::rename_texture`.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::ops::{Edit, TileNames};
use bevy::prelude::*;

/// How many tiles a frame the sweep gets through.
///
/// Above the one-a-frame [`super::open_tiles`] streams at, because these tiles
/// are opened and closed again rather than kept, and because a person watching
/// a progress bar is not also flying the camera. Eight puts Azeroth's 687 at
/// about a second and a half in a debug build, and the frame stays inside its
/// budget on the tiles that are only scanned.
const PER_FRAME: usize = 8;

/// Which list of paths a sweep is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Subject {
    /// `MTEX`, the ground textures — the map-wide half of the Chunk list's
    /// **swap**, which does one chunk.
    #[default]
    Texture,
    /// `MMDX`, the placed models.
    Model,
    /// `MWMO`, the placed buildings.
    Building,
}

impl Subject {
    pub fn name(self) -> &'static str {
        match self {
            Subject::Texture => "texture",
            Subject::Model => "model",
            Subject::Building => "WMO",
        }
    }

    /// The archive folder each one lives under, for a panel that wants to say
    /// what kind of path is wanted.
    pub fn root(self) -> &'static str {
        match self {
            Subject::Texture => r"Tileset\",
            Subject::Model => r"World\",
            Subject::Building => r"World\wmo\",
        }
    }

    fn rename(self, tile: &mut vale_edit::adt::AdtFile, from: &str, to: &str) -> usize {
        match self {
            Subject::Texture => tile.rename_texture(from, to),
            Subject::Model => tile.rename_model(from, to),
            Subject::Building => tile.rename_building(from, to),
        }
    }
}

/// The three lists, as a panel needs them.
pub const SUBJECTS: [(&str, Subject); 3] = [
    ("Textures", Subject::Texture),
    ("Models", Subject::Model),
    ("WMOs", Subject::Building),
];

/// What the last sweep came to.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub scanned: usize,
    pub tiles: usize,
    pub names: usize,
    /// What it was, in words, for the panel and the status line.
    pub what: String,
}

/// A sweep in progress.
pub struct Run {
    subject: Subject,
    from: String,
    to: String,
    /// The tiles still to look at, popped from the back.
    queue: Vec<(u32, u32)>,
    total: usize,
    /// Whether to write, or only to count — see [`Sweep::find`].
    replace: bool,
    /// Whether the change on the history has been opened yet.
    ///
    /// Opened on the first tile that actually changes rather than at the start,
    /// so a replace that matches nothing leaves no empty entry on the stack for
    /// `Ctrl+Z` to land on.
    begun: bool,
    report: Report,
    /// Tiles this sweep opened and the session did not have. Closed again on
    /// the way out, or a find over a continent leaves the whole continent
    /// resident — which is the growth `close_tiles` was written to stop, from
    /// a direction it cannot see.
    opened: Vec<(u32, u32)>,
}

impl Run {
    pub fn done(&self) -> usize {
        self.total - self.queue.len()
    }

    pub fn total(&self) -> usize {
        self.total
    }
}

/// The tool's own state: the form, whatever is running, and the last answer.
#[derive(Resource, Default)]
pub struct Sweep {
    pub subject: Subject,
    pub from: String,
    pub to: String,
    pub running: Option<Run>,
    pub last: Option<Report>,
}

impl Sweep {
    /// **Count, without writing anything.**
    ///
    /// The half of "select every instance of this model" this editor can
    /// honestly offer: not a selection — that is one decision for placements,
    /// chunks and tiles together and is not made here — but the answer to *how
    /// many and where*, which is the question a person asks before deciding to
    /// replace. It is also the only way to find out whether a path is spelled
    /// the way you think it is before changing 400 tiles.
    pub fn find(&mut self, session: &EditSession, assets: &GameAssets) {
        self.start(session, assets, false);
    }

    /// …and the same walk, writing.
    pub fn replace(&mut self, session: &EditSession, assets: &GameAssets) {
        self.start(session, assets, true);
    }

    fn start(&mut self, session: &EditSession, assets: &GameAssets, replace: bool) {
        if self.running.is_some() || self.from.trim().is_empty() {
            return;
        }
        if replace && self.to.trim().is_empty() {
            return;
        }
        let from = self.from.trim().to_string();
        let to = self.to.trim().to_string();
        // **Every tile the map claims**, and the claims are the session's own
        // copy of the WDT — which is the grid the tiles tool writes, so a tile
        // made this session is swept and one unclaimed this session is not.
        let mut queue: Vec<(u32, u32)> = session.claimed.iter().copied().collect();
        // Sorted, so a run that is watched proceeds up the map rather than in
        // hash order, and so two runs over the same map report in the same
        // order. Popped from the back, hence the reverse.
        queue.sort_unstable();
        queue.reverse();
        let total = queue.len();
        let _ = assets;
        self.running = Some(Run {
            subject: self.subject,
            from,
            to,
            queue,
            total,
            replace,
            begun: false,
            report: Report {
                what: String::new(),
                ..Report::default()
            },
            opened: Vec::new(),
        });
    }
}

/// Take [`PER_FRAME`] tiles off whatever is running.
pub fn advance(
    mut sweep: ResMut<Sweep>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
) {
    let (Some(mut session), true) = (session, sweep.running.is_some()) else {
        return;
    };
    let mut done = false;
    if let Some(run) = sweep.running.as_mut() {
        for _ in 0..PER_FRAME {
            let Some(coord) = run.queue.pop() else { break };
            step(run, &mut session, &assets, coord);
        }
        done = run.queue.is_empty();
    }
    if done {
        let run = sweep.running.take().expect("checked above");
        finish(run, &mut session, &mut sweep);
    }
}

/// One tile of the sweep.
fn step(run: &mut Run, session: &mut EditSession, assets: &GameAssets, coord: (u32, u32)) {
    run.report.scanned += 1;
    let already_open = session.tiles.contains_key(&coord);
    // The cheap rejection, on the bytes. See the module comment.
    if !already_open {
        let Some(bytes) = session.tile_bytes(assets, coord) else {
            return;
        };
        if !mentions(&bytes, &run.from) {
            return;
        }
        if !session.open(assets, coord) {
            return;
        }
        run.opened.push(coord);
    }
    let key = session.key(coord);
    let (subject, from, to) = (run.subject, run.from.clone(), run.to.clone());
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let before = TileNames::capture(tile);
    let moved = subject.rename(tile, &from, &to);
    if moved == 0 {
        // Nothing here after all — an open tile that never named it, or a
        // false positive from the byte scan.
        return;
    }
    run.report.tiles += 1;
    run.report.names += moved;
    if !run.replace {
        // A find puts the tile back exactly as it was. The rename is how the
        // count is taken, because it is the one piece of code that knows what
        // "names this path" means — asking twice, two ways, is how the two
        // answers come to disagree.
        before.restore(tile);
        return;
    }
    let after = TileNames::capture(tile);
    if !run.begun {
        // **One entry for the whole sweep**, opened here so that a replace
        // matching nothing pushes none. It stays open across every frame the
        // run takes and is closed in `finish`; nothing else can edit in
        // between, because the panel's own controls are the only route in and
        // they are disabled while a run is going.
        session
            .history
            .begin(format!("Replace {} {from} with {to}", subject.name()));
        run.begun = true;
    }
    session.history.record(
        &key,
        [Edit::Names {
            before: Box::new(before),
            after: Box::new(after),
        }],
    );
    session.publish(coord);
    session.stale.insert(coord);
    session.save(coord);
}

/// Close what was opened, close the change, and say what happened.
fn finish(run: Run, session: &mut EditSession, sweep: &mut Sweep) {
    let Run {
        subject,
        from,
        to,
        replace,
        begun: run_begun,
        mut report,
        opened,
        ..
    } = run;
    if replace && run_begun {
        session.history.end();
    }
    // **Everything this sweep opened goes again, changed or not.**
    //
    // A tile this sweep had to open is by construction one the session did not
    // have, and the session has the block around the camera — so it is a tile
    // nothing is drawing and nobody is looking at. Its bytes are in the overlay
    // and in the project folder before it closes, and `EditSession::undo` opens
    // what it names, so nothing is lost by letting go of the parse.
    //
    // Keeping the changed ones was the first draft and it is wrong at the only
    // scale this tool matters at: a replace over a zone is four hundred tiles
    // and eight hundred megabytes, held for the rest of the session by a
    // `stale` flag that the next frame drains anyway.
    for coord in opened {
        session.close(coord);
    }
    report.what = match replace {
        true => format!("{} {from} → {to}", subject.name()),
        false => format!("{} {from}", subject.name()),
    };
    session.status = match (replace, report.tiles) {
        (_, 0) => format!(
            "no tile of this map references that {} ({} tiles scanned)",
            subject.name(),
            report.scanned
        ),
        (true, tiles) => format!(
            "replaced {} with {to} on {tiles} tile(s), {} name(s); {} tiles scanned",
            from, report.names, report.scanned
        ),
        (false, tiles) => format!(
            "{from} is referenced on {tiles} tile(s), {} time(s); {} tiles scanned",
            report.names, report.scanned
        ),
    };
    sweep.last = Some(report);
}

/// **Does this tile's file mention that path at all**, ignoring case.
///
/// The filter that keeps a sweep off 600 parses. An over-approximation on
/// purpose: it says *maybe* where the real answer needs the name lists parsed,
/// and the tiles it lets through are then asked properly.
fn mentions(bytes: &[u8], path: &str) -> bool {
    let needle = path.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return false;
    }
    let first = needle[0].to_ascii_lowercase();
    let upper = needle[0].to_ascii_uppercase();
    bytes.windows(needle.len()).any(|window| {
        (window[0] == first || window[0] == upper)
            && window
                .iter()
                .zip(needle)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

pub struct SweepPlugin;

impl Plugin for SweepPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Sweep>()
            .add_systems(Update, (scripted, advance, report).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The byte filter finds the path however it is spelled, and says no to a
    /// tile that does not carry it.
    ///
    /// Case is the half that matters: the archives answer case-insensitively,
    /// so one tileset is spelled three ways across a map and a case-sensitive
    /// scan would quietly sweep a third of the zone.
    #[test]
    fn the_byte_filter_ignores_case_and_refuses_what_is_absent() {
        let tile = b"MTEX\0\0\0\0Tileset\\Elwynn\\ElwynnGrass01.blp\0Tileset\\Dun.blp\0";
        assert!(mentions(tile, r"Tileset\Elwynn\ElwynnGrass01.blp"));
        assert!(mentions(tile, r"tileset\elwynn\elwynngrass01.blp"));
        assert!(mentions(tile, r"TILESET\ELWYNN\ELWYNNGRASS01.BLP"));
        assert!(!mentions(tile, r"Tileset\Barrens\BarrensDirt.blp"));
        assert!(!mentions(tile, ""));
        // Longer than the file is not a match and does not panic.
        assert!(!mentions(b"ab", r"Tileset\Elwynn\ElwynnGrass01.blp"));
    }

    /// A rename over a tile's three lists keeps every index, which is the whole
    /// reason this operation is cheap.
    #[test]
    fn a_rename_keeps_every_index_in_the_tile() {
        use vale_edit::adt::AdtFile;
        let mut tile =
            vale_edit::adt::blank_tile(0, 0, r"TilesetlwynnlwynnGrass01.blp", 0.0, 12);
        let grass = tile.name_model(r"World\Tree\Oak.m2");
        let rock = tile.name_model(r"World\Rock\Boulder.m2");
        assert_eq!((grass, rock), (0, 1));

        // A longer name, so the offsets have to be rebuilt rather than patched.
        assert_eq!(
            tile.rename_model(r"world\tree\oak.m2", r"World\Tree\WeepingWillow.m2"),
            1
        );
        assert_eq!(
            tile.model_names(),
            vec![
                r"World\Tree\WeepingWillow.m2".to_string(),
                r"World\Rock\Boulder.m2".to_string()
            ],
            "the order is kept, so every MDDF index still names what it named"
        );
        assert_eq!(
            tile.rename_model(r"World\Tree\Oak.m2", "x"),
            0,
            "gone is gone"
        );

        // …and it survives a write and a re-parse, which is what says the
        // rebuilt MMID agrees with the rebuilt MMDX.
        let round = AdtFile::parse(&tile.write()).expect("a tile this crate wrote");
        assert_eq!(round.model_names(), tile.model_names());
    }
}

/// `--find <path>` — arm the sweep and run it, once, on the first frame the
/// session is up.
///
/// The scripted route into this tool, on `crate::tools::place::scripted`'s own
/// terms and for the same reason: the panel is two text fields and a button,
/// none of which a `--shot` run can press, so without this the only thing a
/// script could do with a sweep is photograph an empty form. It finds and never
/// replaces — see [`crate::Args::find`].
///
/// The report goes to the log as well as to the status line, because a run that
/// quits on `--after` is read from its output rather than from its last frame.
fn scripted(
    mut sweep: ResMut<Sweep>,
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    args: Res<crate::Args>,
    tool: Res<crate::tools::Tool>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (Some(asked), Some(session)) = (args.find.clone(), session) else {
        // No flag is nothing to do; no session yet is a frame too early, and
        // the session is inserted within the first few.
        *done = args.find.is_none();
        return;
    };
    if *tool != crate::tools::Tool::Sweep {
        return;
    }
    *done = true;
    sweep.from = asked;
    sweep.find(&session, &assets);
}

/// …and say what it came to, once it is over.
fn report(sweep: Res<Sweep>, args: Res<crate::Args>, mut said: Local<bool>) {
    if *said || args.find.is_none() || sweep.running.is_some() {
        return;
    }
    let Some(report) = &sweep.last else { return };
    *said = true;
    info!(
        "--find: {} — {} tile(s), {} name(s), {} of the map's tiles scanned",
        report.what, report.tiles, report.names, report.scanned
    );
}
