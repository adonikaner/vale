//! Chat channels, the session's side: the file the channel list is filled
//! from, the packets that update it, the zone channels joined on a zone
//! change, and the lines sent to the chat frame.
//!
//! The board — the ten slots and the mask — is
//! [`crate::lua::panels::channels`], held by the Lua host so the twenty-five
//! globals can answer during a call. This module is everything else, in the
//! same split `social` uses: [`load_cache`] reads `chat-cache.txt`,
//! [`supply_tables`] hands the board the table, [`auto_join`] keeps the zone
//! channels current and rejoins the custom ones, [`apply_answers`] turns a
//! notice into a `CHAT_MSG_CHANNEL_*` event, [`list_locally`] answers
//! `/chatlist` with no name, [`send_verbs`] drains what the interface
//! queued, and [`save_on_logout`] and [`save_on_exit`] write the two lines
//! the file keeps about channels.
//!
//! ## The zone channels are the client's to join
//!
//! vmangos sends nothing about channels until asked. The reference joins
//! `General - <zone>`, `Trade - City` and `LocalDefense - <zone>` itself on
//! entering the world and again on every zone change — `CMSG_JOIN_CHANNEL`
//! with the name it composed from `ChatChannels.dbc` and `AreaTable.dbc`
//! (see `assets::tables::channels`) — and leaves the old zone's on the way.
//! [`auto_join`] does exactly that, on the area `game::place::worldmap`
//! keeps, and diffs the wanted set against the slots so a zone change is a
//! leave and a join per channel that changed and nothing for the rest. The
//! guild recruitment channel joins the same way in a city while the
//! character has no guild (`PLAYER_GUILDID` zero) and the file's option is
//! `AUTO`, which is every file measured.
//!
//! To the interface, a zone change is not a leave followed by a join. The server
//! answers the pair with `YOU_LEFT` then `YOU_JOINED`, and the reference
//! shows one line, `Changed Channel: [1. General - Dun Morogh]` — the same
//! notice code as `YOU_JOINED`, so the client decides. The slot is marked
//! switching before the leave goes out, the `YOU_LEFT` on a switching slot
//! raises nothing and keeps the slot, and the `YOU_JOINED` that replaces a
//! slot of the same row is raised as `YOU_CHANGED`. Without that the chat
//! frame would drop the channel from its list on the `YOU_LEFT` and never
//! show General again.
//!
//! ## The first notices arrive before there is a chat frame
//!
//! The joins go out the moment the world is entered, and the interface
//! loads about a second later; a `CHAT_MSG_CHANNEL_NOTICE` raised into an
//! interface that is not there yet expires, which is why the first version
//! of this showed no `Joined Channel` lines at all. The lines are held on
//! the same rule `chat::route` holds the server's own login burst, and
//! delivered in order once the frame directory is up.
//!
//! ## A notice about a person carries a guid
//!
//! The interface wants a name in `arg2`, and a channel member need not be
//! anywhere near: the name comes from the object manager's players table
//! when it has it, and is asked for through the same door the friends list
//! uses (`want_social_guid`, which puts the guid on the next
//! `CMSG_NAME_QUERY` round) when it does not. A notice waiting on a name is
//! held for [`NAME_PATIENCE`] seconds and then raised with the name blank
//! rather than dropped.
//!
//! ## What the file keeps, and what is written back
//!
//! `chat-cache.txt` (`assets::interface::chatcache`) is read at login for
//! the joined mask, the custom channels to rejoin, the recruitment option
//! and window 1's lists. On the way out the top-level `CHANNELS` block and
//! `ZONECHANNELS` line are rewritten in place — the custom channels held
//! and the mask — and nothing else in the file is touched, because the
//! colours and the window blocks are the reference's own and unread here.
//! A character with no file gets none; the file is the reference's to
//! create.

use std::path::PathBuf;

use bevy::app::AppExit;
use bevy::prelude::*;

use super::super::events::{ChatMessageReceived, PlayerLeavingWorld};
use crate::lua::panels::channels::Channels;
use crate::world::session::{ClientConfig, Session, WorldStatus};
use vale_assets::interface::chatcache;
use vale_protocol::play::channels::{mode_change_word, ChannelList, ChannelNotify, Notice, NoticeTail};
use vale_protocol::play::chat::ChatType;
use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::socket::session::ChannelVerb;

