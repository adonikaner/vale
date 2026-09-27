//! **The C functions `Interface\GlueXML\CharacterCreate.lua` calls** — the third
//! glue screen, and the first one with a decision in it.
//!
//! ```text
//! GetAvailableRaces()        (name, fileString) x 8, Alliance then Horde
//! GetClassesForRace()        (name, FILENAME) x n, for the selected race
//! GetSelectedRace/Sex/Class  …and which of them is chosen, one-based
//! SetSelectedRace/Sex/Class
//! GetNameForRace()           (name, fileString) for the selected race
//! GetFactionForRace()        (name, "Alliance") for the selected race
//! GetHairCustomization()     "NORMAL" / "HORNS" — a word, not a number
//! GetFacialHairCustomization()   …and its per-gender twin
//! HasCharCustomization(i)    whether axis i has more than one option
//! CycleCharCustomization(i, d)   …and the arrows that walk it
//! RandomizeCharCustomization()
//! ResetCharCustomize()
//! Get/SetCharacterCreateFacing   the drag and the two rotate buttons
//! UpdateCustomizationScene()     "redraw", which this renderer does anyway
//! CreateCharacter(name)      the Accept button
//! ```
//!
//! ## Why the state is here and not queued
//!
//! `CharacterRace_OnClick` is four lines and the third and fourth are
//! `SetSelectedRace(id)` followed by `SetCharacterRace(id)` — which immediately
//! asks `GetFactionForRace()`, `GetNameForRace()` and `GetClassesForRace()` and
//! expects all three to describe the race just chosen. A write recorded and
//! drained a system later answers the *previous* race to all three, which draws
//! the right button highlighted with the wrong description under it and the
//! wrong class buttons beside it. So [`Board`] is `Rc<RefCell<…>>` held by
//! [`crate::lua::host::LuaHost`] and every one of these is unscoped: none of
//! them needs the world, because **none of this is on the wire** — see
//! [`vale_assets::tables::charcreate`], which is the whole rule.
//!
//! The one exception is `CreateCharacter`, which owns a socket for a round trip
//! and therefore records, exactly as `DefaultServerLogin` does.
//!
//! ## One-based everywhere, and three different things are counted
//!
//! `GetSelectedRace()` is an index into the **race list**, `GetSelectedClass`'s
//! partner `SetSelectedClass(id)` an index into *that race's* class list, and
//! `GetSelectedSex()` is 1 for male and 2 for female. None of the three is a
//! `ChrRaces`, `ChrClasses` or `PLAYER_BYTES` value, and the wire wants all
//! three of those — [`Board::race_id`] and its neighbours are the one place the
//! two numberings cross. Getting it wrong makes a night elf when the player
//! picked a dwarf, which the server accepts.
//!
//! ## The random is a counter and that is deliberate
//!
//! `ResetCharCustomize()` is `CharacterCreate_OnShow`'s first line and its own
//! comment says "randomly selects a combination". A client with a real entropy
//! source there is a client whose `--audit --glue` run differs every time, so
//! this walks a fixed-seed [`Board::rng`] instead: different every press, the
//! same on every run. Nothing downstream can tell, and the probe can.

use std::cell::RefCell;
use std::rc::Rc;

use vale_assets::tables::charcreate::CharCreate;

/// **The globals this file registers**, sorted — the same census list
/// [`super::glue::WRITES`] is for the two screens before it.
///
/// All of them, reads included: none is a scoped read, because none of them
/// touches the world. See the module comment.
pub const GLOBALS: [&str; 17] = [
    "CreateCharacter",
    "CycleCharCustomization",
    "GetAvailableRaces",
    "GetCharacterCreateFacing",
    "GetClassesForRace",
    "GetFacialHairCustomization",
    "GetFactionForRace",
    "GetHairCustomization",
    "GetNameForRace",
    "GetSelectedClass",
    "GetSelectedRace",
    "GetSelectedSex",
    "HasCharCustomization",
    "RandomizeCharCustomization",
    "SetCharacterCreateFacing",
    "SetSelectedClass",
    "SetSelectedRace",
    // `ResetCharCustomize`, `SetSelectedSex` and `UpdateCustomizationScene`
    // bring it to twenty; see `the_registered_set_is_the_list`, which is what
    // keeps this honest rather than the count in this comment.
];

