//! **The training window as the panel indexes it**: the headers, the order, and
//! the three filters that decide which lines exist at all.
//!
//! `Blizzard_TrainerUI` never asks about a spell id. It asks
//! `GetNumTrainerServices()` and then `GetTrainerServiceInfo(i)` for each row's
//! `(name, subText, type, isExpanded)` — so **the whole panel is arithmetic over
//! one ordered array**, exactly as the spellbook is (see [`crate::tables::book`], which
//! is the same shape one door along). The wire carries none of that order:
//! `SMSG_TRAINER_LIST` is a bag of services in the server's map order, and the
//! grouping, the sorting and the filtering are all the client's.
//!
//! ## The rules
//!
//! ```text
//! intake         per service, its group id is the *taught* skill line
//!                (SkillLineAbility masked by race and class
//!                with NO tab gate); a service whose line is 0 is DROPPED; the
//!                group's per-state counts go up; then one header row per group is
//!                appended with spell = -1 and the group id in its second field
//! group order    (class/talent/pet): the -1 group first, then groups
//!                all of whose services carry a point cost last, then by the skill
//!                line's own name, case-insensitively
//! group order    (tradeskill): by group id, numerically — 1 Step, 2 Learn
//! rebuild        a group is shown when some state the type filter allows
//!                has a row in it AND its own bit is in the skill-line filter; a
//!                service is shown when its state is in the type filter, its group
//!                is shown, and its group is expanded; a HEADER ignores the type
//!                filter and the collapse. Then sort by trainer type.
//! service order  visible before hidden, then group position, then a
//!                header before its own services, then required level ascending,
//!                then required skill rank ascending, then spell name and rank
//!                (case-SENSITIVE — the other way round from the group
//!                comparator)
//! ```
//!
//! Two of those are the ones that would be guessed wrong. **A class trainer has
//! headers**: `ClassTrainer_SelectFirstLearnableSkill` selecting index *2* is
//! not an off-by-one, it is skipping the first skill line's header row. And
//! **the group a row lands in is the line of the spell as the packet names it**,
//! which the spellbook's own `line_of` would answer 0 for on most of them,
//! because the spellbook applies a tab gate this path does not — see
//! [`Skills::ability_line`].
//!
//! ## Three pseudo-groups are a `GlobalStrings` key rather than a name
//!
//! A tradeskill trainer's two headers are `TRADESKILL_SERVICE_STEP` and
//! `TRADESKILL_SERVICE_LEARN` — a literal table in the client,
//! indexed by the group id — and a talent trainer's "already known" group is
//! `KNOWN_TALENTS_HEADER`. All three are *keys*, looked up through
//! the client's own string table, so [`Group::title`] is the one door for
//! resolving them, exactly as [`crate::tables::book::Spellbook::tab_name`] is.
//!
//! ## What is here and what is one layer up
//!
//! This module holds the **rule**: given a service list and a character's race
//! and class, what the rows are, in what order, and which of them are visible.
//! It needs no session, no socket and no window. The *player's* trainer — the
//! packet arriving, the presses going out — is
//! `crates/client/src/game/trainer.rs`.
//!
//! **The input is [`Service`] rather than `SMSG_TRAINER_LIST`'s own struct**,
//! and that is the crate graph rather than a preference: `vale-assets` does
//! not depend on `vale-protocol` and should not start, so the packet is
//! decoded there and handed here as plain fields. The one field that needs
//! saying twice is [`State`], because its three *words* are strings out of the
//! client's own pool rather than anything the server sends — the
//! wire's side of it is `vale_protocol::play::trainer::TrainerState`, and the two
//! agree by the byte, which is what the type filter shifts by.

use crate::tables::skills::{Skills, GENERAL};
use crate::tables::spellbook::{SpellInfo, Spells};

/// **The two tables the rules need, behind one door.**
///
/// A trait rather than a pair of arguments because the *character* is baked
/// into the skill-line question — the client reads the race and class off the
/// unit — so a caller that passed the tables would have to pass those too, at
/// every call, and a test that only cares about the ordering would have to
/// stand up two DBCs to ask it.
pub trait Catalog {
    /// `Spell.dbc`, or `None` for an id it has no row for — which a real
    /// service list does contain.
    fn spell(&self, id: u32) -> Option<SpellInfo>;
    /// **Which skill line this spell is on for this character**, gate-free —
    /// see [`Skills::ability_line`], which is where the gate's absence is
    /// argued. [`GENERAL`] means the row is dropped.
    fn skill_line(&self, spell: u32) -> u32;
    /// What a skill line is called, for the header and for the requirement
    /// line. Empty for one the table does not carry.
    fn skill_line_name(&self, line: u32) -> String;
}

/// The ordinary [`Catalog`]: the shipped tables, for one character.
pub struct Tables<'a> {
    pub spells: &'a Spells,
    /// **Absent is an empty window**, not a flat one: every service resolves to
    /// [`GENERAL`] and the intake drops those. That is the reference's own
    /// behaviour — with no `SkillLineAbility.dbc` it has nothing to head the
    /// list with.
    pub skills: Option<&'a Skills>,
    pub race: u8,
    pub class: u8,
}

impl Catalog for Tables<'_> {
    fn spell(&self, id: u32) -> Option<SpellInfo> {
        self.spells.info(id)
    }

    fn skill_line(&self, spell: u32) -> u32 {
        self.skills
            .map_or(GENERAL, |s| s.ability_line(spell, self.race, self.class))
    }

    fn skill_line_name(&self, line: u32) -> String {
        self.skills
            .and_then(|s| s.line(line))
            .map_or_else(String::new, |l| l.name.clone())
    }
}

/// What kind of trainer this is — the one thing that changes how the whole
/// window is grouped. `vale_protocol::play::trainer::TrainerType`'s twin; see the
/// module note on why there are two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Class,
    Talent,
    Tradeskill,
    Pet,
}

