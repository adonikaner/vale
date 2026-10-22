//! The buttons that play the sounds a sound column names, and stop them.

use bevy_egui::egui;

use crate::tables::sounds::{Heard, Listen};
use crate::ui::tables::Workspace;

/// Whether a row of `table` is a sound a button can play. See
/// [`crate::tables::sounds::Heard`].
pub(super) fn plays_sounds(table: &str) -> bool {
    matches!(
        table,
        "SoundEntries" | "ZoneMusic" | "ZoneIntroMusicTable" | "SoundAmbience"
    )
}

/// The channel the game plays a sound named from `table` on: a zone's music
/// and its intro on the music channel, its ambience on the ambience channel,
/// and anything else as an effect. It sets the volume, and the ambience
/// channel loops.
fn channel_of(table: &str) -> vale_client::sound::mixer::Channel {
    use vale_client::sound::mixer::Channel;
    match table {
        "ZoneMusic" | "ZoneIntroMusicTable" => Channel::Music,
        "SoundAmbience" => Channel::Ambience,
        _ => Channel::Effects,
    }
}

/// The `SoundEntries` rows a row of a sound table plays by day and by night,
/// read from the edited table; 0 where there is none. A `SoundEntries` row is
/// its own sound, and an intro has one for both.
fn sounds_of(work: &Workspace<'_>, table: &str, id: u32) -> [u32; 2] {
    use vale_assets::tables::sound::fields::{ambience, intro, zone_music};
    let at = |field: usize| {
        work.session
            .table(table)
            .and_then(|open| open.u32_at(open.row_of(id)?, field))
            .filter(|entry| *entry != u32::MAX)
            .unwrap_or(0)
    };
    match table {
        "SoundEntries" => [id, 0],
        "ZoneMusic" => [at(zone_music::SOUND), at(zone_music::SOUND + 1)],
        "ZoneIntroMusicTable" => [at(intro::SOUND), 0],
        "SoundAmbience" => [at(ambience::SOUND), at(ambience::SOUND + 1)],
        _ => [0, 0],
    }
}

/// The play buttons for a row of a sound table, each a stop button while its
/// sound plays. `both` draws a day and a night button when the row's two
/// sounds differ; without it, or when they are the same, there is one button
/// and it plays the day's. `from` is the table the reference is on, which
/// decides the channel of a bare `SoundEntries` row.
pub(super) fn sound_buttons(
    ui: &mut egui::Ui,
    work: &mut Workspace<'_>,
    from: &str,
    table: &'static str,
    id: u32,
    both: bool,
) {
    if !plays_sounds(table) {
        return;
    }
    let channel = match table {
        "SoundEntries" => channel_of(from),
        _ => channel_of(table),
    };
    let [day, night] = sounds_of(work, table, id);
    let two = both && night != 0 && night != day;
    for (at_night, entry, when) in [(false, day, "day"), (true, night, "night")] {
        if at_night && !two {
            break;
        }
        let heard = Heard {
            table,
            id,
            night: at_night,
        };
        let on = work.browser.hearing == Some(heard);
        let mark = match on {
            true => "\u{25a0}",
            false => "\u{25b6}",
        };
        let text = match two {
            true => format!("{mark} {when}"),
            false => mark.to_string(),
        };
        let name = match entry {
            0 => String::new(),
            _ => work
                .browser
                .describe_id(work.session, "SoundEntries", entry)
                .map(|label| label.title)
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| format!("SoundEntries {entry}")),
        };
        // One button for a row with two different sounds plays the day one.
        let hover = match (on, two || night == 0 || night == day) {
            (true, _) => format!("Stop {name}"),
            (false, true) => format!("Play {name}"),
            (false, false) => format!("Play {name} (day)"),
        };
        if ui
            .add_enabled(entry != 0, egui::Button::new(text).small())
            .on_hover_text(hover)
            .on_disabled_hover_text("No sound")
            .clicked()
        {
            work.browser.listen = Some(match on {
                true => Listen::Stop,
                false => Listen::Play {
                    heard,
                    entry,
                    channel,
                },
            });
        }
    }
}
