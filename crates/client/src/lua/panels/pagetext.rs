//! **`ItemTextFrame`** — the window a sign, a plaque, a tombstone and a book on
//! a stand open, and the one panel in the directory whose subject is a *thing*
//! rather than a person.
//!
//! Six reads and three verbs, and every one of them was missing: the panel drew
//! nothing at all and clicking a sign in the world did nothing, which is what
//! made this a dead frame rather than a partial one. The state behind it is
//! [`crate::interface::pagetext`] and the packets are
//! [`vale_protocol::play::pagetext`].
//!
//! ```text
//! ItemTextGetItem()       the title           1 call
//! ItemTextGetMaterial()   what it is written on  2
//! ItemTextGetText()       the page's words    1
//! ItemTextGetCreator()    who signed it       1
//! ItemTextGetPage()       which page          1
//! ItemTextHasNextPage()   is there another    1
//! ItemTextPrevPage()      the two arrows      1 (XML)
//! ItemTextNextPage()                          1 (XML)
//! CloseItemText()         and the close       3
//! ```
//!
//! **`ItemTextGetMaterial` answers a name, not a number.** The panel builds
//! `Interface\ItemTextFrame\ItemText-<material>-TopLeft` and three more corners
//! out of whatever comes back, and hands the same string to the directory's own
//! `GetMaterialTextColors` for the ink. So a wrong answer here is four missing
//! textures rather than a wrong colour, and the names are read out of
//! `PageTextMaterial.dbc` — see [`vale_assets::tables::pagetext`].
//!
//! **`nil` is the answer for parchment**, which is the commonest page in the
//! game: `if ( not material ) then material = "Parchment"` is the file's own
//! first line about it, and the branch it takes then *hides* the four corner
//! textures. Answering the string `"Parchment"` instead would show them, and
//! the art for them does not exist.

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 6] = [
    "ItemTextGetCreator",
    "ItemTextGetItem",
    "ItemTextGetMaterial",
    "ItemTextGetPage",
    "ItemTextGetText",
    "ItemTextHasNextPage",
];

/// **What the page window may ask.** Every default is a shut window, which is
/// what the two stand-ins answer and what a client with no session answers.
pub trait PageTextAnswers {
    /// `ItemTextGetItem()` — the title. Empty when nothing is open.
    fn page_title(&self) -> String {
        String::new()
    }
    /// `ItemTextGetMaterial()` — the `PageTextMaterial.dbc` name, or `None`
    /// for parchment. See the module comment on why `None` and not the word.
    fn page_material(&self) -> Option<String> {
        None
    }
    /// `ItemTextGetText()` — the words on the page that is showing.
    fn page_text(&self) -> String {
        String::new()
    }
    /// `ItemTextGetPage()` — one-based, which is what the panel compares
    /// against 1 to decide whether to draw the arrows at all.
    fn page_number(&self) -> u32 {
        1
    }
    /// `ItemTextHasNextPage()` — whether the chain names another.
    fn page_has_next(&self) -> bool {
        false
    }
}

impl PageTextAnswers for super::super::api::Live<'_, '_, '_> {
    fn page_title(&self) -> String {
        self.page.title().to_string()
    }

    fn page_material(&self) -> Option<String> {
        self.page.material().map(str::to_string)
    }

    fn page_text(&self) -> String {
        self.page.text().to_string()
    }

    fn page_number(&self) -> u32 {
        self.page.page()
    }

    fn page_has_next(&self) -> bool {
        self.page.has_next()
    }
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "ItemTextGetItem",
        scope.create_function(move |_, ()| Ok(answers.page_title()))?,
    )?;
    globals.set(
        "ItemTextGetMaterial",
        scope.create_function(move |_, ()| Ok(answers.page_material()))?,
    )?;
    globals.set(
        "ItemTextGetText",
        scope.create_function(move |_, ()| Ok(answers.page_text()))?,
    )?;
    globals.set(
        "ItemTextGetPage",
        scope.create_function(move |_, ()| Ok(answers.page_number()))?,
    )?;
    // **A 1 or nil, not a boolean** — the 5.0 dialect's own shape for a flag,
    // and what `if ( next ) then` is written against. The same spelling
    // `GetNumBankSlots`' second answer takes.
    globals.set(
        "ItemTextHasNextPage",
        scope.create_function(move |_, ()| Ok(answers.page_has_next().then_some(1)))?,
    )?;
    // **`ItemTextGetCreator` is answered as nothing, and that is the whole
    // answer.** Nothing on this wire states who wrote a page: the field belongs
    // to the *mail* family, on a signed letter, and this window's other caller.
    // `ItemTextFrame_OnEvent` branches on it and takes the unsigned arm for
    // every page in the game.
    globals.set(
        "ItemTextGetCreator",
        scope.create_function(|_, ()| Ok(mlua::Value::Nil))?,
    )?;
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::pagetext::PagePress>>>;

/// Register the three writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::pagetext::PagePress as P;
    let globals = lua.globals();
    for (name, press) in [
        ("ItemTextNextPage", P::Next),
        ("ItemTextPrevPage", P::Prev),
        ("CloseItemText", P::Close),
    ] {
        let queue = std::rc::Rc::clone(queue);
        let f = lua.create_function(move |_, _: mlua::MultiValue| {
            queue.borrow_mut().push(press);
            Ok(())
        })?;
        globals.set(name, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The read list is sorted and unique — the same shape every panel's list
    /// is checked in.
    #[test]
    fn the_read_list_is_sorted_and_unique() {
        let mut sorted = super::READS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.as_slice(), super::READS.as_slice());
    }
}
