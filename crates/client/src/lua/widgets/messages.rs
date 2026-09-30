//! `MessageFrame` and `ScrollingMessageFrame`, the frames that hold lines of
//! text.
//!
//! The directory calls `AddMessage` 49 times, which made it the fifth
//! most-called widget method this client lacked. The two calls the player
//! reads are:
//!
//! ```lua
//! UIErrorsFrame:AddMessage(message, 1.0, 0.1, 0.1, 1.0);   -- "You are too far away!"
//! DEFAULT_CHAT_FRAME:AddMessage(string, info.r, info.g, info.b, info.id);
//! ```
//!
//! ## Lines are drawn as font-string paints
//!
//! The lines inside a message frame have no objects. The 1.12.1 client lays
//! them out in the style of the unnamed `<FontString>` declared inside the
//! frame. `UIErrorsFrame`'s is `inherits="ErrorFont" justifyH="CENTER"`, which
//! is the only thing that makes the error text large, centred and in Friz
//! Quadrata.
//!
//! The draw pass reads the style from that child and emits one ordinary
//! [`super::regions::Paint`] per line, so every line of chat and every error
//! goes through the same text path as a `<FontString>`. See [`lines`].
//!
//! ## `displayDuration` fades a line and does not delete it
//!
//! `<MessageFrame displayDuration="5" insertMode="TOP">`: a line is fully
//! visible for five seconds and new ones arrive at the top. After the five
//! seconds the line fades, and it stays in the buffer. When this module
//! deleted it instead, the chat was empty two minutes after logging in and
//! nothing could be scrolled back to.
//!
//! The 1.12.1 client applies these rules each frame:
//!
//! * each line has its own visible time and fade time, copied from the
//!   frame's settings at `AddMessage`;
//! * the visible time counts down first, then the fade time, and the line's
//!   alpha falls linearly from full to zero across the fade;
//! * a line that has faded out is hidden and stays in the buffer. Only `Clear`
//!   and the [`MAX_LINES`] bound remove a line;
//! * lines fade only while the frame is at the live end (`AtBottom`), so a
//!   chat that has been scrolled back does not fade;
//! * a scroll press restores the lines. `ScrollUp` at the top, and
//!   `ScrollDown` and `ScrollToBottom` at the bottom, return every line to
//!   full alpha with restarted clocks and do not move the offset; the press
//!   only brings the text back. A press that does move the offset restores the
//!   lines the same way.
//!
//! The defaults come from the 1.12.1 client, not this module: a scrolling
//! message frame fades by default, with [`TIME_VISIBLE_SECS`] and
//! [`FADE_SECS`]. `ChatFrameTemplate` overrides the first to 120 and leaves
//! the second; `RaidWarningFrame` declares neither and takes both.
//!
//! A plain `MessageFrame` (`UIErrorsFrame` and the two raid frames) is a
//! different widget type, and only the clocks above are confirmed for it: it
//! uses the same two defaults, and its `displayDuration`/`fadeDuration` markup
//! sets them. Whether it deletes a faded line rather than hiding it is not
//! established, and it makes no difference here: a message frame cannot be
//! scrolled, so a hidden line and a deleted one look the same.
//!
//! ## A frame shows only the lines that fit
//!
//! The frame's height limits what it shows. `ChatFrame1` is 120 units tall and
//! its lines are 16.1 apart (Arial Narrow at 14), so it shows seven lines, and
//! the eighth is above the window rather than below the frame. Drawing every
//! line a frame held put the chat through the edit box, through the action bar
//! and off the bottom of the screen; [`view`] exists to prevent that.
//!
//! `insertMode` also sets which end the lines stack from: the error frame's
//! newest line is at the top and pushes the rest down, and the chat's is at
//! the bottom, growing upwards into a frame that is mostly empty when only two
//! things have been said. A top-inserting frame's block is top-aligned and a
//! bottom-inserting frame's block is bottom-aligned, and neither leaves its own
//! rectangle.
//!
//! The lines above the window can be scrolled to: `ScrollUp` moves the window
//! one line back through the history, `ScrollDown` one forward,
//! `PageUp`/`PageDown` a windowful, `ScrollToBottom` back to the live end.
//! These are the game's names (`ChatFrame1UpButton`'s `OnClick` is
//! `this:GetParent():ScrollUp()`), and the offset is clamped at both ends by
//! the amount of history there is, which is what `AtBottom` reports on.
//!
//! ## Lines wider than the frame wrap
//!
//! The budget is counted in rows and the history in lines, which is why
//! wrapping is part of this file's geometry: a chat line that wraps into three
//! rows occupies three of `ChatFrame1`'s seven rows and pushes two others off
//! the top. [`window`] walks back from the live end adding wrapped heights
//! until the budget is spent, and a line that does not fit whole is not drawn,
//! because half a wrapped line hanging out of the top of the frame is the same
//! overflow one row up.
//!
//! The painter does the wrapping, through the same [`super::regions::Paint`]
//! `wrap` flag a `<FontString>` with a declared width uses, at the width this
//! module measured, so the rows reserved and the rows drawn agree. As a
//! result, a line is measured with its `|c` escapes removed (what the reader
//! sees), and a single word longer than the frame is charged
//! `ceil(width / frame)` rows where the painter breaks it at the glyph that
//! crosses the edge.
//!
//! ## Differences from the 1.12.1 client
//!
//! * The per-line clock is restarted, not paused, while the frame is
//!   scrolled. In the 1.12.1 client lines stop fading while scrolled and are
//!   restored when shown again, which gives the same result on every path:
//!   everything is at full alpha while scrolled and the countdown restarts on
//!   the way back to the live end. The two differ only for a frame scrolled
//!   back and forth within one line's lifetime.
//! * Neither shipped directory calls `SetFading`, `SetTimeVisible` or
//!   `SetFadeDuration`; they are installed for addons. Only `SetFadeDuration`
//!   takes effect, because it writes the same key as the `fadeDuration`
//!   attribute (`FADE_KEY`). The other two are recorded and not read, so the
//!   visible time comes only from markup or the default.

