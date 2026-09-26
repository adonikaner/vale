//! Readers for the game's `Interface\` files and the interface rules the client
//! holds.
//!
//! `crates/client/src/lua/` runs the interface; this module only reads and
//! describes it.
//!
//! ```text
//! toc.rs       the interface's load list and load order, which cannot be
//!              reconstructed from anything else in FrameXML
//! xml.rs       the markup: the element tree of the ninety files
//! widgets.rs   the classification of the 105 element names: widget, region,
//!              font, handler, structure
//! backdrop.rs  the layout of a <Backdrop> edge file
//! font.rs      text metrics for the game's four typefaces
//! strings.rs   GlobalStrings.lua, 4,592 keys
//! messages.rs  the 343-record message table: which string the client shows
//!              where, with its surface, sound and speech line
//! chattype.rs  the 94 chat types, name and colour, indexed by the id the
//!              client's routing returns
//! combatlog.rs the combat log rules: which sentence a blow produces, whose
//!              names go in it, and which chat type it goes to
//! cvars.rs     the 200 CVar defaults every options panel is written against
//! bindings.rs  Bindings.xml's 234 declarations, the 229 rows a key-bindings
//!              panel lists, and the `bind` file format used by the shipped
//!              defaults and the player's own file
//! keys.rs      key names: what a key may be called and what the client
//!              refuses, from rules in the client rather than any file
//! uimodel.rs   the widget whose contents are a model
//! whoquery.rs  the `/who` query parser: what `/who z-"Elwynn Forest" 5-10`
//!              means
//! chatcache.rs the character's chat-cache.txt: which channels it is in, and
//!              what each window shows
//! wtf.rs       `WTF\Config.wtf`, the settings kept between sessions, and
//!              `realmlist.wtf`, one `set` line for one of them
//! addons.rs    `Interface\AddOns\` on disk: each addon's `.toc` directives,
//!              the per-character AddOns.txt, why an addon does not load, and
//!              the load order
//! ```

pub mod addons;
pub mod backdrop;
pub mod bindings;
pub mod chattype;
pub mod combatlog;
pub mod cvars;
pub mod font;
pub mod keys;
pub mod messages;
pub mod strings;
pub mod toc;
pub mod uimodel;
pub mod widgets;
pub mod chatcache;
pub mod whoquery;
pub mod wtf;
pub mod xml;
