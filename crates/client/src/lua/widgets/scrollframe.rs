//! **The `<ScrollFrame>`: a window onto a bigger child.**
//!
//! ```text
//! <ScrollFrame name="QuestDetailScrollFrame" inherits="UIPanelScrollFrameTemplate">
//!   <ScrollChild>
//!     <Frame name="QuestDetailScrollChildFrame"> …the actual text… </Frame>
//!   </ScrollChild>
//! </ScrollFrame>
//! ```
//!
//! ## The child's anchor is the C loader's, and forgetting it blanks six panels
//!
//! The game's `<ScrollChild>` frames carry a `<Size>` and **no `<Anchors>`**,
//! because the real client's `SetScrollChild` positions them: top-left to the
//! scroll frame's top-left, moved by the scroll offset. A host that builds the
//! child and never anchors it leaves it with no rectangle at all — and every
//! font string inside anchors to a rect-less parent and is never painted. That
//! was the whole of "the quest log is blank": the text was set correctly, into
//! regions with nowhere to be. [`adopt`] is that anchor.
//!
//! ## The scroll is an offset on that same anchor
//!
//! `SetVerticalScroll(v)` moves the child **up** by `v` — the anchor's `y`
//! becomes `+v`, since a rectangle's y grows upward — and fires
//! `OnVerticalScroll`, whose shipped body sets the scroll bar and the two
//! arrow buttons. The bar's own `OnValueChanged` calls `SetVerticalScroll`
//! back; **the no-change guard here is what terminates that loop**, not a
//! nicety.
//!
//! ## …and the range is announced, because nothing else would say it
//!
//! The reference fires `OnScrollRangeChanged` from its layout engine whenever
//! the child's rectangle moves; this host's layout is a lazy solve with no
//! notion of "changed", so [`sweep`] measures each scroll frame once per
//! interface tick and fires when the answer moves. The range is measured over
//! the child's **subtree** rather than its own declared box, because a long
//! quest description is a font string hanging below a fixed-size child — the
//! text overflows the frame, and the box alone would answer "nothing to
//! scroll" over a story three screens long.
//!
//! What is still absent is the **wheel** ([`super::super::api::mouse`]'s own note) and the
//! thumb drag; the arrow buttons work, because they go through the slider's
//! `SetValue` → `OnValueChanged` → here.

use crate::game::events::EventArg;

use super::widget;

/// The scroll frame's one child, recorded by [`adopt`].
pub(super) const CHILD_KEY: &str = "__scrollChild";
/// The current vertical offset, in the interface's own units.
pub(super) const VSCROLL_KEY: &str = "__verticalScroll";
/// The last range [`sweep`] announced, so it only speaks when the answer moves.
const RANGE_KEY: &str = "__scrollRange";
/// The registry list of every scroll frame that has a child — what [`sweep`]
/// walks, so the tick never searches the whole tree.
const REGISTRY: &str = "vale-scroll-frames";

/// **The one child a scroll frame is a window onto**, or `None`.
///
/// One raw read, so the draw walk can ask it of every frame it descends into —
/// see [`super::draw::scroll_window`], which is the only caller and which is
/// where the rule that a scroll frame clips its *child* and not its subtree is
/// written down.
pub(super) fn scroll_child(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.raw_get::<Option<mlua::Table>>(CHILD_KEY).ok().flatten()
}

