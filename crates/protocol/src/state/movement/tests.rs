use super::*;

#[test]
fn movement_info_round_trips() {
    let info = MovementInfo {
        flags: move_flags::FORWARD | move_flags::JUMPING,
        time: 123_456,
        position: Position {
            x: -10256.0,
            y: -388.5,
            z: 42.25,
            orientation: 1.5,
        },
        fall_time: 800,
        jump: JumpInfo {
            z_speed: 7.95,
            cos_angle: 0.5,
            sin_angle: 0.25,
            xy_speed: 7.0,
        },
        ..Default::default()
    };
    let bytes = info.to_bytes();
    let parsed = MovementInfo::read(&mut Reader::new(&bytes)).expect("parsed");
    assert_eq!(parsed, info);
}

#[test]
fn transport_and_swim_blocks_are_positional() {
    // Both optional blocks present: proves the reader consumes them in the
    // order MovementInfo::Write emits them, not alphabetically.
    let info = MovementInfo {
        flags: move_flags::ONTRANSPORT | move_flags::SWIMMING,
        time: 1,
        transport: Some(TransportInfo {
            guid: 0x1234_5678_9abc,
            position: Position { x: 1.0, y: 2.0, z: 3.0, orientation: 0.5 },
        }),
        pitch: -0.25,
        ..Default::default()
    };
    let parsed = MovementInfo::read(&mut Reader::new(&info.to_bytes())).expect("parsed");
    assert_eq!(parsed, info);
}

/// The near teleport, built exactly as `SendTeleportToController` writes it:
/// packed guid, counter, whole `MovementInfo`.
///
/// Untested until now, which is why it is here beside the far one — this is
/// the packet whose absence got the account banned, and a parse that
/// silently returned `None` would look identical to not handling it at all.
#[test]
fn a_near_teleport_carries_the_position_the_server_moved_us_to() {
    let info = MovementInfo {
        flags: move_flags::NONE,
        time: 9_999,
        position: Position { x: -8463.8, y: -3134.3, z: 8.8, orientation: 1.25 },
        ..Default::default()
    };
    let mut w = crate::bytes::Writer::new();
    w.packed_guid(0x0000_0000_0000_0042);
    w.u32(7);
    info.write(&mut w);

    let tp = parse_teleport(&w.buf).expect("a teleport");
    assert_eq!(tp.guid, 0x42);
    assert_eq!(tp.counter, 7);
    assert_eq!(tp.info.position, info.position);
}

/// **`SMSG_CLIENT_CONTROL_UPDATE` is a packed guid and one byte**, and the two
/// halves answer different questions.
///
/// `Unit::UpdateControl` sends the possessed unit's guid with a 1 when a
/// possess begins, the character's own with a 1 when it ends, and the
/// character's own with a **0** for a fear or a confusion. So a reader that
/// took the guid alone could not tell a release from a fear, and one that took
/// the byte alone could not tell which body the keys are pointed at.
#[test]
fn a_control_update_carries_a_body_and_whether_it_may_be_moved() {
    let packet = |guid: u64, allow: u8| {
        let mut w = crate::bytes::Writer::new();
        w.packed_guid(guid);
        w.u8(allow);
        w.buf
    };
    let possessed = parse_client_control(&packet(0xF130_0000_0000_002A, 1)).expect("a control");
    assert_eq!(possessed.guid, 0xF130_0000_0000_002A);
    assert!(possessed.allow_move);

    let feared = parse_client_control(&packet(0x42, 0)).expect("a control");
    assert_eq!(feared.guid, 0x42);
    assert!(!feared.allow_move, "the server is walking this body itself");

    // A body with no byte on the end is refused rather than read as "and you
    // may not move", which would freeze the character for the rest of the
    // session with nothing to unfreeze it.
    assert!(parse_client_control(&[0x01, 0x2A]).is_none());
    assert!(parse_client_control(&[]).is_none());
    // …and a zero guid names nobody: `CMSG_SET_ACTIVE_MOVER` with one is what
    // `GetConfirmedMover` reads as "no mover client side, is this a fake
    // client?" and answers by dropping every movement packet.
    assert!(parse_client_control(&packet(0, 1)).is_none());
}

/// `Player::SendNewWorld`: `u32 mapId` then four floats, and **no guid** —
/// it can only ever be about us.
#[test]
fn a_far_teleport_carries_the_map_it_is_sending_us_to() {
    let mut w = crate::bytes::Writer::new();
    w.u32(1); // Kalimdor
    w.f32(-8463.8).f32(-3134.3).f32(8.8).f32(2.5);

    let nw = parse_new_world(&w.buf).expect("a new world");
    assert_eq!(nw.map_id, 1);
    assert_eq!(nw.position.x, -8463.8);
    assert_eq!(nw.position.orientation, 2.5);
}

/// **`SMSG_TRANSFER_PENDING` in both of its two lengths**, which is the whole
/// of the packet: four bytes for an ordinary far teleport, twelve when the
/// character is standing on a boat.
///
/// The map id is the thing the loading screen is picked by, so reading the
/// transport tail as part of it — or refusing the twelve-byte form — puts the
/// wrong parchment on the screen rather than none.
#[test]
fn a_pending_transfer_says_which_map_and_nothing_else() {
    let mut w = crate::bytes::Writer::new();
    w.u32(33); // Shadowfang Keep
    let pending = parse_transfer_pending(&w.buf).expect("a pending transfer");
    assert_eq!(pending.map_id, 33);
    assert_eq!(pending.transport, None);

    let mut w = crate::bytes::Writer::new();
    w.u32(1).u32(20808).u32(0); // onto Kalimdor, off a boat, from Azeroth
    let pending = parse_transfer_pending(&w.buf).expect("a pending transfer");
    assert_eq!(pending.map_id, 1);
    assert_eq!(pending.transport, Some((20808, 0)));

    // …and a body with no map in it at all is refused rather than read as
    // "you are going to Azeroth", which would raise the wrong screen and then
    // wait for a map change that never comes.
    assert!(parse_transfer_pending(&[]).is_none());
    assert!(parse_transfer_pending(&[0u8; 3]).is_none());
}

/// A short body is not a teleport to (0, 0, 0). Both parsers answer `None`
/// rather than a plausible position, because the caller's response to `None`
/// — stay put, and still send the ack — is right and the response to a
/// wrong position is a character standing at the centre of the map.
#[test]
fn a_truncated_teleport_is_refused_rather_than_read_as_the_origin() {
    assert!(parse_new_world(&[0u8; 19]).is_none());
    assert!(parse_new_world(&[]).is_none());
    assert!(parse_teleport(&[0x01, 0x01, 0x00]).is_none());
}

/// The same guard the update parser has: a coordinate the server could not
/// have meant is refused, because adopting one strands the character outside
/// every loaded tile with nothing to bring it back.
#[test]
fn a_far_teleport_to_an_impossible_place_is_refused() {
    let mut w = crate::bytes::Writer::new();
    w.u32(0);
    w.f32(f32::NAN).f32(0.0).f32(0.0).f32(0.0);
    assert!(parse_new_world(&w.buf).is_none());

    let mut w = crate::bytes::Writer::new();
    w.u32(0);
    w.f32(1.0e9).f32(0.0).f32(0.0).f32(0.0);
    assert!(parse_new_world(&w.buf).is_none());
}

#[test]
fn starting_to_run_forward_is_one_packet() {
    let events = transitions(0, move_flags::FORWARD);
    assert_eq!(
        events,
        vec![MoveEvent {
            opcode: Opcode::MSG_MOVE_START_FORWARD,
            flags: move_flags::FORWARD
        }]
    );
}

#[test]
fn turning_around_stops_before_it_starts() {
    // CheckMoveStart rejects a packet that introduces BACKWARD under any
    // opcode but MSG_MOVE_START_BACKWARD, so this must be two packets.
    let events = transitions(move_flags::FORWARD, move_flags::BACKWARD);
    assert_eq!(
        events,
        vec![
            MoveEvent { opcode: Opcode::MSG_MOVE_STOP, flags: 0 },
            MoveEvent {
                opcode: Opcode::MSG_MOVE_START_BACKWARD,
                flags: move_flags::BACKWARD
            },
        ]
    );
}

#[test]
fn strafing_while_already_running_keeps_the_forward_flag() {
    let events = transitions(
        move_flags::FORWARD,
        move_flags::FORWARD | move_flags::STRAFE_LEFT,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].opcode, Opcode::MSG_MOVE_START_STRAFE_LEFT);
    // Dropping FORWARD here would read as a fresh strafe start and, worse,
    // make the server extrapolate the wrong heading.
    assert_eq!(
        events[0].flags,
        move_flags::FORWARD | move_flags::STRAFE_LEFT
    );
}

#[test]
fn opposite_keys_cancel() {
    let both = Controls {
        forward: true,
        backward: true,
        ..Default::default()
    };
    assert_eq!(both.to_flags(), move_flags::NONE);
}

#[test]
fn running_forward_covers_run_speed_in_one_second() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.advance(1.0, None);
    // Orientation 0 is +X (north), so a full second of run speed lands
    // exactly one run-speed north. The server checks this to within 10%.
    assert!((m.position().x - BASE_RUN_SPEED).abs() < 0.001, "{:?}", m.position());
    assert!(m.position().y.abs() < 0.001);
}

#[test]
fn strafing_left_moves_ninety_degrees_off_the_facing() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { strafe_left: true, ..Default::default() });
    m.advance(1.0, None);
    assert!(m.position().x.abs() < 0.001, "{:?}", m.position());
    assert!((m.position().y - BASE_RUN_SPEED).abs() < 0.001);
}

#[test]
fn ground_rising_under_the_character_is_stood_on() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    let ground = |_x: f32, _y: f32, _z: f32| Some(50.0f32);
    assert_eq!(m.advance(1.0, Some(&ground)), None);
    // A step up is a step up, however large: at a 25 ms tick a real slope
    // is millimetres, so a big rise is the tile arriving late and standing
    // on it is the least-wrong answer. Falling is the *other* direction.
    assert!((m.position().z - 50.0).abs() < 0.001, "{:?}", m.position());
    assert!(!m.is_airborne());
}

/// **The bug that got the account banned, stated as a property.**
///
/// Ground below the feet used to be closed at a clamped 40 yards a second
/// with no flags set — and `CheckTeleport` calls a z difference over 2.0
/// yards between two flagless packets a teleport hack, which on the
/// reference server is `Threshold = 3`, `Penalty = 19`: banned. The client
/// the server expects falls, and a fall carries `MOVEFLAG_JUMPING`, which
/// is exactly what the check exempts.
#[test]
fn ground_falling_away_is_a_fall_with_a_flag_on_it_rather_than_a_glide() {
    let mut m = Mover::new(Position { z: 50.0, ..Default::default() }, Speeds::default());
    let ground = |_x: f32, _y: f32, _z: f32| Some(0.0f32);

    // The take-off is reported at once, because the server anchors the arc
    // at the first packet carrying the flag.
    assert_eq!(
        m.advance(0.025, Some(&ground)),
        Some(Opcode::MSG_MOVE_HEARTBEAT)
    );
    assert!(m.is_airborne());
    assert!(!m.is_jumping(), "walking off an edge is not a jump");
    assert!(m.info.has(move_flags::JUMPING));
    assert!(m.info.is_moving(), "a fall owes the server heartbeats");

    // …and it accelerates, rather than descending at a fixed rate.
    let first = 50.0 - m.position().z;
    m.advance(0.025, Some(&ground));
    let second = 50.0 - m.position().z - first;
    assert!(second > first * 1.5, "not accelerating: {first} then {second}");

    // The landing is its own opcode: any other packet dropping the flag is
    // `CHEAT_TYPE_BAD_FALL_STOP`.
    let mut landed = None;
    for _ in 0..400 {
        if let Some(op) = m.advance(0.025, Some(&ground)) {
            landed = Some(op);
            break;
        }
    }
    assert_eq!(landed, Some(Opcode::MSG_MOVE_FALL_LAND));
    assert_eq!(m.position().z, 0.0);
    assert!(!m.is_airborne());
    assert!(!m.info.has(move_flags::JUMPING));
    assert_eq!(m.info.fall_time, 0);
    // 50 yards under this gravity is 2.28 s; the flat 40 y/s glide it
    // replaced would have taken 1.25.
    assert!(m.info.fall_time == 0);
}

/// A jump is the same arc with an upward start, and it comes back down on
/// its own — apex `v²/2g` = 1.64 yards, flight `2v/g` = 0.825 s.
#[test]
fn a_jump_leaves_the_ground_and_returns_to_it() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    let ground = |_x: f32, _y: f32, _z: f32| Some(0.0f32);

    let event = m.jump().expect("a jump");
    assert_eq!(event.opcode, Opcode::MSG_MOVE_JUMP);
    assert!(m.is_jumping());
    // The wire is down-positive: the real client reports a jump as -7.9558.
    assert_eq!(m.info.jump.z_speed, -JUMP_SPEED);

    let mut apex: f32 = 0.0;
    let mut flight = 0.0;
    for _ in 0..200 {
        let landed = m.advance(0.01, Some(&ground)).is_some();
        apex = apex.max(m.position().z);
        flight += 0.01;
        if landed {
            break;
        }
    }
    let expected_apex = JUMP_SPEED * JUMP_SPEED / (2.0 * GRAVITY);
    assert!((apex - expected_apex).abs() < 0.05, "apex {apex}");
    assert!((flight - 2.0 * JUMP_SPEED / GRAVITY).abs() < 0.05, "flight {flight}");
    assert!(!m.is_airborne());
}

/// **One jump per landing.** `CHEAT_TYPE_MULTI_JUMP` counts `MSG_MOVE_JUMP`
/// opcodes and resets on `MSG_MOVE_FALL_LAND`; the second is rejected
/// outright on the reference server (`MultiJump.Reject = 1`), so a held key
/// that produced one a tick would have the client flying while the server
/// discarded every packet.
#[test]
fn a_second_jump_before_landing_is_refused() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    assert!(m.jump().is_some());
    assert!(m.jump().is_none());
}

/// A jump carries the horizontal velocity of the stride it left with, and
/// **keeps it** — 1.12 has no air control, and neither does the
/// extrapolation the server checks us against.
#[test]
fn a_jump_keeps_the_velocity_it_took_off_with() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    let run = m.info.speed(&m.speeds);
    m.jump();
    assert!((m.info.jump.xy_speed - run).abs() < 0.001);

    // Let go of everything mid-air: the arc is unchanged.
    m.set_controls(Controls::default());
    let ground = |_x: f32, _y: f32, _z: f32| Some(0.0f32);
    m.advance(0.1, Some(&ground));
    assert!(
        (m.position().x - run * 0.1).abs() < 0.01,
        "{:?}",
        m.position()
    );
    assert!(m.info.has(move_flags::JUMPING), "the flag survived a stop");
}

/// A rooted character does not move and does not *say* it is moving: a
/// packet carrying both is `CHEAT_TYPE_ROOT_MOVE`, five of which is a kick,
/// and the check is sticky once it fires.
#[test]
fn rooting_stops_the_character_and_clears_the_moving_flags() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.set_rooted(true);
    assert!(m.info.has(move_flags::ROOT));
    assert!(!m.info.is_moving());

    m.advance(1.0, None);
    assert_eq!(m.position().x, 0.0, "a rooted character walked");

    // …and the keys are still held, so the unroot puts them back without
    // waiting for the player to let go and press again.
    m.set_rooted(false);
    assert!(!m.info.has(move_flags::ROOT));
    assert!(m.info.has(move_flags::FORWARD));
}

