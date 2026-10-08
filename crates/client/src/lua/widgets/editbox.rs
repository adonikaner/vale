//! `EditBox`, the widget that takes keyboard input.
//!
//! The shipped directory has sixteen of them. The most used is
//! `ChatFrameEditBox`, the line at the bottom of the screen that carries
//! everything a player says, `.tele` and `/script` included. An edit box's
//! state cannot be derived from the world: it is a string, a caret and a focus
//! flag that only the keyboard writes.
//!
//! ## The chat input chain
//!
//! ```text
//! Enter                     the key table, this client's half
//!   -> OPENCHAT             Bindings.xml, declaration 87
//!   -> ChatFrame_OpenChat("")            ChatFrame.lua:1545
//!   -> editBox:Show()  -> OnShow -> ChatEdit_OnShow -> this:SetFocus()
//!   … typing …             lua::keyboard, the strokes -> this file
//!   -> Enter -> OnEnterPressed -> ChatEdit_SendText   ChatFrame.lua:1940
//!   -> ChatEdit_ParseText decides the chat type from SLASH_* and SlashCmdList
//!   -> SendChatMessage(text, "SAY")      lua::verbs, a registered closure
//! ```
//!
//! Every line of that except the first and the last comes from the game's
//! interface archive. Slash-command parsing, the `/script` check and the meaning of a leading `.`
//! are decisions `ChatFrame.lua` makes, so this module does not make them.
//! `SLASH_SCRIPT2 = "/run"`: 1.12 ships `/run` as a synonym for `/script`.
//!
//! ## Where the text is stored
//!
//! `SetText` and `GetText` are implemented in [`super::regions`]: a frame with
//! no font string keeps its string on itself, so an edit box has no text store
//! of its own. This module holds the state around the text: the caret, the
//! focus, the insets that push the text right of the header, the letter cap and
//! the history. [`text_was_set`] is the hook that makes the shared `SetText`
//! fire `OnTextSet` on an edit box, the only kind that has that script.
//!
//! ## Keyboard focus
//!
//! There is one keyboard focus in the client, so it is one registry key rather
//! than a flag per frame, the same shape [`super::super::api::mouse`] uses for
//! the pointer. `Hide()` clears the focus if the hidden box held it.
//! `ChatEdit_OnEscapePressed` ends with `editBox:Hide()` and nothing else, so
//! without this every keystroke after the first message would go to an
//! invisible box. A hidden box cannot take the focus; see `SetFocus` in
//! [`install`].
//!
//! ## Selection rules
//!
//! An edit box's selection is a start index and an end index; equal indices
//! mean nothing is selected. The 1.12.1 client applies these rules:
//!
//! * An insert deletes the selection first, before the letter cap and the
//!   numeric filter apply, so typing or pasting over a selection replaces it.
//! * A paste is inserted one character at a time through the same path, so the
//!   letter cap truncates a paste rather than refusing it.
//! * Copy and cut do nothing with an empty selection. The clipboard is not
//!   touched; the whole line is not copied.
//! * Every caret move extends the selection when Shift is held. There is no
//!   separate select mode.
//! * `SetSelection` treats an end below the start as the end of the text, so
//!   `HighlightText(0, -1)`, which Ctrl-A calls, selects everything.
//!
//! [`Mods`] holds the modifiers a box reads: Shift extends the selection, and
//! Alt lets an arrow past `ignoreArrows` (see [`ignores_arrows`]). A Ctrl chord
//! arrives as its own [`Stroke`] variant.
//!
//! ## Differences from the 1.12.1 client
//!
//! * In the 1.12.1 client the caret is independent of the selection, so
//!   `HighlightText(start, end)` selects without moving the caret. This module
//!   stores an anchor and the caret instead. The two models agree for every
//!   keystroke and differ only for a `HighlightText` followed by a Shift-move.
//!   This is the only reconstructed part of the selection.
//! * The 1.12.1 client also accepts Ctrl-B/F (move), Ctrl-P/N (history),
//!   Ctrl-K/U/W (kill) and Ctrl-D (delete forward) in an edit box. These are
//!   not implemented; nothing in the shipped directory or its bindings mentions
//!   them.
//! * The caret's x is measured in the box's own face from the game's own
//!   `.TTF`; see [`super::text::width`]. An earlier estimate of half the font
//!   height per character drifted by one character width every eight capitals.
//! * `ignoreArrows` is honoured for the caret only. The chat box declares it,
//!   so the arrows do not move its caret. In the 1.12.1 client the arrows then
//!   reach the game and turn the character; here they reach nothing, because
//!   [`super::super::api::keyboard`] suppresses the whole key table while an
//!   edit box has focus.
//! * There is no IME and no `numeric` or `multiLine`. `GetInputLanguage`
//!   answers the Roman keyboard, which `INPUT_ROMAN = "A"` in
//!   `GlobalStrings.lua` labels.

use super::widget;
use crate::interface::events::EventArg;

/// The methods an edit box carries beyond the ones every frame has. Sorted;
/// the shipped directory calls every one of them.
///
/// `SetText`/`GetText` are absent: they are [`super::button`]'s forwarding
/// pair on the shared method table, and an edit box uses them unchanged.
pub const METHODS: [&str; 20] = [
    "AddHistoryLine",
    "ClearFocus",
    "GetFontObject",
    "GetInputLanguage",
    "GetNumber",
    "HasFocus",
    "HighlightText",
    "Insert",
    "SetAltArrowKeyMode",
    "SetAutoFocus",
    "SetFocus",
    "SetFont",
    "SetFontObject",
    "SetJustifyH",
    "SetJustifyV",
    "SetMaxLetters",
    "SetMultiLine",
    "SetNumber",
    "SetTextColor",
    "SetTextInsets",
];

/// `SetAutoFocus`: recorded and read by nothing. This client gives a box the
/// keyboard on a click and takes it away on `ClearFocus`, whatever the flag.
/// An addon's search box sets it false on every screen it builds.
const AUTO_FOCUS_KEY: &str = "__autoFocus";

/// Where the caret sits, as a character index into the text: 0 is before the
/// first character and `len` is after the last.
///
/// Characters rather than bytes, because every reader either counts glyphs
/// (the drawn x) or splits the string (an insert), and a byte index into a
/// UTF-8 string panics on the first accented name typed into a whisper.
const CARET_KEY: &str = "__caret";
/// The other end of the selection, on the same terms as [`CARET_KEY`], or
/// absent when nothing is selected.
///
/// The 1.12.1 client keeps a start and an end and marks an empty selection by
/// making them equal. This module stores one end and marks an empty selection
/// by leaving the key off. The states are the same, with one fewer way to be
/// inconsistent, and [`selection`] is the only reader, so the difference does
/// not reach callers.
const ANCHOR_KEY: &str = "__anchor";
/// `password="1"`. `AccountLoginPasswordEdit` is the one box in either
/// directory that declares it.
const PASSWORD_KEY: &str = "__password";
/// `letters="255"` on the chat template, or `SetMaxLetters`. 0 means no cap,
/// as it does in the game.
const MAX_LETTERS_KEY: &str = "__maxLetters";
/// `SetTextInsets(left, right, top, bottom)`, which `ChatEdit_UpdateHeader`
/// calls with `15 + header:GetWidth()` so the typed text starts after `Say:`.
const INSETS_KEY: &str = "__textInsets";
/// The colour of the typed text, which the same function sets to the chat
/// type's colour. Kept on the frame rather than on a region because an edit
/// box's text has no region; see the module comment.
const COLOUR_KEY: &str = "__textColour";
/// Lines `AddHistoryLine` has been given, oldest first, and where an arrow walk
/// currently is inside them.
const HISTORY_KEY: &str = "__history";
const HISTORY_AT_KEY: &str = "__historyAt";
/// `historyLines="32"`, the cap on the list above.
const HISTORY_LINES_KEY: &str = "__historyLines";
/// `ignoreArrows="true"`, which the chat box declares. The module comment
/// states the part of its behaviour this client does not implement.
const IGNORE_ARROWS_KEY: &str = "__ignoreArrows";

