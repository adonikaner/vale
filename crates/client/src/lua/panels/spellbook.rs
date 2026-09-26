//! **The C functions `SpellBookFrame.lua` calls** — the panel's whole surface.
//!
//! The spellbook is one of the purest examples of what this client's split with
//! `Interface\FrameXML\` is: **the panel itself is entirely the game's file**.
//! 494 lines of Lua draw the pages, turn them, flash the tabs, place the
//! cooldown swirls and decide whether a button is enabled — and the client owes
//! it eleven functions and three events. Nothing here draws a spellbook; it
//! answers questions about one.
//!
//! ```text
//! GetNumSpellTabs()         how many pages of tabs there are
//! GetSpellTabInfo(i)        …and what each is: name, texture, offset, count
//! GetSpellName(id, book)    (name, rank) — the two lines on a button
//! GetSpellTexture(id, book) its icon
//! GetSpellCooldown(id,book) (start, duration, enable) — the swirl
//! IsSpellPassive(id, book)  a black border rather than a gold one
//! IsCurrentCast(id, book)   …and a pressed one
//! CastSpell(id, book)       clicking it
//! UpdateSpells()            "re-sort the book" — see below
//! GameTooltip:SetSpell      the plate, in `lua::tooltip`
//! ```
//!
//! …plus `HasPetSpells`, `GetSpellAutocast`, `ToggleSpellAutocast` and
//! `PickupSpell`, which stay in [`super::super::api::stubs`] because they are about a pet
//! and a cursor this client has neither of.
//!
//! ## `id` is a row, not a spell
//!
//! Every one of those takes what `SpellBook_GetSpellID` computed:
//!
//! ```lua
//! return id + SpellBookFrame.selectedSkillLineOffset
//!          + ( SPELLS_PER_PAGE * (SPELLBOOK_PAGENUMBERS[...] - 1) );
//! ```
//!
//! — a button's index, plus the tab's offset, plus twelve per page turned. So
//! it indexes the **flat book** [`vale_assets::tables::book`] builds, one-based, and
//! it is not a spell id and must never be treated as one. The whole reason that
//! module follows the client's two sort orders is that this
//! arithmetic only lands on the right spell if this client's order is the
//! game's.
//!
//! ## `bookType` is read and refused rather than ignored
//!
//! Every call carries `"spell"` or `"pet"`. This client has no pet, so `"pet"`
//! answers the absent value throughout — and *that is what `HasPetSpells()`
//! returning nil already promised*, so the panel never asks. Answering the
//! player's own book for a pet index would be a plausible wrong answer, which is
//! the class this project keeps paying for.
//!
//! ## `UpdateSpells` looks like a no-op and is the whole reason the panel fills
//!
//! In the real client it is the build: it sorts the tab array, sorts the
//! flat spell array, and **raises `SPELLS_CHANGED`** in the same function. This
//! client does the first two in [`crate::game::combat::spellbook::rebuild`], on a latch,
//! whenever the server's set or the character's identity moves — so the obvious
//! reading is that the C function has nothing left to do.
//!
//! **That reading is wrong, and the audit is what said so.** `SpellButton1`
//! through `12` are filled by `SpellButton_UpdateButton`, which the panel calls
//! from exactly two places: its own `OnLoad` — where every button is still
//! invisible and returns on line one — and its `OnEvent`, on `SPELLS_CHANGED`.
//! `SpellBookFrame_OnShow` does not call it. So the *only* thing that puts a
//! spell on a button when the book is opened is the event `UpdateSpells` raises,
//! and a no-op leaves a correctly drawn, correctly paged, entirely blank
//! spellbook — with nothing in any log.
//!
//! It is therefore [`update_spells`], which fires the event where it stands.
//! Synchronously, because `SpellBookFrame_Update` runs on after it and the real
//! client's `FrameScript_SignalEvent` is a direct dispatch; deferring it to the
//! next frame's queue would work and would flash an empty book on every open.

use super::super::api::{one_or_nil, Answers, SpellTab};
// The unit-token surface these answers read the world through — imported
// here now that the subject's own answers live beside its registration.
use crate::game::api;

/// **The reads this module registers into the scope**, for the count that
/// measures the gap — the same kind of list [`super::super::api::READS`] is, and
/// checked the same way.
///
/// The other two names the panel needs are **writes** and are in
/// [`super::super::api::verbs::REGISTERED`] with the rest of them: `CastSpell`, which
/// records, and `UpdateSpells`, which fires an event. The split is the one this
/// whole directory is built on — see [`super::super::api::verbs`] — rather than a filing
/// choice, so keeping them in one list here would blur exactly the distinction
/// that matters.
pub const READS: [&str; 8] = [
    "GetNumSpellTabs",
    "GetSpellCooldown",
    "GetSpellName",
    "GetSpellTabInfo",
    "GetSpellTexture",
    "IsCurrentCast",
    "IsSpellPassive",
    "PlayerHasSpells",
];

