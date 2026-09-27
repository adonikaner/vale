//! **The C functions `QuestFrame.lua` and `QuestLogFrame.lua` call** — the
//! conversation's twenty reads and eight writes, and the log's nine.
//!
//! ```text
//! the giver           GetGreetingText  GetNumAvailableQuests  GetAvailableTitle
//!                                      GetNumActiveQuests     GetActiveTitle
//! the page            GetTitleText  GetQuestText  GetObjectiveText
//!                     GetProgressText  GetRewardText
//! what it pays        GetNumQuestChoices  GetNumQuestRewards  GetQuestItemInfo
//!                     GetRewardMoney  GetRewardSpell  GetQuestMoneyToGet
//!                     GetNumQuestItems  IsQuestCompletable
//! the log             GetNumQuestLogEntries  GetQuestLogTitle
//!                     GetQuestLogQuestText   GetQuestLogLeaderBoard
//!                     GetNumQuestLeaderBoards  GetQuestLogSelection
//!                     GetNumQuestLogChoices/Rewards  GetQuestLogRewardMoney
//!                     GetQuestLogRewardSpell  GetQuestLogRequiredMoney
//!                     GetQuestLogTimeLeft  IsCurrentQuestFailed
//! the writes          SelectAvailableQuest SelectActiveQuest AcceptQuest
//!                     DeclineQuest CompleteQuest GetQuestReward CloseQuest
//!                     QuestLog_SetSelection AbandonQuest
//! ```
//!
//! The split the spellbook set and the bags kept: the panels are the game's own
//! 1,400 lines of Lua, the *rule* is in [`vale_protocol::play::quest`] where a test
//! can run it with no window, and this file is the registration and the argument
//! handling.
//!
//! ## Every index that crosses this file is one-based
//!
//! `for i=1, GetNumAvailableQuests()`, `GetQuestLogTitle(i)`,
//! `GetQuestItemInfo("choice", i)`. The wire is zero-based in exactly one place
//! — `CMSG_QUESTGIVER_CHOOSE_REWARD`'s index — and that subtraction happens in
//! [`crate::interface::quest`], once, beside the send.
//!
//! ## `GetQuestItemInfo` takes a *word* rather than a list
//!
//! `"choice"`, `"reward"` and `"required"` are three different arrays in three
//! different packets, and the panel asks for all three through one function with
//! a string first argument. A host that answered the wrong array draws the right
//! number of buttons holding the wrong items — which is why the word is matched
//! rather than defaulted.
//!
//! ## …and the log's objective lines are a *join*
//!
//! `GetQuestLogLeaderBoard(i, questIndex)` answers `text, type, finished`, and
//! the text is composed from two packets: the template's own objective wording
//! (`CMSG_QUEST_QUERY`) and the counter in the log's packed word. Neither is
//! enough alone — the template says "Kobold Vermin slain" and the log says 3 of
//! 8 — so the sentence is built here and the `%s: %d/%d` shape is the game's.

use super::super::api::{one_or_nil, Answers};
// The unit-token surface these answers read the world through — imported
// here now that the subject's own answers live beside its registration.
use crate::interface::api;

/// One line of a quest's rewards, as `GetQuestItemInfo` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewardLine {
    /// The item's own entry — what `GameTooltip:SetQuestItem` needs and the
    /// only field here the panel never asks for directly. Kept beside the four
    /// it does because the reward arrays are already walked once per redraw and
    /// a second lookup by name would not identify the row.
    pub entry: u32,
    pub name: String,
    /// `Interface\Icons\…`, or `None` while the item's template is in flight.
    pub texture: Option<String>,
    pub count: u32,
    pub quality: u32,
    /// Whether the character can use it — `GetQuestItemInfo`'s fifth answer,
    /// which greys the button out. Always usable here: what decides it is the
    /// item's own class and race masks against the character's, and this client
    /// does not read those for a reward it has not been given yet.
    pub usable: bool,
}

/// **What `GetRewardSpell` answers**, and it is not a spell id.
///
/// The name invites the wrong shape and the wrong shape is silent. 1.12's
/// function returns **three** values — `texture, name, isTradeskillSpell` — and
/// `QuestFrameItems_Update` gates the whole "You will learn:" block on the
/// *first* of them being non-nil:
///
/// ```lua
/// if ( GetRewardSpell() ) then numQuestSpellRewards = 1; end
/// ```
///
/// A host that answers a spell id answers **`0` for "no reward spell"**, and
/// `0` is true in Lua — so every details page and every reward page in the game
/// grew a "You will learn:" heading over an empty item button, on quests with
/// no spell reward at all. It is the exact failure this project keeps a rule
/// about: a well-formed panel of nonsense that no headless count can see.
///
/// The client's own answer is four reads:
///
/// ```text
/// reward spell    the reward spell id; <= 0 or out of range -> nil, nil, nil
/// Spell.dbc[117]  spellIconID -> SpellIcon.dbc[1], the BLP path   (nil if absent)
/// Spell.dbc[120]  the localised name
/// Spell.dbc[6]    Attributes & 0x20 -> 1, else nil
/// ```
///
/// **The texture is nil-able on its own**, and that is deliberate rather than
/// tidy: the client pushes nil for a missing icon row and *still* pushes the
/// name, which the interface then reads as "no spell reward" and hides the
/// block. Answering an empty string there instead would show a nameless blank
/// button, which is the bug being fixed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewardSpell {
    /// `Interface\Icons\…`, or `None` when the icon table has nothing for it.
    pub texture: Option<String>,
    pub name: String,
    /// `SPELL_ATTR_TRADESPELL` — "You will be able to craft:" instead.
    pub tradeskill: bool,
}

/// One line of the log's objectives, as `GetQuestLogLeaderBoard` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectiveLine {
    /// The whole sentence, counter included — see the module note.
    pub text: String,
    /// `"monster"`, `"item"`, `"object"`, `"event"` or `"reputation"`, which is
    /// what `QuestLog_Update` colours the row by.
    pub kind: &'static str,
    pub finished: bool,
}