/// What the character may do about one service, and **the three words the
/// interface asks for**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Available,
    Unavailable,
    Used,
}

impl State {
    /// The word `GetTrainerServiceInfo` answers with — the client's own
    /// strings, and the same three words
    /// `SetTrainerServiceTypeFilter` parses back.
    pub fn word(self) -> &'static str {
        match self {
            State::Available => "available",
            State::Unavailable => "unavailable",
            State::Used => "used",
        }
    }

    /// **Which bit of the type filter is this state's** — the wire byte itself.
    pub fn index(self) -> usize {
        match self {
            State::Available => 0,
            State::Unavailable => 1,
            State::Used => 2,
        }
    }
}

/// One thing a trainer will teach, as the rules need it — the decoded half of
/// `SMSG_TRAINER_LIST`'s row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    /// **The teaching spell**, which is what the name and rank are read from
    /// and what `CMSG_TRAINER_BUY_SPELL` echoes.
    pub spell: u32,
    pub state: State,
    /// Copper, the reputation discount already applied.
    pub cost: u32,
    /// Talent and profession points — `GetTrainerServiceCost`'s other two.
    pub point_cost: (u32, u32),
    pub req_level: u8,
    pub req_skill: u32,
    pub req_skill_value: u32,
    pub req_spells: [u32; 3],
}

/// `SPELL_EFFECT_LEARN_SPELL` — `0x24`, the effect every class-trainer service
/// carries and the one the client scans all three slots for.
pub const EFFECT_LEARN_SPELL: u32 = 36;

/// `SPELL_EFFECT_LEARN_PET_SPELL` — `0x39`, scanned in the same loop and the
/// only difference between a hunter's own row and its pet's.
pub const EFFECT_LEARN_PET_SPELL: u32 = 57;

/// `SPELL_EFFECT_SKILL_STEP` — `0x2c`, which is what tells a tradeskill
/// trainer's *Step* rows from its *Learn* ones.
pub const EFFECT_SKILL_STEP: u32 = 44;

/// The group a tradeskill trainer's rank-up rows go under.
pub const TRADESKILL_STEP_GROUP: i32 = 1;
/// …and its recipe rows.
pub const TRADESKILL_LEARN_GROUP: i32 = 2;
/// The group a talent trainer folds everything already known into. Negative
/// because the client uses the sign of that field for something else entirely —
/// see [`Row::service`].
pub const KNOWN_TALENTS_GROUP: i32 = -1;

/// The `GlobalStrings.lua` keys the three pseudo-groups are named by — see the
/// module note. Indexed by [`Group::key`].
const STEP_KEY: &str = "TRADESKILL_SERVICE_STEP";
const LEARN_KEY: &str = "TRADESKILL_SERVICE_LEARN";
const KNOWN_TALENTS_KEY: &str = "KNOWN_TALENTS_HEADER";

/// The filter bit for one state — **the state byte itself**, which is what
/// the client shifts by and what `SetTrainerServiceTypeFilter`'s word table
/// (`available` 0, `unavailable` 1, `used` 2) agrees with.
fn state_bit(state: State) -> u32 {
    1 << state.index()
}

/// Which word `SetTrainerServiceTypeFilter` was given, as a bit. `None` for a
/// word the client would refuse — it answers 6 for anything it does not
/// know and the caller prints *Bad service type*.
pub fn filter_bit(word: &str) -> Option<u32> {
    Some(match word {
        "available" => 1,
        "unavailable" => 2,
        "used" => 4,
        _ => return None,
    })
}

/// **Every filter, all on** — `SetTrainerServiceTypeFilter("all", …)`'s own
/// constant, `7`.
pub const ALL_TYPES: u32 = 7;

/// The type filter a window opens with: available and unavailable, **not** used
/// (3), or available and used at a talent trainer
/// (5). The addon's three saved variables default to the
/// same three values, so this is what a first visit shows either way.
pub fn default_types(kind: Kind) -> u32 {
    match kind {
        Kind::Talent => 5,
        _ => 3,
    }
}

/// One header's group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The `SkillLine` id, or one of the three pseudo-groups.
    pub id: i32,
    /// What the header says. **Empty when [`Group::key`] is set** — see
    /// [`Group::title`], which is the one door.
    pub name: String,
    /// The `GlobalStrings.lua` key this group is named by, for the three
    /// pseudo-groups; `None` for a real skill line.
    pub key: Option<&'static str>,
    /// How many services in this group are in each state — indexed by the state
    /// byte, which is what the filter shifts by.
    pub counts: [usize; 3],
    /// Whether **every** service in it costs talent or profession points, which
    /// is what sorts a primary profession's group last. The client computes it
    /// as a sticky AND: a new group starts at whatever its first service says
    /// and a service with no point cost clears it for good.
    pub all_points: bool,
}

impl Group {
    /// What to draw on the header — the skill line's own name, or the words
    /// behind the key for the three pseudo-groups.
    ///
    /// **A key with no entry answers the key**, which is this client's rule for
    /// every `GlobalStrings` lookup: a visible name nobody wrote is a bug
    /// report, where an empty header is a blank line.
    pub fn title(&self, strings: impl Fn(&str) -> Option<String>) -> String {
        match self.key {
            Some(key) => strings(key).unwrap_or_else(|| key.to_string()),
            None => self.name.clone(),
        }
    }
}