/// The two packets, as messages for [`apply_answers`].
#[derive(Message, Debug, Clone)]
pub enum ChannelAnswer {
    Notify(Box<ChannelNotify>),
    List(Box<ChannelList>),
}

/// How long a notice waits for a stranger's name before it is raised with
/// the name blank. A name query is one round trip.
const NAME_PATIENCE: f32 = 3.0;

/// The character's `chat-cache.txt`, located at login; see the module doc.
/// `text` is the file as read, which the write-back edits in place.
#[derive(Resource, Default)]
pub struct ChatCacheFile {
    pub path: Option<PathBuf>,
    text: Option<String>,
    loaded: bool,
}

pub struct ChannelsPlugin;

impl Plugin for ChannelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChannelAnswer>()
            .init_resource::<ChatCacheFile>()
            .add_systems(
                Update,
                (
                    load_cache,
                    supply_tables,
                    auto_join,
                    apply_answers,
                    list_locally,
                    send_verbs,
                    save_on_logout,
                    save_on_exit,
                )
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// Read the character's file once per login and fill the channel list from
/// it. The path needs the account, the realm and the character, which the
/// session has once it is in the world. The interpreter is rebuilt at logout,
/// so the list starts empty each time and the file is read again.
fn load_cache(
    session: Res<Session>,
    world: Res<WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut files: ResMut<ChatCacheFile>,
) {
    if leaving.read().next().is_some() {
        files.loaded = false;
        files.text = None;
        files.path = None;
    }
    if files.loaded || !world.in_world {
        return;
    }
    let Some(host) = host else { return };
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let account = config.0.account_for([Some(active.account.as_str())]);
    files.path = chatcache::path(&account, &active.realm, &world.character).map(|path| config.0.path(path));
    files.text = files.path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    let mut board = host.channels().borrow_mut();
    match &files.text {
        Some(text) => {
            let cache = chatcache::parse(text);
            seed(&mut board, &cache);
            info!(
                "chat channels: {} -> mask {}, {} custom, recruitment {}",
                files.path.as_ref().map_or(String::new(), |p| p.display().to_string()),
                cache.zone_channels,
                cache.channels.len(),
                cache.guild_recruitment
            );
        }
        None => info!(
            "chat channels: no chat-cache.txt for {} — the table's defaults",
            world.character
        ),
    }
    files.loaded = true;
}

/// What the file says, onto the board. A zero mask is unset — see the
/// reader's note — and leaves the table's default in place.
fn seed(board: &mut Channels, cache: &chatcache::ChatCache) {
    if cache.zone_channels != 0 {
        board.joined_mask = Some(cache.zone_channels);
    }
    board.window_mask = cache.window(1).map(|window| window.zone_channels);
    board.rejoin = cache.channels.clone();
    board.rejoined = false;
    board.recruit_automatically = cache.recruits_automatically();
}

/// The two tables the board composes names from, once.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if host.channels().borrow().table.is_some() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let mut board = host.channels().borrow_mut();
    board.table = tables.chat_channels().cloned().map(std::sync::Arc::new);
    board.areas = tables.areas().cloned().map(std::sync::Arc::new);
}

/// Keep the zone channels current, and rejoin the custom channels once; see
/// the module doc.
fn auto_join(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    status: Res<WorldStatus>,
    files: Res<ChatCacheFile>,
    place: Option<Res<super::super::place::worldmap::WorldMapState>>,
    mut last_area: Local<Option<u32>>,
) {
    let Some(host) = host else {
        *last_area = None;
        return;
    };
    let Some(active) = session.active.as_ref() else {
        *last_area = None;
        return;
    };
    let Some(place) = place else { return };
    // The file first, so a character who left General is not rejoined to it
    // in the frame before the file is read.
    if !status.in_world || !files.loaded || place.area == 0 {
        return;
    }
    let mut board = host.channels().borrow_mut();
    if board.table.is_none() {
        return;
    }
    // The guild, off the character's own field — zero for none — read
    // every time the area moves, which is as often as it can matter.
    let in_guild = active
        .live
        .world()
        .lock()
        .ok()
        .and_then(|world| {
            world
                .player()
                .and_then(|me| me.field(vale_protocol::state::fields::player::GUILDID))
        })
        .is_some_and(|guild| guild != 0);
    if *last_area == Some(place.area) && board.in_guild == in_guild {
        return;
    }
    board.in_guild = in_guild;
    *last_area = Some(place.area);
    let wanted = board.wanted(place.area);
    let held = board.zone_slots();
    let mut verbs = zone_change(&mut board, &held, &wanted);
    if !board.rejoined {
        board.rejoined = true;
        verbs.extend(board.rejoin.iter().map(|name| ChannelVerb::Join {
            name: name.clone(),
            password: String::new(),
        }));
    }
    drop(board);
    for verb in verbs {
        active.live.channel(verb);
    }
}

