//! **The duel's four script functions**, all of them writes.
//!
//! ```text
//! StartDuel(name)       /duel <name>                 ChatFrame.lua
//! StartDuelUnit(unit)   the unit popup's Duel line   UnitPopup.lua
//! AcceptDuel()          DUEL_REQUESTED's Accept      StaticPopup.lua
//! CancelDuel()          its Decline, and /forfeit    StaticPopup.lua, ChatFrame.lua
//! ```
//!
//! A queue rather than a [`crate::game::bindings::Binding`], because the first
//! two carry a string and a binding is `Copy`. Drained by
//! [`crate::game::session::duel`], which is where each is explained.

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::game::session::duel::DuelPress>>>;

/// Register the four writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::game::session::duel::DuelPress as P;
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = std::rc::Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(press) = $body {
                    queue.borrow_mut().push(press);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }
    push!("StartDuel", Option<String>, |name| name
        .filter(|n| !n.is_empty())
        .map(P::Start));
    push!("StartDuelUnit", Option<String>, |unit| unit
        .filter(|u| !u.is_empty())
        .map(P::StartUnit));
    push!("AcceptDuel", Option<i64>, |_a| Some(P::Accept));
    push!("CancelDuel", Option<i64>, |_a| Some(P::Cancel));
    Ok(())
}
