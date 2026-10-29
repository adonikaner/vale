//! Brush presets: named sets of brush settings for the terrain brush and the
//! texture brush.
//!
//! ## What a preset is
//!
//! A preset holds every setting of one brush except what it paints with. A
//! terrain preset holds the mode, radius, strength, core, falloff, shape,
//! noise scale and Flatten's target and tilt. A texture preset holds the
//! radius, core, shape, strength, falloff, opacity, Paint or Erase, the
//! four-texture-limit switches and the spray; the texture itself is chosen
//! separately, so one preset paints any texture.
//!
//! One preset of each brush is chosen at a time, and choosing one sets the
//! brush to it. A change to the brush afterwards, in the inspector, on the
//! toolbar or by a key, changes the brush only: the chosen preset is then
//! modified ([`Set::modified`]) until Save writes the brush's settings into it
//! or Discard sets the brush back to it. Choosing another preset drops the
//! unsaved changes. Save as New makes a preset of the brush's settings, and
//! Copy makes one of a preset's own.
//!
//! ## Where they are written
//!
//! `Edit\brushes.txt`, beside the project folders and for the reason
//! [`crate::prefs::favourites`] gives: presets are part of the tool rather
//! than of a world, and are not published. A missing file, or one with no
//! preset of a brush, gives that brush the starter set ([`starters`]).
//!
//! ```text
//! [terrain] Hill
//! mode raise
//! radius 20
//! ...
//! [textures] Soft Edge
//! radius 12
//! ...
//! ```
//!
//! A section starts with the brush in brackets and the preset's name. Each
//! line after it is a setting and its value. A setting a section leaves out
//! keeps the brush's default, and a line that does not parse is skipped.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use vale_edit::ops::brush::{Falloff, Mode, Only, Shape};
use vale_edit::ops::paint::PaintBrush;

use crate::tools::ground::terrain::Terrain;

/// The file's name under `Edit\`.
pub(crate) const FILE: &str = "brushes.txt";

/// The terrain brush's settings, as a preset holds them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sculpt {
    /// The mode. A Flatten is held with a target of 0; the target is taken
    /// where each stroke starts.
    pub(crate) mode: Mode,
    pub(crate) radius: f32,
    pub(crate) strength: f32,
    pub(crate) core: f32,
    pub(crate) falloff: Falloff,
    pub(crate) shape: Shape,
    pub(crate) scale: f32,
    pub(crate) only: Only,
    pub(crate) flatten_to_cursor: bool,
    pub(crate) flatten_height: f32,
    pub(crate) tilt_angle: f32,
    pub(crate) tilt_toward: f32,
}

impl Sculpt {
    /// The terrain tool's current settings.
    pub(crate) fn of(terrain: &Terrain) -> Sculpt {
        let brush = &terrain.brush;
        Sculpt {
            mode: match brush.mode {
                Mode::Flatten { .. } => Mode::Flatten { to: 0.0 },
                mode => mode,
            },
            radius: brush.radius,
            strength: brush.strength,
            core: brush.core,
            falloff: brush.falloff,
            shape: brush.shape,
            scale: brush.scale,
            only: brush.only,
            flatten_to_cursor: terrain.flatten_to_cursor,
            flatten_height: terrain.flatten_height,
            tilt_angle: terrain.tilt_angle,
            tilt_toward: terrain.tilt_toward,
        }
    }

    /// Set the terrain tool to these settings.
    pub(crate) fn apply(&self, terrain: &mut Terrain) {
        let brush = &mut terrain.brush;
        brush.mode = self.mode;
        brush.radius = self.radius;
        brush.strength = self.strength;
        brush.core = self.core;
        brush.falloff = self.falloff;
        brush.shape = self.shape;
        brush.scale = self.scale;
        brush.only = self.only;
        terrain.flatten_to_cursor = self.flatten_to_cursor;
        terrain.flatten_height = self.flatten_height;
        terrain.tilt_angle = self.tilt_angle;
        terrain.tilt_toward = self.tilt_toward;
    }
}