/// **A key pressed *while* rooted reports nothing**, which is the half of the
/// rule above that was missing and the half a player actually reaches: nobody
/// is holding W at the instant a stun lands, they press it *afterwards*.
///
/// `transitions` starts from the flags the block already has and adds to them,
/// so the press put `MOVEFLAG_FORWARD` on top of the server's own
/// `MOVEFLAG_ROOT` and sent it as `MSG_MOVE_START_FORWARD`. That is
/// `CHEAT_TYPE_ROOT_MOVE` exactly — the check reads `currentMoveFlags &
/// MOVEFLAG_MASK_MOVING` against either block's `MOVEFLAG_ROOT` — and it
/// carries both a per-tick and a running penalty. Every stun in the game was
/// one keypress away from an `Anticheat.log` entry.
#[test]
fn a_key_pressed_while_rooted_sends_nothing() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_rooted(true);

    let events = m.set_controls(Controls { forward: true, ..Default::default() });
    assert!(events.is_empty(), "a rooted character announced a move: {events:?}");
    assert!(!m.info.is_moving(), "MASK_MOVING went back into a rooted block");
    assert!(m.info.has(move_flags::ROOT), "the root was cleared by a keypress");

    // Nor does it travel, on either simulation.
    m.advance(1.0, None);
    assert_eq!(m.position().x, 0.0);

    // The press is still *recorded*, so the unroot resumes from the held keys —
    // and that packet is the one the check exempts.
    m.set_rooted(false);
    assert!(m.info.has(move_flags::FORWARD), "the key was forgotten rather than held");
}

/// **A root stops the translation and a stun stops the *turn*, and they are two
/// different fields.**
///
/// Both arrive together for a stun — `HandleAuraModStun` sets
/// `UNIT_FLAG_STUNNED` and then calls `SetRooted(true)` — so obeying only the
/// root leaves a character who cannot walk and can still spin on the spot, which
/// is what a stun looked like. The client's own input tick asks the two
/// separately (see [`crate::state::objects::Entity::is_stunned`], which
/// states it).
#[test]
fn a_root_leaves_the_turn_and_a_stun_takes_it() {
    let keys = Controls { turn_left: true, ..Default::default() };

    let mut rooted = Mover::new(Position::default(), Speeds::default());
    rooted.set_controls(keys);
    rooted.set_rooted(true);
    rooted.advance(1.0, None);
    assert_eq!(rooted.position().x, 0.0, "a rooted character walked");
    assert!(rooted.position().orientation > 0.0, "a rooted character could not turn");
    assert!(rooted.can_turn());

    let mut stunned = Mover::new(Position::default(), Speeds::default());
    stunned.set_controls(keys);
    stunned.set_rooted(true);
    stunned.set_restraint(Restraint { stunned: true, dead: false, on_taxi: false });
    stunned.advance(1.0, None);
    assert_eq!(stunned.position().orientation, 0.0, "a stunned character turned");
    assert!(!stunned.can_turn());

    // **And the mouse goes through the same gate**, which is the half no turn
    // key can reach: the camera drag sets an absolute heading, so a stunned
    // character span freely on the mouse while A and D did nothing.
    assert!(!stunned.face(2.0), "mouse-look turned a stunned character");
    assert_eq!(stunned.position().orientation, 0.0);
    assert!(rooted.face(2.0), "mouse-look was refused to a merely rooted character");
    assert_eq!(rooted.position().orientation, 2.0);
}

/// **A corpse does neither, and its root being lifted does not give either
/// back** — which is the state a release passes through: `BuildPlayerRepop`
/// calls `SetRooted(false)` while the health is still zero.
#[test]
fn a_dead_character_neither_turns_nor_walks_nor_jumps() {
    let keys = Controls { forward: true, turn_left: true, ..Default::default() };
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_restraint(Restraint { stunned: false, dead: true, on_taxi: false });

    assert!(m.set_controls(keys).is_empty(), "a corpse announced a move");
    assert!(!m.info.is_moving());
    assert!(m.jump().is_none(), "a corpse jumped");
    m.advance(1.0, None);
    assert_eq!(m.position().x, 0.0);
    assert_eq!(m.position().orientation, 0.0);

    // The unroot of a *dead* character puts nothing back, because `can_move` is
    // still false: reporting `FORWARD` while `advance` refuses to translate is
    // the shape `CheckSpeedHack` measures.
    m.set_rooted(true);
    m.set_rooted(false);
    assert!(!m.info.is_moving(), "a corpse resumed the keys on an unroot");
}

/// The fall maths is the server's own function, so it is checked against
/// closed form rather than against itself: with no initial speed a fall is
/// `½gt²` until terminal velocity, and a jump is `-vt + ½gt²`.
#[test]
fn the_fall_curve_is_the_servers_own() {
    let drop = fall_elevation(1.0, false, 0.0);
    assert!((drop - GRAVITY * 0.5).abs() < 0.001, "{drop}");

    // A jump's start velocity enters negative — the caller subtracts the
    // result — so at half a second the character is *above* the start.
    let rise = fall_elevation(0.5, false, -JUMP_SPEED);
    assert!(rise < 0.0, "a jump went down: {rise}");
    assert!((rise + (JUMP_SPEED * 0.5 - GRAVITY * 0.25 * 0.5)).abs() < 0.001);

    // Past terminal velocity it is linear: 60.148 yards a second.
    let a = fall_elevation(10.0, false, 0.0);
    let b = fall_elevation(11.0, false, 0.0);
    assert!((b - a - TERMINAL_VELOCITY).abs() < 0.01, "{}", b - a);

    // **Safe fall only clamps the *start* velocity**, and the reference
    // implementation then goes on using the ordinary terminal velocity for
    // the linear part — so with no start velocity the two are the same
    // curve. Asserted rather than corrected: this is a port of what the
    // server runs, and "improving" it would put the two ends of the socket
    // on different parabolas.
    assert_eq!(fall_elevation(10.0, true, 0.0), fall_elevation(10.0, false, 0.0));
    // Where it does bite is a knockback thrown downwards faster than 7.
    assert!(fall_elevation(1.0, true, 30.0) < fall_elevation(1.0, false, 30.0));
}

/// A knockback is adopted verbatim, because the ack is compared field by
/// field to within 0.01 and a mismatch leaves the change pending — which is
/// the same `CHEAT_TYPE_PENDING_ACK_DELAY` as never answering at all.
#[test]
fn a_knockback_is_echoed_back_exactly_as_it_arrived() {
    let mut w = Writer::new();
    w.u8(0b1).u8(9); // packed guid 9
    w.u32(4);
    // speedZ -9.5: the server sends `-verticalSpeed`, so a throw *upward*
    // is negative on the wire.
    w.f32(0.5).f32(-0.25).f32(12.0).f32(-9.5);
    let kb = parse_knock_back(&w.buf).expect("a knockback");
    assert_eq!((kb.guid, kb.counter), (9, 4));

    let mut m = Mover::new(Position::default(), Speeds::default());
    m.knock_back(kb.cos_angle, kb.sin_angle, kb.xy_speed, kb.z_speed);
    assert_eq!(m.info.jump.cos_angle, 0.5);
    assert_eq!(m.info.jump.sin_angle, -0.25);
    assert_eq!(m.info.jump.xy_speed, 12.0);
    assert_eq!(m.info.jump.z_speed, -9.5);
    assert!(m.is_airborne() && !m.is_jumping());

    // …and the simulation reads it as *up*: the character rises.
    let start = m.position().z;
    m.advance(0.2, None);
    assert!(m.position().z > start, "a knock-up should rise");

    // …and the block that goes back carries them, which is what the server
    // compares. `JUMPING` is what puts the jump section on the wire at all.
    let body = knock_back_ack_body(kb.guid, kb.counter, &m.info);
    let mut r = Reader::new(&body);
    assert_eq!(r.u64(), 9);
    assert_eq!(r.u32(), 4);
    let echoed = MovementInfo::read(&mut r).expect("a movement block");
    assert_eq!(echoed.jump, m.info.jump);
}

/// **Root's ack is one field shorter than the other three.**
/// `HandleMoveRootAck` reads guid, counter and the block; the water-walk
/// family goes through `HandleMovementFlagChangeToggleAck`, which reads a
/// trailing `u32 apply` — and reading garbage there is a rejected ack.
#[test]
fn a_flag_change_ack_carries_apply_for_everything_but_the_root() {
    let root = FlagChange::of(Opcode::SMSG_FORCE_MOVE_ROOT).expect("a root");
    assert_eq!(root.flag, move_flags::ROOT);
    assert!(root.apply);
    assert_eq!(root.ack, Opcode::CMSG_FORCE_MOVE_ROOT_ACK);
    assert!(!root.apply_field());

    let unroot = FlagChange::of(Opcode::SMSG_FORCE_MOVE_UNROOT).expect("an unroot");
    assert!(!unroot.apply);
    assert_eq!(unroot.ack, Opcode::CMSG_FORCE_MOVE_UNROOT_ACK);

    // The other three share one ack opcode between their two directions,
    // which is exactly why the trailing field has to be there.
    let walk = FlagChange::of(Opcode::SMSG_MOVE_WATER_WALK).expect("water walk");
    let land = FlagChange::of(Opcode::SMSG_MOVE_LAND_WALK).expect("land walk");
    assert_eq!(walk.ack, land.ack);
    assert!(walk.apply && !land.apply);
    assert!(walk.apply_field());

    let info = MovementInfo::default();
    let short = flag_change_ack_body(7, 3, &info, None);
    let long = flag_change_ack_body(7, 3, &info, Some(true));
    assert_eq!(long.len(), short.len() + 4);
    assert_eq!(&long[long.len() - 4..], &1u32.to_le_bytes());
}

/// The order of the two questions is the whole reason [`Footing`] is one
/// trait: a wall shortens the stride, and the floor is then looked up where
/// the character actually ended up. Asked the other way round, a character
/// stopped at a doorway is stood on the floor inside the room.
#[test]
fn a_wall_shortens_the_stride_before_the_floor_is_looked_up() {
    struct WalledRoom;
    impl Footing for WalledRoom {
        fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
            // The room's floor is raised, and it starts at x = 5.
            Some(if x >= 5.0 { 100.0 } else { 0.0 })
        }
        fn step(&self, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
            [to[0].min(5.0 - 0.5), to[1], from[2]]
        }
    }
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.advance(1.0, Some(&WalledRoom));
    assert!((m.position().x - 4.5).abs() < 0.001, "{:?}", m.position());
    assert_eq!(m.position().z, 0.0, "stood on the floor behind the wall");
}

/// **One jump is one landing**, and the report that made this worth pinning is
/// an animation one: "when the player jumps and lands… there is a brief jitter
/// of the landing animation a 2nd time".
///
/// The renderer fires the landing clip on the *edge* of
/// [`Mover::is_airborne`] and on nothing else, so a second play of it means a
/// second edge — and a second edge here is not only a wrong picture, it is a
/// second `MSG_MOVE_FALL_LAND` on the wire, which `CheckMoveFlags` reads as a
/// fall stop with no fall in front of it.
///
/// Run at a run, over flat ground, in the session thread's own 25 ms steps.
#[test]
fn one_jump_is_one_landing() {
    struct Flat;
    impl Footing for Flat {
        fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
            Some(0.0)
        }
    }
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    // Settle on the ground first, so the take-off is the only edge in the run.
    m.advance(0.025, Some(&Flat));
    assert!(!m.is_airborne(), "standing before the jump");
    assert!(m.jump().is_some(), "the jump was refused");
    assert!(m.is_airborne());

    let (mut landings, mut takeoffs, mut airborne) = (0, 0, true);
    for _ in 0..200 {
        if m.advance(0.025, Some(&Flat)) == Some(Opcode::MSG_MOVE_FALL_LAND) {
            landings += 1;
        }
        if m.is_airborne() != airborne {
            airborne = m.is_airborne();
            takeoffs += usize::from(airborne);
        }
    }
    assert_eq!(landings, 1, "one arc, one landing packet");
    assert_eq!(takeoffs, 0, "and it did not leave the ground again");
    assert!(!m.is_airborne(), "…and it is still standing five seconds later");
}

/// A flat world with a lake in it: ground at zero, water standing at `surface`
/// everywhere with `x >= 0` and dry land behind that.
struct Lake {
    surface: f32,
    bed: f32,
}

impl Footing for Lake {
    fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(if x >= 0.0 { self.bed } else { 0.0 })
    }
    fn liquid(&self, x: f32, _y: f32) -> Option<f32> {
        (x >= 0.0).then_some(self.surface)
    }
}

/// **Water deeper than `0.75 * height` is swum in, and the entry is announced
/// with the one opcode the server will accept it under.**
///
/// `CheckMoveFlags` flags any packet *other* than `MSG_MOVE_START_SWIM` that
/// introduces `MOVEFLAG_SWIMMING`, so the flag and the opcode have to leave
/// together — and the character is placed on the water line rather than left on
/// the bed, which is what makes walking into a lake float rather than drown.
#[test]
fn deep_enough_water_starts_the_swim_and_says_so() {
    let world = Lake { surface: 0.0, bed: -10.0 };
    let mut m = Mover::new(Position { x: -1.0, ..Default::default() }, Speeds::default());
    m.set_controls(Controls { forward: true, ..Default::default() });
    // Facing +x by default (orientation 0), so this walks into the lake.
    let mut events = Vec::new();
    for _ in 0..40 {
        if let Some(op) = m.advance(0.05, Some(&world)) {
            events.push(op);
        }
    }
    assert!(
        events.contains(&Opcode::MSG_MOVE_START_SWIM),
        "the swim was never announced: {events:?}"
    );
    assert!(m.is_swimming());
    assert!(m.info.has(move_flags::SWIMMING), "the flag rides with it");
    // Floating at the entry depth, not standing on the bed ten yards down.
    assert!(
        (m.position().z - (world.surface - MIN_SWIM_DEPTH)).abs() < 0.001,
        "{:?}",
        m.position()
    );
}

/// **A water-walking character walks onto the lake instead of into it.**
///
/// The same walk as the test above, with `MOVEFLAG_WATERWALKING` on: no
/// `MSG_MOVE_START_SWIM`, no `MOVEFLAG_SWIMMING`, and the character standing on
/// the surface rather than floating at the entry depth under it.
///
/// The bed is ten yards down, which is the number that makes this a real
/// assertion: a client that ignored the flag would put the character *there*.
#[test]
fn a_water_walker_walks_onto_the_lake_rather_than_into_it() {
    let world = Lake { surface: 0.0, bed: -10.0 };
    let mut m = Mover::new(Position { x: -1.0, ..Default::default() }, Speeds::default());
    m.info.flags |= move_flags::WATERWALKING;
    m.set_controls(Controls { forward: true, ..Default::default() });
    let mut events = Vec::new();
    for _ in 0..40 {
        if let Some(op) = m.advance(0.05, Some(&world)) {
            events.push(op);
        }
    }
    assert!(
        !events.contains(&Opcode::MSG_MOVE_START_SWIM),
        "a water-walker started swimming: {events:?}"
    );
    assert!(!m.is_swimming());
    assert!(m.position().x > 0.0, "it never reached the water: {:?}", m.position());
    assert!(
        (m.position().z - world.surface).abs() < 0.001,
        "not on the surface: {:?}",
        m.position()
    );
}