/// One row of the log, as `GetQuestLogTitle` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogRow {
    pub title: String,
    pub level: u32,
    /// A **header** row — a zone or category name with quests under it. The
    /// grouping is the *client's*: nothing on the wire says a quest belongs
    /// under "Elwynn Forest", and `ZoneOrSort`'s sign decides which of two
    /// tables answers. See [`vale_assets::tables::questsort`] and
    /// [`crate::interface::quest::Quests::rebuild_rows`].
    pub is_header: bool,
    /// …and whether that header is folded up — `GetQuestLogTitle`'s fifth
    /// answer, which decides between the plus and minus button art.
    pub collapsed: bool,
    pub complete: bool,
}

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 44] = [
    "GetActiveTitle",
    "GetAvailableTitle",
    "GetGreetingText",
    "GetNumActiveQuests",
    "GetNumAvailableQuests",
    "GetNumQuestChoices",
    "GetNumQuestItems",
    "GetNumQuestLeaderBoards",
    "GetNumQuestLogChoices",
    "GetNumQuestLogEntries",
    "GetNumQuestLogRewards",
    "GetNumQuestRewards",
    "GetNumQuestWatches",
    "GetQuestIndexForWatch",
    "GetObjectiveText",
    "GetProgressText",
    "GetQuestItemInfo",
    "GetQuestLogChoiceInfo",
    "GetQuestLogLeaderBoard",
    "GetQuestLogQuestText",
    "GetQuestLogRequiredMoney",
    "GetQuestLogRewardInfo",
    "GetQuestLogRewardMoney",
    "GetQuestLogRewardSpell",
    "GetQuestLogSelection",
    "GetQuestLogTimeLeft",
    "GetQuestLogTitle",
    "GetQuestMoneyToGet",
    "GetQuestText",
    "GetRewardMoney",
    "GetRewardSpell",
    "GetRewardText",
    "GetTitleText",
    "IsCurrentQuestFailed",
    "IsQuestCompletable",
    "IsQuestWatched",
    // **Registered with the reads and they are writes** — see the notes beside
    // them in [`install`]. Every one of these has to be visible before the call
    // returns, which is what keeps them out of the recorded queue.
    "AddQuestWatch",
    "CollapseQuestHeader",
    "GetAbandonQuestItems",
    "GetAbandonQuestName",
    "SetAbandonQuest",
    "ExpandQuestHeader",
    "RemoveQuestWatch",
    "SelectQuestLogEntry",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    macro_rules! read {
        ($name:expr, |$a:ident| $body:expr) => {{
            let f = scope.create_function(move |_, ()| {
                let $a = answers;
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
        ($name:expr, $args:ty, |$a:ident, $arg:ident| $body:expr) => {{
            let f = scope.create_function(move |_, $arg: $args| {
                let $a = answers;
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }

    // --- the giver's own two lists ---

    read!("GetGreetingText", |a| a.quest_greeting_text());
    read!("GetNumAvailableQuests", |a| a.quest_offers(false));
    read!("GetNumActiveQuests", |a| a.quest_offers(true));
    // **The reference pushes the title and nothing else**; the level is a second answer
    // this client offers and `QuestFrameGreetingPanel_OnShow` drops on the
    // floor — it does `questTitleButton:SetText(GetAvailableTitle(i))`. The
    // *gossip* pair really does carry a level, which is why it is here at all.
    read!("GetAvailableTitle", Option<i64>, |a, n| a
        .quest_offer_title(false, index(n)));
    read!("GetActiveTitle", Option<i64>, |a, n| a
        .quest_offer_title(true, index(n)));

    // --- the page's own text, whichever page it is ---
    //
    // Five names, one answer apiece, and the panel calls whichever pair belongs
    // to the page it was told about — so a name that answered the *wrong* page
    // would put a reward's text under a details page's title.
    read!("GetTitleText", |a| a.quest_page_text(Page::Title));
    read!("GetQuestText", |a| a.quest_page_text(Page::Details));
    read!("GetObjectiveText", |a| a.quest_page_text(Page::Objectives));
    read!("GetProgressText", |a| a.quest_page_text(Page::Progress));
    read!("GetRewardText", |a| a.quest_page_text(Page::Reward));

    // --- what it pays, and what it wants ---

    read!("GetNumQuestChoices", |a| a.quest_items(Which::Choice).len());
    read!("GetNumQuestRewards", |a| a.quest_items(Which::Reward).len());
    read!("GetNumQuestItems", |a| a.quest_items(Which::Required).len());
    read!("GetRewardMoney", |a| a.quest_money(false));
    read!("GetQuestMoneyToGet", |a| a.quest_money(true));
    // **Three answers or three nils** — see [`RewardSpell`], which is where the
    // reason a spell id would be wrong lives.
    read!("GetRewardSpell", |a| spell_answer(a.quest_reward_spell()));
    read!("GetQuestLogRewardSpell", |a| spell_answer(
        a.quest_log_reward_spell()
    ));
    read!("IsQuestCompletable", |a| one_or_nil(a.quest_completable()));

    // **The word decides the array** — see the module note.
    globals.set(
        "GetQuestItemInfo",
        scope.create_function(move |_, (which, n): (Option<String>, Option<i64>)| {
            let line = Which::of(which.as_deref())
                .and_then(|which| answers.quest_items(which).into_iter().nth(index(n)?))
                .unwrap_or_default();
            Ok((
                line.name,
                line.texture,
                line.count,
                line.quality,
                one_or_nil(line.usable),
            ))
        })?,
    )?;

    // --- the log ---

    // **`(numEntries, numQuests)` and they are different numbers.** The first
    // counts headers and is what the scroll frame is sized by; the second is
    // quests alone, and `QuestLog_Update` compares *it* against zero to decide
    // whether to draw `QUESTLOG_NO_QUESTS_TEXT` over the list. A host that
    // answered the same number twice showed an empty log as "no quests" only
    // when it had no headings either — which is to say, never.
    read!("GetNumQuestLogEntries", |a| (
        a.quest_log_rows(),
        a.quest_log_quest_rows()
    ));
    read!("GetQuestLogSelection", |a| a.quest_log_selection());
    // **The one interface write that is not recorded**, and it is registered
    // here — among the reads — for exactly that reason: it takes effect before
    // the call returns. `QuestLog_SetSelection` calls it and then, four lines
    // later in the same body, calls `QuestLog_UpdateQuestDetails`, which opens
    // with `GetQuestLogSelection()`. Recorded like every other write, the pane
    // filled with the row selected *before* the click — a one-click lag on
    // every selection in the log. See
    // [`crate::interface::quest::Quests::selected`], which carries the reason the
    // field can be written through a shared reference at all.
    globals.set(
        "SelectQuestLogEntry",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(row) = index(n) {
                answers.select_log_row(row + 1);
            }
            Ok(())
        })?,
    )?;
    // --- the tracker ---
    //
    // Five names over one five-slot array, and **all five have to answer inside
    // their own call**: `QuestLogTitleButton_OnClick` presses one of the two
    // writes and then calls `QuestWatch_Update()`, whose first line is
    // `for i=1, GetNumQuestWatches()`. Recorded and drained a system later, the
    // tracker is one click behind for ever — the same fault
    // `SelectQuestLogEntry` had and for the same reason. See
    // [`crate::interface::quest::Quests::watches`].
    read!("GetNumQuestWatches", |a| a.quest_watch_count());
    read!("IsQuestWatched", Option<i64>, |a, n| one_or_nil(
        index(n).is_some_and(|row| a.quest_is_watched(row + 1))
    ));
    // **A row, or nil for a quest that has left the log** — which is what
    // `QuestWatch_Update`'s own `if ( questIndex ) then` is testing, and why 0
    // would be wrong: it indexes a row that does not exist.
    read!("GetQuestIndexForWatch", Option<i64>, |a, n| index(n)
        .and_then(|watch| a.quest_watch_row(watch + 1)));
    // --- abandoning ---
    //
    // **Three names in front of `AbandonQuest`, and all three were missing** —
    // which is why the button did nothing at all: `QuestLogFrameAbandonButton`'s
    // `OnClick` opens with `SetAbandonQuest()`, so the body died on its first
    // statement and neither popup was ever shown. `AbandonQuest` itself has
    // been registered and reachable the whole time.
    //
    // `SetAbandonQuest` is a write and takes effect at once: the next two
    // statements in that same body read `GetAbandonQuestItems()` and
    // `GetAbandonQuestName()`, both of which are about what it just latched.
    globals.set(
        "SetAbandonQuest",
        scope.create_function(move |_, ()| {
            answers.quest_set_abandon();
            Ok(())
        })?,
    )?;
    read!("GetAbandonQuestName", |a| a.quest_abandon_name());
    // **`nil` when nothing would be destroyed**, which is what
    // `if ( items ) then` branches on to choose between the two popups.
    read!("GetAbandonQuestItems", |a| a.quest_abandon_items());

    // --- folding a heading ---
    //
    // **Both take effect inside their own call**, and the reason is the same
    // one `SelectQuestLogEntry` has: `QuestLog_SetSelection` calls one of these
    // instead of selecting whenever the row it read was a header and then
    // **returns**, and its caller `QuestLogTitleButton_OnClick` runs
    // `QuestLog_Update()` in the very next statement. Recorded and drained a
    // system later, the panel redraws the list as it was before the click and
    // stays one click behind for the rest of the session — see [`Display`] in
    // `interface::quest`, which has the four-click sequence that was reported.
    //
    // `0` is *all* the headings, which is what `QuestLogCollapseAllButton_OnClick`
    // passes outright — so the argument is not an index that has to be
    // one-based, it is an index **or** the sentinel, and `index()` would turn
    // the sentinel into nothing. See
    // [`crate::interface::quest::Quests::set_collapsed`].
    for (name, collapsed) in [("CollapseQuestHeader", true), ("ExpandQuestHeader", false)] {
        globals.set(
            name,
            scope.create_function(move |_, n: Option<i64>| {
                answers.quest_set_collapsed(row_or_all(n), collapsed);
                Ok(())
            })?,
        )?;
    }
    globals.set(
        "AddQuestWatch",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(row) = index(n) {
                answers.quest_add_watch(row + 1);
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "RemoveQuestWatch",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(row) = index(n) {
                answers.quest_remove_watch(row + 1);
            }
            Ok(())
        })?,
    )?;

    read!("GetQuestLogQuestText", |a| {
        let (details, objectives) = a.quest_log_text();
        (details, objectives)
    });
    read!("GetNumQuestLeaderBoards", Option<i64>, |a, n| a
        .quest_log_objectives(index(n).map(|i| i + 1).unwrap_or(0))
        .len());
    read!("GetNumQuestLogChoices", |a| a
        .quest_log_items(true)
        .len());
    read!("GetNumQuestLogRewards", |a| a
        .quest_log_items(false)
        .len());
    read!("GetQuestLogRewardMoney", |a| a.quest_log_money(false));
    // **The three that killed the whole detail pane.** `IsCurrentQuestFailed` is
    // `QuestLog_UpdateQuestDetails`' *fourth line* — before the title, the
    // description, the objectives and the rewards are written — so a missing
    // one leaves the panel showing the four placeholder strings the markup
    // shipped with (`text="Quest title"`, `QUEST_DESCRIPTION`, `QUEST_REWARDS`,
    // `REQUIRED_MONEY`) and never hides the money line. That is exactly the
    // screenshot, and no headless probe could see it: `--panels` dies earlier,
    // in `QuestLog_SetSelection`, on a colour `QuestLog_Update` assigns.
    read!("IsCurrentQuestFailed", |a| one_or_nil(a.quest_log_failed()));
    read!("GetQuestLogTimeLeft", |a| a.quest_log_time_left());
    read!("GetQuestLogRequiredMoney", |a| a.quest_log_money(true));

    // `(title, level, questTag, isHeader, isCollapsed, isComplete)` — six, and
    // the panel reads all six into one line.
    globals.set(
        "GetQuestLogTitle",
        scope.create_function(move |_, n: Option<i64>| {
            let row = index(n)
                .and_then(|i| answers.quest_log_row(i + 1))
                .unwrap_or_default();
            Ok((
                row.title,
                row.level,
                // `questTag` — "Elite", "Dungeon", "Raid" or nil. Off
                // `QuestTemplate::quest_type`, which this client reads and does
                // not yet name; nil is the ordinary quest and is what nearly
                // every row is.
                mlua::Value::Nil,
                one_or_nil(row.is_header),
                one_or_nil(row.collapsed),
                one_or_nil(row.complete),
            ))
        })?,
    )?;
    globals.set(
        "GetQuestLogLeaderBoard",
        scope.create_function(move |_, (n, row): (Option<i64>, Option<i64>)| {
            let row = index(row).map(|i| i + 1).unwrap_or(0);
            let line = index(n)
                .and_then(|i| answers.quest_log_objectives(row).into_iter().nth(i))
                .unwrap_or_default();
            Ok((line.text, line.kind, one_or_nil(line.finished)))
        })?,
    )?;
    globals.set(
        "GetQuestLogRewardInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let line = index(n)
                .and_then(|i| answers.quest_log_items(false).into_iter().nth(i))
                .unwrap_or_default();
            Ok((line.name, line.texture, line.count, line.quality, one_or_nil(line.usable)))
        })?,
    )?;
    globals.set(
        "GetQuestLogChoiceInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let line = index(n)
                .and_then(|i| answers.quest_log_items(true).into_iter().nth(i))
                .unwrap_or_default();
            Ok((line.name, line.texture, line.count, line.quality, one_or_nil(line.usable)))
        })?,
    )?;
    Ok(())
}

/// A [`RewardSpell`] as the three values 1.12's two functions push.
///
/// `None` is **three** nils rather than one — the reference returns 3 either
/// way, and the panel assigns all
/// three at once, so a single nil would leave `isTradeskillSpell` holding
/// whatever the last call left in that global.
fn spell_answer(spell: Option<RewardSpell>) -> (Option<String>, Option<String>, mlua::Value) {
    match spell {
        None => (None, None, mlua::Value::Nil),
        Some(spell) => (
            spell.texture,
            Some(spell.name),
            one_or_nil(spell.tradeskill),
        ),
    }
}

/// A one-based row index, with **0 kept as 0** — the "every heading" sentinel
/// the collapse verbs take. Anything negative or absent is 0 too, which is the
/// safe reading: folding all of them is visible and reversible, folding a row
/// nobody named is not.
fn row_or_all(n: Option<i64>) -> usize {
    usize::try_from(n.unwrap_or(0)).unwrap_or(0)
}

/// A one-based Lua index as a zero-based one, or `None` for anything else.
fn index(n: Option<i64>) -> Option<usize> {
    usize::try_from(n?.checked_sub(1)?).ok()
}

/// Which of the five texts a page read wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Title,
    Details,
    Objectives,
    Progress,
    Reward,
}

