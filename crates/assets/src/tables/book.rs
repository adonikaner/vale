//! **The spellbook as the panel indexes it**: the tabs, and the one flat list
//! they slice.
//!
//! `SpellBookFrame.lua` never asks about a spell id. It asks
//! `GetSpellTabInfo(i)` for a tab's `(name, texture, offset, numSpells)`, adds
//! the offset and the page to a button's own index, and hands the result to
//! `GetSpellName(id)` — so **the whole panel is arithmetic over one ordered
//! array**, and if this client's order differs from the game's by one row, every
//! button on every page is wrong by one.
//!
//! Nothing in the protocol carries that order. `SMSG_INITIAL_SPELLS` is an
//! unordered bag of ids; the sort is entirely the client's, which is why it
//! follows the client rather than a reconstruction.
//!
//! ## The rules
//!
//! The build is one step per spell ("add this spell to the book") and two
//! sorts run after it:
//!
//! ```text
//! add        the spell's skill line (crate::tables::skills) picks or creates its tab,
//!            the tab's count goes up by one, and the id is appended to one
//!            flat array (0x400 entries — SpellBookFrame.lua's own
//!            MAX_SPELLS = 1024)
//! filter     …but only if the record passes two Attributes tests first: bit 7
//!            (never displayed) and bit 5 (a trade recipe). Neither reaches the
//!            book. See crate::tables::spellbook::SpellInfo::in_book
//! sort       sort the tabs, then sort the spells, then SPELLS_CHANGED
//! tabs       tab order:   skill line 0 first, always; the rest by the skill
//!            line's own name, case-insensitively
//! spells     spell order: by *tab index* (the spell's skill line's position
//!            in the array the line above has already sorted), then by spell
//!            name, then by rank number
//! offsets    an offset is the running sum of every earlier tab's count
//! ```
//!
//! **The two sorts are only consistent because of their order**: the spell
//! comparator asks for a tab *index*, and the tabs were sorted first. Sorting
//! the spells first — or keying them on the raw skill line id — produces a flat list whose
//! runs do not line up with the offsets, and a panel that draws the right spells
//! on the wrong pages.
//!
//! ## The General tab is not a table row
//!
//! Skill line 0 is what a spell with no `SkillLineAbility` of its own falls to
//! (see [`crate::tables::skills`]), and `GetSpellTabInfo` special-cases
//! it twice: its name is the **`GlobalStrings.lua` key `GENERAL`** — the same
//! string the chat tab uses, looked up through the client's own string table
//! rather than hard-coded — and its texture is the literal
//! `Interface\Icons\Ability_Kick`. It sorts first unconditionally,
//! before the alphabetical comparison is even reached.
//!
//! ## What is here and what is one layer up
//!
//! This module holds the **rule**: given a set of known spell ids and a
//! character's race and class, what the pages are and what is on them. It needs
//! no session, no renderer and no window, so it is unit-testable and
//! `vale spellbook` checks the same copy the client runs. The *player's* book
//! — which ids the server sent, and rebuilding when they change — is
//! `crates/client/src/game/spellbook.rs`.

use crate::tables::skills::{Skills, GENERAL};
use crate::tables::spellbook::{SpellInfo, Spells};

/// The texture the General tab wears — a literal in the client
/// rather than anything in a table.
pub const GENERAL_ICON: &str = r"Interface\Icons\Ability_Kick";

/// The `GlobalStrings.lua` key the General tab is named by. Looked
/// up rather than spelled, because the game looks it up: a localised client
/// draws its own word here.
pub const GENERAL_NAME_KEY: &str = "GENERAL";

/// How many spells a page holds — `SPELLS_PER_PAGE` in `SpellBookFrame.lua`.
/// Here rather than only there because the page count is what decides whether
/// the panel's arrows are enabled, and this module is what the check reads.
pub const SPELLS_PER_PAGE: usize = 12;

/// The cap the client's own flat array has — `0x400` entries, and
/// `MAX_SPELLS = 1024` in `SpellBookFrame.lua`. A book longer than this is
/// truncated rather than grown, which is what the client does.
pub const MAX_SPELLS: usize = 1024;