/// **…and a swimmer who gains the flag leaves the water through the ordinary
/// exit**, which is why the flag is spelled as "there is no liquid here" rather
/// than as an early return in [`Mover::advance`]: dropping `MOVEFLAG_SWIMMING`
/// under any opcode but `MSG_MOVE_STOP_SWIM` is a packet the server's own
/// `CheckMoveFlags` objects to.
#[test]
fn a_swimmer_given_water_walking_stops_swimming_and_says_so() {
    let world = Lake { surface: 0.0, bed: -10.0 };
    let mut m = Mover::new(Position { x: 1.0, z: -3.0, ..Default::default() }, Speeds::default());
    m.advance(0.05, Some(&world));
    assert!(m.is_swimming(), "the swim never started");

    m.info.flags |= move_flags::WATERWALKING;
    let mut events = Vec::new();
    for _ in 0..10 {
        if let Some(op) = m.advance(0.05, Some(&world)) {
            events.push(op);
        }
    }
    assert!(
        events.contains(&Opcode::MSG_MOVE_STOP_SWIM),
        "the exit was never announced: {events:?}"
    );
    assert!(!m.is_swimming());
    assert!((m.position().z - world.surface).abs() < 0.001, "{:?}", m.position());
}

/// **A hovering character stands a yard off the ground** —
/// [`crate::state::movement::HOVER_HEIGHT`], the client's own `1.0f`.
#[test]
fn a_hovering_character_stands_a_yard_up() {
    let world = |_: f32, _: f32, _: f32| Some(0.0f32);
    let mut m = Mover::new(Position { z: 5.0, ..Default::default() }, Speeds::default());
    m.info.flags |= move_flags::HOVER;
    for _ in 0..200 {
        m.advance(0.05, Some(&world));
    }
    assert!(
        (m.position().z - crate::state::movement::HOVER_HEIGHT).abs() < 0.001,
        "{:?}",
        m.position()
    );
}

/// **The exit has a dead band under the entry, and it is 1/36 of a yard.**
///
/// A swimmer held exactly on the water line sits at `MIN_SWIM_DEPTH`, which is
/// *not* deeper than the entry threshold — so without the hysteresis the very
/// next tick would leave the water, the one after would enter it again, and the
/// socket would carry two packets a tick for as long as anyone swam.
#[test]
fn a_swimmer_floating_at_the_entry_depth_does_not_chatter() {
    let world = Lake { surface: 0.0, bed: -10.0 };
    let mut m = Mover::new(Position { x: 5.0, z: -3.0, ..Default::default() }, Speeds::default());
    assert_eq!(
        m.advance(0.05, Some(&world)),
        Some(Opcode::MSG_MOVE_START_SWIM)
    );
    for _ in 0..40 {
        assert_eq!(m.advance(0.05, Some(&world)), None, "a second transition");
    }
    assert!(m.is_swimming());
}

/// **The shallows put you back on your feet, and `MSG_MOVE_STOP_SWIM` must not
/// carry the flag** — `CHEAT_TYPE_FLY_HACK_SWIM` is exactly that packet.
#[test]
fn wading_out_stops_the_swim_with_the_flag_already_cleared() {
    // A bed just under the surface: shallower than the exit threshold.
    let world = Lake { surface: 0.0, bed: -0.2 };
    let mut m = Mover::new(Position { x: 5.0, z: -3.0, ..Default::default() }, Speeds::default());
    assert_eq!(
        m.advance(0.05, Some(&world)),
        Some(Opcode::MSG_MOVE_START_SWIM)
    );
    // The floor lifts them out on the next step.
    assert_eq!(
        m.advance(0.05, Some(&world)),
        Some(Opcode::MSG_MOVE_STOP_SWIM)
    );
    assert!(!m.info.has(move_flags::SWIMMING));
}

/// **A fall that ends in water does not land.** `IsFallEndOpcode` is
/// `MSG_MOVE_FALL_LAND` *or* `MSG_MOVE_START_SWIM`, so the swim start is the
/// arc's end as far as the server is concerned — and sending a `FALL_LAND` as
/// well would be a second one for the same arc.
#[test]
fn falling_into_a_lake_ends_the_arc_without_landing() {
    let world = Lake { surface: 0.0, bed: -30.0 };
    let mut m = Mover::new(Position { x: 5.0, z: 20.0, ..Default::default() }, Speeds::default());
    let mut events = Vec::new();
    for _ in 0..80 {
        if let Some(op) = m.advance(0.05, Some(&world)) {
            events.push(op);
        }
    }
    assert!(events.contains(&Opcode::MSG_MOVE_START_SWIM));
    assert!(
        !events.contains(&Opcode::MSG_MOVE_FALL_LAND),
        "the water is the end of the arc: {events:?}"
    );
    assert!(!m.is_airborne(), "the arc is over");
    assert_eq!(m.info.fall_time, 0);
    assert!(!m.info.has(move_flags::JUMPING));
}

/// **The pitch is the whole of a swimmer's vertical, and it splits one speed
/// into two components rather than adding a second one.**
///
/// A diver at 45° covers `cos 45°` of their swim speed across the map and
/// `sin 45°` of it downwards; a diver pointed straight down covers no ground at
/// all. Anything else swims faster underwater than on the surface.
#[test]
fn a_swimmers_pitch_splits_one_speed_and_does_not_add_one() {
    let world = Lake { surface: 0.0, bed: -100.0 };
    let speeds = Speeds::default();
    let mut m = Mover::new(Position { x: 5.0, z: -3.0, ..Default::default() }, speeds);
    m.advance(0.05, Some(&world));
    assert!(m.is_swimming());

    m.set_pitch(-std::f32::consts::FRAC_PI_4);
    m.set_controls(Controls { forward: true, ..Default::default() });
    let before = m.position();
    m.advance(1.0, Some(&world));
    let after = m.position();

    let flat = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
    let down = before.z - after.z;
    let expected = speeds.swim() * std::f32::consts::FRAC_1_SQRT_2;
    assert!((flat - expected).abs() < 0.01, "flat {flat}, want {expected}");
    assert!((down - expected).abs() < 0.01, "down {down}, want {expected}");

    // …and straight down covers essentially no ground. Not *exactly* none:
    // `set_pitch` holds a hundredth of a radian off the pole, because the value
    // is serialised and `MaNGOS::IsValidMapCoord` wants it finite — so a full
    // second of a vertical dive is a few centimetres sideways against a whole
    // swim speed downwards.
    m.set_pitch(-std::f32::consts::FRAC_PI_2);
    let before = m.position();
    m.advance(1.0, Some(&world));
    let after = m.position();
    let flat = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
    assert!(flat < 0.05, "a vertical dive travelled {flat} yards sideways");
    assert!(after.z < before.z - speeds.swim() * 0.99);
}

/// **A swimmer does not jump**, and does not sink or float on their own.
///
/// 1.12 declares no ascend binding and no ascend flag, so the depth is the
/// pitch's and nothing else — an idle swimmer stays where they were left, which
/// is what treading water feels like. The surface is a ceiling rather than a
/// place to be: rising through it is what *stops* the swimming.
#[test]
fn a_swimmer_neither_jumps_nor_drifts() {
    let world = Lake { surface: 0.0, bed: -100.0 };
    let mut m = Mover::new(Position { x: 5.0, z: -8.0, ..Default::default() }, Speeds::default());
    m.advance(0.05, Some(&world));
    assert!(m.is_swimming());
    // The entry line, and then nothing moves it.
    let held = m.position().z;
    assert!(m.jump().is_none(), "a swimmer put MSG_MOVE_JUMP on the wire");
    for _ in 0..40 {
        m.advance(0.05, Some(&world));
    }
    assert!((m.position().z - held).abs() < 0.001, "drifted to {}", m.position().z);

    // …and swimming up stops at the line rather than through it.
    m.set_pitch(std::f32::consts::FRAC_PI_2);
    m.set_controls(Controls { forward: true, ..Default::default() });
    for _ in 0..40 {
        m.advance(0.05, Some(&world));
    }
    assert!(m.position().z <= world.surface - MIN_SWIM_DEPTH + 0.001);
}

/// **The ascend key is a movement flag, and it is legal only in the water.**
///
/// 1.12 does have an up and a down — `MOVEFLAG_PITCH_UP`/`PITCH_DOWN` and the
/// three consecutive opcodes 191/192/193 — and the client reaches them only
/// after testing `MOVEFLAG_SWIMMING`. So the same key that jumps
/// on land must put nothing on the wire there: `MSG_MOVE_START_PITCH_UP` sent
/// by a character standing on a hill is a flag the reference cannot produce.
#[test]
fn the_ascend_key_is_refused_on_land_and_announced_in_the_water() {
    let dry = |_x: f32, _y: f32, _z: f32| Some(0.0);
    let mut m = Mover::new(Position::default(), Speeds::default());
    let held = Controls { ascend: true, ..Default::default() };
    assert!(m.set_controls(held).is_empty(), "a pitch packet on dry land");
    assert!(!m.info.has(move_flags::PITCH_UP));
    for _ in 0..10 {
        assert_eq!(m.advance(0.05, Some(&dry)), None);
    }
    assert!(!m.info.has(move_flags::PITCH_UP));
    // …and the jump is still the jump there.
    assert!(m.jump().is_some());

    let world = Lake { surface: 0.0, bed: -100.0 };
    let mut m = Mover::new(Position { x: 5.0, z: -3.0, ..Default::default() }, Speeds::default());
    assert_eq!(
        m.advance(0.05, Some(&world)),
        Some(Opcode::MSG_MOVE_START_SWIM)
    );
    let events = m.set_controls(held);
    assert_eq!(
        events.last().map(|e| e.opcode),
        Some(Opcode::MSG_MOVE_START_PITCH_UP),
    );
    assert!(m.info.has(move_flags::PITCH_UP), "the start must carry its own flag");
    // …and leaving the water takes it away, on the stop-swim packet: a pitch
    // bit standing on dry land is a state the reference has no route to. Two
    // steps, because the first is the one the rising floor lifts them on and
    // the second is the one that reads the new depth.
    let shallow = Lake { surface: 0.0, bed: -0.2 };
    m.advance(0.05, Some(&shallow));
    assert_eq!(
        m.advance(0.05, Some(&shallow)),
        Some(Opcode::MSG_MOVE_STOP_SWIM)
    );
    assert!(!m.info.has(move_flags::PITCH_UP));
}

/// **Held alone it is straight up**, which is the whole of what the key is for:
/// no travel key is down, so there is no horizontal to compose with, and the
/// rate is the swim speed because `MOVEFLAG_PITCH_UP` is in
/// `MOVEFLAG_MASK_MOVING` and the server measures it as swimming.
///
/// The companion assertion is the one that needed [`move_flags::MASK_TRAVEL`]:
/// keyed on `is_moving()` — which the pitch bit satisfies — [`strode`] would
/// have swum the character forwards at full speed for as long as they held the
/// key to rise.
#[test]
fn ascending_rises_at_the_swim_speed_and_travels_nowhere() {
    let world = Lake { surface: 0.0, bed: -100.0 };
    let speeds = Speeds::default();
    let mut m = Mover::new(Position { x: 5.0, z: -3.0, ..Default::default() }, speeds);
    m.advance(0.05, Some(&world));
    assert!(m.is_swimming());
    // Dive first — entering the water *places* the character on the line, so
    // there is nothing to rise from until they have gone down.
    m.set_pitch(-std::f32::consts::FRAC_PI_2);
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.advance(3.0, Some(&world));
    assert!(m.position().z < -10.0, "the dive went nowhere: {:?}", m.position());
    m.set_pitch(0.0);
    m.set_controls(Controls { ascend: true, ..Default::default() });

    let before = m.position();
    m.advance(1.0, Some(&world));
    let after = m.position();
    assert!(
        (after.z - before.z - speeds.swim()).abs() < 0.01,
        "rose {} in a second, want {}",
        after.z - before.z,
        speeds.swim()
    );
    assert_eq!((after.x, after.y), (before.x, before.y), "it travelled");

    // …and it stops at the water line like every other way of going up.
    for _ in 0..200 {
        m.advance(0.05, Some(&world));
    }
    assert!(m.position().z <= world.surface - MIN_SWIM_DEPTH + 0.001);
}

/// **A world that answers no liquid is the world every existing caller has**,
/// which is what the defaulted [`Footing::liquid`] is for: the CLI's terrain
/// closure and every test that predates this one must walk exactly as before.
#[test]
fn a_footing_with_no_water_in_it_never_swims() {
    let ground = |_x: f32, _y: f32, _z: f32| Some(0.0);
    let mut m = Mover::new(Position { z: -50.0, ..Default::default() }, Speeds::default());
    for _ in 0..40 {
        assert_ne!(m.advance(0.05, Some(&ground)), Some(Opcode::MSG_MOVE_START_SWIM));
    }
    assert!(!m.is_swimming());
}

#[test]
fn monster_move_stop_packet_has_no_destination() {
    let mut w = Writer::new();
    w.u8(0b1).u8(7); // packed guid 7
    w.f32(1.0).f32(2.0).f32(3.0);
    w.u32(99);
    w.u8(1); // MonsterMoveStop
    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(mm.guid, 7);
    assert_eq!(mm.start, [1.0, 2.0, 3.0]);
    assert_eq!(mm.destination(), None);
    assert!(mm.path.is_empty());
}

#[test]
fn monster_move_reads_destination_and_duration() {
    let mut w = Writer::new();
    w.u8(0b1).u8(9);
    w.f32(0.0).f32(0.0).f32(0.0);
    w.u32(1);
    w.u8(0); // normal facing
    w.u32(0x100); // spline flags
    w.u32(2500); // duration
    w.u32(1); // node count
    w.f32(10.0).f32(20.0).f32(30.0);
    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(mm.destination(), Some([10.0, 20.0, 30.0]));
    assert_eq!(mm.path, vec![[0.0, 0.0, 0.0], [10.0, 20.0, 30.0]]);
    assert_eq!(mm.duration_ms, 2500);
    assert_eq!(mm.facing, SplineFacing::Travel);
}