/// …and the three whose names do not fit the array above without making it
/// unreadable. Kept apart only so the sorted list stays sorted; the test walks
/// both.
pub const MORE_GLOBALS: [&str; 3] = [
    "ResetCharCustomize",
    "SetSelectedSex",
    "UpdateCustomizationScene",
];

/// **What the character-create screen has chosen**, and the tables it is
/// choosing out of.
///
/// One value rather than a resource because the reads have to answer *during* a
/// handler — see the module comment — and because it is small, plain data that
/// `crate::glue::charcreate` copies out once a frame to build the plinth.
#[derive(Default)]
pub struct Board {
    /// `ChrRaces` + `CharBaseInfo` + the rest, loaded when the glue loads.
    /// `None` before the archives are open, which answers an empty screen
    /// rather than a wrong one.
    pub tables: Option<std::sync::Arc<CharCreate>>,
    /// **Zero-based index into [`CharCreate::races`]**, not a race id.
    pub race: usize,
    /// …and into that race's own class list.
    pub class: usize,
    /// 0 male, 1 female — the `PLAYER_BYTES` value, which is
    /// `GetSelectedSex() - 1`.
    pub gender: u8,
    /// One index into each of the five customization axes, in
    /// `CycleCharCustomization`'s own order: skin, face, hair style, hair
    /// colour, facial hair.
    pub picks: [usize; 5],
    /// The fixed-seed walk `ResetCharCustomize` and `RandomizeCharCustomization`
    /// use — see the module comment for why it is not entropy.
    rng: u32,
}

/// The five axes, as `CycleCharCustomization` numbers them.
const AXES: usize = 5;

impl Board {
    /// The `ChrRaces` id of the chosen race — **the number that goes on the
    /// wire**, which the interface never sees.
    pub fn race_id(&self) -> u8 {
        self.chosen_race().map_or(0, |r| r.id)
    }

    /// …and the `ChrClasses` id, the same way.
    pub fn class_id(&self) -> u8 {
        self.tables
            .as_ref()
            .and_then(|t| t.classes_for(self.race_id()).get(self.class))
            .map_or(0, |c| c.id)
    }

    fn chosen_race(&self) -> Option<&vale_assets::tables::charcreate::RaceChoice> {
        self.tables.as_ref()?.races().get(self.race)
    }

    fn looks(&self) -> Option<&vale_assets::tables::charcreate::Looks> {
        self.tables.as_ref()?.looks(self.race_id(), self.gender)
    }

    /// **The five appearance bytes, in `PLAYER_BYTES` order** — which is the
    /// order `CMSG_CHAR_CREATE` wants them in and the order `SMSG_CHAR_ENUM`
    /// answers them in, so the character that is created and the one that then
    /// stands on the character-select plinth are described the same way.
    ///
    /// An axis with nothing in it answers 0, which is the value every one of
    /// these fields has for a character the tables cannot describe.
    pub fn appearance(&self) -> [u8; 5] {
        let mut out = [0u8; 5];
        let Some(looks) = self.looks() else { return out };
        for (axis, value) in out.iter_mut().enumerate() {
            let list = looks.axis(axis + 1);
            *value = list.get(self.picks[axis]).copied().unwrap_or(0);
        }
        out
    }

    /// **What the character being made is wearing** — its starting outfit, out
    /// of `CharStartOutfit.dbc`.
    ///
    /// Not underwear: the client clears the twelve visible slots and both hands
    /// and then equips the row for this exact `(race, class, gender)`, so the
    /// create screen shows a warrior in a Recruit's shirt with a Worn Shortsword
    /// in hand. An empty answer is the honest degradation and looks like the
    /// underwear the composite paints under it.
    pub fn outfit(&self) -> Vec<vale_assets::tables::charcreate::OutfitPiece> {
        self.tables.as_ref().map_or_else(Vec::new, |t| {
            t.outfit(self.race_id(), self.class_id(), self.gender).to_vec()
        })
    }

