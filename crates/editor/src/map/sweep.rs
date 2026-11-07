//! A find-and-replace over every tile of a map, and the liquid type repair,
//! which walks the same tiles.
//!
//! ## The liquid type repair
//!
//! A wet liquid cell's low nibble is its liquid's type, and the 1.12.1 client
//! draws nothing on a cell of type 0. Tiles wetted before
//! `vale_edit::adt::liquid::Pool::set_wet` wrote the type carry 0 on every
//! wet cell. [`Sweep::repair_liquid`] runs
//! `vale_edit::ops::water::repair_liquid_types` over every tile the map
//! claims, as one undo entry, and saves each tile it changes. A tile's bytes
//! say nothing scannable about a cell's type, so unlike a rename it opens
//! every tile.
//!
//! ## The sweep edits the whole map, not the open tiles
//!
//! Every tool in `crate::tools` edits what is open: a brush reaches the 7x7
//! the camera is streaming, a layer swap reaches the chunk under the pointer.
//! Retiring a tileset from a zone, or changing one tree model across a forest,
//! reaches tiles that are mostly not open and mostly not near the camera, and
//! there are hundreds of them.
//!
//! A rename stays a small operation because every index in an ADT is a
//! position in a name list. `MCLY` indexes `MTEX`, `MDDF` indexes `MMID`,
//! `MODF` indexes `MWID`; so changing a name in place renumbers nothing and
//! touches no record. One `Edit::Names` per tile is the whole edit, and a
//! forest of two thousand trees changes model without any placement moving.
//!
//! ## Spread over frames, not run on a thread
//!
//! The work needs `&mut EditSession` (the history, the open tiles, the
//! overlay), none of which can be sent to another thread, so `crate::jobs`
//! does not fit it. Instead [`Run`] holds a queue and [`advance`] takes
//! [`PER_FRAME`] tiles off it each frame. The window stays responsive, and the
//! panel's progress bar shows the real count.
//!
//! ## Most tiles are rejected without being parsed
//!
//! Azeroth has 687 tiles, each about 2 MB to parse. A sweep that parsed all of
//! them would take eleven seconds to change forty of them.
//!
//! So a rename filters the queue on the raw bytes: a tile whose file does not
//! contain the path as a substring cannot name it, and the check scans a
//! buffer already in hand. It is a deliberate cheap over-approximation, since
//! the path could appear inside some other string; a tile that passes the
//! check and turns out not to name the path reports no change.
//!
//! The scan ignores case. The archives look paths up case-insensitively and
//! one texture is spelled three ways across a map; a case-sensitive scan would
//! skip the tiles that spell it differently, and a swap could miss half the
//! zone.
//!
//! ## Duplicates are not merged
//!
//! Renaming `A` to `B` on a tile that already names `B` leaves two entries
//! reading `B`: correct on screen, with one `MTEX` slot wasted. Merging them
//! would renumber the list and every record indexing it, which is a different
//! operation with its own failure modes. See
//! `vale_edit::adt::place::AdtFile::rename_texture`.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::ops::{Edit, TileNames};
use bevy::prelude::*;

/// How many tiles a frame the sweep gets through.
///
/// Above the one-a-frame [`crate::map::open::open_tiles`] streams at, because these tiles
/// are opened and closed again rather than kept, and because the user is
/// waiting on the progress bar rather than moving the camera. Eight puts Azeroth's 687 at
/// about a second and a half in a debug build, and the frame stays inside its
/// budget on the tiles that are only scanned.
const PER_FRAME: usize = 8;

/// Which list of paths a sweep is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Subject {
    /// `MTEX`, the ground textures: the map-wide counterpart of the Chunk
    /// list's swap, which changes one chunk.
    #[default]
    Texture,
    /// `MMDX`, the placed models.
    Model,
    /// `MWMO`, the placed buildings.
    Building,
}

impl Subject {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Subject::Texture => "texture",
            Subject::Model => "model",
            Subject::Building => "WMO",
        }
    }

    /// The archive folder each one lives under, for a panel to show which type
    /// of path is wanted.
    pub(crate) fn root(self) -> &'static str {
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
pub(crate) const SUBJECTS: [(&str, Subject); 3] = [
    ("Textures", Subject::Texture),
    ("Models", Subject::Model),
    ("WMOs", Subject::Building),
];

/// What the last sweep came to.
#[derive(Debug, Clone, Default)]
pub(crate) struct Report {
    pub(crate) scanned: usize,
    pub(crate) tiles: usize,
    pub(crate) names: usize,
    /// What it was, in words, for the panel and the status line.
    pub(crate) what: String,
}

/// What a sweep does to each tile.
#[derive(Debug, Clone)]
enum Job {
    /// Find or replace one path in one of the three name lists.
    Rename { subject: Subject, from: String, to: String },
    /// Give every wet liquid cell whose type is 0 its liquid's type. See
    /// `vale_edit::ops::water::repair_liquid_types`.
    LiquidTypes,
}

