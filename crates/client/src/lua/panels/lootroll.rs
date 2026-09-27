//! **The C functions `GroupLootFrame` calls** — two reads and two writes.
//!
//! ```text
//! GetLootRollItemInfo(id)   texture, name, count, quality, bindOnPickUp
//! GetLootRollItemLink(id)   …and the |Hitem: link a shift-click inserts
//! GetLootRollTimeLeft(id)   milliseconds, and it may be negative
//! RollOnLoot(id, rollType)  need, greed or pass
//! ConfirmLootRoll(id, type) …the same, past the bind-on-pickup question
//! ```
//!
//! …plus `GameTooltip:SetLootRollItem(id)`, which is registered in
//! [`super::super::widgets::tooltip`] beside the other seventeen scoped plates.
//!
//! The same split every other panel keeps: the frame is the game's own
//! `LootFrame.xml`, the *rule* is in [`vale_protocol::play::lootroll`] and
//! [`crate::interface::lootroll`], and this file is the registration and the
//! argument handling. Every id that crosses it is the **client's own roll id**,
//! never anything on the wire — see [`crate::interface::lootroll`], which is
//! where the two namings cross.
//!
//! ## `count` is 1 and it is not a placeholder
//!
//! `GetLootRollItemInfo` answers five values and the third of them is a literal
//! `1.0` — the client pushes the constant and never touches the roll record.
//! It has to: `SMSG_LOOT_START_ROLL` carries no stack count at all, where
//! `SMSG_LOOT_RESPONSE`'s rows do. So a roll on eight Linen Cloth draws the same
//! frame as a roll on one, and the reference has the same gap.
//!
//! ## …and an id nothing holds answers five values, not none
//!
//! The miss path pushes `nil, nil, 1, 1, nil` rather than returning empty. `GroupLootFrame_OnShow` reads all five into one line and
//! then indexes `ITEM_QUALITY_COLORS[quality]`, so a nil in the fourth slot is
//! an error inside the frame's own `OnShow` — which is the whole panel, not one
//! label.
//!
//! **`GetLootRollTimeLeft` is the other way**: it returns *no* values
//! for a miss. That is safe because the only caller is
//! `GroupLootFrame_OnUpdate`, which runs on a visible frame, and a visible frame
//! has a roll.

use super::super::api::Answers;

/// One roll, as the four things `GetLootRollItemInfo` answers that are not the
/// constant.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RollItem {
    /// `Interface\Icons\…`, or `None` while the template is still in flight —
    /// the same cold-cache state a loot row has. **Unlike a loot row there is no
    /// fallback icon**: `SMSG_LOOT_START_ROLL` carries no display id, so until
    /// the template lands there is nothing at all to draw.
    pub texture: Option<String>,
    pub name: String,
    /// `ITEM_QUALITY_*`, straight into `ITEM_QUALITY_COLORS[quality]`.
    pub quality: u32,
    /// **Whether the frame wears its gold border**, which is the one thing
    /// `GroupLootFrame_OnShow` branches on: `Bonding == 1`, and it swaps the
    /// backdrop, the corner texture and the `Decoration` overlay for it.
    pub bind_on_pickup: bool,
}

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 3] = [
    "GetLootRollItemInfo",
    "GetLootRollItemLink",
    "GetLootRollTimeLeft",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // `texture, name, count, quality, bindOnPickUp` — and **five values even for
    // a miss**, see the module note.
    globals.set(
        "GetLootRollItemInfo",
        scope.create_function(move |_, id: Option<i64>| {
            let roll = roll_id(id).and_then(|id| answers.loot_roll_item(id));
            let Some(roll) = roll else {
                // **Five values, and a usable quality among them** — see the
                // module note. The client pushes exactly this.
                return Ok((None, None, 1i64, 1i64, mlua::Value::Nil));
            };
            Ok((
                roll.texture,
                Some(roll.name),
                // **Always one.** See the module note: the packet has no count.
                1i64,
                i64::from(roll.quality),
                super::super::api::one_or_nil(roll.bind_on_pickup),
            ))
        })?,
    )?;

    // …and the link, which is what a shift-click inserts into the chat line and
    // what a ctrl-click hands `DressUpItemLink`.
    globals.set(
        "GetLootRollItemLink",
        scope.create_function(move |_, id: Option<i64>| {
            Ok(roll_id(id).and_then(|id| answers.loot_roll_link(id)))
        })?,
    )?;

    // …and the clock the bar is filled from. **No values at all for a miss**,
    // which is the reference's own answer and is not the same choice as above.
    globals.set(
        "GetLootRollTimeLeft",
        scope.create_function(move |_, id: Option<i64>| {
            Ok(roll_id(id).and_then(|id| answers.loot_roll_time_left(id)))
        })?,
    )?;
    Ok(())
}

