//! The item workspace's state: which item is open, what an edit to a field
//! is, and what the appearance a row names looks like.
//!
//! ## Why the workspace replaces the viewport
//!
//! An item has no place in the world, so, as with [`super::tables`], the
//! viewport is not the document and the workspace replaces it.
//! [`crate::ui::items`] draws it.
//!
//! It is here rather than in `ui/` for the same reason as [`super::tables`]:
//! what is open, what is selected and what an edit does are the editor's state,
//! and the panel is a view of them. `--tool items --item 2589` must be able to
//! put this in a known state before anything is drawn.
//!
//! ## The whole table is read once
//!
//! `item_template` is about 24,000 rows on a full world database and the
//! browser wants to search all of it on every keystroke. So it is read once,
//! on a task, as the eleven columns a list row needs
//! ([`vale_mangos::item::all_items_query`]). That costs about the same as the
//! creature tool's map read, which is 24,610 rows in 0.6 s, and a search is
//! then a filter over memory.
//!
//! The creature picker instead runs a `LIKE` query per keystroke. That suits a
//! picker that looks for one creature to place. This list is where the work is
//! done, and with the table in memory two letters typed into a name filter a
//! `Vec` instead of making a database round trip.
//!
//! It is read again when an apply lands and when *Reload* is pressed. It is
//! not polled.
//!
//! ## Appearances are resolved from the archives and memoised
//!
//! `display_id` is a row of `ItemDisplayInfo.dbc`. Resolving it into the
//! models a weapon hangs on the hand, the eight textures a garment paints and
//! the icon in the bag takes a DBC lookup, a gender-suffix chain and a path
//! build. A picker asks for one per cell on every frame it draws.
//!
//! [`Items::look`] resolves it once per `(display id, slot, race, gender)` and
//! keeps the result. `None` is a display id the table does not carry, kept so
//! it is not asked again.
//!
//! ## The preview body is a panel setting, not part of the item
//!
//! A helm is cut per race and gender — the row names `Helm_Plate_D_04.mdx` and
//! the archive holds sixteen files — so a preview of one has to choose a body
//! before it can choose a file. The choice is [`Items::race`] and
//! [`Items::gender`], on the panel, defaulting to a human male: it changes the
//! picture and never the row.

use super::Tool;
use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_mangos::item::{self, RowValue};
use vale_mangos::row::{Key, Life, RowEdit};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::{HashMap, HashSet};

/// One row of the browse list, as the eleven columns
/// [`vale_mangos::item::all_items_query`] reads.
///
/// Read into numbers once, so drawing a list of twenty-four thousand does not
/// parse text per frame.
#[derive(Debug, Clone)]
pub struct Known {
    /// The entry the project says the item has, which is the one the list
    /// shows, the panel opens by and the store keys its claim under.
    pub entry: u32,
    /// The entry the last read of the table found it at, which differs
    /// while the project changes the item's entry and has not applied it. It
    /// is where the whole row is read from. See [`fold_the_store_in`].
    pub read_entry: u32,
    /// Which content patch of the row the server would load — the other half of
    /// the key an edit is written under.
    pub patch: u32,
    pub name: String,
    pub class: u32,
    pub subclass: u32,
    pub quality: u32,
    pub display_id: u32,
    pub inventory_type: u32,
    pub item_level: u32,
    pub required_level: u32,
    pub flags: u32,
    /// What this project does to the row.
    ///
    /// [`Life::Update`] for a row as the database has it, whether or not this
    /// project edits a column of it, [`Life::Insert`] for one the project
    /// creates, which has no row in the database at all, and [`Life::Delete`]
    /// for one it removes — see `vale_mangos::item`'s module comment on why
    /// a removal needs a restart.
    pub claim: Life,
}

impl Known {
    /// Its key, which is the entry and the patch together.
    pub fn key(&self) -> Key {
        item::template_key(self.entry, self.patch)
    }

    /// What to call it in a list.
    pub fn label(&self) -> String {
        format!("{} ({})", self.name, self.entry)
    }

    /// The dimmer second line of a list row. See [`sub_line`].
    pub fn sub(&self) -> String {
        sub_line(self.class, self.subclass, self.inventory_type, self.item_level)
    }

    /// What the search box is matched against: the name and the second
    /// line, lowercased, with a separator no query can contain so that a match
    /// cannot run from the one into the other.
    fn haystack(&self) -> String {
        format!(
            "{}\u{1}{}",
            self.name.to_ascii_lowercase(),
            self.sub().to_ascii_lowercase()
        )
    }

    /// This row with the project's edits over it, so the list shows what
    /// the project says rather than what was read.
    ///
    /// `None` when the project says nothing about it, which is the ordinary
    /// case and is why this answers an `Option` rather than always cloning.
    pub fn with_edits(&self, edits: &vale_mangos::row::Edits) -> Option<Known> {
        // Look the row up once and read its columns from it. Asking the store
        // for each of the 129 columns in turn built a key per question: 2.3
        // million allocations for one pass over the list, which cost 2.6 s per
        // keystroke in the search box.
        let key = self.key();
        let row = edits.row(item::TEMPLATE, &key);
        let life = row.map(|row| row.life).unwrap_or_default();
        let touched = row.is_some_and(|row| !row.columns.is_empty());
        if !touched && life == self.claim {
            return None;
        }
        let get = |column: &str| row.and_then(|row| row.columns.get(column)).cloned();
        let number = |column: &str, had: u32| {
            get(column)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .map(|value| value as u32)
                .unwrap_or(had)
        };
        Some(Known {
            name: get("name")
                .map(|value| unquote(&value))
                .unwrap_or_else(|| self.name.clone()),
            class: number("class", self.class),
            subclass: number("subclass", self.subclass),
            quality: number("quality", self.quality),
            display_id: number("display_id", self.display_id),
            inventory_type: number("inventory_type", self.inventory_type),
            item_level: number("item_level", self.item_level),
            required_level: number("required_level", self.required_level),
            flags: number("flags", self.flags),
            claim: life,
            ..self.clone()
        })
    }
}

/// The inside of a SQL string literal, for a list row to show.
///
/// It reads a literal the same way as `crate::ui::creatures::unquote`, and is
/// kept separate because that one belongs to a panel and this one to the list.
/// The store holds every value as a literal, and a name drawn with its quotes
/// looks like a bug in the list.
pub fn unquote(literal: &str) -> String {
    let Some(inner) = literal
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return literal.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            other => out.push(other),
        }
    }
    out
}

/// One row of the brief query.
fn read_known(row: &item::Row) -> Option<Known> {
    let integer = |column: &str| row.integer(column).unwrap_or(0) as u32;
    let entry = row.integer("entry")? as u32;
    Some(Known {
        entry,
        read_entry: entry,
        patch: integer("patch"),
        name: row.text("name").unwrap_or_default().to_string(),
        class: integer("class"),
        subclass: integer("subclass"),
        quality: integer("quality"),
        display_id: integer("display_id"),
        inventory_type: integer("inventory_type"),
        item_level: integer("item_level"),
        required_level: integer("required_level"),
        flags: integer("flags"),
        claim: Life::Update,
    })
}

/// One row of `item_template`, whole.
#[derive(Debug, Clone)]
pub struct ItemRow {
    pub entry: u32,
    pub patch: u32,
    pub row: item::Row,
}

/// What one display id looks like, resolved against the archives.
///
/// The three things a panel can draw of an appearance. They are different
/// kinds of data: an icon is a texture, a model is geometry, and the component
/// textures are neither. The component textures are painted into the wearer's
/// own skin and have no picture of their own outside a dressed character.
#[derive(Debug, Clone, Default)]
pub struct Look {
    /// The full path of the bag icon, composed through `ItemTables::icon_path`.
    pub icon: Option<String>,
    /// The bare icon name the row carries, for searching.
    pub icon_name: String,
    /// The attached models, with the skin each names — what a preview draws.
    /// Empty for a garment, which is the ordinary case.
    pub models: Vec<crate::portraits::Worn>,
    /// The eight body-component textures, empty where the row names none.
    pub textures: [String; 8],
    /// `geosetGroup[3]` — which geometry variants the wearer switches to.
    pub geoset_groups: [u32; 3],
    /// Which of the wearer's own geosets a helmet hides, as the row's ids.
    pub helmet_hides: [u32; 2],
}

impl Look {
    /// Whether the row names geometry of its own, as against painting the body.
    pub fn has_models(&self) -> bool {
        !self.models.is_empty()
    }

    /// How many of the eight components it paints.
    pub fn painted(&self) -> usize {
        self.textures.iter().filter(|path| !path.is_empty()).count()
    }
}

/// One row of `ItemDisplayInfo.dbc`, as the three facts a picker needs of all
/// 29,604 of them.
///
/// Built once per session and walked on every keystroke. Asking
/// `ItemDisplays::appearance` per row per search instead costs a DBC lookup and
/// eight string builds for each of twenty-nine thousand rows on the frame a
/// letter is typed, which is too slow for a search box.
#[derive(Debug, Clone)]
pub struct DisplayFacts {
    pub id: u32,
    /// The bare icon name, lower case, which is the only text a display row
    /// carries and so the only word one can be searched by.
    pub icon: String,
    /// Whether it hangs geometry on the wearer, as against painting its skin.
    pub has_model: bool,
    /// Which `Item\ObjectComponents\` directory that model is in, or `None`
    /// for a row that names none.
    ///
    /// The row does not say. `modelName` is a bare file name and the directory
    /// comes from the slot the item is worn in, so the only way to know whether
    /// `Helm_Plate_D_04.mdx` is a helm is that `Item\ObjectComponents\Head\`
    /// holds it, which the archives' file listing answers. See
    /// [`build_display_facts`], where the listing is read once.
    pub directory: Option<&'static str>,
    /// Whether it names a skin for the wearer's own cape geoset. The back is
    /// the one slot whose geometry the character already has and whose
    /// texture the item names.
    ///
    /// `Item\ObjectComponents\Cape\` holds no `.m2` at all (measured: 0 files,
    /// against Head's 1,801), so a cloak carries `modelTexture[0]` and no
    /// model, and a slot filter that asks for geometry offers nothing for the
    /// back. See `vale_assets::tables::item::Slot::is_cloak`.
    pub cloak_texture: bool,
    /// Which body components it paints, a bit per [`Component`] in that enum's
    /// own order.
    pub paints: u8,
}

