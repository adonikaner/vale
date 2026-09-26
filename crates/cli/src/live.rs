//! `vale live` — enter the world and stay there, with a command line.

use crate::common::*;
use vale_config::Config;
use vale_protocol::play::chat::ChatType;

/// Enter the world and *stay* there: a keepalive'd session you can walk around.
///
/// This is the headless twin of the GUI's live session, and the fastest way to
/// find out whether a movement packet is accepted. Commands are lines rather
/// than keys because a raw-mode terminal would be a dependency and a portability
/// problem for no benefit — the point is the protocol, not the input handling.
pub fn cmd_live(cfg: &Config, character: Option<&str>) -> Result<(), String> {
    use vale_assets::MapTerrain;
    use vale_protocol::state::movement::Controls;
    use vale_protocol::socket::session::LiveSession;
    use std::io::{BufRead, Write};
    use std::sync::Arc;
    use std::time::Duration;

    let (session, chosen) = open_world(cfg, character)?;
    println!(
        "[live] entering world as {} (guid {}) on map {}",
        chosen.name, chosen.guid, chosen.map
    );

    // Terrain gives the character ground to walk on. It is optional: without
    // game data the session still runs, the character just keeps its altitude.
    // Every map, not one — a far teleport changes which one is under the
    // character's feet.
    let ground = match MapTerrain::open(&cfg.gamedata) {
        Ok(terrain) => {
            match terrain.directory(chosen.map) {
                Some(name) => println!("[live] terrain from {name}"),
                None => eprintln!("[live] map {} is not in Map.dbc", chosen.map),
            }
            let terrain = Arc::new(terrain);
            // Terrain only: the CLI has no renderer, so nothing here has ever
            // loaded a building. The `z` the mover offers is therefore ignored
            // — with no WMO hulls there is only one surface over a point.
            Some(Box::new(move |map: u32, x: f32, y: f32, _z: f32| {
                terrain.height_at(map, x, y)
            }) as vale_protocol::socket::session::GroundHeight)
        }
        Err(e) => {
            eprintln!("[live] no terrain ({e}); the character will not follow the ground");
            None
        }
    };

    // **The volumes the character reports standing in** — how a dungeon portal
    // works at all; see `vale_protocol::play::areatrigger`. Optional like the
    // terrain above: without it the session runs and the portals do nothing,
    // which is the behaviour every session had before this existed.
    let triggers = match vale_assets::tables::areatrigger::AreaTriggers::open(&cfg.gamedata) {
        Ok(table) => {
            println!(
                "[live] {} area trigger(s), {} on this map",
                table.rows().len(),
                table.on_map(chosen.map).count()
            );
            Some(Box::new(Portals(table)) as vale_protocol::play::areatrigger::TriggerTable)
        }
        Err(e) => {
            eprintln!("[live] no area triggers ({e}); portals will do nothing");
            None
        }
    };

    // **What it takes to open something**, for the two game-object verbs — see
    // `vale_assets::tables::lock`. Optional on the same terms as the two
    // above, and the degradation is the pointer column: without it every door
    // and every ore vein reads as unlocked and shows the plain hand.
    load_locks(cfg);

    // The same `WDB\` the renderer writes, so a session spent here warms the
    // names for one spent there — and `bags` names what it holds on the first
    // read rather than a query interval later.
    let caches = vale_protocol::play::wdb::for_session(&session);
    println!("[live] wdb: {}", caches.summary());
    let live = LiveSession::spawn(session, chosen, ground, triggers, caches);
    live.wait_until_in_world(Duration::from_secs(20))?;
    println!("{}", live_help());

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or("").to_ascii_lowercase();
        let arg = words.next();
        // **Everything after the verb, unsplit** — which `sendmail` needs and
        // `arg` cannot give it: a subject line has spaces in it. The same
        // helper `say` uses.
        let rest = rest_of(&line);

        match command.as_str() {
            "" => {}
            "q" | "quit" | "exit" => break,
            "?" | "h" | "help" => println!("{}", live_help()),
            "stop" | "x" => live.set_controls(Controls::default()),
            "w" | "s" | "a" | "d" | "ql" | "qr" => {
                let mut controls = Controls::default();
                match command.as_str() {
                    "w" => controls.forward = true,
                    "s" => controls.backward = true,
                    "a" => controls.strafe_left = true,
                    "d" => controls.strafe_right = true,
                    "ql" => controls.turn_left = true,
                    _ => controls.turn_right = true,
                }
                live.set_controls(controls);
                // Holding a key with no release just walks off into the
                // distance while you type, so a duration is the default.
                if let Some(seconds) = arg.and_then(|a| a.parse::<f32>().ok()) {
                    std::thread::sleep(Duration::from_secs_f32(seconds.clamp(0.0, 60.0)));
                    live.set_controls(Controls::default());
                    print_where(&live);
                }
            }
            // **The fastest way to check the arc against the real server.** A
            // jump is four packets' worth of rules that nothing else exercises
            // — `MSG_MOVE_JUMP` with a legal `xyspeed`, one jump per landing, a
            // `fallTime` that counts, and a `MSG_MOVE_FALL_LAND` at the bottom
            // — and every one of them is a kick or a rejection when it is
            // wrong, with nothing said on this end. Watch `Anticheat.log`.
            "jump" | "j" => {
                live.jump();
                let start = live.status().position.z;
                let mut apex = start;
                for _ in 0..40 {
                    std::thread::sleep(Duration::from_millis(25));
                    let status = live.status();
                    apex = apex.max(status.position.z);
                    if !status.airborne {
                        break;
                    }
                }
                let end = live.status().position.z;
                println!(
                    "  jumped from z {start:.2}, apex {apex:.2} (+{:.2}), landed {end:.2}, {} movement packets",
                    apex - start,
                    live.status().movement_sent
                );
            }
            "face" => match arg.and_then(|a| a.parse::<f32>().ok()) {
                Some(degrees) => live.face(degrees.to_radians()),
                None => println!("usage: face <degrees>"),
            },
            "where" | "p" => print_where(&live),
            "near" | "n" => print_nearby(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(12)),
            "stat" => print_status(&live),
            // **The character sheet's own numbers**, off the same decode the
            // interface reads — see `print_sheet` for why this is the only
            // check the private half of the stat block can have.
            "sheet" => print_sheet(&live),
            // …and its other half: what is *in* the bags, which is the one
            // subject whose whole state is update fields nothing announces —
            // see `print_bags`.
            "bags" => print_bags(&live),
            // **The bank, from the socket up** — the only check the family
            // can have against a server: the window is a guid, the contents
            // are fields, and the two verbs only ever answer with a refusal.
            // See [`open_bank`].
            "bank" => open_bank(&live),
            "bankbuy" => bank_buy(&live),
            "deposit" => bank_move(&live, arg, false),
            "withdraw" => bank_move(&live, arg, true),
            // **The loot window, from the socket up** — the one panel neither
            // this driver nor any test could reach until now, which is why the
            // round that fixed its blank names could only reason about the
            // timing. See [`loot_nearest`].
            "loot" => loot_nearest(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1)),
            "take" => match arg.and_then(|a| a.parse::<usize>().ok()) {
                Some(row) => take_loot(&live, row),
                None => println!("  usage: take <row>   (1 is the first row, coins included)"),
            },
            // **Just listen.** The queue is drained by whichever loot verb ran
            // last and by nothing else, so "did anything arrive after that?" —
            // which is exactly the question the auto-close rule is about — needs
            // a verb that only reads.
            "lootq" => drain_loot(&live, arg.and_then(|a| a.parse::<u64>().ok()).unwrap_or(5) * 1000),
            // **...and the group's half of it**, which is the one part of the
            // loot family that needs a second character and could not be
            // reached at all until now: `SMSG_LOOT_START_ROLL` only ever
            // arrives when a party is standing over the body. See [`roll_on`].
            "roll" => roll_on(&live, arg),
            // **The game objects, and the one verb that touches them.** The same
            // argument as the loot pair beside it: nothing else in this project
            // can show what the server does with a `CMSG_GAMEOBJ_USE`, and the
            // four things it can do (change a state, open a loot window, cast a
            // gathering spell at us, open a quest page) each arrive down a
            // different path with no reply to the packet itself.
            // **The mailbox, which is the one window in the game a packet does
            // not open.** `GAMEOBJECT_TYPE_MAILBOX` has an empty arm in
            // `GameObject::Use`, so `use` on one does nothing at all and there
            // is no other way to see what a real inbox carries: the header's
            // sender field is a union three bytes wide in three different ways,
            // and only a live server can produce the second and third.
            "mail" => open_mailbox(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1)),
            // …and one letter's words, which are a second round trip and the
            // one thing that marks a letter read.
            "letter" => match arg.and_then(|a| a.parse::<usize>().ok()) {
                Some(row) => read_letter(&live, row),
                None => println!("  usage: letter <row>   (1 is the first letter)"),
            },
            // …and taking what is in it, which is where `SMSG_SEND_MAIL_RESULT`
            // shows its conditional tail: a successful item take appends a guid
            // and a count and nothing else does.
            "maildel" => mail_verb(&live, arg, MailKind::Delete),
            "mailret" => mail_verb(&live, arg, MailKind::Return),
            "mailtake" => mail_verb(&live, arg, MailKind::TakeItem),
            // **The trade window, from the socket up** — the only check the
            // family can have, and it wants two characters: one here and one
            // at a renderer or a second `live`. See [`trade_with`].
            "trade" => trade_with(&live, arg),
            "tradeitem" => trade_item(&live, arg),
            "tradegold" => trade_verb(
                &live,
                arg.and_then(|a| a.parse::<u32>().ok())
                    .map(vale_protocol::socket::session::TradeVerb::SetGold),
                "usage: tradegold <copper>",
            ),
            "tradeok" => trade_verb(
                &live,
                Some(vale_protocol::socket::session::TradeVerb::Accept),
                "",
            ),
            "tradeyes" => trade_verb(
                &live,
                Some(vale_protocol::socket::session::TradeVerb::Begin),
                "",
            ),
            "tradecancel" => trade_verb(
                &live,
                Some(vale_protocol::socket::session::TradeVerb::Cancel),
                "",
            ),
            "tradeq" => drain_trade(&live, arg.and_then(|a| a.parse::<u64>().ok()).unwrap_or(5) * 1000),
            "mailmoney" => mail_verb(&live, arg, MailKind::TakeMoney),
            // …and sending one, which is the only way to see the nine trailing
            // bytes `CMSG_SEND_MAIL` carries actually being accepted.
            "sendmail" => send_letter(&live, rest),
            "objects" | "obj" => list_objects(&live),
            "use" => use_nearest(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1)),
            "release" => {
                let guid = LOOT_GUID.with(|g| g.get());
                if guid == 0 {
                    println!("  no window open");
                } else {
                    live.loot_release(guid);
                    drain_loot(&live, 1000);
                }
            }
            // Attack the nth-nearest living unit, and then watch it. The point
            // is not the combat: an attacking creature is the only source of a
            // chase spline and of `UPDATEFLAG_MELEE_ATTACKING`, and neither can
            // be observed by walking around.
            "attack" => attack_nearest(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1)),
            "unattack" => live.attack(None),
            // **The three commands the player controls added**, and each is here
            // for the same reason `attack` is: it makes the server say something
            // it otherwise never says. `spells` is the login burst's own
            // `SMSG_INITIAL_SPELLS`, which nothing else in this harness can see;
            // `target` is `CMSG_SET_SELECTION` and the `UNIT_FIELD_TARGET` that
            // comes back on us; `cast` is the whole outbound chain — the aiming
            // rule, the packet, and `SMSG_CAST_RESULT` in the game's own words.
            "target" | "t" => {
                target_nearest(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1))
            }
            "untarget" => {
                live.target(None);
                println!("  selection cleared");
            }
            "spells" => print_spellbook(cfg, &live),
            "cast" => match arg.and_then(|a| a.parse::<u32>().ok()) {
                Some(spell_id) => cast_spell(cfg, &live, spell_id),
                None => println!("usage: cast <spellId>   (`spells` lists what this character knows)"),
            },
            // Walk up to the nth-nearest living unit and watch it, *without*
            // swinging. The distinction earns its own command: a geared level
            // 60 kills the things it can reach in one blow, so `attack` gets
            // one frame of a live opponent, where standing next to an
            // aggressive one of your own level gets a fight that lasts — and a
            // fight that lasts is the only way to see a creature turn.
            "goto" | "g" => goto_nearest(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1)),
            // Follow one entity for a few seconds, printing every time the
            // server moves it. A position that jumps hundreds of yards is a
            // parse bug; one that drifts is the dead reckoning.
            "watch" => watch_entity(&live, arg.and_then(|a| a.parse().ok()).unwrap_or(1), 10),
            // **A leading `.` is not a client command.** The server parses it out
            // of an ordinary say (`ProcessChatMessageAfterSecurityCheck` ->
            // `ChatHandler::ParseCommands`), so `.tele tanaris` goes out as chat
            // and comes back as `CHAT_MSG_SYSTEM`. It is also the only way this
            // client can currently be somewhere else on purpose.
            c if c.starts_with('.') => live.say(ChatType::Say, None, line.trim().to_string()),
            "say" | "yell" | "emote" => match rest_of(&line) {
                Some(text) => live.say(
                    match command.as_str() {
                        "yell" => ChatType::Yell,
                        "emote" => ChatType::Emote,
                        _ => ChatType::Say,
                    },
                    None,
                    text.to_string(),
                ),
                None => println!("usage: {command} <text>"),
            },
            // **The one way to make an `SMSG_EMOTE` happen.** Without it the
            // packet only ever arrives about somebody else, so "this client
            // does not animate emotes" and "nobody emoted" are the same empty
            // screen — the same reason `attack` exists.
            //
            // The id is an `EmotesText.dbc` row and comes back as an
            // `Emotes.dbc` one: 34 `TEXTEMOTE_DANCE` returns row 10
            // `STATE_DANCE`, which `vale emote` resolves to animation 69
            // `EmoteDance`.
            "dance" | "wave" | "bow" | "textemote" => {
                let id = match command.as_str() {
                    "dance" => Some(34),
                    "wave" => Some(101),
                    "bow" => Some(17),
                    _ => rest_of(&line).and_then(|r| r.parse::<u32>().ok()),
                };
                match id {
                    Some(id) => {
                        live.text_emote(id, 0, 0);
                        println!("  sent EmotesText {id}; watch `stat` for the SMSG_EMOTE back");
                    }
                    None => println!("usage: textemote <EmotesText.dbc id>"),
                }
            }
            "whisper" => match rest_of(&line).and_then(|r| r.split_once(char::is_whitespace)) {
                Some((who, text)) => {
                    live.say(ChatType::Whisper, Some(who.to_string()), text.trim().to_string())
                }
                None => println!("usage: whisper <name> <text>"),
            },
            "chat" => println!("(chat is printed as it arrives)"),
            other => println!("unknown command {other:?} — try `help`"),
        }

        // Drained here rather than on its own thread, because this loop blocks on
        // stdin: a line that arrives while nobody is typing is shown at the next
        // prompt. It is still shown exactly once — see `ObjectManager::take_chat`.
        print_chat(&live);

        if !live.is_running() {
            break;
        }
        print!("> ");
        std::io::stdout().flush().ok();
    }

    let status = live.status();
    if let Some(e) = status.error {
        return Err(format!("session ended: {e}"));
    }
    println!("[live] leaving the world.");
    Ok(())
}

