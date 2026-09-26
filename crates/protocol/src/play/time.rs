//! `SMSG_LOGIN_SETTIMESPEED` — **the world's clock, and the only packet that
//! carries it.**
//!
//! The server states the time once, at login, and never mentions it again: it is
//! sent from `Player::SendInitialPacketsBeforeAddToMap` and appears nowhere else
//! in vmangos. Everything after that is the client's own arithmetic, which is
//! why this module carries a rate as well as a reading.
//!
//! ```text
//! u32  packed date and time
//! f32  game speed — minutes of world time per second of real time
//! ```
//!
//! **The date is six fields bit-packed into one word**, and the layout is
//! `secsToTimeBitFields` in `shared/Util.h` rather than a guess:
//!
//! ```text
//! bits  0..5   minute      0..59
//! bits  6..10  hour        0..23
//! bits 11..13  weekday     0 = Sunday
//! bits 14..19  day of month, zero-based
//! bits 20..23  month, zero-based
//! bits 24..31  year - 2000
//! ```
//!
//! Only the hour and the minute are used by anything so far — they are what
//! `Light.dbc`'s bands are indexed by — but the whole word is parsed, because a
//! field read out of the middle of a packed word is exactly the kind of thing
//! that is checkable while the packet is in front of you and expensive to
//! rediscover.
//!
//! **The rate is 0.01666667 and that is 1/60**, which makes a world minute a
//! real minute and a world day a real day. It is `sWorld.GetGameTime()`'s own
//! rate and it is a *server config*, not a constant — a server running an
//! accelerated day sends a bigger number — so it is read and kept rather than
//! assumed.

use crate::bytes::Reader;

/// A day, in the half-minute unit `LightIntBand`'s times are stated in.
///
/// The same constant as `vale_assets::tables::light::DAY`, and deliberately not
/// shared with it: this crate does not depend on the asset crate, and a protocol
/// module reaching into a file-format module to borrow a number would be the
/// wrong direction for the sake of one integer.
pub const DAY_HALF_MINUTES: u32 = 2880;

/// World minutes per real second on an unmodified vmangos — `0.01666667`, which
/// is 1/60 and makes a world day a real day. Used only when the packet arrives
/// without its second field; see [`parse_login_set_time_speed`].
pub const DEFAULT_SPEED: f32 = 1.0 / 60.0;

/// The world's date and time as the server states it, plus how fast it runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameTime {
    pub year: u32,
    /// Zero-based, as the wire states it — January is 0.
    pub month: u32,
    /// Zero-based day of the month, likewise.
    pub day: u32,
    /// 0 is Sunday.
    pub weekday: u32,
    pub hour: u32,
    pub minute: u32,
    /// World minutes per real second. 1/60 on an unmodified vmangos.
    pub speed: f32,
}

impl GameTime {
    /// The time of day in half-minutes past midnight — the unit the light bands
    /// are indexed by, and the only thing anything currently asks this for.
    pub fn half_minutes(&self) -> u32 {
        (self.hour * 60 + self.minute) * 2 % DAY_HALF_MINUTES
    }

    /// The same clock `real_ms` milliseconds later.
    ///
    /// **The client runs this clock itself, because the server never says it
    /// again.** Wrapping is the whole of the arithmetic: a session that outlasts
    /// midnight has to roll over, and the light bands already interpolate across
    /// that boundary — so a clock that saturated at 23:59 would freeze the
    /// world at dusk for the rest of the evening.
    pub fn advanced(&self, real_ms: u64) -> GameTime {
        let minutes = self.speed as f64 * (real_ms as f64 / 1000.0);
        let total = (self.hour * 60 + self.minute) as f64 + minutes;
        let wrapped = total.rem_euclid(24.0 * 60.0) as u32;
        GameTime {
            hour: wrapped / 60,
            minute: wrapped % 60,
            // The date is left where it was. Rolling it over means knowing how
            // many days have passed and how long a month is, and nothing reads
            // it — a client that displayed a date would want to ask the server
            // again rather than count.
            ..*self
        }
    }
}

