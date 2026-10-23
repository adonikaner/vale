//! The flags that make the gestures of the pointer tools on the world, which a
//! scripted run has no pointer for: the terrain selection, the measuring
//! points, the chunk selection, the placement picker, the sweep, the group
//! rectangle, and the flight path and area trigger tools.

use super::Arity::*;
use super::{first, named, number, parse_pair, parse_rect, parse_two, points, Flag};
use crate::tools::Tool;

/// The pointer tools' scripted gestures.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tools {
    /// `--vertices <radius>[,<rise>]`: with `--tool terrain`, select the
    /// vertices within that many yards of the camera's target, as a press of
    /// Select vertices would, and with a second number move them up by that
    /// many yards. A selection is a drag, which a scripted run cannot make.
    /// See `tools::ground::terrain`.
    pub(crate) vertices: Option<(f32, Option<f32>)>,
    /// `--minimaps`: draw the minimaps of the tile under the camera and its
    /// eight neighbours, in the background, into the project. See
    /// `crate::map::tiles::start_minimaps`.
    pub(crate) minimaps: bool,
    /// `--tilt <angle>,<bearing>`: the terrain tool's tilt, in degrees, and
    /// with `--vertices` and no rise, the selection tilted by it.
    pub(crate) tilt: Option<(f32, f32)>,
    /// `--measure "<x,y>[;<x,y>]"`: open the measuring tool with one or two
    /// points kept, on the ground at those world positions. A click is the
    /// only other way to keep a point, and a scripted run cannot click. See
    /// [`crate::tools::measure`].
    pub(crate) measure: Option<Vec<(f32, f32)>>,
    /// `--chunks "<x,y>[;<x,y>]"`: open the chunk tool with the block between
    /// the chunks at two world positions selected, or the one chunk at one
    /// position. The selection is otherwise a press and a drag, which a
    /// scripted run cannot make. See [`crate::tools::ground::chunks`].
    pub(crate) chunks: Option<Vec<(f32, f32)>>,
    /// `--stitch`: stitch the `--chunks` selection to the ground around it
    /// once it is made, with the tool's default settings. See
    /// [`crate::tools::ground::chunks::stitch`].
    pub(crate) stitch: bool,
    /// `--place [<path>]`: open the placement panel on its Place half, with
    /// that model armed.
    ///
    /// Placing is a mode reached by a click on a segmented control and then a
    /// click on a row of a list, so without this flag a scripted run cannot
    /// reach the picker or the translucent preview, and neither can be
    /// checked. A duplicate row once armed an `.mdx` path that no archive
    /// holds, so the preview never appeared; a run that arms a path and takes
    /// a picture checks that a preview appears.
    ///
    /// With no path it opens the picker and arms nothing, which is how the
    /// folder browser is photographed.
    pub(crate) place: Option<Option<String>>,
    /// `--find <path>`: open on the sweep tool, look for that path over every
    /// tile of the map, and print what it found.
    ///
    /// It exists for the same reason as `--place`: a sweep is a choice of list,
    /// two text fields and a button, so without this a scripted run can only
    /// photograph an empty form. It finds and never replaces. Because it writes
    /// nothing, it is safe on a command line that may be run twice. The find
    /// is the part a run needs to check: whether a path is spelled as expected
    /// and how many tiles carry it.
    ///
    /// The list it searches is `--tool`'s: `--tool sweep` is the default and
    /// means textures, and the panel's own control is what chooses models or
    /// buildings for a person.
    pub(crate) find: Option<String>,
    /// `--enclose <x0,y0,x1,y1>`: a rectangle in window pixels, released over
    /// the world once it has streamed in, as though it had been dragged on
    /// empty ground with the chosen tool. See `tools::placements::group`.
    ///
    /// A scripted run cannot make this selection otherwise: the rectangle is a
    /// drag, and a script has no pointer. It is released seven seconds before
    /// the `--after` picture is taken, so the picture shows the group and the
    /// frame sample, which covers the last five seconds, is taken with it
    /// selected.
    pub(crate) enclose: Option<[f32; 4]>,
    /// `--taxi-node <id>`: the flight path tool, open on one node, with the
    /// camera over it.
    pub(crate) taxi_node: Option<u32>,
    /// `--trigger <id>`: the area trigger tool, open on one trigger of the
    /// open map, with the camera over it.
    pub(crate) trigger: Option<u32>,
    /// `--taxi-path <id>`: the flight path tool, open on one path, with the
    /// camera over its middle.
    pub(crate) taxi_path: Option<u32>,
    /// `--taxi-connect <from>,<to>`: a new path between two nodes on the open
    /// map, and the path back, seeded as a click on Connect makes them. The
    /// scripted form of a gesture that needs a pointer.
    pub(crate) taxi_connect: Option<(u32, u32)>,
    /// `--taxi-node-add <x>,<y>`: a new node on the ground at that place, once
    /// its tile is open.
    pub(crate) taxi_node_add: Option<(f32, f32)>,
    /// `--flight-map`: Make a flight map (or Fit the flight map) on the open
    /// map, once its nodes are read: the `WorldMapContinent.dbc` row and the
    /// picture.
    pub(crate) flight_map: bool,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--vertices", One, |a, v| {
        a.tools.vertices = first(v).and_then(|text| {
            let mut numbers = text.split(',').map(|part| part.trim().parse::<f32>());
            let radius = numbers.next()?.ok()?;
            Some((radius, numbers.next().and_then(Result::ok)))
        });
    }),
    Flag::new("--minimaps", Switch, |a, _| a.tools.minimaps = true),
    Flag::new("--tilt", One, |a, v| {
        a.tools.tilt = first(v).and_then(|text| {
            let (angle, toward) = text.split_once(',')?;
            Some((angle.trim().parse().ok()?, toward.trim().parse().ok()?))
        });
    }),
    Flag::new("--measure", One, |a, v| {
        a.tools.measure = points(v);
        a.view.tool = Some(Tool::Measure);
    }),
    Flag::new("--chunks", One, |a, v| {
        a.tools.chunks = points(v);
        a.view.tool = Some(Tool::Chunks);
    }),
    Flag::new("--stitch", Switch, |a, _| {
        a.tools.stitch = true;
        a.view.tool = Some(Tool::Chunks);
    }),
    // The path is optional, so a following flag is not eaten.
    Flag::new("--place", Optional, |a, v| a.tools.place = Some(v.first().cloned())),
    Flag::new("--find", One, |a, v| a.tools.find = named(v)),
    Flag::new("--enclose", One, |a, v| a.tools.enclose = first(v).and_then(parse_rect)),
    Flag::new("--trigger", One, |a, v| {
        a.tools.trigger = number(v);
        a.view.tool = Some(Tool::Triggers);
    }),
    Flag::new("--taxi-node", One, |a, v| {
        a.tools.taxi_node = number(v);
        a.view.tool = Some(Tool::Flightpaths);
    }),
    Flag::new("--taxi-path", One, |a, v| {
        a.tools.taxi_path = number(v);
        a.view.tool = Some(Tool::Flightpaths);
    }),
    Flag::new("--taxi-connect", One, |a, v| {
        a.tools.taxi_connect = first(v).and_then(parse_two);
        a.view.tool = Some(Tool::Flightpaths);
    }),
    Flag::new("--taxi-node-add", One, |a, v| {
        a.tools.taxi_node_add = first(v).and_then(parse_pair);
        a.view.tool = Some(Tool::Flightpaths);
    }),
    Flag::new("--flight-map", Switch, |a, _| {
        a.tools.flight_map = true;
        a.view.tool = Some(Tool::Flightpaths);
    }),
];
