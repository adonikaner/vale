//! **What happens to *you*** — the sounds the client plays for the character
//! whose keyboard this is, rather than for a unit in the world.
//!
//! Two sounds today, and they are here for the same reason: **the interface
//! does not play either of them.** The whole of `Interface\FrameXML\` was
//! searched for `LEVELUPSOUND` and it appears in none of the files the `.toc`
//! loads; `QuestDetailAcceptButton_OnClick` is one line (`AcceptQuest()`) with
//! no `PlaySound` in it, while the cancel and the hand-in beside it each have
//! one. So in both cases the C client is the only thing that can be making the
//! noise. That is the same shape as the level-up *glow*, which is
//! `SpellVisualEffectName` row 21 and reached by no chain either — the two
//! halves of one event, neither of them anybody else's.
//!
//! `SoundEntries` carries both rows under their own names and the lookup is the
//! one [`super::interface`] already makes for `PlaySound`: **entry 124,
//! `LEVELUPSOUND`, `Sound\Spells\LevelUp.wav`** and **entry 890, `QUESTADDED`,
//! `Sound\Interface\iQuestActivate.wav`**. Flat rather than placed — neither is
//! an event in the world, both are events about the listener.
//!
//! ## Where the quest chime's name comes from
//!
//! Not from a guess and not from a later client: the 5875 client keeps a table
//! of **messages**, five fields a record, whose third field is the name of a
//! sound to play with the line and whose value is the literal string `"NONE"`
//! when there is none. The play is a string compare against `NONE` and then
//! the sound by name, *before* the text is composed — and record 137 is
//! `{"ERR_QUEST_ACCEPTED_S", 0, "QUESTADDED", …}`, raised from the quest-log
//! field callback with the quest's own title. Its neighbours are `ERR_QUEST_COMPLETE_S` with
//! `igQuestListComplete` (which `QuestFrame.lua` plays for itself) and the
//! `ERR_QUEST_FAILED_*` family with nothing.
//!
//! **One half of that record is deliberately not here**: the same call puts
//! `"Quest accepted: %s"` in the chat frame (branch 0 of the table's kind field,
//! with the record's own chat type in the fifth field). This client raises the
//! sound and not the line, because the line needs the quest's *title*, which is
//! a `CMSG_QUEST_QUERY` round trip away — and the reference honours that by
//! deferring the whole message until the record lands, raising it again from
//! inside the cache's own callback. So the chime here
//! is at worst a round trip *earlier* than the reference's, never absent.

use super::mixer::{Place, Voices};
use bevy::prelude::*;

/// The `SoundEntries` name, not its id. The id is 124 in 5875 and a name is
/// what survives a patch — the same argument [`super::interface`]'s
/// `PlaySound` rests on, and `vale sound LEVELUPSOUND` is the check.
const LEVEL_UP: &str = "LEVELUPSOUND";

/// …and the quest chime's, entry 890. `vale sound QUESTADDED`.
const QUEST_ADDED: &str = "QUESTADDED";

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (level_up, quest_accepted).in_set(super::SoundSet));
    }
}

/// One chime per `SMSG_LEVELUP_INFO`.
///
/// Read off the same message the interface's `PLAYER_LEVEL_UP` is raised from,
/// so a level gained while the interface is still loading is still heard.
fn level_up(
    mut gained: MessageReader<crate::interface::events::PlayerLevelUp>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    // Drained whether or not anything plays: an unread message would fire on
    // whatever frame this system next ran.
    let levels = gained.read().count();
    if levels == 0 {
        return;
    }
    let bank = game.sounds();
    let Some(entry) = bank.entry_named(LEVEL_UP) else {
        return;
    };
    // Once, however many levels arrived in one frame — two chimes on top of
    // each other is one chime and a phase artefact.
    voices.play(&bank, entry.id, Place::Flat);
}

/// One chime per quest that entered the log.
///
/// Read off [`crate::interface::quest::QuestAccepted`], which is the client's own
/// edge on `PLAYER_QUEST_LOG_*` — nothing on the wire announces an accept. Once
/// per frame however many arrived, for the same reason the level-up is: a
/// quest chain that hands you two at once is one sound in the reference too,
/// and two copies of a 1.3-second wav started on the same frame is a phase
/// artefact rather than emphasis.
fn quest_accepted(
    mut accepted: MessageReader<crate::interface::quest::QuestAccepted>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    // Drained whether or not anything plays; see [`level_up`].
    let quests = accepted.read().count();
    if quests == 0 {
        return;
    }
    let bank = game.sounds();
    let Some(entry) = bank.entry_named(QUEST_ADDED) else {
        return;
    };
    voices.play(&bank, entry.id, Place::Flat);
}
