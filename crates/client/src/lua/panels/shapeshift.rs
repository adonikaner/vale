//! The three reads the stance bar is drawn from.
//!
//! ```text
//! GetNumShapeshiftForms()       -> n              how many buttons
//! GetShapeshiftFormInfo(i)      -> texture, name, isActive, isCastable
//! GetShapeshiftFormCooldown(i)  -> start, duration, enable
//! ```
//!
//! `ShapeshiftBar_Update` reads the first and `ShapeshiftBar_UpdateState` the
//! other two, once per button, every time the character's auras move. The rules
//! are all in [`crate::interface::shapeshift`]; this file is only the surface.
//!
//! `GetNumShapeshiftForms` must answer the real count. Zero is a valid answer
//! meaning "this character has no forms", so a stub answering 0 makes
//! `ShapeshiftBar_Update` hide the frame while every probe reports the panel
//! clean. A stub did exactly that and hid a warrior's three stances with no
//! error anywhere. Stubs answering a valid zero are a recurring failure in this
//! directory.

use super::super::api::Answers;

/// The scoped reads this file registers, sorted; see
/// [`super::super::api::READS`].
pub const READS: [&str; 3] = [
    "GetNumShapeshiftForms",
    "GetShapeshiftFormCooldown",
    "GetShapeshiftFormInfo",
];

/// What the interface may ask about the stance bar.
pub trait ShapeshiftAnswers {
    /// `GetNumShapeshiftForms()`: how many buttons the bar has.
    fn shapeshift_form_count(&self) -> usize;

    /// `GetShapeshiftFormInfo(i)` -> `(texture, name, isActive, isCastable)`.
    ///
    /// `None` for an index past the end, which answers four nils; that is the
    /// case in which `ShapeshiftBar_UpdateState` hides the button.
    fn shapeshift_form_info(&self, index: usize) -> Option<ShapeshiftInfo>;

    /// `GetShapeshiftFormCooldown(i)` -> `(start, duration, enable)`, in
    /// [`Answers::now`]'s base.
    ///
    /// `(0, 0, 1)` for an index past the end, not three zeroes: the 1.12.1
    /// client answers `1` for `enable` there, and `CooldownFrame_SetTimer` is
    /// called with the answer unconditionally.
    fn shapeshift_form_cooldown(&self, index: usize) -> (f64, f64, u32);
}

/// One button, as `GetShapeshiftFormInfo` answers for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShapeshiftInfo {
    /// The icon path: the active icon while the form is on, where the spell
    /// has one.
    pub texture: String,
    pub name: String,
    /// Whether this is the form the character is in; see
    /// [`crate::interface::shapeshift::is_active`], where the two different
    /// questions behind it are.
    pub is_active: bool,
    /// Whether it could be cast right now. `ShapeshiftBar_UpdateState` greys the
    /// icon to 0.4 when this is nil.
    pub is_castable: bool,
}

impl super::super::api::Live<'_, '_, '_> {
    /// Whether a form that is off could be put on: the same test every other
    /// button in the game is tinted from, asked about a spell that is on no
    /// action-bar slot. See [`crate::interface::api::spell_is_usable`].
    fn form_is_castable(&self, spell_id: u32) -> bool {
        self.tables
            .as_ref()
            .and_then(|tables| tables.spellbook()?.info(spell_id))
            .is_some_and(|info| crate::interface::api::spell_is_usable(&info, self.units, self.inventory).0)
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
            .get(crate::interface::api::UnitId::Player)
            .map_or(0, |me| me.shapeshift_form);
        let is_active =
            crate::interface::shapeshift::is_active(form, current, &self.auras.player);
        // The active icon while the form is on, the ordinary one otherwise.
        // The 1.12.1 client also shows the ordinary icon for an active form
        // that states no active icon.
        let texture = match is_active && !form.active_texture.is_empty() {
            true => form.active_texture.clone(),
            false => form.texture.clone(),
        };
        Some(ShapeshiftInfo {
            texture,
            name: form.name.clone(),
            is_active,
            // A form already on is castable. The 1.12.1 client reports an
            // active form as castable without further checks, because pressing
            // it cancels the form and a cancel is always available. Only an
            // inactive form is tested.
            is_castable: is_active || self.form_is_castable(form.spell_id),
        })
    }

    fn shapeshift_form_cooldown(&self, index: usize) -> (f64, f64, u32) {
        let info = self
            .shapeshift
            .form(index)
            .and_then(|form| self.tables.as_ref()?.spellbook()?.info(form.spell_id));
        let Some(info) = info else {
            // `1` for the third value, not `0`; see the trait.
            return (0.0, 0.0, 1);
        };
        // The same composition the spellbook's own rows take, so a stance's
        // swirl and the same spell's swirl in the book cannot disagree.
        let (start, duration, enable) =
            crate::interface::api::cooldown_of(self.cooldowns, &info, self.now);
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

    // Four values, which the panel unpacks on one line:
    // `texture, name, isActive, isCastable = GetShapeshiftFormInfo(i)`. The
    // texture comes first, the reverse of every other `Get*Info` in the
    // directory.
    globals.set(
        "GetShapeshiftFormInfo",
        scope.create_function(move |_, index: Option<usize>| {
            let Some(info) = answers.shapeshift_form_info(index.unwrap_or(0)) else {
                // Four nils for an index past the end, rather than an error;
                // `ShapeshiftBar_UpdateState` hides the button on them.
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