fn live_help() -> &'static str {
    "\ncommands (a number is how many seconds to hold the key):\n\
     \x20 w|s|a|d [secs]   forward / back / strafe left / strafe right\n\
     \x20 ql|qr [secs]     turn left / right\n\
     \x20 stop             release everything\n\
     \x20 jump|j           jump, and report the arc it flew\n\
     \x20 face <degrees>   turn to an absolute facing\n\
     \x20 where            position, tile and heading\n\
     \x20 near [n]         the nearest entities\n\
     \x20 attack [n]       walk to the nth nearest unit, swing, and watch it\n\
     \x20 unattack         stop attacking\n\
     \x20 target|t [n]     select the nth nearest unit, and report what came back\n\
     \x20 untarget         clear the selection\n\
     \x20 spells           the spellbook the login burst stated, resolved\n\
     \x20 cast <spellId>   aim it by the game's own rule, send it, print the reply\n\
     \x20 goto|g [n]       walk to the nth nearest unit and watch it, no blow\n\
     \x20 watch [n]        follow the nth nearest unit for ten seconds\n\
     \x20 say|yell|emote <text>\n\
     \x20 dance|wave|bow   play a text emote — the only way to see SMSG_EMOTE\n\
     \x20 textemote <id>   any EmotesText.dbc row\n\
     \x20 whisper <name> <text>\n\
     \x20 .<command>       a GM command — `.tele tanaris`, `.gm on`\n\
     \x20 stat             packets, latency, warnings\n\
     \x20 sheet            the character sheet's numbers, as the panel reads them\n\
     \x20 bags             every worn slot, every bag and every stack in it\n\
     \x20 bank             walk to the nearest banker, open the bank, print its squares\n\
     \x20 bankbuy          buy the next bag slot; deposit <bag> <slot> | withdraw <slot>\n\
     \x20 loot [n]          open the nth nearest corpse, and print the rows\n\
     \x20 take <row>        take one row — the coins are row 1 when there are any\n\
     \x20 release           let the body go\n\
     \x20 roll [need|greed|pass]  vote in the group roll the server just opened\n\
     \x20 lootq [secs]      just read the loot queue — what arrived, and what did not\n\
     \x20 objects|obj       every game object in view: type, lock, and what a click would do\n\
     \x20 use [n]           walk to the nth nearest usable one and click it\n\
     \x20 mail [n]          walk to the nth nearest mailbox and read the inbox\n\
     \x20 letter <row>      one letter's words, and the mark-as-read beside them\n\
     \x20 mailtake|mailmoney|mailret|maildel <row>   the four buttons on one\n\
     \x20 sendmail <name> <subject>   post one, on the default parchment\n\
     \x20 trade <name>     ask a player in view to trade; tradeyes answers one\n\
     \x20 tradeitem <bag> <slot> [n]   offer a bag square in trade slot n (1..7)\n\
     \x20 tradegold <copper> | tradeok | tradecancel | tradeq [secs]\n\
     \x20 quit\n"
}

/// Everything after the first word, or `None` if there is nothing there.
///
/// Taken off the raw line rather than from the `split_whitespace` iterator,
/// because a message is not a list of words: `say  hello   world` must arrive as
/// typed apart from its surrounding space.
fn rest_of(line: &str) -> Option<&str> {
    let rest = line.trim().split_once(char::is_whitespace)?.1.trim();
    (!rest.is_empty()).then_some(rest)
}

/// Show whatever chat has arrived, in the 1.12 client's own phrasing.
fn print_chat(live: &vale_protocol::socket::session::LiveSession) {
    for (who, message) in live.take_chat() {
        let kind = message.chat_type();
        // System output has no speaker, and an emote reads as one sentence
        // ("Rex yawns") rather than as somebody saying something.
        let line = match (kind, who.is_empty()) {
            (_, true) => message.text.clone(),
            (Some(ChatType::Emote | ChatType::MonsterEmote | ChatType::TextEmote), _) => {
                format!("{who} {}", message.text)
            }
            (Some(k), _) if !k.prefix().is_empty() => {
                format!("{who} {}: {}", k.prefix(), message.text)
            }
            _ => format!("{who}: {}", message.text),
        };
        println!("[chat] {line}");
    }
}

thread_local! {
    /// **Whose body the window is on**, so `take` and `release` need no argument.
    ///
    /// A thread-local rather than a field on anything, because `vale live` is
    /// a `for line in stdin` loop with no state of its own — every other verb is
    /// stateless and this is the one pair that is not.
    static LOOT_GUID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// **Right-click the nth nearest corpse.**
///
/// The whole of why this exists: `SMSG_LOOT_RESPONSE` names no item — every row
/// is an entry and a display id — so the loot window is the one place in the
/// client where a name arrives *after* the panel drew, and until this verb there
/// was no way to exercise that without a person at the keyboard. See
/// `crate::game::npc::loot` in the renderer, whose two rules (the window is held
/// shut until its rows can be named, and the last row taken closes it) are what
/// this checks.
///
/// **The nearest *corpse*, which is a different set from `attack`'s.** A body
/// worth opening is dead and is flagged lootable for us — `UNIT_DYNAMIC_FLAGS`
/// bit 0 is per-viewer, so it is already the honest answer to "would a
/// right-click do anything".
fn loot_nearest(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    use vale_protocol::state::update::ObjectType;
    let me = live.status().position;
    let target = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        let mut bodies: Vec<_> = world
            .iter()
            .filter(|e| e.object_type == Some(ObjectType::Unit) && !e.is_alive())
            .filter_map(|e| {
                let p = e.position?;
                let d = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2)).sqrt();
                Some((d, e.guid, world.name_of(e), e.lootable()))
            })
            .collect();
        bodies.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        bodies.into_iter().nth(n.saturating_sub(1))
    };
    let Some((distance, guid, name, lootable)) = target else {
        println!("  no corpse nearby");
        return;
    };
    println!(
        "  opening {name} ({guid:#x}) at {distance:.1}y — UNIT_DYNAMIC_FLAGS says {}",
        if lootable { "there is something on it" } else { "it is empty" }
    );
    // **Walk to it first.** `HandleLootOpcode` refuses past `INTERACTION_DISTANCE`
    // and answers with an `ERR_LOOT_TOO_FAR` wearing the same opcode, which is a
    // real answer and a confusing one to read as a bug.
    if distance > 4.0 {
        close_on(live, guid);
    }
    live.loot(guid);
    LOOT_GUID.with(|g| g.set(guid));
    drain_loot(live, 1500);
}

thread_local! {
    /// `Lock.dbc` joined to `LockType.dbc`, read once per session.
    ///
    /// A thread-local for the reason [`LOOT_GUID`] is one: `vale live` is a
    /// `for line in stdin` loop with nowhere to hang state. Read at start-up
    /// rather than per command, because the two verbs that want it are typed
    /// repeatedly and the DBC chain is not free.
    static OBJECT_LOCKS: std::cell::RefCell<vale_assets::tables::lock::Locks> =
        std::cell::RefCell::new(vale_assets::tables::lock::Locks::default());
    /// …and `Spell.dbc`'s own half of the same question: which spell opens which
    /// kind of lock. Read beside it, because neither answers anything alone.
    static OPEN_LOCK_SPELLS: std::cell::RefCell<Option<vale_assets::tables::spellbook::Spells>> =
        const { std::cell::RefCell::new(None) };
}

/// Read the two lock tables into [`OBJECT_LOCKS`], or leave them empty and say
/// so — an empty table is every game object reading as unlocked, which is a
/// worse pointer rather than a broken one.
fn load_locks(cfg: &Config) {
    use vale_assets::tables::dbc::dbc_path;
    let mut assets = match open_assets(cfg) {
        Ok(assets) => assets,
        Err(e) => {
            eprintln!("[live] no archives ({e}); game object locks will read as unlocked");
            return;
        }
    };
    let locks = vale_assets::tables::lock::Locks::parse(
        &assets.read(&dbc_path("Lock")).unwrap_or_default(),
        &assets.read(&dbc_path("LockType")).unwrap_or_default(),
    );
    let (rows, names) = locks.counts();
    OBJECT_LOCKS.with(|cell| *cell.borrow_mut() = locks);
    // `Spell.dbc` alone: none of the six tables the renderer's catalogue also
    // wants matter to this question, and each is a file read a CLI session does
    // not need to pay for.
    let spells = vale_assets::tables::spellbook::Spells::parse(
        &assets.read(&dbc_path("Spell")).unwrap_or_default(),
        &[],
        &[],
        &[],
        &[],
        &[],
        &[],
    )
    .ok();
    println!(
        "[live] {rows} lock(s), {names} lock type(s) named, {} lock type(s) have an opening spell",
        spells.as_ref().map_or(0, |s| s.opener_kinds())
    );
    OPEN_LOCK_SPELLS.with(|cell| *cell.borrow_mut() = spells);
}

