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
//! ## The zoom is stored on the widget, not in a CVar
//!
//! The 1.12.1 client has two zoom CVars, `minimapZoom` and
//! `minimapInsideZoom`, and uses one or the other depending on whether the
//! character is indoors. This client keeps one number on the single minimap
//! widget the game creates, and applies the indoor/outdoor difference where
//! the radius is chosen ([`vale_assets::tables::minimap::radius_yards`])
//! instead of holding two levels. The visible difference is that entering a
//! building does not restore the zoom last used indoors; nothing in the
//! directory reads that preference.
//!
//! `MINIMAP_UPDATE_ZOOM` is not raised. Its only handler enables and disables
//! the two zoom buttons at the ends of the range, and `Minimap_ZoomInClick`
//! and `Minimap_ZoomOutClick` already do that themselves. The event reports a
//! zoom change made from outside those buttons, and nothing in this client
//! changes the zoom except those two buttons.

use vale_assets::tables::minimap::{DEFAULT_ZOOM, ZOOM_LEVELS};

/// The key of the zoom level on the widget's table.
const ZOOM_KEY: &str = "__minimapZoom";

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
    /// 0..[`ZOOM_LEVELS`): an index into the outdoor and indoor radius tables
    /// in [`vale_assets::tables::minimap`].
    pub zoom: usize,
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
    let get_zoom = lua.create_function(|_, this: mlua::Table| Ok(zoom_of(&this)))?;
    methods.set("GetZoom", get_zoom)?;

    // Clamped rather than rejected, as the 1.12.1 client does: a value above
    // 5 becomes 5. `Minimap_ZoomInClick` relies on this; it increments first
    // and checks afterwards whether it has reached the top.
    let set_zoom = lua.create_function(|lua, (this, zoom): (mlua::Table, Option<i64>)| {
        let zoom = zoom.unwrap_or(0).clamp(0, ZOOM_LEVELS as i64 - 1);
        super::widget::set_paint(lua, &this, ZOOM_KEY, zoom)
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

/// The frame's zoom level, defaulting to the `minimapZoom` CVar's shipped
/// value; see [`DEFAULT_ZOOM`]. That default is why a session opens at 133
/// yards.
fn zoom_of(frame: &mlua::Table) -> usize {
    let held: Option<i64> = frame.raw_get(ZOOM_KEY).ok().flatten();
    held.map_or(DEFAULT_ZOOM, |zoom| {
        zoom.clamp(0, ZOOM_LEVELS as i64 - 1) as usize
    })
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
    (kind.as_deref() == Some("Minimap")).then(|| MinimapWidget { zoom: zoom_of(frame) })
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
