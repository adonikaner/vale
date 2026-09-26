//! The display id picker's state: which column it writes, which table it
//! chooses from, the search, the page and the pictures.
//!
//! `creature_template.display_id1` to `display_id4` and `mount_display_id`
//! name rows of `CreatureDisplayInfo.dbc`, and `gameobject_template.displayId`
//! names a row of `GameObjectDisplayInfo.dbc`. Neither table carries a name.
//! A row has a model path, and a creature's row has skin names beside it, so
//! the search is over those and the picture is the row rendered through
//! [`crate::portraits`]. [`crate::ui::displays`] draws the dialog.
//!
//! The item workspace keeps its own picker over `ItemDisplayInfo`
//! ([`crate::tools::items`]), whose rows are a different join: an icon, and
//! either a model or a set of body textures.

use std::collections::HashMap;

use vale_client::assets::GameAssets;

use super::quests::ColumnTarget;
use crate::portraits::Worn;
use crate::session::EditSession;

/// Which of the two display tables a column names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Creature,
    Object,
}

impl Table {
    /// The DBC's bare name, as a column's `Kind::Ref` spells it.
    pub fn name(self) -> &'static str {
        match self {
            Table::Creature => "CreatureDisplayInfo",
            Table::Object => "GameObjectDisplayInfo",
        }
    }

    /// The table a reference names, or `None` for a reference to anything
    /// else.
    pub fn of(dbc: &str) -> Option<Table> {
        match dbc {
            "CreatureDisplayInfo" => Some(Table::Creature),
            "GameObjectDisplayInfo" => Some(Table::Object),
            _ => None,
        }
    }
}

/// One row of a display table, as the picker searches and draws it.
#[derive(Debug, Clone)]
pub struct Fact {
    pub id: u32,
    /// The model path the row resolves to.
    pub path: String,
    /// What a query is matched against: the path and, for a creature, its
    /// skins, in lower case.
    text: String,
}

/// The picker while it is open.
#[derive(Debug)]
pub struct DisplayPick {
    pub table: Table,
    /// The column a choice is written to.
    pub target: ColumnTarget,
    pub query: String,
    pub page: usize,
    /// The id under the pointer, drawn large under the grid.
    pub preview: Option<u32>,
    /// Whether the search box takes focus on the next frame.
    pub focus: bool,
    /// Every row of the table that resolves to a model, built on the first
    /// frame the dialog is drawn.
    facts: Option<Vec<Fact>>,
    hits: Vec<u32>,
    /// The query [`Self::hits`] was built for.
    built: Option<String>,
    /// What a creature display id resolves to, memoised: the grid asks for a
    /// row's picture on every frame it is drawn.
    worn: HashMap<u32, Option<Worn>>,
}

/// How many cells a page holds.
pub const PAGE: usize = 40;

impl DisplayPick {
    /// Open the picker on a column, previewing the id the column holds.
    pub fn open(table: Table, target: ColumnTarget, showing: u32) -> DisplayPick {
        DisplayPick {
            table,
            target,
            query: String::new(),
            page: 0,
            preview: Some(showing),
            focus: true,
            facts: None,
            hits: Vec::new(),
            built: None,
            worn: HashMap::new(),
        }
    }

    /// Every row the table resolves to a model.
    pub fn facts(&mut self, session: &mut EditSession, assets: &GameAssets) -> &[Fact] {
        if self.facts.is_none() {
            self.facts = Some(build_facts(session, assets, self.table));
        }
        self.facts.as_deref().unwrap_or(&[])
    }

    /// The ids the query matches, cached until the query changes.
    ///
    /// A number matches the row with that id; text matches the model path
    /// and, for a creature, a skin name.
    pub fn matches(&mut self, session: &mut EditSession, assets: &GameAssets) -> &[u32] {
        let query = self.query.trim().to_ascii_lowercase();
        if self.built.as_ref() == Some(&query) {
            return &self.hits;
        }
        self.facts(session, assets);
        self.built = Some(query.clone());
        self.page = 0;
        self.hits = filter(self.facts.as_deref().unwrap_or(&[]), &query);
        &self.hits
    }

    /// The model path a display id resolves to, from the facts.
    pub fn path_of(&self, id: u32) -> Option<&str> {
        self.facts
            .as_deref()?
            .iter()
            .find(|fact| fact.id == id)
            .map(|fact| fact.path.as_str())
    }

    /// What a creature display id is drawn as, memoised. See [`resolve_worn`].
    pub fn worn(&mut self, assets: &GameAssets, id: u32) -> Option<Worn> {
        if let Some(had) = self.worn.get(&id) {
            return had.clone();
        }
        let found = resolve_worn(assets, id);
        self.worn.insert(id, found.clone());
        found
    }
}