/// **Every game object in view**, with what the client makes of it.
///
/// The pointer column is the whole of what the renderer's cursor decides, run
/// through the same [`vale_assets::look::object`] rule — so a row saying
/// `Mine` here is a mining pick on the cursor there, and the two cannot drift.
fn list_objects(live: &vale_protocol::socket::session::LiveSession) {
    let Some(rows) = nearby_objects(live) else {
        println!("  no game objects in view");
        return;
    };
    println!("  {} game objects in view:", rows.len());
    println!("     n   dist  entry  type  lock  state  pointer        verb         name");
    for (n, row) in rows.iter().enumerate().take(30) {
        println!(
            "    {:>2}  {:>5.1}  {:>5}  {:>4}  {:>4}  {:>5}  {:<13}  {:<11}  {}",
            n + 1,
            row.distance,
            row.entry,
            row.object_type,
            row.lock,
            row.state.map_or("-".to_string(), |s| s.to_string()),
            row.pointer,
            // **Blank for a row a click would never reach**, which is most of
            // them: the zone markers, the anvils, the transports. Printing
            // "use" against those reads as a promise nothing keeps.
            // **Three answers, not two.** A sign is `look`: hovered, never
            // clicked, and its plate follows the pointer. Printing "-" for one
            // is what hid the whole population the first time round.
            match (row.usable, row.cast, row.hover.floating) {
                (false, _, true) => "look".to_string(),
                (false, _, false) => "-".to_string(),
                (true, Some(spell), _) => format!("cast {spell}"),
                (true, None, _) => "use".to_string(),
            },
            row.name
        );
    }
    if rows.len() > 30 {
        println!("    … and {} more", rows.len() - 30);
    }
}

/// **Walk up to the nth nearest *usable* game object and click it.**
///
/// Usable rather than nearest-of-all, because a zone is carpeted with invisible
/// type-5 markers the pointer never lands on, and they would otherwise fill
/// every row of the numbering.
///
/// **The walk is not optional.** `HandleGameObjectUseOpcode` refuses past
/// `INTERACTION_DISTANCE` and says so through the message table rather than by
/// answering this packet, so a use sent from across a field looks exactly like
/// a use that was ignored.
fn use_nearest(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    let Some(rows) = nearby_objects(live) else {
        println!("  no game objects in view");
        return;
    };
    // **`use` numbers the ones a click acts on**, which is not what `objects`
    // lists: a street sign is in that list and can never be the target of this
    // verb, because `HandleGameObjectUseOpcode` refuses the type outright.
    let usable: Vec<&ObjectRow> = rows.iter().filter(|r| r.usable).collect();
    let Some(row) = usable.get(n.saturating_sub(1)) else {
        println!("  only {} usable game object(s) in view", usable.len());
        return;
    };
    println!(
        "  closing on {} ({:.1}y, guid {:#x}, entry {}, type {}, lock {})",
        row.name, row.distance, row.guid, row.entry, row.object_type, row.lock
    );
    let guid = row.guid;
    if row.distance > MELEE_RANGE {
        close_on(live, guid);
    }
    // **Three observables, because a use has no reply and lands in three
    // different places.** The object's own state is a door or a lever; the
    // *player's* stand state is a chair — `GameObject::Use` seats the user and
    // touches nothing on the chair at all, which is what made a working chair
    // read as a dead packet the first time this was run; and the loot queue is a
    // chest, a vein or a herb.
    let watch = || {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        let object = world.get(guid).and_then(|e| e.game_object_state());
        let mine = world
            .iter()
            .find(|e| e.is_self)
            .and_then(|e| e.stand_state());
        (object, mine)
    };
    let before = watch();
    // **Which of the two verbs this object's own kind takes** — see
    // `vale_assets::look::object`, where the server arm that ignores a use
    // packet on a chest is quoted. Resolved here rather than assumed, and
    // printed, because "which packet went out" is the whole of what this verb
    // is for.
    match row.cast {
        Some(spell) => {
            println!("  casting spell {spell} at it (a chest is opened by a cast, not by a use)");
            live.cast(spell, vale_protocol::play::spells::CastTarget::Object(guid));
        }
        None => {
            println!("  sending CMSG_GAMEOBJ_USE");
            live.use_object(guid);
        }
    }
    LOOT_GUID.with(|g| g.set(guid));
    // **Long enough for a gathering cast to finish**, which is the slowest of
    // the four answers: a herb is two seconds of casting and then a round trip,
    // where a door's state change is immediate. A shorter wait reports "nothing
    // came back" for a use that worked perfectly.
    drain_loot(live, 8000);
    let after = watch();
    let moved = |what: &str, a: Option<u8>, b: Option<u8>| {
        println!(
            "  {what} {a:?} -> {b:?}{}",
            if a == b { "" } else { "   <- it moved" }
        );
    };
    moved("GAMEOBJECT_STATE ", before.0, after.0);
    moved("our own stand state", before.1, after.1);
    if before == after {
        println!("  nothing moved — a chest, a vein, or a refusal");
    }
}

/// One game object as both verbs above want it.
struct ObjectRow {
    guid: u64,
    entry: u32,
    name: String,
    object_type: u32,
    lock: u32,
    state: Option<u8>,
    distance: f32,
    usable: bool,
    /// **What hovering it is worth**, which is the other question — see
    /// `vale_assets::look::object::hover_of`. A street sign is `look`.
    hover: vale_assets::look::object::Hover,
    pointer: String,
    /// The spell a click would cast at it, for the two kinds that are opened
    /// that way. `None` is "the use packet".
    cast: Option<u32>,
}

/// Every game object in the world, nearest first, judged by the client's own
/// rule. `None` when there are none.
fn nearby_objects(live: &vale_protocol::socket::session::LiveSession) -> Option<Vec<ObjectRow>> {
    use vale_assets::look::object::{opener, over_object, Kind};
    use vale_protocol::state::update::ObjectType;

    let locks = OBJECT_LOCKS.with(|cell| cell.borrow().clone());
    let me = live.status().position;
    // The character's own book, which is what decides whether a vein is
    // gatherable at all: the ore is the same, the spell is the player's.
    let known = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.spellbook.known.clone()
    };
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let mut rows: Vec<ObjectRow> = world
        .iter()
        .filter(|e| e.object_type == Some(ObjectType::GameObject))
        .filter_map(|e| {
            let p = e.position?;
            let info = world.gameobject_of(e);
            let object_type = info.map_or(0, |i| i.object_type);
            let kind = Kind::of(object_type);
            let lock = info
                .and_then(|i| Some(*i.data.get(kind.lock_word()?)?))
                .unwrap_or(0);
            Some(ObjectRow {
                guid: e.guid,
                entry: e.entry().unwrap_or(0),
                name: world.unit_name_of(e),
                object_type,
                lock,
                state: e.game_object_state(),
                distance: ((p.x - me.x).powi(2) + (p.y - me.y).powi(2)).sqrt(),
                // **An unresolved template is not usable**, which is the same
                // answer the renderer gives for the round trip the query costs.
                usable: info.is_some() && kind.usable(),
                hover: info
                    .map(|i| vale_assets::look::object::hover_of(kind, &i.data))
                    .unwrap_or_default(),
                pointer: match over_object(kind, &locks, lock) {
                    Some(cursor) => format!("{cursor:?}"),
                    None => "-".to_string(),
                },
                cast: OPEN_LOCK_SPELLS.with(|cell| {
                    let catalogue = cell.borrow();
                    opener(kind, &locks, lock, catalogue.as_ref()?, |spell| {
                        known.contains(&spell)
                    })
                }),
            })
        })
        .collect();
    rows.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    (!rows.is_empty()).then_some(rows)
}

/// Take one **screen row** — the same one-based numbering the interface uses,
/// with the coins first when there are any.
///
/// The crossing into the server's sparse index is
/// [`vale_protocol::play::loot::Loot::at`]'s, exactly as the renderer does it, so
/// this verb exercises the same arithmetic rather than a second copy of it.
fn take_loot(live: &vale_protocol::socket::session::LiveSession, row: usize) {
    let Some(loot) = LAST_LOOT.with(|l| l.borrow().clone()) else {
        println!("  no window open — `loot` first");
        return;
    };
    if loot.is_money(row) {
        println!("  taking the coins");
        live.loot_money();
    } else if let Some(item) = loot.at(row) {
        println!("  taking row {row} — server index {}, entry {}", item.index, item.entry);
        live.loot_item(item.index);
    } else {
        println!("  row {row} is past the end ({} rows)", loot.rows());
        return;
    }
    drain_loot(live, 1500);
}

thread_local! {
    /// The last window the server described, so `take` can cross a screen row
    /// into the server's index without asking again.
    static LAST_LOOT: std::cell::RefCell<Option<vale_protocol::play::loot::Loot>> =
        const { std::cell::RefCell::new(None) };
}

/// **Vote in the last group roll the server told us about.**
///
/// `roll need`, `roll greed`, `roll pass` -- and bare `roll` to print what is
/// open. The roll is named on the wire by `(guid, item slot)` and the interface
/// names it by an id this client invents; the CLI has no interface, so it keeps
/// the last start it saw and votes on that.
///
/// The whole of why this exists: `SMSG_LOOT_START_ROLL` only arrives when a
/// party is standing over a body with something uncommon on it, so **every
/// other check in this project can reason about the roll family and none can
/// see it**. What this can show that nothing else can: whether the vote is
/// accepted, whether the per-vote `SMSG_LOOT_ROLL` comes back for our own
/// press, and whether the row on the body really does become clickable when
/// everybody passes -- which is the client-side rule with no packet behind it.
fn roll_on(live: &vale_protocol::socket::session::LiveSession, arg: Option<&str>) {
    use vale_protocol::play::lootroll::RollVote;
    let Some((guid, slot, entry)) = LAST_ROLL.with(|r| r.get()) else {
        println!("  no roll has been announced - `loot` a body while in a party");
        return;
    };
    let name = live
        .world()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .items
        .get(&entry)
        .map(|item| item.name.clone())
        .unwrap_or_else(|| format!("entry {entry}"));
    let vote = match arg.map(str::to_ascii_lowercase).as_deref() {
        Some("need") => RollVote::Need,
        Some("greed") => RollVote::Greed,
        Some("pass") => RollVote::Pass,
        Some(other) => {
            println!("  no such vote: {other} - need, greed or pass");
            return;
        }
        None => {
            println!("  open: {name} (entry {entry}) on {guid:#x} slot {slot}");
            println!("  usage: roll need | roll greed | roll pass");
            return;
        }
    };
    println!("  {vote:?} on {name} - CMSG_LOOT_ROLL {guid:#x} slot {slot}");
    live.loot_roll(guid, slot, vote);
    drain_loot(live, 2000);
}

thread_local! {
    /// **The last roll the server opened**, as `(guid, item slot, entry)`.
    ///
    /// A thread-local beside [`LOOT_GUID`] and for the same reason: this driver
    /// is a `for line in stdin` loop with no state of its own. One rather than a
    /// list, because the verb is a check on the wire rather than a raid frame --
    /// and because the *renderer* is where the four-frame rule lives.
    static LAST_ROLL: std::cell::Cell<Option<(u64, u32, u32)>> =
        const { std::cell::Cell::new(None) };
}

