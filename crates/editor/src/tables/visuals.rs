//! The spell chain's rows found and pictured by their models: the search
//! words of a kit and a visual, a kit's first model, and the kit's effect
//! slots by attachment point.

use crate::session::EditSession;
use vale_assets::tables::schema;
use vale_edit::dbc::Cell;
use bevy::prelude::*;
use super::browser::Browser;
use super::labels::{KIT_MODELS, VISUAL_SLOTS, basename};

/// The model that pictures a row of the spell chain in a list: an effect's
/// own, a kit's first, and for a visual the first model of the first kit
/// that has one, then its missile's and its area's. `None` for a row with no
/// model and for a row of any other table.
pub fn first_model(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
) -> Option<String> {
    use vale_assets::tables::spell::fields;
    let named = |id: &u32| *id != 0 && *id != u32::MAX;
    match table {
        "SpellVisualEffectName" => session
            .table(table)?
            .string_at(record, fields::EFFECT_MODEL)
            .filter(|path| !path.is_empty()),
        "SpellVisualKit" => {
            let kits = session.table(table)?;
            let effects: Vec<u32> = KIT_MODELS
                .iter()
                .filter_map(|&field| kits.u32_at(record, field))
                .filter(named)
                .collect();
            effects.into_iter().find_map(|effect| {
                let row = browser.record_of(session, "SpellVisualEffectName", effect)?;
                first_model(browser, session, "SpellVisualEffectName", row)
            })
        }
        "SpellVisual" => {
            let visuals = session.table(table)?;
            let kits: Vec<u32> = VISUAL_SLOTS
                .iter()
                .filter_map(|&(field, _)| visuals.u32_at(record, field))
                .filter(named)
                .collect();
            let of_a_kit = kits.into_iter().find_map(|kit| {
                let row = browser.record_of(session, "SpellVisualKit", kit)?;
                first_model(browser, session, "SpellVisualKit", row)
            });
            if of_a_kit.is_some() {
                return of_a_kit;
            }
            [fields::MISSILE_MODEL, fields::AREA_MODEL]
                .into_iter()
                .filter_map(|field| visuals.u32_at(record, field))
                .filter(named)
                .find_map(|effect| {
                    let row = browser.record_of(session, "SpellVisualEffectName", effect)?;
                    first_model(browser, session, "SpellVisualEffectName", row)
                })
        }
        _ => None,
    }
}

/// What a row of the spell chain is found by in a search and is not labelled
/// with: every model a kit hangs, by its effect's name and its file's name,
/// and for a visual the models of each of its kits, its missile and its area,
/// and the name of every spell that uses it.
///
/// A label has room for one model and one spell. A person looking for a kit
/// remembers a model, such as a file with `frost` in its name, and the kit
/// that hangs it third is the one they want as often as the kit that hangs
/// it first.
pub(super) fn search_words(
    browser: &mut Browser,
    session: &EditSession,
    table: &str,
    record: usize,
) -> String {
    use vale_assets::tables::spell::fields;
    let mut words = String::new();
    match table {
        "SpellVisualKit" => kit_words(browser, session, &mut words, record),
        "SpellVisual" => {
            let Some(visuals) = session.table(table) else {
                return words;
            };
            for &(field, _) in &VISUAL_SLOTS {
                let row = visuals
                    .u32_at(record, field)
                    .filter(|kit| *kit != 0 && *kit != u32::MAX)
                    .and_then(|kit| browser.record_of(session, "SpellVisualKit", kit));
                if let Some(row) = row {
                    kit_words(browser, session, &mut words, row);
                }
            }
            for field in [fields::MISSILE_MODEL, fields::AREA_MODEL] {
                if let Some(effect) = visuals.u32_at(record, field) {
                    effect_words(browser, session, &mut words, effect);
                }
            }
            // Each name once: the nine ranks of a spell share a visual.
            let id = visuals.u32_at(record, 0).unwrap_or_default();
            let mut seen: Vec<String> = Vec::new();
            for at in browser.used_by(session, "SpellVisual", id) {
                if at.table != "Spell" {
                    continue;
                }
                let name = session
                    .table("Spell")
                    .and_then(|spells| {
                        spells.string_at(
                            at.record,
                            vale_assets::tables::spellbook::spell_fields::NAME,
                        )
                    })
                    .unwrap_or_default();
                if !name.is_empty() && !seen.contains(&name) {
                    words.push(' ');
                    words.push_str(&name);
                    seen.push(name);
                }
            }
        }
        _ => {}
    }
    words
}

/// Add the name and the model file's name of every effect a kit hangs.
fn kit_words(browser: &mut Browser, session: &EditSession, words: &mut String, record: usize) {
    let Some(kits) = session.table("SpellVisualKit") else {
        return;
    };
    for &field in &KIT_MODELS {
        if let Some(effect) = kits.u32_at(record, field) {
            effect_words(browser, session, words, effect);
        }
    }
}

/// Add an effect's name and its model file's name. An id that is no row,
/// which includes the two values for none, adds nothing.
fn effect_words(browser: &mut Browser, session: &EditSession, words: &mut String, effect: u32) {
    use vale_assets::tables::spell::fields;
    let Some(names) = session.table("SpellVisualEffectName") else {
        return;
    };
    let Some(row) = browser.record_of(session, "SpellVisualEffectName", effect) else {
        return;
    };
    for field in [fields::EFFECT_NAME, fields::EFFECT_MODEL] {
        let text = names.string_at(row, field).unwrap_or_default();
        words.push(' ');
        words.push_str(basename(&text));
    }
}

