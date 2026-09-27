//! Chat channels, as the chat frame sees them: ten numbered slots and the
//! twenty-five globals `ChatFrame.lua` calls about them.
//!
//! ## The numbers are the client's
//!
//! A channel on the wire is a name. The interface types `/2 hello`, prints
//! `[2. Trade - City]`, and keys every line by `arg8`, the number — and the
//! number exists nowhere but in this board: `YOU_JOINED` takes the lowest free
//! slot of ten (`ChatTypeInfo["CHANNEL1".."CHANNEL10"]` is why ten), `YOU_LEFT`
//! frees it, and [`Channels::slot_of`] answers `GetChannelName` either way
//! round. A zone channel arriving while the same table row already holds a
//! slot **replaces it in place**, which is what keeps General at 1 through a
//! zone change and what the interface calls `YOU_CHANGED` — notice `0x02`,
//! the same code as `YOU_JOINED`; see [`vale_protocol::play::channels`].
//!
//! ## What a chat window is registered for
//!
//! `ChatFrame_OnLoad` calls `GetChatWindowChannels(id)` and keeps the pairs it
//! answers as `channelList`/`zoneChannelList`; `ChatFrame_OnEvent` then shows
//! a channel line only if the frame has that channel, by zone id (`arg7`)
//! for a zone channel and by name for a custom one. The reference reads the
//! pairs out of `chat-cache.txt` (`assets::interface::chatcache`): window
//! 1's `ZONECHANNELS` mask when it is set, and the custom channels rejoined
//! at login. [`Channels::window_channels`] answers that, with the three
//! `INITIAL` rows of `ChatChannels.dbc` by shortcut and id where the mask is
//! unset, plus whatever `AddChatWindowChannel` added this session. Zone
//! channels are matched by id, so the shortcut is the right name for them.
//!
//! ## What is held here and what is the session's
//!
//! This is the board — the slots, the mask of zone channels the character
//! has not left, the custom channels to rejoin, the guild-recruitment
//! option — and the verbs the interface queued. The packets that fill it,
//! the file it is seeded from, the auto-join on a zone change and the events
//! it raises are `crate::interface::channels`, exactly as `social`
//! splits.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::tables::area::Areas;
use vale_assets::tables::channels::{flags, joined_bit, ChatChannels, ZoneChannel};
use vale_protocol::socket::session::ChannelVerb;

/// The globals this panel answers — the chat frame's own list, from
/// `ChatFrame.lua`'s slash commands and `ChatEdit_*`.
pub const VERBS: [&str; 26] = [
    "AddChatWindowChannel",
    "ChannelBan",
    "ChannelInvite",
    "ChannelKick",
    "ChannelModerate",
    "ChannelModerator",
    "ChannelMute",
    "ChannelToggleAnnouncements",
    "ChannelUnban",
    "ChannelUnmoderator",
    "ChannelUnmute",
    "DisplayChannelOwner",
    "EnumerateServerChannels",
    "GetChannelList",
    "GetChannelName",
    "GetChatWindowChannels",
    "JoinChannelByName",
    "LeaveChannelByName",
    "ListChannelByName",
    "ListChannels",
    "RemoveChatWindowChannel",
    "SetChannelOwner",
    "SetChannelPassword",
    "ChannelSilence",
    "ChannelUnSilence",
    "ChannelVoiceOff",
];

/// How many numbered channels the interface has colours for.
pub const SLOTS: usize = 10;

/// One joined channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// The name on the wire: `General - Elwynn Forest`, `World`.
    pub name: String,
    /// The `ChatChannels.dbc` id for a zone channel — the chat frame's
    /// `arg7` — and 0 for a custom one.
    pub zone_id: u32,
    /// `YOU_JOINED`'s second word, the number appended to a name when the
    /// server splits a channel; vmangos sends 0.
    pub instance: u32,
    /// The zone changed and this slot's channel is being replaced: the
    /// `YOU_LEFT` on the way is not an event, and the `YOU_JOINED` that
    /// follows it is a `YOU_CHANGED`. See `interface::channels`.
    pub switching: bool,
}

