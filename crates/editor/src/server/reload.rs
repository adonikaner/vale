//! Telling the running server to re-read something, and reading its answer.
//!
//! One line of chat out, one line of chat back. That is the whole mechanism,
//! and everything below is about the three ways it can go wrong.
//!
//! ## The line is whispered to the character's own name
//!
//! `ChatHandler::ParseCommands` (`Chat.cpp:2023`) takes a line beginning `.` or
//! `!` as a command — but **it refuses, and returns false, when the account is
//! `SEC_PLAYER` and `PlayerCommands` is off**, and `ProcessChatMessageAfterSecurityCheck`
//! (`Handlers/ChatHandler.cpp:64`) then lets the line through as ordinary chat.
//! A `.reload spell_template` sent as a `/say` on an account without the level
//! for it is therefore **broadcast to everybody in range**, in the server's own
//! words rather than in an error.
//!
//! Whispering it to yourself has the same two outcomes with a harmless second
//! one: the command runs, or a private line appears in your own log. Both the
//! whisper and the say branch of `HandleMessagechatOpcode` run `ParseCommands`
//! (`Handlers/ChatHandler.cpp:256..289`), so nothing is given up by using it.
//!
//! **And the echo is the refusal, with no string to match.** A whisper the
//! server did not take as a command comes back as `CHAT_MSG_WHISPER` and
//! `CHAT_MSG_WHISPER_INFORM` carrying the line verbatim, so
//! [`Answer::NotACommand`] is decided by comparing against what was sent rather
//! than against a sentence that could be reworded.
//!
//! ## One at a time, because nothing correlates a reply to a request
//!
//! The server answers a GM command with a system line and no identifier of any
//! kind. The only correlation available is order, so exactly one command is
//! outstanding at a time and the queue behind it drains as each is answered.
//! That also keeps the wire under `UpdateSpeakTime`, the anti-flood clock every
//! chat message passes through.
//!
//! ## What a success looks like, and why a refusal is quoted rather than matched
//!
//! The confirmations are C++ string literals in vmangos — ``DB table
//! `spell_template` reloaded.`` is `Commands/ServerCommands.cpp:1413` — so they
//! are the same on every install and [`confirms`] matches them.
//!
//! The two refusals are **not**. `LANG_NO_CMD` and `LANG_COMMAND_UNAVAILABLE`
//! are rows 6 and 50 of the `mangos_string` table ("There is no such command",
//! "This command is not available to you." on the install this was read from),
//! which any server may have reworded or translated. So nothing here matches
//! them: every system line that arrives while a command is outstanding is kept
//! ([`Request::heard`]) and reported in the server's own words. A person reading
//! "This command is not available to you." learns more than they would from this
//! crate's guess at what that meant.
//!
//! ## `.reload` is `SEC_DEVELOPER`
//!
//! `Chat.cpp`'s command table puts the whole `reload` subtree at `SEC_DEVELOPER`
//! (5 of the 7 levels in `shared/Common.h:183`), which is a level above
//! `SEC_GAMEMASTER`. An account that can `.tele` cannot necessarily `.reload`,
//! and the failure is the quoted refusal above rather than anything this end can
//! predict — the wire carries no way to ask what level the session has.

use crate::session::EditSession;
use vale_client::game::events::ChatMessageReceived;
use vale_client::world::session::{Session, WorldStatus};
use vale_protocol::play::chat::ChatType;
use bevy::prelude::*;
use std::collections::VecDeque;

/// **How long a command waits for an answer before it is given up on.**
///
/// A reload is real work: `.reload spell_template` re-reads every row of a
/// 30,000-row table and rebuilds the spell store off it, on the world thread.
/// So this is long enough that a slow answer is not reported as no answer, and
/// short enough that a scripted run does not sit on it. Being generous costs
/// only the delay before a genuinely silent command is reported as silent.
///
/// **Do not shorten it to the round trip.** Measured at 1.9 s end to end for
/// `.reload spell_template` on a local server, and the larger half of that is
/// this end: `game::session::chat::poll` holds every line until the Lua host
/// has `Interface\FrameXML\` loaded, which on a debug first login lands about
/// 3.4 s after the session exists. Nothing is lost — the backlog holds 256
/// lines — but a command sent before the interface is up is answered after it,
/// and a patience cut to the network time would report a live server as
/// silent.
const PATIENCE: f64 = 10.0;

/// How many answered commands are kept to be looked at.
///
/// A bound rather than a number with an argument behind it: this is a log of
/// the last few things said to the server, and a session that leaves the editor
/// open for a day must not grow one entry per save for ever.
const KEPT: usize = 32;

