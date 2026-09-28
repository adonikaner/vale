//! The server tables a template column names by id, and how a picker lists
//! them.
//!
//! `creature_template.gossip_menu_id` names a `gossip_menu` entry,
//! `equipment_id` a `creature_equip_template` entry, `loot_id` a
//! `creature_loot_template` entry, and so on. None of those tables has a name
//! column, and most of them are several rows under one id, so a picker cannot
//! list them the way it lists creatures. What identifies one of these ids to a
//! person is what is in it and who uses it:
//!
//! ```text
//! gossip_menu                 the first line its first npc_text says
//! creature_equip_template     the three items it holds
//! creature_spells             its own name column
//! npc_vendor_template         how many items, and the creatures that name it
//! npc_trainer_template        how many spells at build 5875, and the same
//! the four loot tables        how many rows at the server's patch, and the
//!                             creatures or chests that name it
//! ```
//!
//! [`query`] answers one row per id with the columns `id`, `title`, `n`,
//! `users` and `named`, and [`Listed::read`] turns such a row into what a
//! picker draws. A search term is matched against what the title shows and
//! against the names of the templates that use the id, and an all-digit term
//! also matches the id.
//!
//! The rows each table is filtered to are the rows the server loads: a loot row
//! or an equipment row between its `patch_min` and `patch_max`, a trainer row
//! whose build range holds 5875. The templates that use an id are counted over
//! every patch of the template, which can count an entry whose winning row no
//! longer names the id; the count is a description, not a check.

use crate::schema::{Row, RowValue};

/// One table a column names by id.
#[derive(Debug)]
pub struct List {
    /// The table, as a column's [`crate::schema::Kind::Ref`] names it.
    pub table: &'static str,
    /// What one id of it is, in a picker's heading: "gossip menu".
    pub noun: &'static str,
    /// What `n` counts, singular: "item", "text".
    counts: &'static str,
    /// One row per id with `id` and `n`, and whatever `title` reads, from `l`.
    /// `{patch}` is the server's content patch.
    rows: &'static str,
    /// Further joins the title needs, after the listing's own.
    joins: &'static str,
    /// The title's SQL, or `''` for a table whose title is its users.
    title: &'static str,
    /// What a search term is matched against besides the users' names, with
    /// `{like}` for the term. Empty for a table whose rows carry no text.
    matches: &'static str,
    /// The templates that name an id: the table, the column, and a condition
    /// that narrows the rows (a game object's type).
    users: (&'static str, &'static str, &'static str),
}

/// Every table a picker can list, in the order a reader looks for one.
pub static LISTS: [List; 9] = [
    List {
        table: "gossip_menu",
        noun: "gossip menu",
        counts: "text",
        rows: "SELECT `entry` AS id, MIN(`text_id`) AS text_id, COUNT(*) AS n \
               FROM `gossip_menu` GROUP BY `entry`",
        joins: "LEFT JOIN `npc_text` t ON t.`ID` = l.text_id \
                LEFT JOIN `broadcast_text` b ON b.`entry` = t.`BroadcastTextID0`",
        title: "COALESCE(NULLIF(b.`male_text`, ''), b.`female_text`, '')",
        matches: "b.`male_text` LIKE {like} OR b.`female_text` LIKE {like}",
        users: (crate::creature::TEMPLATE, "gossip_menu_id", ""),
    },
    List {
        table: "creature_equip_template",
        noun: "equipment set",
        counts: "set",
        rows: "SELECT `entry` AS id, 1 AS n, \
               (SELECT i.`name` FROM `item_template` i WHERE i.`entry` = e.`item1` \
                AND i.`patch` <= {patch} ORDER BY i.`patch` DESC LIMIT 1) AS a, \
               (SELECT i.`name` FROM `item_template` i WHERE i.`entry` = e.`item2` \
                AND i.`patch` <= {patch} ORDER BY i.`patch` DESC LIMIT 1) AS b, \
               (SELECT i.`name` FROM `item_template` i WHERE i.`entry` = e.`item3` \
                AND i.`patch` <= {patch} ORDER BY i.`patch` DESC LIMIT 1) AS c \
               FROM `creature_equip_template` e \
               WHERE {patch} BETWEEN e.`patch_min` AND e.`patch_max`",
        joins: "",
        title: "CONCAT_WS(' \u{b7} ', l.a, l.b, l.c)",
        matches: "l.a LIKE {like} OR l.b LIKE {like} OR l.c LIKE {like}",
        users: (crate::creature::TEMPLATE, "equipment_id", ""),
    },
    List {
        table: crate::creaturespells::TABLE,
        noun: "spell list",
        counts: "spell",
        rows: "SELECT `entry` AS id, `name` AS named_list, \
               (`spellId_1` <> 0) + (`spellId_2` <> 0) + (`spellId_3` <> 0) + \
               (`spellId_4` <> 0) + (`spellId_5` <> 0) + (`spellId_6` <> 0) + \
               (`spellId_7` <> 0) + (`spellId_8` <> 0) AS n \
               FROM `creature_spells`",
        joins: "",
        title: "l.named_list",
        matches: "l.named_list LIKE {like}",
        users: (crate::creature::TEMPLATE, "spell_list_id", ""),
    },
    List {
        table: crate::vendor::TEMPLATE,
        noun: "vendor list",
        counts: "item",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `npc_vendor_template` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::creature::TEMPLATE, "vendor_id", ""),
    },
    List {
        table: crate::trainer::TEMPLATE,
        noun: "trainer list",
        counts: "spell",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `npc_trainer_template` \
               WHERE 5875 BETWEEN `build_min` AND `build_max` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::creature::TEMPLATE, "trainer_id", ""),
    },
    List {
        table: crate::loot::CREATURE,
        noun: "creature loot",
        counts: "row",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `creature_loot_template` \
               WHERE {patch} BETWEEN `patch_min` AND `patch_max` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::creature::TEMPLATE, "loot_id", ""),
    },
    List {
        table: crate::loot::PICKPOCKETING,
        noun: "pickpocketing loot",
        counts: "row",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `pickpocketing_loot_template` \
               WHERE {patch} BETWEEN `patch_min` AND `patch_max` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::creature::TEMPLATE, "pickpocket_loot_id", ""),
    },
    List {
        table: crate::loot::SKINNING,
        noun: "skinning loot",
        counts: "row",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `skinning_loot_template` \
               WHERE {patch} BETWEEN `patch_min` AND `patch_max` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::creature::TEMPLATE, "skinning_loot_id", ""),
    },
    // A chest's and a fishing hole's `lootId` are both `data1`, and those are
    // the two types the server takes loot from; see
    // `crate::gameobject::loot_column`.
    List {
        table: crate::loot::GAMEOBJECT,
        noun: "game object loot",
        counts: "row",
        rows: "SELECT `entry` AS id, COUNT(*) AS n FROM `gameobject_loot_template` \
               WHERE {patch} BETWEEN `patch_min` AND `patch_max` GROUP BY `entry`",
        joins: "",
        title: "''",
        matches: "",
        users: (crate::gameobject::TEMPLATE, "data1", "`type` IN (3, 25)"),
    },
];