/// The board.
#[derive(Default)]
pub struct Channels {
    /// Ten slots, numbered from 1 on the interface.
    pub slots: Vec<Option<Slot>>,
    /// The table, once the session has supplied it.
    pub table: Option<Arc<ChatChannels>>,
    pub areas: Option<Arc<Areas>>,
    /// The reference's `ZONECHANNELS` mask — bit `id - 1` for each zone
    /// channel the character has not left. `None` until the file or the
    /// table defaults it.
    pub joined_mask: Option<u32>,
    /// Window 1's own `ZONECHANNELS` from the file, `None` when unset.
    pub window_mask: Option<u32>,
    /// The custom channels the file says to rejoin at login, and whether
    /// they have been.
    pub rejoin: Vec<String>,
    pub rejoined: bool,
    /// `OPTION_GUILD_RECRUITMENT_CHANNEL AUTO`: join the recruitment channel
    /// in a city while the character has no guild. `true` until a file
    /// says otherwise, which is the reference's default.
    pub recruit_automatically: bool,
    /// Whether the character is in a guild — `PLAYER_GUILDID` non-zero,
    /// read by the session.
    pub in_guild: bool,
    /// Custom channels a window added this session — see the module note.
    pub window_added: Vec<String>,
    /// `ListChannels()` was called: the session composes the line.
    pub list_wanted: bool,
    /// Bumped on every change to the slots, for the session's own bookkeeping.
    pub version: u32,
}

impl Channels {
    fn slots_mut(&mut self) -> &mut Vec<Option<Slot>> {
        if self.slots.len() < SLOTS {
            self.slots.resize(SLOTS, None);
        }
        &mut self.slots
    }

