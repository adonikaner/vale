//! What a *column* of one of the server's tables is: how it is read, what it
//! means, and which part of a form it is drawn in.
//!
//! ## Why the column vocabulary is shared between tables
//!
//! These types were first written in [`crate::creature`], for
//! `creature_template` and `creature`. [`crate::item`] describes a subject of
//! the same shape — a keyed row of typed columns with no client file behind it
//! — and uses every one of these types unchanged. Two copies of [`Kind`] would
//! mean a new widget has to be added in two places, and one of them could be
//! missed.
//!
//! The vocabulary is in this module and each table's column list is in the
//! module about that table. `creature` re-exports these names, so a caller
//! that says `creature::Kind::Text` still compiles unchanged.
//!
//! This is not the `Table`-with-two-backends refactor. That one is about making
//! a DBC record and a server row answer one trait; this module is the smaller
//! part of it that two subjects already needed, and it moves no decision about
//! what a row means out of the module that owns the table.
//!
//! ## A `Kind` describes how a form draws a column, not what the server stores
//!
//! The server does not have kinds: every column of `item_template` is an
//! integer, a float or a string to MySQL, and `quality` holding 7 is a row it
//! loads and the client draws in no colour at all. What [`Kind`] carries is the
//! meaning — this number is one of seven named qualities, that one is a row of
//! `Spell.dbc`, this one is copper — so that a form can offer the meaning
//! instead of the number. A wrong `Kind` produces a wrong widget and never a
//! wrong statement: the value written is still the number.

/// How a column is read, written and drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// Part of the row's key. Never written.
    Key,
    Unsigned,
    /// A resistance, a stat value, a spell charge count — anything vmangos
    /// stores signed.
    Signed,
    Float,
    Text,
    /// A bit mask, with the names of the bits that have them.
    Flags(&'static [Bit]),
    /// A small enumeration, with the names of its values.
    Choice(&'static [Value]),
    /// An id in another table, named so a form can offer to follow it. The
    /// string is the table's own name — a DBC's bare name (`CreatureDisplayInfo`)
    /// or a server table's (`creature_template`).
    Ref(&'static str),
    /// An enumeration whose options depend on another column of the same row.
    ///
    /// `item_template.subclass` is the only one: 0 is Axe on a weapon, Cloth
    /// on a piece of armour and Bandage on a consumable, and which list
    /// applies is whatever `class` holds. A form draws it by asking
    /// [`crate::item::subclasses`] for the row's class; anything that cannot
    /// (a dump, a check with no row in hand) draws the number, which is what
    /// the column is.
    Subclass,
    /// An amount of copper, drawn as gold, silver and copper.
    ///
    /// `buy_price` on a Deadmines drop is 1,478,900, which is 147g 89s and is
    /// hard to read as a bare number. The stored value is the copper either way.
    Money,
    /// Milliseconds — a weapon's swing, an item spell's cooldown.
    Millis,
    /// Seconds — how long an item lasts once it is created.
    Seconds,
    /// Text of several lines: a quest's description, its objectives, what
    /// the giver says when it is handed in.
    ///
    /// The same column type as [`Kind::Text`] and the same literal; what differs
    /// is the box a form draws, and that `$B` in it is a line break the client
    /// makes — see [`crate::quest`].
    Paragraph,
    /// A signed id whose sign picks the table.
    ///
    /// `quest_template.ZoneOrSort` is an `AreaTable.dbc` row when it is positive
    /// and a `QuestSort.dbc` row, negated, when it is not; `ReqCreatureOrGOId`
    /// is a `creature_template` entry or a negated `gameobject_template` one.
    /// The first name is the positive table and the second the negative.
    Either(&'static str, &'static str),
    /// Copper, signed. `quest_template.RewOrReqMoney` is what the quest pays
    /// when it is positive and what it costs to hand in when it is negative.
    SignedMoney,
}

impl Kind {
    /// Whether the column holds a string, and so is written as a quoted literal.
    pub fn is_text(self) -> bool {
        matches!(self, Kind::Text | Kind::Paragraph)
    }

    /// What the kind is called in a census of a table's columns.
    pub fn describe(self) -> String {
        match self {
            Kind::Key => "key".to_string(),
            Kind::Unsigned => "number".to_string(),
            Kind::Signed => "number (signed)".to_string(),
            Kind::Float => "float".to_string(),
            Kind::Text => "text".to_string(),
            Kind::Paragraph => "text, several lines".to_string(),
            Kind::Flags(bits) => format!("mask of {} named bit(s)", bits.len()),
            Kind::Choice(values) => format!("one of {} value(s)", values.len()),
            Kind::Ref(table) => format!("a row of {table}"),
            Kind::Either(positive, negative) => {
                format!("a row of {positive}, or of {negative} negated")
            }
            Kind::Subclass => "one of the class's subclasses".to_string(),
            Kind::Money => "copper".to_string(),
            Kind::SignedMoney => "copper, signed".to_string(),
            Kind::Millis => "milliseconds".to_string(),
            Kind::Seconds => "seconds".to_string(),
        }
    }
}

/// One named bit of a mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bit {
    pub bit: u32,
    pub name: &'static str,
    /// vmangos' own trailing comment, where it has one.
    pub about: &'static str,
}

/// One named value of an enumeration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Value {
    pub value: u32,
    pub name: &'static str,
}

