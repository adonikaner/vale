//! The command line of `vale-client`: the character to log in as, and the
//! flags that turn a run into a scripted check. [`Args::parse`] reads it;
//! `crate::run` applies it.

use bevy::prelude::*;

use crate::render;
use crate::scripted::Screenshots;
use crate::window::{WindowScript, RESIZE_FRACTION};
use crate::world::camera;

/// The command line: who to log in as, and whether this run is a scripted look
/// at the screen rather than someone sitting in front of it.
///
/// ```text
/// vale-client <Character>
/// vale-client <Character> --shot <path> [--after <seconds>] [--view <distance,pitch°,yaw°>]
///                            [--tune none|msaa,novsync,shadows]
///                            [--without doodads,buildings,entities,water,particles,interface,...]
///                            [--night off|0..1] [--hour <hh:mm>] [--weather <kind>[,<grade>]]
///                            [--script <lua>] [--overlay wireframe,collision,…]
///                            [--panel <tab>] [--relogin <seconds>[,<gap>]]
/// ```
///
/// These are flags rather than environment variables so that a check runs in
/// one line. The environment-variable form needed three `$env:` assignments,
/// did not end, and had to be killed. The `--shot` form logs in, frames the
/// view, writes the PNG and quits.
pub(crate) struct Args {
    /// Enters the world without waiting to be clicked, mirroring `vale live
    /// <Character>`. `None` leaves the app sitting at the empty world.
    pub(crate) character: Option<String>,
    pub(crate) shot: Screenshots,
    /// `--tune`: the render settings to start with, `None` for the defaults.
    pub(crate) tuning: Option<camera::RenderTuning>,
    /// `--without`: the world layers to leave out; see
    /// [`render::tuning::WorldTuning::without`]. The counterpart of `--tune`:
    /// it prices a pass where `--tune` prices a setting.
    pub(crate) world: Option<render::tuning::WorldTuning>,
    /// `--night <off|0..1>`: how deep the deep night is. `None` keeps the
    /// default, which is on.
    ///
    /// `--tune` trades CPU against GPU against image quality on this machine,
    /// and `--without` removes a layer so a pass can be priced. This flag adds
    /// something the game does not have, which is why it is separate from both.
    /// It exists so the option can be compared with a shot of the same framing
    /// under `--night off`, which is the comparison that shows whether it is too
    /// dark.
    ///
    /// See [`render::night`], which owns it and says why the default is on:
    /// the grade is exactly the identity in daylight, so an unflagged run at
    /// noon renders the same client every earlier measurement was taken against.
    pub(crate) night: Option<render::night::NightTuning>,
    /// `--hour <hh:mm>`: sets the world's clock by hand for the whole run.
    ///
    /// The scripted counterpart of the debug window's hour slider, which exists
    /// for the same reason: the world's clock runs at the server's rate, so
    /// waiting for dusk takes up to twenty-four minutes. A script cannot reach
    /// the slider any more than it can press `F4` or move the mouse. Before this
    /// flag, no `--shot` could photograph the night.
    ///
    /// `hh:mm`, or a bare hour. It sets `WorldClock::override_half_minutes`,
    /// the same latch the slider sets, so the server's clock is still read
    /// underneath and the HUD says the hour was set by hand.
    pub(crate) hour: Option<u32>,
    /// `--weather <kind>[,<grade>]`: a weather packet the server never sent,
    /// applied for the whole run, such as `snow` or `rain,0.4`. The counterpart
    /// of `--hour`: a scripted shot cannot wait for a zone to start snowing, and
    /// a GM command needs an account the run may not have. See
    /// [`render::weather::ForcedWeather`].
    pub(crate) weather: Option<render::weather::ForcedWeather>,
    /// `--script`: one Lua chunk, run once the interface has settled.
    ///
    /// An interface path that cannot be driven from a script cannot be checked
    /// without a person at the keyboard.
    /// `--script 'ActionButtonDown(1); ActionButtonUp(1)'` confirms that the
    /// casting chain reaches the server, and it is how the fix for a
    /// casting-chain regression was confirmed.
    pub(crate) script: Option<String>,
    /// `--overlay <names>`: the visualisations to turn on; see
    /// [`render::overlay::DebugOverlay::with`]. The counterpart of `--without`:
    /// it adds rather than subtracts (a wireframe, the bounding boxes, the solid
    /// triangles underfoot). Off unless named.
    ///
    /// Behind the feature that owns the overlays, like `--panel`, so a build
    /// without it ignores the flag rather than failing to compile.
    #[cfg(feature = "diagnostics")]
    pub(crate) overlay: Option<render::overlay::DebugOverlay>,
    /// `--panel <tab>`: open the debug window at that tab on the first frame.
    ///
    /// The debug window is the one thing on screen a script could not
    /// otherwise reach. A `--shot` checks the world by framing it and the
    /// interface through `--script`, but the window is behind `F4`, and a
    /// scripted run cannot press a key any more than it can move the mouse (see
    /// [`crate::HoverProbe`] for the pointer). Without this flag a rebuilt panel could
    /// only be checked by a person, which is the cost to design against.
    ///
    /// The value is a tab name as the tab strip spells it: `frame`, `scene`,
    /// `render`, `world`, `net`, `interface`. A name that matches nothing logs a
    /// warning and opens the default tab, rather than silently photographing
    /// the wrong one.
    pub(crate) panel: Option<String>,
    /// `--capture`: arm the recent-packet ring as soon as there is a session.
    ///
    /// The ring is armed by a checkbox, and a scripted run cannot tick one any
    /// more than it can press `F4`; `--panel` and `--overlay` exist for the same
    /// reason. Without this flag the one instrument that shows what a packet
    /// contained could only be used by a person at the keyboard, which is the
    /// cost to design against.
    ///
    /// Arming it costs a lock and a bounded copy per packet (see
    /// [`vale_protocol::socket::world::Capture`]), so it is off unless asked
    /// for, like every other instrument here.
    #[cfg(feature = "diagnostics")]
    pub(crate) capture: bool,
    /// `--audit`: load the interface, print what broke, and do not open a
    /// window. See [`crate::lua::audit`]. It is the run-time half of
    /// `vale framexml`, and the only check that can say which missing name
    /// breaks which panel.
    pub(crate) audit: bool,
    /// `--draw`, with `--audit`: after the load (and after `--script`, when one
    /// is given), dump every visible object with its solved rectangle and its
    /// paint. It answers "what is this quad on the screen" without a window.
    pub(crate) draw: bool,
    /// `--spin <frames>`, with `--audit`: run that many simulated frames of the
    /// interface's per-frame work and print the timing shape. It is the
    /// headless check for a frame rate that oscillates; see [`crate::lua::audit`].
    pub(crate) spin: usize,
    /// `--party <n>`, with `--audit`: how many people the double is grouped
    /// with, 0..4. The party size is the one dimension of the harness's world
    /// that a report blamed, so it is a subtraction: two runs that differ only
    /// in this flag price the party frames, as `--without` prices a render
    /// pass. Defaults to 2, which every earlier round measured against.
    pub(crate) party: Option<usize>,
    /// `--raid <n>`, with `--audit`: how many people are in the double's raid,
    /// the double included, 0..40. Defaults to 0, a party. This number decides
    /// which of its two screens the raid panel draws, and the interface hides
    /// every party frame while it is above zero, so a harness that was always
    /// in a raid would stop checking the party.
    pub(crate) raid: Option<usize>,
    /// `--events`, with `--audit`: fire every event this client can raise at
    /// the frames that registered for it, and report what the handlers broke
    /// on. The load exercises only `OnLoad`, one of the game's 36 script kinds;
    /// this exercises `OnEvent`, the one that runs during play.
    pub(crate) events: bool,
    /// `--type <line>`, with `--audit`: open the chat line, type that into it
    /// and press Enter, then say what would have gone on the wire. This checks
    /// the path a person drives without a person; see
    /// [`crate::lua::audit::type_a_line`].
    pub(crate) typed: Option<String>,
    /// `--panels`, with `--audit`: open every panel `UIPanelWindows` names and
    /// report what its `OnShow` broke on. The fourth script kind an instrument
    /// reaches, and the one in which 1.12 fills a panel; see
    /// [`crate::lua::audit::open_every_panel`].
    pub(crate) panels: bool,
    /// `--clicks`, with `--audit`: press every visible button on every one of
    /// those panels, re-collecting after each round so the tabs are followed,
    /// and report what the `OnClick` bodies broke on. The fifth script kind an
    /// instrument reaches, and the one a player uses most; see
    /// [`crate::lua::audit::click_everything`].
    pub(crate) clicks: bool,
    /// `--bindings`, with `--audit`: run the `<Binding>` body of every command
    /// the game's own default key table binds, on both edges where the
    /// declaration wants one, and report what they broke on. The sixth script
    /// kind an instrument reaches, and the only one a keystroke runs; see
    /// [`crate::lua::audit::press_every_binding`].
    pub(crate) bindings: bool,
    /// `--glue`, with `--audit`: load `Interface\GlueXML\` instead of
    /// `Interface\FrameXML\` — the login screen and character select — and put
    /// the login screen up. Composes with every flag above it: `--glue --clicks`
    /// presses every button on the screen a player sees first.
    pub(crate) glue: bool,
    /// `--size <w>x<h>` and `--resize <w>x<h>`: the window to open at, and the
    /// window to become part way through the run.
    ///
    /// The same reasoning as `--tune` and `--without`, for an input that could
    /// not be varied from a script at all. Every aspect-ratio report is about a
    /// window somebody resized. Before these flags, only a person at the
    /// keyboard could see such a fault: a scripted shot always ran at Bevy's
    /// default 1280x720, the one shape at which nothing goes wrong.
    pub(crate) window: WindowScript,
    /// `--hover <x>,<y>`: plant the pointer on the window and report what it
    /// landed on, in logical window pixels from the top left.
    ///
    /// The one input a scripted run could not otherwise supply. Everything
    /// else this client does can be driven from `--script`, but the mouse pick
    /// reads `Window::cursor_position` and Lua cannot move the mouse, so
    /// hovering had no check that did not involve a person. The substitute was
    /// a PowerShell loop moving the OS cursor across the window while the
    /// client logged what it saw. Two game-object hover bugs were found that
    /// way, and each would have been a one-line run with this flag. See
    /// [`crate::scripted::hover_probe`].
    pub(crate) hover: Option<Vec2>,
    /// `--relogin <seconds>`: leave the world for character select and come
    /// straight back in, once, after this long in the world.
    ///
    /// No other flag reaches the second login. `--script` runs one chunk
    /// against the interface, and the interface does not survive the logout:
    /// the Lua host is rebuilt for `Interface\GlueXML\`, taking any frame a
    /// chunk created with it, so a run that logs out ends at the character
    /// screen. Every other scripted check therefore covers only the first
    /// login. The map, the tiles, the global building and the caches are all
    /// re-derived on the second, and the teardown, the host rebuild and the
    /// cache purge were written from reasoning rather than tested by a run.
    ///
    /// The logout is the game's own: a `Binding::Logout` on the queue the key
    /// press writes to, so the server's delay and `SMSG_LOGOUT_COMPLETE` are
    /// both real. Give `--after` room for the time in the world, this delay,
    /// the server's logout delay, the wait below, and the second login.
    ///
    /// `--relogin <seconds>,<gap>` waits `gap` seconds at the character
    /// screen. [`render::residency`]'s sweep runs every five seconds and treats
    /// the logout as the deadline, so a second login taken immediately would
    /// reuse caches that a real player's client would have purged. The default
    /// gap allows at least one purge.
    pub(crate) relogin: Option<(f32, f32)>,
    /// `--crowd <players>[,<mobs>]`: fill the frame with entities the server
    /// never sent, to price the entity pass, which had no subtraction.
    ///
    /// `--without` does this for a render layer and `--party` for the
    /// interface; neither reaches the entity population. The only other way to
    /// get forty players into one frame is forty people, so the entity pass was
    /// the one cost in this renderer that could not be measured from a script.
    /// See [`world::crowd`].
    #[cfg(feature = "diagnostics")]
    pub(crate) crowd: Option<crate::world::crowd::Crowd>,
}