/// The one focused edit box, or nothing. In the registry rather than a global
/// for the reason [`super::widget::roots`] gives: interface code assigning to a
/// name must not be able to break the keyboard.
const REG_FOCUS: &str = "vale.keyboardFocus";

/// How many history lines a box that declares none keeps. Every box in the
/// directory that has a history declares the chat template's
/// `historyLines="32"`, so this is a bound rather than a measured default.
const DEFAULT_HISTORY_LINES: usize = 32;

/// A flag marking an edit box, set at creation on the sixteen objects that are
/// edit boxes and absent on the other 3,730 frames.
///
/// [`widget::KIND_KEY`] already holds `"EditBox"`, but the draw walk asks this
/// once per visible frame per video frame, and `get::<mlua::String>` returns a
/// Lua string that has to be rooted and dropped. A `bool` is a hash lookup
/// that misses. Measured at one framing, the string form added 0.6 ms to the
/// interface's median frame, a significant share of a 60 Hz budget.
/// [`widget::CLASS_KEY`] uses the same shape for the same reason.
const IS_EDIT_BOX_KEY: &str = "__isEditBox";

/// Give a fresh frame the state this module owns, which is nothing for every
/// kind except `EditBox`. Called from [`super::frames::create_frame`], beside
/// the pointer's initialisation.
pub(in crate::lua) fn init(frame: &mlua::Table, kind: &str) -> mlua::Result<()> {
    if kind != "EditBox" {
        return Ok(());
    }
    frame.set(IS_EDIT_BOX_KEY, true)
}

/// Create the five regions an `<EditBox>` has before any of its markup is
/// read: one `FontString` and four `Texture`s.
///
/// In the 1.12.1 client a bare `CreateFrame("EditBox", nil, UIParent)`
/// returns from `GetRegions()`
///
/// ```text
/// 5 | 1:FontString 2:Texture 3:Texture 4:Texture 5:Texture
/// ```
///
/// and `GuildControlPopupFrameEditBox`, which declares two `<Texture>`s and two
/// `<FontString>`s of its own, returns 8: those five, then the declared pair of
/// textures at 6 and 7, then the label at 8. The trailing
/// `<FontString inherits="ChatFontNormal"/>` adds nothing to the count, because
/// it configures the string the widget already made rather than making a
/// second; [`crate::lua::xml::Loader::object_for`] binds the two.
///
/// The count and the order are measured; the purpose of the four textures is
/// inferred. `GetTexture()` returns `"Solid Texture"` on all four (they are
/// colour-set with no file), which fits the selection highlight and the caret
/// of a widget that supports `SetMultiLine`: three quads for a selection that
/// wraps, and one for the caret. This client draws neither from these regions:
/// [`super::draw::edit_box`] emits the text and the caret from the frame
/// itself.
///
/// The regions exist because interface code indexes `GetRegions()` by
/// position. pfUI's friends skin does
///
/// ```lua
/// local _,_,_,_,_,left,right = GuildControlPopupFrameEditBox:GetRegions()
/// left:Hide() right:Hide()
/// ```
///
/// to reach the two border textures at 6 and 7. With four regions, those
/// positions were nil, and the error aborted the `for` loop in pfUI's one
/// `ADDON_LOADED` handler that runs every skin, so 31 of its 36 skins never
/// ran, including the tooltip, questlog, merchant, gossip, mail, options and
/// taxi skins.
///
/// They are made with no anchors, so they solve to no rectangle and draw
/// nothing.
pub(in crate::lua) fn furnish(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let text = super::regions::create(lua, "FontString", None, Some(frame.clone()), None)?;
    // Stored under this module's own key, not `regions::TEXT_REGION_KEY`. That
    // key names the region this frame's `SetText` writes into, which is a
    // button's `<ButtonText>`. An edit box's typed line is not held in a
    // region: it lives on the frame and is laid out by
    // [`super::draw::edit_box`], because it needs a caret and an inset that no
    // font string models.
    //
    // Under `TEXT_REGION_KEY`, every `SetText` on an edit box went into this
    // region, which took the text off the frame the draw reads, and, since
    // `regions::own_font` wants the font string with no text, left the box
    // with no style either. On the login screen the account name drew
    // centred, in the wrong face, with no caret.
    frame.set(OWN_STRING_KEY, text)?;
    for _ in 0..4 {
        super::regions::create(lua, "Texture", None, Some(frame.clone()), None)?;
    }
    Ok(())
}

/// The font string an edit box is created with; see [`furnish`]. The loader
/// binds the element's own `<FontString>` to it, and `regions::own_font` finds
/// it among the children like any other.
pub(in crate::lua) const OWN_STRING_KEY: &str = "__editString";

/// The font string under [`OWN_STRING_KEY`]. The one reader outside this file
/// is [`crate::lua::xml::Loader::object_for`].
pub(in crate::lua) fn own_string(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.raw_get::<Option<mlua::Table>>(OWN_STRING_KEY).ok().flatten()
}