/// A sweep in progress.
pub(crate) struct Run {
    job: Job,
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
    /// Tiles this sweep opened and the session did not have. They are closed
    /// again when the sweep finishes; otherwise a find over a continent would
    /// leave the whole continent resident. `close_tiles` exists to stop that
    /// growth, but it does not see tiles opened this way.
    opened: Vec<(u32, u32)>,
}

impl Run {
    pub(crate) fn done(&self) -> usize {
        self.total - self.queue.len()
    }

    pub(crate) fn total(&self) -> usize {
        self.total
    }
}

/// The tool's own state: the form, whatever is running, and the last answer.
#[derive(Resource, Default)]
pub(crate) struct Sweep {
    pub(crate) subject: Subject,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) running: Option<Run>,
    pub(crate) last: Option<Report>,
}

impl Sweep {
    /// Count the tiles and names that match, without writing anything.
    ///
    /// This is the part of "select every instance of this model" that the
    /// editor supports. It makes no selection: a selection is one decision
    /// covering placements, chunks and tiles together, and it is not made
    /// here. It answers how many and where, which a user needs to know before
    /// deciding to replace, and it is the only way to check that a path is
    /// spelled as intended before changing 400 tiles.
    pub(crate) fn find(&mut self, session: &EditSession, assets: &GameAssets) {
        self.start(session, assets, false);
    }

    /// The same walk as [`Self::find`], writing the replacement.
    pub(crate) fn replace(&mut self, session: &EditSession, assets: &GameAssets) {
        self.start(session, assets, true);
    }

    fn start(&mut self, session: &EditSession, assets: &GameAssets, replace: bool) {
        if self.running.is_some() || self.from.trim().is_empty() {
            return;
        }
        if replace && self.to.trim().is_empty() {
            return;
        }
        let job = Job::Rename {
            subject: self.subject,
            from: self.from.trim().to_string(),
            to: self.to.trim().to_string(),
        };
        self.begin(session, assets, job, replace);
    }

    /// Give every wet liquid cell of type 0 on every tile of the map its
    /// liquid's type, as one undo entry. The 1.12.1 client draws nothing on a
    /// cell of type 0; see `vale_edit::adt::liquid::cell_type`.
    pub(crate) fn repair_liquid(&mut self, session: &EditSession, assets: &GameAssets) {
        if self.running.is_some() {
            return;
        }
        self.begin(session, assets, Job::LiquidTypes, true);
    }