/// What became of one command.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Answer {
    /// Written down, with something else outstanding or no session to send on.
    #[default]
    Queued,
    /// On the wire, waiting.
    Waiting,
    /// The server answered, in its own words — see [`confirms`].
    Answered(String),
    /// **It came back as chat.** The server did not treat it as a command at
    /// all, which is `ParseCommands`' `SEC_PLAYER` branch and means the account
    /// this playtest logged in on has no GM level. The module comment is why
    /// this is recognisable rather than guessed at.
    NotACommand,
    /// [`PATIENCE`] passed with nothing that answers it. Whatever the server
    /// did say in that window is in [`Request::heard`].
    Silent,
    /// There was no session to send it on, or the playtest ended under it.
    Nowhere,
}

impl Answer {
    /// Whether this command is still going to change.
    pub fn pending(&self) -> bool {
        matches!(self, Answer::Queued | Answer::Waiting)
    }

    /// One line for a person: what happened, in the server's words where there
    /// are any.
    pub fn line(&self, command: &str) -> String {
        match self {
            Answer::Queued => format!("{command}: waiting to go out"),
            Answer::Waiting => format!("{command}: sent"),
            Answer::Answered(said) => format!("{command}: {said}"),
            Answer::NotACommand => format!(
                "{command}: the server said it back instead of running it — \
                 this account has no GM level"
            ),
            Answer::Silent => format!("{command}: no answer in {PATIENCE:.0}s"),
            Answer::Nowhere => format!("{command}: no playtest to send it on"),
        }
    }
}

/// One command, from written down to answered.
#[derive(Debug, Clone)]
pub struct Request {
    /// The line as it goes on the wire, leading `.` included.
    pub command: String,
    /// What a confirmation has to name in backticks — the bare table name for
    /// a `.reload`. Empty for a command whose answer this cannot recognise, in
    /// which case the first system line to arrive is taken as the answer.
    pub names: String,
    /// When it went out, on the app's own clock.
    pub sent_at: Option<f64>,
    pub answer: Answer,
    /// Every system line that arrived while it was outstanding, in order.
    ///
    /// Kept rather than interpreted: a refusal is a `mangos_string` row and so
    /// is not this crate's to recognise. See the module comment.
    pub heard: Vec<String>,
}

impl Request {
    /// `.reload <table>`, the only shape anything sends today.
    pub fn table(table: &str) -> Self {
        Self {
            command: format!(".reload {table}"),
            names: table.to_string(),
            sent_at: None,
            answer: Answer::Queued,
            heard: Vec::new(),
        }
    }

    /// One line for a person — the command and what became of it.
    pub fn line(&self) -> String {
        match (&self.answer, self.heard.first()) {
            // Nothing came back that answers it, but the server did say
            // something. That something is nearly always the refusal, and it
            // is worth more than the word "silent".
            (Answer::Silent, Some(said)) => format!("{}: {said}", self.command),
            _ => self.answer.line(&self.command),
        }
    }
}

/// What has been said to the server and what came back.
#[derive(Resource, Default)]
pub struct Reloads {
    /// Written down, not yet sent. One goes out at a time.
    queue: VecDeque<Request>,
    /// On the wire.
    outstanding: Option<Request>,
    /// The last [`KEPT`], newest last.
    pub answered: Vec<Request>,
    /// Tables to reload as soon as there is a session to say it on — see
    /// [`Reloads::when_there_is_a_session`].
    deferred: Vec<String>,
}

impl Reloads {
    /// Ask the server to re-read one of its tables.
    ///
    /// Queued rather than sent: the send is a system, so this can be called
    /// from anywhere that can reach the resource, including from a `&mut World`
    /// with no session in hand.
    pub fn table(&mut self, table: &str) {
        self.ask(Request::table(table));
    }

    /// …and any other GM line, for a command this does not know the shape of.
    pub fn ask(&mut self, request: Request) {
        self.queue.push_back(request);
    }

    /// **Ask once there is a session to ask on.**
    ///
    /// [`Self::table`] fails a command with nowhere to go, on purpose — a queue
    /// that outlived a playtest would send itself at whatever server the next
    /// one logged into. This is the case that rule was wrong about: **a
    /// playtest being started is the moment an edit is supposed to go live**,
    /// and at that moment the session does not exist yet.
    ///
    /// vmangos reads `spell_template` at *its* startup and never again, so a
    /// row applied while the editor was editing is a row the running server has
    /// not seen. Without this, pressing Playtest played the *old* spell and the
    /// first save inside the world was what fixed it — which is a reload
    /// arriving one gesture after the change that needed it.
    ///
    /// Deferred rather than queued: it waits for a session and is sent on the
    /// frame one appears. A playtest that never connects drops it, because
    /// [`forget_the_deferred`] clears the list when the playtest ends.
    pub fn when_there_is_a_session(&mut self, table: &str) {
        let table = table.to_string();
        if !self.deferred.contains(&table) {
            self.deferred.push(table);
        }
    }

