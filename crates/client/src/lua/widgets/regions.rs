//! The two region kinds that draw: `Texture` and `FontString`.
//!
//! A frame is a container: it holds events, children and a rectangle, and it
//! paints nothing. Everything visible in the 1.12 interface is one of these two
//! kinds. The ninety XML files declare 1,652 of them against 1,812 frames, so
//! regions are about half of the object model.
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
//! `<NormalTexture>` inside a `<Button>` makes a `Texture` and also fills the
//! button's normal-texture slot, so both `button:GetNormalTexture()` and
//! `getglobal(name)` find it. Twelve of the thirteen texture element names work
//! this way. The slot table is in [`vale_assets::interface::widgets::Region`],
//! because the loader and `vale framexml` both read it.
//!
//! ## What is recorded, and who reads it
//!
//! Nothing in this module draws: `SetTexture` records a path, `SetText` records
//! a string, `SetVertexColor` records four numbers. [`crate::ui::framexml`]
//! walks the tree, asks [`super::layout`] where each region lands, and paints
//! it. [`Paint`] holds everything a region contributes to a frame, read once
//! instead of eight table lookups scattered through the draw loop.
//!
//! Every write of a field the draw walk reads goes through
//! [`widget::set_paint`] or is followed by [`widget::mark_paint`], so the walk
//! can be skipped when nothing it reads has changed.
//!
//! ## `alphaMode` is a blend mode
//!
//! `<Texture alphaMode="ADD">` is on the cast bar's spark and flash. 136
//! elements in the directory carry the attribute, and the values are `ADD`,
//! `BLEND`, `ALPHAKEY`, `MOD` and `DISABLE`: the same set the M2 blend table
//! uses, because both select the same fixed-function blend state. The value is
//! recorded verbatim rather than mapped to anything here.

use super::widget;

/// The methods a region carries beyond [`super::widget::METHODS`], sorted.
///
/// In the game the methods are split between the two kinds: a `Texture`
/// answers about pixels and a `FontString` about text, and a method on the
/// wrong one is a nil call. Here they are one table because a region's kind is
/// a field rather than a type; splitting them would mean cutting this list in
/// two.
///
/// The four measurements are on the frame table too; see
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

/// The font object a string was last given by `SetFontObject`, so
/// `GetFontObject` can return it. The face itself is copied onto the string's
/// own keys at the call; see [`apply_font_style`].
const FONT_OBJECT_KEY: &str = "__fontObject";
/// A font object's own name, for `GetName`.
const FONT_NAME_KEY: &str = "__fontName";
/// The registry slot holding the font objects' method table.
const REG_FONT_METHODS: &str = "vale.fontMethods";

const TEXTURE_KEY: &str = "__texture";
/// `SetPortraitTexture`'s unit token: the one thing a `Texture` can hold that
/// is not a file.
///
/// It has its own key rather than a special path in [`TEXTURE_KEY`], because
/// the two are drawn by different code in different directories: a path is
/// decoded by `ui::framexml`'s BLP cache, and a portrait is rendered by
/// `render::portraits` with a camera. A sentinel string in the texture key
/// would require every reader of that key (the loader, `GetTexture`, the art
/// cache) to check for it.
const PORTRAIT_KEY: &str = "__portraitUnit";

/// How far this texture has been rotated, in radians clockwise on the screen.
/// [`set_rotation`] is its only writer.
const ROTATION_KEY: &str = "__rotation";
const TEXT_KEY: &str = "__text";
/// A frame's own font string region, which is separate from its text.
///
/// `<ButtonText>` is a slot, so the loader stores the region it made on the
/// parent. When it was stored under [`TEXT_KEY`], where a region keeps its
/// string, the key had two meanings depending on the kind of object:
/// `GetTextWidth` on a button read the font string and tried to measure a
/// table (60 `OnLoad`s, every `MoneyFrame` in the game);
/// `PanelTemplates_SetDisabledTabState`'s `tab:GetText()` read a string where
/// the forwarding getter wanted the region; and a `<Button text="CANCEL">`
/// whose template also gave it a `<ButtonText>` had its label overwritten by
/// the region that was loaded after it.
///
/// The region therefore has its own key. [`slot_key`] puts the region here
/// rather than under the string's key.
pub(in crate::lua) const TEXT_REGION_KEY: &str = "__textRegion";
const LAYER_KEY: &str = "__layer";
const BLEND_KEY: &str = "__blend";
const COLOUR_KEY: &str = "__colour";
/// A `Texture`'s vertex colour, set by `SetVertexColor` and kept apart from
/// its own colour (`<Color>` or `SetTexture(r, g, b, a)`). The client
/// multiplies the two, so `SkillFrame`'s bar background, a white `<Color>` at
/// alpha 0.2 tinted with `SetVertexColor(0, 0, 0.75, 0.5)`, draws at alpha
/// 0.1. When the vertex colour replaced the texture's colour, that background
/// drew at alpha 0.5 across the whole bar and every skill bar looked full.
const VERTEX_COLOUR_KEY: &str = "__vertexColour";
/// Whether a script has set this string's colour with `SetTextColor`. While it
/// is set, applying a font object leaves the colour alone, as in 1.12. See
/// [`apply_font_style`], the reason it exists.
const COLOUR_SET_KEY: &str = "__colourSet";
const JUSTIFY_H_KEY: &str = "__justifyH";
const JUSTIFY_V_KEY: &str = "__justifyV";
/// `<Shadow>`; see [`set_shadow`]. `MasterFont` declares one, so nearly every
/// string in the interface carries one.
const SHADOW_OFFSET_KEY: &str = "__shadowOffset";
const SHADOW_COLOUR_KEY: &str = "__shadowColour";
/// `outline="NORMAL"` / `"THICK"`; the measurement is at [`Paint::outline`].
/// It is an attribute on the `<Font>` rather than a child element, so it
/// arrives beside `font` rather than beside `<Shadow>`.
const OUTLINE_KEY: &str = "__outline";
/// A font string whose text folds at its rectangle's width instead of running
/// past it. Written only by the tooltip's wrap arguments (see
/// [`super::tooltip`]) and read by the painter, which wraps the glyphs; this
/// module only estimates where the lines fold.
const WRAP_KEY: &str = "__wrap";
const COORDS_KEY: &str = "__texCoords";
/// The typeface and size a font string draws in: `font="Fonts\FRIZQT__.TTF"`
/// and `<FontHeight><AbsValue val="12"/>`, which reach a `<FontString>` through
/// `inherits="GameFontNormal"` rather than being written on it.
const FONT_KEY: &str = "__font";
const FONT_HEIGHT_KEY: &str = "__fontHeight";

/// The keys a `<Font>` object writes onto a string, as a set, so that a face
/// can be copied from one region and applied whole to another.
///
/// This exists for [`super::button`]'s three fonts: a button keeps three font
/// objects and picks between them per state, while this client keeps a
/// region's face on the region. Each declaration is captured as a snapshot and
/// the one the state chooses is applied again.
///
/// `justifyH` and `justifyV` are not in the set. They are declared on the
/// `<ButtonText>` far more often than on the `<Font>` (`$parentName` in
/// `CharSelectCharacterButtonTemplate` is `justifyH="LEFT"` with no font of its
/// own), so carrying them through a state change would re-centre a label the
/// markup left-aligned.
const FONT_STYLE_KEYS: [&str; 6] = [
    FONT_KEY,
    FONT_HEIGHT_KEY,
    COLOUR_KEY,
    SHADOW_OFFSET_KEY,
    SHADOW_COLOUR_KEY,
    OUTLINE_KEY,
];

/// Capture a region's face; see [`FONT_STYLE_KEYS`].
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

/// Apply a captured face to a region. A key the snapshot does not carry is
/// cleared, which keeps the three states independent: `GameFontDisable`
/// declares only a `<Color>` and inherits the rest, so a partial write would
/// leave the previous state's colour on a font that declares none.
///
/// The colour is the exception once a script has set one. In 1.12, a colour
/// set with `SetTextColor` survives a later `SetFontObject`: applying a font
/// object does not change a colour the script set.
///
/// This client keeps the script's colour and the font's colour in the same
/// key, so [`COLOUR_SET_KEY`] records that a script set it.
///
/// `SetTextColor` on a button does not set [`COLOUR_SET_KEY`]. It colours the
/// normal face only, and the highlight and disabled faces keep their font's
/// colour, so a hovered quest log title turns white; see
/// [`super::button::set_text_colour`].
pub(in crate::lua) fn apply_font_style(
    lua: &mlua::Lua,
    region: &mlua::Table,
    style: &mlua::Table,
) -> mlua::Result<()> {
    let keeps_its_colour = region
        .raw_get::<Option<bool>>(COLOUR_SET_KEY)
        .unwrap_or(None)
        .unwrap_or(false);
    for key in FONT_STYLE_KEYS {
        if keeps_its_colour && key == COLOUR_KEY {
            continue;
        }
        if key == FONT_KEY || key == FONT_HEIGHT_KEY {
            store_face(lua, region, key, style.raw_get::<mlua::Value>(key)?)?;
        } else {
            widget::set_paint(lua, region, key, style.raw_get::<mlua::Value>(key)?)?;
        }
    }
    Ok(())
}