/// The texture brush's settings, as a preset holds them: everything but the
/// texture and its foliage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Paint {
    pub(crate) radius: f32,
    pub(crate) core: f32,
    pub(crate) shape: Shape,
    pub(crate) strength: f32,
    pub(crate) falloff: Falloff,
    pub(crate) opacity: f32,
    pub(crate) erase: bool,
    pub(crate) existing_only: bool,
    pub(crate) reuse_hidden: bool,
    pub(crate) density: f32,
    pub(crate) grain: f32,
}

impl Paint {
    /// The texture brush's current settings.
    pub(crate) fn of(brush: &PaintBrush) -> Paint {
        Paint {
            radius: brush.radius,
            core: brush.core,
            shape: brush.shape,
            strength: brush.strength,
            falloff: brush.falloff,
            opacity: brush.opacity,
            erase: brush.erase,
            existing_only: brush.existing_only,
            reuse_hidden: brush.reuse_hidden,
            density: brush.density,
            grain: brush.grain,
        }
    }

    /// Set the texture brush to these settings, keeping its texture.
    pub(crate) fn apply(&self, brush: &mut PaintBrush) {
        brush.radius = self.radius;
        brush.core = self.core;
        brush.shape = self.shape;
        brush.strength = self.strength;
        brush.falloff = self.falloff;
        brush.opacity = self.opacity;
        brush.erase = self.erase;
        brush.existing_only = self.existing_only;
        brush.reuse_hidden = self.reuse_hidden;
        brush.density = self.density;
        brush.grain = self.grain;
    }
}

/// One named preset.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preset<T> {
    pub(crate) name: String,
    pub(crate) settings: T,
}

/// One brush's presets and which of them is chosen.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Set<T> {
    pub(crate) list: Vec<Preset<T>>,
    pub(crate) chosen: Option<usize>,
}

impl<T> Default for Set<T> {
    fn default() -> Set<T> {
        Set { list: Vec::new(), chosen: None }
    }
}

/// What a person asked of a preset list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Ask {
    Choose(usize),
    /// A copy placed after the original and chosen.
    Copy(usize),
    Rename(usize, String),
    Delete(usize),
    /// A new preset from the brush's current settings, chosen.
    New,
    /// Write the brush's current settings into the chosen preset.
    Save,
    /// Set the brush back to the chosen preset.
    Discard,
}

impl<T: Clone + PartialEq> Set<T> {
    /// While no preset is chosen, choose the first preset equal to the
    /// brush's settings, so the brush's starting settings show as their
    /// preset.
    fn follow(&mut self, current: &T) {
        if self.chosen.is_none() {
            self.chosen = self.list.iter().position(|preset| preset.settings == *current);
        }
    }

    /// Whether the brush's settings differ from the chosen preset's: the
    /// changes Save would write and Discard would drop. False while none is
    /// chosen.
    pub(crate) fn modified(&self, current: &T) -> bool {
        self.chosen
            .and_then(|at| self.list.get(at))
            .is_some_and(|preset| preset.settings != *current)
    }

    /// Carry out `ask`. Answers the settings to set the brush to when a
    /// preset was chosen, and whether the list changed.
    fn act(&mut self, ask: Ask, current: &T) -> (Option<T>, bool) {
        match ask {
            Ask::Choose(at) => match self.list.get(at) {
                Some(preset) => {
                    self.chosen = Some(at);
                    (Some(preset.settings.clone()), false)
                }
                None => (None, false),
            },
            Ask::Copy(at) => {
                let Some(preset) = self.list.get(at).cloned() else { return (None, false) };
                let name = self.free_name(&format!("{} Copy", preset.name));
                self.list.insert(at + 1, Preset { name, settings: preset.settings.clone() });
                self.chosen = Some(at + 1);
                (Some(preset.settings), true)
            }
            Ask::Rename(at, name) => {
                let name = name.trim().to_string();
                match (self.list.get_mut(at), name.is_empty()) {
                    (Some(preset), false) => {
                        preset.name = name;
                        (None, true)
                    }
                    _ => (None, false),
                }
            }
            Ask::Delete(at) => {
                if at >= self.list.len() {
                    return (None, false);
                }
                self.list.remove(at);
                self.chosen = match self.chosen {
                    Some(chosen) if chosen == at => None,
                    Some(chosen) if chosen > at => Some(chosen - 1),
                    other => other,
                };
                (None, true)
            }
            Ask::New => {
                let name = self.free_name("New Brush");
                self.list.push(Preset { name, settings: current.clone() });
                self.chosen = Some(self.list.len() - 1);
                (None, true)
            }
            Ask::Save => match self.chosen.and_then(|at| self.list.get_mut(at)) {
                Some(preset) if preset.settings != *current => {
                    preset.settings = current.clone();
                    (None, true)
                }
                _ => (None, false),
            },
            Ask::Discard => {
                let settings = self.chosen.and_then(|at| self.list.get(at)).map(|preset| preset.settings.clone());
                (settings, false)
            }
        }
    }

