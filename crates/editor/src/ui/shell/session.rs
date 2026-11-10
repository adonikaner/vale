//! The top bar's Session… button and the popover it opens: hosting or
//! joining a shared session, and who is in it. The view of `crate::collab`.
//!
//! ## The button
//!
//! Its label states the session, as the bar's other buttons state what they
//! open: `Session…` with no session, `Hosting · 3`, `Joining…`,
//! `Joined · Alex · 3`, or `Offline · 192.168.1.20` for a copy of a host's
//! project with no session. A dot before it is in the session's colour: the
//! accent while in one, the warning colour while connecting, fetching or
//! offline, and the bad colour after a session ended with an error.
//!
//! ## The popover
//!
//! With no session, a Host / Join switch over the two forms. In a session,
//! the people in it as list rows, each in their colour, where they are and
//! what tool they hold; a press on a row goes to them, and the host's right
//! click on a row removes that editor. Under the list, the address to give
//! the other editors, and End Session, or Work Offline and Leave.
//!
//! An offline copy shows where it came from, how many edits it keeps to
//! send, Reconnect, and Remove Link. See `crate::collab::offline`.
//!
//! ## Claims
//!
//! In a session, under the people: every claim, checked out or held by
//! unsubmitted edits, with Release on this author's check outs and Break on
//! another's, and Check Out for the tile under the camera. A claim held by
//! edits ends only when they are submitted, withdrawn or taken back. See
//! `vale_collab::checkout`.

use crate::collab::{colour, Ask, Collab, Mode, State};
use vale_collab::Claim;
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
    let offline = collab.link.as_ref().filter(|_| collab.offline());
    let (dot, label) = match &collab.state {
        State::Off | State::Ended(_) if offline.is_some() => {
            let address = offline.map_or("", |link| link.address.as_str());
            let host = address.rsplit_once(':').map_or(address, |(host, _)| host);
            (Some(theme::WARN), format!("Offline \u{b7} {host}"))
        }
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
        State::Off | State::Ended(_) if offline.is_some() => {
            format!(
                "Offline copy of a host's project. {} kept to send at the next Reconnect.",
                crate::collab::counted(collab.pending, "edit", "edits")
            )
        }
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
pub(crate) fn panel(ui: &mut egui::Ui, collab: &mut Collab, project: &str, here: &vale_edit::TileKey) {
    ui.set_width(WIDTH);
    match collab.state.clone() {
        State::Off | State::Ended(_) if collab.offline() => offline(ui, collab, project),
        State::Off | State::Ended(_) => forms(ui, collab, project),
        State::Hosting { project, addr } => {
            title(ui, "Hosting", &project);
            theme::row(ui, "Address", |ui| {
                ui.label(theme::number(addr.to_string()).color(theme::INK));
                if ui.small_button("Copy").on_hover_text("Copy the address for the other editors.").clicked() {
                    ui.ctx().copy_text(addr.to_string());
                }
            });
            theme::note(ui, "Other editors on this network join at this address with the password. Each join saves the project first, so the joining editor starts from the files on disk.");
            people(ui, collab, true);
            claims(ui, collab, here);
            counts(ui, collab);
            ui.add_space(6.0);
            if ui.button("End Session").on_hover_text("Close every connection. The project stays as it is.").clicked() {
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
            theme::note(ui, "Only the files the copy lacks or holds different bytes of are fetched.");
            leave(ui, collab, "Cancel");
        }
        State::Joined { project } => {
            let host = collab.host_name().unwrap_or("the host").to_string();
            title(ui, &format!("Joined {host}'s Session"), &project);
            theme::note(
                ui,
                "Edits and imports reach the other editors as they are made, and theirs reach this editor. \
                 When two editors change the same thing, the change that reaches the host second is undone, \
                 and the status line names the other editor.",
            );
            theme::note(
                ui,
                "Only the host saves, publishes, makes and deletes tiles, adds, removes and renames doodads \
                 and buildings, and adds or removes table records and server rows. Delete asks the host to \
                 delete the files. The Playtest popover names the server a playtest logs in to.",
            );
            people(ui, collab, false);
            claims(ui, collab, here);
            counts(ui, collab);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Work Offline")
                    .on_hover_text("Leave the session and keep the copy open. Its edits are kept and sent at the next Reconnect.")
                    .clicked()
                {
                    collab.ask = Some(Ask::GoOffline);
                }
                if ui
                    .button("Leave")
                    .on_hover_text("Leave the session and open the project that was open before joining. The copy keeps the edits sent to the host.")
                    .clicked()
                {
                    collab.ask = Some(Ask::Leave);
                }
            });
        }
    }
}

