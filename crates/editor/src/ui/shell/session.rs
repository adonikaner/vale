//! The top bar's Session… button and the popover it opens: hosting or
//! joining a shared session, and who is in it. The view of `crate::collab`.
//!
//! ## The button
//!
//! Its label states the session, as the bar's other buttons state what they
//! open: `Session…` with no session, `Hosting · 3`, `Joining…`, or
//! `Joined · Alex · 3`. A dot before it is in the session's colour: the
//! accent while in one, the warning colour while connecting or fetching, and
//! the bad colour after a session ended with an error.
//!
//! ## The popover
//!
//! With no session, a Host / Join switch over the two forms. In a session,
//! the people in it as list rows, each in their colour, where they are and
//! what tool they hold; a press on a row goes to them, and the host's right
//! click on a row removes that editor. Under the list, the address to give
//! the other editors, and End Session or Leave.

use crate::collab::{colour, Ask, Collab, Mode, State};
use crate::ui::widgets::theme;
use bevy_egui::egui;

/// The popover's width, in points.
const WIDTH: f32 = 360.0;

/// A person's colour, as the panel paints it.
pub(crate) fn person_colour(user: vale_collab::UserId) -> egui::Color32 {
    let [r, g, b] = colour(user);
    egui::Color32::from_rgb(r, g, b)
}

/// The top bar's button. Answers its response, for the popover's anchor.
pub(crate) fn button(ui: &mut egui::Ui, collab: &Collab) -> egui::Response {
    let count = collab.people.len();
    let (dot, label) = match &collab.state {
        State::Off => (None, "Session\u{2026}".to_string()),
        State::Ended(_) => (Some(theme::BAD), "Session\u{2026}".to_string()),
        State::Hosting { .. } => (Some(theme::ACCENT), format!("Hosting \u{b7} {count}")),
        State::Connecting { .. } | State::Fetching { .. } => (Some(theme::WARN), "Joining\u{2026}".to_string()),
        State::Joined { .. } => {
            let host = collab.host_name().unwrap_or("host");
            (Some(theme::ACCENT), format!("Joined \u{b7} {host} \u{b7} {count}"))
        }
    };
    // The dot is painted rather than typed: the editor's font has no round
    // bullet glyph. Spaces before the label leave its room.
    let text = match dot {
        Some(_) => format!("    {label}"),
        None => label,
    };
    let hover = match &collab.state {
        State::Off => "Host a shared session on this network, or join one. In a shared session several editors work on one project at once.".to_string(),
        State::Ended(reason) => format!("No session. The last one ended: {reason}"),
        State::Hosting { addr, .. } => format!("Hosting on {addr}. Other editors join at this address."),
        State::Connecting { to } => format!("Connecting to {to}."),
        State::Fetching { .. } => "Fetching the host's project.".to_string(),
        State::Joined { project } => format!("Joined. {project} is a copy of the host's project."),
    };
    let response = ui.button(text).on_hover_text(hover);
    if let Some(dot) = dot {
        let at = response.rect.left_center() + egui::vec2(ui.spacing().button_padding.x + DOT, 0.0);
        ui.painter().circle_filled(at, DOT, dot);
    }
    response
}

/// The radius of the state dot before the button's label, in points.
const DOT: f32 = 3.5;

/// The popover's contents.
pub(crate) fn panel(ui: &mut egui::Ui, collab: &mut Collab, project: &str) {
    ui.set_width(WIDTH);
    match collab.state.clone() {
        State::Off | State::Ended(_) => forms(ui, collab, project),
        State::Hosting { project, addr } => {
            title(ui, "Hosting", &project);
            theme::row(ui, "Address", |ui| {
                ui.label(theme::number(addr.to_string()).color(theme::INK));
                if ui.small_button("Copy").on_hover_text("Copy the address for the other editors.").clicked() {
                    ui.ctx().copy_text(addr.to_string());
                }
            });
            theme::note(ui, "Other editors on this network join at this address with the password. Each join saves your project first, so that the editor joining starts from the files on disk.");
            people(ui, collab, true);
            counts(ui, collab);
            ui.add_space(6.0);
            if ui.button("End Session").on_hover_text("Close every connection. Your project stays as it is.").clicked() {
                collab.ask = Some(Ask::Leave);
            }
        }
        State::Connecting { to } => {
            title(ui, "Joining", &to);
            theme::waiting(ui, format!("Connecting to {to}\u{2026}"));
            leave(ui, collab, "Cancel");
        }
        State::Fetching { project, files, of_files, bytes, of_bytes } => {
            title(ui, "Joining", &project);
            let fraction = match of_bytes {
                0 => 1.0,
                of => bytes as f32 / of as f32,
            };
            let line = format!("{files} of {of_files} files \u{b7} {}", megabytes(bytes));
            theme::progress(ui, fraction, "PROJECT", &line, ui.available_width());
            theme::note(ui, "Only the files your copy lacks or holds different bytes of are fetched.");
            leave(ui, collab, "Cancel");
        }
        State::Joined { project } => {
            let host = collab.host_name().unwrap_or("the host").to_string();
            title(ui, &format!("Joined {host}'s session"), &project);
            theme::note(
                ui,
                "Your edits and imports reach the other editors as you make them, and their edits reach \
                 you. When two editors change the same thing, the change that reaches the host second is \
                 undone, and the status line names the other editor.",
            );
            theme::note(
                ui,
                "Only the host saves, publishes, makes and deletes tiles, and adds or removes doodads, \
                 table records and server rows. Delete asks the host to delete the files. The playtest \
                 login chooses between the local server and the host's server.",
            );
            people(ui, collab, false);
            counts(ui, collab);
            leave(ui, collab, "Leave");
        }
    }
}

