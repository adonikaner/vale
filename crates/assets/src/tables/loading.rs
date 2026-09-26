//! **The picture the world is hidden behind while it loads**, and the bar drawn
//! over it.
//!
//! Two questions, both of them the client's own and neither of them on the wire.
//! The server says "you are going to map 33" (`SMSG_TRANSFER_PENDING`) and
//! nothing else; which parchment that puts on the screen, and where the progress
//! bar sits on it, are decided here.
//!
//! ## Which picture: `Map.dbc` field 38 -> `LoadingScreens.dbc`
//!
//! `LoadingScreens.dbc` is 40 rows of `{id, name, path}` — `Kalimdor`,
//! `InstanceDeadmines`, `InstanceRagefireChasm` — and the join onto it is
//! **`Map.dbc` field 38**, which vmangos does not read (its `MapEntryfmt`
//! skips it) and which no table in this repo had touched.
//!
//! The client's background loader is a short table walk:
//!
//! ```text
//! the map the screen is being raised for
//!   -1 is "no map" — see below
//! checked against Map.dbc's own id bound
//! the MapRec's +0x98 = field 38 = the LoadingScreens id
//! checked against LoadingScreens.dbc's bound
//! the LoadingScreensRec's +8 = field 2 = the .blp path
//! ...and does the archive actually have it?
//! ```
//!
//! **Every one of those five failure paths lands on the same fallback**, which
//! is `Interface\Glues\loading` ([`FALLBACK`]): a map id
//! out of range, a map with no screen, a screen id out of range, a row that is
//! not there, and — the one that is not a bounds check — **a path the archive
//! chain does not carry**. So a named-but-missing picture is not a black screen
//! here any more than it is there.
//!
//! The join checks out against the five maps whose answer is known by name:
//! Azeroth -> 4 (`LoadScreenEasternKingdom`), Kalimdor -> 3, Alterac Valley ->
//! 104, Shadowfang Keep -> 204, Ragefire Chasm -> 195. `vale loading` is the
//! census over all 44.
//!
//! ## Where the bar goes: a two-entry table, and the fill is one subtraction
//!
//! The bar is not FrameXML and not a widget — it is two textures in a table
//! the client walks both to load and to draw them:
//!
//! ```text
//!   texture
//!   is this the one that fills?
//!   centre x        0.5   both
//!   centre y        0.075 both
//!   width           0.525 fill / 0.6  border
//!   height          0.025 fill / 0.05 border
//! ```
//!
//! …drawn as `[cx - w/2, cx + w/2] x [cy - h/2, cy + h/2]`, and the fill's
//! **right edge alone** replaced by
//! `left + progress * w`. That is the whole of the animation: one
//! quad whose right edge moves.
//!
//! ## The space all six numbers are in, and it is **not** the window
//!
//! The background is the unit square. Its four vertices are written out
//! literally — `(0,0) (1,0) (0,1) (1,1)` — and the
//! bar's rect is in the same normalised space. `y` runs up: 0.075 is a bar near
//! the bottom, which is where it is.
//!
//! **What that unit square covers is a centred 4:3 box, not the window.** The
//! frame function saves the viewport, sets it to the whole window, clears, and
//! only then narrows it:
//!
//! ```text
//! save the viewport
//! set it to the whole window and clear it
//! r — see below, and it is NOT the aspect — against 1.0:
//!   > 1   width  = 1/r,  left   = (1 - 1/r)/2      ; pillarbox
//!   < 1   height = r,    bottom = (1 - r)/2        ; letterbox
//!   == 1  the whole window
//! SetViewport(l, r, b, t, 0, 1)
//! ...and put the old one back afterwards
//! ```
//!
//! **`r` is the whole answer.** The client stores it alongside the FOV pair
//! `render::glue::vertical_fov` already documents, `a/√(a²+1)` and
//! `1/√(a²+1)`, as **`aspect * 0.75`**. So `r` is the aspect
//! *relative to 4:3*, and what the viewport fits is a **4:3 box**: on a 16:9
//! window the picture is the 4:3 image with black either side of it, exactly as
//! it looked on the monitors it was drawn for. **And the bar is inside that box
//! too** — one viewport covers both draws.
//!
//! Stretching the art across the window instead is the one mistake here that
//! produces a complete, plausible picture, and it is what this client did at
//! first. Reading `r` as the bare aspect is the
//! *second* — it fits a square, which is just as plausible (the art is 512x512)
//! and just as wrong. [`fit`] is the rule and [`TARGET_ASPECT`] is the number.
//!
//! Everything above is measurement. What this client does with it — when the
//! screen goes up, what "progress" is a fraction *of*, and when it comes down —
//! is `crates/client/src/game/loading.rs`, and none of it is the reference's.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// `Map.dbc`: which `LoadingScreens.dbc` row a map's picture is.
///
/// Field 38, `+0x98` in the record — see the module comment, where the walk
/// that reads it is.
const MAP_LOADING_SCREEN: usize = 38;

