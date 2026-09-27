//! **The one widget that takes the keyboard**: `EditBox`.
//!
//! Sixteen of them in the shipped directory, and the one that matters is
//! `ChatFrameEditBox` — the line at the bottom of the screen that everything a
//! player *says* goes through, `.tele` and `/script` included. Until this file
//! the client had a stand-in for it in egui, because an edit box is the one
//! widget whose state cannot be derived from the world: it is a string, a caret
//! and a focus flag that only the keyboard writes.
//!
//! ## The whole chain is the game's own, and this is the bottom of it
//!
//! ```text
//! Enter                     the key table, this client's half
//!   -> OPENCHAT             Bindings.xml, declaration 87
//!   -> ChatFrame_OpenChat("")            ChatFrame.lua:1545
//!   -> editBox:Show()  -> OnShow -> ChatEdit_OnShow -> this:SetFocus()
//!   … typing …             lua::keyboard, the strokes -> this file
//!   -> Enter -> OnEnterPressed -> ChatEdit_SendText   ChatFrame.lua:1940
//!   -> ChatEdit_ParseText decides the *kind* off SLASH_* and SlashCmdList
//!   -> SendChatMessage(text, "SAY")      lua::verbs, a registered closure
//! ```
//!
//! Every line of that except the first and the last is the archive's. That is
//! the point of the round it arrived in: the deleted egui pane had its own
//! slash-command table, its own `/script` check and its own idea of what a `.`
//! meant, and all three are decisions `ChatFrame.lua` already makes — one of
//! them differently. (`SLASH_SCRIPT2 = "/run"`: 1.12 ships `/run` as a synonym,
//! where the stand-in refused it as "a later client's".)
//!
//! ## The text lives where every other widget's text lives
//!
//! `SetText` and `GetText` are [`super::regions`]' already — a frame with no
//! font string keeps its string on itself — so an edit box needed no store of
//! its own and does not have one. What is here is the state around it: the
//! **caret**, the **focus**, the insets the header pushes the text past, the
//! letter cap and the history. See [`text_was_set`], which is the hook that
//! makes the shared `SetText` fire `OnTextSet` for the one kind that has one.
//!
//! ## Focus is a single registry slot, and hiding drops it
//!
//! There is exactly one keyboard focus in the client, so it is one registry
//! key rather than a flag per frame — the same shape [`super::super::api::mouse`] uses for
//! the pointer's. `Hide()` clears it if it was ours, which is not a nicety:
//! `ChatEdit_OnEscapePressed` ends with `editBox:Hide()` and nothing else, so
//! without that line every keystroke after the first message would still be
//! swallowed by an invisible box.
//!
//! ## A selection is two indices, and every one of its rules is the client's
//!
//! An edit box keeps `selStart` and `selEnd`, and **`start == end` is how it
//! says "nothing is selected"**. Everything else follows from that pair:
//!
//! * **an insert deletes the selection first** — it is the third thing
//!   the insert path does, before the letter cap and before the numeric filter,
//!   so a select-all-then-type *replaces*, and so does a paste.
//! * **a paste is inserted a character at a time** through that same path,
//!   which is why the letter cap **truncates** a paste rather than
//!   refusing it.
//! * **copy and cut do nothing at all with an empty selection** — each returns
//!   before the clipboard is opened. Not "copy the whole line": nothing.
//! * **every caret move takes "is Shift held" as its extend flag** — each
//!   reads modifier 0 and pushes it into the move. There is no separate
//!   select-mode.
//! * **`SetSelection` treats an end below the start as "to the end of the
//!   text"**, which is the whole of why `HighlightText(0, -1)` — what Ctrl-A
//!   does — selects everything.
//!
//! [`Mods`] is the two modifiers a box reads, and no more: modifier 0 (Shift)
//! extends, modifier 1 (Ctrl) is the chord, and modifier 2 is the one that lets
//! an arrow past `ignoreArrows` — see [`ignores_arrows`].
//!
//! ## What is not modelled, each of them visible
//!
//! * **the caret is not an endpoint of the selection in the reference.** It
//!   keeps a third index, so `HighlightText(start, end)` there
//!   selects without moving it. This holds an **anchor and the caret**, which is
//!   the same thing for every path a keystroke can take and differs only for a
//!   `HighlightText` followed by a Shift-move. A reconstruction, and the only
//!   one in the selection.
//! * **the readline family is measured and not implemented.** The same key
//!   table carries Ctrl-B/F (move), Ctrl-P/N (history), Ctrl-K/U/W (kill) and
//!   Ctrl-D (delete forward) — and nothing in the
//!   shipped directory or its bindings mentions any of them.
//! * **the caret's x is measured now**, in the box's own face, off the game's
//!   own `.TTF` — see [`super::text::width`]. It was an estimate of half the
//!   font height per character, which drifted a character's width every eight
//!   capitals; what it took was not the painter answering back but the files
//!   themselves, which say the same thing the painter's layout does.
//! * **`ignoreArrows` is honoured and its other half is not.** The chat box
//!   declares it, so the arrows do not move this caret — but in the real client
//!   they then reach the *game* and turn the character, and here they reach
//!   nothing, because [`super::super::api::keyboard`] suppresses the whole key table while
//!   an edit box has focus.
//! * **no IME and no `numeric`/`multiLine`.** `GetInputLanguage` answers the
//!   Roman keyboard, which is what `INPUT_ROMAN = "A"` in `GlobalStrings.lua`
//!   is the label for.