    fn begin(&mut self, session: &EditSession, assets: &GameAssets, job: Job, replace: bool) {
        // Every tile the map claims. The claims are the session's own copy of
        // the WDT, which is the grid the tiles tool writes, so a tile made this
        // session is swept and one unclaimed this session is not.
        let mut queue: Vec<(u32, u32)> = session.claimed.iter().copied().collect();
        // Sorted, so a run that is watched proceeds up the map rather than in
        // hash order, and so two runs over the same map report in the same
        // order. Popped from the back, hence the reverse.
        queue.sort_unstable();
        queue.reverse();
        let total = queue.len();
        let _ = assets;
        self.running = Some(Run {
            job,
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
pub(crate) fn advance(
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
    if !already_open {
        // The cheap rejection, on the bytes, for a rename. See the module
        // comment. A liquid cell's type has no text to scan for, so the
        // liquid repair opens every tile.
        if let Job::Rename { from, .. } = &run.job {
            let Some(bytes) = session.tile_bytes(assets, coord) else {
                return;
            };
            if !mentions(&bytes, from) {
                return;
            }
        }
        if !session.open(assets, coord) {
            return;
        }
        run.opened.push(coord);
    }
    let key = session.key(coord);
    let Some(tile) = session.tiles.get_mut(&coord) else {
        return;
    };
    let (edits, label) = match &run.job {
        Job::Rename { subject, from, to } => {
            let before = TileNames::capture(tile);
            let moved = subject.rename(tile, from, to);
            if moved == 0 {
                // Nothing here after all — an open tile that never named it,
                // or a false positive from the byte scan.
                return;
            }
            run.report.tiles += 1;
            run.report.names += moved;
            if !run.replace {
                // A find puts the tile back exactly as it was. The rename is
                // how the count is taken, because it is the one piece of code
                // that knows what "names this path" means; counting a second
                // way could give a different answer.
                before.restore(tile);
                return;
            }
            let after = TileNames::capture(tile);
            let edit = Edit::Names {
                before: Box::new(before),
                after: Box::new(after),
            };
            (vec![edit], format!("Replace {} {from} with {to}", subject.name()))
        }
        Job::LiquidTypes => {
            let repaired = vale_edit::ops::water::repair_liquid_types(tile);
            if repaired.edits.is_empty() {
                return;
            }
            run.report.tiles += 1;
            run.report.names += repaired.cells;
            (repaired.edits, "Repair liquid cell types".to_string())
        }
    };
    if !run.begun {
        // One history entry for the whole sweep, opened here so that a run
        // that changes nothing pushes none. It stays open across every frame
        // the run takes and is closed in `finish`. Nothing else can edit in
        // between, because the panel's own controls are the only route in and
        // they are disabled while a run is going.
        session.history.begin(label);
        run.begun = true;
    }
    session.history.record(&key, edits);
    session.publish(coord);
    session.stale.insert(coord);
    session.save(coord);
}

/// Close what was opened, close the change, and say what happened.
fn finish(run: Run, session: &mut EditSession, sweep: &mut Sweep) {
    let Run {
        job,
        replace,
        begun: run_begun,
        mut report,
        opened,
        ..
    } = run;
    if replace && run_begun {
        session.history.end();
    }
    // Close every tile this sweep opened, changed or not.
    //
    // A tile this sweep had to open is by construction one the session did not
    // have, and the session has the block around the camera, so nothing is
    // drawing the tile and nobody is looking at it. Its bytes are in the
    // overlay and in the project folder before it closes, and
    // `EditSession::undo` opens what it names, so nothing is lost by dropping
    // the parse.
    //
    // Keeping the changed tiles open does not scale: a replace over a zone is
    // four hundred tiles and eight hundred megabytes, which would stay resident
    // for the rest of the session because of a `stale` flag that the next frame
    // drains anyway.
    for coord in opened {
        session.close(coord);
    }
    let (subject, from, to) = match job {
        Job::Rename { subject, from, to } => (subject, from, to),
        Job::LiquidTypes => {
            report.what = "liquid cell types".to_string();
            session.status = match report.tiles {
                0 => format!("no liquid cell needed a type ({} tiles scanned)", report.scanned),
                tiles => format!(
                    "gave {} liquid cell(s) their type on {tiles} tile(s); {} tiles scanned",
                    report.names, report.scanned
                ),
            };
            sweep.last = Some(report);
            return;
        }
    };
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

/// Whether this tile's file contains that path anywhere, ignoring case.
///
/// The filter that spares a sweep 600 parses. It over-approximates on purpose:
/// a match means only that the tile may name the path, because the exact
/// answer needs the name lists parsed, and the tiles it passes are then
/// parsed and checked.
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

pub(crate) struct SweepPlugin;

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
    /// Case matters here: the archives look paths up case-insensitively, so
    /// one tileset is spelled three ways across a map, and a case-sensitive
    /// scan would sweep only a third of the zone and report no error.
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

    /// A rename over a tile's three lists keeps every index, which is why this
    /// operation is cheap.
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

/// `--find <path>` and `--repair-liquid` — arm the sweep and run it, once, on
/// the first frame the session is up.
///
/// The scripted route into this tool, working like
/// `crate::tools::placements::place::scripted` and for the same reason: the
/// panel is two text fields and buttons, none of which a `--shot` run can
/// press, so without this a script could only capture the empty form. `--find` finds and
/// never replaces — see [`crate::args::tools::Tools::find`]. `--repair-liquid`
/// writes; see [`crate::args::tools::Tools::repair_liquid`].
///
/// The report goes to the log as well as to the status line, because a run that
/// quits on `--after` is read from its output rather than from its last frame.
fn scripted(
    mut sweep: ResMut<Sweep>,
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    args: Res<crate::args::Args>,
    tool: Res<crate::tools::Tool>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let wanted = args.tools.find.is_some() || args.tools.repair_liquid;
    let (true, Some(session)) = (wanted, session) else {
        // No flag is nothing to do; no session yet is a frame too early, and
        // the session is inserted within the first few.
        *done = !wanted;
        return;
    };
    if *tool != crate::tools::Tool::Sweep {
        return;
    }
    *done = true;
    match args.tools.find.clone() {
        Some(asked) => {
            sweep.from = asked;
            sweep.find(&session, &assets);
        }
        None => sweep.repair_liquid(&session, &assets),
    }
}

/// Log what a scripted sweep came to, once it has finished.
fn report(sweep: Res<Sweep>, args: Res<crate::args::Args>, mut said: Local<bool>) {
    let wanted = args.tools.find.is_some() || args.tools.repair_liquid;
    if *said || !wanted || sweep.running.is_some() {
        return;
    }
    let Some(report) = &sweep.last else { return };
    *said = true;
    let flag = match args.tools.find.is_some() {
        true => "--find",
        false => "--repair-liquid",
    };
    info!(
        "{flag}: {} — {} tile(s), {} name(s) or cell(s), {} of the map's tiles scanned",
        report.what, report.tiles, report.names, report.scanned
    );
}
