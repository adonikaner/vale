//! **The `GameTooltip`: the one widget whose contents the C side writes.**
//!
//! Every other widget in the directory is filled by Lua; a tooltip is filled by
//! the *client* — `OnEnter` calls `GameTooltip:SetAction(slot)` and the client
//! composes the lines. Until this module none of those calls existed, and since
//! a nil method aborts the body it is in, **every `OnEnter` in the game that
//! showed a tooltip died on its first population call** — the two on the HUD
//! were `ActionButton.lua:377` (`SetAction`) and `BuffFrame.lua:244`
//! (`SetInventoryItem`), and the ranking behind them was topped by the same
//! family (`SetInventoryItem` ×8, `SetUnitDebuff` ×8).
//!
//! ## The shape is the game's own template
//!
//! `GameTooltipTemplate.xml` declares the whole visible shape: the `<Backdrop>`
//! (`UI-Tooltip-Background` + `UI-Tooltip-Border`, edge 16, insets 5), and a
//! ladder of **thirty hidden `FontString` pairs** — `$parentTextLeft1` at
//! `TOPLEFT (10, -10)`, each next left chained `TOPLEFT` to the previous
//! `BOTTOMLEFT (0, -2)`, line 1 in `GameTooltipHeaderText` and the rest in
//! `GameTooltipText`. So the pad is 10, the line gap is 2, and the lines are
//! **real named regions** (`GameTooltipTextLeft1`), because the directory
//! addresses them by name — `GameTooltip.xml`'s own `OnEvent` recolours
//! `TextLeft1` on `UPDATE_MOUSEOVER_UNIT`. This module *adopts* the declared
//! ladder and grows past it by creating more of the same, which is the real
//! class's behaviour (its template also stops at 30).
//!
//! What the template does **not** state is the tooltip's size or the right
//! column's place — the real client's line layout computes both. Here that is
//! [`reflow`]: width = the widest line (+ the two-column gap) floored by
//! `SetMinimumWidth`, height = the lines summed, both padded by 10 — with the
//! line widths **measured** in the game's own typefaces, and a row the face's
//! own line box rather than the declared font height. See
//! [`super::text::width`], which is the one door every measurement in the
//! interface goes through. It was an estimate of half the font height per
//! character until this round, against a mean advance of 0.609 em: the plate
//! came out ten units short of `"BM Only OFF"` and, because a `FontString`
//! centres by default, the overflow came out of *both* sides of the border.
//!
//! ## The rules that are the client's
//!
//! Three behaviours here are the 1.12.1 client's own, from its tooltip
//! bindings (`AddLine`, `SetText`, `AddDoubleLine`, and the default colour
//! `0xffffd200`):
//!
//! * **an uncoloured line is gold**, `255/210/0` — not white. `AddLine`'s
//!   colour block applies only when the *r-slot is a number*; the corpus'
//!   archaic `AddLine(text, "", 1.0, 1.0, 1.0)` shape has `""` there, so the
//!   whole tail drops and the line renders the default gold.
//! * **`SetText` shows the tooltip; `AddLine` does not.** The corpus never
//!   calls `Show()` after `SetText` and always may after `AddLine`.
//! * **`Hide` drops the owner and the lines** and fires `OnTooltipCleared` —
//!   which is what keeps `UnitFrame_OnUpdate`'s `IsOwned` gate from
//!   resurrecting a tooltip the pointer has left.
//!
//! The **owner anchor law** (`SetOwner`'s `ANCHOR_*` words) is the documented
//! 1.12 set: the tooltip hangs its corner off the owner's — `ANCHOR_RIGHT` is
//! this `BOTTOMLEFT` on the owner's `TOPRIGHT`, and so on around the compass.
//! `ANCHOR_NONE` leaves anchoring to the caller, which is what
//! `GameTooltip_SetDefaultAnchor` does with it.
//!
//! ## Population answers the live world, or says nothing
//!
//! `SetAction` and `SetUnit` are **scoped reads** — registered per call by
//! [`super::super::api::install`] like every other question the interface asks,
//! because a tooltip's contents are the world's state at the moment of the
//! hover. The line *law* for a spell is name | rank, cost | range, cast time |
//! cooldown, each cell omitted when absent — and the **format strings are the
//! game's own globals** (`MANA_COST`, `SPELL_RANGE`, `SPELL_CAST_TIME_SEC`…),
//! read out of the environment `GlobalStrings.lua` filled, exactly as the real
//! client reads them. A key the file does not carry displays as nothing, which
//! is the client's own behaviour and the project's standing rule.
//!
//! **…and under them the reagents and the description**, which are the two
//! lines a screenshot comparison said were missing. Both arrive already
//! resolved — `SPELL_REAGENTS` ("Reagents: ") is a `GlobalStrings.lua` key like
//! every other word here, but the item *names* behind it come from the server's
//! templates and the sentence's `$s1`/`$d` variables are substituted in
//! [`vale_assets::tables::spelltext`], both on the far side of
//! [`crate::interface::api::spell_tip`]. So this module still only composes: the
//! order is the game's own — name, cost/range, cast/cooldown, reagents, then
//! the sentence in **green**, which is the one colour on the plate that is not
//! gold or white.
//!
//! The population methods with **no state behind them** — the bags, the buffs,
//! the merchant — are in [`super::super::api::stubs`], counted apart as always. An empty
//! population hides the tooltip and *keeps* the owner, so a refresh loop keeps
//! its gate; `Hide` is the one that lets go.
//!
//! ## …and the one plate nothing in the directory asks for
//!
//! Every other population here is reached from Lua — an `OnEnter` body calls
//! `SetAction`, `SetBagItem`, `SetSpell`. The **world mouseover** is not: 5875's
//! `WorldFrame` declares no `<OnEnter>` at all, so nothing in the shipped files
//! ever puts a unit's plate on screen. The real client does it from C, on the
//! frame the pointer crosses the unit's edge, and [`TooltipPlugin`] is that
//! half: it anchors, fills and hides the same `GameTooltip` every other caller
//! uses, through the directory's own `GameTooltip_SetDefaultAnchor`.

use bevy::prelude::*;

use super::super::api::{one_or_nil, Answers, LuaWorld};
use super::super::host::LuaHost;
use super::frames;
use super::regions;
use super::widget;
use crate::interface::api::SpellTip;

/// **The world mouseover's own plate**, which is the one population no
/// `<OnEnter>` in the directory reaches. See the module comment.
pub struct TooltipPlugin;

impl Plugin for TooltipPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // **After the whole of `GameSet` and before the event dispatch.** The
            // plate has to hold this unit's name by the time
            // `UPDATE_MOUSEOVER_UNIT` reaches `GameTooltip.xml`'s handler, whose
            // entire body recolours `GameTooltipTextLeft1` — a recolour that
            // arrives first paints the *previous* unit's name and then the fill
            // overwrites it, which is a plate that is permanently one hover
            // behind on colour alone.
            show_world_tooltip
                .after(crate::interface::GameSet)
                .before(super::super::api::events::dispatch),
        );
    }
}

/// Fill the `GameTooltip` for whatever the pointer is over, and take it down
/// again when it leaves.
///
/// **On the change and not per frame**, which is [`crate::interface::target`]'s rule
/// for the same reason: `SetUnit` clears and re-appends every line, and doing
/// that sixty times a second would throw away the interface's solved rectangles
/// as fast as it computes them — the exact fault the container frames' own
/// `SetOwner` loop was found to have.
///
/// The two chunks are the reference's own calls, in its own order.
/// `GameTooltip_SetDefaultAnchor` is `GameTooltip.lua`'s, and it is what puts
/// the plate at the bottom right rather than at the pointer.
fn show_world_tooltip(
    host: Option<NonSendMut<LuaHost>>,
    world: LuaWorld,
    hovered: Res<crate::interface::target::Hovered>,
    // …and the other half of the same pick — see
    // [`crate::interface::object::HoveredObject`]. Exactly one of the two is
    // ever filled, which is what lets the three branches below be a plain `if`.
    object: Res<crate::interface::object::HoveredObject>,
    // **The window, only to know when the pointer has moved** — see [`Latch`].
    // Where a floating plate actually *lands* is `GetCursorPosition()`'s answer
    // inside the chunk, not this; this is the same conversion through the same
    // `Viewport`, which is the type `lua::api::mouse` uses for the same job and
    // is deliberately not re-derived here.
    windows: Query<&bevy::window::Window, With<bevy::window::PrimaryWindow>>,
    // …and the scale that conversion runs at — see [`crate::ui::scale`].
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut last: Local<Option<Latch>>,
) {
    let Some(host) = host else { return };
    if !host.loaded() {
        return;
    }
    // **Hidden through the directory's own gate, not unconditionally.** A plate
    // a *button* owns must survive the pointer leaving a unit behind it — the
    // owner test is what `GameTooltip_OnHide` and `UnitFrame_OnUpdate` both
    // branch on, and it is `default`, the flag `GameTooltip_SetDefaultAnchor`
    // sets and nothing else does.
    let plate = if hovered.guid.is_some() {
        Plate::Unit
    } else if object.guid.is_none() || object.name.is_empty() {
        Plate::None
    } else if object.hover.floating {
        Plate::Floating
    } else {
        Plate::Object
    };
    // **What is compared is the whole plate and not just the guid**, for two
    // reasons a guid alone would miss. A game object's name arrives a round trip
    // after the object does, so a guid-only latch draws a nameless plate once
    // and never corrects it — the same trap the loot window's two-second name
    // beat was. And a **floating** plate is positioned by where the pointer is,
    // so it has to be redrawn as the pointer moves across the sign — which is
    // what the last field is, and why it is `None` for every other kind of
    // plate: a unit's plate must not be rebuilt sixty times a second for a
    // pointer twitching inside it.
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
    // **What was on screen a moment ago, not what is now**, which only the
    // `Plate::None` arm reads: a plate the pointer has just left is taken down
    // on its *own* terms rather than the terms of whatever it left onto.
    let previous = last.as_ref().map_or(Plate::None, |latch| latch.plate);
    *last = Some(now);
    let live = world.live();
    // Errors are the host's to record, on the same terms as every other chunk it
    // runs: a broken plate must not take the frame down.
    let _ = host.run(&live, |lua| {
        let chunk = lua
            .load(plate.body(previous))
            .set_name("mouseover")
            .into_function()?;
        // **The name and the lock line are *arguments*, never interpolated into
        // the chunk.** A template name is server data with a quote in it as far
        // as this is concerned, and pasting one into a Lua source string is an
        // injection with the server on the other end of it.
        let chunk = match plate {
            Plate::Object | Plate::Floating => chunk.bind(lock_arguments(&object.plate, &object.name))?,
            _ => chunk,
        };
        frames::protected(lua, &chunk)
    });
}

