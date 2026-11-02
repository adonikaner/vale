//! The World Map part of the Zones workspace. The state is
//! [`crate::import::worldmap`].
//!
//! ```text
//! mod.rs       the list of maps, and the chosen map's head and layout
//! picture.rs   the picture: the sectors chosen on it, the explored areas
//!              drawn over it and dragged, the rectangles of what is on it
//! overlays.rs  the explored areas: the list, the chosen overlay's numbers and
//!              areas, new art
//! places.rs    a continent's zones and the whole world's continents on its
//!              picture, chosen, moved and resized there
//! rows.rs      the map's settings: directory, rectangle, a continent's place
//!              on the whole world, its zone grid and highlight, removing it
//! create.rs    the New map form
//! ```

mod create;
mod overlays;
mod picture;
mod places;
mod rows;

use bevy_egui::egui;

use crate::import::worldmap::{picture::Source, Ask, Entry, Kind, Which, WorldMaps};
use crate::session::EditSession;
use crate::ui::shell::asks::{ShellAsk, ShellAsks};
use crate::ui::subjects::listing;
use crate::ui::widgets::theme;
use vale_client::assets::GameAssets;
use vale_edit::worldmap::{self as pieces, Fit};

/// What the part is drawn from.
pub(super) struct Work<'a> {
    pub(super) maps: &'a mut WorldMaps,
    pub(super) session: &'a mut EditSession,
    pub(super) assets: &'a GameAssets,
    pub(super) asks: &'a mut ShellAsks,
    pub(super) now: f64,
}

/// What the picture shows and what a click on it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum View {
    /// The picture, and a click chooses sectors.
    #[default]
    Picture,
    /// The explored areas drawn over it, and a click chooses one.
    Explored,
    /// A continent's zones or the whole world's continents outlined over it,
    /// and a click chooses one.
    Places,
}

/// The World Map part in the middle of a window.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, assets, now, .. } = bench;
    let mut work = Work { maps: &mut editing.worldmap, session, assets, asks: &mut editing.asks, now };
    listing::frame(ui, &mut work, "world-maps", list, details);
}

/// The inspector beside it: how the client reads the pictures and rows.
pub(crate) fn inspect(ui: &mut egui::Ui, _: crate::ui::tools::Subject<'_>, _: &mut crate::ui::Editing<'_>) {
    theme::heading(ui, "How the client shows it");
    theme::note(
        ui,
        "The world map draws twelve 256x256 BLP pieces, four across and three down, and shows the \
         top-left 1002x668 pixels of them. A map's picture covers its WorldMapArea rectangle, and \
         the player's marker is placed by the same rectangle.",
    );
    theme::heading(ui, "Which map opens");
    theme::note(
        ui,
        "On a continent's map the world map opens on the zone the character is in, or on the \
         continent when the zone has no map. On a map with no continent it opens on that map's \
         single map, as in a battleground. Anywhere else it shows the whole world.",
    );
    theme::heading(ui, "Sectors");
    theme::note(
        ui,
        "Click sectors of the picture to choose them. Import picture and Draw from minimap then \
         change only those sectors; with none chosen they change the whole map.",
    );
    theme::heading(ui, "Zones and continents");
    theme::note(
        ui,
        "A zone's place on its continent's picture is its WorldMapArea rectangle inside the \
         continent's, and a continent's place on the whole world's picture is its WorldMapContinent \
         offsets and scale. The Zones and Continents views show them and move them by dragging.",
    );
    theme::heading(ui, "Explored areas");
    theme::note(
        ui,
        "A zone's picture is drawn bare, and each WorldMapOverlay row's art is drawn over it once \
         any of its four areas is explored. Its label rectangle is where the area's name shows \
         under the pointer.",
    );
}

