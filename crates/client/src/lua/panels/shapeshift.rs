//! **The four reads the stance bar is drawn from.**
//!
//! ```text
//! GetNumShapeshiftForms()       -> n              how many buttons
//! GetShapeshiftFormInfo(i)      -> texture, name, isActive, isCastable
//! GetShapeshiftFormCooldown(i)  -> start, duration, enable
//! ```
//!
//! `ShapeshiftBar_Update` reads the first and `ShapeshiftBar_UpdateState` the
//! other two, once per button, every time the character's auras move. The rules
//! are all in [`crate::game::combat::shapeshift`]; this file is only the surface.
//!
//! **`GetNumShapeshiftForms` was a stub answering 0**, which is worth a sentence
//! because it is the shape of failure this directory keeps producing: zero is a
//! perfectly good answer — it means "this character has no forms" — so
//! `ShapeshiftBar_Update` hid the frame, every probe reported the panel clean,
//! and a warrior's three stances were missing with nothing anywhere saying so.

use super::super::api::Answers;

/// The **scoped reads** this file registers, sorted — see
/// [`super::super::api::READS`].
pub const READS: [&str; 3] = [
    "GetNumShapeshiftForms",
    "GetShapeshiftFormCooldown",
    "GetShapeshiftFormInfo",
];

/// **What the interface may ask about the stance bar.**
pub trait ShapeshiftAnswers {
    /// `GetNumShapeshiftForms()` — how many buttons the bar has.
    fn shapeshift_form_count(&self) -> usize;

    /// **`GetShapeshiftFormInfo(i)`** -> `(texture, name, isActive, isCastable)`.
    ///
    /// `None` for an index past the end, which answers four nils — the branch
    /// `ShapeshiftBar_UpdateState` hides the button on.
    fn shapeshift_form_info(&self, index: usize) -> Option<ShapeshiftInfo>;

    /// `GetShapeshiftFormCooldown(i)` -> `(start, duration, enable)`, in
    /// [`Answers::now`]'s base.
    ///
    /// **`(0, 0, 1)` for an index past the end**, not three zeroes: the client
    /// pushes `1.0` for the third of them on that path, and
    /// `CooldownFrame_SetTimer` is called with the answer unconditionally.
    fn shapeshift_form_cooldown(&self, index: usize) -> (f64, f64, u32);
}

/// One button, as `GetShapeshiftFormInfo` answers for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShapeshiftInfo {
    /// The icon path — **the active one while the form is on**, where the spell
    /// has one.
    pub texture: String,
    pub name: String,
    /// Whether this is the form the character is in — see
    /// [`crate::game::combat::shapeshift::is_active`], where the two different
    /// questions behind it are.
    pub is_active: bool,
    /// Whether it could be cast right now. `ShapeshiftBar_UpdateState` greys the
    /// icon to 0.4 when this is nil.
    pub is_castable: bool,
}

impl super::super::api::Live<'_, '_, '_> {
    /// Whether a form that is *off* could be put on — the same test every other
    /// button in the game is tinted from, asked about a spell that is on no
    /// action-bar slot. See [`crate::game::api::spell_is_usable`].
    fn form_is_castable(&self, spell_id: u32) -> bool {
        self.tables
            .as_ref()
            .and_then(|tables| tables.spellbook()?.info(spell_id))
            .is_some_and(|info| crate::game::api::spell_is_usable(&info, self.units).0)
    }
}

impl ShapeshiftAnswers for super::super::api::Live<'_, '_, '_> {
    fn shapeshift_form_count(&self) -> usize {
        self.shapeshift.forms.len()
    }

    fn shapeshift_form_info(&self, index: usize) -> Option<ShapeshiftInfo> {
        let form = self.shapeshift.form(index)?;
        let current = self
            .units
            .get(crate::game::api::UnitId::Player)
            .map_or(0, |me| me.shapeshift_form);
        let is_active =
            crate::game::combat::shapeshift::is_active(form, current, &self.auras.player);
        // **The active icon while the form is on, the ordinary one otherwise**
        // — the client falls back to the ordinary one for a form that
        // states no active icon at all.
        let texture = match is_active && !form.active_texture.is_empty() {
            true => form.active_texture.clone(),
            false => form.texture.clone(),
        };
        Some(ShapeshiftInfo {
            texture,
            name: form.name.clone(),
            is_active,
            // **A form already on is castable**, which is the reference's own
            // shape rather than a simplification: it branches on
            // `isActive` and pushes `1.0` without asking anything else, because
            // the press is a *cancel* on that path and a cancel is always
            // available. Only an inactive one is tested.
            is_castable: is_active || self.form_is_castable(form.spell_id),
        })
    }

    fn shapeshift_form_cooldown(&self, index: usize) -> (f64, f64, u32) {
        let info = self
            .shapeshift
            .form(index)
            .and_then(|form| self.tables.as_ref()?.spellbook()?.info(form.spell_id));
        let Some(info) = info else {
            // …and `1` for the third, not `0` — see the trait.
            return (0.0, 0.0, 1);
        };
        // The same composition the spellbook's own rows take, so a stance's
        // swirl and the same spell's swirl in the book cannot disagree.
        let (start, duration, enable) =
            crate::game::api::cooldown_of(self.cooldowns, &info, self.now);
        (start, duration, u32::from(enable))
    }
}

/// Register the three reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let one_or_nil = super::super::api::one_or_nil;

    globals.set(
        "GetNumShapeshiftForms",
        scope.create_function(move |_, ()| Ok(answers.shapeshift_form_count()))?,
    )?;

    // **Four values, and the panel unpacks all four on one line.**
    // `texture, name, isActive, isCastable = GetShapeshiftFormInfo(i)` — and
    // note the order: the *texture* leads, which is the reverse of every other
    // `Get*Info` in the directory and is what a reader transcribing from another
    // one gets wrong.
    globals.set(
        "GetShapeshiftFormInfo",
        scope.create_function(move |_, index: Option<usize>| {
            let Some(info) = answers.shapeshift_form_info(index.unwrap_or(0)) else {
                // Four nils, which is what `ShapeshiftBar_UpdateState` hides the
                // button on — an index past the end rather than an error.
                return Ok((None, None, mlua::Value::Nil, mlua::Value::Nil));
            };
            Ok((
                Some(info.texture),
                Some(info.name),
                one_or_nil(info.is_active),
                one_or_nil(info.is_castable),
            ))
        })?,
    )?;

    globals.set(
        "GetShapeshiftFormCooldown",
        scope.create_function(move |_, index: Option<usize>| {
            Ok(answers.shapeshift_form_cooldown(index.unwrap_or(0)))
        })?,
    )?;
    Ok(())
}
