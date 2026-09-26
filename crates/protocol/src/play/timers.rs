//! **The bars the server counts down for you** — the breath meter, and its two
//! siblings.
//!
//! Three packets and no state: `SMSG_START_MIRROR_TIMER`,
//! `SMSG_STOP_MIRROR_TIMER` and `SMSG_PAUSE_MIRROR_TIMER`. They are called
//! *mirror* timers because the client draws them and the server owns them —
//! which is the opposite of the two clocks in [`crate::play::death`] and the same
//! arrangement as [`crate::play::logout`]'s twenty seconds. The client is told a
//! remaining, a duration and a **rate**, and interpolates between statements;
//! it never decides that a breath has run out.
//!
//! ## The interface's half is a Lua call, and it takes six arguments
//!
//! `Interface\FrameXML\MirrorTimer.lua` ships `MirrorTimer_Show(timer, value,
//! maxvalue, scale, paused, label)` and **nothing in either directory calls
//! it** — `UIParent.lua` registers `MIRROR_TIMER_START` and answers it with
//! `MirrorTimer_Show(arg1, arg2, arg3, arg4, arg5, arg6)`, so the C side is the
//! only caller and the six arguments are its contract.
//!
//! `arg1` is a **string**, not the wire's number: the client keeps a
//! three-entry name table — `"EXHAUSTION"`, `"BREATH"`,
//! `"FEIGNDEATH"`, in the wire's own order — and `MirrorTimerColors` is keyed
//! by it. A `%s_LABEL` format over that name is how the sixth argument is
//! built: `EXHAUSTION_LABEL` is "Fatigue" and `BREATH_LABEL`
//! is "Breath" in `GlobalStrings.lua`, and **`FEIGNDEATH_LABEL` is not in the
//! file at all** — so the third timer draws an unlabelled bar, which is the
//! reference's own behaviour and the same rule every other message in this
//! client keeps.
//!
//! ## …and one of the three events is shipped broken
//!
//! `MirrorTimerFrame_OnEvent` answers `MIRROR_TIMER_PAUSE` by first testing
//! `arg1 ~= this.timer` — the *name* — and then reading the same `arg1` as
//! `arg1 > 0`, the paused flag. Both cannot be true of one argument. vmangos
//! says so in its own comment ("Default UI handler for this is bugged, args
//! dont match") and answers a pause with a full `SMSG_START_MIRROR_TIMER`
//! resend instead, so the broken arm is never reached on the reference server.
//! It is still read here, and still raised with the flag as `arg1`, because
//! that is what the client does: reproducing the packet faithfully is not the
//! same as reproducing Blizzard's handler correctly.

use crate::bytes::Reader;

/// Which of the three bars a packet is about.
///
/// The numbering is vmangos' `MirrorTimer::Type` and the names are the client's
/// own table, in the same order — which is the cross-check: two
/// independent sources agreeing on `FATIGUE, BREATH, FEIGNDEATH` at 0, 1, 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorTimer {
    Fatigue,
    Breath,
    FeignDeath,
}

impl MirrorTimer {
    /// The wire's `u32`, or `None` for a type this build has no bar for.
    ///
    /// **Unknown is data, not an error.** vmangos has a fourth timer
    /// (`ENVIRONMENTAL`, the lava tick) and stops sending at
    /// `NUM_CLIENT_TIMERS`, but a different server need not — and the client's
    /// own table is three entries long, so a fourth would index past it.
    pub fn from_wire(kind: u32) -> Option<MirrorTimer> {
        match kind {
            0 => Some(MirrorTimer::Fatigue),
            1 => Some(MirrorTimer::Breath),
            2 => Some(MirrorTimer::FeignDeath),
            _ => None,
        }
    }

    /// The name the interface knows it by — `arg1` of every one of the three
    /// events, and the key into `MirrorTimerColors`.
    pub fn name(self) -> &'static str {
        match self {
            MirrorTimer::Fatigue => "EXHAUSTION",
            MirrorTimer::Breath => "BREATH",
            MirrorTimer::FeignDeath => "FEIGNDEATH",
        }
    }

    /// …and the `GlobalStrings.lua` key for its caption, which the client builds
    /// with the `%s_LABEL` format string beside the name table.
    ///
    /// A key the shipped file does not carry — `FEIGNDEATH_LABEL` is one —
    /// resolves to nothing, and the bar is drawn without a caption.
    pub fn label_key(self) -> String {
        format!("{}_LABEL", self.name())
    }
}

/// `SMSG_START_MIRROR_TIMER`, whole.
///
/// `Player::SendMirrorTimerStart` writes `u32 type, u32 remaining, u32 duration,
/// i32 scale, u8 paused, u32 spellId` and this reads all six, including the two
/// the shipped interface throws away: `spell_id` reaches no `arg` at all
/// (`MirrorTimer_Show` takes six arguments and the sixth is the label), and
/// `paused` is `arg5`.
///
/// **`scale` is a rate and it is signed**, which is the field that makes this a
/// mirror rather than a countdown: `-1` is one second of bar per second of real
/// time going *down*, and `10` — the value every one of these is refilled with —
/// is ten times that going up, which is why surfacing refills the breath bar in
/// a fraction of the time it took to empty. `MirrorTimerFrame_OnUpdate` is
/// `frame.value = frame.value + frame.scale * elapsed` and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MirrorTimerStart {
    pub timer: MirrorTimer,
    /// Milliseconds left on the bar *now*.
    pub remaining_ms: u32,
    /// Milliseconds the bar is long.
    pub duration_ms: u32,
    /// Seconds of bar per second of real time; negative is draining.
    pub scale: i32,
    pub paused: bool,
    /// The aura holding it, or 0. Nothing in `Interface\FrameXML\` reads it.
    pub spell_id: u32,
}

