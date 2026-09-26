//! **The wire** — what has crossed it, how fast, how long the round trip is,
//! what it is made of, and what any one packet contained.
//!
//! This client has a dozen packet-family counters and each exists because its
//! family leaves no other trace: an emote drives an animation and is gone, an
//! `SMSG_AI_REACTION` drives only a sound, so a zero is the sole sign the family
//! is being dropped. What a counter cannot answer is what the session is made
//! of. A stream that is 90% `SMSG_MONSTER_MOVE` and one that is 90%
//! `SMSG_UPDATE_OBJECT` describe two different problems and produce identical
//! counter read-outs.
//!
//! ## What is on the tab
//!
//! * **Bytes and packets each way, with rates.** The rates are computed here
//!   from deltas rather than published, because a rate is a property of the
//!   observation window and the session thread does not know how often anyone
//!   is looking. Counted at the socket's own two funnels, so `recv_until`'s
//!   discards and the four-byte header are both in them — see
//!   [`vale_protocol::socket::world::WireCounts`].
//! * **The latency trace.** One number is a round trip; the shape of sixty of
//!   them is whether the connection is steady, which decides whether a movement
//!   correction is this client's fault or the network's.
//! * **The opcode table**, filtered by direction and by whether anything reads
//!   the opcode, searchable by name, and sorted by whichever column is asked
//!   for. Each row carries its share of the packets and of the bytes.
//! * **The packet capture.** A bounded ring of recent packets kept whole, with
//!   a hex and ASCII view of any one of them. Disarmed by default; see
//!   [`vale_protocol::socket::world::Capture`] for what it costs.
//! * **The counters**, tabulated.
//!
//! ## Sampled, not published
//!
//! [`NetSample`] takes a reading on [`super::SAMPLE_INTERVAL`] while the window
//! is open and nothing at all while it is shut. The traffic map and the capture
//! ring behind it are rebuilt on the session thread on the same interval and
//! handed over as `Arc`s, so the per-frame status clone this reads out of stays
//! free — see `SessionStatus::traffic`.

use bevy_egui::egui;

use crate::world::session::COUNTERS;

use super::{row, Readout, BAD, DIM, GOOD, WARN};

/// How many latency readings to keep. Sixty at a fifth of a second each is the
/// last twelve seconds, which is the window a hitch lives in.
const TRACE: usize = 60;

/// Which column the opcode table is sorted by.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    /// By packet count: what the stream is made of, which is the question the
    /// table exists for and therefore the default.
    #[default]
    Packets,
    /// By bytes, which gives a different order — one `SMSG_UPDATE_OBJECT`
    /// outweighs fifty movement broadcasts.
    Bytes,
    /// By name, for reading one row whose name is already known.
    Name,
}

impl Sort {
    const ALL: [(Sort, &'static str); 3] = [
        (Sort::Packets, "packets"),
        (Sort::Bytes, "bytes"),
        (Sort::Name, "name"),
    ];
}

/// Which rows the opcode table shows.
///
/// The unhandled filter is the one that finds faults: an opcode arriving a
/// thousand times with nothing reading it is a subject this client is deaf to,
/// and from every other counter it looks identical to one that never arrives.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    /// Received opcodes that reached a handler.
    Handled,
    /// Received opcodes that reached none.
    Unhandled,
    /// Opcodes this client sent.
    Sent,
}