/// The leaves and joins one zone change costs, given what is held and what
/// is wanted, in the order they go out. A row whose name changed is marked
/// switching on the board so its `YOU_LEFT` is silent.
fn zone_change(
    board: &mut Channels,
    held: &[(u32, String)],
    wanted: &[vale_assets::tables::channels::ZoneChannel],
) -> Vec<ChannelVerb> {
    let mut verbs = Vec::new();
    for (id, old) in held {
        match wanted.iter().find(|want| want.id == *id) {
            Some(want) if want.name.eq_ignore_ascii_case(old) => {}
            Some(want) => {
                board.switching(*id);
                verbs.push(ChannelVerb::Leave(old.clone()));
                verbs.push(ChannelVerb::Join { name: want.name.clone(), password: String::new() });
            }
            None => verbs.push(ChannelVerb::Leave(old.clone())),
        }
    }
    for want in wanted {
        if !held.iter().any(|(id, _)| *id == want.id) {
            verbs.push(ChannelVerb::Join { name: want.name.clone(), password: String::new() });
        }
    }
    verbs
}

/// A notice held for a name — see the module note.
struct Waiting {
    answer: ChannelAnswer,
    since: f32,
}

fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<Session>,
    time: Res<Time>,
    mut answers: MessageReader<ChannelAnswer>,
    mut waiting: Local<Vec<Waiting>>,
    // The lines raised before the chat frame exists — see the module note.
    mut backlog: Local<Vec<ChatMessageReceived>>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    let Some(host) = host else {
        answers.clear();
        waiting.clear();
        backlog.clear();
        return;
    };
    let listening = host.directory() == Some(crate::lua::host::Directory::Frame);
    let now = time.elapsed_secs();
    let mut board = host.channels().borrow_mut();
    let arrived: Vec<Waiting> = answers
        .read()
        .map(|answer| Waiting { answer: answer.clone(), since: now })
        .collect();
    let queue: Vec<Waiting> = std::mem::take(&mut *waiting).into_iter().chain(arrived).collect();
    // The names, through the world lock — taken once for the whole batch.
    let world = session.active.as_ref().map(|active| active.live.world());
    let mut names = Names { world: world.map(|world| &**world), asked: Vec::new() };
    let mut lines = Vec::new();
    for item in queue {
        let patient = now - item.since < NAME_PATIENCE;
        let decided = match &item.answer {
            ChannelAnswer::Notify(notify) => notice_lines(&mut board, notify, &mut names, patient),
            ChannelAnswer::List(list) => list_lines(&board, list, &mut names, patient),
        };
        match decided {
            Some(decided) => lines.extend(decided),
            None => waiting.push(item),
        }
    }
    names.ask();
    for line in super::chat::route(listening, &mut backlog, lines) {
        chat.write(line);
    }
}

/// The names of the people a notice is about, and the guids it had to ask
/// for.
struct Names<'a> {
    world: Option<&'a std::sync::Mutex<vale_protocol::state::objects::ObjectManager>>,
    asked: Vec<u64>,
}

impl Names<'_> {
    /// `Some(name)` when known; `None` after noting the guid to ask for. A
    /// guid of zero is nobody, and nobody's name is the empty string.
    fn of(&mut self, guid: u64) -> Option<String> {
        if guid == 0 {
            return Some(String::new());
        }
        let known = self
            .world
            .and_then(|world| world.lock().ok())
            .and_then(|world| world.players.get(&guid).map(|info| info.name.clone()));
        if known.is_none() {
            self.asked.push(guid);
        }
        known
    }

    fn ask(&mut self) {
        if self.asked.is_empty() {
            return;
        }
        if let Some(Ok(mut world)) = self.world.map(|world| world.lock()) {
            for guid in self.asked.drain(..) {
                world.want_social_guid(guid);
            }
        }
    }
}