#[test]
fn monster_move_reads_the_final_facing_rather_than_skipping_it() {
    // A creature that has just run up to a player is sent with `Final_Target`
    // and turns to look at it on arrival. The facing block sits *before* the
    // flags and duration, so its three widths — 3 floats, a u64, one float —
    // also decide where everything after it starts.
    let build = |mode: u8, facing: &[u8]| {
        let mut w = Writer::new();
        w.u8(0b1).u8(9);
        w.f32(0.0).f32(0.0).f32(0.0);
        w.u32(1);
        w.u8(mode);
        w.buf.extend_from_slice(facing);
        w.u32(0x100).u32(1000).u32(1);
        w.f32(7.0).f32(0.0).f32(0.0);
        w.buf
    };

    let mut angle = Writer::new();
    angle.f32(1.5);
    let mm = parse_monster_move(&build(4, &angle.buf)).expect("parsed");
    assert_eq!(mm.facing, SplineFacing::Angle(1.5));
    // Everything after the facing still lands: the destination proves it.
    assert_eq!(mm.destination(), Some([7.0, 0.0, 0.0]));

    let mut target = Writer::new();
    target.u64(0xF130_0000_0000_002A);
    let mm = parse_monster_move(&build(3, &target.buf)).expect("parsed");
    assert_eq!(mm.facing, SplineFacing::Target(0xF130_0000_0000_002A));
    assert_eq!(mm.destination(), Some([7.0, 0.0, 0.0]));

    let mut spot = Writer::new();
    spot.f32(1.0).f32(2.0).f32(3.0);
    let mm = parse_monster_move(&build(2, &spot.buf)).expect("parsed");
    assert_eq!(mm.facing, SplineFacing::Spot([1.0, 2.0, 3.0]));
    assert_eq!(mm.destination(), Some([7.0, 0.0, 0.0]));
}

#[test]
fn a_linear_path_reconstructs_its_intermediate_nodes_from_packed_offsets() {
    // `WriteLinearPath` writes the destination first and the earlier nodes as
    // offsets *from it*, packed three-to-a-u32 in quarter yards. Reading them
    // is what stops a creature cutting the corner of its own patrol route.
    let destination = [100.0f32, 40.0, 10.0];
    let want = [[20.0f32, 8.0, 10.0], [60.0, 36.0, 10.0]];
    let pack = |node: [f32; 3]| -> u32 {
        let q = |v: f32| ((v / 0.25) as i32) as u32;
        (q(destination[0] - node[0]) & 0x7FF)
            | ((q(destination[1] - node[1]) & 0x7FF) << 11)
            | ((q(destination[2] - node[2]) & 0x3FF) << 22)
    };

    let mut w = Writer::new();
    w.u8(0b1).u8(9);
    w.f32(20.0).f32(8.0).f32(10.0);
    w.u32(1);
    w.u8(0);
    w.u32(0x100).u32(4000);
    w.u32(3); // three nodes: two offsets then the destination
    w.f32(destination[0]).f32(destination[1]).f32(destination[2]);
    w.u32(pack(want[0])).u32(pack(want[1]));

    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(mm.path.len(), 3);
    for (got, expected) in mm.path.iter().zip(want.iter().chain(&[destination])) {
        for axis in 0..3 {
            assert!(
                (got[axis] - expected[axis]).abs() < 0.25,
                "{got:?} vs {expected:?}"
            );
        }
    }
}

#[test]
fn a_negative_packed_offset_does_not_land_512_yards_away() {
    // Each field is masked to its width server-side, so a negative offset
    // arrives as a positive number with the width's top bit set. Read
    // unsigned, a node one yard back becomes one 512 yards forward — which is
    // a creature that visibly teleports and then walks home.
    assert_eq!(unpack_offset(0), [0.0, 0.0, 0.0]);
    // -1 in each field: 0x7FF, 0x7FF, 0x3FF at their shifts.
    let all_ones = 0x7FF | (0x7FF << 11) | (0x3FF << 22);
    assert_eq!(unpack_offset(all_ones), [-0.25, -0.25, -0.25]);
    // The most negative each field can hold, and the most positive.
    assert_eq!(unpack_offset(0x400)[0], -256.0);
    assert_eq!(unpack_offset(0x3FF)[0], 255.75);
}

#[test]
fn a_flying_path_takes_its_destination_from_the_last_node_not_the_first() {
    // `WriteCatmullRomPath` writes every node absolutely and in order, so the
    // vector right after the count is node zero. Reading it as the
    // destination sends the unit to its first waypoint — in range, wrong
    // place, and no error anywhere. `Mask_CatmullRom` is `Flying`.
    let mut w = Writer::new();
    w.u8(0b1).u8(9);
    w.f32(0.0).f32(0.0).f32(0.0);
    w.u32(1);
    w.u8(0);
    w.u32(0x100 | spline_flags::FLYING).u32(3000);
    w.u32(3);
    w.f32(1.0).f32(0.0).f32(50.0);
    w.f32(2.0).f32(0.0).f32(60.0);
    w.f32(3.0).f32(0.0).f32(70.0);

    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(mm.destination(), Some([3.0, 0.0, 70.0]));
    // Four, not three: `WriteCatmullRomPath` appends from `getPoint(2)`,
    // which is `controls[1]`, and the header's start field carries
    // `controls[0]`. See the note beside the prepend in `parse_monster_move`.
    assert_eq!(mm.path.len(), 4);
    assert_eq!(mm.path[0], [0.0, 0.0, 0.0], "the path did not begin where the unit is");
}

/// **The first leg, which neither writer sends inside the path.**
///
/// `WriteLinearPath` emits offsets for `real_path[0..last_idx)` where
/// `real_path = &spline.getPoint(1)`, so the earliest node in the body is
/// `controls[1]`; `controls[0]` is in the header, as
/// `spline.getPoint(spline.first())`.
///
/// Dropping it does not lose a point, it loses a whole **leg**: the creature
/// is placed on its second waypoint the instant the packet arrives, then
/// walks a shorter path than the duration accounts for and is snapped back
/// by the next packet. Every coordinate in the path is a real waypoint the
/// server named, which is why nothing catches this by looking at them.
#[test]
fn a_linear_path_keeps_the_leg_from_where_the_unit_is_standing() {
    // Four control points: the server writes the destination, then offsets
    // for controls[1] and controls[2]. controls[0] is the header's start.
    let start = [0.0f32, 0.0, 0.0];
    let destination = [30.0f32, 0.0, 0.0];
    let offset = |p: [f32; 3]| {
        // destination - p, in quarter-yard fixed point, 11/11/10 bits.
        let q = |v: f32| ((v / 0.25).round() as i32) as u32;
        (q(destination[0] - p[0]) & 0x7FF)
            | ((q(destination[1] - p[1]) & 0x7FF) << 11)
            | ((q(destination[2] - p[2]) & 0x3FF) << 22)
    };

    let mut w = Writer::new();
    w.u8(0b1).u8(9);
    w.f32(start[0]).f32(start[1]).f32(start[2]);
    w.u32(1);
    w.u8(0);
    w.u32(0x100).u32(4000);
    w.u32(3); // nodeCount = last_idx + 1, for four control points
    w.f32(destination[0]).f32(destination[1]).f32(destination[2]);
    w.u32(offset([10.0, 0.0, 0.0]));
    w.u32(offset([20.0, 0.0, 0.0]));

    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(
        mm.path,
        vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0], [30.0, 0.0, 0.0]],
        "the leg from the unit's own position is missing"
    );
}

/// …and a cyclic catmull-rom path already leads with it, so it must not be
/// prepended twice. `WriteCatmullRomCyclicPath` writes `getPoint(1)` —
/// `controls[0]`, the same point the header carries — as the fake vertex
/// `Enter_Cycle` says to erase after the first lap.
#[test]
fn a_cyclic_path_is_not_given_a_third_copy_of_its_first_point() {
    let mut w = Writer::new();
    w.u8(0b1).u8(9);
    w.f32(5.0).f32(0.0).f32(0.0);
    w.u32(1);
    w.u8(0);
    w.u32(0x100 | spline_flags::FLYING | spline_flags::CYCLIC).u32(3000);
    w.u32(3);
    w.f32(5.0).f32(0.0).f32(0.0); // the fake leading vertex
    w.f32(5.0).f32(0.0).f32(0.0); // controls[0] again
    w.f32(25.0).f32(0.0).f32(0.0);

    let mm = parse_monster_move(&w.buf).expect("parsed");
    assert_eq!(mm.path.len(), 3, "the header's start was prepended again");
    assert_eq!(mm.path[0], [5.0, 0.0, 0.0]);
}

#[test]
fn a_broadcast_movement_packet_names_the_player_that_moved() {
    // What HandleMovementOpcodes rebuilds and sends to everyone nearby:
    // the mover's packed guid, then the block it received verbatim.
    let info = MovementInfo {
        flags: move_flags::FORWARD,
        time: 5000,
        position: Position { x: -10561.0, y: -1186.0, z: 28.0, orientation: 3.0 },
        ..Default::default()
    };
    let mut w = Writer::new();
    w.u8(0b11).u8(0x0D).u8(0x01); // packed guid 0x010D
    w.bytes(&info.to_bytes());

    let (guid, parsed) = parse_movement_broadcast(&w.buf).expect("parsed");
    assert_eq!(guid, 0x010D);
    assert_eq!(parsed, info);
}

#[test]
fn time_skipped_is_not_treated_as_a_movement_block() {
    // MSG_MOVE_TIME_SKIPPED is packed guid + u32. It would parse as a
    // movement block without erroring and report a garbage position, so the
    // allowlist — not the parser — is what keeps it out.
    assert!(!is_broadcast_movement(Opcode::MSG_MOVE_TIME_SKIPPED));
    assert!(is_broadcast_movement(Opcode::MSG_MOVE_HEARTBEAT));
    assert!(is_broadcast_movement(Opcode::MSG_MOVE_SET_FACING));
}

#[test]
fn wrapping_keeps_orientation_finite() {
    assert!((wrap_angle(-0.5) - (2.0 * std::f32::consts::PI - 0.5)).abs() < 0.001);
    assert!(wrap_angle(100.0) < 2.0 * std::f32::consts::PI);
}

/// Every heading the eight travel combinations produce, as offsets from a zero
/// facing — the angles `Mover::advance`, the prediction and the server's own
/// extrapolation all walk.
#[test]
fn every_travel_combination_has_its_own_heading() {
    use std::f32::consts::PI;
    let offset = |flags| {
        MovementInfo {
            flags,
            position: Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
            ..Default::default()
        }
        .heading()
    };

    assert_eq!(offset(move_flags::FORWARD), 0.0);
    assert_eq!(offset(move_flags::BACKWARD), PI);
    assert_eq!(offset(move_flags::STRAFE_LEFT), PI / 2.0);
    assert_eq!(offset(move_flags::STRAFE_RIGHT), -PI / 2.0);
    assert_eq!(offset(move_flags::FORWARD | move_flags::STRAFE_LEFT), PI / 4.0);
    assert_eq!(offset(move_flags::FORWARD | move_flags::STRAFE_RIGHT), -PI / 4.0);
    assert_eq!(offset(move_flags::BACKWARD | move_flags::STRAFE_LEFT), PI * 0.75);
    assert_eq!(offset(move_flags::BACKWARD | move_flags::STRAFE_RIGHT), -PI * 0.75);
    // Not travelling: straight ahead, which is what a zero stride is multiplied
    // by anyway.
    assert_eq!(offset(move_flags::NONE), 0.0);
}

/// **The drawn body's offset from the aim, which is the whole of how 1.12 draws
/// a strafe** — the ground ships no sideways gait, so the legs run and the
/// *root* turns into the slide.
///
/// Held against [`MovementInfo::heading`] in the same test, because the two are
/// one statement seen twice: the body offset is the heading offset, capped at a
/// quarter turn. A pure strafe walks at ±90° and faces ±90°, so the legs point
/// exactly along the travel; a diagonal walks at ±45° or ±135° and faces ±45°,
/// which is the same rule with the backpedal's mirror folded in.
#[test]
fn the_body_faces_along_the_line_it_travels() {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    use crate::state::movement::strafe_body_offset;

    assert_eq!(strafe_body_offset(move_flags::STRAFE_LEFT), FRAC_PI_2);
    assert_eq!(strafe_body_offset(move_flags::STRAFE_RIGHT), -FRAC_PI_2);
    // A forward diagonal: the body takes the same 45° the heading does.
    assert_eq!(
        strafe_body_offset(move_flags::FORWARD | move_flags::STRAFE_LEFT),
        FRAC_PI_4
    );
    // **The backpedal mirrors**, which is the part that cannot be guessed: a
    // back-and-left diagonal faces the body forward-*right* and backpedals
    // along the line of travel, so the legs never cross.
    assert_eq!(
        strafe_body_offset(move_flags::BACKWARD | move_flags::STRAFE_LEFT),
        -FRAC_PI_4
    );
    assert_eq!(
        strafe_body_offset(move_flags::BACKWARD | move_flags::STRAFE_RIGHT),
        FRAC_PI_4
    );

    // Nothing else turns the body off its aim — a plain run, a plain reverse,
    // standing still, and two strafe keys that cancel.
    for flags in [
        move_flags::NONE,
        move_flags::FORWARD,
        move_flags::BACKWARD,
        move_flags::STRAFE_LEFT | move_flags::STRAFE_RIGHT,
    ] {
        assert_eq!(strafe_body_offset(flags), 0.0, "{flags:#x}");
    }
}

/// **Holding a strafe key must change where the character goes, whatever else
/// is held**, which is the complaint this pins: with the server's own
/// precedence, S and then Q walked exactly where S alone did, so a strafe was
/// invisible for as long as the reverse key was down.
///
/// Stated as a *displacement* rather than as an angle, because that is the
/// thing on screen: the four combinations must land in four different places,
/// and each must travel the same distance as the plain reverse — the property
/// that keeps `CheckSpeedHack` unmoved (see [`MovementInfo::heading`]).
#[test]
fn a_strafe_moves_the_character_sideways_even_while_reversing() {
    let speeds = Speeds::default();
    let travel = |flags| {
        let info = MovementInfo {
            flags,
            position: Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
            ..Default::default()
        };
        strode(&info, &speeds, 1.0)
    };

    let back = travel(move_flags::BACKWARD);
    let back_left = travel(move_flags::BACKWARD | move_flags::STRAFE_LEFT);
    let back_right = travel(move_flags::BACKWARD | move_flags::STRAFE_RIGHT);
    let left = travel(move_flags::STRAFE_LEFT);

    // A plain strafe goes sideways at run speed, which is the case that was
    // never in doubt and is here so the test cannot pass on four zeroes.
    assert!(left[0].abs() < 1e-4 && (left[1] - speeds.run()).abs() < 1e-4, "{left:?}");

    // Reversing left and reversing right both go left and right of straight
    // back, by the same amount and in opposite directions.
    assert!(back_left[1] > 0.1, "back-and-left went nowhere sideways: {back_left:?}");
    assert!(back_right[1] < -0.1, "back-and-right went nowhere sideways: {back_right:?}");
    assert!((back_left[1] + back_right[1]).abs() < 1e-4);
    assert!(back_left[0] < 0.0 && back_right[0] < 0.0, "not still reversing");

    // …and each covers the same ground as the plain reverse, so the server's
    // distance check sees nothing new.
    let length = |[x, y]: [f32; 2]| (x * x + y * y).sqrt();
    for diagonal in [back_left, back_right] {
        assert!(
            (length(diagonal) - length(back)).abs() < 1e-3,
            "travelled {} against {}",
            length(diagonal),
            length(back)
        );
    }
}