/// The left column: the parts, the heading, New map, the search and the maps.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    if let Some(part) = crate::ui::shell::rail::parts(ui, crate::tools::Tool::WorldMap) {
        work.asks.push(ShellAsk::Part(part));
    }
    let entries: Vec<Entry> = work.maps.entries(work.session, work.assets).to_vec();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("World Map").strong().size(15.0).color(theme::INK));
        ui.label(theme::number(format!("{} maps", entries.len())));
    });
    if ui
        .button("New map\u{2026}")
        .on_hover_text("A zone map for a zone on a continent, a continent for a map, or a single map for a map.")
        .clicked()
    {
        work.maps.create = Some(Default::default());
    }
    listing::search_box(ui, &mut work.maps.query, "Search maps");
    let query = work.maps.query.trim().to_lowercase();
    let shown: Vec<&Entry> = entries
        .iter()
        .filter(|entry| {
            query.is_empty()
                || entry.name.to_lowercase().contains(&query)
                || entry.directory.to_lowercase().contains(&query)
        })
        .collect();
    if !query.is_empty() {
        listing::shown_count(ui, shown.len());
    }
    theme::list_area(ui, None, listing::ROW_HEIGHT).show(ui, |ui| {
        for entry in shown {
            let owned = work.maps.owns(work.session, &entry.directory);
            let continent = entry.continent.and_then(|id| work.maps.entry_of(Which::Row(id))).map(|c| c.name.clone());
            let mut sub = match (entry.kind, &entry.area) {
                (Kind::Continent | Kind::Single, Some(area)) => format!("{}, map {}", entry.kind.name(), area.map),
                (Kind::Zone, _) if !query.is_empty() => format!("Zone of {}", continent.unwrap_or_default()),
                (kind, _) => kind.name().to_string(),
            };
            if owned {
                sub.push_str(", the project's picture");
            }
            let trailing = match entry.which {
                Which::World => String::new(),
                Which::Row(id) => id.to_string(),
            };
            let chosen = work.maps.chosen == Some(entry.which) && work.maps.create.is_none();
            let row = ui
                .horizontal(|ui| {
                    if entry.kind == Kind::Zone && query.is_empty() {
                        ui.add_space(14.0);
                    }
                    theme::list_row(
                        ui,
                        theme::ListRow { title: &entry.name, sub: &sub, trailing: &trailing, tint: theme::INK, picture: false },
                        chosen,
                    )
                })
                .inner;
            let row = row.response.on_hover_text(format!(
                "Interface\\WorldMap\\{0}\\{0}1.blp to {0}12.blp",
                entry.directory
            ));
            if row.clicked() {
                work.maps.choose(entry.which);
            }
        }
    });
}

/// The chosen map, or the New map form.
fn details(ui: &mut egui::Ui, work: &mut Work<'_>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(ui.available_width().min(pieces::WIDTH as f32 + 16.0));
        if work.maps.create.is_some() {
            create::form(ui, work);
            return;
        }
        let Some(entry) = work.maps.chosen_entry().cloned() else {
            theme::note(ui, "Choose a map on the left, or make one with New map.");
            return;
        };
        about(ui, work, &entry);
        let mut view = match entry.kind {
            Kind::Zone | Kind::Single if work.maps.explored => View::Explored,
            Kind::Continent | Kind::World if work.maps.placing => View::Places,
            _ => View::Picture,
        };
        ui.add_space(6.0);
        let other = match entry.kind {
            Kind::Zone | Kind::Single => ("Explored areas", View::Explored),
            Kind::Continent => ("Zones", View::Places),
            Kind::World => ("Continents", View::Places),
        };
        ui.allocate_ui(egui::vec2(ui.available_width().min(300.0), 24.0), |ui| {
            theme::segmented(ui, &mut view, &[("Picture", View::Picture), other], |a, b| a == b);
        });
        match entry.kind {
            Kind::Zone | Kind::Single => work.maps.explored = view == View::Explored,
            Kind::Continent | Kind::World => work.maps.placing = view == View::Places,
        }
        match view {
            View::Picture => picture_actions(ui, work, &entry),
            View::Explored => overlays::actions(ui, work, &entry),
            View::Places => places::actions(ui, work, &entry),
        }
        ui.add_space(4.0);
        picture::picture(ui, work, &entry, view);
        match view {
            View::Explored => overlays::form(ui, work, &entry),
            View::Places => places::form(ui, work, &entry),
            View::Picture => {}
        }
        ui.add_space(8.0);
        rows::settings(ui, work, &entry);
    });
}

/// The map's name, its files, its row and where its pieces come from.
fn about(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&entry.name).strong().size(17.0).color(theme::INK));
        ui.label(egui::RichText::new(entry.kind.name()).color(theme::INK_DIM));
    });
    let directory = &entry.directory;
    theme::property(ui, "Files", "The twelve pieces the client reads, numbered row by row from the top left.", |ui| {
        ui.label(theme::number(format!("Interface\\WorldMap\\{directory}\\{directory}1-12.blp")));
    });
    if let Some(area) = &entry.area {
        theme::property(ui, "Row", "The WorldMapArea row: its id, its map, and its area (0 for a continent).", |ui| {
            ui.label(theme::number(format!("WorldMapArea {}, map {}, area {}", area.id, area.map, area.area)));
        });
    }
    let current = work.maps.picture.current(entry, work.session, work.assets);
    let (found, own) = (current.found, current.own.count_ones());
    let words = match (found, own) {
        (0, _) => "none found: the world map draws this map black".to_string(),
        (n, 0) if n == pieces::PIECES => "all 12 from the archives".to_string(),
        (n, 0) => format!("{n} of 12 from the archives; the rest are missing and drawn black"),
        (n, m) if m == n => format!("{n} of 12 from the project"),
        (n, m) => format!("{m} from the project, {} from the archives, {} missing", n - m, pieces::PIECES - n),
    };
    theme::property(ui, "Pieces", "Where the pieces the client reads now come from.", |ui| {
        ui.label(egui::RichText::new(words).color(theme::INK_DIM));
    });
}

