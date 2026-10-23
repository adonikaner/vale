//! The windows and workspaces of the server-row subjects: the views of
//! `crate::subjects`, one file per subject under the same stem.
//!
//! ```text
//! items.rs      the workspace item_template is edited in: that form, a
//!               browser, and a third part in the inspector: a picture of what
//!               the display id a row names looks like, and the grid of every
//!               appearance it is chosen from
//! quests.rs     the workspace quest_template is edited in: the list, a form
//!               whose objectives and rewards are lines rather than numbered
//!               columns, the quest as the log would show it, who gives and
//!               takes it, and the window the creature tool opens on one
//!               creature's quests
//! templates.rs  the Creatures and Game Objects workspaces: every
//!               creature_template or gameobject_template row as a list, the
//!               open row in the template window's form, and its model and
//!               other tables in the inspector, written once over the spawn
//!               subject's view
//! listing.rs    the parts of the workspaces that are the same: the docked
//!               list, its search box, and the form's head
//! behaviour.rs  the three windows a creature's behaviour is edited in: its
//!               events, the spell list its template names, and one script,
//!               with every parameter named by the event's type, the row's
//!               command or the target
//! loot.rs       the window a loot set is edited in, opened from a selected
//!               creature, a selected game object or the open item: its rows
//!               by group, each drawn as the server reads it, and the
//!               reference sets they name, followed and edited in place
//! services.rs   the Vendor and Trainer windows of a selected creature: its
//!               own list and its template's, each row drawn as the server
//!               reads it, with the reasons it would skip one
//! gossip.rs     the gossip window: a creature's menu, what it says and the
//!               options it offers, each leading to the next menu
//! conditions.rs the condition window: one row of `conditions`, its values
//!               named by its type, opened from any form that names one
//! displays.rs   the dialog a creature's or a game object's display id is
//!               chosen in: a paged grid of rendered models, searched by
//!               model path, with a large preview
//! windows.rs    the windows that follow a creature's or a game object's
//!               template, opened from its tool or its workspace: quests,
//!               loot, behaviour, vendor, trainer, gossip and conditions
//! ```

pub mod behaviour;
pub mod conditions;
pub mod displays;
pub mod gossip;
pub mod items;
pub mod listing;
pub mod loot;
pub mod quests;
pub mod services;
pub mod templates;
pub mod windows;
