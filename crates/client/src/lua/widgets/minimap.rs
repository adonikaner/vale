//! **The `<Minimap>` widget** — the one frame kind in `Interface\FrameXML\`
//! whose contents are the *world* rather than art, and the last of the four
//! `vale_assets::interface::widgets::FRAME_KINDS` names that had nothing at all behind
//! it.
//!
//! ```text
//! Minimap:GetZoom()          which of six                 5 calls
//! Minimap:GetZoomLevels()    …how many there are          2
//! Minimap:SetZoom(n)         …and the two buttons' write  2
//! Minimap:PingLocation(x, y) a click on the map           1
//! Minimap:GetPingPosition()  …and where it has got to     1
//! ```
//!
//! That is the **whole** of what the directory asks of it — five methods, found
//! by grepping all 167 files of `Interface\FrameXML\` for `Minimap:` — because
//! everything else about the widget is the client's own. Where it looks, how far
//! it sees and what shape it is cut to are the client's and live in
//! [`vale_assets::tables::minimap`]; what it *draws* is `crate::ui::framexml`'s. This
//! file holds only the state a script can read and write.
//!
//! ## The zoom is on the widget, not in a CVar
//!
//! 1.12 keeps two of them — `minimapZoom` and `minimapInsideZoom`, registered
//! side by side — and swaps between them on whether the character
//! is indoors. This client keeps one number on the one widget the game ever
//! makes, and the indoor/outdoor split is applied where the *radius* is chosen
//! ([`vale_assets::tables::minimap::radius_yards`]) rather than by holding two
//! levels. The visible difference is that walking into a building does not
//! restore the zoom you last used inside it, which is a preference nothing in
//! the directory reads.
//!
//! **`MINIMAP_UPDATE_ZOOM` is deliberately not raised.** Its only handler
//! enables and disables the two zoom buttons at the ends of the range, and
//! `Minimap_ZoomInClick` and `Minimap_ZoomOutClick` already do exactly that
//! themselves on the way past — so the event is the *external* zoom change's
//! notification, and nothing in this client changes the zoom except those two
//! buttons.

use vale_assets::tables::minimap::{DEFAULT_ZOOM, ZOOM_LEVELS};

/// Where the zoom level lives on the widget's own table.
const ZOOM_KEY: &str = "__minimapZoom";

/// …and the ping, as an offset in the widget's own units from its centre.
const PING_KEY: &str = "__minimapPing";

/// **The widget methods this file registers.** Sorted, and counted by
/// `vale framexml` as answered rather than stubbed — `GetZoom` was on
/// [`super::super::api::stubs::METHODS`] answering a constant `0`, which is a real zoom
/// level and therefore indistinguishable from a working one.
pub const METHODS: [&str; 7] = [
    "GetPingPosition",
    "GetZoom",
    "GetZoomLevels",
    "PingLocation",
    "SetBlipTexture",
    "SetMaskTexture",
    "SetZoom",
];

/// What one minimap frame is showing, as plain data — everything the painter
/// needs from the widget and nothing that borrows Lua.
///
/// It carries no position: where the map is centred is the *world's* answer and
/// comes through [`crate::interface::minimap::MinimapView`], because the draw walk
/// runs with no borrow of the world at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinimapWidget {
    /// 0..[`ZOOM_LEVELS`) — an index into the client's own two radius tables.
    pub zoom: usize,
}

/// Install the five methods onto the shared frame method table.
///
/// Called from [`super::frames::register_methods`] **before**
/// [`super::super::api::stubs::install_methods`], which is what lets `GetZoom` here shadow
/// the stub rather than the other way round.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `SetMaskTexture(file)` and `SetBlipTexture(file)` — the round mask the
    // map is cut by and the sheet the tracking dots come from. Recorded and
    // read by nothing: the painter cuts the map with the game's own mask and
    // draws no blips. A square-minimap addon sets the first on load.
    for (name, key) in [("SetMaskTexture", "__maskTexture"), ("SetBlipTexture", "__blipTexture")] {
        let set = lua.create_function(move |_lua, (this, file): (mlua::Table, Option<String>)| {
            this.set(key, file)
        })?;
        methods.set(name, set)?;
    }
    let get_zoom = lua.create_function(|_, this: mlua::Table| Ok(zoom_of(&this)))?;
    methods.set("GetZoom", get_zoom)?;

    // **Clamped rather than refused**, which is the client's own handling:
    // it writes 5 over anything larger before the value ever reaches a
    // radius table. `Minimap_ZoomInClick` relies on it — it increments first and
    // asks whether it has reached the top afterwards.
    let set_zoom = lua.create_function(|_, (this, zoom): (mlua::Table, Option<i64>)| {
        let zoom = zoom.unwrap_or(0).clamp(0, ZOOM_LEVELS as i64 - 1);
        this.raw_set(ZOOM_KEY, zoom)
    })?;
    methods.set("SetZoom", set_zoom)?;

    let levels = lua.create_function(|_, _: mlua::MultiValue| Ok(ZOOM_LEVELS))?;
    methods.set("GetZoomLevels", levels)?;

    // **The ping is held in the widget's own units and not in the world**, which
    // is a stated deviation: 1.12 anchors it to the ground, so a ping stays over
    // the place that was clicked while the character walks away from it, and
    // here it stays where it was drawn. Five seconds of a cosmetic marker
    // nothing else reads (`MINIMAP_PING` is a party member's ping and this
    // client raises no such event), against a world borrow this method cannot
    // take — see the module comment on where the world enters.
    //
    // `Minimap_SetPing` multiplies what it gets by the frame's width, so the
    // *read* is a fraction where the *write* is units. That asymmetry is the
    // file's, not this client's.
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

/// The zoom a frame is on, defaulting to the `minimapZoom` CVar's own shipped
/// value — see [`DEFAULT_ZOOM`], which is why a session opens at 133 yards.
fn zoom_of(frame: &mlua::Table) -> usize {
    let held: Option<i64> = frame.raw_get(ZOOM_KEY).ok().flatten();
    held.map_or(DEFAULT_ZOOM, |zoom| {
        zoom.clamp(0, ZOOM_LEVELS as i64 - 1) as usize
    })
}

/// **What a frame is showing, or `None` for a frame that is not a minimap.**
///
/// The test is the widget's own kind — `CreateFrame("Minimap", …)`'s first
/// argument, which the XML loader sets from the element name — rather than the
/// frame's *name*, so `Minimap` being a global is a coincidence this does not
/// depend on. There is exactly one in the game, and an addon making a second
/// gets a second world view rather than a blank square.
pub(super) fn widget(frame: &mlua::Table) -> Option<MinimapWidget> {
    let kind: Option<String> = frame.raw_get(super::widget::KIND_KEY).ok().flatten();
    (kind.as_deref() == Some("Minimap")).then(|| MinimapWidget { zoom: zoom_of(frame) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// **`Minimap_ZoomInClick`'s own body, run for real** — the four calls it
    /// makes, including the test against `GetZoomLevels() - 1` that decides
    /// whether the button disables itself.
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

    /// A frame that is not a `<Minimap>` is not one, however it is named — see
    /// [`widget`].
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