/// Which appearances the picker offers.
///
/// The whole table is 29,604 rows, which at ninety-six a page is three hundred
/// pages, too many to scroll through. These are the two narrowings that depend
/// on the item rather than on what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Everything the table holds.
    All,
    /// The ones that could be worn in this item's slot. This is the default
    /// and the most useful narrowing; see [`fits_slot`].
    #[default]
    Slot,
    /// The ones that carry a model of their own, in any slot.
    Models,
}

impl Filter {
    pub fn name(self) -> &'static str {
        match self {
            Filter::All => "everything",
            Filter::Slot => "fits this slot",
            Filter::Models => "has a model",
        }
    }

    pub const ALL: [Filter; 3] = [Filter::Slot, Filter::Models, Filter::All];
}

/// Whether an appearance could be worn in a slot.
///
/// Two rules, because a slot is either geometry or paint:
///
/// * a slot with models (head, shoulders, the hands, a shield, the back) wants
///   a row that carries one. A helm row with no model draws nothing on a head.
/// * a slot with components wants a row that paints every component that slot
///   paints. `Slot::components` holds the lists. A boot paints the lower leg
///   and the foot, and a robe paints the lower leg and not the foot, so asking
///   for all of them tells the two apart.
///
/// The test is loose in one direction on purpose. A row carries its whole
/// armour set's textures rather than one garment's (`vale_assets`'
/// `Slot::components` documents this, and it is why a leggings row also names
/// a chest), so a chest filter admits leggings of a set that has a chest. The
/// error is over-inclusion, which costs a few extra cells; the opposite error
/// would hide the appearance somebody is looking for.
///
/// A slot that is neither — a ring, a trinket, a bag, an ammo pouch — has no
/// appearance of its own at all, so everything is offered and the search box
/// does all the narrowing.
pub fn fits_slot(facts: &DisplayFacts, slot: vale_assets::tables::item::Slot) -> bool {
    // A cloak is the exception to both rules: it is the wearer's own
    // group-15 geoset with a texture the item names, so it has no model and
    // paints no body component — see [`DisplayFacts::cloak_texture`].
    if slot.is_cloak() {
        return facts.cloak_texture;
    }
    if let Some(directory) = slot.object_directory() {
        // Match the model's directory, not only "has a model". All 9,263 rows
        // that carry geometry pass "has a model", so a helm picker would offer
        // every sword in the game, 97 pages of them. The directory the file is
        // in is the exact answer and the archives list it. A row whose model
        // this editor cannot place in any directory falls back to the loose
        // test rather than being hidden.
        return match facts.directory {
            Some(had) => had == directory,
            None => facts.has_model,
        };
    }
    let wanted = slot.components();
    if wanted.is_empty() {
        return true;
    }
    wanted
        .iter()
        .all(|component| facts.paints & (1 << component.index()) != 0)
}

/// Which slot an inventory type's appearance is chosen for.
///
/// `Slot::from_inventory_type` with the weapons filled in, which it leaves out
/// on purpose: which hand a weapon goes in depends on the equipment slot rather
/// than the item's type, and a picker has to choose one to show.
pub fn slot_of(inventory_type: u32) -> vale_assets::tables::item::Slot {
    use vale_assets::tables::item::Slot;
    match inventory_type {
        13 | 15 | 17 | 21 | 25 | 26 => Slot::MainHand,
        22 => Slot::OffHand,
        14 | 23 => Slot::Shield,
        other => Slot::from_inventory_type(other),
    }
}

/// How many items the picker's grid offers per page.
///
/// `ItemDisplayInfo` is tens of thousands of rows, too many for one grid. It
/// is paged for the same reason as [`crate::ui::data`]: paging keeps every row
/// reachable instead of cutting the list off after the first few hundred.
pub const DISPLAY_PAGE: usize = 96;

/// What the item workspace is holding.
#[derive(Resource, Debug)]
pub struct Items {
    /// Every item the server would load, as the database has it.
    pub all: Vec<Known>,
    /// The items this project creates, which are in no database and are
    /// rebuilt from the store whenever it moves.
    pub created: Vec<Known>,
    /// Which `EditSession::server_edit_revision` [`Self::created`] was built
    /// for.
    created_for: Option<u64>,
    /// The highest entry the table holds, read with the list. It is one of the
    /// inputs to a new item's entry; see [`Items::next_entry`].
    pub max_entry: Option<u32>,
    /// The read, while it is running.
    task: Option<Task<Result<TableRead, String>>>,
    /// What [`Self::all`] was read for: `EditSession::database_writes`, which
    /// counts this session's applies and put-backs. It is not the edit counter,
    /// which moves on every keystroke. It is not the applied signature either,
    /// which is `None` before an apply this session did not make and `None`
    /// again after that apply is put back; keyed on it, the list kept showing a
    /// row the database no longer held.
    loaded: Option<u64>,
    /// Why there is nothing, when there is nothing.
    pub trouble: Option<String>,
    /// What is in the search box.
    pub query: String,
    /// The rows that match it, as indices over [`Items::at`]'s numbering.
    matches: Vec<usize>,
    /// The query, revision and list lengths [`Self::matches`] was built for, so
    /// a query that has not changed is not searched again on every frame the
    /// panel is drawn.
    built: Option<(String, u64, usize, usize)>,
    /// What each row of [`Self::all`] is found by, lowercased, built once
    /// per read — see [`Known::haystack`] and [`Items::matches`].
    haystacks: Vec<String>,
    /// Which read the haystacks were built for, as an `all_generation` value.
    haystacks_for: Option<u64>,
    /// How many times a read has replaced [`Self::all`].
    all_generation: u64,
    /// The open item, by entry.
    pub open: Option<u32>,
    /// The item the list last brought into view, or was clicked on. The list
    /// scrolls to the open item when it is another. See
    /// `crate::ui::theme::list_area`.
    pub revealed: Option<u32>,
    /// The open item's whole row, read on demand.
    pub row: Option<ItemRow>,
    row_task: Option<Task<Result<ItemRow, String>>>,
    /// What [`Self::row`] was made at: `EditSession::database_writes`, and
    /// whether the project created the item, in which case it is an empty
    /// row. Either changing makes it be read again; see
    /// `crate::server::fresh`.
    row_at: Option<(u64, bool)>,
    /// What a display id looks like, resolved once per body; see the module
    /// comment.
    looks: HashMap<(u32, u32, u8, u8), Option<Look>>,
    /// The body wearing a display id, for an appearance that has no model of
    /// its own; see [`Items::body`]. It is a separate memo beside
    /// [`Self::looks`] because it answers a different question with the same
    /// key: what the row paints, as against what the row is.
    bodies: HashMap<(u32, u32, u8, u8), Option<crate::portraits::Worn>>,
    /// Which body a preview is drawn on. A helm is cut per race and gender,
    /// and a garment is painted on a body, so both kinds of preview have
    /// one to stand on.
    pub race: u8,
    pub gender: u8,
    /// Whether the display picker is open, and what is typed into it.
    pub picking_display: bool,
    pub display_query: String,
    pub display_page: usize,
    /// The ids that match the picker's query, and what they were built for.
    display_hits: Vec<u32>,
    display_built: Option<(String, u32, Filter, u32)>,
    /// Every display row's three facts, built once; see
    /// [`DisplayFacts`]. `None` until the archives have been asked.
    display_facts: Option<Vec<DisplayFacts>>,
    /// Which narrowing the picker is offering — see [`Filter`].
    pub filter: Filter,
    /// Which id the picker's preview pane is showing — the one under the
    /// pointer, or the one chosen.
    pub display_preview: Option<u32>,
    /// Whether the picker's box should take the keyboard on the next frame.
    pub display_focus: bool,
    /// Whether the scripted flags have been acted on. They name a row that
    /// is not read on the frame the flag is read, so they are applied once the
    /// table arrives rather than at startup. See
    /// `crate::server::items::on_the_command_line`, which waits on this.
    pub scripted_done: bool,
    seeded: bool,
    /// A client table's row that a reference on the form was clicked through
    /// to, as the table and the id: a spell, a skill, a faction. The shell
    /// handles it after everything is drawn and opens the table browser on it,
    /// because the shell holds the tool while the form is drawn. See
    /// `crate::ui::draw`.
    pub show_row: Option<(&'static str, u32)>,
    /// A quest that a reference on the form was clicked through to, which
    /// opens the quest workspace.
    pub show_quest: Option<u32>,
    /// A part of the Items workspace the strip at the head of the list asks
    /// the shell to switch to, on [`Self::show_row`]'s terms.
    pub switch_to: Option<super::Tool>,
}

impl Default for Items {
    fn default() -> Items {
        Items {
            all: Vec::new(),
            created: Vec::new(),
            created_for: None,
            max_entry: None,
            task: None,
            loaded: None,
            trouble: None,
            query: String::new(),
            matches: Vec::new(),
            built: None,
            haystacks: Vec::new(),
            haystacks_for: None,
            all_generation: 0,
            open: None,
            revealed: None,
            row: None,
            row_task: None,
            row_at: None,
            looks: HashMap::new(),
            bodies: HashMap::new(),
            // A human male, which is the body `vale attach` reports against
            // and the one whose helms are in every archive.
            race: 1,
            gender: 0,
            picking_display: false,
            display_query: String::new(),
            display_page: 0,
            display_hits: Vec::new(),
            display_built: None,
            display_facts: None,
            filter: Filter::default(),
            display_preview: None,
            display_focus: false,
            scripted_done: false,
            seeded: false,
            show_row: None,
            show_quest: None,
            switch_to: None,
        }
    }
}

/// What one read of the table answered.
#[derive(Debug)]
pub struct TableRead {
    pub items: Vec<Known>,
    pub highest: Option<u32>,
}

impl Items {
    /// How many items there are: the database's and this project's own.
    pub fn count(&self) -> usize {
        self.all.len() + self.created.len()
    }