/// `BOOKTYPE_PET` — `SpellBookFrame.lua`'s own constant.
const BOOKTYPE_PET: &str = "pet";

/// Whether a `(id, bookType)` pair addresses a row this client has.
///
/// `None` for the pet book, which is every call this client answers nothing to;
/// otherwise the one-based row. An absent `bookType` is the player's own book,
/// which is what `GameTooltip:SetSpell(id)` from an addon means.
pub(in crate::lua) fn row(index: Option<usize>, book_type: Option<String>) -> Option<usize> {
    if book_type.as_deref() == Some(BOOKTYPE_PET) {
        return None;
    }
    index.filter(|i| *i > 0)
}

/// `UpdateSpells()` — **raise `SPELLS_CHANGED` where the call stands.**
///
/// See the module comment: this is the panel's own refresh, and the twelve
/// buttons are blank without it.
///
/// The one function in this directory that dispatches an event from *inside* a
/// Lua call, which [`super::super::widgets::frames::call_handler`] already supports — it puts
/// `this`, `event` and `arg1..9` back whether or not the handler raised, and
/// says in its own comment that this is what makes a nested fire safe. What is
/// deliberately not done here is reporting a handler's failure: there is no
/// `LuaHost` to note it on from a registered closure, so a body that breaks
/// under this event breaks silently. `--audit --events` fires the same name at
/// the same frames and *does* report, which is where that gap is covered.
///
/// Registered once, in [`super::super::api::verbs::register`], because it needs no world —
/// it is the only spellbook function that does not.
pub(in crate::lua) fn update_spells(lua: &mlua::Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, ()| {
        let _ = super::super::widgets::frames::fire(lua, "SPELLS_CHANGED", &[])?;
        Ok(())
    })
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // **`PlayerHasSpells()` is a constant `1`**, and that is the whole function:
    // the client pushes the double `1.0` and returns,
    // with no read of anything. It is not a stub here for the same reason — a
    // constant answered as a constant is the reference exactly.
    //
    // Its one caller is `SpellbookMicroButton`'s `OnEnter`, choosing between
    // *Spellbook & Abilities* and *Abilities* for the tooltip; 1.12 shipped the
    // second string and never the branch that reaches it.
    globals.set(
        "PlayerHasSpells",
        scope.create_function(move |_, ()| Ok(1_i64))?,
    )?;

    // Written out rather than through a macro like `api::install`'s two. Each of
    // these returns a different arity — one value, two, three, four — and the
    // macro would have to be told which, at which point it is longer than the
    // seven bodies it replaces.
    globals.set(
        "GetNumSpellTabs",
        scope.create_function(move |_, ()| Ok(answers.num_spell_tabs()))?,
    )?;
    globals.set(
        "GetSpellTabInfo",
        scope.create_function(move |_, index: Option<usize>| {
            // **Four nils rather than an error** for a tab that is not there.
            // `SpellBookFrame_Update` asks about all eight `MAX_SKILLLINE_TABS`
            // on every update whatever the character has, and takes `name` being
            // absent as "hide this tab".
            //
            // As `Option`s rather than `mlua::Value`s, which would want the
            // `'scope` lifetime on `lua` to build a string with. Same
            // multi-return either way.
            Ok(match index.and_then(|i| answers.spell_tab_info(i)) {
                Some(tab) => (
                    Some(tab.name),
                    Some(tab.texture),
                    Some(tab.offset),
                    Some(tab.count),
                ),
                None => (None, None, None, None),
            })
        })?,
    )?;

    globals.set(
        "GetSpellName",
        scope.create_function(move |_, (index, book): (Option<usize>, Option<String>)| {
            // **The rank is `""` and not nil** for a spell with none, because
            // `SpellButton_UpdateButton` compares it: `if ( subSpellName ~= "" )`
            // decides where the name is anchored, and nil there moves every
            // rankless spell's label by two units.
            Ok(row(index, book)
                .and_then(|row| answers.spell_name(row))
                .map_or((None, None), |(name, rank)| (Some(name), Some(rank))))
        })?,
    )?;
    globals.set(
        "GetSpellTexture",
        scope.create_function(move |_, (index, book): (Option<usize>, Option<String>)| {
            Ok(row(index, book).and_then(|row| answers.spell_texture(row)))
        })?,
    )?;
    globals.set(
        "GetSpellCooldown",
        scope.create_function(move |_, (index, book): (Option<usize>, Option<String>)| {
            let (start, duration, enable) = row(index, book)
                .map_or((0.0, 0.0, true), |row| answers.spell_cooldown(row));
            Ok((start, duration, one_or_nil(enable)))
        })?,
    )?;
    globals.set(
        "IsSpellPassive",
        scope.create_function(move |_, (index, book): (Option<usize>, Option<String>)| {
            Ok(one_or_nil(
                row(index, book).is_some_and(|row| answers.spell_passive(row)),
            ))
        })?,
    )?;
    globals.set(
        "IsCurrentCast",
        scope.create_function(move |_, (index, book): (Option<usize>, Option<String>)| {
            Ok(one_or_nil(
                row(index, book).is_some_and(|row| answers.spell_is_current_cast(row)),
            ))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::lua::api::tests::{eval, Stub};

    /// A book with a General tab and a Fire tab — the smallest thing that has an
    /// offset worth adding.
    fn book() -> Stub {
        Stub::default()
            .tab("General", &[("Attack", "")])
            .tab(
                "Fire",
                &[("Fireball", "Rank 1"), ("Fireball", "Rank 2"), ("Fire Blast", "Rank 1")],
            )
    }

    /// **`SpellBook_GetSpellID`'s own arithmetic, run for real** — a button
    /// index plus the tab's offset — and the check that it lands on the spell
    /// the tab claims. Every other test in this file is downstream of this one:
    /// if the offsets and the flat list disagree, the panel draws real spells in
    /// the wrong places and nothing raises.
    #[test]
    fn a_tabs_offset_indexes_the_flat_book() {
        let world = book();
        assert_eq!(eval(&world, "return GetNumSpellTabs()"), "Integer(2)");
        // The four answers, in the order the panel unpacks them.
        assert_eq!(
            eval(&world, "local n,t,o,c = GetSpellTabInfo(2); return n..' '..o..' '..c"),
            r#"String("Fire 1 3")"#
        );
        // …and button 1 of that tab is the first Fireball, not the first spell.
        assert_eq!(
            eval(
                &world,
                "local _,_,o = GetSpellTabInfo(2); local n,r = GetSpellName(1 + o, 'spell'); return n..'|'..r"
            ),
            r#"String("Fireball|Rank 1")"#
        );
        assert_eq!(
            eval(
                &world,
                "local _,_,o = GetSpellTabInfo(2); return GetSpellName(3 + o, 'spell')"
            ),
            r#"String("Fire Blast")"#
        );
    }

    /// **A tab the character does not have answers four nils**, which is what
    /// `SpellBookFrame_Update` reads as "hide it" — it asks about all eight
    /// `MAX_SKILLLINE_TABS` on every update whatever the book holds, so an error
    /// here is the whole panel.
    #[test]
    fn a_tab_past_the_end_is_nil_rather_than_an_error() {
        let world = book();
        assert_eq!(eval(&world, "return tostring(GetSpellTabInfo(8))"), r#"String("nil")"#);
        assert_eq!(eval(&world, "return tostring(GetSpellTabInfo(0))"), r#"String("nil")"#);
        assert_eq!(
            eval(&world, "return tostring(GetSpellName(99, 'spell'))"),
            r#"String("nil")"#
        );
    }

    /// **The rank is `""` and not nil** for a spell with none — see the
    /// registration, where `SpellButton_UpdateButton`'s own comparison is.
    #[test]
    fn a_rankless_spell_has_an_empty_rank_not_a_missing_one() {
        let world = book();
        assert_eq!(
            eval(&world, "local n,r = GetSpellName(1, 'spell'); return n..'['..r..']'"),
            r#"String("Attack[]")"#
        );
    }

    /// **The pet book answers nothing at all**, on every one of them — see the
    /// module comment. The panel never asks, because `HasPetSpells()` is nil;
    /// an addon might, and the player's row of that number is the wrong answer.
    #[test]
    fn the_pet_book_is_refused_rather_than_answered_from_the_players() {
        let world = book();
        for call in [
            "GetSpellName(1, 'pet')",
            "GetSpellTexture(1, 'pet')",
            "IsSpellPassive(1, 'pet')",
            "IsCurrentCast(1, 'pet')",
        ] {
            assert_eq!(
                eval(&world, &format!("return tostring({call})")),
                r#"String("nil")"#,
                "{call}"
            );
        }
        // …and the cooldown, whose absent shape is three values rather than nil.
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetSpellCooldown(1, 'pet'); return s..' '..d..' '..tostring(e)"
            ),
            r#"String("0 0 1")"#
        );
    }

    /// The game's own boolean on both predicates — `1` or `nil`, never
    /// `true`/`false`. See [`super::super::api`], where the whole argument is.
    #[test]
    fn the_predicates_answer_one_or_nil() {
        let world = book();
        assert_eq!(
            eval(&world, "return tostring(IsSpellPassive(1, 'spell'))"),
            r#"String("nil")"#
        );
        assert_eq!(
            eval(&world, "return tostring(IsCurrentCast(1, 'spell'))"),
            r#"String("nil")"#
        );
    }
}


/// **What the interface may ask about the spellbook.**
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
pub trait SpellbookAnswers {

    // --- the spellbook ---
    //
    // Indices are **one-based and into the flat book**, which is what
    // `SpellBook_GetSpellID` composes and hands back — see
    // [`vale_assets::tables::book`].

    /// `GetNumSpellTabs`.
    fn num_spell_tabs(&self) -> usize;
    /// `GetSpellTabInfo(i)` — `(name, texture, offset, count)`, with the name
    /// already resolved through `GlobalStrings.lua` for the General tab.
    fn spell_tab_info(&self, index: usize) -> Option<SpellTab>;
    /// `GetSpellName(id)` — `(name, rank)`. The rank is the *sub*-name the panel
    /// draws under it, and is `""` rather than `nil` for a spell with none,
    /// because `SpellButton_UpdateButton` compares it against `""`.
    fn spell_name(&self, index: usize) -> Option<(String, String)>;
    fn spell_texture(&self, index: usize) -> Option<String>;
    /// `(start, duration, enable)`, in [`Answers::now`]'s base — the same triple
    /// [`Answers::action_cooldown`] answers, because it is the same clocks.
    fn spell_cooldown(&self, index: usize) -> (f64, f64, bool);
    fn spell_passive(&self, index: usize) -> bool;
    /// `IsCurrentCast` — whether *this* row is the spell going out right now,
    /// which is what puts the pressed border on a spell button.
    fn spell_is_current_cast(&self, index: usize) -> bool;
    /// `GameTooltip:SetSpell` — the same shape a bar slot's tooltip is composed
    /// from, so the two plates cannot disagree.
    fn spell_tooltip(&self, index: usize) -> Option<api::SpellTip>;
}

impl SpellbookAnswers for super::super::api::Live<'_, '_, '_> {

    fn num_spell_tabs(&self) -> usize {
        self.book.book.tabs.len()
    }

    fn spell_tab_info(&self, index: usize) -> Option<SpellTab> {
        let tab = self.book.book.tab(index)?;
        Some(SpellTab {
            name: self
                .book
                .book
                .tab_name(index, |key| self.strings?.get(key).map(str::to_string)),
            texture: tab.icon.clone().unwrap_or_default(),
            offset: tab.offset,
            count: tab.count,
        })
    }

    fn spell_name(&self, index: usize) -> Option<(String, String)> {
        let info = self.book.book.spell(index)?;
        Some((info.name.clone(), info.rank.clone()))
    }

    fn spell_texture(&self, index: usize) -> Option<String> {
        Some(self.book.book.spell(index)?.icon.clone())
    }

    fn spell_cooldown(&self, index: usize) -> (f64, f64, bool) {
        let Some(info) = self.book.book.spell(index) else {
            return (0.0, 0.0, true);
        };
        api::cooldown_of(self.cooldowns, info, self.now)
    }

    fn spell_passive(&self, index: usize) -> bool {
        self.book.book.spell(index).is_some_and(|s| s.is_passive())
    }

    fn spell_is_current_cast(&self, index: usize) -> bool {
        // **The cast this client believes is in flight**, which is what the bar
        // reads too — `Casting` is started at send rather than on
        // `SMSG_SPELL_START`, so the border appears in the frame of the click.
        self.casting.started.is_some()
            && self
                .book
                .book
                .spell(index)
                .is_some_and(|info| info.id == self.casting.spell_id)
    }

    fn spell_tooltip(&self, index: usize) -> Option<api::SpellTip> {
        let names = |entry| self.reagent_name(entry);
        Some(api::spell_tip(
            self.book.book.spell(index)?,
            &api::TipContext {
                level: self.tip_level(),
                // A *spell* plate has no requirement lines; see
                // [`Self::item_context`], which is the one that pays for them.
                race: 0,
                class: 0,
                catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        ))
    }
}