use super::widget;
use crate::interface::events::EventArg;

/// The methods an edit box carries beyond the ones every frame has. Sorted, and
/// every one of them a name the shipped directory calls.
///
/// `SetText`/`GetText` are deliberately absent: they are [`super::button`]'s
/// forwarding pair on the shared method table, and an edit box wants exactly
/// what they already do.
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

/// `SetAutoFocus` — recorded and read by nothing: this client gives a box the
/// keyboard on a click, and takes it on `ClearFocus`, whatever the flag. An
/// addon's search box sets it false on every screen it builds.
const AUTO_FOCUS_KEY: &str = "__autoFocus";

/// Where the caret sits, as a **character** index into the text — 0 is before
/// the first character and `len` is after the last.
///
/// Characters rather than bytes because everything that reads it is either
/// counting glyphs (the drawn x) or splitting the string (an insert), and a
/// byte index into a UTF-8 string is a panic waiting for the first accented
/// name typed into a whisper.
const CARET_KEY: &str = "__caret";
/// **The other end of the selection**, on the same terms as [`CARET_KEY`], or
/// absent when nothing is selected.
///
/// The reference stores the pair outright (`selStart`, `selEnd`) and says
/// "empty" by making them equal; this stores one of them and
/// says it by leaving the key off. Same states, one fewer way to be
/// inconsistent — and [`selection`] is the only reader, so the difference does
/// not leak.
const ANCHOR_KEY: &str = "__anchor";
/// `password="1"` — `AccountLoginPasswordEdit` is the one box in either
/// directory that declares it. One of the reference's edit-box flags.
const PASSWORD_KEY: &str = "__password";
/// `letters="255"` on the chat template, or `SetMaxLetters`. 0 is "no cap",
/// which is the game's own meaning for it.
const MAX_LETTERS_KEY: &str = "__maxLetters";
/// `SetTextInsets(left, right, top, bottom)` — what `ChatEdit_UpdateHeader`
/// calls with `15 + header:GetWidth()` so the typed text starts after `Say:`.
const INSETS_KEY: &str = "__textInsets";
/// The colour of the typed text, which the same function sets to the chat
/// type's own. Kept here rather than on a region because an edit box's text has
/// no region — see the module comment.
const COLOUR_KEY: &str = "__textColour";
/// Lines `AddHistoryLine` has been given, oldest first, and where an arrow walk
/// currently is inside them.
const HISTORY_KEY: &str = "__history";
const HISTORY_AT_KEY: &str = "__historyAt";
/// `historyLines="32"`, the cap on the list above.
const HISTORY_LINES_KEY: &str = "__historyLines";
/// `ignoreArrows="true"` — the chat box declares it. See the module comment for
/// the half of it this client does not do.
const IGNORE_ARROWS_KEY: &str = "__ignoreArrows";

/// The one focused edit box, or nothing. In the registry rather than a global
/// for the reason [`super::widget::roots`] gives: interface code assigning to a
/// name must not be able to break the keyboard.
const REG_FOCUS: &str = "vale.keyboardFocus";

/// How many history lines a box that declares none keeps. The chat template's
/// own `historyLines="32"` is what every box in the directory that has a
/// history says, so this is a bound rather than a measured default.
const DEFAULT_HISTORY_LINES: usize = 32;

/// **A flag rather than the kind string**, set at creation for the sixteen
/// objects that are edit boxes and absent on the other 3,730 frames.
///
/// [`widget::KIND_KEY`] already says `"EditBox"` and reading it would need no
/// second key — but this is asked once per visible frame per *frame of video*
/// by the draw walk, and `get::<mlua::String>` hands back a Lua string that has
/// to be rooted and dropped. A `bool` is a hash lookup that misses. Measured at
/// one framing: the string form put **0.6 ms** on the interface's median frame,
/// which on a 60 Hz budget is not a rounding error. Same argument, same shape,
/// as [`widget::CLASS_KEY`] next door.
const IS_EDIT_BOX_KEY: &str = "__isEditBox";

/// Give a fresh frame the state this module owns — which for every kind but one
/// is nothing at all. Called from [`super::frames::create_frame`], beside the
/// pointer's own.
pub(in crate::lua) fn init(frame: &mlua::Table, kind: &str) -> mlua::Result<()> {
    if kind != "EditBox" {
        return Ok(());
    }
    frame.set(IS_EDIT_BOX_KEY, true)
}

