//! `<ScrollFrame>`: a frame that shows a scrolled view of one larger child.
//!
//! ```text
//! <ScrollFrame name="QuestDetailScrollFrame" inherits="UIPanelScrollFrameTemplate">
//!   <ScrollChild>
//!     <Frame name="QuestDetailScrollChildFrame"> …the actual text… </Frame>
//!   </ScrollChild>
//! </ScrollFrame>
//! ```
//!
//! ## The scroll child's anchor
//!
//! The game's `<ScrollChild>` frames carry a `<Size>` and no `<Anchors>`,
//! because the 1.12.1 client's `SetScrollChild` positions them: top-left to the
//! scroll frame's top-left, moved by the scroll offset. A child that is never
//! anchored has no rectangle, so every font string anchored inside it has a
//! parent without a rectangle and is never painted. Without this anchor six
//! panels, the quest log among them, draw blank even though their text is set.
//! [`adopt`] sets the anchor.
//!
//! ## Scrolling is an offset on the same anchor
//!
//! `SetVerticalScroll(v)` moves the child up by `v` (the anchor's `y` becomes
//! `+v`, since a rectangle's y grows upward) and fires `OnVerticalScroll`,
//! whose shipped body sets the scroll bar and the two arrow buttons. The bar's
//! `OnValueChanged` calls `SetVerticalScroll` back. The no-change guard in
//! `SetVerticalScroll` terminates that loop.
//!
//! ## Announcing the scroll range
//!
//! The 1.12.1 client fires `OnScrollRangeChanged` whenever the child's
//! rectangle changes. This host's layout is a lazy solve that does not record
//! changes, so [`sweep`] measures each scroll frame once per interface tick and
//! fires when the range changes. The range is measured over the child's
//! subtree rather than its declared box, because a long quest description is a
//! font string hanging below a fixed-size child: the text overflows the child,
//! and the box alone would give a range of 0 for text three screens long.
//!
//! Not implemented: the mouse wheel (see the note in
//! [`super::super::api::mouse`]) and dragging the thumb. The arrow buttons work,
//! because they go through the slider's `SetValue` → `OnValueChanged` →
//! `SetVerticalScroll`.

use crate::interface::events::EventArg;

use super::widget;

/// The scroll frame's one child, recorded by [`adopt`].
pub(super) const CHILD_KEY: &str = "__scrollChild";
/// The current vertical offset, in the interface's own units.
pub(super) const VSCROLL_KEY: &str = "__verticalScroll";
/// The last range [`sweep`] announced, so it fires only when the range changes.
const RANGE_KEY: &str = "__scrollRange";
/// The registry list of every scroll frame that has a child. [`sweep`] walks
/// this list, so the tick never searches the whole tree.
const REGISTRY: &str = "vale-scroll-frames";

/// The scroll frame's child, or `None`.
///
/// One raw read, so the draw walk can call it for every frame it descends
/// into. The only caller is [`super::draw::scroll_window`], which documents
/// that a scroll frame clips its child and not its whole subtree.
pub(super) fn scroll_child(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.raw_get::<Option<mlua::Table>>(CHILD_KEY).ok().flatten()
}

/// Adopt a `<ScrollChild>`: record it, and give it the anchor the 1.12.1
/// client's `SetScrollChild` gives, top-left to top-left at the current
/// scroll. Without this anchor the quest text does not paint; see the module
/// note.
pub(in crate::lua) fn adopt(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    child: &mlua::Table,
) -> mlua::Result<()> {
    widget::set_paint(lua, frame, CHILD_KEY, child.clone())?;
    let scroll = frame
        .raw_get::<Option<f64>>(VSCROLL_KEY)?
        .unwrap_or(0.0);
    widget::set_only_point(
        lua,
        child,
        "TOPLEFT",
        Some(mlua::Value::Table(frame.clone())),
        Some("TOPLEFT"),
        (0.0, scroll),
    )?;
    let list: mlua::Table = match lua.named_registry_value(REGISTRY) {
        Ok(list) => list,
        Err(_) => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(REGISTRY, list.clone())?;
            list
        }
    };
    // Re-adoption (SetScrollChild from a script) must not list a frame twice.
    for entry in list.sequence_values::<mlua::Table>().flatten() {
        if entry == *frame {
            return Ok(());
        }
    }
    list.push(frame.clone())
}