/// **Read the loot queue for `ms`, print every answer, and keep the window
/// current.**
///
/// This is the instrument half. `PlayerEvent` is drained by the *renderer* in an
/// ordinary session and by nothing at all here, so a CLI that did not read it
/// would print nothing and grow a queue; every loot verb ends in this.
fn drain_loot(live: &vale_protocol::socket::session::LiveSession, ms: u64) {
    use vale_protocol::play::spells::PlayerEvent;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut quiet = true;
    while std::time::Instant::now() < deadline {
        for event in live.take_events() {
            match event {
                PlayerEvent::LootOpened(loot) => {
                    quiet = false;
                    match loot.error {
                        Some(error) => println!("  refused: {error:?} ({})", error.key()),
                        None => {
                            print_loot(live, &loot);
                            LAST_LOOT.with(|l| *l.borrow_mut() = Some(loot));
                        }
                    }
                }
                PlayerEvent::LootRemoved { index } => {
                    quiet = false;
                    println!("  SMSG_LOOT_REMOVED index {index}");
                    LAST_LOOT.with(|l| {
                        if let Some(loot) = l.borrow_mut().as_mut() {
                            loot.remove(index);
                        }
                    });
                    report_emptiness();
                }
                PlayerEvent::LootMoneyCleared => {
                    quiet = false;
                    println!("  SMSG_LOOT_CLEAR_MONEY");
                    LAST_LOOT.with(|l| {
                        if let Some(loot) = l.borrow_mut().as_mut() {
                            loot.gold = 0;
                        }
                    });
                    report_emptiness();
                }
                PlayerEvent::LootMoneyGained { copper } => {
                    quiet = false;
                    println!("  SMSG_LOOT_MONEY_NOTIFY — your share is {copper} copper");
                }
                PlayerEvent::LootRollStarted(start) => {
                    quiet = false;
                    println!(
                        "  SMSG_LOOT_START_ROLL on {:#x} slot {} - entry {}, {} ms to vote",
                        start.guid, start.item_slot, start.entry, start.countdown_ms
                    );
                    // The name is a round trip away and the renderer asks for
                    // it; here nothing else would, so this verb does.
                    live.world()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .want_item(start.entry);
                    LAST_ROLL.with(|r| r.set(Some((start.guid, start.item_slot, start.entry))));
                    println!("  (roll need | roll greed | roll pass)");
                }
                PlayerEvent::LootRollCast(cast) => {
                    quiet = false;
                    // **Both a choice and a die land here**, told apart by the
                    // number rather than by the type beside it - see
                    // `vale_protocol::play::lootroll::RollLine::of`, which is
                    // the branch this prints.
                    let line =
                        vale_protocol::play::lootroll::RollLine::of(cast.number, cast.kind);
                    println!(
                        "  SMSG_LOOT_ROLL {:#x} rolled {} type {} -> {:?} ({})",
                        cast.roller,
                        cast.number,
                        cast.kind,
                        line,
                        line.key(false, cast.kind)
                    );
                }
                PlayerEvent::LootRollWon(won) => {
                    quiet = false;
                    println!(
                        "  SMSG_LOOT_ROLL_WON - {:#x} took entry {} with {} (type {})",
                        won.winner, won.entry, won.number, won.kind
                    );
                    LAST_ROLL.with(|r| r.set(None));
                    // **The row is freed on both endings**, and there is no
                    // packet for it; mirroring the renderer's rule here is what
                    // makes `take` work on a row that opened blocked.
                    unblock(won.item_slot);
                }
                PlayerEvent::LootRollAllPassed(passed) => {
                    quiet = false;
                    println!(
                        "  SMSG_LOOT_ALL_PASSED - entry {} is back on the body",
                        passed.entry
                    );
                    LAST_ROLL.with(|r| r.set(None));
                    unblock(passed.item_slot);
                }
                PlayerEvent::LootClosed { guid } => {
                    quiet = false;
                    println!("  SMSG_LOOT_RELEASE_RESPONSE for {guid:#x} — the body is shut");
                    LAST_LOOT.with(|l| *l.borrow_mut() = None);
                    LOOT_GUID.with(|g| g.set(0));
                }
                // Everything else on this queue belongs to somebody who is not
                // reading it in this process; naming it is more useful than
                // dropping it silently.
                other => println!("  (event: {})", event_name(&other)),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    if quiet {
        println!("  (nothing came back)");
    }
}

/// **Free a row a roll was holding**, in this driver's own copy of the window.
///
/// The renderer does the same thing to `LootWindow`; here the copy is
/// [`LAST_LOOT`], and without it a `take` on the row that was just rolled for
/// would be refused locally and print nothing at all. See
/// `vale_protocol::play::loot::Loot::unblock`.
fn unblock(item_slot: u32) {
    let Ok(index) = u8::try_from(item_slot) else {
        return;
    };
    LAST_LOOT.with(|l| {
        if let Some(loot) = l.borrow_mut().as_mut() {
            if loot.unblock(index) {
                println!("  ...and its row is clickable again");
            }
        }
    });
}

/// **Say whether the body is empty and whether anything closed the window** —
/// the question the auto-close rule is about, asked after every removal.
fn report_emptiness() {
    let empty = LAST_LOOT.with(|l| {
        l.borrow()
            .as_ref()
            .map(|loot| loot.gold == 0 && loot.items.iter().all(|i| i.taken))
    });
    if empty == Some(true) {
        println!("  …the body is now empty — anything after this line is the server closing it");
    }
}

/// The rows, exactly as `GetLootSlotInfo` would answer them.
fn print_loot(
    live: &vale_protocol::socket::session::LiveSession,
    loot: &vale_protocol::play::loot::Loot,
) {
    println!(
        "  window on {:#x}: {} row(s), kind {:?}",
        loot.guid,
        loot.rows(),
        loot.kind
    );
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    for row in 1..=loot.rows() {
        if loot.is_money(row) {
            println!("    {row:>2}  {} copper", loot.gold);
            continue;
        }
        let Some(item) = loot.at(row) else { continue };
        let name = world
            .items
            .get(&item.entry)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| format!("#{} (not named yet)", item.entry));
        println!("    {row:>2}  {name} x{}", item.count);
    }
}

/// A one-word label for an event this driver is not about, so the loot output
/// does not swallow the rest of the queue in silence.
fn event_name(event: &vale_protocol::play::spells::PlayerEvent) -> String {
    format!("{event:?}")
        .split(['(', ' ', '{'])
        .next()
        .unwrap_or("?")
        .to_string()
}

/// The nth-nearest living unit, nearest first. Shared by `attack` and `watch`
/// so the two agree about which creature "1" is.
fn nth_nearest_unit(
    live: &vale_protocol::socket::session::LiveSession,
    n: usize,
) -> Option<(u64, String, f32)> {
    use vale_protocol::state::update::ObjectType;
    let me = live.status().position;
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let mut units: Vec<_> = world
        .iter()
        .filter(|e| e.object_type == Some(ObjectType::Unit) && e.is_alive())
        .filter_map(|e| {
            let p = e.position?;
            let d = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2)).sqrt();
            Some((d, e.guid, world.name_of(e)))
        })
        .collect();
    units.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    units
        .into_iter()
        .nth(n.saturating_sub(1))
        .map(|(d, guid, name)| (guid, name, d))
}

/// Select the nth-nearest unit, and report the round trip.
///
/// **The selection is the client's own and it is immediate** — this only tells
/// the server. What makes the command worth having is what comes *back*: the
/// server writes the guid into our own `UNIT_FIELD_TARGET`, so a moment later
/// the player entity states who we picked. A disagreement there is a dropped
/// `CMSG_SET_SELECTION`, which is otherwise entirely silent.
fn target_nearest(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    let Some((guid, name, distance)) = nth_nearest_unit(live, n) else {
        println!("  nothing nearby to select");
        return;
    };
    live.target(Some(guid));
    println!("  selected {name} ({guid:#x}) at {distance:.1} yards");

    // The echo takes a round trip plus an update block. Half a second is several
    // times the loopback latency and short enough not to be noticed.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let echoed = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.player().and_then(|p| p.target_guid())
    };
    match echoed {
        Some(echoed) if echoed == guid => println!("  UNIT_FIELD_TARGET agrees"),
        Some(other) => println!("  UNIT_FIELD_TARGET says {other:#x} — the selection did not take"),
        None => println!("  UNIT_FIELD_TARGET is still empty"),
    }
}

/// The spellbook, resolved against `Spell.dbc`.
///
/// **`SMSG_INITIAL_SPELLS` is said once, in the login burst, and nothing ever
/// restates it** — so an empty list here is a failure with no other symptom: the
/// action bar would simply be blank for the whole session.
fn print_spellbook(cfg: &Config, live: &vale_protocol::socket::session::LiveSession) {
    let (known, buttons) = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        (world.spellbook.known.clone(), world.action_buttons.clone())
    };
    println!(
        "  {} spells known, {} action-bar slots occupied",
        known.len(),
        buttons.len()
    );
    if known.is_empty() {
        println!("  (nothing — SMSG_INITIAL_SPELLS was missed or misread)");
        return;
    }
    let Some(catalog) = catalog(cfg) else {
        println!("  {known:?}");
        return;
    };
    let mut castable = 0;
    for id in &known {
        match catalog.info(*id) {
            Some(info) if info.is_passive() => {}
            Some(info) => {
                castable += 1;
                println!(
                    "  {:6} {:28} {:>4} {:<6} {:>5} ms  {:>3} yd",
                    info.id,
                    info.label(),
                    vale_protocol::state::objects::power_type::display(
                        info.power_type as u8,
                        info.power_cost,
                    ),
                    power_word(info.power_type),
                    info.cast_time_ms,
                    info.range_yards as u32,
                );
            }
            None => println!("  {id:6} (not in Spell.dbc)"),
        }
    }
    println!("  {castable} castable, {} passive", known.len() - castable);
    for button in &buttons {
        let name = catalog
            .info(button.action)
            .map(|i| i.label())
            .unwrap_or_else(|| format!("action {}", button.action));
        println!("  slot {:3}  kind {:#04x}  {name}", button.slot, button.kind);
    }
}

/// Aim a spell the way the client would, send it, and print what came back.
///
/// This is the whole outbound chain in one command, and the interesting half is
/// the aiming: 14,002 of the game's 22,360 spells commit with **no target at
/// all**, so a client that shipped the current selection with every cast would
/// get "Invalid target" back for every buff in the game. See
/// [`vale_assets::tables::spellbook::resolve_aim`].
fn cast_spell(cfg: &Config, live: &vale_protocol::socket::session::LiveSession, spell_id: u32) {
    use vale_assets::tables::faction::Reaction;
    use vale_assets::tables::spellbook::{resolve_aim, CastAim, Candidate};
    use vale_protocol::play::spells::CastTarget;

    let Some(catalog) = catalog(cfg) else {
        println!("  no Spell.dbc — cannot aim a cast without it");
        return;
    };
    let Some(info) = catalog.info(spell_id) else {
        println!("  spell {spell_id} is not in Spell.dbc");
        return;
    };

    // The selection, as the aiming rule needs to see it. The CLI has no faction
    // table open, so the reaction is the neutral default — which is attackable,
    // and which the server range- and faction-checks anyway.
    let (selected, me) = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        let mine = world.player().and_then(|p| p.target_guid()).filter(|g| *g != 0);
        let candidate = mine.map(|guid| Candidate {
            guid,
            is_self: false,
            reaction: Reaction::Neutral,
            unit_flags: world.get(guid).map(|e| e.unit_flags()).unwrap_or(0),
            dead: world.get(guid).and_then(|e| e.is_dead()).unwrap_or(false),
        });
        let me = world.player_guid.map(|guid| Candidate {
            guid,
            is_self: true,
            reaction: Reaction::Friendly,
            unit_flags: 0,
            dead: false,
        });
        (candidate, me)
    };

    let aim = resolve_aim(&info, selected, me, false, None);
    println!("  {} — aiming word {:#06x}", info.label(), vale_assets::tables::spellbook::aim_mask(&info));
    match aim {
        CastAim::SelfImplicit => {
            println!("  TARGET_FLAG_SELF: the spell names its own target");
            live.cast(spell_id, CastTarget::SelfImplicit);
        }
        CastAim::Unit(guid) => {
            println!("  TARGET_FLAG_UNIT at {guid:#x}");
            live.cast(spell_id, CastTarget::Unit(guid));
        }
        // **The renderer would put the targeting cursor up here** and wait for a
        // click; this session has no pointer, so it says so and sends nothing.
        // Not a refusal — the difference matters, because "refused" is what this
        // client used to do wrongly.
        CastAim::WantsTarget => {
            println!("  the targeting cursor: the window would wait to be pointed");
            return;
        }
        // …and the placed half of the same cursor. The point comes off a ray
        // against the floor, which this session has no camera to cast — so it
        // reports the shape and sends nothing, rather than inventing a place.
        CastAim::WantsGround => {
            println!("  the targeting cursor, for a *place*: the window would wait for a click");
            println!("  (TARGET_FLAG_DEST_LOCATION, three floats — see `game::target`)");
            return;
        }
        // **The main-hand weapon, picked by the client rather than pointed at.**
        // See `CastAim::Item`: the imbues, the poisons and the sharpening
        // stones all carry `SPELL_ATTR_HELD_ITEM_ONLY` beside their item bit.
        CastAim::Item(guid) => {
            println!("  TARGET_FLAG_ITEM at {guid:#x} — the main hand");
            live.cast(spell_id, CastTarget::Item(guid));
        }
        // …and the residue, which is an enchanting formula: aimed by clicking a
        // bag slot, which this session has no pointer for.
        CastAim::WantsItem => {
            println!("  the targeting cursor, for an *item*: the window would wait for a bag click");
            return;
        }
        CastAim::Refused(key) => {
            // Refused *locally*, exactly as the client would — nothing is sent.
            println!("  refused before the socket: {key}");
            return;
        }
    }

    // The reply is `SMSG_CAST_RESULT` and it comes back either way; the failure
    // reason is an index into the game's own string table.
    std::thread::sleep(std::time::Duration::from_millis(600));
    let events = live.take_events();
    if events.is_empty() {
        println!("  no SMSG_CAST_RESULT yet");
    }
    for event in events {
        match event {
            vale_protocol::play::spells::PlayerEvent::CastFailed { reason, .. } => {
                let key = vale_protocol::play::spells::cast_failure_key(reason);
                println!(
                    "  refused by the server: {} ({reason:#04x})",
                    key.unwrap_or("(a reason the client never displays)")
                );
            }
            other => println!("  {other:?}"),
        }
    }
}