/// A paint write for a face key, which also invalidates the layout when it changes a string's intrinsic size.
fn store_face(
    lua: &mlua::Lua,
    object: &mlua::Table,
    key: &str,
    value: impl mlua::IntoLua,
) -> mlua::Result<()> {
    let before = widget::paint_generation(lua);
    widget::set_paint(lua, object, key, value)?;
    if widget::paint_generation(lua) != before {
        super::layout::invalidate(lua)?;
    }
    Ok(())
}

/// Write a list of numbers, replacing the stored table and bumping paint only when the numbers differ.
///
/// A list that differs from the held one only by a trailing alpha of 1 is
/// drawn the same (a three-component colour is read as opaque), so it is
/// stored as given but does not bump: the party portraits are coloured every
/// tick by two setters, one passing three components and one four.
fn store_numbers(lua: &mlua::Lua, object: &mlua::Table, key: &str, values: &[f64]) -> mlua::Result<()> {
    let held: Option<Vec<f64>> = object.raw_get(key).unwrap_or(None);
    if held.as_deref() == Some(values) {
        return Ok(());
    }
    let drawn_same = held.as_deref().is_some_and(|held| same_up_to_opaque_alpha(held, values));
    object.raw_set(key, values.to_vec())?;
    if !drawn_same {
        widget::mark_paint_on(lua, object);
    }
    Ok(())
}

/// Whether two number lists differ only by one having a fourth element of 1.0
/// that the other lacks.
fn same_up_to_opaque_alpha(a: &[f64], b: &[f64]) -> bool {
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    short.len() == 3 && long.len() == 4 && long[..3] == *short && long[3] == 1.0
}

/// The width `text` draws at in the face `style` carries, in the game's own
/// units.
///
/// `GetTextWidth`, the tooltip's auto-size, the caret and a message frame's
/// fold all measure through this one function, so they cannot disagree about
/// where a word ends. It measures with the game's own `.TTF`s rather than a
/// ratio; the measurement and its one stated fallback are in
/// [`super::text::width`].
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

/// How many rows `text` folds into at width `fold`, in this region's own face.
///
/// The row counterpart of [`text_width`], and the single measurement for the
/// same reason: the height a caller reserves and the galley the painter draws
/// must fold at the same places, in the same face, at the same size.
/// [`super::text::rows`] is the measurement (word by word, with a first word
/// too long for the row broken rather than moved to the next row), and the
/// painter folds by it too.
///
/// The tooltip previously estimated `ceil(measured / fold)`, which is the row
/// count only when the words land exactly on the fold. Real text leaves part
/// of a row empty at every break, so the estimate is one short once a sentence
/// is three rows long: Shield Bash's description reserved three rows and drew
/// four, putting its last line below the bottom of the tooltip.
pub(super) fn text_rows(lua: &mlua::Lua, style: &mlua::Table, text: &str, fold: f64) -> usize {
    super::text::rows(
        lua,
        font_of(style).as_deref(),
        font_height_of(style),
        text,
        fold,
    )
}

/// How tall the string is once it has folded: the answer `GetTextHeight` and
/// `GetStringHeight` give.
///
/// One line box for a string that does not fold, and one per row for one that
/// does, at the same fold width and in the same face [`Paint::wrap`] tells the
/// painter to use, so a caller that sizes a widget from this reserves the rows
/// that are drawn.
///
/// A single line box for every string breaks the gossip menu.
/// `GossipFrame.lua`'s `GossipResize` is
/// `titleButton:SetHeight(titleButton:GetTextHeight() + 2)`, and each option
/// button is anchored `TOPLEFT` to the previous one's `BOTTOMLEFT`, so a
/// button's height is the next button's offset. `GossipTitleButtonTemplate`
/// declares its `<ButtonText>` 275 wide, so a long option folds to two rows;
/// sized for one, the second row of each option is painted under the first
/// row of the next.
///
/// Empty text is one line box rather than nothing, because a caller adding
/// padding to it expects that, and earlier versions answered it.
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

/// The size of a `FontString` that was never given one: the size of the
/// string it holds, in its own face.
///
/// `None` for anything that is not a font string, and for one holding no text.
///
/// Most of the interface's text depends on this rule. A `FontString` is the
/// one region kind the directory routinely declares with one anchor and no
/// `<Size>`: `CharacterStatFrame1Label` is
/// `<FontString inherits="GameFontNormalSmall"><Anchors><Anchor point="LEFT"/>`
/// and nothing else, because in the 1.12.1 client a string's own extent is its
/// size, and `GetWidth` on one answers `GetStringWidth`. Without the rule the
/// two axes come from `SetWidth`/`SetHeight`, which nothing wrote, so the
/// rectangle is 0x0 and [`super::draw`] drops it before it is painted. On a
/// screen with the character sheet open that was 38 of the 41 strings holding
/// text: every stat label and every stat value, the level line, the tab
/// captions.
///
/// The rule applies per axis, and only where the object declared no size,
/// because the two axes are declared independently: `<AbsDimension x="120"
/// y="0"/>` is a string with a width and an intrinsic height.
///
/// A string with a width of its own is as tall as its text folds to.
/// `<FontString name="$parentSpellName" maxLines="3">` with
/// `<AbsDimension x="103" y="0"/>` is the spellbook's own declaration: a width
/// and no height, which in the 1.12.1 client means "wrap at 103 and be as tall
/// as that takes, up to three rows". Without the fold, "Rallying Cry of the
/// Dragonslayer" ran out through the right edge of the page and over the tab
/// strip, where the game wraps it onto a second line, and the row below it
/// stayed where a one-line name would have put it.
///
/// The row count is [`super::text::rows`], measured in the same face at the
/// same height the painter lays out in, and the painter is told to fold at the
/// same rectangle ([`Paint::wrap`]), so a string that reserves two rows draws
/// in two. `maxLines` caps it where the file states one; where it does not,
/// the string is as tall as it needs, as in the 1.12.1 client for the wrapping
/// labels that declare no cap.
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

/// What `GetWidth` and `GetHeight` answer for a `FontString`: the size of its
/// own string, when nothing gave it an explicit size.
///
/// `None` for anything that is not a font string, and for one that was given
/// a size. Both fall through to the solved rectangle, which is what every
/// frame in the tree answers.
///
/// ## Why a font string's size is not its rectangle
///
/// A region the files gave no anchors fills its parent (see the loader's
/// note), which is correct for the art and wrong for a label: `<ButtonText>`,
/// `QuestLogDummyText` and every tab's text are declared with no `<Anchors>`,
/// so a `GetWidth` read from the rectangle answers the container's width for
/// all of them.
///
/// Of the 120 `GetWidth`/`GetHeight` call sites in `Interface\FrameXML\`,
/// every one whose receiver is a `FontString` expects the string's size:
///
/// ```text
/// QuestLogFrame.lua:163   QuestLogDummyText:SetText(...)   file comment: "*SUPER HACK*"
/// QuestLogFrame.lua:199   QuestLogDummyText:GetWidth()     measures the text set at :163
/// QuestLogFrame.lua:623   watchText:GetWidth()             sizes QuestWatchFrame
/// UIPanelTemplates.lua:58 tabText:GetWidth() + padding     sizes a tab
/// UIPanelTemplates.lua:71 tabText:SetWidth(0)              0 unsets the width
/// HelpFrame.lua:307       this:SetWidth(…Text:GetWidth()+40)   a <ButtonText>
/// GameTooltip.xml:63      SmallTextTooltipText:GetWidth()+20
/// UIDropDownMenu.lua:190  normalText:GetWidth()
/// PlayerFrame.lua:228     PlayerFrameGroupIndicatorText:GetWidth()
/// TutorialFrame.lua:28    TutorialFrameText:GetHeight()
/// ```
///
/// `HelpFrame.lua:307` decides the `<ButtonText>` case, the only one where the
/// two answers could reasonably differ: it sizes a button to its own label, so
/// with the rectangle answer the button's width is read back and the button
/// grows by 40 every time the line runs.
///
/// `UIPanelTemplates.lua:71` shows that `SetWidth(0)` means "unset", not
/// "zero wide", which is how [`super::layout`]'s solve already reads it.
///
/// ## What this does not change
///
/// The rectangle. A `<ButtonText>` still fills its button and is still drawn
/// centred in it: the painter reads [`super::layout::rect`] directly and never
/// this. Only the answer to `GetWidth` and `GetHeight` changes.
pub(super) fn reported_size(lua: &mlua::Lua, object: &mlua::Table, want_width: bool) -> Option<f64> {
    if !is_kind(object, FONT_STRING) {
        return None;
    }
    let key = if want_width { widget::WIDTH_KEY } else { widget::HEIGHT_KEY };
    // A declared size wins, and 0 is not a declared size. That is what
    // `PanelTemplates_TabResize`'s `tabText:SetWidth(0)` means, and it is the
    // same reading [`super::layout`]'s solve makes of it.
    //
    // Answered here rather than left to the rectangle, so a `FontString` never
    // consults its anchors for this: `SetAllPoints` would otherwise override a
    // declared width, and the tab whose width had just been capped would read
    // back its container's. The cost is that a font string sized by two
    // opposite anchors reports its string rather than the span; nothing in
    // `Interface\FrameXML\` reads one, and every existing reader is measuring
    // text.
    if let Some(set) = object.raw_get::<Option<f64>>(key).ok().flatten().filter(|set| *set > 0.0) {
        return Some(set);
    }
    // An empty string answers 0, not the container's size, which is also what
    // an unset `FontString` reported before this function existed.
    let Some((width, height)) = intrinsic(lua, object) else {
        return Some(0.0);
    };
    Some(if want_width { width } else { height })
}

