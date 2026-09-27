//! **The stance bar** — the warrior's three stances, the druid's five forms,
//! the paladin's auras, and the one button a rogue gets.
//!
//! `Interface\FrameXML\BonusActionBarFrame.lua` holds both halves of this and
//! they are different subjects sharing a file: `BonusActionBar_*` slides the
//! *bonus* bar, which is [`super::action::ActionBar::bonus_bar`] and was already
//! answered, and `ShapeshiftBar_*` draws the row of form buttons above it, which
//! is this module. Nothing here crosses the wire in either direction — the list,
//! its order and which button is pressed in are all client reads of
//! `Spell.dbc`, which is why it stayed unanswered for as long as it did: no
//! packet was missing to point at it.
//!
//! ## What the interface asks
//!
//! ```text
//! GetNumShapeshiftForms()          how many buttons to show
//! GetShapeshiftFormInfo(i)         texture, name, isActive, isCastable
//! GetShapeshiftFormCooldown(i)     start, duration, enable
//! CastShapeshiftForm(i)            press one
//! ```
//!
//! and it is told the list moved by `UPDATE_SHAPESHIFT_FORMS`.
//!
//! ## The list is derived, and the derivation is the interesting half
//!
//! The reference keeps the spell ids in an array and maintains it
//! incrementally: every spell learned is tested and appended, every spell
//! unlearned is removed, and the array is re-sorted and the event raised.
//! This client rebuilds it off the spellbook version instead,
//! which reaches the same states through the same event and cannot drift out of
//! step with the book.
//!
//! **The test and the sort are the reference's**, and both are stated at the
//! functions that carry them: [`vale_assets::tables::spellbook::SpellInfo::is_shapeshift_button`]
//! is the two-bit-plus-aura rule, and `shapeshift_order` is the column the
//! reference's sort comparator reads. The sort is worth one sentence
//! because it is the only thing standing between a correct list and a bar whose
//! buttons are in spell-id order: a druid would get Cat, Travel, Bear, Aquatic,
//! Moonkin, which is wrong in a way no count would report.
//!
//! ## What "active" means, and it is two different things
//!
//! `GetShapeshiftFormInfo`'s third answer is the form byte comparison
//! (`UNIT_FIELD_BYTES_1`'s form against the spell's own
//! `EffectMiscValue`) — **for a spell that applies a shapeshift aura at all**.
//! A paladin's aura and a rogue's Stealth apply none, and for those the
//! reference asks a different question entirely: whether *this
//! spell's* aura is on the player. Both are answered here, and the second is why
//! this module reads [`super::auras::Auras`].

use vale_assets::tables::spellbook::SpellInfo;
use bevy::prelude::*;

use super::events::UpdateShapeshiftForms;
use crate::assets::GameAssets;
use crate::world::session::{LocalPlayer, Session, WorldEntity};

/// One button on the stance bar.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeshiftForm {
    pub spell_id: u32,
    pub name: String,
    /// The ordinary icon.
    pub texture: String,
    /// …and the one worn while the form is on, empty for a button with none —
    /// see [`vale_assets::tables::spellbook::spell_fields::ACTIVE_ICON_ID`].
    pub active_texture: String,
    /// **The form this puts the character into**, or 0 for a button that is not
    /// a form — a paladin's aura, or Stealth. See *What "active" means*.
    pub form_id: u32,
}

/// **The stance bar as the panel reads it**, rebuilt when the book moves.
#[derive(Resource, Default)]
pub struct ShapeshiftBar {
    pub forms: Vec<ShapeshiftForm>,
    /// The `spellbook_version` this was built from — the same latch
    /// [`super::spellbook`] uses and for the same reason.
    built_from: Option<u32>,
    /// …and which parse of the DBCs the forms were resolved from — see
    /// `GameAssets::tables_generation`.
    built_with: u64,
}

impl ShapeshiftBar {
    /// One-based, the way `GetShapeshiftFormInfo(i)` is called.
    pub fn form(&self, index: usize) -> Option<&ShapeshiftForm> {
        self.forms.get(index.checked_sub(1)?)
    }
}

pub struct ShapeshiftPlugin;

impl Plugin for ShapeshiftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShapeshiftBar>().add_systems(
            Update,
            // **The rebuild before the press**, stated rather than inherited on
            // the same terms as [`super::pet`]'s: a press looks a spell id up in
            // the list the rebuild wrote.
            (rebuild, press, leave_world)
                .chain()
                .in_set(super::GameSet),
        );
    }
}

