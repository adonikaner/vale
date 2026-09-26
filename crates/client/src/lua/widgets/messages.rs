//! **The frames that hold lines of text**: `MessageFrame` and
//! `ScrollingMessageFrame`.
//!
//! `AddMessage` is called **49 times** by the directory, which made it the fifth
//! most-called widget method this client owed — and the two that matter are the
//! two the player reads:
//!
//! ```lua
//! UIErrorsFrame:AddMessage(message, 1.0, 0.1, 0.1, 1.0);   -- "You are too far away!"
//! DEFAULT_CHAT_FRAME:AddMessage(string, info.r, info.g, info.b, info.id);
//! ```
//!
//! ## A line has no widget, so it is drawn as one
//!
//! The lines inside a message frame have no objects: the C widget lays them out
//! itself, in the style of the **unnamed `<FontString>`** declared inside the
//! frame. `UIErrorsFrame`'s is `inherits="ErrorFont" justifyH="CENTER"`, and that
//! is the whole of what says the error text is large, centred and in Friz
//! Quadrata.
//!
//! So the draw pass reads the style off that child and emits one ordinary
//! [`super::regions::Paint`] per line — which means every line of chat and every
//! error goes through the *same* text path as a `<FontString>`, with no second
//! way to draw a word in this client. See [`lines`].
//!
//! ## `displayDuration` fades a line; it does not delete one
//!
//! `<MessageFrame displayDuration="5" insertMode="TOP">`: a line is fully
//! visible for five seconds and new ones arrive at the top. **What happens at
//! the end of those five seconds is a fade, and the line stays in the buffer** —
//! which this module had as a deletion, and which is the whole of the "old chat
//! text is deleted far too quickly" report: two minutes after logging in the
//! chat was empty and nothing could scroll back to it.
//!
//! The rules are the widget's own per-frame tick:
//!
//! * each line carries **its own copy** of `timeVisible` and `fadeDuration`,
//!   stamped from the frame's at `AddMessage`;
//! * `timeVisible` counts down first; then `fadeDuration` does, and the line's
//!   alpha is `255 * fadeRemaining / fadeDuration` — a linear ramp;
//! * when the ramp reaches nothing the line's font string is **hidden** and the
//!   record stays in the array. Only `Clear` and the [`MAX_LINES`] bound ever
//!   remove one;
//! * the walk is refused outright unless the frame is **at the live end**
//!   (`AtBottom`), so a chat that has been scrolled back does not fade at all;
//! * and a scroll press re-arms it. `ScrollUp` at the top, `ScrollDown` and
//!   `ScrollToBottom` at the bottom put every line back to full alpha with
//!   fresh clocks and **return without moving the index** — the press is spent
//!   bringing the text back. A press that *does* move goes through the same
//!   re-arming, one line at a time.
//!
//! The defaults are the widget's own, not this module's: a scrolling message
//! frame ships **fading on** with [`TIME_VISIBLE_SECS`] and [`FADE_SECS`].
//! `ChatFrameTemplate`
//! overrides the first to 120 and leaves the second alone; `RaidWarningFrame`
//! declares neither and therefore takes both.
//!
//! **A plain `MessageFrame` is a different class** — `UIErrorsFrame` and the two
//! raid frames — and only the *clocks* above are checked against it: its own
//! constructor reads the same two defaults into its own fields, and its
//! `displayDuration`/`fadeDuration` markup goes to its own setters. Whether it *deletes* a
//! faded-out line rather than merely hiding it is not established, and it
//! decides nothing here: a message frame cannot be scrolled, so a line nothing
//! can get back to and a line that is gone look the same.
//!
//! ## A frame holds what fits in it, and the rest is scrolled to
//!
//! **The frame's height is a bound, not a suggestion.** `ChatFrame1` is 120
//! units tall and its lines are 16.1 apart (Arial Narrow at 14), so it shows
//! **seven** of them and the eighth is above the window rather than below the
//! frame. Drawing every line a frame holds put the chat through the edit box,
//! through the action bar and off the bottom of the screen — which is what it
//! looked like, and which is the whole reason [`view`] exists.
//!
//! Which end they stack from is the `insertMode` again: the error frame's newest
//! line is at the **top** and pushes the rest down, and the chat's is at the
//! **bottom**, growing upwards into a frame that is mostly empty when only two
//! things have been said. So a top-inserting frame's block is top-aligned and a
//! bottom-inserting one's is bottom-aligned, and neither ever leaves its own
//! rectangle.
//!
//! …and what is above the window is reachable: `ScrollUp` moves the window one
//! line back through the history, `ScrollDown` one forward, `PageUp`/`PageDown`
//! a windowful, `ScrollToBottom` back to the live end. Those are the game's own
//! names — `ChatFrame1UpButton`'s `OnClick` is `this:GetParent():ScrollUp()` —
//! and the offset is clamped at both ends by however much history there is to
//! show, which is what `AtBottom` answers about.
//!
//! ## …and a line wider than the frame folds
//!
//! **The budget is rows and the history is lines**, which is the whole of the
//! wrap and is why it was this file's geometry rather than a call: a chat line
//! that folds into three occupies three of `ChatFrame1`'s seven rows and pushes
//! two others off the top. [`window`] walks back from the live end adding folded
//! heights until the budget is spent, and a line that would not fit whole is not
//! drawn at all — half a folded line hanging out of the top of the frame is the
//! same artefact one row up.
//!
//! The fold itself is the painter's, through the same [`super::regions::Paint`]
//! `wrap` flag a `<FontString>` with a declared width uses, at the same width
//! this measured — so the rows reserved and the rows drawn cannot disagree. Two
//! things follow that are worth stating: a line is measured with its `|c`
//! escapes cut out (which is what the reader sees), and a single word longer
//! than the frame is charged `ceil(width / frame)` rows where the painter breaks
//! it at whatever glyph crosses the edge.
//!
//! ## What is not modelled
//!
//! * **the per-line clock does not freeze while the frame is scrolled**, it is
//!   restarted. The widget stops ticking and re-arms whatever a relayout lays
//!   out, which for every path comes to the
//!   same thing: everything is at full alpha while scrolled and the countdown
//!   begins again on the way back to the live end. The difference would show
//!   only in a frame scrolled back and forth inside one line's lifetime.
//! * **`SetFading`, `SetTimeVisible` and `SetFadeDuration`** are real widget
//!   methods and neither shipped directory calls one, so they are markup-only
//!   here. See [`METHODS`], which is the set the directory does call.