/// …and which of the three item arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Choice,
    Reward,
    Required,
}

impl Which {
    /// The word `GetQuestItemInfo` takes. **Matched rather than defaulted**:
    /// answering the wrong array draws the right number of buttons holding the
    /// wrong items.
    pub fn of(word: Option<&str>) -> Option<Which> {
        Some(match word? {
            "choice" => Which::Choice,
            "reward" => Which::Reward,
            "required" => Which::Required,
            _ => return None,
        })
    }
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::quest::QuestPress>>>;

/// Register the eight writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::quest::QuestPress as P;
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

    push!("SelectAvailableQuest", Option<i64>, |n| index(n)
        .map(|i| P::SelectAvailable(i + 1)));
    push!("SelectActiveQuest", Option<i64>, |n| index(n)
        .map(|i| P::SelectActive(i + 1)));
    push!("AcceptQuest", (), |_a| Some(P::Accept));
    push!("DeclineQuest", (), |_a| Some(P::Decline));
    push!("CompleteQuest", (), |_a| Some(P::Complete));
    push!("CloseQuest", (), |_a| Some(P::Close));
    push!("AbandonQuest", (), |_a| Some(P::Abandon));
    // **`GetQuestReward(choice)` is a write despite the name**, and it is the
    // one that ends the conversation: the reference's own function hands in the
    // quest and takes the highlighted reward. A quest with nothing to choose
    // between passes nil, which is the 0 the wire wants.
    push!("GetQuestReward", Option<i64>, |n| Some(P::TakeReward(
        index(n).map(|i| i + 1).unwrap_or(0)
    )));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// **The word decides the array.** Three packets, three arrays, one
    /// function — and a host that answered the wrong one draws the right number
    /// of buttons holding the wrong items.
    #[test]
    fn the_item_word_picks_which_of_three_arrays_answers() {
        assert_eq!(Which::of(Some("choice")), Some(Which::Choice));
        assert_eq!(Which::of(Some("reward")), Some(Which::Reward));
        assert_eq!(Which::of(Some("required")), Some(Which::Required));
        assert_eq!(Which::of(Some("rewards")), None, "not fuzzy");
        assert_eq!(Which::of(None), None);
    }