/// **A charge, end to end**: the ride is walked, the keys are ignored while it
/// lasts, and it ends owing the one packet without which the session is over.
///
/// The failure this pins is not a wrong position — it is silence.
/// `HandleMovementOpcodes` opens with `if (pMover->HasPendingSplineDone())
/// return;`, so a client that never answers has every subsequent `MSG_MOVE_*`
/// discarded, and the only other thing that clears the flag is `Map::Add` — a
/// login. Hence the report: *"it glitches the char's position and requires a
/// relog"*.
#[test]
fn a_charge_is_ridden_and_then_acknowledged() {
    let start = Position { x: 0.0, y: 0.0, z: 10.0, orientation: 3.0 };
    let mut m = Mover::new(start, Speeds::default());
    // Holding W throughout: the keys are recorded and must not move anything.
    m.set_controls(Controls { forward: true, ..Default::default() });

    let charge = MonsterMove {
        guid: 7,
        start: [0.0, 0.0, 10.0],
        spline_id: 41,
        path: vec![[0.0, 0.0, 10.0], [24.0, 0.0, 10.0]],
        duration_ms: 1_000,
        // A charge is sent with `SetFacingGUID(target)`.
        facing: SplineFacing::Target(99),
        flags: 0,
        transport: None,
    };
    m.ride(&charge, Some([30.0, 0.0, 10.0]));
    assert!(m.is_riding());
    assert!(!m.can_move() && !m.can_turn(), "the keys still steer mid-charge");
    assert!(m.take_finished_ride().is_none(), "acked before it had run");
    // Run speed × 4, capped at 24 — and not one of the six in `Speeds`.
    assert!((m.travel_speed() - 24.0).abs() < 0.01, "{}", m.travel_speed());

    // Half way: on the path, not where W would have taken it.
    for _ in 0..20 {
        assert_eq!(m.advance(0.025, None), None, "a ride owes no MSG_MOVE_*");
    }
    assert!((m.position().x - 12.0).abs() < 0.2, "{:?}", m.position());
    assert!(m.is_riding() && m.take_finished_ride().is_none());
    // The gait is chosen from these: a charge is a run, not a slide.
    assert!(m.info.has(move_flags::FORWARD) && m.info.has(move_flags::SPLINE_ENABLED));

    for _ in 0..21 {
        m.advance(0.025, None);
    }
    assert!(!m.is_riding(), "the ride never ended");
    assert_eq!(m.take_finished_ride(), Some(41));
    assert_eq!(m.take_finished_ride(), None, "the ack is owed once");

    // **Within the server's own 10-yard window** — `HandleSplineDone` measures
    // the acked position against `movespline->FinalDestination()` and refuses
    // beyond that, which is why the ride is walked rather than skipped.
    assert!((m.position().x - 24.0).abs() < 0.5, "{:?}", m.position());
    // …facing what it charged, which is the packet's own `Final_Target`.
    assert!(m.position().orientation.abs() < 0.01, "{}", m.position().orientation);
    // …and the block is an ordinary running character's again, with the key
    // that was held all along back in it.
    assert!(!m.info.has(move_flags::SPLINE_ENABLED));
    assert!(m.info.has(move_flags::FORWARD));
    assert!(m.can_move() && m.can_turn());
}

/// **A taxi flight is the same ride with one flag on it, and the flag changes
/// three things.**
///
/// `FlightPathMovementGenerator::Reset` is an ordinary `MoveSplineInit` with
/// `SetFly()`, so the packet is the one a charge arrives as. What `Flying`
/// (0x200) then decides:
///
/// * the path is a **curve** rather than a polyline, so the character does not
///   pass through the corner points at all;
/// * the heading is the curve's **derivative**, so it turns through a corner
///   instead of snapping at it — the whole of the "snaps in straight lines"
///   report;
/// * `MOVEFLAG_FLYING` goes on the block, which is what the renderer picks the
///   `Fly` clip from — and is removed when the ride ends, exactly as
///   `FlightPathMovementGenerator::Finalize` removes it.
#[test]
fn a_flying_ride_curves_turns_smoothly_and_says_it_is_flying() {
    let start = Position { x: 0.0, y: 0.0, z: 100.0, orientation: 0.0 };
    let mut m = Mover::new(start, Speeds::default());
    // A right-angle turn, which is where a polyline is at its worst.
    let flight = MonsterMove {
        guid: 7,
        start: [0.0, 0.0, 100.0],
        spline_id: 55,
        path: vec![
            [0.0, 0.0, 100.0],
            [100.0, 0.0, 100.0],
            [100.0, 100.0, 100.0],
        ],
        duration_ms: 10_000,
        facing: SplineFacing::Travel,
        flags: spline_flags::FLYING,
        transport: None,
    };
    m.ride(&flight, None);
    assert!(m.info.has(move_flags::FLYING), "the clip is chosen from this");
    assert!(m.info.has(move_flags::SPLINE_ENABLED) && m.info.has(move_flags::FORWARD));

    // **The path is not the polyline.** Four tenths of the way along, a
    // straight run would still be on `y = 0`; the curve has left it — and it
    // has left it *outward*, which is Catmull-Rom's own behaviour on a sharp
    // corner rather than a mistake here: to arrive at the corner already
    // heading 45 degrees, a curve that started heading 0 has to bow the other
    // way first. A real taxi swings wide on a hard turn for the same reason.
    for _ in 0..160 {
        m.advance(0.025, None);
    }
    let before = m.position();
    assert!(before.y < -1.0, "the curve leaves the polyline: {before:?}");

    // …and the corner itself is passed *through*, because Catmull-Rom
    // interpolates its control points: it rounds the approach, not the point.
    for _ in 0..40 {
        m.advance(0.025, None);
    }
    let corner = m.position();
    assert!((corner.x - 100.0).abs() < 0.2 && corner.y.abs() < 0.2, "{corner:?}");
    // The heading there is the blend of the two legs — `(p2 - p0) / 2`, which
    // is 45 degrees — rather than either of them.
    assert!(
        (corner.orientation - std::f32::consts::FRAC_PI_4).abs() < 0.05,
        "mid-turn heading {}",
        corner.orientation
    );

    // The heading never jumps: sampled every 25 ms, no two readings are more
    // than a few degrees apart. A polyline turns 90 degrees in one step.
    let mut m = Mover::new(start, Speeds::default());
    m.ride(&flight, None);
    let mut last = m.position().orientation;
    let mut worst = 0.0f32;
    for _ in 0..401 {
        m.advance(0.025, None);
        let now = m.position().orientation;
        // **The short way round**, because the heading is wrapped: a body
        // turning through north reads as a 2π jump to a naive subtraction, and
        // that is the wrap rather than the turn.
        let turned = (now - last + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        worst = worst.max(turned.abs());
        last = now;
    }
    assert!(worst < 0.05, "the biggest single turn was {worst} rad");

    // …and the flag comes off with the ride, or a landed character keeps
    // flapping for the rest of the session.
    assert!(!m.is_riding());
    assert!(!m.info.has(move_flags::FLYING));
    assert_eq!(m.take_finished_ride(), Some(55));
}

/// **A ride that is not flying is walked exactly as it was** — the linear
/// branch is the one every creature in the world is on, and the curve must not
/// have moved it.
#[test]
fn an_ordinary_ride_is_still_a_polyline() {
    let start = Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 };
    let mut m = Mover::new(start, Speeds::default());
    let walk = MonsterMove {
        guid: 7,
        start: [0.0, 0.0, 0.0],
        spline_id: 56,
        path: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [100.0, 100.0, 0.0]],
        duration_ms: 10_000,
        facing: SplineFacing::Travel,
        flags: 0,
        transport: None,
    };
    m.ride(&walk, None);
    assert!(!m.info.has(move_flags::FLYING));
    for _ in 0..200 {
        m.advance(0.025, None);
    }
    let middle = m.position();
    assert!((middle.x - 100.0).abs() < 0.5, "on the corner: {middle:?}");
    assert!(middle.y.abs() < 0.5, "on the corner: {middle:?}");
}

/// **A stop packet is a ride too, and it is owed the same answer.**
///
/// `ChargeMovementGenerator::Initialize` opens with `if (!unit.IsStopped())
/// unit.StopMoving()`, which is a second `MoveSplineInit::Launch` — vmangos
/// writes *"Will trigger CMSG_MOVE_SPLINE_DONE from client"* beside it. Reading
/// it as "nothing to walk, nothing to say" leaves the flag set exactly as
/// ignoring the charge itself would.
#[test]
fn a_stop_packet_owes_the_ack_at_once_and_a_new_spline_supersedes_it() {
    let mut m = Mover::new(Position { x: 5.0, y: 5.0, z: 0.0, orientation: 1.0 }, Speeds::default());
    let stop = MonsterMove {
        guid: 7,
        start: [5.0, 5.0, 0.0],
        spline_id: 40,
        path: Vec::new(),
        duration_ms: 0,
        facing: SplineFacing::Angle(2.0),
        flags: 0,
        transport: None,
    };
    m.ride(&stop, None);
    assert!(!m.is_riding());
    assert!((m.position().orientation - 2.0).abs() < 1e-5, "the facing was dropped");
    assert_eq!(m.take_finished_ride(), Some(40));

    // …and an ack for a spline the server has already replaced is never sent:
    // `HandleMoveSplineDoneOpcode` compares the id against `movespline->GetId()`
    // and returns, so it would clear nothing and only look like an answer.
    m.ride(&stop, None);
    m.ride(
        &MonsterMove {
            spline_id: 41,
            path: vec![[5.0, 5.0, 0.0], [10.0, 5.0, 0.0]],
            duration_ms: 500,
            facing: SplineFacing::Travel,
            ..stop.clone()
        },
        None,
    );
    assert!(m.is_riding());
    assert_eq!(m.take_finished_ride(), None, "the superseded stop was still owed");
}

/// **A teleport voids a ride rather than acknowledging it.**
///
/// `HandleMoveSplineDoneOpcode` returns early while `IsBeingTeleported()`, so
/// the relocation *is* the hand-back — and a client that rode on would walk the
/// character straight back off the point the server had just put them on.
#[test]
fn a_teleport_abandons_a_ride_without_acking_it() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.ride(
        &MonsterMove {
            guid: 7,
            start: [0.0, 0.0, 0.0],
            spline_id: 41,
            path: vec![[0.0, 0.0, 0.0], [24.0, 0.0, 0.0]],
            duration_ms: 1_000,
            facing: SplineFacing::Travel,
            flags: 0,
            transport: None,
        },
        None,
    );
    m.advance(0.1, None);

    m.abandon_ride();
    m.resync(Position { x: -900.0, y: 100.0, z: 40.0, orientation: 0.0 });
    assert!(!m.is_riding());
    assert_eq!(m.take_finished_ride(), None, "acked across a teleport");
    assert!(m.can_move(), "the character never got the wheel back");

    // …and the simulation is running from where the teleport put it, not from
    // wherever the spline had reached.
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.advance(0.1, None);
    assert!(m.position().x < -890.0, "{:?}", m.position());
}

/// The ack's body, which the server reads in one order and no other:
/// `MovementInfo`, then the spline id, then a float it skips.
#[test]
fn the_spline_done_body_is_the_block_then_the_id() {
    let info = MovementInfo {
        flags: move_flags::FORWARD,
        time: 1_234,
        position: Position { x: 1.0, y: 2.0, z: 3.0, orientation: 0.5 },
        ..Default::default()
    };
    let body = spline_done_body(&info, 0xAABB_CCDD);
    let mut r = Reader::new(&body);
    let echoed = MovementInfo::read(&mut r).expect("a movement block");
    assert_eq!(echoed.position, info.position);
    assert_eq!(echoed.time, 1_234);
    assert_eq!(r.u32(), 0xAABB_CCDD);
    // The trailing word `Unused<float>()` skips. Its value is free; its
    // presence is not — the server reads past the id either way.
    assert_eq!(r.f32(), 1.0);
}

/// **A spline's velocity has a vertical in it, and the heading beside it does
/// not.**
///
/// `MoveSpline::ComputePosition` ends in `atan2(hermite.y, hermite.x)`, so what
/// a ride hands its readers is a *bearing* — flat by construction. That is right
/// for pointing a body and useless for continuing one, and a taxi flight spends
/// most of its length climbing or diving: a reader that walked the bearing at
/// the path's speed held its altitude between readings and stepped down to each
/// new one, forty times a second, with the camera framed on it.
#[test]
fn a_climbing_spline_states_a_climbing_velocity() {
    let spline = Spline {
        // Thirty yards along +x and twelve up, over a second.
        path: vec![[0.0, 0.0, 100.0], [30.0, 0.0, 112.0]],
        duration_ms: 1_000,
        elapsed_ms: 250,
        facing: SplineFacing::Travel,
        cyclic: false,
        flying: false,
        falling: false,
        on_transport: None,
    };
    let v = spline.velocity();
    assert!((v[0] - 30.0).abs() < 0.1, "{v:?}");
    assert!(v[1].abs() < 0.01, "{v:?}");
    assert!((v[2] - 12.0).abs() < 0.1, "{v:?}");
    // …and its length is the speed the server stated, which is the property
    // that keeps a continuation from drifting ahead of or behind the ride.
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    assert!((speed - spline.speed()).abs() < 0.1, "{speed} against {}", spline.speed());

    // The heading is flat, which is the half this exists beside rather than
    // replaces: it is a bearing and says nothing about the climb.
    let (_, heading) = spline.position_and_heading();
    assert!(heading.abs() < 1e-3, "the bearing picked up a vertical: {heading}");
}

/// **A curve is differentiated by the same evaluator that positions it.**
///
/// A flying spline is walked as Catmull-Rom (`MoveSplineFlag::Flying` is
/// `Mask_CatmullRom`), so its velocity is the curve's tangent and not the
/// chord of the leg it is on. Measured through the same `sample` the position
/// comes out of, so the two cannot disagree — which is checked here by walking
/// the spline forward and comparing where it actually got to.
#[test]
fn a_flying_splines_velocity_agrees_with_where_it_goes() {
    let mut spline = Spline {
        path: vec![
            [0.0, 0.0, 100.0],
            [20.0, 0.0, 105.0],
            [40.0, 20.0, 115.0],
            [40.0, 40.0, 120.0],
        ],
        duration_ms: 4_000,
        elapsed_ms: 1_000,
        facing: SplineFacing::Travel,
        cyclic: false,
        flying: true,
        falling: false,
        on_transport: None,
    };
    let (from, _) = spline.position_and_heading();
    let v = spline.velocity();
    // Twenty-five milliseconds on — one simulation tick, which is the interval
    // a continuation actually has to cover.
    spline.elapsed_ms += 25;
    let (to, _) = spline.position_and_heading();
    for axis in 0..3 {
        let predicted = from[axis] + v[axis] * 0.025;
        assert!(
            (predicted - to[axis]).abs() < 0.02,
            "axis {axis}: predicted {predicted}, the curve reached {}",
            to[axis]
        );
    }
    // …and it is really climbing, so the agreement is not three zeroes.
    assert!(v[2] > 0.5, "{v:?}");
}