use super::regions::Paint;

/// The methods a message frame carries. Sorted; each a name the directory calls.
pub const METHODS: [&str; 16] = [
    "AddMessage",
    "AtBottom",
    "AtTop",
    "Clear",
    "GetNumMessages",
    "PageDown",
    "PageUp",
    "ScrollDown",
    "ScrollToBottom",
    "ScrollToTop",
    "ScrollUp",
    "SetFadeDuration",
    "SetFading",
    "SetMaxLines",
    "SetTimeVisible",
    "UpdateColorByID",
];

/// Where the lines live, and the attributes that shape them.
const LINES_KEY: &str = "__messages";
const DURATION_KEY: &str = "__displayDuration";
/// `fadeDuration` — the second half of [`DURATION_KEY`], and an attribute no
/// shipped file declares, which is why the *default* is the number that decides
/// what the chat looks like.
const FADE_KEY: &str = "__fadeDuration";
const INSERT_TOP_KEY: &str = "__insertTop";
/// `pub(super)` because a **`<FontString>`** declares it too, and means
/// something different by it: on a message frame it is how many lines are kept,
/// and on a font string it is how many rows a wrapped one may fold into
/// (`$parentSpellName` is `maxLines="3"`). Same attribute, same loader path —
/// see [`super::regions::intrinsic`], which is the only other reader.
pub(super) const MAX_LINES_KEY: &str = "__maxLines";
/// How many lines back from the live end the window is — 0 is "showing the
/// newest", which is where a frame sits until something scrolls it.
const SCROLL_KEY: &str = "__scroll";

/// How many lines a frame keeps when nothing said otherwise.
///
/// The widget takes its own bound as a constructor argument rather
/// than from a constant, so there is no default to quote; every message frame in
/// either shipped directory either declares `maxLines="128"` — which is
/// `ChatFrameTemplate`, and therefore all seven chat frames — or holds a handful
/// of lines at a time. This is that number, and it is what makes the buffer the
/// thing that eventually drops a line now that the clock does not.
const MAX_LINES: usize = 128;

/// How long a line is fully visible when the markup declares no
/// `displayDuration` — the scrolling message frame's own default.
pub const TIME_VISIBLE_SECS: f64 = 10.0;

/// …and how long it then takes to fade to nothing.
///
/// No file in either directory declares a `fadeDuration`, so this is the ramp
/// every faded line in the game takes, the chat's two-minute one included.
pub const FADE_SECS: f64 = 3.0;

/// One line, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub colour: [f32; 4],
    /// **How far through its fade it is**: 1.0 while it is inside its
    /// `displayDuration`, ramping to 0.0 across the `fadeDuration` after it, and
    /// 0.0 for a line that has faded out and is being kept only so that
    /// scrolling can bring it back. Multiplies the colour's own alpha.
    pub alpha: f32,
}

/// **Every line a message frame is holding**, newest first when the frame
/// inserts at the top, each with how far through its fade it is.
///
/// `now` is the clock [`super::super::host::LuaHost::drawn`] stamps each frame. Nothing
/// is dropped for being old — a faded line comes back the moment the frame is
/// scrolled, which is the widget's own behaviour and the point of the whole
/// module doc above. Returns an empty list — and reads one table key — for the
/// 3,743 frames that are not message frames.
pub fn lines(frame: &mlua::Table, now: f64) -> Vec<Line> {
    let Ok(Some(stored)) = frame.raw_get::<Option<mlua::Table>>(LINES_KEY) else {
        return Vec::new();
    };
    // **A frame that has been scrolled back does not fade**, so there is nothing
    // to compute for one: the widget refuses the whole walk unless `AtBottom`.
    let fading = scroll(frame) == 0;
    let visible: f64 = frame.raw_get::<Option<f64>>(DURATION_KEY).ok().flatten().unwrap_or(TIME_VISIBLE_SECS);
    let fade: f64 = frame.raw_get::<Option<f64>>(FADE_KEY).ok().flatten().unwrap_or(FADE_SECS);
    let mut out = Vec::new();
    for entry in stored.sequence_values::<mlua::Table>().flatten() {
        let alpha = if fading {
            let at: f64 = entry.get("at").unwrap_or(0.0);
            faded(now - at, visible, fade)
        } else {
            1.0
        };
        let colour: Vec<f64> = entry.get("colour").unwrap_or_default();
        out.push(Line {
            text: entry.get::<Option<String>>("text").ok().flatten().unwrap_or_default(),
            colour: [
                colour.first().copied().unwrap_or(1.0) as f32,
                colour.get(1).copied().unwrap_or(1.0) as f32,
                colour.get(2).copied().unwrap_or(1.0) as f32,
                colour.get(3).copied().unwrap_or(1.0) as f32,
            ],
            alpha,
        });
    }
    // **The newest is nearest the insert edge.** Stored oldest-first, so a
    // top-inserting frame reads back reversed and a bottom-inserting one does
    // not — which is the difference between the error frame (newest at the top,
    // pushing the rest down) and the chat (newest at the bottom).
    if frame.raw_get::<Option<bool>>(INSERT_TOP_KEY).ok().flatten().unwrap_or(false) {
        out.reverse();
    }
    out
}