    /// One-based in, zero-based out, and everything else is nothing.
    #[test]
    fn a_lua_index_is_one_based_and_anything_else_is_nothing() {
        assert_eq!(index(Some(1)), Some(0));
        assert_eq!(index(Some(4)), Some(3));
        assert_eq!(index(Some(0)), None);
        assert_eq!(index(Some(-2)), None);
        assert_eq!(index(None), None);
    }

    /// **An empty log answers zeroes rather than nils**, which is what
    /// `QuestLog_Update` needs: its first line is `local numEntries, numQuests =
    /// GetNumQuestLogEntries()` and its second does arithmetic on both.
    #[test]
    fn an_empty_log_answers_numbers() {
        let world = Stub::default();
        assert_eq!(
            eval(&world, "local a, b = GetNumQuestLogEntries(); return a + b"),
            "Integer(0)"
        );
        assert_eq!(eval(&world, "return GetQuestLogSelection()"), "Integer(0)");
        assert_eq!(eval(&world, "return GetNumQuestLeaderBoards(1)"), "Integer(0)");
        // …and a row past the end is an empty row rather than a raise.
        assert_eq!(
            eval(&world, "local t = GetQuestLogTitle(3); return t"),
            r#"String("")"#
        );
    }

    /// **No reward spell is nil, not zero.** `QuestFrameItems_Update` gates the
    /// whole "You will learn:" block on `if ( GetRewardSpell() )`, and `0` is
    /// true in Lua — so an id-shaped answer put a heading and an empty item
    /// button on every details and reward page in the game. Three nils, and
    /// three of them rather than one, because the panel assigns all three.
    #[test]
    fn no_reward_spell_answers_three_nils_rather_than_a_zero() {
        let world = Stub::default();
        assert_eq!(
            eval(&world, "if GetRewardSpell() then return 1 else return 0 end"),
            "Integer(0)",
            "a zero here shows a blank 'You will learn:' on every quest"
        );
        assert_eq!(
            eval(
                &world,
                "local a, b, c = GetRewardSpell(); if a or b or c then return 1 else return 0 end"
            ),
            "Integer(0)"
        );
        assert_eq!(
            eval(
                &world,
                "if GetQuestLogRewardSpell() then return 1 else return 0 end"
            ),
            "Integer(0)"
        );
        // …and the shape the other arm hands over, straight out of the helper.
        let spell = RewardSpell {
            texture: Some("Interface\\Icons\\Spell_Fire_Fireball".into()),
            name: "Fireball".into(),
            tradeskill: true,
        };
        let (texture, name, trade) = spell_answer(Some(spell));
        assert_eq!(texture.as_deref(), Some("Interface\\Icons\\Spell_Fire_Fireball"));
        assert_eq!(name.as_deref(), Some("Fireball"));
        assert!(!trade.is_nil(), "the tradeskill sentence is the third answer");
        // An icon the table cannot resolve is nil *and keeps its name*, which
        // is the client's own two-arm push.
        let (texture, name, trade) = spell_answer(Some(RewardSpell {
            texture: None,
            name: "Fireball".into(),
            tradeskill: false,
        }));
        assert_eq!(texture, None);
        assert_eq!(name.as_deref(), Some("Fireball"));
        assert!(trade.is_nil());
    }