/// `Spell.dbc` and its three small tables, opened on demand.
///
/// Not held for the session: `spells` and `cast` are typed a handful of times
/// and parsing the table costs a fraction of a second, where holding it would
/// mean opening the whole archive chain at login for a command that may never be
/// used.
fn catalog(cfg: &Config) -> Option<vale_assets::tables::spellbook::Spells> {
    let mut assets = crate::common::open_assets(cfg).ok()?;
    let mut read = |table: &str| {
        assets
            .read(&vale_assets::tables::dbc::dbc_path(table))
            .unwrap_or_default()
    };
    vale_assets::tables::spellbook::Spells::parse(
        &read("Spell"),
        &read("SpellCastTimes"),
        &read("SpellRange"),
        &read("SpellIcon"),
        &read("SpellDuration"),
        &read("SpellRadius"),
        &read("SpellDispelType"),
    )
    .ok()
}

fn power_word(kind: u32) -> &'static str {
    match kind {
        1 => "rage",
        2 => "focus",
        3 => "energy",
        4 => "happiness",
        _ => "mana",
    }
}

/// Melee reach, near enough. `NOMINAL_MELEE_RANGE` is 5 yards in vmangos and
/// the check is against the *combined* bounding radii, so stopping short of it
/// leaves room for a creature that is drifting.
const MELEE_RANGE: f32 = 3.5;

