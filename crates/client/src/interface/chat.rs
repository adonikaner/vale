//! Chat lines from the session, raised as events for the game's own chat
//! frame, and lines the interface asks to say, sent to the session.
//!
//! [`poll`] reads each [`crate::world::incoming::ChatArrived`] and writes one
//! [`super::events::ChatMessageReceived`] per line, under the `CHAT_MSG_*`
//! name of the line's type. `ChatFrame.lua` then decides the wording, the
//! colour, the `[Name]` link and the window the line lands in. A second copy
//! of the first of those decisions in `ui::chat::render` was removed for that
//! reason.
//!
//! ## The event names come from `ChatTypeGroup`
//!
//! `ChatFrame.lua` opens with sixty `ChatTypeGroup["…"] = { "CHAT_MSG_…" }`
//! declarations. The strings on the right are the events a chat window
//! registers for, and the keys on the left are what `GetChatWindowMessages`
//! answers with. [`event_name`] is a transcription of the right-hand side,
//! not a set of names chosen here. [`super::events::FIRED`] checks it: every
//! name this maps to has to be in that list, or the client raises an event
//! the list does not declare.
//!
//! ## Who drains the queue
//!
//! [`vale_protocol::state::objects::ObjectManager::take_chat`] empties itself
//! for whoever asks first, so there can be exactly one caller in the process:
//! [`crate::world::incoming::drain_queues`], which writes a
//! [`crate::world::incoming::ChatArrived`] per line. [`poll`] reads that
//! message, and anything else that wants the lines reads
//! [`super::events::ChatMessageReceived`], the one-writer, many-readers
//! pattern [`super::events`] provides.
//!
//! ## Lines that arrive before the interface has loaded
//!
//! This client logs in and then loads the interface (175 files, about a
//! second); 1.12 loads it at startup and enters the world afterwards. A
//! `bevy` message that nothing reads expires, so every line the server sends
//! in that second would be dropped. `lua::host::load_bindings` re-announces
//! `PLAYER_ENTERING_WORLD` for the same reason, which works because that
//! event states something a reader can re-read.
//!
//! A chat line cannot be re-read, and the server sends the whole message of
//! the day inside that second: vmangos' `HandlePlayerLogin` splits
//! `sWorld.GetMotd()` on `@` and sends one `PSendSysMessage` per line,
//! immediately after `SMSG_LOGIN_VERIFY_WORLD` (`CharacterHandler.cpp:561`,
//! whose own comment notes 1.12 has no `SMSG_MOTD`). Without a hold, the MOTD
//! was decoded correctly and never shown ("Server MOTD does not show").
//!
//! [`poll`] therefore holds what arrives before the interface is up and
//! raises it in order once there is one. The hold is bounded by
//! [`BACKLOG_LIMIT`].
//!
//! ## Channel lines are numbered here
//!
//! `CHAT_MSG_CHANNEL` arrives with a name and the chat frame wants a number:
//! `[2. Trade - City]`, `ChatTypeInfo["CHANNEL2"]`, and a match against the
//! window's list by row id. [`poll`] fills the five channel arguments from
//! the board `lua::panels::channels` keeps, and [`send`] turns the number
//! `SendChatMessage(msg, "CHANNEL", lang, "2")` carries back into the name
//! the wire wants. A line on a channel the board does not hold is raised
//! with number 0, and `ChatFrame_OnEvent` declines it as the reference
//! would, as a channel the frame is not registered for.

use bevy::prelude::*;

use super::events::ChatMessageReceived;
use crate::world::incoming::ChatArrived;
use crate::world::session::Session;
use vale_protocol::play::chat::ChatType;

pub struct ChatPlugin;

impl Plugin for ChatPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChatArrived>().add_systems(
            Update,
            (poll.after(crate::world::incoming::drain_queues), send).in_set(super::GameSet),
        );
    }
}

/// Every chat type's default colour, re-exported from
/// `vale_assets::interface::chattype` (94 rows).
///
/// The table is in `assets` because three consumers read the same row: the
/// interface wants the name to raise `UPDATE_CHAT_COLOR` under, the combat
/// log's routing wants only the index, and `vale combatlog` wants both with
/// no renderer linked.
///
/// The colour must be raised: `ChatFrame.lua`'s
/// `ChatTypeInfo["SAY"] = { sticky = 1 }` declares the row and nothing else.
/// `info.r`, `info.g` and `info.b` are `nil` until the client raises
/// `UPDATE_CHAT_COLOR` for that type, and `AddMessage` with a nil colour draws
/// white, so a client that never raises it draws all chat in white.
pub use vale_assets::interface::chattype::TYPES as DEFAULT_COLOURS;