/// The ids whose facts match a lower-case query: every id for an empty
/// query, the one id a number names, and every row whose text contains the
/// words.
fn filter(facts: &[Fact], query: &str) -> Vec<u32> {
    let by_id: Option<u32> = query.parse().ok();
    facts
        .iter()
        .filter(|fact| query.is_empty() || by_id == Some(fact.id) || fact.text.contains(query))
        .map(|fact| fact.id)
        .collect()
}

/// Read the table once and resolve every row to a model. Rows that resolve
/// to no model are left out: there is nothing to draw or to search.
fn build_facts(session: &mut EditSession, assets: &GameAssets, table: Table) -> Vec<Fact> {
    if !session.open_table(assets, table.name()) {
        return Vec::new();
    }
    let Some(open) = session.table(table.name()) else {
        return Vec::new();
    };
    let Ok(tables) = assets.display_tables() else {
        return Vec::new();
    };
    let mut facts = Vec::with_capacity(open.record_count());
    for record in 0..open.record_count() {
        let Some(id) = open.u32_at(record, 0) else {
            continue;
        };
        let (path, skins) = match table {
            Table::Creature => match tables.creature(id) {
                Some(display) => (display.path, display.skins),
                None => continue,
            },
            Table::Object => match tables.game_object(id) {
                Some(display) => (display.path, Vec::new()),
                None => continue,
            },
        };
        let mut text = path.to_ascii_lowercase();
        for skin in &skins {
            if !skin.is_empty() {
                text.push(' ');
                text.push_str(&skin.to_ascii_lowercase());
            }
        }
        facts.push(Fact { id, path, text });
    }
    facts
}

/// What a creature display id resolves to: the model, its skins, and what
/// `vale_assets::look::dress::dress` says a creature of that display
/// wears.
///
/// `None` for id 0 and for an id the tables do not resolve.
pub fn resolve_worn(assets: &GameAssets, display_id: u32) -> Option<Worn> {
    if display_id == 0 {
        return None;
    }
    let tables = assets.display_tables().ok()?;
    let display = tables.creature(display_id)?;
    let dressed = vale_assets::look::dress::dress(
        &tables,
        &display,
        // Nothing worn: a `creature` row carries no equipment, and what an NPC
        // has on is baked into the skin the display row names.
        &vale_assets::look::dress::Wearer::default(),
    );
    Some(Worn {
        path: display.path.clone(),
        skins: display.skins.clone(),
        hair: dressed.hair,
        dress: dressed.dress,
        // A creature's body texture is a file, baked or varied, and the skins
        // above name it. Only a player's is composed. See `Worn::look`.
        look: None,
        cloak: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reference to either display table opens the picker; a reference to
    /// any other table does not.
    #[test]
    fn only_the_two_display_tables_are_picked_from() {
        assert_eq!(Table::of("CreatureDisplayInfo"), Some(Table::Creature));
        assert_eq!(Table::of("GameObjectDisplayInfo"), Some(Table::Object));
        assert_eq!(Table::of("ItemDisplayInfo"), None);
        assert_eq!(Table::of("Spell"), None);
        assert_eq!(Table::Creature.name(), "CreatureDisplayInfo");
    }

    /// A query is matched against the path, the skins and the id.
    #[test]
    fn a_query_matches_the_path_the_skins_or_the_id() {
        let mut pick = DisplayPick::open(
            Table::Creature,
            ColumnTarget {
                table: "creature_template",
                key: vale_mangos::creature::template_key(1, 10),
                column: "display_id1",
                in_database: None,
                label: "Edit creature",
                subject: "creature 1 display_id1".to_string(),
            },
            0,
        );
        pick.facts = Some(vec![
            Fact {
                id: 100,
                path: "Creature\\Wolf\\Wolf.m2".to_string(),
                text: "creature\\wolf\\wolf.m2 wolfskingrey".to_string(),
            },
            Fact {
                id: 200,
                path: "Creature\\Boar\\Boar.m2".to_string(),
                text: "creature\\boar\\boar.m2".to_string(),
            },
        ]);
        let facts = pick.facts.as_deref().unwrap();
        assert_eq!(filter(facts, ""), vec![100, 200]);
        assert_eq!(filter(facts, "wolf"), vec![100]);
        assert_eq!(filter(facts, "grey"), vec![100]);
        assert_eq!(filter(facts, "200"), vec![200]);
        assert_eq!(filter(facts, "cat"), Vec::<u32>::new());
        assert_eq!(pick.path_of(200), Some("Creature\\Boar\\Boar.m2"));
        assert_eq!(pick.path_of(300), None);
    }
}