/// One tab of the book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// The skill line, or [`GENERAL`] (0).
    pub skill_line: u32,
    /// What the tab is called. **For [`GENERAL`] this is
    /// [`GENERAL_NAME_KEY`]**, not a word — the caller resolves it through
    /// `GlobalStrings.lua`, which is what the client does. See
    /// [`Spellbook::tab_name`], which is the one door for that.
    name: String,
    /// The tab's art. Always present for General; `None` for a skill line whose
    /// icon id is not in `SpellIcon.dbc`.
    pub icon: Option<String>,
    /// Where this tab's run starts in [`Spellbook::spells`], and how long it is
    /// — `GetSpellTabInfo`'s third and fourth answers.
    pub offset: usize,
    pub count: usize,
}

impl Tab {
    /// How many pages the tab holds — `ceil(count / SPELLS_PER_PAGE)`, and at
    /// least one, because `SpellBook_GetCurrentPage` starts every tab at page 1
    /// and the panel compares against it.
    pub fn pages(&self) -> usize {
        self.count.div_ceil(SPELLS_PER_PAGE).max(1)
    }
}

/// A character's book: the tabs, and the flat list they index into.
#[derive(Debug, Default, Clone)]
pub struct Spellbook {
    pub tabs: Vec<Tab>,
    /// Every spell the character knows, in the client's own order — tab, then
    /// name, then rank. **Passives are in it**: the panel draws one with a
    /// blackened border rather than hiding it (`SpellButton_UpdateButton`'s
    /// `isPassive` branch), which is why this is not the same list the action
    /// bar's own `known` is.
    pub spells: Vec<SpellInfo>,
}

impl Spellbook {
    /// Build the book for a character, from the ids the server sent.
    ///
    /// `known` is unordered and may contain ids `Spell.dbc` has no row for —
    /// both are true of a real `SMSG_INITIAL_SPELLS` — and each is dropped
    /// rather than faked, because a spell with no row has no name to draw and
    /// no icon to draw it with.
    ///
    /// With no [`Skills`] every spell falls to [`GENERAL`] and the book is one
    /// long tab. That is a degradation with a visible shape rather than a wrong
    /// answer: the spells are all there, in name order, on one page set.
    pub fn build(
        known: &[u32],
        race: u8,
        class: u8,
        catalog: &Spells,
        skills: Option<&Skills>,
    ) -> Spellbook {
        // One pass: resolve each id and ask which line it is on. Ids the
        // catalog does not know are gone by the end of it.
        let mut entries: Vec<(u32, SpellInfo)> = known
            .iter()
            .filter_map(|id| {
                let info = catalog.info(*id)?;
                // **The client's own filter**: a
                // spell whose `Attributes` carry bit 7 or bit 5 is never added
                // to the book. See [`crate::tables::spellbook::SpellInfo::in_book`],
                // which describes both tests.
                // It is here rather than at the call site because it decides what
                // the *tabs* are as well as what is on them — a page of hidden
                // spells is also a skill line the real client never shows, and a
                // page of recipes is the whole of what a profession's tab would
                // otherwise be.
                if !info.in_book() {
                    return None;
                }
                let line = skills.map_or(GENERAL, |s| s.line_of(*id, race, class));
                Some((line, info))
            })
            .collect();

        // **The tabs first**, because the spell order is keyed on a tab's
        // *position* — see the module comment, where the whole of why this
        // ordering matters is written out.
        let mut lines: Vec<u32> = entries.iter().map(|(line, _)| *line).collect();
        lines.sort_unstable();
        lines.dedup();
        let name_of = |line: u32| -> String {
            if line == GENERAL {
                GENERAL_NAME_KEY.to_string()
            } else {
                skills
                    .and_then(|s| s.line(line))
                    .map_or_else(String::new, |l| l.name.clone())
            }
        };
        lines.sort_by(|a, b| {
            // General first, unconditionally — the client's comparator returns
            // before it looks either line up.
            match (*a == GENERAL, *b == GENERAL) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                // …and a stable tie-break on the id, which the client does not
                // need because `qsort` leaves equal elements wherever they
                // were. Two lines with the same name is not a case the shipped
                // table has; deciding it here keeps the build reproducible.
                _ => name_of(*a)
                    .to_lowercase()
                    .cmp(&name_of(*b).to_lowercase())
                    .then(a.cmp(b)),
            }
        });
        let position = |line: u32| lines.iter().position(|l| *l == line).unwrap_or(lines.len());

