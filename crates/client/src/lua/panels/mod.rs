//! The C functions each panel of the game's interface calls, one file per
//! panel.
//!
//! These are the parts of [`super::api`] that serve one subject rather than
//! the interpreter: `ContainerAnswers` answers what `ContainerFrame.lua` asks,
//! `TaxiAnswers` answers what `TaxiFrame.lua` asks, and the `Answers` trait
//! this file defines combines all of them. Each read is declared, answered,
//! registered and listed in the file named after its subject.
//!
//! The `panels!` list at the end of this file names every panel once: its
//! subject trait, whether it has scoped reads, and whether it has unscoped
//! functions. It generates `Answers`, [`install`] and [`register`]. A panel
//! keeps its queue and its board in the interpreter's app data (see
//! [`stored`]) and hands them to the systems that drain or fill them through
//! its own `take` and `held`, so adding a panel edits neither
//! [`super::host`] nor [`super::api`].
//!
//! ```text
//! container.rs   the bags: fourteen reads, an item link, and item dragging
//! spellbook.rs   the spellbook, in the client's page order
//! paperdoll.rs   the character sheet's numbers: stats, resistances, armour,
//!                attack and defence
//! auras.rs       the buff bar, and the aura rows of other units
//! quest.rs       the quest log, and a quest giver's pages
//! gossip.rs      talking to an NPC, and the "npc" unit token the other NPC
//!                panels use
//! merchant.rs    the merchant window
//! mail.rs        the mailbox, which belongs to no NPC: the inbox, one letter,
//!                and the letter being written
//! trainer.rs     the training window, a load-on-demand addon
//! tradeskill.rs  the profession window, another load-on-demand addon, mostly
//!                a rule over SkillLineAbility.dbc
//! craft.rs       the Enchanting window, the craft counterpart of tradeskill.rs
//! talent.rs      the talent trees, another load-on-demand addon, mostly a rule
//!                over two DBCs
//! keybindings.rs the key bindings panel, a fourth load-on-demand addon: nine
//!                C functions over the key table the keyboard also reads
//! taxi.rs        the flight map, whose background is painted from a table
//! pagetext.rs    text read from an object: a sign, a plaque, or a book on a
//!                stand; like the mailbox, a window no NPC owns
//! duel.rs        the four script functions a duel uses, all of them writes
//! summon.rs      the summon popup's three reads
//! worldstate.rs  the world state frame above the minimap: how many lines,
//!                and what each says
//! tutorial.rs    whether tutorial tips are on
//! raidtarget.rs  which raid target icon a unit carries
//! questshare.rs  whether the selected quest may be shared
//! uioptions.rs   the options panel's ShowingHelm and ShowingCloak
//! inspect.rs     the inspect window's reads: who may be inspected, and the
//!                honor tab
//! loot.rs        the loot window of a corpse
//! lootroll.rs    the four group-loot roll frames
//! reputation.rs  the reputation panel, which has two sort orders
//! skills.rs      the skills panel, which has the same shape as reputation.rs
//! party.rs       the party
//! social.rs      the friends list, the ignore list and the /who search
//! guild.rs       the guild tab: the roster, the ranks and their rights, and
//!                the guild names `GetGuildInfo` answers from
//! petition.rs    the guild charter: the registrar window's four functions
//!                and the petition window's eight
//! tabard.rs      the tabard designer: two functions and the ten methods of
//!                `TabardModel`
//! channels.rs    chat channels: ten numbered slots and the chat frame's
//!                twenty-five channel globals
//! raid.rs        the raid roster: the party roster with its first byte set,
//!                a separate panel, and eight reads the party does not use
//! pet.rs         the three predicates the pet frame and its menu depend on
//! stable.rs      the stable master: seven reads over one packet, four commands
//!                and a DBC of prices
//! bank.rs        the bank: three reads and two commands; the thirty bank
//!                slots are read the same way as the paper doll's
//! trade.rs       the trade window between two players: six reads over two
//!                packets, ten commands
//! shapeshift.rs  the stance bar beside the pet bar: it shares the pet bar's
//!                unbound commands but none of its packets; the stance
//!                buttons come from the client's reading of Spell.dbc, with
//!                nothing on the wire
//! worldmap.rs    the world map
//! charcreate.rs  the character creation screen
//! glue.rs        the login and character select screens
//! addons.rs      the list behind character select's AddOns button: every
//!                addon in the folder, which ones each character has enabled,
//!                and the loader `LoadAddOn` runs
//! ```
pub(crate) mod addons;
pub mod auras;
pub mod bank;
pub mod channels;
pub(crate) mod charcreate;
pub(crate) mod container;
pub(crate) mod craft;
pub mod glue;
pub mod gossip;
pub(crate) mod keybindings;
pub mod loot;
pub mod mail;
pub(crate) mod pagetext;
pub mod duel;
pub(crate) mod summon;
pub(crate) mod worldstate;
pub(crate) mod tutorial;
pub(crate) mod raidtarget;
pub(crate) mod questshare;
pub(crate) mod uioptions;
pub(crate) mod guild;
pub mod inspect;
pub(crate) mod lootroll;
pub(crate) mod merchant;
pub(crate) mod paperdoll;
pub(crate) mod party;
pub mod pet;
pub(crate) mod petition;
pub mod quest;
pub mod raid;
pub mod reputation;
pub(crate) mod shapeshift;
pub mod skills;
pub(crate) mod stable;
pub(crate) mod tabard;
pub mod trade;
pub(crate) mod social;
pub mod spellbook;
pub mod talent;
pub mod taxi;
pub(crate) mod tradeskill;
pub mod trainer;
pub(crate) mod worldmap;