impl Filter {
    const ALL: [(Filter, &'static str); 4] = [
        (Filter::All, "all"),
        (Filter::Handled, "handled"),
        (Filter::Unhandled, "unhandled"),
        (Filter::Sent, "sent"),
    ];

    /// Whether a row passes. `handled` is meaningless for an outbound row: a
    /// packet this client wrote is by definition one it meant to write.
    fn admits(self, inbound: bool, handled: bool) -> bool {
        match self {
            Filter::All => true,
            Filter::Handled => inbound && handled,
            Filter::Unhandled => inbound && !handled,
            Filter::Sent => !inbound,
        }
    }
}

/// **Whether the capture has been armed from outside the window** — `--capture`,
/// through [`super::arm_capture`].
///
/// A resource rather than a field of [`NetSample`] because that sample is a
/// `Local` on the draw system and does not exist until the window is opened,
/// and a scripted run arms the ring before anyone opens it. Read once, when the
/// tab is first drawn, so the checkbox agrees with the ring it is reading.
#[derive(bevy::prelude::Resource, Default)]
pub struct CaptureRequest(pub bool);

/// What the panel has accumulated about the wire.
#[derive(Default)]
pub struct NetSample {
    /// The last reading's totals and when it was taken, so the next one can be
    /// a rate.
    at: f32,
    last: Totals,
    /// Packets and bytes a second, each way, over the last interval.
    rates: Rates,
    /// The last [`TRACE`] latency readings, oldest first.
    latency: Vec<u32>,
    pub sort: Sort,
    pub filter: Filter,
    /// A substring the opcode name must contain, case-insensitively. Empty
    /// admits everything.
    pub search: String,
    /// Whether the capture checkbox is ticked. Held here rather than read back
    /// from the session so that the tick responds in the frame it is clicked;
    /// the session's own flag is what [`show`] reports beside it.
    pub capturing: bool,
    /// Which captured packet is open in the byte view, by sequence number.
    ///
    /// A sequence number rather than an index, because the ring moves under the
    /// selection: an index would silently follow the list along and show a
    /// different packet every time one arrived.
    pub selected: Option<u64>,
}

#[derive(Clone, Copy, Default)]
struct Totals {
    bytes_in: u64,
    bytes_out: u64,
    packets_in: u64,
    packets_out: u64,
}

#[derive(Clone, Copy, Default)]
struct Rates {
    bytes_in: f32,
    bytes_out: f32,
    packets_in: f32,
    packets_out: f32,
}

impl NetSample {
    /// Take a reading. Called on [`super::SAMPLE_INTERVAL`] by the window.
    pub fn take(&mut self, now: f32, status: &crate::world::session::WorldStatus) {
        let wire = status.traffic.wire;
        let totals = Totals {
            bytes_in: wire.bytes_in,
            bytes_out: wire.bytes_out,
            packets_in: wire.packets_in,
            packets_out: wire.packets_out,
        };
        let elapsed = now - self.at;
        // The first reading produces no rate: the interval before it is the
        // whole life of the session, and dividing by it would open the panel on
        // a plausible and wrong number. The same reason `WorldClock::last_ms` is
        // an `Option`.
        if self.at > 0.0 && elapsed > 0.0 {
            let per_sec = |now: u64, then: u64| (now.saturating_sub(then)) as f32 / elapsed;
            self.rates = Rates {
                bytes_in: per_sec(totals.bytes_in, self.last.bytes_in),
                bytes_out: per_sec(totals.bytes_out, self.last.bytes_out),
                packets_in: per_sec(totals.packets_in, self.last.packets_in),
                packets_out: per_sec(totals.packets_out, self.last.packets_out),
            };
        }
        self.at = now;
        self.last = totals;
        if status.in_world {
            self.latency.push(status.latency_ms);
            if self.latency.len() > TRACE {
                self.latency.remove(0);
            }
        }
    }
}

pub fn show(
    ui: &mut egui::Ui,
    params: &mut Readout,
    net: &mut NetSample,
    request: &mut CaptureRequest,
) {
    // A capture armed by `--capture` before anyone opened the window. Taken
    // once, so the checkbox opens ticked rather than disagreeing with the ring
    // it is reading — and so that unticking it afterwards really disarms.
    if request.0 {
        net.capturing = true;
        request.0 = false;
    }
    // Destructured rather than field-accessed, because the log-out button at
    // the bottom takes the session mutably while everything above it reads the
    // status, and the borrow checker sees a `&mut Readout` as one thing.
    let Readout { world: status, rig, session, report, .. } = params;
    if session.active.is_none() {
        // Everything about entering a session — the login form, the character
        // list, the errors belonging to them — is the game's own
        // `Interface\GlueXML\`. This panel is a diagnostic surface.
        ui.colored_label(DIM, "No session.");
        return;
    }
    ui.strong("Session");
    row(
        ui,
        "character",
        format!(
            "{} — {} ({}, {})",
            status.character, status.map_name, status.tile.0, status.tile.1
        ),
    );
    let p = status.position;
    row(
        ui,
        "position",
        format!(
            "{:.1}, {:.1}, {:.1}   facing {:.0}°",
            p.x,
            p.y,
            p.z,
            status.orientation.to_degrees()
        ),
    );
    // WoW coordinates, because every other tool in this project — the CLI, the
    // server's own logs — uses them, and a read-out in Bevy's would make
    // cross-checking a translation exercise.
    let t = rig.target;
    row(
        ui,
        "camera",
        format!(
            "{:.1}, {:.1}, {:.1}   yaw {:.0}°  pitch {:.0}°  {:.0}y out",
            t.x,
            t.y,
            t.z,
            rig.yaw.to_degrees(),
            rig.pitch.to_degrees(),
            rig.distance
        ),
    );
    row(ui, "entities", status.entity_count.to_string());
    if !status.in_world {
        ui.colored_label(BAD, "The session has ended.");
    }

    ui.add_space(8.0);
    ui.strong("Wire");
    row(
        ui,
        "received",
        format!(
            "{} in {} packets   ({}/s, {:.0} pkt/s)",
            bytes(net.last.bytes_in),
            net.last.packets_in,
            bytes(net.rates.bytes_in as u64),
            net.rates.packets_in,
        ),
    );
    row(
        ui,
        "sent",
        format!(
            "{} in {} packets   ({}/s, {:.0} pkt/s)",
            bytes(net.last.bytes_out),
            net.last.packets_out,
            bytes(net.rates.bytes_out as u64),
            net.rates.packets_out,
        ),
    );
    // The mean packet size each way. A stream whose inbound mean is near the
    // header size is mostly movement broadcasts; one in the kilobytes is mostly
    // update blocks, and the two want different things looked at.
    row(
        ui,
        "mean size",
        format!(
            "{} received · {} sent",
            bytes(mean(net.last.bytes_in, net.last.packets_in)),
            bytes(mean(net.last.bytes_out, net.last.packets_out)),
        ),
    );
    // How much of the outbound half is movement, and whether a forced change
    // has ever been acknowledged. The second has a four-second deadline —
    // missing one is a kick some seconds later naming a cheat this client never
    // attempted.
    row(
        ui,
        "movement sent",
        format!(
            "{}{}",
            status.movement_sent,
            match status.flag_changes {
                0 => String::new(),
                n => format!("   ·   {n} forced change(s) acknowledged"),
            }
        ),
    );

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.strong("Latency");
        ui.colored_label(DIM, format!("{} ms round trip", status.latency_ms));
    });
    latency_trace(ui, &net.latency);
    // The other half of "the world is arriving late". A stall is the session
    // thread being kept off the CPU past a quarter of a second, which is time
    // the server spent moving everything in view. The two look identical from
    // outside and only one of them is the network's fault.
    ui.colored_label(
        DIM,
        match status.stalls {
            0 => "The session thread has not been starved.".to_string(),
            n => format!(
                "{n} step(s) over 250 ms, worst {} ms. The session thread is being kept \
                 off the CPU; this is not wire latency.",
                status.worst_stall_ms
            ),
        },
    );
    // The one way the world arrives in the wrong place rather than late. A
    // passenger is placed by composing their offset with wherever this client
    // says the deck is, so a deck placed wrongly moves the character and nothing
    // else on this panel changes. Two of the nine transport routes really do
    // teleport twice a cycle, so what this separates is a jump from a jump
    // nothing accounts for. See `SessionStatus::platform_jumps`.
    ui.colored_label(
        DIM,
        match status.platform_jumps {
            0 => "No deck has moved further than a transport can travel.".to_string(),
            n => format!(
                "{n} carry(s) over 250y, worst {:.0}y: a transport teleport, or a deck placed wrongly.",
                status.worst_platform_jump
            ),
        },
    );
    // The other way it arrives late, which costs unbounded time rather than a
    // quarter of a second at a go: the tick's socket-draining slice running out
    // with the stream still coming. Shown as a share of the ticks, because the
    // count says nothing without the denominator.
    ui.colored_label(
        DIM,
        match (status.read_slice_overruns, status.ticks) {
            (0, _) => "The socket drains inside every tick.".to_string(),
            (n, 0) => format!("{n} tick(s) ran out of read slice."),
            (n, t) => format!(
                "{n} of {t} ticks ({:.1}%) ran out of read slice. The backlog is in the kernel, \
                 and the whole world is being read late.",
                n as f32 / t as f32 * 100.0
            ),
        },
    );

    ui.add_space(8.0);
    opcodes(ui, status, net);

    ui.add_space(8.0);
    capture(ui, status, session, net);

    ui.add_space(8.0);
    ui.strong("Packet families");
    ui.colored_label(
        DIM,
        "None of these leaves any other trace. A zero is the only sign that one is \
         being dropped.",
    );
    egui::Grid::new("net-counters")
        .num_columns(4)
        .spacing([12.0, 2.0])
        .show(ui, |ui| {
            for (index, (label, field)) in COUNTERS.iter().enumerate() {
                ui.colored_label(DIM, *label);
                ui.label(field(&status.counters).to_string());
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });

    ui.add_space(8.0);
    // …and whatever a pass has to say about the wire.
    for line in report.lines(crate::ui::report::Section::Net) {
        ui.label(line);
    }

    // This is the one control on the panel that is not a diagnostic, and it is
    // the blunt way out rather than the only one: `GameMenuButtonLogout` works
    // (Esc, then Logout) and ends on character select, on the socket it came in
    // on. This drops the session outright, back to the password box, which is
    // deliberately cruder: the game's own button asks the server and waits up to
    // twenty seconds for an answer, so a session wedged mid-logout can still be
    // abandoned here.
    if ui.button("Drop the session").clicked() {
        session.log_out();
    }
}

