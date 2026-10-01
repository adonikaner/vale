//! Who may be inspected: the rule behind `CanInspect`.
//!
//! The 1.12.1 client allows an inspect when the unit is a player, is the
//! character itself or on the character's side, and is within
//! [`INSPECT_DISTANCE`]. "On the same side" is two tests: neither unit is
//! charmed, and both units' faction templates name the same faction-group mask
//! (`FactionTemplate.dbc` field 3, [`crate::tables::faction::Factions::group`]).
//! So a Horde player cannot inspect an Alliance one, and nobody can inspect a
//! mind-controlled player.
//!
//! `CanInspect` also prints why it refused, through the message table, and it
//! prints at most one line: the first test that fails decides it. A unit that
//! is not a player, and one on the other side, both print
//! `ERR_INVALID_INSPECT_TARGET`, and neither is then range-checked aloud.

/// How close a player must be to be inspected, in yards. The client compares
/// squared distances, against 100.
pub const INSPECT_DISTANCE: f32 = 10.0;

/// What `CanInspect` knows about the unit its token names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    /// The unit is a player.
    pub player: bool,
    /// The unit is the character itself, which skips the side test.
    pub own: bool,
    /// Neither unit is charmed, and both faction templates name the same
    /// faction-group mask.
    pub same_side: bool,
    /// The squared distance between the two units, in yards.
    pub distance_sq: f32,
}

/// `CanInspect`'s answer: whether the unit may be inspected, and the message
/// table key the client prints when it may not.
///
/// `token_given` is whether the token was a non-empty string. A token that
/// names nobody prints `ERR_UNIT_NOT_FOUND` when one was given and
/// `ERR_GENERIC_NO_TARGET` when the string was empty.
pub fn can_inspect(target: Option<&Target>, token_given: bool) -> (bool, Option<&'static str>) {
    let Some(target) = target else {
        let key = if token_given {
            "ERR_UNIT_NOT_FOUND"
        } else {
            "ERR_GENERIC_NO_TARGET"
        };
        return (false, Some(key));
    };
    if !target.player || !(target.own || target.same_side) {
        return (false, Some("ERR_INVALID_INSPECT_TARGET"));
    }
    if target.distance_sq > INSPECT_DISTANCE * INSPECT_DISTANCE {
        return (false, Some("ERR_OUT_OF_RANGE"));
    }
    (true, None)
}

/// Whether two units are on the same side, for [`Target::same_side`]: neither
/// is charmed, and both faction templates resolve to the same group mask. A
/// template the table does not hold is not on anybody's side.
pub fn same_side(
    mine: Option<u32>,
    theirs: Option<u32>,
    charmed: bool,
    group: impl Fn(u32) -> Option<u32>,
) -> bool {
    if charmed {
        return false;
    }
    match (mine.and_then(&group), theirs.and_then(&group)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near_ally() -> Target {
        Target {
            player: true,
            own: false,
            same_side: true,
            distance_sq: 25.0,
        }
    }

    /// A player on the same side within ten yards may be inspected, and
    /// nothing is printed.
    #[test]
    fn a_near_player_on_the_same_side_may_be_inspected() {
        assert_eq!(can_inspect(Some(&near_ally()), true), (true, None));
        let edge = Target { distance_sq: 100.0, ..near_ally() };
        assert_eq!(can_inspect(Some(&edge), true), (true, None));
    }

    /// Each refusal prints its own line, and the first failing test decides.
    #[test]
    fn each_refusal_prints_its_own_line() {
        assert_eq!(can_inspect(None, true), (false, Some("ERR_UNIT_NOT_FOUND")));
        assert_eq!(can_inspect(None, false), (false, Some("ERR_GENERIC_NO_TARGET")));
        let creature = Target { player: false, distance_sq: 900.0, ..near_ally() };
        assert_eq!(
            can_inspect(Some(&creature), true),
            (false, Some("ERR_INVALID_INSPECT_TARGET"))
        );
        let enemy = Target { same_side: false, ..near_ally() };
        assert_eq!(
            can_inspect(Some(&enemy), true),
            (false, Some("ERR_INVALID_INSPECT_TARGET"))
        );
        let far = Target { distance_sq: 100.5, ..near_ally() };
        assert_eq!(can_inspect(Some(&far), true), (false, Some("ERR_OUT_OF_RANGE")));
    }

    /// The character itself passes the side test, whatever its faction.
    #[test]
    fn the_character_may_inspect_itself() {
        let me = Target { own: true, same_side: false, distance_sq: 0.0, ..near_ally() };
        assert_eq!(can_inspect(Some(&me), true), (true, None));
    }

    /// The side is the faction-group mask, and a charm or an unknown template
    /// puts a unit on nobody's side.
    #[test]
    fn the_side_is_the_group_mask() {
        // Templates 1 and 2 are Alliance (mask 3), 5 is Horde (mask 5).
        let group = |t: u32| match t {
            1 | 2 => Some(3),
            5 => Some(5),
            _ => None,
        };
        assert!(same_side(Some(1), Some(2), false, group));
        assert!(!same_side(Some(1), Some(5), false, group));
        assert!(!same_side(Some(1), Some(2), true, group));
        assert!(!same_side(Some(1), Some(99), false, group));
        assert!(!same_side(None, Some(2), false, group));
    }
}