/// A channel line with the channel's own five arguments filled from the
/// board: `arg4` the numbered display, `arg7` the row, `arg8` the number,
/// `arg9` the bare name, `arg10` the instance.
fn channel_line(board: &Channels, event: &'static str, name: &str) -> ChatMessageReceived {
    let (number, zone_id, instance) = board
        .slot_of(name)
        .map_or((0, board.zone_id_of(name), 0), |(n, slot)| (n, slot.zone_id, slot.instance));
    ChatMessageReceived {
        event,
        channel: Channels::display(number, name),
        channel_name: name.to_string(),
        zone_channel: zone_id,
        number,
        instance,
        ..Default::default()
    }
}

/// The name a tail's guid resolves to, or `None` while the answer is worth
/// waiting for. Blank once patience has run out.
fn name_or_wait(names: &mut Names, guid: u64, patient: bool) -> Option<String> {
    match names.of(guid) {
        Some(name) => Some(name),
        None if patient => None,
        None => Some(String::new()),
    }
}

/// The lines one notice becomes: none or one, or `None` while it waits for a
/// name. See the module doc for each rule.
fn notice_lines(
    board: &mut Channels,
    notify: &ChannelNotify,
    names: &mut Names,
    patient: bool,
) -> Option<Vec<ChatMessageReceived>> {
    let notice = super::chat::event_name(ChatType::ChannelNotice);
    let notice_user = super::chat::event_name(ChatType::ChannelNoticeUser);
    let name = notify.channel.as_str();
    let line = match (notify.notice, &notify.tail) {
        (Notice::YouJoined, NoticeTail::Joined { instance, .. }) => {
            let (number, changed) = board.joined_channel(name, *instance);
            let mut line = channel_line(board, notice, name);
            line.channel = Channels::display(number, name);
            line.number = number;
            line.text = if changed { "YOU_CHANGED" } else { "YOU_JOINED" }.to_string();
            line
        }
        (Notice::YouLeft, _) => {
            let before = channel_line(board, notice, name);
            match board.left_channel(name) {
                Some((_, true)) => return Some(Vec::new()),
                _ => ChatMessageReceived { text: "YOU_LEFT".to_string(), ..before },
            }
        }
        (Notice::Joined, NoticeTail::Guid(guid)) | (Notice::Left, NoticeTail::Guid(guid)) => {
            let who = name_or_wait(names, *guid, patient)?;
            let event = super::chat::event_name(if notify.notice == Notice::Joined {
                ChatType::ChannelJoin
            } else {
                ChatType::ChannelLeave
            });
            ChatMessageReceived { author: who, ..channel_line(board, event, name) }
        }
        (Notice::ModeChange, NoticeTail::Mode { guid, old, new }) => {
            let Some(word) = mode_change_word(*old, *new) else {
                return Some(Vec::new());
            };
            let who = name_or_wait(names, *guid, patient)?;
            ChatMessageReceived {
                text: word.to_string(),
                author: who,
                ..channel_line(board, notice_user, name)
            }
        }
        (Notice::Invite, NoticeTail::Guid(guid)) => {
            // `CHAT_INVITE_NOTICE = "%2$s has invited you to join the channel
            // '%1$s'."` — the channel unnumbered, since it is not joined, and
            // the one notice the chat frame shows without holding the channel.
            let who = name_or_wait(names, *guid, patient)?;
            ChatMessageReceived {
                event: notice_user,
                text: "INVITE".to_string(),
                author: who,
                channel: name.to_string(),
                channel_name: name.to_string(),
                ..Default::default()
            }
        }
        (kind, NoticeTail::Guid(guid)) => {
            let who = name_or_wait(names, *guid, patient)?;
            ChatMessageReceived {
                text: kind.word().unwrap_or("").to_string(),
                author: who,
                ..channel_line(board, notice_user, name)
            }
        }
        (kind, NoticeTail::Pair { target, source }) => {
            // Both asked for before either decides, so a wait is one round
            // trip and not two.
            let (who, by) = (names.of(*target), names.of(*source));
            if patient && (who.is_none() || by.is_none()) {
                return None;
            }
            let (who, by) = (who.unwrap_or_default(), by.unwrap_or_default());
            ChatMessageReceived {
                text: kind.word().unwrap_or("").to_string(),
                author: who,
                target: by,
                ..channel_line(board, notice_user, name)
            }
        }
        (kind, NoticeTail::Name(who)) | (kind, NoticeTail::Owner(who)) => ChatMessageReceived {
            text: kind.word().unwrap_or("").to_string(),
            author: who.clone(),
            ..channel_line(board, notice_user, name)
        },
        (kind, _) => ChatMessageReceived {
            text: kind.word().unwrap_or("").to_string(),
            ..channel_line(board, notice, name)
        },
    };
    Some(vec![line])
}