/// What a value of an enumeration is called, or the number itself.
///
/// A value the list does not name is a value somebody typed or a value this
/// list has not learnt, and both are worth seeing as themselves rather than as
/// the first entry of the list.
pub fn value_word(values: &[Value], value: u32) -> String {
    values
        .iter()
        .find(|named| named.value == value)
        .map(|named| named.name.to_string())
        .unwrap_or_else(|| value.to_string())
}

/// What a mask's set bits are called, for a field that has to say in words
/// what it holds.
///
/// A bit the list does not name is reported as its own hex value rather than
/// left out, because it is either a bit this list has not learnt or a value
/// typed by hand and both are worth seeing.
pub fn mask_words(bits: &[Bit], value: u32) -> String {
    let mut out: Vec<String> = bits
        .iter()
        .filter(|bit| value & bit.bit != 0)
        .map(|bit| bit.name.to_string())
        .collect();
    let named: u32 = bits.iter().map(|bit| bit.bit).fold(0, |all, bit| all | bit);
    let rest = value & !named;
    if rest != 0 {
        out.push(format!("+{rest:#010x} unnamed"));
    }
    match out.is_empty() {
        true => "none".to_string(),
        false => out.join(", "),
    }
}

/// A copper amount in the game's own three coins.
///
/// The client's own arrangement: gold, then silver, then copper, and a coin
/// that is zero is left out unless it is the only one. `0` is "nothing", which
/// is what a vendor that does not buy an item holds.
pub fn money_words(copper: u64) -> String {
    if copper == 0 {
        return "nothing".to_string();
    }
    let (gold, silver, copper) = (copper / 10_000, (copper / 100) % 100, copper % 100);
    let mut out: Vec<String> = Vec::new();
    if gold > 0 {
        out.push(format!("{gold}g"));
    }
    if silver > 0 {
        out.push(format!("{silver}s"));
    }
    if copper > 0 {
        out.push(format!("{copper}c"));
    }
    out.join(" ")
}

/// A duration in milliseconds, in words. `2600` is a swing timer and
/// `1800000` is half an hour, and the same column holds both.
pub fn millis_words(millis: i64) -> String {
    match millis {
        0 => "none".to_string(),
        millis => seconds_words(millis / 1000, millis % 1000),
    }
}

/// A duration in seconds plus milliseconds, in words.
pub fn seconds_words(seconds: i64, millis: i64) -> String {
    if seconds == 0 && millis == 0 {
        return "none".to_string();
    }
    if seconds == 0 {
        return format!("{millis} ms");
    }
    let (hours, minutes, rest) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    let mut out: Vec<String> = Vec::new();
    if hours > 0 {
        out.push(format!("{hours}h"));
    }
    if minutes > 0 {
        out.push(format!("{minutes}m"));
    }
    if rest > 0 || out.is_empty() {
        match millis {
            0 => out.push(format!("{rest}s")),
            millis => out.push(format!("{rest}.{:03}s", millis.abs())),
        }
    }
    out.join(" ")
}

/// A lookup from a reference to a name: the table a `Kind::Ref` column names
/// and the id it holds, answered with the row's name when the caller has one.
/// The sentences in `eventai`, `scripts` and `creaturespells` take one, so a
/// sentence reads `Cast Shoot` where the caller can name spell 6660 and
/// `Cast Spell 6660` where it cannot.
pub type Names<'a> = &'a dyn Fn(&str, u32) -> Option<String>;