/// **Adopt a `<ScrollChild>`**: record it, and give it the anchor the real
/// client's `SetScrollChild` gives — top-left to top-left, at the current
/// scroll. See the module note; this is the whole of why the quest text
/// paints.
pub(in crate::lua) fn adopt(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    child: &mlua::Table,
) -> mlua::Result<()> {
    frame.raw_set(CHILD_KEY, child.clone())?;
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

/// **The methods this module answers**, for the count that measures the gap —
/// the same bargain every sibling list here makes.
pub const METHODS: [&str; 7] = [
    "GetHorizontalScrollRange",
    "GetScrollChild",
    "GetVerticalScroll",
    "GetVerticalScrollRange",
    "SetScrollChild",
    "SetVerticalScroll",
    "UpdateScrollChildRect",
];

/// The methods, installed on the shared frame table beside every other
/// widget's — see [`super::super::panels::loot::install_methods`], which states the rule.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set_child =
        lua.create_function(|lua, (this, child): (mlua::Table, Option<mlua::Table>)| {
            match child {
                Some(child) => adopt(lua, &this, &child),
                None => this.raw_set(CHILD_KEY, mlua::Value::Nil),
            }
        })?;
    methods.set("SetScrollChild", set_child)?;

    let get_child = lua.create_function(|_, this: mlua::Table| {
        this.raw_get::<Option<mlua::Table>>(CHILD_KEY)
    })?;
    methods.set("GetScrollChild", get_child)?;

    // **The write the scroll bar's `OnValueChanged` ends in.** The no-change
    // guard terminates the bar ↔ frame loop — see the module note.
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
        // The shipped body moves the bar and the arrows; its failure must not
        // take the write down, on the same terms `SetValue`'s does not.
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

    // The reference recalculates the child's box and announces; here the
    // announcement is the whole of it, since the solve is lazy.
    let update = lua.create_function(|lua, this: mlua::Table| {
        announce(lua, &this);
        Ok(())
    })?;
    methods.set("UpdateScrollChildRect", update)?;
    Ok(())
}

/// **Announce each scroll frame whose range moved**, once per interface tick —
/// the stand-in for the reference's layout engine firing `OnScrollRangeChanged`
/// on a child resize. Called from [`super::super::api::update`]'s tick.
pub(in crate::lua) fn sweep(lua: &mlua::Lua) {
    let Ok(list) = lua.named_registry_value::<mlua::Table>(REGISTRY) else {
        return;
    };
    let frames: Vec<mlua::Table> = list.sequence_values::<mlua::Table>().flatten().collect();
    for frame in frames {
        // **A scroll frame nobody can see is not measured.** The range is a
        // recursive rect solve over the child's whole subtree, and the
        // directory registers a scroll frame in nearly every panel — so the
        // sweep was paying the quest log's and the spellbook's walks on every
        // tick of a session that had neither open (Tracy: ~1 ms of the
        // interface tick, most of it on hidden frames). The announcement is
        // only deferred, not lost: the first tick after the panel shows finds
        // the stale `RANGE_KEY` and fires — the same one-tick latency any
        // post-`OnShow` fill already has — and the panels' own
        // `UpdateScrollChildRect`/`FauxScrollFrame_Update` calls go through
        // the method, not this sweep, so they are untouched.
        if !super::layout::visible(&frame) {
            continue;
        }
        let now = range(lua, &frame);
        let last = frame.raw_get::<Option<f64>>(RANGE_KEY).ok().flatten();
        // Half a unit of hysteresis: the solve is f64 arithmetic over anchors
        // that re-assert per tick, and re-firing on noise would re-run the
        // shipped body's dozen `getglobal`s thirty times a second.
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

/// **How far there is to scroll**: the content hanging below the frame's own
/// bottom edge, measured over the child's subtree — see the module note for
/// why the child's declared box is not enough. Scroll-independent: the current
/// offset is added back, so the answer does not shrink as the view descends.
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

/// The lowest solved edge in a subtree, hidden branches skipped exactly as the
/// draw walk skips them — a hidden objective line must not deepen the scroll.
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

    /// **The adopted child has a rectangle and its font strings solve** — the
    /// whole of the blank-quest-log bug: an unanchored scroll child had no
    /// rect, so every region inside it was dropped by the layout and never
    /// painted.
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

    /// `SetVerticalScroll` moves the child up, answers through
    /// `GetVerticalScroll`, and fires `OnVerticalScroll` — once per change,
    /// which is what stops the bar ↔ frame loop recursing.
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

    /// The range is the content below the frame's bottom — measured over the
    /// **subtree**, because a long description is a font string hanging below
    /// a fixed-size child — and it does not shrink as the view scrolls down.
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