/// **Build the list**, and say so when it changes.
///
/// The filter and the sort are both the reference's — see the module comment.
/// The sort is `(order, id)` with `-1` last, which is the reference's
/// comparator written as a key: that comparator's whole content is the order column, the `-1` special
/// case, and the spell id as the tie-break.
fn rebuild(
    session: Res<Session>,
    assets: Res<GameAssets>,
    mut bar: ResMut<ShapeshiftBar>,
    mut changed: MessageWriter<UpdateShapeshiftForms>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let version = active.live.spellbook_version();
    // …and which parse of the DBCs it was resolved from, which nothing on
    // the wire states — see `GameAssets::tables_generation`, and
    // `combat::spellbook::rebuild`, where the reason the server's version is
    // not enough on its own is.
    let parse = assets.tables_generation();
    if bar.built_from == Some(version) && bar.built_with == parse {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(catalog) = tables.spellbook() else {
        return;
    };
    let known = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.spellbook.known.clone()
    };
    bar.built_from = Some(version);
    bar.built_with = parse;

    let mut forms: Vec<(i32, u32, ShapeshiftForm)> = known
        .iter()
        .filter_map(|id| catalog.info(*id))
        .filter(SpellInfo::is_shapeshift_button)
        .map(|info| {
            (
                // **`-1` sorts last**, and it is the only value that does not
                // sort by its own number. Mapped to `i32::MAX`
                // rather than compared specially, which is the same ordering in
                // one fewer branch.
                match info.shapeshift_order {
                    -1 => i32::MAX,
                    order => order,
                },
                info.id,
                ShapeshiftForm {
                    spell_id: info.id,
                    name: info.name.clone(),
                    texture: info.icon.clone(),
                    active_texture: info.active_icon.clone(),
                    form_id: info.shapeshift_form(),
                },
            )
        })
        .collect();
    forms.sort_by_key(|(order, id, _)| (*order, *id));
    let forms: Vec<ShapeshiftForm> = forms.into_iter().map(|(_, _, form)| form).collect();
    if bar.forms == forms {
        // The book moved for some other reason. Restating an unchanged list
        // would rebuild the bar on every rank learned all game.
        return;
    }
    bar.forms = forms;
    changed.write(UpdateShapeshiftForms);
}

/// **`CastShapeshiftForm(i)` — press one**, which is three outcomes rather than
/// one cast.
///
/// * **the form is on and cannot be cancelled** — the three warrior stances,
///   whose `SpellShapeshiftForm.dbc` flags carry bit 1: nothing happens at all,
///   and no packet goes;
/// * **the form is on and can be** — the druid's five: the aura is *cancelled*,
///   which is `CMSG_CANCEL_AURA` and not a cast;
/// * **the form is off** — cast it, with no target.
///
/// The middle branch is the one worth having: a client that cast the spell again
/// to leave a form would put the character back into the form it was already in,
/// which reads on screen as the button doing nothing.
fn press(
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    bar: Res<ShapeshiftBar>,
    assets: Res<GameAssets>,
    session: Res<Session>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    auras: Res<super::auras::Auras>,
) {
    use crate::input::bindings::{Binding, BindingPressed};

    for BindingPressed(binding) in pressed.read() {
        let Binding::CastShapeshiftForm(index) = binding else {
            continue;
        };
        let Some(form) = bar.form(usize::from(*index)) else {
            continue;
        };
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        let current = player.single().map(|me| me.shapeshift_form).unwrap_or(0);
        let on = is_active(form, current, &auras.player);
        if on {
            let cancellable = assets.display_tables().ok().is_none_or(|tables| {
                !tables.shapeshift().cannot_be_cancelled(form.form_id)
            });
            if cancellable {
                active.live.cancel_aura(form.spell_id);
            }
            // …and if it cannot be cancelled, nothing at all — see the note.
            continue;
        }
        // **No target**: a form spell says who it hits, and sending the current
        // selection is how `SPELL_FAILED_BAD_TARGETS` gets returned for a
        // self-buff — see [`vale_protocol::play::spells::CastTarget`].
        active
            .live
            .cast(form.spell_id, vale_protocol::play::spells::CastTarget::SelfImplicit);
    }
}

/// **Is this button's form on?** — the two questions the module comment
/// describes, chosen by whether the spell shapeshifts at all.
///
/// Takes the two facts rather than the resources holding them, so that the
/// interface's read and this module's own press ask the identical question:
/// `current_form` is `UNIT_FIELD_BYTES_1`'s form byte and `player_auras` is the
/// player's own aura list.
pub fn is_active(form: &ShapeshiftForm, current_form: u8, player_auras: &[super::auras::Aura]) -> bool {
    if form.form_id != 0 {
        return u32::from(current_form) == form.form_id;
    }
    // A paladin's aura or Stealth: the button is pressed in while its own aura
    // is on the player, which is a different lookup entirely.
    player_auras.iter().any(|aura| aura.spell == form.spell_id)
}

/// Let go of the last character's forms — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut bar: ResMut<ShapeshiftBar>,
) {
    if leaving.read().next().is_some() {
        *bar = ShapeshiftBar::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(id: u32, order: i32) -> (i32, u32, ShapeshiftForm) {
        (
            match order {
                -1 => i32::MAX,
                order => order,
            },
            id,
            ShapeshiftForm {
                spell_id: id,
                name: String::new(),
                texture: String::new(),
                active_texture: String::new(),
                form_id: 0,
            },
        )
    }

    /// **The sort is the whole difference between a correct bar and a wrong
    /// one**, and it is not a sort by spell id: a druid's five forms are learned
    /// in one order and drawn in another.
    ///
    /// Bear is 5487 and Cat is 768, so a list ordered by id draws Cat first;
    /// the column puts Bear at 0 and Cat at 2. And `-1` — Stealth's — sorts
    /// last however small the id is.
    #[test]
    fn the_bar_is_ordered_by_the_column_and_minus_one_sorts_last() {
        let mut forms = vec![
            form(768, 2),    // Cat
            form(1784, -1),  // Stealth
            form(5487, 0),   // Bear
            form(9634, 0),   // Dire Bear — the same button as Bear
            form(1066, 1),   // Aquatic
        ];
        forms.sort_by_key(|(order, id, _)| (*order, *id));
        let ids: Vec<u32> = forms.iter().map(|(_, id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![5487, 9634, 1066, 768, 1784],
            "Bear, Dire Bear, Aquatic, Cat, then the orderless one last"
        );
    }
}