/// The list for a table a column names, or `None` for a table this module
/// does not list.
pub fn list(table: &str) -> Option<&'static List> {
    LISTS.iter().find(|list| list.table == table)
}

/// Which ids a [`query`] answers for.
#[derive(Debug, Clone, Copy)]
pub enum Filter<'a> {
    /// Every id whose title or users match the term, or every id for an empty
    /// term, up to the limit. An all-digit term also matches the id.
    Search(&'a str),
    /// These ids and no others, for the names a form draws beside a number.
    Ids(&'a [u32]),
}

/// One row per id of `list`, as [`Listed::read`] reads it, ordered by id.
pub fn query(list: &List, filter: Filter<'_>, wow_patch: u32, limit: usize) -> String {
    let (template, column, narrowed) = list.users;
    let narrowed = match narrowed {
        "" => String::new(),
        more => format!(" AND {more}"),
    };
    let users = format!(
        "SELECT `{column}` AS id, COUNT(DISTINCT `entry`) AS users, MIN(`name`) AS named \
         FROM `{template}` WHERE `{column}` <> 0{narrowed} GROUP BY `{column}`"
    );
    let wanted = match filter {
        Filter::Ids(ids) => {
            let ids: Vec<String> = ids.iter().map(u32::to_string).collect();
            match ids.is_empty() {
                true => "FALSE".to_string(),
                false => format!("l.id IN ({})", ids.join(", ")),
            }
        }
        Filter::Search(term) if term.trim().is_empty() => "TRUE".to_string(),
        Filter::Search(term) => {
            let like = crate::sql::text(&format!("%{}%", term.trim()));
            let mut any = vec![format!(
                "l.id IN (SELECT `{column}` FROM `{template}` WHERE `name` LIKE {like}{narrowed})"
            )];
            if !list.matches.is_empty() {
                any.push(list.matches.replace("{like}", &like));
            }
            if let Ok(id) = term.trim().parse::<u32>() {
                any.push(format!("l.id = {id}"));
            }
            format!("({})", any.join(" OR "))
        }
    };
    let rows = list.rows.replace("{patch}", &wow_patch.to_string());
    format!(
        "SELECT l.id, {title} AS title, l.n, u.users, u.named FROM ({rows}) l {joins} \
         LEFT JOIN ({users}) u ON u.id = l.id \
         WHERE {wanted} ORDER BY l.id LIMIT {limit}",
        title = list.title,
        joins = list.joins,
    )
}

/// One id as a picker lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Listed {
    pub id: u32,
    /// What is in it, or who uses it for a table whose rows carry no text.
    pub title: String,
    /// The count, and who uses it when the title is something else.
    pub sub: String,
}

impl Listed {
    /// A row [`query`] answered, or `None` for one with no id.
    pub fn read(list: &List, row: &Row) -> Option<Listed> {
        let id = row.integer("id")? as u32;
        let n = row.integer("n").unwrap_or(0).max(0) as usize;
        let users = row.integer("users").unwrap_or(0).max(0) as usize;
        let who = users_words(list, users, row.text("named"));
        let counted = match n {
            1 => format!("1 {}", list.counts),
            n => format!("{n} {}s", list.counts),
        };
        let title = row.text("title").unwrap_or_default().trim().to_string();
        Some(match title.is_empty() {
            true => Listed { id, title: who, sub: counted },
            false => Listed {
                id,
                title,
                sub: format!("{counted} \u{b7} {who}"),
            },
        })
    }
}