    /// Everything the plinth needs, as the same type the world dresses a
    /// character from.
    pub fn look(&self) -> vale_assets::look::character::Appearance {
        let a = self.appearance();
        vale_assets::look::character::Appearance {
            race: self.race_id(),
            gender: self.gender,
            skin: a[0],
            face: a[1],
            hair_style: a[2],
            hair_colour: a[3],
            facial_hair: a[4],
        }
    }

    /// Move one axis by `delta`, **wrapping**, which is what the two arrows do:
    /// `CharacterCustomization_Left` at index 0 goes to the last option rather
    /// than stopping.
    fn cycle(&mut self, axis: usize, delta: i32) {
        let Some(len) = self.axis_len(axis) else { return };
        if len == 0 {
            return;
        }
        let at = self.picks[axis - 1] as i64;
        let len = len as i64;
        self.picks[axis - 1] = (at + i64::from(delta)).rem_euclid(len) as usize;
    }

    fn axis_len(&self, axis: usize) -> Option<usize> {
        (1..=AXES).contains(&axis).then_some(())?;
        Some(self.looks()?.axis(axis).len())
    }

    /// A new look for the current body, and the whole of both
    /// `RandomizeCharCustomization` and `ResetCharCustomize`.
    fn randomize(&mut self) {
        for axis in 1..=AXES {
            let len = self.axis_len(axis).unwrap_or(0);
            self.picks[axis - 1] = if len == 0 { 0 } else { self.next_random() % len };
        }
    }

    /// A 32-bit xorshift, seeded once. See the module comment: the point is that
    /// it *varies* within a session and does not vary between runs.
    fn next_random(&mut self) -> usize {
        // The seed matters only in that it is not zero, which xorshift cannot
        // leave.
        if self.rng == 0 {
            self.rng = 0x9e37_79b9;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as usize
    }

    /// **Put every index back inside its list**, after a change of race or
    /// gender.
    ///
    /// A tauren male has 19 skins and a gnome 5, so a player who picked skin 12
    /// and then pressed Gnome is pointing past the end of the list — and the
    /// appearance byte that comes out of that is 0, silently. Clamping rather
    /// than zeroing keeps the two ends of the range: pick the last option, which
    /// is what the reference's own index does when its list shortens.
    fn clamp(&mut self) {
        let classes = self
            .tables
            .as_ref()
            .map_or(0, |t| t.classes_for(self.race_id()).len());
        if self.class >= classes {
            self.class = 0;
        }
        for axis in 1..=AXES {
            let len = self.axis_len(axis).unwrap_or(0);
            self.picks[axis - 1] = self.picks[axis - 1].min(len.saturating_sub(1));
        }
    }
}

pub type Held = Rc<RefCell<Board>>;

/// **The one thing here that needs a socket** — drained by
/// `crate::glue::charcreate`, on the same terms as [`super::glue::GlueRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRequest {
    /// `CreateCharacter(name)` — the Accept button. Carries only the name,
    /// because everything else about the character is [`Board`]'s and the
    /// draining system reads it there rather than being handed a copy that
    /// could disagree.
    Create(String),
}

pub type Queue = Rc<RefCell<Vec<CreateRequest>>>;