/// **The ramp**: full alpha for `visible` seconds, then linear to nothing across
/// `fade`.
///
/// The widget computes it as `255 * fadeRemaining / fadeDuration`, with two
/// `fade <= 0` branches — a line with no fade
/// to run is hidden the instant its time is up, with no ramp at all, which is
/// what a `fadeDuration="0"` would mean.
fn faded(age: f64, visible: f64, fade: f64) -> f32 {
    if age <= visible {
        return 1.0;
    }
    if fade <= 0.0 {
        return 0.0;
    }
    (1.0 - (age - visible) / fade).clamp(0.0, 1.0) as f32
}

/// The style the lines are drawn in: the frame's own unnamed `<FontString>`.
///
/// `None` for a frame that declares none, which is a message frame that would
/// draw nothing in the real client either.
pub fn style(frame: &mlua::Table) -> Option<Paint> {
    // The same declaration an `EditBox` uses for the line being typed into it,
    // which is why the reader is next door — see [`super::regions::own_font`].
    super::regions::own_font(frame)
}

/// How far apart to stack the lines: **the face's own line box**, measured off
/// the game's own `.TTF` — see [`super::text::line_height`].
///
/// It was 1.2 times the declared height, which is the ordinary ratio for a
/// TrueType face and was this file's one stated assumption. The measurement
/// disagrees in both directions — Friz Quadrata's line box is 1.215 of its em
/// and Arial Narrow's is 1.148 — and it is the chat, set in the second of
/// those, that stacks the most lines against the assumption.
pub fn spacing(lua: &mlua::Lua, style: &Paint) -> f32 {
    let height = if style.font_height > 0.0 {
        f64::from(style.font_height)
    } else {
        f64::from(vale_assets::interface::font::DEFAULT_HEIGHT)
    };
    super::text::line_height(lua, style.font.as_deref(), height) as f32
}

/// **How many lines this frame has room for**, which is what makes its height a
/// bound rather than a suggestion.
///
/// At least one: a frame too short for a single line still shows the newest,
/// which is the useful failure. `None` for a frame that is not a message frame,
/// has no style to lay lines out in, or has no solved rectangle yet.
pub fn rows(lua: &mlua::Lua, frame: &mlua::Table) -> Option<usize> {
    let style = style(frame)?;
    let rect = super::layout::rect(lua, frame)?;
    let spacing = f64::from(spacing(lua, &style));
    if spacing <= 0.0 {
        return None;
    }
    Some(((rect.height / spacing).floor() as usize).max(1))
}

/// **A line as it will be drawn**: the line, and how many rows it folds into at
/// this frame's own width.
///
/// The second number is the whole of the wrap. Everything else about folding is
/// the painter's — see [`super::regions::Paint::wrap`], which is the same
/// mechanism a `<FontString>` with a declared width uses, measured by the same
/// [`super::text::rows`] so the height reserved here and the galley drawn there
/// agree by construction.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub line: Line,
    /// At least 1, and more for a line that is wider than the frame.
    pub rows: usize,
}

/// **What a message frame is showing**, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub lines: Vec<Row>,
    /// `insertMode="TOP"` — the block hangs from the frame's top edge rather
    /// than standing on its bottom one.
    pub from_top: bool,
    /// How many rows the block occupies in total, which is what the bottom-up
    /// frames measure their stack from.
    pub rows: usize,
}

/// **The window of lines a frame is showing**, top to bottom, and which edge it
/// is stuck to.
///
/// `rows` is what fits (see [`rows`]); the scroll offset says how far back
/// through the history the window has been moved, and is clamped here rather
/// than trusted — lines expire and the buffer is bounded, so an offset that was
/// legal when it was set may be past the end by the time it is read.
pub fn view(lua: &mlua::Lua, frame: &mlua::Table, now: f64) -> Option<Shown> {
    window(lua, lines(frame, now), frame)
}