/// The methods this module installs, listed for the count of implemented
/// methods, as every sibling module's list is.
pub const METHODS: [&str; 7] = [
    "GetHorizontalScrollRange",
    "GetScrollChild",
    "GetVerticalScroll",
    "GetVerticalScrollRange",
    "SetScrollChild",
    "SetVerticalScroll",
    "UpdateScrollChildRect",
];

/// Install the methods on the shared frame table beside every other widget's.
/// [`super::super::panels::loot::install_methods`] states the rule.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set_child =
        lua.create_function(|lua, (this, child): (mlua::Table, Option<mlua::Table>)| {
            match child {
                Some(child) => adopt(lua, &this, &child),
                None => widget::set_paint(lua, &this, CHILD_KEY, mlua::Value::Nil),
            }
        })?;
    methods.set("SetScrollChild", set_child)?;

    let get_child = lua.create_function(|_, this: mlua::Table| {
        this.raw_get::<Option<mlua::Table>>(CHILD_KEY)
    })?;
    methods.set("GetScrollChild", get_child)?;

    // The scroll bar's `OnValueChanged` ends in this write. The no-change
    // guard terminates the bar ↔ frame loop; see the module note.
    let set_scroll = lua.create_function(|lua, (this, value): (mlua::Table, Option<f64>)| {
        let value = value.unwrap_or(0.0).max(0.0);
        if this.raw_get::<Option<f64>>(VSCROLL_KEY)?.unwrap_or(0.0) == value {
            return Ok(());
        }
        this.raw_set(VSCROLL_KEY, value)?;
        if let Some(child) = this.raw_get::<Option<mlua::Table>>(CHILD_KEY)? {
            widget::set_only_point(
                lua,
                &child,
                "TOPLEFT",
                Some(mlua::Value::Table(this.clone())),
                Some("TOPLEFT"),
                (0.0, value),
            )?;
        }
        // The shipped body moves the bar and the arrows. An error in it does
        // not undo the write, as an error in `SetValue`'s handler does not.
        let _ = super::frames::run_script(
            lua,
            &this,
            "OnVerticalScroll",
            &[EventArg::Number(value)],
        );
        Ok(())
    })?;
    methods.set("SetVerticalScroll", set_scroll)?;

    let get_scroll = lua.create_function(|_, this: mlua::Table| {
        Ok(this.raw_get::<Option<f64>>(VSCROLL_KEY)?.unwrap_or(0.0))
    })?;
    methods.set("GetVerticalScroll", get_scroll)?;

    let get_range = lua.create_function(|lua, this: mlua::Table| Ok(range(lua, &this)))?;
    methods.set("GetVerticalScrollRange", get_range)?;
    // Nothing in the shipped directory scrolls sideways; the FauxScrollFrame
    // reads it only to pass it back.
    let flat = lua.create_function(|_, _: mlua::MultiValue| Ok(0.0_f64))?;
    methods.set("GetHorizontalScrollRange", flat)?;

    // In the 1.12.1 client this recalculates the child's box and fires
    // `OnScrollRangeChanged`. Here the layout solve is lazy, so it only fires.
    let update = lua.create_function(|lua, this: mlua::Table| {
        announce(lua, &this);
        Ok(())
    })?;
    methods.set("UpdateScrollChildRect", update)?;
    Ok(())
}