/// The event a line of each chat type is raised under.
///
/// Transcribed from `ChatFrame.lua`'s `ChatTypeGroup` table; see the module
/// comment. The one that is not a plain upper-casing is
/// [`ChatType::TextEmote`] -> `CHAT_MSG_TEXT_EMOTE`, which is a separate event
/// from `CHAT_MSG_EMOTE` and is in the same `SAY` group.
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

/// The chat type a `SendChatMessage` type word means: the inverse of
/// [`event_name`] for the eleven types a player can send.
///
/// The words are `ChatTypeInfo`'s keys, the table `ChatEdit_ParseText` looks
/// a `/s` up in and writes into `editBox.chatType`, so what arrives here is
/// always one of them. `None` for a type this client cannot put on the wire:
/// the four that need state it does not have (`BATTLEGROUND`, and the three
/// channel forms). Refusing them is required: a `/bg` sent as a SAY would be
/// broadcast to the zone.
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
        // The target is the channel's number; [`send`] resolves it.
        "CHANNEL" => ChatType::Channel,
        _ => return None,
    })
}

/// `arg6`, the speaker's flag: the wire's `tag` byte as the word the handler
/// looks `CHAT_FLAG_<word>` up by in `GlobalStrings.lua`.
///
/// The empty string is the ordinary case, not "unknown". The handler tests
/// `strlen(arg6) > 0` before looking anything up, so a code this client does
/// not recognise must come back empty rather than as a name that resolves to
/// nothing.
fn flag_of(tag: u8) -> &'static str {
    match tag {
        1 => "AFK",
        2 => "DND",
        3 => "GM",
        _ => "",
    }
}

/// A line the client wrote itself (a usage message, a script error, a local
/// note), raised as `CHAT_MSG_SYSTEM`.
///
/// The game already has a place for a line with no speaker that is coloured
/// like server output: `ChatFrame_OnEvent`'s `type == "SYSTEM"` branch, which
/// is three lines long. A client note therefore appears in the same frame, in
/// the same colour and through the same code as the server's own notes.
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

/// How many lines are held for an interface that has not loaded yet.
///
/// The interval is about a second and the server's login burst is the MOTD
/// plus a few notices, so this is two orders of magnitude of headroom. It is
/// a cap: if the interface never loads, the queue must not grow for the rest
/// of the session.
const BACKLOG_LIMIT: usize = 256;