/// …and the same slice over lines the caller has already read.
///
/// The draw pass gates on "does this frame hold any lines at all" — one table
/// read for the 3,743 frames that are not message frames — and reading them
/// twice to answer it would allocate the chat's whole buffer a second time
/// every frame of video.
///
/// **The budget is rows and the offset is lines**, which is not an inconsistency
/// but the two things the game itself counts in: a frame's height divided by its
/// line box is how many *rows* fit, and `ScrollUp` moves the window by one
/// *message*. A line that folds into three therefore pushes two others off the
/// screen, which is exactly what the real chat does and what this client used to
/// do by drawing over the action bar instead.
pub fn window(lua: &mlua::Lua, all: Vec<Line>, frame: &mlua::Table) -> Option<Shown> {
    let style = style(frame)?;
    let rect = super::layout::rect(lua, frame)?;
    let spacing = f64::from(spacing(lua, &style));
    if spacing <= 0.0 {
        return None;
    }
    // At least one row: a frame too short for a line still shows the newest,
    // which is the useful failure. See [`rows`], whose arithmetic this is.
    let budget = ((rect.height / spacing).floor() as usize).max(1);
    let from_top = inserts_at_top(frame);
    // **Always at least the newest line.** The offset is in lines and the
    // budget in rows, so there is no exact "how far back may this go" to clamp
    // against without folding the whole history; showing the last line is the
    // bound that matters and it is the one a reader can always get back to with
    // `ScrollToBottom`.
    let offset = scroll(frame).min(all.len().saturating_sub(1));

    let mut taken: Vec<Row> = Vec::new();
    let mut used = 0usize;
    // Newest first, whichever end that is: a top-inserting frame's list is
    // already reversed by [`lines`].
    let newest_first: Box<dyn Iterator<Item = &Line>> = if from_top {
        Box::new(all.iter())
    } else {
        Box::new(all.iter().rev())
    };
    for line in newest_first.skip(offset) {
        let rows = fold(lua, &style, &line.text, rect.width);
        // **A line that does not fit is not shown at all**, unless it is the
        // only one — half a folded line drawn out of the top of the frame is
        // the artefact this whole function exists to stop.
        if !taken.is_empty() && used + rows > budget {
            break;
        }
        used += rows;
        taken.push(Row { line: line.clone(), rows });
        if used >= budget {
            break;
        }
    }
    // Collected newest-first; a bottom-inserting frame draws oldest at the top.
    if !from_top {
        taken.reverse();
    }
    Some(Shown { lines: taken, from_top, rows: used })
}

/// How many rows one line folds into at `width`.
///
/// **The `|c` escapes are cut out first** — that is [`super::text::rows`]'s own
/// behaviour, and it is what makes a chat line fold at the words the reader can
/// see rather than at the thirty-two characters of `|Hplayer:Bram|h[Bram]|h`.
fn fold(lua: &mlua::Lua, style: &Paint, text: &str, width: f64) -> usize {
    let height = if style.font_height > 0.0 {
        f64::from(style.font_height)
    } else {
        f64::from(vale_assets::interface::font::DEFAULT_HEIGHT)
    };
    super::text::rows(lua, style.font.as_deref(), height, text, width)
}

/// Whether the newest line is at the top — `insertMode="TOP"`.
fn inserts_at_top(frame: &mlua::Table) -> bool {
    frame
        .raw_get::<Option<bool>>(INSERT_TOP_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// How many lines back from the live end this frame is showing.
fn scroll(frame: &mlua::Table) -> usize {
    frame
        .raw_get::<Option<i64>>(SCROLL_KEY)
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0) as usize
}

/// Move the window, clamped to what there is to show.
///
/// `by` is in lines and positive is *back through the history*, which is what
/// `ScrollUp` means on both kinds of frame — the direction on the screen
/// differs between them and the direction through time does not.
fn scroll_by(lua: &mlua::Lua, frame: &mlua::Table, by: i64) -> mlua::Result<()> {
    // **Every scroll press brings the faded text back**, whether or not it also
    // moves the window: a press that cannot move is spent re-arming every
    // line, and a press that does move re-arms each line the relayout lays
    // out. Both come out here as one restamp.
    restamp(lua, frame)?;
    let held = held(frame) as i64;
    // The rectangle may not solve (a frame the loader has not anchored yet), in
    // which case one line is the honest bound — it keeps the offset from
    // running away rather than pretending to know the height.
    let rows = rows(lua, frame).unwrap_or(1) as i64;
    let limit = (held - rows).max(0);
    let to = (scroll(frame) as i64 + by).clamp(0, limit);
    frame.set(SCROLL_KEY, to)
}

/// How many lines the frame is holding, faded ones included.
fn held(frame: &mlua::Table) -> usize {
    frame
        .raw_get::<Option<mlua::Table>>(LINES_KEY)
        .ok()
        .flatten()
        .map_or(0, |stored| stored.raw_len())
}

/// **Put every line back to full alpha with a fresh clock**, which is where a
/// scroll press on a faded frame goes.
fn restamp(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let Some(stored) = frame.raw_get::<Option<mlua::Table>>(LINES_KEY)? else {
        return Ok(());
    };
    let now = now(lua);
    for entry in stored.sequence_values::<mlua::Table>().flatten() {
        entry.set("at", now)?;
    }
    Ok(())
}

/// `displayDuration`, `fadeDuration`, `insertMode` and `maxLines` from the
/// loader.
pub(in crate::lua) fn set_from_markup(
    frame: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "displayDuration" => match value.parse::<f64>() {
            Ok(seconds) => frame.set(DURATION_KEY, seconds),
            Err(_) => Ok(()),
        },
        "fadeDuration" => match value.parse::<f64>() {
            Ok(seconds) => frame.set(FADE_KEY, seconds),
            Err(_) => Ok(()),
        },
        "insertMode" => frame.set(INSERT_TOP_KEY, value.eq_ignore_ascii_case("TOP")),
        "maxLines" => match value.parse::<usize>() {
            Ok(lines) => frame.set(MAX_LINES_KEY, lines),
            Err(_) => Ok(()),
        },
        _ => Ok(()),
    }
}

