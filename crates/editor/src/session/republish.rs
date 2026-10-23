//! What the client's caches are told when the overlay changes, and the edited
//! tables put where a preview reads them.

use vale_client::assets::GameAssets;
use bevy::prelude::*;
use super::EditSession;

/// Tell every cache in the client what has changed under it.
///
/// Publishing bytes into the overlay is half of a change. The other half is
/// that the overlay is asked only when a path is read, and the client has four
/// banks that read a path once and keep what they read:
///
/// ```text
/// ModelCache             every model, dressing and skin, by archive path
/// GameAssets             the DBC banks, the strings, the bindings, the sounds
/// DisplayCache           an Arc of the parsed display tables, and what it
///                        resolved from them
/// UiTextures             the interface painter's minimap pictures, by the
///                        path the index names
/// ```
///
/// Each needs to be told. The third is easy to miss behind the second:
/// `GameAssets::forget_tables` drops the bank's copy, and `DisplayCache` goes
/// on holding the `Arc` it took at the first entity. Without the fourth, a
/// redrawn minimap reaches only the first playtest: the picture is read then
/// and answered from the cache at every playtest after.
///
/// None of this rebuilds what is drawn. What is standing keeps its
/// handles; the next thing to ask reads the file. For the lab that is the
/// next hang, for the storyboard the next loop of the timeline, and for a
/// playtest the login.
#[allow(clippy::too_many_arguments)]
pub fn forget_what_changed(
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    models: Option<ResMut<vale_client::render::models::ModelCache>>,
    displays: Option<ResMut<vale_client::world::entities::DisplayCache>>,
    mut stage: ResMut<crate::preview::stage::Stage>,
    mut ui_textures: Option<ResMut<vale_client::ui::mesh::textures::UiTextures>>,
) {
    let Some(mut session) = session else { return };

    // The minimap pictures first, since they are the one bank here that
    // is not the model cache and the paths are about to be drained.
    if session.republished_all {
        if let Some(textures) = ui_textures.as_mut() {
            textures.forget_all_minimaps();
        }
    } else {
        for path in &session.republished {
            if let Some(textures) = ui_textures.as_mut() {
                textures.forget_minimap(path);
            }
        }
    }

    // The tables first, because a display the models are then asked for
    // is resolved through them.
    if std::mem::take(&mut session.tables_republished) {
        let began = std::time::Instant::now();
        assets.forget_tables();
        // Read again here, and not on the next entity that needs a display.
        // The stage's actors are already standing when a kit is edited
        // beside its preview, so there is no next entity: the effect and
        // pose passes would find no tables from then on, and the actors
        // would stand idle with nothing hung on them.
        //
        // The re-parse is 73 reads with `Spell.dbc` among them twice, about
        // a tenth of a second. Doing it in this call puts it where the
        // person has just made an edit, and gives it somewhere to be
        // measured.
        if let Some(mut displays) = displays {
            displays.reread(&assets);
        }
        // The lab's compare keys on this revision, so without the bump the
        // model view keeps the effect it hung off the tables from before.
        session.republished_revision += 1;
        // What is standing is shown the edit now. The effect pass hangs a
        // kit when a counter changes, so a playing stage starts its loop
        // again, and a paused one is put back at its own time, which moves
        // the counters of its phase again and leaves it paused.
        if stage.showing.is_some() || stage.pushing.is_some() {
            match stage.playing {
                true => stage.restart(),
                false => {
                    let at = stage.at;
                    stage.seek(at);
                }
            }
        }
        let ok = assets.display_tables().is_ok();
        info!(
            "tables re-read in {:?}{}",
            began.elapsed(),
            match ok {
                true => "",
                false => " (they did not parse)",
            }
        );
    }

    let Some(mut models) = models else { return };
    if session.republished_all {
        session.republished_all = false;
        session.republished.clear();
        session.republished_revision += 1;
        models.forget_all();
        return;
    }
    if session.republished.is_empty() {
        return;
    }
    session.republished_revision += 1;
    for path in std::mem::take(&mut session.republished) {
        models.forget(&path);
    }
}

