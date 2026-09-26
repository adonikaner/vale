//! **What to tell the player**, in the game's own words.
//!
//! One job: turn a `GlobalStrings.lua` key into a
//! [`UI_ERROR_MESSAGE`][super::events::UiErrorMessage] or a
//! [`UI_INFO_MESSAGE`][super::events::UiInfoMessage]. That is exactly the split
//! the real client makes, and the archives say so in six lines —
//! `Interface\FrameXML\UIErrorsFrame.lua` in its entirety is
//!
//! ```lua
//! function UIErrorsFrame_OnEvent(event, message)
//!     if ( event == "SYSMSG" ) then
//!         this:AddMessage(message, arg2, arg3, arg4, 1.0);
//!     elseif ( event == "UI_INFO_MESSAGE" ) then
//!         this:AddMessage(message, 1.0, 1.0, 0.0, 1.0);
//!     elseif ( event == "UI_ERROR_MESSAGE" ) then
//!         this:AddMessage(message, 1.0, 0.1, 0.1, 1.0);
//! ```
//!
//! — no table, no lookup, no formatting, and **the whole difference between the
//! two is a colour**: `(1, 1, 0)` yellow against `(1, 0.1, 0.1)` red. **The C
//! side has already resolved the text**; the interface only picks a colour and a
//! frame to put it in. So the resolution belongs here, on this side of the
//! `game/` – `ui/` line, and what crosses the line is a finished string.
//!
//! **Nothing in this client may compose a message.** Every line comes from
//! `Interface\FrameXML\GlobalStrings.lua` by key — see [`vale_assets::interface::strings`]
//! — so what the player reads is what the real client would have said. There is
//! deliberately no "and if the key is missing, say this instead": three of the
//! game's 146 cast-failure reasons have no string at all and the real client shows
//! **nothing** for them, so a fallback here would put text on screen that the game
//! never displays.
//!
//! ## This used to be a queue, and the queue was the bug
//!
//! [`Messages`] was a `Vec<String>` with a `take()` that emptied it. That works
//! for exactly one consumer and fails silently for two, which is the wrong shape
//! for a surface whose entire future is "N frames and M addons each registered for
//! this". See [`super::events`] for the argument in full; this module is what is
//! left of the old one after the queue came out, and it kept the part that was
//! always right — that the *rule* lives here and the pixels do not.

use super::events::{UiErrorMessage, UiInfoMessage};
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_assets::interface::strings::Strings;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::sync::Arc;

/// The game's own strings, held once.
///
/// A resource rather than a call per system: [`GameAssets::strings`] takes a lock
/// and this is read on every failed cast, which is not often but is inside an
/// input path.
#[derive(Resource, Default)]
pub struct UiStrings(pub Option<Arc<Strings>>);

impl UiStrings {
    pub fn get(&self) -> Option<&Strings> {
        self.0.as_deref()
    }
}

/// Say something to the player, by key.
///
/// A [`SystemParam`] bundling the string table with the writer, so that **the
/// only reachable way to produce a [`UiErrorMessage`] is through a key**. That is
/// the rule at the top of this file made structural rather than remembered: a
/// system that wanted to invent a sentence would have to take
/// `MessageWriter<UiErrorMessage>` itself and say so in its signature.
#[derive(SystemParam)]
pub struct UiErrors<'w> {
    strings: Res<'w, UiStrings>,
    writer: MessageWriter<'w, UiErrorMessage>,
}