    /// The eight writes record in call order and answer nothing.
    #[test]
    fn the_writes_record_in_call_order() {
        use crate::interface::quest::QuestPress as P;
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        let value: mlua::Value = lua
            .load(
                r#"SelectAvailableQuest(2);
                   SelectActiveQuest(1);
                   AcceptQuest();
                   CompleteQuest();
                   GetQuestReward(3);
                   GetQuestReward();
                   SelectAvailableQuest(0);
                   return DeclineQuest()"#,
            )
            .eval()
            .expect("the chunk runs");
        assert!(value.is_nil(), "a verb answers nothing");
        assert_eq!(
            *queue.borrow(),
            vec![
                P::SelectAvailable(2),
                P::SelectActive(1),
                P::Accept,
                P::Complete,
                P::TakeReward(3),
                P::TakeReward(0),
                P::Decline,
            ],
            "a zero index records nothing, and a nil reward is choice 0"
        );
    }
}


/// **What the interface may ask about quests — the log the character carries, and the page in front of a giver.**
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
pub trait QuestAnswers {

    // --- what is on the body ---
    //
    // See [`super::loot`], which is where the six reads and the two numberings
    // are. Every row that crosses this boundary is the **screen's**, one-based.


    // --- quests ---
    //
    // See [`super::quest`], which is where the twenty-two reads and the
    // one-based indices are. Every index that crosses this boundary is the
    // interface's own, one-based, and the one zero-based number on the wire is
    // converted beside the send.

    /// `GetGreetingText()`.
    fn quest_greeting_text(&self) -> String;
    /// How many quests the giver has on offer (`false`) or wants back (`true`).
    fn quest_offers(&self, active: bool) -> usize;
    /// `GetAvailableTitle(n)` / `GetActiveTitle(n)` — the title, plus a level
    /// the questgiver panel ignores and the gossip pair uses.
    ///
    /// **Which half a quest is in is [`crate::interface::quest::is_active_offer`]'s**,
    /// and it is exactly icons 3 and 4 — not the `>= 4` the `DIALOG_STATUS`
    /// enum invites.
    fn quest_offer_title(&self, active: bool, index: Option<usize>) -> (String, u32);
    /// One of the five texts a page can be asked for.
    fn quest_page_text(&self, page: super::quest::Page) -> String;
    /// One of the three item arrays — see [`super::quest::Which`].
    fn quest_items(&self, which: super::quest::Which) -> Vec<super::quest::RewardLine>;
    /// `GetRewardMoney()` or `GetQuestMoneyToGet()`.
    fn quest_money(&self, required: bool) -> u32;
    /// `GetRewardSpell()` — **not a spell id**; see
    /// [`super::quest::RewardSpell`], which is where the reason lives.
    fn quest_reward_spell(&self) -> Option<super::quest::RewardSpell>;
    /// …and the same three answers about the *selected log row*.
    fn quest_log_reward_spell(&self) -> Option<super::quest::RewardSpell>;
    /// `IsQuestCompletable()` — whether the Continue button is live.
    fn quest_completable(&self) -> bool;
    /// `GetNumQuestLogEntries()`' first answer — rows, **headings included**.
    fn quest_log_rows(&self) -> usize;
    /// …and its second, which is quests alone.
    fn quest_log_quest_rows(&self) -> usize;
    /// `SelectQuestLogEntry(n)` — **a write, and it takes effect at once**;
    /// see [`crate::interface::quest::Quests::selected`].
    fn select_log_row(&self, row: usize);
    /// `GetQuestLogSelection()` — one-based, 0 for nothing.
    fn quest_log_selection(&self) -> usize;
    /// `GetQuestLogQuestText()` — `(details, objectives)`.
    fn quest_log_text(&self) -> (String, String);
    /// The objective lines of a log row, or of the selected one for 0.
    fn quest_log_objectives(&self, row: usize) -> Vec<super::quest::ObjectiveLine>;
    /// The selected quest's rewards: its choices (`true`) or what it gives
    /// outright (`false`).
    fn quest_log_items(&self, choices: bool) -> Vec<super::quest::RewardLine>;
    /// `GameTooltip:SetQuestRewardSpell()` (`false`) and its log twin (`true`) —
    /// the *plate* behind the same spell [`Answers::quest_reward_spell`] names,
    /// which is a different question from the icon and the heading.
    fn quest_reward_spell_tip(&self, from_log: bool) -> Option<api::SpellTip>;
    /// `GetQuestLogRewardMoney()` (`false`) or `GetQuestLogRequiredMoney()`
    /// (`true`) — **one signed field seen from both ends**; see the
    /// implementation, where vmangos' `GetRewOrReqMoney` is.
    fn quest_log_money(&self, required: bool) -> u32;
    /// `IsCurrentQuestFailed()` — the selected row's `QUEST_STATE_FAIL`.
    fn quest_log_failed(&self) -> bool;
    /// `GetQuestLogTimeLeft()` — seconds, or `None` for a quest with no clock,
    /// which is nearly all of them and is the interface's own test.
    fn quest_log_time_left(&self) -> Option<u32>;
    /// One row of the log, one-based.
    fn quest_log_row(&self, row: usize) -> Option<super::quest::LogRow>;