/// **The five regions an `<EditBox>` is born with**, before a line of its markup
/// is read — one `FontString` and four `Texture`s.
///
/// Measured in the reference, twice. A bare
/// `CreateFrame("EditBox", nil, UIParent)` answers
///
/// ```text
/// 5 | 1:FontString 2:Texture 3:Texture 4:Texture 5:Texture
/// ```
///
/// and `GuildControlPopupFrameEditBox`, which declares two `<Texture>`s and two
/// `<FontString>`s of its own, answers **8** — those five, then the declared
/// pair of textures at 6 and 7, then the label at 8. The trailing
/// `<FontString inherits="ChatFontNormal"/>` adds nothing to the count, because
/// it *configures* the string the widget already made rather than making a
/// second; see [`crate::lua::xml::Loader::object_for`], which is the half that
/// binds them.
///
/// **The count and the order are the measurement; what the four textures are for
/// is a reading.** `GetTexture()` answers `"Solid Texture"` on all four — they
/// are colour-set with no file — which fits the selection highlight and the
/// caret of a widget that supports `SetMultiLine` (three quads for a selection
/// that wraps, and one for the caret) and nothing else in the file suggests
/// otherwise. This client draws neither from these regions: the text and the
/// caret are emitted by [`super::draw::edit_box`] off the frame itself.
///
/// So why make them at all? Because **interface code indexes `GetRegions()`
/// positionally**, and it is not obscure: pfUI's friends skin is
///
/// ```lua
/// local _,_,_,_,_,left,right = GuildControlPopupFrameEditBox:GetRegions()
/// left:Hide() right:Hide()
/// ```
///
/// — the two border textures at 6 and 7. Answering four regions put a nil there,
/// and the raise aborted the `for` loop in pfUI's one `ADDON_LOADED` handler
/// that runs every skin, so **31 of its 36 skins never ran**: the tooltip,
/// questlog, merchant, gossip, mail, options and taxi skins among them.
///
/// They are made with no anchors, so they solve to no rectangle and draw
/// nothing — which is what a caret nobody has put anywhere should do.
pub(in crate::lua) fn furnish(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let text = super::regions::create(lua, "FontString", None, Some(frame.clone()), None)?;
    // **Under a key of this module's own, and deliberately not
    // `regions::TEXT_REGION_KEY`.** That key means "the region this frame's
    // `SetText` writes into", which is a button's `<ButtonText>` — and an edit
    // box's typed line is *not* held in a region: it lives on the frame and is
    // laid out by [`super::draw::edit_box`], because it needs a caret and an
    // inset that no font string models.
    //
    // Putting the string there instead diverted every `SetText` on every edit
    // box in the game into this region, which took the text off the frame the
    // draw reads and — since `regions::own_font` wants the font string with *no*
    // text — left the box with no style either. Measured on the login screen:
    // the account name drew centred, in the wrong face, with no caret.
    frame.set(OWN_STRING_KEY, text)?;
    for _ in 0..4 {
        super::regions::create(lua, "Texture", None, Some(frame.clone()), None)?;
    }
    Ok(())
}

/// **The font string an edit box was born with** — see [`furnish`]. The
/// loader binds the element's own `<FontString>` to it, and
/// `regions::own_font` finds it in the children like any other.
pub(in crate::lua) const OWN_STRING_KEY: &str = "__editString";

/// …and the one reader outside this file: [`crate::lua::xml::Loader::object_for`].
pub(in crate::lua) fn own_string(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.raw_get::<Option<mlua::Table>>(OWN_STRING_KEY).ok().flatten()
}

/// Is this object an edit box? One table read, and the gate on every hook in
/// this file that hangs off a method every frame shares.
pub(in crate::lua) fn is_edit_box(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<bool>>(IS_EDIT_BOX_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// The four `<EditBox>` attributes this client acts on, from the loader — and
/// the loader's own list has to name them, which is what `password` went
/// several rounds without. See [`crate::lua::xml`].
pub(in crate::lua) fn set_from_markup(
    object: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "letters" => object.set(MAX_LETTERS_KEY, value.parse::<i64>().unwrap_or(0)),
        "historyLines" => object.set(HISTORY_LINES_KEY, value.parse::<i64>().unwrap_or(0)),
        // **`password="1"`, not `"true"`** — the glue's own spelling, which is
        // why this takes anything but a zero rather than testing for a word.
        "password" => object.set(PASSWORD_KEY, value != "0"),
        // A tri-state for the same reason `enableMouse` is one: a template may
        // turn its parent template's flag back off.
        "ignoreArrows" => object.set(IGNORE_ARROWS_KEY, value == "true"),
        _ => Ok(()),
    }
}

/// **The focused edit box**, if the keyboard is going to one.
pub fn focused(lua: &mlua::Lua) -> Option<mlua::Table> {
    lua.named_registry_value::<Option<mlua::Table>>(REG_FOCUS)
        .ok()
        .flatten()
}

/// Its name, for the HUD and for [`super::super::api::keyboard::KeyboardFocus`].
pub(in crate::lua) fn focused_name(lua: &mlua::Lua) -> Option<String> {
    focused(lua)?.raw_get::<Option<String>>(widget::NAME_KEY).ok().flatten()
}

/// Take the focus, firing `OnEditFocusLost` on whoever had it and
/// `OnEditFocusGained` on this one.
///
/// **Both, and in that order**, because the two are how a box knows to stop
/// drawing its caret — and because 1.12's own `SetFocus` moves the focus rather
/// than adding one. Errors from the handlers come back to be reported; the
/// focus is moved either way, since a raising handler must not leave the
/// keyboard pointing at a frame that thinks it lost it.
fn take_focus(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if let Some(previous) = focused(lua) {
        if previous == *frame {
            return Ok(());
        }
        lua.set_named_registry_value(REG_FOCUS, mlua::Value::Nil)?;
        let _ = super::frames::run_script(lua, &previous, "OnEditFocusLost", &[]);
    }
    lua.set_named_registry_value(REG_FOCUS, frame.clone())?;
    super::frames::run_script(lua, frame, "OnEditFocusGained", &[])
}

/// **A click on a text field takes the keyboard**, which is the edit box's
/// own doing and not a handler's.
///
/// The whole directory contains no `OnMouseDown` on an `<EditBox>` at all, and
/// the glue's only two `SetFocus` calls are in `AccountLogin_OnShow` — so
/// without this the one box an `OnShow` happens to focus works and every other
/// text field in the game is dead, `CharacterCreateNameEdit` among them. Called
/// from [`super::super::api::mouse::dispatch`] on the left press, beside the slider's grab,
/// because both are the same kind of fact: a widget 1.12 gives its own mouse to.
///
/// A press on anything that is **not** a text field is deliberately *not* a
/// release — the chat line survives a click on the world in the reference, and
/// `ChatEdit_OnEditFocusLost` is what would close it.
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
    super::frames::run_script(lua, frame, "OnEditFocusLost", &[])
}

