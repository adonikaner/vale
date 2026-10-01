//! The `GameTooltip`, the one widget whose contents the client fills rather
//! than Lua.
//!
//! Every other widget in the directory is filled by Lua. A tooltip is filled by
//! the client: `OnEnter` calls `GameTooltip:SetAction(slot)` and the client
//! composes the lines. Calling a nil method aborts the Lua body it is in, so
//! without these methods every `OnEnter` that shows a tooltip stops at its
//! first population call. The two such calls on the HUD are
//! `ActionButton.lua:377` (`SetAction`) and `BuffFrame.lua:244`
//! (`SetInventoryItem`); the most frequent missing calls were in the same
//! family (`SetInventoryItem` ×8, `SetUnitDebuff` ×8).
//!
//! ## Layout from `GameTooltipTemplate.xml`
//!
//! `GameTooltipTemplate.xml` declares the whole visible shape: the `<Backdrop>`
//! (`UI-Tooltip-Background` + `UI-Tooltip-Border`, edge 16, insets 5), and a
//! ladder of thirty hidden `FontString` pairs. `$parentTextLeft1` is at
//! `TOPLEFT (10, -10)`, each next left line is anchored `TOPLEFT` to the
//! previous line's `BOTTOMLEFT (0, -2)`, line 1 uses `GameTooltipHeaderText`
//! and the rest use `GameTooltipText`. The pad is therefore 10 and the line gap
//! 2. The lines are named regions (`GameTooltipTextLeft1`) because the
//! directory addresses them by name: `GameTooltip.xml`'s `OnEvent` recolours
//! `TextLeft1` on `UPDATE_MOUSEOVER_UNIT`. This module adopts the declared
//! ladder and creates more lines of the same shape past line 30, as the
//! 1.12.1 client does.
//!
//! The template does not state the tooltip's size or the right column's
//! position; the client computes both from the lines. Here that is
//! [`reflow`]: width is the widest line (plus the two-column gap), floored by
//! `SetMinimumWidth`; height is the sum of the lines; both are padded by 10.
//! Line widths are measured in the game's typefaces, and a row's height is the
//! face's line box rather than the declared font height. All text measurement
//! in the interface goes through [`super::text::width`]. An earlier estimate
//! of half the font height per character, against a measured mean advance of
//! 0.609 em, made the plate ten units too narrow for `"BM Only OFF"`; because a
//! `FontString` is centred by default, the text overflowed both sides of the
//! border.
//!
//! ## Client rules this module follows
//!
//! Three behaviours here match the 1.12.1 client's `AddLine`, `SetText` and
//! `AddDoubleLine`, whose default colour is `0xffffd200`:
//!
//! * A line with no colour is gold, `255/210/0`, not white. `AddLine` applies
//!   its colour arguments only when the r argument is a number. The corpus
//!   form `AddLine(text, "", 1.0, 1.0, 1.0)` has `""` there, so all three
//!   colour arguments are ignored and the line is gold.
//! * `SetText` shows the tooltip; `AddLine` does not. The corpus never calls
//!   `Show()` after `SetText` and sometimes does after `AddLine`.
//! * `Hide` clears the owner and the lines and fires `OnTooltipCleared`. This
//!   stops `UnitFrame_OnUpdate`'s `IsOwned` check from showing again a
//!   tooltip the pointer has left.
//!
//! The owner anchors (`SetOwner`'s `ANCHOR_*` words) are the documented 1.12
//! set: the tooltip hangs one of its corners off one of the owner's.
//! `ANCHOR_RIGHT` puts this tooltip's `BOTTOMLEFT` on the owner's `TOPRIGHT`,
//! and the others follow the same pattern around the compass. `ANCHOR_NONE`
//! leaves anchoring to the caller, which is how
//! `GameTooltip_SetDefaultAnchor` uses it.
//!
//! ## Population reads the live world
//!
//! `SetAction` and `SetUnit` are scoped reads, registered per call by
//! [`super::super::api::install`] like every other query the interface makes,
//! because a tooltip shows the world's state at the moment of the hover. The
//! lines for a spell are name | rank, cost | range, cast time | cooldown, and
//! each cell is omitted when it has no value. The format strings are the
//! game's globals (`MANA_COST`, `SPELL_RANGE`, `SPELL_CAST_TIME_SEC`…), read
//! from the environment `GlobalStrings.lua` filled, as the client reads them.
//! A key the file does not define displays as nothing, which is the client's
//! behaviour and the project's rule.
//!
//! Below those lines come the reagents and the description. Both arrive
//! already resolved. `SPELL_REAGENTS` ("Reagents: ") is a `GlobalStrings.lua`
//! key, but the item names come from the server's templates, and the
//! description's `$s1`/`$d` variables are substituted in
//! [`vale_assets::tables::spelltext`]; both happen behind
//! [`crate::interface::api::spell_tip`]. This module only composes the lines,
//! in the game's order: name, cost/range, cast/cooldown, reagents, then the
//! description in green, the one colour on the plate that is not gold or
//! white.
//!
//! The population methods with no state behind them (bags, buffs, merchant)
//! are in [`super::super::api::stubs`] and are counted separately. An empty
//! population hides the tooltip and keeps the owner, so a refresh loop keeps
//! its ownership check; only `Hide` clears the owner.
//!
//! ## World mouseover tooltip
//!
//! Every other population here is reached from Lua: an `OnEnter` body calls
//! `SetAction`, `SetBagItem` or `SetSpell`. The world mouseover is not. Build
//! 5875's `WorldFrame` declares no `<OnEnter>`, so nothing in the shipped
//! files shows a unit's tooltip. The client shows it itself, on the frame the
//! pointer crosses onto the unit. [`TooltipPlugin`] does that here: it
//! anchors, fills and hides the same `GameTooltip` every other caller uses,
//! through the directory's `GameTooltip_SetDefaultAnchor`.

use bevy::prelude::*;

use super::super::api::{one_or_nil, Answers, LuaWorld};
use super::super::host::LuaHost;
use super::frames;
use super::regions;
use super::widget;
use crate::interface::api::SpellTip;

/// Shows the world mouseover tooltip, the one population no `<OnEnter>` in the
/// directory reaches. See the module comment.
pub struct TooltipPlugin;

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // Runs after all of `GameSet` and before the event dispatch. The
            // plate must hold this unit's name when `UPDATE_MOUSEOVER_UNIT`
            // reaches `GameTooltip.xml`'s handler, whose whole body recolours
            // `GameTooltipTextLeft1`. If the recolour ran first it would colour
            // the previous unit's name and the fill would then overwrite it,
            // leaving the colour one hover behind.
            show_world_tooltip
                .after(crate::interface::GameSet)
                .before(super::super::api::events::dispatch),
        );
    }
}

/// Fill the `GameTooltip` for whatever the pointer is over, and take it down
/// again when it leaves.
///
/// Runs on a change, not every frame, which is [`crate::interface::target`]'s
/// rule for the same reason: `SetUnit` clears and re-appends every line, and
/// doing that sixty times a second discards the interface's solved rectangles
/// as fast as they are computed. The container frames' `SetOwner` loop had
/// this fault.
///
/// The chunks make the same calls as the 1.12.1 client, in the same order.
/// `GameTooltip_SetDefaultAnchor` is defined in `GameTooltip.lua`; it puts the
/// plate at the bottom right rather than at the pointer.
fn show_world_tooltip(
    host: Option<NonSendMut<LuaHost>>,
    world: LuaWorld,
    hovered: Res<crate::interface::target::Hovered>,
    // The game-object result of the same pick; see
    // [`crate::interface::object::HoveredObject`]. At most one of `hovered` and
    // `object` is filled, so the branches below are a plain `if` chain.
    object: Res<crate::interface::object::HoveredObject>,
    // The window, read only to detect pointer movement; see [`Latch`]. A
    // floating plate's position comes from `GetCursorPosition()` inside the
    // chunk. This is the same conversion through the same `Viewport` type that
    // `lua::api::mouse` uses, not a separate derivation.
    windows: Query<&bevy::window::Window, With<bevy::window::PrimaryWindow>>,
    // The scale that conversion uses; see [`crate::ui::scale`].
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut last: Local<Option<Latch>>,
) {
    let Some(host) = host else { return };
    if !host.loaded() {
        return;
    }
    // The plate is hidden only when `GameTooltip.default` is set, not
    // unconditionally. A plate a button owns must stay when the pointer leaves
    // a unit behind the button. `GameTooltip_OnHide` and `UnitFrame_OnUpdate`
    // both test ownership, and `default` is the flag that only
    // `GameTooltip_SetDefaultAnchor` sets.
    let plate = if hovered.guid.is_some() {
        Plate::Unit
    } else if object.guid.is_none() || object.name.is_empty() {
        Plate::None
    } else if object.hover.floating {
        Plate::Floating
    } else {
        Plate::Object
    };
    // The whole plate is compared, not only the guid, for two reasons. A game
    // object's name arrives a round trip after the object, so a guid-only
    // latch would draw a nameless plate once and never correct it; the loot
    // window's two-second name delay had the same cause. A floating plate is
    // positioned at the pointer, so it must be redrawn as the pointer moves
    // across the sign; that is the `at` field. `at` is `None` for every other
    // kind of plate so that a unit's plate is not rebuilt every frame for small
    // pointer movements inside it.
    let now = Latch {
        plate,
        guid: hovered.guid.or(object.guid),
        name: object.name.clone(),
        lock: object.plate.clone(),
        at: (plate == Plate::Floating)
            .then(|| {
                let window = windows.single().ok()?;
                let view = super::layout::Viewport::of(
                    f64::from(window.width()),
                    f64::from(window.height()),
                    ui_scale.get(),
                );
                let at = window.cursor_position()?;
                let (x, y) = view.to_units(f64::from(at.x), f64::from(at.y));
                Some((x.round() as i32, y.round() as i32))
            })
            .flatten(),
    };
    if last.as_ref() == Some(&now) {
        return;
    }
    // The plate shown before this change, read only by the `Plate::None` arm:
    // a plate the pointer has just left is removed according to its own kind,
    // not the kind of whatever the pointer moved onto.
    let previous = last.as_ref().map_or(Plate::None, |latch| latch.plate);
    *last = Some(now);
    let live = world.live();
    // The host records errors as it does for every other chunk it runs; a
    // failing plate must not stop the frame.
    let _ = host.run(&live, |lua| {
        let chunk = lua
            .load(plate.body(previous))
            .set_name("mouseover")
            .into_function()?;
        // The name and the lock line are passed as arguments, never
        // interpolated into the chunk. A template name is server data and may
        // contain a quote; pasting it into Lua source would let the server
        // inject code.
        let chunk = match plate {
            Plate::Object | Plate::Floating => chunk.bind(lock_arguments(&object.plate, &object.name))?,
            _ => chunk,
        };
        frames::protected(lua, &chunk)
    });
}

/// The ten values both world-plate chunks take, in the order they unpack them.
///
/// A flat tuple rather than a table, because `bind` takes a tuple and a table
/// would need building for no benefit, and because the two chunks differ only
/// in the anchor: the same ten names and the same two `if`s.
///
/// A colour is three numbers rather than a `|cff` prefix. The 1.12.1 client
/// colours these lines through `AddLine`'s colour arguments, and a markup
/// prefix would appear in `GameTooltipTextLeft2:GetText()`, where an addon
/// reads it.
type LockArguments = (String, bool, f32, f32, f32, Option<&'static str>, String, f32, f32, f32);

/// [`LockArguments`], out of the judged plate.
fn lock_arguments(plate: &crate::interface::object::LockPlate, name: &str) -> LockArguments {
    let [lr, lg, lb] = plate.locked_colour;
    let [rr, rg, rb] = plate.requires_colour;
    // A key with no argument draws nothing; the 1.12.1 client draws no line
    // for an item name that has not arrived. This is decided here rather than
    // in the chunk so that the Lua stays two plain `if`s.
    let key = plate.requires_key.filter(|_| !plate.requires.is_empty());
    (
        name.to_string(),
        plate.locked,
        lr,
        lg,
        lb,
        key,
        plate.requires.clone(),
        rr,
        rg,
        rb,
    )
}

/// What [`show_world_tooltip`] compares between frames. The plate is rebuilt
/// when its content or position would differ, and not otherwise.
#[derive(PartialEq)]
struct Latch {
    /// Which plate this frame drew, kept so that the next change knows what it
    /// is removing. A floating plate is hidden at once and every other kind
    /// fades; see [`Plate::body`]'s last two arms.
    plate: Plate,
    guid: Option<u64>,
    name: String,
    /// Both lines under the name, colours included, because either can change
    /// while the guid and the name do not. A strongbox that has just been
    /// picked loses its "Locked" line, and a vein changes colour when a skill
    /// point is gained.
    lock: crate::interface::object::LockPlate,
    /// The pointer position in whole interface units, set only for a plate
    /// that follows the pointer. See the note where it is set in
    /// [`show_world_tooltip`].
    at: Option<(i32, i32)>,
}

/// Which of the four world plates the pointer wants.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Plate {
    Unit,
    /// A game object a click acts on: a door, a chest, an ore vein. Its plate
    /// goes where a unit's goes.
    Object,
    /// A game object that can only be looked at, such as a street sign, whose
    /// plate follows the pointer. This is `GAMEOBJECT_TYPE_GENERIC`'s
    /// `floatingTooltip`; see [`vale_assets::look::object::hover_of`].
    Floating,
    None,
}