    // --- the tracker ---
    //
    // Every index here is a **log row**, one-based, and the storage behind it
    // holds quest ids — see [`crate::interface::quest::Quests::watches`], which
    // is where the conversion and the reason for it are. All five take effect
    // inside their own call.

    /// `SetAbandonQuest()` — **a write, and it takes effect at once**; latch
    /// the selected quest as the one the confirmation popup is about. See
    /// [`crate::interface::quest::Quests::abandon`].
    fn quest_set_abandon(&self);
    /// `GetAbandonQuestName()` — the latched quest's title.
    fn quest_abandon_name(&self) -> Option<String>;
    /// `GetAbandonQuestItems()` — the items abandoning it would destroy, joined
    /// with `", "`, or `None` when there are none. See the implementation,
    /// which follows the client arm for arm.
    fn quest_abandon_items(&self) -> Option<String>;

    /// `CollapseQuestHeader(row)` / `ExpandQuestHeader(row)` — **a write, and
    /// it takes effect at once**; `0` is every heading. See
    /// [`crate::interface::quest::Display`], which is where the one-click lag
    /// this shape avoids is written out.
    fn quest_set_collapsed(&self, row: usize, collapsed: bool);

    /// `GetNumQuestWatches()`.
    fn quest_watch_count(&self) -> usize;
    /// `IsQuestWatched(row)`.
    fn quest_is_watched(&self, row: usize) -> bool;
    /// `GetQuestIndexForWatch(i)` — the row the `i`th tracked quest is on now,
    /// or `None` for one that has left the log.
    fn quest_watch_row(&self, watch: usize) -> Option<usize>;
    /// `AddQuestWatch(row)` / `RemoveQuestWatch(row)` — **writes, and they take
    /// effect at once**.
    fn quest_add_watch(&self, row: usize);
    fn quest_remove_watch(&self, row: usize);
}