/// The value of type `T` in the state's app data, inserted as `T::default()`
/// if there is none yet, and cloned out.
///
/// Each panel keeps its queue and its board here rather than as a field of
/// [`super::host::LuaHost`]: `register` fetches it, the functions it
/// registers capture it, and the panel's own `take` or `held` hands it to the
/// systems that drain or fill it. `T` is an `Rc`, so the clone is the same
/// queue. App data is keyed by type, and every panel's queue and board is a
/// distinct type, so no two panels share one.
pub(in crate::lua) fn stored<T: Default + Clone + 'static>(lua: &mlua::Lua) -> T {
    if let Some(value) = lua.app_data_ref::<T>() {
        return value.clone();
    }
    let value = T::default();
    lua.set_app_data(value.clone());
    value
}

/// Takes and clears a panel's queue of writes. See [`stored`].
pub(in crate::lua) fn taken<T: 'static>(host: &super::host::LuaHost) -> Vec<T> {
    std::mem::take(&mut *stored::<std::rc::Rc<std::cell::RefCell<Vec<T>>>>(host.state()).borrow_mut())
}

/// One scoped step of [`install`]: a panel's `install`, with the verb queue
/// for the panels whose reads also record.
macro_rules! scoped {
    (install $stem:ident, $lua:ident, $scope:ident, $answers:ident, $queue:ident) => {
        $stem::install($lua, $scope, $answers)?;
    };
    (install_with_queue $stem:ident, $lua:ident, $scope:ident, $answers:ident, $queue:ident) => {
        $stem::install($lua, $scope, $answers, $queue)?;
    };
    (register $($rest:tt)*) => {};
}

/// One unscoped step of [`register`]: a panel's `register`.
macro_rules! unscoped {
    (register $stem:ident, $lua:ident) => {
        $stem::register($lua)?;
    };
    ($other:ident $($rest:tt)*) => {};
}

