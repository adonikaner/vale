//! What can be done to a row other than editing a field: a table's commands,
//! worded for one row, and what each one does.

use crate::session::EditSession;
use bevy::prelude::*;
use super::browser::Browser;
use super::learning::teachers_of;
use super::zones::area;

/// A command on one row of a table.
///
/// The list's buttons, the form's buttons and a row's right-click menu all
/// take a row's commands from [`commands`] and word them through
/// [`command_label`], so a command reads the same wherever it is pressed,
/// and one that does not apply to a row is offered in none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    AddToSkillLine,
    CreateTeachingSpell,
    LookLike,
    CloneChain,
    GiveToRacesAndClasses,
    /// A new sub-area of a zone. Offered on a zone's row.
    AddSubArea,
    /// Give the area to the Areas tool's brush and switch to that tool.
    PaintArea,
    Clone,
    Delete,
    CopyId,
}

/// The commands the rows of a table take, in the order a menu lists them:
/// the table's own first, then the three every row has.
pub fn commands(table: &str) -> Vec<Command> {
    let mut all = match table {
        "Spell" => vec![
            Command::AddToSkillLine,
            Command::CreateTeachingSpell,
            Command::LookLike,
        ],
        "SpellVisual" => vec![Command::CloneChain],
        "SkillLine" => vec![Command::GiveToRacesAndClasses],
        "AreaTable" => vec![Command::AddSubArea, Command::PaintArea],
        _ => Vec::new(),
    };
    all.extend([Command::Clone, Command::Delete, Command::CopyId]);
    all
}

/// What a command is called for one row, or `None` when it does not apply to
/// that row. A command with a consequence states it in its name: a menu acts
/// on a row that has not been opened, so there is no form to read it from.
pub fn command_label(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
    command: Command,
) -> Option<String> {
    let id = session.table(table)?.u32_at(record, 0)?;
    Some(match command {
        Command::AddToSkillLine => "Add to a skill line".to_string(),
        // Offered while the spell has no instant teaching spell, which is the
        // one a trainer's list names.
        Command::CreateTeachingSpell => {
            let teachers = teachers_of(browser, session, id);
            if teachers.iter().any(|&(_, instant)| instant) {
                return None;
            }
            match teachers.is_empty() {
                true => "Create a teaching spell".to_string(),
                false => "Create an instant teaching spell".to_string(),
            }
        }
        Command::LookLike => "Visual from another spell\u{2026}".to_string(),
        Command::CloneChain => "Clone chain".to_string(),
        Command::GiveToRacesAndClasses => "Add race/class row".to_string(),
        // Offered on a zone. A sub-area's own sub-area would be a third
        // level, which the table does not have.
        Command::AddSubArea => {
            if session.table(table)?.u32_at(record, area::PARENT)? != 0 {
                return None;
            }
            "Add a sub-area".to_string()
        }
        Command::PaintArea => "Paint on map".to_string(),
        Command::Clone => "Clone".to_string(),
        Command::Delete => match browser.used_by(session, table, id).len() {
            0 => "Delete".to_string(),
            1 => "Delete: 1 reference will point at nothing".to_string(),
            n => format!("Delete: {n} references will point at nothing"),
        },
        Command::CopyId => format!("Copy id {id}"),
    })
}

/// What a command does, for the tooltip on its button and its menu entry.
pub fn command_about(command: Command) -> &'static str {
    match command {
        Command::AddToSkillLine => {
            "Adds a SkillLineAbility row for this spell and opens the skill line picker. \
             One undo entry. A race or class mask of zero means every race or every class."
        }
        Command::CreateTeachingSpell => {
            "Adds a Spell row under the next id that teaches this spell: Learn Spell as its \
             first effect, an instant cast, and this spell's name, rank and icon. One undo \
             entry. The Trainer window can then add it to a trainer's list."
        }
        Command::LookLike => {
            "Choose another spell, previewed in the dialog, and either share its SpellVisual \
             row with this spell or clone that row for this spell."
        }
        Command::CloneChain => {
            "Copies this visual with every kit and effect it references, points the copies \
             at each other, and opens the copied visual. The original and the spells that \
             use it do not change."
        }
        Command::GiveToRacesAndClasses => {
            "Adds a SkillRaceClassInfo row for this skill line with every race, every class \
             and no flags set. One undo entry."
        }
        Command::AddSubArea => {
            "Adds an AreaTable row with this zone as its parent, on the zone's map, with the \
             zone's flags, reverb, ambience and music and an unused explore bit. One undo entry."
        }
        Command::PaintArea => {
            "Switches to the Areas tool with this area on the brush. An area covers the map \
             chunks that carry its id; a new area covers no chunks until it is painted."
        }
        Command::Clone => "Copies the row under the next id and appends \" (copy)\" to its name.",
        Command::Delete => "Removes the row. One undo entry.",
        Command::CopyId => "Copies the row's id to the clipboard.",
    }
}
