//! **The two things that draw**: a `Texture` and a `FontString`.
//!
//! A frame is a container — it holds events, children and a rectangle, and it
//! paints nothing. Everything visible in the 1.12 interface is one of these two,
//! and there are 1,652 of them declared in the ninety XML files against 1,812
//! frames, so this is not a corner of the object model but half of it.
//!
//! ```xml
//! <Layer level="ARTWORK">
//!     <Texture name="$parentFlash" file="Interface\Buttons\UI-QuickslotRed" hidden="true"/>
//!     <FontString name="$parentHotKey" inherits="NumberFontNormalSmallGray" justifyH="RIGHT"/>
//! </Layer>
//! ```
//!
//! ## A slot region is the same object reached two ways
//!
//! `<NormalTexture>` inside a `<Button>` makes a `Texture` **and** fills the
//! button's normal-texture slot, so `button:GetNormalTexture()` finds it and so
//! does `getglobal(name)`. Twelve of the thirteen texture element names work this
//! way — see [`vale_assets::interface::widgets::Region`], which is where the slot table
//! is, because the loader and `vale framexml` both read it.
//!
//! ## What is recorded, and who reads it
//!
//! **Nothing here draws** — `SetTexture` records a path, `SetText` records a
//! string, `SetVertexColor` records four numbers — but something else now does.
//! [`crate::ui::framexml`] walks the tree, asks [`super::layout`] where each of
//! these lands, and paints it. So this module's job is unchanged and its output
//! finally has a consumer, which is why [`Paint`] exists: one struct holding
//! everything a region contributes to a frame, read once instead of eight table
//! lookups scattered through a draw loop.
//!
//! ## `alphaMode` is a blend mode, and it is the game's own word
//!
//! `<Texture alphaMode="ADD">` on the cast bar's spark and flash: 136 elements
//! in the directory carry one, and the values are `ADD`, `BLEND`, `ALPHAKEY`,
//! `MOD` and `DISABLE` — the same set the M2 blend table uses, which is not a
//! coincidence, since both end up in the same fixed-function ROP. Recorded
//! verbatim rather than mapped to anything here.

use super::widget;

/// The methods a region carries beyond [`super::widget::METHODS`], sorted.
///
/// Split in two the way the objects are: a `Texture` answers about pixels and a
/// `FontString` about text, and a method on the wrong one is a nil call. They
/// are one table today because a region's kind is a field rather than a type,
/// and the day that matters the split is this list cut in half.
///
/// **The four measurements are on the frame table too** — see
/// [`install_measures`]. A `Button` answers `GetTextWidth` about its own
/// `<ButtonText>`, and the six tabs that size themselves to their label are
/// frames, not regions.
pub const METHODS: [&str; 28] = [
    "GetBlendMode",
    "GetDrawLayer",
    "GetFontObject",
    "GetObjectType",
    "GetStringHeight",
    "GetStringWidth",
    "GetText",
    "GetTextColor",
    "GetTextHeight",
    "GetTextWidth",
    "GetTexture",
    "GetVertexColor",
    "IsObjectType",
    "SetBlendMode",
    "SetDrawLayer",
    "SetFont",
    "SetFontObject",
    "SetGradientAlpha",
    "SetJustifyH",
    "SetJustifyV",
    "SetShadowColor",
    "SetShadowOffset",
    "SetTexCoord",
    "SetText",
    "SetTextColor",
    "SetTextHeight",
    "SetTexture",
    "SetVertexColor",
];

/// **The font object a string was last given**, by `SetFontObject`, so
/// `GetFontObject` answers it back. The face itself is copied onto the string's
/// own keys at the call — see [`apply_font_style`].
const FONT_OBJECT_KEY: &str = "__fontObject";
/// A font object's own name, for `GetName`.
const FONT_NAME_KEY: &str = "__fontName";
/// The registry slot holding the font objects' method table.
const REG_FONT_METHODS: &str = "vale.fontMethods";

const TEXTURE_KEY: &str = "__texture";
/// **`SetPortraitTexture`'s unit token** — the one thing a `Texture` can hold
/// that is not a file.
///
/// Its own key rather than a magic path in [`TEXTURE_KEY`], because the two are
/// answered by different machinery a directory apart: a path is decoded by
/// `ui::framexml`'s BLP cache and this is a picture `render::portraits` takes
/// with a camera. Storing a sentinel string in the texture key would have made
/// every reader of that key — the loader, `GetTexture`, the art cache — have to
/// know about it.
const PORTRAIT_KEY: &str = "__portraitUnit";

/// …and how far this texture has been **turned**, in radians clockwise on the
/// screen. See [`set_rotation`], which is its only writer.
const ROTATION_KEY: &str = "__rotation";
const TEXT_KEY: &str = "__text";
/// **A frame's own font string, which is a different thing from its text.**
///
/// `<ButtonText>` is a slot, so the loader stores the region it made on the
/// parent — and it used to store it under [`TEXT_KEY`], where a *region* keeps
/// its string. One key, two meanings, decided by which kind of object you
/// happened to be holding: `GetTextWidth` on a button read the font string and
/// tried to measure a table (60 `OnLoad`s, every `MoneyFrame` in the game),
/// `PanelTemplates_SetDisabledTabState`'s `tab:GetText()` read a *string* where
/// the forwarding getter wanted the region, and a `<Button text="CANCEL">` whose
/// template also gave it a `<ButtonText>` had its label silently overwritten by
/// the region that arrived after it.
///
/// Two keys, because they are two things. [`slot_key`] is what puts the region
/// here rather than under the string's key.
pub(in crate::lua) const TEXT_REGION_KEY: &str = "__textRegion";
const LAYER_KEY: &str = "__layer";
const BLEND_KEY: &str = "__blend";
const COLOUR_KEY: &str = "__colour";
/// **Whether a script has set this string's colour itself** — 1.12's own
/// flag, set by `SetTextColor` and read by nothing that puts a
/// font *object* on. See [`apply_font_style`], which is the whole of why it
/// exists.
const COLOUR_SET_KEY: &str = "__colourSet";
const JUSTIFY_H_KEY: &str = "__justifyH";
const JUSTIFY_V_KEY: &str = "__justifyV";
/// `<Shadow>` — see [`set_shadow`]. `MasterFont`'s, so nearly every string in
/// the interface carries one.
const SHADOW_OFFSET_KEY: &str = "__shadowOffset";
const SHADOW_COLOUR_KEY: &str = "__shadowColour";
/// `outline="NORMAL"` / `"THICK"` — see [`Paint::outline`], which is where the
/// measurement is. An *attribute* on the `<Font>` rather than a child element,
/// which is why it arrives beside `font` rather than beside `<Shadow>`.
const OUTLINE_KEY: &str = "__outline";
/// A font string whose text folds at its rectangle's width instead of running
/// past it. Written today only by the tooltip's wrap arguments — see
/// [`super::tooltip`] — and read by the painter, which wraps the glyphs for
/// real where this side only estimated the fold.
const WRAP_KEY: &str = "__wrap";
const COORDS_KEY: &str = "__texCoords";
/// The typeface and size a font string draws in — `font="Fonts\FRIZQT__.TTF"`
/// and `<FontHeight><AbsValue val="12"/>`, which reach a `<FontString>` through
/// `inherits="GameFontNormal"` rather than being written on it.
const FONT_KEY: &str = "__font";
const FONT_HEIGHT_KEY: &str = "__fontHeight";

/// **Everything a `<Font>` object writes onto a string**, as a set — so that a
/// face can be lifted off one region and put on another whole.
///
/// This exists for [`super::button`]'s three of them: a button keeps *three*
/// font objects and picks between them per state, where this client keeps a region's face on the region. The
/// difference is bridged by snapshotting each declaration and re-applying the
/// one the state chooses.
///
/// **`justifyH`/`justifyV` are deliberately not in it.** They are declared on
/// the `<ButtonText>` far more often than on the `<Font>` — `$parentName` in
/// `CharSelectCharacterButtonTemplate` is `justifyH="LEFT"` with no font of its
/// own — so carrying them through a state change would re-centre a label the
/// markup left-aligned.
const FONT_STYLE_KEYS: [&str; 6] = [
    FONT_KEY,
    FONT_HEIGHT_KEY,
    COLOUR_KEY,
    SHADOW_OFFSET_KEY,
    SHADOW_COLOUR_KEY,
    OUTLINE_KEY,
];

/// Lift the face off a region — see [`FONT_STYLE_KEYS`].
pub(in crate::lua) fn capture_font_style(
    lua: &mlua::Lua,
    region: &mlua::Table,
) -> mlua::Result<mlua::Table> {
    let style = lua.create_table()?;
    for key in FONT_STYLE_KEYS {
        style.set(key, region.raw_get::<mlua::Value>(key)?)?;
    }
    Ok(style)
}

/// …and put one on. A key the snapshot does not carry is **cleared**, which is
/// what makes the three states independent: `GameFontDisable` declares only a
/// `<Color>` and inherits the rest, so a partial write would leave the previous
/// state's colour behind on a font that never mentions one.
///
/// **Except the colour, once a script has set one.** In 1.12 a font string's
/// colour and its font *object* are two different fields with two different
/// setters, and `SetTextColor` marks the string as carrying its own: it
/// stores the colour and sets a flag saying it is explicit, while
/// `SetFontObject` writes a different field entirely, so putting a face on
/// cannot reach the colour.
///
/// This client keeps one set of keys for both, so without [`COLOUR_SET_KEY`] a
/// button wearing its `<HighlightFont>` on hover would erase the colour the
/// interface had just written on its label. That is not hypothetical and it is
/// not rare: `QuestLogTitleButtonTemplate` declares all three faces *and*
/// `QuestLog_Update` calls `SetTextColor` on the button itself for the quest's
/// difficulty — so every row in the log flipped between its difficulty colour
/// and `GameFontNormal`'s gold as the pointer crossed it, which is what a
/// screenshot reported as the log "flashing different colours".
pub(in crate::lua) fn apply_font_style(region: &mlua::Table, style: &mlua::Table) -> mlua::Result<()> {
    let keeps_its_colour = region
        .raw_get::<Option<bool>>(COLOUR_SET_KEY)
        .unwrap_or(None)
        .unwrap_or(false);
    for key in FONT_STYLE_KEYS {
        if keeps_its_colour && key == COLOUR_KEY {
            continue;
        }
        region.set(key, style.raw_get::<mlua::Value>(key)?)?;
    }
    Ok(())
}

/// **How wide the text `style` is set in would draw `text`**, in the game's
/// own units.
///
/// One door, so that `GetTextWidth`, the tooltip's auto-size, the caret and a
/// message frame's fold cannot disagree about where a word ends — and behind it
/// the game's own `.TTF`s rather than a ratio; see [`super::text::width`],
/// which is where the measurement and its one stated fallback are.
///
/// `style` is whatever carries the face and the height: a `FontString` for its
/// own string, a frame's own font string for the lines it lays out itself.
pub(super) fn text_width(lua: &mlua::Lua, style: &mlua::Table, text: &str) -> f64 {
    let font: Option<String> = style.raw_get(FONT_KEY).ok().flatten();
    let height = style
        .raw_get::<Option<f64>>(FONT_HEIGHT_KEY)
        .ok()
        .flatten()
        .filter(|height| *height > 0.0)
        .unwrap_or(f64::from(vale_assets::interface::font::DEFAULT_HEIGHT));
    super::text::width(lua, font.as_deref(), height, text)
}

