//! **The C functions `BuffFrame.lua` and `TargetFrame.lua` call** — the buff
//! bar's whole surface, and the target plate's two rows of icons.
//!
//! Two families, and the split is the game's rather than this file's:
//!
//! ```text
//! GetPlayerBuff(i, filter)        our own bar — a handle into the display cache
//! GetPlayerBuffTexture(handle)    …and the four questions asked *about* a handle
//! GetPlayerBuffApplications(h)
//! GetPlayerBuffTimeLeft(h)
//! GetPlayerBuffDispelType(h)
//! CancelPlayerBuff(h)             a write; see `lua::verbs`
//! GameTooltip:SetPlayerBuff(h)    …and the plate, in `lua::tooltip`
//!
//! UnitBuff(unit, i, castable)     anybody — indexed straight, no handles
//! UnitDebuff(unit, i, dispellable)
//! GameTooltip:SetUnitBuff/Debuff
//! ```
//!
//! ## The handle is the whole shape of the first family
//!
//! `GetPlayerBuff` is passed a **zero-based index into a filtered view** — the
//! sixteen helpful buttons carry ids 0..15 and the eight harmful ones 0..7, each
//! passing its own — and it answers something else entirely: an opaque handle
//! that the four reads below it and `CancelPlayerBuff` all take. Nothing in the
//! file ever does arithmetic on that handle; it is stored on the button
//! (`this.buffIndex`) and handed back.
//!
//! So it is free to be an index into the client's own display cache, which is
//! what [`crate::game::combat::auras::Auras::player`] is and what the reference's is
//! too. **`-1` for "there is none"**, never nil: `BuffButton_Update`'s first act
//! after the call is `if ( buffIndex < 0 )`.
//!
//! And it is passed back in freely when there is none. A *helpful* button asks
//!
//! ```lua
//! local debuffType = GetPlayerBuffDispelType(GetPlayerBuff(this:GetID(), "HARMFUL"));
//! ```
//!
//! purely to colour its border, so `GetPlayerBuffDispelType(-1)` is a call every
//! buff on screen makes on every update and it has to answer nil rather than
//! raise.
//!
//! ## …and the second family is one-based, per half
//!
//! `TargetDebuffButton_Update` runs `for i = 1, MAX_TARGET_BUFFS` and again
//! `for i = 1, MAX_TARGET_DEBUFFS`, drawing until the first nil. So the two
//! halves are numbered separately and from one — a single list would make the
//! first debuff appear as buff seventeen.
//!
//! ## What is answered `nil` on purpose
//!
//! `UnitBuff`'s and `UnitDebuff`'s **third arguments** (`SHOW_CASTABLE_BUFFS`,
//! `SHOW_DISPELLABLE_DEBUFFS`) are read and ignored — see
//! [`crate::game::combat::auras`], where the deviation is stated. And a token this
//! client keeps no aura list for answers nothing at all rather than the
//! player's, which is the class of plausible wrong answer this project keeps
//! paying for.

use super::super::api::{aura_info, Answers};

/// **The reads this module registers into the scope**, for the count that
/// measures the gap — the same kind of list [`super::super::api::READS`] is.
///
/// `CancelPlayerBuff` is a **write** and is in [`super::super::api::verbs::REGISTERED`];
/// the three `GameTooltip:Set*Buff` methods are widget methods and are in
/// [`super::super::widgets::tooltip::SCOPED_METHODS`].
pub const READS: [&str; 7] = [
    "GetPlayerBuff",
    "GetPlayerBuffApplications",
    "GetPlayerBuffDispelType",
    "GetPlayerBuffTexture",
    "GetPlayerBuffTimeLeft",
    "UnitBuff",
    "UnitDebuff",
];