/// **The ten values both world-plate chunks take**, in the order they unpack
/// them.
///
/// Flat rather than a table because a `bind` takes a tuple and 1.12 has no
/// table constructor worth building here, and because the two chunks are
/// otherwise identical — the same ten names, the same two `if`s, a different
/// anchor.
///
/// **A colour is three numbers rather than a `|cff` prefix** on purpose: the
/// reference colours these lines through `AddLine`'s own arguments, and a
/// markup prefix would survive into `GameTooltipTextLeft2:GetText()` where an
/// addon reads it.
type LockArguments = (String, bool, f32, f32, f32, Option<&'static str>, String, f32, f32, f32);

/// [`LockArguments`], out of the judged plate.
fn lock_arguments(plate: &crate::interface::object::LockPlate, name: &str) -> LockArguments {
    let [lr, lg, lb] = plate.locked_colour;
    let [rr, rg, rb] = plate.requires_colour;
    // **A key with no argument draws nothing**, which is the reference's own
    // `if (!rec) goto finish` for an item name that has not arrived. Folded
    // here rather than in the chunk so that the Lua stays two plain `if`s.
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

/// What [`show_world_tooltip`] compares one frame against the last, so that the
/// plate is rebuilt when it would say — or sit — somewhere different, and never
/// otherwise.
#[derive(PartialEq)]
struct Latch {
    /// Which plate this frame drew — kept so that the *next* one knows what it
    /// is taking down. A floating plate goes at once and every other kind
    /// fades; see [`Plate::body`]'s last two arms.
    plate: Plate,
    guid: Option<u64>,
    name: String,
    /// **The whole of the two lines under the name**, colours included —
    /// because both of them move without the guid or the name moving. A
    /// strongbox that has just been picked loses its "Locked" line, and a vein
    /// changes colour the moment a skill point lands.
    lock: crate::interface::object::LockPlate,
    /// Where the pointer is, in whole interface units, and **only for a plate
    /// that follows it**. See the field's own note in the function above.
    at: Option<(i32, i32)>,
}

/// Which of the four world plates the pointer wants.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Plate {
    Unit,
    /// A game object a click acts on: a door, a chest, an ore vein. Its plate
    /// goes where a unit's goes.
    Object,
    /// **…and one that can only be looked at**, whose plate follows the pointer
    /// — the street signs. `GAMEOBJECT_TYPE_GENERIC`'s own `floatingTooltip`,
    /// see [`vale_assets::look::object::hover_of`].
    Floating,
    None,
}

impl Plate {
    /// The chunk, which is the game's own calls in the game's own order.
    ///
    /// **A game object's plate is composed here rather than by a
    /// `GameTooltip:Set*` method, because 1.12 has none.** The ninety FrameXML
    /// files carry `SetUnit`, `SetBagItem`, `SetLootItem` and thirteen more, and
    /// nothing for a game object at all — the reference draws that plate from
    /// the C side, which is what this is. What it must not do is invent the
    /// words: `LOCKED_WITH_SPELL_KNOWN` is `GlobalStrings.lua`'s own key and it
    /// is read out of the interface's globals rather than copied into Rust.
    ///
    /// **Which of that file's three lock strings this is, and what colour it
    /// takes, is decided in Rust** — see
    /// [`crate::interface::object::LockPlate`], which follows the client. The
    /// chunk is handed a key and looks it up with `getglobal`, so the *wording*
    /// is still the archive's and the *choice* is still the client's; nothing
    /// here invents either.
    ///
    /// All three of `LOCKED_WITH_SPELL`, `LOCKED_WITH_SPELL_KNOWN` and
    /// `LOCKED_WITH_ITEM` read `"Requires %s"` in 1.12, so a client that always
    /// picked one of them drew the right words in the wrong colour — and the
    /// colour is the whole of what the line says about *you*. The `LOCKED` line
    /// above it is the same story one field up.
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
            // **The floating one, and it is `SetDefaultAnchor` with a different
            // point.** 1.12 has no `ANCHOR_CURSOR` — grepping the ninety files
            // for one is how that was settled — because in the reference the
            // *C side* decides where a world plate goes, and this is the C
            // side. So it does exactly what `GameTooltip_SetDefaultAnchor`
            // does, out of `GameTooltip.lua`: own the tooltip with
            // `ANCHOR_NONE`, place it by hand, and set `default` so that
            // `Plate::None` above can take it down again.
            //
            // **`GetCursorPosition()` is already in the interface's own space**
            // — origin bottom left, y up, scale divided out — which is what
            // `SetPoint`'s offsets want, so there is no conversion here. See
            // `lua::api::mouse`, which explains why the directory's own callers
            // divide by `GetEffectiveScale()` again and this does not.
            //
            // The nudge is this client's: the plate's bottom-left corner sits
            // up and to the right of the pointer so the pointer is not standing
            // on its own tooltip. **It does not clamp to the screen**, so a sign
            // hovered within a plate's height of the top edge draws partly off
            // it; the reference flips the anchor there and this does not yet.
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
            // **The pointer left, and the plate does not go at once.**
            // `FadeOut` rather than `Hide` is the reference's own choice on
            // this path: `UnitFrame_OnLeave` calls exactly this on the branch
            // where newbie tips are off, and the world mouseover is the same
            // loss one layer down — the C side's, which is what this is. See
            // [`FADE_HOLD`].
            //
            // Still behind `default`, which is unchanged and load-bearing: a
            // plate a *button* owns must not be faded out by the pointer
            // leaving a unit behind it.
            //
            // **…except the floating one, which goes the instant the pointer
            // does.** A street sign's plate is not anchored to a frame the
            // pointer can be "still near": it is nailed to the cursor, so a
            // three-second ramp draws a name hanging in the world beside a sign
            // the pointer has already walked off — and, because the plate is
            // rebuilt on every pointer move, sweeping across a row of signs
            // leaves a fading one behind at each. `Hide` funnels through
            // [`dropped`], which is where the fade mark and the alpha come off,
            // so this is the same door every other disappearance uses.
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

/// …and the ones installed per scope, because they answer the live world.
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

/// What a tooltip keeps on itself. Underscored, as everything the C side owns.
const LINES_KEY: &str = "__tipLines";
const OWNER_KEY: &str = "__tipOwner";
const MIN_WIDTH_KEY: &str = "__tipMinWidth";
const PADDING_KEY: &str = "__tipPadding";
/// The line pool: two arrays of `FontString` tables, left and right columns.
/// Held directly rather than re-found by name per call — an unnamed tooltip
/// (legal from Lua) has no names to find its lines by.
const LEFT_KEY: &str = "__tipLeft";
const RIGHT_KEY: &str = "__tipRight";

/// The two region fields the size estimate reads — named here rather than
/// imported for the reason [`super::super::api::stubs`] gives about its own copies: this
/// module writes neither.
const FONT_HEIGHT_KEY: &str = "__fontHeight";
const FONT_KEY: &str = "__font";
/// …and the one this module *does* write, through [`regions::set_wrap`]; read
/// back here by the size estimate.
const WRAP_KEY: &str = "__wrap";

/// The template's own geometry: `TextLeft1` at `(10, -10)` and the chain offset
/// `-2`. See the module comment — these are the file's numbers, not choices.
const PAD: f64 = 10.0;
const LINE_GAP: f64 = 2.0;
/// The air between the two columns of a double line. **A choice, not a
/// reading**: the template's static right-column anchors are placeholders the
/// real layout overwrites, and nothing in the files states the gap it uses.
const COLUMN_GAP: f64 = 10.0;
/// Where a wrapped line folds, in pixels of text per row. **A stated stand-in,
/// not a reading** — the real client's wrap width comes out of its own line
/// layout and nothing in the files states it; this is eyeballed against the
/// newbie tooltips, which are the corpus' main wrap=1 callers. Without *any*
/// fold, `GameTooltip_AddNewbieTip`'s explanation line made the plate as wide
/// as the sentence — 900 px of tooltip across the bottom of the screen.
const WRAP_WIDTH: f64 = 280.0;
/// A line whose font never resolved still occupies a row — the same 12 the
/// rest of this directory falls back to.
const DEFAULT_LINE_HEIGHT: f64 = 12.0;

/// The engine's default text colour: **gold**, `0xffffd200`. The client's own —
/// see the module comment for the archaic-`AddLine` shape that makes it
/// visible.
const GOLD: [f64; 4] = [1.0, 210.0 / 255.0, 0.0, 1.0];
/// The spell tooltip's own two: the name's white and the rank column's gray.
const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];
const GRAY: [f64; 4] = [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0];
/// **The red a requirement the character does not meet is drawn in**, stated
/// here only for the test that pins it: the plate draws
/// [`crate::interface::plate::Ink::Red`]. Written out independently so a change to that colour fails a test.
#[cfg(test)]
const RED: [f64; 4] = [1.0, 32.0 / 255.0, 32.0 / 255.0, 1.0];

/// **How long a plate the pointer has left stays put, and how long it takes to
/// go.** Seconds; the ramp between them is linear.
///
/// `GameTooltip:FadeOut()` is a **C method**. It appears exactly once in the
/// 175 files — `UnitFrame_OnLeave`, and only on the branch where newbie tips
/// are *off*; with them on the same body calls `Hide()` outright — and nothing
/// in the archives defines it. So its two durations are inside the client,
/// and nothing in the archives states them.
///
/// **These numbers are borrowed, not measured, and that is the honest half.**
/// What the archives do state is the *shape*, and one instance of it:
///
/// * the shape is `FadingFrame.lua`'s — fade in, hold, **linear** fade out,
///   then hide. That is 1.12's own idea of a frame that goes away by itself,
///   and `UIParent.lua`'s `UIFrameFade` is linear too, so there is no easing
///   anywhere in this game's interface to imitate;
/// * the numbers are `ZoneText.xml`'s, which is the one hold-then-fade the
///   directory spells out: `ZoneHoldDuration = 1.0`,
///   `ZoneFadeOutDuration = 2.0`. (`UI.xsd` gives a `MessageFrame` a default
///   `fadeDuration` of 3.0 over a `displayDuration` of 10.0 — the same shape
///   again, slower, for text that scrolls rather than a plate that is left.)
///
/// The fade *in* is zero: the reference's plate appears at once, and a tooltip
/// that eased in would be visible on every hover rather than only on the ones
/// this is about.
const FADE_HOLD: f64 = 1.0;
const FADE_OUT: f64 = 2.0;

/// How far into the fade a frame is, in seconds. Absent on a frame that is not
/// fading, which is every frame in the game almost all of the time — so the
/// hooks that cancel a fade can gate on one raw read.
const FADE_KEY: &str = "__fading";

/// Where a frame with a fade in progress is remembered.
///
/// The registry rather than a global, for [`super::super::api::update`]'s reason: interface
/// code must not be able to stop the ramp with one assignment. A list rather
/// than a walk for [`super::slider`]'s: 3,785 frames looked at per tick to find
/// the nought or one that is fading is not a walk this client can afford, and
/// the two sweeps this rides beside are the same shape.
const REG_FADING: &str = "vale.fadingFrames";

/// Is this object a `GameTooltip`? The kind gate for [`dropped`], which is
/// called from the shared `Hide` every frame in the game goes through.
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

/// **Line pair `index`, adopting the template's ladder before creating.**
///
/// The declared regions are named `<name>TextLeft<i>` / `TextRight<i>` and are
/// already this frame's children; a pair past the ladder is created as more of
/// the same — named, `ARTWORK`, the left chained under the previous left, the
/// faces copied from the line above so a grown line keeps the template's text
/// font rather than resetting to nothing.
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
            // Adopt the declared region if the loader made one — it is a
            // global whose parent is this frame. Checked, because a second
            // tooltip's lines must not be captured by a name collision.
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
                    // The faces ride down from the line above; with no line
                    // above (a Lua-made tooltip) the defaults stand.
                    if let Some(previous) = &previous {
                        if let Some(font) = previous.raw_get::<Option<String>>(FONT_KEY)? {
                            regions::set_font(&region, &font)?;
                        }
                        if let Some(height) = previous.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)? {
                            regions::set_font_height(&region, height as f32)?;
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
            region.set(super::widget::SHOWN_KEY, false)?;
            pair.push(region);
        }
        lefts.push(pair[0].clone())?;
        rights.push(pair[1].clone())?;
    }
    Ok((lefts.get(index)?, rights.get(index)?))
}