/// **How many rows `text` folds into at `fold`, in this region's own face.**
///
/// The row half of [`text_width`], and one door for the same reason: the height
/// a caller *reserves* and the galley the painter *draws* must fold at the same
/// places, in the same face, at the same size. [`super::text::rows`] is the
/// measurement — word by word, with a first word too long for the row broken
/// rather than pushed — and it is what the painter itself folds by.
///
/// It exists because the tooltip used to estimate instead: `ceil(measured /
/// fold)`, which is the row count only for a sentence whose words happen to
/// land exactly on the fold. Real prose wastes a fraction of a row at every
/// break, so the estimate is one short as soon as the sentence is three rows
/// long — measured on Shield Bash's own description, which reserved three rows
/// and drew four, putting its last line under the bottom of the plate.
pub(super) fn text_rows(lua: &mlua::Lua, style: &mlua::Table, text: &str, fold: f64) -> usize {
    super::text::rows(
        lua,
        font_of(style).as_deref(),
        font_height_of(style),
        text,
        fold,
    )
}

/// **How tall the string is once it has folded** — the answer `GetTextHeight`
/// and `GetStringHeight` give.
///
/// One line box for a string that does not fold, and one per row for one that
/// does, at the same fold width and in the same face [`Paint::wrap`] tells the
/// painter to use — so a caller that sizes a widget from this reserves the rows
/// that are actually drawn.
///
/// **This used to answer one line box always**, which is what made the gossip
/// menu unreadable. `GossipFrame.lua`'s `GossipResize` is
/// `titleButton:SetHeight(titleButton:GetTextHeight() + 2)`, and each option
/// button is anchored `TOPLEFT` to the previous one's `BOTTOMLEFT` — so a
/// button's height *is* the next button's offset. `GossipTitleButtonTemplate`
/// declares its `<ButtonText>` 275 wide, so a long option really does fold to
/// two rows; the button was sized for one, and the second row of each option
/// was painted under the first row of the next.
///
/// Empty text is one line box rather than nothing, because that is what a
/// caller adding padding to it expects and what this answered before.
pub(super) fn text_height(lua: &mlua::Lua, style: &mlua::Table, text: &str) -> f64 {
    let line = line_height_of(lua, style);
    let Some(fold) = wrap_width(style, true) else {
        return line;
    };
    if text.is_empty() {
        return line;
    }
    let cap = style
        .raw_get::<Option<usize>>(super::messages::MAX_LINES_KEY)
        .ok()
        .flatten()
        .unwrap_or(usize::MAX)
        .max(1);
    text_rows(lua, style, text, fold).min(cap).max(1) as f64 * line
}

/// **How big a `FontString` that was never given a size is** — the size of the
/// string it holds, in its own face.
///
/// `None` for anything that is not a font string, and for one holding no text.
///
/// This is the rule that decides whether most of the interface's words appear at
/// all. A `FontString` is the one region kind the directory routinely declares
/// with **one anchor and no `<Size>`**: `CharacterStatFrame1Label` is
/// `<FontString inherits="GameFontNormalSmall"><Anchors><Anchor point="LEFT"/>`
/// and nothing else, because in the real client a string's own extent *is* its
/// size — `GetWidth` on one answers `GetStringWidth`. Without the rule the two
/// axes come out of `SetWidth`/`SetHeight`, which nothing wrote, so the rectangle
/// is 0x0 and [`super::draw`] drops it before it is ever painted: measured at
/// **38 of the 41 strings holding text** on a screen with the character sheet
/// open — every stat label and every stat value, the level line, the tab
/// captions. It reads as "the panel has no content", which is exactly how it was
/// reported.
///
/// **Per axis, and only where the object said nothing**, because the two are
/// declared independently — `<AbsDimension x="120" y="0"/>` is a string with a
/// width and an intrinsic height.
///
/// **And a string with a width of its own is as tall as it folds**, which is
/// the other half of the same rule.
///
/// `<FontString name="$parentSpellName" maxLines="3">` with
/// `<AbsDimension x="103" y="0"/>` is the spellbook's own declaration: a
/// **width and no height**, which in the real client means "wrap at 103 and be
/// as tall as that takes, up to three rows". Without the fold, "Rallying Cry of
/// the Dragonslayer" ran straight out through the right edge of the page and
/// over the tab strip — the exact thing the retail client wraps onto a second
/// line — and the row below it stayed where a one-line name would have put it.
///
/// The count is [`super::text::rows`], measured in the same face at the same
/// height the painter lays out in, and the painter is told to fold at the same
/// rectangle ([`Paint::wrap`]) — so a string that reserves two rows draws in
/// two. `maxLines` caps it where the file states one; where it does not, the
/// string is as tall as it needs, which is the real client's behaviour for the
/// wrapping labels that declare no cap.
pub(super) fn intrinsic(lua: &mlua::Lua, object: &mlua::Table) -> Option<(f64, f64)> {
    // The cheap gate first: this runs under the layout solve, which is asked
    // about every object in the tree.
    if !is_kind(object, FONT_STRING) {
        return None;
    }
    let text: String = object.raw_get::<Option<String>>(TEXT_KEY).ok().flatten()?;
    if text.is_empty() {
        return None;
    }
    let line = line_height_of(lua, object);
    let rows = match wrap_width(object, true) {
        Some(fold) => {
            let cap = object
                .raw_get::<Option<usize>>(super::messages::MAX_LINES_KEY)
                .ok()
                .flatten()
                .unwrap_or(usize::MAX)
                .max(1);
            super::text::rows(lua, font_of(object).as_deref(), font_height_of(object), &text, fold)
                .min(cap)
        }
        None => 1,
    };
    Some((text_width(lua, object, &text), rows as f64 * line))
}

/// **What `GetWidth` and `GetHeight` answer for a `FontString`** — its own
/// string, whenever nothing gave it an explicit one.
///
/// `None` for anything that is not a font string, and for one that *was* given
/// a size — both of which fall through to the solved rectangle, which is what
/// every frame in the tree answers.
///
/// ## Why this is not the rectangle
///
/// A region the files gave no anchors fills its parent (see the loader's own
/// note), which is right for the art and wrong for a label: `<ButtonText>`,
/// `QuestLogDummyText` and every tab's text are declared with no `<Anchors>`,
/// so a `GetWidth` off the rectangle answers **the container's width** for all
/// of them.
///
/// The directory settles it, and it is unanimous. Of the 120 `GetWidth`/
/// `GetHeight` call sites in `Interface\FrameXML\`, every one whose receiver is
/// a `FontString` wants the string:
///
/// ```text
/// QuestLogFrame.lua:163   QuestLogDummyText:SetText(...)   "*SUPER HACK*" — its own comment
/// QuestLogFrame.lua:199   QuestLogDummyText:GetWidth()     …and this is what it is for
/// QuestLogFrame.lua:623   watchText:GetWidth()             sizes QuestWatchFrame
/// UIPanelTemplates.lua:58 tabText:GetWidth() + padding     sizes a tab
/// UIPanelTemplates.lua:71 tabText:SetWidth(0)              …and 0 puts it back
/// HelpFrame.lua:307       this:SetWidth(…Text:GetWidth()+40)   a <ButtonText>
/// GameTooltip.xml:63      SmallTextTooltipText:GetWidth()+20
/// UIDropDownMenu.lua:190  normalText:GetWidth()
/// PlayerFrame.lua:228     PlayerFrameGroupIndicatorText:GetWidth()
/// TutorialFrame.lua:28    TutorialFrameText:GetHeight()
/// ```
///
/// `HelpFrame.lua:307` is the one that settles the `<ButtonText>` case, which is
/// the only case where the two answers could reasonably have differed: it sizes
/// a **button** to its own label, so the rectangle answer is the button's width
/// and the button grows by 40 every time the line runs.
///
/// `UIPanelTemplates.lua:71` settles the other half: **`SetWidth(0)` is "unset",
/// not "zero wide"**, which is already how [`super::layout`]'s solve reads it.
///
/// ## What it does not change
///
/// The rectangle. A `<ButtonText>` still fills its button and is still drawn
/// centred in it — the painter reads [`super::layout::rect`] directly and never
/// this. Only the answer to the *question* moves.
pub(super) fn reported_size(lua: &mlua::Lua, object: &mlua::Table, want_width: bool) -> Option<f64> {
    if !is_kind(object, FONT_STRING) {
        return None;
    }
    let key = if want_width { widget::WIDTH_KEY } else { widget::HEIGHT_KEY };
    // **A declared size wins, and 0 is not one** — which is what
    // `PanelTemplates_TabResize`'s `tabText:SetWidth(0)` means, and it is the
    // same reading [`super::layout`]'s solve already makes of it.
    //
    // Answered here rather than left to the rectangle, so a `FontString` never
    // consults its anchors for this at all: `SetAllPoints` would otherwise beat
    // a declared width, and the tab that had just been capped would read back
    // its container's. **What that gives up** is a font string sized by two
    // opposite anchors, which would report its string rather than the span —
    // nothing in `Interface\FrameXML\` reads one, and every reader that does
    // exist is measuring text.
    if let Some(set) = object.raw_get::<Option<f64>>(key).ok().flatten().filter(|set| *set > 0.0) {
        return Some(set);
    }
    // **An empty string is 0 and not the container**, which is the honest
    // answer and is what an unset `FontString` already reported before this.
    let Some((width, height)) = intrinsic(lua, object) else {
        return Some(0.0);
    };
    Some(if want_width { width } else { height })
}

/// **Where this region's text folds, if it folds at all.**
///
/// A declared width and nothing else. Two things it deliberately is *not*:
///
/// * not the **solved** rectangle, which for a string with no width of its own
///   is the width of the string — every label in the interface would then
///   "fold" at exactly its own length, which is a fold that never fires but
///   costs a measurement on every one of them;
/// * not applied to a `<Texture>`, which has a width for entirely other
///   reasons.
///
/// [`WRAP_KEY`] is the second door, and it is the tooltip's: that widget folds
/// lines whose width it computed itself rather than declared.
/// `is_font` is passed in rather than read, because both callers have already
/// asked what kind of region this is and [`paint`] runs once per drawn object
/// per frame — see [`is_kind`] on why that read is not free.
fn wrap_width(object: &mlua::Table, is_font: bool) -> Option<f64> {
    match object.raw_get::<Option<bool>>(WRAP_KEY).ok().flatten() {
        // The tooltip caps its own plate; the painter folds at the rectangle.
        Some(true) => {
            return object
                .raw_get::<Option<f64>>(widget::WIDTH_KEY)
                .ok()
                .flatten()
                .filter(|w| *w > 0.0);
        }
        // **An explicit `false` is "never fold", not "fall through"**, and the
        // difference is the tooltip's whole layout.
        //
        // `reflow` writes each cell's own *measured* width back as a declared
        // width, so falling through to the rule below folded every tooltip line
        // at exactly the width it had just been measured at — a knife edge that
        // tips whenever the painter and the measurement disagree by a fraction
        // of a pixel, which they do: `Face::width` is a bare sum of advances and
        // the painter adds the face's kerning. A unit's name tipping over put
        // its second row on top of "Level 23 Humanoid", because the plate had
        // already been sized for one row and the line under it is anchored to
        // this line's declared bottom.
        //
        // The tooltip is the only writer of this key, so nothing else changes.
        Some(false) => return None,
        None => {}
    }
    if !is_font {
        return None;
    }
    object
        .raw_get::<Option<f64>>(widget::WIDTH_KEY)
        .ok()
        .flatten()
        .filter(|w| *w > 0.0)
}