/// Fire `OnScrollRangeChanged` on each scroll frame whose range changed, once
/// per interface tick. The 1.12.1 client fires it when a child is resized;
/// this sweep stands in for that. Called from [`super::super::api::update`]'s
/// tick.
pub(in crate::lua) fn sweep(lua: &mlua::Lua) {
    let Ok(list) = lua.named_registry_value::<mlua::Table>(REGISTRY) else {
        return;
    };
    let frames: Vec<mlua::Table> = list.sequence_values::<mlua::Table>().flatten().collect();
    for frame in frames {
        // A hidden scroll frame is not measured. The range is a recursive
        // rect solve over the child's whole subtree, and the directory
        // registers a scroll frame in nearly every panel, so measuring hidden
        // frames cost the quest log's and the spellbook's walks on every tick
        // with neither open (Tracy: ~1 ms of the interface tick, most of it on
        // hidden frames). The announcement is deferred, not lost: the first
        // tick after the panel shows finds the stale `RANGE_KEY` and fires,
        // the same one-tick latency any post-`OnShow` fill has. The panels'
        // own `UpdateScrollChildRect`/`FauxScrollFrame_Update` calls go
        // through the method, not this sweep, so they are unaffected.
        if !super::layout::visible(&frame) {
            continue;
        }
        let now = range(lua, &frame);
        let last = frame.raw_get::<Option<f64>>(RANGE_KEY).ok().flatten();
        // Half a unit of hysteresis: the solve is f64 arithmetic over anchors
        // that are re-set every tick, and firing on rounding noise would re-run
        // the shipped body's dozen `getglobal`s thirty times a second.
        if last.is_none_or(|last| (last - now).abs() > 0.5) {
            let _ = frame.raw_set(RANGE_KEY, now);
            announce(lua, &frame);
        }
    }
}

/// Fire `OnScrollRangeChanged` the way the template reads it: `arg1` the
/// horizontal range (always 0 here), `arg2` the vertical.
fn announce(lua: &mlua::Lua, frame: &mlua::Table) {
    let vertical = range(lua, frame);
    let _ = frame.raw_set(RANGE_KEY, vertical);
    let _ = super::frames::run_script(
        lua,
        frame,
        "OnScrollRangeChanged",
        &[EventArg::Number(0.0), EventArg::Number(vertical)],
    );
}

/// The scroll range: the height of content below the frame's bottom edge,
/// measured over the child's subtree (the module note gives the reason the
/// child's declared box is not enough). The current offset is added back, so
/// the range does not shrink as the view scrolls down.
fn range(lua: &mlua::Lua, frame: &mlua::Table) -> f64 {
    let Some(own) = super::layout::rect(lua, frame) else {
        return 0.0;
    };
    let Ok(Some(child)) = frame.raw_get::<Option<mlua::Table>>(CHILD_KEY) else {
        return 0.0;
    };
    let Some(bottom) = lowest(lua, &child, 0) else {
        return 0.0;
    };
    let scroll = frame
        .raw_get::<Option<f64>>(VSCROLL_KEY)
        .ok()
        .flatten()
        .unwrap_or(0.0);
    (own.bottom - bottom + scroll).max(0.0)
}