    /// How many are waiting for a session — for the panel, and for the test.
    pub fn deferred(&self) -> usize {
        self.deferred.len()
    }

    /// Whether anything is queued or on the wire.
    pub fn busy(&self) -> bool {
        self.outstanding.is_some() || !self.queue.is_empty()
    }

    /// How many are still to be answered, for a status line.
    pub fn pending(&self) -> usize {
        self.queue.len() + usize::from(self.outstanding.is_some())
    }

    /// The one thing worth putting on a status line: what is happening now, or
    /// what happened last.
    pub fn status(&self) -> Option<String> {
        if let Some(out) = self.outstanding.as_ref() {
            return Some(match self.queue.len() {
                0 => out.line(),
                n => format!("{} ({n} more)", out.line()),
            });
        }
        self.answered.last().map(Request::line)
    }

    /// Everything, in the order it was asked — for a panel.
    pub fn all(&self) -> impl Iterator<Item = &Request> {
        self.answered
            .iter()
            .chain(self.outstanding.iter())
            .chain(self.queue.iter())
    }

    /// Move the outstanding request to the history with its answer on it.
    fn settle(&mut self, answer: Answer) {
        let Some(mut request) = self.outstanding.take() else {
            return;
        };
        request.answer = answer;
        match &request.answer {
            Answer::Answered(_) => info!("server: {}", request.line()),
            _ => warn!("server: {}", request.line()),
        }
        self.answered.push(request);
        if self.answered.len() > KEPT {
            let over = self.answered.len() - KEPT;
            self.answered.drain(..over);
        }
    }
}

/// **Whether a system line is the server confirming that `names` was re-read.**
///
/// The rule is the shape every one of vmangos' confirmations has: the name in
/// backticks and the word *reloaded*. It is deliberately not an equality test
/// against one sentence, because the sentences differ — some carry a
/// parenthetical (``DB table `creature_involvedrelation` (creature quest takers)
/// reloaded.``) and the wildcard forms name several tables at once (``DB tables
/// `*_loot_template` reloaded.``).
///
/// An empty `names` matches any line, which is the honest answer for a command
/// whose confirmation this does not know: the next thing the server says is the
/// answer, whatever it is.
pub fn confirms(line: &str, names: &str) -> bool {
    if names.is_empty() {
        return true;
    }
    line.contains(&format!("`{names}`")) && line.contains("reloaded")
}

/// Put the next command on the wire.
///
/// **It can only run while a playtest is in the world.** Anything queued with
/// no session is answered [`Answer::Nowhere`] rather than held: a queue that
/// survives the end of a playtest would send the whole of it at whatever server
/// the next one logs into.
fn send(
    time: Res<Time>,
    client: Res<Session>,
    status: Res<WorldStatus>,
    mut reloads: ResMut<Reloads>,
) {
    let Some(active) = client.active.as_ref() else {
        // The playtest ended, or there never was one. Fail everything held
        // rather than keeping it.
        //
        // **Guarded on there being something to fail**, and that is not
        // tidiness: `ResMut`'s `DerefMut` marks the resource changed whether or
        // not anything moved, and [`report`] writes the status line off exactly
        // that mark. Without the guard every frame of every session with no
        // playtest in it would rewrite the status line with the last thing the
        // server said, for as long as the editor was open.
        if !reloads.busy() {
            return;
        }
        if reloads.outstanding.is_some() {
            reloads.settle(Answer::Nowhere);
        }
        while let Some(mut request) = reloads.queue.pop_front() {
            request.answer = Answer::Nowhere;
            warn!("server: {}", request.line());
            reloads.answered.push(request);
        }
        return;
    };
    // Read-only until there is something to send, for the reason the branch
    // above gives: a `DerefMut` here would mark the resource changed on every
    // frame of every playtest.
    if reloads.outstanding.is_some() || reloads.queue.is_empty() {
        return;
    }
    let Some(mut request) = reloads.queue.pop_front() else {
        return;
    };
    // **Whispered to ourselves** — see the module comment, which is the whole
    // of why this is not a say. The name is the one the session is playing,
    // which `WorldStatus` carries; with no name there is nothing to whisper to
    // and the command is not sent at all.
    let me = status.character.trim().to_string();
    if me.is_empty() {
        request.answer = Answer::Nowhere;
        warn!("server: {}", request.line());
        reloads.answered.push(request);
        return;
    }
    info!("server: sending {}", request.command);
    active
        .live
        .say(ChatType::Whisper, Some(me), request.command.clone());
    request.sent_at = Some(time.elapsed_secs_f64());
    request.answer = Answer::Waiting;
    reloads.outstanding = Some(request);
}