/// Whether this object is an edit box. One table read; every hook in this file
/// that is called from a method all frames share checks it first.
pub(in crate::lua) fn is_edit_box(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<bool>>(IS_EDIT_BOX_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Apply the four `<EditBox>` attributes this client acts on, from the loader.
/// The loader's own attribute list must also name each of them; see
/// [`crate::lua::xml`].
pub(in crate::lua) fn set_from_markup(
    object: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "letters" => object.set(MAX_LETTERS_KEY, value.parse::<i64>().unwrap_or(0)),
        "historyLines" => object.set(HISTORY_LINES_KEY, value.parse::<i64>().unwrap_or(0)),
        // The glue spells it `password="1"`, not `"true"`, so any value other
        // than `"0"` sets the flag.
        "password" => object.set(PASSWORD_KEY, value != "0"),
        // A tri-state for the same reason `enableMouse` is one: a template may
        // turn its parent template's flag back off.
        "ignoreArrows" => object.set(IGNORE_ARROWS_KEY, value == "true"),
        _ => Ok(()),
    }
}

/// The focused edit box, if the keyboard is going to one.
pub fn focused(lua: &mlua::Lua) -> Option<mlua::Table> {
    lua.named_registry_value::<Option<mlua::Table>>(REG_FOCUS)
        .ok()
        .flatten()
}

/// The focused edit box's name, for the HUD and for
/// [`super::super::api::keyboard::KeyboardFocus`].
pub(in crate::lua) fn focused_name(lua: &mlua::Lua) -> Option<String> {
    focused(lua)?.raw_get::<Option<String>>(widget::NAME_KEY).ok().flatten()
}

/// Take the focus, firing `OnEditFocusLost` on whoever had it and
/// `OnEditFocusGained` on this one.
///
/// Both fire, in that order, because they are how a box knows to start or stop
/// drawing its caret, and because 1.12's `SetFocus` moves the focus rather
/// than adding one. Errors from the handlers are returned to be reported. The
/// focus moves either way, so that a handler that raises cannot leave the
/// keyboard pointing at a frame that believes it lost it.
fn take_focus(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if let Some(previous) = focused(lua) {
        if previous == *frame {
            return Ok(());
        }
        lua.set_named_registry_value(REG_FOCUS, mlua::Value::Nil)?;
        widget::mark_paint(lua);
        let _ = super::frames::run_script(lua, &previous, "OnEditFocusLost", &[]);
    }
    lua.set_named_registry_value(REG_FOCUS, frame.clone())?;
    widget::mark_paint(lua);
    super::frames::run_script(lua, frame, "OnEditFocusGained", &[])
}

/// Give the keyboard to a clicked edit box. The widget does this itself, not a
/// script handler.
///
/// The directory has no `OnMouseDown` on any `<EditBox>`, and the glue's only
/// two `SetFocus` calls are in `AccountLogin_OnShow`. Without this, only a box
/// that an `OnShow` focuses would take input, and every other text field,
/// `CharacterCreateNameEdit` among them, would not. Called from
/// [`super::super::api::mouse::dispatch`] on the left press, beside the
/// slider's grab: in 1.12 both widgets handle their own mouse press.
///
/// A press on anything that is not a text field does not release the focus.
/// In the 1.12.1 client the chat line stays open after a click on the world;
/// `ChatEdit_OnEditFocusLost` would close it.
pub(in crate::lua) fn clicked(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if !is_edit_box(frame) {
        return Ok(());
    }
    take_focus(lua, frame)
}

/// Drop the focus if this frame holds it.
fn release_focus(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let Some(current) = focused(lua) else {
        return Ok(());
    };
    if current != *frame {
        return Ok(());
    }
    lua.set_named_registry_value(REG_FOCUS, mlua::Value::Nil)?;
    widget::mark_paint(lua);
    super::frames::run_script(lua, frame, "OnEditFocusLost", &[])
}

/// Release the focus when an edit box is hidden. Called from `Hide`, beside
/// the tooltip's release. `ChatEdit_OnEscapePressed` hides the box and never
/// clears the focus, so without this a second message could not be typed.
pub(super) fn hidden(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if !is_edit_box(frame) {
        return Ok(());
    }
    release_focus(lua, frame)
}

/// After `SetText` on an edit box: move the caret to the end, clear the
/// selection and fire `OnTextSet` and `OnTextChanged`, as the game does.
///
/// Called from [`super::regions::set_frame_text`], the body behind the shared
/// `SetText`; for every other kind it is a no-op costing one table read.
/// `OnTextSet` runs `ChatEdit_ParseText`, which turns a typed `/s hello` into a
/// `SAY` with the command cut off, so the chat line needs this hook.
///
/// It re-enters safely: `ChatEdit_OnTextSet` parses the text and calls
/// `SetText` with the command removed, so each round is strictly shorter and
/// the second one takes the "does not start with /" exit.
pub(super) fn text_was_set(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if !is_edit_box(frame) {
        return Ok(());
    }
    let text = text(frame);
    widget::set_paint(lua, frame, CARET_KEY, text.chars().count() as i64)?;
    widget::set_paint(lua, frame, ANCHOR_KEY, mlua::Value::Nil)?;
    let _ = super::frames::run_script(lua, frame, "OnTextSet", &[]);
    let _ = super::frames::run_script(lua, frame, "OnTextChanged", &[]);
    Ok(())
}

/// What is in the box.
pub fn text(frame: &mlua::Table) -> String {
    super::regions::text_of(frame).unwrap_or_default()
}

/// Where the caret is, clamped into the text. A script that sets the text
/// through a path this file does not see would otherwise leave it past the end.
pub fn caret(frame: &mlua::Table) -> usize {
    let letters = text(frame).chars().count();
    frame
        .raw_get::<Option<i64>>(CARET_KEY)
        .ok()
        .flatten()
        .unwrap_or(0)
        .clamp(0, letters as i64) as usize
}

/// What is selected, as a half-open character range, or `None`.
///
/// Ordered, so a selection dragged backwards reads the same as one dragged
/// forwards. The 1.12.1 client orders and clamps the range when it is set.
pub fn selection(frame: &mlua::Table) -> Option<(usize, usize)> {
    let letters = text(frame).chars().count() as i64;
    let anchor = frame.raw_get::<Option<i64>>(ANCHOR_KEY).ok().flatten()?;
    let anchor = anchor.clamp(0, letters) as usize;
    let caret = caret(frame);
    (anchor != caret).then(|| (anchor.min(caret), anchor.max(caret)))
}

/// The selected text, or `None` when nothing is.
///
/// A password box returns the empty string rather than its contents, as the
/// 1.12.1 client does: it puts an empty string on the clipboard instead of the
/// text. The box still has a selection, so Ctrl-X still deletes it.
fn selected_text(frame: &mlua::Table) -> Option<String> {
    let (start, end) = selection(frame)?;
    if is_password(frame) {
        return Some(String::new());
    }
    Some(text(frame).chars().skip(start).take(end - start).collect())
}

/// Whether this box is drawn as dots and copies as the empty string; see
/// [`selected_text`].
pub fn is_password(frame: &mlua::Table) -> bool {
    frame.raw_get::<Option<bool>>(PASSWORD_KEY).ok().flatten().unwrap_or(false)
}

/// Select `start..end`, clamped as the 1.12.1 client clamps it.
///
/// An `end` below the `start` means the end of the text, which makes
/// `HighlightText(0, -1)`, what Ctrl-A sends, select everything. An empty
/// range clears the selection.
fn select(lua: &mlua::Lua, frame: &mlua::Table, start: i64, end: i64) -> mlua::Result<()> {
    let letters = text(frame).chars().count() as i64;
    let start = start.clamp(0, letters);
    let end = if end < start { letters } else { end.min(letters) };
    if start == end {
        return clear_selection(lua, frame);
    }
    widget::set_paint(lua, frame, ANCHOR_KEY, start)?;
    widget::set_paint(lua, frame, CARET_KEY, end)
}

/// Nothing is selected any more. The caret stays where it is.
fn clear_selection(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    widget::set_paint(lua, frame, ANCHOR_KEY, mlua::Value::Nil)
}

/// Put the caret somewhere, extending the selection or dropping it.
///
/// All six caret-moving keys go through this function, because in the 1.12.1
/// client Shift extends the selection on every caret move, not on particular
/// keys; see the module comment.
fn move_caret(lua: &mlua::Lua, frame: &mlua::Table, to: i64, extend: bool) -> mlua::Result<()> {
    let letters = text(frame).chars().count() as i64;
    let to = to.clamp(0, letters);
    if !extend {
        clear_selection(lua, frame)?;
    } else if frame.raw_get::<Option<i64>>(ANCHOR_KEY)?.is_none() {
        // The first Shift-move drops the anchor where the caret was standing.
        widget::set_paint(lua, frame, ANCHOR_KEY, caret(frame) as i64)?;
    }
    widget::set_paint(lua, frame, CARET_KEY, to)
}

/// The string with `start..end` taken out of it.
fn without(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .enumerate()
        .filter(|(index, _)| *index < start || *index >= end)
        .map(|(_, c)| c)
        .collect()
}

/// `SetTextInsets(left, right, top, bottom)`, or, when that was never called,
/// the box's own backdrop insets.
///
/// The fallback comes from the markup rather than a margin chosen here.
/// Nothing calls `SetTextInsets` on `AccountLoginAccountEdit`, and its
/// `<Backdrop>` says
///
/// ```xml
/// <BackgroundInsets><AbsInset left="10" right="5" top="4" bottom="9"/></BackgroundInsets>
/// <EdgeSize><AbsValue val="16"/></EdgeSize>
/// ```
///
/// which is the interior of the plate, the only thing in the markup that says
/// where the inside of this box is. With zero insets the typed line starts at
/// the frame's own left edge, under the sixteen-unit border piece: the account
/// name drew with its first letter cut in half and the password caret outside
/// the box.
///
/// `ChatFrameEditBox` does not reach the fallback: `ChatEdit_UpdateHeader`
/// calls `SetTextInsets(15 + header:GetWidth(), 13, 0, 0)` on every open.
///
/// This fallback is inferred from the markup. The default text inset of the
/// 1.12.1 client is not established here; zero is known to be wrong, because
/// it draws the text under art the same file positions.
pub fn insets(frame: &mlua::Table) -> [f32; 4] {
    let Ok(Some(table)) = frame.raw_get::<Option<mlua::Table>>(INSETS_KEY) else {
        return super::backdrop::read(frame).map_or([0.0; 4], |backdrop| backdrop.insets);
    };
    let read = |key: &str| table.get::<Option<f64>>(key).ok().flatten().unwrap_or(0.0) as f32;
    [read("left"), read("right"), read("top"), read("bottom")]
}

/// The colour of the typed text, when `SetTextColor` has said one.
pub fn colour(frame: &mlua::Table) -> Option<[f32; 4]> {
    let stored: Option<Vec<f64>> = frame.raw_get(COLOUR_KEY).ok().flatten();
    match stored.as_deref() {
        Some([r, g, b, a]) => Some([*r as f32, *g as f32, *b as f32, *a as f32]),
        Some([r, g, b]) => Some([*r as f32, *g as f32, *b as f32, 1.0]),
        _ => None,
    }
}

/// One keystroke, after the modifiers and the keyboard layout have been
/// applied. [`super::super::api::keyboard`] is the only producer.
///
/// A typed key arrives as a character rather than a key code, because a text
/// field takes what the layout produced: a French keyboard's `A` arrives here
/// as `"q"` on a US mapping, and this file does not depend on the layout.
#[derive(Debug, Clone, PartialEq)]
pub enum Stroke {
    /// One typed character, as the window system produced it.
    Char(String),
    Backspace,
    Delete,
    Left(Mods),
    Right(Mods),
    Home(Mods),
    End(Mods),
    /// Enter, Escape and Tab each run a script of their own rather than edit
    /// the text.
    Enter,
    Escape,
    Tab,
    /// Up and Down walk the `AddHistoryLine` list towards older and newer
    /// lines, in a box that does not ignore arrows.
    Up(Mods),
    Down(Mods),
    /// Ctrl-A, which the client handles as `HighlightText(0, -1)`.
    SelectAll,
    /// Ctrl-C and Ctrl-Insert. The text to put on the clipboard comes back out
    /// of [`dispatch`]; this file never touches the OS.
    Copy,
    /// Ctrl-X and Shift-Delete: [`Stroke::Copy`] and then the deletion.
    Cut,
    /// Ctrl-V and Shift-Insert, carrying the clipboard text already read by
    /// [`super::super::api::keyboard`], which talks to the window system.
    Paste(String),
}

/// The two modifiers an edit box reads.
///
/// Shift extends the selection on every caret move. Alt lets the four arrow
/// keys reach a box that declares `ignoreArrows`. That Alt is the modifier for
/// this is an inference: Shift and Ctrl are identified by what they do
/// elsewhere, Alt is the remaining modifier, and the behaviour matches what
/// later clients call `SetAltArrowKeyMode`.
///
/// Ctrl is not here: a Ctrl chord arrives as its own [`Stroke`] variant,
/// because which chord a key forms depends on the keyboard layout, which
/// [`super::super::api::keyboard`] handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Extend the selection rather than dropping it.
    pub shift: bool,
    /// Reach a box that declares `ignoreArrows` anyway.
    pub alt: bool,
}