/// The picture's actions: Import, Draw, Remove from project, the sectors
/// chosen, the drawing's progress, the new picture's choices.
fn picture_actions(ui: &mut egui::Ui, work: &mut Work<'_>, entry: &Entry) {
    let drawing = work.maps.picture.drawing.is_some();
    let selected = work.maps.picture.selected;
    let scope = match selected.count_ones() {
        0 => String::new(),
        1 => " into 1 sector".to_string(),
        n => format!(" into {n} sectors"),
    };
    let own = work.maps.picture.current(entry, work.session, work.assets).own;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!drawing, egui::Button::new(format!("Import picture{scope}\u{2026}")))
            .on_hover_text(
                "Choose a PNG, TGA or BLP picture. It is fitted to the chosen sectors, or to the whole \
                 map when none is chosen, and shown here before anything is written.",
            )
            .clicked()
        {
            work.maps.ask = Some(Ask::Import);
        }
        let draw = ui.add_enabled(entry.area.is_some() && !drawing, egui::Button::new(format!("Draw from minimap{scope}")));
        let draw = match entry.area {
            Some(_) => draw.on_hover_text(
                "Draw the rectangle from the minimap tiles under it, as the terrain seen from above, \
                 into the chosen sectors or the whole map. It is shown here before anything is written.",
            ),
            None => draw.on_disabled_hover_text("The whole world has no rectangle to draw. Import a picture for it."),
        };
        if draw.clicked() {
            work.maps.ask = Some(Ask::Draw);
        }
        let removable = own & work.maps.picture.target();
        if removable != 0
            && ui
                .add_enabled(!drawing, egui::Button::new(format!("Remove from project{scope}")))
                .on_hover_text("Remove the project's pieces of these sectors, so the archives' are read again.")
                .clicked()
        {
            work.maps.ask = Some(Ask::Remove);
        }
        if selected != 0 && ui.small_button("Clear sectors").clicked() {
            work.maps.picture.selected = 0;
        }
    });
    theme::note(
        ui,
        match selected {
            0 => "Click sectors of the picture to change only those; with none chosen the whole map changes.",
            _ => "The chosen sectors are outlined. Click a sector again to leave it out.",
        },
    );
    if let Some(drawing) = &work.maps.picture.drawing {
        let progress = &drawing.progress;
        let label = format!("row {} of {}", progress.done(), progress.total());
        theme::progress(ui, progress.fraction(), "minimap", &label, ui.available_width());
        ui.ctx().request_repaint();
    }
    if let Some(said) = &work.maps.said {
        theme::note(ui, said.clone());
    }
    let Some(pending) = work.maps.picture.pending_of(entry.which) else { return };
    theme::heading(ui, "New picture");
    let fit = match &pending.source {
        Source::File { path, width, height, fit, .. } => {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().to_string());
            theme::note(ui, format!("From {name}, {width}x{height}, into {} sectors.", pending.sectors.count_ones()));
            let [l, t, r, b] = pieces::sectors_box(pending.sectors).unwrap_or([0, 0, pieces::WIDTH, pieces::HEIGHT]);
            (!pieces::same_shape(*width, *height, r - l, b - t)).then_some(*fit)
        }
        Source::Minimap { drawn, missing } => {
            theme::note(ui, match missing {
                0 => format!("Drawn from {drawn} minimap tiles, into {} sectors.", pending.sectors.count_ones()),
                m => format!(
                    "Drawn from {drawn} minimap tiles, into {} sectors; {m} under the rectangle have no minimap \
                     picture and are black.",
                    pending.sectors.count_ones()
                ),
            });
            None
        }
    };
    let mut showing_current = work.maps.picture.showing_current;
    ui.allocate_ui(egui::vec2(ui.available_width().min(260.0), 24.0), |ui| {
        theme::segmented(ui, &mut showing_current, &[("New", false), ("Current", true)], |a, b| a == b);
    });
    work.maps.picture.showing_current = showing_current;
    if let Some(mut fit) = fit {
        theme::row_about(
            ui,
            "Fit",
            "The picture is not the shape of the sectors it goes into. Stretch scales it to them; Crop \
             scales it to cover them and keeps the middle.",
            |ui| {
                ui.allocate_ui(egui::vec2(ui.available_width().min(200.0), 24.0), |ui| {
                    theme::segmented(ui, &mut fit, &[("Stretch", Fit::Stretch), ("Crop", Fit::Crop)], |a, b| a == b);
                });
            },
        );
        work.maps.picture.refit(fit);
    }
    ui.horizontal(|ui| {
        let write = ui
            .add_sized(
                [140.0, 26.0],
                egui::Button::new(egui::RichText::new("Write to map").color(egui::Color32::from_rgb(0x0C, 0x16, 0x1E)))
                    .fill(theme::ACCENT),
            )
            .on_hover_text(format!(
                "Write the changed sectors as pieces in Interface\\WorldMap\\{}\\, in place of the ones read \
                 now. Remove from project undoes it.",
                entry.directory
            ));
        if write.clicked() {
            work.maps.ask = Some(Ask::Write);
        }
        if ui.button("Discard").on_hover_text("Drop the new picture; nothing is written.").clicked() {
            work.maps.picture.pending = None;
        }
    });
}