/// **A hidden edit box does not hold the keyboard.** Called from `Hide`, beside
/// the tooltip's own release, and the reason typing works twice in a row —
/// `ChatEdit_OnEscapePressed` hides and never clears.
pub(super) fn hidden(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if !is_edit_box(frame) {
        return Ok(());
    }
    release_focus(lua, frame)
}

/// **`SetText` landed on an edit box** — reset the caret and fire the two
/// scripts the game fires.
///
/// Called from the shared `SetText` in [`super::button`], which is where the
/// forwarding lives; a no-op for every other kind, at the cost of one table
/// read. `OnTextSet` is what runs `ChatEdit_ParseText`, which is what turns a
/// typed `/s hello` into a `SAY` with the command cut off — so the chat line
/// does not work at all without this hook.
///
/// **It re-enters and that is not a hazard**: `ChatEdit_OnTextSet` parses the
/// text and calls `SetText` with the command *removed*, so each round is
/// strictly shorter and the second one takes the "does not start with /" exit.
pub(super) fn text_was_set(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    if !is_edit_box(frame) {
        return Ok(());
    }
    let text = text(frame);
    frame.set(CARET_KEY, text.chars().count() as i64)?;
    frame.set(ANCHOR_KEY, mlua::Value::Nil)?;
    let _ = super::frames::run_script(lua, frame, "OnTextSet", &[]);
    let _ = super::frames::run_script(lua, frame, "OnTextChanged", &[]);
    Ok(())
}

/// What is in the box.
pub fn text(frame: &mlua::Table) -> String {
    super::regions::text_of(frame).unwrap_or_default()
}

/// Where the caret is, clamped into the text — a body that calls `SetText`
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

/// **What is selected**, as a half-open character range, or nothing.
///
/// Ordered, so a selection dragged backwards reads the same as one dragged
/// forwards — the reference orders it inside `SetSelection` itself, which is
/// the same place it clamps.
pub fn selection(frame: &mlua::Table) -> Option<(usize, usize)> {
    let letters = text(frame).chars().count() as i64;
    let anchor = frame.raw_get::<Option<i64>>(ANCHOR_KEY).ok().flatten()?;
    let anchor = anchor.clamp(0, letters) as usize;
    let caret = caret(frame);
    (anchor != caret).then(|| (anchor.min(caret), anchor.max(caret)))
}

/// The selected text, or `None` when nothing is.
///
/// **A password box answers the empty string rather than its contents**, which
/// is the reference's own refusal and not a policy invented here: a password
/// box puts an empty string on the clipboard instead of the buffer. It still *has* a selection, so
/// the cut half of Ctrl-X still deletes.
fn selected_text(frame: &mlua::Table) -> Option<String> {
    let (start, end) = selection(frame)?;
    if is_password(frame) {
        return Some(String::new());
    }
    Some(text(frame).chars().skip(start).take(end - start).collect())
}

/// Is this box drawn as dots and copied as nothing? See [`selected_text`].
pub fn is_password(frame: &mlua::Table) -> bool {
    frame.raw_get::<Option<bool>>(PASSWORD_KEY).ok().flatten().unwrap_or(false)
}

/// **Select `start..end`**, in the reference's own two clamps.
///
/// An `end` below the `start` means *to the end of the text* — the rule that
/// makes `HighlightText(0, -1)` select everything, which is what Ctrl-A sends.
/// An empty range clears the selection outright.
fn select(frame: &mlua::Table, start: i64, end: i64) -> mlua::Result<()> {
    let letters = text(frame).chars().count() as i64;
    let start = start.clamp(0, letters);
    let end = if end < start { letters } else { end.min(letters) };
    if start == end {
        return clear_selection(frame);
    }
    frame.set(ANCHOR_KEY, start)?;
    frame.set(CARET_KEY, end)
}

/// Nothing is selected any more. The caret stays where it is.
fn clear_selection(frame: &mlua::Table) -> mlua::Result<()> {
    frame.set(ANCHOR_KEY, mlua::Value::Nil)
}

/// Put the caret somewhere, extending the selection or dropping it.
///
/// The one door for all six moving keys, because "Shift decides" is a property
/// of the *move* in the reference rather than of any one key — see the module
/// comment.
fn move_caret(frame: &mlua::Table, to: i64, extend: bool) -> mlua::Result<()> {
    let letters = text(frame).chars().count() as i64;
    let to = to.clamp(0, letters);
    if !extend {
        clear_selection(frame)?;
    } else if frame.raw_get::<Option<i64>>(ANCHOR_KEY)?.is_none() {
        // The first Shift-move drops the anchor where the caret was standing.
        frame.set(ANCHOR_KEY, caret(frame) as i64)?;
    }
    frame.set(CARET_KEY, to)
}

