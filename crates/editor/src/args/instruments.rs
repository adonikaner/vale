//! The flags for the instruments: a picture of the editor, and a tour of
//! places with what the process holds at each.

use super::Arity::*;
use super::{number, points, text, Flag};

/// The picture and the tour.
#[derive(Debug, Clone, Default)]
pub struct Instruments {
    /// `--shot <file>`: write a picture of the editor and quit.
    pub shot: Option<String>,
    /// `--after <seconds>`: how long to wait before `--shot`. See
    /// [`crate::instruments::shot::Shot`].
    pub after: Option<f32>,
    /// `--tour <x,y;x,y;…>`: jump from place to place, printing what the
    /// process holds at each, and quit. See [`crate::instruments::tour`].
    pub tour: Option<Vec<(f32, f32)>>,
    /// `--dwell <seconds>`: how long `--tour` stays at each stop.
    pub dwell: Option<f32>,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--shot", One, |a, v| a.instruments.shot = text(v)),
    Flag::new("--after", One, |a, v| a.instruments.after = number(v)),
    Flag::new("--tour", One, |a, v| a.instruments.tour = points(v)),
    Flag::new("--dwell", One, |a, v| a.instruments.dwell = number(v)),
];