/// See [`MirrorTimerStart`]. `None` for a short body or a timer type this
/// client has no bar for.
pub fn parse_start_mirror_timer(body: &[u8]) -> Option<MirrorTimerStart> {
    let mut r = Reader::new(body);
    if !r.has(4 * 4 + 1 + 4) {
        return None;
    }
    let timer = MirrorTimer::from_wire(r.u32())?;
    Some(MirrorTimerStart {
        timer,
        remaining_ms: r.u32(),
        duration_ms: r.u32(),
        scale: r.u32() as i32,
        paused: r.u8() != 0,
        spell_id: r.u32(),
    })
}

/// `SMSG_STOP_MIRROR_TIMER`: `u32 type` and nothing else. The bar is hidden.
pub fn parse_stop_mirror_timer(body: &[u8]) -> Option<MirrorTimer> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    MirrorTimer::from_wire(r.u32())
}

/// `SMSG_PAUSE_MIRROR_TIMER`: `u32 type, u8 state` — see the module comment for
/// why the shipped handler cannot act on both.
pub fn parse_pause_mirror_timer(body: &[u8]) -> Option<(MirrorTimer, bool)> {
    let mut r = Reader::new(body);
    if !r.has(4 + 1) {
        return None;
    }
    let timer = MirrorTimer::from_wire(r.u32())?;
    Some((timer, r.u8() != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole packet, field for field against `SendMirrorTimerStart`.
    #[test]
    fn a_start_carries_six_fields_and_the_rate_is_signed() {
        let mut body = Vec::new();
        body.extend_from_slice(&1u32.to_le_bytes()); // BREATH
        body.extend_from_slice(&45_000u32.to_le_bytes());
        body.extend_from_slice(&60_000u32.to_le_bytes());
        body.extend_from_slice(&(-1i32).to_le_bytes());
        body.push(0);
        body.extend_from_slice(&0u32.to_le_bytes());

        let start = parse_start_mirror_timer(&body).expect("a whole body parses");
        assert_eq!(start.timer, MirrorTimer::Breath);
        assert_eq!(start.remaining_ms, 45_000);
        assert_eq!(start.duration_ms, 60_000);
        // **Negative, and read as such.** As a `u32` this is 4294967295, which
        // `MirrorTimerFrame_OnUpdate` would multiply by the frame time — a bar
        // that fills to its maximum in the first frame and never moves again.
        assert_eq!(start.scale, -1);
        assert!(!start.paused);
        assert_eq!(start.spell_id, 0);

        // …and a body one byte short is `None` rather than a partial read.
        assert!(parse_start_mirror_timer(&body[..body.len() - 1]).is_none());
    }

    /// **The names are the client's, in the client's order** — the three-entry
    /// table, which is what `MirrorTimerColors` is keyed by and
    /// what `MIRROR_TIMER_STOP`'s `arg1` is compared against. A wrong name here
    /// is a bar that appears and can never be told to go away.
    #[test]
    fn each_timer_names_itself_the_way_the_interface_spells_it() {
        assert_eq!(MirrorTimer::from_wire(0), Some(MirrorTimer::Fatigue));
        assert_eq!(MirrorTimer::from_wire(1), Some(MirrorTimer::Breath));
        assert_eq!(MirrorTimer::from_wire(2), Some(MirrorTimer::FeignDeath));
        // The fourth is vmangos' own lava tick, which it never sends.
        assert_eq!(MirrorTimer::from_wire(3), None);

        assert_eq!(MirrorTimer::Fatigue.name(), "EXHAUSTION");
        assert_eq!(MirrorTimer::Breath.name(), "BREATH");
        assert_eq!(MirrorTimer::FeignDeath.name(), "FEIGNDEATH");
        // …and the caption key is the name with the client's own suffix, which
        // is how `BREATH_LABEL` ("Breath") is reached from a wire value of 1.
        assert_eq!(MirrorTimer::Breath.label_key(), "BREATH_LABEL");
    }

    /// The two short packets, including the one whose handler is broken.
    #[test]
    fn a_stop_is_a_type_and_a_pause_is_a_type_and_a_flag() {
        assert_eq!(
            parse_stop_mirror_timer(&1u32.to_le_bytes()),
            Some(MirrorTimer::Breath)
        );
        assert!(parse_stop_mirror_timer(&[0, 0]).is_none());

        let mut body = 1u32.to_le_bytes().to_vec();
        body.push(1);
        assert_eq!(
            parse_pause_mirror_timer(&body),
            Some((MirrorTimer::Breath, true))
        );
        assert!(parse_pause_mirror_timer(&body[..4]).is_none());
    }
}