/// What a sentence calls a row of `table` when it has no name for it:
/// `spell 999`, `creature 68`. A table with no word of its own is its name.
pub fn ref_word(table: &str) -> &str {
    match table {
        "Spell" => "spell",
        "Emotes" => "emote",
        "EmotesText" => "text emote",
        "FactionTemplate" => "faction template",
        "Languages" => "language",
        "SoundEntries" => "sound",
        "AreaTable" => "area",
        "Map" => "map",
        "creature_template" => "creature",
        "gameobject_template" => "game object",
        "item_template" => "item",
        "quest_template" => "quest",
        "broadcast_text" => "text",
        other => other,
    }
}

/// A lookup that knows no names, for a caller with no tables open.
pub fn no_names(_: &str, _: u32) -> Option<String> {
    None
}

/// A duration in milliseconds, short enough for a sentence: `250 ms`, `5 s`,
/// `1.5 s`, `2 min`, `1 min 30 s`. `0` is `0 s`.
pub fn span_words(millis: u64) -> String {
    if millis == 0 {
        return "0 s".to_string();
    }
    if millis < 1000 {
        return format!("{millis} ms");
    }
    if millis < 60_000 {
        let whole = millis / 1000;
        let rest = millis % 1000;
        if rest == 0 {
            return format!("{whole} s");
        }
        let fraction = format!("{rest:03}");
        return format!("{whole}.{} s", fraction.trim_end_matches('0'));
    }
    let seconds = millis / 1000;
    let (hours, minutes, rest) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
    let mut out: Vec<String> = Vec::new();
    if hours > 0 {
        out.push(format!("{hours} h"));
    }
    if minutes > 0 {
        out.push(format!("{minutes} min"));
    }
    if rest > 0 {
        out.push(format!("{rest} s"));
    }
    out.join(" ")
}

/// A range of two durations in milliseconds: one duration when the ends are
/// equal or the second is 0, `5 s to 10 s` otherwise.
pub fn span_range(least: u64, most: u64) -> String {
    match least == most || most == 0 {
        true => span_words(least),
        false => format!("{} to {}", span_words(least), span_words(most)),
    }
}

/// Which part of the form a column is drawn in.
///
/// A creature template is 78 columns and an item template is 129, which is more
/// than a person reads at once. The groups are what the first section shows and
/// what the rest are folded under; they are this crate's own arrangement and
/// carry no meaning on the server.
///
/// One enumeration serves every table, so that a form drawing a group has one
/// list to walk. A group no table of a subject uses simply never appears — the
/// form collects the groups it finds in the columns it was given, in the order
/// the table declares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// The key, the name and the level band: what identifies the thing.
    Identity,
    /// What it looks like: the models, the icon, how big it is drawn.
    Appearance,
    /// What it is made of: type, rank, class, the multipliers, the stats it
    /// carries.
    Stats,
    /// What it does in a fight.
    Combat,
    /// Who may have it, which is an item's own: level, class, race, skill,
    /// reputation, the quest it comes from.
    Requirements,
    /// What it is worth: what a vendor pays, what it costs, how it stacks.
    Economy,
    /// The spells it carries, and how each is set off.
    Effects,
    /// What it holds, for a bag or a quiver.
    Container,
    /// What it drops, or what is got from destroying it.
    Loot,
    /// What it offers a player: vendor, trainer, quests, gossip.
    Services,
    /// How it moves and what it is immune to.
    Behaviour,
    /// The words on it: a description, a page of text, the language it is
    /// written in.
    Text,
    /// Flags and script hooks.
    Advanced,
    /// What a quest asks for: the items, the kills, the spells cast.
    Objectives,
    /// What a quest gives: the items, the money, the reputation.
    Rewards,
    /// Where a quest sits among the others: what comes before it, what
    /// follows, and which quests exclude it.
    Chain,
    /// The emotes a quest giver plays while its text is on screen.
    Emotes,
    /// A spawn's place in the world.
    Place,
    /// How a spawn comes back.
    Respawn,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Identity => "Identity",
            Group::Appearance => "Appearance",
            Group::Stats => "Stats",
            Group::Combat => "Combat",
            Group::Requirements => "Requirements",
            Group::Economy => "Economy",
            Group::Effects => "Effects",
            Group::Container => "Container",
            Group::Loot => "Loot",
            Group::Services => "Services",
            Group::Behaviour => "Behaviour",
            Group::Text => "Text",
            Group::Advanced => "Advanced",
            Group::Objectives => "Objectives",
            Group::Rewards => "Rewards",
            Group::Chain => "Chain",
            Group::Emotes => "Emotes",
            Group::Place => "Place",
            Group::Respawn => "Respawn",
        }
    }
}