    /// One of them, by an index over both lists — the database's first.
    ///
    /// Everything that lists, searches or counts goes through this rather than
    /// through [`Self::all`], which is only what the last read answered. An
    /// item this project created that could be listed but not found would be a
    /// row nobody can get back to.
    pub fn at(&self, index: usize) -> Option<&Known> {
        match self.all.get(index) {
            Some(known) => Some(known),
            None => self.created.get(index - self.all.len()),
        }
    }

    /// One item by entry, from either list.
    pub fn by_entry(&self, entry: u32) -> Option<&Known> {
        self.all
            .iter()
            .chain(self.created.iter())
            .find(|known| known.entry == entry)
    }

    /// One item as it should be drawn: the database's reading with the
    /// project's edits over it.
    pub fn shown(&self, index: usize, edits: &vale_mangos::row::Edits) -> Option<Known> {
        let base = self.at(index)?;
        Some(base.with_edits(edits).unwrap_or_else(|| base.clone()))
    }

    /// The open item, with the project's edits over it.
    pub fn open_item(&self, edits: &vale_mangos::row::Edits) -> Option<Known> {
        let entry = self.open?;
        let base = self.by_entry(entry)?;
        Some(base.with_edits(edits).unwrap_or_else(|| base.clone()))
    }

    /// The rows the query matches, as indices, with the newest search cached.
    ///
    /// Two kinds of query, and they are not exclusive, as in
    /// [`crate::tools::tables::Browser::matches`]: a number matches the row
    /// with that entry, and any text matches a row whose name or kind contains
    /// it. So `2589` finds Linen Cloth and `linen` finds every linen item.
    pub fn matches(&mut self, edits: &vale_mangos::row::Edits, revision: u64) -> &[usize] {
        let asked = (
            self.query.trim().to_ascii_lowercase(),
            revision,
            self.all.len(),
            self.created.len(),
        );
        if self.built.as_ref() == Some(&asked) {
            return &self.matches;
        }
        let query = asked.0.clone();
        self.built = Some(asked);
        self.matches.clear();
        let by_entry: Option<u32> = query.parse().ok();
        // The words a row is found by, lowercased once per read rather than
        // composed and lowercased for every row on every keystroke.
        if self.haystacks_for != Some(self.all_generation) || self.haystacks.len() != self.all.len() {
            self.haystacks = self.all.iter().map(Known::haystack).collect();
            self.haystacks_for = Some(self.all_generation);
        }
        // The rows this project edits, found once: a handful out of seventeen
        // thousand. Every other row is searched as it was read, with no lookup
        // and no copy.
        let touched: std::collections::HashSet<(u32, u32)> = edits
            .rows()
            .filter(|(table, _, _)| *table == item::TEMPLATE)
            .filter_map(|(_, key, _)| {
                let patch = key
                    .0
                    .iter()
                    .find(|(column, _)| column == "patch")
                    .and_then(|(_, value)| value.parse::<u32>().ok())?;
                Some((key.first()? as u32, patch))
            })
            .collect();
        for index in 0..self.count() {
            let Some(base) = self.at(index) else {
                continue;
            };
            let plain = index < self.all.len() && !touched.contains(&(base.entry, base.patch));
            let hit = match plain {
                true => {
                    query.is_empty()
                        || by_entry == Some(base.entry)
                        || self.haystacks[index].contains(&query)
                }
                false => {
                    let known = base.with_edits(edits).unwrap_or_else(|| base.clone());
                    query.is_empty()
                        || by_entry == Some(known.entry)
                        || known.haystack().contains(&query)
                }
            };
            if hit {
                self.matches.push(index);
            }
        }
        &self.matches
    }

    /// Forget the cached search, for a caller that has changed what is in the
    /// lists rather than what is in the box.
    pub fn forget_matches(&mut self) {
        self.built = None;
    }

    /// The entry a new item gets.
    ///
    /// The highest of three: the reserved base, one past what the table holds,
    /// and one past the highest this project has already claimed. The creature
    /// tool uses the same arithmetic for the same reasons. See
    /// `vale_mangos::item::RESERVED_ENTRY_BASE`.
    pub fn next_entry(&self, edits: &vale_mangos::row::Edits) -> u32 {
        let in_database = self.max_entry.map(|highest| highest + 1).unwrap_or(0);
        let claimed = edits
            .rows()
            .filter(|(table, _, row)| *table == item::TEMPLATE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .map(|highest| highest as u32 + 1)
            .unwrap_or(0);
        item::RESERVED_ENTRY_BASE.max(in_database).max(claimed)
    }

    /// Make a new item, and open it.
    ///
    /// Every column is written at once, as one claim and one undo entry. See
    /// `vale_mangos::item::new_item` for where the values come from and why
    /// they are not zeros. The patch is the server's own, not zero: a row
    /// written at 0 would be overridden by any later content-patch row of the
    /// same entry. A new entry has no such row today, but would as soon as
    /// somebody imports one.
    pub fn create(&mut self, session: &mut EditSession, name: &str, patch: u32, now: f64) -> u32 {
        let entry = self.next_entry(&session.server_edits);
        let key = item::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in item::new_item(name) {
            row.columns.insert(change.column.to_string(), change.value);
        }
        let subject = format!("item {entry}");
        session.set_server_row(
            item::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "New item",
                subject: &subject,
                now,
            }),
        );
        self.open = Some(entry);
        self.row = None;
        self.forget_matches();
        entry
    }

    /// A copy of the open item. Copying is used far more often than a blank
    /// row: the same sword one point better is this row with a new entry.
    ///
    /// Every column is copied, the database's value where the project has not
    /// changed it and the project's where it has, so the copy is what is
    /// shown, not what the table holds.
    ///
    /// `None` when the whole row has not been read yet, which is the frame or
    /// two after an item is clicked. Copying from a half-read row would take
    /// the table's defaults for the columns that had not arrived.
    pub fn duplicate(&mut self, session: &mut EditSession, patch: u32, now: f64) -> Option<u32> {
        let from = self
            .row
            .as_ref()
            .filter(|held| Some(held.entry) == self.open)?;
        let source = item::template_key(from.entry, from.patch);
        let entry = self.next_entry(&session.server_edits);
        let key = item::template_key(entry, patch);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for column in item::TEMPLATE_COLUMNS
            .iter()
            .filter(|column| column.editable())
        {
            let value = session
                .server_edits
                .get(item::TEMPLATE, &source, column.name)
                .map(str::to_string)
                .or_else(|| column.literal(&from.row));
            if let Some(value) = value {
                row.columns.insert(column.name.to_string(), value);
            }
        }
        // Named as a copy, because two rows with one name cannot be told apart
        // in a list of twenty-four thousand. The name is usually the first
        // thing changed, and the suffix shows which row is the copy.
        let named = row
            .columns
            .get("name")
            .map(|literal| unquote(literal))
            .unwrap_or_default();
        row.columns.insert(
            "name".to_string(),
            vale_mangos::sql::text(&format!("{named} (copy)")),
        );
        let subject = format!("item {entry}");
        session.set_server_row(
            item::TEMPLATE,
            &key,
            Some(&row),
            Some(crate::session::Gesture {
                label: "Copy item",
                subject: &subject,
                now,
            }),
        );
        self.open = Some(entry);
        self.row = None;
        self.forget_matches();
        Some(entry)
    }

    /// Whether an entry is already an item, which a renumbering has to check
    /// before it writes.
    ///
    /// It checks the rows the server would load plus the ones this project
    /// creates, two ways: by the entry the project gives each row and by the
    /// entry the database has it at. Both are checked because an apply runs
    /// against the database. An item this project moves away from
    /// 852 still occupies 852 until that move has run, and the rows of a plan
    /// run in key order, so a second item moved onto 852 could meet the first
    /// still there.
    ///
    /// An entry that exists only at a patch the server does not load is not
    /// seen here and is caught at the apply, which asks the database itself.
    pub fn entry_taken(&self, entry: u32) -> bool {
        self.all
            .iter()
            .chain(self.created.iter())
            .any(|known| known.entry == entry || known.read_entry == entry)
    }

    /// Move an item to another entry.
    ///
    /// [`plan_move`] decides whether it may happen, with no session in hand;
    /// this performs it, and it is one operation whatever the row is: the
    /// project's claim on the row is re-keyed. A row that is in the database
    /// keeps a record of where the database has it, which is what the `UPDATE`
    /// will name — see `vale_mangos::row::RowEdit::from`. So the item is one
    /// row in the list under its new entry before and after the apply, and
    /// every later edit lands on the same claim.
    ///
    /// The project's other rows that name the item follow it: a quest this
    /// project edits to reward entry 852 says the new entry afterwards. The
    /// database's rows follow at the apply — see
    /// `vale_mangos::item::REFERENCES`.
    pub fn rekey(
        &mut self,
        session: &mut EditSession,
        known: &Known,
        to: u32,
        now: f64,
    ) -> Result<(), String> {
        let Some((from, to_key)) = plan_move(known, to, self.entry_taken(to))? else {
            return Ok(());
        };
        // Where the database has the row, for a claim that has not said yet.
        let at_base = item::template_key(known.read_entry, known.patch);
        // One subject for every write, so a move is one press of Ctrl+Z:
        // it is two store writes and one per row that follows, and the undo
        // stack folds them by the subject they name. See
        // `crate::session::Gesture`.
        let subject = format!("item {} entry", known.entry);
        let gesture = crate::session::Gesture {
            label: "Move item entry",
            subject: &subject,
            now,
        };
        crate::server::follow::rekey(
            session,
            item::TEMPLATE,
            &from,
            &to_key,
            Some(&at_base),
            &item::REFERENCES,
            gesture,
        )?;
        // The panel follows the row, which is at the new entry from here.
        self.open = Some(to);
        self.row = None;
        self.created_for = None;
        self.forget_matches();
        Ok(())
    }

