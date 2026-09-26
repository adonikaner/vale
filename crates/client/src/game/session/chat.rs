//! **What was said, on its way to the game's own chat frame.**
//!
//! One system. It drains the session's chat queue and writes one
//! [`super::super::events::ChatMessageReceived`] per line, under the `CHAT_MSG_*` name
//! the kind arrives as — and from there it is `ChatFrame.lua`'s problem, which is
//! the point: the wording, the colour, the `[Name]` link and which window it
//! lands in are all decisions the shipped directory already makes, in Lua, and
//! this client had been making a second, private copy of the first of them in
//! `ui::chat::render`.
//!
//! ## The names are `ChatTypeGroup`'s, and that table is the authority
//!
//! `ChatFrame.lua` opens with sixty `ChatTypeGroup["…"] = { "CHAT_MSG_…" }`
//! declarations, and between them they are the whole contract: the strings on
//! the right are the events a chat window registers for, and the keys on the
//! left are what `GetChatWindowMessages` answers with. So [`event_name`] is a
//! transcription of the right-hand side rather than a set of names chosen here,
//! and the check that it *is* one is [`super::super::events::FIRED`] — every name this
//! maps to has to be in that list, or the client is raising an event it does not
//! admit to raising.
//!
//! ## Why the drain moved out of `ui/`
//!
//! [`vale_protocol::state::objects::ObjectManager::take_chat`] empties itself for
//! whoever asks first, so there can be exactly one caller in the process. It used
//! to be the egui pane, which meant the game's own chat frame could not have the
//! lines even in principle. It is here now, and the pane reads the *message*
//! alongside every other listener — the same "one write, N readers" the whole of
//! [`super::super::events`] exists for, applied to the one channel that had been left
//! out of it.
//!
//! ## The first lines of a session are said to an empty room
//!
//! This client logs in and *then* loads the interface — 175 files, about a
//! second — where 1.12 loads it at startup and enters the world afterwards. A
//! `bevy` message nothing reads expires, so every line the server sends in that
//! second was dropped: `lua::host::load_bindings` re-announces
//! `PLAYER_ENTERING_WORLD` for exactly this reason, and that works because an
//! event is a *statement about state a reader can go and re-read*.
//!
//! **A chat line is not.** Nobody can re-derive it, and the server says the
//! whole of the message of the day inside that window — vmangos'
//! `HandlePlayerLogin` splits `sWorld.GetMotd()` on `@` and sends one
//! `PSendSysMessage` per line, immediately after `SMSG_LOGIN_VERIFY_WORLD`
//! (`CharacterHandler.cpp:561`, whose own comment notes 1.12 has no
//! `SMSG_MOTD`). So the MOTD was reaching the client, being decoded correctly,
//! and going nowhere — reported as "Server MOTD does not show".
//!
//! [`poll`] therefore **holds** what arrives before the interface is up and
//! raises it in order once there is one. Held rather than dropped, and bounded:
//! see [`BACKLOG_LIMIT`].
//!
//! ## A channel line is numbered here
//!
//! `CHAT_MSG_CHANNEL` arrives with a name and the chat frame wants a number:
//! `[2. Trade - City]`, `ChatTypeInfo["CHANNEL2"]`, and a match against the
//! window's list by row id. [`poll`] fills the five channel arguments from
//! the board `lua::panels::channels` keeps, and [`send`] turns the number
//! `SendChatMessage(msg, "CHANNEL", lang, "2")` carries back into the name
//! the wire wants. A line on a channel the board does not hold is raised
//! with number 0, and `ChatFrame_OnEvent` declines it as the reference
//! would — a channel the frame is not registered for.

use bevy::prelude::*;

use super::super::events::ChatMessageReceived;
use crate::world::session::Session;
use vale_protocol::play::chat::ChatType;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (poll, send).in_set(super::super::GameSet));
    }
}