/// `SMSG_CHANNEL_LIST` becomes one `CHAT_MSG_CHANNEL_LIST` line: the names,
/// comma-separated, which `CHAT_CHANNEL_LIST_GET = "[%s] "` prefixes with the
/// channel. The member flags are not drawn — the reference's marking of an
/// owner in this list has not been read.
fn list_lines(
    board: &Channels,
    list: &ChannelList,
    names: &mut Names,
    patient: bool,
) -> Option<Vec<ChatMessageReceived>> {
    let mut people = Vec::with_capacity(list.members.len());
    for member in &list.members {
        people.push(name_or_wait(names, member.guid, patient)?);
    }
    let people: Vec<String> = people.into_iter().filter(|name| !name.is_empty()).collect();
    let event = super::chat::event_name(ChatType::ChannelList);
    Some(vec![ChatMessageReceived {
        text: people.join(", "),
        ..channel_line(board, event, &list.channel)
    }])
}

/// `/chatlist` with no name: the client's own slots, one line each, with
/// no channel in `arg4` so the frame prints the line as it is. The
/// reference's wording for this list has not been read; the numbered form
/// is the one it uses everywhere else.
fn list_locally(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    let Some(host) = host else { return };
    let mut board = host.channels().borrow_mut();
    if !board.list_wanted {
        return;
    }
    board.list_wanted = false;
    let event = super::chat::event_name(ChatType::ChannelList);
    for (number, slot) in board.joined() {
        chat.write(ChatMessageReceived {
            event,
            text: Channels::display(number, &slot.name),
            ..Default::default()
        });
    }
}

fn send_verbs(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(mut host) = host else { return };
    let verbs = host.take_channel_verbs();
    if verbs.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for verb in verbs {
        active.live.channel(verb);
    }
}

/// Write the file's two channel lines back when leaving the world; see the
/// module doc.
fn save_on_logout(
    mut leaving: MessageReader<PlayerLeavingWorld>,
    files: Res<ChatCacheFile>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if leaving.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_back(&files, &host);
}

fn save_on_exit(
    mut exits: MessageReader<AppExit>,
    files: Res<ChatCacheFile>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if exits.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_back(&files, &host);
}

fn write_back(files: &ChatCacheFile, host: &crate::lua::host::LuaHost) {
    let (Some(path), Some(text)) = (files.path.as_ref(), files.text.as_ref()) else {
        return;
    };
    let mut board = host.channels().borrow_mut();
    let mask = board.mask();
    let customs = board.custom_channels();
    drop(board);
    let rewritten = chatcache::rewrite_channels(text, &customs, mask);
    if rewritten == *text {
        return;
    }
    match vale_config::write_file(path, rewritten) {
        Ok(()) => info!(
            "chat channels: {} rewritten — mask {mask}, {} custom",
            path.display(),
            customs.len()
        ),
        Err(e) => warn!("chat channels: could not write {}: {e}", path.display()),
    }
}