/// A roll id out of a Lua argument that may be anything.
///
/// **Zero is a real id** — the counter starts there — so unlike every row
/// number in this directory the filter is only on the sign.
fn roll_id(id: Option<i64>) -> Option<u32> {
    u32::try_from(id?).ok()
}

/// **The two writes**, registered unscoped because they record — see
/// [`super::super::api::verbs`], which is where the argument is.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    // **`RollOnLoot(id, rollType)` — the three buttons.** `LootFrame.xml` wires
    // 0 to the close button, 1 to the dice and 2 to the coin.
    //
    // A vote outside 0..2 records nothing: the server refuses anything at or
    // above `MAX_ROLL_FROM_CLIENT` and a packet nobody answers is worse than a
    // press that did nothing. That is this client's reading rather than the
    // reference's, which sends whatever it is handed.
    for (name, confirmed) in [("RollOnLoot", false), ("ConfirmLootRoll", true)] {
        let queue = std::rc::Rc::clone(queue);
        let roll = lua.create_function(move |_, (id, vote): (Option<i64>, Option<i64>)| {
            let Some(id) = roll_id(id) else { return Ok(()) };
            let Some(vote) = vote
                .and_then(|vote| u8::try_from(vote).ok())
                .and_then(vale_protocol::play::lootroll::RollVote::of)
            else {
                return Ok(());
            };
            queue
                .borrow_mut()
                .push(crate::interface::lootroll::RollPress {
                    id,
                    vote,
                    confirmed,
                });
            Ok(())
        })?;
        globals.set(name, roll)?;
    }
    Ok(())
}

/// The queue the two writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::lootroll::RollPress>>>;

/// **What the interface may ask about a group roll.**
///
/// Split out of `Answers` on the same terms every other panel's is — see
/// [`super::loot::LootAnswers`], where the argument for the split is.
pub trait LootRollAnswers {
    /// `GetLootRollItemInfo(id)`, minus the constant count.
    fn loot_roll_item(&self, id: u32) -> Option<RollItem>;
    /// `GetLootRollItemLink(id)`. `None` until the template lands.
    fn loot_roll_link(&self, id: u32) -> Option<String>;
    /// `GetLootRollTimeLeft(id)` — **milliseconds, and it may be negative**.
    fn loot_roll_time_left(&self, id: u32) -> Option<f64>;
}