/// Apply a run of keystrokes to the focused box, and return the errors raised.
///
/// Does nothing when no box is focused, which is the ordinary case and the one
/// the caller checks first.
///
/// The second value is the text to put on the clipboard, if a stroke asked for
/// a copy. This file decides what is copied and [`super::super::api::keyboard`]
/// does the copying, the same split this directory uses for every other
/// outside resource. `None` means leave the clipboard unchanged, which is what
/// an empty selection produces.
pub(in crate::lua) fn dispatch(
    lua: &mlua::Lua,
    strokes: &[Stroke],
) -> mlua::Result<(Vec<String>, Option<String>)> {
    let mut errors = Vec::new();
    let mut copied = None;
    for stroke in strokes {
        // Re-read per stroke: `OnEnterPressed` hides the box and drops the
        // focus, so a later keystroke in the same frame goes to no box.
        let Some(frame) = focused(lua) else { break };
        match apply(lua, &frame, stroke) {
            // The last copy in a frame wins; in practice a frame has at most one.
            Ok(Some(text)) => copied = Some(text),
            Ok(None) => {}
            Err(e) => errors.push(format!("{}: {}", stroke_name(stroke), first_line(&e))),
        }
    }
    Ok((errors, copied))
}

fn stroke_name(stroke: &Stroke) -> &'static str {
    match stroke {
        Stroke::Char(_) => "OnChar",
        Stroke::Enter => "OnEnterPressed",
        Stroke::Escape => "OnEscapePressed",
        Stroke::Tab => "OnTabPressed",
        _ => "edit",
    }
}

/// One stroke, and the text it wants copied — see [`dispatch`].
pub(in crate::lua) fn apply(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    stroke: &Stroke,
) -> mlua::Result<Option<String>> {
    let nothing = |result: mlua::Result<()>| result.map(|()| None);
    match stroke {
        // The space bar inserts a character and also runs `OnSpacePressed`.
        // `ChatEdit_OnSpacePressed` re-parses the line, which turns `/s ` into
        // a `SAY` header as soon as the command is typed rather than when the
        // line is sent.
        Stroke::Char(text) => {
            insert(lua, frame, text)?;
            let _ = super::frames::run_script(lua, frame, "OnChar", &[EventArg::Text(text.clone())]);
            if text == " " {
                let _ = super::frames::run_script(lua, frame, "OnSpacePressed", &[]);
            }
            Ok(None)
        }
        Stroke::Backspace => nothing(remove(lua, frame, -1)),
        Stroke::Delete => nothing(remove(lua, frame, 1)),
        Stroke::Left(mods) => nothing(step(lua, frame, -1, *mods)),
        Stroke::Right(mods) => nothing(step(lua, frame, 1, *mods)),
        // `ignoreArrows` does not affect Home and End; in the 1.12.1 client it
        // applies to the four arrow keys only. The chat line, which declares
        // the attribute, can therefore still be selected end to end with
        // Shift-Home.
        Stroke::Home(mods) => nothing(move_caret(lua, frame, 0, mods.shift)),
        Stroke::End(mods) => {
            let letters = text(frame).chars().count() as i64;
            nothing(move_caret(lua, frame, letters, mods.shift))
        }
        Stroke::Up(mods) => nothing(history(lua, frame, -1, *mods)),
        Stroke::Down(mods) => nothing(history(lua, frame, 1, *mods)),
        Stroke::SelectAll => nothing(select(lua, frame, 0, -1)),
        // With nothing selected nothing is copied, and the clipboard keeps
        // what it held, as in the 1.12.1 client.
        Stroke::Copy => Ok(selected_text(frame)),
        Stroke::Cut => {
            let copied = selected_text(frame);
            if copied.is_some() {
                remove(lua, frame, 1)?;
            }
            Ok(copied)
        }
        Stroke::Paste(text) => {
            insert(lua, frame, text)?;
            Ok(None)
        }
        // These three only run their scripts. `ChatEdit_OnEnterPressed`, in
        // the archive, defines what Enter does: send, remember the sticky
        // type, hide. Clearing the box here as well would duplicate that
        // script's work.
        Stroke::Enter => nothing(super::frames::run_script(lua, frame, "OnEnterPressed", &[])),
        Stroke::Escape => nothing(super::frames::run_script(lua, frame, "OnEscapePressed", &[])),
        Stroke::Tab => nothing(super::frames::run_script(lua, frame, "OnTabPressed", &[])),
    }
}