/// The string with `start..end` taken out of it.
fn without(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .enumerate()
        .filter(|(index, _)| *index < start || *index >= end)
        .map(|(_, c)| c)
        .collect()
}

/// `SetTextInsets(left, right, top, bottom)`, or — **failing that, the box's own
/// backdrop insets**.
///
/// The fallback is the half that matters on screen, and it is a statement the
/// file makes rather than a margin chosen here. `AccountLoginAccountEdit` calls
/// `SetTextInsets` nowhere, and its `<Backdrop>` says
///
/// ```xml
/// <BackgroundInsets><AbsInset left="10" right="5" top="4" bottom="9"/></BackgroundInsets>
/// <EdgeSize><AbsValue val="16"/></EdgeSize>
/// ```
///
/// — which is the *interior of the plate*, the only thing in the markup that
/// says where the inside of this box is. With zero insets the typed line starts
/// at the frame's own left edge, which is **under the sixteen-unit border
/// piece**: the account name came out with its first letter cut in half and the
/// password caret drawn entirely outside the box.
///
/// `ChatFrameEditBox` is unaffected either way — `ChatEdit_UpdateHeader` calls
/// `SetTextInsets(15 + header:GetWidth(), 13, 0, 0)` on every open, so the
/// explicit value wins there and this fallback is never reached.
///
/// **An inference from the markup rather than a reading of the client**, and
/// marked as one: what 1.12's C side defaults an unstated text inset to is not
/// established here. What is established is that zero is wrong, because zero
/// draws the text under art the same file positions.
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

/// **Everything one keystroke can be**, once the modifiers and the layout have
/// been applied — see [`super::super::api::keyboard`], which is the only producer.
///
/// A character rather than a key code, because the whole point of the text
/// field is that it takes what the *layout* produced: a French keyboard's `A`
/// arrives here as `"q"` on a US mapping and this file must not care.
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
    /// The three the game gives a script of its own, rather than an edit.
    Enter,
    Escape,
    Tab,
    /// History, oldest and newest — the two arrows a box that does not ignore
    /// them walks its `AddHistoryLine` list with.
    Up(Mods),
    Down(Mods),
    /// Ctrl-A — `HighlightText(0, -1)`, which is what the client sends.
    SelectAll,
    /// Ctrl-C and Ctrl-Insert. The text to put on the clipboard comes back out
    /// of [`dispatch`]; this file never touches the OS.
    Copy,
    /// Ctrl-X and Shift-Delete: [`Stroke::Copy`] and then the deletion.
    Cut,
    /// Ctrl-V and Shift-Insert, with the clipboard **already read** — see
    /// [`super::super::api::keyboard`], which is where the window system lives.
    Paste(String),
}

/// **The modifiers an edit box reads, and it reads exactly these two.**
///
/// Shift is the extend flag every move takes (modifier 0 in the reference's own
/// numbering, established by the Ctrl chords beside it reading modifier 1).
/// Alt is the one that lets an arrow past `ignoreArrows` — the client gates
/// the four arrow keys on modifier 2, and
/// **that identification is an inference**: 0 and 1 are pinned by what they
/// guard, 2 is the remaining modifier and the behaviour it produces is the one
/// later clients call `SetAltArrowKeyMode`.
///
/// Ctrl is deliberately not here: a Ctrl chord arrives as its own variant
/// already decided, because which chord a key is depends on the *layout* and
/// that is [`super::super::api::keyboard`]'s business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    /// Extend the selection rather than dropping it.
    pub shift: bool,
    /// Reach a box that declares `ignoreArrows` anyway.
    pub alt: bool,
}