/// One line of the window — a header, or something to learn.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Which [`Board::groups`] entry this line belongs to.
    pub group: usize,
    /// The service's index in the packet's own list, or `None` for a header.
    ///
    /// The client stores this as the *sign* of the row's first field — a header
    /// is `-1` where a service holds its spell id — which is why every accessor
    /// begins by testing that field against zero.
    pub service: Option<usize>,
    /// The teaching spell's id, as the wire gave it; `0` for a header. What
    /// `CMSG_TRAINER_BUY_SPELL` echoes.
    pub spell: u32,
    /// The spell's own `Spell.dbc` name — **not** the taught spell's; empty for
    /// a header, which draws [`Group::title`] instead.
    pub name: String,
    /// …and its rank, `GetTrainerServiceInfo`'s second answer.
    pub sub_text: String,
    /// **The taught skill line's own name** — `GetTrainerServiceSkillLine`'s
    /// answer: the first `LEARN_*` slot's trigger spell — the
    /// service spell itself when it has none — through its `SkillLineAbility`
    /// row into `SkillLine.dbc`'s name. **Not the group title**: a tradeskill
    /// trainer's groups are the Step/Learn pseudo-headers, and the
    /// profession-learn popup formats this name into its sentence — which is
    /// how it read "Development Skills" for one session. Empty for a header,
    /// and for a line the tables cannot name.
    pub line_name: String,
    pub icon: Option<String>,
    /// The spell's own description, **unsubstituted** — `$s1` and its siblings
    /// are resolved at the read, by whoever has the character's level.
    pub description: String,
    /// `None` for a header, which is how `GetTrainerServiceInfo` answers
    /// `"header"` for the third return.
    pub state: Option<State>,
    pub cost: u32,
    pub point_cost: (u32, u32),
    pub req_level: u8,
    /// The required skill line's name and rank, `None` when there is none.
    pub req_skill: Option<(String, u32)>,
    /// The prerequisite spells that are actually set, named — the zero padding
    /// is gone, which is what `GetTrainerServiceNumAbilityReq` counts.
    pub req_spells: Vec<u32>,
    /// Whether the service teaches a spell at all, and whether it teaches it to
    /// a *pet* — `IsTrainerServiceLearnSpell`'s two answers, off the same
    /// three-slot effect scan the grouping uses.
    pub learn_spell: bool,
    pub learn_pet_spell: bool,
    /// Whether this line is on screen at all under the current three filters.
    /// Sorted to the front, and counted by [`Board::visible`].
    pub visible: bool,
    /// The sort key the required-skill rank supplies, kept apart from
    /// [`Row::req_skill`] because a header has none and still has to compare.
    req_skill_value: u32,
}

/// A trainer's window: the groups, the rows they index, and the three masks.
#[derive(Debug, Default)]
pub struct Board {
    pub kind: Kind,
    pub groups: Vec<Group>,
    /// Every row, in the client's own order — **visible first**, which is what
    /// lets `GetTrainerServiceInfo(i)` index this directly for `i` up to
    /// [`Board::visible`].
    pub rows: Vec<Row>,
    visible: usize,
    types: u32,
    lines: u32,
    expanded: u32,
    /// Which row `SelectTrainerService` picked, by **spell id** rather than by
    /// position — the client keeps the id and re-finds the row on
    /// every rebuild, which is the whole reason a selection
    /// survives a filter change.
    ///
    /// **An atomic, and not for threads.** `SelectTrainerService` is one of the
    /// two interface writes in this client that cannot be recorded and drained a
    /// system later: `ClassTrainer_SetSelection` calls it and
    /// `ClassTrainerFrame_Update`, four lines further down the same click, asks
    /// `GetTrainerSelectionIndex()` to decide which row to draw the highlight
    /// bar on. A frame of lag there is a click that fills the detail pane with
    /// the spell you pressed and moves the highlight to the one you pressed
    /// *before* it. The client stores the selection immediately, and it
    /// signals no event. `SelectQuestLogEntry` is the same shape
    /// one panel over; `crate::client`'s `game::quest::Quests::selected` carries
    /// the same note.
    selected: std::sync::atomic::AtomicU32,
}