/// The kit column that names an effect, if exactly one kit column does.
///
/// Returns `(record, field)` of the `SpellVisualKit` row and the kit's own id.
/// The attachment an effect hangs from is not a field of the effect or of the
/// kit: it is determined by which column of the kit names the effect, and
/// `spell::fields::EFFECTS` is that mapping. Moving an effect to the head is
/// therefore a move between two columns of one row.
///
/// Returns `None` when no kit names the effect, or when more than one does.
/// In the second case a move would change several spells, and the choice of
/// which is left to the user.
pub fn effect_slot(
    browser: &mut Browser,
    session: &EditSession,
    effect: u32,
) -> Option<(usize, usize, u32)> {
    let mut found: Option<(usize, usize, u32)> = None;
    for at in browser.used_by(session, "SpellVisualEffectName", effect) {
        if at.table != "SpellVisualKit" {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let id = session.table("SpellVisualKit")?.u32_at(at.record, 0)?;
        found = Some((at.record, at.field, id));
    }
    found
}



/// Move an effect from one column of a kit to another, which changes its
/// attachment point. One entry on the undo stack.
///
/// The old column is written with `-1` rather than 0. Both mean no effect (see
/// `SpellVisuals::build`, "the table's two ways of saying nothing"), and `-1`
/// is what the shipped rows and [`blank_defaults`](vale_assets::tables::schema::blank_defaults)
/// use.
///
/// Returns `false` when the two columns are the same or the table is not open.
pub fn move_effect_slot(
    session: &mut EditSession,
    record: usize,
    from: usize,
    to: usize,
    effect: u32,
) -> bool {
    if from == to {
        return false;
    }
    let Some(table) = session.tables.get_mut("SpellVisualKit") else {
        return false;
    };
    let Some(clear) = Cell::new(table, record, from, u32::MAX) else {
        return false;
    };
    let Some(set) = Cell::new(table, record, to, effect) else {
        return false;
    };
    clear.apply(table);
    set.apply(table);
    session.history.begin("Move effect to another attachment");
    session.history.record_cell("SpellVisualKit", clear);
    session.history.record_cell("SpellVisualKit", set);
    session.history.end();
    session.table_edited("SpellVisualKit");
    true
}

/// Where an effect is hung, from its use in the tables: a sentence for a card,
/// and the attachment ids to try in order.
///
/// A kit column's attachment is the pairing in `spell::fields::EFFECTS`, which
/// is the point the client hangs the model from. A missile model is aimed at
/// the chest, and an area model stands at the feet. An effect that nothing
/// names defaults to the chest, and the sentence says so.
pub fn effect_anchor(
    browser: &mut Browser,
    session: &EditSession,
    effect: u32,
) -> (String, &'static [u32]) {
    use vale_assets::world::m2::attach;
    const CHEST: &[u32] = &[attach::CHEST, 15];
    const FEET: &[u32] = &[attach::BASE, 0];
    const HEAD: &[u32] = &[attach::HEAD, attach::HELM];
    const LEFT: &[u32] = &[attach::SPELL_HAND_LEFT, attach::HAND_LEFT];
    const RIGHT: &[u32] = &[attach::SPELL_HAND_RIGHT, attach::HAND_RIGHT];
    const BREATH: &[u32] = &[17, attach::HEAD];
    let uses = browser.used_by(session, "SpellVisualEffectName", effect);
    for at in &uses {
        if at.table == "SpellVisualKit" {
            let kit = session
                .table("SpellVisualKit")
                .and_then(|kits| kits.u32_at(at.record, 0))
                .unwrap_or(0);
            let column = schema::for_table("SpellVisualKit")
                .and_then(|schema| schema.column(at.field))
                .map(|column| column.name)
                .unwrap_or("a");
            let ids: &'static [u32] = match at.field {
                3 => HEAD,
                4 => CHEST,
                5 | 12 => FEET,
                6 => LEFT,
                7 => RIGHT,
                8 => BREATH,
                _ => CHEST,
            };
            return (
                format!(
                    "Anchor: this effect is the {column} of kit {kit}, so the attachment point defaults to \
                     that column's point. The exported model stores only the offset, so in game it is placed \
                     relative to the same attachment."
                ),
                ids,
            );
        }
    }
    for at in &uses {
        if at.table == "SpellVisual" {
            let visual = session
                .table("SpellVisual")
                .and_then(|visuals| visuals.u32_at(at.record, 0))
                .unwrap_or(0);
            let (word, ids): (&str, &'static [u32]) = match at.field {
                vale_assets::tables::spell::fields::MISSILE_MODEL => ("missile model", CHEST),
                _ => ("area model", FEET),
            };
            return (
                format!(
                    "Anchor: this effect is the {word} of visual {visual}, so the attachment point defaults \
                     to match. The exported model stores only the offset."
                ),
                ids,
            );
        }
    }
    (
        "No kit or visual references this effect, so the attachment point defaults to the chest. The \
         exported model stores only the offset from the chosen point."
            .to_string(),
        CHEST,
    )
}