/// Put the edited tables where the client reads them, while a preview is
/// open.
///
/// The storyboard's preview is the client's own passes (see `crate::preview::stage`),
/// and those read `DisplayCache` — the client's parse of the archives — not
/// the editor's open `DbcFile`s. Without this, a kit edited on the form
/// changes the rail, which reads the edit, and not the picture, which does
/// not; a baked model then never appears in the storyboard.
///
/// Only while something is being previewed, and only once it has settled.
/// A field edit arrives on every frame of a drag and the re-read is not
/// cheap; [`SETTLE`] after the last one is one re-read per thing a person
/// does. With no preview open nothing here runs at all, which is every other
/// tool in the editor.
pub fn publish_for_the_preview(
    time: Res<Time>,
    stage: Res<crate::preview::stage::Stage>,
    tool: Res<crate::tools::Tool>,
    // The workspace in the pop-out window draws displays too.
    popout: Res<crate::popout::Popout>,
    session: Option<ResMut<EditSession>>,
    // The revision last seen, when it changed, the revision last published,
    // and whether the stage was open last frame. The third makes this run
    // once per change rather than once per settle: publishing does not
    // clear `unsaved_tables` — only saving does — so a re-arm on the clock
    // alone would put the tables where the client reads them every settle for
    // as long as the preview was open.
    mut seen: Local<(u64, f64, u64, bool)>,
) {
    /// How long a run of edits is left to settle before the tables are put
    /// where the client reads them. A field edit arrives on every frame of a
    /// drag and the re-read is about a tenth of a second, so it is not done
    /// per frame. It is kept short because a choice in a picker or a typed
    /// number is one edit, and the person is looking at the preview for it.
    const SETTLE: f64 = 0.15;

    let Some(mut session) = session else { return };
    let (had, changed_at, published, was_open) = &mut *seen;
    // Two things count as a preview being open.
    //
    // The stage is one: a spell played on two actors, which reads the tables
    // when it builds a loop. The lights tool is the other, and its preview is
    // the viewport itself — the world is drawn by the client's own passes
    // reading the client's own light chain, so an edited band shows in the
    // terrain, the fog and the sky with nothing else to open. Without this the
    // publish never ran for it and a colour edit reached nothing: the table
    // changed in the session and `GameAssets` went on holding the parse it
    // made at startup.
    //
    // A tool that draws creatures, game objects or items by their display ids
    // is a third: the display workspaces' previews, the template workspaces'
    // portraits and the spawn tools' models are all resolved through the
    // client's display tables. Without it a new display row resolved to
    // nothing, and a new skin showed on nothing, until a playtest published
    // the tables.
    use crate::tools::Tool;
    let draws = |tool: Tool| {
        matches!(
            tool,
            Tool::CreatureDisplays
                | Tool::ObjectDisplays
                | Tool::ItemDisplays
                | Tool::CreatureData
                | Tool::GameObjectData
                | Tool::Items
                | Tool::Creatures
                | Tool::GameObjects
        )
    };
    let draws_displays = draws(*tool) || popout.tools().any(draws);
    let previewing = stage.is_open() || *tool == Tool::Lights || draws_displays;
    if !previewing {
        // Nothing is being previewed. The next thing that needs the tables is
        // a playtest, which publishes for itself.
        *was_open = false;
        return;
    }
    // The frame a preview opens publishes at once. Most edits are made
    // with the stage shut — a kit's form closes it — and the stage's first
    // loop begins on the frame it opens, so a publish that waited for the
    // settle hung the first cast off the tables from before the edit and the
    // person saw the change one loop later, or never on a stage that was
    // paused. The preview then showed an edit only after the person
    // navigated away and back.
    let opened = !*was_open;
    *was_open = true;
    let now = time.elapsed_secs_f64();
    let revision = session.table_revision;
    if *had != revision {
        *had = revision;
        *changed_at = now;
    }
    if *published == revision || (!opened && now - *changed_at < SETTLE) {
        return;
    }
    *published = revision;
    let unsaved: Vec<String> = session.unsaved_tables.iter().cloned().collect();
    if unsaved.is_empty() {
        return;
    }
    for name in &unsaved {
        session.publish_table(name);
    }
    session.tables_republished = true;
    session.status = match unsaved.len() {
        1 => format!("{} written to the overlay for the preview", unsaved[0]),
        n => format!("{n} tables written to the overlay for the preview"),
    };
}