/// Put text in at the caret, respecting the letter cap.
///
/// A selection is deleted first. The 1.12.1 client does this before the cap
/// and the numeric filter apply, so a typed character and a paste both replace
/// what is highlighted.
fn insert(lua: &mlua::Lua, frame: &mlua::Table, typed: &str) -> mlua::Result<()> {
    let selected = selection(frame);
    let (current, at) = match selected {
        Some((start, end)) => (without(&text(frame), start, end), start),
        None => (text(frame), caret(frame)),
    };
    let letters = current.chars().count();
    let cap = frame
        .raw_get::<Option<i64>>(MAX_LETTERS_KEY)
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0) as usize;
    // A full box drops what does not fit, with no sound and no message, as
    // the 1.12.1 client does. `letters="255"` on the chat line matches the
    // server's `CMSG_MESSAGECHAT` limit. For one keystroke the whole keystroke
    // is dropped; for a paste the tail is, because the 1.12.1 client pastes
    // one character at a time through this path and each meets the cap on its
    // own.
    let room = if cap == 0 { usize::MAX } else { cap.saturating_sub(letters) };
    let typed: String = typed.chars().take(room).collect();
    if typed.is_empty() && selected.is_none() {
        return Ok(());
    }
    let mut next = String::with_capacity(current.len() + typed.len());
    next.extend(current.chars().take(at));
    next.push_str(&typed);
    next.extend(current.chars().skip(at));
    write(lua, frame, &next, at + typed.chars().count())
}

/// Backspace (`-1`) or Delete (`+1`). If there is a selection, it is removed
/// instead, whatever the direction, and no character beyond it is removed.
fn remove(lua: &mlua::Lua, frame: &mlua::Table, direction: i64) -> mlua::Result<()> {
    let current = text(frame);
    if let Some((start, end)) = selection(frame) {
        return write(lua, frame, &without(&current, start, end), start);
    }
    let at = caret(frame);
    let cut = if direction < 0 { at.checked_sub(1) } else { Some(at) };
    let Some(cut) = cut.filter(|cut| *cut < current.chars().count()) else {
        return Ok(());
    };
    write(lua, frame, &without(&current, cut, cut + 1), cut)
}

/// Move the caret one character, unless the box declares `ignoreArrows` and Alt
/// is not held to override it.
fn step(lua: &mlua::Lua, frame: &mlua::Table, direction: i64, mods: Mods) -> mlua::Result<()> {
    if ignores_arrows(frame, mods) {
        return Ok(());
    }
    move_caret(lua, frame, caret(frame) as i64 + direction, mods.shift)
}

/// Whether this box ignores an arrow key. In the 1.12.1 client `ignoreArrows`
/// leaves the four arrow keys unhandled, which lets the character turn while
/// the chat line is open, unless Alt is held, in which case the box takes
/// them. [`Mods`] states how the modifier was identified.
fn ignores_arrows(frame: &mlua::Table, mods: Mods) -> bool {
    !mods.alt
        && frame
            .raw_get::<Option<bool>>(IGNORE_ARROWS_KEY)
            .ok()
            .flatten()
            .unwrap_or(false)
}

/// Walk the history list. `-1` is older (Up), `1` is newer (Down).
fn history(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    direction: i64,
    mods: Mods,
) -> mlua::Result<()> {
    if ignores_arrows(frame, mods) {
        return Ok(());
    }
    let Some(lines) = frame.raw_get::<Option<mlua::Table>>(HISTORY_KEY)? else {
        return Ok(());
    };
    let count = lines.raw_len();
    if count == 0 {
        return Ok(());
    }
    // `count + 1` is "past the newest", which is the empty line the box shows
    // when a walk comes back down to where it started.
    let at = frame.raw_get::<Option<i64>>(HISTORY_AT_KEY)?.unwrap_or(count as i64 + 1);
    let at = (at + direction).clamp(1, count as i64 + 1);
    frame.set(HISTORY_AT_KEY, at)?;
    let line: String = if at > count as i64 {
        String::new()
    } else {
        lines.raw_get::<Option<String>>(at as usize)?.unwrap_or_default()
    };
    let letters = line.chars().count();
    write(lua, frame, &line, letters)
}

/// The one place an edit changes the text, so each of the five edits above
/// fires `OnTextChanged`.
///
/// `SetText` is not used here because it also fires `OnTextSet`, which 1.12
/// fires only when the text is set, not when it is typed.
/// `ChatEdit_OnTextSet` re-parses the line, so firing it per keystroke would
/// remove the `/` of a command while it is being typed.
fn write(lua: &mlua::Lua, frame: &mlua::Table, next: &str, caret: usize) -> mlua::Result<()> {
    super::regions::set_text_value(lua, frame, mlua::Value::String(lua.create_string(next)?))?;
    widget::set_paint(lua, frame, CARET_KEY, caret as i64)?;
    // Every edit ends the selection, because the range no longer refers to
    // the same text.
    clear_selection(lua, frame)?;
    let _ = super::frames::run_script(lua, frame, "OnTextChanged", &[]);
    Ok(())
}

fn first_line(e: &mlua::Error) -> String {
    let text = e.to_string();
    text.lines().next().unwrap_or_default().to_string()
}