impl Args {
    pub(crate) fn parse(args: impl Iterator<Item = String>) -> Args {
        let mut parsed = Args {
            character: None,
            // 25 seconds is about what the login and the first nine tiles take,
            // so the default is a shot of a world that has finished arriving.
            shot: Screenshots { after: 25.0, ..default() },
            tuning: None,
            world: None,
            night: None,
            hour: None,
            weather: None,
            script: None,
            #[cfg(feature = "diagnostics")]
            overlay: None,
            panel: None,
            #[cfg(feature = "diagnostics")]
            capture: false,
            audit: false,
            draw: false,
            spin: 0,
            party: None,
            raid: None,
            events: false,
            typed: None,
            panels: false,
            clicks: false,
            bindings: false,
            glue: false,
            window: WindowScript::default(),
            hover: None,
            relogin: None,
            #[cfg(feature = "diagnostics")]
            crowd: None,
        };
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            // A flag's value is the next argument, and a missing one is left as
            // the default rather than being an error — this is a developer tool,
            // and a mistyped `--after` should still give a screenshot.
            let mut value = || args.next();
            match arg.as_str() {
                "--shot" => parsed.shot.path = value(),
                "--after" => parsed.shot.after = value().and_then(|s| s.parse().ok()).unwrap_or(25.0),
                "--view" => {
                    parsed.shot.view = value().and_then(|s| {
                        let n: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                        <[f32; 3]>::try_from(n.as_slice()).ok()
                    })
                }
                "--tune" => parsed.tuning = value().map(|s| tune(&s)),
                // Takes its value like every other flag here, so it always
                // takes one: a bare `--night` would swallow the character name.
                // `--night on` keeps the default, which is what `on` parses to.
                "--night" => {
                    parsed.night = Some(render::night::NightTuning::parse(value().as_deref()))
                }
                "--hour" => parsed.hour = value().as_deref().and_then(parse_hour),
                "--weather" => {
                    parsed.weather = value().as_deref().and_then(|text| {
                        let parsed = render::weather::ForcedWeather::parse(text);
                        if parsed.is_none() {
                            warn!("--weather {text}: expected rain|snow|sand|fine, with an optional ,0..1 grade");
                        }
                        parsed
                    })
                }
                "--without" => {
                    parsed.world = value().map(|s| render::tuning::WorldTuning::without(&s))
                }
                "--hover" => {
                    parsed.hover = value().and_then(|s| {
                        let n: Vec<f32> =
                            s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                        <[f32; 2]>::try_from(n.as_slice()).ok().map(Vec2::from)
                    })
                }
                "--script" => parsed.script = value(),
                "--relogin" => {
                    let spelt = value().unwrap_or_default();
                    let mut halves = spelt.split(',').map(str::trim);
                    let after = halves.next().and_then(|s| s.parse().ok()).unwrap_or(20.0);
                    let gap = halves.next().and_then(|s| s.parse().ok()).unwrap_or(12.0);
                    parsed.relogin = Some((after, gap));
                }
                #[cfg(feature = "diagnostics")]
                "--crowd" => parsed.crowd = value().map(|s| crate::world::crowd::Crowd::parse(&s)),
                #[cfg(feature = "diagnostics")]
                "--overlay" => {
                    parsed.overlay = value().map(|s| render::overlay::DebugOverlay::with(&s))
                }
                "--panel" => parsed.panel = value(),
                #[cfg(feature = "diagnostics")]
                "--capture" => parsed.capture = true,
                "--audit" => parsed.audit = true,
                "--draw" => parsed.draw = true,
                "--spin" => parsed.spin = value().and_then(|s| s.parse().ok()).unwrap_or(0),
                "--party" => parsed.party = value().and_then(|s| s.parse().ok()),
                "--raid" => parsed.raid = value().and_then(|s| s.parse().ok()),
                "--events" => parsed.events = true,
                "--panels" => parsed.panels = true,
                "--clicks" => parsed.clicks = true,
                "--bindings" => parsed.bindings = true,
                "--glue" => parsed.glue = true,
                "--type" => parsed.typed = value(),
                "--size" => parsed.window.open = value().as_deref().and_then(parse_size),
                "--resize" => parsed.window.then = value().as_deref().and_then(parse_size),
                _ if arg.starts_with("--") => warn!("unknown argument {arg}"),
                _ => parsed.character = Some(arg),
            }
        }
        // Derived from `--after` rather than given, so the run has one clock. A
        // resize with its own time would be a second value to keep in step with
        // the shutter, and the only relationship between them that matters is
        // that the resize comes first.
        parsed.window.at = parsed.shot.after * RESIZE_FRACTION;
        parsed
    }
}