/// One aura, as far as an interface call needs it. Owned rather than borrowed
/// because [`Answers`] erases the world's lifetimes — see that module.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuraInfo {
    pub spell: u32,
    /// `Interface\Icons\…`, empty when nothing knows.
    pub icon: String,
    pub name: String,
    /// The sentence, for the hover plate.
    pub description: String,
    /// One-based; the interface draws it only above 1.
    pub applications: u8,
    /// "Magic" / "Curse" / "Disease" / "Poison", or empty.
    pub dispel_type: String,
    /// Seconds left, `0.0` when there is no clock — see
    /// [`crate::game::combat::auras::Aura::time_left`] for why not nil.
    pub time_left: f64,
    /// Whether it runs until it is cancelled, which is `GetPlayerBuff`'s
    /// **second** return and what hides the duration text.
    pub until_cancelled: bool,
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // `GetPlayerBuff(index, filter)` -> `(handle, untilCancelled)`.
    //
    // **Two returns**, and the second is what `BuffButton_OnUpdate` checks
    // before it reads a timer at all: `if ( this.untilCancelled == 1 ) then
    // buffDuration:Hide(); return; end`. So an aura the server has said nothing
    // about is honestly "no timer" rather than a countdown from zero.
    globals.set(
        "GetPlayerBuff",
        scope.create_function(move |_, (index, filter): (Option<i64>, Option<String>)| {
            // A negative index is not a slot. The file never passes one, but
            // `usize::try_from` on the way in is cheaper than the alternative
            // being a panic.
            let handle = usize::try_from(index.unwrap_or(0))
                .map_or(-1, |index| answers.player_buff(index, filter.as_deref().unwrap_or("")));
            let until_cancelled = answers
                .player_buff_at(handle)
                .is_some_and(|aura| aura.until_cancelled);
            Ok((handle, super::super::api::one_or_nil(until_cancelled)))
        })?,
    )?;

    // The four reads *about* a handle. Each takes the `-1` the interface passes
    // around and answers the absent value — see the module comment.
    macro_rules! by_handle {
        ($name:expr, |$aura:ident| $body:expr) => {{
            let f = scope.create_function(move |_, handle: Option<i64>| {
                let handle = handle.unwrap_or(-1).try_into().unwrap_or(-1);
                Ok(answers.player_buff_at(handle).map(|$aura| $body))
            })?;
            globals.set($name, f)?;
        }};
    }
    by_handle!("GetPlayerBuffTexture", |aura| aura.icon);
    by_handle!("GetPlayerBuffDispelType", |aura| aura.dispel_type);

    // …and the two that must answer a **number** rather than nothing, because
    // the caller compares them. `GetPlayerBuffApplications`' answer meets
    // `if ( count > 1 )` and `GetPlayerBuffTimeLeft`'s meets
    // `if ( timeLeft < BUFF_WARNING_TIME )`, both on the next line.
    globals.set(
        "GetPlayerBuffApplications",
        scope.create_function(move |_, handle: Option<i64>| {
            let handle = handle.unwrap_or(-1).try_into().unwrap_or(-1);
            Ok(answers
                .player_buff_at(handle)
                .map_or(0, |aura| i64::from(aura.applications)))
        })?,
    )?;
    globals.set(
        "GetPlayerBuffTimeLeft",
        scope.create_function(move |_, handle: Option<i64>| {
            let handle = handle.unwrap_or(-1).try_into().unwrap_or(-1);
            Ok(answers
                .player_buff_at(handle)
                .map_or(0.0, |aura| aura.time_left))
        })?,
    )?;

    // `UnitBuff(unit, i)` -> `(texture, applications)` and
    // `UnitDebuff(unit, i)` -> `(texture, applications, dispelType)`.
    //
    // The debuff's third return is what `DebuffTypeColor` is indexed by, and it
    // is `nil` rather than `""` for an undispellable one — `if ( debuffType )`
    // is the branch that picks the "none" red.
    macro_rules! half {
        ($name:expr, $helpful:expr) => {{
            let f = scope.create_function(
                move |_, (token, index, _filter): (Option<String>, Option<usize>, Option<mlua::Value>)| {
                    let aura = answers.unit_aura(
                        token.as_deref().unwrap_or(""),
                        index.unwrap_or(0),
                        $helpful,
                    );
                    Ok(match aura {
                        Some(aura) => (
                            Some(aura.icon),
                            Some(i64::from(aura.applications)),
                            (!aura.dispel_type.is_empty()).then_some(aura.dispel_type),
                        ),
                        None => (None, None, None),
                    })
                },
            )?;
            globals.set($name, f)?;
        }};
    }
    half!("UnitBuff", true);
    half!("UnitDebuff", false);
    Ok(())
}

/// **`GameTooltip:SetPlayerBuff` / `SetUnitBuff` / `SetUnitDebuff`'s contents.**
///
/// The name and the sentence, and nothing between them. A buff's plate in 1.12
/// is not a spell's — there is no mana cost, no range and no cast time on it,
/// because the aura is not something you are about to cast — so this is
/// deliberately *not* routed through [`super::super::widgets::tooltip`]'s spell-line law.
///
/// The duration line the real client draws under the name is **not** composed:
/// it wants `SecondsToTimeAbbrev`-style wording per aura and the timer is
/// already drawn under the icon by `BuffButton_OnUpdate`, so what a hover adds
/// here is a repeat. Stated rather than overlooked.
pub(in crate::lua) fn tooltip_lines(aura: &AuraInfo) -> (&str, &str) {
    (&aura.name, &aura.description)
}

#[cfg(test)]
mod tests {
    use crate::lua::api::tests::{eval, Stub};

    /// A player carrying two buffs and a debuff, and a target carrying one of
    /// each — the smallest world that exercises both families.
    fn world() -> Stub {
        Stub::default()
            .buff("player", 100, "Ice Armor", true, true)
            .buff("player", 200, "Arcane Intellect", true, true)
            .buff("player", 300, "Curse of Agony", false, false)
            .buff("target", 400, "Battle Shout", true, true)
            .buff("target", 500, "Rend", false, false)
    }

