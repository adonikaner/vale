//! **What floats over an NPC's head** — the quest `!` and `?`, and the green
//! `!` that says you have not been to this flight master before.
//!
//! ## They are models, not textures, and that is the finding
//!
//! `Interface\GossipFrame\AvailableQuestIcon.blp` exists and is the *panel's*
//! icon — the little mark beside a title in the gossip list. What goes over a
//! head in the world is a `.mdx`, one of seven:
//!
//! ```text
//! Interface\Buttons\TalkToMeQuestion_LTBlue.mdx
//! Interface\Buttons\TalkToMeQuestionMark.mdx
//! Interface\Buttons\TalkToMeBlue.mdx
//! Interface\Buttons\TalkToMeGreen.mdx
//! Interface\Buttons\TalkToMeGrey.mdx
//! Interface\Buttons\TalkToMeQuestion_Grey.mdx
//! Interface\Buttons\TalkToMe.mdx
//! ```
//!
//! ## …and which one goes where is two tables
//!
//! An earlier version of this note assigned the four it needed by the colours
//! the game is known for. Both tables are the client's own, and the second of
//! them corrects one of those four.
//!
//! **The model table** has eight entries — a lazily-filled model handle and
//! the path it is loaded from — in this order:
//!
//! ```text
//! slot 0  (none)
//! slot 1  TalkToMe                 gold !
//! slot 2  TalkToMeQuestion_Grey    grey ?
//! slot 3  TalkToMeGrey             grey !
//! slot 4  TalkToMeGreen           green !
//! slot 5  TalkToMeBlue             blue !
//! slot 6  TalkToMeQuestionMark     gold ?
//! slot 7  TalkToMeQuestion_LTBlue   lt ?
//! ```
//!
//! **An eight-entry table turns a `DIALOG_STATUS` into one of those slots**;
//! the client compares the result with what the unit is already wearing, and
//! only reloads on a change. Its contents are `[0, 3, 0, 2, 7, 1, 6, 6]`, which is
//! [`slot_for_status`] and which lands on the `DIALOG_STATUS` enum exactly:
//! nothing for `NONE` and `CHAT`, grey `!` for `UNAVAILABLE`, grey `?` for
//! `INCOMPLETE`, gold `!` for `AVAILABLE`, gold `?` for the two rewards —
//! **and light blue `?` for `REWARD_REP`**, which is the entry the old reading
//! got wrong. That the table is exactly eight long is what says the enum is
//! too, and it is what makes `TalkToMeBlue` and `TalkToMeGreen` unreachable
//! from a quest status.
//!
//! ## The green `!` is the taxi one, and it is a different caller entirely
//!
//! `TalkToMeGreen` is slot 4 and no quest status selects it. What does is
//! `SMSG_TAXINODE_STATUS`'s own handler, which calls the mark loader directly
//! with a slot rather than through the status table, and the rule is short:
//!
//! ```text
//! the unit, by the packet's guid
//! UNIT_NPC_FLAGS bit 3 = FLIGHTMASTER, or nothing happens
//! the packet's one status byte:
//!   known   -> slot 0
//!   unknown -> slot 4, which is TalkToMeGreen
//! ```
//!
//! Two things about it are worth keeping. The `UNIT_NPC_FLAGS` gate is the
//! client's own and it is the same shape the quest handler uses one bit along
//! (bit 1 for `QUESTGIVER`). And **there is one mark per unit**: the client
//! keeps a single slot per unit, so a flight master who is also a quest giver
//! wears whichever of the two packets arrived last, and that is the
//! reference's behaviour rather than a simplification here.

