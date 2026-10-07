//! The `<Minimap>` widget: the one frame kind in `Interface\FrameXML\` that
//! shows the world rather than art, and the last of the four
//! `vale_assets::interface::widgets::FRAME_KINDS` names that had no
//! implementation.
//!
//! ```text
//! Minimap:GetZoom()          current level of six         5 calls
//! Minimap:GetZoomLevels()    number of levels             2
//! Minimap:SetZoom(n)         written by the two buttons   2
//! Minimap:PingLocation(x, y) a click on the map           1
//! Minimap:GetPingPosition()  the ping's position          1
//! ```
//!
//! Those five methods are all the directory calls on it, found by searching
//! all 167 files of `Interface\FrameXML\` for `Minimap:`. The rest of the
//! widget's behaviour belongs to the client. Where it looks, how far it sees
//! and the shape it is cut to are in [`vale_assets::tables::minimap`]; what it
//! draws is in `crate::ui::framexml`. This file holds only the state a script
//! can read and write.
//!
//! ## Two zoom levels, indoor and outdoor, stored on the widget
//!
//! The 1.12.1 client keeps two zoom levels, the outdoor one (`minimapZoom`)
//! and the indoor one (`minimapInsideZoom`), and `GetZoom` and `SetZoom` act
//! on whichever matches where the character stands. This client keeps both on
//! the widget's table, each starting at its CVar's shipped default, and reads
//! which one is current from [`INSIDE_KEY`], which [`announce_inside`] writes.
//! The levels are not written back to the CVars, so they do not outlast the
//! session.
//!
//! `MINIMAP_UPDATE_ZOOM` is raised when the character goes in or out, as the
//! 1.12.1 client raises it, and when the world is first entered. Its handler
//! in `Minimap.lua` re-reads `GetZoom` and enables or disables the two zoom
//! buttons, which would otherwise keep the state of the level just left.

use bevy::prelude::*;
use vale_assets::tables::minimap::{DEFAULT_ZOOM, ZOOM_LEVELS};

/// The key of the outdoor zoom level on the widget's table.
const ZOOM_KEY: &str = "__minimapZoom";

/// The key of the indoor zoom level on the widget's table.
const INSIDE_ZOOM_KEY: &str = "__minimapInsideZoom";

/// The registry key that says whether the character is inside a building,
/// which chooses the zoom level `GetZoom` and `SetZoom` act on.
const INSIDE_KEY: &str = "vale.minimap.inside";

/// The key of the ping, stored as an offset from the widget's centre in the
/// widget's units.
const PING_KEY: &str = "__minimapPing";

/// The widget methods this file registers. Sorted, and counted by
/// `vale framexml` as implemented rather than stubbed. `GetZoom` was
/// previously in [`super::super::api::stubs::METHODS`] returning a constant
/// `0`, which is a valid zoom level and so looked like a working
/// implementation.
pub const METHODS: [&str; 7] = [
    "GetPingPosition",
    "GetZoom",
    "GetZoomLevels",
    "PingLocation",
    "SetBlipTexture",
    "SetMaskTexture",
    "SetZoom",
];

/// What one minimap frame is showing, as plain data: everything the painter
/// needs from the widget and nothing that borrows Lua.
///
/// It has no position. The map's centre comes from the world, through
/// [`crate::interface::minimap::MinimapView`], because the draw walk runs
/// without any borrow of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinimapWidget {
    /// 0..[`ZOOM_LEVELS`): the outdoor zoom level, an index into the outdoor
    /// radius table in [`vale_assets::tables::minimap`].
    pub zoom: usize,
    /// …and the indoor one, into the indoor table.
    pub inside_zoom: usize,
}

impl MinimapWidget {
    /// The level that applies: the indoor one inside a building.
    pub fn level(&self, inside: bool) -> usize {
        if inside {
            self.inside_zoom
        } else {
            self.zoom
        }
    }
}

