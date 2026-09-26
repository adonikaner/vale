//! **`SetPortraitTexture(texture, unit)`** — the one C function that turns a
//! `<Texture>` into a picture of somebody.
//!
//! ```text
//! UnitFrame.lua:26   SetPortraitTexture(this.portrait, this.unit);
//! ```
//!
//! That one line, in `UnitFrame_OnEvent` and `UnitFrame_Update`, is the whole of
//! how the player frame, the target frame, target-of-target, the four party
//! frames and the pet frame get their faces. It was a stub for twenty rounds and
//! a stub is exactly the wrong shape for it: `stubs.rs`' own first line says a
//! stub is a function that can honestly answer a constant, and "no picture" is
//! not an answer a unit frame has any use for.
//!
//! ## It records twice, and the second one is what makes it affordable
//!
//! The call does two things and neither of them draws:
//!
//! * it writes the **token** onto the region ([`super::super::widgets::regions::set_portrait_unit`]),
//!   so the draw walk emits a portrait where that texture would have been; and
//! * it pushes the token onto a queue, so `crate::render::portraits` knows which
//!   pictures are worth setting a camera up for.
//!
//! The second is not redundant. The draw walk's output lives behind the
//! interface's own tuning switch and its own clock, and a render pass that read
//! it would be a render pass that stops working when somebody presses F4 —
//! whereas the set of tokens the interface has *ever* asked about is nine
//! strings and never grows. The same shape as [`super::sound`] and
//! [`super::super::panels::worldmap`], for the same reason a verb records at all.
//!
//! ## The first argument is an object *or* a name
//!
//! `SetPortraitToTexture`'s is too and for the same reason — `ContainerFrame.lua`
//! passes a name where `UnitFrame.lua` passes the region — so the resolution is
//! the one [`super::super::panels::container::set_portrait_to_texture`] already makes. A first
//! argument that is neither records nothing rather than raising: this is called
//! from `OnEvent`, and an error there takes the frame's whole handler with it.

use std::cell::RefCell;
use std::rc::Rc;

/// Unit tokens the interface has asked for a picture of, in call order.
///
/// A `Vec` and not a set: the drain is per frame and the reader accumulates,
/// so ordering costs nothing and duplicates are what a `HashSet` two lines
/// later already handles. See [`crate::render::portraits::Portraits::wanted`].
pub type PortraitQueue = Rc<RefCell<Vec<String>>>;

/// Register the one verb. Unscoped — it records, and needs no world.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &PortraitQueue) -> mlua::Result<()> {
    let queue = Rc::clone(queue);
    let f = lua.create_function(
        move |lua, (target, unit): (mlua::Value, Option<String>)| {
            let region = match target {
                mlua::Value::Table(region) => Some(region),
                mlua::Value::String(name) => lua.globals().get(name.to_string_lossy())?,
                _ => None,
            };
            let Some(region) = region else { return Ok(()) };
            // **A nil unit clears the portrait rather than leaving the last
            // one.** `PartyMemberFrame_UpdateMember` calls this with whatever
            // `this.unit` holds, and for an empty party slot that is a token
            // naming nobody — a face left standing there is the face of
            // whoever was in the group before.
            let token = unit.filter(|t| !t.is_empty());
            super::super::widgets::regions::set_portrait_unit(&region, token.as_deref())?;
            if let Some(token) = token {
                queue.borrow_mut().push(token);
            }
            Ok(())
        },
    )?;
    lua.globals().set("SetPortraitTexture", f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> (mlua::Lua, PortraitQueue) {
        let lua = mlua::Lua::new();
        let queue: PortraitQueue = Rc::new(RefCell::new(Vec::new()));
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        register(&lua, &queue).expect("registers");
        lua.load(r#"f = CreateFrame("Frame"); portrait = f:CreateTexture("TargetPortrait")"#)
            .exec()
            .expect("a texture to paint");
        (lua, queue)
    }

    fn painted(lua: &mlua::Lua) -> crate::lua::widgets::regions::Paint {
        let region: mlua::Table = lua.load("return portrait").eval().expect("the region");
        crate::lua::widgets::regions::paint(&region).expect("a texture region")
    }

    /// The token lands on the region *and* on the queue, and the call answers
    /// nothing — a verb has no return value.
    #[test]
    fn a_portrait_call_records_on_the_region_and_on_the_queue() {
        let (lua, queue) = host();
        let value: mlua::Value = lua
            .load(
                r#"SetPortraitTexture(portrait, "target");
                   return SetPortraitTexture(portrait, "player")"#,
            )
            .eval()
            .expect("the chunk runs");
        assert!(value.is_nil(), "a verb answers nothing");
        assert_eq!(*queue.borrow(), vec!["target".to_string(), "player".into()]);
        assert_eq!(
            painted(&lua).portrait,
            Some("player".to_string()),
            "the region holds the last token, not the first"
        );
    }

    /// **The first argument may be a name**, which is how half the directory
    /// calls the portrait functions.
    #[test]
    fn the_texture_may_be_named_rather_than_passed() {
        let (lua, _queue) = host();
        lua.load(r#"SetPortraitTexture("TargetPortrait", "target")"#)
            .exec()
            .expect("the chunk runs");
        assert_eq!(painted(&lua).portrait, Some("target".to_string()));
    }

    /// **`SetTexture` takes it back off**, which is what keeps a face from
    /// hiding under a pet frame's icon for the rest of the session.
    #[test]
    fn setting_a_path_clears_the_portrait() {
        let (lua, _queue) = host();
        lua.load(
            r#"SetPortraitTexture(portrait, "target");
               portrait:SetTexture("Interface\\Icons\\INV_Misc_QuestionMark")"#,
        )
        .exec()
        .expect("the chunk runs");
        let paint = painted(&lua);
        assert_eq!(paint.portrait, None);
        assert!(paint.texture.is_some());
    }

    /// A nil or empty token clears it and records nothing — an empty party slot
    /// must not keep the last member's face.
    #[test]
    fn an_absent_unit_clears_the_portrait_and_records_nothing() {
        let (lua, queue) = host();
        lua.load(
            r#"SetPortraitTexture(portrait, "party1");
               SetPortraitTexture(portrait, nil)"#,
        )
        .exec()
        .expect("the chunk runs");
        assert_eq!(painted(&lua).portrait, None);
        assert_eq!(*queue.borrow(), vec!["party1".to_string()]);
    }

    /// A first argument that is neither a region nor a name is a no-op rather
    /// than an error: this is called from `OnEvent`, where a raise takes the
    /// whole handler down.
    #[test]
    fn a_bad_first_argument_does_not_raise() {
        let (lua, queue) = host();
        lua.load(r#"SetPortraitTexture(nil, "player"); SetPortraitTexture(7, "player")"#)
            .exec()
            .expect("no raise");
        assert!(queue.borrow().is_empty(), "and records nothing");
    }
}