/// `SMSG_LOGIN_SETTIMESPEED`: the packed word and the rate.
pub fn parse_login_set_time_speed(body: &[u8]) -> Option<GameTime> {
    let mut r = Reader::new(body);
    if r.remaining() < 4 {
        return None;
    }
    let packed = r.u32();
    // The rate is the second field and a session that somehow arrives without
    // it is not a reason to lose the time: 1/60 is what every 1.12 server
    // sends, and a frozen clock is worse than a clock running at the usual rate.
    let speed = if r.remaining() >= 4 {
        r.f32()
    } else {
        DEFAULT_SPEED
    };
    Some(GameTime {
        minute: packed & 0x3F,
        hour: (packed >> 6) & 0x1F,
        weekday: (packed >> 11) & 0x07,
        day: (packed >> 14) & 0x3F,
        month: (packed >> 20) & 0x0F,
        year: 2000 + ((packed >> 24) & 0xFF),
        speed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packed the way `secsToTimeBitFields` packs it, so the test states the
    /// layout independently of the parser rather than restating its shifts.
    fn pack(year: u32, month: u32, day: u32, weekday: u32, hour: u32, minute: u32) -> u32 {
        ((year - 2000) << 24) | (month << 20) | (day << 14) | (weekday << 11) | (hour << 6) | minute
    }

    fn body(packed: u32, speed: f32) -> Vec<u8> {
        let mut out = packed.to_le_bytes().to_vec();
        out.extend_from_slice(&speed.to_le_bytes());
        out
    }

    /// **The six fields come out where they went in**, which is the check that
    /// matters: every one of them is a slice of one word, and a shift that is
    /// wrong by one gives a plausible time rather than a failure.
    #[test]
    fn the_packed_word_unpacks_into_six_fields() {
        // 2026-07-27 is a Monday; the wire is zero-based on month and day.
        let packed = pack(2026, 6, 26, 1, 14, 37);
        let t = parse_login_set_time_speed(&body(packed, 1.0 / 60.0)).expect("a time");
        assert_eq!(
            (t.year, t.month, t.day, t.weekday, t.hour, t.minute),
            (2026, 6, 26, 1, 14, 37),
        );
        assert!((t.speed - 1.0 / 60.0).abs() < 1e-7, "vmangos' own rate");
    }

    /// The two extremes of every field at once — the case a one-bit-too-narrow
    /// mask fails and a mid-range time does not.
    #[test]
    fn the_last_minute_of_the_year_survives_the_masks() {
        let packed = pack(2255, 11, 30, 6, 23, 59);
        let t = parse_login_set_time_speed(&body(packed, 1.0)).expect("a time");
        assert_eq!((t.hour, t.minute, t.weekday), (23, 59, 6));
        assert_eq!((t.year, t.month, t.day), (2255, 11, 30));
    }

    /// Half-minutes are what the light bands index by: noon is 1,440 of them,
    /// which is neither 720 nor 43,200.
    #[test]
    fn the_time_of_day_converts_to_the_light_tables_unit() {
        let noon = parse_login_set_time_speed(&body(pack(2026, 0, 0, 0, 12, 0), 1.0)).unwrap();
        assert_eq!(noon.half_minutes(), 1440);
        let midnight = parse_login_set_time_speed(&body(pack(2026, 0, 0, 0, 0, 0), 1.0)).unwrap();
        assert_eq!(midnight.half_minutes(), 0);
    }

    /// **At vmangos' rate a world minute is a real minute**, which is the whole
    /// content of the speed field and the reason a day/night cycle needs no
    /// scaling anywhere else.
    #[test]
    fn the_clock_runs_at_one_minute_per_minute() {
        let t = parse_login_set_time_speed(&body(pack(2026, 0, 0, 0, 12, 0), 1.0 / 60.0)).unwrap();
        let hour_later = t.advanced(60 * 60 * 1000);
        assert_eq!((hour_later.hour, hour_later.minute), (13, 0));
    }

    /// And it wraps, because a session outlasts midnight — the case a clamp
    /// would leave stuck at dusk. See [`GameTime::advanced`].
    #[test]
    fn the_clock_wraps_past_midnight_rather_than_stopping() {
        let t = parse_login_set_time_speed(&body(pack(2026, 0, 0, 0, 23, 30), 1.0 / 60.0)).unwrap();
        let after = t.advanced(60 * 60 * 1000);
        assert_eq!((after.hour, after.minute), (0, 30));
        assert_eq!(after.half_minutes(), 60);
    }

    /// A truncated body is data, not a panic — see the crate's own rule.
    #[test]
    fn a_short_body_is_refused_rather_than_read_crooked() {
        assert_eq!(parse_login_set_time_speed(&[1, 2, 3]), None);
        // The word alone, with the rate missing, keeps the time.
        let t = parse_login_set_time_speed(&pack(2026, 0, 0, 0, 6, 0).to_le_bytes()).unwrap();
        assert_eq!(t.hour, 6);
    }
}