/// The width at which this region's text folds, or `None` if it does not fold.
///
/// It is the declared width and nothing else. It is not:
///
/// * the solved rectangle, which for a string with no width of its own is the
///   width of the string. Every label in the interface would then fold at
///   exactly its own length, a fold that never happens but costs a measurement
///   on every label;
/// * applied to a `<Texture>`, which has a width for other reasons.
///
/// [`WRAP_KEY`] is the second source, used by the tooltip: that widget folds
/// lines at a width it computed itself rather than declared.
/// `is_font` is passed in rather than read, because both callers have already
/// checked the region's kind and [`paint`] runs once per drawn object per
/// frame; see [`is_kind`] on the cost of that read.
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
        // An explicit `false` means "never fold", not "use the rule below";
        // the tooltip's layout depends on it.
        //
        // `reflow` writes each cell's own measured width back as a declared
        // width, so the rule below would fold every tooltip line at exactly
        // the width it had just been measured at. The line then folds whenever
        // the painter and the measurement disagree by a fraction of a pixel,
        // which they do: `Face::width` is a plain sum of advances and the
        // painter adds the face's kerning. A unit's name that folded put its
        // second row on top of "Level 23 Humanoid", because the tooltip had
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

/// Whether a string is anchored on both its left and right sides, in which
/// case it folds at the width its anchors give it, although `SetWidth` never
/// set one.
///
/// [`wrap_width`] answers the width, and it can only answer one a region
/// declared. A `<FontString>` anchored `TOPLEFT` and `BOTTOMRIGHT` to its
/// parent has no declared width and a valid solved one, and interface code
/// writes a paragraph that way:
///
/// ```lua
/// f.text:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10)
/// f.text:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -10, 10)
/// ```
///
/// Without this the fold is `INFINITY` and the sentence runs out through both
/// sides of the box it was measured into. pfUI's first-run wizard showed this,
/// and so does every report of text running out of frames.
///
/// The painter already folds at the rectangle it is handed
/// ([`crate::ui::mesh::text`]), so this function only makes the decision.
/// A `SetAllPoints` counts, because it sets both corners at once.
///
/// It costs one table read and a walk of two or three anchors per drawn font
/// string. The height estimate is not changed: a string anchored on both
/// sides also takes its height from its anchors, so nothing reads the
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

/// The face a region draws in. With [`font_height_of`], the size it draws at,
/// these are the two values [`text_width`] resolves inline, split out because
/// [`intrinsic`] needs them to call [`super::text::rows`] in the same face.
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

/// The height of one line of text in this style: the face's own line box, not
/// the declared height. The two differ and neither is always the larger: Friz
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

/// The four methods that measure a string, installed on both method tables. A
/// region measures its own string and a frame the one in its `<ButtonText>`;
/// the same body answers both because [`text_of`] and [`text_region`] already
/// handle the two shapes.
///
/// Each measurement has two names because the directory uses both: it calls
/// `GetTextWidth` on a frame and `GetStringWidth` on a font string, in the
/// same file. The measurements use the game's own typefaces.
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

/// The five draw layers, back to front, by the game's own names. The layer is
/// the innermost sort key the interface is drawn in.
///
/// `<Layer level="ARTWORK">`. Defined here rather than in the draw pass
/// because `GetDrawLayer` answers these names, and this list defines their
/// order.
pub const LAYERS: [&str; 5] = ["BACKGROUND", "BORDER", "ARTWORK", "OVERLAY", "HIGHLIGHT"];

/// The three values 1.12's `outline=` takes.
///
/// There are two outline widths, not one, because the directory uses both and
/// they look different: `GlueFontNormal` and its siblings say `NORMAL`, and
/// the `NumberFont*` family (the damage numbers, an action button's count, the
/// bag totals) say `THICK`, which keeps a white number legible over any icon
/// in the game.
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
    /// One and two. The pair is inferred from the two names: the widths the
    /// 1.12.1 client draws them at, in texels, are not established here. What
    /// is established is that both exist and that neither is zero: a
    /// screenshot of the 1.12.1 client's glue text shows a hard black edge
    /// about a pixel wide at the size `GlueFontNormal` draws at.
    pub fn radius(self) -> f32 {
        match self {
            Outline::None => 0.0,
            Outline::Normal => 1.0,
            Outline::Thick => 2.0,
        }
    }
}