/// **A correction keeps the ride; building a fresh mover threw it away.**
///
/// This is the login bug, at the level it actually lives at.
/// `SessionLoop::enter_world` waits out the login burst and then adopts the
/// server's position — and it used to do that with `Mover::new`, which is a
/// mover with no ride in it and nothing owed.
///
/// That is invisible for every ordinary login and permanent for one:
/// `Player::ContinueTaxiFlight` runs at the end of `HandlePlayerLogin`, *after*
/// `Map::Add`, so logging in mid-flight puts the resumed `SMSG_MONSTER_MOVE` in
/// the same burst as the create block that ends the wait. The spline went, so
/// the character never flew; and `finished_ride` went with it, so the
/// `CMSG_MOVE_SPLINE_DONE` the server was waiting on was never sent — and
/// `HandleMovementOpcodes` discards **everything** while that is pending.
#[test]
fn adopting_the_servers_position_does_not_lose_a_ride_or_the_ack_it_owes() {
    let mut m = Mover::new(Position::default(), Speeds::default());
    m.ride(
        &MonsterMove {
            guid: 7,
            start: [0.0, 0.0, 100.0],
            spline_id: 41,
            path: vec![[0.0, 0.0, 100.0], [320.0, 0.0, 100.0]],
            duration_ms: 10_000,
            facing: SplineFacing::Travel,
            flags: spline_flags::FLYING,
            transport: None,
        },
        None,
    );
    assert!(m.is_riding());

    // What `enter_world` does with the self create block once the burst lands.
    m.resync(Position { x: 1.0, y: 2.0, z: 100.0, orientation: 0.5 });
    m.speeds = Speeds::default();

    assert!(m.is_riding(), "the resumed flight was thrown away");
    assert!(!m.can_move(), "the keys got the wheel back mid-flight");

    // …and it still ends, and still owes the ack that releases the server's
    // `SetSplineDonePending` — which is the half that made it permanent.
    for _ in 0..420 {
        m.advance(0.025, None);
    }
    assert!(!m.is_riding(), "the flight never finished");
    assert_eq!(m.take_finished_ride(), Some(41), "nothing was owed to the server");
}

/// **`UNIT_FLAG_TAXI_FLIGHT` grounds the keys**, which is the belt to the
/// braces above: the flag outlives a session and a ride does not.
///
/// Without it, a resumed flight that is ever missed again leaves the character
/// walking around on the ground while the server holds them on a gryphon and
/// throws every packet away. With it the worst case is a character who cannot
/// move until the server says the flight is over, which is what the server
/// believes anyway.
#[test]
fn the_servers_taxi_flag_stops_the_character_walking_with_no_ride_running() {
    let mut m = Mover::new(Position { x: 10.0, y: 10.0, z: 40.0, orientation: 0.0 }, Speeds::default());
    assert!(m.can_move() && m.can_turn());

    m.set_restraint(Restraint { stunned: false, dead: false, on_taxi: true });
    assert!(!m.can_move(), "the keys still had the wheel");
    assert!(!m.can_turn(), "the mouse still had the wheel");

    // Nothing is ridden and nothing is rooted, so this is the flag alone.
    m.set_controls(Controls { forward: true, ..Default::default() });
    let before = m.position();
    m.advance(0.5, None);
    assert!(
        (m.position().x - before.x).abs() < 1e-4 && (m.position().y - before.y).abs() < 1e-4,
        "walked to {:?} while the server was flying it",
        m.position()
    );

    // …and landing gives it back, which is `FlightPathMovementGenerator::
    // Finalize` clearing the flag.
    m.set_restraint(Restraint::default());
    assert!(m.can_move() && m.can_turn());
}

/// A constant slope, as a [`Footing`]: the ground is `grade` yards up for every
/// yard along `+x`, with no bottom and no top.
struct Slope {
    grade: f32,
}

impl Footing for Slope {
    fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(x * self.grade)
    }
}

/// **A jump taken while running uphill stays on top of the hill.**
///
/// The report is *"jumping uphill makes you fall through the terrain"*, and
/// what it is describing is the ascent rather than the landing. A jump leaves
/// the ground at 7.96 y/s and decelerates; a 45-degree slope under a 7.6 y/s
/// run rises at 7.6 y/s and does not. The two cross within a tick or two of the
/// apex, and until this was clamped the arc was written into the position
/// unconditionally — so the character, and the camera framed on them, spent the
/// rest of the ascent buried in the hillside. Measured before the clamp: 0.15
/// yards under the ground at 31 degrees, 1.34 at 45, 2.82 at 56.
///
/// Both halves are asserted. Never below the ground is the fix; still landing
/// on it is what stops the fix from being "the character can no longer jump".
#[test]
fn a_jump_uphill_never_goes_under_the_hill() {
    for grade in [0.3f32, 0.6, 1.0, 1.5] {
        let hill = Slope { grade };
        let mut m = Mover::new(
            Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
            Speeds::default(),
        );
        m.set_controls(Controls { forward: true, ..Default::default() });
        for _ in 0..20 {
            m.advance(0.025, Some(&hill));
        }
        m.jump();
        let mut deepest = 0.0f32;
        for _ in 0..200 {
            m.advance(0.025, Some(&hill));
            let p = m.position();
            deepest = deepest.max(p.x * grade - p.z);
        }
        assert!(
            deepest < 1e-3,
            "grade {grade}: the arc went {deepest} yards into the hill",
        );
        let p = m.position();
        assert!(
            (p.z - p.x * grade).abs() < 1e-3,
            "grade {grade}: ended at {p:?}, ground {}",
            p.x * grade,
        );
        assert!(!m.is_airborne(), "grade {grade}: still in the air after 5 s");
    }
}

/// **A character in the air over a point the client has no floor for holds**,
/// exactly as a walking one holds its altitude.
///
/// The two were opposite, and the airborne half is *"you fall through the floor
/// in Ironforge"*. `Footing::floor` answers `None` for **no data**, never for
/// no floor — which inside a building's own `MODF` box is every point the hull
/// does not cover: a doorway, a stair the file states no `MOPY` for, a group
/// still being read. A walking character keeps the height the server gave them
/// there; an airborne one used to write the parabola anyway and carry on down
/// through the world until vmangos' `UndermapRecall` caught them.
///
/// The arc must not advance either, or a second of held fall arrives all at
/// once the moment the geometry does.
#[test]
fn an_arc_over_a_point_with_no_data_holds_rather_than_falling() {
    /// Floor everywhere except a band of `x`, which answers "no data".
    struct Holed;
    impl Footing for Holed {
        fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
            (!(5.0..15.0).contains(&x)).then_some(0.0)
        }
    }

    let mut m = Mover::new(
        Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    m.set_controls(Controls { forward: true, ..Default::default() });
    m.advance(0.025, Some(&Holed));
    m.jump();
    // Run well past the far side of the hole and far past the arc's own length.
    for _ in 0..400 {
        m.advance(0.025, Some(&Holed));
    }
    let p = m.position();
    assert!(p.x > 15.0, "never crossed the hole: {p:?}");
    assert!(
        p.z > -0.001,
        "fell to {} while crossing a stretch with no floor data",
        p.z,
    );
}

/// A hill under a leg, as a [`Footing`]: the ground is `base` everywhere except
/// a hollow centred on `dip_at` that is `depth` yards deep and `half` wide.
///
/// A closure would do — `Footing` is implemented for one — but the three cases
/// below want the same shape three times and a named one says what it is.
struct Hill {
    base: f32,
    dip_at: f32,
    half: f32,
    depth: f32,
}

impl Footing for Hill {
    fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
        let d = (x - self.dip_at).abs();
        Some(if d >= self.half {
            self.base
        } else {
            self.base - self.depth * (1.0 - d / self.half)
        })
    }
}

/// The leg the three grounding tests are written against: twenty yards along
/// +x, both ends on flat ground at 100, with a four-yard hollow in the middle.
fn over_a_hollow(elapsed_ms: u32) -> (Spline, Hill) {
    (
        Spline {
            path: vec![[0.0, 0.0, 100.0], [20.0, 0.0, 100.0]],
            duration_ms: 1_000,
            elapsed_ms,
            facing: SplineFacing::Travel,
            cyclic: false,
            flying: false,
            falling: false,
            on_transport: None,
        },
        Hill { base: 100.0, dip_at: 10.0, half: 10.0, depth: 4.0 },
    )
}

/// **A creature walking across a hollow walks down into it.**
///
/// This is the floating-NPC report. The server states a straight line in three
/// dimensions between two points it grounded, and the ground between them is
/// not straight; a client that draws the chord leaves the creature hanging in
/// the air over every dip and buried in every rise. Measured across the whole
/// world database's ground spawns, a twenty-yard leg's chord is more than half
/// a yard off the ground under it **half the time**.
#[test]
fn a_spline_leg_follows_the_ground_between_the_points_the_server_stated() {
    let (spline, hill) = over_a_hollow(500);
    let (at, _) = spline.position_and_heading();
    // The chord says 100 — both ends are at 100 — and the ground says 96.
    assert!((at[2] - 100.0).abs() < 1e-4, "the chord moved: {at:?}");
    let z = spline.grounded_z(&hill, at).expect("terrain and a walkable leg");
    assert!((z - 96.0).abs() < 1e-3, "four yards of hollow ignored: {z}");
}

/// **…and it is exactly the server's own z at the points the server named.**
///
/// The whole safety of the rule is here: what is corrected is only the part
/// this client invented. At either end of a leg the answer must be the node's
/// stated height whatever the terrain under it is doing, so the client can
/// never disagree with the server about a position the server actually sent.
#[test]
fn the_stated_endpoints_of_a_leg_are_reproduced_exactly() {
    for (elapsed, expected) in [(0u32, 100.0f32), (1_000, 100.0)] {
        let (spline, hill) = over_a_hollow(elapsed);
        let (at, _) = spline.position_and_heading();
        let z = spline.grounded_z(&hill, at).expect("terrain");
        assert!((z - expected).abs() < 1e-3, "at {elapsed} ms: {z} rather than {expected}");
    }
}

/// **A creature the server put thirty yards up stays thirty yards up.**
///
/// The rule is not "snap to the ground", it is "carry the lift the server's own
/// endpoints have across the terrain" — which is what keeps a unit on a bridge
/// out of the canyon under it, and a swimming one off the lake bed. A plain
/// snap would drop both.
#[test]
fn a_leg_the_server_stated_above_the_ground_keeps_its_height() {
    let mut spline = over_a_hollow(500).0;
    let hill = over_a_hollow(0).1;
    spline.path = vec![[0.0, 0.0, 130.0], [20.0, 0.0, 130.0]];
    let (at, _) = spline.position_and_heading();
    let z = spline.grounded_z(&hill, at).expect("terrain");
    // Both ends are thirty above their own ground; the middle is thirty above
    // *its* ground, which the hollow has put at 96.
    assert!((z - 126.0).abs() < 1e-3, "{z}");
}

/// **Three splines whose vertical is not the ground's are left alone**, and a
/// world with no terrain answer is left alone too — the caller keeps the chord
/// in all four cases.
#[test]
fn a_flight_a_fall_and_a_map_with_no_terrain_are_not_grounded() {
    let hill = over_a_hollow(0).1;
    let at = [10.0, 0.0, 100.0];

    let mut flying = over_a_hollow(500).0;
    flying.flying = true;
    assert_eq!(flying.grounded_z(&hill, at), None, "a taxi flight was walked into the ground");

    let mut falling = over_a_hollow(500).0;
    falling.falling = true;
    assert_eq!(falling.grounded_z(&hill, at), None, "a knock-back was walked into the ground");

    // A leg with nothing to walk.
    let mut degenerate = over_a_hollow(500).0;
    degenerate.path = vec![[0.0, 0.0, 100.0]];
    assert_eq!(degenerate.grounded_z(&hill, at), None);

    // …and no data for the ground, which is the tile that has not arrived.
    let nothing = |_: f32, _: f32, _: f32| None;
    assert_eq!(over_a_hollow(500).0.grounded_z(&nothing, at), None);
}

/// **The correction is continuous across a node**, which is what stops it from
/// trading a float for a twitch: a creature walking a three-point path must not
/// step vertically as it passes the middle waypoint.
#[test]
fn crossing_a_waypoint_does_not_step() {
    let hill = Hill { base: 100.0, dip_at: 15.0, half: 10.0, depth: 3.0 };
    let mut spline = Spline {
        // Two ten-yard legs; the middle node sits on the flat at 100 and the
        // hollow straddles the join.
        path: vec![[0.0, 0.0, 100.0], [10.0, 0.0, 100.0], [20.0, 0.0, 98.0]],
        duration_ms: 2_000,
        elapsed_ms: 0,
        facing: SplineFacing::Travel,
        cyclic: false,
        flying: false,
        falling: false,
        on_transport: None,
    };
    let mut last: Option<f32> = None;
    for ms in (0..=2_000).step_by(20) {
        spline.elapsed_ms = ms;
        let (at, _) = spline.position_and_heading();
        let z = spline.grounded_z(&hill, at).expect("terrain");
        if let Some(previous) = last {
            assert!(
                (z - previous).abs() < 0.2,
                "a {:.2}y step at {ms} ms",
                (z - previous).abs()
            );
        }
        last = Some(z);
    }
}

/// **The twelve are a complete table, and run/walk runs backwards.**
///
/// `SendMovementFlagChangeToAll`'s switch pairs apply with clear on every flag
/// but one: run/walk comes from `SendToggleRunWalkToAll(unit, run)`, where the
/// flag is `MOVEFLAG_WALK_MODE`, so `SET_RUN_MODE` is the packet that *clears*
/// it. Getting that pair the obvious way round makes every creature the server
/// tells to run walk instead, at walk speed, for the rest of its life — there
/// is no third packet to correct it with.
#[test]
fn the_spline_flag_table_covers_twelve_and_pairs_run_mode_backwards() {
    use Opcode::*;
    let pairs = [
        (SMSG_SPLINE_MOVE_ROOT, SMSG_SPLINE_MOVE_UNROOT, move_flags::ROOT),
        (
            SMSG_SPLINE_MOVE_WATER_WALK,
            SMSG_SPLINE_MOVE_LAND_WALK,
            move_flags::WATERWALKING,
        ),
        (
            SMSG_SPLINE_MOVE_FEATHER_FALL,
            SMSG_SPLINE_MOVE_NORMAL_FALL,
            move_flags::SAFE_FALL,
        ),
        (
            SMSG_SPLINE_MOVE_SET_HOVER,
            SMSG_SPLINE_MOVE_UNSET_HOVER,
            move_flags::HOVER,
        ),
        (
            SMSG_SPLINE_MOVE_START_SWIM,
            SMSG_SPLINE_MOVE_STOP_SWIM,
            move_flags::SWIMMING,
        ),
        // …and the one that reads the other way: the *walk* opcode applies.
        (
            SMSG_SPLINE_MOVE_SET_WALK_MODE,
            SMSG_SPLINE_MOVE_SET_RUN_MODE,
            move_flags::WALK_MODE,
        ),
    ];
    for (on, off, flag) in pairs {
        let set = SplineFlagChange::of(on).expect("an apply");
        let clear = SplineFlagChange::of(off).expect("a clear");
        assert_eq!(set.flag, flag, "{on:?}");
        assert_eq!(clear.flag, flag, "{off:?}");
        assert!(set.apply, "{on:?} should apply");
        assert!(!clear.apply, "{off:?} should clear");
    }

    // Neither of the other two families answers here, which is what keeps the
    // dispatch's three guards from stealing each other's packets: these are
    // never acknowledged and those two must be.
    assert!(SplineFlagChange::of(SMSG_FORCE_MOVE_ROOT).is_none());
    assert!(SplineFlagChange::of(MSG_MOVE_ROOT).is_none());
    assert!(SplineFlagChange::of(SMSG_SPLINE_SET_RUN_SPEED).is_none());
}