/// The client's own truth for an optional flag argument — see
/// [`super::super::api::to_boolean`], which is the client's coercion rather than
/// Lua's. The corpus writes `1` here; what the local copy of Lua's rule got
/// wrong was the `0` on the other side of it.
fn truthy(value: Option<&mlua::Value>) -> bool {
    super::super::api::to_boolean(value, true)
}

/// A colour component the way the binding reads one: `lua_tonumber`'s coercion,
/// so a numeric string counts and `""` does not.
fn component(value: Option<&mlua::Value>) -> Option<f64> {
    match value {
        Some(mlua::Value::Integer(n)) => Some(*n as f64),
        Some(mlua::Value::Number(n)) => Some(*n),
        Some(mlua::Value::String(s)) => s.to_string_lossy().trim().parse().ok(),
        _ => None,
    }
}

/// **The colour gate**: the block applies only when the r-slot is a number —
/// anything else drops the whole tail to the default gold. When it passes, g
/// and b are ungated reads defaulting to 0. See the module comment for why the
/// wrong reading here paints every zone tooltip white instead of gold.
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

/// Write one cell: text (through the one stringify door), colour, wrap, shown.
fn write_cell(
    lua: &mlua::Lua,
    region: &mlua::Table,
    text: mlua::Value,
    colour: [f64; 4],
    wrap: bool,
) -> mlua::Result<()> {
    regions::set_text_value(lua, region, text)?;
    regions::set_colour(region, colour)?;
    regions::set_wrap(region, wrap)?;
    region.set(super::widget::SHOWN_KEY, true)
}

/// Append one line — both columns, the right one optional, the wrap flag the
/// left's — and re-solve the plate. Does **not** show: that split is the
/// binding's own (module comment).
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

/// **Blank every line and fire `OnTooltipCleared`.** The minimum width is
/// content and resets with it; the padding is a frame property and survives.
fn clear(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    let (lefts, rights) = columns(lua, this)?;
    for list in [lefts, rights] {
        for region in list.sequence_values::<mlua::Table>().flatten() {
            region.set(super::widget::SHOWN_KEY, false)?;
            regions::set_text_value(lua, &region, mlua::Value::Nil)?;
            regions::set_wrap(&region, false)?;
        }
    }
    this.set(LINES_KEY, 0_i64)?;
    this.set(MIN_WIDTH_KEY, 0.0_f64)?;
    // **A plate with something new to say is not fading**, and this is the one
    // door every population comes through — `SetOwner` (so
    // `GameTooltip_SetDefaultAnchor`, so every world plate and every
    // `UnitFrame_OnEnter`), `SetText`, and `Hide` by way of [`dropped`]. Putting
    // the cancel here rather than on each of them is what keeps the alpha and
    // the mark from being restored in three places and forgotten in a fourth.
    cancel_fade(lua, this)?;
    // Swallowed for the reason `Show` swallows `OnShow`'s: the state is
    // cleared either way, and a broken handler must not fail the `SetOwner`
    // that is only passing through here.
    let _ = frames::run_script(lua, this, "OnTooltipCleared", &[]);
    Ok(())
}

/// One line's cell sizes: `(width, height)`, `(0, 0)` for a cell that holds
/// nothing.
///
/// **Measured, not estimated** — see [`regions::text_width`]. It used to be
/// half the font height per character, against Friz Quadrata's own mean advance
/// of 0.609 em — and a plate narrower than its text is the one this project was
/// looking at when it wrote this line.
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
    // **A row is the face's line box, not the declared font height** — 14.6 for
    // Friz Quadrata at 12. The two are what a `<FontHeight>` means and what a
    // line of it occupies, and taking the first for the second stacked the
    // tooltip's lines 2.6 units closer together than the glyphs they hold.
    let height = regions::line_height_of(lua, region).max(DEFAULT_LINE_HEIGHT);
    if text.is_empty() {
        // The corpus' `AddLine(" ")` spacer is a real row; a truly empty text
        // still charges its slot so `NumLines` and the height agree.
        return (0.0, height);
    }
    let measured = regions::text_width(lua, region, &text);
    // **A wrapped cell folds at [`WRAP_WIDTH`] and charges the rows it actually
    // folds into** — counted by [`regions::text_rows`], which is the same
    // measurement the painter lays the galley out by.
    //
    // It used to be `ceil(measured / WRAP_WIDTH)`, which is the row count only
    // if every break lands exactly on the fold. Prose does not: a word that will
    // not fit leaves the tail of its row empty, so the estimate runs one row
    // short from about three rows on and the plate is drawn a line too shallow.
    // Shield Bash's description is the measured case — 47 words over four rows,
    // estimated at three, with "for 6 sec." printed below the bottom border.
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