/// `hh:mm`, or a bare hour, as the half-minutes past midnight `WorldClock` is
/// in. See [`Args::hour`].
///
/// A value that does not parse returns `None` and logs a warning, so the run
/// does not silently photograph whatever hour the server is at. `--without`
/// and `--night` follow the same rule for the same reason.
fn parse_hour(text: &str) -> Option<u32> {
    let read = || {
        let (hours, minutes) = match text.trim().split_once(':') {
            Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
            None => (text.trim().parse::<u32>().ok()?, 0),
        };
        (hours <= 23 && minutes <= 59).then_some((hours * 60 + minutes) * 2)
    };
    let parsed = read();
    if parsed.is_none() {
        warn!("--hour {text}: expected hh:mm, or a bare hour in 0..23");
    }
    parsed
}

/// `--tune`'s value: the settings that are on, by name, comma-separated.
/// `none` (or anything else that names nothing) leaves them all off.
///
/// F5, F9 and F10 toggle the same three settings at the keyboard, but a
/// scripted shot has no keyboard, and a setting that cannot be scripted cannot
/// have its A/B logged beside the numbers. This lets two `--shot` runs differ
/// in one setting at one framing, the same subtraction that attributed the
/// alpha-map seam grid and the anisotropy lattice.
fn tune(list: &str) -> camera::RenderTuning {
    let on = |name: &str| list.split(',').any(|p| p.trim().eq_ignore_ascii_case(name));
    camera::RenderTuning {
        msaa: on("msaa"),
        // Spelled as the off switch because vsync is the one setting whose
        // default is on, and leaving it out of a list like `msaa,shadows` must
        // not silently unlock the frame rate. An unlocked frame is a different
        // measurement and has to be asked for by name.
        vsync: !on("novsync"),
        sun_shadows: on("shadows"),
    }
}