use super::regions::Paint;

/// The methods a message frame carries. Sorted. The directory calls each of
/// them except `SetFading`, `SetTimeVisible` and `SetFadeDuration`.
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
/// `fadeDuration`, the fade that follows [`DURATION_KEY`]. No shipped file
/// declares it, so the default, [`FADE_SECS`], decides how the chat fades.
const FADE_KEY: &str = "__fadeDuration";
const INSERT_TOP_KEY: &str = "__insertTop";
/// `pub(super)` because a `<FontString>` declares the same attribute with a
/// different meaning: on a message frame it is how many lines are kept, and on
/// a font string it is how many rows a wrapped string may take
/// (`$parentSpellName` is `maxLines="3"`). Same attribute, same loader path;
/// [`super::regions::intrinsic`] is the only other reader.
pub(super) const MAX_LINES_KEY: &str = "__maxLines";
/// How many lines back from the live end the window is. 0 shows the newest,
/// which is where a frame stays until something scrolls it.
const SCROLL_KEY: &str = "__scroll";

/// How many lines a frame keeps when the markup sets no `maxLines`.
///
/// The 1.12.1 client has no single default for this. Every message frame in
/// either shipped directory either declares `maxLines="128"`
/// (`ChatFrameTemplate`, and therefore all seven chat frames) or holds a
/// handful of lines at a time, so this uses 128. Since faded lines are kept,
/// this bound is what eventually drops a line.
const MAX_LINES: usize = 128;

/// How long a line is fully visible when the markup declares no
/// `displayDuration`; the scrolling message frame's default.
pub const TIME_VISIBLE_SECS: f64 = 10.0;

/// How long a line takes to fade out after [`TIME_VISIBLE_SECS`].
///
/// No file in either directory declares a `fadeDuration`, so every faded line
/// in the game uses this, including the chat's lines after their 120 seconds.
pub const FADE_SECS: f64 = 3.0;

/// One line, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub colour: [f32; 4],
    /// How far through its fade the line is: 1.0 inside its
    /// `displayDuration`, falling to 0.0 across the `fadeDuration` after it,
    /// and 0.0 for a line that has faded out and is kept only so that
    /// scrolling can bring it back. Multiplies the colour's own alpha.
    pub alpha: f32,
}