/// **Solve the plate**: size every cell, hang each right cell off the plate's
/// right edge, and size the tooltip itself. Run after every content change —
/// a tooltip is a handful of lines, so this is arithmetic, not a walk.
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
            // The template's static right-column anchor is a placeholder the
            // real layout overwrites; here the overwrite is one point — the
            // cell's top-right on the plate's, at this line's own depth.
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

/// **`FadeOut()` — hold this plate where it is, then take it away.**
///
/// It does *not* clear the lines or the owner, which is the whole difference
/// between it and `Hide`: the plate has to keep saying what it said for as long
/// as it is on the screen. [`fade_sweep`] calls `Hide` at the end, and that is
/// where the letting-go happens, on the one path it has always happened on.
///
/// Fading something already down is nothing to do — `Hide` has been through
/// here and taken the contents with it — and fading something already fading
/// does **not** restart the hold, because the one caller in the directory sits
/// on `OnLeave` and a pointer skimming an edge fires that repeatedly.
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
    this.set(super::widget::ALPHA_KEY, 1.0_f64)?;
    let list = fading(lua)?;
    list.raw_push(this.clone())
}

/// **Stop a fade and put the alpha back**, which is what anything that shows or
/// re-populates the frame has to do.
///
/// Two hooks reach it and between them they cover every path: [`clear`], which
/// every population and every `Hide` goes through, and the shared `Show` — see
/// [`unfade`], which is that one's gate.
fn cancel_fade(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if this.raw_get::<Option<f64>>(FADE_KEY)?.is_none() {
        return Ok(());
    }
    this.set(FADE_KEY, mlua::Value::Nil)?;
    this.set(super::widget::ALPHA_KEY, 1.0_f64)?;
    let list = fading(lua)?;
    for (index, entry) in list.sequence_values::<mlua::Table>().enumerate() {
        if entry? == *this {
            return list.raw_remove(index + 1);
        }
    }
    Ok(())
}

/// **The `Show` hook**, the mirror of [`dropped`]: a frame brought back is a
/// frame that is not going anywhere. Called from the shared `Show` for every
/// object in the game, so the gate is one raw read of a key almost nothing
/// carries.
///
/// It is not `is_tooltip`-gated, and that is deliberate: `FadeOut` lives in the
/// one shared method table, so any frame can be told to fade and any frame that
/// was must be able to stop.
pub(super) fn unfade(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    cancel_fade(lua, this)
}