/// Install [`METHODS`] onto the shared frame method table. See
/// [`super::frames::register_methods`], and [`super::button`] for why one
/// table serves every kind.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // A box that is not visible takes no focus. Several panels end their
    // accept handler with `HideUIPanel(…); ChatFrameEditBox:SetFocus();`
    // (the guild registrar's purchase button is one), which returns the
    // keyboard to the chat line when it is open. With the chat line closed
    // the call must do nothing: a hidden box holding the focus suppresses
    // every key binding and shows nowhere that it has it.
    let set_focus = lua.create_function(|lua, this: mlua::Table| {
        if !super::layout::visible(&this) {
            return Ok(());
        }
        take_focus(lua, &this)
    })?;
    methods.set("SetFocus", set_focus)?;
    let clear_focus = lua.create_function(|lua, this: mlua::Table| release_focus(lua, &this))?;
    methods.set("ClearFocus", clear_focus)?;
    let has_focus = lua.create_function(|lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(focused(lua) == Some(this)))
    })?;
    methods.set("HasFocus", has_focus)?;

    // `editBox:Insert(text)`: 1.12 pastes an item link into the chat line with
    // it, and to `ChatEdit_ParseText` the result is the same as typing.
    let insert_method =
        lua.create_function(|lua, (this, text): (mlua::Table, Option<String>)| {
            insert(lua, &this, &text.unwrap_or_default())
        })?;
    methods.set("Insert", insert_method)?;

    // `HighlightText([start, end])`. The shipped directory uses both call
    // shapes, on two adjacent lines of `InputBoxTemplate`:
    //
    // ```xml
    // <OnEditFocusGained>this:HighlightText();</OnEditFocusGained>
    // <OnEditFocusLost>this:HighlightText(0, 0);</OnEditFocusLost>
    // ```
    //
    // So a click into a money field or a stack-split box selects everything it
    // holds and the first digit typed replaces it. The bare call is
    // `(0, -1)`, because that is what Ctrl-A sends and what [`select`] reads
    // as "to the end".
    let highlight = lua.create_function(
        |lua, (this, start, end): (mlua::Table, Option<i64>, Option<i64>)| {
            select(lua, &this, start.unwrap_or(0), end.unwrap_or(-1))
        },
    )?;
    methods.set("HighlightText", highlight)?;

    // `SetTextInsets(15 + header:GetWidth(), 13, 0, 0)`. A nil argument is
    // read as zero rather than raising, for the reason [`super::widget`]'s
    // `SetID` gives: an error would abort the rest of `ChatEdit_UpdateHeader`.
    let set_insets = lua.create_function(
        |lua, (this, l, r, t, b): (mlua::Table, Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
            let table = lua.create_table()?;
            for (name, value) in ["left", "right", "top", "bottom"].into_iter().zip([l, r, t, b]) {
                table.set(name, value.unwrap_or(0.0))?;
            }
            this.set(INSETS_KEY, table)?;
            widget::mark_paint(lua);
            Ok(())
        },
    )?;
    methods.set("SetTextInsets", set_insets)?;

    let set_auto_focus = lua.create_function(|_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        this.set(
            AUTO_FOCUS_KEY,
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("SetAutoFocus", set_auto_focus)?;
    // `SetAltArrowKeyMode`: whether the arrow keys walk the history only with
    // Alt held. Recorded only; this client's history walk reads no flag.
    // Every chat-frame addon sets it on the edit box.
    let set_alt_arrow = lua.create_function(|_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        this.set(
            "__altArrowKeyMode",
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("SetAltArrowKeyMode", set_alt_arrow)?;
    // `SetMultiLine`: recorded only. The box draws one line whatever the flag;
    // the module comment lists this under the differences from 1.12.1.
    let set_multi_line = lua.create_function(|_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        this.set(
            "__multiLine",
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("SetMultiLine", set_multi_line)?;

    // `SetJustifyH` on a frame applies to the frame's own string, as
    // `SetTextColor` does: a button's label, or the box's typed text, which
    // this client draws left-aligned whatever the setting.
    for (name, key) in [("SetJustifyH", "justifyH"), ("SetJustifyV", "justifyV")] {
        let set = lua.create_function(move |lua, (this, how): (mlua::Table, Option<String>)| {
            let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
            super::regions::set_justify(lua, &target, key, how.as_deref().unwrap_or("CENTER"))
        })?;
        methods.set(name, set)?;
    }

    // `SetTextColor` on a frame has three meanings. On a frame with no font
    // string of its own, which includes an edit box, it sets the colour of
    // the text the frame draws itself; this tints the chat line by the chat
    // type. On a button it sets the normal face's colour, which is how a
    // button greys its label. On any other frame it sets the frame's own font
    // string's colour.
    let set_colour = lua.create_function(
        |lua, (this, r, g, b, a): (mlua::Table, Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
            let rgba = vec![
                r.unwrap_or(1.0),
                g.unwrap_or(1.0),
                b.unwrap_or(1.0),
                a.unwrap_or(1.0),
            ];
            // On a button the colour is the normal face's and the other faces
            // keep their own; see [`super::button::set_text_colour`]. On any
            // other frame it marks the string as carrying its own colour, so a
            // later face change does not erase it; see
            // [`super::regions::set_text_colour`].
            match super::regions::text_region(&this) {
                Some(region) if super::button::is_button(&this) => super::button::set_text_colour(
                    lua,
                    &this,
                    &region,
                    [rgba[0], rgba[1], rgba[2], rgba[3]],
                ),
                Some(region) => super::regions::set_text_colour(
                    lua,
                    &region,
                    [rgba[0], rgba[1], rgba[2], rgba[3]],
                ),
                None => {
                    // Handlers set the same colour every tick, so an unchanged one is not a repaint.
                    let stored: Option<Vec<f64>> = this.raw_get(COLOUR_KEY).ok().flatten();
                    if stored.as_deref() == Some(rgba.as_slice()) {
                        return Ok(());
                    }
                    this.set(COLOUR_KEY, rgba)?;
                    widget::mark_paint(lua);
                    Ok(())
                }
            }
        },
    )?;
    methods.set("SetTextColor", set_colour)?;

    // `SetFont` and `SetFontObject` on a frame forward the same way. The face
    // of a button, message frame or edit box is set on its own string when it
    // has one, and on the frame's own keys otherwise, which is where the edit
    // box and the message frame keep theirs.
    let set_font = lua.create_function(
        |lua, (this, path, height, flags): (mlua::Table, Option<String>, Option<f64>, Option<String>)| {
            let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
            super::regions::set_font_triplet(lua, &target, path.as_deref(), height, flags.as_deref())
        },
    )?;
    methods.set("SetFont", set_font)?;
    let set_font_object = lua.create_function(|lua, (this, value): (mlua::Table, mlua::Value)| {
        let Some(font) = super::regions::resolve_font_object(lua, value)? else {
            return Ok(());
        };
        let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
        super::regions::apply_font_style(lua, &target, &font)?;
        target.set("__fontObject", font)
    })?;
    methods.set("SetFontObject", set_font_object)?;
    let get_font_object = lua.create_function(|_lua, this: mlua::Table| {
        let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
        target.raw_get::<mlua::Value>("__fontObject")
    })?;
    methods.set("GetFontObject", get_font_object)?;

    let set_max = lua.create_function(|_lua, (this, letters): (mlua::Table, Option<i64>)| {
        this.set(MAX_LETTERS_KEY, letters.unwrap_or(0).max(0))
    })?;
    methods.set("SetMaxLetters", set_max)?;

    // `SetNumber`/`GetNumber` read and write the same string as a number. The
    // stack-split and money entry boxes use only these.
    let set_number = lua.create_function(|lua, (this, value): (mlua::Table, Option<f64>)| {
        let value = value.unwrap_or(0.0);
        // Integers print without a decimal point, as a stack size must; 1.12
        // has one number type and formats it the same way.
        let text = if value.fract() == 0.0 {
            format!("{}", value as i64)
        } else {
            format!("{value}")
        };
        super::regions::set_text_value(lua, &this, mlua::Value::String(lua.create_string(&text)?))?;
        text_was_set(lua, &this)
    })?;
    methods.set("SetNumber", set_number)?;
    let get_number = lua.create_function(|_lua, this: mlua::Table| {
        Ok(text(&this).trim().parse::<f64>().unwrap_or(0.0))
    })?;
    methods.set("GetNumber", get_number)?;

    // `AddHistoryLine` is called by `ChatEdit_ParseText` on every command it
    // recognises and by `ChatEdit_SendText` on every line sent.
    let add_history = lua.create_function(|lua, (this, line): (mlua::Table, Option<String>)| {
        let Some(line) = line.filter(|line| !line.is_empty()) else {
            return Ok(());
        };
        let lines = match this.raw_get::<Option<mlua::Table>>(HISTORY_KEY)? {
            Some(lines) => lines,
            None => {
                let lines = lua.create_table()?;
                this.set(HISTORY_KEY, lines.clone())?;
                lines
            }
        };
        lines.push(line)?;
        let cap = this
            .raw_get::<Option<i64>>(HISTORY_LINES_KEY)?
            .filter(|cap| *cap > 0)
            .unwrap_or(DEFAULT_HISTORY_LINES as i64) as usize;
        while lines.raw_len() > cap {
            lines.raw_remove(1)?;
        }
        // A fresh line puts the walk back at the end, which is where the next
        // Up starts from.
        this.set(HISTORY_AT_KEY, mlua::Value::Nil)
    })?;
    methods.set("AddHistoryLine", add_history)?;

    // The keyboard input language, not the in-game spoken language.
    // `INPUT_ROMAN = "A"` is the label `ChatEdit_OnInputLanguageChanged` puts
    // on the button beside the line; the other three values are the CJK input
    // methods 1.12 shipped for.
    let input_language = lua.create_function(|_lua, _this: mlua::Table| Ok("ROMAN"))?;
    methods.set("GetInputLanguage", input_language)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host with the object model installed and one edit box in it.
    fn boxed(extra: &str) -> mlua::Lua {
        let lua = mlua::Lua::new();
        super::super::frames::install(&lua).expect("the object model installs");
        lua.load(format!(
            r#"
            typed, set, changed = 0, 0, 0;
            entered, escaped = 0, 0;
            edit = CreateFrame("EditBox", "TestEditBox");
            edit:SetScript("OnChar", function() typed = typed + 1; last = arg1; end);
            edit:SetScript("OnTextSet", function() set = set + 1; end);
            edit:SetScript("OnTextChanged", function() changed = changed + 1; end);
            edit:SetScript("OnEnterPressed", function() entered = entered + 1; end);
            edit:SetScript("OnEscapePressed", function() escaped = escaped + 1; end);
            {extra}
            "#
        ))
        .exec()
        .expect("the chunk runs");
        lua
    }

    fn edit(lua: &mlua::Lua) -> mlua::Table {
        lua.globals().get("edit").expect("the edit box")
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    fn type_in(lua: &mlua::Lua, line: &str) {
        let strokes: Vec<Stroke> = line.chars().map(|c| Stroke::Char(c.to_string())).collect();
        press(lua, &strokes);
    }

    /// Apply strokes, assert none of them raised, and return the text to copy.
    fn press(lua: &mlua::Lua, strokes: &[Stroke]) -> Option<String> {
        let (errors, copied) = dispatch(lua, strokes).expect("the strokes apply");
        assert!(errors.is_empty(), "{errors:?}");
        copied
    }

    /// Shift held, which is the only modifier most of these need.
    const SHIFT: Mods = Mods { shift: true, alt: false };
    const PLAIN: Mods = Mods { shift: false, alt: false };

    /// A key reaches only the box that has the focus. Without a focus there is
    /// no typing, which keeps WASD from becoming text.
    #[test]
    fn only_a_focused_box_takes_the_keyboard() {
        let lua = boxed("");
        type_in(&lua, "hello");
        // `""`, not nil: in 1.12 an empty edit box returns an empty string
        // where an empty font string returns nil, and
        // `MoneyInputFrame_GetCopper` depends on that. See
        // [`super::regions::empty_text`].
        assert_eq!(
            eval(&lua, "return edit:GetText()"),
            r#"String("")"#,
            "no focus, no text"
        );
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Nil");

        lua.load("edit:SetFocus()").exec().expect("runs");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Integer(1)");
        type_in(&lua, "hello");
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("hello")"#);
        assert_eq!(eval(&lua, "return typed"), "Integer(5)");
    }

    /// The caret is an insertion point rather than an append: typing in the
    /// middle of a line puts the characters where the caret is.
    #[test]
    fn the_caret_is_where_the_next_character_lands() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "helo");
        press(&lua, &[Stroke::Left(PLAIN), Stroke::Char("l".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("hello")"#);
        // Backspace removes the character before the caret, Delete the one after.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Delete]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ello")"#);
        press(&lua, &[Stroke::End(PLAIN), Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ell")"#);
        // Neither goes past the end of the string.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Backspace, Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ell")"#);
    }

    /// `ignoreArrows`, declared by the chat line, stops the arrows moving the
    /// caret, so that in the 1.12.1 client they turn the character instead.
    #[test]
    fn a_box_that_ignores_arrows_does_not_move_its_caret() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "ignoreArrows", "true").expect("the attribute applies");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "abc");
        press(
            &lua,
            &[Stroke::Left(PLAIN), Stroke::Left(PLAIN), Stroke::Char("X".to_string())],
        );
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abcX")"#);

        // With Alt held, the same two arrows move the caret.
        let alt = Mods { shift: false, alt: true };
        press(&lua, &[Stroke::Left(alt), Stroke::Left(alt), Stroke::Char("Y".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abYcX")"#);

        // Home and End are not affected, so the chat line can be selected end
        // to end. Only the four arrow keys are.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Char("Z".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ZabYcX")"#);
    }

    /// A selection is two indices and Shift extends it. The other selection
    /// tests depend on this.
    #[test]
    fn shift_extends_and_a_bare_move_drops_it() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "hello");
        assert_eq!(selection(&edit(&lua)), None, "typing selects nothing");

        // Home with Shift from the end: the whole line, ordered forwards even
        // though it was gathered backwards.
        press(&lua, &[Stroke::Home(SHIFT)]);
        assert_eq!(selection(&edit(&lua)), Some((0, 5)));
        // A plain move drops the selection rather than shrinking it.
        press(&lua, &[Stroke::End(PLAIN)]);
        assert_eq!(selection(&edit(&lua)), None);
        // One Shift-Left selects one character from the caret.
        press(&lua, &[Stroke::Left(SHIFT)]);
        assert_eq!(selection(&edit(&lua)), Some((4, 5)));
        // Shrinking back to the anchor leaves no selection, the equivalent of
        // equal start and end in the 1.12.1 client.
        press(&lua, &[Stroke::Right(SHIFT)]);
        assert_eq!(selection(&edit(&lua)), None);
    }

    /// Typing over a selection replaces it, and so does a paste; one Backspace
    /// removes it.
    #[test]
    fn an_edit_replaces_what_is_selected() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "hello world");

        // Select "hello" and type over it.
        press(&lua, &[Stroke::Home(PLAIN)]);
        for _ in 0..5 {
            press(&lua, &[Stroke::Right(SHIFT)]);
        }
        assert_eq!(selection(&edit(&lua)), Some((0, 5)));
        press(&lua, &[Stroke::Char("H".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("H world")"#);
        assert_eq!(selection(&edit(&lua)), None, "the edit ended the selection");

        // A paste replaces a selection too, whichever direction it was made in.
        press(&lua, &[Stroke::SelectAll, Stroke::Paste("goodbye".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("goodbye")"#);

        // Backspace with a selection removes the selection and no character
        // beyond it.
        press(&lua, &[Stroke::SelectAll, Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("")"#);
    }

    /// Ctrl-C, Ctrl-X and Ctrl-A. An empty selection copies nothing, not the
    /// whole line.
    #[test]
    fn copy_and_cut_answer_only_what_is_selected() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "hello");

        assert_eq!(press(&lua, &[Stroke::Copy]), None, "nothing selected, nothing copied");
        assert_eq!(press(&lua, &[Stroke::Cut]), None);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("hello")"#);

        // Ctrl-A is `HighlightText(0, -1)`.
        assert_eq!(press(&lua, &[Stroke::SelectAll]), None, "selecting copies nothing");
        assert_eq!(selection(&edit(&lua)), Some((0, 5)));
        assert_eq!(press(&lua, &[Stroke::Copy]), Some("hello".to_string()));
        assert_eq!(
            eval(&lua, "return edit:GetText()"),
            r#"String("hello")"#,
            "a copy is not an edit"
        );

        // A cut copies and then deletes.
        press(&lua, &[Stroke::Home(PLAIN)]);
        for _ in 0..2 {
            press(&lua, &[Stroke::Right(SHIFT)]);
        }
        assert_eq!(press(&lua, &[Stroke::Cut]), Some("he".to_string()));
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("llo")"#);
    }

    /// The two `HighlightText` call shapes `InputBoxTemplate` makes on focus
    /// gain and loss; the bare call selects everything.
    #[test]
    fn highlight_text_takes_the_directorys_two_shapes() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "1234");

        lua.load("edit:HighlightText()").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), Some((0, 4)));
        lua.load("edit:HighlightText(0, 0)").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), None);
        // A range past the end is clamped to the text rather than raising.
        lua.load("edit:HighlightText(1, 99)").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), Some((1, 4)));
    }

    /// A password box copies the empty string, as in the 1.12.1 client. A cut
    /// still deletes; only the clipboard gets nothing.
    #[test]
    fn a_password_box_copies_nothing() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "password", "1").expect("the attribute applies");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "hunter2");
        assert_eq!(press(&lua, &[Stroke::SelectAll, Stroke::Copy]), Some(String::new()));
        assert_eq!(
            eval(&lua, "return edit:GetText()"),
            r#"String("hunter2")"#,
            "the box still holds it — it is the clipboard that is refused"
        );
        assert_eq!(press(&lua, &[Stroke::SelectAll, Stroke::Cut]), Some(String::new()));
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("")"#);
    }

    /// The letter cap truncates a paste rather than refusing it, because the
    /// 1.12.1 client pastes one character at a time through the insert path
    /// and each character meets the cap on its own.
    #[test]
    fn the_cap_takes_as_much_of_a_paste_as_fits() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "letters", "5").expect("the attribute applies");
        lua.load("edit:SetFocus()").exec().expect("runs");
        press(&lua, &[Stroke::Paste("abcdefgh".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abcde")"#);
        // The replaced selection frees its room first, so select-all then
        // paste works in a full box.
        press(&lua, &[Stroke::SelectAll, Stroke::Paste("xyz".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("xyz")"#);
    }

    /// `SetText` fires `OnTextSet` and typing does not. This lets
    /// `ChatEdit_ParseText` cut `/s ` off a line without removing the slash as
    /// it is typed.
    #[test]
    fn setting_the_text_and_typing_it_fire_different_scripts() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        lua.load(r#"edit:SetText("hi")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return set"), "Integer(1)");
        assert_eq!(eval(&lua, "return changed"), "Integer(1)");
        assert_eq!(eval(&lua, "return typed"), "Integer(0)");

        type_in(&lua, "!");
        assert_eq!(eval(&lua, "return set"), "Integer(1)", "typing does not set");
        assert_eq!(eval(&lua, "return changed"), "Integer(2)");
        // The caret moved to the end of the set text, so the typing appended.
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("hi!")"#);
    }

    /// Enter and Escape run scripts and do not edit the text; their behaviour
    /// is in the archive's `ChatEdit_*` handlers.
    #[test]
    fn enter_and_escape_run_the_games_own_bodies() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "hi");
        dispatch(&lua, &[Stroke::Enter]).expect("runs");
        assert_eq!(eval(&lua, "return entered"), "Integer(1)");
        assert_eq!(
            eval(&lua, "return edit:GetText()"),
            r#"String("hi")"#,
            "Enter alone does not clear the line — the handler does"
        );
        dispatch(&lua, &[Stroke::Escape]).expect("runs");
        assert_eq!(eval(&lua, "return escaped"), "Integer(1)");
    }

    /// Hiding drops the focus. `ChatEdit_OnEscapePressed` ends in `Hide()`
    /// and nothing else, so without this every keystroke after the first sent
    /// message goes into an invisible box.
    #[test]
    fn hiding_the_box_gives_the_keyboard_back() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Integer(1)");
        lua.load("edit:Hide()").exec().expect("runs");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Nil");
        assert!(focused(&lua).is_none());
    }

    /// `SetFocus` on a hidden box does nothing, so a panel that hands the
    /// keyboard back to a closed chat line does not leave it on a box nobody
    /// can see.
    #[test]
    fn a_hidden_box_takes_no_focus() {
        let lua = boxed("");
        lua.load("edit:Hide(); edit:SetFocus()").exec().expect("runs");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Nil");
        assert!(focused(&lua).is_none());
        lua.load("edit:Show(); edit:SetFocus()").exec().expect("runs");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Integer(1)");
    }

    /// The letter cap is the box's own `letters` attribute, and a full box drops
    /// the keystroke rather than raising.
    #[test]
    fn the_letter_cap_is_the_markups_own() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "letters", "3").expect("the attribute applies");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "abcdef");
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abc")"#);
    }

    /// The history is what `AddHistoryLine` was given, walked oldest-upwards,
    /// and capped by `historyLines`.
    #[test]
    fn the_arrows_walk_what_was_remembered() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "historyLines", "2").expect("the attribute applies");
        lua.load(
            r#"edit:SetFocus();
               edit:AddHistoryLine("first");
               edit:AddHistoryLine("second");
               edit:AddHistoryLine("third");"#,
        )
        .exec()
        .expect("runs");
        press(&lua, &[Stroke::Up(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("third")"#);
        press(&lua, &[Stroke::Up(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("second")"#);
        // "first" was dropped from the two-line history.
        press(&lua, &[Stroke::Up(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("second")"#);
        press(&lua, &[Stroke::Down(PLAIN), Stroke::Down(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("")"#);
    }

    /// Every name in [`METHODS`] is installed, and the list is sorted. Every
    /// such list in this directory follows this rule, because `vale framexml`
    /// measures the interface's missing methods against them.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = boxed("");
        for name in METHODS {
            assert_eq!(
                eval(&lua, &format!("return type(edit.{name})")),
                r#"String("function")"#,
                "{name} is claimed in METHODS and is not installed"
            );
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }

    /// A second `SetFocus` moves the focus rather than adding one, and both
    /// boxes receive their focus script.
    #[test]
    fn the_focus_moves_and_says_so() {
        let lua = boxed(
            r#"gained, lost = 0, 0;
               other = CreateFrame("EditBox", "OtherEditBox");
               edit:SetScript("OnEditFocusLost", function() lost = lost + 1; end);
               other:SetScript("OnEditFocusGained", function() gained = gained + 1; end);"#,
        );
        lua.load("edit:SetFocus(); other:SetFocus()").exec().expect("runs");
        assert_eq!(eval(&lua, "return lost"), "Integer(1)");
        assert_eq!(eval(&lua, "return gained"), "Integer(1)");
        assert_eq!(eval(&lua, "return edit:HasFocus()"), "Nil");
        assert_eq!(eval(&lua, "return other:HasFocus()"), "Integer(1)");
    }
}