/// Every line a message frame is holding, newest first when the frame inserts
/// at the top, each with how far through its fade it is.
///
/// `now` is the clock [`super::super::host::LuaHost::drawn`] and
/// `drawn_if_changed` stamp each frame. Nothing is dropped for being old: a faded line returns as soon as
/// the frame is scrolled, as in the 1.12.1 client (see the module comment).
/// Returns an empty list, after reading one table key, for the 3,743 frames
/// that are not message frames.
pub fn lines(frame: &mlua::Table, now: f64) -> Vec<Line> {
    lines_and_change(frame, now).0
}

/// [`lines`], for the draw walk: also marks the walk as animating
/// ([`super::widget::mark_animating`]) when a line's alpha will change with
/// the clock alone.
pub(in crate::lua) fn drawn_lines(lua: &mlua::Lua, frame: &mlua::Table, now: f64) -> Vec<Line> {
    let (lines, changing) = lines_and_change(frame, now);
    if changing {
        super::widget::mark_animating(lua);
    }
    lines
}

/// The lines, and whether any of them is still fading or still due to fade.
fn lines_and_change(frame: &mlua::Table, now: f64) -> (Vec<Line>, bool) {
    let Ok(Some(stored)) = frame.raw_get::<Option<mlua::Table>>(LINES_KEY) else {
        return (Vec::new(), false);
    };
    // A frame that has been scrolled back does not fade, so there is no alpha
    // to compute for it: lines fade only while the frame is `AtBottom`.
    let fading = scroll(frame) == 0;
    let visible: f64 = frame.raw_get::<Option<f64>>(DURATION_KEY).ok().flatten().unwrap_or(TIME_VISIBLE_SECS);
    let fade: f64 = frame.raw_get::<Option<f64>>(FADE_KEY).ok().flatten().unwrap_or(FADE_SECS);
    let mut out = Vec::new();
    let mut changing = false;
    for entry in stored.sequence_values::<mlua::Table>().flatten() {
        let alpha = if fading {
            let at: f64 = entry.get("at").unwrap_or(0.0);
            // A line younger than its display time plus its fade has an alpha
            // that the clock alone will change.
            changing |= now - at <= visible + fade.max(0.0);
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
    // The newest line is nearest the insert edge. Lines are stored
    // oldest-first, so a top-inserting frame reads them back reversed and a
    // bottom-inserting one does not: the error frame has its newest at the
    // top, pushing the rest down, and the chat has its newest at the bottom.
    if frame.raw_get::<Option<bool>>(INSERT_TOP_KEY).ok().flatten().unwrap_or(false) {
        out.reverse();
    }
    (out, changing)
}

/// The fade: full alpha for `visible` seconds, then linear to zero across
/// `fade`.
///
/// With a fade of zero or less, as `fadeDuration="0"` would set, a line is
/// hidden as soon as its visible time is up, with no fade.
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
/// `None` for a frame that declares none; such a message frame draws nothing
/// in the 1.12.1 client either.
pub fn style(frame: &mlua::Table) -> Option<Paint> {
    // An `EditBox` uses the same declaration for its typed line, so the reader
    // is shared; see [`super::regions::own_font`].
    super::regions::own_font(frame)
}

/// How far apart to stack the lines: the face's own line box, measured from
/// the game's own `.TTF`; see [`super::text::line_height`].
///
/// A fixed 1.2 times the declared height, the usual ratio for a TrueType face,
/// is wrong in both directions: Friz Quadrata's line box is 1.215 of its em
/// and Arial Narrow's is 1.148. The chat, set in Arial Narrow, stacks the most
/// lines, so the error accumulates most there.
pub fn spacing(lua: &mlua::Lua, style: &Paint) -> f32 {
    let height = if style.font_height > 0.0 {
        f64::from(style.font_height)
    } else {
        f64::from(vale_assets::interface::font::DEFAULT_HEIGHT)
    };
    super::text::line_height(lua, style.font.as_deref(), height) as f32
}

/// How many lines this frame has room for, which makes its height a limit on
/// what it shows.
///
/// At least one: a frame too short for a single line still shows the newest. `None` for a frame that is not a message frame,
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

/// A line as it will be drawn: the line, and how many rows it wraps into at
/// this frame's width.
///
/// The row count is all this module decides about wrapping. The painter does
/// the rest through [`super::regions::Paint::wrap`], the same mechanism a
/// `<FontString>` with a declared width uses, measured by the same
/// [`super::text::rows`], so the height reserved here and the galley drawn
/// there agree.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub line: Line,
    /// At least 1, and more for a line that is wider than the frame.
    pub rows: usize,
}

/// What a message frame is showing, top to bottom.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub lines: Vec<Row>,
    /// `insertMode="TOP"`: the block is aligned to the frame's top edge rather
    /// than its bottom edge.
    pub from_top: bool,
    /// How many rows the block occupies in total, from which bottom-inserting
    /// frames position their stack.
    pub rows: usize,
}