/// The lowest solved edge in a subtree. Hidden branches are skipped as the draw
/// walk skips them, so a hidden objective line does not add to the range.
fn lowest(lua: &mlua::Lua, object: &mlua::Table, depth: u32) -> Option<f64> {
    if depth > 8 {
        return None;
    }
    if !object
        .raw_get::<Option<bool>>(widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
    {
        return None;
    }
    let mut found = super::layout::rect(lua, object).map(|r| r.bottom);
    if let Ok(children) = widget::children(object) {
        for child in children.sequence_values::<mlua::Table>().flatten() {
            if let Some(bottom) = lowest(lua, &child, depth + 1) {
                found = Some(found.map_or(bottom, |f| f.min(bottom)));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        super::super::frames::install(&lua).expect("the object model installs");
        super::super::layout::set_screen(&lua, 800.0, 600.0).expect("a screen");
        lua
    }

    /// The adopted child has a rectangle, so its font strings solve. An
    /// unanchored scroll child has no rectangle, so the layout drops every
    /// region inside it and the quest log draws blank.
    #[test]
    fn an_adopted_scroll_child_solves_where_its_frame_is() {
        let lua = lua();
        lua.load(
            r#"
            scroll = CreateFrame("ScrollFrame", "S", UIParent);
            scroll:SetWidth(300); scroll:SetHeight(200);
            scroll:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 100, -50);
            child = CreateFrame("Frame", "SChild", scroll);
            child:SetWidth(300); child:SetHeight(400);
            "#,
        )
        .exec()
        .expect("the chunk runs");
        let globals = lua.globals();
        let scroll: mlua::Table = globals.get("S").unwrap();
        let child: mlua::Table = globals.get("SChild").unwrap();
        assert!(
            super::super::layout::rect(&lua, &child).is_none(),
            "an unadopted child has no rectangle, which was the bug"
        );
        adopt(&lua, &scroll, &child).expect("adopts");
        let own = super::super::layout::rect(&lua, &scroll).expect("the frame solves");
        let solved = super::super::layout::rect(&lua, &child).expect("the child solves");
        assert_eq!(solved.left, own.left);
        assert_eq!(solved.top(), own.top(), "top-left to top-left");
        assert_eq!(solved.height, 400.0, "at its own declared size");
    }

    /// `SetVerticalScroll` moves the child up, is read back by
    /// `GetVerticalScroll`, and fires `OnVerticalScroll` once per change. Firing
    /// only on a change stops the bar ↔ frame loop recursing.
    #[test]
    fn scrolling_moves_the_child_and_fires_once_per_change() {
        let lua = lua();
        lua.load(
            r#"
            scroll = CreateFrame("ScrollFrame", "S", UIParent);
            scroll:SetWidth(300); scroll:SetHeight(200);
            scroll:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 0, 0);
            child = CreateFrame("Frame", "SChild", scroll);
            child:SetWidth(300); child:SetHeight(400);
            fired = 0;
            scroll:SetScript("OnVerticalScroll", function() fired = fired + 1; last = arg1; end);
            "#,
        )
        .exec()
        .expect("the chunk runs");
        let globals = lua.globals();
        let scroll: mlua::Table = globals.get("S").unwrap();
        let child: mlua::Table = globals.get("SChild").unwrap();
        adopt(&lua, &scroll, &child).expect("adopts");
        let before = super::super::layout::rect(&lua, &child).expect("solves");
        lua.load("S:SetVerticalScroll(60); S:SetVerticalScroll(60)")
            .exec()
            .expect("scrolls");
        let after = super::super::layout::rect(&lua, &child).expect("still solves");
        assert_eq!(after.top() - before.top(), 60.0, "the child moved up");
        assert_eq!(
            lua.load("return fired").eval::<f64>().unwrap(),
            1.0,
            "the repeat was a no-op — the guard that stops the bar loop"
        );
        assert_eq!(lua.load("return S:GetVerticalScroll()").eval::<f64>().unwrap(), 60.0);
    }

    /// The range is the content below the frame's bottom, measured over the
    /// subtree because a long description is a font string hanging below a
    /// fixed-size child. It does not shrink as the view scrolls down.
    #[test]
    fn the_range_is_the_overflow_and_survives_scrolling() {
        let lua = lua();
        lua.load(
            r#"
            scroll = CreateFrame("ScrollFrame", "S", UIParent);
            scroll:SetWidth(300); scroll:SetHeight(200);
            scroll:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 0, 0);
            child = CreateFrame("Frame", "SChild", scroll);
            child:SetWidth(300); child:SetHeight(150);
            "#,
        )
        .exec()
        .expect("the chunk runs");
        let globals = lua.globals();
        let scroll: mlua::Table = globals.get("S").unwrap();
        let child: mlua::Table = globals.get("SChild").unwrap();
        adopt(&lua, &scroll, &child).expect("adopts");
        // The child fits: nothing to scroll.
        assert_eq!(range(&lua, &scroll), 0.0);
        // A region hanging 300 below the child's top overflows a 200 frame.
        lua.load(
            r#"
            text = SChild:CreateFontString("SText");
            text:SetWidth(300); text:SetHeight(300);
            text:SetPoint("TOPLEFT", SChild, "TOPLEFT", 0, 0);
            "#,
        )
        .exec()
        .expect("a tall region");
        assert_eq!(range(&lua, &scroll), 100.0, "300 of content in a 200 window");
        lua.load("S:SetVerticalScroll(80)").exec().expect("scrolls");
        assert_eq!(range(&lua, &scroll), 100.0, "the range is scroll-independent");
    }
}
