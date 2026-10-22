//! Opening the tables a workspace needs, one per frame, and pointing the
//! browser at the rail's table.

use crate::session::EditSession;
use vale_assets::tables::schema;
use vale_assets::tables::schema::Kind;
use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::Modal;
use super::browser::Browser;
use super::tabs::{ANY, table_named};
use vale_assets::tables::schema::chain_for;
use super::fields::set_field;

/// Open the tables the chosen subject needs, one table per frame.
///
/// This follows the same pattern as [`crate::map::open::open_tiles`]. `Spell.dbc` is
/// 16 MB, and parsing it on the frame the Spells tool is chosen causes a
/// visible stall, so each table is requested once and the panel draws whatever
/// is open.
pub fn open_tables(
    tool: Res<crate::tools::Tool>,
    // A table workspace in a pop-out window has the browser unless the main
    // window's tool has a table. See `crate::popout`.
    popout: Res<crate::popout::Popout>,
    assets: Res<GameAssets>,
    args: Res<crate::args::Args>,
    session: Option<ResMut<EditSession>>,
    browser: Option<ResMut<Browser>>,
    mut stage: ResMut<crate::preview::stage::Stage>,
    mut lab: ResMut<crate::preview::lab::Lab>,
    mut pointed: Local<Option<String>>,
    time: Res<Time>,
    // When `--row` opened its row, and whether `--set` has been written.
    mut scripted: Local<(Option<f64>, bool)>,
) {
    let (Some(mut session), Some(mut browser)) = (session, browser) else {
        return;
    };
    let Some(wanted) = popout.browser_tool(*tool).table() else {
        // Clear the stage here when no table tool is chosen. The panel sets
        // `showing`, and the panel is not drawn once the rail moves to a World
        // tool. Without this the stage stays up, the actors stay in the world
        // and the camera stays locked on the storyboard's characters.
        stage.close();
        return;
    };
    // The rail's table is where the browser starts, not a table it is held on.
    // Pointing the browser at it on every frame would undo a followed
    // reference on the next frame: `SpellVisual` is opened, then the next run
    // of this system points the browser back at `Spell` and loses the row. So
    // the rail's choice is applied only when it changes, and the browser moves
    // freely after that.
    // A follow from outside the data tools arrives together with the rail's
    // change and is not undone by it; see [`Browser::followed_in`].
    //
    // The Tables workspace has no table of its own: it starts on the table
    // it last showed, or on the list of tables.
    let any = wanted == ANY;
    let start = match any {
        true => browser.any.clone(),
        false => wanted.to_string(),
    };
    let followed_in = std::mem::take(&mut browser.followed_in);
    // The back stack belongs to the workspace it was made in, and is cleared
    // when the workspace changes. If it were kept, `< back` would open the
    // last workspace's tables under this one's tool: Zones would show
    // SkillLineAbility with no tab to leave by, since Zones has one table and
    // draws no tabs. A follow from outside pushes the table the browser last
    // showed, which is not a way back either, so it is cleared too.
    if pointed.as_deref() != Some(wanted) {
        browser.back.clear();
    }
    if pointed.as_deref() != Some(wanted) && followed_in {
        *pointed = Some(wanted.to_string());
    }
    if pointed.as_deref() != Some(wanted) {
        *pointed = Some(wanted.to_string());
        // `--table` names the table `--row` refers to, and is applied once.
        //
        // It accepts any table, not only the tool's own tabs. The tabs are a
        // few of the tables; a table reached by following a reference
        // (`SpellCategory`, `SpellIcon`, `SpellRange`) has no tab but can be
        // opened, and so can one with no schema. An earlier version accepted
        // `--table` on one of those and then ignored it, which left the run on
        // `Spell`.
        // A name that is not a table logs a warning, as `--without` does.
        match (args.table.as_deref(), browser.seeded) {
            (Some(table), false) => match table_named(table) {
                Some(name) => browser.look_at(name),
                None => {
                    warn!("--table {table}: no table by that name");
                    browser.look_at(&start);
                }
            },
            _ => browser.look_at(&start),
        }
    }
    if any {
        browser.any = browser.table.clone();
    }

    // Open the table the browser is now pointed at. On the list of tables
    // there is none.
    let open = browser.table.clone();
    if open.is_empty() {
        stage.close();
        return;
    }
    if !session.open_table(&assets, &open) {
        // A file that does not open, such as one of the four zero-byte
        // ones, returns the Tables workspace to its list. The session's
        // status line says why.
        if any {
            browser.look_at(ANY);
            browser.any.clear();
        }
        return;
    }

    // Then open the subject's root table and its chain, one per frame, so no
    // single frame waits on all of them.
    //
    // The root table is opened here because `chain_for` lists the tables a
    // subject references and never the subject itself. Without this, a
    // browser opened directly on a table the root references (`--table
    // SpellCategory`, or a followed reference) leaves `Spell` closed. The row
    // and its fields still draw, but the reverse index is built over the open
    // tables only and finds nothing, so every category reads `unused` while
    // 136 of the 166 are used by 22,360 spells. The reverse index is only
    // complete when every table that references the target is open.
    //
    // In the Tables workspace the root is the open table itself.
    let root: &str = match any {
        true => &open,
        false => wanted,
    };
    if let Some(next) = std::iter::once(root)
        .chain(chain_for(root).iter().copied())
        .find(|name| !session.tables.contains_key(*name))
    {
        session.open_table(&assets, next);
        return;
    }
    browser.index(&session, &open);

    // `--row <id>`, once the table it names is here. See `crate::args::Args::row`.
    if let (Some(id), false) = (args.row, browser.seeded) {
        browser.seeded = true;
        if args.story {
            browser.view = crate::ui::tables::View::Storyboard;
        }
        // The stage holds the seek until the actors are spawned, then pauses
        // at that time; see `Stage::seek`.
        if let Some(seek) = args.seek {
            stage.seek(seek);
        }
        match session.table(&open).and_then(|table| table.row_of(id)) {
            Some(record) => {
                browser.open = Some(record);
                // `--lab` on an effect row opens the attachment lab on it,
                // which otherwise requires a press on the form.
                if args.lab && open == "SpellVisualEffectName" {
                    crate::ui::preview::lab::open_on(&mut lab, &mut stage, &mut browser, &session, record);
                }
                // `--bits <column>` opens the mask dialog on that column. The
                // dialog is otherwise opened by a control a scripted run
                // cannot press. A name that is not a mask column of this table
                // logs a warning instead of opening nothing, as `--without`
                // does.
                if let Some(wanted) = args.bits.as_deref() {
                    match schema::for_table(&open).and_then(|schema| {
                        schema.columns.iter().find(|column| {
                            column.name.eq_ignore_ascii_case(wanted.trim())
                                && matches!(column.kind, Kind::Flags(bits) if !bits.is_empty())
                        })
                    }) {
                        Some(column) => {
                            if let Kind::Flags(bits) = column.kind {
                                browser.modal = Some(Modal::Bits {
                                    table: open.clone(),
                                    record,
                                    field: column.field,
                                    column: column.name,
                                    bits,
                                });
                            }
                        }
                        None => warn!("--bits {wanted}: {open} has no named mask by that name"),
                    }
                }
                // `--choose <column>` opens the reference picker on that
                // column, for the same reason. The picker selects the row
                // the column holds, so one over the spell chain opens
                // playing it.
                if let Some(wanted) = args.choose.as_deref() {
                    match schema::for_table(&open).and_then(|schema| {
                        schema.columns.iter().find_map(|column| match column.kind {
                            Kind::Reference(points_at)
                                if column.name.eq_ignore_ascii_case(wanted.trim()) =>
                            {
                                Some((column.field, points_at))
                            }
                            _ => None,
                        })
                    }) {
                        Some((field, points_at)) => {
                            browser.modal = Some(Modal::Pick {
                                table: open.clone(),
                                record,
                                field,
                                points_at,
                            });
                            browser.pick_query.clear();
                            browser.pick_focus = true;
                        }
                        None => {
                            // A display table's file field opens its browser.
                            use crate::ui::tables::appearance::{file_field, FileField};
                            let file = schema::for_table(&open).and_then(|schema| {
                                schema.columns.iter().find_map(|column| {
                                    let named = column.name.eq_ignore_ascii_case(wanted.trim());
                                    named
                                        .then(|| file_field(&open, column.field).map(|of| (column.field, of)))
                                        .flatten()
                                })
                            });
                            match file {
                                Some((field, FileField::Texture(_))) => {
                                    browser.modal = Some(Modal::Textures { record, field });
                                    browser.texture_preview = None;
                                    browser.texture_query.clear();
                                    browser.texture_page = 0;
                                    browser.pick_focus = true;
                                }
                                Some((field, _)) => {
                                    browser.modal = Some(Modal::Models { record, field });
                                    browser.model_query.clear();
                                    browser.pick_focus = true;
                                }
                                None => warn!(
                                    "--choose {wanted}: {open} has no reference or file field by that name"
                                ),
                            }
                        }
                    }
                }
                // `--like <spell id>` opens the dialog that takes another
                // spell's look, with that spell selected.
                if let (Some(like), true) = (args.like, open == "Spell") {
                    browser.modal = Some(Modal::LooksLike { record });
                    browser.pick_query.clear();
                    browser.pick_selected = Some(like);
                }
                // `--browse` opens the model browser over the row. It is
                // otherwise two presses in, so a scripted `--shot` could not
                // reach it.
                if args.browse && open == "SpellVisualEffectName" {
                    browser.modal = Some(Modal::Models {
                        record,
                        field: vale_assets::tables::spell::fields::EFFECT_MODEL,
                    });
                    browser.model_query.clear();
                    browser.pick_focus = true;
                }
            }
            None => session.status = format!("{open} has no row {id}"),
        }
    }

    // `--set <column>=<value>`, once the row has been open for
    // [`SET_AFTER`] seconds: long enough for a preview beside the form to be
    // playing, so the write is an edit made under a standing preview.
    /// How long after the row opens `--set` writes its field.
    const SET_AFTER: f64 = 6.0;
    if let (Some((wanted, value)), Some(record), false) =
        (args.set.as_ref(), browser.open, scripted.1)
    {
        let now = time.elapsed_secs_f64();
        let since = *scripted.0.get_or_insert(now);
        if browser.seeded && now - since >= SET_AFTER {
            scripted.1 = true;
            let column = schema::for_table(&open).and_then(|schema| {
                schema
                    .columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(wanted))
            });
            match column {
                Some(column) => {
                    set_field(
                        &mut session,
                        &open,
                        record,
                        column.field,
                        *value,
                        &format!("Edit {}", column.name),
                        now,
                    );
                }
                None => warn!("--set {wanted}: {open} has no column by that name"),
            }
        }
    }
}