impl UiErrors<'_> {
    /// Show the game's own line for `key`. A key the file does not carry shows
    /// **nothing**, which is the real client's behaviour — see the module comment.
    pub fn key(&mut self, key: &str) {
        let Some(text) = self.strings.get().and_then(|s| s.get(key)) else {
            return;
        };
        let text = text.to_string();
        self.writer.write(UiErrorMessage(text));
    }

    /// The same, with the one argument the dozen `%s` keys take filled in.
    pub fn formatted(&mut self, key: &str, argument: &str) {
        let Some(text) = self.strings.get().and_then(|s| s.format(key, argument)) else {
            return;
        };
        self.writer.write(UiErrorMessage(text));
    }

    /// **…and the same again where the argument is itself a key.**
    ///
    /// `ERR_TAME_FAILED` is the whole of `"%s."`, and what goes in the hole is
    /// one of twelve `PETTAME_*` sentences — so the tame refusal is two lookups
    /// and neither of them is a word this client writes. See
    /// [`vale_protocol::play::pet::tame_failure`], whose table is the
    /// client's.
    ///
    /// A missing *inner* key shows nothing at all, rather than a full stop on
    /// its own.
    pub fn substituted(&mut self, key: &str, argument_key: &str) {
        let Some(argument) = self.strings.get().and_then(|s| s.get(argument_key)) else {
            return;
        };
        let argument = argument.to_string();
        self.formatted(key, &argument);
    }
}

/// **…and the yellow half of the same thing**, as a parameter of its own.
///
/// Not a second writer on [`UiErrors`], and the reason is a `SystemParam` one:
/// every one of Bevy's message queues a param names has to be *initialised* in
/// the world before that param will validate, so folding this in would have made
/// `UiErrors` refuse to run in the two dozen test worlds and headless harnesses
/// that only ever wanted the red one. Two params, two queues, one rule about
/// keys — which is also the honest shape, since they are two event names the
/// interface registers for separately.
#[derive(SystemParam)]
pub struct UiInfo<'w> {
    strings: Res<'w, UiStrings>,
    writer: MessageWriter<'w, UiInfoMessage>,
}

impl UiInfo<'_> {
    /// **The same, in yellow** — `UI_INFO_MESSAGE` rather than
    /// `UI_ERROR_MESSAGE`.
    ///
    /// **Which of the two a key takes is a column in the client's own message
    /// table and not a judgement here**. The table's entries are `{key, type,
    /// sound, …}`, and the type has four cases: a dialog, this, the red one and
    /// a chat line. Cases 1 and 2 both raise an event — `UI_INFO_MESSAGE` (225)
    /// for one and `UI_ERROR_MESSAGE` (224) for the other. So **type 1 is this
    /// and type 2 is the red one**.
    ///
    /// `ERR_NEWTAXIPATH` — "New flight path discovered!" — has type 1, which is
    /// the whole of why the flight-path message is yellow.
    pub fn key(&mut self, key: &str) {
        let Some(text) = self.strings.get().and_then(|s| s.get(key)) else {
            return;
        };
        let text = text.to_string();
        self.writer.write(UiInfoMessage(text));
    }

}

/// **A sound a message asked for**, by `SoundEntries` name.
///
/// Raised here and played in [`crate::sound::messages`], for the reason every
/// other sound in this client crosses that line the same way: `game/` decides
/// *that* something is announced and `sound/` owns the one door to the mixer.
/// The name rather than the id, like [`crate::sound::player`]'s two — it is
/// what the table holds and what survives a patch.
#[derive(Message, Debug, Clone)]
pub struct MessageSound(pub &'static str);

/// **Say one of the game's own 343 messages**, letting the client's own table
/// decide everything about it except that it happened.
///
/// [`UiErrors`] and [`UiInfo`] make the caller choose a surface. For the keys
/// in [`vale_assets::interface::messages`] that choice is not the caller's
/// to make and never was: it is the `+0x04` column of the record, beside the
/// sound at `+0x08`, and this client was getting both wrong in the same two
/// places — `ERR_DECLINE_GROUP_S` and `ERR_NEW_LEADER_S` are chat lines the
/// error frame was flashing in red, and the decline's own
/// `igPlayerInviteDecline` had nothing playing it.
///
/// So this parameter takes a key and nothing else. Three surfaces because the
/// table names three, and the sound goes out **before** the text, which is the
/// order the client does it in — the sound plays before the string is
/// composed, so a message with no string still makes its noise.
///
/// **A key the table does not carry is still said, in red**, and whether that
/// is a mistake depends on the key. `GlobalStrings.lua` has 466 `ERR_*` keys
/// and this table indexes 343 of them, so thirteen that this client legitimately
/// raises — the master-looter family, `ERR_OBJECT_IS_BUSY`,
/// `ERR_NOT_WHILE_DISARMED` — simply have no surface column, and red is the only
/// thing they can be: chat and yellow are *opt-in* columns and 200 of the 343
/// rows take red anyway. A key that is not an `ERR_` at all is different — the
/// `SPELL_FAILED_*` family and `PLAYER_LOGOUT_FAILED_ERROR` belong on
/// [`UiErrors`] and reaching here with one is this client's mistake, so that
/// case says so.
#[derive(SystemParam)]
pub struct Announce<'w> {
    strings: Res<'w, UiStrings>,
    error: MessageWriter<'w, UiErrorMessage>,
    info: MessageWriter<'w, UiInfoMessage>,
    chat: MessageWriter<'w, crate::game::events::ChatMessageReceived>,
    sound: MessageWriter<'w, MessageSound>,
}