    /// `wanted`, or `wanted` with the lowest number after it that no preset
    /// is named.
    fn free_name(&self, wanted: &str) -> String {
        let taken = |name: &str| self.list.iter().any(|preset| preset.name == name);
        match taken(wanted) {
            false => wanted.to_string(),
            true => (2..).map(|n| format!("{wanted} {n}")).find(|name| !taken(name)).unwrap_or_default(),
        }
    }
}

/// Both brushes' presets, and where they are written.
#[derive(Resource, Default)]
pub(crate) struct Brushes {
    loaded_from: Option<PathBuf>,
    pub(crate) sculpt: Set<Sculpt>,
    pub(crate) paint: Set<Paint>,
}

impl Brushes {
    /// Read the file under `dir` once. Idempotent per directory.
    pub(crate) fn load_from(&mut self, dir: &Path) {
        if self.loaded_from.as_deref() == Some(dir) {
            return;
        }
        self.loaded_from = Some(dir.to_path_buf());
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        let (sculpt, paint) = parse(&text);
        let (starter_sculpt, starter_paint) = starters();
        self.sculpt = Set { list: if sculpt.is_empty() { starter_sculpt } else { sculpt }, chosen: None };
        self.paint = Set { list: if paint.is_empty() { starter_paint } else { paint }, chosen: None };
    }

    /// While no terrain preset is chosen, choose the one equal to the terrain
    /// brush, if any. See [`Set::follow`].
    pub(crate) fn follow_sculpt(&mut self, terrain: &Terrain) {
        self.sculpt.follow(&Sculpt::of(terrain));
    }

    /// The same for the texture brush.
    pub(crate) fn follow_paint(&mut self, brush: &PaintBrush) {
        self.paint.follow(&Paint::of(brush));
    }

    /// Carry out an ask of the terrain list: set the brush when a preset was
    /// chosen or discarded to, and write the file when the list changed.
    pub(crate) fn act_sculpt(&mut self, ask: Ask, terrain: &mut Terrain) {
        let (set, changed) = self.sculpt.act(ask, &Sculpt::of(terrain));
        if let Some(settings) = set {
            settings.apply(terrain);
        }
        if changed {
            self.save();
        }
    }

    /// The same for the texture list.
    pub(crate) fn act_paint(&mut self, ask: Ask, brush: &mut PaintBrush) {
        let (set, changed) = self.paint.act(ask, &Paint::of(brush));
        if let Some(settings) = set {
            settings.apply(brush);
        }
        if changed {
            self.save();
        }
    }

    /// Write both lists to the file.
    fn save(&self) {
        let Some(dir) = &self.loaded_from else { return };
        if let Err(e) = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(dir.join(FILE), render(&self.sculpt.list, &self.paint.list)))
        {
            warn!("{} could not be written: {e}", dir.join(FILE).display());
        }
    }
}