/// Install the five methods onto the shared frame method table.
///
/// Called from [`super::frames::register_methods`] before
/// [`super::super::api::stubs::install_methods`], so that `GetZoom` here takes
/// precedence over the stub.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `SetMaskTexture(file)` and `SetBlipTexture(file)`: the round mask that
    // shapes the map and the texture the tracking dots come from. Stored and
    // not read: the painter shapes the map with the game's mask and draws no
    // blips. A square-minimap addon calls `SetMaskTexture` on load.
    for (name, key) in [("SetMaskTexture", "__maskTexture"), ("SetBlipTexture", "__blipTexture")] {
        let set = lua.create_function(move |_lua, (this, file): (mlua::Table, Option<String>)| {
            this.set(key, file)
        })?;
        methods.set(name, set)?;
    }
    let get_zoom = lua.create_function(|lua, this: mlua::Table| {
        Ok(zoom_of(&this, current_key(lua)))
    })?;
    methods.set("GetZoom", get_zoom)?;

    // Clamped rather than rejected, as the 1.12.1 client does: a value above
    // 5 becomes 5. `Minimap_ZoomInClick` relies on this; it increments first
    // and checks afterwards whether it has reached the top. It sets the level
    // of the place the character is in, indoor or outdoor.
    let set_zoom = lua.create_function(|lua, (this, zoom): (mlua::Table, Option<i64>)| {
        let zoom = zoom.unwrap_or(0).clamp(0, ZOOM_LEVELS as i64 - 1);
        super::widget::set_paint(lua, &this, current_key(lua), zoom)
    })?;
    methods.set("SetZoom", set_zoom)?;

    let levels = lua.create_function(|_, _: mlua::MultiValue| Ok(ZOOM_LEVELS))?;
    methods.set("GetZoomLevels", levels)?;

    // The ping is stored in the widget's units, not in world coordinates. This
    // differs from 1.12, which fixes the ping to the ground, so the ping stays
    // over the clicked place while the character walks away; here it stays
    // where it was drawn. The ping is a five-second cosmetic marker that
    // nothing else reads (`MINIMAP_PING` is a party member's ping, and this
    // client raises no such event), and storing it in world coordinates would
    // need a world borrow this method cannot take; see the module comment on
    // where the world comes in.
    //
    // `Minimap_SetPing` multiplies the value it receives by the frame's width,
    // so the read returns a fraction while the write takes units. That
    // asymmetry comes from the FrameXML file, not from this client.
    let ping = lua.create_function(|_, (this, x, y): (mlua::Table, Option<f32>, Option<f32>)| {
        this.raw_set(PING_KEY, vec![x.unwrap_or(0.0), y.unwrap_or(0.0)])
    })?;
    methods.set("PingLocation", ping)?;

    let ping_position = lua.create_function(|_, this: mlua::Table| {
        let held: Option<Vec<f32>> = this.raw_get(PING_KEY).ok().flatten();
        let held = held.unwrap_or_default();
        let (x, y) = (
            held.first().copied().unwrap_or(0.0),
            held.get(1).copied().unwrap_or(0.0),
        );
        let side = |key: &str| {
            this.raw_get::<Option<f64>>(key)
                .ok()
                .flatten()
                .unwrap_or(0.0) as f32
        };
        let width = side(super::widget::WIDTH_KEY).max(f32::EPSILON);
        let height = side(super::widget::HEIGHT_KEY).max(f32::EPSILON);
        Ok((x / width, y / height))
    })?;
    methods.set("GetPingPosition", ping_position)?;
    Ok(())
}

/// The widget key of the zoom level that applies now; see [`INSIDE_KEY`].
fn current_key(lua: &mlua::Lua) -> &'static str {
    let inside: Option<bool> = lua.named_registry_value(INSIDE_KEY).ok().flatten();
    if inside == Some(true) {
        INSIDE_ZOOM_KEY
    } else {
        ZOOM_KEY
    }
}

/// One of the frame's two zoom levels, defaulting to its CVar's shipped
/// value: `"3"` for both `minimapZoom` and `minimapInsideZoom`, which is
/// [`DEFAULT_ZOOM`]. That default is why a session opens at 133 yards.
fn zoom_of(frame: &mlua::Table, key: &str) -> usize {
    let held: Option<i64> = frame.raw_get(key).ok().flatten();
    held.map_or(DEFAULT_ZOOM, |zoom| {
        zoom.clamp(0, ZOOM_LEVELS as i64 - 1) as usize
    })
}

/// **Tell the interface which zoom level applies**, and raise
/// `MINIMAP_UPDATE_ZOOM` when that changes: on the first frame in the world
/// and on every move between inside and outside a building, as the 1.12.1
/// client does. Leaving the world forgets the last answer, so the next entry
/// raises it again.
pub fn announce_inside(
    host: Option<NonSendMut<super::super::host::LuaHost>>,
    world: super::super::api::LuaWorld,
    view: Res<crate::interface::minimap::MinimapView>,
    mut last: Local<Option<bool>>,
    mut pressed: MessageWriter<crate::input::bindings::BindingPressed>,
) {
    let Some(mut host) = host else { return };
    if !view.in_world {
        *last = None;
        return;
    }
    if *last == Some(view.indoors) {
        return;
    }
    *last = Some(view.indoors);
    let live = world.live();
    let inside = view.indoors;
    if let Err(e) = host.run(&live, |lua| lua.set_named_registry_value(INSIDE_KEY, inside)) {
        warn!("minimap: could not record the indoor flag: {e}");
    }
    for binding in host.fire_event("MINIMAP_UPDATE_ZOOM", &[], &live) {
        pressed.write(crate::input::bindings::BindingPressed(binding));
    }
}

