//! **The client's half of the character-creation screen** — one packet, one
//! plinth, and the tables the screen chooses out of.
//!
//! ```text
//! assets::charcreate      what a character MAY be — races, classes, five axes
//!   -> lua::charcreate     …and which of them the player has picked
//!   -> here                …handed the socket, and stood on the plinth
//! ```
//!
//! This is the same three-part shape every other subject in this directory has,
//! with one difference worth stating: **the state is not here.** It is
//! [`crate::lua::panels::charcreate::Board`], inside the Lua host, because every read on
//! that screen has to answer inside the handler that wrote it — see that
//! module's own note. What is here is the two things the interface cannot do for
//! itself: hand it the tables, and turn Accept into a packet.
//!
//! ## Why Accept does not go on the task pool
//!
//! Every other round trip this client makes at a glue screen — the logon, the
//! re-enumeration after a logout — moves the socket onto
//! [`bevy::tasks::AsyncComputeTaskPool`] and reports `Screen::Connecting` while
//! it runs. This one cannot, because `Screen` is **derived from what exists**:
//! moving the handshake out would make [`Screen::Login`] true for as long as the
//! task ran, and `SET_GLUE_SCREEN("login")` would take the create screen down
//! and lose the name that was typed into it. See
//! [`crate::world::session::Session::create_character`], which is the blocking
//! half and says what that costs.
//!
//! ## What a success and a failure each do to the screen
//!
//! ```text
//! created  -> SET_GLUE_SCREEN("charselect") + CHARACTER_LIST_UPDATE
//!             + UPDATE_SELECTED_CHARACTER(the new last row)
//! refused  -> Session::error, which game::glue::watch_dialog turns into
//!             the game's own OKAY box over the screen you are still on
//! ```
//!
//! Both are the reference's: 1.12 leaves you on the create screen with a dialog
//! when the server says no, and goes back to the character list when it says
//! yes. Neither is `CharacterCreate.lua`'s own doing — that file registers **no
//! events at all**, so both edges are the client's.

use bevy::prelude::*;

use crate::lua::panels::charcreate::CreateRequest;
use crate::world::session::Session;

/// The frame name `SetGlueScreen("charcreate")` shows, and the one this module
/// tests the current screen against — `GlueScreenInfo["charcreate"]` in
/// `GlueParent.lua`.
pub const CREATE_SCREEN: &str = "charcreate";

pub struct CharCreatePlugin;

impl Plugin for CharCreatePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (supply_tables, apply, stand_on_plinth)
                .chain()
                .in_set(super::super::GameSet),
        );
    }
}

/// **Hand the Lua host the tables the create screen chooses out of**, once per
/// interface.
///
/// Per *interface* rather than once at startup because
/// `crate::lua::host::unload_interface` replaces the whole host — board included
/// — every time the directory swaps, and a client that logs out and back to the
/// character screen would otherwise find a create screen with no races on it.
/// The parse behind [`crate::assets::GameAssets::char_create`] is cached, so the
/// second supply is a pointer copy.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(mut host) = host else { return };
    // Only while the glue is the loaded directory: `Interface\FrameXML\` calls
    // none of these names, and opening the archives to answer a question nobody
    // is asking is what the lazy banks exist to avoid.
    if host.directory() != Some(crate::lua::host::Directory::Glue) || host.char_create_ready() {
        return;
    }
    host.set_char_create_tables(assets.char_create());
}