    /// **`GetPlayerBuff` is zero-based into a filtered view and answers a handle
    /// into the whole list**, which is the shape the twenty-four buttons are
    /// written against.
    #[test]
    fn the_bar_indexes_its_two_halves_from_zero_apiece() {
        let world = world();
        assert_eq!(eval(&world, r#"return GetPlayerBuff(0, "HELPFUL")"#), "Integer(0)");
        assert_eq!(eval(&world, r#"return GetPlayerBuff(1, "HELPFUL")"#), "Integer(1)");
        // …and the third helpful button, of sixteen, finds nothing.
        assert_eq!(eval(&world, r#"return GetPlayerBuff(2, "HELPFUL")"#), "Integer(-1)");
        // The harmful half starts at its own zero and answers a handle into the
        // *same* list — which is why the two numberings can differ.
        assert_eq!(eval(&world, r#"return GetPlayerBuff(0, "HARMFUL")"#), "Integer(2)");
    }

    /// **`-1` is passed back in constantly and must answer nothing.**
    /// `BuffButton_Update` asks a helpful button for its *harmful* dispel type
    /// on every update purely to colour a border.
    #[test]
    fn a_handle_of_minus_one_answers_nothing_rather_than_raising() {
        let world = world();
        assert_eq!(
            eval(&world, r#"return GetPlayerBuffDispelType(GetPlayerBuff(0, "HARMFUL"))"#),
            "String(\"Curse\")"
        );
        assert_eq!(eval(&world, "return GetPlayerBuffTexture(-1)"), "Nil");
        // The two the caller compares with `<` and `>` answer a number.
        assert_eq!(eval(&world, "return GetPlayerBuffApplications(-1)"), "Integer(0)");
        // …and a time is a number too, since the caller compares it with `<`.
        assert_eq!(eval(&world, "return GetPlayerBuffTimeLeft(-1)"), "Integer(0)");
    }

    /// `UnitBuff` and `UnitDebuff` are **one-based per half** and stop at the
    /// first nil, which is how both loops in `TargetDebuffButton_Update` end.
    #[test]
    fn another_units_rows_are_one_based_per_half() {
        let world = world();
        assert_eq!(
            eval(&world, r#"return UnitBuff("target", 1)"#),
            "String(\"Interface\\\\Icons\\\\Battle Shout\")"
        );
        assert_eq!(eval(&world, r#"return UnitBuff("target", 2)"#), "Nil");
        assert_eq!(
            eval(&world, r#"local t, n, d = UnitDebuff("target", 1); return d"#),
            "String(\"Curse\")"
        );
        assert_eq!(eval(&world, r#"return UnitDebuff("target", 2)"#), "Nil");
    }

    /// **A token this client has no list for answers nothing**, rather than the
    /// player's — the four party frames call `RefreshBuffs` with `"party1"` at
    /// every login.
    #[test]
    fn a_unit_with_no_list_answers_nothing() {
        let world = world();
        assert_eq!(eval(&world, r#"return UnitBuff("party1", 1)"#), "Nil");
        assert_eq!(eval(&world, r#"return UnitDebuff("mouseover", 1)"#), "Nil");
        assert_eq!(eval(&world, "return UnitBuff()"), "Nil");
    }

    /// An aura with no clock is **until cancelled**, which is what hides the
    /// duration text rather than showing a countdown from zero.
    #[test]
    fn an_aura_the_server_has_not_timed_runs_until_cancelled() {
        let world = world();
        assert_eq!(
            eval(&world, r#"local h, u = GetPlayerBuff(0, "HELPFUL"); return u"#),
            "Integer(1)"
        );
    }
}


/// **What the interface may ask about what is on a unit.**
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
pub trait AuraAnswers {

    // --- the auras ---
    //
    // See [`super::auras`] for the two numbering schemes and why the first
    // family answers a *handle* rather than a value.

    /// `GetPlayerBuff(index, filter)`'s handle — **`-1` for none**, never nil.
    fn player_buff(&self, index: usize, filter: &str) -> i32;
    /// What a handle points at, or `None` for the `-1` the interface passes
    /// back in freely.
    fn player_buff_at(&self, handle: i32) -> Option<super::auras::AuraInfo>;
    /// `UnitBuff` / `UnitDebuff` — the `index`-th aura of one half, **one-based
    /// per half**, or `None` past the end and for a unit this client keeps no
    /// list for.
    fn unit_aura(&self, token: &str, index: usize, helpful: bool)
        -> Option<super::auras::AuraInfo>;
}

impl AuraAnswers for super::super::api::Live<'_, '_, '_> {

    fn player_buff(&self, index: usize, filter: &str) -> i32 {
        self.auras.player_buff(index, filter)
    }

    fn player_buff_at(&self, handle: i32) -> Option<super::auras::AuraInfo> {
        Some(aura_info(self.auras.player_at(handle)?, self.now))
    }

    fn unit_aura(
        &self,
        token: &str,
        index: usize,
        helpful: bool,
    ) -> Option<super::auras::AuraInfo> {
        let list = self.auras.of(Self::id(token)?)?;
        Some(aura_info(
            crate::game::combat::auras::nth_of_half(list, index, helpful)?,
            self.now,
        ))
    }
}