impl Plate {
    /// The Lua chunk for this plate: the game's calls in the game's order.
    ///
    /// A game object's plate is composed here rather than by a
    /// `GameTooltip:Set*` method, because 1.12 has none. The ninety FrameXML
    /// files use `SetUnit`, `SetBagItem`, `SetLootItem` and thirteen more, and
    /// none for a game object; the 1.12.1 client composes that plate itself,
    /// and this code does the same. The words are not copied into Rust:
    /// `LOCKED_WITH_SPELL_KNOWN` is a `GlobalStrings.lua` key, read from the
    /// interface's globals.
    ///
    /// Which of that file's three lock strings is used, and its colour, is
    /// decided in Rust by [`crate::interface::object::LockPlate`], which
    /// follows the client. The chunk receives a key and looks it up with
    /// `getglobal`, so the wording comes from the archive and the choice
    /// follows the client.
    ///
    /// `LOCKED_WITH_SPELL`, `LOCKED_WITH_SPELL_KNOWN` and `LOCKED_WITH_ITEM`
    /// all read `"Requires %s"` in 1.12, so always picking one of them gives
    /// the right words in the wrong colour. The colour is what tells the player
    /// whether they meet the requirement. The same applies to the `LOCKED`
    /// line above it.
    fn body(self, previous: Plate) -> &'static str {
        match self {
            Plate::Unit => {
                "GameTooltip_SetDefaultAnchor(GameTooltip, UIParent);                  GameTooltip:SetUnit(\"mouseover\");"
            }
            Plate::Object => {
                "local name, locked, lr, lg, lb, key, arg, rr, rg, rb = ...; \
                 GameTooltip_SetDefaultAnchor(GameTooltip, UIParent); \
                 GameTooltip:SetText(name); \
                 if ( locked ) then GameTooltip:AddLine(LOCKED, lr, lg, lb); end \
                 if ( key ) then \
                 GameTooltip:AddLine(format(getglobal(key), arg), rr, rg, rb); end \
                 GameTooltip:Show();"
            }
            // The floating plate: `SetDefaultAnchor` with a different point.
            // None of the ninety FrameXML files uses an `ANCHOR_CURSOR`; the
            // 1.12.1 client positions world plates itself, and this code does
            // the same. It does what `GameTooltip_SetDefaultAnchor` in
            // `GameTooltip.lua` does: own the tooltip with `ANCHOR_NONE`, place
            // it explicitly, and set `default` so that `Plate::None` below can
            // remove it.
            //
            // `GetCursorPosition()` already returns interface space (origin
            // bottom left, y up, scale divided out), which is what `SetPoint`'s
            // offsets take, so no conversion is needed. `lua::api::mouse`
            // explains why the directory's callers divide by
            // `GetEffectiveScale()` again and this code does not.
            //
            // The 14-unit offset is this project's choice: the plate's
            // bottom-left corner sits up and right of the pointer so the
            // pointer does not cover the tooltip. The plate is not clamped to
            // the screen, so a sign hovered within a plate's height of the top
            // edge draws partly off screen. The 1.12.1 client flips the anchor
            // there; this code does not yet.
            Plate::Floating => {
                "local name, locked, lr, lg, lb, key, arg, rr, rg, rb = ...; \
                 local x, y = GetCursorPosition(); \
                 GameTooltip:SetOwner(UIParent, \"ANCHOR_NONE\"); \
                 GameTooltip:SetPoint(\"BOTTOMLEFT\", \"UIParent\", \"BOTTOMLEFT\", x + 14, y + 14); \
                 GameTooltip.default = 1; \
                 GameTooltip:SetText(name); \
                 if ( locked ) then GameTooltip:AddLine(LOCKED, lr, lg, lb); end \
                 if ( key ) then \
                 GameTooltip:AddLine(format(getglobal(key), arg), rr, rg, rb); end \
                 GameTooltip:Show();"
            }
            // The pointer has left; the plate fades rather than disappearing
            // at once. The 1.12.1 client uses `FadeOut` rather than `Hide` here:
            // `UnitFrame_OnLeave` calls `FadeOut` when newbie tips are off, and
            // losing the world mouseover is the same event handled by the
            // client rather than by a frame. See [`FADE_HOLD`].
            //
            // The call is still conditional on `default`: a plate a button owns
            // must not fade because the pointer left a unit behind the button.
            //
            // A floating plate is the exception and is hidden at once. It is
            // positioned at the cursor, not at a frame, so a three-second fade
            // would leave a name hanging beside a sign the pointer has left;
            // and because the plate is rebuilt on every pointer move, sweeping
            // across a row of signs would leave a fading plate at each. `Hide`
            // goes through [`dropped`], which clears the fade mark and the
            // alpha, as for every other way a tooltip is hidden.
            Plate::None if previous == Plate::Floating => {
                "if ( GameTooltip.default ) then GameTooltip:Hide(); end"
            }
            Plate::None => "if ( GameTooltip.default ) then GameTooltip:FadeOut(); end",
        }
    }
}

/// The methods installed once on the shared frame table, sorted. Counted by
/// `vale framexml` beside every other module's list.
pub const METHODS: [&str; 11] = [
    "AddDoubleLine",
    "AddLine",
    "ClearLines",
    "FadeOut",
    "GetOwner",
    "IsOwned",
    "NumLines",
    "SetMinimumWidth",
    "SetOwner",
    "SetPadding",
    "SetText",
];

/// The methods installed per scope, because they read the live world.
pub const SCOPED_METHODS: [&str; 24] = [
    "SetAction",
    "SetBagItem",
    "SetBuybackItem",
    "SetHyperlink",
    "SetInboxItem",
    "SetInventoryItem",
    "SetLootItem",
    "SetLootRollItem",
    "SetMerchantItem",
    "SetPetAction",
    "SetPlayerBuff",
    "SetQuestItem",
    "SetQuestLogItem",
    "SetQuestLogRewardSpell",
    "SetQuestRewardSpell",
    "SetSendMailItem",
    "SetSpell",
    "SetTalent",
    "SetTradePlayerItem",
    "SetTradeTargetItem",
    "SetTrainerService",
    "SetUnit",
    "SetUnitBuff",
    "SetUnitDebuff",
];

/// Keys of the state a tooltip stores on its own table. They start with an
/// underscore, like every field the host owns.
const LINES_KEY: &str = "__tipLines";
const OWNER_KEY: &str = "__tipOwner";
const MIN_WIDTH_KEY: &str = "__tipMinWidth";
const PADDING_KEY: &str = "__tipPadding";
/// The line pool: two arrays of `FontString` tables, left and right columns.
/// Held directly rather than looked up by name on each call, because an
/// unnamed tooltip (allowed from Lua) has no names to look its lines up by.
const LEFT_KEY: &str = "__tipLeft";
const RIGHT_KEY: &str = "__tipRight";

/// The two region fields the size estimate reads. They are named here rather
/// than imported, for the reason [`super::super::api::stubs`] gives for its
/// own copies: this module writes neither.
const FONT_HEIGHT_KEY: &str = "__fontHeight";
const FONT_KEY: &str = "__font";
/// The region field this module does write, through [`regions::set_wrap`];
/// the size estimate reads it back.
const WRAP_KEY: &str = "__wrap";

/// The template's geometry: `TextLeft1` at `(10, -10)` and the chain offset
/// `-2`. See the module comment. These values come from the file.
const PAD: f64 = 10.0;
const LINE_GAP: f64 = 2.0;
/// The space between the two columns of a double line. This value is chosen,
/// not read from a file: the template's right-column anchors are placeholders
/// the layout overwrites, and no file states the gap.
const COLUMN_GAP: f64 = 10.0;
/// The width at which a wrapped line breaks, in pixels of text per row. This
/// value is an approximation: the client's wrap width comes from its line
/// layout and no file states it. It was matched by eye against the newbie
/// tooltips, the corpus' main callers with wrap=1. Without any wrap width,
/// `GameTooltip_AddNewbieTip`'s explanation line made the plate as wide as the
/// sentence, 900 px across the bottom of the screen.
const WRAP_WIDTH: f64 = 280.0;
/// A line whose font never resolved still occupies a row of 12, the same
/// fallback the rest of this directory uses.
const DEFAULT_LINE_HEIGHT: f64 = 12.0;

/// The client's default text colour, gold, `0xffffd200`. See the module
/// comment for the old `AddLine` form that makes it visible.
const GOLD: [f64; 4] = [1.0, 210.0 / 255.0, 0.0, 1.0];
/// The spell tooltip's two other colours: white for the name and gray for the
/// rank column.
const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];
const GRAY: [f64; 4] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0];
/// The red used for a requirement the character does not meet, defined here
/// only for the test that checks it: the plate draws
/// [`crate::interface::plate::Ink::Red`]. It is written out independently so
/// that a change to that colour fails a test.
#[cfg(test)]
const RED: [f64; 4] = [1.0, 32.0 / 255.0, 32.0 / 255.0, 1.0];

/// How long a plate the pointer has left stays fully visible, and how long it
/// then takes to fade. Seconds; the fade between them is linear.
///
/// `GameTooltip:FadeOut()` is implemented by the client, not in Lua. It
/// appears once in the 175 files, in `UnitFrame_OnLeave`, and only when newbie
/// tips are off; with them on, the same body calls `Hide()`. No archive file
/// defines it, so no archive file states its two durations.
///
/// These numbers are taken from other parts of the interface, not measured.
/// The archives state the shape of the fade and one instance of it:
///
/// * The shape is `FadingFrame.lua`'s: fade in, hold, linear fade out, then
///   hide. `UIParent.lua`'s `UIFrameFade` is also linear; nothing in the
///   game's interface eases.
/// * The numbers are `ZoneText.xml`'s, the one hold-then-fade the directory
///   states: `ZoneHoldDuration = 1.0`, `ZoneFadeOutDuration = 2.0`. (`UI.xsd`
///   gives a `MessageFrame` a default `fadeDuration` of 3.0 after a
///   `displayDuration` of 10.0: the same shape, slower, for scrolling text.)
///
/// The fade-in is zero: the 1.12.1 client shows the plate at once, and a
/// tooltip that faded in would show the fade on every hover.
const FADE_HOLD: f64 = 1.0;
const FADE_OUT: f64 = 2.0;

/// How far into the fade a frame is, in seconds. Absent on a frame that is not
/// fading, which is nearly every frame nearly all the time, so the hooks that
/// cancel a fade can check it with one raw read.
const FADE_KEY: &str = "__fading";

/// The registry key of the list of frames with a fade in progress.
///
/// The registry rather than a global, for [`super::super::api::update`]'s
/// reason: interface code must not be able to stop the fade with one
/// assignment. A list rather than a walk, for [`super::slider`]'s reason:
/// visiting 3,785 frames per tick to find the zero or one that is fading costs
/// too much, and the two sweeps that run beside this one use the same shape.
const REG_FADING: &str = "vale.fadingFrames";

/// Whether this object is a `GameTooltip`. The kind check for [`dropped`],
/// which is called from the shared `Hide` that every frame uses.
fn is_tooltip(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<String>>(super::widget::KIND_KEY)
        .ok()
        .flatten()
        .is_some_and(|kind| kind == "GameTooltip")
}

/// The two line arrays, created on first use.
fn columns(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<(mlua::Table, mlua::Table)> {
    let mut out = Vec::with_capacity(2);
    for key in [LEFT_KEY, RIGHT_KEY] {
        let list = match this.get::<Option<mlua::Table>>(key)? {
            Some(list) => list,
            None => {
                let list = lua.create_table()?;
                this.set(key, list.clone())?;
                list
            }
        };
        out.push(list);
    }
    Ok((out.remove(0), out.remove(0)))
}

/// Line pair `index`, using the template's declared lines before creating new
/// ones.
///
/// The declared regions are named `<name>TextLeft<i>` / `TextRight<i>` and are
/// already this frame's children. A pair past the declared ladder is created
/// in the same form: named, `ARTWORK`, the left line anchored under the
/// previous left, and the fonts copied from the line above so that a new line
/// keeps the template's text font instead of having none.
fn line_pair(
    lua: &mlua::Lua,
    this: &mlua::Table,
    index: usize,
) -> mlua::Result<(mlua::Table, mlua::Table)> {
    let (lefts, rights) = columns(lua, this)?;
    while lefts.raw_len() < index {
        let i = lefts.raw_len() + 1;
        let name = this.raw_get::<Option<String>>(super::widget::NAME_KEY)?;
        let previous: Option<mlua::Table> = lefts.get(i.saturating_sub(1))?;

        let mut pair = Vec::with_capacity(2);
        for side in ["Left", "Right"] {
            // Use the declared region if the loader created one: a global
            // whose parent is this frame. The parent is checked so that a
            // name collision cannot take another tooltip's line.
            let declared = name
                .as_deref()
                .and_then(|name| {
                    lua.globals()
                        .get::<Option<mlua::Table>>(format!("{name}Text{side}{i}"))
                        .ok()
                        .flatten()
                })
                .filter(|region| {
                    region
                        .raw_get::<Option<mlua::Table>>(super::widget::PARENT_KEY)
                        .ok()
                        .flatten()
                        .is_some_and(|parent| parent == *this)
                });
            let region = match declared {
                Some(region) => region,
                None => {
                    let full = name.as_deref().map(|name| format!("{name}Text{side}{i}"));
                    let region = regions::create(
                        lua,
                        "FontString",
                        full.as_deref(),
                        Some(this.clone()),
                        Some("ARTWORK"),
                    )?;
                    // The fonts are copied from the line above; with no line
                    // above (a tooltip created from Lua) the defaults remain.
                    if let Some(previous) = &previous {
                        if let Some(font) = previous.raw_get::<Option<String>>(FONT_KEY)? {
                            regions::set_font(lua, &region, &font)?;
                        }
                        if let Some(height) = previous.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)? {
                            regions::set_font_height(lua, &region, height as f32)?;
                        }
                    }
                    if side == "Left" {
                        match &previous {
                            Some(previous) => widget::add_point(
                                lua,
                                &region,
                                "TOPLEFT",
                                Some(mlua::Value::Table(previous.clone())),
                                Some("BOTTOMLEFT"),
                                (0.0, -(LINE_GAP as f32)),
                            )?,
                            None => widget::add_point(
                                lua,
                                &region,
                                "TOPLEFT",
                                Some(mlua::Value::Table(this.clone())),
                                Some("TOPLEFT"),
                                (PAD as f32, -(PAD as f32)),
                            )?,
                        }
                    }
                    region
                }
            };
            write_shown(lua, &region, false)?;
            pair.push(region);
        }
        lefts.push(pair[0].clone())?;
        rights.push(pair[1].clone())?;
    }
    Ok((lefts.get(index)?, rights.get(index)?))
}

/// The client's truth test for an optional flag argument; see
/// [`super::super::api::to_boolean`], which follows the client's coercion
/// rather than Lua's. The corpus passes `1` here; the difference from Lua's
/// rule is how `0` is treated.
fn truthy(value: Option<&mlua::Value>) -> bool {
    super::super::api::to_boolean(value, true)
}

/// A colour component as the client's tooltip methods accept one: the same
/// coercion as Lua's `lua_tonumber`, so a numeric string counts and `""` does
/// not.
fn component(value: Option<&mlua::Value>) -> Option<f64> {
    match value {
        Some(mlua::Value::Integer(n)) => Some(*n as f64),
        Some(mlua::Value::Number(n)) => Some(*n),
        Some(mlua::Value::String(s)) => s.to_string_lossy().trim().parse().ok(),
        _ => None,
    }
}

/// The colour arguments apply only when the r argument is a number; otherwise
/// all three are ignored and the line is the default gold. When r is a number,
/// g and b are read unconditionally and default to 0. See the module comment:
/// the zone tooltips pass `""` as r, so applying the colour regardless would
/// draw them white instead of gold.
fn colour_or_gold(args: &[mlua::Value], at: usize) -> [f64; 4] {
    match component(args.get(at)) {
        Some(r) => [
            r,
            component(args.get(at + 1)).unwrap_or(0.0),
            component(args.get(at + 2)).unwrap_or(0.0),
            1.0,
        ],
        None => GOLD,
    }
}

/// Write one cell: text (through the shared stringify function), colour, wrap,
/// shown.
fn write_cell(
    lua: &mlua::Lua,
    region: &mlua::Table,
    text: mlua::Value,
    colour: [f64; 4],
    wrap: bool,
) -> mlua::Result<()> {
    regions::set_text_value(lua, region, text)?;
    regions::set_colour(lua, region, colour)?;
    regions::set_wrap(lua, region, wrap)?;
    write_shown(lua, region, true)
}

/// Write a shown flag directly, disturbing the pile only when the stored value changes.
fn write_shown(lua: &mlua::Lua, object: &mlua::Table, shown: bool) -> mlua::Result<()> {
    if object.raw_get::<mlua::Value>(super::widget::SHOWN_KEY)? != mlua::Value::Boolean(shown) {
        object.set(super::widget::SHOWN_KEY, shown)?;
        super::widget::disturb_pile(lua);
    }
    Ok(())
}

