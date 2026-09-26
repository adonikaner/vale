//! `SMSG_WEATHER` — **the one packet that says what the sky is doing**, and the
//! whole of the wire's opinion on it.
//!
//! ```text
//! u32  type      0 fine, 1 rain, 2 snow, 3 storm (sand)
//! f32  grade     0..1, how hard
//! u32  sound     a SoundEntries id, or 0 — the loop to play under it
//! u8   instant   1 = change now, 0 = ramp
//! ```
//!
//! Thirteen bytes, from `Weather::SendWeatherForPlayersInZone` and
//! `SendWeatherUpdateToPlayer`, and the server sends it on every zone change
//! and every time its own `Weather::Update` rolls a new grade for the zone —
//! roughly once every ten minutes of real time. Nothing else on the wire
//! mentions weather: there is no `Weather.dbc` in 1.12, no field on the player,
//! and no packet that asks.
//!
//! ## What the client does with it
//!
//! The reference's handler reads the four fields and sets the weather with
//! `(type, grade, smooth = instant == 0)`, then hands the sound id to the
//! ambience bed. Setting it follows these rules:
//!
//! * **A type outside 0..3 is refused outright**, as is any update while the
//!   `force-weather-type-on` debug CVar has pinned a type.
//! * **The grade is clamped to 0..1**, and the previous grade is kept beside
//!   it: the target, and where the ramp started from. An instant change writes
//!   the target into both.
//! * **The ramp runs at ten seconds per unit of grade**:
//!   `t = (now - started) / 1000`, `duration = |target - from| * 10`, and the
//!   grade in force is a plain lerp of the two at `t / duration` (clamped). So a sky going from clear to a downpour takes ten seconds and
//!   one easing from drizzle to shower takes three.
//! * **Density is not the grade.** What the emitters are handed is
//!   `max(0, (grade - 0.25) * 4/3)`. A grade under a quarter is *no*
//!   particles at all — which is why a zone at "light rain" can be silent
//!   drizzle with the sound running and nothing falling. See
//!   [`Weather::density`].
//!
//! The *sound* half is not this module's: it is one SoundEntries id, handed to
//! the same bed the place's own ambience plays on. See
//! `crate::sound::ambience` in the client, which is where the reference's own
//! routing of it is stated.

use crate::bytes::Reader;

/// What is falling. The wire's four values, named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeatherKind {
    #[default]
    Fine,
    Rain,
    Snow,
    /// `WEATHER_TYPE_STORM` in vmangos, and a sandstorm on the screen: the
    /// reference's third emitter is `Shaders\Vertex\sand.bls` over the same mist
    /// texture, tinted.
    Storm,
}

impl WeatherKind {
    /// The wire's number, or `None` for one the reference refuses.
    pub fn from_wire(kind: u32) -> Option<WeatherKind> {
        Some(match kind {
            0 => WeatherKind::Fine,
            1 => WeatherKind::Rain,
            2 => WeatherKind::Snow,
            3 => WeatherKind::Storm,
            _ => return None,
        })
    }
}

/// One `SMSG_WEATHER`, as stated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    pub kind: WeatherKind,
    /// Clamped to `0..1` on the way in, as the client clamps it.
    pub grade: f32,
    /// A SoundEntries id, or 0 for silence. The nine the server ever sends are
    /// 8533..8538 and 8556..8558 — see the client's `vale weather`.
    pub sound: u32,
    /// Change now rather than ramp. vmangos sends 0 for every packet it builds.
    pub instant: bool,
}

impl Weather {
    /// **What the emitters are handed for a grade** — `max(0, (g - 0.25) * 4/3)`,
    /// which is the one arithmetic in this file that is a rule rather than a
    /// field.
    pub fn density(grade: f32) -> f32 {
        ((grade - DENSITY_FLOOR) * DENSITY_SCALE).max(0.0)
    }

    /// The seconds a ramp between two grades takes — `|to - from| * 10`.
    pub fn ramp_seconds(from: f32, to: f32) -> f32 {
        (to - from).abs() * RAMP_SECONDS_PER_UNIT
    }
}

/// The grade below which nothing falls.
pub const DENSITY_FLOOR: f32 = 0.25;
/// What is left of the range above it, stretched back to 0..1.
pub const DENSITY_SCALE: f32 = 4.0 / 3.0;
/// Seconds of ramp per unit of grade.
pub const RAMP_SECONDS_PER_UNIT: f32 = 10.0;

/// Read one packet. `None` for a short body or a type the reference refuses,
/// which the handler reports on the warnings channel rather than dropping.
pub fn parse_weather(body: &[u8]) -> Option<Weather> {
    let mut r = Reader::new(body);
    if !r.has(13) {
        return None;
    }
    let kind = WeatherKind::from_wire(r.u32())?;
    let grade = r.f32();
    let sound = r.u32();
    let instant = r.u8() != 0;
    if !grade.is_finite() {
        return None;
    }
    Some(Weather {
        kind,
        grade: grade.clamp(0.0, 1.0),
        sound,
        instant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    fn packet(kind: u32, grade: f32, sound: u32, instant: u8) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(kind).f32(grade).u32(sound).u8(instant);
        w.buf
    }

    /// The four fields, in the order `Weather::SendWeatherForPlayersInZone`
    /// writes them — and the grade clamped as `Set` clamps it, so a server
    /// that sends `0.9999` ("go nuts") and one that sends `1.2` are the same
    /// sky.
    #[test]
    fn the_four_fields_read_in_the_servers_order() {
        let w = parse_weather(&packet(1, 0.6667, 8535, 0)).expect("rain");
        assert_eq!(w.kind, WeatherKind::Rain);
        assert!((w.grade - 0.6667).abs() < 1e-6);
        assert_eq!(w.sound, 8535);
        assert!(!w.instant);

        let w = parse_weather(&packet(2, 1.5, 8538, 1)).expect("snow");
        assert_eq!(w.kind, WeatherKind::Snow);
        assert_eq!(w.grade, 1.0, "clamped, as the client clamps it");
        assert!(w.instant);
    }

    /// **A type past 3 is refused, not defaulted** — and a short
    /// body is refused rather than read as fine weather with no sound, which
    /// would silently clear a storm.
    #[test]
    fn a_type_the_reference_refuses_is_refused_here() {
        assert!(parse_weather(&packet(4, 0.5, 0, 0)).is_none());
        assert!(parse_weather(&packet(0, 0.5, 0, 0)[..12]).is_none());
        assert!(parse_weather(&[]).is_none());
    }

    /// **Density is not the grade.** A quarter is the floor and the rest is
    /// stretched — so light rain at a third is a tenth of a downpour, and
    /// drizzle under the floor is sound with nothing falling.
    #[test]
    fn density_starts_a_quarter_of_the_way_up_the_grade() {
        assert_eq!(Weather::density(0.0), 0.0);
        assert_eq!(Weather::density(0.25), 0.0);
        assert!((Weather::density(0.3333) - 0.1111).abs() < 1e-3);
        assert!((Weather::density(1.0) - 1.0).abs() < 1e-6);
        // …and a ramp is ten seconds per unit, so clear to downpour is ten and
        // drizzle to shower is a third of that.
        assert!((Weather::ramp_seconds(0.0, 1.0) - 10.0).abs() < 1e-6);
        assert!((Weather::ramp_seconds(0.6667, 0.3334) - 3.333).abs() < 1e-3);
    }
}