/// The seven models, by the path the archives hold them under.
///
/// `.m2` and not `.mdx`: the client names the authoring extension and the
/// archives hold the compiled one, the same rename every model reference in
/// this game needs.
pub mod model {
    pub const AVAILABLE: &str = r"Interface\Buttons\TalkToMe.m2";
    pub const UNAVAILABLE: &str = r"Interface\Buttons\TalkToMeGrey.m2";
    pub const REWARD: &str = r"Interface\Buttons\TalkToMeQuestionMark.m2";
    pub const INCOMPLETE: &str = r"Interface\Buttons\TalkToMeQuestion_Grey.m2";
    /// **The green `!`: a flight master whose node this character has never
    /// stood at.** See [`super::TAXI_UNDISCOVERED`], which is the slot it sits
    /// in and the only thing that selects it.
    pub const GREEN: &str = r"Interface\Buttons\TalkToMeGreen.m2";
    /// Reward-is-reputation, which the old reading here gave the gold `?`.
    pub const REWARD_REP: &str = r"Interface\Buttons\TalkToMeQuestion_LTBlue.m2";
    /// **Slot 5, and the one this client still has no caller for.** It is
    /// selected by something that is not a quest or a taxi path, and what that
    /// is is not known.
    pub const BLUE: &str = r"Interface\Buttons\TalkToMeBlue.m2";
}

/// The model table, in slot order. Slot 0 is "no mark".
pub const SLOTS: [Option<&str>; 8] = [
    None,
    Some(model::AVAILABLE),
    Some(model::INCOMPLETE),
    Some(model::UNAVAILABLE),
    Some(model::GREEN),
    Some(model::BLUE),
    Some(model::REWARD),
    Some(model::REWARD_REP),
];

/// **The slot an undiscovered flight master wears**.
pub const TAXI_UNDISCOVERED: usize = 4;

/// Which slot each of the eight `DIALOG_STATUS` values selects.
pub const STATUS_SLOTS: [usize; 8] = [0, 3, 0, 2, 7, 1, 6, 6];

/// The model one of the eight slots names, or `None` for slot 0 and for
/// anything past the end of the table.
///
/// Takes a plain index rather than an enum because **that is what the tables
/// are**: two arrays the client indexes with a number, and the number's meaning
/// belongs to whoever read it off the wire. It is also what keeps this crate on
/// the right side of its own boundary — `DialogStatus` is `vale-protocol`'s.
pub fn slot_model(slot: usize) -> Option<&'static str> {
    SLOTS.get(slot).copied().flatten()
}

/// …and which slot a `DIALOG_STATUS` selects — the client's table, verbatim.
///
/// A status past the end of the table answers slot 0, which is the honest
/// reading of a bounds check the client does not appear to make: the enum is
/// eight long and vmangos never sends more.
pub fn slot_for_status(status: u32) -> usize {
    STATUS_SLOTS.get(status as usize).copied().unwrap_or(0)
}

/// **Which model goes over a quest giver's head**, from the two facts that
/// decide it, or `None` for a status that marks nothing.
///
/// Kept as a two-boolean call rather than folded into [`slot_for_status`]
/// because the caller has the wire's enum and this crate must not: `offer` is
/// "there is a quest to take here" against "there is one to hand in", `ready` is
/// gold against grey, and `reputation` is the one status that is neither —
/// finished, and paid in reputation, which the client draws in light blue.
///
/// Three booleans is one more than this project likes seeing in a row, and it
/// is the shape that survives the boundary; the *table* above is the authority
/// and this is the projection of it the quest pass happens to need.
pub fn over_head(offer: bool, ready: bool, reputation: bool) -> Option<&'static str> {
    let slot = match (offer, ready, reputation) {
        (_, _, true) => 7,
        (true, true, _) => 1,
        (true, false, _) => 3,
        (false, true, _) => 6,
        (false, false, _) => 2,
    };
    slot_model(slot)
}

/// How far above a unit's own height the mark floats, in yards.
///
/// **This client's own number**, and the module comment's rule applies: it is
/// not in any file and the code that places it was not followed. Half a yard
/// clears a human's head without floating; the alternative — reading the mark
/// model's own bounding box and stacking it — is what the reference plausibly
/// does and has not been checked.
pub const CLEARANCE: f32 = 0.5;