        // …then the spells, by that position.
        entries.sort_by(|(a_line, a), (b_line, b)| {
            position(*a_line)
                .cmp(&position(*b_line))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.rank_order().cmp(&b.rank_order()))
                // The client stops here and leaves the rest to `qsort`'s own
                // order. Two rows this far in are the same spell twice under
                // different ids; the id keeps the build reproducible.
                .then_with(|| a.id.cmp(&b.id))
        });
        entries.truncate(MAX_SPELLS);

        // The offsets are then a scan, which is what the client computes on
        // demand — kept rather than recomputed, since the panel asks for every
        // tab's every frame it is open.
        let mut tabs: Vec<Tab> = Vec::with_capacity(lines.len());
        let mut offset = 0usize;
        for line in lines {
            let count = entries.iter().filter(|(l, _)| *l == line).count();
            if count == 0 {
                // Only reachable when the truncation above cut a tab's whole
                // run off the end. A tab with nothing on it is not a tab.
                continue;
            }
            tabs.push(Tab {
                skill_line: line,
                name: name_of(line),
                icon: if line == GENERAL {
                    Some(GENERAL_ICON.to_string())
                } else {
                    skills.and_then(|s| s.line(line)).and_then(|l| l.icon.clone())
                },
                offset,
                count,
            });
            offset += count;
        }

        Spellbook {
            tabs,
            spells: entries.into_iter().map(|(_, info)| info).collect(),
        }
    }

    /// **What tab `index` is called**, one-based as every id in the interface
    /// is, with `strings` answering the `GlobalStrings.lua` key General is named
    /// by.
    ///
    /// One door for the lookup, because the General case is the only place in
    /// the client where a *tab* name is a key rather than a word, and two
    /// callers doing it themselves is two chances to draw the literal
    /// "GENERAL".
    pub fn tab_name(&self, index: usize, strings: impl Fn(&str) -> Option<String>) -> String {
        let Some(tab) = self.tab(index) else {
            return String::new();
        };
        if tab.skill_line == GENERAL {
            // The key itself if the string table has no such row, which is what
            // `GlobalStrings.lua`'s own absent keys draw as elsewhere in this
            // client — visible, and not a blank tab.
            return strings(GENERAL_NAME_KEY).unwrap_or_else(|| GENERAL_NAME_KEY.to_string());
        }
        tab.name.clone()
    }

    /// One-based, the way `GetSpellTabInfo(i)` is called.
    pub fn tab(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index.checked_sub(1)?)
    }

    /// One-based, the way `GetSpellName(id)` is called — `SpellBook_GetSpellID`
    /// composes it out of a button index, a tab offset and a page.
    pub fn spell(&self, id: usize) -> Option<&SpellInfo> {
        self.spells.get(id.checked_sub(1)?)
    }

    pub fn is_empty(&self) -> bool {
        self.spells.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `SpellInfo` with only the fields this module sorts on.
    fn spell(id: u32, name: &str, rank: &str, level: u32) -> SpellInfo {
        SpellInfo {
            id,
            name: name.to_string(),
            rank: rank.to_string(),
            spell_level: level,
            ..SpellInfo::default()
        }
    }

    /// The book, built from resolved entries directly — the same sort
    /// [`Spellbook::build`] runs, without a `Spells` to stand up. Keeps these
    /// tests about the ordering rather than about DBC parsing, which
    /// [`crate::tables::skills`] covers on its own.
    fn book(entries: Vec<(u32, SpellInfo)>, names: &[(u32, &str)]) -> Spellbook {
        let name_of = |line: u32| {
            names
                .iter()
                .find(|(id, _)| *id == line)
                .map_or(String::new(), |(_, n)| (*n).to_string())
        };
        let mut lines: Vec<u32> = entries.iter().map(|(l, _)| *l).collect();
        lines.sort_unstable();
        lines.dedup();
        lines.sort_by(|a, b| match (*a == GENERAL, *b == GENERAL) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => name_of(*a)
                .to_lowercase()
                .cmp(&name_of(*b).to_lowercase())
                .then(a.cmp(b)),
        });
        let position = |line: u32| lines.iter().position(|l| *l == line).unwrap_or(usize::MAX);
        let mut entries = entries;
        entries.sort_by(|(al, a), (bl, b)| {
            position(*al)
                .cmp(&position(*bl))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.rank_order().cmp(&b.rank_order()))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut tabs = Vec::new();
        let mut offset = 0;
        for line in lines {
            let count = entries.iter().filter(|(l, _)| *l == line).count();
            tabs.push(Tab {
                skill_line: line,
                name: name_of(line),
                icon: (line == GENERAL).then(|| GENERAL_ICON.to_string()),
                offset,
                count,
            });
            offset += count;
        }
        Spellbook {
            tabs,
            spells: entries.into_iter().map(|(_, i)| i).collect(),
        }
    }

    /// **A `DO_NOT_DISPLAY` spell is not in the book, and neither is its tab.**
    ///
    /// The client's own filter drops the spell before it reaches the
    /// "pick or create a tab" step, so a skill line whose every spell carries the
    /// bit produces no tab at all — which is the half that matters, because the
    /// screenshot that reported this showed a `GENERIC (DND)` tab beside the
    /// mage's four.
    ///
    /// Goes through [`Spellbook::build`] against a real `Spell.dbc` rather than
    /// the local `book` helper, because the filter is in `build` — and because
    /// the *attribute column* is the thing under test, which the helper does not
    /// read at all.
    #[test]
    fn a_hidden_spell_takes_neither_a_row_nor_a_tab() {
        use crate::tables::dbc::testing::dbc;
        use crate::tables::spellbook::{
            spell_attributes::{DO_NOT_DISPLAY, TRADESPELL},
            spell_fields, Spells,
        };

        let row = |id: u32, name: u32, attributes: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::NAME] = name;
            row[spell_fields::ATTRIBUTES] = attributes;
            row
        };
        let catalog = Spells::parse(
            &dbc(
                &[
                    row(1, 1, 0),
                    row(2, 10, DO_NOT_DISPLAY),
                    // …and the *bit* is tested rather than the word: a spell
                    // carrying every other attribute and not this one stays.
                    // `TRADESPELL` is excepted because it is the book's *other*
                    // refusal — see `a_recipe_is_not_in_the_book_and_the_opener_still_is`
                    // — and a row carrying both would not say which one bounced it.
                    row(3, 25, !(DO_NOT_DISPLAY | TRADESPELL)),
                ],
                173,
                b"\0Fireball\0Hostile Intent\0Frostbolt\0",
            ),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("a catalog");

        let built = Spellbook::build(&[1, 2, 3], 1, 8, &catalog, None);
        let ids: Vec<u32> = built.spells.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![1, 3], "the hidden spell is not in the flat list");
        assert_eq!(built.tabs.len(), 1, "and its line is not a tab of its own");
        assert_eq!(built.tabs[0].count, 2);
    }

    /// **A spell with a non-zero `castUI` is not in the book**, and it carries
    /// neither attribute bit — the client's third test. The rows are the
    /// measured ones: *Great Stamina* Rank 1 (4195, `Attributes 0x00040100`,
    /// `castUI 1`), which is the hunter's pet-training spell and Beast
    /// Training's row, and *Enchant Bracer - Minor Health* (7418, `castUI 3`),
    /// an Enchanting recipe. *Beast Training* itself (5149, `castUI 0`) stays.
    #[test]
    fn a_craft_window_spell_is_not_in_the_book() {
        use crate::tables::dbc::testing::dbc;
        use crate::tables::spellbook::{spell_fields, Spells};

        let row = |id: u32, name: u32, cast_ui: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::NAME] = name;
            row[spell_fields::ATTRIBUTES] = 0x0004_0100;
            row[spell_fields::CAST_UI] = cast_ui;
            row
        };
        let catalog = Spells::parse(
            &dbc(
                &[row(5149, 1, 0), row(4195, 16, 1), row(7418, 30, 3)],
                173,
                b"\0Beast Training\0Great Stamina\0Enchant Bracer - Minor Health\0",
            ),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("a catalog");
        let built = Spellbook::build(&[5149, 4195, 7418], 1, 3, &catalog, None);
        let ids: Vec<u32> = built.spells.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![5149]);
        assert_eq!(catalog.info(4195).unwrap().craft_kind(), Some(1));
        assert_eq!(catalog.info(7418).unwrap().craft_kind(), Some(3));
        assert_eq!(catalog.info(5149).unwrap().craft_kind(), None);
    }

    /// **A recipe is not in the book either, and neither is the row a bit-7
    /// spell shares its shape with.**
    ///
    /// The second half of the client's filter, and it is the half
    /// that had been missing: the two rows here are the measured attributes of
    /// *Roasted Boar Meat* (2540, `0x00010020`) — a cooking recipe, which
    /// belongs in the trade-skill window and nowhere else — and *Honorless
    /// Target* (2479, `0x09000120`), which carries the same bit and no other
    /// reason to be refused. Both were on the page a screenshot reported.
    ///
    /// The third row is *Cooking* itself (2550, `0x00010010`), the profession's
    /// own opener, and it stays: it is what the profession's spellbook tab is
    /// *for*, and a filter that took it would leave the player with no way to
    /// open the window the recipes moved to.
    #[test]
    fn a_recipe_is_not_in_the_book_and_the_opener_still_is() {
        use crate::tables::dbc::testing::dbc;
        use crate::tables::spellbook::{spell_fields, Spells};

        let row = |id: u32, name: u32, attributes: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::NAME] = name;
            row[spell_fields::ATTRIBUTES] = attributes;
            row
        };
        let catalog = Spells::parse(
            &dbc(
                &[
                    row(2550, 1, 0x0001_0010),
                    row(2540, 9, 0x0001_0020),
                    row(2479, 27, 0x0900_0120),
                ],
                173,
                b"\0Cooking\0Roasted Boar Meat\0Honorless Target\0",
            ),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("a catalog");

        let built = Spellbook::build(&[2550, 2540, 2479], 1, 8, &catalog, None);
        let ids: Vec<u32> = built.spells.iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![2550], "only the profession's opener is in the book");
        assert_eq!(built.tabs.len(), 1);
        assert_eq!(built.tabs[0].count, 1);
    }

    /// **General first whatever it is called**, then the rest alphabetically —
    /// the client's comparator returns before it looks either line up.
    #[test]
    fn general_sorts_first_and_the_rest_by_name() {
        let built = book(
            vec![
                (8, spell(1, "Fireball", "Rank 1", 1)),
                (GENERAL, spell(2, "Attack", "", 1)),
                (6, spell(3, "Frostbolt", "Rank 1", 1)),
                (237, spell(4, "Arcane Missiles", "Rank 1", 1)),
            ],
            &[(8, "Fire"), (6, "Frost"), (237, "Arcane")],
        );
        let order: Vec<u32> = built.tabs.iter().map(|t| t.skill_line).collect();
        // General, Arcane, Fire, Frost — and note that is *not* the numeric
        // order of the ids, which is the bug this test exists to catch.
        assert_eq!(order, vec![GENERAL, 237, 8, 6]);
    }

    /// **An offset is the running sum**, and the flat list's runs line up with
    /// it. Both halves in one test, because either alone would
    /// pass while the panel drew the wrong page.
    #[test]
    fn the_offsets_slice_the_flat_list_into_the_tabs() {
        let built = book(
            vec![
                (8, spell(1, "Fireball", "Rank 1", 1)),
                (8, spell(2, "Flamestrike", "Rank 1", 1)),
                (GENERAL, spell(3, "Attack", "", 1)),
                (6, spell(4, "Frostbolt", "Rank 1", 1)),
            ],
            &[(8, "Fire"), (6, "Frost")],
        );
        let tab = |i: usize| built.tab(i).expect("the tab");
        assert_eq!((tab(1).offset, tab(1).count), (0, 1), "General");
        assert_eq!((tab(2).offset, tab(2).count), (1, 2), "Fire");
        assert_eq!((tab(3).offset, tab(3).count), (3, 1), "Frost");
        // …and what the panel's own arithmetic reaches: button 1 of tab 2 is
        // `1 + offset`, one-based.
        assert_eq!(built.spell(1 + tab(2).offset).expect("a spell").id, 1);
        assert_eq!(built.spell(2 + tab(2).offset).expect("a spell").id, 2);
        assert_eq!(built.spell(1 + tab(3).offset).expect("a spell").id, 4);
    }

    /// Rank ascending, and **numerically** — the whole reason the client parses
    /// digits out of the string instead of comparing it.
    #[test]
    fn ranks_run_in_number_order_not_string_order() {
        let built = book(
            (1..=12)
                .map(|n| (8, spell(n, "Fireball", &format!("Rank {n}"), 1)))
                .collect(),
            &[(8, "Fire")],
        );
        let ranks: Vec<u32> = built.spells.iter().map(SpellInfo::rank_order).collect();
        assert_eq!(ranks, (1..=12).collect::<Vec<_>>());
    }

    /// A rank with no digits falls back to `spellLevel` — the client's own last
    /// line, and the only thing that separates two racials of one name.
    #[test]
    fn a_rank_with_no_digits_falls_back_to_the_spell_level() {
        assert_eq!(spell(1, "Perception", "Racial", 40).rank_order(), 40);
        assert_eq!(spell(1, "Fireball", "Rank 1", 40).rank_order(), 1);
        assert_eq!(spell(1, "Fireball", "", 0).rank_order(), 0);
        // "Rank 10" is ten and not one — the digits run together, and the scan
        // stops at the first non-digit *after* a digit.
        assert_eq!(spell(1, "Fireball", "Rank 10", 0).rank_order(), 10);
        assert_eq!(spell(1, "Shapeshift", "Rank 2 (Cat)", 0).rank_order(), 2);
    }

    /// A tab always has at least one page, because `SPELLBOOK_PAGENUMBERS`
    /// starts every tab at 1 and the arrows compare against the maximum.
    #[test]
    fn a_tab_holds_a_page_per_twelve_and_never_fewer_than_one() {
        let tab = |count| Tab {
            skill_line: 0,
            name: String::new(),
            icon: None,
            offset: 0,
            count,
        };
        assert_eq!(tab(0).pages(), 1);
        assert_eq!(tab(1).pages(), 1);
        assert_eq!(tab(12).pages(), 1);
        assert_eq!(tab(13).pages(), 2);
        assert_eq!(tab(24).pages(), 2);
    }

    /// **General's name is a key**, resolved through the string table — the one
    /// tab in the book whose `name` is not a word.
    #[test]
    fn the_general_tab_is_named_through_globalstrings() {
        let built = book(vec![(GENERAL, spell(1, "Attack", "", 1))], &[]);
        assert_eq!(
            built.tab_name(1, |key| (key == GENERAL_NAME_KEY).then(|| "General".to_string())),
            "General"
        );
        // …and with no such key it draws the key rather than nothing.
        assert_eq!(built.tab_name(1, |_| None), GENERAL_NAME_KEY);
        assert_eq!(built.tab(1).expect("the tab").icon.as_deref(), Some(GENERAL_ICON));
    }

    /// Indices are one-based on both accessors, and out of range is `None`
    /// rather than a panic — the panel asks for all eight skill-line tabs on
    /// every update whatever the character has.
    #[test]
    fn both_accessors_are_one_based_and_bounded() {
        let built = book(vec![(GENERAL, spell(1, "Attack", "", 1))], &[]);
        assert!(built.tab(0).is_none());
        assert!(built.tab(1).is_some());
        assert!(built.tab(2).is_none());
        assert!(built.spell(0).is_none());
        assert_eq!(built.spell(1).expect("a spell").id, 1);
        assert!(built.spell(2).is_none());
    }
}
