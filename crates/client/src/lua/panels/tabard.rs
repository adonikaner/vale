//! The tabard designer: two C functions and the ten methods of
//! `TabardModel`.
//!
//! ```text
//! GetTabardCreationCost()           copper, for the designer's money frame
//! CloseTabardCreation()             the designer's window was hidden
//!
//! TabardModel:InitializeTabardColors()
//! TabardModel:CycleVariation(id, delta)
//! TabardModel:GetUpperEmblemTexture(texture)
//! TabardModel:GetLowerEmblemTexture(texture)
//! TabardModel:GetUpperEmblemFileName()   GetLowerEmblemFileName()
//! TabardModel:GetUpperBackgroundFileName()  GetLowerBackgroundFileName()
//! TabardModel:CanSaveTabardNow()
//! TabardModel:Save()
//! ```
//!
//! [`vale_protocol::play::guild`] has the two packets,
//! [`vale_assets::look::emblem`] has the files a design names, and
//! [`crate::interface::tabard`] is the systems that fill this board and send
//! what it queues.
//!
//! ## The design is five numbers on the board
//!
//! The window's five rows are the five numbers of an emblem, in wire order:
//! emblem, emblem colour, border, border colour, background. A row's arrows
//! call `CycleVariation(row, -1 or 1)`, which moves that number and wraps it
//! at its count. `InitializeTabardColors` starts the design at the guild's
//! saved emblem, or at a random one for a guild that has none.
//!
//! `TabardFrame_UpdateTextures` reads the design back in the same handler
//! that cycled it, so the design is held here and not in the ECS.
//! [`Tabard::design_version`] tells [`crate::interface::tabard`] that the
//! model's preview has to be drawn again.
//!
//! ## The two emblem textures are masks
//!
//! `GetUpperEmblemTexture` and `GetLowerEmblemTexture` fill the four
//! textures behind the model. The 1.12.1 client draws the emblem's shape
//! there in white, whatever colour the design has: every texel is white with
//! the emblem file's alpha. The texture is given the emblem's path behind
//! [`ALPHA_MASK_PREFIX`], which the interface's art cache reads as that
//! instruction.
//!
//! ## What `Save` checks before it sends
//!
//! In this order, each refusal printing its sentence and sending nothing: a
//! number outside its count, no guild record held, a design equal to the
//! guild's saved emblem, a rank that is not the guild master's, and less
//! money than the cost. The server checks the guild, the rank and the money
//! again.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::ObjectLike;
use vale_assets::look::emblem;
use vale_protocol::play::guild::{Emblem, EMBLEM_COST, EMBLEM_COUNTS};

/// The unscoped functions this file registers, sorted.
pub const VERBS: [&str; 2] = ["CloseTabardCreation", "GetTabardCreationCost"];

/// The widget methods this file registers, sorted.
pub const METHODS: [&str; 10] = [
    "CanSaveTabardNow",
    "CycleVariation",
    "GetLowerBackgroundFileName",
    "GetLowerEmblemFileName",
    "GetLowerEmblemTexture",
    "GetUpperBackgroundFileName",
    "GetUpperEmblemFileName",
    "GetUpperEmblemTexture",
    "InitializeTabardColors",
    "Save",
];

/// A texture path with this prefix is drawn as a mask: the file after the
/// prefix, with every texel made white and its alpha kept.
pub const ALPHA_MASK_PREFIX: &str = "vale-mask:";

/// A press the interface made, for [`crate::interface::tabard`] to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// `CloseTabardCreation()`.
    Close,
    /// `TabardModel:Save()`, with a design that passed [`Tabard::save_fault`].
    Save(Emblem),
    /// A refusal the client states itself, as a `GlobalStrings.lua` key.
    Refuse(&'static str),
}