#[cfg(test)]
mod tests {
    use super::*;

    /// **The status table is the client's and it names the models the colours
    /// say it should** — which is the corroboration that makes the transcription
    /// a measurement rather than eight numbers copied correctly.
    ///
    /// The four sizes are the other half of it: an exclamation is a model whose
    /// name carries no "Question", and the four of those are 14 KB against the
    /// three questions' 36 KB.
    #[test]
    fn every_dialog_status_lands_on_the_model_its_colour_names() {
        // NONE and CHAT: a unit with only gossip is not marked.
        assert_eq!(slot_model(slot_for_status(0)), None);
        assert_eq!(slot_model(slot_for_status(2)), None);
        // UNAVAILABLE and INCOMPLETE are the two grey ones, one of each shape.
        assert_eq!(slot_model(slot_for_status(1)), Some(model::UNAVAILABLE));
        assert_eq!(slot_model(slot_for_status(3)), Some(model::INCOMPLETE));
        // AVAILABLE is the gold `!`, and the two rewards are the gold `?`.
        assert_eq!(slot_model(slot_for_status(5)), Some(model::AVAILABLE));
        assert_eq!(slot_model(slot_for_status(6)), Some(model::REWARD));
        assert_eq!(slot_model(slot_for_status(7)), Some(model::REWARD));
        // **REWARD_REP is the light blue one**, which is the entry the reading
        // this file used to carry got wrong: it gave it the gold `?`.
        assert_eq!(slot_model(slot_for_status(4)), Some(model::REWARD_REP));

        for status in 0..8u32 {
            let Some(path) = slot_model(slot_for_status(status)) else {
                continue;
            };
            let offer = matches!(status, 1 | 5);
            assert_eq!(!path.contains("Question"), offer, "status {status}: {path}");
        }
    }

    /// **No quest status reaches the green `!` or the blue one.** That is the
    /// whole reason the taxi mark needs a caller of its own, and an eight-entry
    /// table that happened to name green would have made this client draw it
    /// over the wrong heads.
    #[test]
    fn the_green_and_blue_marks_are_not_quest_marks() {
        for status in 0..8u32 {
            let path = slot_model(slot_for_status(status));
            assert_ne!(path, Some(model::GREEN), "status {status} took the taxi mark");
            assert_ne!(path, Some(model::BLUE), "status {status} took the blue mark");
        }
        // …and the taxi slot really is the green one.
        assert_eq!(slot_model(TAXI_UNDISCOVERED), Some(model::GREEN));
    }

    /// `over_head` is the projection the quest pass takes, and it agrees with
    /// the table it is a projection of.
    #[test]
    fn the_two_boolean_form_agrees_with_the_table() {
        for status in 0..8u32 {
            let offer = matches!(status, 1 | 5);
            let ready = matches!(status, 4..=7);
            let reputation = status == 4;
            if status != 0 && status != 2 {
                assert_eq!(
                    over_head(offer, ready, reputation),
                    slot_model(slot_for_status(status)),
                    "status {status}"
                );
            }
        }
    }

    /// Every model the tables can name is a `.m2` the renderer can ask for.
    #[test]
    fn every_slot_names_a_model_this_client_can_load() {
        assert_eq!(SLOTS[0], None, "slot 0 is the absence of a mark");
        for slot in 1..SLOTS.len() {
            let path = slot_model(slot).expect("a model");
            assert!(path.ends_with(".m2"), "{path}");
            assert!(path.starts_with(r"Interface\Buttons\"), "{path}");
        }
        // All seven are distinct, which is what says nothing was transcribed
        // twice.
        let mut paths: Vec<&str> = SLOTS.iter().flatten().copied().collect();
        paths.sort_unstable();
        paths.dedup();
        assert_eq!(paths.len(), 7);
    }
}