impl Board {
    /// Build a window from what the server sent.
    ///
    /// A service whose taught spell is on no skill line for this character is a
    /// group id of [`GENERAL`], and the intake **drops** those —
    /// see [`Tables::skills`] for what that means when the table is missing
    /// altogether.
    pub fn build(kind: Kind, services: &[Service], catalog: &dyn Catalog) -> Board {
        let mut groups: Vec<Group> = Vec::new();
        // Rows keyed by their group **id** while the groups are still in
        // insertion order; the positions are filled in once they are sorted.
        let mut rows: Vec<(i32, Row)> = Vec::new();

        for (index, service) in services.iter().enumerate() {
            let info = catalog.spell(service.spell);
            let described = info.clone().unwrap_or_default();
            let id = group_of(kind, service.state, service.spell, &described, catalog);
            // **Line 0 is dropped, not shown under a General header.** The
            // client decrements its own counter and moves on.
            if id == GENERAL as i32 {
                continue;
            }
            let points = service.point_cost.0 != 0 || service.point_cost.1 != 0;
            let position = match groups.iter().position(|g| g.id == id) {
                Some(position) => {
                    // The sticky AND — see [`Group::all_points`].
                    groups[position].all_points &= points;
                    position
                }
                None => {
                    groups.push(new_group(id, catalog, points));
                    groups.len() - 1
                }
            };
            groups[position].counts[service.state.index()] += 1;
            rows.push((id, Row {
                group: position,
                service: Some(index),
                spell: service.spell,
                name: info.as_ref().map_or_else(String::new, |i| i.name.clone()),
                sub_text: info.as_ref().map_or_else(String::new, |i| i.rank.clone()),
                line_name: {
                    let taught = taught_spell(&described);
                    let line =
                        catalog.skill_line(if taught != 0 { taught } else { service.spell });
                    catalog.skill_line_name(line)
                },
                icon: info.as_ref().and_then(|i| non_empty(&i.icon)),
                description: info
                    .as_ref()
                    .map_or_else(String::new, |i| i.description.clone()),
                state: Some(service.state),
                cost: service.cost,
                point_cost: service.point_cost,
                req_level: service.req_level,
                req_skill: (service.req_skill != 0)
                    .then(|| (catalog.skill_line_name(service.req_skill), service.req_skill_value)),
                req_spells: service.req_spells.iter().copied().filter(|s| *s > 0).collect(),
                learn_spell: described.effects.iter().any(|e| {
                    e.kind == EFFECT_LEARN_SPELL || e.kind == EFFECT_LEARN_PET_SPELL
                }),
                learn_pet_spell: described
                    .effects
                    .iter()
                    .any(|e| e.kind == EFFECT_LEARN_PET_SPELL),
                visible: true,
                req_skill_value: service.req_skill_value,
            }));
        }

        // **The groups first**, because the row order is keyed on a group's
        // *position* — the same ordering dependency the spellbook has, and for
        // the same reason.
        order_groups(&mut groups, kind);
        let position_of = |id: i32| groups.iter().position(|g| g.id == id).unwrap_or(0);
        let mut rows: Vec<Row> = rows
            .into_iter()
            .map(|(id, mut row)| {
                row.group = position_of(id);
                row
            })
            .collect();

        // …then one header per group, appended before the sort exactly as the
        // intake does.
        for position in 0..groups.len() {
            rows.push(Row {
                group: position,
                service: None,
                spell: 0,
                name: String::new(),
                sub_text: String::new(),
                line_name: String::new(),
                icon: None,
                description: String::new(),
                state: None,
                cost: 0,
                point_cost: (0, 0),
                req_level: 0,
                req_skill: None,
                req_spells: Vec::new(),
                learn_spell: false,
                learn_pet_spell: false,
                visible: true,
                req_skill_value: 0,
            });
        }

        let mut board = Board {
            kind,
            groups,
            rows,
            visible: 0,
            types: default_types(kind),
            // **All ones**: every skill line shown, every group expanded
            // (both all ones in the client).
            lines: u32::MAX,
            expanded: u32::MAX,
            selected: std::sync::atomic::AtomicU32::new(0),
        };
        board.rebuild();
        board
    }

    /// How many rows are on screen — `GetNumTrainerServices()`.
    pub fn visible(&self) -> usize {
        self.visible
    }