impl Announce<'_> {
    /// **Every queue [`Announce`] writes to**, registered in one call.
    ///
    /// Bevy validates a `SystemParam`'s message queues before the system runs
    /// and *panics* on one that is not initialised, so a world holding a system
    /// that announces has to hold all four — which is the same trap
    /// [`UiInfo`]'s note is about, one surface further along. Four test worlds
    /// listed the red queue by hand and took the whole suite down the first
    /// time a system they run started using the table.
    ///
    /// A function rather than four lines at each call site because the number
    /// is going to change: the error-speech column is a fifth queue whenever it
    /// is wired up, and finding every world that needs it is not a search
    /// anybody should have to do twice. `add_message` is idempotent, so a world
    /// that already has some of them is unharmed.
    pub fn register(app: &mut App) -> &mut App {
        app.add_message::<UiErrorMessage>()
            .add_message::<UiInfoMessage>()
            .add_message::<crate::game::events::ChatMessageReceived>()
            .add_message::<MessageSound>()
    }
}

impl Announce<'_> {
    /// The game's own line for `key`, on the surface the table names.
    pub fn key(&mut self, key: &str) {
        let text = self.strings.get().and_then(|s| s.get(key)).map(str::to_string);
        self.say(key, text);
    }

    /// The same, with the one argument the `%s` and `%d` keys take filled in.
    pub fn formatted(&mut self, key: &str, argument: &str) {
        let text = self.strings.get().and_then(|s| s.format(key, argument));
        self.say(key, text);
    }

    /// **A line the caller has already composed**, said on `key`'s own surface.
    ///
    /// [`Self::formatted`] fills exactly one `%s`, which is the shape of most
    /// of the table — but not of the two families that most want this: a quest
    /// objective is `"%s slain: %d/%d"` and a discovery with experience is
    /// `"Discovered %s: %d experience gained"`. Their substitution is the
    /// caller's, because it needs a creature name or an amount that this bundle
    /// has no access to; **the surface is still the table's**, which is the
    /// whole point of routing through here rather than writing to a queue
    /// directly.
    ///
    /// The key is still looked up: the sound, and the choice of window, come
    /// off it. Passing a line whose key is not in the table falls back to the
    /// red frame and warns, exactly as [`Self::key`] does.
    pub fn composed(&mut self, key: &str, line: String) {
        self.say(key, Some(line));
    }

    /// **`"%s slain: %d/%d"` and its two siblings**, which is the one shape in
    /// this table that takes three arguments.
    ///
    /// `ERR_QUEST_ADD_KILL_SII`, `ERR_QUEST_ADD_FOUND_SII` and
    /// `ERR_QUEST_ADD_ITEM_SII` are a name and two counters, positional and in
    /// that order in all three — so this needs no printf, only the string out
    /// of the archive. **The wording is the game's**: a key the file does not
    /// carry says nothing rather than a sentence invented here, which is the
    /// same rule `leader_board_line` follows one layer up.
    pub fn counted(&mut self, key: &str, name: &str, have: u32, want: u32) {
        let text = self.strings.get().and_then(|s| s.get(key)).map(|format| {
            format
                .replacen("%s", name, 1)
                .replacen("%d", &have.to_string(), 1)
                .replacen("%d", &want.to_string(), 1)
        });
        self.say(key, text);
    }

    /// **The raw `GlobalStrings` value for a key**, for the two families whose
    /// substitution neither [`Self::formatted`] nor [`Self::counted`] can do.
    ///
    /// `LOOT_ITEM_SELF_MULTIPLE` is `"You receive loot: %sx%d."` — a link and a
    /// count, where `formatted` fills one argument and `counted` expects three.
    /// Rather than a fourth shape, the caller gets the string and fills it, and
    /// says the result through [`Self::loot`]. A key the archive does not carry
    /// answers `None` and the caller says nothing, which is the rule this whole
    /// module is written on.
    pub fn string(&self, key: &str) -> Option<String> {
        self.strings.get().and_then(|s| s.get(key)).map(str::to_string)
    }

    /// **A line the C client puts in the chat frame as a loot line**, on
    /// `CHAT_MSG_LOOT`.
    ///
    /// Not [`Self::composed`], which routes by the *message table* — these six
    /// keys are not in that table at all. They are plain `GlobalStrings` the C
    /// side composes and pushes into the chat frame itself, so the surface is
    /// stated here rather than looked up. See
    /// [`crate::game::character::received`], the only caller.
    pub fn loot(&mut self, line: String) {
        use vale_protocol::play::chat::ChatType;
        self.chat.write(crate::game::events::ChatMessageReceived {
            event: crate::game::session::chat::event_name(ChatType::Loot),
            text: line,
            author: String::new(),
            flag: "",
            channel: String::new(),
            ..Default::default()
        });
    }

    fn say(&mut self, key: &str, text: Option<String>) {
        use vale_assets::interface::messages::{message, Surface};
        use vale_protocol::play::chat::ChatType;

        let Some(entry) = message(key) else {
            if !key.starts_with("ERR_") {
                warn!("{key} is not a message-table key — say it through UiErrors instead");
            }
            if let Some(text) = text {
                self.error.write(UiErrorMessage(text));
            }
            return;
        };

        // The sound first, and unconditionally: the client plays it above the
        // point where a missing string gives up, so the five keys with no
        // string still announce themselves.
        if let Some(name) = entry.sound {
            self.sound.write(MessageSound(name));
        }

        // …and a key with no string shows nothing, which is the client's own
        // behaviour — see this module's header.
        let Some(text) = text else {
            return;
        };
        match entry.surface {
            Surface::Chat => {
                let kind = ChatType::from_code(entry.chat).unwrap_or(ChatType::System);
                self.chat.write(crate::game::events::ChatMessageReceived {
                    event: crate::game::session::chat::event_name(kind),
                    text,
                    author: String::new(),
                    flag: "",
                    channel: String::new(),
                    ..Default::default()
                });
            }
            Surface::Info => {
                self.info.write(UiInfoMessage(text));
            }
            // The fourth case is a chat line too and no shipped row takes it;
            // routing it to the red frame would be a silent invention, so it
            // goes where the other 200 do and says so in the table.
            Surface::Error | Surface::ChatVariant => {
                self.error.write(UiErrorMessage(text));
            }
        }
    }
}