impl LootRollAnswers for super::super::api::Live<'_, '_, '_> {
    fn loot_roll_item(&self, id: u32) -> Option<RollItem> {
        let roll = self.rolls.get(id)?;
        // The session cache, as the loot window's rows: a roll's item is not
        // carried anywhere, and it is often one this character has never met.
        let template = self.session_template(roll.entry)?;
        Some(RollItem {
            // **No question-mark fallback here.** The loot window has one
            // because its rows carry a display id and can draw an icon before
            // the name arrives; a roll carries neither, so an unnamed roll has
            // nothing to show and this whole answer is `None` above.
            texture: self
                .tables
                .as_ref()
                .and_then(|t| t.item_icon(template.display_id)),
            name: template.name.clone(),
            quality: template.quality,
            bind_on_pickup: template.bonding == 1,
        })
    }

    fn loot_roll_link(&self, id: u32) -> Option<String> {
        let roll = self.rolls.get(id)?;
        let template = self.session_template(roll.entry)?;
        Some(super::container::item_link(
            template.entry,
            template.quality,
            &template.name,
        ))
    }

    fn loot_roll_time_left(&self, id: u32) -> Option<f64> {
        self.rolls.time_left_ms(id, self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};
    use vale_protocol::play::lootroll::RollVote;

    /// **Five answers, and the third of them is the constant** — a roll's packet
    /// has no stack count.
    #[test]
    fn a_roll_answers_its_icon_its_name_a_count_of_one_and_its_colour() {
        let mut world = Stub::default();
        world.roll = Some(RollItem {
            texture: Some("Interface\\Icons\\INV_Sword_04".into()),
            name: "Bloodrazor".into(),
            quality: 3,
            bind_on_pickup: true,
        });
        assert_eq!(
            eval(
                &world,
                "local t, n, c, q, b = GetLootRollItemInfo(0); return t .. '|' .. n .. '|' .. c .. '|' .. q .. '|' .. b"
            ),
            r#"String("Interface\\Icons\\INV_Sword_04|Bloodrazor|1|3|1")"#
        );
        // …and `bindOnPickUp` is nil rather than 0 when it does not bind, which
        // is what `GroupLootFrame_OnShow`'s `if ( bindOnPickUp )` reads.
        world.roll.as_mut().expect("a roll").bind_on_pickup = false;
        assert_eq!(eval(&world, "return GetLootRollItemInfo(0)"), r#"String("Interface\\Icons\\INV_Sword_04")"#);
        assert_eq!(
            eval(&world, "local _, _, _, _, b = GetLootRollItemInfo(0); return b"),
            "Nil"
        );
    }

    /// **An id nothing holds still answers five values**, because
    /// `GroupLootFrame_OnShow` indexes `ITEM_QUALITY_COLORS[quality]` with the
    /// fourth of them — a nil there takes the whole panel down.
    #[test]
    fn an_unknown_roll_answers_a_usable_quality_rather_than_nothing() {
        let world = Stub::default();
        assert_eq!(
            eval(
                &world,
                "local t, n, c, q, b = GetLootRollItemInfo(9); return tostring(t) .. '|' .. c .. '|' .. q"
            ),
            r#"String("nil|1|1")"#
        );
        // …and the clock is the other way round: no values at all.
        assert_eq!(eval(&world, "return GetLootRollTimeLeft(9)"), "Nil");
        assert_eq!(eval(&world, "return GetLootRollItemLink(9)"), "Nil");
        // A negative id is not a roll, and neither is nil.
        assert_eq!(eval(&world, "return GetLootRollItemInfo(-1)"), "Nil");
        assert_eq!(eval(&world, "return GetLootRollTimeLeft(nil)"), "Nil");
    }

    /// **Zero is a real roll id**, which is the one place this directory's usual
    /// "positive or nothing" filter would be wrong: the counter starts there.
    #[test]
    fn the_first_roll_of_a_session_has_id_zero() {
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load("RollOnLoot(0, 1)").exec().expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![crate::interface::lootroll::RollPress {
                id: 0,
                vote: RollVote::Need,
                confirmed: false,
            }]
        );
    }

    /// The two writes differ in exactly one field, and it is the one that
    /// decides whether the bind-on-pickup question gets asked again.
    #[test]
    fn confirm_is_the_same_press_with_the_question_already_answered() {
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load(
            "RollOnLoot(3, 0); RollOnLoot(3, 2); ConfirmLootRoll(3, 1); \
             RollOnLoot(3, 7); RollOnLoot(3); RollOnLoot(nil, 1)",
        )
        .exec()
        .expect("the chunk runs");
        let pressed = queue.borrow().clone();
        assert_eq!(pressed.len(), 3, "a vote of 7, a missing one and a missing id record nothing");
        assert_eq!(pressed[0].vote, RollVote::Pass);
        assert!(!pressed[0].confirmed);
        assert_eq!(pressed[1].vote, RollVote::Greed);
        assert_eq!(pressed[2].vote, RollVote::Need);
        assert!(pressed[2].confirmed, "ConfirmLootRoll is the confirmed one");
    }
}