/// Everything a region contributes to a frame, read in one pass.
///
/// A struct rather than eight calls from the draw loop, because the loop runs
/// over every visible region every frame and each Lua table lookup from Rust
/// has a cost. It is also the one list of what this client records about a
/// region: when something is drawn wrong, this shows whether the field was
/// read at all.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paint {
    /// `Interface\Buttons\UI-QuickslotRed`, with no extension, as the files
    /// write it.
    pub texture: Option<String>,
    /// The text of a `FontString`, already stringified the way 1.12 does it.
    pub text: Option<String>,
    /// For a texture, its own colour (`<Color>` or `SetTexture(r, g, b, a)`)
    /// multiplied by its vertex colour (`SetVertexColor`); for a font string,
    /// its text colour. `[1, 1, 1, 1]` when nothing set one, which for a
    /// texture is "as painted".
    pub colour: [f32; 4],
    /// `ADD`, `BLEND`, `ALPHAKEY`, `MOD`, `DISABLE` — one of [`BLENDS`], and a
    /// `&'static str` for the reason [`FONT_STRING`] gives.
    pub blend: &'static str,
    /// `BACKGROUND`..`HIGHLIGHT`, as an index into [`LAYERS`].
    pub layer: usize,
    /// `SetTexCoord(left, right, top, bottom)`, when one was set.
    pub coords: Option<[f32; 4]>,
    /// `SetTexCoord`'s eight-argument form, which is four `(u, v)` corners
    /// rather than a rectangle: `SetTexCoord(ULx, ULy, LLx, LLy, URx, URy, LRx, LRy)`.
    ///
    /// One function in the two shipped directories uses it, and depends on it:
    /// `DrawRouteLine`, the flight map's line drawer in `TaxiFrame.lua`
    /// (written by Daniel Stephens and included unchanged), rotates a
    /// horizontal line texture into place using only these eight numbers and a
    /// bounding box. Drawn as an axis-aligned quad instead, every flight path
    /// on the map is a rectangle.
    pub corners: Option<[f32; 8]>,
    /// The typeface file, and the height in the game's own units.
    pub font: Option<String>,
    pub font_height: f32,
    /// `LEFT` | `CENTER` | `RIGHT`, and `TOP` | `MIDDLE` | `BOTTOM`.
    pub justify_h: &'static str,
    pub justify_v: &'static str,
    /// `<Shadow>`: the offset of the copy drawn under the glyphs, and its
    /// colour. `None` for a face that declares none, which is uncommon:
    /// `MasterFont` declares one and `GameFontNormal` inherits it.
    pub shadow: Option<([f32; 2], [f32; 4])>,
    /// `outline="NORMAL"`: the black edge round every glyph, which makes the
    /// game's text read bold rather than thin.
    ///
    /// An attribute on the `<Font>` and not a child element, so it arrives the
    /// way `font=` does: through `inherits`, from a face declared once. It is on
    /// nearly all text on the screens before the world:
    /// `GlueFontNormal`, `…Small`, `…Large` and `…Huge` all declare it, and the
    /// account and password labels, the Login button, the three link buttons and
    /// the version line all inherit one of the four. The number fonts in
    /// `Interface\FrameXML\` declare `THICK`. Drawn without it, the same text is
    /// a thin gold line over a busy background, where the 1.12.1 client draws
    /// it with a hard black edge.
    pub outline: Outline,
    /// Fold the text at the rectangle's width. [`wrap_width`] is the one place
    /// that decides, so the height [`intrinsic`] reserves and the galley the
    /// painter builds cannot disagree about whether there is a fold.
    pub wrap: bool,
    /// How many rows the text may fold into: `<FontString maxLines="3">`, or 0
    /// for "as many as it takes".
    ///
    /// Passed to the painter as well as used in the height, because the two
    /// are computed separately: without it here a four-row name would reserve
    /// three rows and draw four, overprinting whatever is beneath it.
    pub max_rows: usize,
    /// Whether this region is a `FontString`, which decides what an empty one
    /// means. A `Texture` with a colour and no path is a solid fill
    /// (`SetTexture(0, 0, 0, 0.5)` is how the directory writes one); a
    /// `FontString` paints its text or nothing. When the two shared the fill
    /// rule, every empty `MessageFrame`'s font declaration (`UIErrorsFrame`,
    /// `RaidWarningFrame`, `RaidBossEmoteFrame`) drew as a solid gold 512-unit
    /// bar across the middle of the screen.
    pub is_font: bool,
    /// The unit this region shows a portrait of, when `SetPortraitTexture` has
    /// been called on it; see [`PORTRAIT_KEY`].
    ///
    /// A token (`"player"`, `"target"`) rather than a guid, resolved by the
    /// pass that renders the portrait rather than here. 1.12 resolves it at the
    /// call and leaves a still image; this deviation is stated in
    /// `crate::render::portraits`. The interface calls this again on every
    /// `UNIT_PORTRAIT_UPDATE` and every target change, so the two agree in
    /// every case a session reaches, and the live form cannot show an
    /// outdated face.
    ///
    /// It does not replace [`Self::texture`]: `TargetPortrait` is a plain
    /// `<Texture>` with art in the XML and gets its portrait at run time, and
    /// the path is the fallback when a portrait cannot be rendered.
    pub portrait: Option<String>,
    /// Radians this texture has been rotated by, clockwise on the screen; 0
    /// throughout both shipped directories. See [`set_rotation`], whose one
    /// caller is the world map's player arrow.
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
    let rgba = |key: &str| {
        floats(object, key).map_or([1.0; 4], |(values, count)| match count {
            // Three components is the shape of `<Color r g b/>`, which appears
            // 73 times, and it means opaque.
            3 => [values[0], values[1], values[2], 1.0],
            4 => values,
            _ => [1.0; 4],
        })
    };
    let mut colour = rgba(COLOUR_KEY);
    // A texture's vertex colour multiplies its own; see [`VERTEX_COLOUR_KEY`].
    if !is_font {
        let vertex = rgba(VERTEX_COLOUR_KEY);
        for (channel, tint) in colour.iter_mut().zip(vertex) {
            *channel *= tint;
        }
    }
    // A region reads only the fields its own kind can carry, the rule the
    // `shadow` line below already follows, applied to the other ten. This
    // function runs once per drawn object per frame (376 of them with the bags
    // open, 102 without), and every field is an `mlua` call: a Lua string
    // push, a hash and a table lookup each. Without the split, half of them
    // would read fields the object's kind cannot have.
    //
    // The split follows what the two widget kinds are, not what is usually
    // set. `SetFont`, `SetJustifyH`, `SetJustifyV`, `maxLines`, the wrap and
    // `<Shadow>` belong to `FontString`, and `SetTexture`, `SetTexCoord` and
    // `alphaMode` belong to `Texture`. The painter matches this: a font string
    // draws its text or nothing (`is_font` returns before the texture is
    // read), and a texture never reaches `label`.
    //
    // The rotation is on the texture side of that split and is a twelfth
    // read: one `raw_get` per drawn texture per walk, which at 300 textures on
    // a 30 Hz clock is under a millisecond per second. It is read here rather
    // than resolved by name in the painter because it is a property of the
    // region: one object sets it (the world map's player arrow), and any
    // other object that sets it is drawn rotated with no further change.
    let (texture, portrait, coords, corners, blend, rotation) = if is_font {
        (None, None, None, None, "BLEND", 0.0)
    } else {
        // The two forms are distinguished by the count and never mixed. Four
        // numbers is a rectangle; eight is the four corners of a quad that may
        // be rotated, which is how the flight map draws a route (see
        // [`Paint::corners`]). Any other count is neither.
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
        // The text is read on both paths, unlike the six fields above, because
        // `worth_drawing` reads it for a `Texture` too: a coloured region with
        // no text is a solid fill and a textured one is a picture. Telling
        // those apart keeps every empty `MessageFrame` from drawing as a gold
        // bar.
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

/// Rotate a texture about its own centre, in radians clockwise on the screen.
///
/// A Rust write, not a Lua method: nothing in either directory rotates a
/// `<Texture>` (1.12's `SetRotation` belongs to `<Model>`), and the one object
/// here that needs rotating is a frame this client builds itself, the world
/// map's player arrow. The arrow is in [`crate::lua::panels::worldmap`]; the
/// 1.12 arrow is an `.mdx` model rather than a quad.
pub(in crate::lua) fn set_rotation(lua: &mlua::Lua, region: &mlua::Table, radians: f32) -> mlua::Result<()> {
    if radians == 0.0 {
        // Cleared rather than stored, so the painter's `!= 0.0` test remains
        // the only check before it builds a rotated quad.
        return widget::set_paint(lua, region, ROTATION_KEY, mlua::Value::Nil);
    }
    widget::set_paint(lua, region, ROTATION_KEY, f64::from(radians))
}

/// The two region kinds, as byte strings so the comparison costs no allocation.
///
/// `paint` runs once per drawn object per frame (376 of them with the bags
/// open), and `raw_get::<String>` on a key is a Lua string push, a hash and a
/// heap allocation for a word that is one of two. Reading it as an
/// [`mlua::String`] and comparing bytes avoids the allocation.
const FONT_STRING: &[u8] = b"FontString";
const TEXTURE: &[u8] = b"Texture";

/// Whether this object is of the given drawing kind. [`intrinsic`] and
/// [`paint`] both check it, in the way described above.
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

/// One word of a fixed vocabulary, or the default, with no allocation either
/// way.
///
/// A word the vocabulary does not have falls back to the default rather than
/// being carried through. The layer read below does the same, and the
/// painter's own `match` arms would treat it as the default anyway.
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

/// The index of a word in a fixed vocabulary; a draw layer is stored this way.
fn word_index<const N: usize>(
    object: &mlua::Table,
    key: &str,
    vocabulary: [&'static str; N],
) -> Option<usize> {
    let found = object.raw_get::<Option<mlua::String>>(key).ok().flatten()?;
    vocabulary.into_iter().position(|known| found == known.as_bytes())
}

/// The key `SetVertexColor` writes: [`VERTEX_COLOUR_KEY`] on a `Texture`,
/// [`COLOUR_KEY`] on anything else.
fn vertex_key(object: &mlua::Table) -> &'static str {
    match object.raw_get::<Option<mlua::String>>(widget::KIND_KEY) {
        Ok(Some(kind)) if kind == TEXTURE => VERTEX_COLOUR_KEY,
        _ => COLOUR_KEY,
    }
}

/// A short run of numbers out of a Lua array, read in place.
///
/// `raw_get::<Vec<f64>>` allocates one `Vec` per call and both callers want at
/// most four values; at 376 regions a frame that is 752 heap allocations for
/// eight numbers each. Answers `(values, how many there were)`; a list longer
/// than four is reported by its real length so the caller can reject it.
fn floats(object: &mlua::Table, key: &str) -> Option<([f32; 4], usize)> {
    let table = object.raw_get::<Option<mlua::Table>>(key).ok().flatten()?;
    let count = table.raw_len();
    let mut out = [0.0f32; 4];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = table.raw_get::<Option<f64>>(index + 1).ok().flatten().unwrap_or(0.0) as f32;
    }
    Some((out, count))
}

/// The eight-argument `SetTexCoord`, read whole; see [`Paint::corners`].
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
/// own declarations are; a colour with no offset is not a shadow, since a copy
/// drawn under the glyphs at no offset is invisible.
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

/// The registry key the shared metatable lives under: one metatable for every
/// region in the game; see [`widget::metatable`].
const REG_META: &str = "vale.regionMeta";

/// The draw layer a region with no `level` lands in. `ARTWORK` is the game's own
/// default and the middle of the five, so a texture that declares no layer
/// lands over the background and under the text rather than at one end.
const DEFAULT_LAYER: &str = "ARTWORK";

/// The layer a slot region lands in. The markup never states it; the widget
/// kind decides it.
///
/// A `<Button>`'s parts are not declared inside `<Layers><Layer>`: they are
/// `<NormalTexture>`, `<PushedTexture>`, `<HighlightTexture>` and
/// `<ButtonText>`, none of which carries a `drawLayer` in either directory.
/// Without this rule all of them land in [`DEFAULT_LAYER`] together and the
/// tie-break is creation order, which is the order the template declares them
/// in.
///
/// `GlueButtonTemplate` declares `<ButtonText>` first, so every button on the
/// login screen would draw its label and then its plate over it: the words
/// come out muddy brown under a semi-transparent red panel, which looks like a
/// disabled button. `ActionButtonTemplate` has the same order.
///
/// The rule follows how the game arranges a button: the state art is behind,
/// the highlight is on the layer named for it, and the label is on top. A slot
/// this does not name keeps the default.
pub(in crate::lua) fn slot_layer(slot: &str) -> Option<&'static str> {
    match slot {
        // The label, above everything the button paints under it.
        "Text" => Some("OVERLAY"),
        // The mouse-over texture, on the layer named for it, which is above
        // `OVERLAY` in [`LAYERS`], as the game orders them.
        "Highlight" => Some("HIGHLIGHT"),
        // The state plates and the two thumbs: the art the label sits on.
        "Normal" | "Pushed" | "Disabled" | "Checked" | "DisabledChecked" => Some("BORDER"),
        _ => None,
    }
}