/// **…and a string pinned on both sides folds at the width its anchors give
/// it**, which `SetWidth` never told it about.
///
/// [`wrap_width`] answers the *number*, and it can only answer one a region
/// declared. A `<FontString>` anchored `TOPLEFT` and `BOTTOMRIGHT` to its parent
/// has no declared width at all and a perfectly good solved one — and that is
/// how interface code writes a paragraph:
///
/// ```lua
/// f.text:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10)
/// f.text:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -10, 10)
/// ```
///
/// Without this the fold is `INFINITY` and the sentence runs straight out
/// through both walls of the box it was measured into — which is what pfUI's
/// first-run wizard did, and what every "text clipping out of frames" report is.
///
/// The painter already folds at the rectangle it is handed
/// ([`crate::ui::mesh::text`]), so all that is needed here is the *decision*.
/// A `SetAllPoints` counts, being both corners at once.
///
/// It costs one table read and a walk of two or three anchors per drawn font
/// string. The height estimate is deliberately left alone: a string pinned on
/// both sides has its height from its anchors too, so nothing reads the
/// reserved rows for one.
fn pinned_across(object: &mlua::Table) -> bool {
    let Ok(points) = object.raw_get::<mlua::Table>(widget::POINTS_KEY) else {
        return false;
    };
    let (mut left, mut right) = (false, false);
    for entry in points.sequence_values::<mlua::Table>().flatten() {
        if entry.get::<Option<bool>>("all").ok().flatten().unwrap_or(false) {
            return true;
        }
        let Ok(Some(point)) = entry.get::<Option<String>>("point") else {
            continue;
        };
        let point = point.to_ascii_uppercase();
        left |= point.contains("LEFT");
        right |= point.contains("RIGHT");
    }
    left && right
}

/// The face a region draws in, and the size it draws at — the two halves
/// [`text_width`] resolves inline, split out because [`intrinsic`] needs them
/// to ask [`super::text::rows`] the same question in the same face.
fn font_of(style: &mlua::Table) -> Option<String> {
    style.raw_get(FONT_KEY).ok().flatten()
}

fn font_height_of(style: &mlua::Table) -> f64 {
    style
        .raw_get::<Option<f64>>(FONT_HEIGHT_KEY)
        .ok()
        .flatten()
        .filter(|height| *height > 0.0)
        .unwrap_or(f64::from(vale_assets::interface::font::DEFAULT_HEIGHT))
}

/// **How tall one line of it is** — the face's own line box, not the declared
/// height. The two are not the same and neither is always the larger: Friz
/// Quadrata at 12 lays out at 14.6 and Arial Narrow at 14 lays out at 13.8.
pub(super) fn line_height_of(lua: &mlua::Lua, style: &mlua::Table) -> f64 {
    let font: Option<String> = style.raw_get(FONT_KEY).ok().flatten();
    let height = style
        .raw_get::<Option<f64>>(FONT_HEIGHT_KEY)
        .ok()
        .flatten()
        .filter(|height| *height > 0.0)
        .unwrap_or(f64::from(vale_assets::interface::font::DEFAULT_HEIGHT));
    super::text::line_height(lua, font.as_deref(), height)
}

/// **The four questions about how much room a string takes**, installed on
/// *both* method tables — a region asks about its own string and a frame about
/// the one in its `<ButtonText>`, and the same body answers both because
/// [`text_of`] and [`text_region`] already know which shape it is in.
///
/// They were four stubs until the game's own typefaces were measured, and the
/// two names of each are not an alias this client invented: the directory calls
/// `GetTextWidth` on a frame and `GetStringWidth` on a font string, in the same
/// file.
pub(super) fn install_measures(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let width = lua.create_function(|lua, this: mlua::Table| {
        // The style is whatever holds the face: a font string is its own, and a
        // frame's is the region it keeps its label in.
        let held = text_region(&this).unwrap_or_else(|| this.clone());
        let text = text_of(&this).unwrap_or_default();
        Ok(text_width(lua, &held, &text))
    })?;
    methods.set("GetTextWidth", width.clone())?;
    methods.set("GetStringWidth", width)?;
    let height = lua.create_function(|lua, this: mlua::Table| {
        let held = text_region(&this).unwrap_or_else(|| this.clone());
        let text = text_of(&this).unwrap_or_default();
        Ok(text_height(lua, &held, &text))
    })?;
    methods.set("GetTextHeight", height.clone())?;
    methods.set("GetStringHeight", height)?;
    Ok(())
}

/// The five draw layers, back to front — the game's own names, and the innermost
/// key the interface is drawn in.
///
/// `<Layer level="ARTWORK">`. Here rather than in the draw pass because it is
/// what `GetDrawLayer` answers and something has to say what order the answers
/// are in.
pub const LAYERS: [&str; 5] = ["BACKGROUND", "BORDER", "ARTWORK", "OVERLAY", "HIGHLIGHT"];

/// **Everything a region contributes to a frame**, read in one pass.
///
/// A struct rather than eight calls from the draw loop, and the reason is
/// arithmetic: the loop runs over every visible region every frame, and a Lua
/// table lookup from Rust is not free. It is also the list of what this client
/// records about a region, which is worth having in one place — the day
/// something is drawn wrong, this is where to look for whether it was even read.
/// The three values 1.12's `outline=` takes.
///
/// **Two of them and not one**, because the directory uses both and they are
/// visibly different: `GlueFontNormal` and its siblings say `NORMAL`, and the
/// `NumberFont*` family — the damage numbers, an action button's count, the
/// bag totals — say `THICK`, which is what keeps a white number legible over
/// any icon in the game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Outline {
    #[default]
    None,
    Normal,
    Thick,
}

