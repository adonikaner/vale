//! **Text emotes**: `/dance` typed here goes out as `CMSG_TEXT_EMOTE`, and
//! everybody's `SMSG_TEXT_EMOTE` becomes the sentence and the voice line.
//!
//! The animation is not this module's: the server answers a text emote
//! with an `SMSG_EMOTE` of its own, which `world::entities` already plays.
//! What was missing was the *words* — `Corwin dances.`, `Bob waves at
//! you.` — and the shout a `/charge` makes, both of which the client
//! composes for itself out of `EmotesText.dbc`, `EmotesTextData.dbc` and
//! `EmotesTextSound.dbc` (`assets::tables::emotetext`, which states the
//! column rule and what was measured).
//!
//! ## The way out
//!
//! `ChatFrame.lua` turns `/dance` into `DoEmote("DANCE", rest)` through its
//! own `EMOTEn_CMDm` and `EMOTEn_TOKEN` globals, and the emote menu calls
//! it with a token alone. `DoEmote` is the one C function in that path:
//! the token names an `EmotesText` row, the row names an `Emotes` id, and
//! the packet carries both with the target's guid — the current selection,
//! or a player named after the command (`/wave Bob`) when that name is
//! known. A token the table does not have is dropped, which is what the
//! reference does with `/foo`.
//!
//! ## The way in
//!
//! The packet carries the speaker's guid and the *target's* name. The
//! speaker's name and gender come from the players table, asked for through
//! the same door the friends list uses when it does not have them
//! ([`NAME_PATIENCE`], as the channel notices); the sentence is chosen by
//! who is speaking to whom, and lands as `CHAT_MSG_TEXT_EMOTE` with the
//! sentence in `arg1`, which `ChatFrame_OnEvent` prints as it is. The
//! voice line is looked up by the speaker's race and gender and played
//! where they stand, through the mixer the greetings use; a speaker out of
//! view has no position and makes no sound.

use bevy::prelude::*;

use super::events::ChatMessageReceived;
use crate::sound::mixer::{Place, Voices};
use crate::world::session::{Session, WorldEntity, WorldStatus};
use vale_assets::tables::emotetext::{Speaker, Target};
use vale_protocol::play::chat::ChatType;
use vale_protocol::play::emotetext::TextEmote;
use vale_protocol::play::spells::PlayerEvent;

/// `SMSG_TEXT_EMOTE`, as a message for [`apply_answers`].
#[derive(Message, Debug, Clone)]
pub struct EmoteAnswer(pub Box<TextEmote>);

/// How long a line waits for the speaker's name.
const NAME_PATIENCE: f32 = 3.0;

pub struct EmoteTextPlugin;

impl Plugin for EmoteTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<EmoteAnswer>().add_systems(
            Update,
            (send_emotes, apply_answers).chain().in_set(super::GameSet),
        );
    }
}

/// `DoEmote(token, rest)` — see the module note.
fn send_emotes(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    selection: Res<super::target::Selection>,
) {
    let Some(mut host) = host else { return };
    let emoted = host.take_emoted();
    if emoted.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(texts) = tables.emote_texts() else {
        return;
    };
    for emote in emoted {
        let Some(row) = texts.by_token(&emote.token) else {
            info!("emote: {:?} is not a token in EmotesText.dbc", emote.token);
            continue;
        };
        // A name after the command, when it is somebody known; the
        // selection otherwise.
        let named = emote.target.as_deref().and_then(|name| {
            let world = active.live.world().lock().ok()?;
            world
                .players
                .iter()
                .find(|(_, info)| info.name.eq_ignore_ascii_case(name))
                .map(|(guid, _)| *guid)
        });
        let target = named.or(selection.guid).unwrap_or(0);
        active.live.text_emote(row.id, row.emote, target);
    }
}

/// A line held for the speaker's name.
struct Waiting {
    emote: TextEmote,
    since: f32,
}

#[allow(clippy::too_many_arguments)]
fn apply_answers(
    session: Res<Session>,
    status: Res<WorldStatus>,
    assets: Res<crate::assets::GameAssets>,
    time: Res<Time>,
    mut answers: MessageReader<EmoteAnswer>,
    mut waiting: Local<Vec<Waiting>>,
    mut chat: MessageWriter<ChatMessageReceived>,
    mut voices: Voices,
    // Where a speaker in view stands, for the voice line — and the
    // character's own race and gender, which no name query answers.
    placed: Query<(Entity, &WorldEntity, &Transform)>,
) {
    let Some(active) = session.active.as_ref() else {
        answers.clear();
        waiting.clear();
        return;
    };
    let now = time.elapsed_secs();
    let arrived: Vec<Waiting> = answers
        .read()
        .map(|EmoteAnswer(emote)| Waiting { emote: (**emote).clone(), since: now })
        .collect();
    let queue: Vec<Waiting> = std::mem::take(&mut *waiting).into_iter().chain(arrived).collect();
    if queue.is_empty() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(texts) = tables.emote_texts() else {
        return;
    };
    let me = status.character.clone();
    let Ok(mut world) = active.live.world().lock() else {
        return;
    };
    for item in queue {
        let patient = now - item.since < NAME_PATIENCE;
        let emote = &item.emote;
        // The speaker: the character, somebody named, or somebody still
        // being asked about.
        let (speaker, race_gender) = if world.player_guid == Some(emote.sender) {
            let own = placed
                .iter()
                .find(|(_, entity, _)| entity.is_self)
                .map(|(_, entity, _)| (entity.race_class.map_or(0, |(race, _)| u32::from(race)), u32::from(entity.gender.unwrap_or(0))));
            (Speaker::You, own.unwrap_or((0, 0)))
        } else {
            match world.players.get(&emote.sender) {
                Some(info) => (
                    Speaker::Other { name: info.name.clone(), female: info.gender == 1 },
                    (info.race, info.gender),
                ),
                None if patient => {
                    world.want_social_guid(emote.sender);
                    waiting.push(item);
                    continue;
                }
                None => (Speaker::Other { name: String::new(), female: false }, (0, 0)),
            }
        };
        let target = if emote.target.is_empty() {
            Target::Nobody
        } else if !me.is_empty() && emote.target.eq_ignore_ascii_case(&me) {
            Target::You
        } else if matches!(&speaker, Speaker::Other { name, .. } if name.eq_ignore_ascii_case(&emote.target)) {
            Target::Themselves
        } else {
            Target::Other(emote.target.clone())
        };
        if let Some(sentence) = texts.sentence(emote.text_emote, &speaker, &target) {
            chat.write(ChatMessageReceived {
                event: super::chat::event_name(ChatType::TextEmote),
                text: sentence,
                author: match &speaker {
                    Speaker::You => me.clone(),
                    Speaker::Other { name, .. } => name.clone(),
                },
                ..Default::default()
            });
        }
        // The voice line, on the speaker — a child of their entity, so it
        // goes where they go.
        let (race, gender) = race_gender;
        if let Some(sound) = texts.sound(emote.text_emote, race, gender) {
            if let Some((id, _, at)) = placed.iter().find(|(_, entity, _)| entity.guid == emote.sender) {
                let bank = assets.sounds();
                voices.play(&bank, sound, Place::On(id, at.translation));
            }
        }
    }
}

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<EmoteAnswer> {
    match event {
        PlayerEvent::TextEmote(emote) => Some(EmoteAnswer(emote.clone())),
        _ => None,
    }
}