    /// Mark an item for removal, or give up one this project created.
    ///
    /// An item in the database becomes a [`Life::Delete`] row, which stays in
    /// the list marked until it is applied, and replaces any column edit the
    /// claim had: a row that is removed has no columns to set. One this project
    /// created is in no database, so its claim is taken back instead — see
    /// [`Self::discard_created`].
    ///
    /// An item whose entry this project changes is refused. Its claim is
    /// keyed by the new entry and the database has it at the old one, so a
    /// `Delete` under the claim's key would remove nothing. Taking the move
    /// back first leaves one entry to name.
    pub fn remove(
        &mut self,
        session: &mut EditSession,
        known: &Known,
        now: f64,
    ) -> Result<(), String> {
        if known.claim == Life::Insert {
            self.discard_created(session, known.entry, now);
            return Ok(());
        }
        if known.read_entry != known.entry {
            return Err(format!(
                "this project moves item {} to {}; move it back before removing it",
                known.read_entry, known.entry
            ));
        }
        let subject = format!("item {} remove", known.entry);
        let row = RowEdit {
            life: Life::Delete,
            ..RowEdit::default()
        };
        session.set_server_row(
            item::TEMPLATE,
            &known.key(),
            Some(&row),
            Some(crate::session::Gesture {
                label: "Remove item",
                subject: &subject,
                now,
            }),
        );
        self.forget_matches();
        Ok(())
    }

    /// Keep an item that was marked for removal: the claim is taken back,
    /// and with it any column edit made before the mark, which a `Delete` row
    /// does not carry.
    pub fn keep(&mut self, session: &mut EditSession, known: &Known, now: f64) {
        if known.claim != Life::Delete {
            return;
        }
        let subject = format!("item {} keep", known.entry);
        session.set_server_row(
            item::TEMPLATE,
            &known.key(),
            None,
            Some(crate::session::Gesture {
                label: "Keep item",
                subject: &subject,
                now,
            }),
        );
        self.forget_matches();
    }

    /// Give up a row this project created, which is in no database, so
    /// giving it up removes nothing there.
    pub fn discard_created(&mut self, session: &mut EditSession, entry: u32, now: f64) {
        let Some(known) = self.by_entry(entry).cloned() else {
            return;
        };
        if known.claim != Life::Insert {
            return;
        }
        let subject = format!("item {entry}");
        session.set_server_row(
            item::TEMPLATE,
            &known.key(),
            None,
            Some(crate::session::Gesture {
                label: "Discard new item",
                subject: &subject,
                now,
            }),
        );
        if self.open == Some(entry) {
            self.open = None;
            self.row = None;
        }
        self.forget_matches();
    }

    /// What a display id looks like on the chosen body, memoised.
    pub fn look(
        &mut self,
        assets: &GameAssets,
        display_id: u32,
        inventory_type: u32,
    ) -> Option<Look> {
        let key = (display_id, inventory_type, self.race, self.gender);
        if let Some(had) = self.looks.get(&key) {
            return had.clone();
        }
        let found = resolve_look(assets, display_id, inventory_type, self.race, self.gender);
        self.looks.insert(key, found.clone());
        found
    }

    /// A body wearing what this display id paints, memoised.
    ///
    /// The picture of the other kind of item. A sword, a helm and a pauldron
    /// hang geometry off the wearer and can be shown by themselves. A shirt, a
    /// pair of gloves and a robe have no geometry at all: they paint textures
    /// into the wearer's own 256x256 body composite, so the only picture of one
    /// is a body wearing it. Two thirds of `ItemDisplayInfo.dbc` is that kind.
    ///
    /// The body is dressed through `look::dress::dress`, the single
    /// implementation of the rule that decides what a display row paints, which
    /// geosets it switches the body to and what a cloak does. That rule belongs
    /// to `assets`; a second implementation here could draw a garment
    /// differently from a character wearing it in the world.
    ///
    /// `None` where the race and gender resolve to no model, which is the same
    /// answer the creature picker gives for a display id the tables do not
    /// know.
    pub fn body(
        &mut self,
        assets: &GameAssets,
        display_id: u32,
        inventory_type: u32,
    ) -> Option<crate::portraits::Worn> {
        let key = (display_id, inventory_type, self.race, self.gender);
        if let Some(had) = self.bodies.get(&key) {
            return had.clone();
        }
        let found = resolve_body(assets, display_id, inventory_type, self.race, self.gender);
        self.bodies.insert(key, found.clone());
        found
    }

    /// Every display row's facts, built once and kept.
    ///
    /// The whole table, which is 29,604 rows: three small fields each, built
    /// from one pass of `ItemDisplays::appearance`. Done lazily rather than at
    /// startup, because a session that never opens the picker should not pay
    /// for it at all.
    pub fn display_facts(&mut self, assets: &GameAssets) -> &[DisplayFacts] {
        if self.display_facts.is_none() {
            self.display_facts = Some(build_display_facts(assets));
        }
        self.display_facts.as_deref().unwrap_or(&[])
    }

    /// The display ids the picker offers, cached until the query, the
    /// filter or the item's slot changes.
    ///
    /// The query is matched against the id and against the row's icon name,
    /// which is the only text a display row carries — `INV_Sword_39` for a
    /// sword, `INV_Chest_Cloth_17` for a robe. It is what the archives call the
    /// appearance and so the only word anybody can search one by.
    pub fn display_matches(&mut self, assets: &GameAssets, inventory_type: u32) -> &[u32] {
        let query = self.display_query.trim().to_ascii_lowercase();
        let asked = (
            query.clone(),
            assets.tables_generation() as u32,
            self.filter,
            inventory_type,
        );
        if self.display_built.as_ref() == Some(&asked) {
            return &self.display_hits;
        }
        if self.display_facts.is_none() {
            self.display_facts = Some(build_display_facts(assets));
        }
        self.display_built = Some(asked);
        self.display_page = 0;
        self.display_hits.clear();
        let by_id: Option<u32> = query.parse().ok();
        let slot = slot_of(inventory_type);
        let filter = self.filter;
        let facts = self.display_facts.as_deref().unwrap_or(&[]);
        for row in facts {
            let allowed = match filter {
                Filter::All => true,
                Filter::Models => row.has_model,
                Filter::Slot => fits_slot(row, slot),
            };
            if !allowed {
                continue;
            }
            let hit = query.is_empty() || by_id == Some(row.id) || row.icon.contains(&query);
            if hit {
                self.display_hits.push(row.id);
            }
        }
        &self.display_hits
    }

    /// Forget the picker's cached hits, for a caller that has changed what it
    /// is filtering by rather than what is typed in it.
    pub fn forget_display_matches(&mut self) {
        self.display_built = None;
    }

    /// Open the picker, on the id the row already names.
    pub fn pick_display(&mut self, showing: u32) {
        self.picking_display = true;
        self.display_focus = true;
        self.display_preview = Some(showing);
        self.display_built = None;
    }

    /// Close the picker.
    pub fn close_picker(&mut self) {
        self.picking_display = false;
        self.display_preview = None;
    }
}

/// An archive path with the extension a BLP is stored under.
///
/// The display row names the icon and the directory table names the folder;
/// neither says `.blp`, and the archives are keyed by the whole file name.
/// The dimmer second line of an item's list row: what kind of item it is,
/// where it is worn, and its item level. Shared by the item list and the
/// reference picker, which draw an item alike.
pub fn sub_line(class: u32, subclass: u32, inventory_type: u32, item_level: u32) -> String {
    let class_word = item::value_word(&item::CLASSES, class);
    let subclass_word = item::value_word(item::subclasses(class), subclass);
    let worn = match inventory_type {
        0 => String::new(),
        other => format!(
            " \u{b7} {}",
            item::value_word(&item::INVENTORY_TYPES, other)
        ),
    };
    match item_level {
        0 => format!("{class_word} \u{b7} {subclass_word}{worn}"),
        level => format!("{class_word} \u{b7} {subclass_word}{worn} \u{b7} ilvl {level}"),
    }
}

/// The archive path of an item's bag icon, from its display id, or `None`
/// when the display row names none.
///
/// `icon_path` joins the directory and the name and adds no extension. The
/// row carries a bare `INV_Sword_39` and the folder comes from
/// `StringLookups.dbc`, and neither includes `.blp`, so the path it returns
/// names no file in the archives. Used as is, every row of the list drew an
/// empty square. `ui::data` appends `.blp` to a spell icon path for the same
/// reason.
pub fn icon_of(assets: &GameAssets, display_id: u32) -> Option<String> {
    if display_id == 0 {
        return None;
    }
    let tables = assets.display_tables().ok()?;
    let icon_name = tables.items()?.inventory_icon(display_id)?;
    tables
        .item_tables()
        .icon_path(&icon_name)
        .map(|path| with_blp(&path))
}

fn with_blp(path: &str) -> String {
    match path.to_ascii_lowercase().ends_with(".blp") {
        true => path.to_string(),
        false => format!("{path}.blp"),
    }
}

/// One pass of `ItemDisplayInfo.dbc`, as the facts a picker filters by.
///
/// Gender 0 throughout: this reads which components a row names and whether it
/// names a model, and neither depends on the body. Only the texture file names
/// do.
fn build_display_facts(assets: &GameAssets) -> Vec<DisplayFacts> {
    let Ok(tables) = assets.display_tables() else {
        return Vec::new();
    };
    let Some(displays) = tables.items() else {
        return Vec::new();
    };
    let held = object_components(assets);
    let mut out = Vec::with_capacity(displays.len());
    for id in displays.ids() {
        let Some(appearance) = displays.appearance(id, 0) else {
            continue;
        };
        let mut paints = 0u8;
        for (at, path) in appearance.textures.iter().enumerate() {
            if !path.is_empty() {
                paints |= 1 << at;
            }
        }
        out.push(DisplayFacts {
            id,
            icon: displays
                .inventory_icon(id)
                .unwrap_or_default()
                .to_ascii_lowercase(),
            has_model: appearance.has_geometry(),
            directory: directory_of(&held, &appearance.models[0]),
            cloak_texture: !appearance.model_textures[0].is_empty() && !appearance.has_geometry(),
            paints,
        });
    }
    out
}

/// The five directories a worn model can be in, as
/// `vale_assets::tables::item::Slot::object_directory` spells them.
const OBJECT_DIRECTORIES: [&str; 5] = ["Head", "Shoulder", "Weapon", "Shield", "Cape"];