/// **Send what was waiting for a session, on the frame one exists.**
///
/// The other half of [`Reloads::when_there_is_a_session`]. It waits for the
/// character to be *in the world* rather than merely for a socket:
/// `ParseCommands` refuses a command from a session whose player is not in the
/// world and logs it server-side (`Chat.cpp:2059`), so a line sent at the
/// handshake is a line silently dropped.
fn send_the_deferred(client: Res<Session>, status: Res<WorldStatus>, mut reloads: ResMut<Reloads>) {
    if reloads.deferred.is_empty() {
        return;
    }
    // A name and a live session: the two halves of "there is somebody to say
    // this as". `WorldStatus::character` is empty until the world is entered.
    if client.active.is_none() || status.character.trim().is_empty() {
        return;
    }
    for table in std::mem::take(&mut reloads.deferred) {
        info!("server: {table} was applied before this playtest — reloading it now");
        reloads.table(&table);
    }
}

/// …and drop it when the playtest ends without ever connecting.
///
/// A deferred reload is about *this* playtest. Carrying one to the next would
/// be the failure [`Reloads::table`] refuses by design — a command sent at
/// whatever server the next login reaches — with the difference that this one
/// would arrive a session late rather than not at all.
fn forget_the_deferred(
    // **The playtest's state and not the socket**, which is the whole of why
    // this is not two lines. `Session::active` is `None` for the several
    // seconds a login takes, so a guard on the socket would throw the list away
    // in the gap between pressing Playtest and arriving — which is exactly the
    // window it exists to cross.
    state: Res<crate::playtest::Playtest>,
    mut reloads: ResMut<Reloads>,
) {
    if !state.editing() || reloads.deferred.is_empty() {
        return;
    }
    let waiting = std::mem::take(&mut reloads.deferred);
    warn!(
        "server: {} table(s) were applied and never reloaded — no playtest connected",
        waiting.len()
    );
}

/// Match what the server said against what is outstanding.
fn listen(mut arrived: MessageReader<ChatMessageReceived>, mut reloads: ResMut<Reloads>) {
    for line in arrived.read() {
        // **Read-only while nothing is outstanding**, for the reason [`send`]
        // gives: a `DerefMut` on a frame that carries chat and no command would
        // mark the resource changed and rewrite the status line.
        let Some(out) = reloads.outstanding.as_ref() else {
            continue;
        };
        match line.event {
            "CHAT_MSG_SYSTEM" => match confirms(&line.text, &out.names) {
                true => reloads.settle(Answer::Answered(line.text.clone())),
                false => {
                    if let Some(out) = reloads.outstanding.as_mut() {
                        out.heard.push(line.text.clone());
                    }
                }
            },
            // The line came back as chat: `ParseCommands` did not take it.
            "CHAT_MSG_WHISPER" | "CHAT_MSG_WHISPER_INFORM" if line.text == out.command => {
                reloads.settle(Answer::NotACommand);
            }
            _ => {}
        }
    }
}

/// …and give up on one nothing answers.
fn give_up(time: Res<Time>, mut reloads: ResMut<Reloads>) {
    let now = time.elapsed_secs_f64();
    let late = reloads
        .outstanding
        .as_ref()
        .and_then(|out| out.sent_at)
        .is_some_and(|at| now - at >= PATIENCE);
    if late {
        reloads.settle(Answer::Silent);
    }
}

/// Put what is happening on the status line, so the shell over a playtest says
/// it as well as the log.
fn report(reloads: Res<Reloads>, session: Option<ResMut<EditSession>>) {
    let (Some(mut session), true) = (session, reloads.is_changed()) else {
        return;
    };
    if let Some(line) = reloads.status() {
        session.status = line;
    }
}

/// `--reload <table>[,<table>…]`, which is the only way this is reached without
/// a person — see [`crate::Args::reload`].
///
/// It fires once, on the first frame a playtest is in the world, and then never
/// again: the flag is a check that the wire works, not a thing to repeat every
/// time a session comes back.
fn on_the_command_line(
    args: Res<crate::Args>,
    client: Res<Session>,
    mut reloads: ResMut<Reloads>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(tables) = args.reload.as_deref() else {
        *done = true;
        return;
    };
    if client.active.is_none() {
        return;
    }
    *done = true;
    for table in tables.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        info!("--reload: asking the server to re-read {table}");
        reloads.table(table);
    }
}