/// **Do what Accept asked**, which is one packet and one round trip.
fn apply(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut session: ResMut<Session>,
    mut state: ResMut<super::glue::GlueState>,
    mut screen: MessageWriter<super::super::events::SetGlueScreen>,
    mut list: MessageWriter<super::super::events::CharacterListUpdate>,
    mut selected: MessageWriter<super::super::events::UpdateSelectedCharacter>,
) {
    let Some(mut host) = host else { return };
    for request in host.take_create_requests() {
        let CreateRequest::Create(name) = request;
        let choice = host.char_create_choice();
        // **Cleared before the attempt**, and that is not tidiness:
        // `game::glue::watch_dialog` raises its box on a *change* of
        // `Session::error`, so a second Accept refused the same way as the first
        // would show nothing at all.
        session.error = None;
        match session.create_character(
            &name,
            choice.race,
            choice.class,
            choice.gender,
            choice.appearance,
        ) {
            Ok(()) => {
                // **Back to the list, with the new character on it.** The list
                // was re-read by the create itself — `SMSG_CHAR_CREATE`'s whole
                // body is one byte, so there is nothing else it could be built
                // from — and the new character is the last row, which is the
                // order the server answers `CMSG_CHAR_ENUM` in.
                let count = session
                    .selection
                    .as_ref()
                    .map_or(0, |held| held.characters.len());
                state.selected = count;
                screen.write(super::super::events::SetGlueScreen("charselect".into()));
                list.write(super::super::events::CharacterListUpdate);
                selected.write(super::super::events::UpdateSelectedCharacter(state.selected));
                info!("created {name}");
            }
            // **The screen stays where it is** and the dialog says why — see the
            // module comment. Recorded rather than shown from here, because the
            // one place a refusal becomes a box is `watch_dialog`.
            Err(failure) => {
                warn!("character creation refused: {}", failure.detail);
                session.error = Some(failure);
            }
        }
    }
}

/// **Put the character being made on the create screen's own plinth.**
///
/// Written into [`super::glue::GlueState::create_plinth`] rather than over
/// [`super::glue::GlueState::plinth`], which is character select's, because the
/// two screens are up at different times and *both* records have to survive the
/// switch: going back from create to select must show the highlighted character
/// again without re-deriving it, and coming back to create must not show
/// whoever was highlighted. `crate::render::glue` picks between them by which
/// `<Model>` frame the scene is on, which is the same question it already asks
/// to decide which attachment point to stand somebody at.
///
/// **And it wears its class's starting outfit**, as in the reference: it
/// clears the twelve visible slots and both hands and then equips the
/// `CharStartOutfit.dbc` row for this exact `(race, class, gender)`. So the
/// create screen is a warrior in a Recruit's shirt holding a Worn Shortsword and
/// not a character in its underwear — and the outfit changes under the race,
/// class and gender buttons, which is why this is derived every frame rather
/// than built once.
///
/// The split into worn and held is [`vale_assets::tables::charcreate::OutfitPiece`]'s
/// and carries the reference's own omission with it: **no ranged weapon**, since
/// the clear touches slots 15 and 16 and not 17.
fn stand_on_plinth(
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut state: ResMut<super::glue::GlueState>,
) {
    let Some(host) = host else { return };
    // **Only while that screen is up.** Off it, the record is dropped so the
    // renderer cannot draw a half-made character behind something else — and so
    // that re-opening the screen rebuilds rather than showing the last body for
    // a frame.
    if state.screen != CREATE_SCREEN {
        if state.create_plinth.is_some() {
            state.create_plinth = None;
        }
        return;
    }
    let choice = host.char_create_choice();
    let mut weapons: [vale_assets::tables::item::Weapon; 3] = Default::default();
    let mut equipment = Vec::new();
    for piece in choice.outfit.iter().filter(|p| !p.is_ranged()) {
        match piece.hand() {
            // The same `(display id, inventory type)` pair `SMSG_CHAR_ENUM`
            // carries per slot, through the same rule character select's two
            // hands go through — see `Weapon::from_char_enum`, whose one
            // question is `inventoryType == 14`.
            Some(hand) => {
                weapons[hand] = vale_assets::tables::item::Weapon::from_char_enum(
                    piece.display_id,
                    piece.inventory_type,
                );
            }
            None => equipment.push((piece.display_id, u32::from(piece.inventory_type))),
        }
    }
    let wanted = super::glue::Plinth {
        appearance: choice.look,
        equipment,
        weapons,
    };
    // On a change only, on the same terms as `glue::stand_on_plinth`: the
    // renderer rebuilds a dressed character whenever this differs, and writing
    // it every frame would respawn one sixty times a second.
    if state.create_plinth.as_ref() != Some(&wanted) {
        state.create_plinth = Some(wanted);
    }
}