/// What each of them holds, read once from the archives' own listing.
///
/// Lower case, with the extension taken off, because that is what a row's
/// `modelName` is once `m2::model_path` has turned `.mdx` into `.m2`.
fn object_components(assets: &GameAssets) -> Vec<(&'static str, HashSet<String>)> {
    let listed = assets
        .with_archive(|chain| Ok(chain.list_prefix("item\\objectcomponents\\")))
        .unwrap_or_default();
    let mut out: Vec<(&'static str, HashSet<String>)> = OBJECT_DIRECTORIES
        .iter()
        .map(|directory| (*directory, HashSet::new()))
        .collect();
    for path in listed {
        if !path.ends_with(".m2") {
            continue;
        }
        let Some(rest) = path.strip_prefix("item\\objectcomponents\\") else {
            continue;
        };
        let Some((directory, file)) = rest.split_once('\\') else {
            continue;
        };
        let stem = file.trim_end_matches(".m2").to_string();
        if let Some((_, held)) = out
            .iter_mut()
            .find(|(name, _)| name.eq_ignore_ascii_case(directory))
        {
            held.insert(stem);
        }
    }
    out
}

/// Which of those directories holds this row's model, or `None`.
///
/// A helm is the one that is not a plain lookup: the row names
/// `Helm_Plate_D_04.mdx` and the archive holds sixteen files,
/// `helm_plate_d_04_hum.m2` through `..._trf.m2`. So a name that is in no
/// directory as itself is tried again with each of the sixteen race-and-gender
/// suffixes, which is `ItemAppearance::attachment_at`'s naming rule applied in
/// reverse.
fn directory_of(held: &[(&'static str, HashSet<String>)], model: &str) -> Option<&'static str> {
    if model.is_empty() {
        return None;
    }
    let stem = vale_assets::world::m2::model_path(model)
        .trim_end_matches(".m2")
        .to_ascii_lowercase();
    for (directory, files) in held {
        if files.contains(&stem) {
            return Some(directory);
        }
    }
    for race in 1..=8u8 {
        let Some(code) = vale_assets::tables::item::race_code(race) else {
            continue;
        };
        for sex in ['m', 'f'] {
            let cut = format!("{stem}_{code}{sex}");
            for (directory, files) in held {
                if files.contains(&cut) {
                    return Some(directory);
                }
            }
        }
    }
    None
}

/// Whether an item may be moved to another entry, as the key its claim is
/// under and the key it goes to.
///
/// `Ok(None)` is a move to the entry the row already has, which is what a form
/// answers on every frame somebody is looking at it and must not become an
/// edit.
///
/// The refusals are the ones that have to be caught before anything is
/// written rather than reported afterwards: an `INSERT` onto an occupied key
/// fails the apply, and an `UPDATE` onto one fails on the primary key with the
/// database's own words.
pub fn plan_move(known: &Known, to: u32, taken: bool) -> Result<Option<(Key, Key)>, String> {
    if to == known.entry {
        return Ok(None);
    }
    // A removed item is not moved: the claim is a `Delete` row, which
    // carries no columns and names the entry it removes.
    if known.claim == Life::Delete {
        return Err("this project removes that item; keep it first".to_string());
    }
    if to == 0 {
        return Err("0 is not an item entry".to_string());
    }
    if to > item::MAX_ENTRY {
        return Err(format!(
            "{to} is past {}, which is all `entry` can hold",
            item::MAX_ENTRY
        ));
    }
    // Moving a row back to where the database has it is always allowed: the
    // entry is taken, and by this row.
    if taken && to != known.read_entry {
        return Err(format!("entry {to} is already an item"));
    }
    Ok(Some((known.key(), item::template_key(to, known.patch))))
}

/// Resolve a [`Look`] from the archives: their side of the `display_id` column.
///
/// `inventory_type` decides the slot, which decides the directory the models
/// live in and the attachment each hangs from — see
/// `vale_assets::tables::item::Slot`. A type with no slot (a ring, a
/// trinket, a reagent) has no models at all, which is not a failure: those
/// items are an icon and nothing else.
fn resolve_look(
    assets: &GameAssets,
    display_id: u32,
    inventory_type: u32,
    race: u8,
    gender: u8,
) -> Option<Look> {
    if display_id == 0 {
        return None;
    }
    let tables = assets.display_tables().ok()?;
    let displays = tables.items()?;
    let appearance = displays.appearance(display_id, gender)?;
    let icon_name = displays.inventory_icon(display_id).unwrap_or_default();
    let icon = icon_of(assets, display_id);

    // Which hand a weapon is in depends on the equipment slot, not the item,
    // so a preview has to choose — see [`slot_of`].
    let slot = slot_of(inventory_type);
    let models = appearance
        .attachments(slot, race, gender)
        .into_iter()
        .map(|attached| crate::portraits::Worn {
            path: attached.path,
            // Slot 4 is the object skin, which is the one an item model
            // asks for: `ModelCache::attached` puts the texture there and this
            // is the same dressing through `worn_unposed`. A shorter array
            // would leave the model's own texture name in place, which for an
            // item is nothing at all and draws magenta.
            skins: vec![
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                attached.texture.unwrap_or_default(),
            ],
            hair: None,
            dress: vale_assets::world::m2::Dress::Creature,
            // An item's own model is a file with a file's textures; nothing
            // about it is composed. See `Items::body`, which is the other kind.
            look: None,
            cloak: None,
        })
        .collect();

    Some(Look {
        icon,
        icon_name,
        models,
        textures: appearance.textures.clone(),
        geoset_groups: appearance.geoset_groups,
        helmet_hides: appearance.helmet_geoset_vis,
    })
}

/// Resolve the dressed body that [`Items::body`] memoises.
///
/// `ChrRaces.dbc`'s own display id for the race and gender — the same number
/// the character-select plinth stands a body on — resolved through the display
/// tables and dressed in the one item, with nothing else on and nothing in its
/// hands. The appearance is the first of everything: skin 0, face 0, hair
/// style 1, because what is being looked at is the garment and not the model.
fn resolve_body(
    assets: &GameAssets,
    display_id: u32,
    inventory_type: u32,
    race: u8,
    gender: u8,
) -> Option<crate::portraits::Worn> {
    let tables = assets.display_tables().ok()?;
    let body = crate::lab::BODIES
        .iter()
        .find(|body| body.race == race && body.gender == gender)?;
    let display = tables.creature(body.display_id)?;
    let appearance = vale_assets::look::character::Appearance {
        race,
        gender,
        skin: 0,
        face: 0,
        hair_style: 1,
        hair_colour: 0,
        facial_hair: 0,
    };
    let dressed = vale_assets::look::dress::dress(
        &tables,
        &display,
        &vale_assets::look::dress::Wearer {
            appearance: Some(appearance),
            equipment: &[(display_id, inventory_type)],
            weapons: Default::default(),
            sheath_state: vale_assets::look::sheath::UNARMED,
        },
    );
    Some(crate::portraits::Worn {
        path: display.path.clone(),
        // Empty and unused: a composed body keys and dresses by
        // its look rather than by a file — see `portraits::Worn::look`.
        skins: Vec::new(),
        hair: dressed.hair,
        dress: dressed.dress,
        look: dressed.look,
        cloak: dressed.cloak,
    })
}

/// The prototype a tooltip is composed from, out of `item_template`'s
/// columns.
///
/// `value` answers a column's SQL literal: the project's value where it has
/// one and the database's otherwise, which is what the form shows. So the plate
/// drawn from this follows every edit on the frame it is made.
///
/// The fields are `SMSG_ITEM_QUERY_SINGLE_RESPONSE`'s, and the server fills
/// that packet from these columns one for one (`ItemHandler.cpp`,
/// `HandleItemQuerySingleOpcode`); the names differ only in spelling. A column
/// that is absent or not a number reads as zero, which is the table's own
/// default for every numeric column.
pub fn item_info(value: &dyn Fn(&str) -> Option<String>) -> vale_protocol::state::query::ItemInfo {
    use vale_protocol::state::query::{ItemDamage, ItemInfo, ItemSpell, ItemStat};
    let integer = |column: &str| -> i64 {
        value(column)
            .and_then(|literal| literal.trim().parse::<f64>().ok())
            .map(|number| number as i64)
            .unwrap_or(0)
    };
    let unsigned = |column: &str| integer(column) as u32;
    let float = |column: &str| -> f32 {
        value(column)
            .and_then(|literal| literal.trim().parse::<f32>().ok())
            .unwrap_or(0.0)
    };
    let text = |column: &str| value(column).map(|literal| unquote(&literal)).unwrap_or_default();
    let mut info = ItemInfo {
        entry: unsigned("entry"),
        display_id: unsigned("display_id"),
        inventory_type: unsigned("inventory_type"),
        name: text("name"),
        class: unsigned("class"),
        subclass: unsigned("subclass"),
        quality: unsigned("quality"),
        flags: unsigned("flags"),
        buy_price: unsigned("buy_price"),
        sell_price: unsigned("sell_price"),
        allowable_class: integer("allowable_class") as i32,
        allowable_race: integer("allowable_race") as i32,
        item_level: unsigned("item_level"),
        required_level: unsigned("required_level"),
        required_skill: unsigned("required_skill"),
        required_skill_rank: unsigned("required_skill_rank"),
        required_spell: unsigned("required_spell"),
        stackable: unsigned("stackable"),
        container_slots: unsigned("container_slots"),
        armor: integer("armor") as i32,
        resistances: [
            integer("holy_res") as i32,
            integer("fire_res") as i32,
            integer("nature_res") as i32,
            integer("frost_res") as i32,
            integer("shadow_res") as i32,
            integer("arcane_res") as i32,
        ],
        delay: unsigned("delay"),
        ammo_type: unsigned("ammo_type"),
        bonding: unsigned("bonding"),
        description: text("description"),
        page_text: unsigned("page_text"),
        page_material: unsigned("page_material"),
        start_quest: unsigned("start_quest"),
        lock_id: unsigned("lock_id"),
        material: integer("material") as i32,
        sheath: unsigned("sheath"),
        random_property: unsigned("random_property"),
        block: unsigned("block"),
        item_set: unsigned("set_id"),
        max_durability: unsigned("max_durability"),
        bag_family: unsigned("bag_family"),
        ..ItemInfo::default()
    };
    for (slot, stat) in info.stats.iter_mut().enumerate() {
        *stat = ItemStat {
            kind: unsigned(&format!("stat_type{}", slot + 1)),
            value: integer(&format!("stat_value{}", slot + 1)) as i32,
        };
    }
    for (slot, damage) in info.damage.iter_mut().enumerate() {
        *damage = ItemDamage {
            min: float(&format!("dmg_min{}", slot + 1)),
            max: float(&format!("dmg_max{}", slot + 1)),
            school: unsigned(&format!("dmg_type{}", slot + 1)),
        };
    }
    for (slot, spell) in info.spells.iter_mut().enumerate() {
        let n = slot + 1;
        *spell = ItemSpell {
            spell_id: unsigned(&format!("spellid_{n}")),
            trigger: unsigned(&format!("spelltrigger_{n}")),
            charges: integer(&format!("spellcharges_{n}")) as i32,
            cooldown_ms: integer(&format!("spellcooldown_{n}")) as i32,
            category: unsigned(&format!("spellcategory_{n}")),
            category_cooldown_ms: integer(&format!("spellcategorycooldown_{n}")) as i32,
        };
    }
    info
}

impl Items {
    /// One column of the open item as the form shows it: the project's
    /// literal where it has one and the database's otherwise. `None` while the
    /// row is being read, which is a frame or two after an item is opened.
    pub fn shown_value(
        &self,
        edits: &vale_mangos::row::Edits,
        known: &Known,
        column: &str,
    ) -> Option<Option<String>> {
        let row = self.row.as_ref().filter(|held| held.entry == known.entry)?;
        let key = known.key();
        Some(
            edits
                .get(item::TEMPLATE, &key, column)
                .map(str::to_string)
                .or_else(|| {
                    item::column(item::TEMPLATE, column).and_then(|found| found.literal(&row.row))
                }),
        )
    }

    /// The open item's whole prototype on the same terms, for the tooltip.
    /// `None` while the row is being read.
    pub fn open_info(
        &self,
        edits: &vale_mangos::row::Edits,
        known: &Known,
    ) -> Option<vale_protocol::state::query::ItemInfo> {
        self.row.as_ref().filter(|held| held.entry == known.entry)?;
        let value = |column: &str| self.shown_value(edits, known, column).flatten();
        let mut info = item_info(&value);
        // The entry the project gives it, which is the one the list shows.
        info.entry = known.entry;
        Some(info)
    }
}

/// Read the whole table, on a task, when an apply has changed what is in it.
fn read_the_table(
    mut items: ResMut<Items>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    // Collect a finished read whatever the tool is, so switching away and back
    // does not lose one that was already paid for.
    if let Some(task) = items.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            items.task = None;
            match done {
                Ok(read) => {
                    info!("items: {} row(s) read", read.items.len());
                    let mut all = read.items;
                    // A removal that has been applied is kept in the list,
                    // under the name the last read gave it: the project still
                    // claims it, and a claim that is in no list cannot be kept
                    // or looked at. See `rebuild_created` for one that no read
                    // in this session has seen.
                    if let Some(session) = session.as_ref() {
                        let gone: Vec<Known> = items
                            .all
                            .iter()
                            .filter(|known| {
                                session.server_edits.life(item::TEMPLATE, &known.key())
                                    == Life::Delete
                                    && !all.iter().any(|read| read.entry == known.entry)
                            })
                            .cloned()
                            .collect();
                        if !gone.is_empty() {
                            all.extend(gone);
                            all.sort_by_key(|known| known.entry);
                        }
                    }
                    items.all = all;
                    items.all_generation += 1;
                    items.max_entry = read.highest;
                    items.trouble = None;
                    items.created_for = None;
                    // The open row was read out of the table this replaces.
                    items.row = None;
                    items.forget_matches();
                }
                Err(e) => {
                    warn!("items: {e}");
                    items.all.clear();
                    items.all_generation += 1;
                    items.trouble = Some(e);
                }
            }
        }
        return;
    }
    // Read while a playtest is running too, which is the one place this
    // differs from the creature tool. An item edit takes effect on a reload,
    // so the workspace is usable inside a playtest, and refusing to read the
    // table there would leave it empty. It must not read when the tool is not
    // chosen, which is the guard below.
    let _ = &state;
    if *tool != Tool::Items {
        return;
    }
    let Some(session) = session else { return };
    let key = session.database_writes;
    if items.loaded == Some(key) {
        return;
    }
    // Set before the read, not after: a failed read must not ask again on every
    // frame, which against a server that is not there is a connection attempt
    // sixty times a second.
    items.loaded = Some(key);
    let Some((at, _source)) = settings.resolve() else {
        items.all.clear();
        items.all_generation += 1;
        items.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    items.trouble = None;
    items.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&item::all_items_query(patch))?;
        // The highest entry in the whole table, whatever patch it is at: a
        // new item numbered from the winning rows alone would collide with a
        // row that exists at a patch this server is not loading.
        let highest = db
            .row(item::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        Ok(TableRead {
            items: rows.iter().filter_map(read_known).collect(),
            highest,
        })
    }));
}