/// **Every chat type's default colour** — one line, because the table moved.
///
/// It used to be 94 rows here. It is `vale_assets::interface::chattype`
/// now, for the reason every other client-derived table is in `assets`:
/// three separate consumers want the same row. The
/// interface wants the *name* to raise `UPDATE_CHAT_COLOR` under, the combat
/// log's routing answers the *index* and nothing else, and `vale combatlog`
/// wants both with no renderer linked.
///
/// The colour half is what this client does with it and is unchanged:
/// `ChatFrame.lua`'s `ChatTypeInfo["SAY"] = { sticky = 1 }` declares the row
/// and nothing else — `info.r`, `info.g` and `info.b` are `nil` until the C
/// client raises `UPDATE_CHAT_COLOR` for that type, and `AddMessage` with a nil
/// colour draws white. So a client that never raises it puts the whole of the
/// chat on screen in white, which is how the absence was reported.
pub use vale_assets::interface::chattype::TYPES as DEFAULT_COLOURS;

/// **The event a kind of line arrives under.**
///
/// Transcribed from `ChatFrame.lua`'s `ChatTypeGroup` table; see the module
/// comment. The one that is not a straight upper-casing is
/// [`ChatType::TextEmote`] -> `CHAT_MSG_TEXT_EMOTE`, which is a *separate* event
/// from `CHAT_MSG_EMOTE` and lands in the same `SAY` group.
pub fn event_name(kind: ChatType) -> &'static str {
    match kind {
        ChatType::Say => "CHAT_MSG_SAY",
        ChatType::Party => "CHAT_MSG_PARTY",
        ChatType::Raid => "CHAT_MSG_RAID",
        ChatType::Guild => "CHAT_MSG_GUILD",
        ChatType::Officer => "CHAT_MSG_OFFICER",
        ChatType::Yell => "CHAT_MSG_YELL",
        ChatType::Whisper => "CHAT_MSG_WHISPER",
        ChatType::WhisperInform => "CHAT_MSG_WHISPER_INFORM",
        ChatType::Emote => "CHAT_MSG_EMOTE",
        ChatType::TextEmote => "CHAT_MSG_TEXT_EMOTE",
        ChatType::System => "CHAT_MSG_SYSTEM",
        ChatType::MonsterSay => "CHAT_MSG_MONSTER_SAY",
        ChatType::MonsterYell => "CHAT_MSG_MONSTER_YELL",
        ChatType::MonsterEmote => "CHAT_MSG_MONSTER_EMOTE",
        ChatType::MonsterWhisper => "CHAT_MSG_MONSTER_WHISPER",
        ChatType::Channel => "CHAT_MSG_CHANNEL",
        ChatType::ChannelJoin => "CHAT_MSG_CHANNEL_JOIN",
        ChatType::ChannelLeave => "CHAT_MSG_CHANNEL_LEAVE",
        ChatType::ChannelList => "CHAT_MSG_CHANNEL_LIST",
        ChatType::ChannelNotice => "CHAT_MSG_CHANNEL_NOTICE",
        ChatType::ChannelNoticeUser => "CHAT_MSG_CHANNEL_NOTICE_USER",
        ChatType::Afk => "CHAT_MSG_AFK",
        ChatType::Dnd => "CHAT_MSG_DND",
        ChatType::Ignored => "CHAT_MSG_IGNORED",
        ChatType::Skill => "CHAT_MSG_SKILL",
        ChatType::Loot => "CHAT_MSG_LOOT",
    }
}

/// **The kind a `SendChatMessage` type word means**, and the inverse of
/// [`event_name`] for the eleven of them a player can actually send.
///
/// The words are `ChatTypeInfo`'s keys — the same table `ChatEdit_ParseText`
/// looks a `/s` up in and the same one it writes into `editBox.chatType`, so
/// what arrives here is always one of them. `None` for a kind this client cannot
/// put on the wire, which is the honest answer for the four that need state it
/// does not have (`BATTLEGROUND`, and the three channel forms), and it is the
/// right way to fail: a `/bg` said as a SAY would be broadcast to the zone.
pub fn kind_of_word(word: &str) -> Option<ChatType> {
    Some(match word {
        "SAY" => ChatType::Say,
        "PARTY" => ChatType::Party,
        "RAID" => ChatType::Raid,
        "GUILD" => ChatType::Guild,
        "OFFICER" => ChatType::Officer,
        "YELL" => ChatType::Yell,
        "WHISPER" => ChatType::Whisper,
        "EMOTE" => ChatType::Emote,
        "AFK" => ChatType::Afk,
        "DND" => ChatType::Dnd,
        // The target is the channel's *number*; [`send`] resolves it.
        "CHANNEL" => ChatType::Channel,
        _ => return None,
    })
}

