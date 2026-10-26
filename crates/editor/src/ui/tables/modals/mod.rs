//! The dialogs a table's form opens, one at a time, over the middle of the
//! window. [`modals`] draws the one that is up.
//!
//! ```text
//! pick.rs       the reference picker: a target table's rows, or its icons as
//!               a grid, and the picker that plays the visuals, kits or
//!               effects it lists
//! looks_like.rs `Look like a spell…`: a spell's visual chosen by another
//!               spell, played
//! rows.rs       a table's rows as a list, opened to find one row
//! bits.rs       a mask as one checkbox per bit
//! models.rs     the model browser a model field opens
//! ```
//!
//! The file dialog of a `SoundEntries` row's Files section is in
//! `super::sound_files`, with the section that opens it.

pub(crate) mod bits;
pub(crate) mod looks_like;
pub(crate) mod models;
pub(crate) mod pick;
pub(crate) mod rows;

use bevy_egui::egui;

use crate::tables::Modal;
use crate::ui::tables::Workspace;
use crate::ui::tables::previews::previews;
use looks_like::looks_like;
use pick::pick_previewing;

/// How wide a reference picker's dialog is, and how many rows a page of it
/// holds. A search over 22,360 spells is paged rather than cut off, so every
/// row is reachable; a page is what one scroll of the list comfortably is.
pub(in crate::ui::tables) const PICKER_WIDTH: f32 = 380.0;
pub(in crate::ui::tables) const PICKER_PAGE: usize = 100;

/// The dialog that is up, if one is: a reference picker or the model browser.
/// Drawn once per frame from the form, whichever field opened it.
///
/// Both are paged rather than truncated. Cutting a picker at two hundred hits
/// and the model browser at three hundred left most of the twelve thousand
/// icons or six thousand models unreachable. A page is turned with the arrows,
/// `PageUp` and `PageDown`, and a search narrows the list. The page returns to
/// the first whenever the hits are rebuilt.
///
/// `preview` is the stage and the lab, for the two dialogs that play what
/// they list ([`pick_previewing`], [`looks_like`]). A caller with no stage
/// passes `None`, and a chain picker is then the plain list.
pub(in crate::ui) fn modals(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    preview: Option<(&mut crate::preview::stage::Stage, &mut crate::preview::lab::Lab)>,
) {
    let Some(modal) = work.browser.modal.clone() else {
        return;
    };
    if let Some((stage, lab)) = preview {
        let closes = match &modal {
            Modal::Pick {
                table,
                record,
                field,
                points_at,
            } if previews(points_at) => Some(pick_previewing(
                ui, work, stage, lab, table, *record, *field, *points_at,
            )),
            Modal::LooksLike { record } => Some(looks_like(ui, work, stage, *record)),
            _ => None,
        };
        if let Some(closes) = closes {
            if closes {
                work.browser.modal = None;
            }
            return;
        }
    }
    let table_name = work.browser.table.clone();
    let mut close = false;
    match modal {
        Modal::Pick {
            table,
            record,
            field,
            points_at,
        } => close = pick::pick(ui, work, table, record, field, points_at),
        // Drawn above when there is a stage to play on. Where there is
        // none the dialog has nothing to show.
        Modal::LooksLike { .. } => {
            close = true;
        }
        Modal::Rows { table } => close = rows::rows(ui, work, table),
        Modal::Bits {
            table,
            record,
            field,
            column,
            bits,
        } => close = bits::bits(ui, work, table, record, field, column, bits),
        Modal::Textures { record, field } => {
            if crate::ui::tables::appearance::textures(ui, work, &table_name, record, field) {
                close = true;
            }
        }
        Modal::Models { record, field } => close = models::models(ui, work, table_name, record, field),
        Modal::SoundFile { record, slot } => close = crate::ui::tables::sound_files::pick_file(ui, work, record, slot),
    }
    if close {
        work.browser.modal = None;
    }
}