/// Read the open item's whole row, on a task, when the open item changes.
fn read_the_row(
    mut items: ResMut<Items>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
) {
    if let Some(task) = items.row_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            items.row_task = None;
            match done {
                Ok(row) => items.row = Some(row),
                Err(e) => warn!("item row: {e}"),
            }
        }
        return;
    }
    if *tool != Tool::Items {
        return;
    }
    let Some(entry) = items.open else { return };
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let created_now = items
        .by_entry(entry)
        .is_some_and(|known| known.claim == Life::Insert);
    if items.row.as_ref().is_some_and(|held| held.entry == entry) && items.row_at == Some((writes, created_now)) {
        return;
    }
    items.row_at = Some((writes, created_now));
    // A row this project created has no row in the database, so there is
    // nothing to read: its columns are the store's own and the form reads them
    // from there. Asking would be a query per frame that always answers
    // nothing.
    let created = items
        .by_entry(entry)
        .is_some_and(|known| known.claim == Life::Insert);
    if created {
        let patch = items.by_entry(entry).map(|known| known.patch).unwrap_or(0);
        items.row = Some(ItemRow {
            entry,
            patch,
            row: item::Row::new(),
        });
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    let patch = super::creatures::server_patch(&settings);
    // Read from where the database has it, which is not the entry the
    // panel opened while the project moves the item and has not applied it.
    let read_at = items
        .by_entry(entry)
        .map(|known| known.read_entry)
        .unwrap_or(entry);
    items.row_task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        // A row the database does not hold is an empty row, which the form
        // shows as absent. Answering nothing left the row unread, so it was
        // asked for again on every following frame.
        let Some(row) = db.row(&item::winning_template_query(read_at, patch))? else {
            return Ok(ItemRow {
                entry,
                patch,
                row: item::Row::new(),
            });
        };
        let at_patch = row.integer("patch").unwrap_or(0) as u32;
        Ok(ItemRow {
            entry,
            patch: at_patch,
            row,
        })
    }));
}

/// Give every row that was read the entry the project says it has.
///
/// An item whose entry the project changes is in the table that was read at the
/// entry the database has it at, until the move is applied and the table is
/// read again. The store keys that row's claim under the new entry, so the
/// list has to show it there or the row and its claim are two things: one
/// unedited row at the old entry and one claim nobody can open. After this,
/// [`Known::key`] is the claim's key for every row, moved or not, before an
/// apply and after it.
///
/// Every row is reset first, so a move that was taken back returns the row to
/// the entry it was read at.
pub fn fold_the_store_in(all: &mut [Known], edits: &vale_mangos::row::Edits) {
    let moves: Vec<(u32, u32, u32)> = edits
        .moves(item::TEMPLATE)
        .filter_map(|(from, to)| {
            let patch = from
                .0
                .iter()
                .find(|(column, _)| column == "patch")
                .and_then(|(_, value)| value.parse::<u32>().ok())?;
            Some((from.first()? as u32, patch, to.first()? as u32))
        })
        .collect();
    for known in all.iter_mut() {
        known.entry = moves
            .iter()
            .find(|(from, patch, _)| *from == known.read_entry && *patch == known.patch)
            .map(|(_, _, to)| *to)
            .unwrap_or(known.read_entry);
    }
}

/// Rebuild the items this project creates, from the store, when it changes.
///
/// A second list rather than rows appended to [`Items::all`], because that one
/// is what a read answered and is replaced whole by the next read. The
/// creature tool is arranged the same way for the same reason.
fn rebuild_created(mut items: ResMut<Items>, session: Option<Res<EditSession>>) {
    let Some(session) = session else { return };
    if items.created_for == Some(session.server_edit_revision) {
        return;
    }
    items.created_for = Some(session.server_edit_revision);
    fold_the_store_in(&mut items.all, &session.server_edits);
    items.created.clear();
    for (table, key, row) in session.server_edits.rows() {
        if table != item::TEMPLATE || row.life != Life::Insert {
            continue;
        }
        let number = |column: &str| {
            row.columns
                .get(column)
                .and_then(|value| value.trim().parse::<i64>().ok())
                .map(|value| value as u32)
                .unwrap_or(0)
        };
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        // Once it has been applied it is in the table that was read, and
        // listing it from both would draw it twice and count it twice. The
        // project still claims it as a creation, which `Known::with_edits`
        // reads off the store for the row that is in [`Items::all`].
        if items.all.iter().any(|known| known.entry == entry) {
            continue;
        }
        let patch = key
            .0
            .iter()
            .find(|(column, _)| column == "patch")
            .and_then(|(_, value)| value.parse::<u32>().ok())
            .unwrap_or(0);
        items.created.push(Known {
            entry,
            read_entry: entry,
            patch,
            name: row
                .columns
                .get("name")
                .map(|l| unquote(l))
                .unwrap_or_default(),
            class: number("class"),
            subclass: number("subclass"),
            quality: number("quality"),
            display_id: number("display_id"),
            inventory_type: number("inventory_type"),
            item_level: number("item_level"),
            required_level: number("required_level"),
            flags: number("flags"),
            claim: Life::Insert,
        });
    }
    // A removal no read has seen: applied in an earlier session, so the
    // item is in neither list. A stand-in carrying the entry and the patch is
    // enough to list it, open it and keep it; its name is not known any more.
    for (table, key, row) in session.server_edits.rows() {
        if table != item::TEMPLATE || row.life != Life::Delete {
            continue;
        }
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        if items.all.iter().any(|known| known.entry == entry) {
            continue;
        }
        let patch = key
            .0
            .iter()
            .find(|(column, _)| column == "patch")
            .and_then(|(_, value)| value.parse::<u32>().ok())
            .unwrap_or(0);
        items.created.push(Known {
            entry,
            read_entry: entry,
            patch,
            name: format!("removed item {entry}"),
            class: 0,
            subclass: 0,
            quality: 0,
            display_id: 0,
            inventory_type: 0,
            item_level: 0,
            required_level: 0,
            flags: 0,
            claim: Life::Delete,
        });
    }
    items.created.sort_by_key(|known| known.entry);
    items.forget_matches();
}