/// What a frame is showing, or `None` for a frame that is not a minimap.
///
/// The test is the widget's kind (`CreateFrame("Minimap", …)`'s first
/// argument, which the XML loader sets from the element name), not the frame's
/// name, so this does not depend on the global `Minimap`. The game creates
/// exactly one; an addon that creates a second gets a second world view
/// rather than a blank square.
pub(super) fn widget(frame: &mlua::Table) -> Option<MinimapWidget> {
    let kind: Option<String> = frame.raw_get(super::widget::KIND_KEY).ok().flatten();
    (kind.as_deref() == Some("Minimap")).then(|| MinimapWidget {
        zoom: zoom_of(frame, ZOOM_KEY),
        inside_zoom: zoom_of(frame, INSIDE_ZOOM_KEY),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// Runs the calls `Minimap_ZoomInClick` makes, including the comparison
    /// with `GetZoomLevels() - 1` that decides whether the button disables
    /// itself.
    #[test]
    fn the_zoom_buttons_walk_the_clients_six_levels() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (levels, opened, after_in, at_top, after_out): (usize, usize, usize, bool, usize) =
            host.run(&world, |lua| {
                lua.load(
                    r#"
                    local map = CreateFrame("Minimap", "TestMinimap");
                    local levels = map:GetZoomLevels();
                    local opened = map:GetZoom();
                    map:SetZoom(map:GetZoom() + 1);
                    local afterIn = map:GetZoom();
                    map:SetZoom(99);
                    local atTop = (map:GetZoom() == levels - 1);
                    map:SetZoom(-4);
                    return levels, opened, afterIn, atTop, map:GetZoom();
                    "#,
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(levels, ZOOM_LEVELS, "the client answers six zoom levels");
        assert_eq!(opened, DEFAULT_ZOOM, "the minimapZoom CVar's shipped `3`");
        assert_eq!(after_in, DEFAULT_ZOOM + 1);
        assert!(at_top, "a zoom past the last level clamps rather than refusing");
        assert_eq!(after_out, 0, "…and so does one below the first");
    }

    /// The indoor and outdoor levels are separate: `SetZoom` inside a building
    /// leaves the outdoor level alone, and each starts at its CVar's `3`.
    #[test]
    fn indoors_and_outdoors_keep_their_own_zoom() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (outside, inside_start, back_out, painted): (usize, usize, usize, (usize, usize)) = host
            .run(&world, |lua| {
                let map: mlua::Table = lua
                    .load(r#"local m = CreateFrame("Minimap", "TwoZooms"); m:SetZoom(5); return m"#)
                    .eval()?;
                let get = |lua: &mlua::Lua| lua.load("return TwoZooms:GetZoom()").eval::<usize>();
                let outside = get(lua)?;
                lua.set_named_registry_value(INSIDE_KEY, true)?;
                let inside_start = get(lua)?;
                lua.load("TwoZooms:SetZoom(1)").exec()?;
                lua.set_named_registry_value(INSIDE_KEY, false)?;
                let back_out = get(lua)?;
                let painted = widget(&map).map(|w| (w.zoom, w.inside_zoom)).unwrap_or_default();
                Ok((outside, inside_start, back_out, painted))
            })
            .expect("the body runs");
        assert_eq!(outside, 5);
        assert_eq!(inside_start, DEFAULT_ZOOM, "minimapInsideZoom's shipped `3`");
        assert_eq!(back_out, 5, "the indoor SetZoom did not move the outdoor level");
        assert_eq!(painted, (5, 1));
        assert_eq!(MinimapWidget { zoom: 5, inside_zoom: 1 }.level(true), 1);
    }

    /// A frame that is not a `<Minimap>` is not treated as one, whatever its
    /// name; see [`widget`].
    #[test]
    fn only_a_minimap_frame_is_a_minimap() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (map, plain): (bool, bool) = host
            .run(&world, |lua| {
                let map: mlua::Table = lua
                    .load(r#"return CreateFrame("Minimap", "RealMinimap")"#)
                    .eval()?;
                let plain: mlua::Table = lua
                    .load(r#"return CreateFrame("Frame", "Minimap2")"#)
                    .eval()?;
                Ok((widget(&map).is_some(), widget(&plain).is_some()))
            })
            .expect("the frames are made");
        assert!(map);
        assert!(!plain, "a Frame called Minimap-something is still a Frame");
    }
}