/// `LoadingScreens.dbc`: the `.blp` path, field 2.
const SCREEN_PATH: usize = 2;

/// `LoadingScreens.dbc`: the row's own name — `Kalimdor`, `InstanceDeadmines`.
/// Read only so that `vale loading` can print it beside the map.
const SCREEN_NAME: usize = 1;

/// **The picture shown for a map that names none, or names one the archives do
/// not have.**
///
/// Reached by all five failure paths in the module
/// comment. No extension, like every other `Interface\` path the client builds:
/// the file is `Interface\Glues\loading.blp`, and it is byte-for-byte the size
/// of `LoadScreenEasternKingdom.blp`.
pub const FALLBACK: &str = "Interface\\Glues\\loading";

/// One of the two pieces of the loading bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarPiece {
    /// `Interface\Glues\LoadingBar\…`, without the extension.
    pub path: &'static str,
    /// Whether this is the piece whose right edge tracks the progress. The
    /// border's is fixed.
    pub fills: bool,
    /// Centre and size, as fractions of the viewport, `y` up from the bottom.
    pub centre_x: f32,
    pub centre_y: f32,
    pub width: f32,
    pub height: f32,
}

impl BarPiece {
    /// Where this piece is drawn, as `[left, bottom, right, top]` in the unit
    /// square with `y` up.
    ///
    /// `progress` is ignored for the border and clamped for the fill: a
    /// fraction outside `0..=1` would draw a quad reaching past its own frame,
    /// and the client's own update clamps at both ends too.
    pub fn rect(&self, progress: f32) -> [f32; 4] {
        let left = self.centre_x - self.width * 0.5;
        let bottom = self.centre_y - self.height * 0.5;
        let right = if self.fills {
            left + self.width * progress.clamp(0.0, 1.0)
        } else {
            self.centre_x + self.width * 0.5
        };
        [left, bottom, right, self.centre_y + self.height * 0.5]
    }
}

/// The two pieces, **in the order the client draws them** — the client's table
/// walks upwards, so the fill is laid down first and the border over
/// it, which is what lets the frame's inner shadow sit on top of the fill
/// rather than under it.
pub const BAR: [BarPiece; 2] = [
    BarPiece {
        path: "Interface\\Glues\\LoadingBar\\Loading-BarFill",
        fills: true,
        centre_x: 0.5,
        centre_y: 0.075,
        width: 0.525,
        height: 0.025,
    },
    BarPiece {
        path: "Interface\\Glues\\LoadingBar\\Loading-BarBorder",
        fills: false,
        centre_x: 0.5,
        centre_y: 0.075,
        width: 0.6,
        height: 0.05,
    },
];

/// **The shape the loading screen is drawn at, whatever the window is** — 4:3,
/// out of the client's `0.75`. See the module comment.
///
/// It is *not* the art's own shape: every loading screen the game ships is
/// 512x512 and is stretched to this. Which is the point — the picture looks the
/// way it looked on the monitors it was drawn for, and the window gets bars.
pub const TARGET_ASPECT: f32 = 4.0 / 3.0;

/// **The part of the window the loading screen is drawn into** — a centred
/// [`TARGET_ASPECT`] box — as `[left, bottom, right, top]` in `0..=1` of the
/// window, `y` up.
///
/// `aspect` is the window's `width / height`. The three branches and the
/// addresses they come from are in the module comment; what they amount to is
/// that the picture keeps its 4:3 shape and the window gets black bars, on
/// whichever pair of edges it has spare.
///
/// **Everything the loading screen draws goes through this**, the bar included:
/// the reference sets one viewport and draws both, so the bar is 60% of the
/// *box* rather than 60% of the window. Fitting the picture and leaving the bar
/// on the window would be a bar that slides out from under its own frame as the
/// window widens.
pub fn fit(aspect: f32) -> [f32; 4] {
    if !aspect.is_finite() || aspect <= 0.0 {
        // A zero-height window has no aspect and no pixels; answering the whole
        // of it is the harmless reading, and the alternative is a NaN rectangle
        // reaching a painter.
        return [0.0, 0.0, 1.0, 1.0];
    }
    // The client's own `r`, and it is compared with 1 rather than the aspect
    // being compared with 4:3 — the same test, written the way the client
    // writes it.
    let r = aspect / TARGET_ASPECT;
    if r > 1.0 {
        let width = 1.0 / r;
        let left = (1.0 - width) * 0.5;
        [left, 0.0, left + width, 1.0]
    } else {
        let bottom = (1.0 - r) * 0.5;
        [0.0, bottom, 1.0, bottom + r]
    }
}