/// **`arg6`, the speaker's flag** — the wire's `tag` byte as the word the
/// handler looks `CHAT_FLAG_<word>` up by in `GlobalStrings.lua`.
///
/// The empty string is not "unknown", it is the ordinary case, and the handler
/// tests `strlen(arg6) > 0` before looking anything up — so a code this client
/// does not recognise must come back empty rather than as a name that resolves
/// to nothing.
fn flag_of(tag: u8) -> &'static str {
    match tag {
        1 => "AFK",
        2 => "DND",
        3 => "GM",
        _ => "",
    }
}

/// **A line the client wrote itself**, raised as `CHAT_MSG_SYSTEM`.
///
/// A usage message, a script error, a local note. These used to be pushed onto
/// a private log in the egui pane with a private colour, which was a second way
/// to put a sentence on the screen. They do not need one: the game already has
/// a place for a line with no speaker that is coloured like server output, and
/// `ChatFrame_OnEvent`'s `type == "SYSTEM"` branch is three lines long. So a
/// client note now arrives in the same frame, the same colour and the same code
/// as the server's own — which is what it was standing in for all along.
pub fn system_note(out: &mut MessageWriter<ChatMessageReceived>, text: impl Into<String>) {
    out.write(ChatMessageReceived {
        event: event_name(ChatType::System),
        text: text.into(),
        author: String::new(),
        flag: "",
        channel: String::new(),
        ..Default::default()
    });
}

/// **How many lines are held for an interface that has not loaded yet.**
///
/// The window is about a second and the server's own login burst is the MOTD
/// plus a handful of notices, so this is two orders of magnitude of headroom.
/// It is a cap rather than a promise: something that keeps this from ever
/// draining (an interface that will not load) must not grow a queue for the rest
/// of the session, which is the one way a hold is worse than a drop.
const BACKLOG_LIMIT: usize = 256;

/// Drain what arrived and raise one event per line.
///
/// **Or hold it, if there is nothing loaded to hear it** — see the module
/// comment, which is the whole of why the message of the day never showed.
fn poll(
    session: Res<Session>,
    status: Res<crate::world::session::WorldStatus>,
    time: Res<Time>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut backlog: Local<Vec<ChatMessageReceived>>,
    // Lines whose speaker is not in view and has no name yet — see
    // [`unnamed`].
    mut unnamed: Local<Vec<(f32, vale_protocol::play::chat::ChatMessage)>>,
    mut out: MessageWriter<ChatMessageReceived>,
) {
    let Some(active) = session.active.as_ref() else {
        backlog.clear();
        unnamed.clear();
        return;
    };
    let listening = host
        .as_deref()
        .and_then(crate::lua::host::LuaHost::directory)
        == Some(crate::lua::host::Directory::Frame);
    let board = host.as_deref().map(crate::lua::host::LuaHost::channels);
    let now = time.elapsed_secs();
    let taken = active.live.take_chat();
    // **A speaker out of view has a guid and no name**, and the reference
    // asks for the name rather than printing the guid: a channel is heard
    // across the whole zone. The line is held until `CMSG_NAME_QUERY`
    // answers, through the same door the friends list uses, and printed
    // with the placeholder after [`NAME_PATIENCE`] seconds.
    let mut resolved: Vec<(String, vale_protocol::play::chat::ChatMessage)> = Vec::new();
    {
        let Ok(mut world) = active.live.world().lock() else {
            return;
        };
        let me = status.character.clone();
        let held = std::mem::take(&mut *unnamed);
        for (since, message) in held {
            match speaker(&world, &message, &me) {
                Some(who) => resolved.push((who, message)),
                None if now - since < NAME_PATIENCE => unnamed.push((since, message)),
                None => resolved.push((world.chat_sender(&message), message)),
            }
        }
        for (who, message) in taken {
            if unnamed_speaker(&world, &message) {
                world.want_social_guid(message.sender);
                unnamed.push((now, message));
            } else {
                resolved.push((who, message));
            }
        }
    }
    let arrived = resolved.into_iter().filter_map(|(who, message)| {
        // **A kind the client does not know is dropped rather than guessed at.**
        // There is no `CHAT_MSG_` name to raise it under, and inventing one
        // means a string no `RegisterEvent` in any file can match — a line that
        // silently goes nowhere either way, minus the pretence that it did not.
        let kind = message.chat_type()?;
        let mut line = ChatMessageReceived {
            event: event_name(kind),
            text: message.text.clone(),
            author: who,
            flag: flag_of(message.tag),
            ..Default::default()
        };
        // The channel's five, off the board — see the module note.
        if let Some(name) = message.channel.as_deref() {
            let slot = board.and_then(|held| {
                let board = held.borrow();
                board
                    .slot_of(name)
                    .map(|(n, slot)| (n, slot.zone_id, slot.instance))
                    .or(Some((0, board.zone_id_of(name), 0)))
            });
            let (number, zone_id, instance) = slot.unwrap_or((0, 0, 0));
            line.channel = crate::lua::panels::channels::Channels::display(number, name);
            line.channel_name = name.to_string();
            line.zone_channel = zone_id;
            line.number = number;
            line.instance = instance;
        }
        Some(line)
    });
    for line in route(listening, &mut backlog, arrived) {
        out.write(line);
    }
}