/// Install the message-frame methods onto the shared frame method table.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // **The fade settings — recorded, and read by nothing yet**: this
    // client's lines do not fade. `SetFading(false)` is what a chat addon
    // calls first, and a nil method there takes the whole chat setup down.
    for (name, key) in [
        ("SetFading", "__fading"),
        ("SetTimeVisible", "__timeVisible"),
        ("SetFadeDuration", "__fadeDuration"),
    ] {
        let set = lua.create_function(move |_lua, (this, value): (mlua::Table, mlua::Value)| this.set(key, value))?;
        methods.set(name, set)?;
    }
    let add = lua.create_function(
        |lua,
         (this, text, r, g, b, id): (
            mlua::Table,
            mlua::Value,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
        )| {
            // **A number is a message too.** `AddMessage(1)` is not in the
            // shipped directory, but `SetText` had the same trap and the same
            // answer: 1.12 stringifies whatever it is given.
            let text = match text {
                mlua::Value::String(s) => s.to_string_lossy(),
                mlua::Value::Integer(n) => n.to_string(),
                mlua::Value::Number(n) => format!("{n}"),
                _ => return Ok(()),
            };
            let entry = lua.create_table()?;
            entry.set("text", text)?;
            entry.set(
                "colour",
                lua.create_sequence_from([
                    r.unwrap_or(1.0),
                    g.unwrap_or(1.0),
                    b.unwrap_or(1.0),
                    1.0,
                ])?,
            )?;
            entry.set("at", now(lua))?;
            // **The fifth argument, which is not decoration.** Every chat line
            // the directory adds carries `info.id` — the chat *type*'s own
            // number — and `UpdateColorByID` below recolours the history by
            // it, which is how changing the colour of whispers changes the
            // whispers already on screen. A line with no id (an error message,
            // a tooltip line) simply matches nothing.
            if let Some(id) = id {
                entry.set("id", id)?;
            }
            let stored = match this.raw_get::<Option<mlua::Table>>(LINES_KEY)? {
                Some(stored) => stored,
                None => {
                    let made = lua.create_table()?;
                    this.set(LINES_KEY, made.clone())?;
                    made
                }
            };
            stored.push(entry)?;
            // **A frame that has been scrolled back stays where it was put.**
            // The offset is counted from the live end, so a line arriving at
            // that end would otherwise slide the window forward and change what
            // is on screen under the reader — which is precisely what a person
            // who scrolled up is trying to stop. The real client flashes its
            // bottom button instead, and `AtBottom` is what it asks.
            if scroll(&this) > 0 {
                this.set(SCROLL_KEY, scroll(&this) as i64 + 1)?;
            }
            let cap = this
                .raw_get::<Option<usize>>(MAX_LINES_KEY)?
                .unwrap_or(MAX_LINES)
                .max(1);
            // **Bounded, because a chat frame is unbounded otherwise.** The
            // oldest goes; `raw_remove(1)` is O(n) and n is at most `cap`.
            while stored.raw_len() > cap {
                stored.raw_remove(1)?;
            }
            Ok(())
        },
    )?;
    methods.set("AddMessage", add)?;

    let clear = lua.create_function(|lua, this: mlua::Table| {
        this.set(SCROLL_KEY, 0_i64)?;
        this.set(LINES_KEY, lua.create_table()?)
    })?;
    methods.set("Clear", clear)?;

    // **Recolour the history**, which is what makes a chat colour a *setting*
    // rather than a property of the moment a line arrived. `ChatFrame_OnEvent`
    // calls it on every `UPDATE_CHAT_COLOR` — including the ninety-four this
    // client raises at load, which is where the audit found it missing.
    let update_by_id = lua.create_function(
        |_lua, (this, id, r, g, b): (mlua::Table, f64, f64, f64, f64)| {
            let Some(stored) = this.raw_get::<Option<mlua::Table>>(LINES_KEY)? else {
                return Ok(());
            };
            for entry in stored.sequence_values::<mlua::Table>().flatten() {
                if entry.get::<Option<f64>>("id")? != Some(id) {
                    continue;
                }
                // The alpha is the line's own and is deliberately not touched:
                // a fading `UIErrorsFrame` line keeps its fade.
                let alpha = entry
                    .get::<Vec<f64>>("colour")
                    .ok()
                    .and_then(|c| c.get(3).copied())
                    .unwrap_or(1.0);
                entry.set("colour", vec![r, g, b, alpha])?;
            }
            Ok(())
        },
    )?;
    methods.set("UpdateColorByID", update_by_id)?;

    // **The window through the history** — the game's own six names, and the
    // arrows down the left of every chat frame call the first two.
    //
    // A page is whatever fits *now*, which is the real client's behaviour: a
    // resized chat frame pages by its new height rather than by the height it
    // was declared at.
    let up = lua.create_function(|lua, this: mlua::Table| scroll_by(lua, &this, 1))?;
    methods.set("ScrollUp", up)?;
    let down = lua.create_function(|lua, this: mlua::Table| scroll_by(lua, &this, -1))?;
    methods.set("ScrollDown", down)?;
    let page_up = lua.create_function(|lua, this: mlua::Table| {
        let page = rows(lua, &this).unwrap_or(1) as i64;
        scroll_by(lua, &this, page)
    })?;
    methods.set("PageUp", page_up)?;
    let page_down = lua.create_function(|lua, this: mlua::Table| {
        let page = rows(lua, &this).unwrap_or(1) as i64;
        scroll_by(lua, &this, -page)
    })?;
    methods.set("PageDown", page_down)?;
    // …and this one is the press the reader makes most: `ChatFrameTemplate`'s
    // own bottom button. It restamps like the rest, which is what turns a chat
    // that has faded out back on.
    let to_bottom = lua.create_function(|lua, this: mlua::Table| {
        restamp(lua, &this)?;
        this.set(SCROLL_KEY, 0_i64)
    })?;
    methods.set("ScrollToBottom", to_bottom)?;
    // `i64::MAX` rather than the count, because [`scroll_by`] clamps to
    // whatever there is — one rule for the limit rather than two.
    let to_top = lua.create_function(|lua, this: mlua::Table| scroll_by(lua, &this, i64::MAX))?;
    methods.set("ScrollToTop", to_top)?;
    // …and the two questions the arrows' own `OnUpdate` asks about them. 1.12's
    // truth for a boolean: 1 or nil, never `false` — see [`super::super::api`].
    let at_bottom =
        lua.create_function(|_lua, this: mlua::Table| Ok(super::super::api::one_or_nil(scroll(&this) == 0)))?;
    methods.set("AtBottom", at_bottom)?;
    let at_top = lua.create_function(|lua, this: mlua::Table| {
        let held = held(&this) as i64;
        let page = rows(lua, &this).unwrap_or(1) as i64;
        Ok(super::super::api::one_or_nil(scroll(&this) as i64 >= (held - page).max(0)))
    })?;
    methods.set("AtTop", at_top)?;
    let count = lua.create_function(|_lua, this: mlua::Table| Ok(held(&this)))?;
    methods.set("GetNumMessages", count)?;
    let set_max = lua.create_function(|_lua, (this, lines): (mlua::Table, Option<usize>)| {
        this.set(MAX_LINES_KEY, lines.unwrap_or(MAX_LINES).max(1))
    })?;
    methods.set("SetMaxLines", set_max)?;
    Ok(())
}