/// Walk up to a unit until it is within melee reach, and report the distance.
///
/// **`attack` is worth very little without this.** `CMSG_ATTACKSWING` at twenty
/// yards is answered with `SMSG_ATTACKSWING_NOTINRANGE`, the creature never
/// notices, and the one command that exists to produce a chase spline, a swing
/// and a *combat facing* produces none of the three — which was measured the
/// slow way: four scripted runs in a row picked a target that had wandered out
/// of reach, and each of them read as "the server sends nothing" rather than as
/// "nothing was asked".
///
/// `None` if the unit left the object manager on the way.
fn close_on(live: &vale_protocol::socket::session::LiveSession, guid: u64) -> Option<f32> {
    use vale_protocol::state::movement::Controls;
    use std::time::Duration;

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let me = live.status().position;
        let at = {
            let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
            world.get(guid)?.position?
        };
        let d = ((at.x - me.x).powi(2) + (at.y - me.y).powi(2)).sqrt();
        if d <= MELEE_RANGE || std::time::Instant::now() >= deadline {
            live.set_controls(Controls::default());
            return Some(d);
        }
        live.face((at.y - me.y).atan2(at.x - me.x));
        live.set_controls(Controls { forward: true, ..Default::default() });
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Walk up to the nth-nearest living unit and watch it, throwing no blow.
fn goto_nearest(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    match nth_nearest_unit(live, n) {
        Some((guid, name, d)) => {
            println!("  closing on {name} ({d:.1}y away, guid {})", guid & 0xFFFF_FFFF);
            match close_on(live, guid) {
                Some(d) => {
                    println!("  standing {d:.1}y from {name}");
                    watch_guid(live, guid, &name, 15);
                }
                None => println!("  {name} left before we got there"),
            }
        }
        None => println!("  no living unit nearby"),
    }
}

fn attack_nearest(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    match nth_nearest_unit(live, n) {
        Some((guid, name, d)) => {
            println!("  closing on {name} ({d:.1}y away, guid {})", guid & 0xFFFF_FFFF);
            match close_on(live, guid) {
                Some(d) => {
                    println!("  swinging at {name} from {d:.1}y");
                    live.attack(Some(guid));
                    // …and then watch *this* creature rather than whichever is
                    // nearest afterwards. The combat facing is only observable
                    // in the seconds between the swing landing and the thing
                    // dying, and by then `nth_nearest_unit` has moved on: it
                    // filters the dead out, so the one unit whose behaviour was
                    // just provoked is the one it stops returning.
                    watch_guid(live, guid, &name, 10);
                }
                None => println!("  {name} left before we got there"),
            }
        }
        None => println!("  no living unit nearby"),
    }
}

/// How far a unit must turn before `watch` prints a line for the turn alone.
///
/// Two degrees: enough that the last fraction of a turn does not print at
/// every 50 ms poll, small enough that a creature tracking a circling player
/// still reports.
const TURN_NOTICE: f32 = 0.035;

/// Print one entity's position every time the server changes it — or turns it.
///
/// `position_updates` counts *server* statements rather than interpolated steps,
/// which is the distinction that matters here: it separates a bad parse (the
/// server said something absurd) from bad dead reckoning (we walked it there).
fn watch_entity(live: &vale_protocol::socket::session::LiveSession, n: usize, seconds: u64) {
    let Some((guid, name, _)) = nth_nearest_unit(live, n) else {
        println!("  no living unit nearby");
        return;
    };
    watch_guid(live, guid, &name, seconds);
}

fn watch_guid(
    live: &vale_protocol::socket::session::LiveSession,
    guid: u64,
    name: &str,
    seconds: u64,
) {
    println!("  watching {name} (guid {}) for {seconds}s", guid & 0xFFFF_FFFF);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    let mut last_updates = u32::MAX;
    let mut last_facing = f32::NAN;
    let mut last_health = None;
    let mut last = None;
    while std::time::Instant::now() < deadline {
        let me = live.status().position;
        {
            let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
            match world.get(guid) {
                None => {
                    println!("    GONE from the object manager");
                    break;
                }
                Some(e) => {
                    let Some(p) = e.position else { continue };
                    // **Facing is a reason to print in its own right.** A
                    // creature standing in melee turns towards its target
                    // without the server saying anything at all — see
                    // `ObjectManager::advance` — so `position_updates` never
                    // moves and a watch keyed on it alone shows an empty
                    // screen for exactly the case it is being read for.
                    let turned = (p.orientation - last_facing).abs() > TURN_NOTICE;
                    if e.position_updates != last_updates || turned || e.health() != last_health {
                        last_updates = e.position_updates;
                        last_facing = p.orientation;
                        last_health = e.health();
                        // How far the server moved it since its last statement.
                        let jump = last
                            .map(|(x, y, z): (f32, f32, f32)| {
                                ((p.x - x).powi(2) + (p.y - y).powi(2) + (p.z - z).powi(2)).sqrt()
                            })
                            .unwrap_or(0.0);
                        last = Some((p.x, p.y, p.z));
                        let d = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2)).sqrt();
                        println!(
                            "    #{last_updates:<4} at ({:>9.1},{:>9.1},{:>7.1}) facing {:>4.0}deg  \
                             {d:>6.1}y  {:>6}  moved {jump:>7.1}y{}{}",
                            p.x,
                            p.y,
                            p.z,
                            p.orientation.to_degrees(),
                            // A trace that goes quiet because the thing died
                            // must not read like one that goes quiet because
                            // nothing is being sent.
                            e.health_label().unwrap_or_default(),
                            // The measurement the facing rule is judged on: the
                            // bearing it is turning towards, and how far off it
                            // still is. A stationary creature that has finished
                            // turning reads `off 0deg`.
                            match e.target_guid().and_then(|g| world.get(g)) {
                                Some(t) => match t.position {
                                    Some(at) => {
                                        let bearing = (at.y - p.y).atan2(at.x - p.x);
                                        let off = vale_protocol::state::movement::shortest_turn(
                                            p.orientation,
                                            bearing,
                                        );
                                        format!(
                                            "  -> {} bearing {:>4.0}deg off {:>4.0}deg",
                                            world.name_of(t),
                                            bearing.to_degrees().rem_euclid(360.0),
                                            off.to_degrees(),
                                        )
                                    }
                                    None => "  -> target not placed".to_string(),
                                },
                                None => String::new(),
                            },
                            match &e.spline {
                                Some(s) => format!(
                                    "  spline {:.1}y over {} nodes in {} ms",
                                    s.length(),
                                    s.path.len(),
                                    s.duration_ms
                                ),
                                None => String::new(),
                            }
                        );
                    }
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    for w in live.status().warnings.iter().take(10) {
        println!("    parse warning: {w}");
    }
}

fn print_where(live: &vale_protocol::socket::session::LiveSession) {
    use vale_assets::tile_for_position;
    let s = live.status();
    let (tx, ty) = tile_for_position(s.position.x, s.position.y);
    println!(
        "  ({:.1}, {:.1}, {:.1}) facing {:.0}deg  tile {tx},{ty}  run {:.1} y/s",
        s.position.x,
        s.position.y,
        s.position.z,
        s.position.orientation.to_degrees(),
        s.speeds.run(),
    );
    // The player's own movement state, which is what the renderer picks its
    // animation from — and the one entity the object manager cannot answer for,
    // since the local `Mover` owns it rather than the server.
    println!(
        "  {}",
        if s.moving {
            format!("moving at {:.1} y/s", s.speed)
        } else {
            "standing still".to_string()
        }
    );
}

thread_local! {
    /// The banker the window is open at, so the three verbs after `bank` need
    /// no argument — a thread-local for the reason [`LOOT_GUID`] is one.
    static BANKER: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// **Walk to the nearest banker and open the bank.**
///
/// `UNIT_NPC_FLAG_BANKER` is `0x100`. A banker with the gossip bit opens the
/// same window through its menu; this driver sends `CMSG_BANKER_ACTIVATE`
/// outright, which `CheckBanker` accepts from either kind. What comes back is
/// `SMSG_SHOW_BANK` — a guid — and the squares are read off the fields that
/// were there all along.
fn open_bank(live: &vale_protocol::socket::session::LiveSession) {
    use vale_protocol::socket::session::NpcVerb;
    const BANKER_FLAG: u32 = 0x100;

    let nearest = {
        let status = live.status();
        let me = status.position;
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        world
            .iter()
            .filter(|e| e.npc_flags() & BANKER_FLAG != 0)
            .filter_map(|e| {
                let p = e.position?;
                let d = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2)).sqrt();
                Some((d, e.guid, world.name_of(e)))
            })
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    };
    let Some((distance, guid, name)) = nearest else {
        println!("  no banker in view");
        return;
    };
    println!("  closing on {name} ({distance:.1}y, guid {guid:#x})");
    if distance > MELEE_RANGE {
        close_on(live, guid);
    }
    live.npc(NpcVerb::BankerActivate(guid));
    std::thread::sleep(std::time::Duration::from_millis(800));
    let mut shown = false;
    for event in live.take_events() {
        match event {
            vale_protocol::play::spells::PlayerEvent::BankShow(banker) => {
                println!("  SMSG_SHOW_BANK from {banker:#x}");
                BANKER.with(|cell| cell.set(banker));
                shown = true;
            }
            other => println!("  {other:?}"),
        }
    }
    if !shown {
        println!("  no SMSG_SHOW_BANK — out of reach, or not a banker");
        return;
    }
    print_bank(live);
}

/// `CMSG_BUY_BANK_SLOT` at the open banker. **Success says nothing**: the
/// count is a byte of `PLAYER_BYTES_2` and the money is a field, so this
/// prints the byte before and after rather than waiting for a reply.
fn bank_buy(live: &vale_protocol::socket::session::LiveSession) {
    use vale_protocol::socket::session::NpcVerb;
    let banker = BANKER.with(|cell| cell.get());
    if banker == 0 {
        println!("  no bank open — run `bank` first");
        return;
    }
    let before = bank_bag_slots(live);
    live.npc(NpcVerb::BuyBankSlot(banker));
    std::thread::sleep(std::time::Duration::from_millis(800));
    for event in live.take_events() {
        match event {
            vale_protocol::play::spells::PlayerEvent::BankSlotResult { code, result } => {
                println!(
                    "  SMSG_BUY_BANK_SLOT_RESULT {code}: {}",
                    result.and_then(|r| r.key()).unwrap_or("(a code the client never displays)")
                );
            }
            other => println!("  {other:?}"),
        }
    }
    println!("  bag slots bought: {before} -> {}", bank_bag_slots(live));
}

/// `deposit <bag> <slot>` and `withdraw <slot>` — one command, whose opcode
/// the session picks off which side of the counter the source is.
fn bank_move(live: &vale_protocol::socket::session::LiveSession, arg: Option<&str>, from_bank: bool) {
    use vale_protocol::play::items::{server_container_slot, BANK_CONTAINER};
    if BANKER.with(|cell| cell.get()) == 0 {
        println!("  no bank open — run `bank` first");
        return;
    }
    let mut words = arg.unwrap_or("").split_whitespace().map(|w| w.parse::<i64>().ok());
    let place = if from_bank {
        words.next().flatten().map(|slot| (i64::from(BANK_CONTAINER), slot))
    } else {
        words.next().flatten().zip(words.next().flatten())
    };
    let Some((bag, slot)) = place else {
        println!(
            "  usage: {}",
            if from_bank { "withdraw <slot>   (1..24, a bank square)" } else { "deposit <bag> <slot>   (bag 0 the backpack)" }
        );
        return;
    };
    let Some((wire_bag, wire_slot)) = usize::try_from(slot)
        .ok()
        .and_then(|slot| server_container_slot(bag as i32, slot))
    else {
        println!("  ({bag}, {slot}) is not a square");
        return;
    };
    println!("  moving ({bag}, {slot}) = wire ({wire_bag}, {wire_slot})");
    live.bank_item(wire_bag, wire_slot);
    std::thread::sleep(std::time::Duration::from_millis(800));
    for event in live.take_events() {
        println!("  {event:?}");
    }
    print_bank(live);
}

/// The third byte of `PLAYER_BYTES_2`, straight off the player's fields.
fn bank_bag_slots(live: &vale_protocol::socket::session::LiveSession) -> u8 {
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    vale_protocol::play::items::Inventory::read(&world).bank_bag_slots
}

/// **The bank's thirty, off the same snapshot `bags` prints from** — which is
/// the point: nothing here arrived with the window.
fn print_bank(live: &vale_protocol::socket::session::LiveSession) {
    use vale_protocol::play::items::{Inventory, BANK_CONTAINERS_FOR_REPORT, BANK_CONTAINER};
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let carried = Inventory::read(&world);
    let name_of = |entry: u32| {
        world
            .items
            .get(&entry)
            .map_or_else(|| format!("#{entry}"), |info| info.name.clone())
    };
    println!("  bank: {} bag slots bought", carried.bank_bag_slots);
    for bag in BANK_CONTAINERS_FOR_REPORT {
        let Some(slots) = carried.container(bag) else {
            continue;
        };
        let label = if bag == BANK_CONTAINER {
            "the bank's own squares".to_string()
        } else {
            carried.bag_item(bag).map_or_else(|| "?".to_string(), |item| name_of(item.entry))
        };
        let used = slots.iter().flatten().count();
        println!("\n  bag {bag}: {label} — {used} of {} slots", slots.len());
        for (index, held) in slots.iter().enumerate() {
            let Some(item) = held else { continue };
            let count = (item.count > 1)
                .then(|| format!(" x{}", item.count))
                .unwrap_or_default();
            println!("    {:>2}  {}{count}", index + 1, name_of(item.entry));
        }
    }
}

/// **What the character is carrying, without a window** — every slot, every
/// stack and every template, out of the same [`vale_protocol::play::items`] read
/// the interface uses.
///
/// The one check this subject can have against a real server, and it is a
/// sharper one than `sheet`'s. The inventory is a **three-deep GUID chase**
/// through update fields nothing on the wire ever announces (see that module's
/// own comment), so what the unit tests prove is the *arithmetic* — that field
/// 19 of the run is the first bag slot — and what only a session can prove is
/// that the objects arrive at all. A bag that reads `0 slots` here with items
/// visibly in it in the retail client is the whole test.
///
/// The **second** column is the other half: `templates` counts the entries
/// `CMSG_ITEM_QUERY_SINGLE` has answered for. A slot with a count and no name
/// is a query still in flight; a slot that never gets one is a query that was
/// never sent, and those two look identical on screen.
fn print_bags(live: &vale_protocol::socket::session::LiveSession) {
    use vale_protocol::play::items::{Inventory, CONTAINERS_FOR_REPORT};

    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let carried = Inventory::read(&world);
    if world.player().is_none() {
        println!("  no player entity yet");
        return;
    }
    let money = world
        .player()
        .and_then(|p| p.field(vale_protocol::state::fields::player::COINAGE))
        .unwrap_or(0);
    println!(
        "  {}g {}s {}c",
        money / 10_000,
        (money / 100) % 100,
        money % 100
    );

    let name_of = |entry: u32| {
        world
            .items
            .get(&entry)
            .map_or_else(|| format!("#{entry}"), |info| info.name.clone())
    };

    println!("\n  worn:");
    for (index, worn) in carried.equipped.iter().enumerate() {
        let Some(item) = worn else { continue };
        // The interface's slot id is one past the field index — see
        // `vale_protocol::play::items`, where the two numberings are crossed.
        let durability = (item.max_durability > 0)
            .then(|| format!("  {}/{}", item.durability, item.max_durability))
            .unwrap_or_default();
        println!(
            "    slot {:>2}  {}{durability}",
            index + 1,
            name_of(item.entry)
        );
    }

    for bag in CONTAINERS_FOR_REPORT {
        let Some(slots) = carried.container(bag) else {
            continue;
        };
        let label = match carried.bag_item(bag) {
            Some(item) => name_of(item.entry),
            None if bag == vale_protocol::play::items::KEYRING_CONTAINER => "key ring".to_string(),
            None => "backpack".to_string(),
        };
        let used = slots.iter().flatten().count();
        println!("\n  bag {bag}: {label} — {used} of {} slots", slots.len());
        for (index, held) in slots.iter().enumerate() {
            let Some(item) = held else { continue };
            let count = (item.count > 1)
                .then(|| format!(" x{}", item.count))
                .unwrap_or_default();
            println!("    {:>2}  {}{count}", index + 1, name_of(item.entry));
        }
    }

    // **The vendor's twelve, which are the player's own fields and not a
    // packet** — printed here because `bags` is the only check this subject can
    // have against a real server, and because the three rules the client
    // applies to those fields (occupancy by price, compaction, the sort) are
    // invisible to a unit test with a hand-written world. What to look for: the
    // **last** line is the thing you most recently sold, and its wire slot need
    // not be the last number.
    if !carried.buyback.is_empty() {
        println!("
  buyback — oldest first, so the last line is the newest sale:");
        for (row, slot) in carried.buyback.iter().enumerate() {
            let count = (slot.item.count > 1)
                .then(|| format!(" x{}", slot.item.count))
                .unwrap_or_default();
            println!(
                "    row {:>2}  wire slot {:>2}  {}{count}  for {}g {}s {}c  (sold at {})",
                row + 1,
                slot.wire_slot,
                name_of(slot.item.entry),
                slot.price / 10_000,
                (slot.price / 100) % 100,
                slot.price % 100,
                slot.sold_at
            );
        }
    }

    let entries = carried.entries();
    let named = entries.iter().filter(|e| world.items.contains_key(e)).count();
    println!(
        "\n  {} distinct items carried: {named} named, {} still waiting on \
         CMSG_ITEM_QUERY_SINGLE",
        entries.len(),
        entries.len() - named
    );
}

/// **The character sheet, without a window** — every number
/// `PaperDollFrame.lua` prints, from the same decode the interface reads.
///
/// The one check this subject can have against a real server. Everything in
/// [`vale_protocol::play::stats`] is unit-tested against hand-written fields, and
/// that proves the *arithmetic*; what it cannot prove is that the fields
/// arrive at all, which is exactly where this client has been bitten before —
/// vmangos does not send a field whose value is zero, and half of this block is
/// `PRIVATE`. A resistance line of dashes here and a real number in the retail
/// client side by side is the whole test.
fn print_sheet(live: &vale_protocol::socket::session::LiveSession) {
    use vale_protocol::play::stats::UnitStats;
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let Some(me) = world.iter().find(|e| e.is_self) else {
        println!("  no player entity yet");
        return;
    };
    let Some(stats) = UnitStats::read(me, &world.weapons_of(me)) else {
        println!("  no stat block — UNIT_FIELD_STAT0 never arrived for our own character");
        return;
    };

    const ATTRIBUTES: [&str; 5] = ["Strength", "Agility", "Stamina", "Intellect", "Spirit"];
    for (i, name) in ATTRIBUTES.iter().enumerate() {
        let (stat, effective, positive, negative) = stats.stat(i + 1).expect("in range");
        println!(
            "  {name:<10} {effective:>5}   (base {}, +{positive}, {negative})",
            stat - positive - negative
        );
    }
    // Armour is resistance 0 and the panel prints it with the attributes; the
    // five it draws down the right are 6, 2, 3, 4, 5 — arcane first, and holy
    // has no frame at all.
    const SCHOOLS: [(usize, &str); 7] = [
        (0, "Armor"),
        (2, "Fire"),
        (3, "Nature"),
        (4, "Frost"),
        (5, "Shadow"),
        (6, "Arcane"),
        (1, "Holy (no frame)"),
    ];
    for (school, name) in SCHOOLS {
        let (base, total, positive, negative) = stats.resistance(school).expect("in range");
        println!("  {name:<15} {total:>5}   (base {base}, +{positive}, {negative})");
    }

    let (speed, offhand) = stats.attack_speed();
    let damage = stats.damage();
    println!(
        "  damage         {:.0}-{:.0} at {speed:.2}s{}   mods +{} {} x{:.2}",
        damage.min,
        damage.max,
        match offhand {
            Some(off) => format!(", off hand {:.0}-{:.0} at {off:.2}s", damage.min_offhand, damage.max_offhand),
            None => String::new(),
        },
        damage.bonus_positive,
        damage.bonus_negative,
        damage.percent,
    );
    let (power, power_pos, power_neg) = stats.attack_power();
    println!("  attack power   {power:>5}   (+{power_pos}, {power_neg})");
    let ranged = stats.ranged_damage();
    let (ranged_power, ranged_pos, ranged_neg) = stats.ranged_attack_power();
    println!(
        "  ranged         {:.0}-{:.0} at {:.2}s, power {ranged_power} (+{ranged_pos}, {ranged_neg}){}",
        ranged.min,
        ranged.max,
        ranged.speed,
        if stats.has_wand() { "  [wand]" } else { "" },
    );
    // The three out of the skill block, which is the half of this module that
    // is vmangos' layout rather than the client's — so it is the half worth
    // looking at hardest against the retail client.
    let (melee, melee_mod) = stats.weapon_skill(0);
    let (off, off_mod) = stats.weapon_skill(1);
    let (ranged_skill, ranged_skill_mod) = stats.ranged_skill();
    let (defense, defense_mod) = stats.defense();
    println!(
        "  skills         melee {melee} (+{melee_mod}), off hand {off} (+{off_mod}), \
         ranged {ranged_skill} (+{ranged_skill_mod}), defense {defense} (+{defense_mod})"
    );
}

fn print_nearby(live: &vale_protocol::socket::session::LiveSession, count: usize) {
    let status = live.status();
    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
    let me = status.position;

    let mut nearby: Vec<_> = world
        .iter()
        .filter(|e| e.position.is_some() && Some(e.guid) != world.player_guid)
        .map(|e| {
            let p = e.position.expect("filtered");
            let d = ((p.x - me.x).powi(2) + (p.y - me.y).powi(2) + (p.z - me.z).powi(2)).sqrt();
            (d, e)
        })
        .collect();
    nearby.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    println!("  {} entities known:", world.len());
    for (d, e) in nearby.iter().take(count) {
        let vitals = match (e.level(), e.health_label()) {
            (Some(level), Some(hp)) => format!("  level {level}, {hp}"),
            (Some(level), None) => format!("  level {level}"),
            _ => String::new(),
        };
        // Creatures move on a spline; players move under their own movement
        // flags, dead-reckoned between heartbeats. Both are `Entity::is_moving`,
        // which is also what the renderer picks an animation from.
        let moving = if e.is_moving() {
            format!(" (moving {:.1} y/s)", e.ground_speed())
        } else {
            String::new()
        };
        // **What the renderer would be animating, beside where it is.** Neither
        // of these has any other trace: a held emote is an update field that is
        // usually absent, and swimming is one bit of the movement flags. Both
        // choose an animation outright, and both are silent when read wrongly.
        let pose = match (e.emote_state(), e.is_swimming()) {
            (0, false) => String::new(),
            (0, true) => "  swimming".to_string(),
            (emote, false) => format!("  emote state {emote}"),
            (emote, true) => format!("  emote state {emote}, swimming"),
        };
        println!("    {d:>7.1}y  {:<34}{vitals}{moving}{pose}", world.name_of(e));

        // Where it is, where it faces, and where it is *going*. The last two
        // should agree while it is on a spline — a creature walks forwards — so
        // printing both side by side is how a wrong facing is seen rather than
        // guessed at, and the absolute position is how a teleport is caught.
        let p = e.position.expect("filtered");
        let facing = p.orientation.to_degrees();
        let travel = e.spline.as_ref().map(|s| {
            let length = s.length();
            format!(
                "  travelling {:>4.0}deg  {:.1}y to go of {length:.1}y over {} nodes in {} ms",
                s.position_and_heading().1.to_degrees().rem_euclid(360.0),
                (1.0 - s.progress()) * length,
                s.path.len(),
                s.duration_ms,
            )
        });
        println!(
            "             at ({:.1}, {:.1}, {:.1}) facing {facing:>4.0}deg{}",
            p.x,
            p.y,
            p.z,
            travel.unwrap_or_default()
        );

        // **The check for "mobs do not face what they are attacking".** The
        // server never states a creature's combat facing — `SetInFront` is
        // `SetOrientation` and no packet — so the client turns it towards
        // `UNIT_FIELD_TARGET` itself, and the only way to see that it worked is
        // the bearing to that target beside the facing above. A stationary
        // creature that has finished turning should have them equal.
        if let Some(target) = e.target_guid().and_then(|g| world.get(g)) {
            let bearing = match target.position {
                Some(at) => {
                    let b = (at.y - p.y).atan2(at.x - p.x).to_degrees().rem_euclid(360.0);
                    format!("bearing {b:>4.0}deg")
                }
                None => "not placed".to_string(),
            };
            println!(
                "             targeting {:<28}{bearing}",
                world.name_of(target)
            );
        }

        // What it has in its hands, and whether they are out. For a *creature*
        // this needs no round trip at all: `Creature::SetVirtualItem` writes the
        // `ItemDisplayInfo` id straight into `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY`,
        // where a player's `PLAYER_VISIBLE_ITEM` carries an entry that has to be
        // asked about. Printed here because it is the one part of an NPC's gear
        // that comes off the wire rather than out of a DBC — the rest is in
        // `vale npc <displayId>`.
        let weapons = e.weapon_displays();
        if weapons.iter().any(|w| *w != 0) {
            let state = match e.sheath_state() {
                1 => "melee drawn",
                2 => "ranged drawn",
                _ => "put away",
            };
            println!(
                "             weapons {weapons:?} ({state}) — display ids, no query needed"
            );
        }
    }
}

fn print_status(live: &vale_protocol::socket::session::LiveSession) {
    let s = live.status();
    println!(
        "  {} packets, {} movement sent, {} ms latency, {} speed change(s) \
         ({} about others), {} flag change(s) ({} about creatures), \n{} pushed sound(s) and {} pushed visual(s)",
        s.packets,
        s.movement_sent,
        s.latency_ms,
        s.speed_changes,
        s.speed_broadcasts,
        s.flag_changes,
        s.spline_flag_changes,
        s.pushed_sounds,
        s.pushed_visuals
    );
    // **The one packet this client volunteers**, and the only instrument the
    // subject has: an instance portal that does nothing looks the same whether
    // the trigger was never noticed or the server declined it, and this says
    // which side of the socket to look at. See `vale_protocol::play::areatrigger`.
    if s.area_triggers > 0 {
        println!("  {} area trigger(s) reported", s.area_triggers);
    }
    // …and the other packet with no visible trace: a Charge. Ignoring one does
    // not misplace the character, it makes the server discard every movement
    // packet for the rest of the session — so a zero here after charging is
    // the whole diagnosis. See `vale_protocol::state::movement::Mover::ride`.
    if s.rides > 0 {
        println!("  {} server-driven ride(s) walked and acknowledged", s.rides);
    }
    // …and the packet with no other trace at all: what it drives is a growl.
    if s.attacks > 0 || s.ai_reactions > 0 {
        println!(
            "  {} swing(s), {} AI reaction(s) — the aggro and alert barks",
            s.attacks, s.ai_reactions
        );
    }
    // **The forced changes are the ones with a deadline.** A root, a water-walk
    // or a knockback that is not acknowledged within four seconds is
    // `CHEAT_TYPE_PENDING_ACK_DELAY` and then enforced anyway — so a session
    // that was kicked with a zero here was kicked for packets it never saw,
    // and one with a non-zero here answered them.
    if s.airborne {
        println!("  in the air ({})", if s.jumping { "jumped" } else { "falling" });
    }
    // **The four packets that drive an animation**, counted here because none of
    // them has any other trace: an emote that arrives and is dropped and an
    // emote that never arrives look identical from inside the world state, and
    // the whole difficulty of this half of the client is that nothing about
    // getting it wrong produces an error.
    if s.attacks + s.emotes + s.casts > 0 {
        println!(
            "  {} swings, {} emotes, {} spell starts/gos",
            s.attacks, s.emotes, s.casts
        );
    }
    // **What this character can do, and what came of trying.** The first two are
    // login-burst facts and a zero in either is a failure with no other symptom:
    // no spellbook is an empty action bar for the whole session. The last two are
    // answers to our own presses, which leave no trace in the world state at all.
    println!(
        "  {} spells known, {} bar slots, {} cast result(s), {} swing refusal(s){}",
        s.spells_known,
        s.action_buttons,
        s.cast_results,
        s.attack_refusals,
        match s.attacking {
            Some(guid) => format!(", swinging at {guid:#x}"),
            None => String::new(),
        }
    );
    // Should be zero. Anything else means a packet was read at the wrong offset
    // and an entity would otherwise have silently vanished — see
    // `Entity::set_server_position`.
    let (rejected, jump_count, jump_log) = {
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        (
            world.rejected_positions,
            world.jump_count,
            world.jump_log.clone(),
        )
    };
    if rejected > 0 {
        println!("    !! {rejected} impossible positions refused");
    }
    // **What "mobs teleport around" actually is.** A jump is a server statement
    // that moved an entity further than it could have walked since the last one,
    // and it is not by itself an error — the server really does relocate things.
    // What the number and the attribution are for is telling a *frequent* jump,
    // which is the client having walked a creature somewhere the server did not,
    // from an occasional one, which is the game.
    if jump_count > 0 {
        println!("    {jump_count} position jumps over {}y:", vale_protocol::state::objects::JUMP_YARDS);
        for line in jump_log.iter().take(8) {
            println!("      {line}");
        }
    }
    for w in s.warnings.iter().take(5) {
        println!("    parse warning: {w}");
    }
    let mut rows: Vec<_> = s.unhandled.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (name, n) in rows.iter().take(8) {
        println!("    {n:>4} x {name}");
    }
}

/// A newtype for the same reason the terrain closure above is one: the whole
/// rule lives in `vale_assets::tables::areatrigger` — where it is unit-tested and
/// where `vale triggers` calls the same copy — and this only carries it
/// across the crate boundary the protocol keeps between itself and the files.
/// `AreaTrigger.dbc` as the session wants it — the one table `vale live`
/// hands the mover so that walking into a portal reports one.
struct Portals(vale_assets::tables::areatrigger::AreaTriggers);

impl vale_protocol::play::areatrigger::Triggers for Portals {
    fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32> {
        self.0.containing(map, point)
    }

    fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool {
        self.0.holds(id, map, point)
    }
}

// --- the mailbox --------------------------------------------------------------
//
// Five verbs over one packet family. **The one thing this can check that
// nothing else can** is the inbox itself: `SMSG_MAIL_LIST_RESULT`'s sender
// field is a union whose width is decided by a tag, and a client that reads it
// wrong loses every letter after the first — which no unit test can catch,
// because the test writes the packet it then reads.

// The mailbox this session last opened, so the six verbs after `mail` do not
// each have to find one. 0 is "none open", which every one of them refuses on —
// and the inbox beside it, because the interface's row numbers are positions in
// the list and the wire's ids are not.
thread_local! {
    static MAILBOX: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static INBOX: std::cell::RefCell<Vec<vale_protocol::play::mail::MailHeader>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Which of the four one-letter verbs a press is.
#[derive(Clone, Copy)]
enum MailKind {
    Delete,
    Return,
    TakeItem,
    TakeMoney,
}

/// **Walk to the nth-nearest mailbox and ask for the inbox.**
///
/// The walk is the same `close_on` every other world verb uses, and it is not
/// optional: `WorldSession::CheckMailBox` runs
/// `GetGameObjectIfCanInteractWith` on every one of the nine mail packets, so a
/// request sent from across the room is answered with silence rather than with
/// a refusal.
fn open_mailbox(live: &vale_protocol::socket::session::LiveSession, n: usize) {
    use vale_protocol::socket::session::MailVerb;
    let Some(rows) = nearby_objects(live) else {
        println!("  no game objects in view");
        return;
    };
    let boxes: Vec<&ObjectRow> = rows.iter().filter(|row| row.object_type == 19).collect();
    let Some(row) = boxes.get(n.saturating_sub(1)) else {
        println!("  only {} mailbox(es) in view", boxes.len());
        return;
    };
    println!(
        "  closing on {} ({:.1}y, guid {:#x}, entry {})",
        row.name, row.distance, row.guid, row.entry
    );
    let guid = row.guid;
    if row.distance > MELEE_RANGE {
        close_on(live, guid);
    }
    MAILBOX.with(|cell| cell.set(guid));
    // **The window opens with no packet** — see `vale_protocol::play::mail`.
    // This is `CheckInbox()`, which is what a real `MAIL_SHOW` ends in.
    live.mail(MailVerb::List(guid));
    drain_mail(live, 3000);
}

/// One letter's words — `CMSG_ITEM_TEXT_QUERY`, and the mark-as-read the
/// reference sends beside it from inside `GetInboxText`.
fn read_letter(live: &vale_protocol::socket::session::LiveSession, row: usize) {
    use vale_protocol::socket::session::MailVerb;
    let mailbox = MAILBOX.with(|cell| cell.get());
    if mailbox == 0 {
        println!("  no mailbox open — run `mail` first");
        return;
    }
    let letter = INBOX.with(|inbox| inbox.borrow().get(row.saturating_sub(1)).cloned());
    let Some(letter) = letter else {
        println!("  no letter {row}");
        return;
    };
    println!(
        "  letter {} \"{}\" — textId {}, template {}",
        letter.id, letter.subject, letter.item_text_id, letter.template_id
    );
    live.mail(MailVerb::MarkAsRead {
        mailbox,
        mail_id: letter.id,
    });
    if letter.item_text_id == 0 {
        println!("  no body id: the words are MailTemplate.dbc's, or there are none");
        return;
    }
    live.mail(MailVerb::TextQuery {
        item_text_id: letter.item_text_id,
        mail_id: letter.id,
    });
    drain_mail(live, 3000);
}

/// The four verbs that name one letter and differ only in their opcode.
fn mail_verb(
    live: &vale_protocol::socket::session::LiveSession,
    arg: Option<&str>,
    kind: MailKind,
) {
    use vale_protocol::socket::session::MailVerb;
    let mailbox = MAILBOX.with(|cell| cell.get());
    if mailbox == 0 {
        println!("  no mailbox open — run `mail` first");
        return;
    }
    let Some(row) = arg.and_then(|a| a.parse::<usize>().ok()) else {
        println!("  usage: <verb> <row>");
        return;
    };
    let letter = INBOX.with(|inbox| inbox.borrow().get(row.saturating_sub(1)).cloned());
    let Some(letter) = letter else {
        println!("  no letter {row}");
        return;
    };
    let mail_id = letter.id;
    // **Which of the two the button would say**, printed rather than assumed:
    // `InboxItemCanDelete` is the client's own rule and the server has a
    // different one — it refuses a delete on a COD letter outright.
    println!(
        "  letter {mail_id}: the button would say {}",
        if letter.can_delete() { "Delete" } else { "Return" }
    );
    live.mail(match kind {
        MailKind::Delete => MailVerb::Delete { mailbox, mail_id },
        MailKind::Return => MailVerb::Return { mailbox, mail_id },
        MailKind::TakeItem => MailVerb::TakeItem { mailbox, mail_id },
        MailKind::TakeMoney => MailVerb::TakeMoney { mailbox, mail_id },
    });
    drain_mail(live, 3000);
}

/// **Send one** — `sendmail <name> <subject…>`, with the default stationery and
/// nothing enclosed.
///
/// Stationery 41 is the parchment every character is offered; a letter with a
/// stationery of 0 is one the *client* refuses to build, which is a gate worth
/// keeping out of this verb's way rather than reproducing here.
fn send_letter(live: &vale_protocol::socket::session::LiveSession, rest: Option<&str>) {
    use vale_protocol::play::mail::OutgoingMail;
    use vale_protocol::socket::session::MailVerb;
    let mailbox = MAILBOX.with(|cell| cell.get());
    if mailbox == 0 {
        println!("  no mailbox open — run `mail` first");
        return;
    }
    let Some((to, subject)) = rest.and_then(|line| line.split_once(' ')) else {
        println!("  usage: sendmail <name> <subject>");
        return;
    };
    println!("  CMSG_SEND_MAIL to {to:?}, subject {subject:?}, stationery 41");
    live.mail(MailVerb::Send(Box::new(OutgoingMail {
        mailbox,
        to: to.to_string(),
        subject: subject.to_string(),
        body: String::new(),
        stationery: DEFAULT_STATIONERY,
        package: 0,
        item: 0,
        money: 0,
        cod: 0,
    })));
    drain_mail(live, 5000);
}

/// `MAIL_STATIONERY_DEFAULT` — the one row of `Stationery.dbc` whose flag bit 0
/// is set, and therefore the only paper a character with an empty bag is
/// offered. See `vale_assets::tables::stationery`.
const DEFAULT_STATIONERY: u32 = 41;

/// **Ask a player in view to trade** — `CMSG_INITIATE_TRADE`, then listen.
///
/// The other side sees a `TRADE_REQUEST` popup; a second `live` answers it
/// with `tradeyes`, and both then get `OPEN_WINDOW`. Everything after that is
/// `SMSG_TRADE_STATUS_EXTENDED` both ways, which [`drain_trade`] prints slot
/// by slot — the only place the fifteen-word record can be seen arriving.
fn trade_with(live: &vale_protocol::socket::session::LiveSession, arg: Option<&str>) {
    use vale_protocol::socket::session::TradeVerb;
    let Some(name) = arg.map(str::trim).filter(|n| !n.is_empty()) else {
        println!("  usage: trade <player name>");
        return;
    };
    let found = {
        use vale_protocol::state::update::ObjectType;
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        let found = world
            .iter()
            .filter(|e| e.object_type == Some(ObjectType::Player))
            .find(|e| world.name_of(e).eq_ignore_ascii_case(name))
            .map(|e| (e.guid, world.name_of(e)));
        found
    };
    let Some((guid, name)) = found else {
        println!("  no player named {name} in view");
        return;
    };
    println!("  CMSG_INITIATE_TRADE {name} ({guid:#x})");
    live.trade(TradeVerb::Initiate(guid));
    drain_trade(live, 5000);
}

/// `tradeitem <bag> <slot> [n]` — a bag square, in the interface's numbering,
/// into trade slot `n` (1..7, default the first). The server echoes the whole
/// offer back, which is what `tradeq` then prints.
fn trade_item(live: &vale_protocol::socket::session::LiveSession, arg: Option<&str>) {
    use vale_protocol::socket::session::TradeVerb;
    let words: Vec<&str> = arg.unwrap_or("").split_whitespace().collect();
    let parsed = (
        words.first().and_then(|w| w.parse::<i32>().ok()),
        words.get(1).and_then(|w| w.parse::<usize>().ok()),
        words.get(2).map_or(Some(1u8), |w| w.parse::<u8>().ok()),
    );
    let (Some(bag), Some(slot), Some(n)) = parsed else {
        println!("  usage: tradeitem <bag> <slot> [tradeslot 1..7]");
        return;
    };
    let Some((server_bag, server_slot)) =
        vale_protocol::play::items::server_container_slot(bag, slot)
    else {
        println!("  bag {bag} slot {slot} is not a square");
        return;
    };
    let Some(trade_slot) = n
        .checked_sub(1)
        .filter(|s| usize::from(*s) < vale_protocol::play::trade::TRADE_SLOT_COUNT)
    else {
        println!("  trade slot must be 1..7");
        return;
    };
    println!("  CMSG_SET_TRADE_ITEM slot {trade_slot} <- bag {server_bag} slot {server_slot}");
    live.trade(TradeVerb::SetItem {
        trade_slot,
        bag: server_bag,
        slot: server_slot,
    });
    drain_trade(live, 3000);
}

/// One bodiless-or-nearly trade verb, then listen.
fn trade_verb(
    live: &vale_protocol::socket::session::LiveSession,
    verb: Option<vale_protocol::socket::session::TradeVerb>,
    usage: &str,
) {
    let Some(verb) = verb else {
        println!("  {usage}");
        return;
    };
    println!("  {verb:?}");
    live.trade(verb);
    drain_trade(live, 3000);
}

/// Listen for whatever the trade family sends back, and print it — the
/// status by name, and an offer slot by slot with the entry resolved where
/// the template has landed.
fn drain_trade(live: &vale_protocol::socket::session::LiveSession, ms: u64) {
    use vale_protocol::play::spells::PlayerEvent;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut quiet = true;
    while std::time::Instant::now() < deadline {
        for event in live.take_events() {
            match event {
                PlayerEvent::TradeStatus(status) => {
                    quiet = false;
                    println!(
                        "  SMSG_TRADE_STATUS {} ({:?}){}",
                        status.code,
                        status.status,
                        status
                            .partner
                            .map(|guid| format!(" from {guid:#x}"))
                            .unwrap_or_default()
                    );
                }
                PlayerEvent::TradeOffer(offer) => {
                    quiet = false;
                    let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
                    println!(
                        "  SMSG_TRADE_STATUS_EXTENDED {}: {} copper, spell {}",
                        if offer.theirs { "theirs" } else { "mine" },
                        offer.money,
                        offer.spell
                    );
                    for (i, item) in offer.items.iter().enumerate() {
                        let Some(item) = item else { continue };
                        let name = world
                            .items
                            .get(&item.entry)
                            .map(|t| t.name.clone())
                            .unwrap_or_else(|| format!("entry {}", item.entry));
                        println!(
                            "    slot {}: {name} x{} (display {}, durability {}/{})",
                            i + 1,
                            item.count,
                            item.display_id,
                            item.durability,
                            item.max_durability
                        );
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if quiet {
        println!("  (nothing from the trade family in {} ms)", ms);
    }
}

/// Listen for whatever the mail family sends back, and print it.
fn drain_mail(live: &vale_protocol::socket::session::LiveSession, ms: u64) {
    use vale_protocol::play::spells::PlayerEvent;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut quiet = true;
    while std::time::Instant::now() < deadline {
        for event in live.take_events() {
            match event {
                PlayerEvent::MailList(list) => {
                    quiet = false;
                    print_inbox(&list);
                    INBOX.with(|inbox| *inbox.borrow_mut() = *list);
                }
                PlayerEvent::MailResult(response) => {
                    quiet = false;
                    println!(
                        "  SMSG_SEND_MAIL_RESULT id {} {:?} {:?}{}{}",
                        response.mail_id,
                        response.action,
                        response.result,
                        response
                            .equip_error
                            .map(|code| format!(" equipError {code}"))
                            .unwrap_or_default(),
                        response
                            .item
                            .map(|(guid, count)| format!(" item {guid:#x} x{count}"))
                            .unwrap_or_default(),
                    );
                }
                PlayerEvent::MailReceived => {
                    quiet = false;
                    println!("  SMSG_RECEIVED_MAIL — something was delivered");
                }
                PlayerEvent::MailNextTime(seconds) => {
                    quiet = false;
                    println!(
                        "  MSG_QUERY_NEXT_MAIL_TIME {seconds} ({})",
                        if seconds == 0.0 { "unread mail waiting" } else { "nothing waiting" }
                    );
                }
                PlayerEvent::ItemText { id, text } => {
                    quiet = false;
                    println!("  SMSG_ITEM_TEXT_QUERY_RESPONSE {id}:\n    {text}");
                }
                _ => {}
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if quiet {
        println!("  (nothing came back — are you standing at the mailbox?)");
    }
}

/// The inbox, one line per letter, with the fields a wrong union width would
/// scramble put first.
fn print_inbox(list: &[vale_protocol::play::mail::MailHeader]) {
    println!("  SMSG_MAIL_LIST_RESULT: {} letter(s)", list.len());
    if list.is_empty() {
        return;
    }
    println!(
        "    {:<4} {:<7} {:<11} {:<10} {:<9} {:<7} {:<6} subject",
        "row", "id", "kind", "sender", "money", "cod", "days"
    );
    for (index, letter) in list.iter().enumerate() {
        let sender = if letter.kind.from_a_player() {
            format!("{:#x}", letter.sender_guid)
        } else {
            format!("#{}", letter.sender_entry)
        };
        println!(
            "    {:<4} {:<7} {:<11} {:<10} {:<9} {:<7} {:<6.1} {}",
            index + 1,
            letter.id,
            format!("{:?}", letter.kind),
            sender,
            letter.money,
            letter.cod,
            letter.days_left,
            letter.subject,
        );
        if let Some(item) = letter.item {
            println!(
                "         parcel: entry {} x{} (durability {}/{})",
                item.entry, item.count, item.durability, item.max_durability
            );
        }
    }
}