    /// One row, by the interface's **one-based** index into the visible prefix.
    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index.checked_sub(1)?).filter(|r| r.visible)
    }

    /// The group a row belongs to.
    pub fn group_of_row(&self, index: usize) -> Option<&Group> {
        self.groups.get(self.row(index)?.group)
    }

    /// Whether a row's own group is expanded — `GetTrainerServiceInfo`'s fourth
    /// answer, which is only ever asked about a header.
    pub fn expanded(&self, index: usize) -> bool {
        self.row(index)
            .is_some_and(|row| self.expanded & (1 << row.group.min(31)) != 0)
    }

    /// `GetTrainerSelectionIndex()` — the **one-based** row the selection is
    /// on, or 0 for none. Re-found from the remembered spell id every time, so
    /// a filter change moves it rather than losing it.
    pub fn selection(&self) -> usize {
        let selected = self.selected();
        if selected == 0 {
            return 0;
        }
        self.rows
            .iter()
            .position(|row| row.spell == selected && row.service.is_some())
            .map_or(0, |i| i + 1)
    }

    /// …and the spell id behind it, which is what survives a rebuild.
    pub fn selected(&self) -> u32 {
        self.selected.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `SelectTrainerService(i)` — remembers the row's **spell**, or clears the
    /// selection when the index is past the end.
    ///
    /// **By shared reference**: see the field, where the reason is.
    pub fn select(&self, index: usize) {
        let spell = self.row(index).map_or(0, |row| row.spell);
        self.selected.store(spell, std::sync::atomic::Ordering::Relaxed);
    }

    /// …and re-select a remembered spell id outright, which is what a rebuild
    /// after a race and class arrive does.
    pub fn select_spell(&self, spell: u32) {
        self.selected.store(spell, std::sync::atomic::Ordering::Relaxed);
    }

    /// **A service the character has just bought turns grey in place.**
    ///
    /// `SMSG_TRAINER_BUY_SUCCEEDED` names a spell and nothing else, and **no
    /// packet resends the list** — so the row's state is the client's to amend.
    /// The reference amends it and calls the *filter* rebuild (which
    /// is [`Board::rebuild`]); it does not re-run the intake.
    ///
    /// Doing it by rebuilding the whole board instead is what reset the window
    /// on every purchase: [`Board::build`] starts the type filter at
    /// [`default_types`] and both masks at all-ones, so the dropdown's setting,
    /// every collapsed skill line and every unticked line came back on the
    /// moment you pressed Train. Only the selection was being carried over, and
    /// it was the only one of the four that looked like state.
    ///
    /// The group's per-state counts move with the row, because they are what
    /// [`Board::rebuild`] decides whether a whole group is shown by: a line
    /// whose last unlearned spell you just bought disappears under the default
    /// filter, header and all, and would otherwise linger as an empty heading.
    ///
    /// Answers whether anything changed, so a purchase the board does not know
    /// about raises no `TRAINER_UPDATE`.
    pub fn mark_used(&mut self, spell: u32) -> bool {
        let Some(row) = self
            .rows
            .iter_mut()
            .find(|row| row.spell == spell && row.service.is_some())
        else {
            return false;
        };
        let Some(was) = row.state.filter(|state| *state != State::Used) else {
            return false;
        };
        row.state = Some(State::Used);
        let group = row.group;
        if let Some(counts) = self.groups.get_mut(group).map(|g| &mut g.counts) {
            counts[was.index()] = counts[was.index()].saturating_sub(1);
            counts[State::Used.index()] += 1;
        }
        self.rebuild();
        true
    }

    /// `GetTrainerServiceTypeFilter(word)`.
    pub fn type_filter(&self, word: &str) -> Option<bool> {
        Some(self.types & filter_bit(word)? != 0)
    }

    /// `SetTrainerServiceTypeFilter(word, on [, exclusive])` — the three-way
    /// body: off clears the bit, on with the third argument makes
    /// it the *only* bit, on without it sets the bit.
    pub fn set_type_filter(&mut self, word: &str, on: bool, exclusive: bool) -> bool {
        let Some(bit) = filter_bit(word) else {
            return false;
        };
        self.types = match (on, exclusive) {
            (false, _) => self.types & !bit,
            (true, true) => bit,
            (true, false) => self.types | bit,
        };
        self.rebuild();
        true
    }

    /// …and its `"all"` form, which is the constant 7 rather than a bit.
    pub fn set_all_type_filters(&mut self) {
        self.types = ALL_TYPES;
        self.rebuild();
    }

    /// `ExpandTrainerSkillLine(i)` / `CollapseTrainerSkillLine(i)` — **by row**,
    /// and **0 means all of them** (the client's sign test on `i - 1`).
    ///
    /// Answers whether anything changed, which is what tells a bad index from a
    /// row that was already in that state; the client prints *Bad index* for
    /// the former.
    pub fn set_expanded(&mut self, index: usize, expanded: bool) -> bool {
        let before = self.expanded;
        if index == 0 {
            self.expanded = if expanded { u32::MAX } else { 0 };
        } else {
            // Only a header names a group here — the client refuses a service
            // row outright.
            let Some(group) = self
                .row(index)
                .filter(|row| row.service.is_none())
                .map(|row| row.group)
            else {
                return false;
            };
            let bit = 1u32 << group.min(31);
            self.expanded = match expanded {
                true => self.expanded | bit,
                false => self.expanded & !bit,
            };
        }
        self.rebuild();
        before != self.expanded
    }

    /// `GetTrainerSkillLineFilter(i)` / `SetTrainerSkillLineFilter(i, on)` —
    /// one bit per group, **one-based** as every index from the interface is.
    pub fn line_filter(&self, group: usize) -> Option<bool> {
        let index = group.checked_sub(1)?;
        (index < self.groups.len()).then(|| self.lines & (1 << index.min(31)) != 0)
    }

    pub fn set_line_filter(&mut self, group: usize, on: bool) -> bool {
        let Some(index) = group.checked_sub(1).filter(|i| *i < self.groups.len()) else {
            return false;
        };
        let bit = 1u32 << index.min(31);
        self.lines = match on {
            true => self.lines | bit,
            false => self.lines & !bit,
        };
        self.rebuild();
        true
    }

    /// **Apply the three filters and re-sort** — the client's rebuild, whole.
    ///
    /// Called on every filter change, which is what the client does too: the
    /// three setters each end in this and then a `TRAINER_UPDATE`.
    fn rebuild(&mut self) {
        let shown: Vec<bool> = self
            .groups
            .iter()
            .enumerate()
            .map(|(position, group)| {
                let any = (0..3).any(|state| {
                    self.types & (1 << state) != 0 && group.counts[state] > 0
                });
                any && self.lines & (1 << position.min(31)) != 0
            })
            .collect();
        let collapsed: Vec<bool> = (0..self.groups.len())
            .map(|position| self.expanded & (1 << position.min(31)) == 0)
            .collect();

        for row in &mut self.rows {
            row.visible = match row.state {
                // A service: its own state has to pass, then its group has to
                // be shown and open.
                Some(state) => {
                    self.types & state_bit(state) != 0
                        && shown.get(row.group).copied().unwrap_or(false)
                        && !collapsed.get(row.group).copied().unwrap_or(false)
                }
                // **A header ignores both the type filter and the collapse** —
                // it is what you press to un-collapse.
                None => shown.get(row.group).copied().unwrap_or(false),
            };
        }
        self.visible = self.rows.iter().filter(|row| row.visible).count();
        sort_rows(&mut self.rows);
    }
}

/// **What a teaching spell actually teaches** — the trigger of its first
/// `LEARN_SPELL`/`LEARN_PET_SPELL` effect, or the spell itself when it has
/// none (the client scans all three slots and falls back to the spell).
pub fn taught_spell(info: &SpellInfo) -> u32 {
    info.effects
        .iter()
        .find(|e| e.kind == EFFECT_LEARN_SPELL || e.kind == EFFECT_LEARN_PET_SPELL)
        .map_or(info.id, |e| e.trigger_spell)
}

/// Which group one service goes in, or [`GENERAL`] for one that is dropped.
#[allow(clippy::too_many_arguments)]
fn group_of(kind: Kind, state: State, spell: u32, info: &SpellInfo, catalog: &dyn Catalog) -> i32 {
    match kind {
        // Step against Learn, off the effect list and nothing else.
        Kind::Tradeskill => {
            match info.effects.iter().any(|e| e.kind == EFFECT_SKILL_STEP) {
                true => TRADESKILL_STEP_GROUP,
                false => TRADESKILL_LEARN_GROUP,
            }
        }
        // A talent already taken goes under the one folded header.
        Kind::Talent if state == State::Used => KNOWN_TALENTS_GROUP,
        _ => {
            // **The line of the spell that is taught**, not of the one that
            // teaches it — the teaching spell has no `SkillLineAbility` row of
            // its own, so looking that one up would answer 0 and drop the row.
            // A spell the catalog has no entry for at all keeps its own id,
            // which is what `taught_spell`'s fallback is for.
            let taught = match info.id == 0 {
                true => spell,
                false => taught_spell(info),
            };
            catalog.skill_line(taught) as i32
        }
    }
}

