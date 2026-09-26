//! **The knob on a scroll bar** — where a `<ThumbTexture>` sits, and what
//! dragging it does.
//!
//! A `<Slider>` is the one widget in the directory whose art is positioned by
//! the *widget* rather than by its own anchors, and that makes it the same trap
//! `<ScrollChild>` was: the game's thumbs carry a `<Size>` and **no
//! `<Anchors>`**, because `CSimpleSlider` places them from the value. A loader
//! that gives an unanchored region the "fill your parent" default — which is
//! right for every other slot texture in the game — draws
//! `UI-ScrollBar-Knob` stretched over the entire track, which is what a
//! screenshot reported as a scroll bar with "nothing in the scroll area".
//!
//! ```xml
//! <Slider name="UIPanelScrollBarTemplate" virtual="true">
//!   <Size><AbsDimension x="16" y="0"/></Size>
//!   …
//!   <ThumbTexture name="$parentThumbTexture" inherits="UIPanelScrollBarButton"
//!                 file="Interface\Buttons\UI-ScrollBar-Knob">
//!     <Size><AbsDimension x="16" y="16"/></Size>
//!   </ThumbTexture>
//! </Slider>
//! ```
//!
//! ## The travel
//!
//! The thumb's *centre* runs from one end of the track to the other, inset by
//! half its own length, so a slider at its minimum shows the knob flush with
//! the top and at its maximum flush with the bottom. That is the only geometry
//! that puts `UI-ScrollBar-Knob` where the shipped art expects it, and it is
//! what makes the two arrow buttons' `SetValue(GetValue() ± height/2)` land
//! where a player expects.
//!
//! **A vertical scroll bar runs value-up-is-*down*.** `ScrollFrame`'s own
//! `OnVerticalScroll` sets the bar from `SetVerticalScroll`'s offset, where 0 is
//! the top of the content — so value 0 is the *top* of the track, which in this
//! client's y-up space is the high end. Getting that backwards is a scroll bar
//! that works and reads upside down.
//!
//! ## The drag
//!
//! 1.12's slider handles its own mouse: the widget is clickable whether or not
//! the markup says `enableMouse`, and there is no `OnClick` anywhere in the
//! directory for one. Pressing anywhere on the track jumps the value to the
//! pointer and keeps following it until the button comes up — which is both the
//! click-to-page and the thumb drag, because the reference does not distinguish
//! them either.
//!
//! It is deliberately **not** routed through [`super::super::api::mouse`]'s drag gesture:
//! that machine is `RegisterForDrag`'s, with a 4-pixel slop and an
//! `OnDragStart`/`OnDragStop` pair, and a slider registers for neither. A
//! pointer-down on a slider is a slider grab from the first pixel.

use super::layout::Rect;

/// The slot a `<ThumbTexture>` lands in — [`super::regions::slot_key`]'s
/// `"Thumb"`, spelled once here so the loader and this module agree.
pub(super) const THUMB_SLOT: &str = "__thumb";

/// Which slider the pointer has hold of, while it has hold of one.
const REG_GRABBED: &str = "vale.slider.grabbed";

/// Whether an object is a slider that should place its own thumb.
///
/// The test is the *slot*, not the widget kind: a `<StatusBar>` never declares
/// a thumb and a `<Slider>` without one has nothing to place, so this is one
/// raw read either way.
pub(super) fn thumb(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.raw_get::<Option<mlua::Table>>(THUMB_SLOT).ok().flatten()
}

/// **Where the thumb goes**, in the slider's own rectangle.
///
/// A free function over four numbers so the travel can be tested with no Lua,
/// no anchors and no window — see the module note, which is where the two ways
/// to get it wrong are.
pub(super) fn travel(track: &Rect, thumb_length: f64, fraction: f64, vertical: bool) -> (f64, f64) {
    let (length, thumb_length) = match vertical {
        true => (track.height, thumb_length.min(track.height)),
        false => (track.width, thumb_length.min(track.width)),
    };
    let room = (length - thumb_length).max(0.0);
    let along = room * fraction.clamp(0.0, 1.0);
    match vertical {
        // **Value up is down the track**: see the module note.
        true => (track.left, track.top() - thumb_length - along),
        false => (track.left + along, track.bottom),
    }
}

