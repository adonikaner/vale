//! The flags for the client-table workspace: the row to open, the views and
//! dialogs on it, and a field written while its preview plays.

use super::Arity::*;
use super::{named, number, text, Flag};

/// The table workspace's row and what is opened on it.
#[derive(Debug, Clone, Default)]
pub struct Tables {
    /// `--row <id>`: open the data workspace at that row of its table.
    ///
    /// `--tool spells --row 133` is Fireball's 173 fields. It exists for the
    /// same reason as `--place`: choosing a row is a click in a list of 22,360,
    /// so without it a scripted run cannot photograph the form, which is the
    /// workspace's main content, and its layout goes unchecked.
    ///
    /// The id is the row's own id, not its index: id `133` is Fireball, while
    /// the row at index 133 is a different spell.
    pub row: Option<u32>,
    /// `--table <name>`: which of the browser's tables `--row` names, by the
    /// name the tabs use (`Spell`, `SpellVisual`, `SpellVisualKit`,
    /// `SpellVisualEffectName`). The rail's Spells entry opens on `Spell`, so
    /// without this an effect row cannot be opened from the command line.
    pub table: Option<String>,
    /// `--lab`: open the attachment lab on the effect `--row` names, which is
    /// otherwise a press on the effect's form. Same reason as `--place`.
    pub lab: bool,
    /// `--browse`: open the model browser over the effect `--row` names.
    ///
    /// Same reason as `--place`: the dialog opens from a press on a form that
    /// is itself two presses in, so without this flag a scripted run cannot
    /// photograph its list, its preview pane or its two kept lists.
    pub browse: bool,
    /// `--story`: open the data workspace on the Storyboard view rather than
    /// on the fields.
    ///
    /// Same reason as `--row`: the view is a click on a segmented control, so
    /// without this a scripted run cannot photograph the one view of a spell
    /// that is not a form.
    pub story: bool,
    /// `--seek <seconds>`: put the storyboard's preview at that point of its
    /// timeline and pause it there.
    ///
    /// The cast lasts one second of a four-second loop, so a `--shot` that
    /// waits for the interface takes its picture at an arbitrary phase of the
    /// loop, and usually misses the lit hands and the impact on the target.
    /// This is the scrub bar, from the command line.
    pub seek: Option<f32>,
    /// `--bits <column>`: open the mask dialog on that column of `--row`.
    ///
    /// Same reason as `--server`. The dialog opens from a `…` button beside a
    /// field, which a scripted run cannot press, so without this a scripted run
    /// cannot photograph the widest dialog added to the spell workspace.
    pub bits: Option<String>,
    /// `--choose <column>`: open the reference picker on that column of
    /// `--row`, selecting the row the column holds. On a display table's
    /// model or texture field it opens the model or texture browser.
    ///
    /// Same reason as `--bits`. On a column of the spell chain the picker
    /// plays the selected row, and this is how a scripted run photographs it.
    pub choose: Option<String>,
    /// `--like <spell id>`: open the dialog that chooses the look of the
    /// spell `--row` names by another spell, with that spell selected and
    /// playing. Same reason as `--bits`.
    pub like: Option<u32>,
    /// `--set <column>=<value>`: write one field of the row `--row` names,
    /// a few seconds after the row opens.
    ///
    /// An edit made while a preview is playing takes a different path from
    /// one made before the preview opens: the actors are already standing
    /// when the tables are read again. A scripted run cannot type into a
    /// field, so without this that path cannot be photographed or timed.
    /// The value is signed, so `-1` empties a kit's slot.
    pub set: Option<(String, u32)>,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--row", One, |a, v| a.tables.row = number(v)),
    Flag::new("--table", One, |a, v| a.tables.table = named(v)),
    Flag::new("--lab", Switch, |a, _| a.tables.lab = true),
    Flag::new("--browse", Switch, |a, _| a.tables.browse = true),
    Flag::new("--story", Switch, |a, _| a.tables.story = true),
    Flag::new("--seek", One, |a, v| a.tables.seek = number(v)),
    Flag::new("--bits", One, |a, v| a.tables.bits = named(v)),
    Flag::new("--choose", One, |a, v| a.tables.choose = named(v)),
    Flag::new("--set", One, |a, v| {
        a.tables.set = text(v).and_then(|pair| {
            let (column, value) = pair.split_once('=')?;
            let value: i64 = value.trim().parse().ok()?;
            Some((column.trim().to_string(), value as u32))
        });
    }),
    Flag::new("--like", One, |a, v| a.tables.like = number(v)),
];