/// How long a line waits for its speaker's name. One round trip.
const NAME_PATIENCE: f32 = 3.0;

/// Whether a line's speaker is a player the object manager has no name for:
/// a guid, no name on the packet (a creature's comes with it), not the
/// character, and not in the players table.
fn unnamed_speaker(
    world: &vale_protocol::state::objects::ObjectManager,
    message: &vale_protocol::play::chat::ChatMessage,
) -> bool {
    message.sender_name.is_none()
        && message.sender != 0
        && world.player_guid != Some(message.sender)
        && !world.players.contains_key(&message.sender)
}

/// The speaker's name once the world has it: the character's own for the
/// character, the players table's for anybody else.
fn speaker(
    world: &vale_protocol::state::objects::ObjectManager,
    message: &vale_protocol::play::chat::ChatMessage,
    me: &str,
) -> Option<String> {
    if world.player_guid == Some(message.sender) && !me.is_empty() {
        return Some(me.to_string());
    }
    world.players.get(&message.sender).map(|info| info.name.clone())
}

/// **Which of these lines may be said now**, and what is held for later.
///
/// Split out of [`poll`] because everything above it needs a socket and this is
/// the whole of the rule: what is held, in what order it comes back, and that
/// the hold is bounded. Anything already waiting comes out **first** — the MOTD
/// arrives before the first `/say` of a session and has to print before it.
/// **Which of these lines may be said now**, and what is held for later.
///
/// Split out of [`poll`] because everything above it needs a socket and this is
/// the whole of the rule: what is held, in what order it comes back, and that
/// the hold is bounded. Anything already waiting comes out **first** — the MOTD
/// arrives before the first `/say` of a session and has to print before it.
pub(super) fn route(
    listening: bool,
    backlog: &mut Vec<ChatMessageReceived>,
    arrived: impl IntoIterator<Item = ChatMessageReceived>,
) -> Vec<ChatMessageReceived> {
    if !listening {
        let room = BACKLOG_LIMIT.saturating_sub(backlog.len());
        backlog.extend(arrived.into_iter().take(room));
        return Vec::new();
    }
    if backlog.is_empty() {
        return arrived.into_iter().collect();
    }
    let mut out = std::mem::take(backlog);
    info!("chat: {} line(s) held for the interface, delivered", out.len());
    out.extend(arrived);
    out
}