/// …and the reverse, for a pointer landing on the track.
pub(super) fn fraction_at(track: &Rect, thumb_length: f64, at: (f64, f64), vertical: bool) -> f64 {
    let (length, thumb_length, along) = match vertical {
        true => (
            track.height,
            thumb_length.min(track.height),
            track.top() - at.1,
        ),
        false => (track.width, thumb_length.min(track.width), at.0 - track.left),
    };
    let room = length - thumb_length;
    if room <= 0.0 {
        return 0.0;
    }
    // The pointer names the thumb's *centre*, which is half a thumb into its
    // own travel — so a press at the very top is 0 rather than a knob whose top
    // edge is under the finger.
    ((along - thumb_length / 2.0) / room).clamp(0.0, 1.0)
}

/// Put every slider's thumb where its value says, once a tick.
///
/// Rides [`super::super::api::update::fire`] beside [`super::scrollframe::sweep`], and for
/// the same reason: the placement depends on the slider's own solved rectangle,
/// which moves when a panel moves, and no script announces that.
pub(in crate::lua) fn sweep(lua: &mlua::Lua) {
    let Ok(list) = lua.named_registry_value::<mlua::Table>(REGISTRY) else {
        return;
    };
    let frames: Vec<mlua::Table> = list.sequence_values::<mlua::Table>().flatten().collect();
    for frame in frames {
        // A hidden slider's thumb is not placed — the same skip, for the same
        // measured reason, as [`super::scrollframe::sweep`]'s. `Show()` runs
        // from the click and event paths, both ordered before this tick, so a
        // freshly-opened panel's thumbs are placed before the same frame's
        // paint; a value written while hidden lands on the first visible tick.
        if !super::layout::visible(&frame) {
            continue;
        }
        place(lua, &frame);
    }
}

/// The registry list of sliders that have a thumb — the same shape
/// [`super::scrollframe`]'s is, and for the same reason: a per-tick walk of
/// 3,746 frames looking for a raw key is a walk this client cannot afford, and
/// there are 40-odd sliders.
const REGISTRY: &str = "vale.sliders";

/// Record a slider so [`sweep`] finds it — called by the loader once the thumb
/// slot has been filled.
pub(in crate::lua) fn adopt(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    let list = match lua.named_registry_value::<Option<mlua::Table>>(REGISTRY)? {
        Some(list) => list,
        None => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(REGISTRY, list.clone())?;
            list
        }
    };
    list.raw_push(frame.clone())
}

/// Anchor one slider's thumb from its value.
pub(in crate::lua) fn place(lua: &mlua::Lua, frame: &mlua::Table) {
    let Some(thumb) = thumb(frame) else { return };
    let Some(track) = super::layout::rect(lua, frame) else {
        return;
    };
    let Some(bar) = super::statusbar::read(frame) else {
        return;
    };
    // The thumb's declared size, which is the one thing about it the markup
    // does say. A thumb with no size of its own is square on the track's
    // narrow axis, which is what every scroll bar in the directory declares
    // anyway.
    let vertical = track.height >= track.width;
    let thumb_length = length(&thumb, &track, vertical);
    let (left, bottom) = travel(&track, thumb_length, f64::from(bar.fraction), vertical);
    // **Anchored to the slider rather than positioned absolutely**, so the knob
    // moves with the panel for free and the layout memo stays valid — and
    // through `set_only_point`, which is a no-op when the offset has not
    // changed. A scroll bar nobody is touching costs nothing per tick.
    let _ = super::widget::set_only_point(
        lua,
        &thumb,
        "BOTTOMLEFT",
        Some(mlua::Value::Table(frame.clone())),
        Some("BOTTOMLEFT"),
        (left - track.left, bottom - track.bottom),
    );
}

/// How long the thumb is along the track, falling back to the track's narrow
/// axis for one that declares no size — which is a square knob, and is what
/// every scroll bar in the directory declares anyway.
fn length(thumb: &mlua::Table, track: &Rect, vertical: bool) -> f64 {
    let declared = match vertical {
        true => thumb.raw_get::<Option<f64>>(super::widget::HEIGHT_KEY),
        false => thumb.raw_get::<Option<f64>>(super::widget::WIDTH_KEY),
    };
    match declared.ok().flatten() {
        Some(length) if length > 0.0 => length,
        _ => match vertical {
            true => track.width,
            false => track.height,
        },
    }
}