/// Append one line (both columns; the right one optional; the wrap flag
/// applies to the left) and re-solve the plate. Does not show the tooltip:
/// in the 1.12.1 client `AddLine` does not show and `SetText` does (module
/// comment).
fn append(
    lua: &mlua::Lua,
    this: &mlua::Table,
    left: (mlua::Value, [f64; 4]),
    right: Option<(mlua::Value, [f64; 4])>,
    wrap: bool,
) -> mlua::Result<()> {
    let index = lines(this) + 1;
    let (left_cell, right_cell) = line_pair(lua, this, index)?;
    write_cell(lua, &left_cell, left.0, left.1, wrap)?;
    if let Some((text, colour)) = right {
        write_cell(lua, &right_cell, text, colour, false)?;
    }
    this.set(LINES_KEY, index as i64)?;
    reflow(lua, this)
}

/// How many lines the tooltip currently holds.
fn lines(this: &mlua::Table) -> usize {
    this.raw_get::<Option<i64>>(LINES_KEY)
        .ok()
        .flatten()
        .unwrap_or(0)
        .max(0) as usize
}

/// Blank every line and fire `OnTooltipCleared`. The minimum width belongs to
/// the content and is reset with it; the padding is a frame property and is
/// kept.
fn clear(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    let (lefts, rights) = columns(lua, this)?;
    for list in [lefts, rights] {
        for region in list.sequence_values::<mlua::Table>().flatten() {
            write_shown(lua, &region, false)?;
            regions::set_text_value(lua, &region, mlua::Value::Nil)?;
            regions::set_wrap(lua, &region, false)?;
        }
    }
    this.set(LINES_KEY, 0_i64)?;
    this.set(MIN_WIDTH_KEY, 0.0_f64)?;
    // A plate that is being repopulated is not fading. Every population goes
    // through this function: `SetOwner` (and so `GameTooltip_SetDefaultAnchor`,
    // every world plate and every `UnitFrame_OnEnter`), `SetText`, and `Hide`
    // through [`dropped`]. Cancelling here rather than in each caller keeps the
    // alpha and fade mark restored in one place.
    cancel_fade(lua, this)?;
    // The error is ignored, as `Show` ignores `OnShow`'s: the state is cleared
    // either way, and a failing handler must not fail the `SetOwner` that
    // called this.
    let _ = frames::run_script(lua, this, "OnTooltipCleared", &[]);
    Ok(())
}

/// One line's cell sizes: `(width, height)`, `(0, 0)` for a cell that holds
/// nothing.
///
/// Widths are measured, not estimated; see [`regions::text_width`]. The
/// earlier estimate of half the font height per character, against Friz
/// Quadrata's mean advance of 0.609 em, made plates narrower than their text.
fn cell_size(lua: &mlua::Lua, region: &mlua::Table) -> (f64, f64) {
    if !region
        .raw_get::<Option<bool>>(super::widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
    {
        return (0.0, 0.0);
    }
    let Some(text) = regions::text_of(region) else {
        return (0.0, 0.0);
    };
    // A row is the face's line box, not the declared font height: 14.6 for
    // Friz Quadrata at 12. `<FontHeight>` is the font size; the line box is
    // the space a line of it occupies. Using the font height as the row
    // height placed the tooltip's lines 2.6 units closer together than their
    // glyphs need.
    let height = regions::line_height_of(lua, region).max(DEFAULT_LINE_HEIGHT);
    if text.is_empty() {
        // The corpus' `AddLine(" ")` spacer is a real row; an empty text still
        // takes a row so that `NumLines` and the height agree.
        return (0.0, height);
    }
    let measured = regions::text_width(lua, region, &text);
    // A wrapped cell breaks at [`WRAP_WIDTH`] and takes the number of rows it
    // actually wraps into, counted by [`regions::text_rows`], the same
    // measurement the painter uses to lay out the text.
    //
    // `ceil(measured / WRAP_WIDTH)` gives the row count only if every break
    // falls exactly at the wrap width. In prose, a word that does not fit
    // leaves the end of its row empty, so that formula is one row short from
    // about three rows on and the plate is one line too short. Shield Bash's
    // description is 47 words over four rows; the formula gave three, and
    // "for 6 sec." was drawn below the bottom border.
    let wrapped = region
        .raw_get::<Option<bool>>(WRAP_KEY)
        .ok()
        .flatten()
        .unwrap_or(false);
    if wrapped && measured > WRAP_WIDTH {
        let rows = regions::text_rows(lua, region, &text, WRAP_WIDTH).max(1);
        return (WRAP_WIDTH, rows as f64 * height);
    }
    (measured, height)
}

/// Lay out the plate: size every cell, anchor each right cell to the plate's
/// right edge, and size the tooltip. Runs after every content change; a
/// tooltip has only a few lines, so this is arithmetic, not a layout walk.
fn reflow(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    let (lefts, rights) = columns(lua, this)?;
    let count = lines(this);
    let mut width: f64 = 0.0;
    let mut height: f64 = 0.0;
    for index in 1..=count {
        let left: Option<mlua::Table> = lefts.get(index)?;
        let right: Option<mlua::Table> = rights.get(index)?;
        let (lw, lh) = left.as_ref().map_or((0.0, 0.0), |cell| cell_size(lua, cell));
        let (rw, rh) = right.as_ref().map_or((0.0, 0.0), |cell| cell_size(lua, cell));

        if let Some(left) = &left {
            widget::set_size(lua, left, Some(lw as f32), Some(lh.max(DEFAULT_LINE_HEIGHT) as f32))?;
        }
        if let Some(right) = right.filter(|_| rw > 0.0) {
            widget::set_size(lua, &right, Some(rw as f32), Some(rh as f32))?;
            // The template's right-column anchor is a placeholder the layout
            // replaces. Here it is replaced by one point: the cell's top-right
            // on the plate's top-right, lowered to this line's depth.
            right.set(super::widget::POINTS_KEY, lua.create_table()?)?;
            widget::add_point(
                lua,
                &right,
                "TOPRIGHT",
                Some(mlua::Value::Table(this.clone())),
                Some("TOPRIGHT"),
                (-(PAD as f32), -((PAD + height) as f32)),
            )?;
        }

        let line_width = lw + if rw > 0.0 { COLUMN_GAP + rw } else { 0.0 };
        width = width.max(line_width);
        if index > 1 {
            height += LINE_GAP;
        }
        height += lh.max(rh).max(if lw + rw > 0.0 { 0.0 } else { DEFAULT_LINE_HEIGHT });
    }

    let floor = this.raw_get::<Option<f64>>(MIN_WIDTH_KEY)?.unwrap_or(0.0);
    let padding = this.raw_get::<Option<f64>>(PADDING_KEY)?.unwrap_or(0.0);
    widget::set_size(
        lua,
        this,
        Some((width.max(floor) + 2.0 * PAD + padding) as f32),
        Some((height + 2.0 * PAD) as f32),
    )
}

/// The registry list of frames with a fade running, created on first use.
fn fading(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    match lua.named_registry_value::<Option<mlua::Table>>(REG_FADING)? {
        Some(list) => Ok(list),
        None => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(REG_FADING, list.clone())?;
            Ok(list)
        }
    }
}

/// `FadeOut()`: hold this plate, then fade it out and hide it.
///
/// It does not clear the lines or the owner; that is the difference from
/// `Hide`. The plate must keep its content while it is on screen.
/// [`fade_sweep`] calls `Hide` at the end, and the owner and lines are
/// cleared there, on the same path as any other `Hide`.
///
/// Fading a hidden plate does nothing, since `Hide` has already cleared it.
/// Fading a plate that is already fading does not restart the hold, because
/// the one caller in the directory is an `OnLeave`, which a pointer moving
/// along an edge fires repeatedly.
fn begin_fade(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if !this
        .raw_get::<Option<bool>>(super::widget::SHOWN_KEY)?
        .unwrap_or(true)
    {
        return Ok(());
    }
    if this.raw_get::<Option<f64>>(FADE_KEY)?.is_some() {
        return Ok(());
    }
    this.set(FADE_KEY, 0.0_f64)?;
    super::widget::set_paint(lua, this, super::widget::ALPHA_KEY, 1.0_f64)?;
    let list = fading(lua)?;
    list.raw_push(this.clone())
}

/// Stop a fade and restore the alpha. Anything that shows or repopulates the
/// frame must do this.
///
/// Two hooks call it, and together they cover every path: [`clear`], which
/// every population and every `Hide` goes through, and the shared `Show`
/// through [`unfade`].
fn cancel_fade(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if this.raw_get::<Option<f64>>(FADE_KEY)?.is_none() {
        return Ok(());
    }
    this.set(FADE_KEY, mlua::Value::Nil)?;
    super::widget::set_paint(lua, this, super::widget::ALPHA_KEY, 1.0_f64)?;
    let list = fading(lua)?;
    for (index, entry) in list.sequence_values::<mlua::Table>().enumerate() {
        if entry? == *this {
            return list.raw_remove(index + 1);
        }
    }
    Ok(())
}

/// The `Show` hook, the counterpart of [`dropped`]: a frame that is shown
/// again stops fading. Called from the shared `Show` for every object, so the
/// check is one raw read of a key almost no frame has.
///
/// It does not check `is_tooltip`: `FadeOut` is in the shared method table,
/// so any frame can be told to fade, and any frame that fades must be able to
/// stop.
pub(super) fn unfade(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    cancel_fade(lua, this)
}

/// Advance every fade by one interface tick.
///
/// Called from [`super::super::api::update::fire`] beside
/// [`super::scrollframe::sweep`] and [`super::slider::sweep`], for the same
/// reason: every other fade in the game runs on this clock (`UIParent`'s single
/// `OnUpdate` walks `FADEFRAMES`), and a plate on a different clock would fade
/// at a different speed from the chat frame beside it. It runs before that
/// pass's early return, because a fading tooltip needs no `OnUpdate` handler.
///
/// `elapsed` is the tick's seconds, not the rendered frame's; see
/// [`super::super::api::update::InterfaceClock::advance`]. The fade is
/// integrated rather than sampled, so it takes the same wall-clock time at
/// any frame rate.
pub(in crate::lua) fn fade_sweep(lua: &mlua::Lua, elapsed: f64) {
    let Ok(list) = lua.named_registry_value::<mlua::Table>(REG_FADING) else {
        return;
    };
    if list.raw_len() == 0 {
        return;
    }
    // A snapshot, for the same reason the `OnUpdate` walk takes one: finishing
    // a fade calls `Hide`, which runs `OnHide` handlers, which may show or hide
    // any frame, including another fading one.
    let frames: Vec<mlua::Table> = list.sequence_values::<mlua::Table>().flatten().collect();
    for frame in frames {
        // Re-read rather than trusted from the snapshot: an earlier frame's
        // `OnHide` may have cancelled this one.
        let Ok(Some(at)) = frame.raw_get::<Option<f64>>(FADE_KEY) else {
            continue;
        };
        let at = at + elapsed.max(0.0);
        if at >= FADE_HOLD + FADE_OUT {
            // `Hide` calls [`dropped`] and so [`clear`], which removes the
            // fade mark and restores the alpha. This code does not do either
            // itself, so that the logic stays in one place.
            let _ = set_shown(&frame, false);
            let _ = cancel_fade(lua, &frame);
            continue;
        }
        let alpha = if at <= FADE_HOLD {
            1.0
        } else {
            1.0 - (at - FADE_HOLD) / FADE_OUT
        };
        let _ = frame.set(FADE_KEY, at);
        let _ = super::widget::set_paint(lua, &frame, super::widget::ALPHA_KEY, alpha.clamp(0.0, 1.0));
    }
}

/// Show or hide through the frame's own installed method, so the flag change,
/// the dedupe and `OnShow`/`OnHide` are the ones every other caller gets.
fn set_shown(this: &mlua::Table, shown: bool) -> mlua::Result<()> {
    let method: Option<mlua::Function> = this.get(if shown { "Show" } else { "Hide" })?;
    match method {
        Some(method) => method.call::<()>(this.clone()),
        None => Ok(()),
    }
}

/// An empty population hides the plate and keeps the owner, so refresh loops
/// (`UnitFrame_OnUpdate`'s `IsOwned` check) still see their ownership; only an
/// explicit `Hide` clears it. The shown flag is written directly because the
/// `Hide` method calls [`dropped`], which would clear the owner.
fn conceal(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if this
        .raw_get::<Option<bool>>(super::widget::SHOWN_KEY)?
        .unwrap_or(true)
    {
        this.set(super::widget::SHOWN_KEY, false)?;
        super::widget::disturb_pile(lua);
        let _ = frames::run_script(lua, this, "OnHide", &[]);
    }
    Ok(())
}

/// The `Hide` hook: a hidden tooltip clears its owner and lines and fires
/// `OnTooltipCleared`. Called by the shared `Hide` on every frame; the kind
/// check is here so the caller stays one line.
pub(super) fn dropped(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if !is_tooltip(this) {
        return Ok(());
    }
    this.set(OWNER_KEY, mlua::Value::Nil)?;
    clear(lua, this)
}