/// Raise one event per line that arrived, or hold the lines while
/// no interface is loaded; the module comment explains why the message of
/// the day needs the hold.
fn poll(
    session: Res<Session>,
    status: Res<crate::world::status::WorldStatus>,
    time: Res<Time>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut arrived: MessageReader<ChatArrived>,
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
    let board = host.as_deref().map(crate::lua::panels::channels::held);
    let now = time.elapsed_secs();
    let taken: Vec<(String, vale_protocol::play::chat::ChatMessage)> = arrived
        .read()
        .map(|line| (line.who.clone(), line.message.clone()))
        .collect();
    // A speaker out of view has a guid and no name. The reference asks for
    // the name rather than printing the guid, since a channel is heard across
    // the whole zone. The line is held until `CMSG_NAME_QUERY` answers, and
    // printed with the placeholder after [`NAME_PATIENCE`] seconds. The names
    // are read under the world lock; the unnamed senders are queued on the
    // friends list's name query (`LiveSession::want_social_guid`) after the
    // lock is released.
    let mut resolved: Vec<(String, vale_protocol::play::chat::ChatMessage)> = Vec::new();
    let mut unnamed_senders = Vec::new();
    {
        let Ok(world) = active.live.world().lock() else {
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
                unnamed_senders.push(message.sender);
                unnamed.push((now, message));
            } else {
                resolved.push((who, message));
            }
        }
    }
    for sender in unnamed_senders {
        active.live.want_social_guid(sender);
    }
    let arrived = resolved.into_iter().filter_map(|(who, message)| {
        // A chat type the client does not know is dropped. There is no
        // `CHAT_MSG_` name to raise it under, and an invented name would match
        // no `RegisterEvent` in any file, so the line would reach no frame
        // either way.
        let kind = message.chat_type()?;
        let mut line = ChatMessageReceived {
            event: event_name(kind),
            text: message.text.clone(),
            author: who,
            flag: flag_of(message.tag),
            ..Default::default()
        };
        // The five channel arguments, from the board; see the module note.
        if let Some(name) = message.channel.as_deref() {
            let slot = board.as_ref().and_then(|held| {
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

/// Which of these lines may be raised now, and which are held for later.
///
/// Separate from [`poll`] because [`poll`] needs a socket and this function
/// holds the whole rule: what is held, in what order it comes back, and that
/// the hold is bounded. Lines already waiting come out first: the MOTD
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

/// Send the lines the interface asked to say.
///
/// This is here rather than in `lua/` because the socket is the session's and
/// a registered Lua closure cannot hold one. `SendChatMessage` records a
/// [`crate::lua::api::verbs::Said`] and this drains it, as a key's verb
/// records a `Binding` and `interface::action` acts on it.
///
/// A line typed with no session is dropped rather than queued. Queueing it
/// would send whatever was typed at the character screen into the world on
/// the next login.
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
                let channels = crate::lua::panels::channels::held(&host);
                let board = channels.borrow();
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
    use crate::interface::events::{FIRED, GameEvent};

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

    /// Every event name this module raises is listed in `FIRED`.
    ///
    /// `vale framexml` and the HUD count the interface's expectations against
    /// `FIRED`, so a `CHAT_MSG_*` missing from it would be reported as "no
    /// frame asked for this" when a frame did. The test also checks that the
    /// transcription from `ChatTypeGroup` is complete: a chat type added to the
    /// protocol with no arm here does not compile, and one with an arm that
    /// `FIRED` does not list fails here.
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
                "{name} is raised and is not in interface::events::FIRED"
            );
        }
        // No two chat types share a name, which would make one of them
        // undeliverable to a frame registered for the other.
        let mut names: Vec<&str> = EVERY_KIND.iter().map(|k| event_name(*k)).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two kinds share one event name");
    }

    /// Nine arguments, none of them nil, which is the shape
    /// `ChatFrame_OnEvent` is written against: it calls `strlen` on four of
    /// them before it has decided what type of line it is holding.
    #[test]
    fn a_say_carries_ten_arguments_and_no_holes() {
        use crate::interface::events::EventArg;
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

    /// The two directions agree: every chat type a player can send comes back
    /// from [`kind_of_word`] as the type whose `CHAT_MSG_*` name it is heard
    /// under. One table in `ChatFrame.lua` (`ChatTypeInfo`) defines both, so a
    /// disagreement here is a line sent as one type and echoed as another.
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
        // The types that need state this client does not have are refused
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

    /// The message of the day is kept through the second it takes to load the
    /// interface. vmangos sends it as `CHAT_MSG_SYSTEM` immediately after
    /// `SMSG_LOGIN_VERIFY_WORLD` (1.12 has no `SMSG_MOTD`), well before
    /// `Interface\FrameXML\` is loaded here, and a `bevy` message with no
    /// reader expires. The lines are held, then delivered in order, ahead of
    /// anything said since.
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

    /// The hold is bounded. An interface that never loads must not grow a
    /// queue for the rest of the session; a dropped line is recoverable and an
    /// unbounded queue is not.
    #[test]
    fn the_hold_stops_at_its_limit() {
        let mut backlog = Vec::new();
        let flood = (0..BACKLOG_LIMIT + 50).map(|n| line(&format!("line {n}")));
        assert!(route(false, &mut backlog, flood).is_empty());
        assert_eq!(backlog.len(), BACKLOG_LIMIT);
        assert_eq!(backlog[0].text, "line 0", "the oldest are the ones kept");
    }

    /// The flag is the word `GlobalStrings.lua` keys on, and anything else is
    /// empty rather than a name that resolves to nothing.
    #[test]
    fn the_speaker_flag_is_the_games_own_word() {
        assert_eq!(flag_of(0), "");
        assert_eq!(flag_of(1), "AFK");
        assert_eq!(flag_of(2), "DND");
        assert_eq!(flag_of(3), "GM");
        assert_eq!(flag_of(99), "");
    }
}