/// **Take hold of a slider under the pointer**, answering whether one was
/// there. Called from the mouse dispatch before anything else looks at the
/// press — see the module note on why this is not the drag gesture.
pub(in crate::lua) fn grab(lua: &mlua::Lua, frame: &mlua::Table, at: Option<(f64, f64)>) -> bool {
    if thumb(frame).is_none() {
        return false;
    }
    let _ = lua.set_named_registry_value(REG_GRABBED, frame.clone());
    if let Some(at) = at {
        drag(lua, at);
    }
    true
}

/// …and let go, whatever was held.
pub(in crate::lua) fn release(lua: &mlua::Lua) {
    let _ = lua.set_named_registry_value(REG_GRABBED, mlua::Value::Nil);
}

/// Follow the pointer while a slider is held, firing `OnValueChanged` through
/// the widget's own `SetValue` so the scroll frame moves with it.
pub(in crate::lua) fn drag(lua: &mlua::Lua, at: (f64, f64)) {
    let Ok(Some(frame)) = lua.named_registry_value::<Option<mlua::Table>>(REG_GRABBED) else {
        return;
    };
    let Some(track) = super::layout::rect(lua, &frame) else {
        return;
    };
    let Some(thumb) = thumb(&frame) else { return };
    let vertical = track.height >= track.width;
    let thumb_length = length(&thumb, &track, vertical);
    let (min, max) = super::statusbar::range(&frame);
    if max <= min {
        return;
    }
    let fraction = fraction_at(&track, thumb_length, at, vertical);
    let value = min + (max - min) * fraction;
    // Through the widget's own `SetValue`, so `OnValueChanged` runs and the
    // scroll frame it drives moves — the whole point of the gesture.
    super::statusbar::set_value(lua, &frame, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(in crate::lua) fn track() -> Rect {
        Rect {
            left: 100.0,
            bottom: 200.0,
            width: 16.0,
            height: 100.0,
        }
    }

    /// **Value 0 is the top of a vertical bar**, which is the half that reads
    /// upside down when it is wrong: `SetVerticalScroll(0)` is the top of the
    /// content, and the thumb has to agree with it.
    #[test]
    fn a_vertical_thumb_starts_at_the_top_and_ends_at_the_bottom() {
        let track = track();
        let (_, bottom) = travel(&track, 16.0, 0.0, true);
        assert_eq!(bottom, track.top() - 16.0, "flush with the top");
        let (_, bottom) = travel(&track, 16.0, 1.0, true);
        assert_eq!(bottom, track.bottom, "flush with the bottom");
        let (_, bottom) = travel(&track, 16.0, 0.5, true);
        assert_eq!(bottom, track.bottom + (100.0 - 16.0) / 2.0);
    }

    /// A thumb longer than its track does not travel, and does not produce a
    /// negative room that would send it off the end.
    #[test]
    fn a_thumb_that_fills_its_track_does_not_move() {
        let track = track();
        for fraction in [0.0, 0.5, 1.0] {
            let (_, bottom) = travel(&track, 500.0, fraction, true);
            assert_eq!(bottom, track.bottom);
        }
    }

    /// The pointer names the thumb's centre, so a press at either end is
    /// exactly 0 or 1 rather than half a knob short of it.
    #[test]
    fn a_press_on_the_track_reads_back_as_the_fraction_it_would_place() {
        let track = track();
        let at = |y| fraction_at(&track, 16.0, (100.0, y), true);
        assert_eq!(at(track.top()), 0.0);
        assert_eq!(at(track.bottom), 1.0);
        assert!((at(track.bottom + 50.0) - 0.5).abs() < 1e-9);
        // …and it is the inverse of the placement, which is what makes the knob
        // sit under the finger rather than beside it.
        let fraction = 0.25;
        let (_, bottom) = travel(&track, 16.0, fraction, true);
        let centre = bottom + 8.0;
        assert!((fraction_at(&track, 16.0, (100.0, centre), true) - fraction).abs() < 1e-9);
    }
}