/// The designer's state, shared between the interpreter and the ECS.
#[derive(Default)]
pub struct Tabard {
    /// The tabard designer the window is open at.
    pub vendor: Option<u64>,
    /// The design the five rows hold.
    pub design: Emblem,
    /// A save was sent and has not been answered.
    pub pending: bool,
    /// The saved emblem of the character's guild, or `None` while the
    /// guild's query has not answered and for a character in no guild.
    pub guild: Option<Emblem>,
    /// The character's guild rank, 0 for the guild master, and its money in
    /// copper. `Save` checks both.
    pub rank: u32,
    pub money: u32,
    /// The state of the generator `InitializeTabardColors` draws from.
    random: u64,
    /// Incremented when the design changes.
    pub design_version: u32,
}

impl Tabard {
    /// Start the design at the guild's saved emblem. A guild with no record
    /// held, or with an emblem that has a number outside its count, starts
    /// at a random design.
    pub fn initialize(&mut self) {
        self.design = match self.guild.filter(|emblem| emblem.in_range()) {
            Some(emblem) => emblem,
            None => {
                let mut fields = [0i32; 5];
                for (value, count) in fields.iter_mut().zip(EMBLEM_COUNTS) {
                    *value = (self.next_random() % count as u64) as i32;
                }
                Emblem::from_fields(fields)
            }
        };
        self.design_version = self.design_version.wrapping_add(1);
    }

    /// A 64-bit xorshift step, seeded from the clock on first use.
    fn next_random(&mut self) -> u64 {
        if self.random == 0 {
            self.random = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0x9e37_79b9_7f4a_7c15, |since| since.as_nanos() as u64)
                | 1;
        }
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        self.random
    }

    /// `CycleVariation(row, delta)`: move one number and wrap it at its
    /// count. A row outside 1..=5 and a step as large as the count change
    /// nothing.
    pub fn cycle(&mut self, row: i64, delta: i64) {
        let Some(index) = usize::try_from(row - 1).ok().filter(|i| *i < 5) else {
            return;
        };
        let count = i64::from(EMBLEM_COUNTS[index]);
        if delta.abs() >= count {
            return;
        }
        let mut fields = self.design.fields();
        fields[index] = ((i64::from(fields[index]) + count + delta).rem_euclid(count)) as i32;
        self.design = Emblem::from_fields(fields);
        self.design_version = self.design_version.wrapping_add(1);
    }

    /// `CanSaveTabardNow`: the guild's record is held and no save is
    /// waiting for its answer. The rank and the money are not part of it.
    pub fn can_save(&self) -> bool {
        self.guild.is_some() && !self.pending
    }

    /// The key of the refusal `Save` prints for this design, or `None` for a
    /// design that is sent.
    pub fn save_fault(&self) -> Option<&'static str> {
        if !self.design.in_range() {
            return Some("ERR_GUILDEMBLEM_INVALID_TABARD_COLORS");
        }
        let Some(saved) = self.guild else {
            return Some("ERR_GUILDEMBLEM_NOGUILD");
        };
        if saved == self.design {
            Some("ERR_GUILDEMBLEM_SAME")
        } else if self.rank != 0 {
            Some("ERR_GUILDEMBLEM_NOTGUILDMASTER")
        } else if self.money < EMBLEM_COST {
            Some("ERR_NOT_ENOUGH_MONEY")
        } else {
            None
        }
    }
}

pub type Held = Rc<RefCell<Tabard>>;
pub type Queue = Rc<RefCell<Vec<Press>>>;

/// The board as the widget methods find it: in the interpreter's app data.
struct Shared {
    held: Held,
    queue: Queue,
}