/// **A message-table line a packet named, to be said on the table's own
/// surface.**
///
/// For the systems that cannot take [`Announce`] themselves: it writes four
/// queues, and a system already writing one of them (the chat queue, most
/// often, or the red frame through [`UiErrors`]) would hold two writers of it,
/// which Bevy refuses at the first frame. The key is a `'static` string
/// because every caller names a fixed key, and the lookup is [`Announce::key`]'s.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableLine(pub &'static str);

/// Say every [`TableLine`] written this frame.
fn say_table_lines(mut lines: MessageReader<TableLine>, mut say: Announce) {
    for TableLine(key) in lines.read() {
        say.key(key);
    }
}

pub struct MessagesPlugin;

impl Plugin for MessagesPlugin {
    fn build(&self, app: &mut App) {
        // Every surface [`Announce`] can reach, including the sound queue
        // `sound::messages` drains — registered here rather than in `sound/` so
        // that a headless world with no mixer in it still validates the
        // parameter.
        Announce::register(app);
        app.init_resource::<UiStrings>()
            .add_message::<TableLine>()
            .add_systems(Update, load_strings.in_set(super::GameSet))
            // **After everything in the set that writes one**, so a line a
            // packet named is said on the frame it arrived rather than the next.
            .add_systems(Update, say_table_lines.after(super::GameSet));
    }
}