/// A fresh group, named.
fn new_group(id: i32, catalog: &dyn Catalog, points: bool) -> Group {
    let key = match id {
        KNOWN_TALENTS_GROUP => Some(KNOWN_TALENTS_KEY),
        TRADESKILL_STEP_GROUP => Some(STEP_KEY),
        TRADESKILL_LEARN_GROUP => Some(LEARN_KEY),
        _ => None,
    };
    Group {
        id,
        name: match key {
            Some(_) => String::new(),
            None => u32::try_from(id)
                .map(|id| catalog.skill_line_name(id))
                .unwrap_or_default(),
        },
        key,
        counts: [0; 3],
        all_points: points,
    }
}

/// The group comparator — one for everything but a tradeskill trainer, and
/// one for a tradeskill trainer.
fn order_groups(groups: &mut [Group], kind: Kind) {
    if kind == Kind::Tradeskill {
        groups.sort_by_key(|g| g.id);
        return;
    }
    groups.sort_by(|a, b| {
        use std::cmp::Ordering;
        match (a.id == KNOWN_TALENTS_GROUP, b.id == KNOWN_TALENTS_GROUP) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a
                .all_points
                .cmp(&b.all_points)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                // The client stops here and leaves the rest to `qsort`'s own
                // order; the id keeps the build reproducible.
                .then_with(|| a.id.cmp(&b.id))
        }
    });
}

