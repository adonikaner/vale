//! The view of `crate::import::picked`: a click on a file's row read as a
//! choice of several files, and the right-click menu over the files a
//! right-click acts on.
//!
//! The menu is drawn as the viewport's right-click menu is
//! (`crate::ui::shell::context`), with its items, and closes the same way:
//! when an item is pressed, when Escape is pressed, and when a mouse button is
//! pressed outside it. It is kept in egui's memory under the list's id, apart
//! from the row that opened it, since the list draws only the rows on screen
//! and a row scrolled away has no response to hang a menu on.

use bevy_egui::egui;

use crate::import::picked::{Click, Picked};
use crate::ui::shell::context::WIDTH;

/// The click the modifiers held now make.
pub(super) fn click_of(ui: &egui::Ui) -> Click {
    ui.input(|input| Click::of(input.modifiers.command, input.modifiers.shift))
}

/// What a press on a file's row asked for.
#[derive(Debug, Clone, Copy)]
pub(super) enum Hit {
    /// A click, read with the modifiers held.
    Click(Click),
    /// A right-click at this point, which opens the menu.
    Menu(egui::Pos2),
}

/// What a row's response asked for, if anything. Read in the row's frame;
/// applied with [`apply`] once the list is drawn, which is when the order of
/// the shown files is at hand.
pub(super) fn hit_of(ui: &egui::Ui, response: &egui::Response) -> Option<Hit> {
    if response.secondary_clicked() {
        return Some(Hit::Menu(response.interact_pointer_pos().unwrap_or(response.rect.left_bottom())));
    }
    response.clicked().then(|| Hit::Click(click_of(ui)))
}

/// Apply a press on the row of `path` to the chosen files: a click by
/// [`Picked::click`], over the files `shown` in the list's order; a
/// right-click by [`Picked::right_click`], which also opens the menu `menu`
/// over the files it acts on. Answers the file whose details the part shows
/// next.
pub(super) fn apply(ui: &egui::Ui, hit: Hit, picked: &mut Picked, path: &str, shown: &[&str], menu: egui::Id) -> Option<String> {
    match hit {
        Hit::Click(how) => picked.click(path, how, shown),
        Hit::Menu(at) => {
            let targets = picked.right_click(path);
            ui.data_mut(|data| data.insert_temp(menu, Menu { at, targets }));
            Some(path.to_string())
        }
    }
}

/// An open menu: where it was opened, and the files it acts on.
#[derive(Debug, Clone, Default)]
struct Menu {
    at: egui::Pos2,
    targets: Vec<String>,
}

/// Draw the menu under `id`, if one is open. `items` draws the entries for
/// the files the menu acts on and answers whether one was pressed.
pub(super) fn menu(ctx: &egui::Context, id: egui::Id, items: impl FnOnce(&mut egui::Ui, &[String]) -> bool) {
    let Some(open) = ctx.data(|data| data.get_temp::<Menu>(id)) else {
        return;
    };
    let mut chosen = false;
    let area = egui::Area::new(id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(open.at)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(WIDTH);
                ui.set_max_width(WIDTH);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                chosen = items(ui, &open.targets);
            });
        });
    let outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos))
    });
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if chosen || outside || escape {
        ctx.data_mut(|data| data.remove_temp::<Menu>(id));
    }
}

/// `1 file` or `3 files`.
pub(super) fn files_words(count: usize) -> String {
    match count {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    }
}
