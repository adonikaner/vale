//! The interface options panel's two checkboxes that are not CVars: show helm
//! and show cloak.
//!
//! `UIOptionsFrameCheckButtons` declares both with a `func`/`setFunc` pair
//! instead of a `cvar`:
//!
//! ```text
//! ShowingHelm()     1 when PLAYER_FLAGS_HIDE_HELM is clear, else nil
//! ShowingCloak()    …and PLAYER_FLAGS_HIDE_CLOAK
//! ShowHelm(value)   CMSG_TOGGLE_HELM, when the flag differs from value
//! ShowCloak(value)  CMSG_TOGGLE_CLOAK, likewise
//! ```
//!
//! The flags are the character's own, stored by the server and sent in
//! `PLAYER_FLAGS`. The two packets have no body and the server flips the bit
//! (vmangos' `HandleShowingHelmOpcode`), so a toggle is sent only when the
//! state the checkbox asks for differs from the flag. `UIOptionsFrame_Save`
//! calls both setters on every press of Okay, changed or not, and sending
//! unconditionally would flip each option on every Okay.
//!
//! What a flag changes is drawn by the world's own dressing:
//! `vale_assets::look::dress::worn_is_shown` leaves the piece out of the
//! entity's equipment list, the list changes, and the model is rebuilt as for
//! any change of gear. The reads are [`crate::lua::panels::uioptions`].

use bevy::prelude::*;

use crate::input::bindings::{Binding, BindingPressed};
use crate::world::session::{Session, WorldEntity};
use vale_assets::look::dress::{HIDE_CLOAK, HIDE_HELM};

pub struct UiOptionsPlugin;

impl Plugin for UiOptionsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            press
                .after(crate::input::bindings::BindingSet)
                .in_set(super::GameSet),
        );
    }
}

/// Which toggle, if any, a `ShowHelm`/`ShowCloak` call needs, given the
/// character's `PLAYER_FLAGS`. `Some(true)` is the helm's packet and
/// `Some(false)` the cloak's.
pub fn toggle_for(binding: &Binding, player_flags: u32) -> Option<bool> {
    let (helm, show) = match binding {
        Binding::ShowHelm(show) => (true, *show),
        Binding::ShowCloak(show) => (false, *show),
        _ => return None,
    };
    let bit = if helm { HIDE_HELM } else { HIDE_CLOAK };
    let shown = player_flags & bit == 0;
    (shown != show).then_some(helm)
}

/// `ShowHelm(value)` and `ShowCloak(value)` on the wire.
fn press(
    mut pressed: MessageReader<BindingPressed>,
    units: Query<&WorldEntity>,
    session: Res<Session>,
) {
    for BindingPressed(binding) in pressed.read() {
        let Some(me) = units.iter().find(|unit| unit.is_self) else {
            continue;
        };
        let Some(helm) = toggle_for(binding, me.player_flags) else {
            continue;
        };
        if let Some(active) = session.active.as_ref() {
            active.live.toggle_worn(helm);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A toggle goes out only when the flag differs from the state asked for,
    /// so an Okay that leaves a checkbox as it was sends nothing.
    #[test]
    fn a_toggle_is_sent_only_when_the_flag_differs() {
        assert_eq!(toggle_for(&Binding::ShowHelm(true), 0), None);
        assert_eq!(toggle_for(&Binding::ShowHelm(false), 0), Some(true));
        assert_eq!(toggle_for(&Binding::ShowHelm(true), HIDE_HELM), Some(true));
        assert_eq!(toggle_for(&Binding::ShowHelm(false), HIDE_HELM), None);
        assert_eq!(toggle_for(&Binding::ShowCloak(false), HIDE_HELM), Some(false));
        assert_eq!(toggle_for(&Binding::ShowCloak(true), HIDE_HELM), None);
        assert_eq!(toggle_for(&Binding::ConfirmSummon, 0), None);
    }
}
