//! The editor's command line: every flag, its default, and the parse.
//!
//! ```text
//! view.rs         the map, the place and the camera, the tool, the overlays
//!                 and the windows open at the start
//! tools.rs        scripted gestures of the pointer tools on the world
//! spawns.rs       scripted gestures of the creature and game object tools
//! subjects.rs     the server-row subjects' windows and workspaces
//! tables.rs       the client-table workspace: a row, its dialogs, an edit
//! import.rs       height maps and textures in and out
//! server.rs       playtests, Apply and Put back, publishing
//! instruments.rs  the picture and the tour
//! tests.rs        the parse, and every flag spelling
//! ```
//!
//! Each group is a struct of its own fields and a table of its flags: the
//! flag's spelling, how many arguments it takes, and a function that stores
//! them. [`Args::parse`] walks the tables. A setter may write a field of
//! another group: a flag that names a spawn also chooses the creature tool,
//! which is [`view::View::tool`].
//!
//! The list is kept short. The client's flags exist to make a scripted run
//! reproducible; the editor is used by a person at the window, and every one
//! of these flags is also reachable from a panel once the editor is open.

use bevy::prelude::*;
use std::iter::Peekable;

pub(crate) mod view;
pub(crate) mod tools;
pub(crate) mod spawns;
pub(crate) mod subjects;
pub(crate) mod tables;
pub(crate) mod import;
pub(crate) mod server;
pub(crate) mod instruments;

/// The command line, one field per group of flags.
#[derive(Resource, Debug, Clone, Default)]
pub(crate) struct Args {
    pub(crate) view: view::View,
    pub(crate) tools: tools::Tools,
    pub(crate) spawns: spawns::Spawns,
    pub(crate) subjects: subjects::Subjects,
    pub(crate) tables: tables::Tables,
    pub(crate) import: import::Import,
    pub(crate) server: server::Server,
    pub(crate) instruments: instruments::Instruments,
}

/// How many of the arguments after a flag are its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arity {
    /// None: the flag is a switch.
    Switch,
    /// The next argument, whatever it is. At the end of the line there is
    /// none, and the setter is given nothing.
    One,
    /// The next argument unless it starts with `--`, so a flag that follows
    /// is not taken as the value.
    Optional,
    /// The next two arguments, or as many as there are.
    Two,
    /// The next argument, then one more unless it starts with `--`.
    OneThenOptional,
    /// The next argument when it is a row subject's word, as
    /// [`crate::server::stack::Subject::named`] reads one.
    SubjectWord,
}

/// One flag: its spelling, how many arguments it takes, and what it stores.
///
/// The setter is given the arguments [`Arity`] took, which may be fewer than
/// the arity names when the line ends early.
pub(crate) struct Flag {
    pub(crate) name: &'static str,
    pub(crate) arity: Arity,
    pub(crate) set: fn(&mut Args, &[String]),
}

impl Flag {
    pub(crate) const fn new(name: &'static str, arity: Arity, set: fn(&mut Args, &[String])) -> Flag {
        Flag { name, arity, set }
    }
}

/// Every group's flags, in the order they are looked up. No spelling is in two
/// tables; `every_flag_is_in_one_table` checks it.
const GROUPS: [&[Flag]; 8] = [
    view::FLAGS,
    tools::FLAGS,
    spawns::FLAGS,
    subjects::FLAGS,
    tables::FLAGS,
    import::FLAGS,
    server::FLAGS,
    instruments::FLAGS,
];

impl Args {
    pub(crate) fn parse(args: impl Iterator<Item = String>) -> Args {
        let mut parsed = Args::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match Args::flag(&arg) {
                Some(flag) => {
                    let values = take(flag.arity, &mut args);
                    (flag.set)(&mut parsed, &values);
                }
                // A bare word is the map, so `vale-ide Kalimdor` works the
                // way `vale-client Alden` does.
                None if !arg.starts_with("--") => parsed.view.map = arg,
                None => eprintln!("unknown flag {arg}"),
            }
        }
        parsed
    }

    /// The flag spelled `name`, from whichever group has it.
    fn flag(name: &str) -> Option<&'static Flag> {
        GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .find(|flag| flag.name == name)
    }
}