impl QuestAnswers for super::super::api::Live<'_, '_, '_> {


    // --- quests: the join between the log's counters and the templates' words ---

    fn quest_greeting_text(&self) -> String {
        match self.quests.page() {
            crate::interface::quest::Page::Greeting(page) => page.text.clone(),
            _ => String::new(),
        }
    }

    fn quest_offers(&self, active: bool) -> usize {
        match self.quests.page() {
            crate::interface::quest::Page::Greeting(page) => page
                .offers
                .iter()
                .filter(|o| crate::interface::quest::is_active_offer(o.icon) == active)
                .count(),
            _ => 0,
        }
    }

    fn quest_offer_title(&self, active: bool, index: Option<usize>) -> (String, u32) {
        let crate::interface::quest::Page::Greeting(page) = self.quests.page() else {
            return (String::new(), 0);
        };
        index
            .and_then(|i| {
                page.offers
                    .iter()
                    .filter(|o| crate::interface::quest::is_active_offer(o.icon) == active)
                    .nth(i)
            })
            .map(|o| (o.title.clone(), o.level))
            .unwrap_or_default()
    }

    /// **The five page texts, with their `$` variables substituted** — see
    /// [`crate::interface::messages::substitute`]. The database stores `$B$B` and
    /// `$N` verbatim and vmangos writes the column out unchanged, so this is
    /// the only place they can be resolved; untouched they reach the parchment
    /// as literals, which is what a screenshot of "Hello there, $c." is.
    fn quest_page_text(&self, page: super::quest::Page) -> String {
        use super::quest::Page as Which;
        use crate::interface::quest::Page as P;
        let raw = match (self.quests.page(), page) {
            // **The title is on four of the five pages**, which is why it is
            // asked for by a name of its own rather than falling out of
            // whichever page is up.
            (P::Details(d), Which::Title) => &d.title,
            (P::Progress(d), Which::Title) => &d.title,
            (P::Reward(d), Which::Title) => &d.title,
            (P::Details(d), Which::Details) => &d.details,
            (P::Details(d), Which::Objectives) => &d.objectives,
            (P::Progress(d), Which::Progress) => &d.text,
            (P::Reward(d), Which::Reward) => &d.text,
            _ => return String::new(),
        };
        crate::interface::messages::substitute(raw, self.speaker())
    }

    fn quest_items(&self, which: super::quest::Which) -> Vec<super::quest::RewardLine> {
        use super::quest::Which as W;
        use crate::interface::quest::Page as P;
        let items = match (self.quests.page(), which) {
            (P::Details(d), W::Choice) => &d.choices,
            (P::Details(d), W::Reward) => &d.rewards,
            (P::Reward(d), W::Choice) => &d.choices,
            (P::Reward(d), W::Reward) => &d.rewards,
            (P::Progress(d), W::Required) => &d.required_items,
            _ => return Vec::new(),
        };
        items
            .iter()
            .map(|item| self.quest_line(item.entry, item.count, item.display_id))
            .collect()
    }

    fn quest_money(&self, required: bool) -> u32 {
        use crate::interface::quest::Page as P;
        match (self.quests.page(), required) {
            // The progress page's field is already the absolute amount —
            // vmangos writes `RewOrReqMoney < 0 ? -RewOrReqMoney : 0` there
            // (`GossipDef.cpp:827`) — where the other two carry the signed one.
            // See [`Self::quest_log_money`], which is the same trap once.
            (P::Progress(d), true) => d.required_money,
            (P::Details(d), false) => (d.reward_money as i32).max(0) as u32,
            (P::Reward(d), false) => (d.reward_money as i32).max(0) as u32,
            _ => 0,
        }
    }

    fn quest_reward_spell(&self) -> Option<super::quest::RewardSpell> {
        use crate::interface::quest::Page as P;
        self.reward_spell(match self.quests.page() {
            P::Details(d) => d.reward_spell,
            P::Reward(d) => d.reward_spell,
            _ => 0,
        })
    }

    fn quest_log_reward_spell(&self) -> Option<super::quest::RewardSpell> {
        self.reward_spell(self.selected_template()?.reward_spell)
    }

    fn quest_completable(&self) -> bool {
        match self.quests.page() {
            crate::interface::quest::Page::Progress(d) => d.completable,
            // **A reward page is completable by definition** - it is only shown
            // for a quest the server has already accepted the hand-in of.
            crate::interface::quest::Page::Reward(_) => true,
            _ => false,
        }
    }

    fn quest_log_rows(&self) -> usize {
        self.quests.row_count()
    }

    fn quest_log_quest_rows(&self) -> usize {
        self.quests.quest_rows()
    }

    fn select_log_row(&self, row: usize) {
        self.quests.select_entry(row);
    }

    fn quest_set_collapsed(&self, row: usize, collapsed: bool) {
        self.quests.set_collapsed(row, collapsed);
    }

    fn quest_set_abandon(&self) {
        self.quests.set_abandon_quest();
    }

    fn quest_abandon_name(&self) -> Option<String> {
        let quest_id = self.quests.abandon_quest()?;
        Some(crate::interface::messages::substitute(
            &self.quests.template(quest_id)?.title,
            self.speaker(),
        ))
    }

    /// **What abandoning would actually destroy**, which is a narrower set than
    /// "the quest's required items" in two ways, and the client applies both:
    ///
    /// ```text
    /// GetItemCount(entry) > 0     -- skip one the character has none of
    /// …then walk every *other* quest in the log and strike out any
    ///   entry one of them also requires -- that one is not destroyed
    /// join what is left with ", "
    /// …and answer nothing at all if nothing is left
    /// ```
    ///
    /// The second is the half that matters and the half a plain reading of the
    /// name would miss: an item two quests want survives the abandon, so
    /// naming it in "Abandon %s, destroying %s?" would be a lie in the one
    /// direction a confirmation dialog must not lie.
    ///
    /// The item-record test (a field being 4 or 5) the reference makes
    /// between those two is
    /// **not** reproduced: it reads a field of the item record this client does
    /// not carry, and getting it wrong in either direction is a wrong sentence
    /// rather than a wrong action — the abandon itself destroys what the server
    /// says it destroys either way.
    fn quest_abandon_items(&self) -> Option<String> {
        let quest_id = self.quests.abandon_quest()?;
        let template = self.quests.template(quest_id)?;
        let mut entries: Vec<u32> = template
            .objective_lines
            .iter()
            .map(|line| line.item)
            .filter(|entry| *entry != 0 && self.inventory.carried.count_of(*entry) > 0)
            .collect();
        // …and strike out anything another quest in the log also asks for.
        for slot in self.quests.log() {
            if slot.quest_id == quest_id {
                continue;
            }
            let Some(other) = self.quests.template(slot.quest_id) else {
                continue;
            };
            entries.retain(|entry| !other.objective_lines.iter().any(|line| line.item == *entry));
        }
        let names: Vec<String> = entries
            .into_iter()
            .filter_map(|entry| self.session_template(entry).map(|t| t.name))
            .collect();
        (!names.is_empty()).then(|| names.join(", "))
    }

    fn quest_log_selection(&self) -> usize {
        self.quests.selected()
    }

    fn quest_log_text(&self) -> (String, String) {
        // Substituted here too — the log shows the *same* strings the details
        // page did, out of the same template.
        let who = self.speaker();
        self.selected_template()
            .map(|t| {
                (
                    crate::interface::messages::substitute(&t.details, who),
                    crate::interface::messages::substitute(&t.objectives, who),
                )
            })
            .unwrap_or_default()
    }

    fn quest_log_objectives(&self, row: usize) -> Vec<super::quest::ObjectiveLine> {
        let row = if row == 0 { self.quests.selected() } else { row };
        let Some(slot) = self.quests.at(row) else {
            return Vec::new();
        };
        let Some(template) = self.quests.template(slot.quest_id) else {
            return Vec::new();
        };
        // **`EndText` first, as an `"event"` line** — see
        // [`vale_protocol::play::quest::QuestTemplate::end_text`], where the
        // three uses that pin it are. The client answers it for index 1 and
        // returns before it looks at any of the four objectives, so it is a
        // *prefix* rather than an extra line at the end.
        //
        // It is what makes an exploration quest trackable at all: with nothing
        // countable this is the only line the quest has, and
        // `QuestWatch_Update` skips a quest whose `GetNumQuestLeaderBoards` is
        // 0. Empty on every quest with a real objective, so it costs those
        // nothing.
        let explored = (!template.end_text.is_empty()).then(|| super::quest::ObjectiveLine {
            text: crate::interface::messages::substitute(&template.end_text, self.speaker()),
            kind: "event",
            finished: slot.complete(),
        });
        explored
            .into_iter()
            .chain(template
            .objective_lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let (have, want, kind, name) = match (line.target, line.item) {
                    (Some(target), _) => (
                        u32::from(slot.counts[index]),
                        line.target_count,
                        match target {
                            vale_protocol::play::quest::Target::Creature(_) => "monster",
                            vale_protocol::play::quest::Target::GameObject(_) => "object",
                        },
                        self.objective_target_name(target),
                    ),
                    // **An item objective counts what is in the bags**, not what
                    // the log's counter says - the server does not write one for
                    // an item. This is the client's own arithmetic and the
                    // reference's too.
                    (None, item) if item != 0 => (
                        self.inventory.carried.count_of(item),
                        line.item_count,
                        "item",
                        self.session_template(item).map(|t| t.name),
                    ),
                    // **The third kind, and it has no counter at all.** An
                    // objective with neither a target nor an item is an *event*
                    // — "Speak to Marshal McBride" — whose whole content is the
                    // template's own wording. `GetQuestLogLeaderBoard` answers
                    // `"event"` for it, pushing the string and no
                    // numbers.
                    _ if !line.text.is_empty() => {
                        return Some(super::quest::ObjectiveLine {
                            text: crate::interface::messages::substitute(&line.text, self.speaker()),
                            kind: "event",
                            finished: slot.complete(),
                        })
                    }
                    _ => return None,
                };
                if want == 0 {
                    return None;
                }
                // **`ObjectiveText` overrides the looked-up name**, which is
                // what a hand-written objective is for; it is empty on the
                // overwhelming majority of quests, and *that* is the case the
                // sentence is composed for.
                let name = match line.text.is_empty() {
                    false => line.text.clone(),
                    // A name still in flight leaves the sentence blank rather
                    // than showing an id — the same one-round-trip nothing a
                    // log row's title shows, and it fills in when
                    // `world::templates` wakes the panel.
                    true => name.unwrap_or_default(),
                };
                Some(super::quest::ObjectiveLine {
                    text: self.leader_board_line(kind, &name, have, want),
                    kind,
                    finished: have >= want,
                })
            }))
            .collect()
    }

    fn quest_log_items(&self, choices: bool) -> Vec<super::quest::RewardLine> {
        let Some(template) = self.selected_template() else {
            return Vec::new();
        };
        let items = match choices {
            true => &template.reward_choices,
            false => &template.rewards,
        };
        items
            .iter()
            .map(|(entry, count)| self.quest_line(*entry, *count, 0))
            .collect()
    }

    fn quest_reward_spell_tip(&self, from_log: bool) -> Option<api::SpellTip> {
        use crate::interface::quest::Page as P;
        let spell = match from_log {
            true => self.selected_template()?.reward_spell,
            false => match self.quests.page() {
                P::Details(d) => d.reward_spell,
                P::Reward(d) => d.reward_spell,
                _ => 0,
            },
        };
        let catalog = self.tables.as_ref().and_then(|t| t.spellbook());
        let info = catalog?.info(spell)?;
        let names = |entry| self.reagent_name(entry);
        Some(api::spell_tip(
            &info,
            &api::TipContext {
                level: self.tip_level(),
                race: 0,
                class: 0,
                catalog,
                item_names: &names,
                home: Some(self.home.clone()),
            },
        ))
    }

    /// **One signed field, read from both ends.** vmangos writes
    /// `Quest::GetRewOrReqMoney()` — an `int32` — into the same slot of the
    /// template, the details page and the offer-reward page: positive is what
    /// the quest pays, **negative is what it charges**. Read as a `u32` a
    /// two-copper fee is four billion gold, so the sign is what decides which
    /// of the two questions is being answered.
    fn quest_log_money(&self, required: bool) -> u32 {
        let money = self.selected_template().map_or(0, |t| t.reward_money) as i32;
        match required {
            true => money.min(0).unsigned_abs(),
            false => money.max(0) as u32,
        }
    }

    fn quest_log_failed(&self) -> bool {
        self.quests
            .at(self.quests.selected())
            .is_some_and(|slot| slot.failed())
    }

    /// **The log slot's timer is an absolute unix second**, not a remaining
    /// one: `Player::AddQuest` writes `time(nullptr) + limittime` into
    /// `PLAYER_QUEST_LOG_n_3`, and 0 means the quest is not timed at all — which
    /// is the overwhelming majority and is why this answers `None` rather than
    /// 0 (`QuestLog_UpdateQuestDetails` tests `if ( questTimer )` and would show
    /// "Time Remaining: 0 Seconds" on every quest in the log).
    ///
    /// **Measured against this machine's clock**, which is the one assumption in
    /// it: the server stamped the field from its own `time()`, so a client whose
    /// clock is minutes out counts a timed quest down wrongly. Nothing on the
    /// wire offers a better reference — `SMSG_LOGIN_SETTIMESPEED` carries the
    /// *game* calendar, not unix time.
    fn quest_log_time_left(&self) -> Option<u32> {
        let expires = u64::from(self.quests.at(self.quests.selected())?.timer);
        if expires == 0 {
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        // An expired one still answers a number — the server has not failed it
        // yet, and the panel would otherwise flip to "no timer" a moment before
        // `SMSG_QUESTUPDATE_FAILEDTIMER` lands.
        Some(expires.saturating_sub(now) as u32)
    }

    fn quest_log_row(&self, row: usize) -> Option<super::quest::LogRow> {
        // **A heading is a row too**, and it is the row this read exists for:
        // `QuestLog_Update` branches on the fourth answer before it touches any
        // of the others, and a heading's title is the zone name rather than a
        // quest's.
        if let Some(crate::interface::quest::Row::Header(zone)) = self.quests.row(row) {
            return Some(super::quest::LogRow {
                title: self.quests.heading_name(zone).to_string(),
                level: 0,
                is_header: true,
                collapsed: self.quests.is_collapsed(row),
                complete: false,
            });
        }
        let slot = self.quests.at(row)?;
        let template = self.quests.template(slot.quest_id);
        Some(super::quest::LogRow {
            // **A row with no template yet is a blank rather than an id**, which
            // is the honest picture for the one round trip it lasts: the
            // reference shows nothing there either.
            title: template.map(|t| t.title.clone()).unwrap_or_default(),
            level: template.map_or(0, |t| t.level),
            is_header: false,
            collapsed: false,
            complete: slot.complete(),
        })
    }

    // --- the tracker, which is five words of the client's own state ---

    fn quest_watch_count(&self) -> usize {
        self.quests.watch_count()
    }

    fn quest_is_watched(&self, row: usize) -> bool {
        self.quests.is_watched(row)
    }

    fn quest_watch_row(&self, watch: usize) -> Option<usize> {
        self.quests.watch_row(watch)
    }

    fn quest_add_watch(&self, row: usize) {
        self.quests.add_watch(row);
    }

    fn quest_remove_watch(&self, row: usize) {
        self.quests.remove_watch(row);
    }
}