    pub fn joined(&self) -> impl Iterator<Item = (u32, &Slot)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|slot| (i as u32 + 1, slot)))
    }

    /// The slot a number or a name refers to, as `(number, slot)`.
    pub fn slot_of(&self, key: &str) -> Option<(u32, &Slot)> {
        if let Ok(number) = key.trim().parse::<u32>() {
            return self.joined().find(|(n, _)| *n == number);
        }
        self.joined().find(|(_, slot)| slot.name.eq_ignore_ascii_case(key.trim()))
    }

    /// The number a name holds, for `SendChatMessage`'s target and the chat
    /// line's `arg8`.
    pub fn number_of(&self, name: &str) -> Option<u32> {
        self.joined()
            .find(|(_, slot)| slot.name.eq_ignore_ascii_case(name))
            .map(|(n, _)| n)
    }

    /// The table row a channel name belongs to, or 0 for a custom channel.
    pub fn zone_id_of(&self, name: &str) -> u32 {
        self.table
            .as_ref()
            .and_then(|table| table.row_of(name))
            .map_or(0, |row| row.id)
    }

    /// **`YOU_JOINED` landed**: the slot the channel takes, and whether it
    /// replaced a zone channel of the same row — which the interface calls a
    /// change rather than a join. Returns `(number, changed)`.
    pub fn joined_channel(&mut self, name: &str, instance: u32) -> (u32, bool) {
        let zone_id = self.zone_id_of(name);
        // **The three `INITIAL` rows have their numbers whether or not they
        // are joined** — General 1, Trade 2, LocalDefense 3 — and a custom
        // channel starts past them. See `ChatChannels::reserved_number`.
        let reserved = self
            .table
            .as_ref()
            .and_then(|table| (zone_id != 0).then(|| table.reserved_number(zone_id)).flatten())
            .map(|number| number as usize - 1);
        let first_free = self.table.as_ref().map_or(0, |table| table.reserved_count() as usize);
        let slots = self.slots_mut();
        let same_row = zone_id != 0;
        let replaced = same_row.then(|| {
            slots.iter().position(|slot| slot.as_ref().is_some_and(|s| s.zone_id == zone_id))
        }).flatten();
        let same_name = slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|s| s.name.eq_ignore_ascii_case(name)));
        let at = replaced
            .or(same_name)
            .or(reserved)
            .or_else(|| slots.iter().skip(first_free).position(Option::is_none).map(|i| i + first_free));
        let Some(at) = at else {
            // Ten channels already: the eleventh has no number and no colour,
            // and the reference refuses it the same way.
            return (0, false);
        };
        let changed = replaced.is_some() && same_name.is_none();
        slots[at] = Some(Slot { name: name.to_string(), zone_id, instance, switching: false });
        self.version = self.version.wrapping_add(1);
        (at as u32 + 1, changed)
    }

    /// **`YOU_LEFT` landed**: `Some((number, was_switching))` for a slot that
    /// held the channel. A switching slot is kept for the join that follows.
    pub fn left_channel(&mut self, name: &str) -> Option<(u32, bool)> {
        let at = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|s| s.name.eq_ignore_ascii_case(name)))?;
        let switching = self.slots[at].as_ref().is_some_and(|s| s.switching);
        if !switching {
            self.slots[at] = None;
        }
        self.version = self.version.wrapping_add(1);
        Some((at as u32 + 1, switching))
    }

    /// Mark the slot holding a zone channel as being replaced by the zone
    /// change — see [`Slot::switching`].
    pub fn switching(&mut self, zone_id: u32) {
        for slot in self.slots.iter_mut().flatten() {
            if slot.zone_id == zone_id {
                slot.switching = true;
            }
        }
    }

    /// The zone channels currently held, as `(id, name)`.
    pub fn zone_slots(&self) -> Vec<(u32, String)> {
        self.joined()
            .filter(|(_, slot)| slot.zone_id != 0)
            .map(|(_, slot)| (slot.zone_id, slot.name.clone()))
            .collect()
    }

    /// The custom channels currently held — what the file's `CHANNELS` block
    /// is written from.
    pub fn custom_channels(&self) -> Vec<String> {
        self.joined()
            .filter(|(_, slot)| slot.zone_id == 0)
            .map(|(_, slot)| slot.name.clone())
            .collect()
    }

    /// The mask, defaulted from the table on first use.
    pub fn mask(&mut self) -> u32 {
        if let Some(mask) = self.joined_mask {
            return mask;
        }
        let mask = self.table.as_ref().map_or(0, |table| table.default_joined());
        self.joined_mask = Some(mask);
        mask
    }

    /// A zone channel left by hand stays left across zones — the
    /// `ZONECHANNELS` rule.
    pub fn leave_zone_channel(&mut self, id: u32) {
        let mask = self.mask();
        self.joined_mask = Some(mask & !joined_bit(id));
    }

    /// …and joining one by hand puts it back.
    pub fn rejoin_zone_channel(&mut self, id: u32) {
        let mask = self.mask();
        self.joined_mask = Some(mask | joined_bit(id));
    }

    /// Whether the guild recruitment channel is wanted: the option is `AUTO`
    /// and the character has no guild.
    pub fn recruits(&self) -> bool {
        self.recruit_automatically && !self.in_guild
    }

    /// The channels a character standing in `area` should be in.
    pub fn wanted(&mut self, area: u32) -> Vec<ZoneChannel> {
        let mask = self.mask();
        let recruit = self.recruits();
        let (Some(table), Some(areas)) = (self.table.clone(), self.areas.clone()) else {
            return Vec::new();
        };
        table.wanted(&areas, area, mask, recruit)
    }

    /// What `GetChatWindowChannels` answers — see the module note.
    pub fn window_channels(&self) -> Vec<(String, u32)> {
        let mut out: Vec<(String, u32)> = match (&self.table, self.window_mask) {
            (Some(table), Some(mask)) if mask != 0 => table
                .rows()
                .iter()
                .filter(|row| mask & joined_bit(row.id) != 0)
                .map(|row| (row.shortcut.clone(), row.id))
                .collect(),
            (Some(table), _) => table
                .rows()
                .iter()
                .filter(|row| row.has(flags::INITIAL))
                .map(|row| (row.shortcut.clone(), row.id))
                .collect(),
            // The shipped table's three INITIAL rows, for a window that loads
            // before the table is supplied — which is every window, since the
            // interface loads inside the frame the host is built.
            (None, _) => vec![("General".into(), 1), ("Trade".into(), 2), ("LocalDefense".into(), 22)],
        };
        for name in self.rejoin.iter().chain(&self.window_added) {
            if !out.iter().any(|(held, _)| held.eq_ignore_ascii_case(name)) {
                out.push((name.clone(), 0));
            }
        }
        out
    }

    /// `1. General - Elwynn Forest` — the `arg4` of every channel line.
    pub fn display(number: u32, name: &str) -> String {
        format!("{number}. {name}")
    }
}

pub type Held = Rc<RefCell<Channels>>;
pub type Queue = Rc<RefCell<Vec<ChannelVerb>>>;

/// The channel a verb names, as typed: a number is a slot, anything else a
/// name. Answers the name on the wire, or the text itself for a channel the
/// board does not hold — the server's `NOT_MEMBER` is the right refusal.
fn wire_name(board: &Channels, key: &str) -> String {
    board
        .slot_of(key)
        .map(|(_, slot)| slot.name.clone())
        .unwrap_or_else(|| key.trim().to_string())
}

pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    // `JoinChannelByName(name, password, chatFrameId)` -> `zoneChannel, name`.
    // `SlashCmdList["JOIN"]` keeps only a non-nil first answer, so a custom
    // channel answers 0 rather than nil.
    let get = Rc::clone(held);
    let push = Rc::clone(queue);
    globals.set(
        "JoinChannelByName",
        lua.create_function(
            move |lua, (name, password, _frame): (Option<String>, Option<String>, Option<mlua::Value>)| {
                let name = name.unwrap_or_default().trim().to_string();
                if name.is_empty() {
                    return Ok((mlua::Value::Nil, mlua::Value::Nil));
                }
                let mut board = get.borrow_mut();
                let zone_id = board.zone_id_of(&name);
                if zone_id != 0 {
                    board.rejoin_zone_channel(zone_id);
                }
                push.borrow_mut().push(ChannelVerb::Join {
                    name: name.clone(),
                    password: password.unwrap_or_default(),
                });
                Ok((
                    mlua::Value::Number(f64::from(zone_id)),
                    mlua::Value::String(lua.create_string(&name)?),
                ))
            },
        )?,
    )?;

    let get = Rc::clone(held);
    let push = Rc::clone(queue);
    globals.set(
        "LeaveChannelByName",
        lua.create_function(move |_, name: Option<String>| {
            let name = name.unwrap_or_default();
            let mut board = get.borrow_mut();
            let wire = wire_name(&board, &name);
            let zone_id = board.zone_id_of(&wire);
            if zone_id != 0 {
                board.leave_zone_channel(zone_id);
            }
            push.borrow_mut().push(ChannelVerb::Leave(wire));
            Ok(())
        })?,
    )?;

    // `GetChannelName(numberOrName)` -> `number, name, instance`; a miss is
    // `0, nil, 0`, which is what both `ChatEdit_ExtractChannel` (`channelNum
    // <= 0`) and `ChatEdit_UpdateHeader` (`if channelName`) test.
    let get = Rc::clone(held);
    globals.set(
        "GetChannelName",
        lua.create_function(move |lua, key: Option<mlua::Value>| {
            let key = match key {
                Some(mlua::Value::String(s)) => s.to_str()?.to_string(),
                Some(mlua::Value::Integer(n)) => n.to_string(),
                Some(mlua::Value::Number(n)) => (n as i64).to_string(),
                _ => String::new(),
            };
            let board = get.borrow();
            Ok(match board.slot_of(&key) {
                Some((number, slot)) => (
                    mlua::Value::Number(f64::from(number)),
                    mlua::Value::String(lua.create_string(&slot.name)?),
                    mlua::Value::Number(f64::from(slot.instance)),
                ),
                None => (mlua::Value::Number(0.0), mlua::Value::Nil, mlua::Value::Number(0.0)),
            })
        })?,
    )?;

    // `GetChannelList()` -> `number, name, number, name, …`.
    let get = Rc::clone(held);
    globals.set(
        "GetChannelList",
        lua.create_function(move |lua, ()| {
            let board = get.borrow();
            let mut out = Vec::new();
            for (number, slot) in board.joined() {
                out.push(mlua::Value::Number(f64::from(number)));
                out.push(mlua::Value::String(lua.create_string(&slot.name)?));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;

    // `ListChannels()` is local — the client prints its own slots — and
    // `ListChannelByName(name)` asks the server who is on one.
    let get = Rc::clone(held);
    globals.set(
        "ListChannels",
        lua.create_function(move |_, ()| {
            get.borrow_mut().list_wanted = true;
            Ok(())
        })?,
    )?;
    let get = Rc::clone(held);
    let push = Rc::clone(queue);
    globals.set(
        "ListChannelByName",
        lua.create_function(move |_, name: Option<String>| {
            let wire = wire_name(&get.borrow(), &name.unwrap_or_default());
            push.borrow_mut().push(ChannelVerb::List(wire));
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    let push = Rc::clone(queue);
    globals.set(
        "SetChannelPassword",
        lua.create_function(move |_, (name, password): (Option<String>, Option<String>)| {
            let wire = wire_name(&get.borrow(), &name.unwrap_or_default());
            push.borrow_mut().push(ChannelVerb::Password {
                name: wire,
                password: password.unwrap_or_default(),
            });
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    let push = Rc::clone(queue);
    globals.set(
        "DisplayChannelOwner",
        lua.create_function(move |_, name: Option<String>| {
            let wire = wire_name(&get.borrow(), &name.unwrap_or_default());
            push.borrow_mut().push(ChannelVerb::Owner(wire));
            Ok(())
        })?,
    )?;

    // The name-and-player nine, one shape each.
    macro_rules! with_player {
        ($global:literal, $verb:ident) => {{
            let get = Rc::clone(held);
            let push = Rc::clone(queue);
            globals.set(
                $global,
                lua.create_function(move |_, (name, player): (Option<String>, Option<String>)| {
                    let wire = wire_name(&get.borrow(), &name.unwrap_or_default());
                    let player = player.unwrap_or_default().trim().to_string();
                    if player.is_empty() {
                        return Ok(());
                    }
                    push.borrow_mut().push(ChannelVerb::$verb { name: wire, player });
                    Ok(())
                })?,
            )?;
        }};
    }
    with_player!("SetChannelOwner", SetOwner);
    with_player!("ChannelModerator", Moderator);
    with_player!("ChannelUnmoderator", Unmoderator);
    with_player!("ChannelMute", Mute);
    with_player!("ChannelUnmute", Unmute);
    with_player!("ChannelInvite", Invite);
    with_player!("ChannelKick", Kick);
    with_player!("ChannelBan", Ban);
    with_player!("ChannelUnban", Unban);

    // …and the two toggles, the name alone.
    macro_rules! toggle {
        ($global:literal, $verb:ident) => {{
            let get = Rc::clone(held);
            let push = Rc::clone(queue);
            globals.set(
                $global,
                lua.create_function(move |_, name: Option<String>| {
                    let wire = wire_name(&get.borrow(), &name.unwrap_or_default());
                    push.borrow_mut().push(ChannelVerb::$verb(wire));
                    Ok(())
                })?,
            )?;
        }};
    }
    toggle!("ChannelToggleAnnouncements", Announcements);
    toggle!("ChannelModerate", Moderate);

    // **The three the wire has no packet for.** `ChannelSilence`,
    // `ChannelUnSilence` and `ChannelVoiceOff` are in the slash list and
    // 1.12's server has nothing behind them — `Channel::Voice` and `DeVoice`
    // are empty in vmangos. Answered, so the command does not raise, and
    // sent nowhere.
    for name in ["ChannelSilence", "ChannelUnSilence", "ChannelVoiceOff"] {
        globals.set(name, lua.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?)?;
    }
    // `EnumerateServerChannels()` lists the realm's public channels for a
    // menu 1.12 does not draw; the six zone channels are what a server has.
    let get = Rc::clone(held);
    globals.set(
        "EnumerateServerChannels",
        lua.create_function(move |lua, ()| {
            let board = get.borrow();
            let names: Vec<mlua::Value> = match &board.table {
                Some(table) => table
                    .rows()
                    .iter()
                    .map(|row| lua.create_string(&row.shortcut).map(mlua::Value::String))
                    .collect::<mlua::Result<_>>()?,
                None => Vec::new(),
            };
            Ok(mlua::Variadic::from(names))
        })?,
    )?;

    // The chat window's own list — see the module note.
    let get = Rc::clone(held);
    globals.set(
        "GetChatWindowChannels",
        lua.create_function(move |lua, _id: Option<mlua::Value>| {
            let board = get.borrow();
            let mut out = Vec::new();
            for (name, zone_id) in board.window_channels() {
                out.push(mlua::Value::String(lua.create_string(&name)?));
                out.push(mlua::Value::Number(f64::from(zone_id)));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;
    let get = Rc::clone(held);
    globals.set(
        "AddChatWindowChannel",
        lua.create_function(move |_, (_id, name): (Option<mlua::Value>, Option<String>)| {
            let name = name.unwrap_or_default().trim().to_string();
            let mut board = get.borrow_mut();
            let zone_id = board.zone_id_of(&name);
            if zone_id == 0 && !name.is_empty()
                && !board.window_added.iter().any(|n| n.eq_ignore_ascii_case(&name))
            {
                board.window_added.push(name);
            }
            Ok(f64::from(zone_id))
        })?,
    )?;
    let get = Rc::clone(held);
    globals.set(
        "RemoveChatWindowChannel",
        lua.create_function(move |_, (_id, name): (Option<mlua::Value>, Option<String>)| {
            let name = name.unwrap_or_default();
            get.borrow_mut()
                .window_added
                .retain(|n| !n.eq_ignore_ascii_case(name.trim()));
            Ok(())
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::channels::ChatChannel;

    fn board() -> Channels {
        let mut board = Channels::default();
        board.table = Some(Arc::new(ChatChannels::from_rows(vec![
            ChatChannel { id: 1, flags: 0x3, pattern: "General - %s".into(), shortcut: "General".into() },
            ChatChannel { id: 2, flags: 0x3B, pattern: "Trade - %s".into(), shortcut: "Trade".into() },
            ChatChannel { id: 22, flags: 0x10003, pattern: "LocalDefense - %s".into(), shortcut: "LocalDefense".into() },
            ChatChannel { id: 25, flags: 0x20032, pattern: "GuildRecruitment - %s".into(), shortcut: "GuildRecruitment".into() },
        ])));
        board.recruit_automatically = true;
        board
    }

    #[test]
    fn a_join_takes_the_lowest_free_slot_and_a_zone_change_keeps_the_number() {
        let mut board = board();
        assert_eq!(board.joined_channel("General - Elwynn Forest", 0), (1, false));
        // A custom channel starts past the three reserved numbers, and
        // LocalDefense is 3 whether or not Trade is there.
        assert_eq!(board.joined_channel("World", 0), (4, false));
        assert_eq!(board.joined_channel("LocalDefense - Elwynn Forest", 0), (3, false));
        // Leaving World frees 4; Trade takes its own 2.
        assert_eq!(board.left_channel("world"), Some((4, false)));
        assert_eq!(board.joined_channel("Trade - City", 0), (2, false));
        // The zone changes: General is replaced in place, as a change.
        board.switching(1);
        assert_eq!(board.left_channel("General - Elwynn Forest"), Some((1, true)), "kept for the join");
        assert_eq!(board.joined_channel("General - Westfall", 0), (1, true));
        assert_eq!(board.slot_of("1").map(|(_, s)| s.name.as_str()), Some("General - Westfall"));
        assert_eq!(board.slot_of("general - westfall").map(|(n, _)| n), Some(1));
        assert_eq!(board.number_of("Trade - City"), Some(2));
        // The same name joined twice is the same slot and not a change.
        assert_eq!(board.joined_channel("Trade - City", 0), (2, false));
        // The custom channels are what the file's block is written from.
        board.joined_channel("World", 0);
        assert_eq!(board.custom_channels(), vec!["World"]);
    }

    #[test]
    fn the_eleventh_channel_has_no_number() {
        let mut board = board();
        // Seven customs fit past the three reserved; the eighth does not.
        for i in 0..SLOTS - 3 {
            assert_eq!(board.joined_channel(&format!("Room{i}"), 0).0, i as u32 + 4);
        }
        assert_eq!(board.joined_channel("Room7", 0), (0, false));
        // …and a table-less board numbers from 1.
        let mut bare = Channels::default();
        assert_eq!(bare.joined_channel("Room", 0), (1, false));
    }

    #[test]
    fn a_zone_channel_left_by_hand_stays_left() {
        let mut board = board();
        assert_eq!(board.mask(), 0b11 | (1 << 21) | (1 << 24), "the real file's 18874371");
        board.leave_zone_channel(2);
        assert_eq!(board.mask() & joined_bit(2), 0);
        board.rejoin_zone_channel(2);
        assert_ne!(board.mask() & joined_bit(2), 0);
        // A file's mask is taken as it is.
        board.joined_mask = Some(3);
        assert_eq!(board.mask(), 3);
    }

    #[test]
    fn recruitment_is_wanted_without_a_guild_and_under_auto() {
        let mut board = board();
        assert!(board.recruits());
        board.in_guild = true;
        assert!(!board.recruits());
        board.in_guild = false;
        board.recruit_automatically = false;
        assert!(!board.recruits());
    }

    #[test]
    fn a_window_is_registered_for_the_initial_rows_by_id_and_custom_ones_by_name() {
        let mut board = board();
        board.window_added.push("World".into());
        board.rejoin.push("Trash".into());
        assert_eq!(
            board.window_channels(),
            vec![
                ("General".to_string(), 1),
                ("Trade".to_string(), 2),
                ("LocalDefense".to_string(), 22),
                ("Trash".to_string(), 0),
                ("World".to_string(), 0),
            ]
        );
        // The file's own window mask, when set, is the list.
        board.window_mask = Some(joined_bit(2));
        assert_eq!(board.window_channels()[0], ("Trade".to_string(), 2));
        assert_eq!(board.window_channels().len(), 3);
        // …and a zero mask is unset.
        board.window_mask = Some(0);
        assert_eq!(board.window_channels().len(), 5);
        assert_eq!(Channels::display(2, "Trade - City"), "2. Trade - City");
        // …and the fallback, before the table is here, is the same three.
        assert_eq!(Channels::default().window_channels().len(), 3);
    }
}
