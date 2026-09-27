//! **The C functions `GossipFrame.lua` calls** — four reads, four writes.
//!
//! ```text
//! GetGossipText()             the greeting, resolved through the text cache
//! GetGossipOptions()          (text, word) per line — the word is an icon path
//! GetGossipAvailableQuests()  (title, level) per quest on offer
//! GetGossipActiveQuests()     …and per quest the giver wants back
//! SelectGossipOption(n)  SelectGossipAvailableQuest(n)  SelectGossipActiveQuest(n)
//! CloseGossip()
//! ```
//!
//! ## The three list reads return *flattened pairs*, and the length is the list
//!
//! `GossipFrameOptionsUpdate(GetGossipOptions())` walks `arg` two at a time —
//! `for i=1, arg.n, 2` — so each read is one multi-return of `2N` values, not a
//! table and not a count. The second of an option's pair is the **word** the
//! panel pastes into `Interface\GossipFrame\<word>GossipIcon`, out of the
//! client's own eleven-entry table (`vale_protocol::play::gossip::ICON_WORDS`).

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 4] = [
    "GetGossipActiveQuests",
    "GetGossipAvailableQuests",
    "GetGossipOptions",
    "GetGossipText",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "GetGossipText",
        scope.create_function(move |_, ()| Ok(answers.gossip_text()))?,
    )?;

    // The flattened-pairs shape all three lists share — see the module note.
    globals.set(
        "GetGossipOptions",
        scope.create_function(move |lua, ()| {
            let mut out = mlua::MultiValue::new();
            for (text, word) in answers.gossip_options() {
                out.push_back(mlua::Value::String(lua.create_string(&text)?));
                out.push_back(mlua::Value::String(lua.create_string(word)?));
            }
            Ok(out)
        })?,
    )?;
    for (name, active) in [
        ("GetGossipAvailableQuests", false),
        ("GetGossipActiveQuests", true),
    ] {
        globals.set(
            name,
            scope.create_function(move |lua, ()| {
                let mut out = mlua::MultiValue::new();
                for (title, level) in answers.gossip_quests(active) {
                    out.push_back(mlua::Value::String(lua.create_string(&title)?));
                    out.push_back(mlua::Value::Integer(i64::from(level)));
                }
                Ok(out)
            })?,
        )?;
    }
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::gossip::GossipPress>>>;

/// Register the four writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::gossip::GossipPress as P;
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
    let index = |n: Option<i64>| usize::try_from(n?).ok().filter(|i| *i > 0);
    push!("SelectGossipOption", Option<i64>, |n| index(n).map(P::Option));
    push!("SelectGossipAvailableQuest", Option<i64>, |n| index(n)
        .map(P::Available));
    push!("SelectGossipActiveQuest", Option<i64>, |n| index(n).map(P::Active));
    push!("CloseGossip", Option<i64>, |_a| Some(P::Close));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// **Flattened pairs, walked two at a time** — the shape
    /// `GossipFrameOptionsUpdate`'s own `for i=1, arg.n, 2` requires. A table
    /// or a count here draws no options with no error.
    #[test]
    fn the_lists_are_flattened_pairs_the_panel_walks_two_at_a_time() {
        let world = Stub::default();
        // The Stub answers two options; count them the way the panel does.
        assert_eq!(
            eval(
                &world,
                r#"local a = { GetGossipOptions() };
                   local lines = 0;
                   for i = 1, table.getn(a), 2 do lines = lines + 1 end
                   return lines .. "|" .. a[2]"#
            ),
            r#"String("2|vendor")"#
        );
        assert_eq!(
            eval(
                &world,
                "local t, l = GetGossipAvailableQuests(); return t .. '|' .. l"
            ),
            r#"String("Probe Quest 1|5")"#
        );
    }

    /// The writes record one-based and drop nonsense.
    #[test]
    fn the_writes_record_in_call_order() {
        use crate::interface::gossip::GossipPress as P;
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load(
            "SelectGossipOption(2); SelectGossipAvailableQuest(1); \
             SelectGossipActiveQuest(1); SelectGossipOption(0); CloseGossip()",
        )
        .exec()
        .expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![P::Option(2), P::Available(1), P::Active(1), P::Close]
        );
    }
}


/// **What the interface may ask about the gossip menu.**
///
/// Split out of `Answers`, which was one trait with **132 methods** covering
/// fifteen unrelated subjects in a 4,454-line file. Here rather than in
/// [`super::super::api`] so that a read's four pieces — this declaration, the answer
/// below it, the registration further up this file and the name in [`READS`] —
/// are all in the file the subject is named after.
///
/// [`super::super::api::Answers`] is now the sum of the twelve of these rather than
/// the place any of them live, so nothing that *consumes* the API changed:
/// `&dyn Answers` still resolves every one of them.
pub trait GossipAnswers {


    // --- talking to an NPC ---
    //
    // See [`super::gossip`] and [`super::merchant`]. The three gossip lists are
    // flattened pairs the panel walks two at a time; the merchant sentinels are
    // `-1` stock and an empty name for a template still in flight.

    /// `GetGossipText()` — resolved through the text cache; see
    /// [`crate::interface::gossip::GossipWindow::text`].
    fn gossip_text(&self) -> String;
    /// `GetGossipOptions()` — `(text, icon word)` per line.
    fn gossip_options(&self) -> Vec<(String, &'static str)>;
    /// `GetGossipAvailableQuests()` / `GetGossipActiveQuests()` —
    /// `(title, level)` per quest in the asked-for half.
    fn gossip_quests(&self, active: bool) -> Vec<(String, u32)>;
}

impl GossipAnswers for super::super::api::Live<'_, '_, '_> {


    // --- talking to an NPC ---

    fn gossip_text(&self) -> String {
        // The same five variables an `npc_text` row carries — see
        // [`crate::interface::messages::substitute`]. Done here rather than in
        // `interface::gossip` because that module has no world borrow and `$N` is
        // the character's own name.
        crate::interface::messages::substitute(&self.gossip.text(), self.speaker())
    }

    fn gossip_options(&self) -> Vec<(String, &'static str)> {
        self.gossip.menu().map_or_else(Vec::new, |menu| {
            menu.options
                .iter()
                .map(|o| (o.text.clone(), vale_protocol::play::gossip::icon_word(o.icon)))
                .collect()
        })
    }

    fn gossip_quests(&self, active: bool) -> Vec<(String, u32)> {
        self.gossip.menu().map_or_else(Vec::new, |menu| {
            menu.quests
                .iter()
                .filter(|q| crate::interface::quest::is_active_offer(q.icon) == active)
                .map(|q| (q.title.clone(), q.level))
                .collect()
        })
    }
}