/// The row comparator.
fn sort_rows(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        use std::cmp::Ordering;
        // Hidden rows go to the back, so the visible ones are a prefix.
        b.visible
            .cmp(&a.visible)
            .then_with(|| a.group.cmp(&b.group))
            // A header before its own services.
            .then_with(|| a.service.is_some().cmp(&b.service.is_some()))
            .then_with(|| a.req_level.cmp(&b.req_level))
            .then_with(|| a.req_skill_value.cmp(&b.req_skill_value))
            // **Case-sensitive**, unlike the group comparator — `strncmp`
            // here where the group comparator uses `strnicmp`.
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.sub_text.cmp(&b.sub_text))
            .then_with(|| a.spell.cmp(&b.spell))
            .then(Ordering::Equal)
    });
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::spellbook::SpellEffect;

    /// A stand-in for the two tables: spells by id, skill lines by spell, and a
    /// name per line. Keeps these tests about the *rules* rather than about DBC
    /// parsing, which `crate::tables::skills` and `crate::tables::spellbook` cover on their own.
    #[derive(Default)]
    struct Fake {
        spells: Vec<SpellInfo>,
        /// `(taught spell, skill line)`.
        lines: Vec<(u32, u32)>,
        names: Vec<(u32, &'static str)>,
    }

    impl Catalog for Fake {
        fn spell(&self, id: u32) -> Option<SpellInfo> {
            self.spells.iter().find(|s| s.id == id).cloned()
        }
        fn skill_line(&self, spell: u32) -> u32 {
            self.lines
                .iter()
                .find(|(s, _)| *s == spell)
                .map_or(GENERAL, |(_, line)| *line)
        }
        fn skill_line_name(&self, line: u32) -> String {
            self.names
                .iter()
                .find(|(id, _)| *id == line)
                .map_or_else(String::new, |(_, n)| (*n).to_string())
        }
    }

    /// A teaching spell: named, ranked, and triggering `taught`.
    fn teacher(id: u32, name: &str, rank: &str, taught: u32) -> SpellInfo {
        let mut effects = [SpellEffect::default(); 3];
        effects[0] = SpellEffect {
            kind: EFFECT_LEARN_SPELL,
            trigger_spell: taught,
            ..SpellEffect::default()
        };
        SpellInfo {
            id,
            name: name.to_string(),
            rank: rank.to_string(),
            effects,
            ..SpellInfo::default()
        }
    }

    fn service(spell: u32, state: State, level: u8) -> Service {
        Service {
            spell,
            state,
            cost: 100,
            point_cost: (0, 0),
            req_level: level,
            req_skill: 0,
            req_skill_value: 0,
            req_spells: [0; 3],
        }
    }

    /// A mage trainer: two lines, three spells, one of them already known.
    fn mage() -> (Fake, Vec<Service>) {
        let catalog = Fake {
            spells: vec![
                teacher(10, "Fireball", "Rank 2", 145),
                teacher(11, "Frostbolt", "Rank 1", 116),
                teacher(12, "Fireball", "Rank 3", 3140),
            ],
            // The *taught* spell is what carries the line, never the teacher.
            lines: vec![(145, 8), (3140, 8), (116, 6)],
            names: vec![(8, "Fire"), (6, "Frost")],
        };
        let services = vec![
            service(12, State::Available, 12),
            service(11, State::Used, 4),
            service(10, State::Available, 6),
        ];
        (catalog, services)
    }

    /// **A class trainer has headers, and each one comes before its own
    /// spells.** The groups are alphabetical by skill-line name — Fire before
    /// Frost — and the spells inside one are by required level.
    #[test]
    fn a_class_trainer_is_headers_then_spells_by_level() {
        let (catalog, services) = mage();
        let mut board = Board::build(Kind::Class, &services, &catalog);
        // Everything on, so the already-known row is in the list too.
        board.set_all_type_filters();
        let shape: Vec<(String, String)> = (1..=board.visible())
            .map(|i| {
                let row = board.row(i).expect("a visible row");
                match row.service {
                    None => (
                        "header".to_string(),
                        board.group_of_row(i).expect("a group").name.clone(),
                    ),
                    Some(_) => (
                        row.state.expect("a state").word().to_string(),
                        format!("{} {}", row.name, row.sub_text),
                    ),
                }
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                ("header".to_string(), "Fire".to_string()),
                ("available".to_string(), "Fireball Rank 2".to_string()),
                ("available".to_string(), "Fireball Rank 3".to_string()),
                ("header".to_string(), "Frost".to_string()),
                ("used".to_string(), "Frostbolt Rank 1".to_string()),
            ]
        );
    }

    /// **The window opens with `used` filtered out** (the filter starts at 3) —
    /// and a group whose every row that leaves is empty loses its header too,
    /// because a group is only shown when some state the filter allows has a
    /// row in it.
    #[test]
    fn the_default_filter_hides_what_is_known_and_the_header_with_it() {
        let (catalog, services) = mage();
        let board = Board::build(Kind::Class, &services, &catalog);
        assert_eq!(default_types(Kind::Class), 3);
        assert_eq!(board.visible(), 3, "one header and its two spells");
        assert!(
            (1..=board.visible()).all(|i| board.row(i).expect("visible").state != Some(State::Used)),
            "nothing already known"
        );
        assert_eq!(
            board.group_of_row(1).expect("a group").name,
            "Fire",
            "Frost had only the known row, so its header is gone too"
        );
    }

    /// **A header ignores the collapse; its services do not.** Collapsing a
    /// line leaves the header on screen — which is what you press to open it
    /// again — and takes every row under it off.
    #[test]
    fn collapsing_a_line_keeps_its_header_and_drops_its_rows() {
        let (catalog, services) = mage();
        let mut board = Board::build(Kind::Class, &services, &catalog);
        assert_eq!(board.visible(), 3);
        assert!(board.set_expanded(1, false), "row 1 is the Fire header");
        assert_eq!(board.visible(), 1);
        assert!(board.row(1).expect("visible").service.is_none());
        assert!(!board.expanded(1));
        // …and 0 is every line at once, which is the Collapse All button.
        assert!(board.set_expanded(0, true));
        assert_eq!(board.visible(), 3);
        assert!(board.expanded(1));
        // A *service* row names no line, so expanding by one is refused.
        assert!(!board.set_expanded(2, false));
    }

    /// **The selection is a spell id, not a row.** Filtering moves every index
    /// and the selected service stays selected, which is what
    /// `ClassTrainerFrame_OnEvent` relies on when `TRAINER_UPDATE` arrives.
    #[test]
    fn the_selection_survives_a_filter_change() {
        let (catalog, services) = mage();
        let mut board = Board::build(Kind::Class, &services, &catalog);
        board.select(3);
        let chosen = board.row(3).expect("visible").spell;
        assert_eq!(board.selection(), 3);
        board.set_all_type_filters();
        assert_eq!(
            board.row(board.selection()).expect("visible").spell,
            chosen,
            "two more rows in the list and the same service still selected"
        );
        // **A selection can end up off screen**, and the index says so by
        // running past the visible count — which is exactly what
        // `ClassTrainerFrame_OnEvent` tests for before it resets the scroll.
        assert!(board.set_expanded(1, false), "collapse the selected row's line");
        assert!(board.selection() > board.visible());
        // …and an index past the end clears it.
        board.select(99);
        assert_eq!(board.selection(), 0);
    }

    /// **A selection is visible to the very next read**, through a shared
    /// reference — which is not a detail of the storage but the behaviour
    /// `ClassTrainerSkillButton_OnClick` is written against: it selects and then
    /// calls `ClassTrainerFrame_Update`, which asks `GetTrainerSelectionIndex()`
    /// to place the highlight bar, in the same click.
    ///
    /// Recorded and applied a system later — which is what every other write in
    /// this window is — the bar lands on the row selected *before* the press
    /// while the detail pane below fills from the row that was pressed. Two rows
    /// lit for one click.
    #[test]
    fn a_selection_answers_before_the_click_that_made_it_returns() {
        let (catalog, services) = mage();
        let board = Board::build(Kind::Class, &services, &catalog);
        // Through `&Board`, exactly as the Lua read does.
        let shared: &Board = &board;
        assert_eq!(shared.selection(), 0);
        shared.select(2);
        assert_eq!(shared.selection(), 2);
        shared.select(3);
        assert_eq!(shared.selection(), 3);
        // …and the spell id behind it is what a rebuild carries over.
        assert_eq!(shared.selected(), shared.row(3).expect("visible").spell);
    }


    /// **Buying a spell greys one row and touches nothing else** — which is the
    /// whole of why it is done in place rather than by laying the board out
    /// again.
    ///
    /// `SMSG_TRAINER_BUY_SUCCEEDED` names a spell and no packet resends the
    /// list, so the state is the client's to amend; the reference amends it and
    /// calls the filter rebuild and never re-runs the intake. A
    /// rebuild from scratch put the type filter back to [`default_types`] and
    /// both masks back to all-ones, so the dropdown's setting and every folded
    /// skill line came back the moment you pressed Train.
    #[test]
    fn buying_a_spell_greys_its_row_and_keeps_the_filters() {
        let (catalog, services) = mage();
        let mut board = Board::build(Kind::Class, &services, &catalog);
        // A window the player has set up: everything shown, the Fire line
        // folded, and a service selected.
        board.set_all_type_filters();
        let all = board.visible();
        assert!(board.set_expanded(1, false), "row 1 is the Fire header");
        let folded = board.visible();
        assert!(folded < all);
        let bought = board
            .rows
            .iter()
            .find(|row| row.state == Some(State::Available))
            .expect("something to buy")
            .spell;
        board.select_spell(bought);

        assert!(board.mark_used(bought));
        assert_eq!(board.type_filter("used"), Some(true), "the dropdown held");
        assert_eq!(board.type_filter("available"), Some(true));
        assert!(!board.expanded(1), "…and so did the folded line");
        assert_eq!(board.visible(), folded, "the same rows are on screen");
        assert_eq!(board.selected(), bought, "and the same service is selected");
        assert_eq!(
            board.rows.iter().find(|row| row.spell == bought).expect("the row").state,
            Some(State::Used),
        );
        // …and the group's counts moved with it, which is what decides whether
        // a whole line is shown under a filter that hides what is known.
        board.set_type_filter("used", false, true);
        assert!(
            (1..=board.visible()).all(|i| board.row(i).expect("visible").state != Some(State::Used)),
            "the bought row is hidden by the filter that hides known spells"
        );
        // Buying it twice, or buying something this board never had, is nothing
        // — so neither raises a TRAINER_UPDATE.
        assert!(!board.mark_used(bought));
        assert!(!board.mark_used(999_999));
    }

    /// A service whose taught spell is on no line at all is **dropped**, not
    /// filed under a General header — and with nothing left there is no window.
    #[test]
    fn a_service_with_no_skill_line_is_dropped() {
        let catalog = Fake {
            spells: vec![teacher(10, "Sharpen Blade", "", 999)],
            lines: Vec::new(),
            names: Vec::new(),
        };
        let board = Board::build(Kind::Class, &[service(10, State::Available, 1)], &catalog);
        assert_eq!(board.visible(), 0);
        assert!(board.groups.is_empty());
    }

    /// The three filter words, the three-way setter, and a word the client
    /// would refuse.
    #[test]
    fn the_type_filter_is_three_words_and_an_exclusive_form() {
        let (catalog, services) = mage();
        let mut board = Board::build(Kind::Class, &services, &catalog);
        assert_eq!(board.type_filter("available"), Some(true));
        assert_eq!(board.type_filter("used"), Some(false));
        assert_eq!(board.type_filter("nonsense"), None);
        assert!(!board.set_type_filter("nonsense", true, false));
        // Exclusive: this one and no other.
        assert!(board.set_type_filter("used", true, true));
        assert_eq!(board.type_filter("available"), Some(false));
        assert_eq!(board.visible(), 2, "the Frost header and its known row");
        assert!(board.set_type_filter("used", false, false));
        assert_eq!(board.visible(), 0, "nothing passes any filter");
    }

    /// A tradeskill trainer groups by *what the row does* rather than by skill
    /// line, and its two headers are `GlobalStrings` keys rather than names.
    #[test]
    fn a_tradeskill_trainer_groups_into_step_and_learn() {
        let mut step = teacher(20, "Journeyman Blacksmith", "", 0);
        step.effects[0] = SpellEffect {
            kind: EFFECT_SKILL_STEP,
            ..SpellEffect::default()
        };
        let catalog = Fake {
            spells: vec![step, teacher(21, "Copper Chain Belt", "", 2661)],
            lines: vec![(2661, 164)],
            names: vec![(164, "Blacksmithing")],
        };
        let board = Board::build(
            Kind::Tradeskill,
            &[
                service(21, State::Available, 1),
                service(20, State::Available, 10),
            ],
            &catalog,
        );
        let titles: Vec<String> = board.groups.iter().map(|g| g.title(|_| None)).collect();
        assert_eq!(
            titles,
            vec![
                "TRADESKILL_SERVICE_STEP".to_string(),
                "TRADESKILL_SERVICE_LEARN".to_string()
            ],
            "step before learn, and each a key rather than a name"
        );
        // …and each learn row carries the TAUGHT line's own name — what the
        // profession-learn popup prints, and not the Step/Learn
        // group header, which is what it printed for one live session.
        let learn = board
            .rows
            .iter()
            .find(|row| row.spell == 21)
            .expect("the learn row");
        assert_eq!(learn.line_name, "Blacksmithing");
        // …and the key resolves through the caller's own string table.
        assert_eq!(
            board.groups[0]
                .title(|key| (key == "TRADESKILL_SERVICE_STEP").then(|| "Skills".to_string())),
            "Skills"
        );
    }

    /// A talent trainer folds everything already taken under one header, and
    /// **that header sorts first** — the only group the comparator names.
    #[test]
    fn a_talent_trainer_folds_what_is_known_and_puts_it_first() {
        let catalog = Fake {
            spells: vec![
                teacher(30, "Improved Fireball", "Rank 1", 11069),
                teacher(31, "Arcane Focus", "Rank 1", 11222),
            ],
            lines: vec![(11069, 8), (11222, 6)],
            names: vec![(8, "Fire"), (6, "Arcane")],
        };
        let mut board = Board::build(
            Kind::Talent,
            &[
                service(31, State::Available, 10),
                service(30, State::Used, 10),
            ],
            &catalog,
        );
        board.set_all_type_filters();
        assert_eq!(board.groups[0].id, KNOWN_TALENTS_GROUP);
        assert_eq!(board.groups[0].title(|_| None), "KNOWN_TALENTS_HEADER");
        // …and its default filter is available + used, not available +
        // unavailable.
        assert_eq!(default_types(Kind::Talent), 5);
    }

    /// **The taught spell decides the group; the teaching spell decides the
    /// words.** Getting these the wrong way round is the failure that renders
    /// as a plausible window: every row would fall to line 0 and vanish.
    #[test]
    fn the_group_follows_the_trigger_and_the_name_does_not() {
        let catalog = Fake {
            spells: vec![teacher(10, "Fireball", "Rank 2", 145)],
            // Only the *taught* spell has a line; the teacher has none.
            lines: vec![(145, 8)],
            names: vec![(8, "Fire")],
        };
        let board = Board::build(Kind::Class, &[service(10, State::Available, 6)], &catalog);
        assert_eq!(board.visible(), 2, "a header and its row");
        let row = board.row(2).expect("the service row");
        assert_eq!(row.spell, 10, "the id that goes back on the wire");
        assert_eq!(row.name, "Fireball");
        assert_eq!(row.sub_text, "Rank 2");
        assert!(row.learn_spell);
        assert!(!row.learn_pet_spell);
        assert_eq!(board.groups[0].name, "Fire");
    }
}