pub struct ReloadPlugin;

impl Plugin for ReloadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reloads>().add_systems(
            Update,
            // **Answers first, then the next one out**, so a command answered
            // this frame frees the wire for the next one on the same frame
            // rather than a frame later. `report` last, because it is a view of
            // what the three before it did.
            (
                listen,
                give_up,
                on_the_command_line,
                // **Before `send`**, so a table that has been waiting for a
                // session goes out on the same frame the session arrives
                // rather than the next one — and before `forget`, so the
                // sending path wins over the dropping path on any frame both
                // could run.
                send_the_deferred,
                forget_the_deferred,
                send,
                report,
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The confirmations, copied from vmangos' own source.**
    ///
    /// Every one of these is a C++ string literal in
    /// `src/game/Commands/ServerCommands.cpp`, which is why they can be matched
    /// at all — see the module comment on why the *refusals* cannot be.
    const CONFIRMATIONS: [(&str, &str); 5] = [
        ("spell_template", "DB table `spell_template` reloaded."),
        (
            "creature_template",
            "DB table `creature_template` reloaded.",
        ),
        (
            "creature_involvedrelation",
            "DB table `creature_involvedrelation` (creature quest takers) reloaded.",
        ),
        ("*_loot_template", "DB tables `*_loot_template` reloaded."),
        (
            "*_questrelation",
            "DB tables `*_questrelation` and `*_involvedrelation` reloaded.",
        ),
    ];

    #[test]
    fn every_shape_of_confirmation_vmangos_sends_is_recognised() {
        for (table, line) in CONFIRMATIONS {
            assert!(confirms(line, table), "{line}");
        }
    }

    /// The one that matters: a confirmation for a *different* table must not
    /// answer this command. The wire is one at a time and matched by order, so
    /// a line taken for the wrong request would report an edit as live when it
    /// is not.
    #[test]
    fn a_confirmation_for_another_table_does_not_answer() {
        assert!(!confirms(
            "DB table `creature_template` reloaded.",
            "spell_template"
        ));
        // …and a name that is a substring of the one that came back. The
        // backticks are what make this safe rather than the name.
        assert!(!confirms("DB table `spell_template` reloaded.", "spell"));
    }

    /// The two refusals are matched by nothing, on purpose.
    #[test]
    fn a_refusal_is_not_a_confirmation() {
        for line in [
            "There is no such command",
            "This command is not available to you.",
            "Incorrect syntax.",
        ] {
            assert!(!confirms(line, "spell_template"), "{line}");
        }
    }

    /// A command whose answer is not known takes the next thing the server
    /// says, whatever it is.
    #[test]
    fn an_unrecognised_command_takes_the_next_line() {
        assert!(confirms("Anything at all.", ""));
    }

    #[test]
    fn a_reload_is_spelled_the_way_the_server_reads_it() {
        let request = Request::table("spell_template");
        assert_eq!(request.command, ".reload spell_template");
        assert_eq!(request.names, "spell_template");
        assert!(request.answer.pending());
    }

    /// The history is bounded, and it is the *oldest* that go.
    #[test]
    fn the_history_keeps_the_newest() {
        let mut reloads = Reloads::default();
        for n in 0..KEPT + 5 {
            reloads.outstanding = Some(Request::table(&format!("t{n}")));
            reloads.settle(Answer::Answered("ok".into()));
        }
        assert_eq!(reloads.answered.len(), KEPT);
        assert_eq!(reloads.answered[0].names, "t5");
        assert_eq!(
            reloads.answered.last().unwrap().names,
            format!("t{}", KEPT + 4)
        );
    }

    /// A silent command reports what the server *did* say, because that is
    /// nearly always the refusal.
    #[test]
    fn a_silent_command_quotes_what_was_heard() {
        let mut request = Request::table("spell_template");
        request.answer = Answer::Silent;
        request
            .heard
            .push("This command is not available to you.".into());
        assert!(request.line().contains("not available to you"));
    }

    #[test]
    fn nothing_is_pending_once_it_is_answered() {
        assert!(Answer::Queued.pending());
        assert!(Answer::Waiting.pending());
        for answer in [
            Answer::Answered("ok".into()),
            Answer::NotACommand,
            Answer::Silent,
            Answer::Nowhere,
        ] {
            assert!(!answer.pending());
        }
    }
}