/// Hold the game's own strings once the archives are open.
fn load_strings(assets: Res<GameAssets>, session: Res<Session>, mut strings: ResMut<UiStrings>) {
    // Deferred until there is a session, because opening the archive chain is
    // ~19 files and the login screen should paint first.
    if strings.0.is_some() || session.active.is_none() {
        return;
    }
    strings.0 = Some(assets.strings());
}

/// **Who a `$`-variable is about** — the character reading the line.
///
/// One struct rather than four arguments because every caller has all four and
/// the two `&str`s are easy to transpose.
#[derive(Debug, Clone, Copy, Default)]
pub struct Speaker<'a> {
    pub name: &'a str,
    /// The game's own word — "Warrior", "Night Elf".
    pub class: &'a str,
    pub race: &'a str,
}

/// **Substitute the `$` variables a quest's or an NPC's text carries.**
///
/// Neither the server nor the files do this: `quest_template.Details` is stored
/// with `$B$B` and `$N` in it and vmangos writes the column out verbatim, so
/// the client is the only thing that can. Untouched, they reach the parchment
/// as literals — "Hello there, $c." over a giver's head, and paragraphs run
/// together because `$B` *is* the line break.
///
/// Five forms, which is 1.12's whole set:
///
/// ```text
/// $B $b   a line break
/// $N $n   the character's name
/// $C $c   …their class, in the game's own word
/// $R $r   …and their race
/// $G a:b; the gendered pair — see below
/// ```
///
/// **`$G` is passed through**, deliberately and statedly: choosing between its
/// two halves needs the character's gender, which this client does not carry on
/// a unit (only the character list and the appearance bake do). A literal is
/// legible; picking the wrong half is a sentence that reads correctly and is
/// about somebody else.
///
/// An empty substitution is left as the variable rather than as a hole — a name
/// that has not arrived yet is a frame, not a fact.
pub fn substitute(text: &str, who: Speaker<'_>) -> String {
    let mut out = text.replace("$B", "\n").replace("$b", "\n");
    for (upper, lower, value) in [
        ("$N", "$n", who.name),
        ("$C", "$c", who.class),
        ("$R", "$r", who.race),
    ] {
        if !value.is_empty() {
            out = out.replace(upper, value).replace(lower, value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::interface::messages::Surface;
    use bevy::ecs::message::Messages as MessageQueue;
    use bevy::ecs::system::RunSystemOnce;

    /// Run one system that writes through [`UiErrors`] and collect what came out
    /// the other side.
    fn written<M>(table: &str, run: impl IntoSystem<(), (), M>) -> Vec<String> {
        let mut world = World::new();
        world.insert_resource(UiStrings(Some(Arc::new(Strings::parse(table.as_bytes())))));
        world.init_resource::<MessageQueue<UiErrorMessage>>();
        world.run_system_once(run).expect("the system runs");
        let queue = world.resource::<MessageQueue<UiErrorMessage>>();
        let mut cursor = queue.get_cursor();
        cursor.read(queue).map(|m| m.0.clone()).collect()
    }

    /// Run one system that writes through [`Announce`] and collect what came
    /// out of **one** of its three surfaces — which is the whole point: the
    /// window is the assertion.
    fn said<M>(
        table: &str,
        surface: Surface,
        run: impl IntoSystem<(), (), M>,
    ) -> Vec<String> {
        let mut world = World::new();
        world.insert_resource(UiStrings(Some(Arc::new(Strings::parse(table.as_bytes())))));
        world.init_resource::<MessageQueue<UiErrorMessage>>();
        world.init_resource::<MessageQueue<UiInfoMessage>>();
        world.init_resource::<MessageQueue<crate::game::events::ChatMessageReceived>>();
        world.init_resource::<MessageQueue<MessageSound>>();
        world.run_system_once(run).expect("the system runs");
        match surface {
            Surface::Info => {
                let queue = world.resource::<MessageQueue<UiInfoMessage>>();
                let mut cursor = queue.get_cursor();
                cursor.read(queue).map(|m| m.0.clone()).collect()
            }
            Surface::Chat => {
                let queue = world.resource::<MessageQueue<crate::game::events::ChatMessageReceived>>();
                let mut cursor = queue.get_cursor();
                cursor.read(queue).map(|m| m.text.clone()).collect()
            }
            _ => {
                let queue = world.resource::<MessageQueue<UiErrorMessage>>();
                let mut cursor = queue.get_cursor();
                cursor.read(queue).map(|m| m.0.clone()).collect()
            }
        }
    }

    /// **A missing key is silence, not a placeholder.** Three of the game's own
    /// cast-failure reasons have no string at all, and the client shows nothing
    /// for them — so putting "SPELL_FAILED_HUNGER_SATIATED" on screen would be
    /// this client inventing an interface the game does not have.
    #[test]
    fn a_key_the_game_does_not_ship_shows_nothing() {
        let table = r#"ERR_BADATTACKPOS = "You are too far away!";"#;
        assert!(written(table, |mut errors: UiErrors| {
            errors.key("SPELL_FAILED_HUNGER_SATIATED");
        })
        .is_empty());

        assert_eq!(
            written(table, |mut errors: UiErrors| errors.key("ERR_BADATTACKPOS")),
            vec!["You are too far away!"]
        );
    }

    /// The one argument the `%s` keys take is filled from the caller.
    #[test]
    fn a_formatted_key_takes_its_argument() {
        let table = r#"SPELL_FAILED_REQUIRES_SPELL_FOCUS = "Requires %s";"#;
        assert_eq!(
            written(table, |mut errors: UiErrors| {
                errors.formatted("SPELL_FAILED_REQUIRES_SPELL_FOCUS", "Anvil");
            }),
            vec!["Requires Anvil"]
        );
    }

    /// **The `$` variables a quest's own text carries**, which nothing else in
    /// the stack resolves: the column is stored with them and the server writes
    /// it out verbatim, so untouched they reach the parchment as literals —
    /// `$B$B` between paragraphs and "Hello there, $c." over a giver's head,
    /// both of which have been photographed.
    #[test]
    fn the_dollar_variables_are_the_clients_to_substitute() {
        let who = Speaker {
            name: "Biggay",
            class: "Warrior",
            race: "Human",
        };
        assert_eq!(
            substitute("Hello there, $c.$B$BI need you, $N the $r.", who),
            "Hello there, Warrior.\n\nI need you, Biggay the Human.",
        );
        // Both cases of each, which is what the shipped rows actually mix.
        assert_eq!(substitute("$n $N $b$B $C $c", who), "Biggay Biggay \n\n Warrior Warrior");
        // **`$G` is passed through**, statedly: choosing a half needs a gender
        // this client does not carry on a unit, and the wrong half is a
        // sentence that reads correctly about somebody else.
        assert_eq!(substitute("$Glad:lass; $N", who), "$Glad:lass; Biggay");
        // …and an unknown speaker leaves the variable rather than a hole: a
        // name that has not arrived yet is a frame, not a fact.
        assert_eq!(
            substitute("Hello, $N.$BSit.", Speaker::default()),
            "Hello, $N.\nSit.",
        );
        assert_eq!(substitute("plain text", who), "plain text");
    }

    /// **Nothing is said before the archives are open.** `load_strings` waits for
    /// a session, so every call in the meantime has no table to look in — and it
    /// must be silent rather than panic, because a refusal can arrive on the first
    /// frame after login.
    #[test]
    fn a_message_before_the_strings_load_is_silent() {
        let mut world = World::new();
        world.init_resource::<UiStrings>();
        world.init_resource::<MessageQueue<UiErrorMessage>>();
        world
            .run_system_once(|mut errors: UiErrors| errors.key("ERR_BADATTACKPOS"))
            .expect("the system runs");
        assert_eq!(world.resource::<MessageQueue<UiErrorMessage>>().len(), 0);
    }

    // --- [`Announce`], and the three surfaces the table names ----------------

    /// What one key put through [`Announce`] came out as: the red line, the
    /// yellow line, the chat line and its event name, and the sound.
    #[derive(Debug, Default, PartialEq)]
    struct Said {
        red: Vec<String>,
        yellow: Vec<String>,
        chat: Vec<(&'static str, String)>,
        sounds: Vec<&'static str>,
    }

    fn announced(table: &str, key: &str, argument: Option<&str>) -> Said {
        let mut world = World::new();
        world.insert_resource(UiStrings(Some(Arc::new(Strings::parse(table.as_bytes())))));
        world.init_resource::<MessageQueue<UiErrorMessage>>();
        world.init_resource::<MessageQueue<UiInfoMessage>>();
        world.init_resource::<MessageQueue<crate::game::events::ChatMessageReceived>>();
        world.init_resource::<MessageQueue<MessageSound>>();
        let key = key.to_string();
        let argument = argument.map(str::to_string);
        world
            .run_system_once(move |mut say: Announce| match &argument {
                Some(argument) => say.formatted(&key, argument),
                None => say.key(&key),
            })
            .expect("the system runs");

        let drain = |world: &World| {
            let red = world.resource::<MessageQueue<UiErrorMessage>>();
            let yellow = world.resource::<MessageQueue<UiInfoMessage>>();
            let chat = world.resource::<MessageQueue<crate::game::events::ChatMessageReceived>>();
            let sounds = world.resource::<MessageQueue<MessageSound>>();
            Said {
                red: red.get_cursor().read(red).map(|m| m.0.clone()).collect(),
                yellow: yellow.get_cursor().read(yellow).map(|m| m.0.clone()).collect(),
                chat: chat
                    .get_cursor()
                    .read(chat)
                    .map(|m| (m.event, m.text.clone()))
                    .collect(),
                sounds: sounds.get_cursor().read(sounds).map(|s| s.0).collect(),
            }
        };
        drain(&world)
    }

    /// **The three surfaces, from three real records**, each of which this
    /// client was getting wrong before the table existed: the party decline was
    /// a red error rather than a chat line *and* silent, the flight-path
    /// discovery was yellow but silent, and the attack refusal was already
    /// right — which is worth pinning too, since a router that moved everything
    /// would be as wrong as one that moved nothing.
    #[test]
    fn the_table_picks_the_surface_and_the_sound() {
        let table = concat!(
            "ERR_DECLINE_GROUP_S = \"%s declines your group invitation.\";\n",
            "ERR_NEWTAXIPATH = \"New flight path discovered!\";\n",
            "ERR_BADATTACKPOS = \"You are too far away!\";\n",
        );

        // Record 68: a chat line, as CHAT_MSG_SYSTEM, with a sound.
        assert_eq!(
            announced(table, "ERR_DECLINE_GROUP_S", Some("Bram")),
            Said {
                chat: vec![(
                    "CHAT_MSG_SYSTEM",
                    "Bram declines your group invitation.".to_string()
                )],
                sounds: vec!["igPlayerInviteDecline"],
                ..Said::default()
            }
        );

        // Record 242: yellow, with a sound.
        assert_eq!(
            announced(table, "ERR_NEWTAXIPATH", None),
            Said {
                yellow: vec!["New flight path discovered!".to_string()],
                sounds: vec!["TaxiNodeDiscovered"],
                ..Said::default()
            }
        );

        // Record 216: red, silent — and it has an error-speech line rather than
        // a sound, which is the exclusion the table carries.
        assert_eq!(
            announced(table, "ERR_BADATTACKPOS", None),
            Said {
                red: vec!["You are too far away!".to_string()],
                ..Said::default()
            }
        );
    }

    /// **A key with a sound and no string still makes its noise.** The client
    /// plays the sound and then gives up on an empty key, in that order — so the five records the game ships with no string are silent on
    /// screen and not silent in the speakers.
    #[test]
    fn the_sound_survives_a_key_with_no_string() {
        let said = announced("", "ERR_NEWTAXIPATH", None);
        assert_eq!(said.sounds, vec!["TaxiNodeDiscovered"]);
        assert_eq!(said.yellow, Vec::<String>::new());
        assert_eq!(said.red, Vec::<String>::new());
    }

    /// **A key outside the table is still said, in red.** Thirteen `ERR_*` keys
    /// this client raises have no record — the master-looter family among them
    /// — and red is the only thing they can be, since the other two surfaces
    /// are opt-in columns.
    #[test]
    fn a_key_with_no_record_falls_back_to_the_red_frame() {
        let table = r#"ERR_LOOT_MASTER_OTHER = "Item is being distributed.";"#;
        assert_eq!(
            announced(table, "ERR_LOOT_MASTER_OTHER", None),
            Said {
                red: vec!["Item is being distributed.".to_string()],
                ..Said::default()
            }
        );
    }

    /// **Every surface the message table can name, taken from one `Announce`**
    /// — which is the check the two reports this round were about: a quest
    /// objective and a discovery with no experience are `UI_INFO_MESSAGE`, and
    /// this client sent both to the wrong window because nothing crossed the
    /// key against the table.
    ///
    /// Three queues, so a line landing in the wrong one is a failure here
    /// rather than a screenshot.
    #[test]
    fn the_table_decides_the_window_and_not_the_caller() {
        let table = concat!(
            "ERR_ZONE_EXPLORED = \"Discovered: %s\";\n",
            "ERR_ZONE_EXPLORED_XP = \"Discovered %s: %d experience gained\";\n",
            "ERR_QUEST_ADD_KILL_SII = \"%s slain: %d/%d\";\n",
            "ERR_MAIL_TO_SELF = \"You can't send mail to yourself.\";\n",
        );
        // …the yellow half.
        assert_eq!(
            said(table, Surface::Info, |mut say: Announce| {
                say.composed("ERR_ZONE_EXPLORED", "Discovered: Northshire Valley".into());
            }),
            vec!["Discovered: Northshire Valley"],
            "a discovery that paid nothing is centre screen, not chat"
        );
        assert_eq!(
            said(table, Surface::Info, |mut say: Announce| {
                say.counted("ERR_QUEST_ADD_KILL_SII", "Kobold Vermin", 3, 8);
            }),
            vec!["Kobold Vermin slain: 3/8"],
            "an objective moving is centre screen"
        );
        // …the chat half, from the *same* family.
        assert_eq!(
            said(table, Surface::Chat, |mut say: Announce| {
                say.composed(
                    "ERR_ZONE_EXPLORED_XP",
                    "Discovered Northshire Valley: 25 experience gained".into(),
                );
            }),
            vec!["Discovered Northshire Valley: 25 experience gained"],
            "…and one that paid is a chat line, which is the same table's other column"
        );
        // …and the red half, unchanged.
        assert_eq!(
            said(table, Surface::Error, |mut say: Announce| {
                say.key("ERR_MAIL_TO_SELF");
            }),
            vec!["You can't send mail to yourself."]
        );
    }

    /// **A key the archive does not carry says nothing**, on every one of the
    /// three doors — the rule the rest of this module already follows, restated
    /// for the two new ones because each is a fresh way in.
    #[test]
    fn the_two_new_doors_say_nothing_for_a_key_with_no_string() {
        let table = r#"ERR_BADATTACKPOS = "You are too far away!";"#;
        assert!(said(table, Surface::Info, |mut say: Announce| {
            say.counted("ERR_QUEST_ADD_KILL_SII", "Kobold Vermin", 3, 8);
        })
        .is_empty());
        // …and `composed` is the exception, deliberately: its caller has
        // *already* looked the string up, so there is nothing left to be
        // missing. What it still takes from the table is the window.
        assert_eq!(
            said(table, Surface::Error, |mut say: Announce| {
                say.composed("ERR_BADATTACKPOS", "You are too far away!".into());
            }),
            vec!["You are too far away!"]
        );
    }

}
