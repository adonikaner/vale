//! The others in a shared session, drawn in the viewport in their colours.
//!
//! Each person on the open map is a pole standing at what their camera looks
//! at, with their name on a label plate above it, and a ring on the ground
//! under their pointer. The chunks they changed in the last [`RECENT`]
//! seconds are outlined on the ground, so a stroke is seen being made and two
//! editors can keep apart. A person on another map is not drawn; the panel
//! says where they are.

use super::{colour, Collab};
use crate::labels::{Label, Source, View, Weight};
use crate::marks::{Look, Marks};
use bevy::prelude::*;
use vale_client::render::axes;

/// The pole's height and the ball on top of it, in yards.
const POLE: f32 = 6.0;
const KNOB: f32 = 0.6;
/// The pointer ring's radius, in yards.
const RING: f32 = 1.6;
/// How far a person is drawn from the camera, in yards.
const RANGE: f32 = 2000.0;
/// How long a chunk another editor changed stays outlined, in seconds.
pub(super) const RECENT: f64 = 1.5;
/// How many pieces each side of an outline is cut into, so it follows the
/// ground.
const PIECES: usize = 8;

pub(super) fn draw(
    collab: Res<Collab>,
    session: Option<Res<crate::session::EditSession>>,
    state: Res<crate::playtest::Playtest>,
    mut marks: ResMut<Marks>,
    mut labels: ResMut<crate::labels::Labels>,
    camera: Query<(&Camera, &Transform), With<vale_client::world::camera::WorldCamera>>,
) {
    let (Some(session), Some(view)) = (session, View::of(&camera)) else {
        labels.clear(Source::People);
        return;
    };
    if !state.editing() || collab.people.len() < 2 {
        labels.clear(Source::People);
        return;
    }
    let bevy = |at: Vec3| axes::to_bevy(at.to_array());
    for (user, tile, chunk) in collab.recent.keys() {
        if !tile.map.eq_ignore_ascii_case(&session.map) {
            continue;
        }
        let Some(open) = session.tiles.get(&(tile.x, tile.y)) else { continue };
        let [r, g, b] = colour(*user);
        let tint = Color::srgb_u8(r, g, b);
        let [x, y] = vale_edit::adt::blank::chunk_origin(tile.x, tile.y, *chunk);
        let side = vale_assets::world::adt::CHUNK_SIZE;
        let corners = [[x, y], [x - side, y], [x - side, y - side], [x, y - side], [x, y]];
        let on_ground = |at: [f32; 2]| {
            let height = vale_edit::adt::heights::height_at(open, at[0], at[1]).unwrap_or(0.0);
            Vec3::new(at[0], at[1], height)
        };
        for pair in corners.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            for piece in 0..PIECES {
                let along = |i: usize| {
                    let t = i as f32 / PIECES as f32;
                    [from[0] + (to[0] - from[0]) * t, from[1] + (to[1] - from[1]) * t]
                };
                marks.ground_line(bevy(on_ground(along(piece))), bevy(on_ground(along(piece + 1))), tint, Look::Ghosted);
            }
        }
    }
    let mut names = Vec::new();
    for (user, person) in &collab.people {
        if Some(*user) == collab.me {
            continue;
        }
        let Some(presence) = &person.presence else { continue };
        if !presence.map.eq_ignore_ascii_case(&session.map) {
            continue;
        }
        let [r, g, b] = colour(*user);
        let tint = Color::srgb_u8(r, g, b);
        let focus = Vec3::from(presence.camera);
        if focus.distance(view.eye()) > RANGE {
            continue;
        }
        let top = focus + Vec3::Z * POLE;
        marks.line(bevy(focus), bevy(top), tint, Look::Ghosted);
        marks.sphere(bevy(top), KNOB, tint, Look::Ghosted);
        if let Some(cursor) = presence.cursor.map(Vec3::from) {
            marks.wide_ring(bevy(cursor), Vec3::Y, RING, 2.0, tint, Look::Ghosted);
        }
        if let Some((at, away)) = view.project(top + Vec3::Z * 1.5) {
            names.push(Label { at, text: person.name.clone(), colour: tint, weight: Weight::Normal, away });
        }
    }
    labels.replace(Source::People, names);
}