/// The presets a brush starts with when the file has none for it. The first
/// of each is the brush's own default settings, so a new install starts with
/// it chosen.
pub(crate) fn starters() -> (Vec<Preset<Sculpt>>, Vec<Preset<Paint>>) {
    let sculpt = Sculpt::of(&Terrain::default());
    let paint = Paint::of(&PaintBrush::default());
    fn preset<T>(name: &str, settings: T) -> Preset<T> {
        Preset { name: name.to_string(), settings }
    }
    (
        vec![
            preset("Hill", sculpt.clone()),
            preset(
                "Hill Strong",
                Sculpt { radius: 30.0, strength: 24.0, core: 0.2, falloff: Falloff::Dome, ..sculpt.clone() },
            ),
            preset("Dig", Sculpt { mode: Mode::Lower, ..sculpt.clone() }),
            preset(
                "Plateau",
                Sculpt {
                    mode: Mode::Flatten { to: 0.0 },
                    radius: 25.0,
                    strength: 3.0,
                    core: 0.5,
                    falloff: Falloff::Dome,
                    flatten_to_cursor: true,
                    ..sculpt.clone()
                },
            ),
            preset("Smooth", Sculpt { mode: Mode::Smooth, strength: 3.0, ..sculpt.clone() }),
            preset("Rough", Sculpt { mode: Mode::Noise, strength: 4.0, ..sculpt.clone() }),
        ],
        vec![
            preset("Medium Edge", paint.clone()),
            preset("Soft Edge", Paint { core: 0.0, falloff: Falloff::Smooth, strength: 2.0, ..paint.clone() }),
            preset("Hard Edge", Paint { core: 0.8, falloff: Falloff::Linear, strength: 6.0, ..paint.clone() }),
            preset("Speckle", Paint { density: 0.5, ..paint.clone() }),
            preset(
                "Square",
                Paint { shape: Shape::Square, core: 0.9, falloff: Falloff::Flat, radius: 8.0, ..paint.clone() },
            ),
            preset("Eraser", Paint { erase: true, ..paint.clone() }),
        ],
    )
}

/// The words the file writes a mode, falloff, shape and flatten direction as.
const MODES: [(&str, Mode); 5] = [
    ("raise", Mode::Raise),
    ("lower", Mode::Lower),
    ("flatten", Mode::Flatten { to: 0.0 }),
    ("smooth", Mode::Smooth),
    ("noise", Mode::Noise),
];
const FALLOFFS: [(&str, Falloff); 5] = [
    ("smooth", Falloff::Smooth),
    ("linear", Falloff::Linear),
    ("flat", Falloff::Flat),
    ("sharp", Falloff::Sharp),
    ("dome", Falloff::Dome),
];
const SHAPES: [(&str, Shape); 3] = [("circle", Shape::Circle), ("square", Shape::Square), ("diamond", Shape::Diamond)];
const ONLY: [(&str, Only); 3] = [("both", Only::Both), ("fill", Only::Fill), ("cut", Only::Cut)];