/// One row of the opcode table, resolved before sorting.
struct OpcodeRow<'a> {
    name: &'a str,
    inbound: bool,
    handled: bool,
    count: u32,
    bytes: u64,
}

/// The opcode table: what the stream is made of, filtered and searchable.
fn opcodes(
    ui: &mut egui::Ui,
    status: &crate::world::session::WorldStatus,
    net: &mut NetSample,
) {
    let traffic = &status.traffic;

    ui.horizontal(|ui| {
        ui.strong("Opcodes");
        ui.colored_label(
            DIM,
            format!(
                "{} received · {} sent",
                traffic.inbound.len(),
                traffic.outbound.len()
            ),
        );
    });

    // The deaf count first, because it is the finding and the table is the
    // evidence. Zero is worth stating: it is the only state in which this client
    // reads everything the server sends it.
    let deaf: u32 = traffic
        .inbound
        .values()
        .filter(|flow| !flow.handled)
        .map(|flow| flow.count)
        .sum();
    let deaf_kinds = traffic.inbound.values().filter(|f| !f.handled).count();
    if deaf == 0 {
        ui.colored_label(GOOD, "Every opcode received has a handler.");
    } else {
        ui.horizontal(|ui| {
            ui.colored_label(
                WARN,
                format!("{deaf} packets of {deaf_kinds} unhandled opcode(s)."),
            );
            if ui.small_button("show only these").clicked() {
                net.filter = Filter::Unhandled;
                net.search.clear();
            }
        });
    }

    // The sort, the filter and the search are the whole interaction: the table
    // runs to a hundred rows and the question decides the order. All three live
    // on [`NetSample`], so they survive a close and reopen the way the tab does.
    ui.horizontal(|ui| {
        ui.colored_label(DIM, "sort");
        for (value, label) in Sort::ALL {
            ui.selectable_value(&mut net.sort, value, label);
        }
    });
    ui.horizontal(|ui| {
        ui.colored_label(DIM, "show");
        for (value, label) in Filter::ALL {
            ui.selectable_value(&mut net.filter, value, label);
        }
    });
    ui.horizontal(|ui| {
        ui.colored_label(DIM, "name");
        ui.add(
            egui::TextEdit::singleline(&mut net.search)
                .desired_width(150.0)
                .hint_text("SMSG_MONSTER"),
        );
        if !net.search.is_empty() && ui.small_button("clear").clicked() {
            net.search.clear();
        }
    });

    let needle = net.search.trim().to_ascii_uppercase();
    let mut rows: Vec<OpcodeRow> = traffic
        .inbound
        .iter()
        .map(|(name, flow)| OpcodeRow {
            name: name.as_str(),
            inbound: true,
            handled: flow.handled,
            count: flow.count,
            bytes: flow.bytes,
        })
        .chain(traffic.outbound.iter().map(|(name, flow)| OpcodeRow {
            name: name.as_str(),
            inbound: false,
            handled: true,
            count: flow.count,
            bytes: flow.bytes,
        }))
        .filter(|r| net.filter.admits(r.inbound, r.handled))
        .filter(|r| needle.is_empty() || r.name.to_ascii_uppercase().contains(&needle))
        .collect();
    match net.sort {
        Sort::Packets => rows.sort_by_key(|row| std::cmp::Reverse(row.count)),
        Sort::Bytes => rows.sort_by_key(|row| std::cmp::Reverse(row.bytes)),
        Sort::Name => rows.sort_by(|a, b| a.name.cmp(b.name)),
    }
    if rows.is_empty() {
        ui.colored_label(
            DIM,
            match traffic.inbound.is_empty() && traffic.outbound.is_empty() {
                true => "Nothing yet. The table fills a fifth of a second after login.",
                false => "No opcode matches this filter.",
            },
        );
        return;
    }

    // The share each row is of what passed the filter, so a row can be read
    // against its neighbours without arithmetic. Taken over the shown rows
    // rather than the whole table, because the filter is the question: with
    // "unhandled" selected, the interesting figure is which of the deaf opcodes
    // dominates, not what they are of the session.
    let shown_packets: u64 = rows.iter().map(|r| u64::from(r.count)).sum();
    let shown_bytes: u64 = rows.iter().map(|r| r.bytes).sum();
    ui.colored_label(
        DIM,
        format!(
            "{} row(s), {} packets, {}",
            rows.len(),
            shown_packets,
            bytes(shown_bytes)
        ),
    );

    egui::ScrollArea::vertical()
        .id_salt("net-opcodes")
        .max_height(220.0)
        .show(ui, |ui| {
            egui::Grid::new("net-opcode-rows")
                .num_columns(5)
                .striped(true)
                .spacing([10.0, 1.0])
                .show(ui, |ui| {
                    for r in rows.iter().take(ROW_LIMIT) {
                        let deaf = r.inbound && !r.handled;
                        ui.colored_label(DIM, if r.inbound { "recv" } else { "sent" });
                        if deaf {
                            ui.colored_label(WARN, r.name)
                                .on_hover_text("Received; no handler reads it.");
                        } else {
                            ui.label(r.name);
                        }
                        ui.monospace(r.count.to_string());
                        ui.monospace(bytes(r.bytes));
                        // The share of the shown packets, as a number and a bar.
                        // The bar is what makes "this stream is mostly one
                        // opcode" readable at a glance, which is the table's
                        // whole purpose.
                        let share = match shown_packets {
                            0 => 0.0,
                            total => r.count as f32 / total as f32,
                        };
                        share_bar(ui, share);
                        ui.end_row();
                    }
                });
        });
    if rows.len() > ROW_LIMIT {
        // Never a silent cap: a table that quietly stopped at two hundred reads
        // as a session with two hundred opcodes in it.
        ui.colored_label(
            DIM,
            format!("{} more row(s) not shown.", rows.len() - ROW_LIMIT),
        );
    }
}