/// **Advance every fade by one interface tick.**
///
/// Rides [`super::super::api::update::fire`] beside [`super::scrollframe::sweep`] and
/// [`super::slider::sweep`], and for their reason: this is the clock every
/// other fade in the game is on — `UIParent`'s single `OnUpdate` is what walks
/// `FADEFRAMES` — and a plate fading on a different one would dissolve at a
/// different speed from the chat frame beside it. Before that pass' own
/// early-out, because a fading tooltip needs no `OnUpdate` anywhere.
///
/// `elapsed` is the tick's own seconds, not the rendered frame's — see
/// [`super::super::api::update::InterfaceClock::advance`]. Integrated rather than sampled, so
/// the fade covers the same ground in the same wall-clock time at any rate.
pub(in crate::lua) fn fade_sweep(lua: &mlua::Lua, elapsed: f64) {
    let Ok(list) = lua.named_registry_value::<mlua::Table>(REG_FADING) else {
        return;
    };
    if list.raw_len() == 0 {
        return;
    }
    // **A snapshot**, for the reason the `OnUpdate` walk takes one: finishing a
    // fade ends in `Hide`, which runs `OnHide` handlers, which may show or hide
    // anything at all — including another fading frame.
    let frames: Vec<mlua::Table> = list.sequence_values::<mlua::Table>().flatten().collect();
    for frame in frames {
        // Re-read rather than trusted from the snapshot: an earlier frame's
        // `OnHide` may have cancelled this one.
        let Ok(Some(at)) = frame.raw_get::<Option<f64>>(FADE_KEY) else {
            continue;
        };
        let at = at + elapsed.max(0.0);
        if at >= FADE_HOLD + FADE_OUT {
            // `Hide` funnels into [`dropped`] and therefore into [`clear`],
            // which is where the mark comes off and the alpha goes back — so
            // this must not do either itself, or the two would be two answers
            // to one question.
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
        let _ = frame.set(super::widget::ALPHA_KEY, alpha.clamp(0.0, 1.0));
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

/// **An empty population hides the plate and keeps the owner** — the refresh
/// loops (`UnitFrame_OnUpdate`'s `IsOwned` gate) keep their claim, and only a
/// real `Hide` lets go. The flag is written directly for exactly that reason:
/// the `Hide` method funnels into [`dropped`].
fn conceal(lua: &mlua::Lua, this: &mlua::Table) -> mlua::Result<()> {
    if this
        .raw_get::<Option<bool>>(super::widget::SHOWN_KEY)?
        .unwrap_or(true)
    {
        this.set(super::widget::SHOWN_KEY, false)?;
        let _ = frames::run_script(lua, this, "OnHide", &[]);
    }
    Ok(())
}

/// **The `Hide` hook**: a hidden tooltip lets go of everything — the owner,
/// the lines, and it says so through `OnTooltipCleared`. Called by the shared
/// `Hide` on every frame; the kind gate is here so the caller stays one line.
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
    // SetOwner(owner, "ANCHOR_RIGHT", x, y) — remember the owner, drop the old
    // contents (a fresh owner never inherits the last hover's lines), and hang
    // the plate off the owner per the anchor law.
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
            // (this corner, on the owner's corner) — the compass law in the
            // module comment. An unknown word takes the commonest anchor
            // rather than none, so a typo shows a misplaced tooltip instead
            // of an invisible one.
            let points = match word.as_str() {
                "ANCHOR_NONE" => {
                    // The caller anchors it — and the stale owner anchor goes
                    // now, or it wins the frame the caller forgets to.
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
                // **One decision, not a clear and a set** — see
                // [`widget::set_only_point`]. `ContainerFrameItemButton_OnUpdate`
                // re-runs `OnEnter` every frame the pointer is over a bag
                // square, so a `SetOwner` that always invalidated threw the
                // whole interface's solved layout away at frame rate.
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

    // IsOwned(frame) — is that frame the live owner? The refresh loops' gate.
    let is_owned = lua.create_function(|_, (this, frame): (mlua::Table, Option<mlua::Table>)| {
        let owner: Option<mlua::Table> = this.raw_get(OWNER_KEY)?;
        Ok(one_or_nil(match (owner, frame) {
            (Some(owner), Some(frame)) => owner == frame,
            _ => false,
        }))
    })?;
    methods.set("IsOwned", is_owned)?;

    // SetText(text, r, g, b, a, wrap) — clear, write line 1, **show**. The
    // text is required, and the refusal is the binding's own usage message.
    //
    // **One method table serves every frame kind**, and `SetText` is the one
    // name the tooltip shares with the buttons — so this closure dispatches
    // on the kind and hands everything that is not a tooltip to the ordinary
    // text setter it replaced. Without the branch, every `button:SetText` in
    // the game writes tooltip lines instead of a label.
    let set_text = lua.create_function(|lua, (this, args): (mlua::Table, mlua::MultiValue)| {
        let args: Vec<mlua::Value> = args.into_iter().collect();
        if !is_tooltip(&this) {
            let value = args.into_iter().next().unwrap_or(mlua::Value::Nil);
            // **The shared body**, not a second copy of it: an `EditBox` fires
            // `OnTextSet` from here and a `Button` writes its font string, and
            // this branch is the one every frame in the game reaches. See
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
        // Unlike AddLine's forced-opaque, SetText's alpha is a real argument —
        // `GameTooltip_AddNewbieTip` passes `(text, r, g, b, 1, 1)`.
        if let Some(alpha) = component(args.get(4)) {
            colour[3] = alpha;
        }
        clear(lua, &this)?;
        append(lua, &this, (text, colour), None, truthy(args.get(5)))?;
        set_shown(&this, true)
    })?;
    methods.set("SetText", set_text)?;

    // AddLine(text, r, g, b, wrap) — append, do not show. A wrapping line
    // folds at the stated stand-in width instead of stretching the plate to
    // the sentence — see [`WRAP_WIDTH`].
    let add_line = lua.create_function(|lua, (this, args): (mlua::Table, mlua::MultiValue)| {
        let args: Vec<mlua::Value> = args.into_iter().collect();
        let text = args.first().cloned().unwrap_or(mlua::Value::Nil);
        let colour = colour_or_gold(&args, 1);
        append(lua, &this, (text, colour), None, truthy(args.get(4)))
    })?;
    methods.set("AddLine", add_line)?;

    // AddDoubleLine(left, right, rL, gL, bL, rR, gR, bR) — each side's colour
    // gates on its own r-slot.
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

    // SetMinimumWidth(w) — a floor on the auto-size; `SetTooltipMoney` calls
    // it so the coins never overhang the plate.
    let min_width = lua.create_function(|lua, (this, w): (mlua::Table, Option<f64>)| {
        this.set(MIN_WIDTH_KEY, w.unwrap_or(0.0).max(0.0))?;
        reflow(lua, &this)
    })?;
    methods.set("SetMinimumWidth", min_width)?;

    // SetPadding(w) — extra width; a frame property that survives clears.
    let padding = lua.create_function(|lua, (this, w): (mlua::Table, Option<f64>)| {
        this.set(PADDING_KEY, w.unwrap_or(0.0).max(0.0))?;
        reflow(lua, &this)
    })?;
    methods.set("SetPadding", padding)?;

    // FadeOut — hold the plate, then ramp it away. See [`begin_fade`] and
    // [`FADE_HOLD`], which is where the two durations are argued for.
    let fade = lua.create_function(|lua, this: mlua::Table| begin_fade(lua, &this))?;
    methods.set("FadeOut", fade)?;
    Ok(())
}

/// Install the population methods that answer the live world, for the length of
/// one scope — the same lifetime and the same argument as every read in
/// [`super::super::api`].
///
/// **Through [`crate::lua::scoped`] rather than onto the method table
/// directly.** These are the methods `GameTooltip:SetUnit` and its siblings
/// resolve to, and an addon *keeps* one: `libtipscan` reads `v[method]` off the
/// plate and calls it on a later frame to scan a tooltip's text. Written
/// straight into the shared table that capture is a destructed callback the
/// moment the scope closes; written through the forwarder it stays callable for
/// the life of the state.
pub(in crate::lua) fn install_scoped<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    // No object model, no methods — the bare-interpreter tests install the
    // reads without frames, and there is nothing here to put a method on.
    let Some(methods) = frames::methods(lua) else {
        return Ok(());
    };
    let methods = crate::lua::scoped::methods(lua, &methods)?;

    // SetAction(slot) — the spell tooltip, composed by the law in the module
    // comment. Answers 1 when there was something to show, which is what
    // `ActionButton_SetTooltip` branches on to keep the refresh timer.
    //
    // **…or the item plate, for a slot holding an item**, through the same
    // `item_lines` a bag square goes through — a hearthstone hovered on the bar
    // and the same hearthstone hovered in the bag must not print different
    // plates. The two are mutually exclusive by the slot's kind byte, so the
    // order below is not a precedence.
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

    // **SetSpell(id, bookType) — the same plate, reached from the book.**
    // `SpellButton_OnEnter`'s only line, and its `if` is on the answer: a
    // population that returned nothing must not leave the hover's refresh timer
    // armed. `id` is a **row** rather than a spell id — see
    // [`super::super::panels::spellbook`] — and the lines are composed by exactly the law
    // above, through the same [`spell_lines`], so a spell hovered on the bar and
    // the same spell hovered in the book cannot print different plates.
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

    // **SetPetAction(slot) — the same plate again, reached from the pet bar.**
    //
    // `PetActionButton_OnEnter`'s whole body for a spell slot, and the reason
    // an autocastable pet ability had no tooltip at all: the method did not
    // exist, so the `OnEnter` raised on the call and the plate the branch above
    // it would have drawn never happened either. See
    // [`super::super::panels::pet::PetAnswers::pet_action_tooltip`], which is
    // where the reference's own fork is quoted; a token slot answers nothing
    // here and never reaches this at all, because the body takes its other
    // branch for one.
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

    // **The profession windows' three plates.** `SetTradeSkillItem(skill
    // [, reagent])` and `SetCraftItem(craft, reagent)` are the item plate by
    // entry — the created item or a reagent, resolved by the panel against
    // the same list it drew — and `SetCraftSpell(craft)` is the spell plate,
    // through the same `spell_lines` the book's hover uses. An entry the
    // cache has not named yet conceals, which is the reference's own cold
    // plate.
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

    // **The three item plates**, which are one composition reached three ways:
    // out of a bag, off a worn slot, or from a link with no object behind it at
    // all. One `item_lines` for all three, on the spellbook's own precedent —
    // a stack hovered in a bag and the same item hovered on the paper doll
    // must not print different plates.
    //
    // `SetBagItem` and `SetInventoryItem` both answer more than one value, and
    // the callers unpack them positionally:
    //
    // ```lua
    // local hasCooldown, repairCost = GameTooltip:SetBagItem(bag, slot);
    // local hasItem, hasCooldown, repairCost = GameTooltip:SetInventoryItem(u, id);
    // ```
    //
    // — different shapes for the two, which is the game's own asymmetry.
    // `repairCost` is **0 rather than nil**: `ContainerFrame_Update` compares it
    // with `>` two lines later and a nil is an error there. There is no repair
    // in this client, so 0 is the honest number as well as the safe one.
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
                    // `hasCooldown` — nil, since `GetContainerItemCooldown` is
                    // still a constant zero (see [`super::super::api::stubs`]).
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

    // **`SetInventoryItem`'s first answer is what decides the fallback.**
    // `PaperDollItemSlotButton_OnEnter` does `if ( not hasItem ) then
    // GameTooltip:SetText(<the slot's own name>)`, so a nil here is what makes
    // an empty head slot hover as "Head" — and `BagSlotButton_OnEnter` uses the
    // same test to show `EQUIP_CONTAINER` over an empty bag button.
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

    // `SetHyperlink("item:2589:0:0:0")` — the plate with no object behind it,
    // which is what a link in the chat frame opens. Only `item:` links are
    // answered; a `spell:` or `quest:` link is a family this client does not
    // carry, and answering the wrong plate for one would be worse than none.
    let set_hyperlink =
        scope.create_function(move |lua, (this, link): (mlua::Table, Option<String>)| {
            let tip = link
                .as_deref()
                .and_then(super::super::panels::container::entry_of)
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
        })?;
    methods.set("SetHyperlink", set_hyperlink)?;

    // SetUnit(token) — the unit tooltip, composed by [`unit_lines`]. The name's
    // colour is deliberately plain white: recolouring it by reaction is the
    // directory's own job (`GameTooltip.xml`'s `UPDATE_MOUSEOVER_UNIT` handler).
    let set_unit = scope.create_function(move |lua, (this, token): (mlua::Table, Option<String>)| {
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

    // **The three aura plates: the name and the sentence, and nothing between
    // them.** A buff's plate is not a spell's — there is no mana cost, no range
    // and no cast time on it, because the aura is not something you are about
    // to cast — so these deliberately do not go through [`spell_lines`]. See
    // [`super::super::panels::auras::tooltip_lines`], where that is written down.
    //
    // `SetPlayerBuff` takes the same handle everything else in that family
    // does, and gets the `-1` freely; the other two take a token and a
    // **one-based index into one half**.
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

    // **The five populations a *panel* hovers with**, and they were the whole of
    // why nothing on a trainer, a vendor, a corpse or a quest page had a plate:
    // each of these names was simply absent, so the `OnEnter` that called it
    // died on a nil method with the tooltip still hidden — indistinguishable
    // from hovering an empty square, and invisible to every count.
    //
    // Four of the five are the *item* plate, reached four ways, and they all go
    // through the same [`item_lines`] the bags do — a reward hovered on a quest
    // page and the same item hovered in a bag must not print different plates.
    // The fifth is the *spell* plate, for what a trainer will teach.
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

    // `GameTooltip:SetTrainerService(i)` — **one-based row**, and the argument
    // `ClassTrainerSkillIcon`'s `OnEnter` passes is
    // `ClassTrainerFrame.selectedService`, so it is the row the panel last
    // selected rather than the one under the pointer.
    let set_trainer_service =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let row = usize::try_from(row.unwrap_or(0)).ok().filter(|i| *i > 0);
            spell_plate(lua, &this, row.and_then(|row| answers.trainer_tooltip(row)))
        })?;
    methods.set("SetTrainerService", set_trainer_service)?;

    // `GameTooltip:SetTalent(tab, i)` — **both one-based**, and the tab is
    // `TalentFrame.selectedTab` rather than anything the button knows, so a
    // hover before a tab has ever been clicked passes nil and must answer
    // nothing rather than raise.
    //
    // The plate is the *held* rank's spell, which is what makes the numbers in
    // it move as points go in — see [`super::super::panels::talent`].
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

    // **The vendor's shelf and the corpse's rows go through their own links**,
    // which the two panels already answer for the chat-frame link — so the
    // entry is looked up exactly once per population rather than through a
    // second accessor that could disagree with the row the panel drew.
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

    // **The buyback tab's plate**, which is the same item plate off a different
    // list. Its own entry accessor rather than a link, because 1.12 ships no
    // `GetBuybackItemLink` — the shelf and the corpse both have one and this
    // does not, which is why the pattern above cannot simply be repeated.
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

    // **…and the parcel in a letter, and the one on its way out.** Two more
    // item plates off two more lists, and the reason they are here rather than
    // stubbed is the one this file's own first paragraph is about: a method
    // that answers nothing *hides* the plate, which is indistinguishable from
    // hovering an empty slot. `InboxFrameItem_OnEnter` calls the first for
    // every row that has an item — and then adds the enclosed money or the COD
    // under it, which is `SetTooltipMoney`'s job and is the directory's own Lua.
    //
    // By **entry** rather than by link, on the same terms `SetBuybackItem` is:
    // 1.12 ships no `GetInboxItemLink`.
    let set_inbox_item =
        scope.create_function(move |lua, (this, row): (mlua::Table, Option<i64>)| {
            let entry = usize::try_from(row.unwrap_or(0))
                .ok()
                .filter(|i| *i > 0)
                .and_then(|row| answers.mail_item_entry(row));
            item_plate(lua, &this, entry)
        })?;
    methods.set("SetInboxItem", set_inbox_item)?;

    // **The draft's, which takes no argument at all** — there is only ever one
    // thing attached, so `GameTooltip:SetSendMailItem()` names it by there
    // being nothing else it could mean.
    let set_send_mail_item = scope.create_function(move |lua, this: mlua::Table| {
        item_plate(lua, &this, answers.mail_send_entry())
    })?;
    methods.set("SetSendMailItem", set_send_mail_item)?;

    // **…and the same plate off a group roll**, which is the one hover in the
    // game reached from a frame rather than from a list: the icon on a
    // `GroupLootFrame` is a `<Button>` whose `OnEnter` calls this with the
    // frame's own `rollID`.
    //
    // **Zero is a real roll id** — the counter starts there — so the filter is
    // on the sign alone, unlike every row number above.
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

    // **…and the same plate off either side of a trade**, by the square's id
    // 1..7. The partner's item is a template the handler asked for as the
    // offer arrived, so the plate is empty for the round trip and full after
    // it — see [`super::super::panels::trade`].
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

    // **`SetQuestItem(word, i)` takes the same three words `GetQuestItemInfo`
    // does**, and for the same reason: three arrays in three packets behind one
    // function. A host that answered the wrong array would print the right
    // number of plates naming the wrong items.
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

    // …and the log's two, which are the same arrays off the *selected* row
    // rather than off a page of dialogue. `"choice"` and `"reward"` are the only
    // two words `QuestLogRewardItemTemplate` ever passes — a log entry has no
    // required-items array, because the panel that shows one is the giver's.
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

    // **And the reward *spell* button's two**, which the same template reaches
    // through `this.rewardType == "spell"`. Neither was registered at all, so
    // hovering the one button on a quest page that teaches something took the
    // whole `OnEnter` down with it.
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

/// **The unit line law** — the client's own, behind `GameTooltip:SetUnit`.
///
/// Unlike the item plate's order (which this file states is a reconstruction),
/// this one is the client's exactly:
///
/// ```text
/// 1  the name                              gold, the engine default
/// 2  <subname>                             a creature template's tag
/// 3  Level <level> <class> (<type>)        the four TOOLTIP_UNIT_LEVEL* keys
/// 4  PvP                                   on UNIT_FIELD_FLAGS bit 12
/// ```
///
/// The third line is the whole of the shape. Two cells are decided first and
/// then a format is *chosen by which of them is non-empty*:
///
/// * **class** is, for a player-controlled unit, `"%s %s"` of race then class
///   — which is what makes the line read "Level 5 Human Warrior" — and for
///   anything else the localised `CreatureType.dbc` name.
/// * **type** is the literal `PLAYER` key for a player-controlled unit, and
///   otherwise the classification word from a five-entry key table, which is
///   empty for a normal *and for a rare* creature.
///
/// so the four keys are `TOOLTIP_UNIT_LEVEL_CLASS_TYPE`, `…_CLASS`, `…_TYPE`
/// and `TOOLTIP_UNIT_LEVEL`, in that precedence. A level of zero or less prints
/// `"??"` rather than the number, which is what a unit far above the player reads as.
///
/// **What this deliberately does not draw**, each because the state behind it
/// does not exist here rather than because the builder does not: a player's
/// **guild** (this client has no `SMSG_GUILD_QUERY` for it), `RESURRECTABLE`,
/// `PLAYER_OFFLINE`, and the faction/reaction lines under them. Each is an absent
/// subsystem, so the honest plate is one that omits the line — see
/// [`super::super::api::stubs`]' first paragraph, which is the same argument.
fn unit_lines(lua: &mlua::Lua, this: &mlua::Table, tip: &crate::interface::api::UnitTip) -> mlua::Result<()> {
    // The name takes the plate's own default colour rather than a stated one:
    // `GameTooltip.xml`'s `UPDATE_MOUSEOVER_UNIT` handler overwrites it with
    // `GameTooltip_UnitColor("mouseover")` a moment later, and a colour written
    // here would be one the directory then has to undo.
    append(lua, this, (text(lua, &tip.name)?, GOLD), None, false)?;
    if !tip.sub_name.is_empty() {
        append(lua, this, (text(lua, &tip.sub_name)?, WHITE), None, false)?;
    }

    // `%d` of the level, or the client's own "??" — a literal in the client
    // rather than a `GlobalStrings.lua` key, so it is one here too.
    let level = if tip.level > 0 { tip.level.to_string() } else { "??".to_string() };
    let class = match (tip.race, tip.class) {
        (Some(race), Some(class)) => format!("{race} {class}"),
        // A player whose race or class never arrived falls through to the
        // creature branch's answer, which for a player is nothing at all.
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

    // **The zone, bare, under the level line** — a party member somewhere else,
    // and nothing at all for anybody else. See
    // [`crate::interface::api::UnitTip::zone`], which is where the decision is made.
    //
    // **The position in the ladder is a reconstruction**, stated as one: the
    // screenshot this was built against shows it as the third line, directly
    // under the level/class line, and the client's own order for it is not
    // known. It is a bare name rather than `ZONE_COLON` — the file ships
    // both and the plate uses neither as a label.
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

/// **The spell line law**: name | rank, cost | range, cast time | cooldown —
/// each cell omitted when it has nothing to say, each format the game's own
/// global — then the reagents and the description, both already resolved (see
/// the module comment).
fn spell_lines(lua: &mlua::Lua, this: &mlua::Table, tip: &SpellTip) -> mlua::Result<()> {
    // **A talent's rank is a line, not a cell** — see [`SpellTip::talent_rank`],
    // which carries the reference's own composition and the reason the two are
    // exclusive.
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
    // has none in 5875's file, so a focus cost displays as nothing — the
    // client's own rule for an absent key.
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

    // An instant that costs nothing is "Instant", one that costs is "Instant
    // cast" — the two keys' own names say the split (`…_INSTANT_NO_MANA`).
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

    // **Reagents, white and wrapped**, under the numbers and above the
    // sentence. `SPELL_REAGENTS` is "Reagents: " — a key with no format slot,
    // so the names are appended rather than substituted — and a count above one
    // is suffixed in brackets. A reagent whose item template has not arrived is
    // simply absent (see [`SpellTip::reagents`]); an empty list draws no line at
    // all, which is every spell that consumes nothing.
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

    // **…and the sentence, gold and wrapped.** Gold rather than the white the
    // numbers above it wear: `0xffffd200`, the client's spell plate's own and
    // the same default an uncoloured `AddLine` already takes
    // here — which is why it is [`GOLD`] and not a second constant.
    //
    // Wrapped because it is a sentence and the plate is otherwise as wide as
    // it: "Causes an explosion of arcane magic around the caster, causing 256
    // to 278 Arcane damage to all targets within 10 yards" is 800 units on one
    // line.
    if !tip.description.is_empty() {
        append(lua, this, (text(lua, &tip.description)?, GOLD), None, true)?;
    }
    Ok(())
}

/// **An item's plate**, drawn from [`crate::interface::plate::item_plate`].
///
/// The line order, the keys and the colours are that function's, which is the
/// one copy: it reads the words through a lookup rather than out of this
/// interpreter, so a caller with no interface running draws the same plate.
/// What is this module's is only the door every line goes through, [`append`],
/// with the live globals table as the lookup.
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
            Some(right) => Some((text(lua, &right)?, WHITE)),
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

/// A `&str` as a Lua value, for [`append`]'s one text door.
fn text(lua: &mlua::Lua, s: &str) -> mlua::Result<mlua::Value> {
    Ok(mlua::Value::String(lua.create_string(s)?))
}

/// A duration in the game's own words: under a minute through the `_SEC` key,
/// otherwise the `_MIN` one, the number printed the way `%.3g` prints it.
fn seconds_text(lua: &mlua::Lua, ms: u32, sec_key: &str, min_key: &str) -> Option<String> {
    let seconds = f64::from(ms) / 1000.0;
    if seconds < 60.0 {
        global_format(lua, sec_key, &g3(seconds))
    } else {
        global_format(lua, min_key, &g3(seconds / 60.0))
    }
}

/// Format through a `GlobalStrings.lua` global — `"%d Mana"` with `"35"` is
/// `"35 Mana"`. `None` when the key is not in the environment, which displays
/// as nothing: the client's own rule, not an error.
fn global_format(lua: &mlua::Lua, key: &str, value: &str) -> Option<String> {
    let format: String = lua.globals().get::<Option<String>>(key).ok().flatten()?;
    Some(substitute(&format, value))
}

/// …and the same through a key with **more than one** slot, which the four
/// `TOOLTIP_UNIT_LEVEL*` formats are the only readers of here.
fn global_format_all(lua: &mlua::Lua, key: &str, values: &[&str]) -> Option<String> {
    let format: String = lua.globals().get::<Option<String>>(key).ok().flatten()?;
    Some(vale_assets::interface::strings::substitute_all(&format, values))
}

/// Replace the first `%`-directive (`%d`, `%s`, `%.3g`…) with an already-
/// printed value. A one-slot substitution, because every key this module
/// reads has exactly one slot.
///
/// **The rule is [`vale_assets::interface::strings::substitute`]'s**, not a copy of it:
/// the same fill is made against the live globals table here and against the
/// shipped table in `interface::messages`, and while there were two of them one
/// handled `%d` and the other did not.
fn substitute(format: &str, value: &str) -> String {
    vale_assets::interface::strings::substitute(format, value)
}

/// `%.3g`: three significant digits, trailing zeros trimmed — `1.5` is "1.5",
/// `2` is "2", `12.5` is "12.5".
fn g3(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let magnitude = x.abs().log10().floor() as i32;
    let decimals = (2 - magnitude).max(0) as usize;
    let printed = format!("{x:.decimals$}");
    // Trailing zeros go only when they are *decimals* — "150" printed with no
    // point must keep all three digits.
    if printed.contains('.') {
        printed.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        printed
    }
}

/// A number printed the way the range key wants one: whole yards without a
/// point, anything else with its fraction.
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

    /// **A street sign's plate goes at once; every other kind fades.** The two
    /// arms are one `match` apart and the difference is invisible in a
    /// screenshot, so it is pinned here: a plate nailed to the cursor must not
    /// outlive the cursor leaving it, and a unit's must keep the reference's
    /// three-second ramp.
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
        // …and the gate is unchanged on both arms: a plate a *button* owns is
        // neither hidden nor faded by the world mouseover.
        for left in [Plate::Floating, Plate::Unit] {
            assert!(Plate::None.body(left).contains("GameTooltip.default"));
        }
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// The frame's own alpha, as a number. **Not through [`eval`]**: a Lua
    /// number that happens to be whole comes back as `Integer(1)` rather than
    /// `Number(1.0)`, so a string comparison against the opaque case fails on
    /// the one value it is asked about most.
    fn alpha(lua: &mlua::Lua) -> f64 {
        lua.load("return GameTooltip:GetAlpha()").eval().expect("a number")
    }

    /// **A plate the pointer has left holds, then ramps, then goes.**
    ///
    /// The shape is `FadingFrame.lua`'s and the two numbers are argued for at
    /// [`FADE_HOLD`]. What this pins is the shape rather than the numbers, so
    /// it is written against them rather than against 1.0 and 2.0 — a round
    /// that pins the durations down should not have to edit a test
    /// about whether a fade is linear.
    #[test]
    fn a_faded_plate_holds_at_full_alpha_and_then_ramps_away() {
        let lua = state();
        lua.load(r#"GameTooltip:SetText("Kobold Vermin")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");

        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        // **It is still up, and still saying what it said.** That is the whole
        // difference between this and `Hide`, which drops the lines and the
        // owner on its way out.
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(1)");
        assert_eq!(alpha(&lua), 1.0);

        // Through the hold: opaque the whole way.
        let step = FADE_HOLD / 4.0;
        for _ in 0..4 {
            fade_sweep(&lua, step);
            assert_eq!(alpha(&lua), 1.0);
            assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");
        }

        // …and then down it goes, linearly and monotonically.
        let mut last = 1.0_f64;
        let step = FADE_OUT / 8.0;
        for _ in 0..7 {
            fade_sweep(&lua, step);
            let now = alpha(&lua);
            assert!(now < last, "the ramp stalled at {now}");
            assert!(now > 0.0, "it arrived early at {now}");
            last = now;
        }
        // Halfway down the ramp is halfway through the alpha, which is what
        // "linear" means and the one thing an eased implementation would fail.
        assert!(
            (last - 1.0 / 8.0).abs() < 1e-9,
            "seven eighths down the ramp should be one eighth of alpha, not {last}"
        );

        // The last tick takes it off the screen, and *that* is where it lets go.
        fade_sweep(&lua, step * 2.0);
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:NumLines()"), "Integer(0)");
        assert_eq!(
            alpha(&lua),
            1.0,
            "a hidden plate left transparent is one that comes back invisible"
        );
    }

    /// **Anything with something new to say cancels the fade**, which is the
    /// invariant that keeps a plate from reappearing half-transparent — and the
    /// one that would break silently, since every check but the eye passes on a
    /// tooltip that is merely faint.
    ///
    /// The three doors are the three the directory actually comes through:
    /// `SetOwner` (so `GameTooltip_SetDefaultAnchor`, so every world plate and
    /// every `UnitFrame_OnEnter`), `SetText`, and a bare `Show`.
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
            // …and the ramp really is off the list, not merely reset: another
            // tick must not take it back down.
            fade_sweep(&lua, FADE_HOLD + FADE_OUT);
            assert_eq!(alpha(&lua), 1.0, "{door} left the fade running");
        }
    }

    /// **A pointer skimming an edge must not restart the hold**, and a plate
    /// already down must not be resurrected by one. `UnitFrame_OnLeave` is the
    /// directory's one caller and it sits on a handler that fires repeatedly.
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

        // …and on a plate that is already down it is nothing at all.
        lua.load("GameTooltip:Hide()").exec().expect("runs");
        lua.load("GameTooltip:FadeOut()").exec().expect("runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        fade_sweep(&lua, FADE_HOLD + FADE_OUT);
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
    }

    /// The sweep costs nothing when nothing is fading, which is the ordinary
    /// case: it rides the interface tick thirty times a second for the whole of
    /// every session.
    #[test]
    fn the_sweep_is_a_no_op_with_nothing_on_the_list() {
        let lua = state();
        fade_sweep(&lua, 1.0);
        assert_eq!(alpha(&lua), 1.0);
    }

    /// **`SetText` clears, writes line 1 and shows** — and an uncoloured text
    /// is the engine's gold, not white. The gold is the read the archaic
    /// `AddLine(text, "", r, g, b)` shape depends on.
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

        // …an explicit colour passes the gate…
        lua.load(r#"GameTooltip:SetText("Hot", 1.0, 0.1, 0.1)"#).exec().expect("runs");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert!((colour[1] - 0.1).abs() < 1e-9);

        // …and a non-number in the r-slot drops the whole tail to gold.
        lua.load(r#"GameTooltip:SetText("Zone", "", 1.0, 1.0, 1.0)"#).exec().expect("runs");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert!((colour[1] - 210.0 / 255.0).abs() < 1e-9, "the gate is lua_isnumber on the r-slot");

        // A tooltip with no text is a refusal, not a blank plate — the
        // binding's own usage error.
        assert!(lua.load("GameTooltip:SetText()").exec().is_err());
    }

    /// **`AddLine` appends and does not show**, which is the split the corpus
    /// is written against — `AddLine … Show()` against `SetText` alone.
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

    /// **A double line right-flushes its second cell** and the plate is as wide
    /// as its widest line plus the pad — the auto-size the real client's line
    /// layout does, on this side's stated glyph estimate.
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
        // …and the floor wins when it is wider.
        lua.load("GameTooltip:SetMinimumWidth(300)").exec().expect("runs");
        let floored: f64 = lua.load("return GameTooltip:GetWidth()").eval().expect("a number");
        assert_eq!(floored, 300.0 + 2.0 * PAD);
    }

    /// **A `wrap` line folds at the stated width instead of stretching the
    /// plate to the sentence.** `GameTooltip_AddNewbieTip` is the corpus'
    /// caller: its explanation line is a hundred-odd characters with `wrap=1`,
    /// and without the fold the plate was as wide as the screen. The flag is
    /// per line — a later unwrapped line still stretches — and a `clear`
    /// drops it with the text.
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

        // The same line without the flag stretches, which is `AddLine`'s
        // ordinary behaviour and the difference the flag makes.
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

    /// **A folded cell reserves the rows the painter will actually draw**, not
    /// `ceil(measured / fold)`.
    ///
    /// The two are the same number only for a sentence whose words happen to
    /// land on the fold; real prose leaves a fraction of a row empty at every
    /// break, so the ratio runs short as soon as the text is a few rows long.
    /// Shield Bash's own description is the measured case — the plate came out
    /// one row shallow and drew its last line under the bottom border.
    ///
    /// The assertion is against [`regions::text_rows`] rather than a constant,
    /// because that is the function the painter folds by: what this pins is that
    /// the two sides ask the *same* question, which is the only property that
    /// makes the plate fit.
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
        // …and the plate is tall enough to hold both lines and the pad, which is
        // the thing a player sees when it is not.
        let plate: f64 = lua.load("return GameTooltip:GetHeight()").eval().expect("a number");
        assert!(
            plate >= height + line + LINE_GAP + 2.0 * PAD - 1e-6,
            "the plate clips its own last row: {plate}"
        );

        // **And the difference the fix makes, on a fixture that shows it under
        // the bare interpreter's uniform-width fallback too.** Four words that
        // each nearly fill a row: the greedy fold puts one per row and charges
        // four, where `ceil(total / fold)` charges three, because the ratio
        // spends the empty tail of every row as though it held glyphs. In the
        // game's own faces the same gap opens on ordinary prose — which is what
        // the sentence above is — but the fallback's uniform advance happens to
        // hide it there.
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

    /// **`SetOwner` hangs the plate off the owner's corner** per the anchor
    /// law, clears the old contents, and `IsOwned` answers for the owner and
    /// nobody else.
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

    /// **Re-claiming the same owner at the same anchor costs the layout memo
    /// nothing** — and it is the hottest call in the interface.
    ///
    /// `ContainerFrameItemButton_OnUpdate` runs `OnEnter` **every frame** the
    /// pointer is over a bag square (Blizzard's own comment: "Might hurt
    /// performance, but need to always update the cursor now"), and `OnEnter`'s
    /// first act is this `SetOwner`. While it wrote a fresh points table and
    /// pushed into it, that was one whole-state invalidation per frame — every
    /// solved rectangle in the interface thrown away sixty times a second for a
    /// tooltip that had not moved. Measured with five full bags open: 8.4 ms of
    /// interpreter per frame against 6.0, with a generation burned on 309 frames
    /// of 300.
    ///
    /// A *different* owner or a different anchor still costs one, which is the
    /// half that has to keep working: the plate really does move.
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
        // …and the anchor is still there rather than having been skipped away.
        assert_eq!(
            lua.load("return GameTooltip:GetLeft()").eval::<f64>().expect("solves"),
            136.0
        );
        // A real move still invalidates — both halves of it.
        lua.load(r#"GameTooltip:SetOwner(owner, "ANCHOR_LEFT");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 1);
        lua.load(r#"GameTooltip:SetOwner(other, "ANCHOR_LEFT");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 2);
        // …and `ANCHOR_NONE` drops the anchors *and* says so, which is what the
        // fresh-table write it replaced never did: a rectangle solved from the
        // anchors that were there must not survive them.
        lua.load(r#"GameTooltip:SetOwner(other, "ANCHOR_NONE");"#)
            .exec()
            .expect("runs");
        assert_eq!(super::super::layout::generation(&lua), settled + 3);
        assert_eq!(eval(&lua, "return GameTooltip:GetNumPoints()"), "Integer(0)");
    }

    /// **`Hide` lets go of everything** — the owner drops, the lines clear —
    /// where an *empty population* hides and keeps the owner. The difference
    /// is what stops a refresh loop resurrecting a plate the pointer left,
    /// while letting it re-fill one whose contents merely came and went.
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
        // …and a plain frame's Hide does not take the tooltip path.
        lua.load("owner:Hide()").exec().expect("runs");
    }

    /// **The template's declared ladder is adopted, not shadowed.** A tooltip
    /// whose `TextLeft1` already exists — the loader's case — writes into that
    /// region, so `GameTooltipTextLeft1:SetTextColor(...)` in the directory's
    /// own `OnEvent` touches the line the client wrote.
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

    /// **`SetAction` composes the spell tooltip from the live world** — the
    /// line law with the game's own format strings — and an empty slot hides
    /// the plate, keeps the owner and answers nil, which is what
    /// `ActionButton_SetTooltip` branches on.
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
        // **The two lines a screenshot said were missing**: the reagents, with
        // the game's own "Reagents: " label and a count above one bracketed,
        // and the sentence under them.
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

    /// **A group mate somewhere else gets a bare zone line**, third, under the
    /// level line — which is what the screenshot that asked for it shows.
    ///
    /// The *decision* is not here: [`crate::interface::api::UnitTip::zone`] is empty
    /// for everybody but a party member in another zone, so this is only the
    /// composition. Its position in the ladder is a reconstruction and the
    /// comment in [`unit_lines`] says so.
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
        // …and a member in the same zone says nothing, which is the whole
        // information in the line.
        let here = Stub::default().unit("party1", "Bram", 7).played();
        assert_eq!(
            plate(&lua, &here, "party1"),
            ["Bram", "Level 60 Human Warrior (Player)"]
        );
    }

    /// **A creature's plate: name, `<subname>`, `Level N Type (Class)`.**
    ///
    /// The composition is the client's — see [`unit_lines`] — and this is the
    /// three-cell branch, which is the one the screenshot that asked for the
    /// feature shows.
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

    /// **A player's plate**, which is the other half of the same law: the class
    /// cell is `"%s %s"` of race then class and the type cell is the literal
    /// `PLAYER` key — "Level 60 Human Warrior (Player)" — with the PvP line
    /// under it when the flag is set.
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

    /// **The two cells choose the format between them**, which is the whole of
    /// the rule: an ordinary creature has no classification word, so its line
    /// takes `TOOLTIP_UNIT_LEVEL_CLASS` and draws no empty brackets. And a
    /// **rare** creature is deliberately in the same bucket — the key table's
    /// fifth entry is the empty string.
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

    /// **A level nobody has stated is `"??"`**, which is the client's own
    /// literal rather than a `GlobalStrings.lua` key — and with no creature type
    /// either the line falls all the way to `TOOLTIP_UNIT_LEVEL`.
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

    /// **An item's plate, composed from the live bags.**
    ///
    /// The words come out of the environment, so the fixture states the same
    /// `GlobalStrings.lua` keys the shipped file carries — and the assertions
    /// are on the *substitution* rather than on the English, which is the half
    /// this module owns.
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

        // The world's own answer is a bare name and quality; the *shape* is
        // asserted by writing the plate directly, because the composition is
        // what this module owns and the stub is a different file's rule.
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
        // **The repair cost is a number, never nil** —
        // `ContainerFrame_Update` compares it with `>` two lines on.
        assert_eq!(eval(&lua, "return repairCost"), "Integer(0)");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Integer(1)");

        // The name takes the quality's own colour, through the same table
        // `GetItemQualityColor` answers from.
        let line: mlua::Table = lua.globals().get("GameTooltipTextLeft1").expect("line 1");
        let colour: Vec<f64> = line.get("__colour").expect("coloured");
        assert_eq!(colour, crate::lua::api::stubs::quality_rgb(1).to_vec());

        // An empty slot hides the plate and keeps the owner, which is what
        // keeps `ContainerFrameItemButton_OnUpdate`'s `IsOwned` gate working.
        let (held, queue) = crate::lua::api::held_for_test(&lua);
        lua.scope(|scope| {
            crate::lua::api::install(&lua, scope, &world, &held, &queue)?;
            lua.load("GameTooltip:SetBagItem(0, 9);").exec()
        })
        .expect("the empty population runs");
        assert_eq!(eval(&lua, "return GameTooltip:IsShown()"), "Nil");
        assert_eq!(eval(&lua, "return GameTooltip:IsOwned(owner)"), "Integer(1)");
    }

    /// **A requirement the character does not meet is red, and that colour is
    /// the whole of the line's usefulness.**
    ///
    /// The report this came from was a potion that did nothing when
    /// right-clicked: the character was under its level, the server refused it
    /// with `EQUIP_ERR_CANT_EQUIP_LEVEL_I`, and the plate said "Requires Level
    /// 45" in the same white as every other line — so the one fact that
    /// explained the click was on screen and invisible.
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
        // The line is there, substituted with `%d` rather than `%s` — which is
        // the other half of the same report, since a formatter that knew only
        // `%s` printed the sentence with the number missing.
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

    /// **`SetInventoryItem`'s first answer is what decides the fallback**, and
    /// it is a different arity from `SetBagItem`'s — the game's own asymmetry.
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

    /// A link opens the same plate with no object behind it — and a link this
    /// client cannot read answers nothing rather than the wrong item.
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

    /// **A vendor's shelf, a corpse's rows and a quest's rewards all hover**,
    /// and they were the report this file's five new populations answer: each
    /// name was simply *absent*, so the `OnEnter` calling it died on a nil
    /// method with the plate still hidden. That is indistinguishable, from the
    /// screen, from hovering an empty square — no count could see it and the
    /// panels all reported open and correct.
    ///
    /// All four go through the same [`item_lines`] the bags do, which is what
    /// makes "the same item hovered in two places prints the same plate" a
    /// property rather than a coincidence.
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

    /// **The three substitutions with more than one slot**, which are the ones
    /// a single-slot `substitute` would silently truncate: a stat's sign and
    /// value, a resistance's sign, value and school, and a bag's size and kind.
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

    /// Every claimed method is installed and both lists are sorted — the rule
    /// every list in this directory carries. The scoped pair is probed inside
    /// a scope, which is the only place it exists.
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

    /// The `%.3g` and substitution helpers print the way the C library did —
    /// the formats are the game's and a "3.500 sec cast" would read as wrong.
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