/// Who uses an id, in words: "Innkeeper Farley and 20 more".
fn users_words(list: &List, users: usize, named: Option<&str>) -> String {
    let noun = match list.users.0 {
        crate::creature::TEMPLATE => "creature",
        _ => "game object",
    };
    match (users, named) {
        (0, _) | (_, None) => format!("named by no {noun}"),
        (1, Some(named)) => named.to_string(),
        (more, Some(named)) => format!("{named} and {} more", more - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[(&str, &str)]) -> Row {
        values
            .iter()
            .map(|(column, value)| (column.to_string(), Some(value.to_string())))
            .collect()
    }

    /// Every table is listed once, and every creature column this module is
    /// for names one of them.
    #[test]
    fn every_list_is_found_by_its_table() {
        for list in &LISTS {
            assert!(std::ptr::eq(super::list(list.table).unwrap(), list), "{}", list.table);
        }
        for column in [
            "gossip_menu_id",
            "equipment_id",
            "spell_list_id",
            "pet_spell_list_id",
            "vendor_id",
            "trainer_id",
            "loot_id",
            "pickpocket_loot_id",
            "skinning_loot_id",
        ] {
            let kind = crate::creature::column(crate::creature::TEMPLATE, column).unwrap().kind;
            match kind {
                crate::schema::Kind::Ref(table) => {
                    assert!(super::list(table).is_some(), "{column} names {table}")
                }
                other => panic!("{column} is {other:?}"),
            }
        }
    }

    /// A search matches the users' names and the title's text, and an
    /// all-digit term the id as well; the term is escaped.
    #[test]
    fn a_search_matches_the_users_the_title_and_the_id() {
        let gossip = list("gossip_menu").unwrap();
        let sql = query(gossip, Filter::Search("goldshire"), 10, 200);
        assert!(sql.contains("`name` LIKE '%goldshire%'"), "{sql}");
        assert!(sql.contains("b.`male_text` LIKE '%goldshire%'"), "{sql}");
        assert!(!sql.contains("l.id = "), "{sql}");
        assert!(sql.ends_with("ORDER BY l.id LIMIT 200"), "{sql}");

        let sql = query(gossip, Filter::Search("342"), 10, 200);
        assert!(sql.contains("l.id = 342"), "{sql}");

        let sql = query(gossip, Filter::Search("o'neill"), 10, 200);
        assert!(sql.contains(r"'%o\'neill%'"), "{sql}");

        let sql = query(list(crate::loot::CREATURE).unwrap(), Filter::Search(""), 10, 50);
        assert!(sql.contains("WHERE TRUE"), "{sql}");
        assert!(sql.contains("10 BETWEEN `patch_min` AND `patch_max`"), "{sql}");
    }

    /// A lookup by ids names exactly those, and an empty set matches nothing.
    #[test]
    fn a_lookup_names_its_ids() {
        let vendors = list(crate::vendor::TEMPLATE).unwrap();
        let sql = query(vendors, Filter::Ids(&[3, 7]), 10, 2);
        assert!(sql.contains("WHERE l.id IN (3, 7)"), "{sql}");
        let sql = query(vendors, Filter::Ids(&[]), 10, 0);
        assert!(sql.contains("WHERE FALSE"), "{sql}");
    }

    /// A chest's loot is named by game objects of the two types that take
    /// loot, and the condition is applied to the search as well.
    #[test]
    fn a_game_objects_loot_is_named_by_chests_and_fishing_holes() {
        let sql = query(list(crate::loot::GAMEOBJECT).unwrap(), Filter::Search("chest"), 10, 200);
        assert!(sql.contains("FROM `gameobject_template` WHERE `data1` <> 0 AND `type` IN (3, 25)"), "{sql}");
        assert!(sql.contains("WHERE `name` LIKE '%chest%' AND `type` IN (3, 25)"), "{sql}");
    }

    /// A row with a title keeps it and says who uses the id beneath; a row
    /// without one is titled by who uses it.
    #[test]
    fn a_row_reads_as_a_title_and_a_line_beneath() {
        let gossip = list("gossip_menu").unwrap();
        let read = Listed::read(
            gossip,
            &row(&[("id", "342"), ("title", "Greetings, $N."), ("n", "2"), ("users", "3"), ("named", "Marshal Dughan")]),
        )
        .unwrap();
        assert_eq!(read.title, "Greetings, $N.");
        assert_eq!(read.sub, "2 texts \u{b7} Marshal Dughan and 2 more");

        let vendors = list(crate::vendor::TEMPLATE).unwrap();
        let read = Listed::read(vendors, &row(&[("id", "7"), ("title", ""), ("n", "1")])).unwrap();
        assert_eq!(read.title, "named by no creature");
        assert_eq!(read.sub, "1 item");
    }
}