/// Declares every panel once: its subject trait, if it has one, and which of
/// the two registrations it takes part in.
///
/// An entry is `stem (Trait) roles;`. `Trait` is the panel's subject trait,
/// which [`Answers`] requires; `()` is a panel with no reads of its own.
/// The roles are `install` (scoped reads, registered by [`install`] for the
/// length of each call), `install_with_queue` (the same, for the four panels
/// whose reads also record onto the verb queue) and `register` (unscoped
/// functions, registered once by [`register`]). Both sequences run in list
/// order. No two panels register the same global name, so the order between
/// panels does not decide which function a name holds.
macro_rules! panels {
    ( $( $stem:ident ( $( $answers:ident )? ) $( $role:ident )* ; )* ) => {
        /// Everything the interface may ask, as one name: the sum of the
        /// subject traits. No method is declared here.
        ///
        /// Each subject declares its own trait beside its Lua registration, so
        /// a feature's declaration, answer, registration and `READS` entry are
        /// in the file the feature is named after. `&dyn Answers` resolves
        /// every method of every subject trait, because a trait object carries
        /// its supertraits' methods.
        ///
        /// [`super::api::UnitAnswers`] and [`super::api::ActionAnswers`] are
        /// the two subjects with no panel; the rest come from the `panels!`
        /// list. Every subject-trait method has a default that answers
        /// "absent", so a stand-in world implements only what it is used for.
        pub trait Answers:
            super::api::UnitAnswers + super::api::ActionAnswers $( $( + $stem::$answers )? )*
        {
        }

        /// Any type that implements every subject trait is an `Answers`.
        impl<T> Answers for T where
            T: super::api::UnitAnswers + super::api::ActionAnswers $( $( + $stem::$answers )? )*
        {
        }

        /// Registers every panel's scoped reads, for the length of one call
        /// into Lua. Called by [`super::api::install`].
        pub(in crate::lua) fn install<'scope, 'env: 'scope>(
            lua: &mlua::Lua,
            scope: &'scope mlua::Scope<'scope, 'env>,
            answers: &'env dyn Answers,
            // The verb queue, for the reads that also record. See
            // [`container`].
            queue: &'env super::api::verbs::Queue,
        ) -> mlua::Result<()> {
            $( $( scoped!($role $stem, lua, scope, answers, queue); )* )*
            Ok(())
        }

        /// Registers every panel's unscoped functions, once, and creates each
        /// panel's state in the app data. Called by
        /// [`super::host::LuaHost::new`].
        pub(in crate::lua) fn register(lua: &mlua::Lua) -> mlua::Result<()> {
            $( $( unscoped!($role $stem, lua); )* )*
            Ok(())
        }
    };
}