/// A proportion as a labelled bar, for the opcode table's share column.
fn share_bar(ui: &mut egui::Ui, share: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(74.0, 10.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 1.0, egui::Color32::from_black_alpha(70));
    let filled = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width() * share.clamp(0.0, 1.0), rect.height()),
    );
    painter.rect_filled(filled, 1.0, egui::Color32::from_rgb(90, 130, 180));
    painter.text(
        rect.right_center() + egui::vec2(-3.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        format!("{:.1}%", share * 100.0),
        egui::FontId::monospace(9.0),
        DIM,
    );
}

/// The most opcode rows to draw at once.
///
/// 1.12 has ~1,300 opcodes and a long session touches a couple of hundred of
/// them; this is above what any real session produces and below what would make
/// the panel's own draw a measurable part of the frame it is measuring.
const ROW_LIMIT: usize = 200;

/// How many captured packets the list shows before it is scrolled.
const CAPTURE_ROWS: usize = 300;

/// The packet capture: a ring of recent packets, and a byte view of one.
///
/// The counters above say how much of each opcode crossed the wire; they cannot
/// say what any one packet contained. Every parser fault in this project starts
/// from that question, because a field taken at the wrong offset produces a
/// plausible value rather than an error.
fn capture(
    ui: &mut egui::Ui,
    status: &crate::world::session::WorldStatus,
    session: &crate::world::session::Session,
    net: &mut NetSample,
) {
    let snapshot = &status.capture;
    ui.horizontal(|ui| {
        ui.strong("Packet capture");
        ui.colored_label(DIM, "off unless armed; the ring costs one atomic load a packet");
    });

    let was = net.capturing;
    ui.horizontal(|ui| {
        ui.checkbox(&mut net.capturing, "record packets");
        if snapshot.armed {
            ui.colored_label(GOOD, "recording");
        }
    });
    if net.capturing != was {
        if let Some(active) = session.active.as_ref() {
            active.live.capture_traffic(net.capturing);
        }
        // Arming clears the ring on the session thread, so a selection kept
        // across it would name a packet from the previous capture.
        net.selected = None;
    }

    if snapshot.packets.is_empty() {
        ui.colored_label(
            DIM,
            match snapshot.armed {
                true => "Armed. Nothing has crossed the socket yet.",
                false => "Nothing captured. Tick the box to record.",
            },
        );
        return;
    }

    // How much of the stream the ring holds. Once it has wrapped the list is a
    // window onto a longer capture, and saying so is the difference between
    // "these are the packets" and "these are the last few hundred".
    let kept = snapshot.packets.len() as u64;
    ui.colored_label(
        DIM,
        match snapshot.seen > kept {
            true => format!(
                "{kept} of {} packets kept; the oldest are dropped.",
                snapshot.seen
            ),
            false => format!("{kept} packet(s), the whole capture."),
        },
    );

    // The same name filter as the table above it, deliberately: a capture is
    // usually opened to find one opcode among several hundred packets, and
    // sharing the box means the filter typed once applies to both.
    let needle = net.search.trim().to_ascii_uppercase();
    let shown: Vec<&vale_protocol::socket::world::CapturedPacket> = snapshot
        .packets
        .iter()
        .rev()
        .filter(|p| net.filter.admits(p.inbound, handled_by(status, p)))
        .filter(|p| needle.is_empty() || p.name.to_ascii_uppercase().contains(&needle))
        .take(CAPTURE_ROWS)
        .collect();
    if shown.is_empty() {
        ui.colored_label(DIM, "No captured packet matches the filter above.");
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("net-capture-rows")
        .max_height(180.0)
        .show(ui, |ui| {
            egui::Grid::new("net-capture-grid")
                .num_columns(5)
                .striped(true)
                .spacing([10.0, 1.0])
                .show(ui, |ui| {
                    for p in &shown {
                        let open = net.selected == Some(p.sequence);
                        ui.monospace(format!("{:>7.3}s", f64::from(p.at_ms) / 1000.0));
                        ui.colored_label(DIM, if p.inbound { "recv" } else { "sent" });
                        let deaf = p.inbound && !handled_by(status, p);
                        let response = ui
                            .scope(|ui| {
                                if deaf {
                                    ui.style_mut().visuals.override_text_color = Some(WARN);
                                }
                                ui.selectable_label(open, p.name.as_str())
                            })
                            .inner;
                        if response.clicked() {
                            net.selected = match open {
                                true => None,
                                false => Some(p.sequence),
                            };
                        }
                        ui.monospace(format!("{} B", p.length));
                        ui.colored_label(DIM, format!("#{}", p.sequence));
                        ui.end_row();
                    }
                });
        });
    if snapshot.packets.len() > shown.len() && needle.is_empty() {
        ui.colored_label(
            DIM,
            format!("Showing the newest {} of {}.", shown.len(), snapshot.packets.len()),
        );
    }

    // The byte view of whichever row is open. Looked up by sequence number
    // rather than held, because the ring moves under the selection.
    let Some(sequence) = net.selected else {
        ui.colored_label(DIM, "Click a packet to read its body.");
        return;
    };
    let Some(packet) = snapshot.packets.iter().find(|p| p.sequence == sequence) else {
        // The selection aged out of the ring. Said rather than silently
        // cleared, so a packet that scrolled away is distinguishable from one
        // that was never selected.
        ui.colored_label(DIM, format!("Packet #{sequence} has left the ring."));
        return;
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.strong(packet.name.as_str());
        ui.colored_label(
            DIM,
            format!(
                "opcode {:#06x} ({}) · {} · {} bytes",
                packet.code,
                packet.code,
                if packet.inbound { "received" } else { "sent" },
                packet.length,
            ),
        );
    });
    if packet.inbound && !handled_by(status, packet) {
        ui.colored_label(WARN, "No handler reads this opcode.");
    }
    if packet.truncated() {
        ui.colored_label(
            DIM,
            format!(
                "Showing the first {} bytes of {}.",
                packet.body.len(),
                packet.length
            ),
        );
    }
    if packet.body.is_empty() {
        // A real shape rather than a failure: five of the attack refusals and
        // both corpse queries have no body at all.
        ui.colored_label(DIM, "Empty body.");
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("net-capture-bytes")
        .max_height(200.0)
        .show(ui, |ui| {
            for line in hex_dump(&packet.body) {
                ui.monospace(line);
            }
        });
}

/// Whether anything reads this packet's opcode, from the traffic table.
///
/// Handled-ness is a property of the opcode rather than of the packet, so the
/// capture stores none and this joins the two. That keeps the capture out of
/// the dispatch, which is where it would otherwise have to be to know.
fn handled_by(
    status: &crate::world::session::WorldStatus,
    packet: &vale_protocol::socket::world::CapturedPacket,
) -> bool {
    status
        .traffic
        .inbound
        .get(&packet.name)
        .is_none_or(|flow| flow.handled)
}

/// How many bytes a hex-dump line holds.
const DUMP_WIDTH: usize = 16;

/// A body as offset, hex and ASCII — the form every other tool prints it in, so
/// a line here can be compared with a server log or a `Reader` walk directly.
fn hex_dump(body: &[u8]) -> Vec<String> {
    body.chunks(DUMP_WIDTH)
        .enumerate()
        .map(|(row, chunk)| {
            let mut hex = String::with_capacity(DUMP_WIDTH * 3);
            for (index, byte) in chunk.iter().enumerate() {
                // A gap at the halfway mark, which is what makes a 64-bit guid
                // countable by eye.
                if index == DUMP_WIDTH / 2 {
                    hex.push(' ');
                }
                hex.push_str(&format!("{byte:02x} "));
            }
            let ascii: String = chunk
                .iter()
                .map(|b| match b.is_ascii_graphic() || *b == b' ' {
                    true => *b as char,
                    false => '.',
                })
                .collect();
            format!(
                "{:04x}  {:<width$} |{ascii}|",
                row * DUMP_WIDTH,
                hex,
                width = DUMP_WIDTH * 3 + 1
            )
        })
        .collect()
}

/// Bytes per packet, or zero for a direction with none.
fn mean(bytes: u64, packets: u64) -> u64 {
    match packets {
        0 => 0,
        n => bytes / n,
    }
}

/// Round-trip time as a bar per reading.
///
/// Bars rather than a line, because latency is a series of discrete answers —
/// each one is a `CMSG_PING` that came back — and a line drawn between them
/// implies a continuous quantity that was never measured. The scale is the
/// window's own worst so the trace always fills its box, with 100 ms marked,
/// which is where a 1.12 server starts feeling remote.
fn latency_trace(ui: &mut egui::Ui, samples: &[u32]) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(60));
    if samples.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "no ping yet",
            egui::FontId::monospace(10.0),
            DIM,
        );
        return;
    }
    let peak = samples.iter().copied().max().unwrap_or(1).max(1) as f32;
    let width = rect.width() / samples.len() as f32;
    for (index, ms) in samples.iter().enumerate() {
        let height = rect.height() * (*ms as f32 / peak);
        let x = rect.left() + width * index as f32;
        let bar = egui::Rect::from_min_max(
            egui::pos2(x, rect.bottom() - height),
            egui::pos2(x + (width - 1.0).max(1.0), rect.bottom()),
        );
        let colour = match *ms {
            0..=79 => egui::Color32::from_rgb(110, 190, 130),
            80..=199 => egui::Color32::from_rgb(200, 180, 90),
            _ => egui::Color32::from_rgb(210, 110, 100),
        };
        painter.rect_filled(bar, 0.0, colour);
    }
    if peak >= 100.0 {
        let y = rect.bottom() - rect.height() * (100.0 / peak);
        painter.hline(
            rect.x_range(),
            y,
            egui::Stroke::new(1.0, egui::Color32::from_rgb(150, 150, 90)),
        );
    }
    painter.text(
        rect.right_top() + egui::vec2(-4.0, 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{peak:.0} ms"),
        egui::FontId::monospace(10.0),
        DIM,
    );
}