/// Install the static methods. Called from [`super::frames::register_methods`],
/// before the stubs so nothing here can be shadowed by one.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // SetOwner(owner, "ANCHOR_RIGHT", x, y): store the owner, clear the old
    // contents (a new owner never keeps the previous hover's lines), and
    // anchor the plate to the owner according to the anchor word.
    let set_owner = lua.create_function(
        |lua,
         (this, owner, anchor, x, y): (
            mlua::Table,
            Option<mlua::Table>,
            Option<String>,
            Option<f64>,
            Option<f64>,
        )| {
            clear(lua, &this)?;
            this.set(OWNER_KEY, owner.clone())?;
            let word = anchor.unwrap_or_default().to_ascii_uppercase();
            // (this corner, on the owner's corner), as described in the
            // module comment. An unknown word uses the most common anchor
            // rather than none, so a typo shows a misplaced tooltip instead
            // of an invisible one.
            let points = match word.as_str() {
                "ANCHOR_NONE" => {
                    // The caller anchors it. The previous owner anchor is
                    // removed now, or it would remain if the caller does not
                    // set one.
                    widget::clear_points(lua, &this)?;
                    None
                }
                "ANCHOR_PRESERVE" => None,
                "ANCHOR_LEFT" => Some(("BOTTOMRIGHT", "TOPLEFT")),
                "ANCHOR_TOPRIGHT" => Some(("BOTTOMRIGHT", "TOPRIGHT")),
                "ANCHOR_TOPLEFT" => Some(("BOTTOMLEFT", "TOPLEFT")),
                "ANCHOR_BOTTOMRIGHT" => Some(("TOPLEFT", "BOTTOMRIGHT")),
                "ANCHOR_BOTTOMLEFT" => Some(("TOPRIGHT", "BOTTOMLEFT")),
                _ => Some(("BOTTOMLEFT", "TOPRIGHT")),
            };
            if let (Some((own, relative)), Some(owner)) = (points, owner) {
                // One call that compares before writing, not a clear and a
                // set; see [`widget::set_only_point`].
                // `ContainerFrameItemButton_OnUpdate` re-runs `OnEnter` every
                // frame the pointer is over a bag slot, so a `SetOwner` that
                // always invalidated discarded the whole interface's solved
                // layout every frame.
                widget::set_only_point(
                    lua,
                    &this,
                    own,
                    Some(mlua::Value::Table(owner)),
                    Some(relative),
                    (x.unwrap_or(0.0), y.unwrap_or(0.0)),
                )?;
            }
            Ok(())
        },
    )?;
    methods.set("SetOwner", set_owner)?;

    let get_owner = lua.create_function(|_, this: mlua::Table| {
        this.raw_get::<mlua::Value>(OWNER_KEY)
    })?;
    methods.set("GetOwner", get_owner)?;

    // IsOwned(frame): whether that frame is the current owner. The refresh
    // loops check this.
    let is_owned = lua.create_function(|_, (this, frame): (mlua::Table, Option<mlua::Table>)| {
        let owner: Option<mlua::Table> = this.raw_get(OWNER_KEY)?;
        Ok(one_or_nil(match (owner, frame) {
            (Some(owner), Some(frame)) => owner == frame,
            _ => false,
        }))
    })?;
    methods.set("IsOwned", is_owned)?;

    // SetText(text, r, g, b, a, wrap): clear, write line 1, and show. The
    // text is required; without it the error is the 1.12.1 client's usage
    // message.
    //
    // One method table serves every frame kind, and `SetText` is the one name
    // the tooltip shares with buttons. This closure therefore checks the kind
    // and passes everything that is not a tooltip to the ordinary text setter.
    // Without the check, every `button:SetText` would write tooltip lines
    // instead of a label.
    let set_text = lua.create_function(|lua, (this, args): (mlua::Table, mlua::MultiValue)| {
        let args: Vec<mlua::Value> = args.into_iter().collect();
        if !is_tooltip(&this) {
            let value = args.into_iter().next().unwrap_or(mlua::Value::Nil);
            // The shared implementation, not a second copy of it: an `EditBox`
            // fires `OnTextSet` from it and a `Button` writes its font string,
            // and every non-tooltip frame reaches this branch. See
            // [`regions::set_frame_text`].
            regions::set_frame_text(lua, &this, value)?;
            return Ok(());
        }
        let text = match args.first() {
            Some(v @ (mlua::Value::String(_) | mlua::Value::Number(_) | mlua::Value::Integer(_))) => {
                v.clone()
            }
            _ => {
                return Err(mlua::Error::RuntimeError(
                    "Usage: SetText(\"text\" [, color])".to_string(),
                ))
            }
        };
        let mut colour = colour_or_gold(&args, 1);
        // AddLine is always opaque, but SetText takes an alpha argument:
        // `GameTooltip_AddNewbieTip` passes `(text, r, g, b, 1, 1)`.
        if let Some(alpha) = component(args.get(4)) {
            colour[3] = alpha;
        }
        clear(lua, &this)?;
        append(lua, &this, (text, colour), None, truthy(args.get(5)))?;
        set_shown(&this, true)
    })?;
    methods.set("SetText", set_text)?;

    // AddLine(text, r, g, b, wrap): append, do not show. A wrapping line
    // breaks at the approximate wrap width instead of widening the plate to
    // the sentence; see [`WRAP_WIDTH`].
    let add_line = lua.create_function(|lua, (this, args): (mlua::Table, mlua::MultiValue)| {
        let args: Vec<mlua::Value> = args.into_iter().collect();
        let text = args.first().cloned().unwrap_or(mlua::Value::Nil);
        let colour = colour_or_gold(&args, 1);
        append(lua, &this, (text, colour), None, truthy(args.get(4)))
    })?;
    methods.set("AddLine", add_line)?;

    // AddDoubleLine(left, right, rL, gL, bL, rR, gR, bR): each side's colour
    // applies only if that side's r argument is a number.
    let add_double = lua.create_function(|lua, (this, args): (mlua::Table, mlua::MultiValue)| {
        let args: Vec<mlua::Value> = args.into_iter().collect();
        let left = args.first().cloned().unwrap_or(mlua::Value::Nil);
        let right = args.get(1).cloned().unwrap_or(mlua::Value::Nil);
        let left_colour = colour_or_gold(&args, 2);
        let right_colour = colour_or_gold(&args, 5);
        append(lua, &this, (left, left_colour), Some((right, right_colour)), false)
    })?;
    methods.set("AddDoubleLine", add_double)?;

    let clear_lines = lua.create_function(|lua, this: mlua::Table| clear(lua, &this))?;
    methods.set("ClearLines", clear_lines)?;

    let num_lines = lua.create_function(|_, this: mlua::Table| Ok(lines(&this) as i64))?;
    methods.set("NumLines", num_lines)?;

    // SetMinimumWidth(w): a minimum for the computed width. `SetTooltipMoney`
    // calls it so the coins never extend past the plate.
    let min_width = lua.create_function(|lua, (this, w): (mlua::Table, Option<f64>)| {
        this.set(MIN_WIDTH_KEY, w.unwrap_or(0.0).max(0.0))?;
        reflow(lua, &this)
    })?;
    methods.set("SetMinimumWidth", min_width)?;

    // SetPadding(w): extra width; a frame property that is kept across clears.
    let padding = lua.create_function(|lua, (this, w): (mlua::Table, Option<f64>)| {
        this.set(PADDING_KEY, w.unwrap_or(0.0).max(0.0))?;
        reflow(lua, &this)
    })?;
    methods.set("SetPadding", padding)?;

    // FadeOut: hold the plate, then fade it out. See [`begin_fade`], and
    // [`FADE_HOLD`] for where the two durations come from.
    let fade = lua.create_function(|lua, this: mlua::Table| begin_fade(lua, &this))?;
    methods.set("FadeOut", fade)?;
    Ok(())
}