/// The whole body is a packed guid, and an empty one is refused.
#[test]
fn a_spline_flag_body_is_a_packed_guid_and_nothing_else() {
    // Mask 0x01 says "byte 0 only", so this is guid 0x2A.
    assert_eq!(parse_spline_flag(&[0x01, 0x2A]), Some(0x2A));
    // A full eight-byte guid, mask 0xFF.
    let mut body = vec![0xFFu8];
    body.extend_from_slice(&0x0123_4567_89AB_CDEFu64.to_le_bytes());
    assert_eq!(parse_spline_flag(&body), Some(0x0123_4567_89AB_CDEF));
    // **The two that are not a packet**: an empty body, and a mask of zero.
    // Both would otherwise read as guid 0, which names nothing and would be
    // applied to an entity conjured for it.
    assert_eq!(parse_spline_flag(&[]), None);
    assert_eq!(parse_spline_flag(&[0x00]), None);
}

/// **A cleared flag and a flag nothing has spoken about are different
/// answers**, which is the whole reason the override is two words.
#[test]
fn an_override_states_bits_rather_than_masking_them() {
    let mut o = FlagOverride::default();
    assert!(o.is_empty());
    // Silence leaves the block alone, bit for bit.
    let base = move_flags::FORWARD | move_flags::ROOT | move_flags::WALK_MODE;
    assert_eq!(o.over(base), base);

    // An unroot has to *beat* a ROOT that is already in the block. A one-word
    // "these bits are set" mask cannot say this at all.
    o.state(SplineFlagChange::of(Opcode::SMSG_SPLINE_MOVE_UNROOT).expect("an unroot"));
    assert_eq!(o.over(base), move_flags::FORWARD | move_flags::WALK_MODE);
    assert!(!o.is_empty());

    // …and the bits it has not spoken about are still the block's.
    assert_eq!(
        o.over(move_flags::SWIMMING),
        move_flags::SWIMMING,
        "an unroot said nothing about swimming"
    );

    // A later statement replaces the earlier one rather than accumulating.
    o.state(SplineFlagChange::of(Opcode::SMSG_SPLINE_MOVE_ROOT).expect("a root"));
    assert_eq!(o.over(0), move_flags::ROOT);
}

/// `MSG_MOVE_TELEPORT` is the observer half of a near teleport, and it is the
/// same body every other broadcast has.
///
/// Sent twice per teleport — once around the old position and once around the
/// new — both carrying the destination, so applying it must be idempotent.
#[test]
fn a_near_teleport_is_an_ordinary_movement_broadcast() {
    assert!(is_broadcast_movement(Opcode::MSG_MOVE_TELEPORT));
    // …and its *ack*, which is the mover's own packet, is not one of these.
    assert!(!is_broadcast_movement(Opcode::MSG_MOVE_TELEPORT_ACK));

    let mut w = Writer::new();
    w.packed_guid(0x2A);
    let info = MovementInfo {
        flags: move_flags::NONE,
        position: Position { x: 1.0, y: 2.0, z: 3.0, orientation: 0.5 },
        ..MovementInfo::default()
    };
    info.write(&mut w);
    let (guid, read) = parse_movement_broadcast(&w.buf).expect("a broadcast body");
    assert_eq!(guid, 0x2A);
    assert_eq!(read.position.x, 1.0);
}

/// **The two passenger transforms are exact inverses**, which is what makes a
/// character standing still on a moving deck stay put on it.
///
/// `GenericTransport::CalculatePassengerOffset` and `CalculatePassengerPosition`
/// — a yaw and a translation, no pitch and no roll. If they were not inverses
/// the offset re-derived each tick would drift, and a passenger would creep
/// across the deck for as long as they stood on it.
#[test]
fn boarding_and_being_carried_are_inverses() {
    let platform = Platform {
        guid: 0xF110_0000_0000_002A,
        position: [-100.0, 250.0, 30.0],
        facing: 1.1,
    };
    let standing = Position { x: -95.0, y: 253.0, z: 32.5, orientation: 2.4 };

    let ferry = Ferry::aboard(platform, standing);
    let back = ferry.carry(platform);
    assert!((back.x - standing.x).abs() < 1e-3, "x {} vs {}", back.x, standing.x);
    assert!((back.y - standing.y).abs() < 1e-3, "y {} vs {}", back.y, standing.y);
    assert!((back.z - standing.z).abs() < 1e-3);
    assert!((back.orientation - standing.orientation).abs() < 1e-3);
}

/// **A platform that moves takes its passenger with it, rigidly.**
///
/// The Deeprun Tram's car travels 2,482 yards along one axis, so the case that
/// matters is a long translation: the passenger has to arrive the same distance
/// away, at the same spot on the deck.
#[test]
fn a_passenger_travels_the_platforms_whole_distance() {
    let start = Platform { guid: 7, position: [0.0, 0.0, 0.0], facing: 0.0 };
    let standing = Position { x: 3.0, y: -2.0, z: 1.5, orientation: 0.5 };
    let ferry = Ferry::aboard(start, standing);

    // …and 2,482 yards later, which is the tram's own travel.
    let moved = Platform { position: [2482.0, 0.0, 0.0], ..start };
    assert!(ferry.moved(moved));
    let carried = ferry.carry(moved);
    assert!((carried.x - (standing.x + 2482.0)).abs() < 1e-2, "{carried:?}");
    assert!((carried.y - standing.y).abs() < 1e-3);
    assert!((carried.z - standing.z).abs() < 1e-3);

    // A platform that has not moved wants no carry at all, which is what keeps
    // this off the hot path for every stationary game object in the world.
    assert!(!ferry.moved(start));
}