/// **Say what the interface asked to say.**
///
/// The other direction, and the reason it is here rather than in `lua/`: the
/// socket is the session's, and a registered Lua closure cannot hold one — so
/// `SendChatMessage` records a [`crate::lua::api::verbs::Said`] and this drains it,
/// exactly as a key's verb records a `Binding` and `game::action` acts on it.
///
/// **A line typed with no session is dropped rather than queued.** There is
/// nowhere for it to go and no round trip to wait for; queueing it would send
/// whatever was typed at the character screen into the world on the next login.
fn send(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
) {
    let Some(mut host) = host else { return };
    let lines = host.take_said();
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for line in lines {
        // A channel line's target is the number the frame typed; the wire
        // wants the name, and a number the board does not hold is a line to
        // nowhere — dropped, as the reference drops `/7 hello` with no
        // seventh channel.
        let target = match line.kind {
            ChatType::Channel => {
                let board = host.channels().borrow();
                match line.target.as_deref().and_then(|key| board.slot_of(key)) {
                    Some((_, slot)) => Some(slot.name.clone()),
                    None => continue,
                }
            }
            _ => line.target,
        };
        active.live.say(line.kind, target, line.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::events::{FIRED, GameEvent};

    /// Every kind the protocol can decode.
    const EVERY_KIND: [ChatType; 26] = [
        ChatType::Say,
        ChatType::Party,
        ChatType::Raid,
        ChatType::Guild,
        ChatType::Officer,
        ChatType::Yell,
        ChatType::Whisper,
        ChatType::WhisperInform,
        ChatType::Emote,
        ChatType::TextEmote,
        ChatType::System,
        ChatType::MonsterSay,
        ChatType::MonsterYell,
        ChatType::MonsterEmote,
        ChatType::MonsterWhisper,
        ChatType::Channel,
        ChatType::ChannelJoin,
        ChatType::ChannelLeave,
        ChatType::ChannelList,
        ChatType::ChannelNotice,
        ChatType::ChannelNoticeUser,
        ChatType::Afk,
        ChatType::Dnd,
        ChatType::Ignored,
        ChatType::Skill,
        ChatType::Loot,
    ];

    /// **Every name this raises is a name the client admits to raising.**
    ///
    /// `FIRED` is what `vale framexml` and the HUD count the interface's
    /// expectations against, so a `CHAT_MSG_*` missing from it reads as "no
    /// frame asked for this" when the truth is the opposite. It is also the
    /// check that the transcription from `ChatTypeGroup` is complete: a kind
    /// added to the protocol with no arm here would not compile, and one with an
    /// arm nobody listed would fail right here.
    #[test]
    fn every_kind_maps_to_an_event_the_client_lists_as_fired() {
        for kind in EVERY_KIND {
            let name = event_name(kind);
            assert!(
                name.starts_with("CHAT_MSG_"),
                "{kind:?} maps to {name}, which is not a chat event"
            );
            assert!(
                FIRED.contains(&name),
                "{name} is raised and is not in game::events::FIRED"
            );
        }
        // …and no two kinds share a name, which would make one of them
        // undeliverable to a frame registered for the other.
        let mut names: Vec<&str> = EVERY_KIND.iter().map(|k| event_name(*k)).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two kinds share one event name");
    }

    /// **Nine arguments, none of them nil**, which is the shape
    /// `ChatFrame_OnEvent` is written against — it calls `strlen` on four of
    /// them before it has decided what kind of line it is holding.
    #[test]
    fn a_say_carries_ten_arguments_and_no_holes() {
        use crate::game::events::EventArg;
        let said = ChatMessageReceived {
            event: event_name(ChatType::Say),
            text: "hello".to_string(),
            author: "Bram".to_string(),
            flag: "",
            channel: String::new(),
            ..Default::default()
        };
        assert_eq!(said.name(), "CHAT_MSG_SAY");
        let args = said.args();
        assert_eq!(args.len(), 10, "arg1..arg10");
        assert_eq!(args[0], EventArg::Text("hello".to_string()));
        assert_eq!(args[1], EventArg::Text("Bram".to_string()));
        for (index, arg) in args.iter().enumerate() {
            match arg {
                EventArg::Text(_) | EventArg::Number(_) => {}
            }
            // arg7 and arg8 are the two the handler compares with `> 0`, and
            // arg10 the instance it compares the same way.
            if index == 6 || index == 7 || index == 9 {
                assert!(matches!(arg, EventArg::Number(_)), "arg{} is a number", index + 1);
            } else {
                assert!(matches!(arg, EventArg::Text(_)), "arg{} is a string", index + 1);
            }
        }
    }

    /// **The two directions agree**: every kind a player can send comes back
    /// from [`kind_of_word`] as the kind whose `CHAT_MSG_*` name it is heard
    /// under. One table in `ChatFrame.lua` (`ChatTypeInfo`) is the authority for
    /// both, so a disagreement here is a line said as one thing and echoed as
    /// another.
    #[test]
    fn a_kind_sent_is_the_kind_heard() {
        for (word, kind) in [
            ("SAY", ChatType::Say),
            ("YELL", ChatType::Yell),
            ("PARTY", ChatType::Party),
            ("RAID", ChatType::Raid),
            ("GUILD", ChatType::Guild),
            ("OFFICER", ChatType::Officer),
            ("WHISPER", ChatType::Whisper),
            ("EMOTE", ChatType::Emote),
            ("AFK", ChatType::Afk),
            ("DND", ChatType::Dnd),
        ] {
            assert_eq!(kind_of_word(word), Some(kind), "{word}");
            assert_eq!(
                event_name(kind),
                format!("CHAT_MSG_{word}"),
                "{word} is heard under a different name than it is sent under"
            );
        }
        // …and the kinds that need state this client does not have are refused
        // rather than downgraded to a say.
        assert_eq!(kind_of_word("BATTLEGROUND"), None);
        assert_eq!(kind_of_word("CHANNEL"), Some(ChatType::Channel));
        assert_eq!(kind_of_word("say"), None, "the words are the table's, upper case");
    }

    fn line(text: &str) -> ChatMessageReceived {
        ChatMessageReceived {
            event: event_name(ChatType::System),
            text: text.to_string(),
            author: String::new(),
            flag: "",
            channel: String::new(),
            ..Default::default()
        }
    }

    fn texts(lines: &[ChatMessageReceived]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    /// **The message of the day survives the second it takes to load the
    /// interface.** vmangos sends it as `CHAT_MSG_SYSTEM` immediately after
    /// `SMSG_LOGIN_VERIFY_WORLD` — there is no `SMSG_MOTD` in 1.12 — which is
    /// well before `Interface\FrameXML\` exists here, and a `bevy` message with
    /// no reader expires. Held, then delivered in order, ahead of anything said
    /// since.
    #[test]
    fn a_line_that_arrives_before_the_interface_is_held_and_then_said() {
        let mut backlog = Vec::new();
        assert!(
            route(false, &mut backlog, [line("Welcome!"), line("Be nice.")]).is_empty(),
            "nothing is raised at an interface that cannot hear it"
        );
        assert_eq!(backlog.len(), 2);

        let said = route(true, &mut backlog, [line("Bram says hello")]);
        assert_eq!(
            texts(&said),
            ["Welcome!", "Be nice.", "Bram says hello"],
            "the held lines come first and keep their order"
        );
        assert!(backlog.is_empty(), "and are not said twice");
        assert_eq!(texts(&route(true, &mut backlog, [line("later")])), ["later"]);
    }

    /// **The hold is bounded.** An interface that never loads must not grow a
    /// queue for the rest of the session; a drop is recoverable and a leak is
    /// not.
    #[test]
    fn the_hold_stops_at_its_limit() {
        let mut backlog = Vec::new();
        let flood = (0..BACKLOG_LIMIT + 50).map(|n| line(&format!("line {n}")));
        assert!(route(false, &mut backlog, flood).is_empty());
        assert_eq!(backlog.len(), BACKLOG_LIMIT);
        assert_eq!(backlog[0].text, "line 0", "the oldest are the ones kept");
    }

    /// The flag is the word `GlobalStrings.lua` keys on, and anything else is
    /// **empty** rather than a name that resolves to nothing.
    #[test]
    fn the_speaker_flag_is_the_games_own_word() {
        assert_eq!(flag_of(0), "");
        assert_eq!(flag_of(1), "AFK");
        assert_eq!(flag_of(2), "DND");
        assert_eq!(flag_of(3), "GM");
        assert_eq!(flag_of(99), "");
    }
}