impl Outline {
    /// How far out the black edge is drawn, in the game's own units.
    ///
    /// **One and two, and that pair is an inference from the two names rather
    /// than a reading of the client**: 1.12 rasterises an outlined glyph in
    /// `CGxFont` and what its two widths are in texels is not established here.
    /// What *is* established is that both exist and that neither is zero — the
    /// reference screenshot's glue text has a hard black edge about a pixel
    /// wide at the size `GlueFontNormal` draws at.
    pub fn radius(self) -> f32 {
        match self {
            Outline::None => 0.0,
            Outline::Normal => 1.0,
            Outline::Thick => 2.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paint {
    /// `Interface\Buttons\UI-QuickslotRed`, with no extension, as the files
    /// write it.
    pub texture: Option<String>,
    /// The text of a `FontString`, already stringified the way 1.12 does it.
    pub text: Option<String>,
    /// A tint on a texture, or the colour of a font string. `[1, 1, 1, 1]` when
    /// nothing set one — which for a texture is "as painted".
    pub colour: [f32; 4],
    /// `ADD`, `BLEND`, `ALPHAKEY`, `MOD`, `DISABLE` — one of [`BLENDS`], and a
    /// `&'static str` for the reason [`FONT_STRING`] gives.
    pub blend: &'static str,
    /// `BACKGROUND`..`HIGHLIGHT`, as an index into [`LAYERS`].
    pub layer: usize,
    /// `SetTexCoord(left, right, top, bottom)`, when one was set.
    pub coords: Option<[f32; 4]>,
    /// …and **the eight-argument form**, which is four `(u, v)` corners rather
    /// than a rectangle: `SetTexCoord(ULx, ULy, LLx, LLy, URx, URy, LRx, LRy)`.
    ///
    /// Exactly one thing in either shipped directory writes it and it is
    /// load-bearing there: `DrawRouteLine` — the flight map's own line drawer,
    /// donated to Blizzard by Daniel Stephens and pasted into `TaxiFrame.lua`
    /// unchanged — rotates a horizontal line texture into place *entirely*
    /// through these eight numbers and a bounding box. Drawn as an axis-aligned
    /// quad instead, every flight path on the map is a rectangle.
    pub corners: Option<[f32; 8]>,
    /// The typeface file, and the height in the game's own units.
    pub font: Option<String>,
    pub font_height: f32,
    /// `LEFT` | `CENTER` | `RIGHT`, and `TOP` | `MIDDLE` | `BOTTOM`.
    pub justify_h: &'static str,
    pub justify_v: &'static str,
    /// `<Shadow>`: the offset of the copy drawn under the glyphs, and its
    /// colour. `None` for a face that declares none — which is rarer than it
    /// looks, since `MasterFont` declares one and `GameFontNormal` inherits it.
    pub shadow: Option<([f32; 2], [f32; 4])>,
    /// **`outline="NORMAL"` — the black edge round every glyph**, which is what
    /// makes the game's own words read *bold* rather than thin.
    ///
    /// An attribute on the `<Font>` and not a child element, so it arrives the
    /// way `font=` does: through `inherits`, from a face declared once. It is on
    /// nearly every word the screens before the world draw —
    /// `GlueFontNormal`, `…Small`, `…Large` and `…Huge` all declare it, and the
    /// account and password labels, the Login button, the three link buttons and
    /// the version line all inherit one of the four — and on the number fonts in
    /// `Interface\FrameXML\`, which declare `THICK`. Drawn without it, the same
    /// text is a thin gold line over a busy background where the reference
    /// client's is a hard-edged one, which is exactly the "less bold" in the
    /// report this was read for.
    pub outline: Outline,
    /// Fold the text at the rectangle's width — see [`wrap_width`], which is
    /// the one place that decides, so the height [`intrinsic`] reserves and the
    /// galley the painter builds cannot disagree about whether there is a fold.
    pub wrap: bool,
    /// …and how many rows it may fold into: `<FontString maxLines="3">`, or 0
    /// for "as many as it takes".
    ///
    /// Passed on to the painter as well as used in the height, because the two
    /// answer different halves: without it here a four-row name would reserve
    /// three rows and *draw* four, which overprints whatever is beneath it.
    pub max_rows: usize,
    /// **Whether this region is a `FontString`** — which decides what an empty
    /// one means. A `Texture` with a colour and no path is a solid fill
    /// (`SetTexture(0, 0, 0, 0.5)` is how the directory writes one); a
    /// `FontString` paints its text or nothing at all. Before this flag the two
    /// shared the fill rule, and every empty `MessageFrame`'s font declaration —
    /// `UIErrorsFrame`, `RaidWarningFrame`, `RaidBossEmoteFrame` — drew as a
    /// solid gold 512-unit bar across the middle of the screen.
    pub is_font: bool,
    /// **The unit whose picture this region is**, when `SetPortraitTexture` has
    /// been called on it — see [`PORTRAIT_KEY`].
    ///
    /// A token (`"player"`, `"target"`) rather than a guid, and resolved by the
    /// pass that takes the picture rather than here. That is a deviation from
    /// 1.12, which resolves at the call and leaves a *still* behind, and it is
    /// stated in `crate::render::portraits`: the interface re-calls this on
    /// every `UNIT_PORTRAIT_UPDATE` and every target change, so the two agree in
    /// every case a session reaches, and the live form cannot show a stale face.
    ///
    /// It does **not** displace [`Self::texture`]: `TargetPortrait` is a plain
    /// `<Texture>` with art in the XML and gets its portrait at run time, and
    /// the path is what a portrait that cannot be taken falls back to.
    pub portrait: Option<String>,
    /// **Radians this texture has been turned by, clockwise on the screen**, and
    /// 0 for the whole of both shipped directories — see [`set_rotation`], whose
    /// one caller is the world map's player arrow.
    pub rotation: f32,
}

/// Read a region's whole paint state.
///
/// Returns `None` for an object that is not a region, which is how the draw pass
/// tells a frame from the things inside it without a second table of kinds.
pub fn paint(object: &mlua::Table) -> Option<Paint> {
    let is_font = match object.raw_get::<Option<mlua::String>>(widget::KIND_KEY).ok()? {
        Some(kind) if kind == FONT_STRING => true,
        Some(kind) if kind == TEXTURE => false,
        _ => return None,
    };
    let colour = floats(object, COLOUR_KEY).map_or([1.0; 4], |(values, count)| match count {
        // Three components is the shape `<Color r g b/>` has 73 times over, and
        // the alpha it means is opaque.
        3 => [values[0], values[1], values[2], 1.0],
        4 => values,
        _ => [1.0; 4],
    });
    // **A region pays only for the fields its own kind can carry**, which is the
    // rule the `shadow` line below has always been under, applied to the other
    // ten. This function runs once per drawn object per frame — 376 of them with
    // the bags open, 102 without — and every field is an `mlua` call: a Lua
    // string push, a hash and a table lookup apiece. Half of them were being
    // asked of an object that has no way to answer.
    //
    // The split is not a guess about what is *usually* set, it is what the two
    // widget kinds *are*. `SetFont`, `SetJustifyH`, `SetJustifyV`, `maxLines`,
    // the wrap and `<Shadow>` are `FontString`'s and `SetTexture`,
    // `SetTexCoord` and `alphaMode` are `Texture`'s — and the painter proves it
    // from the other end: a font string draws its text or nothing (`is_font`
    // returns before the texture is looked at), and a texture never reaches
    // `label`.
    //
    // **The rotation is on this side of that split and it is a twelfth read** —
    // one `raw_get` per drawn *texture* per walk, which at 300 textures on a
    // 30 Hz clock is under a millisecond a second. It is here rather than
    // resolved by name in the painter because it is a property of the region:
    // exactly one object in the client sets it today (the world map's player
    // arrow), and any second one that ever does gets it for free.
    let (texture, portrait, coords, corners, blend, rotation) = if is_font {
        (None, None, None, None, "BLEND", 0.0)
    } else {
        // **The two forms are told apart by the count and never mixed.** Four
        // numbers is a rectangle; eight is four corners of a quad that may be
        // rotated, which is how the flight map draws a route — see
        // [`Paint::corners`]. Anything else is neither.
        let written = floats(object, COORDS_KEY);
        (
            object.raw_get(TEXTURE_KEY).ok().flatten(),
            object.raw_get(PORTRAIT_KEY).ok().flatten(),
            written.as_ref().filter(|(_, count)| *count == 4).map(|(v, _)| *v),
            written.filter(|(_, count)| *count == 8).and_then(|_| corners_of(object)),
            word(object, BLEND_KEY, BLENDS, "BLEND"),
            object
                .raw_get::<Option<f64>>(ROTATION_KEY)
                .ok()
                .flatten()
                .unwrap_or(0.0) as f32,
        )
    };
    let words = is_font.then(|| {
        (
            object.raw_get(FONT_KEY).ok().flatten(),
            object
                .raw_get::<Option<f64>>(FONT_HEIGHT_KEY)
                .ok()
                .flatten()
                .unwrap_or(0.0) as f32,
            word(object, JUSTIFY_H_KEY, JUSTIFY_H, "CENTER"),
            word(object, JUSTIFY_V_KEY, JUSTIFY_V, "MIDDLE"),
            wrap_width(object, true).is_some() || pinned_across(object),
            object
                .raw_get::<Option<usize>>(super::messages::MAX_LINES_KEY)
                .ok()
                .flatten()
                .unwrap_or(0),
        )
    });
    let (font, font_height, justify_h, justify_v, wrap, max_rows) =
        words.unwrap_or((None, 0.0, "CENTER", "MIDDLE", false, 0));
    Some(Paint {
        texture,
        portrait,
        // **The text stays on both paths**, unlike the six above it, because
        // `worth_drawing` reads it for a `Texture` too — a coloured, textless
        // region is a solid fill and a textured one is a picture, and telling
        // those apart is what stopped every empty `MessageFrame` drawing as a
        // gold bar.
        text: object.raw_get(TEXT_KEY).ok().flatten(),
        colour,
        blend,
        layer: word_index(object, LAYER_KEY, LAYERS).unwrap_or(2),
        coords,
        corners,
        font,
        font_height,
        justify_h,
        justify_v,
        wrap,
        max_rows,
        shadow: is_font.then(|| shadow_of(object)).flatten(),
        outline: if is_font { outline_of(object) } else { Outline::None },
        is_font,
        rotation,
    })
}

/// **Turn a texture about its own centre**, in radians clockwise on the screen.
///
/// The client's own write and not a Lua method: nothing in either directory
/// turns a `<Texture>` — 1.12's `SetRotation` belongs to `<Model>` — and the one
/// object here that needs turning is a frame this client builds itself, the
/// world map's player arrow. See [`crate::lua::panels::worldmap`], where the arrow is,
/// and note that 1.12's own is a `.mdx` rather than a quad.
pub(in crate::lua) fn set_rotation(region: &mlua::Table, radians: f32) -> mlua::Result<()> {
    if radians == 0.0 {
        // Cleared rather than stored, so the painter's cheap `!= 0.0` test can
        // stay the whole of the decision to build a turned quad.
        return region.raw_set(ROTATION_KEY, mlua::Value::Nil);
    }
    region.raw_set(ROTATION_KEY, f64::from(radians))
}

/// The two region kinds, as byte strings so the comparison costs no allocation.
///
/// **`paint` runs once per drawn object per frame** — 376 of them with the bags
/// open — and `raw_get::<String>` on a key is a Lua string push, a hash and a
/// heap allocation for a word that is one of two. Reading it as an
/// [`mlua::String`] and comparing bytes keeps the read and drops the rest.
const FONT_STRING: &[u8] = b"FontString";
const TEXTURE: &[u8] = b"Texture";

/// Is this object one of the two drawing kinds? The gate [`intrinsic`] and
/// [`paint`] share, on the terms above.
fn is_kind(object: &mlua::Table, kind: &[u8]) -> bool {
    object
        .raw_get::<Option<mlua::String>>(widget::KIND_KEY)
        .ok()
        .flatten()
        .is_some_and(|found| found == kind)
}

/// The five `alphaMode` words, and the three each `justifyH`/`justifyV` takes.
///
/// Listed so that [`word`] can hand back a `&'static str` rather than a fresh
/// `String`: every one of these is a fixed vocabulary the files choose from, so
/// the value a region carries is always one of the words already in the table.
const BLENDS: [&str; 5] = ["BLEND", "ADD", "ALPHAKEY", "MOD", "DISABLE"];
const JUSTIFY_H: [&str; 3] = ["LEFT", "CENTER", "RIGHT"];
const JUSTIFY_V: [&str; 3] = ["TOP", "MIDDLE", "BOTTOM"];

/// One of a fixed vocabulary, or the default — **no allocation either way**.
///
/// A word the vocabulary does not have falls back to the default rather than
/// being carried through, which is the same thing the layer read below does and
/// what the painter's own `match` arms would do with it anyway.
fn word<const N: usize>(
    object: &mlua::Table,
    key: &str,
    vocabulary: [&'static str; N],
    default: &'static str,
) -> &'static str {
    match object.raw_get::<Option<mlua::String>>(key).ok().flatten() {
        Some(found) => vocabulary
            .into_iter()
            .find(|known| found == known.as_bytes())
            .unwrap_or(default),
        None => default,
    }
}

/// …and its index, which is what a draw layer is.
fn word_index<const N: usize>(
    object: &mlua::Table,
    key: &str,
    vocabulary: [&'static str; N],
) -> Option<usize> {
    let found = object.raw_get::<Option<mlua::String>>(key).ok().flatten()?;
    vocabulary.into_iter().position(|known| found == known.as_bytes())
}

/// A short run of numbers out of a Lua array, read in place.
///
/// `raw_get::<Vec<f64>>` allocates one `Vec` per call and both callers want at
/// most four values — which at 376 regions a frame is 752 heap allocations for
/// eight numbers apiece. Answers `(values, how many there were)`; anything
/// longer than four is reported by its real length so the caller can refuse it.
fn floats(object: &mlua::Table, key: &str) -> Option<([f32; 4], usize)> {
    let table = object.raw_get::<Option<mlua::Table>>(key).ok().flatten()?;
    let count = table.raw_len();
    let mut out = [0.0f32; 4];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = table.raw_get::<Option<f64>>(index + 1).ok().flatten().unwrap_or(0.0) as f32;
    }
    Some((out, count))
}

/// The eight-argument `SetTexCoord`, read whole — see [`Paint::corners`].
///
/// Its own reader rather than a wider [`floats`], because that one is called
/// per drawn region per walk for four different keys and would grow by four
/// table lookups for the one region on the screen that has eight.
fn corners_of(object: &mlua::Table) -> Option<[f32; 8]> {
    let table = object.raw_get::<Option<mlua::Table>>(COORDS_KEY).ok().flatten()?;
    let mut out = [0.0f32; 8];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = table.raw_get::<Option<f64>>(index + 1).ok().flatten()? as f32;
    }
    Some(out)
}

/// The `<Shadow>` a region declares, read as one pair.
///
/// An offset with no colour is black, which is what all six of the directory's
/// own declarations are; a colour with no offset is not a shadow at all, since
/// a copy drawn *under* the glyphs at no offset is invisible.
fn shadow_of(object: &mlua::Table) -> Option<([f32; 2], [f32; 4])> {
    let offset: Vec<f64> = object.raw_get(SHADOW_OFFSET_KEY).ok().flatten()?;
    let [x, y] = offset[..] else { return None };
    if x == 0.0 && y == 0.0 {
        return None;
    }
    let colour: Option<Vec<f64>> = object.raw_get(SHADOW_COLOUR_KEY).ok().flatten();
    let colour = match colour.as_deref() {
        Some([r, g, b, a]) => [*r as f32, *g as f32, *b as f32, *a as f32],
        Some([r, g, b]) => [*r as f32, *g as f32, *b as f32, 1.0],
        _ => [0.0, 0.0, 0.0, 1.0],
    };
    Some(([x as f32, y as f32], colour))
}

/// The `outline=` a region's face declares.
fn outline_of(object: &mlua::Table) -> Outline {
    match object.raw_get::<Option<String>>(OUTLINE_KEY).ok().flatten().as_deref() {
        Some("NORMAL") => Outline::Normal,
        Some("THICK") => Outline::Thick,
        _ => Outline::None,
    }
}

/// The registry key the shared metatable lives under — one for every region in
/// the game; see [`widget::metatable`].
const REG_META: &str = "vale.regionMeta";

/// The draw layer a region with no `level` lands in. `ARTWORK` is the game's own
/// default and it is the middle of the five, which is why a texture that forgot
/// to say lands over the background and under the text rather than at one end.
const DEFAULT_LAYER: &str = "ARTWORK";

/// **Which layer a *slot* region lands in**, which the markup never says and the
/// C widget decides.
///
/// A `<Button>`'s parts are not declared inside `<Layers><Layer>` — they are
/// `<NormalTexture>`, `<PushedTexture>`, `<HighlightTexture>` and
/// `<ButtonText>`, none of which carries a `drawLayer` in either directory. So
/// every one of them landed in [`DEFAULT_LAYER`] together and the tie-break fell
/// to creation order — which is the order the *template* declares them in.
///
/// `GlueButtonTemplate` declares `<ButtonText>` **first**, so every button on
/// the login screen drew its label and then painted its plate over it: the words
/// came out muddy brown under a semi-transparent red panel, which reads exactly
/// as a disabled button. `ActionButtonTemplate` has the same shape.
///
/// The rule is the widget's own arrangement rather than a preference: a button's
/// state art is behind, its highlight is the layer literally named for it, and
/// its label is on top. A slot this does not name keeps the default.
pub(in crate::lua) fn slot_layer(slot: &str) -> Option<&'static str> {
    match slot {
        // The label, above everything the button paints under it.
        "Text" => Some("OVERLAY"),
        // The mouse-over sheet, on the layer named for it — which is above
        // `OVERLAY` in [`LAYERS`], as the game orders them.
        "Highlight" => Some("HIGHLIGHT"),
        // The state plates and the two thumbs: the art the label sits on.
        "Normal" | "Pushed" | "Disabled" | "Checked" | "DisabledChecked" => Some("BORDER"),
        _ => None,
    }
}

/// Put a region in a draw layer, from Rust — the same field `SetDrawLayer`
/// writes, so a layer chosen by the loader and one chosen by a script cannot
/// disagree. See [`slot_layer`], its one caller.
pub(in crate::lua) fn set_layer(region: &mlua::Table, layer: &str) -> mlua::Result<()> {
    region.set(LAYER_KEY, layer)
}

/// Install the region metatable. Called once, from [`super::frames::install`].
pub(in crate::lua) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let methods = lua.create_table()?;
    widget::install(lua, &methods)?;
    register_methods(lua, &methods)?;
    install_measures(lua, &methods)?;
    // …and the region-side names with nothing behind them, last so that none of
    // them can shadow one above — see [`super::super::api::stubs`].
    super::super::api::stubs::install_region_methods(lua, &methods)?;
    lua.set_named_registry_value(REG_META, widget::metatable(lua, methods)?)
}