/// Register the two functions, and keep the board where the methods find it.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();
    lua.set_app_data(Shared {
        held: Rc::clone(held),
        queue: Rc::clone(queue),
    });
    // Ten gold. The cost is the client's own constant: no packet carries it.
    globals.set(
        "GetTabardCreationCost",
        lua.create_function(|_, ()| Ok(EMBLEM_COST))?,
    )?;
    let send = Rc::clone(queue);
    globals.set(
        "CloseTabardCreation",
        lua.create_function(move |_, ()| {
            send.borrow_mut().push(Press::Close);
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Install the ten methods on the widget method table. Every widget kind
/// shares that table; `TabardModel` is the one frame that calls these.
pub(in crate::lua) fn install_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // Run `body` over the board. A host built without the board answers the
    // default, which no shipped file can reach.
    fn with<T: Default>(lua: &mlua::Lua, body: impl FnOnce(&Shared) -> T) -> T {
        lua.app_data_ref::<Shared>().map(|shared| body(&shared)).unwrap_or_default()
    }

    methods.set(
        "InitializeTabardColors",
        lua.create_function(|lua, _this: mlua::Table| {
            with(lua, |shared| shared.held.borrow_mut().initialize());
            Ok(())
        })?,
    )?;
    methods.set(
        "CycleVariation",
        lua.create_function(
            |lua, (_this, row, delta): (mlua::Table, Option<i64>, Option<i64>)| {
                with(lua, |shared| {
                    shared
                        .held
                        .borrow_mut()
                        .cycle(row.unwrap_or(0), delta.unwrap_or(0));
                });
                Ok(())
            },
        )?,
    )?;
    methods.set(
        "CanSaveTabardNow",
        lua.create_function(|lua, _this: mlua::Table| {
            Ok(with(lua, |shared| shared.held.borrow().can_save()).then_some(1))
        })?,
    )?;
    methods.set(
        "Save",
        lua.create_function(|lua, _this: mlua::Table| {
            with(lua, |shared| {
                let mut board = shared.held.borrow_mut();
                let press = match board.save_fault() {
                    Some(key) => Press::Refuse(key),
                    None => {
                        board.pending = true;
                        Press::Save(board.design)
                    }
                };
                shared.queue.borrow_mut().push(press);
            });
            Ok(())
        })?,
    )?;

    // The four file names, without an extension.
    for (name, upper, background) in [
        ("GetUpperEmblemFileName", true, false),
        ("GetLowerEmblemFileName", false, false),
        ("GetUpperBackgroundFileName", true, true),
        ("GetLowerBackgroundFileName", false, true),
    ] {
        methods.set(
            name,
            lua.create_function(move |lua, _this: mlua::Table| {
                let design = with(lua, |shared| shared.held.borrow().design.fields());
                Ok(if background {
                    emblem::background_file(design, upper)
                } else {
                    emblem::emblem_file(design, upper)
                })
            })?,
        )?;
    }

    // Point a texture at the emblem's shape. See the module note.
    for (name, upper) in [("GetUpperEmblemTexture", true), ("GetLowerEmblemTexture", false)] {
        methods.set(
            name,
            lua.create_function(move |lua, (_this, texture): (mlua::Table, Option<mlua::Table>)| {
                let Some(texture) = texture else {
                    return Ok(());
                };
                let design = with(lua, |shared| shared.held.borrow().design.fields());
                let path = format!("{ALPHA_MASK_PREFIX}{}", emblem::emblem_file(design, upper));
                texture.call_method::<()>("SetTexture", path)
            })?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Emblem {
        Emblem::from_fields([3, 4, 5, 6, 7])
    }

    /// The counts the protocol validates a save against are the counts of
    /// the files the design names.
    #[test]
    fn the_two_crates_agree_on_the_counts() {
        let files: Vec<i32> = emblem::COUNTS.iter().map(|c| *c as i32).collect();
        assert_eq!(files, EMBLEM_COUNTS);
    }

    /// A guild with a saved emblem starts the design there, and a guild
    /// without one starts at a design inside the counts.
    #[test]
    fn the_design_starts_at_the_guilds_emblem_or_a_random_one() {
        let mut board = Tabard { guild: Some(saved()), ..Tabard::default() };
        board.initialize();
        assert_eq!(board.design, saved());
        for guild in [None, Some(Emblem::NONE)] {
            let mut board = Tabard { guild, ..Tabard::default() };
            board.initialize();
            assert!(board.design.in_range(), "{:?}", board.design);
        }
    }

    /// A row's number wraps at its count in both directions, and a row that
    /// does not exist changes nothing.
    #[test]
    fn a_row_wraps_at_its_count() {
        let mut board = Tabard::default();
        board.cycle(1, -1);
        assert_eq!(board.design.style, 169);
        board.cycle(1, 1);
        assert_eq!(board.design.style, 0);
        board.cycle(3, 1);
        board.cycle(3, 5);
        assert_eq!(board.design.border_style, 0, "six borders");
        board.cycle(5, -1);
        assert_eq!(board.design.background, 50);
        let before = board.design;
        board.cycle(0, 1);
        board.cycle(6, 1);
        board.cycle(2, 17);
        assert_eq!(board.design, before);
    }

    /// The refusals are tested in the client's order, and the first that
    /// applies is the one printed.
    #[test]
    fn save_refuses_in_order() {
        let mut board = Tabard::default();
        assert_eq!(board.save_fault(), Some("ERR_GUILDEMBLEM_NOGUILD"));
        board.guild = Some(saved());
        board.design = saved();
        assert_eq!(board.save_fault(), Some("ERR_GUILDEMBLEM_SAME"));
        board.design = Emblem::from_fields([9, 4, 5, 6, 7]);
        board.rank = 2;
        assert_eq!(board.save_fault(), Some("ERR_GUILDEMBLEM_NOTGUILDMASTER"));
        board.rank = 0;
        assert_eq!(board.save_fault(), Some("ERR_NOT_ENOUGH_MONEY"));
        board.money = EMBLEM_COST;
        assert_eq!(board.save_fault(), None);
        board.design = Emblem::from_fields([170, 0, 0, 0, 0]);
        assert_eq!(board.save_fault(), Some("ERR_GUILDEMBLEM_INVALID_TABARD_COLORS"));
        assert!(board.can_save(), "the rank and the money are not part of it");
        board.pending = true;
        assert!(!board.can_save());
    }

    /// The methods reach the board through the interpreter: a cycle is read
    /// back by the file name in the same chunk, and a save that passes is
    /// queued and marks the board as waiting.
    #[test]
    fn the_methods_read_and_write_the_board() {
        let lua = mlua::Lua::new();
        let held: Held = Rc::default();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &held, &queue).expect("registers");
        let methods = lua.create_table().expect("table");
        install_methods(&lua, &methods).expect("installs");
        lua.globals().set("M", methods).expect("set");
        {
            let mut board = held.borrow_mut();
            board.guild = Some(saved());
            board.money = EMBLEM_COST;
            board.initialize();
        }
        let (file, can, cost): (String, i64, i64) = lua
            .load(
                r#"
                local model = {}
                M.CycleVariation(model, 1, 1)
                local texture = { SetTexture = function(self, path) shown = path end }
                M.GetUpperEmblemTexture(model, texture)
                return M.GetLowerEmblemFileName(model), M.CanSaveTabardNow(model), GetTabardCreationCost()
                "#,
            )
            .eval()
            .expect("values");
        assert_eq!(file, r"Textures\GuildEmblems\Emblem_04_04_TL_U");
        assert_eq!((can, cost), (1, 100_000));
        let shown: String = lua.globals().get("shown").expect("set by the texture");
        assert_eq!(shown, r"vale-mask:Textures\GuildEmblems\Emblem_04_04_TU_U");
        lua.load("M.Save({}); CloseTabardCreation()").exec().expect("runs");
        assert_eq!(
            queue.borrow().as_slice(),
            [Press::Save(Emblem::from_fields([4, 4, 5, 6, 7])), Press::Close]
        );
        assert!(held.borrow().pending);
    }
}