/// The heading of a running session: what it is, and of what.
fn title(ui: &mut egui::Ui, what: &str, of: &str) {
    ui.label(egui::RichText::new(what).strong().size(14.0).color(theme::INK));
    ui.label(egui::RichText::new(of).size(theme::SMALL).color(theme::INK_DIM));
    ui.add_space(4.0);
}

/// Host and Join, with no session running.
fn forms(ui: &mut egui::Ui, collab: &mut Collab, project: &str) {
    ui.label(egui::RichText::new("Shared Session").strong().size(14.0).color(theme::INK));
    theme::note(ui, "Several editors work on one project at once over this network. Each edit reaches the other editors as it is made.");
    if let State::Ended(reason) = &collab.state {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(reason).size(theme::SMALL).color(theme::BAD));
    }
    ui.add_space(6.0);
    theme::segmented(ui, &mut collab.form.mode, &[("Host", Mode::Host), ("Join", Mode::Join)], |a, b| a == b);
    ui.add_space(6.0);
    let field = ui.available_width() - theme::LABEL_WIDTH - ui.spacing().item_spacing.x;
    theme::row(ui, "Name", |ui| {
        ui.add(egui::TextEdit::singleline(&mut collab.form.name).hint_text("Shown to the other editors").desired_width(field));
    });
    match collab.form.mode {
        Mode::Host => {
            theme::row(ui, "Password", |ui| {
                ui.add(egui::TextEdit::singleline(&mut collab.form.password).password(true).desired_width(field));
            });
            theme::row(ui, "Port", |ui| {
                ui.add(egui::TextEdit::singleline(&mut collab.form.port).desired_width(60.0));
            });
            let at = crate::collab::lan_address().map_or_else(|| "this machine's address".to_string(), |ip| ip.to_string());
            theme::note(ui, format!("Other editors join at {at}:{} with the password. They work on {project}.", collab.form.port.trim()));
            ui.add_space(6.0);
            if theme::primary(ui, "Start Hosting").clicked() {
                collab.ask = Some(Ask::Host);
            }
        }
        Mode::Join => {
            theme::row(ui, "Host", |ui| {
                ui.add(egui::TextEdit::singleline(&mut collab.form.address).hint_text("192.168.1.20").desired_width(field));
            });
            theme::row(ui, "Password", |ui| {
                ui.add(egui::TextEdit::singleline(&mut collab.form.password).password(true).desired_width(field));
            });
            theme::note(
                ui,
                format!(
                    "{project} is saved and closed while you are joined, and opened again when you leave. \
                     The host's project is kept here as a copy, so a second join fetches only what changed."
                ),
            );
            ui.add_space(6.0);
            if theme::primary(ui, "Join").clicked() {
                collab.ask = Some(Ask::Join);
            }
        }
    }
}

/// The people in the session, one row each.
fn people(ui: &mut egui::Ui, collab: &mut Collab, hosting: bool) {
    theme::heading(ui, &format!("People ({})", collab.people.len()));
    let mut ask = None;
    for (user, person) in &collab.people {
        let me = Some(*user) == collab.me;
        let title = match (me, *user) {
            (true, _) => format!("{} (you)", person.name),
            (false, 0) => format!("{} (host)", person.name),
            (false, _) => person.name.clone(),
        };
        let sub = match &person.presence {
            Some(presence) => format!("{} \u{b7} {}", presence.map, presence.tool),
            None => "Arriving\u{2026}".to_string(),
        };
        let row = theme::list_row(ui, theme::ListRow { title: &title, sub: &sub, trailing: "", tint: theme::INK, picture: true }, false);
        let swatch = row.picture.shrink(7.0);
        ui.painter().circle_filled(swatch.center(), swatch.width() * 0.5, person_colour(*user));
        let mut response = row.response;
        if !me {
            response = response.on_hover_text(format!("Go to {}", person.name));
            if response.clicked() {
                ask = Some(Ask::GoTo(*user));
            }
            if hosting {
                response.context_menu(|ui| {
                    if ui.button(format!("Remove {}", person.name)).clicked() {
                        ask = Some(Ask::Remove(*user));
                        ui.close();
                    }
                });
            }
        }
    }
    if ask.is_some() {
        collab.ask = ask;
    }
}

/// The steps shared so far.
fn counts(ui: &mut egui::Ui, collab: &Collab) {
    let mut line = format!("{} edits sent \u{b7} {} received", collab.sent, collab.received);
    if collab.reverted > 0 {
        line.push_str(&format!(" \u{b7} {} of yours undone", collab.reverted));
    }
    theme::note(ui, line);
}

/// The button that leaves a session being joined or joined.
fn leave(ui: &mut egui::Ui, collab: &mut Collab, text: &str) {
    ui.add_space(6.0);
    if ui.button(text).on_hover_text("Leave the session and open your own project again.").clicked() {
        collab.ask = Some(Ask::Leave);
    }
}

/// Bytes as megabytes, to one decimal.
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