/// The word for a value in one of the lists above.
fn word<T: PartialEq>(list: &[(&'static str, T)], value: &T) -> &'static str {
    list.iter().find(|(_, had)| had == value).map_or("", |(name, _)| name)
}

/// The value a word names in one of the lists above.
fn named<T: Copy>(list: &[(&str, T)], text: &str) -> Option<T> {
    list.iter().find(|(name, _)| *name == text).map(|(_, value)| *value)
}

/// `yes` or `no`.
fn yes(on: bool) -> &'static str {
    match on {
        true => "yes",
        false => "no",
    }
}

/// The file's text for both lists.
fn render(sculpt: &[Preset<Sculpt>], paint: &[Preset<Paint>]) -> String {
    let mut out = String::new();
    for preset in sculpt {
        let s = &preset.settings;
        out += &format!("[terrain] {}\n", preset.name);
        out += &format!("mode {}\n", word(&MODES, &s.mode));
        out += &format!("radius {}\nstrength {}\ncore {}\n", s.radius, s.strength, s.core);
        out += &format!("falloff {}\nshape {}\n", word(&FALLOFFS, &s.falloff), word(&SHAPES, &s.shape));
        out += &format!("scale {}\nonly {}\n", s.scale, word(&ONLY, &s.only));
        out += &format!("flatten-to-pointer {}\nflatten-height {}\n", yes(s.flatten_to_cursor), s.flatten_height);
        out += &format!("tilt {}\nuphill {}\n\n", s.tilt_angle, s.tilt_toward);
    }
    for preset in paint {
        let p = &preset.settings;
        out += &format!("[textures] {}\n", preset.name);
        out += &format!("radius {}\ncore {}\nstrength {}\n", p.radius, p.core, p.strength);
        out += &format!("falloff {}\nshape {}\n", word(&FALLOFFS, &p.falloff), word(&SHAPES, &p.shape));
        out += &format!("opacity {}\nerase {}\n", p.opacity, yes(p.erase));
        out += &format!("existing-only {}\nreuse-hidden {}\n", yes(p.existing_only), yes(p.reuse_hidden));
        out += &format!("density {}\ngrain {}\n\n", p.density, p.grain);
    }
    out
}

/// Both lists out of the file's text. See the module comment for the format.
fn parse(text: &str) -> (Vec<Preset<Sculpt>>, Vec<Preset<Paint>>) {
    let mut sculpt: Vec<Preset<Sculpt>> = Vec::new();
    let mut paint: Vec<Preset<Paint>> = Vec::new();
    // Which list the lines after a section heading go to.
    #[derive(PartialEq)]
    enum In {
        Nothing,
        Sculpt,
        Paint,
    }
    let mut section = In::Nothing;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(rest) = line.strip_prefix('[') {
            let Some((brush, name)) = rest.split_once(']') else {
                section = In::Nothing;
                continue;
            };
            let name = name.trim().to_string();
            section = match (brush.trim(), name.is_empty()) {
                (_, true) => In::Nothing,
                ("terrain", false) => {
                    sculpt.push(Preset { name, settings: Sculpt::of(&Terrain::default()) });
                    In::Sculpt
                }
                ("textures", false) => {
                    paint.push(Preset { name, settings: Paint::of(&PaintBrush::default()) });
                    In::Paint
                }
                _ => In::Nothing,
            };
            continue;
        }
        let Some((key, value)) = line.split_once(char::is_whitespace) else { continue };
        let value = value.trim();
        let number = value.parse::<f32>().ok().filter(|n| n.is_finite());
        let flag = match value {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        };
        match section {
            In::Nothing => {}
            In::Sculpt => {
                let Some(s) = sculpt.last_mut().map(|preset| &mut preset.settings) else { continue };
                match key {
                    "mode" => s.mode = named(&MODES, value).unwrap_or(s.mode),
                    "radius" => s.radius = number.unwrap_or(s.radius),
                    "strength" => s.strength = number.unwrap_or(s.strength),
                    "core" => s.core = number.unwrap_or(s.core),
                    "falloff" => s.falloff = named(&FALLOFFS, value).unwrap_or(s.falloff),
                    "shape" => s.shape = named(&SHAPES, value).unwrap_or(s.shape),
                    "scale" => s.scale = number.unwrap_or(s.scale),
                    "only" => s.only = named(&ONLY, value).unwrap_or(s.only),
                    "flatten-to-pointer" => s.flatten_to_cursor = flag.unwrap_or(s.flatten_to_cursor),
                    "flatten-height" => s.flatten_height = number.unwrap_or(s.flatten_height),
                    "tilt" => s.tilt_angle = number.unwrap_or(s.tilt_angle),
                    "uphill" => s.tilt_toward = number.unwrap_or(s.tilt_toward),
                    _ => {}
                }
            }
            In::Paint => {
                let Some(p) = paint.last_mut().map(|preset| &mut preset.settings) else { continue };
                match key {
                    "radius" => p.radius = number.unwrap_or(p.radius),
                    "core" => p.core = number.unwrap_or(p.core),
                    "strength" => p.strength = number.unwrap_or(p.strength),
                    "falloff" => p.falloff = named(&FALLOFFS, value).unwrap_or(p.falloff),
                    "shape" => p.shape = named(&SHAPES, value).unwrap_or(p.shape),
                    "opacity" => p.opacity = number.unwrap_or(p.opacity),
                    "erase" => p.erase = flag.unwrap_or(p.erase),
                    "existing-only" => p.existing_only = flag.unwrap_or(p.existing_only),
                    "reuse-hidden" => p.reuse_hidden = flag.unwrap_or(p.reuse_hidden),
                    "density" => p.density = number.unwrap_or(p.density),
                    "grain" => p.grain = number.unwrap_or(p.grain),
                    _ => {}
                }
            }
        }
    }
    (sculpt, paint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips() {
        let (sculpt, paint) = starters();
        let (read_sculpt, read_paint) = parse(&render(&sculpt, &paint));
        assert_eq!(read_sculpt, sculpt);
        assert_eq!(read_paint, paint);
    }

    /// A setting a section leaves out keeps the default, and a bad line or an
    /// unknown section is skipped.
    #[test]
    fn a_short_or_damaged_section_keeps_the_defaults() {
        let text = "[terrain] Tall\nradius 44\nstrength many\nnonsense\n[elsewhere] Skip\nradius 3\n[textures] Pale\nopacity 0.4\n";
        let (sculpt, paint) = parse(text);
        assert_eq!(sculpt.len(), 1);
        assert_eq!(sculpt[0].name, "Tall");
        assert_eq!(sculpt[0].settings.radius, 44.0);
        assert_eq!(sculpt[0].settings.strength, Sculpt::of(&Terrain::default()).strength);
        assert_eq!(paint.len(), 1);
        assert_eq!(paint[0].settings.opacity, 0.4);
        assert_eq!(paint[0].settings.radius, PaintBrush::default().radius);
    }

    /// The first starter of each brush is the brush's default, so a fresh
    /// brush shows it as chosen.
    #[test]
    fn a_fresh_brush_chooses_its_first_starter() {
        let (sculpt, _) = starters();
        let mut set = Set { list: sculpt, chosen: None };
        set.follow(&Sculpt::of(&Terrain::default()));
        assert_eq!(set.chosen, Some(0));
    }

    /// A change to the brush leaves the chosen preset as it was and marks it
    /// modified; Discard answers the preset's settings, and Save writes the
    /// change into it.
    #[test]
    fn a_change_is_saved_or_discarded() {
        let (sculpt, _) = starters();
        let mut set = Set { list: sculpt, chosen: Some(1) };
        let stored = set.list[1].settings.clone();
        let mut changed = stored.clone();
        changed.radius = 55.0;
        set.follow(&changed);
        assert_eq!(set.list[1].settings, stored);
        assert!(set.modified(&changed));

        let (applied, wrote) = set.act(Ask::Discard, &changed);
        assert_eq!(applied, Some(stored.clone()));
        assert!(!wrote);

        let (applied, wrote) = set.act(Ask::Save, &changed);
        assert_eq!(applied, None);
        assert!(wrote);
        assert_eq!(set.list[1].settings.radius, 55.0);
        assert!(!set.modified(&changed));
    }

    /// A copy is placed after the original under a free name and chosen; a
    /// delete before the chosen one keeps the same preset chosen.
    #[test]
    fn presets_copy_and_delete() {
        let (sculpt, _) = starters();
        let mut set = Set { list: sculpt, chosen: Some(1) };
        let changed = set.list[1].settings.clone();

        let (applied, wrote) = set.act(Ask::Copy(1), &changed);
        assert!(wrote);
        assert_eq!(applied, Some(changed.clone()));
        assert_eq!(set.chosen, Some(2));
        assert_eq!(set.list[2].name, "Hill Strong Copy");
        set.act(Ask::Copy(1), &changed);
        assert_eq!(set.list[2].name, "Hill Strong Copy 2");

        set.chosen = Some(3);
        set.act(Ask::Delete(0), &changed);
        assert_eq!(set.chosen, Some(2));
        set.act(Ask::Delete(2), &changed);
        assert_eq!(set.chosen, None);
    }
}
