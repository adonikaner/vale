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
//! the other editors, and End Session, or Work Offline and Leave. A joined
//! editor also sets there which server it reads and playtests on (see the
//! "Server writes" section of `crate::collab`).
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
                "The host saves: this editor's Save submits its edits to the host. Only the host publishes \
                 and writes migrations. Delete in the Assets workspace asks the host to delete the files.",
            );
            server(ui, collab);
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
    let take_over = ui.button("Take Over as Host").on_hover_text(
        "Turn this copy into the project, with the copy of the host's log as its history, so this editor can host \
         the project when the host is gone. The edits made offline are applied and logged as this editor's. The \
         other editors then join this editor's address.",
    );
    if take_over.clicked() {
        collab.ask = Some(Ask::TakeOver);
    }
}

/// Which server a joined editor reads and playtests on, and the switch
/// between this machine's and the host's. Only the project's edits are
/// shared; each machine's server and world database stay its own unless the
/// host's is chosen. When the Server panel's writes are disabled, the reason
/// is under the switch. See the "Server writes" section of `crate::collab`.
fn server(ui: &mut egui::Ui, collab: &mut Collab) {
    use crate::collab::PlaytestServer;
    let host = collab.host_name().unwrap_or("Host").to_string();
    let address = collab.host_address().unwrap_or("").to_string();
    let shared = collab.shared_database.is_some();
    theme::heading(ui, "Server");
    let before = collab.playtest_server;
    ui.horizontal(|ui| {
        ui.radio_value(&mut collab.playtest_server, PlaytestServer::Local, "This machine's server");
        ui.radio_value(&mut collab.playtest_server, PlaytestServer::Host, format!("{host}'s server"));
    });
    if collab.playtest_server != before {
        collab.server_chosen = true;
    }
    match (collab.playtest_server, shared) {
        (PlaytestServer::Local, true) => theme::note(
            ui,
            format!("A playtest logs in to the realm in realmlist.wtf, and this editor reads this machine's world database and applies changes to it from the Server panel. {host} shares their world database; to read it instead, choose \"{host}'s server\"."),
        ),
        (PlaytestServer::Local, false) => theme::note(
            ui,
            format!("A playtest logs in to the realm in realmlist.wtf, and this editor reads this machine's world database and applies changes to it from the Server panel. {host} does not share their world database."),
        ),
        (PlaytestServer::Host, true) => theme::note(
            ui,
            format!("A playtest logs in to {host}'s realm at {address}, and this editor reads {host}'s world database. Log in with an account that exists on that server."),
        ),
        (PlaytestServer::Host, false) => theme::note(
            ui,
            format!("A playtest logs in to {host}'s realm at {address}. {host} does not share their world database, so this editor reads this machine's. Log in with an account that exists on that server."),
        ),
    }
    if let Some(refused) = collab.server_gate().refused {
        ui.label(egui::RichText::new(format!("Server panel disabled. {refused}")).size(theme::SMALL).color(theme::WARN));
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
                "Sends each joined editor this machine's world database location and login, over the encrypted \
                 connection. A joined editor that sets \"Server\" in its Session popover to this machine's server \
                 reads this database and playtests on this realm. Only this editor applies changes to this database.",
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
        "The host refuses every other editor's edits to a claimed tile, client table record or server row. An edit \
         claims what it changes until its author submits. A check out claims a tile before any edit, until it is \
         released or its holder submits. Check out a tile before working on it offline, so that no other editor \
         changes it before the offline edits are sent.",
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