/// **A turning platform turns its passenger**, position and facing together.
///
/// A quarter turn about the platform's origin puts somebody standing a yard
/// along its x-axis a yard along its y-axis instead, and rotates the direction
/// they are looking by the same quarter turn.
#[test]
fn a_turning_platform_turns_its_passenger() {
    let start = Platform { guid: 7, position: [10.0, 10.0, 0.0], facing: 0.0 };
    let standing = Position { x: 11.0, y: 10.0, z: 0.0, orientation: 0.0 };
    let ferry = Ferry::aboard(start, standing);

    let turned = Platform { facing: std::f32::consts::FRAC_PI_2, ..start };
    let carried = ferry.carry(turned);
    assert!((carried.x - 10.0).abs() < 1e-3, "{carried:?}");
    assert!((carried.y - 11.0).abs() < 1e-3, "{carried:?}");
    assert!(
        (carried.orientation - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
        "the passenger did not turn with the deck"
    );
}

/// **Boarding sets the flag and the block; stepping off takes both back out —
/// and leaves the character where they were standing.**
///
/// The server boards a player *only* on their own packet carrying the flag and
/// the guid (`HandleMoverRelocation` -> `AddPassenger`), so the flag is not
/// decoration: without it nothing on the server side ever moves the passenger.
///
/// The step-off half matters as much. Walking down a gangway is not a teleport,
/// so the position is untouched — the one thing `carry_platform(None)` must not
/// do is recompute anything.
#[test]
fn boarding_sets_the_flag_and_stepping_off_leaves_you_where_you_stood() {
    let mut mover = Mover::new(
        Position { x: 5.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    let platform = Platform { guid: 42, position: [0.0, 0.0, 0.0], facing: 0.0 };

    assert!(!mover.carry_platform(Some(platform)), "a first boarding moves nobody");
    assert!(mover.info.has(move_flags::ONTRANSPORT));
    assert_eq!(mover.info.transport.map(|t| t.guid), Some(42));
    assert!((mover.info.transport.expect("aboard").position.x - 5.0).abs() < 1e-3);

    // The deck moves ten yards, and so does the passenger.
    let moved = Platform { position: [10.0, 0.0, 0.0], ..platform };
    assert!(mover.carry_platform(Some(moved)), "the carry did not happen");
    assert!((mover.position().x - 15.0).abs() < 1e-3, "{:?}", mover.position());

    // …and off. The flag and the block go; the position stays.
    let ashore = mover.position();
    assert!(!mover.carry_platform(None));
    assert!(!mover.info.has(move_flags::ONTRANSPORT));
    assert_eq!(mover.info.transport, None);
    assert_eq!(mover.position().x, ashore.x, "stepping off is not a teleport");
}

/// **Both edges of a boarding are owed the server at once**, which is the one
/// thing that makes a passenger a passenger.
///
/// `HandleMoverRelocation` is the only place in vmangos that boards or lands
/// anybody, and it acts on the flag in the packet it is given. So a client that
/// leaves a boarding to the next heartbeat is not on the boat for as long as the
/// gap lasts — and `Transport::TeleportTransport` walks `m_passengers`, so a
/// crossing taken during that gap leaves the character in the ocean.
///
/// `ferry_changed` is the edge and `mark_sent` is what clears it. Asserted in
/// both directions because the step ashore is the half that is easy to leave
/// out: without it the server keeps the passenger and the boat drags them away
/// from the dock they walked onto.
#[test]
fn boarding_and_stepping_ashore_are_each_owed_a_packet() {
    let mut mover = Mover::new(
        Position { x: 5.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    let platform = Platform { guid: 42, position: [0.0, 0.0, 0.0], facing: 0.0 };
    assert!(!mover.ferry_changed(), "nothing has happened yet");

    mover.carry_platform(Some(platform));
    assert!(mover.ferry_changed(), "the server has not been told we boarded");
    mover.mark_sent();
    assert!(!mover.ferry_changed(), "and now it has");

    // A whole crossing on the same deck is not an edge — the heartbeat covers it.
    let moved = Platform { position: [10.0, 0.0, 0.0], ..platform };
    mover.carry_platform(Some(moved));
    mover.stow_platform();
    assert!(!mover.ferry_changed(), "a ride is not a boarding");

    // …and stepping ashore is.
    mover.carry_platform(None);
    assert!(mover.ferry_changed(), "the server has not been told we left");
    mover.mark_sent();
    assert!(!mover.ferry_changed());

    // Stepping from one deck to another is both at once, and one packet says so.
    mover.carry_platform(Some(platform));
    mover.mark_sent();
    mover.carry_platform(Some(Platform { guid: 43, ..platform }));
    assert!(mover.ferry_changed(), "a different deck is a different passenger");
}

/// **A far teleport forgets the deck rather than carrying the difference.**
///
/// The offset on record is a coordinate in a frame, and the frame is where the
/// platform stood when it was taken. A boat that has just crossed to the other
/// continent is the same guid at a placement an ocean away, so the first carry
/// after the port would move the character by the whole width of the world.
///
/// Dropping it re-boards through the first-boarding arm on the next tick, which
/// moves nobody — the character stays exactly where the server put them.
#[test]
fn a_teleport_forgets_the_deck_instead_of_carrying_an_ocean() {
    let mut mover = Mover::new(
        Position { x: 5.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    let menethil = Platform { guid: 42, position: [-3700.0, -575.0, 0.0], facing: 0.0 };
    mover.carry_platform(Some(menethil));
    mover.stow_platform();

    // The server ports us onto the same boat, on the other continent.
    let landed = Position { x: 5810.0, y: 1240.0, z: 0.0, orientation: 0.0 };
    mover.disembark();
    mover.resync(landed);
    assert!(!mover.info.has(move_flags::ONTRANSPORT), "the block went with it");

    // …and the port itself owes the server nothing: it is the server that
    // decided whether we are still aboard, and a no-flag packet here would land
    // us in the sea. The re-boarding a tick later is the edge that is reported.
    assert!(!mover.ferry_changed(), "a port is not a step ashore");

    let theramore = Platform { guid: 42, position: [5805.0, 1240.0, 0.0], facing: 0.0 };
    assert!(!mover.carry_platform(Some(theramore)), "a re-boarding moves nobody");
    assert!((mover.position().x - landed.x).abs() < 1e-3, "{:?}", mover.position());
    assert!((mover.position().y - landed.y).abs() < 1e-3, "{:?}", mover.position());
    assert!(mover.ferry_changed(), "…but the re-boarding is owed a packet");
}

/// **…and the boat's own teleport keeps it, by adopting the offset and
/// composing the position from the deck's first placement on the new map.**
///
/// The other half of the test above, and the difference is one packet:
/// `SMSG_TRANSFER_PENDING`'s two extra dwords, which `ExecuteTeleportFar`
/// writes only for a character standing on a transport. And the position of
/// the `SMSG_NEW_WORLD` that follows is not a position at all for a passenger:
/// `Player::SendNewWorld` writes `m_movementInfo.GetTransportPos()` — the
/// offset — when `m_transport` is set. `HandleMoveWorldportAck` then relocates
/// the passenger from the offset (`UpdatePassengerPosition`), never to any
/// coordinate it sent, so the offset is the whole of what survives the
/// crossing and the world position is owed to the first real placement.
#[test]
fn a_transports_own_teleport_recomposes_from_the_offset() {
    let mut mover = Mover::new(
        Position { x: -3695.0, y: -580.0, z: 12.0, orientation: 1.0 },
        Speeds::default(),
    );
    let menethil = Platform { guid: 42, position: [-3700.0, -575.0, 0.0], facing: 0.8 };
    mover.carry_platform(Some(menethil));
    mover.stow_platform();
    mover.mark_sent();
    let offset = mover.ferry().expect("aboard").offset;

    // The boat crosses: `SMSG_NEW_WORLD` restates the offset and nothing else.
    mover.transfer_aboard(offset);
    assert!(mover.info.has(move_flags::ONTRANSPORT), "the flag survives");
    assert!(!mover.ferry_changed(), "a port is not an edge");

    // The far side's own create block restates the server-composed position,
    // and the session resyncs to it — a map-scale write that must not be folded
    // into the offset by the next carry or stow.
    let theramore = Platform { guid: 42, position: [5807.6, 1241.0, 0.0], facing: 2.6 };
    let composed = mover.ferry().expect("aboard").carry(theramore);
    mover.resync(composed);
    mover.stow_platform();
    assert_eq!(
        mover.ferry().expect("aboard").offset,
        offset,
        "the offset is frozen across the transfer"
    );

    // The deck's first placement on the new map: the position is rebuilt from
    // the offset absolutely — exactly where the character stood on the deck —
    // rather than by a delta between two frames an ocean apart.
    assert!(mover.carry_platform(Some(theramore)), "the recompose is a carry");
    let landed = mover.position();
    let expected = composed;
    assert!((landed.x - expected.x).abs() < 1e-2, "{landed:?} against {expected:?}");
    assert!((landed.y - expected.y).abs() < 1e-2, "{landed:?} against {expected:?}");
    assert!((landed.z - expected.z).abs() < 1e-2, "{landed:?} against {expected:?}");
    // The stow after the recompose re-derives the offset through
    // `carry` ∘ `aboard`, which agrees to float precision rather than bit for
    // bit.
    let held = mover.ferry().expect("aboard").offset;
    assert!((held.x - offset.x).abs() < 1e-2, "{held:?} against {offset:?}");
    assert!((held.y - offset.y).abs() < 1e-2, "{held:?} against {offset:?}");
    assert!((held.z - offset.z).abs() < 1e-2, "{held:?} against {offset:?}");
    assert!(!mover.ferry_changed(), "nor is it a boarding: we never left");
}

/// **A resync between the transfer and the recompose cannot fling the
/// character.** The sequence that stranded a Ratchet–Booty Bay passenger in
/// the open sea: `SMSG_NEW_WORLD`'s offset adopted as a position, the far
/// side's create block resyncing to the real one fourteen thousand yards away,
/// and the next delta carry folding the difference into the offset — which
/// then went out as `t_pos`, and the server composes a passenger from `t_pos`
/// alone.
#[test]
fn a_resync_between_transfer_and_recompose_cannot_fling() {
    let mut mover = Mover::new(
        Position { x: -1650.0, y: -5100.0, z: 8.0, orientation: 0.4 },
        Speeds::default(),
    );
    let ratchet = Platform { guid: 42, position: [-1652.7, -5107.1, 0.0], facing: 0.9 };
    mover.carry_platform(Some(ratchet));
    mover.stow_platform();
    mover.mark_sent();
    let offset = mover.ferry().expect("aboard").offset;

    mover.transfer_aboard(offset);
    // A resync to somewhere else entirely — the offset misread as a position,
    // a create block, either.
    mover.resync(Position { x: 3.0, y: -12.0, z: 8.0, orientation: 0.0 });
    mover.stow_platform();

    let booty_bay = Platform { guid: 42, position: [-14363.0, 1253.0, 0.0], facing: 1.7 };
    mover.carry_platform(Some(booty_bay));
    let landed = mover.position();
    let deck = (
        (landed.x - booty_bay.position[0]).powi(2) + (landed.y - booty_bay.position[1]).powi(2)
    )
    .sqrt();
    let radius = (offset.x * offset.x + offset.y * offset.y).sqrt();
    assert!(
        (deck - radius).abs() < 1e-1,
        "landed {deck} yards from the deck against an offset of {radius}"
    );
    let held = mover.ferry().expect("aboard").offset;
    assert!((held.x - offset.x).abs() < 1e-2, "{held:?} against {offset:?}");
    assert!((held.y - offset.y).abs() < 1e-2, "{held:?} against {offset:?}");
}

/// **The hold running out mid-transfer is not a step ashore.** Between the
/// teleport and the deck's first placement the world can answer nothing about
/// the ferry, and taking it would send a no-flag packet — `RemovePassenger`,
/// and `TeleportTransport` moves everybody but us.
#[test]
fn a_transfer_in_flight_is_not_unboarded_by_an_empty_answer() {
    let mut mover = Mover::new(
        Position { x: -1650.0, y: -5100.0, z: 8.0, orientation: 0.4 },
        Speeds::default(),
    );
    let ratchet = Platform { guid: 42, position: [-1652.7, -5107.1, 0.0], facing: 0.9 };
    mover.carry_platform(Some(ratchet));
    mover.stow_platform();
    mover.mark_sent();
    let offset = mover.ferry().expect("aboard").offset;
    mover.transfer_aboard(offset);

    assert!(!mover.carry_platform(None), "nothing to carry");
    assert!(mover.ferry().is_some(), "the ferry survives the empty answer");
    assert!(mover.info.has(move_flags::ONTRANSPORT), "and the flag with it");
    assert!(!mover.ferry_changed(), "no edge went out");
}

/// **A passenger who walks while the deck moves keeps the ground they covered**
/// — the report, stated: *"the user cannot move around, and is essentially
/// frozen until the ride completes"*.
///
/// The pair is the point. `carry_platform` moves the character by however far
/// the platform moved and takes the offset on the spot; `advance` then takes
/// the stride the keys asked for; and `stow_platform` records where that stride
/// ended. Leave the second out and the offset the next carry reads is the one
/// from *before* the stride, so every step is undone by the following tick — on
/// a stationary deck the carry never fires and walking works, which is exactly
/// why it read as a ride that freezes you rather than as anything about
/// transports at all.
///
/// Ten ticks of a deck moving a yard each, with `W` held. The passenger must
/// end up ten yards further along the deck *and* ten yards further along
/// because the deck moved.
#[test]
fn a_passenger_walks_while_the_deck_moves() {
    let mut mover = Mover::new(
        Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    mover.set_controls(Controls { forward: true, ..Default::default() });
    let mut platform = Platform { guid: 42, position: [0.0, 0.0, 0.0], facing: 0.0 };

    const TICKS: usize = 10;
    const DT: f32 = 0.1;
    for _ in 0..TICKS {
        mover.carry_platform(Some(platform));
        mover.advance(DT, None);
        mover.stow_platform();
        platform.position[0] += 1.0;
    }

    // The deck moved nine yards under them (the tenth is applied to the
    // platform after the last tick and is carried on the next one), and they
    // walked run-speed x 1 s along it.
    let walked = Speeds::default().run() * TICKS as f32 * DT;
    let expected = walked + (TICKS - 1) as f32 * 1.0;
    assert!(
        (mover.position().x - expected).abs() < 1e-2,
        "walked to {:?}, wanted x = {expected} (deck {} + stride {walked})",
        mover.position(),
        (TICKS - 1) as f32,
    );
    // …and the offset on the wire is the walk alone, which is what the server
    // rebuilds the world position from.
    let offset = mover.info.transport.expect("still aboard").position;
    assert!(
        (offset.x - walked).abs() < 1e-2,
        "the block says {offset:?}, wanted x = {walked}",
    );
}

/// **Mouse-look turns a passenger, and the deck does not put the old heading
/// back.**
///
/// The report is *"character rotation when on a transport moving: the
/// mouse-controlled rotate does not work"*, and it is the frozen stride one
/// axis over. `Mover::face` writes an absolute world heading and it is drained
/// *before* `tick_movement` — so a carry that recomposed the position from the
/// recorded offset overwrote the heading with the one from a tick ago. The turn
/// keys were unaffected, because `advance` turns *after* the carry, which is
/// why it read as "the mouse does not work" rather than "you cannot turn".
///
/// Both halves are asserted: the mouse heading survives, and a deck that
/// genuinely turns still turns its passenger.
#[test]
fn mouse_look_turns_a_passenger_and_the_deck_does_not_undo_it() {
    let mut mover = Mover::new(
        Position { x: 5.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    let mut platform = Platform { guid: 42, position: [0.0, 0.0, 0.0], facing: 0.0 };
    mover.carry_platform(Some(platform));
    mover.stow_platform();

    // The tick a player drags the mouse: `face` lands from the command queue,
    // then the deck moves a yard and the tick runs.
    for step in 1..=5 {
        mover.face(1.5);
        platform.position[0] += 1.0;
        mover.carry_platform(Some(platform));
        mover.advance(0.025, None);
        mover.stow_platform();
        assert!(
            (mover.position().orientation - 1.5).abs() < 1e-3,
            "step {step}: the deck put the heading back to {}",
            mover.position().orientation,
        );
    }

    // …and a deck that turns underneath a passenger who is *not* touching the
    // mouse still turns them, or the assertion above is satisfied by the carry
    // having stopped carrying the heading at all.
    let quarter = std::f32::consts::FRAC_PI_2;
    platform.facing = quarter;
    mover.carry_platform(Some(platform));
    mover.stow_platform();
    assert!(
        (mover.position().orientation - (1.5 + quarter)).abs() < 1e-3,
        "a quarter turn of the deck left the passenger at {}",
        mover.position().orientation,
    );
}

/// **A server correction reaches a passenger** — the same fault as the mouse,
/// through the other door that writes the position before `tick_movement`.
///
/// `Mover::resync` is called from the top of the tick whenever the player's own
/// update-field position changes: a teleport, a knockback the server resolved,
/// an anticheat snap-back. A carry that recomposed from the recorded offset put
/// the character straight back where they had been, so the correction lasted
/// exactly one tick and the server would send it again.
#[test]
fn a_correction_reaches_a_passenger_rather_than_being_carried_over() {
    let mut mover = Mover::new(
        Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    let mut platform = Platform { guid: 42, position: [0.0, 0.0, 0.0], facing: 0.0 };
    mover.carry_platform(Some(platform));
    mover.stow_platform();

    // The server puts the character three yards across the deck…
    mover.resync(Position { x: 0.0, y: 3.0, z: 0.0, orientation: 0.0 });
    // …and the deck then moves a yard.
    platform.position[0] += 1.0;
    mover.carry_platform(Some(platform));
    mover.stow_platform();

    let p = mover.position();
    assert!(
        (p.y - 3.0).abs() < 1e-3 && (p.x - 1.0).abs() < 1e-3,
        "the correction was carried over: {p:?}",
    );
}

/// **A jump taken on a rising lift lands on the lift.**
///
/// The vertical is walked from `Airborne::start` and overwrites whatever the
/// carry put in `z`, so an anchor left in the world's frame measures the
/// parabola against ground the character is no longer over: on a lift climbing
/// at 4 y/s a one-second jump comes down four yards below the deck, which the
/// floor query then reads as having fallen through it.
#[test]
fn a_jump_on_a_rising_lift_rides_it() {
    let mut mover = Mover::new(
        Position { x: 0.0, y: 0.0, z: 100.0, orientation: 0.0 },
        Speeds::default(),
    );
    let mut platform = Platform { guid: 9, position: [0.0, 0.0, 100.0], facing: 0.0 };
    mover.carry_platform(Some(platform));
    mover.stow_platform();
    mover.jump();

    // Half a second of lift at 4 y/s, in the ticks the session thread takes.
    for _ in 0..20 {
        platform.position[2] += 4.0 * 0.025;
        mover.carry_platform(Some(platform));
        mover.advance(0.025, None);
        mover.stow_platform();
    }

    // The offset is what the arc is really measured by: it must be the height
    // an identical jump reaches over still ground, not that minus the two yards
    // the lift climbed.
    let offset = mover.info.transport.expect("still aboard").position;
    let mut still = Mover::new(
        Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 },
        Speeds::default(),
    );
    still.jump();
    for _ in 0..20 {
        still.advance(0.025, None);
    }
    assert!(
        (offset.z - still.position().z).abs() < 1e-2,
        "jumped to {offset:?} above the deck; the same jump over still ground reaches {}",
        still.position().z,
    );
}

/// **The transport block round-trips through the wire in the position the
/// server reads it from**, which is between the main position and the fall time.
///
/// `MovementInfo::Read` takes the transport section immediately after the four
/// position floats and before the swim pitch — so a writer that put it anywhere
/// else would shift every field after it, and the server would read the fall
/// time out of a coordinate.
#[test]
fn a_boarded_block_round_trips_with_its_transport_section() {
    let info = MovementInfo {
        flags: move_flags::ONTRANSPORT | move_flags::FORWARD,
        time: 4242,
        position: Position { x: 1.0, y: 2.0, z: 3.0, orientation: 0.25 },
        transport: Some(TransportInfo {
            guid: 0xF110_0000_0000_002A,
            position: Position { x: -4.0, y: 5.0, z: 0.5, orientation: 1.0 },
        }),
        fall_time: 777,
        ..MovementInfo::default()
    };
    let mut w = Writer::new();
    info.write(&mut w);
    let read = MovementInfo::read(&mut Reader::new(&w.buf)).expect("a block");
    assert_eq!(read.transport.map(|t| t.guid), Some(0xF110_0000_0000_002A));
    assert_eq!(read.transport.map(|t| t.position.x), Some(-4.0));
    assert_eq!(read.fall_time, 777, "the section shifted the fields after it");
}

/// **`SMSG_MONSTER_MOVE_TRANSPORT` is one extra packed guid, and everything
/// after it is local.**
///
/// `MoveSplineInit::Launch` builds `SMSG_MONSTER_MOVE` and then, for a unit it
/// is boarding, changes the opcode and writes the transport's guid before the
/// rest. So the plain parser reads the *transport's* guid bytes as the start
/// coordinates: it succeeds, and every number after it is wrong.
#[test]
fn a_transport_monster_move_carries_a_second_guid_before_the_path() {
    let mut w = Writer::new();
    w.packed_guid(9); // the unit
    w.packed_guid(0xF110_0000_0000_002A); // …and the deck it is on
    w.f32(1.0).f32(2.0).f32(3.0); // start, in the transport's frame
    w.u32(77); // spline id
    w.u8(0); // faceType NORMAL
    w.u32(0); // flags
    w.u32(1000); // duration
    w.u32(1); // one node
    w.f32(4.0).f32(5.0).f32(6.0); // the destination

    let mm = parse_monster_move_transport(&w.buf).expect("a transport move");
    assert_eq!(mm.guid, 9);
    assert_eq!(mm.transport, Some(0xF110_0000_0000_002A));
    assert_eq!(mm.start, [1.0, 2.0, 3.0]);
    assert_eq!(mm.destination(), Some([4.0, 5.0, 6.0]));

    // **…and the plain parser cannot get it right**, which is the point of the
    // opcode having its own arm. On this body it runs out of bytes and answers
    // `None`; on a longer one — a real path has several nodes — it succeeds and
    // reads the transport's guid as the unit's start coordinates. Asserted as
    // "not the right answer" rather than as either outcome, because which of the
    // two happens is a property of the packet's length and neither is a reading
    // anybody should rely on.
    let wrong = parse_monster_move(&w.buf);
    assert!(
        wrong.as_ref().is_none_or(|m| m.start != [1.0, 2.0, 3.0]),
        "the plain parser read a transport move correctly, which it cannot"
    );
}

/// **A passenger's path is converted against the platform, and a platform this
/// client is not holding leaves the unit alone.**
///
/// The alternative to the second half is taking a local path as world
/// coordinates, which places every unit on every boat within a few yards of the
/// map's origin at once.
#[test]
fn a_transport_spline_is_walked_in_the_platforms_frame() {
    struct Deck(Option<Platform>);
    impl Footing for Deck {
        fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
            None
        }
        fn platform_of(&self, _guid: u64) -> Option<Platform> {
            self.0
        }
    }

    let spline = Spline {
        path: vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        duration_ms: 1000,
        elapsed_ms: 0,
        facing: SplineFacing::Travel,
        cyclic: false,
        flying: false,
        falling: false,
        on_transport: Some(7),
    };

    // The deck is 500 yards away and turned a quarter turn, so the path's own
    // origin lands there and its +x leg runs along the deck's +y.
    let deck = Deck(Some(Platform {
        guid: 7,
        position: [500.0, -20.0, 12.0],
        facing: std::f32::consts::FRAC_PI_2,
    }));
    let (at, _) = spline.in_world(Some(&deck)).expect("a placed deck");
    assert!((at[0] - 500.0).abs() < 1e-3, "{at:?}");
    assert!((at[1] + 20.0).abs() < 1e-3, "{at:?}");
    assert!((at[2] - 12.0).abs() < 1e-3, "{at:?}");

    // **No deck, no answer.** Left where it was beats placed at the origin.
    assert_eq!(spline.in_world(Some(&Deck(None))), None);
    assert_eq!(spline.in_world(None), None);

    // …and an ordinary spline is the identity, with or without a world.
    let plain = Spline { on_transport: None, ..spline };
    assert_eq!(plain.in_world(None), Some(plain.position_and_heading()));
}