/// Register the lot. Unscoped: see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, board: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    /// Both list getters have the same shape — a flat `(name, fileString)`
    /// sequence whose length the caller divides by two — so they are built by
    /// one macro rather than written twice.
    macro_rules! pairs {
        ($name:expr, |$board:ident| $rows:expr) => {{
            let held = Rc::clone(board);
            let f = lua.create_function(move |lua, ()| {
                let $board = held.borrow();
                let rows: Vec<(String, String)> = $rows;
                let mut out = Vec::with_capacity(rows.len() * 2);
                for (name, file) in rows {
                    out.push(mlua::Value::String(lua.create_string(&name)?));
                    out.push(mlua::Value::String(lua.create_string(&file)?));
                }
                Ok(mlua::Variadic::from(out))
            })?;
            globals.set($name, f)?;
        }};
    }

    // **`(localised name, fileString)` per race, in the order the buttons are
    // drawn.** `CharacterCreateEnumerateRaces` reads `arg.n/2` as the count and
    // `strupper(arg[i+1].."_"..gender)` as the icon's key, so the second of each
    // pair has to be `clientFileString` and not the printed name — "NightElf",
    // one word, and "Scourge" rather than "Undead".
    pairs!("GetAvailableRaces", |board| board
        .tables
        .as_ref()
        .map(|t| t
            .races()
            .iter()
            .map(|r| (r.name.clone(), r.file_string.clone()))
            .collect())
        .unwrap_or_default());

    // …and the classes of whichever race is chosen. `CLASS_ICON_TCOORDS` is
    // keyed by the second without `strupper`, because `ChrClasses.filename` is
    // already upper case.
    pairs!("GetClassesForRace", |board| board
        .tables
        .as_ref()
        .map(|t| t
            .classes_for(board.race_id())
            .iter()
            .map(|c| (c.name.clone(), c.file_name.clone()))
            .collect())
        .unwrap_or_default());

    /// One number in, nothing out — the three `SetSelected*` writes.
    macro_rules! setter {
        ($name:expr, |$board:ident, $value:ident| $body:expr) => {{
            let held = Rc::clone(board);
            let f = lua.create_function(move |_, $value: Option<i64>| {
                let mut $board = held.borrow_mut();
                let $value = $value.unwrap_or(0);
                $body;
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    // **One-based, and out of range is ignored rather than clamped.** The
    // interface only ever passes a button's own id, so a value outside the list
    // is a bug somewhere else and silently moving the selection to the nearest
    // race would hide it.
    setter!("SetSelectedRace", |board, index| {
        let count = board.tables.as_ref().map_or(0, |t| t.races().len());
        if let Some(at) = usize::try_from(index - 1).ok().filter(|i| *i < count) {
            board.race = at;
            // **A race change is a new body**, so both the class list and all
            // five axes may have shortened under the indices pointing into them.
            board.clamp();
        }
    });
    setter!("SetSelectedClass", |board, index| {
        let count = board
            .tables
            .as_ref()
            .map_or(0, |t| t.classes_for(board.race_id()).len());
        if let Some(at) = usize::try_from(index - 1).ok().filter(|i| *i < count) {
            board.class = at;
        }
    });
    // **1 is male and 2 is female**, which is the one place in this client those
    // two numbers appear: everywhere else a gender is `PLAYER_BYTES`' 0 and 1.
    // `SetCharacterGender`'s own branch is `if ( sex == 1 )`.
    setter!("SetSelectedSex", |board, sex| {
        if (1..=2).contains(&sex) {
            board.gender = (sex - 1) as u8;
            board.clamp();
        }
    });

    /// …and the reads that answer one number.
    macro_rules! number {
        ($name:expr, |$board:ident| $body:expr) => {{
            let held = Rc::clone(board);
            let f = lua.create_function(move |_, ()| {
                let $board = held.borrow();
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }

    number!("GetSelectedRace", |board| board.race as i64 + 1);
    number!("GetSelectedSex", |board| i64::from(board.gender) + 1);

    // **`GetSelectedClass` answers a *pair*, unlike its two neighbours** —
    // `(className, classFileName)` — because `SetCharacterClass` uses the second
    // as a texture-coordinate key and the first as the label. A race with no
    // classes answers two nils, which is the same shape `GetCharacterInfo` uses
    // for a row that is not there.
    {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, ()| {
            let board = held.borrow();
            let chosen = board
                .tables
                .as_ref()
                .and_then(|t| t.classes_for(board.race_id()).get(board.class).cloned());
            Ok(match chosen {
                Some(c) => (Some(c.name), Some(c.file_name)),
                None => (None, None),
            })
        })?;
        globals.set("GetSelectedClass", f)?;
    }

    /// The three reads that answer a pair of strings off the chosen race.
    macro_rules! race_pair {
        ($name:expr, |$race:ident| $body:expr) => {{
            let held = Rc::clone(board);
            let f = lua.create_function(move |_, ()| {
                let board = held.borrow();
                Ok(match board.chosen_race() {
                    Some($race) => $body,
                    None => (None, None),
                })
            })?;
            globals.set($name, f)?;
        }};
    }

    // `(race, fileString)` — `SetCharacterRace` puts the first in the label and
    // `strupper`s the second into `RACE_INFO_*`, `ABILITY_INFO_*` and the icon's
    // own coordinates.
    race_pair!("GetNameForRace", |race| (
        Some(race.name.clone()),
        Some(race.file_string.clone())
    ));
    // `(name, faction)` — the second is compared against the literal
    // `"Alliance"` and is `strupper`'d into `FACTION_INFO_*`, so it has to be
    // `FactionGroup.internalName` and not the localised label.
    race_pair!("GetFactionForRace", |race| (
        Some(race.side_name.clone()),
        Some(race.side.clone())
    ));

    // **Two words rather than two numbers**, and each is pasted into a
    // `GlueStrings.lua` key: `HAIR_<word>_STYLE` / `_COLOR` and
    // `FACIAL_HAIR_<word>`. A race the tables do not describe answers `"NONE"`,
    // which is the client's own fallback string and which
    // `CharacterCreate_UpdateFacialHairCustomization` reads as "hide the fifth
    // customization outright".
    {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, ()| {
            let board = held.borrow();
            Ok(board
                .chosen_race()
                .map_or_else(|| NONE.to_string(), |r| r.hair_word.clone()))
        })?;
        globals.set("GetHairCustomization", f)?;
    }
    {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, ()| {
            let board = held.borrow();
            let gender = usize::from(board.gender).min(1);
            Ok(board.chosen_race().map_or_else(
                || NONE.to_string(),
                |r| {
                    let word = &r.facial_hair_word[gender];
                    if word.is_empty() {
                        NONE.to_string()
                    } else {
                        word.clone()
                    }
                },
            ))
        })?;
        globals.set("GetFacialHairCustomization", f)?;
    }

    // **`HasCharCustomization(i)` is "more than one option", not "any"** —
    // one choice or fewer answers nil, so an axis with exactly one choice
    // hides its arrows rather than drawing two that do nothing.
    {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, axis: Option<usize>| {
            let board = held.borrow();
            let len = board.axis_len(axis.unwrap_or(0)).unwrap_or(0);
            Ok(super::super::api::one_or_nil(len > 1))
        })?;
        globals.set("HasCharCustomization", f)?;
    }

    {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, (axis, delta): (Option<usize>, Option<i32>)| {
            held.borrow_mut()
                .cycle(axis.unwrap_or(0), delta.unwrap_or(0));
            Ok(())
        })?;
        globals.set("CycleCharCustomization", f)?;
    }
    for name in ["RandomizeCharCustomization", "ResetCharCustomize"] {
        let held = Rc::clone(board);
        let f = lua.create_function(move |_, _: mlua::MultiValue| {
            held.borrow_mut().randomize();
            Ok(())
        })?;
        globals.set(name, f)?;
    }

    // **`UpdateCustomizationScene()` is "redraw the character"**, which this
    // renderer does every frame off [`Board`] itself — see
    // `crate::glue::charcreate`. Registered rather than absent because
    // `CharacterCreate_UpdateModel` calls it before `AdvanceTime` on every one
    // of the model frame's ticks, and a nil there would stop the scene's clock.
    globals.set(
        "UpdateCustomizationScene",
        lua.create_function(|_, _: mlua::MultiValue| Ok(()))?,
    )?;

    // **The facing pair, on the frame** — the same shape and the same reason as
    // `Get`/`SetCharacterSelectFacing` one file over:
    // `CharacterCreateFrame_OnUpdate` is
    // `SetCharacterCreateFacing(GetCharacterCreateFacing() + diff)`, so the
    // write has to be visible to the next read inside one handler. Degrees, and
    // `CharacterCreate_OnShow` opens on **-15** of them.
    globals.set(
        "GetCharacterCreateFacing",
        lua.create_function(|lua, ()| {
            let held: Option<mlua::Table> = lua.globals().get(CREATE_FRAME)?;
            Ok(held.as_ref().map_or(0.0, super::super::widgets::model::character_facing))
        })?,
    )?;
    globals.set(
        "SetCharacterCreateFacing",
        lua.create_function(|lua, degrees: Option<f32>| {
            let Some(held) = lua.globals().get::<Option<mlua::Table>>(CREATE_FRAME)? else {
                return Ok(());
            };
            super::super::widgets::model::set_character_facing(&held, degrees.unwrap_or(0.0))
        })?,
    )?;

    // …and the Accept button, which is the only thing here that owns a socket.
    {
        let queue = Rc::clone(queue);
        let f = lua.create_function(move |_, name: Option<String>| {
            let name = name.unwrap_or_default();
            // **A blank name records nothing.** The server answers
            // `CHAR_NAME_NO_NAME` for one, which is a round trip to be told what
            // the empty box already said — and 1.12's own Accept is pressable
            // with nothing typed, so this really is reachable.
            if !name.trim().is_empty() {
                queue.borrow_mut().push(CreateRequest::Create(name));
            }
            Ok(())
        })?;
        globals.set("CreateCharacter", f)?;
    }
    Ok(())
}

/// The frame the two facing functions land on. `CharacterCreate` is the
/// `<ModelFFX>` itself — `SetCharCustomizeFrame("CharacterCreate")` is
/// `CharacterCreate_OnLoad`'s own line 60 saying so.
const CREATE_FRAME: &str = "CharacterCreate";

/// What `GetHairCustomization` answers for a race the tables cannot describe —
/// the client's own fallback, and the string
/// `CharacterCreate_UpdateFacialHairCustomization` tests against to hide the
/// fifth axis.
const NONE: &str = "NONE";

#[cfg(test)]
mod tests {
    use super::*;

    use vale_assets::tables::charcreate::{ClassChoice, Looks, OutfitPiece, RaceChoice};
    use std::collections::BTreeMap;

    fn piece(display_id: u32, inventory_type: u8) -> OutfitPiece {
        OutfitPiece {
            display_id,
            inventory_type,
        }
    }

    /// A board over a three-race stand-in **with unequal axis lengths**, which
    /// is the whole point: the bugs here are about an index surviving a change
    /// of body, and a fixture where every race has the same number of skins
    /// cannot show one.
    fn board() -> Held {
        let race = |id: u8, name: &str, side: &str| RaceChoice {
            id,
            name: name.to_string(),
            file_string: name.to_string(),
            side: side.to_string(),
            side_name: side.to_string(),
            hair_word: "NORMAL".to_string(),
            facial_hair_word: ["NORMAL".to_string(), "PIERCINGS".to_string()],
        };
        let warrior = ClassChoice {
            id: 1,
            name: "Warrior".to_string(),
            file_name: "WARRIOR".to_string(),
        };
        let mage = ClassChoice {
            id: 8,
            name: "Mage".to_string(),
            file_name: "MAGE".to_string(),
        };
        // Alliance first, as `assets::charcreate` orders them: human, dwarf,
        // then the orc.
        let races = vec![
            race(1, "Human", "Alliance"),
            race(3, "Dwarf", "Alliance"),
            race(2, "Orc", "Horde"),
        ];
        let classes = BTreeMap::from([
            (1, vec![warrior.clone(), mage]),
            (2, vec![warrior.clone()]),
            (3, vec![warrior]),
        ]);
        let generous = Looks {
            skins: vec![0, 1, 2],
            faces: vec![3, 4],
            hair_styles: vec![0, 1],
            hair_colours: vec![5, 6],
            facial_hairs: vec![0, 1],
        };
        // …and a body with exactly one of everything, which is what a held index
        // has to survive being moved onto.
        let spare = Looks {
            skins: vec![7],
            faces: vec![0],
            hair_styles: vec![0],
            hair_colours: vec![0],
            facial_hairs: vec![0],
        };
        let looks = BTreeMap::from([
            ((1, 0), generous.clone()),
            ((1, 1), generous),
            ((2, 0), spare.clone()),
            ((2, 1), spare.clone()),
            ((3, 0), spare.clone()),
            ((3, 1), spare),
        ]);
        Rc::new(RefCell::new(Board {
            // …and a starting outfit for the one combination the wearing test
            // uses: a shirt, a sword and a shield, which is the shape every real
            // row has — something worn, something in each hand.
            tables: Some(std::sync::Arc::new(CharCreate::from_parts(
                races,
                classes,
                looks,
                BTreeMap::from([(
                    (1, 1, 0),
                    vec![
                        piece(9891, 4),   // INVTYPE_BODY, a shirt
                        piece(1542, 21),  // INVTYPE_WEAPONMAINHAND
                        piece(18730, 14), // INVTYPE_SHIELD
                        piece(2130, 15),  // INVTYPE_RANGED, which nobody draws
                    ],
                )]),
            ))),
            ..Board::default()
        }))
    }

    fn state(board: &Held, queue: &Queue) -> mlua::Lua {
        let lua = mlua::Lua::new();
        register(&lua, board, queue).expect("registers");
        lua
    }

    fn queue() -> Queue {
        Rc::new(RefCell::new(Vec::new()))
    }

    /// **The two enumerators hand back flat pairs**, which is what
    /// `CharacterCreateEnumerateRaces`' `arg.n/2` counts — and the second of
    /// each pair is the *file* string, which is the icon's key.
    #[test]
    fn the_race_list_is_name_and_file_string_in_button_order() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        let flat: Vec<String> = lua
            .load("local t = {GetAvailableRaces()}; return t")
            .eval()
            .expect("the pairs");
        assert_eq!(flat.len() % 2, 0, "arg.n/2 is the race count");
        assert_eq!(flat[0], "Human");
        assert_eq!(flat[1], "Human", "the file string, not the printed name");
        // Alliance before Horde, which is `assets::charcreate`'s rule reaching
        // the screen in the right order.
        assert_eq!(flat[flat.len() - 1], "Orc");
    }

    /// **`SetSelectedRace` moves everything the next line asks about**, which is
    /// the ordering `CharacterRace_OnClick` depends on: the class list, the
    /// faction and the name all describe the race just chosen, in the same call.
    #[test]
    fn choosing_a_race_is_visible_to_the_very_next_read() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        let (side, name, first_class): (String, String, String) = lua
            .load(
                r#"SetSelectedRace(3)
                   local _, side = GetFactionForRace()
                   local race = GetNameForRace()
                   local c = {GetClassesForRace()}
                   return side, race, c[1]"#,
            )
            .eval()
            .expect("the three reads");
        assert_eq!(side, "Horde");
        assert_eq!(name, "Orc");
        assert_eq!(first_class, "Warrior");
        assert_eq!(board.borrow().race_id(), 2, "…and the wire's own number");
    }

    /// **The three selections are one-based and none of them is a wire value.**
    /// A client that sent `GetSelectedRace()` straight would make a dwarf when
    /// the player picked an orc — accepted by the server, and wrong.
    #[test]
    fn the_interfaces_indices_and_the_wires_ids_are_different_numbers() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        let (race, sex): (i64, i64) = lua
            .load("SetSelectedRace(2); SetSelectedSex(2); return GetSelectedRace(), GetSelectedSex()")
            .eval()
            .expect("the two");
        assert_eq!((race, sex), (2, 2), "one-based, and 2 is female");
        let held = board.borrow();
        assert_eq!(held.race_id(), 3, "index 2 is the dwarf");
        assert_eq!(held.gender, 1, "…and the wire's gender is zero-based");
    }

    /// **A shorter list under a held index is clamped rather than left
    /// dangling** — the failure it prevents is silent, because an index past the
    /// end answers appearance byte 0 and draws a perfectly plausible character.
    #[test]
    fn changing_race_pulls_every_index_back_inside_its_list() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        // The human has three skins in the fixture and the orc one.
        lua.load("SetSelectedRace(1); CycleCharCustomization(1, 2)")
            .exec()
            .expect("cycled");
        assert_eq!(board.borrow().picks[0], 2);
        lua.load("SetSelectedRace(3)").exec().expect("the orc");
        assert_eq!(board.borrow().picks[0], 0, "one skin, so index 0");
        assert_eq!(board.borrow().appearance()[0], 7, "…and its own id");
    }

    /// **Cycling wraps in both directions**, which is what the two arrows do —
    /// and `HasCharCustomization` is "more than one", so a single-option axis
    /// draws no arrows at all.
    #[test]
    fn an_axis_wraps_and_a_single_option_axis_says_it_has_none() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        let (many, one): (Option<i64>, Option<i64>) = lua
            .load(
                "SetSelectedRace(1)
                 local many = HasCharCustomization(1)
                 SetSelectedRace(3)
                 return many, HasCharCustomization(1)",
            )
            .eval()
            .expect("the two");
        assert_eq!(many, Some(1), "three skins is more than one");
        assert_eq!(one, None, "one skin is not a choice");

        lua.load("SetSelectedRace(1); CycleCharCustomization(1, -1)")
            .exec()
            .expect("cycled back from zero");
        assert_eq!(board.borrow().picks[0], 2, "wrapped to the end");
        lua.load("CycleCharCustomization(1, 1)").exec().expect("on");
        assert_eq!(board.borrow().picks[0], 0, "…and round again");
    }

    /// **The appearance leaves in `PLAYER_BYTES` order**, which is the order the
    /// packet and the character list both use. A transposition here is a
    /// character who looks like somebody else and is refused by nothing.
    #[test]
    fn the_five_axes_become_the_five_bytes_in_the_wires_order() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        lua.load(
            "SetSelectedRace(1)
             CycleCharCustomization(1, 1)   -- skin
             CycleCharCustomization(2, 1)   -- face
             CycleCharCustomization(3, 1)   -- hair style
             CycleCharCustomization(4, 1)   -- hair colour
             CycleCharCustomization(5, 1)   -- facial hair",
        )
        .exec()
        .expect("five arrows");
        let held = board.borrow();
        assert_eq!(held.appearance(), [1, 4, 1, 6, 1]);
        let look = held.look();
        assert_eq!((look.skin, look.face), (1, 4));
        assert_eq!(look.hair_style, 1);
        assert_eq!(look.hair_colour, 6);
        assert_eq!(look.facial_hair, 1);
    }

    /// **A word, not a number** — and the fallback is the client's own `"NONE"`,
    /// which is what hides the fifth axis rather than drawing it blank.
    #[test]
    fn the_two_customization_labels_are_the_races_own_words() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        let (hair, male, female): (String, String, String) = lua
            .load(
                "SetSelectedRace(1); SetSelectedSex(1)
                 local hair, male = GetHairCustomization(), GetFacialHairCustomization()
                 SetSelectedSex(2)
                 return hair, male, GetFacialHairCustomization()",
            )
            .eval()
            .expect("the three words");
        assert_eq!(hair, "NORMAL");
        assert_eq!(male, "NORMAL");
        assert_eq!(female, "PIERCINGS", "the gender indexes the pair");

        // …and a board with no tables at all, which is the state before the
        // archives are open.
        let empty: Held = Rc::new(RefCell::new(Board::default()));
        let lua = state(&empty, &queue);
        let word: String = lua.load("return GetHairCustomization()").eval().unwrap();
        assert_eq!(word, "NONE");
    }

    /// **Accept records the name and a blank one records nothing**, on the same
    /// terms as `DefaultServerLogin` — the server's answer to an empty name is a
    /// round trip to be told what the empty box already said.
    #[test]
    fn the_accept_button_records_a_name_and_only_a_name() {
        let (board, queue) = (board(), queue());
        let lua = state(&board, &queue);
        lua.load(r#"CreateCharacter("   "); CreateCharacter("Alden")"#)
            .exec()
            .expect("two presses");
        assert_eq!(
            *queue.borrow(),
            vec![CreateRequest::Create("Alden".into())]
        );
    }

    /// **[`GLOBALS`] plus [`MORE_GLOBALS`] is what [`register`] really
    /// installs** — the same check `lua::glue` keeps, and for the same reason:
    /// `vale framexml` subtracts these names from the ones the directory
    /// calls, and a list that has drifted reports work nobody did.
    #[test]
    fn the_registered_set_is_the_list() {
        let lua = mlua::Lua::new();
        let before: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();
        register(&lua, &board(), &queue()).expect("registers");
        let after: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();

        let mut added: Vec<String> = after.difference(&before).cloned().collect();
        added.sort();
        let mut claimed: Vec<String> = GLOBALS
            .iter()
            .chain(MORE_GLOBALS.iter())
            .map(|n| (*n).to_string())
            .collect();
        claimed.sort();
        assert_eq!(added, claimed, "register and GLOBALS disagree");

        let mut sorted = GLOBALS;
        sorted.sort_unstable();
        assert_eq!(sorted, GLOBALS, "GLOBALS is kept sorted");
    }
}