/// One column of one of the server's tables.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// Its name, as vmangos spells it.
    pub name: &'static str,
    pub kind: Kind,
    pub group: Group,
    /// What it means, for a form that has room for one line.
    pub about: &'static str,
}

impl Column {
    /// Whether a save may write this column.
    pub fn editable(&self) -> bool {
        self.kind != Kind::Key
    }

    /// The value in `row`, as a SQL literal to write back, or `None` when the
    /// row does not carry the column.
    pub fn literal(&self, row: &Row) -> Option<String> {
        let value = RowValue::text(row, self.name)?;
        Some(match self.kind.is_text() {
            true => crate::sql::text(value),
            false => value.to_string(),
        })
    }
}

/// A row as it came back from the database: column name to value, with `None`
/// for a SQL `NULL`.
pub type Row = std::collections::HashMap<String, Option<String>>;

/// No and yes, for a column that is a switch stored as 0 or 1.
pub const NO_YES: [Value; 2] = [Value { value: 0, name: "No" }, Value { value: 1, name: "Yes" }];

/// One column of a row, or `None` for a column it does not carry and for `NULL`.
///
/// `text` rather than `get`, because [`Row`] is a `HashMap` and its own `get`
/// answers `Option<&Option<String>>` — two names for two different readings of
/// the same column is how a caller ends up with `Some(None)` where it meant
/// `None`.
pub trait RowValue {
    fn text(&self, column: &str) -> Option<&str>;
    fn number(&self, column: &str) -> Option<f64> {
        self.text(column)?.parse().ok()
    }
    fn integer(&self, column: &str) -> Option<i64> {
        // A float column read as an integer goes through the float, because
        // MySQL prints `100` for a float column holding 100 and `41.5` for one
        // holding 41.5, and a caller asking for `map` should not have to know
        // which the column is.
        match self.text(column)?.parse::<i64>() {
            Ok(value) => Some(value),
            Err(_) => Some(self.number(column)? as i64),
        }
    }
}

impl RowValue for Row {
    fn text(&self, column: &str) -> Option<&str> {
        self.get(column)?.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three coins, and zero, which reads as "nothing".
    #[test]
    fn money_reads_as_the_game_writes_it() {
        assert_eq!(money_words(0), "nothing");
        assert_eq!(money_words(1), "1c");
        assert_eq!(money_words(100), "1s");
        assert_eq!(money_words(10_000), "1g");
        assert_eq!(money_words(1_478_900), "147g 89s");
    }

    /// A swing timer, a cooldown and half an hour, out of one column.
    #[test]
    fn a_duration_says_what_it_is() {
        assert_eq!(millis_words(0), "none");
        assert_eq!(millis_words(2600), "2.600s");
        assert_eq!(millis_words(1_800_000), "30m");
        assert_eq!(seconds_words(3661, 0), "1h 1m 1s");
        assert_eq!(millis_words(500), "500 ms");
    }

    /// The short form a sentence uses: whole seconds, a trimmed fraction,
    /// minutes past a minute, and a range that collapses when its ends agree.
    #[test]
    fn a_span_is_short() {
        assert_eq!(span_words(0), "0 s");
        assert_eq!(span_words(250), "250 ms");
        assert_eq!(span_words(5000), "5 s");
        assert_eq!(span_words(1500), "1.5 s");
        assert_eq!(span_words(1250), "1.25 s");
        assert_eq!(span_words(120_000), "2 min");
        assert_eq!(span_words(90_000), "1 min 30 s");
        assert_eq!(span_range(5000, 10_000), "5 s to 10 s");
        assert_eq!(span_range(5000, 5000), "5 s");
        assert_eq!(span_range(5000, 0), "5 s");
    }

    /// A value the list does not name is itself, never the first entry.
    #[test]
    fn an_unnamed_value_is_its_own_number() {
        const SOME: [Value; 2] = [
            Value { value: 0, name: "None" },
            Value { value: 1, name: "One" },
        ];
        assert_eq!(value_word(&SOME, 1), "One");
        assert_eq!(value_word(&SOME, 9), "9");
    }
}