/// The window of lines a frame is showing, top to bottom, and which edge it is
/// aligned to.
///
/// [`rows`] gives what fits; the scroll offset gives how far back through the
/// history the window has been moved, and is clamped here rather than trusted.
/// The buffer is bounded, so an offset that was valid when it was set may be
/// past the end when it is read.
pub fn view(lua: &mlua::Lua, frame: &mlua::Table, now: f64) -> Option<Shown> {
    window(lua, drawn_lines(lua, frame, now), frame)
}

/// The same window as [`view`], over lines the caller has already read.
///
/// The draw pass first checks whether the frame holds any lines (one table
/// read for the 3,743 frames that are not message frames); reading them again
/// here would allocate the chat's whole buffer a second time every video
/// frame.
///
/// The budget is counted in rows and the offset in lines, the two units the
/// game itself uses: a frame's height divided by its line box is how many rows
/// fit, and `ScrollUp` moves the window by one message. A line that wraps into
/// three rows therefore pushes two others off the screen, as the 1.12.1 chat
/// does.
pub fn window(lua: &mlua::Lua, all: Vec<Line>, frame: &mlua::Table) -> Option<Shown> {
    let style = style(frame)?;
    let rect = super::layout::rect(lua, frame)?;
    let spacing = f64::from(spacing(lua, &style));
    if spacing <= 0.0 {
        return None;
    }
    // At least one row: a frame too short for a line still shows the newest.
    // The same arithmetic as [`rows`].
    let budget = ((rect.height / spacing).floor() as usize).max(1);
    let from_top = inserts_at_top(frame);
    // The offset is clamped so at least one line is shown. The offset is in
    // lines and the budget in rows, so an exact upper limit would require
    // wrapping the whole history. Showing at least one line is the limit that
    // matters, and `ScrollToBottom` always returns to the live end.
    let offset = scroll(frame).min(all.len().saturating_sub(1));

    let mut taken: Vec<Row> = Vec::new();
    let mut used = 0usize;
    // Newest first, whichever end that is; [`lines`] has already reversed a
    // top-inserting frame's list.
    let newest_first: Box<dyn Iterator<Item = &Line>> = if from_top {
        Box::new(all.iter())
    } else {
        Box::new(all.iter().rev())
    };
    for line in newest_first.skip(offset) {
        let rows = fold(lua, &style, &line.text, rect.width);
        // A line that does not fit is not shown, unless it is the only one,
        // so that no half-wrapped line is drawn out of the top of the frame.
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

/// How many rows one line wraps into at `width`.
///
/// [`super::text::rows`] removes the `|c` escapes first, so a chat line wraps
/// at the words the reader sees rather than at the thirty-two characters of
/// `|Hplayer:Bram|h[Bram]|h`.
fn fold(lua: &mlua::Lua, style: &Paint, text: &str, width: f64) -> usize {
    let height = if style.font_height > 0.0 {
        f64::from(style.font_height)
    } else {
        f64::from(vale_assets::interface::font::DEFAULT_HEIGHT)
    };
    super::text::rows(lua, style.font.as_deref(), height, text, width)
}

/// Whether the newest line is at the top (`insertMode="TOP"`).
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
/// `by` is in lines, and positive moves back through the history, which is
/// what `ScrollUp` means on both kinds of frame: the direction on the screen
/// differs between them and the direction through time does not.
fn scroll_by(lua: &mlua::Lua, frame: &mlua::Table, by: i64) -> mlua::Result<()> {
    // Every scroll press restores the faded text, whether or not it also
    // moves the window. In the 1.12.1 client a press that cannot move
    // restores every line, and a press that does move restores the lines
    // shown after it; here both are one restamp.
    restamp(lua, frame)?;
    let held = held(frame) as i64;
    // The rectangle may not solve (a frame the loader has not anchored yet);
    // then one row is used as the page size, which keeps the offset bounded
    // without assuming a height.
    let rows = rows(lua, frame).unwrap_or(1) as i64;
    let limit = (held - rows).max(0);
    let to = (scroll(frame) as i64 + by).clamp(0, limit);
    super::widget::set_paint(lua, frame, SCROLL_KEY, to)
}

/// How many lines the frame is holding, faded ones included.
fn held(frame: &mlua::Table) -> usize {
    frame
        .raw_get::<Option<mlua::Table>>(LINES_KEY)
        .ok()
        .flatten()
        .map_or(0, |stored| stored.raw_len())
}

/// Return every line to full alpha with a restarted clock. Every scroll press
/// calls this.
fn restamp(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let Some(stored) = frame.raw_get::<Option<mlua::Table>>(LINES_KEY)? else {
        return Ok(());
    };
    let now = now(lua);
    for entry in stored.sequence_values::<mlua::Table>().flatten() {
        entry.set("at", now)?;
    }
    super::widget::mark_paint(lua);
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
    // The fade setters. `SetFadeDuration` writes the key the `fadeDuration`
    // attribute writes, so it takes effect; `SetFading` and `SetTimeVisible`
    // are recorded under keys nothing reads. `SetFading(false)` is the first
    // call a chat addon makes, and a nil method there aborts its whole chat
    // setup.
    for (name, key) in [
        ("SetFading", "__fading"),
        ("SetTimeVisible", "__timeVisible"),
        ("SetFadeDuration", "__fadeDuration"),
    ] {
        let set = lua.create_function(move |lua, (this, value): (mlua::Table, mlua::Value)| {
            super::widget::set_paint(lua, &this, key, value)
        })?;
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
            // A number is accepted as a message. `AddMessage(1)` is not in the
            // shipped directory, but 1.12 converts a number to a string here as
            // it does for `SetText`.
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
            // The fifth argument. Every chat line the directory adds carries
            // `info.id`, the chat type's number, and `UpdateColorByID` below
            // recolours the history by it, which is how changing the colour of
            // whispers changes the whispers already on screen. A line with no
            // id (an error message, a tooltip line) matches nothing.
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
            super::widget::mark_paint(lua);
            // A frame that has been scrolled back keeps its position. The
            // offset is counted from the live end, so without this a line
            // arriving at that end would slide the window forward and change
            // what is on screen under the reader. The 1.12.1 client flashes the
            // bottom button instead, based on `AtBottom`.
            if scroll(&this) > 0 {
                super::widget::set_paint(lua, &this, SCROLL_KEY, scroll(&this) as i64 + 1)?;
            }
            let cap = this
                .raw_get::<Option<usize>>(MAX_LINES_KEY)?
                .unwrap_or(MAX_LINES)
                .max(1);
            // Bounded, because a chat frame would otherwise grow without
            // limit. The oldest line goes; `raw_remove(1)` is O(n) and n is at
            // most `cap`.
            while stored.raw_len() > cap {
                stored.raw_remove(1)?;
            }
            Ok(())
        },
    )?;
    methods.set("AddMessage", add)?;

    let clear = lua.create_function(|lua, this: mlua::Table| {
        super::widget::set_paint(lua, &this, SCROLL_KEY, 0_i64)?;
        super::widget::set_paint(lua, &this, LINES_KEY, lua.create_table()?)
    })?;
    methods.set("Clear", clear)?;

    // Recolour the history, so that a chat colour applies to lines already
    // shown and not only to new ones. `ChatFrame_OnEvent` calls it on every
    // `UPDATE_CHAT_COLOR`, including the ninety-four this client raises at
    // load.
    let update_by_id = lua.create_function(
        |lua, (this, id, r, g, b): (mlua::Table, f64, f64, f64, f64)| {
            let Some(stored) = this.raw_get::<Option<mlua::Table>>(LINES_KEY)? else {
                return Ok(());
            };
            for entry in stored.sequence_values::<mlua::Table>().flatten() {
                if entry.get::<Option<f64>>("id")? != Some(id) {
                    continue;
                }
                // The alpha is the line's own and is left unchanged, so a
                // fading `UIErrorsFrame` line keeps its fade.
                let alpha = entry
                    .get::<Vec<f64>>("colour")
                    .ok()
                    .and_then(|c| c.get(3).copied())
                    .unwrap_or(1.0);
                entry.set("colour", vec![r, g, b, alpha])?;
                super::widget::mark_paint(lua);
            }
            Ok(())
        },
    )?;
    methods.set("UpdateColorByID", update_by_id)?;

    // Scrolling through the history, under the game's six method names. The
    // arrows down the left of every chat frame call the first two.
    //
    // A page is whatever fits at the time of the call, as in the 1.12.1
    // client: a resized chat frame pages by its new height rather than the
    // height it was declared at.
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
    // `ChatFrameTemplate`'s bottom button, the scroll press used most. It
    // restamps like the others, which brings back a chat that has faded out.
    let to_bottom = lua.create_function(|lua, this: mlua::Table| {
        restamp(lua, &this)?;
        super::widget::set_paint(lua, &this, SCROLL_KEY, 0_i64)
    })?;
    methods.set("ScrollToBottom", to_bottom)?;
    // `i64::MAX` rather than the count, because [`scroll_by`] clamps to the
    // history available, so the limit is computed in one place.
    let to_top = lua.create_function(|lua, this: mlua::Table| scroll_by(lua, &this, i64::MAX))?;
    methods.set("ScrollToTop", to_top)?;
    // The two queries the arrows' `OnUpdate` makes. 1.12 returns a boolean
    // as 1 or nil, never `false`; see [`super::super::api`].
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
    let set_max = lua.create_function(|lua, (this, lines): (mlua::Table, Option<usize>)| {
        super::widget::set_paint(lua, &this, MAX_LINES_KEY, lines.unwrap_or(MAX_LINES).max(1))
    })?;
    methods.set("SetMaxLines", set_max)?;
    Ok(())
}

/// The registry key the per-frame clock is stored under; see [`set_now`].
const REG_NOW: &str = "vale.now";

/// Set the interface clock, once a frame.
///
/// `AddMessage` needs to know when a line arrived, and it cannot read the
/// clock from a scope: it is a method on an object, reachable from an event
/// handler, and `GetTime` exists only inside [`super::super::api`]'s scope. So
/// that this module has no separate notion of time, the frame's `GetTime`
/// value is written here at the start of each draw and read back by whatever
/// needs it.
pub fn set_now(lua: &mlua::Lua, now: f64) {
    let _ = lua.set_named_registry_value(REG_NOW, now);
}

/// Read the interface clock. `0.0` before anything set it, which makes every
/// line arbitrarily old rather than arbitrarily new; the other direction
/// would leave a message on the screen indefinitely.
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

    /// The line `UIErrorsFrame_OnEvent` adds, and its colour.
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

    /// `displayDuration` fades a line and does not delete it. When lines were
    /// deleted, the chat was empty two minutes after logging in and nothing
    /// could be scrolled back to.
    ///
    /// As in the 1.12.1 client: full alpha until the duration is up, then
    /// linear to zero across [`FADE_SECS`], and the line stays in the buffer
    /// afterwards.
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

        // With a declared `fadeDuration` of zero the line is hidden as soon as
        // its time is up, with no fade.
        set_from_markup(&frame, "fadeDuration", "0").expect("sets");
        assert_eq!(alpha(105.0), 1.0);
        assert_eq!(alpha(105.1), 0.0);
    }

    /// The defaults are the 1.12.1 client's: a frame that declares no duration
    /// still fades, after ten seconds and over three. `RaidWarningFrame` is the
    /// shipped example.
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

    /// A scroll press restores the faded text, and a frame that has been
    /// scrolled away from the live end does not fade.
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

        // The press only restores the text: the window has not moved.
        set_now(&lua, faded_now);
        let before = showing(&lua, &frame);
        lua.load("chat:ScrollToBottom()").exec().expect("runs");
        assert!(lines(&frame, faded_now).iter().all(|line| line.alpha == 1.0));
        assert_eq!(showing(&lua, &frame), before);

        // Scrolled back, nothing fades however long it has been.
        lua.load("chat:ScrollUp()").exec().expect("runs");
        assert!(lines(&frame, faded_now + 600.0)
            .iter()
            .all(|line| line.alpha == 1.0));
    }

    /// `insertMode="TOP"` reverses the reading order: the newest error is at
    /// the top and pushes the rest down, where the newest line of chat is at
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

    /// Past the line cap, the oldest line is dropped.
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

    /// A chat-like frame: on the screen, with a real height and the unnamed
    /// `<FontString>` its lines are laid out in. With no faces loaded the line
    /// box is 1.2 times the height, so 100 units of frame is six rows.
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
        crate::lua::widgets::regions::set_font_height(lua, &font, 12.0).expect("a height");
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

    /// A frame shows what fits in it and no more. Twenty lines in a frame six
    /// rows tall shows six; the other fourteen are reached by scrolling, not
    /// drawn down through the edit box and off the bottom of the screen.
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

    /// The scroll methods move the window, clamped at both ends by how much
    /// there is to show. `ChatFrame1UpButton`'s `OnClick` is
    /// `this:GetParent():ScrollUp()`, so this covers everything that button
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

        // Up past the oldest stops at the oldest, with the window still full.
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

    /// A frame that has been scrolled back keeps its position when a line
    /// arrives. The offset is counted from the live end, so a new line at that
    /// end would otherwise slide the window under the reader.
    #[test]
    fn a_new_line_does_not_move_a_scrolled_window() {
        let lua = state();
        let frame = chat(&lua);
        say(&lua, 20);
        lua.load("chat:ScrollUp()").exec().expect("runs");
        let before = showing(&lua, &frame);
        lua.load(r#"chat:AddMessage("fresh")"#).exec().expect("runs");
        assert_eq!(showing(&lua, &frame), before, "the window held still");
        // At the live end the window follows new lines.
        lua.load("chat:ScrollToBottom()").exec().expect("runs");
        lua.load(r#"chat:AddMessage("newest")"#).exec().expect("runs");
        assert_eq!(showing(&lua, &frame)[5].text, "newest");
    }

    /// The error frame uses the other insert mode and stacks from the top: its
    /// newest line is at the top, pushing the rest down, and it also shows only
    /// what fits.
    #[test]
    fn a_top_inserting_frame_fills_from_its_own_edge() {
        let lua = state();
        let frame = chat(&lua);
        set_from_markup(&frame, "insertMode", "TOP").expect("sets");
        // Three rows rather than six, so the two edges are distinguished by
        // which lines are dropped as well as by their order.
        lua.load("chat:SetHeight(50)").exec().expect("runs");
        say(&lua, 20);
        let seen = view(&lua, &frame, 0.0).expect("a window");
        assert!(seen.from_top);
        let shown = showing(&lua, &frame);
        assert_eq!(shown.len(), 3);
        assert_eq!(shown[0].text, "line20", "the newest is at the top");
        assert_eq!(shown[2].text, "line18");
    }

    /// A line wider than the frame wraps, and the extra rows count against the
    /// frame's height.
    ///
    /// Without wrapping, a long say ran out through the right edge of
    /// `ChatFrame1` and over whatever was beside it. It must instead take two
    /// of the frame's six rows, which pushes the line above it off.
    #[test]
    fn a_line_wider_than_the_frame_takes_two_rows_and_pushes_one_off() {
        let lua = state();
        let frame = chat(&lua);
        // 430 units wide at height 12. With no faces loaded the fallback
        // measurement is half the height per character, so about 72
        // characters to the row; this line takes two rows, well short of three.
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

        // The same line in a frame with no room for both its rows is still
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