/// The registry key the per-frame clock lives under — see [`set_now`].
const REG_NOW: &str = "vale.now";

/// **One clock for the whole interface**, stamped once a frame.
///
/// `AddMessage` needs to know when a line arrived and it is not a scoped read —
/// it is a method on an object, reachable from an event handler, and `GetTime`
/// only exists inside [`super::super::api`]'s scope. Rather than give this module a
/// second notion of time, the frame's own `GetTime` value is written here at the
/// top of each draw and read back by whatever needs it.
pub fn set_now(lua: &mlua::Lua, now: f64) {
    let _ = lua.set_named_registry_value(REG_NOW, now);
}

/// …and read it. `0.0` before anything set one, which makes every line
/// arbitrarily old rather than arbitrarily new — the safe direction, since a
/// message that never expires stays on the screen for ever.
pub fn now(lua: &mlua::Lua) -> f64 {
    lua.named_registry_value::<Option<f64>>(REG_NOW)
        .ok()
        .flatten()
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::frames;

    fn state() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        lua.load(r#"errors = CreateFrame("MessageFrame", "Errors");"#)
            .exec()
            .expect("the frame is created");
        lua
    }

    /// **`UIErrorsFrame_OnEvent`'s own line**, and its colour.
    #[test]
    fn a_message_is_kept_with_the_colour_it_arrived_in() {
        let lua = state();
        lua.load(r#"errors:AddMessage("You are too far away!", 1.0, 0.1, 0.1, 1.0);"#)
            .exec()
            .expect("runs");
        let frame: mlua::Table = lua.globals().get("Errors").unwrap();
        let lines = lines(&frame, 0.0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "You are too far away!");
        assert_eq!(lines[0].colour, [1.0, 0.1, 0.1, 1.0]);
    }

    /// **`displayDuration` fades a line and does not delete it** — the reported
    /// bug, which was that two minutes after logging in the chat was empty and
    /// nothing could scroll back to what had been said.
    ///
    /// The ramp is the widget's: full alpha until the duration is up, then
    /// linear to nothing across [`FADE_SECS`], and the line stays in the buffer
    /// at the far end of it.
    #[test]
    fn a_line_fades_rather_than_expiring_and_is_kept_either_way() {
        let lua = state();
        let frame: mlua::Table = lua.globals().get("Errors").unwrap();
        set_now(&lua, 100.0);
        set_from_markup(&frame, "displayDuration", "5").expect("sets");
        lua.load(r#"errors:AddMessage("fades");"#).exec().expect("runs");

        let alpha = |now: f64| lines(&frame, now)[0].alpha;
        assert_eq!(alpha(104.0), 1.0, "inside its time, fully visible");
        assert_eq!(alpha(105.0), 1.0, "and at the edge of it");
        assert_eq!(alpha(106.5), 0.5, "half way through the three-second ramp");
        assert_eq!(alpha(108.0), 0.0, "faded out");
        assert_eq!(alpha(600.0), 0.0, "…and stays that way rather than clipping");
        assert_eq!(lines(&frame, 600.0).len(), 1, "the line is still held");

        // A declared `fadeDuration` of nothing is the widget's other branch:
        // the line is gone the instant its time is up, with no ramp.
        set_from_markup(&frame, "fadeDuration", "0").expect("sets");
        assert_eq!(alpha(105.0), 1.0);
        assert_eq!(alpha(105.1), 0.0);
    }

    /// **The defaults are the widget's**, not this module's: a frame that
    /// declares no duration at all still fades, at ten seconds and three.
    /// `RaidWarningFrame` is the shipped example.
    #[test]
    fn a_frame_that_declares_nothing_takes_the_widgets_own_clocks() {
        let lua = state();
        let frame: mlua::Table = lua.globals().get("Errors").unwrap();
        set_now(&lua, 0.0);
        lua.load(r#"errors:AddMessage("warning");"#).exec().expect("runs");
        assert_eq!(lines(&frame, TIME_VISIBLE_SECS)[0].alpha, 1.0);
        assert_eq!(
            lines(&frame, TIME_VISIBLE_SECS + FADE_SECS / 2.0)[0].alpha,
            0.5
        );
        assert_eq!(lines(&frame, TIME_VISIBLE_SECS + FADE_SECS)[0].alpha, 0.0);
    }

    /// **A scroll press brings the faded text back**, and a frame that has
    /// been scrolled away from the live end does not fade at all.
    #[test]
    fn scrolling_restores_a_faded_frame_and_a_scrolled_one_does_not_fade() {
        let lua = state();
        let frame = chat(&lua);
        set_now(&lua, 0.0);
        say(&lua, 20);
        let faded_now = TIME_VISIBLE_SECS + FADE_SECS + 1.0;
        assert!(lines(&frame, faded_now).iter().all(|line| line.alpha == 0.0));
        assert_eq!(
            view(&lua, &frame, faded_now).expect("a window").lines.len(),
            6,
            "the rows are still reserved; the draw pass steps over them"
        );

        // The press is spent on bringing it back: the window has not moved.
        set_now(&lua, faded_now);
        let before = showing(&lua, &frame);
        lua.load("chat:ScrollToBottom()").exec().expect("runs");
        assert!(lines(&frame, faded_now).iter().all(|line| line.alpha == 1.0));
        assert_eq!(showing(&lua, &frame), before);

        // …and scrolled back, nothing fades however long it has been.
        lua.load("chat:ScrollUp()").exec().expect("runs");
        assert!(lines(&frame, faded_now + 600.0)
            .iter()
            .all(|line| line.alpha == 1.0));
    }

    /// **`insertMode="TOP"` reverses the reading order** — the newest error is
    /// at the top and pushes the rest down, where the newest line of chat is at
    /// the bottom.
    #[test]
    fn the_insert_mode_decides_which_end_the_newest_line_is() {
        let lua = state();
        let frame: mlua::Table = lua.globals().get("Errors").unwrap();
        lua.load(r#"errors:AddMessage("first"); errors:AddMessage("second");"#)
            .exec()
            .expect("runs");
        assert_eq!(lines(&frame, 0.0)[0].text, "first");
        set_from_markup(&frame, "insertMode", "TOP").expect("sets");
        assert_eq!(lines(&frame, 0.0)[0].text, "second");
    }

    /// A chat frame is otherwise unbounded, so the oldest line goes.
    #[test]
    fn the_buffer_is_bounded_from_the_old_end() {
        let lua = state();
        let frame: mlua::Table = lua.globals().get("Errors").unwrap();
        lua.load("errors:SetMaxLines(3); for i = 1, 5 do errors:AddMessage(\"line\" .. i); end")
            .exec()
            .expect("runs");
        let lines = lines(&frame, 0.0);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "line3");
        assert_eq!(lines[2].text, "line5");
    }

    /// A chat-shaped frame: on the screen, a real height, and the unnamed
    /// `<FontString>` its lines are laid out in. With no faces loaded the line
    /// box is 1.2 times the height, so 100 units of frame is **six** rows.
    fn chat(lua: &mlua::Lua) -> mlua::Table {
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            chat = CreateFrame("ScrollingMessageFrame", "Chat", UIParent);
            chat:SetWidth(430); chat:SetHeight(100);
            chat:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 32, 95);
            font = chat:CreateFontString(nil, "ARTWORK");
            "#,
        )
        .exec()
        .expect("the fixture loads");
        let font: mlua::Table = lua.globals().get("font").expect("the style region");
        crate::lua::widgets::regions::set_font_height(&font, 12.0).expect("a height");
        lua.globals().get("Chat").expect("the frame")
    }

    fn say(lua: &mlua::Lua, count: usize) {
        for i in 1..=count {
            lua.load(format!(r#"chat:AddMessage("line{i}")"#))
                .exec()
                .expect("runs");
        }
    }

    /// The lines a frame is showing, in draw order.
    fn showing(lua: &mlua::Lua, frame: &mlua::Table) -> Vec<Line> {
        view(lua, frame, 0.0)
            .expect("a message frame with a solved rectangle")
            .lines
            .into_iter()
            .map(|row| row.line)
            .collect()
    }

    /// **A frame shows what fits in it and no more.** Twenty lines in a frame
    /// six rows tall is six lines — the other fourteen are scrolled to, not
    /// drawn down through the edit box and off the bottom of the screen, which
    /// is what every one of them used to do.
    #[test]
    fn the_window_is_the_frames_own_height() {
        let lua = state();
        let frame = chat(&lua);
        assert_eq!(rows(&lua, &frame), Some(6), "100 units at a 14.4 line box");
        say(&lua, 20);
        let seen = view(&lua, &frame, 0.0).expect("a window");
        assert!(!seen.from_top, "a chat frame's newest line is at the bottom");
        let shown = showing(&lua, &frame);
        assert_eq!(shown.len(), 6);
        assert_eq!(shown[0].text, "line15", "the window is at the live end");
        assert_eq!(shown[5].text, "line20");
        // Fewer lines than rows is every line, still nearest the insert edge.
        lua.load("chat:Clear()").exec().expect("runs");
        say(&lua, 2);
        let shown = showing(&lua, &frame);
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[1].text, "line2");
    }

    /// **The arrows work**, and both ends of the travel are clamped by how much
    /// there is to show. `ChatFrame1UpButton`'s `OnClick` is
    /// `this:GetParent():ScrollUp()`, so this is the whole of what that button
    /// does.
    #[test]
    fn scrolling_moves_the_window_and_stops_at_both_ends() {
        let lua = state();
        let frame = chat(&lua);
        say(&lua, 20);
        let shown = |lua: &mlua::Lua| showing(lua, &frame);

        lua.load("chat:ScrollUp(); chat:ScrollUp()").exec().expect("runs");
        assert_eq!(shown(&lua)[5].text, "line18", "two lines back");
        assert_eq!(eval(&lua, "return chat:AtBottom()"), "Nil");
        lua.load("chat:ScrollDown()").exec().expect("runs");
        assert_eq!(shown(&lua)[5].text, "line19");

        // Down past the live end is refused rather than wrapping.
        for _ in 0..10 {
            lua.load("chat:ScrollDown()").exec().expect("runs");
        }
        assert_eq!(shown(&lua)[5].text, "line20");
        assert_eq!(eval(&lua, "return chat:AtBottom()"), "Integer(1)");

        // …and up past the oldest is the oldest, with the window still full.
        lua.load("chat:ScrollToTop()").exec().expect("runs");
        let top = shown(&lua);
        assert_eq!(top.len(), 6);
        assert_eq!(top[0].text, "line1");
        assert_eq!(eval(&lua, "return chat:AtTop()"), "Integer(1)");
        // A page is a windowful, so one page down from the top is six lines on.
        lua.load("chat:PageDown()").exec().expect("runs");
        assert_eq!(shown(&lua)[0].text, "line7");
        lua.load("chat:ScrollToBottom()").exec().expect("runs");
        assert_eq!(shown(&lua)[5].text, "line20");
    }

    /// **A frame that has been scrolled back stays where it was put** when
    /// something new arrives — the offset is counted from the live end, so a
    /// new line at that end would otherwise slide the window under the reader.
    #[test]
    fn a_new_line_does_not_move_a_scrolled_window() {
        let lua = state();
        let frame = chat(&lua);
        say(&lua, 20);
        lua.load("chat:ScrollUp()").exec().expect("runs");
        let before = showing(&lua, &frame);
        lua.load(r#"chat:AddMessage("fresh")"#).exec().expect("runs");
        assert_eq!(showing(&lua, &frame), before, "the window held still");
        // …and at the live end it follows, which is the ordinary case.
        lua.load("chat:ScrollToBottom()").exec().expect("runs");
        lua.load(r#"chat:AddMessage("newest")"#).exec().expect("runs");
        assert_eq!(showing(&lua, &frame)[5].text, "newest");
    }

    /// The error frame is the other insert mode, and it stacks from the *top*:
    /// its newest line is at the top pushing the rest down, and it too shows
    /// only what fits.
    #[test]
    fn a_top_inserting_frame_fills_from_its_own_edge() {
        let lua = state();
        let frame = chat(&lua);
        set_from_markup(&frame, "insertMode", "TOP").expect("sets");
        // Three rows rather than six, so the two edges are told apart by which
        // lines are dropped as well as by their order.
        lua.load("chat:SetHeight(50)").exec().expect("runs");
        say(&lua, 20);
        let seen = view(&lua, &frame, 0.0).expect("a window");
        assert!(seen.from_top);
        let shown = showing(&lua, &frame);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].text, "line20", "the newest is at the top");
        assert_eq!(shown[2].text, "line18");
    }

    /// **A line wider than the frame folds, and the fold costs rows.**
    ///
    /// The reported bug: a long say ran out through the right edge of
    /// `ChatFrame1` and over whatever was beside it. What it must do instead is
    /// take two of the frame's six rows, which means the line above it leaves.
    #[test]
    fn a_line_wider_than_the_frame_takes_two_rows_and_pushes_one_off() {
        let lua = state();
        let frame = chat(&lua);
        // 430 units wide at height 12. With no faces loaded the fallback
        // measurement is half the height per character, so ~72 characters to
        // the row — this is comfortably two and nowhere near three.
        let long = "a".repeat(60) + " " + &"b".repeat(60);
        say(&lua, 5);
        lua.load(format!(r#"chat:AddMessage("{long}")"#))
            .exec()
            .expect("runs");

        let seen = view(&lua, &frame, 0.0).expect("a window");
        assert_eq!(
            seen.lines.last().expect("the newest line").rows,
            2,
            "the long line folds"
        );
        // Six rows of room, one line taking two: five lines are shown, not six.
        assert_eq!(seen.rows, 6, "the block fills the frame and does not exceed it");
        assert_eq!(seen.lines.len(), 5);
        assert_eq!(seen.lines[0].line.text, "line2", "line1 was pushed off");

        // …and the same line in a frame with no room for both its rows is still
        // shown: the newest line is never dropped for being too tall.
        lua.load("chat:SetHeight(15)").exec().expect("runs");
        let seen = view(&lua, &frame, 0.0).expect("a window");
        assert_eq!(seen.lines.len(), 1);
        assert_eq!(seen.lines[0].rows, 2);
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// Every name in [`METHODS`] is installed, and the list is sorted.
    #[test]
    fn the_list_and_the_installation_are_the_same_set() {
        let lua = state();
        for name in METHODS {
            let kind: String = lua
                .load(format!("return type(errors.{name})"))
                .eval()
                .expect("the probe runs");
            assert_eq!(kind, "function", "{name} is claimed in METHODS and is missing");
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }
}