/// `<w>x<h>`, or `None` for anything else — a mistyped size opens the default
/// window rather than a 0x0 one.
fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Args {
        Args::parse(line.split_whitespace().map(str::to_string))
    }

    /// The character is the argument that is not a flag, wherever it sits, and a
    /// scripted shot gets a default `--after` rather than firing on frame zero —
    /// which would photograph an empty world and look like a rendering fault.
    #[test]
    fn the_command_line_says_who_to_be_and_what_to_capture() {
        let plain = parse("Alden");
        assert_eq!(plain.character.as_deref(), Some("Alden"));
        assert!(plain.shot.path.is_none(), "no shot unless one is asked for");

        let scripted = parse("--shot look.png --view 3,10,0 Alden");
        assert_eq!(scripted.character.as_deref(), Some("Alden"));
        assert_eq!(scripted.shot.path.as_deref(), Some("look.png"));
        assert_eq!(scripted.shot.view, Some([3.0, 10.0, 0.0]));
        assert_eq!(scripted.shot.after, 25.0, "the world has to arrive first");

        assert_eq!(parse("x --after 8").shot.after, 8.0);
        // A flag whose value is missing must not eat the character or panic.
        assert_eq!(parse("Alden --view").shot.view, None);
    }

    /// `--night` and `--hour`, which exist for the same reason as `--without`:
    /// a view that cannot be set up from a script is not compared twice.
    /// `--night` always takes its value, since a bare one would swallow the
    /// character, and `--hour` is the only way a scripted shot can show the
    /// night.
    #[test]
    fn the_night_and_the_hour_can_be_set_from_the_line() {
        let plain = parse("Bram");
        assert!(plain.night.is_none() && plain.hour.is_none());

        let dark = parse("Bram --night off --hour 00:00");
        assert_eq!(dark.character.as_deref(), Some("Bram"));
        assert!(!dark.night.expect("--night was given").enabled);
        assert_eq!(dark.hour, Some(0));

        let half = parse("--night 0.5 --hour 21:30 Bram");
        assert_eq!(half.character.as_deref(), Some("Bram"));
        assert_eq!(half.night.expect("--night was given").strength, 0.5);
        assert_eq!(half.hour, Some((21 * 60 + 30) * 2));
        // A bare hour is midnight-relative like the panel's own slider.
        assert_eq!(parse_hour("18"), Some(18 * 120));
        // `--weather` takes its value the same way.
        let snowing = parse("Bram --weather snow,0.8 --hour 12:00");
        assert_eq!(snowing.character.as_deref(), Some("Bram"));
        let forced = snowing.weather.expect("--weather was given").0;
        assert_eq!(forced.kind, vale_protocol::play::weather::WeatherKind::Snow);
        assert_eq!(forced.grade, 0.8);
        assert!(parse("Bram --weather hail").weather.is_none());
        for bad in ["", "24:00", "12:60", "noon", "12:", ":30", "-1"] {
            assert_eq!(parse_hour(bad), None, "{bad:?}");
        }
    }

    /// `--size` and `--resize` are the only way to vary the input every
    /// aspect-ratio report is about; see [`WindowScript`]. A mistyped size opens
    /// the default window rather than a 0x0 one, like every other flag here.
    #[test]
    fn the_window_can_be_given_a_size_and_a_change_of_size() {
        let plain = parse("Alden");
        assert_eq!((plain.window.open, plain.window.then), (None, None));

        let sized = parse("--size 1920x1080 --resize 1280X720 --after 20 G");
        assert_eq!(sized.window.open, Some((1920, 1080)));
        // Capital `X` too: this is a developer flag and both are typed.
        assert_eq!(sized.window.then, Some((1280, 720)));
        // The resize happens within the run rather than after it, so a
        // `--shot` frames a window that has already changed shape.
        assert_eq!(sized.window.at, 20.0 * RESIZE_FRACTION);
        assert!(sized.window.at < sized.shot.after);

        for bad in ["", "1920", "1920x", "x720", "0x720", "1920x0", "axb", "-8x6"] {
            assert_eq!(parse_size(bad), None, "{bad:?}");
        }
    }

    /// `--tune` names the settings that are on; everything unnamed is off,
    /// and `none` is the all-off spelling. No flag at all keeps the defaults —
    /// which are not all-off, so `--tune` and its absence must stay distinct.
    #[test]
    fn tune_names_the_settings_that_are_on() {
        assert!(parse("Alden").tuning.is_none(), "no flag keeps the defaults");

        let none = parse("G --tune none").tuning.unwrap();
        assert!(!none.msaa && !none.sun_shadows);
        assert!(none.vsync, "vsync is off only by name, never by omission");

        let some = parse("G --tune msaa,shadows").tuning.unwrap();
        assert!(some.msaa && some.sun_shadows);
        assert!(some.vsync);

        assert!(!parse("G --tune novsync").tuning.unwrap().vsync);
    }
}