/// A byte count a person can read at a glance.
///
/// Binary units, because they are what a socket buffer is measured in and what
/// every other tool this is cross-checked against prints.
pub fn bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    match n {
        n if n >= MIB => format!("{:.1} MiB", n as f64 / MIB as f64),
        n if n >= KIB => format!("{:.1} KiB", n as f64 / KIB as f64),
        n => format!("{n} B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::session::WorldStatus;

    /// The first reading produces no rate. The interval before it is the whole
    /// life of the session, so dividing by it opens the panel on a plausible
    /// and wrong number, which is worse than a zero because nobody doubts it.
    #[test]
    fn the_first_sample_is_a_baseline_and_not_a_rate() {
        let mut sample = NetSample::default();
        let mut status = WorldStatus::default();
        let mut traffic = vale_protocol::socket::world::Traffic::default();
        traffic.wire.bytes_in = 100_000;
        traffic.wire.packets_in = 900;
        status.traffic = std::sync::Arc::new(traffic);

        sample.take(10.0, &status);
        assert_eq!(sample.rates.bytes_in, 0.0, "no interval to divide by yet");
        assert_eq!(sample.last.bytes_in, 100_000, "but the baseline is taken");

        // A second reading a fifth of a second later: 1,024 bytes in 0.2 s.
        let mut traffic = vale_protocol::socket::world::Traffic::default();
        traffic.wire.bytes_in = 101_024;
        traffic.wire.packets_in = 910;
        status.traffic = std::sync::Arc::new(traffic);
        sample.take(10.2, &status);
        assert!((sample.rates.bytes_in - 5120.0).abs() < 1.0, "{}", sample.rates.bytes_in);
        assert!((sample.rates.packets_in - 50.0).abs() < 0.5);
    }

    /// A counter that goes backwards — which is what a reconnect looks like,
    /// since the wire counts belong to the socket and a new session starts a
    /// new one — must not produce a negative rate or an underflow panic.
    #[test]
    fn a_reset_counter_does_not_underflow() {
        let mut sample = NetSample::default();
        let mut status = WorldStatus::default();
        let mut traffic = vale_protocol::socket::world::Traffic::default();
        traffic.wire.bytes_in = 100_000;
        status.traffic = std::sync::Arc::new(traffic);
        sample.take(10.0, &status);

        status.traffic = std::sync::Arc::new(vale_protocol::socket::world::Traffic::default());
        sample.take(10.2, &status);
        assert_eq!(sample.rates.bytes_in, 0.0);
        assert_eq!(sample.last.bytes_in, 0, "the baseline follows the new socket");
    }

    /// The latency trace is bounded — an unbounded history on a panel somebody
    /// leaves open all session is a leak with a graph on it.
    #[test]
    fn the_latency_trace_is_bounded_and_only_fills_in_world() {
        let mut sample = NetSample::default();
        let mut status = WorldStatus::default();
        status.latency_ms = 42;

        // Not in the world: nothing to plot, and a zero would read as a perfect
        // connection rather than as no connection.
        for _ in 0..10 {
            sample.take(1.0, &status);
        }
        assert!(sample.latency.is_empty());

        status.in_world = true;
        for tick in 0..(TRACE + 20) {
            sample.take(2.0 + tick as f32 * 0.2, &status);
        }
        assert_eq!(sample.latency.len(), TRACE);
    }

    /// The byte formatter crosses both thresholds where a reader expects it to,
    /// and stays in binary units — the same ones every other tool this gets
    /// cross-checked against prints.
    #[test]
    fn bytes_reads_as_a_person_would_say_it() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1024), "1.0 KiB");
        assert_eq!(bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(bytes(1536 * 1024), "1.5 MiB");
    }

    /// The four filters partition the table the way the labels claim.
    ///
    /// `handled` is meaningless outbound — a packet this client wrote is one it
    /// meant to write — so an outbound row must never appear under either of
    /// the two inbound filters, whatever flag it carries.
    #[test]
    fn the_filters_partition_the_table() {
        // (inbound, handled) against each filter.
        let cases = [
            (true, true, [true, true, false, false]),
            (true, false, [true, false, true, false]),
            (false, true, [true, false, false, true]),
        ];
        for (inbound, handled, expected) in cases {
            for ((filter, label), want) in Filter::ALL.into_iter().zip(expected) {
                assert_eq!(
                    filter.admits(inbound, handled),
                    want,
                    "{label} with inbound={inbound} handled={handled}"
                );
            }
        }
        // …and every received row appears under exactly one of the two inbound
        // filters, which is what makes the pair a partition rather than two
        // overlapping views.
        for handled in [true, false] {
            let under = [Filter::Handled, Filter::Unhandled]
                .into_iter()
                .filter(|f| f.admits(true, handled))
                .count();
            assert_eq!(under, 1, "handled={handled}");
        }
    }

    /// The hex dump is offset, hex and ASCII, sixteen bytes to the line, with
    /// the last line short rather than padded with phantom bytes.
    #[test]
    fn the_hex_dump_lays_a_body_out_in_rows_of_sixteen() {
        // A guid-shaped body: eight bytes, then four, then a short tail.
        let body: Vec<u8> = (0u8..20).collect();
        let lines = hex_dump(&body);
        assert_eq!(lines.len(), 2, "20 bytes is two rows of sixteen");
        assert!(lines[0].starts_with("0000  00 01 02 03 04 05 06 07  08"), "{}", lines[0]);
        assert!(lines[1].starts_with("0010  10 11 12 13 "), "{}", lines[1]);
        // The ASCII column marks unprintable bytes rather than omitting them,
        // so the two columns stay in step with each other.
        assert!(lines[0].ends_with("|................|"), "{}", lines[0]);

        // Printable bytes come through as themselves, which is what makes a
        // name inside a packet findable by eye.
        let lines = hex_dump(b"Growlfang");
        assert!(lines[0].ends_with("|Growlfang|"), "{}", lines[0]);

        // An empty body is no rows rather than one blank one.
        assert!(hex_dump(&[]).is_empty());
    }

    /// An opcode nothing has received is treated as handled rather than as
    /// deaf. The traffic table only carries rows for opcodes that arrived, so a
    /// missing row means "no evidence", and marking it as a fault would paint
    /// every outbound packet in the capture as unhandled.
    #[test]
    fn an_unseen_opcode_is_not_reported_as_deaf() {
        use vale_protocol::socket::world::{CapturedPacket, OpcodeFlow, Traffic};
        let mut traffic = Traffic::default();
        traffic.inbound.insert(
            "SMSG_MONSTER_MOVE".to_string(),
            OpcodeFlow { count: 3, bytes: 90, handled: true },
        );
        traffic.inbound.insert(
            "SMSG_AI_REACTION".to_string(),
            OpcodeFlow { count: 1, bytes: 12, handled: false },
        );
        let mut status = WorldStatus::default();
        status.traffic = std::sync::Arc::new(traffic);

        let packet = |name: &str, inbound: bool| CapturedPacket {
            sequence: 0,
            at_ms: 0,
            code: 1,
            name: name.to_string(),
            inbound,
            body: Vec::new(),
            length: 0,
        };
        assert!(handled_by(&status, &packet("SMSG_MONSTER_MOVE", true)));
        assert!(!handled_by(&status, &packet("SMSG_AI_REACTION", true)));
        assert!(
            handled_by(&status, &packet("CMSG_PING", false)),
            "an opcode with no inbound row is not evidence of a missing handler"
        );
    }
}