/// Put a region in a draw layer, from Rust. It writes the same field
/// `SetDrawLayer` writes, so a layer chosen by the loader and one chosen by a
/// script cannot disagree. Its one caller, the XML loader, passes it
/// [`slot_layer`]'s answer.
pub(in crate::lua) fn set_layer(lua: &mlua::Lua, region: &mlua::Table, layer: &str) -> mlua::Result<()> {
    widget::set_paint(lua, region, LAYER_KEY, layer)
}

/// Install the region metatable. Called once, from [`super::frames::install`].
pub(in crate::lua) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let methods = lua.create_table()?;
    widget::install(lua, &methods)?;
    register_methods(lua, &methods)?;
    install_measures(lua, &methods)?;
    // Then the region methods that are stubs, last so that none of them can
    // shadow one installed above; see [`super::super::api::stubs`].
    super::super::api::stubs::install_region_methods(lua, &methods)?;
    lua.set_named_registry_value(REG_META, widget::metatable(lua, methods)?)
}

/// Make a `Texture` or a `FontString`.
///
/// One function for both kinds, because `frame:CreateTexture(…)`,
/// `frame:CreateFontString(…)` and the XML loader's `<Layer>` walk must all
/// produce the same object. A region that differs depending on how it was
/// built can leave half the interface invisible.
pub(in crate::lua) fn create(
    lua: &mlua::Lua,
    kind: &str,
    name: Option<&str>,
    parent: Option<mlua::Table>,
    layer: Option<&str>,
) -> mlua::Result<mlua::Table> {
    let region = lua.create_table()?;
    widget::init(lua, &region, kind, name, parent)?;
    widget::set_paint(lua, &region, LAYER_KEY, layer.unwrap_or(DEFAULT_LAYER))?;
    widget::set_paint(lua, &region, BLEND_KEY, "BLEND")?;

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

    // `SetTexture` takes a path or a colour, distinguished by the arguments:
    // one string argument is a file, three or four numbers are a solid fill.
    // `MoneyFrame` and every backdrop in the directory use the second, and a
    // host that accepted only the first would draw nothing for them.
    let set_texture = lua.create_function(|_lua, args: mlua::MultiValue| {
        let mut args = args.into_iter();
        let this: mlua::Table = mlua::FromLua::from_lua(args.next().unwrap_or(mlua::Value::Nil), _lua)?;
        let rest: Vec<mlua::Value> = args.collect();
        // Any form of this call clears the region's portrait. `PetFrame` and
        // the party frames set a portrait through one function and a path
        // through the other onto the same `<Texture>`; a region that kept both
        // would show a face under an icon for the rest of the session. See
        // [`set_portrait_unit`].
        widget::set_paint(_lua, &this, PORTRAIT_KEY, mlua::Value::Nil)?;
        match rest.first() {
            Some(mlua::Value::String(path)) => {
                widget::set_paint(_lua, &this, TEXTURE_KEY, path.clone())?;
                widget::set_paint(_lua, &this, COLOUR_KEY, mlua::Value::Nil)
            }
            Some(mlua::Value::Number(_) | mlua::Value::Integer(_)) => {
                // `widget::number` and not `as_f64` (see its comment): every
                // integral component of a colour arrives as `Value::Integer`,
                // and `SetTexture(0, 0, 0, 0.5)` would keep only the alpha.
                let colour: Vec<f64> = rest.iter().filter_map(widget::number).collect();
                widget::set_paint(_lua, &this, TEXTURE_KEY, mlua::Value::Nil)?;
                store_numbers(_lua, &this, COLOUR_KEY, &colour)
            }
            // `SetTexture(nil)` clears it, which is how an empty action button
            // loses its icon.
            _ => {
                widget::set_paint(_lua, &this, TEXTURE_KEY, mlua::Value::Nil)?;
                widget::set_paint(_lua, &this, COLOUR_KEY, mlua::Value::Nil)
            }
        }
    })?;
    methods.set("SetTexture", set_texture)?;
    method!("GetTexture", |_lua, this| this
        .raw_get::<mlua::Value>(TEXTURE_KEY));

    // `SetText` takes any value and stores a string, because the directory
    // often passes it numbers (`count:SetText(charges)`), and Lua's
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

    // A `Texture` keeps its vertex colour apart from its own colour, which
    // `paint` multiplies in; a `FontString`'s vertex colour is its text colour.
    method!("SetVertexColor", mlua::Variadic<f64>, |lua, this, rgba| store_numbers(
        lua, &this, vertex_key(&this), &rgba
    ));
    method!("GetVertexColor", |_lua, this| {
        let rgba: Vec<f64> = this.raw_get::<Option<Vec<f64>>>(vertex_key(&this))?.unwrap_or_default();
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
    // A colour set by `SetTextColor` persists and one set by `SetVertexColor`
    // does not: only the first marks the string as carrying its own colour,
    // so only the first survives a later `SetFontObject`. See
    // [`apply_font_style`].
    method!("SetTextColor", mlua::Variadic<f64>, |lua, this, rgba| {
        store_numbers(lua, &this, COLOUR_KEY, &rgba)?;
        widget::set_paint(lua, &this, COLOUR_SET_KEY, true)
    });
    method!("SetTexCoord", mlua::Variadic<f64>, |lua, this, coords| store_numbers(
        lua, &this, COORDS_KEY, &coords
    ));
    // `SetTextHeight` writes the same key `<Font height=…>` does, through the
    // same [`set_font_height`], so the rasteriser's cap applies to a size a
    // script asked for exactly as it does to one the markup declared, and
    // `GetTextHeight` reads back what would be drawn rather than what was
    // asked for.
    //
    // Its one caller in the shipped directory is `ActionButton_UpdateHotkeys`,
    // which shrinks the hotkey to 8 to make room for the range dot. That code
    // runs only when `ActionHasRange` answers, so the method was not needed
    // while `ActionHasRange` was a stub.
    method!("SetTextHeight", f32, |lua, this, height| {
        let held = text_region(&this).unwrap_or_else(|| this.clone());
        set_font_height(lua, &held, height)
    });
    // `SetAlphaGradient(start, length)` is a `FontString`'s fade. It is on
    // this table rather than the frame's because `QuestDescription` is a font
    // string: `QuestFrameDetailPanel:OnShow` calls it to fade out the bottom
    // of a long quest text as the panel scrolls it in.
    //
    // Recorded as nothing. This client draws a string at one alpha, so only
    // the gradient is lost and none of the words. A missing method here would
    // abort the whole `OnShow`, and the quest page would open blank.
    method!("SetAlphaGradient", mlua::MultiValue, |_lua, _this, _args| Ok(()));
    method!("SetJustifyH", String, |lua, this, how| widget::set_paint(
        lua, &this, JUSTIFY_H_KEY, how
    ));
    method!("SetJustifyV", String, |lua, this, how| widget::set_paint(
        lua, &this, JUSTIFY_V_KEY, how
    ));
    method!("SetDrawLayer", String, |lua, this, layer| widget::set_paint(
        lua, &this, LAYER_KEY, layer
    ));
    method!("GetDrawLayer", |_lua, this| this
        .raw_get::<mlua::Value>(LAYER_KEY));
    method!("SetBlendMode", String, |lua, this, mode| widget::set_paint(
        lua, &this, BLEND_KEY, mode
    ));
    method!("GetBlendMode", |_lua, this| this
        .raw_get::<mlua::Value>(BLEND_KEY));

    // `SetFont(path, height, flags)` sets the face by file, as an addon does
    // (`pfUI` sets every one of its strings this way). The three outline
    // words map onto the two values of the `<Font outline=…>` attribute.
    method!(
        "SetFont",
        (Option<String>, Option<f64>, Option<String>),
        |lua, this, args| set_font_triplet(
            lua,
            &this,
            args.0.as_deref(),
            args.1,
            args.2.as_deref()
        )
    );
    method!("SetShadowOffset", (Option<f64>, Option<f64>), |lua, this, args| store_numbers(
        lua,
        &this,
        SHADOW_OFFSET_KEY,
        &[args.0.unwrap_or(0.0), args.1.unwrap_or(0.0)]
    ));
    method!("SetShadowColor", mlua::Variadic<f64>, |lua, this, rgba| store_numbers(
        lua,
        &this,
        SHADOW_COLOUR_KEY,
        &[
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
    // `SetFontObject(GameFontNormal)`: a font object is the global a virtual
    // `<Font>` declares (see [`make_font_object`]). Its face is copied onto
    // the string, as `inherits` does at load.
    method!("SetFontObject", mlua::Value, |lua, this, value| {
        let Some(font) = resolve_font_object(lua, value)? else {
            return Ok(());
        };
        apply_font_style(lua, &this, &font)?;
        this.set(FONT_OBJECT_KEY, font)
    });
    method!("GetFontObject", |_lua, this| this
        .raw_get::<mlua::Value>(FONT_OBJECT_KEY));

    // FrameXML uses `GetObjectType` to check what kind of object it holds:
    // `if ( region:GetObjectType() == "FontString" )`.
    method!("GetObjectType", |_lua, this| this
        .raw_get::<mlua::Value>(widget::KIND_KEY));
    // `IsObjectType` asks whether an object, which may be a frame or a
    // region, is of a given type. `UIParent_ManageFramePosition` walks a
    // table of names and calls `frame:IsObjectType("frame")` on whatever
    // `getglobal` returns, which is sometimes a region; if a region lacks the
    // method, the error aborts all of `UIParent`'s layout pass. Every UIObject
    // in 1.12 has this method, so it is on the region table as well as the
    // frame table.
    //
    // It follows the type tree; see [`widget::derives_from`]. A `CheckButton`
    // answers `1` to `Button` and every frame kind answers `1` to `Frame`; an
    // equality test would answer wrongly for every caller that asks about a
    // general type.
    method!("IsObjectType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });
    Ok(())
}

/// Set a texture path from an XML attribute, for the loader, without going
/// through Lua. [`set_text`], [`set_blend`] and [`set_layer`] do the same for
/// a text, a blend mode and a layer.
pub(in crate::lua) fn set_file(lua: &mlua::Lua, region: &mlua::Table, path: &str) -> mlua::Result<()> {
    widget::set_paint(lua, region, TEXTURE_KEY, path)
}

/// Set a texture path from a C function that paints a portrait:
/// `SetPortraitToTexture` and `SetBagPortaitTexture`, both of which take a
/// texture object and a path and do nothing else. `None` clears it, which is
/// what a bag slot with no bag in it does.
pub(in crate::lua) fn set_texture_path(
    lua: &mlua::Lua,
    region: &mlua::Table,
    path: Option<&str>,
) -> mlua::Result<()> {
    match path {
        Some(path) => widget::set_paint(lua, region, TEXTURE_KEY, path),
        None => widget::set_paint(lua, region, TEXTURE_KEY, mlua::Value::Nil),
    }
}

/// Set the unit a region shows a portrait of, for the C function that paints
/// a unit: `SetPortraitTexture(texture, "target")`. See [`PORTRAIT_KEY`] and
/// [`super::super::api::portrait`].
///
/// `SetTexture` clears it, because `TargetFrame.lua`'s `TargetFrame_Update`
/// sets a portrait and `PetFrame`'s sets the same slot with a path, so a
/// region that kept both would draw a portrait under an icon after the first
/// time it held a unit. The clearing is in the `SetTexture` method rather than
/// here, because that setter is the one replacing the portrait.
pub(in crate::lua) fn set_portrait_unit(
    lua: &mlua::Lua,
    region: &mlua::Table,
    token: Option<&str>,
) -> mlua::Result<()> {
    match token {
        Some(token) => widget::set_paint(lua, region, PORTRAIT_KEY, token),
        None => widget::set_paint(lua, region, PORTRAIT_KEY, mlua::Value::Nil),
    }
}

/// What `GetText()` answers when nothing has been written. The answer depends
/// on the kind.
///
/// An `EditBox` answers `""` and everything else answers nil, as in the
/// 1.12.1 client. The directory depends on both, and on the edit box's answer
/// in particular:
///
/// ```lua
/// local copper = getglobal(moneyFrame:GetName().."Copper"):GetText();
/// if ( copper ~= "" ) then totalCopper = totalCopper + copper; end
/// ```
///
/// Here a nil passes the guard and is then added to a number.
/// `MoneyInputFrame_GetCopper` is called from `TradeFrame_OnShow`'s second
/// line, so a nil from an empty edit box made the trade window open with only
/// its art.
///
/// A single function, because `GetText` is installed twice: here and on the
/// shared frame table by [`super::button`], whose copy shadows this one for
/// every frame in the interface.
pub(super) fn empty_text(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<mlua::Value> {
    let kind: Option<String> = object.raw_get(widget::KIND_KEY)?;
    Ok(match kind.as_deref() {
        Some("EditBox") => mlua::Value::String(lua.create_string("")?),
        _ => mlua::Value::Nil,
    })
}

/// Set a frame's or region's text from Rust. `text="CANCEL"` on a frame is its
/// font string's text, not the frame's.
///
/// A `<Button text="…">` whose template gave it a `<ButtonText>` has a region
/// for the label, and that region is what gets drawn: it has the rectangle,
/// the typeface and the layer, and the frame has none of them. So the text is
/// written to the region when there is one, as the game does, which makes
/// `tab:GetText()` answer.
///
/// A frame with no font string keeps the text on itself, so nothing is lost,
/// and [`adopt_pending_text`] moves it when the region is attached later under
/// [`slot_key`]'s key.
pub(in crate::lua) fn set_text(lua: &mlua::Lua, object: &mlua::Table, text: &str) -> mlua::Result<()> {
    let holder = text_region(object).unwrap_or_else(|| object.clone());
    write_text(lua, &holder, mlua::Value::String(lua.create_string(text)?))
}

/// Write a string, which can change geometry. An auto-sized `FontString` takes
/// its rectangle from the text it holds ([`intrinsic`]), so a new string is a
/// new rectangle and the layout memo has to be invalidated; otherwise a label
/// keeps the width of its first text, which for one that started empty is
/// zero.
///
/// A write that changes nothing invalidates nothing and does not bump the
/// paint generation. The shipped `OnUpdate` bodies set their labels again
/// every tick (`CastingBarFrame`'s timer, the durability text, every
/// `SetText(format(…))` in the interface), and invalidating the whole solve
/// for each would spend a layout generation per frame per label and bring
/// back the garbage-collector saw-tooth. [`super::widget`]'s five geometry
/// setters follow the same rule for the same reason.
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
    widget::mark_paint(lua);
    // Only a font string's own rectangle depends on its text; a frame holding a
    // pending label has no text-sized rectangle. The kind check is a raw read.
    if holder.raw_get::<Option<String>>(widget::KIND_KEY).ok().flatten().as_deref()
        == Some("FontString")
    {
        super::layout::invalidate(lua)?;
    }
    Ok(())
}

/// The same rule from Lua, for any value.
///
/// One function behind both `FontString:SetText` and `Button:SetText`, because
/// they differ only in whether the object has a font string to write to. A
/// `FontString` has none and so writes its own text.
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

/// `frame:SetText(value)` for any kind of frame: the one body behind a method
/// that is installed twice.
///
/// `SetText` is on the shared frame table and two modules put it there:
/// [`super::button`] forwards a label to the button's own font string, and
/// [`super::tooltip`] replaces the whole method because `GameTooltip:SetText`
/// means something else. The tooltip's copy is installed last, so every frame
/// in the game calls it, and its "not a tooltip" branch is the implementation
/// for all other frames. Both copies call this function, so a change such as
/// firing an `EditBox`'s `OnTextSet` applies whichever copy runs.
///
/// See [`super::editbox::text_was_set`] for the second call.
pub(super) fn set_frame_text(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    value: mlua::Value,
) -> mlua::Result<()> {
    set_text_value(lua, frame, value)?;
    super::editbox::text_was_set(lua, frame)
}

/// The style a frame lays out its own text in: its unnamed `<FontString>`.
///
/// Two widgets need this, for the same reason: the text they draw has no
/// object. A `MessageFrame`'s lines and an `EditBox`'s typed line are both
/// laid out by the widget itself, in the style of the one font string declared
/// inside the element with no name, no anchors and no text:
///
/// ```xml
/// <ScrollingMessageFrame name="ChatFrameTemplate" …>
///     <FontString inherits="ChatFontNormal" justifyH="LEFT"/>
/// <EditBox name="ChatFrameEditBoxTemplate" …>
///     <FontString inherits="ChatFontNormal" bytes="256"/>
/// ```
///
/// It is here rather than in either widget's module because it depends on how
/// a region is declared, and so both share one copy.
///
/// The `text.is_none()` test tells the style from a real label: a
/// `<FontString>` carrying a string is something the frame draws, not the
/// style it draws other text in.
///
/// The `is_font` test is also required. A `Texture` has no text either, so
/// without it the first texture declared inside the frame is chosen, and
/// `ChatFrameEditBoxTemplate` declares three of them, in a `<Layer>` above the
/// `<FontString>` at the end. The typed line would then be laid out in the
/// style of `UI-ChatInputBorder-Left`: no face and no height, so Friz Quadrata
/// at the default 12 where the game uses Arial Narrow at 14.
pub(super) fn own_font(frame: &mlua::Table) -> Option<Paint> {
    widget::children(frame)
        .ok()?
        .sequence_values::<mlua::Table>()
        .flatten()
        .find_map(|child| paint(&child).filter(|paint| paint.is_font && paint.text.is_none()))
}

/// A frame's own font string, if it has one; see [`TEXT_REGION_KEY`].
pub(in crate::lua) fn text_region(object: &mlua::Table) -> Option<mlua::Table> {
    object.raw_get::<Option<mlua::Table>>(TEXT_REGION_KEY).ok().flatten()
}

/// A button's font string, created if the button has none. A button's label
/// does not need a `<ButtonText>`: the widget makes a font string when it is
/// given a face or a string.
///
/// In the 1.12.1 client, `SetText` on a button with no font string creates
/// one, parents it to the button and makes it the button's font string, as
/// `<ButtonText>` does. Because the new string has no anchors of its own, it
/// is anchored to the button on the point its `justifyH` names.
///
/// 18 templates and buttons in the two directories declare a `<NormalFont>`
/// and no `<ButtonText>`, and without this their labels are not drawn:
/// `StaticPopupButtonTemplate` is Accept, Decline, Release Spirit and Retrieve
/// Corpse (every dialog in the game), and `UIMenuButtonTemplate`,
/// `UIPanelButtonGrayTemplate` and `GlueMenuButtonTemplate` are the rest. The
/// string is stored on the frame by [`set_frame_text`] and read back correctly
/// by `GetText`, so no error is raised: the dialog draws its plate, sizes its
/// buttons from `GetTextWidth`, and paints no words.
///
/// This client creates the region when the font is set rather than on
/// `SetText`, which has the same result: a font string with no font object
/// draws nothing in 1.12 either, so the buttons this skips (the 1,002 that
/// declare neither) are the ones whose implicit string would be invisible.
/// Creating it at the font means the face is available to apply: this client
/// keeps a region's typeface on the region, where the widget keeps three font
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
    // The same two properties the loader gives a declared `<ButtonText>`: no
    // anchors means fill the button (see the note in [`super::super::xml`]),
    // and the label goes above the plate rather than under it.
    widget::default_all_points(lua, &made).ok()?;
    widget::set_paint(lua, object, TEXT_REGION_KEY, made.clone()).ok()?;
    // Move down a `text="ACCEPT"` attribute that arrived before the face did.
    adopt_pending_text(lua, object, &made);
    Some(made)
}

/// This object's text, whichever of the two ways it holds it.
///
/// A `FontString`'s own string, or the string in a frame's font string, or the
/// one left on a frame that has none. Never raises: its readers are `GetText`
/// and `GetTextWidth`, and a wrong assumption about what `__text` held used to
/// abort the `OnLoad` that called them.
pub(in crate::lua) fn text_of(object: &mlua::Table) -> Option<String> {
    if let Some(region) = text_region(object) {
        return region.raw_get::<Option<String>>(TEXT_KEY).ok().flatten();
    }
    object.raw_get::<Option<String>>(TEXT_KEY).ok().flatten()
}

/// Where a slot region is stored on its parent: `<NormalTexture>` under
/// `__normal`, and `<ButtonText>` under [`TEXT_REGION_KEY`] rather than under
/// the key a region's string uses.
///
/// One function so that the loader and [`super::button`]'s getters cannot
/// diverge: they are the two halves of the same convention.
pub(in crate::lua) fn slot_key(slot: &str) -> String {
    if slot.eq_ignore_ascii_case("Text") {
        return TEXT_REGION_KEY.to_string();
    }
    format!("__{}", slot.to_lowercase())
}

/// Move a frame's pending text onto its font string region as the region is
/// attached. This handles the slot arriving after the attribute: a
/// `<Button text="X">` whose own `<ButtonText>` is a child rather than
/// inherited, so the string is on the frame with no region to draw it.
pub(in crate::lua) fn adopt_pending_text(lua: &mlua::Lua, object: &mlua::Table, region: &mlua::Table) {
    let Some(text) = object.raw_get::<Option<String>>(TEXT_KEY).ok().flatten() else {
        return;
    };
    if let Ok(text) = lua.create_string(&text) {
        let _ = write_text(lua, region, mlua::Value::String(text));
    }
    let _ = widget::set_paint(lua, object, TEXT_KEY, mlua::Value::Nil);
}

pub(in crate::lua) fn set_blend(lua: &mlua::Lua, region: &mlua::Table, mode: &str) -> mlua::Result<()> {
    widget::set_paint(lua, region, BLEND_KEY, mode)
}

pub(in crate::lua) fn set_colour(lua: &mlua::Lua, region: &mlua::Table, rgba: [f64; 4]) -> mlua::Result<()> {
    store_numbers(lua, region, COLOUR_KEY, &rgba)
}

/// Set a colour that persists through face changes, which is what
/// `SetTextColor` writes.
///
/// The difference from [`set_colour`] is [`COLOUR_SET_KEY`], which
/// [`apply_font_style`] reads: in 1.12 a colour set with `SetTextColor` survives
/// a change of font object. `SetTextColor` on a frame that is not a button
/// forwards here. On a button it colours the normal face only, which
/// [`super::button::set_text_colour`] handles.
pub(super) fn set_text_colour(lua: &mlua::Lua, region: &mlua::Table, rgba: [f64; 4]) -> mlua::Result<()> {
    set_colour(lua, region, rgba)?;
    widget::set_paint(lua, region, COLOUR_SET_KEY, true)
}

/// Whether this font string folds its text; see [`WRAP_KEY`].
pub(super) fn set_wrap(lua: &mlua::Lua, region: &mlua::Table, wrap: bool) -> mlua::Result<()> {
    // The fold changes a string's intrinsic height, so this is the write that also invalidates layout.
    store_face(lua, region, WRAP_KEY, wrap)
}

/// `<TexCoords left="0" right="1.0" top="0.79296875" bottom="0.83203125"/>`,
/// which appears 308 times; it writes the same key `SetTexCoord` writes.
///
/// It selects which part of the file a texture draws, which lets much of the
/// interface share one atlas: the four experience-bar fills are four slices of
/// `UI-MainMenuBar-Dwarf` at four y ranges. A loader that dropped these would
/// draw the whole texture in each of the 308 rectangles, squashed into the
/// wrong shape, which looks like wrong art rather than missing art.
pub(in crate::lua) fn set_coords(lua: &mlua::Lua, region: &mlua::Table, coords: [f64; 4]) -> mlua::Result<()> {
    store_numbers(lua, region, COORDS_KEY, &coords)
}

/// `font="Fonts\FRIZQT__.TTF"` and `<FontHeight><AbsValue val="12"/>`.
///
/// These arrive through `inherits`, not on the element. A `<FontString>` in
/// the directory says `inherits="GameFontNormalSmall"` and nothing else; the
/// face, the height and the colour are all on the `<Font>` object in
/// `Fonts.xml`, which the loader applies as a template. A client that read
/// only the element's own attributes would find a typeface on none of the
/// interface's 1,652 regions.
pub(in crate::lua) fn set_font(lua: &mlua::Lua, region: &mlua::Table, path: &str) -> mlua::Result<()> {
    store_face(lua, region, FONT_KEY, path)
}


/// `SetFont`'s three arguments, applied to whatever carries a face: a font
/// string, a frame's own string, or a font object. The flags word is
/// `"OUTLINE"`, `"THICKOUTLINE"`, a comma-joined list of those and
/// `"MONOCHROME"`, or nothing. No flags clears the outline, as the 1.12.1
/// client does for a call that omits them.
pub(in crate::lua) fn set_font_triplet(
    lua: &mlua::Lua,
    region: &mlua::Table,
    path: Option<&str>,
    height: Option<f64>,
    flags: Option<&str>,
) -> mlua::Result<()> {
    if let Some(path) = path {
        store_face(lua, region, FONT_KEY, path)?;
    }
    if let Some(height) = height {
        set_font_height(lua, region, height as f32)?;
    }
    let flags = flags.unwrap_or("").to_ascii_uppercase();
    if flags.contains("THICKOUTLINE") {
        widget::set_paint(lua, region, OUTLINE_KEY, "THICK")
    } else if flags.contains("OUTLINE") {
        widget::set_paint(lua, region, OUTLINE_KEY, "NORMAL")
    } else {
        widget::set_paint(lua, region, OUTLINE_KEY, mlua::Value::Nil)
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

/// A font object by value or by name: `SetFontObject(GameFontNormal)` and
/// `SetFontObject("GameFontNormal")` both occur in addons. Anything else is
/// `None`, which the caller ignores.
pub(in crate::lua) fn resolve_font_object(lua: &mlua::Lua, value: mlua::Value) -> mlua::Result<Option<mlua::Table>> {
    Ok(match value {
        mlua::Value::Table(table) => Some(table),
        mlua::Value::String(name) => lua.globals().get::<Option<mlua::Table>>(name.to_str()?.to_string())?,
        _ => None,
    })
}

/// A font object: the global a virtual `<Font name="…">` declares.
///
/// In the 1.12.1 client every `<Font>` declares a global object whatever
/// `virtual` says, and addons read it: `SystemFont:GetFont()` is pfUI's first
/// line of font handling. Here it is a table carrying the same face keys a
/// string does, filled by the loader through the same `inherits` walk, so
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
        |lua, this, args| set_font_triplet(
            lua,
            &this,
            args.0.as_deref(),
            args.1,
            args.2.as_deref()
        )
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
    method!("SetTextColor", mlua::Variadic<f64>, |lua, this, rgba| set_colour(
        lua,
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
    method!("SetShadowColor", mlua::Variadic<f64>, |lua, this, rgba| store_numbers(
        lua,
        &this,
        SHADOW_COLOUR_KEY,
        &[
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
    method!("SetShadowOffset", (Option<f64>, Option<f64>), |lua, this, args| store_numbers(
        lua,
        &this,
        SHADOW_OFFSET_KEY,
        &[args.0.unwrap_or(0.0), args.1.unwrap_or(0.0)]
    ));
    method!("GetJustifyH", |_lua, this| Ok(this
        .raw_get::<Option<String>>(JUSTIFY_H_KEY)?
        .unwrap_or_else(|| "CENTER".to_string())));
    method!("GetJustifyV", |_lua, this| Ok(this
        .raw_get::<Option<String>>(JUSTIFY_V_KEY)?
        .unwrap_or_else(|| "MIDDLE".to_string())));
    method!("SetJustifyH", String, |lua, this, how| widget::set_paint(lua, &this, JUSTIFY_H_KEY, how));
    method!("SetJustifyV", String, |lua, this, how| widget::set_paint(lua, &this, JUSTIFY_V_KEY, how));
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
            apply_font_style(lua, &this, &other)?;
            this.set(FONT_OBJECT_KEY, other)
        })?;
        methods.set(name, f)?;
    }
    lua.set_named_registry_value(REG_FONT_METHODS, methods.clone())?;
    Ok(methods)
}

/// Set the font height, clamped where it is written rather than where it is
/// drawn.
///
/// [`vale_assets::interface::font::drawn_height`] is the 1.12.1 client's
/// rasteriser cap, and the reasoning for it is documented there. It is applied
/// on the write because everything downstream (the painter, [`text_width`],
/// [`line_height_of`], `GetFont`) reads this one key, and a cap applied on
/// only some of those paths would size a frame for one height and draw text
/// at another. The 1.12.1 client matches: `GetFont` answers the rasterised
/// size, not the requested one.
pub(in crate::lua) fn set_font_height(lua: &mlua::Lua, region: &mlua::Table, height: f32) -> mlua::Result<()> {
    store_face(
        lua,
        region,
        FONT_HEIGHT_KEY,
        f64::from(vale_assets::interface::font::drawn_height(height)),
    )
}

/// `outline="NORMAL"` on a `<Font>`; see [`Paint::outline`].
pub(in crate::lua) fn set_outline(lua: &mlua::Lua, region: &mlua::Table, how: &str) -> mlua::Result<()> {
    widget::set_paint(lua, region, OUTLINE_KEY, how)
}

/// `<Shadow>`: the offset copy under the glyphs, and its colour. Two keys
/// rather than a table, because the draw reads them every frame, and a nested
/// table read per font string costs more.
pub(in crate::lua) fn set_shadow(
    lua: &mlua::Lua,
    region: &mlua::Table,
    offset: [f32; 2],
    colour: [f64; 4],
) -> mlua::Result<()> {
    store_numbers(lua, region, SHADOW_OFFSET_KEY, &offset.map(f64::from))?;
    store_numbers(lua, region, SHADOW_COLOUR_KEY, &colour)
}

pub(in crate::lua) fn set_justify(lua: &mlua::Lua, region: &mlua::Table, key: &str, how: &str) -> mlua::Result<()> {
    let slot = match key {
        "justifyH" => JUSTIFY_H_KEY,
        _ => JUSTIFY_V_KEY,
    };
    widget::set_paint(lua, region, slot, how)
}

/// Lua's own `tostring` for the values `SetText` is given: a string, a number,
/// or something that is neither.
fn stringify(value: &mlua::Value) -> String {
    match value {
        mlua::Value::String(s) => s.to_string_lossy(),
        mlua::Value::Integer(n) => n.to_string(),
        // Lua 5.1 prints an integral float without a fractional part, so
        // `count:SetText(3)` reads "3" rather than "3.0" on a button.
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

    /// A region made from Lua is the same object the loader makes, which is
    /// why [`create`] is one function: `ActionButton.lua` finds `$parentIcon`,
    /// declared in XML, and calls `SetTexture` on it.
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
        // The base widget methods are installed too.
        assert_eq!(eval(&lua, "return icon:IsShown()"), "Integer(1)");
    }

    /// `GetTextHeight` counts the rows the string folds into; the gossip menu
    /// depends on it.
    ///
    /// `GossipResize` sizes each option button to `GetTextHeight() + 2` and the
    /// next button is anchored to that bottom edge, so a one-line answer for a
    /// two-line label paints every option over the one below it.
    ///
    /// Asserted as a ratio rather than in pixels: with no archives open the
    /// measurement falls back to a fixed ratio per character, which is enough
    /// to fold but does not give the width a real face gives.
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

    /// A string with no declared width does not fold, so its height stays one
    /// line box however long it is. Most of the directory's labels are
    /// declared with an anchor and no size and are as wide as their text.
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

    /// `SetTexture` takes a path or a colour, distinguished by the argument's
    /// type. Every backdrop in the directory uses the second form, and a host
    /// that accepted only the first would leave them blank.
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

    /// A texture's vertex colour multiplies its own colour rather than
    /// replacing it. `SkillFrame`'s bar background is a white `<Color>` at 0.2
    /// tinted `(0, 0, 0.75, 0.5)`, and must come out at alpha 0.1.
    #[test]
    fn a_vertex_colour_multiplies_a_textures_own_colour() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame");
            t = f:CreateTexture();
            t:SetTexture(1, 1, 1, 0.2);
            t:SetVertexColor(0, 0, 0.75, 0.5);
            "#,
        )
        .exec()
        .expect("loads");
        let texture: mlua::Table = lua.globals().get("t").expect("the texture");
        let colour = paint(&texture).expect("a region").colour;
        assert_eq!(colour, [0.0, 0.0, 0.75, 0.1]);
        assert_eq!(eval(&lua, "return select(4, t:GetVertexColor())"), "Number(0.5)");
        // A new path keeps the tint.
        lua.load(r#"t:SetTexture("Interface\Buttons\UI-QuickslotRed")"#).exec().expect("runs");
        assert_eq!(paint(&texture).expect("a region").colour, [0.0, 0.0, 0.75, 0.5]);
    }

    /// `SetText` is often given numbers (`count:SetText(charges)`), and 1.12
    /// shows "3" rather than "3.0". Getting that wrong puts a decimal point on
    /// every stack count in the game.
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

    /// Every name [`METHODS`] lists is installed. Every method list in this
    /// directory has this check, for the reason [`super::widget`]'s copy of
    /// this test gives.
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

    /// A font string with a width of its own is as tall as its text folds to,
    /// as in the spellbook's declaration.
    ///
    /// `$parentSpellName` is `<AbsDimension x="103" y="0"/>` with
    /// `maxLines="3"`, so a long name wraps and the rank line under it moves
    /// down with it. With a height of one line box for every string, "Rallying
    /// Cry of the Dragonslayer" ran out through the side of the page and the
    /// row below it did not move.
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
        // A string that declared no width does not fold, which keeps every
        // ordinary label in the interface one line high.
        assert_eq!(height("bare"), line);

        // The painter's fold comes from the same function, so the box and the
        // glyphs cannot disagree about whether there is a fold.
        let region: mlua::Table = globals.get("long").expect("the region");
        let folded = paint(&region).expect("a region");
        assert!(folded.wrap, "the painter must fold what the layout made room for");
        let bare: mlua::Table = globals.get("bare").expect("the region");
        assert!(!paint(&bare).expect("a region").wrap);
    }

    /// A string anchored on both sides folds too, though it declared no width.
    ///
    /// Interface code writes a paragraph with two anchors, not `SetWidth`:
    ///
    /// ```lua
    /// f.text:SetPoint("TOPLEFT", f, "TOPLEFT", 10, -10)
    /// f.text:SetPoint("BOTTOMRIGHT", f, "BOTTOMRIGHT", -10, 10)
    /// ```
    ///
    /// The fold width is the solved rectangle's, which only the painter knows,
    /// so this module only makes the decision; see [`pinned_across`]. Without
    /// it the fold is `INFINITY` and the sentence runs out through both sides
    /// of the box it was measured into, as in pfUI's first-run wizard and
    /// every report of text running out of a frame.
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
        // One anchor does not give a width. A label anchored at a single point
        // is as wide as its own text, so folding it would break every ordinary
        // caption in the interface at its first space.
        assert!(!wraps("oneSided"));
    }

    /// `maxLines` caps the fold in both the height and the paint: the height
    /// stops growing and the painter is told to stop drawing. A cap applied to
    /// only one of the two would either leave a gap or overprint the row
    /// beneath.
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