/// Make a `Texture` or a `FontString`.
///
/// `pub(super)` and one function for both, because `frame:CreateTexture(…)`,
/// `frame:CreateFontString(…)` and the XML loader's `<Layer>` walk must all
/// produce the same object — a region built one way and not the other is the
/// shape of bug that shows up as half an interface being invisible.
pub(in crate::lua) fn create(
    lua: &mlua::Lua,
    kind: &str,
    name: Option<&str>,
    parent: Option<mlua::Table>,
    layer: Option<&str>,
) -> mlua::Result<mlua::Table> {
    let region = lua.create_table()?;
    widget::init(lua, &region, kind, name, parent)?;
    region.set(LAYER_KEY, layer.unwrap_or(DEFAULT_LAYER))?;
    region.set(BLEND_KEY, "BLEND")?;

    region.set_metatable(Some(lua.named_registry_value::<mlua::Table>(REG_META)?))?;
    Ok(region)
}

fn register_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    macro_rules! method {
        ($name:expr, |$lua:ident, $this:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, $this: mlua::Table| $body)?;
            methods.set($name, f)?;
        }};
        ($name:expr, $args:ty, |$lua:ident, $this:ident, $arg:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, ($this, $arg): (mlua::Table, $args)| $body)?;
            methods.set($name, f)?;
        }};
    }

    // **`SetTexture` takes a path or a colour**, and the two are told apart by
    // arity: one string argument is a file, three or four numbers are a solid
    // fill. `MoneyFrame` and every backdrop in the directory use the second, and
    // a host that only took the first turns those into nothing at all.
    let set_texture = lua.create_function(|_lua, args: mlua::MultiValue| {
        let mut args = args.into_iter();
        let this: mlua::Table = mlua::FromLua::from_lua(args.next().unwrap_or(mlua::Value::Nil), _lua)?;
        let rest: Vec<mlua::Value> = args.collect();
        // **Whatever this call means, it means the region is not a portrait
        // any more.** `PetFrame` and the party frames set a portrait through
        // one function and a path through the other onto the same `<Texture>`;
        // a region that kept both would show a face under an icon for the rest
        // of the session. See [`set_portrait_unit`].
        this.set(PORTRAIT_KEY, mlua::Value::Nil)?;
        match rest.first() {
            Some(mlua::Value::String(path)) => {
                this.set(TEXTURE_KEY, path.clone())?;
                this.set(COLOUR_KEY, mlua::Value::Nil)
            }
            Some(mlua::Value::Number(_) | mlua::Value::Integer(_)) => {
                // `widget::number` and not `as_f64` — see its comment; every
                // integral component of a colour arrives as `Value::Integer`,
                // and `SetTexture(0, 0, 0, 0.5)` would keep only the alpha.
                let colour: Vec<f64> = rest.iter().filter_map(widget::number).collect();
                this.set(TEXTURE_KEY, mlua::Value::Nil)?;
                this.set(COLOUR_KEY, colour)
            }
            // `SetTexture(nil)` clears it, which is how an empty action button
            // loses its icon.
            _ => {
                this.set(TEXTURE_KEY, mlua::Value::Nil)?;
                this.set(COLOUR_KEY, mlua::Value::Nil)
            }
        }
    })?;
    methods.set("SetTexture", set_texture)?;
    method!("GetTexture", |_lua, this| this
        .raw_get::<mlua::Value>(TEXTURE_KEY));

    // **`SetText` takes anything and stores a string**, because the directory
    // passes it numbers constantly (`count:SetText(charges)`), and Lua's
    // concatenation rules mean the game sees `"3"` either way.
    method!("SetText", mlua::Value, |lua, this, value| set_text_value(
        lua, &this, value
    ));
    method!("GetText", |lua, this| {
        let held: mlua::Value = this.raw_get(TEXT_KEY)?;
        if matches!(held, mlua::Value::Nil) {
            return empty_text(lua, &this);
        }
        Ok(held)
    });

    method!("SetVertexColor", mlua::Variadic<f64>, |_lua, this, rgba| this
        .set(COLOUR_KEY, rgba.to_vec()));
    method!("GetVertexColor", |_lua, this| {
        let rgba: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(COLOUR_KEY)?.unwrap_or_default();
        Ok((
            rgba.first().copied().unwrap_or(1.0),
            rgba.get(1).copied().unwrap_or(1.0),
            rgba.get(2).copied().unwrap_or(1.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ))
    });
    // `SetGradientAlpha` is the eight-number form of `SetGradient`, and is
    // recorded as nothing on the same terms as `SetAlphaGradient` below.
    method!("SetGradientAlpha", mlua::MultiValue, |_lua, _this, _args| Ok(()));
    // **`SetTextColor` is sticky and `SetVertexColor` is not**, which is the
    // two setters' own difference: only the first marks the string as carrying
    // its own colour, so only the first survives a button changing face. See
    // [`apply_font_style`].
    method!("SetTextColor", mlua::Variadic<f64>, |_lua, this, rgba| {
        this.set(COLOUR_KEY, rgba.to_vec())?;
        this.set(COLOUR_SET_KEY, true)
    });
    method!("SetTexCoord", mlua::Variadic<f64>, |_lua, this, coords| this
        .set(COORDS_KEY, coords.to_vec()));
    // **`SetTextHeight` writes the same key `<Font height=…>` does**, through
    // the same [`set_font_height`] — so the rasteriser's own cap applies to a
    // size a script asked for exactly as it does to one the markup declared, and
    // `GetTextHeight` reads back what would be drawn rather than what was asked
    // for.
    //
    // Its one caller in the shipped directory is `ActionButton_UpdateHotkeys`,
    // which shrinks the hotkey to 8 to make room for the range dot — a body no
    // probe reached until `ActionHasRange` stopped being a stub, which is why a
    // method the interface has always called was missing for twenty rounds.
    method!("SetTextHeight", f32, |_lua, this, height| {
        let held = text_region(&this).unwrap_or_else(|| this.clone());
        set_font_height(&held, height)
    });
    // **`SetAlphaGradient(start, length)` — a `FontString`'s own fade**, and it
    // is on *this* table rather than the frame's because `QuestDescription` is
    // a font string: `QuestFrameDetailPanel:OnShow` calls it to fade the bottom
    // of a long story out as the panel scrolls it in.
    //
    // Recorded as nothing. This client draws a string at one alpha, so what is
    // lost is the gradient and not any of the words — and a *missing* name here
    // takes the whole `OnShow` with it, which is a quest page that opens blank.
    method!("SetAlphaGradient", mlua::MultiValue, |_lua, _this, _args| Ok(()));
    method!("SetJustifyH", String, |_lua, this, how| this
        .set(JUSTIFY_H_KEY, how));
    method!("SetJustifyV", String, |_lua, this, how| this
        .set(JUSTIFY_V_KEY, how));
    method!("SetDrawLayer", String, |_lua, this, layer| this
        .set(LAYER_KEY, layer));
    method!("GetDrawLayer", |_lua, this| this
        .raw_get::<mlua::Value>(LAYER_KEY));
    method!("SetBlendMode", String, |_lua, this, mode| this
        .set(BLEND_KEY, mode));
    method!("GetBlendMode", |_lua, this| this
        .raw_get::<mlua::Value>(BLEND_KEY));

    // **`SetFont(path, height, flags)`** — the face by file, as an addon sets
    // it (`pfUI` sets every one of its strings this way), and the three
    // outline words map onto the `<Font outline=…>` attribute's two.
    method!(
        "SetFont",
        (Option<String>, Option<f64>, Option<String>),
        |_lua, this, args| set_font_triplet(&this, args.0.as_deref(), args.1, args.2.as_deref())
    );
    method!("SetShadowOffset", (Option<f64>, Option<f64>), |_lua, this, args| this.set(
        SHADOW_OFFSET_KEY,
        vec![args.0.unwrap_or(0.0), args.1.unwrap_or(0.0)]
    ));
    method!("SetShadowColor", mlua::Variadic<f64>, |_lua, this, rgba| this.set(
        SHADOW_COLOUR_KEY,
        vec![
            rgba.first().copied().unwrap_or(0.0),
            rgba.get(1).copied().unwrap_or(0.0),
            rgba.get(2).copied().unwrap_or(0.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ]
    ));
    method!("GetTextColor", |_lua, this| {
        let rgba: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(COLOUR_KEY)?.unwrap_or_default();
        Ok((
            rgba.first().copied().unwrap_or(1.0),
            rgba.get(1).copied().unwrap_or(1.0),
            rgba.get(2).copied().unwrap_or(1.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ))
    });
    // **`SetFontObject(GameFontNormal)`** — a font object is the global a
    // virtual `<Font>` declares (see [`make_font_object`]); its face is copied
    // onto the string, which is what `inherits` does at load.
    method!("SetFontObject", mlua::Value, |lua, this, value| {
        let Some(font) = resolve_font_object(lua, value)? else {
            return Ok(());
        };
        apply_font_style(&this, &font)?;
        this.set(FONT_OBJECT_KEY, font)
    });
    method!("GetFontObject", |_lua, this| this
        .raw_get::<mlua::Value>(FONT_OBJECT_KEY));

    // `GetObjectType` is how FrameXML asks what it is holding —
    // `if ( region:GetObjectType() == "FontString" )`.
    method!("GetObjectType", |_lua, this| this
        .raw_get::<mlua::Value>(widget::KIND_KEY));
    // **…and `IsObjectType` is the same question asked the other way round, on
    // an object that may be either.** `UIParent_ManageFramePosition` walks a
    // table of names and asks `frame:IsObjectType("frame")` about whatever
    // `getglobal` handed back — which is sometimes a region, and a region with
    // no such method takes the whole of `UIParent`'s layout pass with it. Every
    // UIObject in 1.12 answers this one; only the *frame* table had it.
    // **…and it honours the type tree** — see [`widget::derives_from`]. A
    // `CheckButton` answers `1` to `Button` and every frame kind answers `1` to
    // `Frame`, which an equality test got wrong for every caller that asks the
    // general question rather than the specific one.
    method!("IsObjectType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });
    Ok(())
}

/// The loader's way in: set a texture path, a text, a blend mode or a layer from
/// an XML attribute, without going through Lua to do it.
pub(in crate::lua) fn set_file(region: &mlua::Table, path: &str) -> mlua::Result<()> {
    region.set(TEXTURE_KEY, path)
}

/// …and the same door for a C function that paints a portrait —
/// `SetPortraitToTexture` and `SetBagPortaitTexture`, both of which take a
/// texture *object* and a path and do nothing else. `None` clears it, which is
/// what a bag slot with no bag in it does.
pub(in crate::lua) fn set_texture_path(region: &mlua::Table, path: Option<&str>) -> mlua::Result<()> {
    match path {
        Some(path) => region.set(TEXTURE_KEY, path),
        None => region.set(TEXTURE_KEY, mlua::Value::Nil),
    }
}

/// …and the *other* door, for the C function that paints a **unit** —
/// `SetPortraitTexture(texture, "target")`. See [`PORTRAIT_KEY`] and
/// [`super::super::api::portrait`].
///
/// **`SetTexture` clears it**, and that is not tidiness: `TargetFrame.lua`'s
/// `TargetFrame_Update` sets a portrait and `PetFrame`'s does the same slot with
/// a path, so a region that kept both would draw a portrait under an icon for
/// ever after the one time it held a unit. The clearing is in [`set_file`]'s
/// sibling below rather than here because it is the *other* setter's duty.
pub(in crate::lua) fn set_portrait_unit(region: &mlua::Table, token: Option<&str>) -> mlua::Result<()> {
    match token {
        Some(token) => region.set(PORTRAIT_KEY, token),
        None => region.set(PORTRAIT_KEY, mlua::Value::Nil),
    }
}

/// **`text="CANCEL"` on a frame is its font string's text**, not the frame's.
///
/// A `<Button text="…">` whose template gave it a `<ButtonText>` has somewhere
/// for the label to go, and that somewhere is what gets drawn — the region has
/// the rectangle, the typeface and the layer, and the frame has none of them. So
/// the attribute is pushed down when there is a region to push it to, which is
/// what the game does and what makes `tab:GetText()` answer.
///
/// A frame with no font string keeps it on itself, so nothing is lost and
/// [`slot_key`]'s other half can move it when the region turns up later.
/// **What `GetText()` answers when nothing has been written**, which is not one
/// value.
///
/// An **`EditBox` answers `""`** and everything else answers **nil**, and the
/// difference is the game's own rather than a tidy-up: an edit box holds a C
/// string buffer that happens to be empty, and a font string holds a pointer
/// that is null. The directory is written against both, and against the edit
/// box's half load-bearingly —
///
/// ```lua
/// local copper = getglobal(moneyFrame:GetName().."Copper"):GetText();
/// if ( copper ~= "" ) then totalCopper = totalCopper + copper; end
/// ```
///
/// — where a nil passes the guard and then adds nil to a number.
/// `MoneyInputFrame_GetCopper` is called from `TradeFrame_OnShow`'s second line,
/// so this was the whole of why the trade window opened as bare art.
///
/// One door, because `GetText` is installed **twice** — here and on the shared
/// frame table by [`super::button`], whose copy shadows this one for every frame
/// in the interface.
pub(super) fn empty_text(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<mlua::Value> {
    let kind: Option<String> = object.raw_get(widget::KIND_KEY)?;
    Ok(match kind.as_deref() {
        Some("EditBox") => mlua::Value::String(lua.create_string("")?),
        _ => mlua::Value::Nil,
    })
}

pub(in crate::lua) fn set_text(lua: &mlua::Lua, object: &mlua::Table, text: &str) -> mlua::Result<()> {
    let holder = text_region(object).unwrap_or_else(|| object.clone());
    write_text(lua, &holder, mlua::Value::String(lua.create_string(text)?))
}

/// **Writing a string is a geometry write.** An auto-sized `FontString` takes its
/// rectangle from the text it holds ([`intrinsic`]), so a new string is a new
/// rectangle and the layout memo has to be told — otherwise a label keeps the
/// width of whatever it said first, which for one that started empty is nothing
/// at all.
///
/// **And a write that changes nothing invalidates nothing**, which is not an
/// optimisation but the difference between this working and the collector
/// saw-tooth coming back: the shipped `OnUpdate` bodies re-assert their labels
/// every tick — `CastingBarFrame`'s timer, the durability text, every
/// `SetText(format(…))` in the interface — and invalidating the whole solve for
/// each would burn a generation per frame per label. The same rule
/// [`super::widget`]'s five geometry setters take, for the same reason.
fn write_text(lua: &mlua::Lua, holder: &mlua::Table, text: mlua::Value) -> mlua::Result<()> {
    let before: Option<String> = holder.raw_get(TEXT_KEY).ok().flatten();
    let after = match &text {
        mlua::Value::String(s) => Some(s.to_string_lossy().to_string()),
        _ => None,
    };
    if before == after {
        return Ok(());
    }
    holder.set(TEXT_KEY, text)?;
    // Only a font string's own rectangle depends on its text; a frame holding a
    // pending label has none of its own. Asking [`intrinsic`] is a raw read.
    if holder.raw_get::<Option<String>>(widget::KIND_KEY).ok().flatten().as_deref()
        == Some("FontString")
    {
        super::layout::invalidate(lua)?;
    }
    Ok(())
}

/// …and the same rule from Lua, for any value.
///
/// One function behind both `FontString:SetText` and `Button:SetText`, because
/// they differ only in whether the object has a font string to push to — a
/// `FontString` has none and so writes its own, which is the behaviour it had
/// when it was written separately.
pub(super) fn set_text_value(
    lua: &mlua::Lua,
    object: &mlua::Table,
    value: mlua::Value,
) -> mlua::Result<()> {
    let text = match value {
        mlua::Value::Nil => mlua::Value::Nil,
        other => mlua::Value::String(lua.create_string(stringify(&other))?),
    };
    let holder = text_region(object).unwrap_or_else(|| object.clone());
    write_text(lua, &holder, text)
}

/// **`frame:SetText(value)`, whatever kind of frame it is** — the one body
/// behind a method that is installed twice.
///
/// `SetText` is on the shared frame table and two modules put it there:
/// [`super::button`] forwards a label to the button's own font string, and
/// [`super::tooltip`] replaces the whole method because `GameTooltip:SetText`
/// means something else entirely — so the tooltip's copy, being installed last,
/// is the one every frame in the game actually calls, and its "not a tooltip"
/// branch is the real implementation. That was fine while the branch was one
/// line; it stopped being fine the moment an `EditBox` needed `OnTextSet` fired
/// from it, because the *other* copy is the one that looked like it mattered.
///
/// One function, called from both. See [`super::editbox::text_was_set`] for what
/// the second half of it is for.
pub(super) fn set_frame_text(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    value: mlua::Value,
) -> mlua::Result<()> {
    set_text_value(lua, frame, value)?;
    super::editbox::text_was_set(lua, frame)
}

/// **The style a frame lays its own text out in**: its unnamed `<FontString>`.
///
/// Two widgets need this and they need it for the same reason — the text they
/// draw has no object. A `MessageFrame`'s lines and an `EditBox`'s typed line
/// are both laid out by the C widget itself, in the style of the one font string
/// declared *inside* the element and given no name, no anchors and no text:
///
/// ```xml
/// <ScrollingMessageFrame name="ChatFrameTemplate" …>
///     <FontString inherits="ChatFontNormal" justifyH="LEFT"/>
/// <EditBox name="ChatFrameEditBoxTemplate" …>
///     <FontString inherits="ChatFontNormal" bytes="256"/>
/// ```
///
/// It is here rather than in either of them because it is a fact about how a
/// region is *declared*, and because the two had one copy of it between them
/// until the second one wanted it.
///
/// The `text.is_none()` test is what tells the style from a real label: a
/// `<FontString>` carrying a string is something the frame draws, not the shape
/// it draws other things in.
///
/// **And `is_font`, which is not belt-and-braces.** A `Texture` has no text
/// either, so without it the first *texture* declared inside the frame wins —
/// and `ChatFrameEditBoxTemplate` declares three of them, in a `<Layer>` above
/// the `<FontString>` at the end. The typed line was being laid out in the
/// style of `UI-ChatInputBorder-Left`: no face, no height, so Friz Quadrata at
/// the default 12 where the game says Arial Narrow at 14.
pub(super) fn own_font(frame: &mlua::Table) -> Option<Paint> {
    widget::children(frame)
        .ok()?
        .sequence_values::<mlua::Table>()
        .flatten()
        .find_map(|child| paint(&child).filter(|paint| paint.is_font && paint.text.is_none()))
}

/// A frame's own font string, if it has one — see [`TEXT_REGION_KEY`].
pub(in crate::lua) fn text_region(object: &mlua::Table) -> Option<mlua::Table> {
    object.raw_get::<Option<mlua::Table>>(TEXT_REGION_KEY).ok().flatten()
}

/// **A button's label does not need a `<ButtonText>`** — the widget makes one
/// the moment it is given a face or a string.
///
/// A button's `SetText` looks for its font string and, finding none,
/// *creates* one, parents it to the button and installs it through the same
/// `SetFontString` that `<ButtonText>` goes through, which then anchors it to
/// the button on the point its `justifyH` names because the fresh string has
/// no anchors of its own.
///
/// **18 templates and buttons over the two directories declare a `<NormalFont>`
/// and no `<ButtonText>`**, and every label on every one of them was being
/// dropped: `StaticPopupButtonTemplate` is Accept, Decline, Release Spirit and
/// Retrieve Corpse — every dialog in the game — and `UIMenuButtonTemplate`,
/// `UIPanelButtonGrayTemplate` and `GlueMenuButtonTemplate` are the rest. The
/// string was stored on the frame by [`set_frame_text`] and read back correctly
/// by `GetText`, which is why nothing raised: the box drew its plate, sized its
/// buttons off `GetTextWidth`, and painted no words.
///
/// **The hook is the font rather than `SetText`**, which is one step short of
/// what the client does and lands in the same place: a font string with no font
/// object draws nothing in 1.12 either, so the buttons this refuses (the 1,002
/// that declare neither) are exactly the ones whose implicit string would be
/// invisible. What it buys is that the face is there to apply — this client
/// keeps a region's typeface *on* the region, where the widget keeps three font
/// objects and picks one per state.
pub(in crate::lua) fn button_text_region(
    lua: &mlua::Lua,
    object: &mlua::Table,
) -> Option<mlua::Table> {
    if let Some(region) = text_region(object) {
        return Some(region);
    }
    let kind: Option<String> = object.raw_get(widget::KIND_KEY).ok().flatten();
    if !matches!(kind.as_deref(), Some("Button") | Some("CheckButton")) {
        return None;
    }
    let made = create(lua, "FontString", None, Some(object.clone()), slot_layer("Text")).ok()?;
    // The same two things the loader gives a declared `<ButtonText>`: no anchors
    // means fill the button (see the note in [`super::super::xml`]), and the label goes
    // above the plate rather than under it.
    widget::default_all_points(lua, &made).ok()?;
    object.set(TEXT_REGION_KEY, made.clone()).ok()?;
    // …and a `text="ACCEPT"` attribute that arrived before the face did.
    adopt_pending_text(lua, object, &made);
    Some(made)
}

/// **What this object's text is, whichever of the two ways it holds it.**
///
/// A `FontString`'s own string, or the string in a frame's font string, or the
/// one left on a frame that has none. Never raises: the readers of this are
/// `GetText` and `GetTextWidth`, and the whole reason they are here is that a
/// wrong guess about which shape `__text` was in aborted the `OnLoad`.
pub(in crate::lua) fn text_of(object: &mlua::Table) -> Option<String> {
    if let Some(region) = text_region(object) {
        return region.raw_get::<Option<String>>(TEXT_KEY).ok().flatten();
    }
    object.raw_get::<Option<String>>(TEXT_KEY).ok().flatten()
}

/// Where a slot region is stored on its parent — `<NormalTexture>` under
/// `__normal`, and `<ButtonText>` under [`TEXT_REGION_KEY`] rather than under
/// the key a region's *string* uses.
///
/// One function so that the loader and [`super::button`]'s getters cannot drift:
/// they are the two halves of the same convention, and they were written
/// separately once already.
pub(in crate::lua) fn slot_key(slot: &str) -> String {
    if slot.eq_ignore_ascii_case("Text") {
        return TEXT_REGION_KEY.to_string();
    }
    format!("__{}", slot.to_lowercase())
}

/// **The other order.** The slot arrived after the attribute did — a
/// `<Button text="X">` whose own `<ButtonText>` is a child rather than
/// inherited — so the string is sitting on the frame with nowhere to draw.
/// Move it down as the region is attached.
pub(in crate::lua) fn adopt_pending_text(lua: &mlua::Lua, object: &mlua::Table, region: &mlua::Table) {
    let Some(text) = object.raw_get::<Option<String>>(TEXT_KEY).ok().flatten() else {
        return;
    };
    if let Ok(text) = lua.create_string(&text) {
        let _ = write_text(lua, region, mlua::Value::String(text));
    }
    let _ = object.set(TEXT_KEY, mlua::Value::Nil);
}

pub(in crate::lua) fn set_blend(region: &mlua::Table, mode: &str) -> mlua::Result<()> {
    region.set(BLEND_KEY, mode)
}

pub(in crate::lua) fn set_colour(region: &mlua::Table, rgba: [f64; 4]) -> mlua::Result<()> {
    region.set(COLOUR_KEY, rgba.to_vec())
}

/// …and the **sticky** one, which is what `SetTextColor` writes.
///
/// The difference is [`COLOUR_SET_KEY`] and it is the whole of a bug that had
/// been reported twice: `Button:SetTextColor` forwards to the button's own font
/// string, and a button with a `<HighlightFont>` re-applies a face every time
/// the pointer crosses it. Without the flag, the forwarded colour was erased on
/// the hover and again on the leave — a trainer's green spells and a quest log's
/// difficulty colours flipping to `GameFontHighlight`'s white and then to
/// `GameFontNormal`'s gold under the pointer, which is what "the colours change
/// randomly when you hover over them" is.
///
/// [`apply_font_style`] is the reader, and 1.12's own two fields (the colour
/// with its flag, against the font object) are why the two setters differ at
/// all.
pub(super) fn set_text_colour(region: &mlua::Table, rgba: [f64; 4]) -> mlua::Result<()> {
    set_colour(region, rgba)?;
    region.set(COLOUR_SET_KEY, true)
}

/// Whether this font string folds its text — see [`WRAP_KEY`].
pub(super) fn set_wrap(region: &mlua::Table, wrap: bool) -> mlua::Result<()> {
    region.set(WRAP_KEY, wrap)
}

/// `<TexCoords left="0" right="1.0" top="0.79296875" bottom="0.83203125"/>` —
/// **308 of them**, and the same store `SetTexCoord` writes to.
///
/// Which part of the file a texture draws, and the reason so much of the
/// interface is one atlas: the four experience-bar fills above are four slices of
/// `UI-MainMenuBar-Dwarf` at four y ranges. A loader that dropped these drew the
/// whole sheet in each of their rectangles — every one of the 308 squashed into
/// the wrong shape, which reads as "the art is wrong" rather than as a gap.
pub(in crate::lua) fn set_coords(region: &mlua::Table, coords: [f64; 4]) -> mlua::Result<()> {
    region.set(COORDS_KEY, coords.to_vec())
}

/// `font="Fonts\FRIZQT__.TTF"` and `<FontHeight><AbsValue val="12"/>`.
///
/// **These arrive through `inherits`, not on the element.** A `<FontString>` in
/// the directory says `inherits="GameFontNormalSmall"` and nothing else; the
/// face, the height and the colour are all on the `<Font>` object in `Fonts.xml`,
/// which the loader applies as a template. So a client that read only the
/// element's own attributes would find a typeface on none of the interface's
/// 1,652 regions.
pub(in crate::lua) fn set_font(region: &mlua::Table, path: &str) -> mlua::Result<()> {
    region.set(FONT_KEY, path)
}


/// **`SetFont`'s three arguments**, applied to whatever carries a face: a
/// font string, a frame's own string, or a font object. The flags word is
/// `"OUTLINE"`, `"THICKOUTLINE"`, a comma-joined list of those and
/// `"MONOCHROME"`, or nothing — and nothing clears the outline, which is what
/// the reference does with a call that omits it.
pub(in crate::lua) fn set_font_triplet(
    region: &mlua::Table,
    path: Option<&str>,
    height: Option<f64>,
    flags: Option<&str>,
) -> mlua::Result<()> {
    if let Some(path) = path {
        set_font(region, path)?;
    }
    if let Some(height) = height {
        set_font_height(region, height as f32)?;
    }
    let flags = flags.unwrap_or("").to_ascii_uppercase();
    if flags.contains("THICKOUTLINE") {
        set_outline(region, "THICK")
    } else if flags.contains("OUTLINE") {
        set_outline(region, "NORMAL")
    } else {
        region.set(OUTLINE_KEY, mlua::Value::Nil)
    }
}

/// `SetFont`'s answer: `path, height, flags`, off the same three keys.
fn font_triplet(lua: &mlua::Lua, region: &mlua::Table) -> mlua::Result<(mlua::Value, f64, mlua::Value)> {
    let path = region.raw_get::<Option<String>>(FONT_KEY)?;
    let height = region.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)?.unwrap_or(0.0);
    let flags = match region.raw_get::<Option<String>>(OUTLINE_KEY)?.as_deref() {
        Some("THICK") => Some("THICKOUTLINE"),
        Some(_) => Some("OUTLINE"),
        None => None,
    };
    Ok((
        match path {
            Some(p) => mlua::Value::String(lua.create_string(&p)?),
            None => mlua::Value::Nil,
        },
        height,
        match flags {
            Some(f) => mlua::Value::String(lua.create_string(f)?),
            None => mlua::Value::Nil,
        },
    ))
}

/// **A font object by value or by name**: `SetFontObject(GameFontNormal)` and
/// `SetFontObject("GameFontNormal")` both occur in addons. Anything else is
/// `None`, which the caller ignores.
pub(in crate::lua) fn resolve_font_object(lua: &mlua::Lua, value: mlua::Value) -> mlua::Result<Option<mlua::Table>> {
    Ok(match value {
        mlua::Value::Table(table) => Some(table),
        mlua::Value::String(name) => lua.globals().get::<Option<mlua::Table>>(name.to_str()?.to_string())?,
        _ => None,
    })
}

/// **A font object**: the global a virtual `<Font name="…">` declares.
///
/// The reference makes one real object per `<Font>` whatever `virtual` says,
/// and an addon reads it — `SystemFont:GetFont()` is pfUI's first line of
/// font handling. Here it is a table carrying the same face keys a string
/// does, filled by the loader through the same `inherits` walk, so
/// [`apply_font_style`] can copy it onto a string and `GetFont` reads it as
/// it reads a string.
pub(in crate::lua) fn make_font_object(lua: &mlua::Lua, name: &str) -> mlua::Result<mlua::Table> {
    let font = lua.create_table()?;
    font.set(FONT_NAME_KEY, name)?;
    font.set(widget::KIND_KEY, "Font")?;
    let meta = lua.create_table()?;
    meta.set("__index", font_object_methods(lua)?)?;
    font.set_metatable(Some(meta))?;
    Ok(font)
}

/// The font objects' methods, built once per state.
fn font_object_methods(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    if let Some(methods) = lua.named_registry_value::<Option<mlua::Table>>(REG_FONT_METHODS)? {
        return Ok(methods);
    }
    let methods = lua.create_table()?;
    macro_rules! method {
        ($name:expr, |$lua:ident, $this:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, $this: mlua::Table| $body)?;
            methods.set($name, f)?;
        }};
        ($name:expr, $args:ty, |$lua:ident, $this:ident, $arg:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, ($this, $arg): (mlua::Table, $args)| $body)?;
            methods.set($name, f)?;
        }};
    }
    method!("GetName", |_lua, this| this.raw_get::<mlua::Value>(FONT_NAME_KEY));
    method!("GetObjectType", |_lua, _this| Ok("Font"));
    method!("IsObjectType", Option<String>, |_lua, _this, kind| Ok(
        super::super::api::one_or_nil(kind.is_some_and(|k| k.eq_ignore_ascii_case("Font")))
    ));
    method!("GetFont", |lua, this| font_triplet(lua, &this));
    method!(
        "SetFont",
        (Option<String>, Option<f64>, Option<String>),
        |_lua, this, args| set_font_triplet(&this, args.0.as_deref(), args.1, args.2.as_deref())
    );
    method!("GetTextColor", |_lua, this| {
        let rgba: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(COLOUR_KEY)?.unwrap_or_default();
        Ok((
            rgba.first().copied().unwrap_or(1.0),
            rgba.get(1).copied().unwrap_or(1.0),
            rgba.get(2).copied().unwrap_or(1.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ))
    });
    method!("SetTextColor", mlua::Variadic<f64>, |_lua, this, rgba| set_colour(
        &this,
        [
            rgba.first().copied().unwrap_or(1.0),
            rgba.get(1).copied().unwrap_or(1.0),
            rgba.get(2).copied().unwrap_or(1.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ]
    ));
    method!("GetShadowColor", |_lua, this| {
        let rgba: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(SHADOW_COLOUR_KEY)?.unwrap_or_default();
        Ok((
            rgba.first().copied().unwrap_or(0.0),
            rgba.get(1).copied().unwrap_or(0.0),
            rgba.get(2).copied().unwrap_or(0.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ))
    });
    method!("SetShadowColor", mlua::Variadic<f64>, |_lua, this, rgba| this.set(
        SHADOW_COLOUR_KEY,
        vec![
            rgba.first().copied().unwrap_or(0.0),
            rgba.get(1).copied().unwrap_or(0.0),
            rgba.get(2).copied().unwrap_or(0.0),
            rgba.get(3).copied().unwrap_or(1.0),
        ]
    ));
    method!("GetShadowOffset", |_lua, this| {
        let offset: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(SHADOW_OFFSET_KEY)?.unwrap_or_default();
        Ok((offset.first().copied().unwrap_or(0.0), offset.get(1).copied().unwrap_or(0.0)))
    });
    method!("SetShadowOffset", (Option<f64>, Option<f64>), |_lua, this, args| this.set(
        SHADOW_OFFSET_KEY,
        vec![args.0.unwrap_or(0.0), args.1.unwrap_or(0.0)]
    ));
    method!("GetJustifyH", |_lua, this| Ok(this
        .raw_get::<Option<String>>(JUSTIFY_H_KEY)?
        .unwrap_or_else(|| "CENTER".to_string())));
    method!("GetJustifyV", |_lua, this| Ok(this
        .raw_get::<Option<String>>(JUSTIFY_V_KEY)?
        .unwrap_or_else(|| "MIDDLE".to_string())));
    method!("SetJustifyH", String, |_lua, this, how| this.set(JUSTIFY_H_KEY, how));
    method!("SetJustifyV", String, |_lua, this, how| this.set(JUSTIFY_V_KEY, how));
    method!("GetSpacing", |_lua, _this| Ok(0.0));
    method!("SetSpacing", mlua::MultiValue, |_lua, _this, _args| Ok(()));
    method!("GetAlpha", |_lua, _this| Ok(1.0));
    method!("SetAlpha", mlua::MultiValue, |_lua, _this, _args| Ok(()));
    method!("GetFontObject", |_lua, this| this.raw_get::<mlua::Value>(FONT_OBJECT_KEY));
    // `SetFontObject` and `CopyFontObject` on a font are the same copy a
    // string makes: every face key, from the other font.
    for name in ["SetFontObject", "CopyFontObject"] {
        let f = lua.create_function(move |lua, (this, value): (mlua::Table, mlua::Value)| {
            let Some(other) = resolve_font_object(lua, value)? else {
                return Ok(());
            };
            apply_font_style(&this, &other)?;
            this.set(FONT_OBJECT_KEY, other)
        })?;
        methods.set(name, f)?;
    }
    lua.set_named_registry_value(REG_FONT_METHODS, methods.clone())?;
    Ok(methods)
}

/// …and the height, **clamped where it is written** rather than where it is
/// drawn.
///
/// [`vale_assets::interface::font::drawn_height`] is the client's own rasteriser cap and
/// the argument for it is there. It belongs on the write because everything
/// downstream — the painter, [`text_width`], [`line_height_of`], `GetFont` —
/// reads this one key, and a cap applied on only some of those paths is a plate
/// measured for one size and filled at another. The client stores the same
/// thing: its getter answers the *rasterised* size, not the requested one.
pub(in crate::lua) fn set_font_height(region: &mlua::Table, height: f32) -> mlua::Result<()> {
    region.set(
        FONT_HEIGHT_KEY,
        f64::from(vale_assets::interface::font::drawn_height(height)),
    )
}

/// `<Shadow>` — the offset copy under the glyphs, and its colour. Two keys
/// rather than a table, because the draw reads them every frame and a nested
/// table read per font string is the shape this directory has priced before.
/// `outline="NORMAL"` on a `<Font>` — see [`Paint::outline`].
pub(in crate::lua) fn set_outline(region: &mlua::Table, how: &str) -> mlua::Result<()> {
    region.set(OUTLINE_KEY, how)
}

pub(in crate::lua) fn set_shadow(
    region: &mlua::Table,
    offset: [f32; 2],
    colour: [f64; 4],
) -> mlua::Result<()> {
    region.set(SHADOW_OFFSET_KEY, offset.map(f64::from).to_vec())?;
    region.set(SHADOW_COLOUR_KEY, colour.to_vec())
}

pub(in crate::lua) fn set_justify(region: &mlua::Table, key: &str, how: &str) -> mlua::Result<()> {
    let slot = match key {
        "justifyH" => JUSTIFY_H_KEY,
        _ => JUSTIFY_V_KEY,
    };
    region.set(slot, how)
}

/// Lua's own `tostring` for the values `SetText` is given: a string, a number,
/// or something that is neither.
fn stringify(value: &mlua::Value) -> String {
    match value {
        mlua::Value::String(s) => s.to_string_lossy(),
        mlua::Value::Integer(n) => n.to_string(),
        // 5.1 prints an integral float without a fractional part, which is what
        // makes `count:SetText(3)` read "3" rather than "3.0" on a button.
        mlua::Value::Number(n) if n.fract() == 0.0 => format!("{}", *n as i64),
        mlua::Value::Number(n) => n.to_string(),
        mlua::Value::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        super::super::frames::install(&lua).expect("the object model installs");
        lua
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// **A region made from Lua is the same object the loader makes**, which is
    /// the whole reason [`create`] is one function: `ActionButton.lua` reaches
    /// `$parentIcon` — declared in XML — and calls `SetTexture` on it.
    #[test]
    fn a_created_region_carries_the_regions_methods() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Button", "ActionButton1");
            icon = f:CreateTexture("ActionButton1Icon", "BACKGROUND");
            icon:SetTexture("Interface\\Icons\\Spell_Fire_Fireball");
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(
            eval(&lua, "return icon:GetObjectType()"),
            r#"String("Texture")"#
        );
        assert_eq!(
            eval(&lua, "return icon:GetDrawLayer()"),
            r#"String("BACKGROUND")"#
        );
        assert_eq!(
            eval(&lua, "return getglobal(\"ActionButton1Icon\") == icon"),
            "Boolean(true)",
            "a named region is a global, as a named frame is"
        );
        assert_eq!(
            eval(&lua, "return icon:GetParent() == f"),
            "Boolean(true)"
        );
        // …and the base methods came with it.
        assert_eq!(eval(&lua, "return icon:IsShown()"), "Integer(1)");
    }

    /// **`GetTextHeight` counts the rows the string folds into**, which is the
    /// gossip menu's bug.
    ///
    /// `GossipResize` sizes each option button to `GetTextHeight() + 2` and the
    /// next button hangs off that bottom edge, so a one-line answer for a
    /// two-line label paints every option over the one below it.
    ///
    /// Asserted as a ratio rather than in pixels: with no archives open the
    /// measurement falls back to a fixed ratio per character, which is enough to
    /// fold and not the number a real face gives.
    #[test]
    fn a_wrapped_label_reports_the_height_it_will_draw_in() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            s = f:CreateFontString(nil, "ARTWORK");
            s:SetTextHeight(10);
            s:SetWidth(60);
            s:SetText("one");
            "#,
        )
        .exec()
        .expect("loads");
        let one: f64 = lua.load("return s:GetStringHeight()").eval().expect("runs");
        assert!(one > 0.0, "a one-row string is one line box: {one}");

        lua.load(r#"s:SetText("a much longer sentence than sixty units of width")"#)
            .exec()
            .expect("loads");
        let many: f64 = lua.load("return s:GetStringHeight()").eval().expect("runs");
        assert!(
            many >= one * 2.0,
            "a folded string must report every row it draws: {many} against {one}"
        );
        assert_eq!(many % one, 0.0, "…and a whole number of them");
    }

    /// A string with **no declared width does not fold**, so its height stays
    /// one line box however long it is. That is the half that must not regress:
    /// most of the directory's labels are declared with an anchor and no size
    /// and are as wide as they need to be.
    #[test]
    fn an_unbounded_label_is_one_line_however_long() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            s = f:CreateFontString(nil, "ARTWORK");
            s:SetTextHeight(10);
            s:SetText("short");
            "#,
        )
        .exec()
        .expect("loads");
        let one: f64 = lua.load("return s:GetStringHeight()").eval().expect("runs");
        lua.load(r#"s:SetText("a very much longer sentence indeed, with no width to fold at")"#)
            .exec()
            .expect("loads");
        let long: f64 = lua.load("return s:GetStringHeight()").eval().expect("runs");
        assert_eq!(one, long);
    }

    /// **`SetTexture` is a path *or* a colour**, told apart by the argument's
    /// type. Every backdrop in the directory uses the second form, and a host
    /// that only understood the first would leave them blank.
    #[test]
    fn set_texture_takes_a_path_or_a_solid_colour() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            t = f:CreateTexture();
            t:SetTexture("Interface\\Buttons\\UI-QuickslotRed");
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(
            eval(&lua, "return t:GetTexture()"),
            r#"String("Interface\\Buttons\\UI-QuickslotRed")"#
        );
        lua.load("t:SetTexture(0, 0, 0, 0.5)").exec().expect("runs");
        assert_eq!(
            eval(&lua, "return t:GetTexture()"),
            "Nil",
            "a colour is not a path, and the old path does not linger"
        );
        lua.load("t:SetTexture(nil)").exec().expect("runs");
        assert_eq!(eval(&lua, "return t:GetTexture()"), "Nil");
    }

    /// **`SetText` is given numbers constantly** — `count:SetText(charges)` —
    /// and 1.12 shows "3" rather than "3.0". Getting that wrong puts a decimal
    /// point on every stack count in the game.
    #[test]
    fn set_text_stringifies_the_way_the_game_does() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            s = f:CreateFontString("Count");
            "#,
        )
        .exec()
        .expect("loads");
        lua.load("s:SetText(3)").exec().expect("runs");
        assert_eq!(eval(&lua, "return s:GetText()"), r#"String("3")"#);
        lua.load(r#"s:SetText("Fireball")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return s:GetText()"), r#"String("Fireball")"#);
        lua.load("s:SetText(nil)").exec().expect("runs");
        assert_eq!(eval(&lua, "return s:GetText()"), "Nil");
    }

    /// Every name [`METHODS`] claims is really installed — the rule every
    /// claimed list in this directory follows, for the reason
    /// [`super::widget`]'s copy of this test gives.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = lua();
        lua.load(r#"probe = CreateFrame("Frame"):CreateTexture();"#)
            .exec()
            .expect("loads");
        for name in METHODS.iter().chain(widget::METHODS.iter()) {
            assert_eq!(
                eval(&lua, &format!("return type(probe.{name})")),
                r#"String("function")"#,
                "{name} is claimed and is not installed on a region"
            );
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }

    /// **A font string with a width of its own is as tall as it folds** — the
    /// spellbook's own declaration, and the bug it fixes.
    ///
    /// `$parentSpellName` is `<AbsDimension x="103" y="0"/>` with
    /// `maxLines="3"`, so a long name wraps and the rank line under it moves
    /// down with it. Before this the height was one line box whatever the
    /// string, so "Rallying Cry of the Dragonslayer" ran out through the side
    /// of the page and the row below it did not move.
    #[test]
    fn a_font_string_with_a_declared_width_is_as_tall_as_it_folds() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            short = f:CreateFontString();
            short:SetWidth(103);
            short:SetText("Attack");
            long = f:CreateFontString();
            long:SetWidth(103);
            long:SetText("Rallying Cry of the Dragonslayer");
            bare = f:CreateFontString();
            bare:SetText("Rallying Cry of the Dragonslayer");
            "#,
        )
        .exec()
        .expect("loads");
        let globals = lua.globals();
        let height = |name: &str| {
            let region: mlua::Table = globals.get(name).expect("the region");
            intrinsic(&lua, &region).expect("a font string with text").1
        };
        let line = height("short");
        assert!(line > 0.0);
        assert_eq!(height("long"), 2.0 * line, "two rows at 103 units");
        // …and a string that declared no width does not fold at all, which is
        // what keeps every ordinary label in the interface one line high.
        assert_eq!(height("bare"), line);

        // The painter is told the same thing from the same place, so the box
        // and the glyphs cannot disagree about whether there is a fold.
        let region: mlua::Table = globals.get("long").expect("the region");
        let folded = paint(&region).expect("a region");
        assert!(folded.wrap, "the painter must fold what the layout made room for");
        let bare: mlua::Table = globals.get("bare").expect("the region");
        assert!(!paint(&bare).expect("a region").wrap);
    }

    /// **A string pinned on both sides folds too**, though it declared no width.
    ///
    /// `SetWidth` is not how interface code writes a paragraph — two anchors
    /// are:
    ///
    /// ```lua
    /// f.text:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10)
    /// f.text:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -10, 10)
    /// ```
    ///
    /// The fold width is the *solved* rectangle's, which only the painter knows,
    /// so all this side owes is the decision — see [`pinned_across`]. Without it
    /// the fold is `INFINITY` and the sentence runs out through both walls of
    /// the box it was measured into, which is every "text clipping out of the
    /// frame" report and pfUI's first-run wizard in particular.
    #[test]
    fn a_string_pinned_left_and_right_folds_at_its_solved_width() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            f:SetWidth(200); f:SetHeight(100);
            para = f:CreateFontString();
            para:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10);
            para:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -10, 10);
            para:SetText("a sentence long enough to need folding");

            oneSided = f:CreateFontString();
            oneSided:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10);
            oneSided:SetText("a sentence long enough to need folding");

            filled = f:CreateFontString();
            filled:SetAllPoints(f);
            filled:SetText("a sentence long enough to need folding");
            "#,
        )
        .exec()
        .expect("the chunk runs");
        let globals = lua.globals();
        let wraps = |name: &str| {
            let region: mlua::Table = globals.get(name).expect("the region");
            paint(&region).expect("a region").wrap
        };
        assert!(wraps("para"), "two anchors give it a width to fold at");
        assert!(wraps("filled"), "and a fill is both corners at once");
        // **One anchor is not a width.** A label hung off a single point is as
        // wide as its own text, so folding it there would break every ordinary
        // caption in the interface at its first space.
        assert!(!wraps("oneSided"));
    }

    /// …and `maxLines` caps it, on both sides of the same rule: the height
    /// stops growing and the painter is told to stop drawing. A cap that
    /// reached only one of the two would either leave a gap or overprint the
    /// row beneath.
    #[test]
    fn max_lines_caps_the_fold_in_the_height_and_in_the_paint() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            capped = f:CreateFontString();
            capped:SetWidth(30);
            capped:SetText("one two three four five six seven eight");
            "#,
        )
        .exec()
        .expect("loads");
        let region: mlua::Table = lua.globals().get("capped").expect("the region");
        let uncapped = intrinsic(&lua, &region).expect("has text").1;
        super::super::messages::set_from_markup(&region, "maxLines", "2").expect("sets");
        let capped = intrinsic(&lua, &region).expect("has text").1;
        assert!(capped < uncapped, "{capped} should be under {uncapped}");
        assert_eq!(paint(&region).expect("a region").max_rows, 2);
    }
}