/// Take a flag's arguments off the line, as many as `arity` says.
fn take<I: Iterator<Item = String>>(arity: Arity, args: &mut Peekable<I>) -> Vec<String> {
    let not_a_flag = |next: &String| !next.starts_with("--");
    match arity {
        Arity::Switch => Vec::new(),
        Arity::One => args.next().into_iter().collect(),
        Arity::Optional => args.next_if(not_a_flag).into_iter().collect(),
        Arity::Two => args.by_ref().take(2).collect(),
        Arity::OneThenOptional => match args.next() {
            Some(first) => std::iter::once(first).chain(args.next_if(not_a_flag)).collect(),
            None => Vec::new(),
        },
        Arity::SubjectWord => args
            .next_if(|word| crate::server::stack::Subject::named(word).is_some())
            .into_iter()
            .collect(),
    }
}

/// The first argument a flag was given, if any.
fn first(values: &[String]) -> Option<&str> {
    values.first().map(String::as_str)
}

/// The first argument, whatever it holds.
fn text(values: &[String]) -> Option<String> {
    values.first().cloned()
}

/// The first argument, unless it is empty or only spaces.
fn named(values: &[String]) -> Option<String> {
    values.first().filter(|text| !text.trim().is_empty()).cloned()
}

/// The first argument read as a number, with spaces round it ignored.
fn number<T: std::str::FromStr>(values: &[String]) -> Option<T> {
    first(values).and_then(|text| text.trim().parse().ok())
}

/// The first argument read as a list of `x,y` points separated by `;`, or
/// nothing when it names none. See [`crate::instruments::tour::Tour::parse`].
fn points(values: &[String]) -> Option<Vec<(f32, f32)>> {
    first(values)
        .map(crate::instruments::tour::Tour::parse)
        .filter(|points| !points.is_empty())
}

/// A subject by name, matched the way somebody would type it.
fn parse_tool(text: &str) -> Option<crate::tools::Tool> {
    // Matches against `tools::ALL` rather than a list of its own. A hand-written
    // list of eight entries missed the ninth tool when it was added, so `--tool`
    // could not open the new panel, and nothing reported it because an
    // unmatched name leaves the editor on its default.
    crate::tools::ALL
        .into_iter()
        .find(|tool| tool.name().eq_ignore_ascii_case(text.trim()))
}

/// `<seconds>` or `<seconds>,<back>` — see [`server::Server::playtest`].
fn parse_playtest(text: &str) -> Option<(f32, Option<f32>)> {
    match text.split_once(',') {
        Some((start, back)) => Some((start.trim().parse().ok()?, back.trim().parse().ok())),
        None => Some((text.trim().parse().ok()?, None)),
    }
}

/// `<distance>`, `<distance>,<pitch>` or `<distance>,<pitch>,<yaw>` — see
/// [`view::View::camera`]. The two angles are degrees, because that is what a person
/// types; the camera holds radians.
fn parse_view(text: &str) -> Option<(f32, Option<f32>, Option<f32>)> {
    let mut parts = text.split(',').map(str::trim);
    let distance = parts.next()?.parse().ok()?;
    let pitch = parts.next().and_then(|v| v.parse().ok());
    let yaw = parts.next().and_then(|v| v.parse().ok());
    Some((distance, pitch, yaw))
}

/// `x0,y0,x1,y1`, as four numbers — see [`tools::Tools::enclose`].
fn parse_rect(text: &str) -> Option<[f32; 4]> {
    let numbers: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    numbers.try_into().ok()
}

/// `a,b` or `a x b`, as two numbers.
fn parse_pair(text: &str) -> Option<(f32, f32)> {
    let (a, b) = text
        .split_once([',', 'x'])
        .or_else(|| text.split_once(' '))?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// Two whole numbers separated by a comma, as `--taxi-connect` takes them.
fn parse_two(text: &str) -> Option<(u32, u32)> {
    let (a, b) = text.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(test)]
mod tests;