/// The command line's four item flags, acted on once the table has come
/// back — see [`crate::Args`].
fn on_the_command_line(
    args: Res<crate::Args>,
    mut items: ResMut<Items>,
    mut session: Option<ResMut<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    time: Res<Time>,
) {
    if items.seeded {
        return;
    }
    let wanted = args.item.is_some()
        || args.item_picker
        || args.item_new
        || args.item_entry.is_some()
        || args.item_remove;
    if !wanted {
        items.seeded = true;
        items.scripted_done = true;
        return;
    }
    // Wait for the table, so `--item <name>` can be a name and `--item-new`
    // can number itself above what the table holds. A read that failed leaves
    // `trouble` set and nothing to wait for.
    if items.task.is_some() || (items.all.is_empty() && items.trouble.is_none()) {
        return;
    }
    items.seeded = true;
    let Some(session) = session.as_mut() else {
        return;
    };
    if args.item_new {
        let patch = super::creatures::server_patch(&settings);
        let entry = items.create(session, "New Item", patch, time.elapsed_secs_f64());
        info!("--item-new: item {entry} created at patch {patch}");
    }
    if let Some(which) = args.item.clone() {
        match which.parse::<u32>() {
            Ok(entry) => items.open = Some(entry),
            Err(_) => {
                let wanted = which.to_ascii_lowercase();
                match items
                    .all
                    .iter()
                    .find(|known| known.name.to_ascii_lowercase() == wanted)
                    .or_else(|| {
                        items
                            .all
                            .iter()
                            .find(|known| known.name.to_ascii_lowercase().contains(&wanted))
                    }) {
                    Some(known) => items.open = Some(known.entry),
                    None => warn!("--item {which}: no item of that name"),
                }
            }
        }
    }
    if args.item_picker {
        let showing = items
            .open
            .and_then(|entry| items.by_entry(entry))
            .map(|known| known.display_id)
            .unwrap_or(0);
        items.pick_display(showing);
    }
    // Not finished if there is a move still to make. `--apply-items` waits
    // on this flag, and a move that landed after the apply would be applied by
    // nothing. See [`scripted_entry`], which sets it instead.
    items.scripted_done = args.item_entry.is_none() && !args.item_remove;
}