/// Which picture each map is hidden behind.
///
/// Empty is a working table and not a failure: every map then answers
/// [`FALLBACK`], which is exactly what the reference does for a map whose row
/// says nothing. See [`LoadingScreens::load`].
#[derive(Debug, Default, Clone)]
pub struct LoadingScreens {
    /// map id -> `LoadingScreens.dbc` id, straight out of field 38. Kept as the
    /// id rather than resolved so that the check can report a map naming a row
    /// the table does not have — which is one of the five fallback paths, and
    /// which collapsing the two tables into one map would hide.
    by_map: HashMap<u32, u32>,
    /// `LoadingScreens.dbc` id -> `(name, path)`.
    screens: HashMap<u32, (String, String)>,
}

impl LoadingScreens {
    /// Read both tables. `read` is given a bare table name, the same shape
    /// [`crate::tables::dbc::DisplayTables::load`] takes.
    ///
    /// **Never fails.** A chain without either table answers [`FALLBACK`] for
    /// every map, which is a client that shows the generic parchment rather than
    /// one that shows nothing — the reference's own degradation, not a new one.
    pub fn load(mut read: impl FnMut(&str) -> Option<Vec<u8>>) -> LoadingScreens {
        let mut out = LoadingScreens::default();
        if let Some(dbc) = read("Map").and_then(|raw| Dbc::parse(&raw).ok()) {
            for record in 0..dbc.record_count {
                let (Some(id), Some(screen)) = (
                    dbc.u32_at(record, 0),
                    dbc.u32_at(record, MAP_LOADING_SCREEN),
                ) else {
                    continue;
                };
                out.by_map.insert(id, screen);
            }
        }
        if let Some(dbc) = read("LoadingScreens").and_then(|raw| Dbc::parse(&raw).ok()) {
            for record in 0..dbc.record_count {
                let (Some(id), Some(name), Some(path)) = (
                    dbc.u32_at(record, 0),
                    dbc.string_at(record, SCREEN_NAME),
                    dbc.string_at(record, SCREEN_PATH),
                ) else {
                    continue;
                };
                if !path.is_empty() {
                    out.screens.insert(id, (name, path));
                }
            }
        }
        out
    }

    /// **The one call the renderer makes**: what to draw while `map` loads.
    ///
    /// Falls back rather than answering `None` for the four table failures the
    /// client falls back for. The fifth — the archive not carrying the file —
    /// cannot be decided here, so the caller checks it: see
    /// [`LoadingScreens::picture_in`].
    pub fn picture(&self, map: u32) -> &str {
        self.by_map
            .get(&map)
            .and_then(|screen| self.screens.get(screen))
            .map_or(FALLBACK, |(_, path)| path.as_str())
    }

    /// …and the same question with the archive's answer folded in, which is the
    /// client's fifth fallback path (does the archive have the file?).
    ///
    /// `have` is asked about the resolved path exactly once, so a caller that
    /// has to open an archive to answer pays for one lookup per raise.
    pub fn picture_in(&self, map: u32, have: impl FnOnce(&str) -> bool) -> &str {
        let path = self.picture(map);
        if path == FALLBACK || have(path) {
            path
        } else {
            FALLBACK
        }
    }

    /// Which `LoadingScreens.dbc` row a map names, whether or not that row
    /// exists. For the check, which is the only caller that cares about the
    /// difference between "names nothing" and "names row 999".
    pub fn screen_of(&self, map: u32) -> Option<u32> {
        self.by_map.get(&map).copied()
    }

    /// One row of `LoadingScreens.dbc`, as `(name, path)`.
    pub fn screen(&self, id: u32) -> Option<(&str, &str)> {
        self.screens
            .get(&id)
            .map(|(name, path)| (name.as_str(), path.as_str()))
    }