/// The two packets this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<ChannelAnswer> {
    Some(match event {
        PlayerEvent::ChannelNotify(notify) => ChannelAnswer::Notify(notify.clone()),
        PlayerEvent::ChannelList(list) => ChannelAnswer::List(list.clone()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::channels::{ChatChannel, ChatChannels, ZoneChannel};
    use std::sync::Arc;

    fn board() -> Channels {
        let mut board = Channels::default();
        board.table = Some(Arc::new(ChatChannels::from_rows(vec![
            ChatChannel { id: 1, flags: 0x3, pattern: "General - %s".into(), shortcut: "General".into() },
            ChatChannel { id: 2, flags: 0x3B, pattern: "Trade - %s".into(), shortcut: "Trade".into() },
            ChatChannel { id: 22, flags: 0x10003, pattern: "LocalDefense - %s".into(), shortcut: "LocalDefense".into() },
        ])));
        board
    }

    fn want(id: u32, name: &str) -> ZoneChannel {
        ZoneChannel { id, name: name.into() }
    }

    /// Walking from Elwynn into Stormwind: General and LocalDefense change
    /// name and keep their slots, Trade appears. Walking out again: Trade
    /// is left for real.
    #[test]
    fn a_zone_change_is_a_leave_and_a_join_per_row_that_changed() {
        let mut board = board();
        board.joined_channel("General - Elwynn Forest", 0);
        board.joined_channel("LocalDefense - Elwynn Forest", 0);
        let held = board.zone_slots();
        let wanted = vec![
            want(1, "General - Stormwind City"),
            want(2, "Trade - City"),
            want(22, "LocalDefense - Stormwind City"),
        ];
        let verbs = zone_change(&mut board, &held, &wanted);
        assert_eq!(
            verbs,
            vec![
                ChannelVerb::Leave("General - Elwynn Forest".into()),
                ChannelVerb::Join { name: "General - Stormwind City".into(), password: String::new() },
                ChannelVerb::Leave("LocalDefense - Elwynn Forest".into()),
                ChannelVerb::Join { name: "LocalDefense - Stormwind City".into(), password: String::new() },
                ChannelVerb::Join { name: "Trade - City".into(), password: String::new() },
            ]
        );
        // The two replaced slots are switching: their YOU_LEFT is silent and
        // keeps the slot, and the YOU_JOINED is a change in the same slot.
        assert_eq!(board.left_channel("General - Elwynn Forest"), Some((1, true)));
        assert_eq!(board.joined_channel("General - Stormwind City", 0), (1, true));
        // Nothing to do when nothing changed.
        board.joined_channel("LocalDefense - Stormwind City", 0);
        board.joined_channel("Trade - City", 0);
        let held = board.zone_slots();
        assert!(zone_change(&mut board, &held, &wanted).is_empty());
        // Leaving the city: Trade goes, for real.
        let wanted = vec![want(1, "General - Stormwind City"), want(22, "LocalDefense - Stormwind City")];
        let held = board.zone_slots();
        assert_eq!(zone_change(&mut board, &held, &wanted), vec![ChannelVerb::Leave("Trade - City".into())]);
        assert_eq!(board.left_channel("Trade - City"), Some((2, false)));
    }

    #[test]
    fn a_you_joined_is_a_numbered_notice_and_a_change_when_it_replaces_the_row() {
        let mut board = board();
        let mut names = Names { world: None, asked: Vec::new() };
        let notify = ChannelNotify {
            notice: Notice::YouJoined,
            channel: "General - Elwynn Forest".into(),
            tail: NoticeTail::Joined { flags: 0x18, instance: 0 },
        };
        let lines = notice_lines(&mut board, &notify, &mut names, true).expect("no name to wait for");
        let line = &lines[0];
        assert_eq!(line.event, "CHAT_MSG_CHANNEL_NOTICE");
        assert_eq!(line.text, "YOU_JOINED");
        assert_eq!(line.channel, "1. General - Elwynn Forest");
        assert_eq!(line.channel_name, "General - Elwynn Forest");
        assert_eq!((line.zone_channel, line.number, line.instance), (1, 1, 0));
        // LocalDefense is 3, with 2 kept for Trade — the reference's own
        // screen in Elwynn.
        let local = ChannelNotify {
            notice: Notice::YouJoined,
            channel: "LocalDefense - Elwynn Forest".into(),
            tail: NoticeTail::Joined { flags: 0, instance: 0 },
        };
        let lines = notice_lines(&mut board, &local, &mut names, true).expect("decided");
        assert_eq!(lines[0].channel, "3. LocalDefense - Elwynn Forest");
        // The zone changes.
        board.switching(1);
        let left = ChannelNotify { notice: Notice::YouLeft, channel: "General - Elwynn Forest".into(), tail: NoticeTail::None };
        assert!(notice_lines(&mut board, &left, &mut names, true).expect("decided").is_empty(), "a switch is silent");
        let joined = ChannelNotify {
            notice: Notice::YouJoined,
            channel: "General - Dun Morogh".into(),
            tail: NoticeTail::Joined { flags: 0x18, instance: 0 },
        };
        let lines = notice_lines(&mut board, &joined, &mut names, true).expect("decided");
        assert_eq!(lines[0].text, "YOU_CHANGED");
        assert_eq!(lines[0].channel, "1. General - Dun Morogh");
        // …and a real leave says so and frees the slot.
        let left = ChannelNotify { notice: Notice::YouLeft, channel: "General - Dun Morogh".into(), tail: NoticeTail::None };
        let lines = notice_lines(&mut board, &left, &mut names, true).expect("decided");
        assert_eq!(lines[0].text, "YOU_LEFT");
        assert_eq!(board.slot_of("1"), None);
    }

    #[test]
    fn a_notice_about_a_stranger_waits_for_the_name_and_then_gives_up() {
        let mut board = board();
        board.joined_channel("World", 0);
        let mut names = Names { world: None, asked: Vec::new() };
        let kicked = ChannelNotify {
            notice: Notice::PlayerKicked,
            channel: "World".into(),
            tail: NoticeTail::Pair { target: 7, source: 9 },
        };
        assert!(notice_lines(&mut board, &kicked, &mut names, true).is_none(), "waits");
        assert_eq!(names.asked, vec![7, 9], "and asks");
        let lines = notice_lines(&mut board, &kicked, &mut names, false).expect("gave up");
        assert_eq!(lines[0].event, "CHAT_MSG_CHANNEL_NOTICE_USER");
        assert_eq!(lines[0].text, "PLAYER_KICKED");
        assert_eq!(lines[0].channel, "4. World", "the first slot past the three reserved");
        // A refusal with no person in it needs no name.
        let wrong = ChannelNotify { notice: Notice::WrongPassword, channel: "Secret".into(), tail: NoticeTail::None };
        let lines = notice_lines(&mut board, &wrong, &mut names, true).expect("decided");
        assert_eq!(lines[0].event, "CHAT_MSG_CHANNEL_NOTICE");
        assert_eq!(lines[0].text, "WRONG_PASSWORD");
        assert_eq!(lines[0].channel, "0. Secret", "not joined, so no number");
        // An invitation names the channel bare.
        let invite = ChannelNotify { notice: Notice::Invite, channel: "Secret".into(), tail: NoticeTail::Guid(0) };
        let lines = notice_lines(&mut board, &invite, &mut names, true).expect("decided");
        assert_eq!(lines[0].text, "INVITE");
        assert_eq!(lines[0].channel, "Secret");
    }

    #[test]
    fn the_file_seeds_the_mask_the_rejoins_and_the_option_and_zero_is_unset() {
        let mut board = board();
        let cache = chatcache::parse(
            "OPTION_GUILD_RECRUITMENT_CHANNEL AUTO\nCHANNELS\nWorld\nEND\nZONECHANNELS 18874371\nWINDOW 1\nCHANNELS\nEND\nZONECHANNELS 2\n",
        );
        seed(&mut board, &cache);
        assert_eq!(board.joined_mask, Some(18_874_371));
        assert_eq!(board.window_mask, Some(2));
        assert_eq!(board.rejoin, vec!["World"]);
        assert!(board.recruit_automatically);
        let mut fresh = self::tests::board();
        seed(&mut fresh, &chatcache::parse("ZONECHANNELS 0\n"));
        assert_eq!(fresh.joined_mask, None, "zero is unset");
        let default = fresh.table.as_ref().unwrap().default_joined();
        assert_eq!(fresh.mask(), default);
    }

    #[test]
    fn the_two_packets_are_the_two_answers() {
        let notify = ChannelNotify { notice: Notice::Muted, channel: "World".into(), tail: NoticeTail::None };
        assert!(matches!(
            answer_of(&PlayerEvent::ChannelNotify(Box::new(notify))),
            Some(ChannelAnswer::Notify(_))
        ));
        let list = ChannelList { channel: "World".into(), flags: 0, members: Vec::new() };
        assert!(matches!(answer_of(&PlayerEvent::ChannelList(Box::new(list))), Some(ChannelAnswer::List(_))));
        assert!(answer_of(&PlayerEvent::MailReceived).is_none());
    }
}