// Each panel's reads live in that panel's own module, one subject per module,
// as elsewhere in this directory. The list order is the order both
// registrations run in.
panels! {
    // The spellbook's seven reads.
    spellbook (SpellbookAnswers) install;
    auras (AuraAnswers) install;
    // The character sheet's fourteen reads.
    paperdoll () install;
    container (ContainerAnswers) install_with_queue;
    // The loot window's two verbs, drained by `crate::interface::loot`.
    loot (LootAnswers) install register;
    // The roll frame's two verbs, drained by `crate::interface::lootroll`.
    lootroll (LootRollAnswers) install register;
    // The quest panel's verbs, drained by `crate::interface::quest`.
    quest (QuestAnswers) install register;
    // The gossip and merchant windows' verbs, drained by their own modules.
    gossip (GossipAnswers) install register;
    merchant (MerchantAnswers) install_with_queue register;
    // The mail window's ten verbs, drained by `crate::interface::mail`.
    mail (MailAnswers) install register;
    // The trainer's seven verbs, drained by `crate::interface::trainer`.
    trainer (TrainerAnswers) install register;
    // The stable master's seven reads and four verbs. One of the reads aims a
    // `<PlayerModel>` and is scoped for that reason; see [`stable`]. The verbs
    // are drained by `crate::interface::stable`.
    stable (StableAnswers) install register;
    // The bank's three reads and two verbs. Without the reads the bank's
    // thirty buttons would read the paper doll's; see [`bank`]. The verbs are
    // drained by `crate::interface::bank`.
    bank (BankAnswers) install register;
    // The six reads a sign, a plaque or a book makes, and the page window's
    // three verbs, drained by `crate::interface::pagetext`. This is the one
    // window in the directory whose subject is an object; see [`pagetext`].
    pagetext (PageTextAnswers) install register;
    // The summon popup's three reads.
    summon (SummonAnswers) install;
    // The world state frame's two reads, the tips panel's one, the raid target
    // icon's one and the quest log's sharing test.
    worldstate (WorldStateAnswers) install;
    tutorial (TutorialAnswers) install;
    raidtarget (RaidTargetAnswers) install;
    questshare (QuestShareAnswers) install;
    // The options panel's two reads that are not CVars.
    uioptions (OptionsAnswers) install;
    // The inspect window's four reads, and its three verbs, drained by
    // `crate::interface::inspect`.
    inspect (InspectAnswers) install register;
    // `GetGuildInfo`, which reads a unit's guild fields, and the guild's
    // forty-four unscoped functions. `crate::interface::guild` fills the board
    // and drains the requests.
    guild (GuildAnswers) install register;
    // The trade window's ten verbs, drained by `crate::interface::trade`.
    trade (TradeAnswers) install register;
    // The two profession windows. Their create buttons push onto the same
    // queue the bags' drag uses; see [`tradeskill`].
    tradeskill (TradeSkillAnswers) install_with_queue;
    craft (CraftAnswers) install_with_queue;
    // The talent panel's one verb. See [`talent`] for the five reads beside it
    // and why they are scoped.
    talent (TalentAnswers) install register;
    // The flight map's three verbs, drained by `crate::interface::taxi`.
    taxi (TaxiAnswers) install register;
    // The map's three writes work like the chat verbs: a handler cannot move
    // the view directly, because the world is borrowed for the length of the
    // call, so it records a request and `interface::worldmap` applies it.
    worldmap (MapAnswers) install register;
    // The party reads, and its six verbs, drained by
    // `crate::interface::party`. The party is the one unit subject whose
    // members may not be in the world at all; see [`party`].
    party (PartyAnswers) install register;
    // The raid's eight reads, which ask different questions of the same
    // roster, and its nine verbs, drained by `crate::interface::raid`. See
    // [`raid`].
    raid (RaidAnswers) install register;
    // Group members' pets. Their tokens are derived from another unit's fields
    // rather than held; see [`pet`].
    pet (PetAnswers) install;
    shapeshift (ShapeshiftAnswers) install;
    // The two screens shown before there is a world. Their reads are about a
    // session that has not started; see [`glue`]. Their functions are drained
    // by `crate::glue::glue`, and are registered unconditionally rather than
    // only while the glue is loaded: nothing in `Interface\FrameXML\` calls
    // them, so they cost nothing there, and registering and removing them with
    // the screen would be a second piece of state to keep in step with the
    // directory swap. `PlayGlueMusic` forwards to `PlayMusic`, so
    // [`super::api::sound`] registers before this list.
    glue (GlueAnswers) install register;
    // The reputation panel's eleven functions. `crate::interface::reputation`
    // fills the board and drains the three packets.
    reputation () register;
    // The social panel's sixteen functions. `crate::interface::social` fills
    // the board and drains the six packets.
    social () register;
    // The guild charter's thirteen functions and the tabard designer's two.
    // `crate::interface::petition` and `crate::interface::tabard` fill the
    // boards and drain the presses.
    petition () register;
    tabard () register;
    // The chat channels' twenty-five functions. `crate::interface::channels`
    // fills the board and drains the two packets.
    channels () register;
    // The skills panel's eleven functions, over a board the same module feeds.
    skills () register;
    // The key bindings panel's eight functions. Their board is also what
    // [`super::host::LuaHost::fire`] resolves a key press through.
    // `RunBinding`, the ninth, is not registered here: it runs a `<Binding>`
    // body, which needs the host rather than the board, and
    // `crate::input::bindings` registers it.
    keybindings () register;
    // The addon list's sixteen functions, over a board `crate::settings::addons`
    // seeds. Unscoped, because `LoadAddOn` runs the loader itself and the
    // loader cannot run inside a scoped read.
    addons () register;
    // The character-create screen's functions. Its reads are unscoped because
    // none of them touches the world: the available choices come entirely from
    // `vale_assets::tables::charcreate`.
    charcreate () register;
    // The duel's four verbs, drained by `crate::interface::duel`.
    duel () register;
}