    /// Every row, for the census.
    pub fn screens(&self) -> impl Iterator<Item = (u32, &str, &str)> {
        self.screens
            .iter()
            .map(|(id, (name, path))| (*id, name.as_str(), path.as_str()))
    }

    /// How many maps carry a screen id at all — 0 for an empty table, which is
    /// what tells "the DBC did not load" from "the DBC says nothing".
    pub fn maps(&self) -> usize {
        self.by_map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DBC with `fields` columns per record and one string block.
    fn dbc(records: &[Vec<u32>], strings: &[u8]) -> Vec<u8> {
        let fields = records.first().map_or(0, Vec::len);
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(records.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for record in records {
            for value in record {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        out
    }

    /// Two maps and two screens, in the shipped shape: field 38 of `Map.dbc`
    /// naming a `LoadingScreens.dbc` id.
    fn tables() -> impl FnMut(&str) -> Option<Vec<u8>> {
        // "\0Kalimdor\0Interface\Glues\LoadingScreens\LoadScreenKalimdor.blp\0"
        let mut strings = vec![0u8];
        let name = strings.len() as u32;
        strings.extend_from_slice(b"Kalimdor\0");
        let path = strings.len() as u32;
        strings.extend_from_slice(b"Interface\\Glues\\LoadingScreens\\LoadScreenKalimdor.blp\0");

        let mut map = vec![0u32; 42];
        map[0] = 1;
        map[MAP_LOADING_SCREEN] = 3;
        // A second map naming a row that is not in the table — one of the five
        // fallback paths, and the reason the two tables are not pre-joined.
        let mut orphan = vec![0u32; 42];
        orphan[0] = 999;
        orphan[MAP_LOADING_SCREEN] = 4242;
        // …and a third naming nothing at all, which is field 38 == 0.
        let mut silent = vec![0u32; 42];
        silent[0] = 451;

        let maps = dbc(&[map, orphan, silent], b"\0");
        let screens = dbc(&[vec![3, name, path]], &strings);
        move |table| match table {
            "Map" => Some(maps.clone()),
            "LoadingScreens" => Some(screens.clone()),
            _ => None,
        }
    }

    /// The join, and each of the four table-side fallbacks it has to survive.
    #[test]
    fn a_map_is_hidden_behind_its_own_screen_or_behind_the_generic_one() {
        let screens = LoadingScreens::load(tables());
        assert_eq!(
            screens.picture(1),
            "Interface\\Glues\\LoadingScreens\\LoadScreenKalimdor.blp"
        );
        // Names a row that is not there.
        assert_eq!(screens.picture(999), FALLBACK);
        // Names nothing.
        assert_eq!(screens.picture(451), FALLBACK);
        // Is not in Map.dbc at all.
        assert_eq!(screens.picture(1234), FALLBACK);
        // …and the pair the check needs to tell the second from the third.
        assert_eq!(screens.screen_of(999), Some(4242));
        assert_eq!(screens.screen_of(451), Some(0));
        assert_eq!(screens.screen_of(1234), None);
    }

    /// **The fifth fallback, which is not a table lookup**: the archive chain
    /// not carrying the file the row names. Without it a patched-out picture is
    /// a black screen, where the reference draws the generic one.
    #[test]
    fn a_picture_the_archives_do_not_have_falls_back_too() {
        let screens = LoadingScreens::load(tables());
        assert_eq!(screens.picture_in(1, |_| true), screens.picture(1));
        assert_eq!(screens.picture_in(1, |_| false), FALLBACK);
        // …and the fallback itself is never re-checked, because there is
        // nothing left to fall back *to*.
        assert_eq!(screens.picture_in(451, |_| false), FALLBACK);
    }

    /// An empty chain is a working table answering the generic picture, not a
    /// panic and not an empty path — see [`LoadingScreens::load`].
    #[test]
    fn no_tables_at_all_is_the_generic_picture_and_not_a_blank_one() {
        let screens = LoadingScreens::load(|_| None);
        assert_eq!(screens.maps(), 0);
        assert_eq!(screens.picture(0), FALLBACK);
    }

    /// The fill's right edge is the only thing that moves, and it moves across
    /// its own width — the client adds `progress * w` to the left edge it has
    /// just computed.
    #[test]
    fn the_fill_grows_from_its_left_edge_and_the_border_does_not_move() {
        let [fill, border] = BAR;
        assert!(fill.fills && !border.fills);

        let empty = fill.rect(0.0);
        let half = fill.rect(0.5);
        let full = fill.rect(1.0);
        // Left, bottom and top are the same at every progress.
        for index in [0, 1, 3] {
            assert_eq!(empty[index], half[index], "{index}");
            assert_eq!(empty[index], full[index], "{index}");
        }
        assert_eq!(empty[2], empty[0], "an empty bar has no width");
        assert!((half[2] - (empty[0] + fill.width * 0.5)).abs() < 1e-6);
        assert!((full[2] - (empty[0] + fill.width)).abs() < 1e-6);

        // …and the border ignores it entirely.
        assert_eq!(border.rect(0.0), border.rect(1.0));
    }

    /// The fill sits **inside** the border on all four sides — the check that
    /// the four floats have not been transposed, which is the one mistake here
    /// that would draw plausibly rather than fail.
    #[test]
    fn the_fill_sits_inside_the_frame_at_full() {
        let [fill, border] = BAR;
        let inner = fill.rect(1.0);
        let outer = border.rect(1.0);
        assert!(inner[0] > outer[0] && inner[2] < outer[2], "{inner:?}");
        assert!(inner[1] > outer[1] && inner[3] < outer[3], "{inner:?}");
        // …and the whole thing is on the screen, near the bottom of it.
        assert!(outer[0] >= 0.0 && outer[2] <= 1.0);
        assert!(outer[1] >= 0.0 && outer[3] < 0.25, "{outer:?}");
    }

    /// A progress outside the range cannot draw outside the frame — both ends,
    /// because the client clamps at both.
    #[test]
    fn a_progress_outside_the_range_is_clamped() {
        let [fill, _] = BAR;
        assert_eq!(fill.rect(-1.0), fill.rect(0.0));
        assert_eq!(fill.rect(9.0), fill.rect(1.0));
    }

    /// **The picture keeps its 4:3 shape and the window gets the bars.**
    ///
    /// The number is the whole finding: the client's `r` is the aspect times
    /// `0.75`, so the box is 4:3 and not — as a first reading of the same three
    /// branches had it — a square. Both are plausible (the art *is* 512x512),
    /// which is why the test names the ratio rather than the shape.
    #[test]
    fn the_screen_is_a_centred_four_by_three_box_whatever_the_window_is() {
        // A 4:3 window is the whole window, exactly — the `== 1` branch, which
        // does nothing at all.
        assert_eq!(fit(TARGET_ASPECT), [0.0, 0.0, 1.0, 1.0]);

        // 16:9 pillarboxes: (4/3)/(16/9) = 0.75 of the width, centred.
        let [left, bottom, right, top] = fit(16.0 / 9.0);
        assert!((right - left - 0.75).abs() < 1e-6, "{left}..{right}");
        assert!((left - 0.125).abs() < 1e-6, "{left}");
        assert_eq!((bottom, top), (0.0, 1.0));

        // …and a window taller than 4:3 letterboxes, which is the same rule on
        // the other axis rather than a second one. 1:1 -> 0.75 of the height.
        let [left, bottom, right, top] = fit(1.0);
        assert_eq!((left, right), (0.0, 1.0));
        assert!((top - bottom - 0.75).abs() < 1e-6);
        assert!((bottom - 0.125).abs() < 1e-6);
    }

    /// It is always 4:3, at every aspect a window can have — the property the
    /// three branches exist to hold, and the one a transposed branch would
    /// break silently.
    #[test]
    fn the_fitted_region_is_four_by_three_at_every_aspect() {
        for aspect in [0.25, 0.5, 1.0, 1.333, 4.0 / 3.0, 1.334, 16.0 / 9.0, 3.2, 8.0] {
            let [left, bottom, right, top] = fit(aspect);
            let drawn = ((right - left) * aspect) / (top - bottom);
            assert!(
                (drawn - TARGET_ASPECT).abs() < 1e-4,
                "aspect {aspect}: drawn at {drawn}"
            );
            // …and it is inside the window, centred.
            assert!(left >= 0.0 && right <= 1.0 && bottom >= 0.0 && top <= 1.0);
            assert!(((left + right) - 1.0).abs() < 1e-6);
            assert!(((bottom + top) - 1.0).abs() < 1e-6);
        }
    }

    /// A window with no pixels answers the whole of itself rather than a NaN
    /// rectangle, which is what a painter would otherwise be handed on the
    /// frame a window is minimised.
    #[test]
    fn a_window_with_no_aspect_is_not_a_nan_rectangle() {
        for aspect in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(fit(aspect), [0.0, 0.0, 1.0, 1.0], "{aspect}");
        }
    }
}