/// An offline copy: where it came from, what it keeps, and Reconnect.
fn offline(ui: &mut egui::Ui, collab: &mut Collab, project: &str) {
    let Some(link) = collab.link.clone() else { return };
    title(ui, "Working Offline", project);
    if let State::Ended(reason) = &collab.state {
        ui.label(egui::RichText::new(reason).size(theme::SMALL).color(theme::WARN));
        ui.add_space(2.0);
    }
    let kept = match collab.pending {
        0 => "No edits are kept to send.".to_string(),
        1 => "1 edit is kept to send.".to_string(),
        n => format!("{n} edits are kept to send."),
    };
    theme::note(
        ui,
        format!(
            "{project} is a copy of {} at {}, as of edit {} of its log. {kept} Reconnect fetches what changed on the \
             host, then sends the kept edits; an edit to something that changed is listed as a conflict in \
             the Changes window.",
            link.project, link.address, link.synced
        ),
    );
    ui.add_space(6.0);
    let field = ui.available_width() - theme::LABEL_WIDTH - ui.spacing().item_spacing.x;
    theme::row(ui, "Name", |ui| {
        ui.add(egui::TextEdit::singleline(&mut collab.form.name).hint_text("Shown to the other editors").desired_width(field));
    });
    theme::row(ui, "Password", |ui| {
        ui.add(egui::TextEdit::singleline(&mut collab.form.password).password(true).desired_width(field));
    });
    ui.add_space(6.0);
    if theme::primary(ui, "Reconnect").clicked() {
        collab.ask = Some(Ask::Reconnect);
    }
    ui.add_space(4.0);
    let unlink = ui
        .add_enabled(collab.pending == 0, egui::Button::new("Remove Link").small())
        .on_hover_text(
            "Remove the link to the host, which makes the copy a project of its own. It keeps its files and has a \
             history of its own from then on.",
        )
        .on_disabled_hover_text("Reconnect and send the kept edits first.");
    if unlink.clicked() {
        collab.ask = Some(Ask::Unlink);
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
            ui.checkbox(&mut collab.form.share_database, "Share the world database");
            theme::note(
                ui,
                "With this on, joined editors read this machine's world database and playtest on its realm while \
                 joined; only this editor applies changes to it. The database's location and login are sent to them, \
                 and the session is not encrypted.",
            );
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
                    "{project} is saved and closed while joined, and opened again on leaving. The host's \
                     project is kept here as a copy, so a second join fetches only what changed."
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
            (true, _) => format!("{} (this editor)", person.name),
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

/// Every claim held, and checking out the tile under the camera.
fn claims(ui: &mut egui::Ui, collab: &mut Collab, here: &vale_edit::TileKey) {
    theme::heading(ui, &format!("Claims ({})", collab.holds.len()));
    theme::note(
        ui,
        "A claim keeps every other editor's edits off a tile, a table or a row. An edit claims what it changes \
         until its author submits. A check out claims a tile before any edit, until it is released or its holder \
         submits; a tile checked out before working on it offline does not change before those edits are sent.",
    );
    let mut ask = None;
    for held in &collab.holds {
        let mine = collab.is_me(&held.author.id);
        ui.horizontal(|ui| {
            let who = match mine {
                true => "this editor".to_string(),
                false => held.author.name.clone(),
            };
            let how = match (held.checked_out, held.edited) {
                (true, true) => "checked out, unsubmitted edits",
                (true, false) => "checked out",
                (false, _) => "unsubmitted edits",
            };
            ui.label(egui::RichText::new(format!("{} \u{b7} {who} \u{b7} {how}", held.claim.name())).color(theme::INK));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match (mine, held.checked_out) {
                (true, true) => {
                    let hover = match held.edited {
                        true => "End the check out. The unsubmitted edits still claim it until they are submitted.",
                        false => "End the check out, so other editors may edit it again.",
                    };
                    if ui.small_button("Release").on_hover_text(hover).clicked() {
                        ask = Some(Ask::Release(vec![held.claim.clone()]));
                    }
                }
                (false, true) => {
                    let hover = format!(
                        "End {}'s check out. Edits {} makes to it offline may then conflict with others.",
                        held.author.name, held.author.name
                    );
                    if ui.small_button("Break").on_hover_text(hover).clicked() {
                        ask = Some(Ask::Break(vec![held.claim.clone()]));
                    }
                }
                (_, false) => {}
            });
        });
    }
    let claim = Claim::Tile(here.clone());
    if !collab.holds.iter().any(|held| held.claim == claim) {
        let text = format!("Check Out {}", claim.name());
        if ui.button(text).on_hover_text("Check out the tile under the camera.").clicked() {
            ask = Some(Ask::CheckOut(vec![claim]));
        }
    }
    if ask.is_some() {
        collab.ask = ask;
    }
}

/// The steps shared so far.
fn counts(ui: &mut egui::Ui, collab: &Collab) {
    let mut line = format!(
        "{} sent \u{b7} {} received",
        crate::collab::counted(collab.sent as usize, "edit", "edits"),
        collab.received
    );
    if collab.reverted > 0 {
        line.push_str(&format!(" \u{b7} {} of this editor's undone", collab.reverted));
    }
    theme::note(ui, line);
}

/// The button that leaves a session being joined or joined.
fn leave(ui: &mut egui::Ui, collab: &mut Collab, text: &str) {
    ui.add_space(6.0);
    if ui.button(text).on_hover_text("Leave the session and open the project that was open before joining.").clicked() {
        collab.ask = Some(Ask::Leave);
    }
}

/// Bytes as megabytes, to one decimal.
fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}