/// Install the population methods that read the live world, for the length of
/// one scope: the same lifetime and the same reason as every read in
/// [`super::super::api`].
///
/// They are installed through [`crate::lua::scoped`], not directly on the
/// method table. `GameTooltip:SetUnit` and the related methods resolve to
/// these, and an addon can keep one: `libtipscan` reads `v[method]` from the
/// plate and calls it on a later frame to scan a tooltip's text. Written
/// directly into the shared table, that stored function would be a destroyed
/// callback once the scope closes; written through the forwarder it stays
/// callable for the life of the Lua state.
pub(in crate::lua) fn install_scoped<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    // Without the object model there is no method table: the bare-interpreter
    // tests install the reads without frames.
    let Some(methods) = frames::methods(lua) else {
        return Ok(());
    };
    let methods = crate::lua::scoped::methods(lua, &methods)?;

    // SetAction(slot): the spell tooltip, composed as described in the module
    // comment. Returns 1 when there was something to show;
    // `ActionButton_SetTooltip` tests this to keep the refresh timer.
    //
    // For a slot holding an item it shows the item plate instead, through the
    // same `item_lines` a bag slot uses, so a hearthstone on the bar and in
    // the bag show the same plate. The slot's kind byte makes the two cases
    // mutually exclusive, so the order below is not a precedence.
    let set_action = scope.create_function(move |lua, (this, slot): (mlua::Table, Option<u8>)| {
        let slot = slot.unwrap_or(0);
        if let Some(tip) = answers.action_item_tooltip(slot) {
            clear(lua, &this)?;
            item_lines(lua, &this, &tip)?;
            set_shown(&this, true)?;
            return Ok(one_or_nil(true));
        }
        match answers.action_tooltip(slot) {
            Some(tip) => {
                clear(lua, &this)?;
                spell_lines(lua, &this, &tip)?;
                set_shown(&this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, &this)?;
                conceal(lua, &this)?;
                Ok(mlua::Value::Nil)
            }
        }
    })?;
    methods.set("SetAction", set_action)?;

    // SetSpell(id, bookType): the same plate, shown from the spellbook.
    // `SpellButton_OnEnter`'s only line tests the return value: a population
    // that returned nothing must not leave the hover's refresh timer running.
    // `id` is a spellbook row, not a spell id; see
    // [`super::super::panels::spellbook`]. The lines are composed by the same
    // [`spell_lines`], so a spell on the bar and in the book show the same
    // plate.
    let set_spell = scope.create_function(
        move |lua, (this, index, book): (mlua::Table, Option<usize>, Option<String>)| {
            let tip = super::super::panels::spellbook::row(index, book).and_then(|row| answers.spell_tooltip(row));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    spell_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok(one_or_nil(true))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok(mlua::Value::Nil)
                }
            }
        },
    )?;
    methods.set("SetSpell", set_spell)?;

    // SetPetAction(slot): the same plate, shown from the pet bar.
    //
    // This is `PetActionButton_OnEnter`'s whole body for a spell slot. Without
    // this method the `OnEnter` raises an error on the call, and an
    // autocastable pet ability has no tooltip. See
    // [`super::super::panels::pet::PetAnswers::pet_action_tooltip`], which
    // describes how the client treats spell slots and token slots. A token
    // slot never reaches this method, because the body takes its other branch
    // for one.
    let set_pet_action = scope.create_function(move |lua, (this, slot): (mlua::Table, Option<usize>)| {
        let tip = slot.and_then(|slot| answers.pet_action_tooltip(slot));
        match tip {
            Some(tip) => {
                clear(lua, &this)?;
                spell_lines(lua, &this, &tip)?;
                set_shown(&this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, &this)?;
                conceal(lua, &this)?;
                Ok(mlua::Value::Nil)
            }
        }
    })?;
    methods.set("SetPetAction", set_pet_action)?;

    // The profession windows' three plates. `SetTradeSkillItem(skill
    // [, reagent])` and `SetCraftItem(craft, reagent)` show the item plate for
    // an entry (the created item or a reagent, resolved by the panel against
    // the list it drew). `SetCraftSpell(craft)` shows the spell plate, through
    // the same `spell_lines` the spellbook hover uses. An entry the cache has
    // not named yet hides the plate, as in the 1.12.1 client.
    let set_trade_skill_item = scope.create_function(
        move |lua, (this, index, reagent): (mlua::Table, Option<usize>, Option<usize>)| {
            let tip = index
                .filter(|i| *i > 0)
                .and_then(|i| answers.trade_tip_item(i, reagent))
                .and_then(|entry| answers.item_tip(entry));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    item_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok(one_or_nil(true))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok(mlua::Value::Nil)
                }
            }
        },
    )?;
    methods.set("SetTradeSkillItem", set_trade_skill_item)?;

    let set_craft_item = scope.create_function(
        move |lua, (this, index, reagent): (mlua::Table, Option<usize>, Option<usize>)| {
            let tip = index
                .filter(|i| *i > 0)
                .zip(reagent.filter(|j| *j > 0))
                .and_then(|(i, j)| answers.craft_tip_item(i, j))
                .and_then(|entry| answers.item_tip(entry));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    item_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok(one_or_nil(true))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok(mlua::Value::Nil)
                }
            }
        },
    )?;
    methods.set("SetCraftItem", set_craft_item)?;

    let set_craft_spell = scope.create_function(
        move |lua, (this, index): (mlua::Table, Option<usize>)| {
            let tip = index.filter(|i| *i > 0).and_then(|i| answers.craft_spell_tip(i));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    spell_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok(one_or_nil(true))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok(mlua::Value::Nil)
                }
            }
        },
    )?;
    methods.set("SetCraftSpell", set_craft_spell)?;

    // The three item plates, one composition reached three ways: from a bag,
    // from an equipped slot, or from a link with no object behind it. All three
    // use one `item_lines`, as the spell plates share `spell_lines`, so an
    // item in a bag and on the paper doll show the same plate.
    //
    // `SetBagItem` and `SetInventoryItem` both return more than one value, and
    // the callers unpack them by position:
    //
    // ```lua
    // local hasCooldown, repairCost = GameTooltip:SetBagItem(bag, slot);
    // local hasItem, hasCooldown, repairCost = GameTooltip:SetInventoryItem(u, id);
    // ```
    //
    // The two return different shapes; this matches the game's FrameXML.
    // `repairCost` is 0 rather than nil: `ContainerFrame_Update` compares it
    // with `>` two lines later, and a nil is an error there. This client has
    // no repair, so 0 is also the correct value.
    let set_bag_item = scope.create_function(
        move |lua, (this, bag, slot): (mlua::Table, Option<i64>, Option<i64>)| {
            let tip = slot
                .filter(|slot| *slot > 0)
                .and_then(|slot| answers.bag_item_tip(bag.unwrap_or(0) as i32, slot as usize));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    item_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    // `hasCooldown` is nil, because `GetContainerItemCooldown`
                    // still returns a constant zero (see
                    // [`super::super::api::stubs`]).
                    Ok((mlua::Value::Nil, mlua::Value::Integer(0)))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok((mlua::Value::Nil, mlua::Value::Integer(0)))
                }
            }
        },
    )?;
    methods.set("SetBagItem", set_bag_item)?;

    // `SetInventoryItem`'s first return value selects the fallback.
    // `PaperDollItemSlotButton_OnEnter` does `if ( not hasItem ) then
    // GameTooltip:SetText(<the slot's own name>)`, so a nil here makes an
    // empty head slot show "Head". `BagSlotButton_OnEnter` uses the same test
    // to show `EQUIP_CONTAINER` over an empty bag button.
    let set_inventory_item = scope.create_function(
        move |lua, (this, token, id): (mlua::Table, Option<String>, Option<i64>)| {
            let token = token.unwrap_or_default();
            let tip = id
                .filter(|id| *id >= 0)
                .and_then(|id| answers.inventory_item_tip(&token, id as u32));
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    item_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok((one_or_nil(true), mlua::Value::Nil, mlua::Value::Integer(0)))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok((mlua::Value::Nil, mlua::Value::Nil, mlua::Value::Integer(0)))
                }
            }
        },
    )?;
    methods.set("SetInventoryItem", set_inventory_item)?;

    // `SetHyperlink("item:2589:0:0:0")`: the plate with no object behind it,
    // which a link in the chat frame opens. Only `item:` links are handled;
    // this client does not support `spell:` or `quest:` links, and showing
    // the wrong plate for one would be worse than showing none.
    let set_hyperlink =
        scope.create_function(move |lua, (this, link): (mlua::Table, Option<String>)| {
            let tip = link
                .as_deref()
                .and_then(super::super::panels::container::link_fields)
                .and_then(|(entry, enchant, random)| {
                    answers.item_link_tip(entry, enchant, random)
                });
            match tip {
                Some(tip) => {
                    clear(lua, &this)?;
                    item_lines(lua, &this, &tip)?;
                    set_shown(&this, true)?;
                    Ok(one_or_nil(true))
                }
                None => {
                    clear(lua, &this)?;
                    conceal(lua, &this)?;
                    Ok(mlua::Value::Nil)
                }
            }
        })?;
    methods.set("SetHyperlink", set_hyperlink)?;

    // SetUnit(token): the unit tooltip, composed by [`unit_lines`]. The name
    // is not coloured by reaction here; `GameTooltip.xml`'s
    // `UPDATE_MOUSEOVER_UNIT` handler does that.
    //
    // `<PlayerModel>` has a `SetUnit` of its own, and every frame kind shares
    // one methods table, so this registration replaces that one for the length
    // of every scope. A frame that is not a tooltip is handed to the model's
    // method, as `SetText` hands a button to the shared text setter. Without
    // this, `CharacterModelFrame:SetUnit("player")` wrote the player's tooltip
    // lines into the character sheet and the paper doll never learned its unit.
    let set_unit = scope.create_function(move |lua, (this, token): (mlua::Table, Option<String>)| {
        if !is_tooltip(&this) {
            super::model::set_unit(lua, &this, token)?;
            return Ok(mlua::Value::Nil);
        }
        let token = token.unwrap_or_default();
        match answers.unit_tooltip(&token) {
            Some(tip) => {
                clear(lua, &this)?;
                unit_lines(lua, &this, &tip)?;
                set_shown(&this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, &this)?;
                conceal(lua, &this)?;
                Ok(mlua::Value::Nil)
            }
        }
    })?;
    methods.set("SetUnit", set_unit)?;

    // The three aura plates: the name and the description, with nothing
    // between them. An aura's plate has no mana cost, range or cast time,
    // because the aura is not about to be cast, so these do not go through
    // [`spell_lines`]. See [`super::super::panels::auras::tooltip_lines`].
    //
    // `SetPlayerBuff` takes the same handle as the other player-buff
    // functions; a missing or out-of-range handle becomes `-1`. The other two
    // take a unit token and a one-based index into either the helpful or the
    // harmful auras.
    let aura_plate = |lua: &mlua::Lua,
                      this: &mlua::Table,
                      aura: Option<super::super::panels::auras::AuraInfo>|
     -> mlua::Result<mlua::Value> {
        match aura {
            Some(aura) => {
                let (name, description) = super::super::panels::auras::tooltip_lines(&aura);
                clear(lua, this)?;
                append(lua, this, (text(lua, name)?, WHITE), None, false)?;
                if !description.is_empty() {
                    append(lua, this, (text(lua, description)?, GOLD), None, true)?;
                }
                set_shown(this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, this)?;
                conceal(lua, this)?;
                Ok(mlua::Value::Nil)
            }
        }
    };

    let set_player_buff =
        scope.create_function(move |lua, (this, handle): (mlua::Table, Option<i64>)| {
            let handle = handle.unwrap_or(-1).try_into().unwrap_or(-1);
            aura_plate(lua, &this, answers.player_buff_at(handle))
        })?;
    methods.set("SetPlayerBuff", set_player_buff)?;

    let unit_aura = move |helpful: bool| {
        move |lua: &mlua::Lua, (this, token, index): (mlua::Table, Option<String>, Option<usize>)| {
            let aura = answers.unit_aura(
                token.as_deref().unwrap_or(""),
                index.unwrap_or(0),
                helpful,
            );
            aura_plate(lua, &this, aura)
        }
    };
    methods.set("SetUnitBuff", scope.create_function(unit_aura(true))?)?;
    methods.set("SetUnitDebuff", scope.create_function(unit_aura(false))?)?;

    // The five populations the panels call on hover. Without them nothing on
    // a trainer, vendor, corpse or quest page has a plate: the `OnEnter` that
    // calls one stops on a nil method with the tooltip still hidden. That
    // looks the same as hovering an empty slot, and no count records it.
    //
    // Four of the five are the item plate, reached four ways, and all go
    // through the same [`item_lines`] as the bags, so a quest reward and the
    // same item in a bag show the same plate. The fifth is the spell plate,
    // for what a trainer teaches.
    let item_plate = |lua: &mlua::Lua,
                      this: &mlua::Table,
                      entry: Option<u32>|
     -> mlua::Result<mlua::Value> {
        match entry.and_then(|entry| answers.item_tip(entry)) {
            Some(tip) => {
                clear(lua, this)?;
                item_lines(lua, this, &tip)?;
                set_shown(this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, this)?;
                conceal(lua, this)?;
                Ok(mlua::Value::Nil)
            }
        }
    };
    let spell_plate = |lua: &mlua::Lua,
                       this: &mlua::Table,
                       tip: Option<crate::interface::api::SpellTip>|
     -> mlua::Result<mlua::Value> {
        match tip {
            Some(tip) => {
                clear(lua, this)?;
                spell_lines(lua, this, &tip)?;
                set_shown(this, true)?;
                Ok(one_or_nil(true))
            }
            None => {
                clear(lua, this)?;
                conceal(lua, this)?;
                Ok(mlua::Value::Nil)
            }
        }
    };

    // `GameTooltip:SetTrainerService(i)`: `i` is a one-based row.
    // `ClassTrainerSkillIcon`'s `OnEnter` passes
    // `ClassTrainerFrame.selectedService`, so it is the row the panel last
    // selected, not the one under the pointer.
    let set_trainer_service =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let row = usize::try_from(row.unwrap_or(0)).ok().filter(|i| *i > 0);
            spell_plate(lua, &this, row.and_then(|row| answers.trainer_tooltip(row)))
        })?;
    methods.set("SetTrainerService", set_trainer_service)?;

    // `GameTooltip:SetTalent(tab, i)`: both are one-based. The tab is
    // `TalentFrame.selectedTab`, not a value the button holds, so a hover
    // before any tab has been clicked passes nil and must return nothing
    // rather than raise an error.
    //
    // The plate shows the spell of the rank currently held, so its numbers
    // change as points are spent; see [`super::super::panels::talent`].
    let set_talent = scope.create_function(
        move |lua, (this, tab, index): (mlua::Table, Option<i64>, Option<i64>)| {
            let one = |n: Option<i64>| usize::try_from(n.unwrap_or(0)).ok().filter(|i| *i > 0);
            let tip = match (one(tab), one(index)) {
                (Some(tab), Some(index)) => answers.talent_tooltip(tab, index),
                _ => None,
            };
            spell_plate(lua, &this, tip)
        },
    )?;
    methods.set("SetTalent", set_talent)?;

    // Vendor and loot rows go through their item links, which the two panels
    // already provide for chat-frame links. The entry is therefore looked up
    // once per population, not through a second accessor that could disagree
    // with the row the panel drew.
    let set_merchant_item =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let row = usize::try_from(row.unwrap_or(0)).ok().filter(|i| *i > 0);
            let entry = row
                .and_then(|row| answers.merchant_item_link(row))
                .as_deref()
                .and_then(super::super::panels::container::entry_of);
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetMerchantItem", set_merchant_item)?;

    // The buyback tab's plate: the same item plate from a different list. It
    // uses an entry accessor rather than a link, because 1.12 has no
    // `GetBuybackItemLink`, unlike the vendor and loot lists.
    let set_buyback_item =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let entry = usize::try_from(row.unwrap_or(0))
                .ok()
                .filter(|i| *i > 0)
                .and_then(|row| answers.buyback_entry(row));
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetBuybackItem", set_buyback_item)?;

    let set_loot_item =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let row = usize::try_from(row.unwrap_or(0)).ok().filter(|i| *i > 0);
            let entry = row
                .and_then(|row| answers.loot_slot_link(row))
                .as_deref()
                .and_then(super::super::panels::container::entry_of);
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetLootItem", set_loot_item)?;

    // The item attached to a received letter and the one attached to a letter
    // being sent: two more item plates from two more lists. They are
    // implemented rather than stubbed for the reason in the module comment: a
    // method that returns nothing hides the plate, which looks the same as
    // hovering an empty slot. `InboxFrameItem_OnEnter` calls `SetInboxItem`
    // for every row that has an item, then adds the enclosed money or the COD
    // below it through `SetTooltipMoney`, which is FrameXML Lua.
    //
    // Looked up by entry rather than by link, as `SetBuybackItem` is, because
    // 1.12 has no `GetInboxItemLink`.
    let set_inbox_item =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let entry = usize::try_from(row.unwrap_or(0))
                .ok()
                .filter(|i| *i > 0)
                .and_then(|row| answers.mail_item_entry(row));
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetInboxItem", set_inbox_item)?;

    // The outgoing letter's item. `GameTooltip:SetSendMailItem()` takes no
    // argument because a letter being sent has only one attachment.
    let set_send_mail_item = scope.create_function(move |lua, this: mlua::Table| {
        item_plate(lua, &this, answers.mail_send_entry())
    })?;
    methods.set("SetSendMailItem", set_send_mail_item)?;

    // The same item plate for a group roll, the one item hover reached from a
    // frame rather than from a list: the icon on a `GroupLootFrame` is a
    // `<Button>` whose `OnEnter` calls this with the frame's `rollID`.
    //
    // Zero is a valid roll id (the counter starts there), so only negative
    // values are rejected, unlike the row numbers above.
    let set_loot_roll_item =
        scope.create_function(move |lua, (this, id): (mlua::Table, Option<i64>)| {
            let entry = id
                .and_then(|id| u32::try_from(id).ok())
                .and_then(|id| answers.loot_roll_link(id))
                .as_deref()
                .and_then(super::super::panels::container::entry_of);
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetLootRollItem", set_loot_roll_item)?;

    // The same item plate for either side of a trade, by slot id 1..7. The
    // partner's item is a template the handler queried when the offer
    // arrived, so the plate is empty until the reply arrives; see
    // [`super::super::panels::trade`].
    for (name, theirs) in [("SetTradePlayerItem", false), ("SetTradeTargetItem", true)] {
        let set_trade_item =
            scope.create_function(move |lua, (this, id): (mlua::Table, Option<i64>)| {
                let entry = id
                    .and_then(|id| u8::try_from(id).ok())
                    .and_then(|id| answers.trade_item(theirs, id))
                    .map(|line| line.entry);
                item_plate(lua, &this, entry)
            })?;
        methods.set(name, set_trade_item)?;
    }

    // `SetQuestItem(word, i)` takes the same three words as
    // `GetQuestItemInfo`, for the same reason: one function serves three
    // arrays from three packets. Reading the wrong array would show the right
    // number of plates with the wrong items.
    let set_quest_item = scope.create_function(
        move |lua, (this, which, n): (mlua::Table, Option<String>, Option<i64>)| {
            let index = usize::try_from(n.unwrap_or(0)).ok().filter(|i| *i > 0);
            let entry = super::super::panels::quest::Which::of(which.as_deref())
                .zip(index)
                .and_then(|(which, i)| {
                    answers.quest_items(which).into_iter().nth(i - 1)
                })
                .map(|line| line.entry);
            item_plate(lua, &this, entry)
        },
    )?;
    methods.set("SetQuestItem", set_quest_item)?;

    // The quest log's version: the same arrays for the selected log row rather
    // than for a dialogue page. `"choice"` and `"reward"` are the only two
    // words `QuestLogRewardItemTemplate` passes; a log entry has no
    // required-items array, because only the quest giver's panel shows one.
    let set_quest_log_item = scope.create_function(
        move |lua, (this, which, n): (mlua::Table, Option<String>, Option<i64>)| {
            let index = usize::try_from(n.unwrap_or(0)).ok().filter(|i| *i > 0);
            let entry = index
                .and_then(|i| {
                    answers
                        .quest_log_items(which.as_deref() == Some("choice"))
                        .into_iter()
                        .nth(i - 1)
                })
                .map(|line| line.entry);
            item_plate(lua, &this, entry)
        },
    )?;
    methods.set("SetQuestLogItem", set_quest_log_item)?;

    // The reward spell button's two methods, which the same template calls
    // when `this.rewardType == "spell"`. Without them, hovering the button on
    // a quest page that teaches a spell raises an error that stops the whole
    // `OnEnter`.
    let set_quest_reward_spell =
        scope.create_function(move |lua, this: mlua::Table| {
            spell_plate(lua, &this, answers.quest_reward_spell_tip(false))
        })?;
    methods.set("SetQuestRewardSpell", set_quest_reward_spell)?;
    let set_quest_log_reward_spell =
        scope.create_function(move |lua, this: mlua::Table| {
            spell_plate(lua, &this, answers.quest_reward_spell_tip(true))
        })?;
    methods.set("SetQuestLogRewardSpell", set_quest_log_reward_spell)?;
    Ok(())
}

/// The unit tooltip's lines, as the 1.12.1 client's `GameTooltip:SetUnit`
/// composes them.
///
/// Unlike the item plate's order, which this file states is a reconstruction,
/// this order matches the client exactly:
///
/// ```text
/// 1  the name                              gold, the engine default
/// 2  <subname>                             a creature template's tag
/// 3  Level <level> <class> (<type>)        the four TOOLTIP_UNIT_LEVEL* keys
/// 4  PvP                                   on UNIT_FIELD_FLAGS bit 12
/// ```
///
/// The third line needs the most work. Two cells are computed first, and the
/// format is chosen by which of them is non-empty:
///
/// * class: for a player-controlled unit, `"%s %s"` of race then class, which
///   makes the line read "Level 5 Human Warrior"; for anything else, the
///   localised `CreatureType.dbc` name.
/// * type: the `PLAYER` key for a player-controlled unit; otherwise the
///   classification word, one of five keys, which is empty for a normal and
///   for a rare creature.
///
/// The four keys are therefore `TOOLTIP_UNIT_LEVEL_CLASS_TYPE`, `…_CLASS`,
/// `…_TYPE` and `TOOLTIP_UNIT_LEVEL`, in that precedence. A level of zero or
/// less prints `"??"` instead of the number; that is how a unit far above the
/// player is shown.
///
/// Not drawn, because the state behind them does not exist in this client:
/// a player's guild (this client has no `SMSG_GUILD_QUERY` for it),
/// `RESURRECTABLE`, `PLAYER_OFFLINE`, and the faction/reaction lines below
/// them. Each depends on a missing subsystem, so the line is omitted rather
/// than filled with a guess; [`super::super::api::stubs`]' first paragraph
/// gives the same reasoning.
fn unit_lines(lua: &mlua::Lua, this: &mlua::Table, tip: &crate::interface::api::UnitTip) -> mlua::Result<()> {
    // The name takes the plate's default colour rather than a specific one:
    // `GameTooltip.xml`'s `UPDATE_MOUSEOVER_UNIT` handler replaces it with
    // `GameTooltip_UnitColor("mouseover")` immediately afterwards.
    append(lua, this, (text(lua, &tip.name)?, GOLD), None, false)?;
    if !tip.sub_name.is_empty() {
        append(lua, this, (text(lua, &tip.sub_name)?, WHITE), None, false)?;
    }

    // `%d` of the level, or "??" as the client shows. `GlobalStrings.lua` has
    // no key for "??", so it is a literal here.
    let level = if tip.level > 0 { tip.level.to_string() } else { "??".to_string() };
    let class = match (tip.race, tip.class) {
        (Some(race), Some(class)) => format!("{race} {class}"),
        // A player whose race or class never arrived falls through to the
        // creature branch, which for a player gives an empty string.
        _ => tip.creature_type.unwrap_or("").to_string(),
    };
    let kind = if tip.player_controlled {
        lua.globals().get::<Option<String>>("PLAYER")?.unwrap_or_default()
    } else if tip.classification.is_empty() {
        String::new()
    } else {
        lua.globals()
            .get::<Option<String>>(tip.classification)?
            .unwrap_or_default()
    };
    let line = match (class.is_empty(), kind.is_empty()) {
        (false, false) => global_format_all(lua, "TOOLTIP_UNIT_LEVEL_CLASS_TYPE", &[&level, &class, &kind]),
        (false, true) => global_format_all(lua, "TOOLTIP_UNIT_LEVEL_CLASS", &[&level, &class]),
        (true, false) => global_format_all(lua, "TOOLTIP_UNIT_LEVEL_TYPE", &[&level, &kind]),
        (true, true) => global_format_all(lua, "TOOLTIP_UNIT_LEVEL", &[&level]),
    };
    if let Some(line) = line {
        append(lua, this, (text(lua, &line)?, WHITE), None, false)?;
    }

    // The zone name, without a label, under the level line: shown for a party
    // member in another zone and for nobody else. The decision is made in
    // [`crate::interface::api::UnitTip::zone`].
    //
    // The line's position is a reconstruction: the reference screenshot shows
    // it as the third line, directly under the level/class line, and the
    // client's order for it is not otherwise known. It is a bare name rather
    // than `ZONE_COLON`; the file has both keys and the plate uses neither as
    // a label.
    if !tip.zone.is_empty() {
        append(lua, this, (text(lua, &tip.zone)?, WHITE), None, false)?;
    }

    if tip.pvp {
        if let Some(pvp) = lua.globals().get::<Option<String>>("PVP_ENABLED")? {
            append(lua, this, (text(lua, &pvp)?, WHITE), None, false)?;
        }
    }
    Ok(())
}

/// The spell tooltip's lines: name | rank, cost | range, cast time | cooldown,
/// each cell omitted when it has no value and each format a game global; then
/// the reagents and the description, both already resolved (see the module
/// comment).
fn spell_lines(lua: &mlua::Lua, this: &mlua::Table, tip: &SpellTip) -> mlua::Result<()> {
    // A talent's rank is its own line, not a cell; see
    // [`SpellTip::talent_rank`], which describes how the client composes it
    // and why the two are exclusive.
    let rank = tip
        .talent_rank
        .is_none()
        .then(|| (!tip.rank.is_empty()).then(|| tip.rank.clone()))
        .flatten();
    let rank = match rank {
        Some(rank) => Some((text(lua, &rank)?, GRAY)),
        None => None,
    };
    append(lua, this, (text(lua, &tip.name)?, WHITE), rank, false)?;
    if let Some((held, max)) = tip.talent_rank {
        let line = lua
            .globals()
            .get::<Option<String>>("TOOLTIP_TALENT_RANK")?
            .map(|format| {
                vale_assets::interface::strings::substitute_all(
                    &format,
                    &[&held.to_string(), &max.to_string()],
                )
            });
        if let Some(line) = line {
            append(lua, this, (text(lua, &line)?, WHITE), None, false)?;
        }
    }

    // `MANA_COST` is "%d Mana"; rage and energy have their own keys and focus
    // has none in build 5875's file, so a focus cost displays as nothing,
    // which is the client's rule for an absent key.
    let cost = (tip.power_cost > 0)
        .then(|| match tip.power_type {
            0 => Some("MANA_COST"),
            1 => Some("RAGE_COST"),
            3 => Some("ENERGY_COST"),
            _ => None,
        })
        .flatten()
        .and_then(|key| global_format(lua, key, &tip.power_cost.to_string()));
    let range = (tip.range_yards > 0.0)
        .then(|| global_format(lua, "SPELL_RANGE", &trim_number(f64::from(tip.range_yards))))
        .flatten();
    two_cells(lua, this, cost, range)?;

    // An instant spell with no cost is "Instant" and one with a cost is
    // "Instant cast", as the key names show (`…_INSTANT_NO_MANA`).
    let cast = if tip.cast_time_ms == 0 {
        let key = if tip.power_cost == 0 {
            "SPELL_CAST_TIME_INSTANT_NO_MANA"
        } else {
            "SPELL_CAST_TIME_INSTANT"
        };
        lua.globals().get::<Option<String>>(key)?
    } else {
        seconds_text(lua, tip.cast_time_ms, "SPELL_CAST_TIME_SEC", "SPELL_CAST_TIME_MIN")
    };
    let cooldown = (tip.cooldown_ms > 0)
        .then(|| seconds_text(lua, tip.cooldown_ms, "SPELL_RECAST_TIME_SEC", "SPELL_RECAST_TIME_MIN"))
        .flatten();
    two_cells(lua, this, cast, cooldown)?;

    // Reagents, white and wrapped, below the numbers and above the
    // description. `SPELL_REAGENTS` is "Reagents: ", a key with no format
    // slot, so the names are appended rather than substituted; a count above
    // one is added in brackets. A reagent whose item template has not arrived
    // is omitted (see [`SpellTip::reagents`]); an empty list, as for every
    // spell that consumes nothing, draws no line.
    if !tip.reagents.is_empty() {
        let named: Vec<String> = tip
            .reagents
            .iter()
            .map(|(name, count)| {
                if *count > 1 {
                    format!("{name} ({count})")
                } else {
                    name.clone()
                }
            })
            .collect();
        let label = lua
            .globals()
            .get::<Option<String>>("SPELL_REAGENTS")?
            .unwrap_or_default();
        append(
            lua,
            this,
            (text(lua, &format!("{label}{}", named.join(", ")))?, WHITE),
            None,
            true,
        )?;
    }

    // The description, gold and wrapped. Gold rather than the white of the
    // numbers above: `0xffffd200`, the colour the client uses on the spell
    // plate and the same default an uncoloured `AddLine` takes here, so it is
    // [`GOLD`] rather than a second constant.
    //
    // Wrapped because otherwise the plate is as wide as the sentence: "Causes
    // an explosion of arcane magic around the caster, causing 256 to 278
    // Arcane damage to all targets within 10 yards" is 800 units on one line.
    if !tip.description.is_empty() {
        append(lua, this, (text(lua, &tip.description)?, GOLD), None, true)?;
    }
    Ok(())
}

/// An item's plate, drawn from [`crate::interface::plate::item_plate`].
///
/// The line order, keys and colours are defined only in that function. It
/// reads the words through a lookup rather than from this interpreter, so a
/// caller with no interface running draws the same plate. This module
/// supplies only [`append`], through which every line goes, and the live
/// globals table as the lookup.
fn item_lines(
    lua: &mlua::Lua,
    this: &mlua::Table,
    tip: &crate::interface::api::ItemTip,
) -> mlua::Result<()> {
    let word = |key: &str| {
        lua.globals()
            .get::<Option<String>>(key)
            .ok()
            .flatten()
    };
    for line in crate::interface::plate::item_plate(tip, &word) {
        let [r, g, b] = line.ink.rgb();
        let right = match line.right {
            Some(right) => {
                let [rr, rg, rb] = line.right_ink.rgb();
                Some((text(lua, &right)?, [rr, rg, rb, 1.0]))
            }
            None => None,
        };
        append(
            lua,
            this,
            (text(lua, &line.left)?, [r, g, b, 1.0]),
            right,
            line.wrap,
        )?;
    }
    Ok(())
}

/// One line of up to two optional cells: both, one alone, or no line at all.
fn two_cells(
    lua: &mlua::Lua,
    this: &mlua::Table,
    left: Option<String>,
    right: Option<String>,
) -> mlua::Result<()> {
    match (left, right) {
        (Some(l), Some(r)) => append(
            lua,
            this,
            (text(lua, &l)?, WHITE),
            Some((text(lua, &r)?, WHITE)),
            false,
        ),
        (Some(l), None) => append(lua, this, (text(lua, &l)?, WHITE), None, false),
        (None, Some(r)) => append(lua, this, (text(lua, &r)?, WHITE), None, false),
        (None, None) => Ok(()),
    }
}

/// A `&str` as a Lua value, for [`append`]'s text arguments.
fn text(lua: &mlua::Lua, s: &str) -> mlua::Result<mlua::Value> {
    Ok(mlua::Value::String(lua.create_string(s)?))
}

/// A duration in the game's words: under a minute through the `_SEC` key,
/// otherwise the `_MIN` one, the number printed the way `%.3g` prints it.
fn seconds_text(lua: &mlua::Lua, ms: u32, sec_key: &str, min_key: &str) -> Option<String> {
    let seconds = f64::from(ms) / 1000.0;
    if seconds < 60.0 {
        global_format(lua, sec_key, &g3(seconds))
    } else {
        global_format(lua, min_key, &g3(seconds / 60.0))
    }
}

/// Format through a `GlobalStrings.lua` global: `"%d Mana"` with `"35"` is
/// `"35 Mana"`. `None` when the key is not in the environment, which displays
/// as nothing; that is the client's rule, not an error.
fn global_format(lua: &mlua::Lua, key: &str, value: &str) -> Option<String> {
    let format: String = lua.globals().get::<Option<String>>(key).ok().flatten()?;
    Some(substitute(&format, value))
}

/// [`global_format`] for a key with more than one slot. The four
/// `TOOLTIP_UNIT_LEVEL*` formats are its only users here.
fn global_format_all(lua: &mlua::Lua, key: &str, values: &[&str]) -> Option<String> {
    let format: String = lua.globals().get::<Option<String>>(key).ok().flatten()?;
    Some(vale_assets::interface::strings::substitute_all(&format, values))
}

/// Replace the first `%`-directive (`%d`, `%s`, `%.3g`…) with an already-
/// printed value. A one-slot substitution, because every key this module
/// reads has exactly one slot.
///
/// This calls [`vale_assets::interface::strings::substitute`] rather than
/// copying it: the same substitution is made against the live globals table
/// here and against the shipped table in `interface::messages`, and when there
/// were two copies one handled `%d` and the other did not.
fn substitute(format: &str, value: &str) -> String {
    vale_assets::interface::strings::substitute(format, value)
}

/// `%.3g`: three significant digits, trailing zeros trimmed. `1.5` is "1.5",
/// `2` is "2", `12.5` is "12.5".
fn g3(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let magnitude = x.abs().log10().floor() as i32;
    let decimals = (2 - magnitude).max(0) as usize;
    let printed = format!("{x:.decimals$}");
    // Trailing zeros are trimmed only after a decimal point; "150" printed
    // with no point must keep all three digits.
    if printed.contains('.') {
        printed.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        printed
    }
}

/// A number printed for the range key: whole yards without a decimal point,
/// anything else with its fraction.
fn trim_number(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;

    fn state() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            GameTooltip = CreateFrame("GameTooltip", "GameTooltip", UIParent);
            owner = CreateFrame("Button", "Owner", UIParent);
            owner:SetWidth(36); owner:SetHeight(36);
            owner:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 100, 100);
            "#,
        )
        .exec()
        .expect("the fixture loads");
        lua
    }

    /// A street sign's plate is hidden at once; every other kind fades. The
    /// two arms are adjacent in one `match` and the difference does not show
    /// in a screenshot, so it is tested here: a plate positioned at the cursor
    /// must disappear when the cursor leaves, and a unit's plate must keep the
    /// 1.12.1 client's three-second fade.
    #[test]
    fn only_the_floating_plate_disappears_the_instant_the_pointer_leaves() {
        assert!(
            Plate::None.body(Plate::Floating).contains("Hide()"),
            "a sign's plate must not fade"
        );
        for left in [Plate::Unit, Plate::Object, Plate::None] {
            assert!(
                Plate::None.body(left).contains("FadeOut()"),
                "{:?} lost its ramp",
                left as u8
            );
        }
        // Both arms still check `default`: a plate a button owns is neither
        // hidden nor faded by the world mouseover.
        for left in [Plate::Floating, Plate::Unit] {
            assert!(Plate::None.body(left).contains("GameTooltip.default"));
        }
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// The frame's alpha, as a number. Not read through [`eval`]: a whole Lua
    /// number comes back as `Integer(1)` rather than `Number(1.0)`, so a
    /// string comparison fails for the opaque case, the value most often
    /// checked.
    fn alpha(lua: &mlua::Lua) -> f64 {
        lua.load("return GameTooltip:GetAlpha()").eval().expect("a number")
    }

    /// A plate the pointer has left holds, then fades linearly, then hides.
    ///
    /// The shape is `FadingFrame.lua`'s and the two durations are explained
    /// at [`FADE_HOLD`]. This test checks the shape, not the durations, so it
    /// is written against the constants rather than 1.0 and 2.0; changing the
    /// durations should not require editing a test about linearity.
    #[test]
    fn a_faded_plate_holds_at_full_alpha_and_then_ramps_away() {
        let lua = state();
        lua.load(r#"GameTooltip:SetText("Kobold Vermin")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");

        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        // It is still shown, with the same lines. That is the difference from
        // `Hide`, which clears the lines and the owner.
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(1)");
        assert_eq!(alpha(&lua), 1.0);

        // During the hold the plate stays opaque.
        let step = FADE_HOLD / 4.0;
        for _ in 0..4 {
            fade_sweep(&lua, step);
            assert_eq!(alpha(&lua), 1.0);
            assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");
        }

        // Then the alpha falls, linearly and monotonically.
        let mut last = 1.0_f64;
        let step = FADE_OUT / 8.0;
        for _ in 0..7 {
            fade_sweep(&lua, step);
            let now = alpha(&lua);
            assert!(now < last, "the ramp stalled at {now}");
            assert!(now > 0.0, "it arrived early at {now}");
            last = now;
        }
        // A given fraction of the fade time removes the same fraction of the
        // alpha; an eased implementation would fail this check.
        assert!(
            (last - 1.0 / 8.0).abs() < 1e-9,
            "seven eighths down the ramp should be one eighth of alpha, not {last}"
        );

        // The last tick hides it, and only then are the lines cleared.
        fade_sweep(&lua, step * 2.0);
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(0)");
        assert_eq!(
            alpha(&lua),
            1.0,
            "a hidden plate left transparent is one that comes back invisible"
        );
    }

    /// Repopulating or showing a plate cancels the fade. This keeps a plate
    /// from reappearing half-transparent, a fault only visible on screen,
    /// since every other check passes on a faint tooltip.
    ///
    /// The three paths are the ones the directory uses: `SetOwner` (and so
    /// `GameTooltip_SetDefaultAnchor`, every world plate and every
    /// `UnitFrame_OnEnter`), `SetText`, and a plain `Show`.
    #[test]
    fn showing_or_re_populating_a_fading_plate_stops_the_fade() {
        for door in [
            r#"GameTooltip:SetOwner(owner, "ANCHOR_RIGHT")"#,
            r#"GameTooltip:SetText("Riverpaw Gnoll")"#,
            "GameTooltip:Hide(); GameTooltip:Show()",
        ] {
            let lua = state();
            lua.load(r#"GameTooltip:SetText("Kobold Vermin")"#).exec().expect("runs");
            lua.load("GameTooltip:FadeOut()").exec().expect("runs");
            fade_sweep(&lua, FADE_HOLD + FADE_OUT / 2.0);
            let faint = alpha(&lua);
            assert!(faint < 1.0 && faint > 0.0, "{door}: not mid-ramp at {faint}");

            lua.load(door).exec().expect("runs");
            assert_eq!(alpha(&lua), 1.0, "{door} left the plate faint");
            // The frame is removed from the fade list, not only reset: another
            // tick must not lower the alpha again.
            fade_sweep(&lua, FADE_HOLD + FADE_OUT);
            assert_eq!(alpha(&lua), 1.0, "{door} left the fade running");
        }
    }

    /// A repeated `FadeOut` must not restart the hold, and must not show a
    /// hidden plate again. `UnitFrame_OnLeave` is the directory's one caller,
    /// and a pointer moving along an edge fires it repeatedly.
    #[test]
    fn fading_twice_neither_restarts_the_hold_nor_wakes_a_hidden_plate() {
        let lua = state();
        lua.load(r#"GameTooltip:SetText("Defias Thug")"#).exec().expect("runs");
        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        fade_sweep(&lua, FADE_HOLD);
        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        fade_sweep(&lua, FADE_OUT / 2.0);
        let now = alpha(&lua);
        assert!((now - 0.5).abs() < 1e-9, "the second call restarted the hold: {now}");

        // On a plate that is already hidden it does nothing.
        lua.load("GameTooltip:Hide()").exec().expect("runs");
        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        fade_sweep(&lua, FADE_HOLD + FADE_OUT);
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
    }

    /// The sweep does nothing when nothing is fading, which is the usual case:
    /// it runs on the interface tick thirty times a second for the whole
    /// session.
    #[test]
    fn the_sweep_is_a_no_op_with_nothing_on_the_list() {
        let lua = state();
        fade_sweep(&lua, 1.0);
        assert_eq!(alpha(&lua), 1.0);
    }

    /// `SetText` clears, writes line 1 and shows, and text with no colour is
    /// the client's default gold, not white. The old `AddLine(text, "", r, g,
    /// b)` form relies on this default.
    #[test]
    fn set_text_writes_a_gold_line_and_shows() {
        let lua = state();
        lua.load(r#"GameTooltip:Hide(); GameTooltip:SetText("Zeppelin Master")"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(1)");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft1:GetText()"),
            r#"String("Zeppelin Master")"#
        );
        let line: mlua::Table = lua.globals().get("GameTooltipTextLeft1").expect("line 1");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert_eq!(colour[0], 1.0);
        assert!((colour[1] - 210.0 / 255.0).abs() < 1e-9, "the default is gold");
        assert_eq!(colour[2], 0.0);

        // An explicit colour is applied.
        lua.load(r#"GameTooltip:SetText("Hot", 1.0, 0.1, 0.1)"#).exec().expect("runs");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert!((colour[1] - 0.1).abs() < 1e-9);

        // A non-number r argument makes all three colour arguments ignored,
        // leaving gold.
        lua.load(r#"GameTooltip:SetText("Zone", "", 1.0, 1.0, 1.0)"#).exec().expect("runs");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert!((colour[1] - 210.0 / 255.0).abs() < 1e-9, "the gate is lua_isnumber on the r-slot");

        // `SetText` with no text raises the client's usage error rather than
        // drawing a blank plate.
        assert!(lua.load("GameTooltip:SetText()").exec().is_err());
    }

    /// `AddLine` appends and does not show. The corpus depends on this:
    /// `AddLine … Show()` versus `SetText` alone.
    #[test]
    fn add_line_appends_without_showing() {
        let lua = state();
        lua.load(r#"GameTooltip:Hide(); GameTooltip:ClearLines(); GameTooltip:AddLine("First")"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(1)");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        lua.load(r#"GameTooltip:AddLine("Second", 1.0, 1.0, 1.0); GameTooltip:Show()"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(2)");
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft2:GetText()"),
            r#"String("Second")"#
        );
    }

    /// A double line right-aligns its second cell, and the plate is as wide as
    /// its widest line plus the pad, as the client sizes it, using this
    /// project's text measurement.
    #[test]
    fn a_double_line_sizes_the_plate_and_flushes_right() {
        let lua = state();
        lua.load(
            r#"
            GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
            GameTooltip:AddDoubleLine("Damage", "12 - 19");
            GameTooltip:Show();
            "#,
        )
        .exec()
        .expect("runs");
        let width: f64 = lua.load("return GameTooltip:GetWidth()").eval().expect("a number");
        assert!(width > 2.0 * PAD, "the plate took a size: {width}");
        // The right cell's right edge sits at the plate's right inset.
        let plate_right: f64 = lua.load("return GameTooltip:GetRight()").eval().expect("solves");
        let cell_right: f64 = lua
            .load("return GameTooltipTextRight1:GetRight()")
            .eval()
            .expect("solves");
        assert!((plate_right - PAD - cell_right).abs() < 1e-6);
        // The minimum width applies when it is wider.
        lua.load("GameTooltip:SetMinimumWidth(300)").exec().expect("runs");
        let floored: f64 = lua.load("return GameTooltip:GetWidth()").eval().expect("a number");
        assert_eq!(floored, 300.0 + 2.0 * PAD);
    }

    /// A `wrap` line breaks at [`WRAP_WIDTH`] instead of widening the plate to
    /// the sentence. `GameTooltip_AddNewbieTip` is the corpus caller: its
    /// explanation line is over a hundred characters with `wrap=1`, and
    /// without wrapping the plate was as wide as the screen. The flag is per
    /// line (a later unwrapped line still widens the plate), and `clear`
    /// resets it with the text.
    #[test]
    fn a_wrapped_line_folds_instead_of_stretching_the_plate() {
        let lua = state();
        lua.load(
            r#"
            long = string.rep("information ", 12);
            GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
            GameTooltip:SetText("Social");
            GameTooltip:AddLine(long, 1.0, 0.82, 0, 1);
            GameTooltip:Show();
            "#,
        )
        .exec()
        .expect("runs");
        let width: f64 = lua.load("return GameTooltip:GetWidth()").eval().expect("a number");
        assert!(
            width <= WRAP_WIDTH + 2.0 * PAD + 1e-6,
            "the plate folded: {width}"
        );
        let height: f64 = lua
            .load("return GameTooltipTextLeft2:GetHeight()")
            .eval()
            .expect("a number");
        assert!(height > DEFAULT_LINE_HEIGHT, "the folded line charges its rows: {height}");

        // The same line without the flag widens the plate, which is
        // `AddLine`'s default behaviour.
        lua.load(
            r#"
            GameTooltip:ClearLines();
            GameTooltip:AddLine(long);
            "#,
        )
        .exec()
        .expect("runs");
        let unwrapped: f64 = lua.load("return GameTooltip:GetWidth()").eval().expect("a number");
        assert!(unwrapped > width, "{unwrapped} vs {width}");
    }

    /// A wrapped cell reserves the rows the painter draws, not
    /// `ceil(measured / wrap width)`.
    ///
    /// The two agree only when every break falls exactly at the wrap width.
    /// Prose leaves part of a row empty at each break, so the ratio is too
    /// small once the text is a few rows long. With Shield Bash's description
    /// the plate was one row too short and drew its last line under the bottom
    /// border.
    ///
    /// The assertion compares against [`regions::text_rows`] rather than a
    /// constant, because the painter wraps with that function: the test checks
    /// that layout and painter compute rows the same way, which is what makes
    /// the plate fit.
    #[test]
    fn a_folded_cell_reserves_the_rows_the_painter_draws() {
        let lua = state();
        let sentence = "Bashes the target with your shield for 45 damage.  It also \
                        interrupts spellcasting and prevents any spell in that school \
                        from being cast for 6 sec.";
        lua.globals().set("sentence", sentence).expect("the fixture sets");
        lua.load(
            r#"
            GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
            GameTooltip:SetText("Shield Bash");
            GameTooltip:AddLine(sentence, 1.0, 0.82, 0, 1);
            GameTooltip:Show();
            "#,
        )
        .exec()
        .expect("runs");

        let cell: mlua::Table = lua.globals().get("GameTooltipTextLeft2").expect("line 2");
        let rows = regions::text_rows(&lua, &cell, sentence, WRAP_WIDTH);
        assert!(rows >= 3, "the fixture is a multi-row sentence: {rows}");

        let line = regions::line_height_of(&lua, &cell).max(DEFAULT_LINE_HEIGHT);
        let height: f64 = lua
            .load("return GameTooltipTextLeft2:GetHeight()")
            .eval()
            .expect("a number");
        assert!(
            (height - rows as f64 * line).abs() < 1e-3,
            "the cell is {height} where {rows} rows of {line} is {}",
            rows as f64 * line
        );
        // The plate is tall enough to hold both lines and the pad.
        let plate: f64 = lua.load("return GameTooltip:GetHeight()").eval().expect("a number");
        assert!(
            plate >= height + line + LINE_GAP + 2.0 * PAD - 1e-6,
            "the plate clips its own last row: {plate}"
        );

        // A fixture where the ratio and the row count differ even under the
        // bare interpreter's uniform-width fallback. Four words that each
        // nearly fill a row: greedy wrapping puts one per row and takes four
        // rows, while `ceil(total / wrap width)` gives three, because the ratio
        // counts the empty end of every row as if it held glyphs. With the
        // game's typefaces the same difference appears on ordinary prose such
        // as the sentence above, but the fallback's uniform advance hides it
        // there.
        let words = vec!["Counterspell".repeat(2); 4].join(" ");
        lua.globals().set("words", words.clone()).expect("the fixture sets");
        lua.load(r#"GameTooltip:ClearLines(); GameTooltip:AddLine(words, 1, 1, 1, 1)"#)
            .exec()
            .expect("runs");
        let cell: mlua::Table = lua.globals().get("GameTooltipTextLeft1").expect("line 1");
        let rows = regions::text_rows(&lua, &cell, &words, WRAP_WIDTH);
        let ratio = (regions::text_width(&lua, &cell, &words) / WRAP_WIDTH).ceil() as usize;
        assert!(ratio < rows, "the ratio undercounts: {ratio} against {rows}");
        let height: f64 = lua
            .load("return GameTooltipTextLeft1:GetHeight()")
            .eval()
            .expect("a number");
        let line = regions::line_height_of(&lua, &cell).max(DEFAULT_LINE_HEIGHT);
        assert!(
            (height - rows as f64 * line).abs() < 1e-3,
            "the cell takes the folded count, not the ratio: {height}"
        );
    }

    /// `SetOwner` anchors the plate to the owner's corner according to the
    /// anchor word and clears the old contents, and `IsOwned` returns true for
    /// the owner and no other frame.
    #[test]
    fn set_owner_anchors_clears_and_claims() {
        let lua = state();
        lua.load(
            r#"
            cleared = 0;
            GameTooltip:SetScript("OnTooltipCleared", function() cleared = cleared + 1; end);
            GameTooltip:AddLine("stale");
            GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
            GameTooltip:SetText("fresh");
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(1)");
        assert!(eval(&lua, "return cleared").starts_with("Integer"), "OnTooltipCleared fired");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(owner)"), "Integer(1)");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(UIParent)"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:GetOwner() == owner"), "Boolean(true)");
        // ANCHOR_RIGHT: this bottom-left on the owner's top-right.
        let left: f64 = lua.load("return GameTooltip:GetLeft()").eval().expect("solves");
        let bottom: f64 = lua.load("return GameTooltip:GetBottom()").eval().expect("solves");
        assert_eq!(left, 136.0, "the owner's right edge");
        assert_eq!(bottom, 136.0, "the owner's top edge");
    }

    /// Setting the same owner with the same anchor again does not invalidate
    /// the layout memo. It is the most frequent call in the interface.
    ///
    /// `ContainerFrameItemButton_OnUpdate` runs `OnEnter` every frame the
    /// pointer is over a bag slot (the FrameXML comment there reads "Might hurt
    /// performance, but need to always update the cursor now"), and `OnEnter`
    /// first calls `SetOwner`. When `SetOwner` wrote a new points table each
    /// time, that invalidated the whole layout every frame, discarding every
    /// solved rectangle sixty times a second for a tooltip that had not moved.
    /// Measured with five full bags open: 8.4 ms of interpreter time per frame
    /// against 6.0, with a new generation on 309 of 300 frames.
    ///
    /// A different owner or a different anchor still invalidates once, because
    /// the plate does move.
    #[test]
    fn re_claiming_the_same_owner_keeps_the_layout_memo() {
        let lua = state();
        lua.load(r#"other = CreateFrame("Frame", "Other", UIParent);"#)
            .exec()
            .expect("runs");
        lua.load(r#"GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");"#)
            .exec()
            .expect("runs");
        let settled = super::super::layout::generation(&lua);
        for _ in 0..10 {
            lua.load(r#"GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");"#)
                .exec()
                .expect("runs");
        }
        assert_eq!(
            super::super::layout::generation(&lua),
            settled,
            "ten identical claims cost nothing"
        );
        // The anchor is still in place; skipping the write did not remove it.
        assert_eq!(
            lua.load("return GameTooltip:GetLeft()").eval::<f64>().expect("solves"),
            136.0
        );
        // A change of anchor, and a change of owner, each invalidate.
        lua.load(r#"GameTooltip:SetOwner(owner, "ANCHOR_LEFT");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 1);
        lua.load(r#"GameTooltip:SetOwner(other, "ANCHOR_LEFT");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 2);
        // `ANCHOR_NONE` removes the anchors and invalidates the layout, so no
        // rectangle solved from the removed anchors remains. Writing a new
        // points table did not invalidate in this case.
        lua.load(r#"GameTooltip:SetOwner(other, "ANCHOR_NONE");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 3);
        assert_eq!(eval(&lua, "return GameTooltip:GetNumPoints()"), "Integer(0)");
    }

    /// `Hide` clears the owner and the lines, whereas an empty population hides
    /// and keeps the owner. This stops a refresh loop from showing again a
    /// plate the pointer has left, while letting it refill one whose contents
    /// were briefly empty.
    #[test]
    fn hide_drops_the_owner_and_clear_fires_the_script() {
        let lua = state();
        lua.load(
            r#"
            GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
            GameTooltip:SetText("something");
            GameTooltip:Hide();
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(owner)"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(0)");
        assert_eq!(eval(&lua, "return GameTooltipTextLeft1:GetText()"), "Nil");
        // A plain frame's Hide does not take the tooltip path.
        lua.load("owner:Hide()").exec().expect("runs");
    }

    /// The template's declared lines are used, not duplicated. A tooltip
    /// whose `TextLeft1` already exists, as when the loader created it, writes
    /// into that region, so `GameTooltipTextLeft1:SetTextColor(...)` in the
    /// directory's `OnEvent` changes the line that was written.
    #[test]
    fn a_declared_line_is_adopted_rather_than_duplicated() {
        let lua = state();
        lua.load(
            r#"
            ladder = GameTooltip:CreateFontString("GameTooltipTextLeft1", "ARTWORK");
            ladder:SetPoint("TOPLEFT", GameTooltip, "TOPLEFT", 10, -10);
            GameTooltip:SetText("Adopted");
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(
            eval(&lua, "return ladder:GetText()"),
            r#"String("Adopted")"#,
            "the declared region is the line, not a twin of it"
        );
    }

    /// `SetAction` composes the spell tooltip from the live world, using the
    /// game's format strings. An empty slot hides the plate, keeps the owner
    /// and returns nil, which `ActionButton_SetTooltip` tests.
    #[test]
    fn set_action_answers_the_live_bar() {
        let lua = state();
        // The format strings, verbatim from `GlobalStrings.lua`.
        lua.load(
            r#"
            MANA_COST = "%d Mana";
            SPELL_RANGE = "%s yd range";
            SPELL_CAST_TIME_SEC = "%.3g sec cast";
            SPELL_REAGENTS = "Reagents: ";
            "#,
        )
        .exec()
        .expect("the strings load");
        let world = Stub::default().action(1, "Fireball");
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                shown = GameTooltip:SetAction(1);
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(eval(&lua, "return shown"), "Integer(1)");
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft1:GetText()"),
            r#"String("Fireball")"#
        );
        assert_eq!(
            eval(&lua, "return GameTooltipTextRight1:GetText()"),
            r#"String("Rank 1")"#,
            "SetAction shows the rank column"
        );
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft2:GetText()"),
            r#"String("30 Mana")"#
        );
        assert_eq!(
            eval(&lua, "return GameTooltipTextRight2:GetText()"),
            r#"String("30 yd range")"#
        );
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft3:GetText()"),
            r#"String("3.5 sec cast")"#
        );
        // The reagents, with the game's "Reagents: " label and a count above
        // one in brackets, and the description below them.
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft4:GetText()"),
            r#"String("Reagents: Rune of Teleportation (2)")"#
        );
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft5:GetText()"),
            r#"String("Hurls a fiery ball.")"#
        );
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(5)");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");

        // The empty slot: hidden, owned, nil.
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load("empty = GameTooltip:SetAction(7);").exec()
        })
        .expect("the empty population runs");
        assert_eq!(eval(&lua, "return empty"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(
            eval(&lua, "return GameTooltip:IsOwned(owner)"),
            "Integer(1)",
            "an empty population keeps the owner — only Hide lets go"
        );
    }

    /// The `GlobalStrings.lua` keys the unit plate is composed out of, verbatim.
    fn unit_strings(lua: &mlua::Lua) {
        lua.load(
            r#"
            TOOLTIP_UNIT_LEVEL = "Level %s";
            TOOLTIP_UNIT_LEVEL_TYPE = "Level %s (%s)";
            TOOLTIP_UNIT_LEVEL_CLASS = "Level %s %s";
            TOOLTIP_UNIT_LEVEL_CLASS_TYPE = "Level %s %s (%s)";
            PLAYER = "Player";
            ELITE = "Elite";
            BOSS = "Boss";
            PVP_ENABLED = "PvP";
            "#,
        )
        .exec()
        .expect("the strings load");
    }

    /// Fill the plate for `token` against `world` and answer its lines.
    fn plate(lua: &mlua::Lua, world: &dyn Answers, token: &str) -> Vec<String> {
        let (held, queue) = crate::lua::api::held_for_test(lua);
        lua.scope(|scope| {
            crate::lua::api::install(lua, scope, world, &held, &queue)?;
            lua.load(format!(
                r#"GameTooltip:SetOwner(owner, "ANCHOR_RIGHT"); GameTooltip:SetUnit("{token}");"#
            ))
            .exec()
        })
        .expect("the population runs");
        let count: usize = lua
            .load("return GameTooltip:NumLines()")
            .eval()
            .expect("the count reads");
        (1..=count)
            .map(|line| {
                lua.load(format!("return GameTooltipTextLeft{line}:GetText()"))
                    .eval::<Option<String>>()
                    .expect("the line reads")
                    .unwrap_or_default()
            })
            .collect()
    }

    /// A party member in another zone gets an unlabelled zone line, third,
    /// under the level line, as the reference screenshot shows.
    ///
    /// The decision is made elsewhere: [`crate::interface::api::UnitTip::zone`]
    /// is empty for everyone except a party member in another zone, so this
    /// tests only the composition. The line's position is a reconstruction,
    /// as the comment in [`unit_lines`] states.
    #[test]
    fn a_party_member_elsewhere_gets_a_zone_line() {
        let lua = state();
        unit_strings(&lua);
        let world = Stub::default()
            .unit("party1", "Bram", 7)
            .played()
            .in_zone("Stranglethorn Vale");
        assert_eq!(
            plate(&lua, &world, "party1"),
            [
                "Bram",
                "Level 60 Human Warrior (Player)",
                "Stranglethorn Vale"
            ]
        );
        // A member in the same zone gets no zone line.
        let here = Stub::default().unit("party1", "Bram", 7).played();
        assert_eq!(
            plate(&lua, &here, "party1"),
            ["Bram", "Level 60 Human Warrior (Player)"]
        );
    }

    /// A creature's plate: name, `<subname>`, `Level N Type (Class)`.
    ///
    /// The composition follows the client; see [`unit_lines`]. This tests the
    /// three-cell branch, the one shown in the reference screenshot.
    #[test]
    fn set_unit_composes_a_creatures_plate() {
        let lua = state();
        unit_strings(&lua);
        // A humanoid (7) elite (1) innkeeper, unflagged.
        let world = Stub::default()
            .unit("target", "Innkeeper Farley", 7)
            .described("Innkeeper", 7, 1, false);
        assert_eq!(
            plate(&lua, &world, "target"),
            ["Innkeeper Farley", "Innkeeper", "Level 60 Humanoid (Elite)"]
        );
    }

    /// A player's plate: the class cell is `"%s %s"` of race then class and
    /// the type cell is the `PLAYER` key, giving "Level 60 Human Warrior
    /// (Player)", with the PvP line under it when the flag is set.
    #[test]
    fn set_unit_composes_a_players_plate() {
        let lua = state();
        unit_strings(&lua);
        let world = Stub::default()
            .unit("target", "Biggay", 7)
            .player_controlled()
            .described("", 0, 0, true);
        assert_eq!(
            plate(&lua, &world, "target"),
            ["Biggay", "Level 60 Human Warrior (Player)", "PvP"]
        );
    }

    /// The two cells together select the format. An ordinary creature has no
    /// classification word, so its line uses `TOOLTIP_UNIT_LEVEL_CLASS` and
    /// draws no empty brackets. A rare creature is treated the same way: the
    /// 1.12.1 client shows no classification word for it.
    #[test]
    fn a_unit_with_no_classification_draws_no_brackets() {
        let lua = state();
        unit_strings(&lua);
        let world = Stub::default()
            .unit("target", "Kobold Vermin", 7)
            .described("", 7, 0, false);
        assert_eq!(plate(&lua, &world, "target"), ["Kobold Vermin", "Level 60 Humanoid"]);

        let rare = Stub::default()
            .unit("target", "Ghostcrawler", 7)
            .described("", 7, 4, false);
        assert_eq!(plate(&lua, &rare, "target"), ["Ghostcrawler", "Level 60 Humanoid"]);
    }

    /// An unknown level is shown as `"??"`, which is not a `GlobalStrings.lua`
    /// key. With no creature type either, the line uses `TOOLTIP_UNIT_LEVEL`.
    #[test]
    fn an_unknown_level_draws_the_clients_own_question_marks() {
        let lua = state();
        unit_strings(&lua);
        let mut world = Stub::default().unit("target", "Something", 7);
        world.units[0].level = 0;
        world.units[0].creature_type = 0;
        assert_eq!(plate(&lua, &world, "target"), ["Something", "Level ??"]);
    }

    /// A token with nobody behind it hides the plate and keeps the owner.
    #[test]
    fn set_unit_on_an_absent_unit_hides_the_plate() {
        let lua = state();
        unit_strings(&lua);
        let world = Stub::default().unit("target", "Kobold Vermin", 7);
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                GameTooltip:SetUnit("mouseover");
                "#,
            )
            .exec()
        })
        .expect("the empty population runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(owner)"), "Integer(1)");
    }

    /// An item's plate, composed from the live bags.
    ///
    /// The words come from the environment, so the fixture defines the same
    /// `GlobalStrings.lua` keys as the shipped file. The assertions check the
    /// substitution rather than the English text, because substitution is
    /// this module's part.
    #[test]
    fn set_bag_item_composes_the_item_plate() {
        let lua = state();
        lua.load(
            r#"
            ITEM_BIND_ON_PICKUP = "Binds when picked up";
            ITEM_SOULBOUND = "Soulbound";
            INVTYPE_WEAPONMAINHAND = "Main Hand";
            DAMAGE_TEMPLATE = "%d - %d Damage";
            SPEED = "Speed";
            DPS_TEMPLATE = "(%.1f damage per second)";
            ARMOR_TEMPLATE = "%d Armor";
            ITEM_MOD_STAMINA = "%c%d Stamina";
            ITEM_RESIST_SINGLE = "%c%d %s Resistance";
            RESISTANCE3_NAME = "Nature Resistance";
            DURABILITY_TEMPLATE = "Durability %d / %d";
            ITEM_MIN_LEVEL = "Requires Level %d";
            ITEM_SPELL_TRIGGER_ONPROC = "Chance on hit:";
            CONTAINER_SLOTS = "%d Slot %s";
            "#,
        )
        .exec()
        .expect("the strings load");

        // The stub world provides only a name and quality. The plate's shape
        // is checked by reading the lines written here, because this module
        // owns the composition and the stub belongs to another file.
        let world = Stub::default().bags();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                cooldown, repairCost = GameTooltip:SetBagItem(0, 1);
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft1:GetText()"),
            r#"String("Linen Cloth")"#
        );
        // The repair cost is a number, never nil: `ContainerFrame_Update`
        // compares it with `>` two lines later.
        assert_eq!(eval(&lua, "return repairCost"), "Integer(0)");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");

        // The name takes the quality's colour, from the same table
        // `GetItemQualityColor` reads.
        let line: mlua::Table = lua.globals().get("GameTooltipTextLeft1").expect("line 1");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert_eq!(colour, crate::lua::api::stubs::quality_rgb(1).to_vec());

        // An empty slot hides the plate and keeps the owner, so
        // `ContainerFrameItemButton_OnUpdate`'s `IsOwned` check still works.
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load("GameTooltip:SetBagItem(0, 9);").exec()
        })
        .expect("the empty population runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(owner)"), "Integer(1)");
    }

    /// A requirement the character does not meet is drawn in red.
    ///
    /// The server refuses an item whose required level is above the
    /// character's with `EQUIP_ERR_CANT_EQUIP_LEVEL_I`, and a right-click on
    /// such a potion does nothing. Drawn in the same white as every other
    /// line, "Requires Level 45" gives the player no sign that it is the
    /// reason; the red colour does.
    #[test]
    fn an_unmet_requirement_is_drawn_in_red() {
        let lua = state();
        lua.load(r#"ITEM_MIN_LEVEL = "Requires Level %d";"#)
            .exec()
            .expect("the strings load");
        let world = Stub::default().bags();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                GameTooltip:SetBagItem(0, 3);
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(
            eval(&lua, "return GameTooltipTextLeft1:GetText()"),
            r#"String("Major Troll's Blood Potion")"#
        );
        // The line is present and `%d` is substituted: a formatter that
        // handled only `%s` printed the sentence without the number.
        let mut found = None;
        for index in 2..=8 {
            let name = format!("GameTooltipTextLeft{index}");
            let Ok(Some(line)) = lua.globals().get::<Option<mlua::Table>>(name.as_str()) else {
                continue;
            };
            if line.get::<Option<String>>("__text").ok().flatten().as_deref()
                == Some("Requires Level 45")
            {
                found = line.get::<Vec<f64>>("__colour").ok();
                break;
            }
        }
        assert_eq!(found, Some(RED.to_vec()), "an unmet level requirement is red");
    }

    /// `SetInventoryItem`'s first return value selects the fallback, and it
    /// returns a different number of values from `SetBagItem`, as in the
    /// game's FrameXML.
    #[test]
    fn set_inventory_item_answers_whether_there_was_an_item() {
        let lua = state();
        lua.load(r#"INVTYPE_WEAPONMAINHAND = "Main Hand";"#).exec().expect("runs");
        let world = Stub::default().bags();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                hasItem = GameTooltip:SetInventoryItem("player", 16);
                empty = GameTooltip:SetInventoryItem("player", 1);
                other = GameTooltip:SetInventoryItem("target", 16);
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(eval(&lua, "return hasItem"), "Integer(1)");
        assert_eq!(eval(&lua, "return empty"), "Nil", "an empty slot is the fallback branch");
        assert_eq!(
            eval(&lua, "return other"),
            "Nil",
            "no packet carries another unit's gear"
        );
    }

    /// A link opens the same plate with no object behind it, and a link this
    /// client cannot read returns nothing rather than the wrong item.
    #[test]
    fn set_hyperlink_reads_an_item_link_and_refuses_the_rest() {
        let lua = state();
        let world = Stub::default().bags();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                item = GameTooltip:SetHyperlink("item:2589:0:0:0");
                name = GameTooltipTextLeft1:GetText();
                spell = GameTooltip:SetHyperlink("spell:133");
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(eval(&lua, "return item"), "Integer(1)");
        assert_eq!(eval(&lua, "return name"), r#"String("Linen Cloth")"#);
        assert_eq!(eval(&lua, "return spell"), "Nil");
    }

    /// Vendor items, loot rows and quest rewards all show a plate on hover.
    /// If one of this file's five panel populations is missing, the `OnEnter`
    /// that calls it stops on a nil method with the plate still hidden. On
    /// screen that looks the same as hovering an empty slot, no count detects
    /// it, and the panels themselves still work, so this test checks it.
    ///
    /// All four item plates go through the same [`item_lines`] as the bags, so
    /// the same item hovered in two places always shows the same plate.
    #[test]
    fn a_shop_a_corpse_and_a_quest_all_fill_the_same_item_plate() {
        let lua = state();
        let world = Stub::default().bags();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load(
                r#"
                GameTooltip:SetOwner(owner, "ANCHOR_RIGHT");
                shelf = GameTooltip:SetMerchantItem(1);
                shelfName = GameTooltipTextLeft1:GetText();
                none = GameTooltip:SetMerchantItem(0);
                shownAfterNone = GameTooltip:IsShown();
                -- a word the three arrays do not know is nothing, not the
                -- first array by default
                bad = GameTooltip:SetQuestItem("rewards", 1);
                "#,
            )
            .exec()
        })
        .expect("the population runs");
        assert_eq!(eval(&lua, "return shelf"), "Integer(1)");
        assert_eq!(eval(&lua, "return shelfName"), r#"String("Linen Cloth")"#);
        assert_eq!(eval(&lua, "return none"), "Nil", "row 0 is not row 1");
        assert_eq!(eval(&lua, "return shownAfterNone"), "Nil", "…and it hides");
        assert_eq!(eval(&lua, "return bad"), "Nil");
    }

    /// The three substitutions with more than one slot, which a single-slot
    /// `substitute` would silently truncate: a stat's sign and value, a
    /// resistance's sign, value and school, and a bag's size and kind.
    #[test]
    fn the_multi_slot_keys_substitute_every_slot() {
        assert_eq!(substitute(&substitute("%c%d Stamina", "+"), "8"), "+8 Stamina");
        assert_eq!(
            substitute(
                &substitute(&substitute("%c%d %s Resistance", "-"), "5"),
                "Nature"
            ),
            "-5 Nature Resistance"
        );
        assert_eq!(
            substitute(&substitute("%d Slot %s", "16"), "Bag"),
            "16 Slot Bag"
        );
    }

    /// Every listed method is installed and both lists are sorted, as every
    /// method list in this directory must be. The scoped list is checked
    /// inside a scope, the only place its methods exist.
    #[test]
    fn the_lists_and_the_registration_are_the_same_set() {
        let lua = state();
        for name in METHODS {
            assert_eq!(
                eval(&lua, &format!("return type(GameTooltip.{name})")),
                r#"String("function")"#,
                "{name} is claimed in METHODS and is not installed"
            );
        }
        let world = Stub::default();
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            for name in SCOPED_METHODS {
                let kind: String = lua
                    .load(format!("return type(GameTooltip.{name})"))
                    .eval()?;
                assert_eq!(kind, "function", "{name} is claimed in SCOPED_METHODS and is not installed");
            }
            Ok(())
        })
        .expect("the scope runs");
        for list in [&METHODS[..], &SCOPED_METHODS[..]] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, list, "the lists are kept sorted");
        }
    }

    /// The `%.3g` and substitution helpers print as C's `printf` does. The
    /// formats are the game's, and "3.500 sec cast" would be wrong.
    #[test]
    fn the_format_helpers_print_like_the_game() {
        assert_eq!(g3(3.5), "3.5");
        assert_eq!(g3(2.0), "2");
        assert_eq!(g3(0.5), "0.5");
        assert_eq!(g3(12.5), "12.5");
        assert_eq!(g3(150.0), "150");
        assert_eq!(substitute("%d Mana", "35"), "35 Mana");
        assert_eq!(substitute("%s yd range", "30"), "30 yd range");
        assert_eq!(substitute("%.3g sec cast", "3.5"), "3.5 sec cast");
        assert_eq!(substitute("Level %s", "60"), "Level 60");
        assert_eq!(trim_number(30.0), "30");
        assert_eq!(trim_number(27.5), "27.5");
    }
}