/// **Apply a run of keystrokes to whichever box has the focus**, and say what
/// broke.
///
/// Nothing at all when nothing is focused, which is the ordinary case and the
/// one the caller checks first — the whole world's keyboard passes through here
/// otherwise.
/// The second half of the answer is **what to put on the clipboard**, if a
/// stroke asked: this file decides *what* is copied and [`super::super::api::keyboard`] does
/// the copying, on the same split as every other piece of the outside world in
/// this directory. `None` is "leave the clipboard alone", which is what an empty
/// selection means.
pub(in crate::lua) fn dispatch(
    lua: &mlua::Lua,
    strokes: &[Stroke],
) -> mlua::Result<(Vec<String>, Option<String>)> {
    let mut errors = Vec::new();
    let mut copied = None;
    for stroke in strokes {
        // **Re-read per stroke.** `OnEnterPressed` hides the box and drops the
        // focus, so the keystroke after it in the same frame belongs to nobody.
        let Some(frame) = focused(lua) else { break };
        match apply(lua, &frame, stroke) {
            // The last copy in a frame wins, which is every real frame's only.
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
        // **The space bar is a character *and* a script.** `ChatEdit_OnSpacePressed`
        // re-parses the line, which is what turns `/s ` into a `SAY` header the
        // moment the command is finished rather than when it is sent.
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
        Stroke::Left(mods) => nothing(step(frame, -1, *mods)),
        Stroke::Right(mods) => nothing(step(frame, 1, *mods)),
        // **Home and End are never gated by `ignoreArrows`** — the reference's
        // gate names four key codes and these are not among them,
        // so the chat line, which declares the attribute, can still be selected
        // end to end. That is what makes Shift-Home the usable gesture there.
        Stroke::Home(mods) => nothing(move_caret(frame, 0, mods.shift)),
        Stroke::End(mods) => {
            let letters = text(frame).chars().count() as i64;
            nothing(move_caret(frame, letters, mods.shift))
        }
        Stroke::Up(mods) => nothing(history(lua, frame, -1, *mods)),
        Stroke::Down(mods) => nothing(history(lua, frame, 1, *mods)),
        Stroke::SelectAll => nothing(select(frame, 0, -1)),
        // **Nothing selected is nothing copied**, and the clipboard is left
        // holding whatever it held — three sites in the reference return before
        // it is even opened.
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
        // **These three do nothing themselves.** The whole of what Enter means
        // is `ChatEdit_OnEnterPressed`, in the archive — send, remember the
        // sticky type, hide. A client that also cleared the box here would be
        // guessing at a body it is running two lines later.
        Stroke::Enter => nothing(super::frames::run_script(lua, frame, "OnEnterPressed", &[])),
        Stroke::Escape => nothing(super::frames::run_script(lua, frame, "OnEscapePressed", &[])),
        Stroke::Tab => nothing(super::frames::run_script(lua, frame, "OnTabPressed", &[])),
    }
}

/// Put text in at the caret, respecting the letter cap.
///
/// **A selection is deleted first** — the reference does it before the cap
/// and before the numeric filter, and it is what makes both a typed character
/// and a paste *replace* what is highlighted.
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
    // **A full box drops what does not fit silently**, which is what the real
    // one does — there is no beep and no message, and `letters="255"` on the
    // chat line is the server's own `CMSG_MESSAGECHAT` limit seen from this
    // side. For one keystroke that is the whole keystroke; for a paste it is
    // the tail, because the reference pastes a character at a time through this
    // same path and each one meets the cap on its own.
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

/// Backspace (`-1`) or Delete (`+1`) — **or the selection, if there is one**,
/// in which case the direction does not matter and neither key takes a
/// character beyond it.
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
fn step(frame: &mlua::Table, direction: i64, mods: Mods) -> mlua::Result<()> {
    if ignores_arrows(frame, mods) {
        return Ok(());
    }
    move_caret(frame, caret(frame) as i64 + direction, mods.shift)
}

/// **`ignoreArrows` is a gate with a key in it.** The reference's flag sends
/// the four arrow codes back unhandled — which is how the chat line lets you
/// turn your character while typing — *unless* modifier 2 is held, in which
/// case the box takes them after all. See [`Mods`] on how firmly that
/// modifier is identified.
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

/// **The one place the text changes**, so `OnTextChanged` cannot be forgotten
/// by one of the five edits above.
///
/// `SetText` is deliberately *not* used: that fires `OnTextSet` too, and 1.12
/// raises that one only when something *set* the text rather than when it was
/// typed into — `ChatEdit_OnTextSet` re-parses the line, so a client that fired
/// it per keystroke would eat the `/` of a command as it was being typed.
fn write(lua: &mlua::Lua, frame: &mlua::Table, next: &str, caret: usize) -> mlua::Result<()> {
    super::regions::set_text_value(lua, frame, mlua::Value::String(lua.create_string(next)?))?;
    frame.set(CARET_KEY, caret as i64)?;
    // **An edit ends the selection**, every time and whichever path made it —
    // the range it named does not survive the string it named it in.
    clear_selection(frame)?;
    let _ = super::frames::run_script(lua, frame, "OnTextChanged", &[]);
    Ok(())
}

fn first_line(e: &mlua::Error) -> String {
    let text = e.to_string();
    text.lines().next().unwrap_or_default().to_string()
}

/// Install [`METHODS`] onto the shared frame method table — see
/// [`super::frames::register_methods`], and [`super::button`] for why one table
/// serves every kind.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set_focus = lua.create_function(|lua, this: mlua::Table| take_focus(lua, &this))?;
    methods.set("SetFocus", set_focus)?;
    let clear_focus = lua.create_function(|lua, this: mlua::Table| release_focus(lua, &this))?;
    methods.set("ClearFocus", clear_focus)?;
    let has_focus = lua.create_function(|lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(focused(lua) == Some(this)))
    })?;
    methods.set("HasFocus", has_focus)?;

    // `editBox:Insert(text)` — 1.12 pastes an item link into the chat line with
    // it, and `ChatEdit_ParseText` never sees the difference from typing.
    let insert_method =
        lua.create_function(|lua, (this, text): (mlua::Table, Option<String>)| {
            insert(lua, &this, &text.unwrap_or_default())
        })?;
    methods.set("Insert", insert_method)?;

    // **`HighlightText([start, end])` — and both call shapes are in the shipped
    // directory**, on the same two lines of `InputBoxTemplate`:
    //
    // ```xml
    // <OnEditFocusGained>this:HighlightText();</OnEditFocusGained>
    // <OnEditFocusLost>this:HighlightText(0, 0);</OnEditFocusLost>
    // ```
    //
    // So a click into a money field or a stack-split box selects everything it
    // holds and the first digit typed replaces it — which is the visible half
    // of this whole subject, and which did nothing at all while this was a
    // stub. The bare call is `(0, -1)`, because that is what Ctrl-A sends
    // and what [`select`] reads as "to the end".
    let highlight = lua.create_function(
        |_lua, (this, start, end): (mlua::Table, Option<i64>, Option<i64>)| {
            select(&this, start.unwrap_or(0), end.unwrap_or(-1))
        },
    )?;
    methods.set("HighlightText", highlight)?;

    // `SetTextInsets(15 + header:GetWidth(), 13, 0, 0)` — a nil argument is a
    // zero rather than a raise, for the reason [`super::widget`]'s `SetID`
    // gives: the raise costs the rest of `ChatEdit_UpdateHeader`.
    let set_insets = lua.create_function(
        |lua, (this, l, r, t, b): (mlua::Table, Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
            let table = lua.create_table()?;
            for (name, value) in ["left", "right", "top", "bottom"].into_iter().zip([l, r, t, b]) {
                table.set(name, value.unwrap_or(0.0))?;
            }
            this.set(INSETS_KEY, table)
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
    // `SetAltArrowKeyMode` — whether the arrow keys walk the history only
    // with Alt held. Recorded; this client's history walk is `AddHistoryLine`'s
    // and reads no flag. Every chat-frame addon sets it on the edit box.
    let set_alt_arrow = lua.create_function(|_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        this.set(
            "__altArrowKeyMode",
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("SetAltArrowKeyMode", set_alt_arrow)?;
    // `SetMultiLine` — recorded; the box draws one line whatever it is told,
    // which the module note states under what is not modelled.
    let set_multi_line = lua.create_function(|_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        this.set(
            "__multiLine",
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("SetMultiLine", set_multi_line)?;

    // **`SetJustifyH` on a frame is its own string's**, as `SetTextColor` is:
    // a button's label, or the box's typed text, which this client draws
    // left-aligned whatever it is told.
    for (name, key) in [("SetJustifyH", "justifyH"), ("SetJustifyV", "justifyV")] {
        let set = lua.create_function(move |_lua, (this, how): (mlua::Table, Option<String>)| {
            let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
            super::regions::set_justify(&target, key, how.as_deref().unwrap_or("CENTER"))
        })?;
        methods.set(name, set)?;
    }

    // **`SetTextColor` is two different things and both are real.** On an edit
    // box it is the typed text's colour, which is what tints the chat line by
    // the type being spoken; on anything else it is the frame's own font
    // string's, which is how a button greys its label. It was a stub for both.
    let set_colour = lua.create_function(
        |_lua, (this, r, g, b, a): (mlua::Table, Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
            let rgba = vec![
                r.unwrap_or(1.0),
                g.unwrap_or(1.0),
                b.unwrap_or(1.0),
                a.unwrap_or(1.0),
            ];
            // **Sticky on the way through**, which is the half that was
            // missing: a forwarded colour that does not mark the string as
            // carrying its own is erased by the next face the button wears.
            // See [`super::regions::set_text_colour`].
            match super::regions::text_region(&this) {
                Some(region) => super::regions::set_text_colour(
                    &region,
                    [rgba[0], rgba[1], rgba[2], rgba[3]],
                ),
                None => this.set(COLOUR_KEY, rgba),
            }
        },
    )?;
    methods.set("SetTextColor", set_colour)?;

    // **`SetFont` and `SetFontObject` on a frame are the same forward.** A
    // button's, a message frame's or an edit box's face is its own string's
    // when it has one, and the frame's own keys otherwise, which is where the
    // edit box and the message frame keep theirs.
    let set_font = lua.create_function(
        |_lua, (this, path, height, flags): (mlua::Table, Option<String>, Option<f64>, Option<String>)| {
            let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
            super::regions::set_font_triplet(&target, path.as_deref(), height, flags.as_deref())
        },
    )?;
    methods.set("SetFont", set_font)?;
    let set_font_object = lua.create_function(|lua, (this, value): (mlua::Table, mlua::Value)| {
        let Some(font) = super::regions::resolve_font_object(lua, value)? else {
            return Ok(());
        };
        let target = super::regions::text_region(&this).unwrap_or_else(|| this.clone());
        super::regions::apply_font_style(&target, &font)?;
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

    // `SetNumber`/`GetNumber` are the numeric face of the same string — the
    // stack-split and money entry boxes are written entirely against them.
    let set_number = lua.create_function(|lua, (this, value): (mlua::Table, Option<f64>)| {
        let value = value.unwrap_or(0.0);
        // Integers print without a decimal point, which is what a stack size
        // has to look like; 1.12 has one number type and formats the same way.
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

    // **The keyboard's own language, not the spoken one.** `INPUT_ROMAN = "A"`
    // is the label `ChatEdit_OnInputLanguageChanged` puts on the button beside
    // the line; the other three are the CJK input methods 1.12 shipped for.
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

    /// Apply strokes and insist none of them raised; the clipboard half is
    /// [`copied`]'s.
    fn press(lua: &mlua::Lua, strokes: &[Stroke]) -> Option<String> {
        let (errors, copied) = dispatch(lua, strokes).expect("the strokes apply");
        assert!(errors.is_empty(), "{errors:?}");
        copied
    }

    /// Shift held, which is the only modifier most of these need.
    const SHIFT: Mods = Mods { shift: true, alt: false };
    const PLAIN: Mods = Mods { shift: false, alt: false };

    /// **A key reaches the box that has the focus and nothing else does.** The
    /// property the whole file exists for: without a focus there is no typing,
    /// which is what stops WASD becoming text.
    #[test]
    fn only_a_focused_box_takes_the_keyboard() {
        let lua = boxed("");
        type_in(&lua, "hello");
        // **`""` and not nil** — an edit box holds an empty buffer where a font
        // string holds no pointer, which is 1.12's own distinction and is what
        // `MoneyInputFrame_GetCopper` is written against. See
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
        // …and backspace takes the character *before* it, delete the one after.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Delete]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ello")"#);
        press(&lua, &[Stroke::End(PLAIN), Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ell")"#);
        // …and neither runs off the end of the string.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Backspace, Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ell")"#);
    }

    /// **`ignoreArrows` is the chat line's own attribute**, and it is what stops
    /// the caret moving there — so that the arrows reach the game and turn the
    /// character instead.
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

        // **…and Alt is the key that gets past it**: the same two
        // arrows with Alt held move the caret after all.
        let alt = Mods { shift: false, alt: true };
        press(&lua, &[Stroke::Left(alt), Stroke::Left(alt), Stroke::Char("Y".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abYcX")"#);

        // **…and Home and End are not gated at all**, which is what leaves the
        // chat line selectable end to end. Four key codes are gated and these
        // are not among them.
        press(&lua, &[Stroke::Home(PLAIN), Stroke::Char("Z".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("ZabYcX")"#);
    }

    /// **A selection is two indices and Shift is what grows it**, which is the
    /// property every other test in this group rests on.
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
        // …and a plain move drops it rather than shrinking it.
        press(&lua, &[Stroke::End(PLAIN)]);
        assert_eq!(selection(&edit(&lua)), None);
        // …and one Shift-Left is one character, from wherever the caret stood.
        press(&lua, &[Stroke::Left(SHIFT)]);
        assert_eq!(selection(&edit(&lua)), Some((4, 5)));
        // …and shrinking back to the anchor is no selection at all, which is
        // the reference's own `start == end`.
        press(&lua, &[Stroke::Right(SHIFT)]);
        assert_eq!(selection(&edit(&lua)), None);
    }

    /// **Typing over a selection replaces it** — among the first things the
    /// insert path does — and so does a paste, and so does one Backspace.
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

        // …and a paste replaces one too, with no regard to which way it was
        // gathered.
        press(&lua, &[Stroke::SelectAll, Stroke::Paste("goodbye".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("goodbye")"#);

        // …and Backspace with a selection takes the selection and **not** a
        // character beyond it.
        press(&lua, &[Stroke::SelectAll, Stroke::Backspace]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("")"#);
    }

    /// **Ctrl-C, Ctrl-X and Ctrl-A**, and the rule that an empty selection
    /// copies *nothing* rather than the line.
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

        // …and a cut is the copy *and* the deletion.
        press(&lua, &[Stroke::Home(PLAIN)]);
        for _ in 0..2 {
            press(&lua, &[Stroke::Right(SHIFT)]);
        }
        assert_eq!(press(&lua, &[Stroke::Cut]), Some("he".to_string()));
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("llo")"#);
    }

    /// **`HighlightText` is the directory's own two calls**, and the bare one
    /// selects everything — `InputBoxTemplate` makes both on focus.
    #[test]
    fn highlight_text_takes_the_directorys_two_shapes() {
        let lua = boxed("");
        lua.load("edit:SetFocus()").exec().expect("runs");
        type_in(&lua, "1234");

        lua.load("edit:HighlightText()").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), Some((0, 4)));
        lua.load("edit:HighlightText(0, 0)").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), None);
        // …and a range, clamped to the text rather than raising.
        lua.load("edit:HighlightText(1, 99)").exec().expect("runs");
        assert_eq!(selection(&edit(&lua)), Some((1, 4)));
    }

    /// **A password box copies the empty string**, which is the reference's own
    /// refusal and not a policy invented here. The cut still
    /// deletes: only the clipboard is denied.
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

    /// **The letter cap truncates a paste rather than refusing it**, because
    /// the reference pastes one character at a time through the insert path
    /// and each one meets the cap on its own.
    #[test]
    fn the_cap_takes_as_much_of_a_paste_as_fits() {
        let lua = boxed("");
        set_from_markup(&edit(&lua), "letters", "5").expect("the attribute applies");
        lua.load("edit:SetFocus()").exec().expect("runs");
        press(&lua, &[Stroke::Paste("abcdefgh".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("abcde")"#);
        // …and the replaced selection frees its own room first, which is what
        // makes a select-all-then-paste work in a full box.
        press(&lua, &[Stroke::SelectAll, Stroke::Paste("xyz".to_string())]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("xyz")"#);
    }

    /// **`SetText` fires `OnTextSet` and typing does not**, which is the whole
    /// of how `ChatEdit_ParseText` gets to cut `/s ` off a line without eating
    /// the slash as it is typed.
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
        // …and the caret went to the end of what was set, so the typing appended.
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("hi!")"#);
    }

    /// Enter and Escape are **scripts, not edits** — everything they mean is in
    /// the archive's own `ChatEdit_*` bodies.
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

    /// **Hiding drops the focus.** `ChatEdit_OnEscapePressed` ends in `Hide()`
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
        // …"first" fell off the end of a two-line history.
        press(&lua, &[Stroke::Up(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("second")"#);
        press(&lua, &[Stroke::Down(PLAIN), Stroke::Down(PLAIN)]);
        assert_eq!(eval(&lua, "return edit:GetText()"), r#"String("")"#);
    }

    /// Every name [`METHODS`] claims is installed, and the list is sorted — the
    /// rule every claimed list in this directory follows, because
    /// `vale framexml` counts the interface gap against them.
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

    /// **A second `SetFocus` moves the focus rather than adding one**, and both
    /// boxes hear about it.
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