/// `--item-entry <n>` renumbers the open item and `--item-remove` removes it,
/// with nobody at the keyboard. Given both, only the move happens, because a
/// moved item is not removed; see [`Items::remove`].
///
/// This is its own system rather than a line in [`on_the_command_line`],
/// because a row that flag has just created is not in [`Items::created`] until
/// [`rebuild_created`] has run on the next frame. Done inline, it reported "no
/// item is open to move" on the frame it created one. It is written as a
/// retry, the same shape as [`scripted_display`]: it does nothing until the
/// row it is about can be found.
fn scripted_entry(
    args: Res<crate::Args>,
    mut items: ResMut<Items>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if (args.item_entry.is_none() && !args.item_remove) || *done || !items.seeded {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(known) = items.open.and_then(|open| items.by_entry(open)).cloned() else {
        return;
    };
    *done = true;
    let now = time.elapsed_secs_f64();
    if let Some(entry) = args.item_entry {
        match items.rekey(session, &known, entry, now) {
            Ok(()) => info!("--item-entry {entry}: item {} moved", known.entry),
            Err(why) => warn!("--item-entry {entry}: {why}"),
        }
        if args.item_remove {
            warn!("--item-remove: not with --item-entry; a moved item is not removed");
        }
    } else {
        let known = known.with_edits(&session.server_edits).unwrap_or(known);
        match items.remove(session, &known, now) {
            Ok(()) => info!("--item-remove: item {} removed", known.entry),
            Err(why) => warn!("--item-remove: {why}"),
        }
    }
    items.scripted_done = true;
}

/// `--item-display <id>`: a display id written to the open item with nobody at
/// the keyboard.
///
/// Its own system because it has to run after the row has arrived: writing
/// `display_id` needs the key, which is the row's entry and patch, and
/// [`on_the_command_line`] fires on the frame the table lands, several frames
/// earlier.
fn scripted_display(
    args: Res<crate::Args>,
    mut items: ResMut<Items>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    let Some(display_id) = args.item_display else {
        return;
    };
    if *done || !items.seeded {
        return;
    }
    let Some(session) = session.as_mut() else {
        return;
    };
    let Some(entry) = items.open else { return };
    let Some(known) = items.by_entry(entry).cloned() else {
        return;
    };
    *done = true;
    let subject = format!("item {entry} display_id");
    session.set_server_edit(
        item::TEMPLATE,
        &known.key(),
        "display_id",
        Some(display_id.to_string()),
        Some(crate::session::Gesture {
            label: "Edit item",
            subject: &subject,
            now: time.elapsed_secs_f64(),
        }),
    );
    items.close_picker();
    info!("--item-display {display_id}: written to item {entry}");
}

pub struct ItemToolPlugin;

impl Plugin for ItemToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Items>().add_systems(
            Update,
            (
                read_the_table,
                rebuild_created,
                read_the_row,
                on_the_command_line,
                scripted_entry,
                scripted_display,
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::row::Edits;

    /// Every numbered group lands in its own slot, and a text column loses its
    /// quotes. The tooltip is composed from this, so a stat read into the wrong
    /// slot or a name drawn with its quotes makes a wrong tooltip.
    #[test]
    fn the_prototype_is_read_column_for_column() {
        let columns: HashMap<&str, &str> = [
            ("name", "'Arcanite Reaper'"),
            ("description", "'It\\'s sharp'"),
            ("quality", "4"),
            ("delay", "3800"),
            ("dmg_min1", "153"),
            ("dmg_max1", "256.5"),
            ("stat_type2", "7"),
            ("stat_value2", "13"),
            ("spellid_3", "18384"),
            ("spelltrigger_3", "1"),
            ("fire_res", "-5"),
            ("allowable_class", "-1"),
            ("set_id", "12"),
        ]
        .into_iter()
        .collect();
        let info = item_info(&|column| columns.get(column).map(|value| value.to_string()));
        assert_eq!(info.name, "Arcanite Reaper");
        assert_eq!(info.description, "It's sharp");
        assert_eq!(info.quality, 4);
        assert_eq!(info.delay, 3800);
        assert_eq!(info.damage[0].min, 153.0);
        assert_eq!(info.damage[0].max, 256.5);
        assert_eq!((info.stats[1].kind, info.stats[1].value), (7, 13));
        assert_eq!(info.stats[0].kind, 0, "an absent column reads as zero");
        assert_eq!((info.spells[2].spell_id, info.spells[2].trigger), (18384, 1));
        assert_eq!(info.resistances[1], -5, "fire is the second school");
        assert_eq!(info.allowable_class, -1);
        assert_eq!(info.item_set, 12);
    }

    fn an_item(entry: u32) -> Known {
        Known {
            entry,
            read_entry: entry,
            patch: 0,
            name: "Linen Cloth".to_string(),
            class: 7,
            subclass: 0,
            quality: 1,
            display_id: 1542,
            inventory_type: 0,
            item_level: 5,
            required_level: 0,
            flags: 0,
            claim: Life::Update,
        }
    }

    /// The list shows what the project says, not what was read: a name
    /// typed into the form is the name in the list beside it.
    #[test]
    fn a_row_is_drawn_with_the_projects_edits_over_it() {
        let base = an_item(2589);
        let mut edits = Edits::default();
        assert!(
            base.with_edits(&edits).is_none(),
            "an untouched row is not rebuilt"
        );
        edits.set(
            item::TEMPLATE,
            &base.key(),
            "name",
            Some(vale_mangos::sql::text("Linen Bandage")),
        );
        edits.set(
            item::TEMPLATE,
            &base.key(),
            "quality",
            Some("3".to_string()),
        );
        let shown = base.with_edits(&edits).expect("edited");
        assert_eq!(shown.name, "Linen Bandage");
        assert_eq!(shown.quality, 3);
        assert_eq!(shown.entry, 2589, "the key is not moved by an edit");
    }

    /// A name is stored as a SQL literal and drawn as itself, with its escapes
    /// resolved, so a list row never shows `'Gnomish Death Ray\'s'`.
    #[test]
    fn a_stored_name_is_unquoted_for_the_list() {
        assert_eq!(unquote("'Linen Cloth'"), "Linen Cloth");
        assert_eq!(unquote("'Gnomish Death Ray\\'s'"), "Gnomish Death Ray's");
        assert_eq!(unquote("2589"), "2589", "a number is not a literal");
    }

    /// The second line says what the item is in words, which is why a
    /// subclass is read through its class.
    #[test]
    fn the_list_line_says_what_the_item_is() {
        let mut sword = an_item(19019);
        sword.class = 2;
        sword.subclass = 7;
        sword.inventory_type = 13;
        sword.item_level = 83;
        assert_eq!(
            sword.sub(),
            "Weapon \u{b7} Sword (one-hand) \u{b7} One-hand \u{b7} ilvl 83"
        );
        let cloth = an_item(2589);
        assert_eq!(cloth.sub(), "Trade Goods \u{b7} Trade Goods \u{b7} ilvl 5");
    }

    /// A new entry is above the reserved base, the table and the project.
    ///
    /// The third is what stops two items created between two reads of the table
    /// from taking the same entry, which is an `INSERT` pair whose second
    /// silently replaces the first.
    #[test]
    fn a_new_entry_clears_the_base_the_table_and_the_project() {
        let mut items = Items::default();
        let edits = Edits::default();
        assert_eq!(items.next_entry(&edits), item::RESERVED_ENTRY_BASE);

        items.max_entry = Some(item::RESERVED_ENTRY_BASE + 40);
        assert_eq!(items.next_entry(&edits), item::RESERVED_ENTRY_BASE + 41);

        let mut edits = Edits::default();
        let key = item::template_key(item::RESERVED_ENTRY_BASE + 100, 10);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        row.columns.insert("class".to_string(), "15".to_string());
        edits.set_row_line(item::TEMPLATE, &key, Some(&row.to_line()));
        assert_eq!(items.next_entry(&edits), item::RESERVED_ENTRY_BASE + 101);
    }

    /// An item this project creates is listed beside the database's, and is
    /// reachable by entry. A row that could be made but not found again could
    /// not be edited.
    #[test]
    fn a_created_item_is_in_the_list_and_findable() {
        let mut items = Items::default();
        items.all = vec![an_item(2589)];
        items.created = vec![Known {
            claim: Life::Insert,
            ..an_item(2_000_000)
        }];
        assert_eq!(items.count(), 2);
        assert_eq!(items.at(0).map(|k| k.entry), Some(2589));
        assert_eq!(items.at(1).map(|k| k.entry), Some(2_000_000));
        assert_eq!(
            items.by_entry(2_000_000).map(|k| k.claim),
            Some(Life::Insert)
        );
    }

    /// A move is a re-key of the row's claim whatever the row is, from the
    /// key the claim is under to the same patch of the new entry.
    #[test]
    fn a_move_is_a_re_key_for_a_new_row_and_an_old_one() {
        let new = Known {
            claim: Life::Insert,
            ..an_item(2_000_000)
        };
        assert_eq!(
            plan_move(&new, 2_000_005, false),
            Ok(Some((
                item::template_key(2_000_000, 0),
                item::template_key(2_000_005, 0)
            )))
        );
        let old = an_item(2589);
        assert_eq!(
            plan_move(&old, 2_000_006, false),
            Ok(Some((
                item::template_key(2589, 0),
                item::template_key(2_000_006, 0)
            )))
        );
    }

    /// A moved row is one row of the list, at the entry the project gives it,
    /// and its key is the claim's key, so the list, the form and the store all
    /// refer to the same row. Taking the move back returns it.
    #[test]
    fn a_moved_row_is_listed_once_at_its_new_entry() {
        let mut all = vec![an_item(2589), an_item(2592)];
        let mut edits = Edits::default();
        let from = item::template_key(2589, 0);
        let to = item::template_key(2_000_006, 0);
        edits.set(item::TEMPLATE, &from, "quality", Some("4".to_string()));
        assert!(edits.rekey(item::TEMPLATE, &from, &to, None));
        fold_the_store_in(&mut all, &edits);
        assert_eq!(all[0].entry, 2_000_006);
        assert_eq!(all[0].read_entry, 2589);
        assert_eq!(all[0].key(), to);
        assert_eq!(
            all[0].with_edits(&edits).map(|shown| shown.quality),
            Some(4)
        );
        assert_eq!(all[1].entry, 2592);

        // The entry the database still has it at is not free for another row,
        // and moving this one back onto it is allowed.
        let items = Items {
            all: all.clone(),
            ..Items::default()
        };
        assert!(items.entry_taken(2589));
        assert!(items.entry_taken(2_000_006));
        assert!(plan_move(&all[0], 2589, true).is_ok());
        assert!(plan_move(&all[1], 2589, true).is_err());

        assert!(edits.rekey(item::TEMPLATE, &to, &from, None));
        fold_the_store_in(&mut all, &edits);
        assert_eq!(all[0].entry, 2589);
    }

    /// A move to where the row already is is not an edit, which is what a
    /// form answers on every frame somebody is looking at it.
    #[test]
    fn a_move_to_the_same_entry_is_nothing() {
        assert_eq!(plan_move(&an_item(2589), 2589, false), Ok(None));
    }

    /// An entry that is already an item is refused, and so are the two the
    /// column cannot hold.
    #[test]
    fn a_move_onto_an_entry_that_exists_is_refused() {
        let row = an_item(2589);
        assert!(plan_move(&row, 2592, true).is_err());
        assert!(plan_move(&row, 0, false).is_err());
        assert!(plan_move(&row, item::MAX_ENTRY + 1, false).is_err());
        assert!(plan_move(&row, item::MAX_ENTRY, false).is_ok());
    }

    /// A slot with models wants a row that carries one, and a slot that paints
    /// wants a row that paints all of what it paints.
    ///
    /// The picker depends on this: the whole table is 29,604 rows and three
    /// hundred pages, and somebody choosing a helm wants to see only helms.
    #[test]
    fn a_slot_takes_the_appearances_that_could_go_in_it() {
        use vale_assets::tables::item::{Component, Slot};
        let facts = |directory: Option<&'static str>, paints: &[Component]| DisplayFacts {
            id: 1,
            icon: String::new(),
            has_model: directory.is_some(),
            directory,
            cloak_texture: false,
            paints: paints
                .iter()
                .fold(0u8, |mask, component| mask | 1 << component.index()),
        };

        // A helm is geometry, and it is geometry in the head directory: a
        // sword has a model too, and offering every one of them is 97 pages.
        let helm = facts(Some("Head"), &[]);
        let sword = facts(Some("Weapon"), &[]);
        let robe = facts(
            None,
            &[
                Component::ArmUpper,
                Component::ArmLower,
                Component::TorsoUpper,
                Component::TorsoLower,
                Component::LegUpper,
                Component::LegLower,
            ],
        );
        assert!(fits_slot(&helm, Slot::Head));
        assert!(!fits_slot(&robe, Slot::Head));
        assert!(!fits_slot(&sword, Slot::Head), "a sword is not a helm");
        assert!(fits_slot(&sword, Slot::MainHand));
        assert!(!fits_slot(&helm, Slot::MainHand));

        // A model this editor cannot place falls back to the loose test,
        // so a row whose file is missing from the archives is still offered
        // rather than disappearing from every slot.
        let unplaced = DisplayFacts {
            has_model: true,
            directory: None,
            ..helm.clone()
        };
        assert!(fits_slot(&unplaced, Slot::Head));
        assert!(fits_slot(&unplaced, Slot::MainHand));

        // A boot paints the lower leg and the foot, which is what tells it
        // from the robe that also paints a lower leg.
        let boot = facts(None, &[Component::LegLower, Component::Foot]);
        assert!(fits_slot(&boot, Slot::Feet));
        assert!(!fits_slot(&robe, Slot::Feet));
        assert!(fits_slot(&robe, Slot::Legs));
        assert!(fits_slot(&robe, Slot::Chest));

        // A slot that is neither offers everything: a ring has no appearance of
        // its own at all, so narrowing it would narrow to nothing.
        assert!(fits_slot(&boot, Slot::Other));
        assert!(fits_slot(&helm, Slot::Other));

        // A cloak is neither a model nor a painted component. The archives
        // hold no `.m2` in `Cape\` at all, so a back-slot filter that asked for
        // geometry offered nothing. This case guards against that.
        let cloak = DisplayFacts {
            has_model: false,
            directory: None,
            cloak_texture: true,
            ..helm.clone()
        };
        assert!(fits_slot(&cloak, Slot::Back));
        assert!(!fits_slot(&helm, Slot::Back), "a helm is not a cloak");
        assert!(!fits_slot(&cloak, Slot::Head));
    }

    /// Which hand a weapon is in depends on the equipment slot, so a picker
    /// chooses one, and every weapon type lands on a slot that has models.
    #[test]
    fn every_weapon_type_picks_a_slot_with_models() {
        use vale_assets::tables::item::Slot;
        for worn in [13u32, 15, 17, 21, 25, 26] {
            assert_eq!(slot_of(worn), Slot::MainHand, "inventory type {worn}");
        }
        assert_eq!(slot_of(22), Slot::OffHand);
        assert_eq!(slot_of(14), Slot::Shield);
        assert_eq!(slot_of(23), Slot::Shield);
        assert_eq!(slot_of(1), Slot::Head);
        assert!(slot_of(13).object_directory().is_some());
        // A slot that is neither geometry nor paint is `Other`, which the
        // filter passes everything for.
        assert_eq!(slot_of(11), Slot::Other);
    }

    /// The search takes an entry and any word of the line, and is cached until
    /// something it searched moves.
    #[test]
    fn the_search_takes_an_entry_or_a_word() {
        let mut items = Items::default();
        items.all = vec![
            an_item(2589),
            Known {
                name: "Wool Cloth".into(),
                ..an_item(2592)
            },
        ];
        let edits = Edits::default();

        items.query = "2592".to_string();
        assert_eq!(items.matches(&edits, 0), &[1]);
        items.query = "cloth".to_string();
        assert_eq!(items.matches(&edits, 0), &[0, 1]);
        items.query = "linen".to_string();
        assert_eq!(items.matches(&edits, 0), &[0]);
        // The kind, which is the second line rather than the name.
        items.query = "trade goods".to_string();
        assert_eq!(items.matches(&edits, 0).len(), 2);
        items.query = "nothing here".to_string();
        assert!(items.matches(&edits, 0).is_empty());
    }

    /// What one keystroke in the search box costs, over a table the size of
    /// the reference install's, with a project that edits a handful of rows.
    /// `--ignored --nocapture` prints it; it is an instrument, not a check.
    #[test]
    #[ignore]
    fn bench_one_keystroke_over_the_whole_table() {
        let mut items = Items::default();
        items.all = (1..=17_710)
            .map(|entry| Known {
                name: format!("Item number {entry}"),
                ..an_item(entry)
            })
            .collect();
        let mut edits = Edits::default();
        for entry in [25, 2589, 9000, 17_000] {
            edits.set(item::TEMPLATE, &item::template_key(entry, 0), "quality", Some("3".into()));
        }
        let mut worst = std::time::Duration::ZERO;
        for (revision, query) in ["i", "it", "ite", "item", "item n", "900"].iter().enumerate() {
            items.query = query.to_string();
            let started = std::time::Instant::now();
            let found = items.matches(&edits, revision as u64).len();
            let took = started.elapsed();
            worst = worst.max(took);
            println!("{query:>8}: {found:>6} rows in {took:?}");
        }
        println!("worst keystroke: {worst:?}");
    }
}
