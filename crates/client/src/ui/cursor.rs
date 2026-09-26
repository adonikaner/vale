//! The game's own mouse pointer.
//!
//! `Interface\Cursor\Point.blp` and its forty-two siblings are 32x32 one-bit-
//! alpha bitmaps in the archives, and winit will take one as a window cursor
//! directly — so this is a decode, an `Image` asset, and a `CursorIcon`
//! component on the window. There is no rendering in it at all, which is why it
//! is thirty lines rather than a pass.
//!
//! **The hotspot is measured rather than assumed.** A cursor bitmap does not
//! say which texel the click lands on; `vale cursor Point` draws the
//! coverage and the arrow's point sits in texel (0, 0), with the alpha coming
//! out 569 clear against 455 fully opaque and nothing in between — a hard-edged
//! stencil drawn into the corner. See [`HOTSPOT`].
//!
//! **And it disappears while you touch the world.** Either button held over the
//! world moves the camera — the right one turns the character with it — and a
//! pointer that stayed put would drift to the edge of the screen and start
//! clicking on whatever is there. The real client hides it and puts it back
//! where it was; `CursorGrabMode::Locked` is what does the second half, since a
//! released cursor otherwise reappears wherever the drag left it. Which gesture
//! counts is [`crate::world::camera::MouseLook`]'s to decide, so a drag that
//! began on a bag icon keeps its pointer.
//!
//! **egui owns this component too, and has to be told not to.** `bevy_egui`'s
//! `process_output_system` writes `CursorIcon::System(..)` onto the *window*
//! every time egui's requested icon changes — an I-beam on entering a text
//! field, `Default` on leaving it — so hovering the login box replaced the
//! game's pointer with the OS arrow, and nothing ever put it back: this is a
//! one-shot insert at `Startup`, not a per-frame write. `EguiGlobalSettings::
//! enable_cursor_icon_updates` is the flag the crate documents for exactly this,
//! and turning it off is the whole fix.
//!
//! Losing the I-beam costs nothing, because **the game has no I-beam to lose**:
//! `vale cursor` lists all 43 files under `Interface\Cursor\` and there is no
//! text cursor among them. 1.12 draws `Point` over an edit box like it draws it
//! over everything else that is not a contextual action.
//!
//! ## Which pointer belongs on a unit is not decided here
//!
//! It is [`vale_assets::look::cursor`]'s: "what is this unit *for*" is a game
//! rule, decidable with no window open, and it is the **client's** rule rather
//! than any file's — so it lives beside every other one. What is left in this module is a decode, an
//! `Image` and a `CursorIcon`, which is why there is no faction table and no
//! npc-flag mask in it.
//!
//! The eight in [`Cursor::ALL`] are the ones something here can currently
//! *choose*. The other thirty-five stay unused and stay surveyed by `vale
//! cursor`: the gathering set wants a profession, `PickLock` a rogue, `LootAll`
//! a corpse — each is its own subject.

use crate::assets::GameAssets;
use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{
    CursorGrabMode, CursorIcon, CursorOptions, CustomCursor, CustomCursorImage, PrimaryWindow,
    SystemCursorIcon,
};
use bevy_egui::EguiGlobalSettings;

use vale_assets::look::cursor::Cursor;
use std::collections::HashMap;

/// Where the point of the arrow is, in the bitmap's own texels.
///
/// Measured: `vale cursor Point` scans for the first covered texel and finds
/// (0, 0), which the coverage map confirms is the tip and not a stray. Every
/// other 32x32 cursor in the archive answers the same, bar the two skinning
/// ones — so this is a property of how the set was drawn, not a coincidence in
/// one file.
const HOTSPOT: (u16, u16) = (0, 0);

/// The decoded pointers, kept so one of them can be put back.
///
/// The insert in [`load`] is one-shot, and a one-shot insert is only as durable
/// as the last thing to write the component — which is how the egui bug above
/// went from "a wrong icon over a text box" to "the game's pointer is gone for
/// the rest of the session". Holding the handle is what lets
/// [`keep_the_game_pointer`] assert the rule every frame instead of trusting it.
///
/// Every contextual pointer is `Option` because a missing one is the same
/// documented degradation the ordinary pointer's is: the mode or the hover still
/// works, it simply looks like the arrow.
#[derive(Resource)]
struct GamePointer {
    /// The arrow, which is the only one that is not optional: without it there
    /// is nothing to put back and the platform keeps its own.
    point: CursorIcon,
    /// Keyed by the rule's own enum, so adding a cursor is a line in
    /// `assets::cursor` and nothing here.
    contextual: HashMap<Cursor, CursorIcon>,
    /// …and the refusing twin of the four the interface can ask for by name —
    /// see [`Cursor::unable_file`]. `unable_cast` used to be a field of its
    /// own; it is this map's `Cast` entry now, because `SetCursor` exposes all
    /// four twins and the sell hint uses `Buy`'s.
    unable: HashMap<Cursor, CursorIcon>,
}

impl GamePointer {
    /// The bitmap for one answer, falling back to the arrow — which is what a
    /// file that would not decode costs, and it is a cosmetic rather than a
    /// failure.
    fn icon(&self, cursor: Cursor) -> &CursorIcon {
        self.contextual.get(&cursor).unwrap_or(&self.point)
    }

    /// …and the same answer's refusing twin, falling back to the answer itself
    /// — a twin that would not decode costs the *warning* rather than the
    /// pointer, which is the right way round.
    fn refusing(&self, cursor: Cursor) -> &CursorIcon {
        self.unable.get(&cursor).unwrap_or_else(|| self.icon(cursor))
    }
}

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load).add_systems(
            Update,
            (
                hide_while_steering,
                // **Before the assertion**, which puts the *ordinary* pointer
                // back over anything that is not a custom icon: the two run in
                // the same frame and this one is what chooses between four
                // customs.
                choose_the_pointer,
                keep_the_game_pointer,
            )
                .chain()
                // …and after the pick it reads, so the sword appears on the
                // frame the pointer crosses the unit rather than the one after.
                .after(crate::game::combat::target::TargetSet)
                // **After the gesture has been decided**, since `hide_while_
                // steering` now reads it rather than the buttons. Unordered, the
                // pointer would vanish a frame late and — worse — stay hidden a
                // frame past the release.
                .after(crate::world::camera::arm_look),
        );
    }
}

/// Decode the pointer and hand it to the window.
///
/// A failure here is a **documented degradation and not an error**: the system
/// cursor is a perfectly usable pointer, and a client that refused to start
/// because a 2.7 kB bitmap would not decode would be trading the whole app for
/// a cosmetic. The warning names the file, which is the only thing worth
/// knowing.
fn load(
    mut commands: Commands,
    assets: Res<GameAssets>,
    mut images: ResMut<Assets<Image>>,
    mut egui: ResMut<EguiGlobalSettings>,
    window: Query<Entity, With<PrimaryWindow>>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let decode = |path: &str| {
        assets.with_archive(|archive| {
            let raw = archive.read(path).map_err(|e| e.to_string())?;
            vale_assets::world::blp::decode(&raw).map_err(|e| e.to_string())
        })
    };
    let blp = match decode(Cursor::Point.file()) {
        Ok(blp) => blp,
        Err(e) => {
            // Left with egui still driving the icon, deliberately: once the
            // pointer is the platform's, the platform's I-beam over a text field
            // is an improvement rather than a regression.
            warn!("{}: {e} — falling back to the system pointer", Cursor::Point.file());
            commands
                .entity(window)
                .insert(CursorIcon::System(SystemCursorIcon::Default));
            return;
        }
    };

    // `Rgba8UnormSrgb` because that is what the pixels are and what winit takes;
    // `MAIN_WORLD` only, since this never reaches the GPU — the window server
    // draws it.
    let upload = |blp: vale_assets::world::blp::Blp, images: &mut Assets<Image>| {
        let image = images.add(Image::new(
            Extent3d {
                width: blp.width,
                height: blp.height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            blp.rgba,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD,
        ));
        CursorIcon::Custom(CustomCursor::Image(CustomCursorImage {
            handle: image,
            texture_atlas: None,
            flip_x: false,
            flip_y: false,
            rect: None,
            hotspot: HOTSPOT,
        }))
    };
    let icon = upload(blp, &mut images);
    // A missing contextual cursor is a cosmetic and not a failure — see
    // [`GamePointer`] — so each is warned about once and left `None`. The mode
    // or the hover still works; it simply looks like the arrow.
    let optional = |path: &str, images: &mut Assets<Image>| match decode(path) {
        Ok(blp) => Some(upload(blp, images)),
        Err(e) => {
            warn!("{path}: {e} — that pointer will be the ordinary one");
            None
        }
    };
    // **Every cursor the rule can name, decoded once at startup** — eight 2.7 kB
    // bitmaps, so the whole set costs less than one texture and there is no
    // reason to be lazy about it. `Point` is already in hand.
    let contextual = Cursor::ALL
        .into_iter()
        .filter(|cursor| *cursor != Cursor::Point)
        .filter_map(|cursor| Some((cursor, optional(cursor.file(), &mut images)?)))
        .collect();
    let unable = Cursor::ALL
        .into_iter()
        .filter_map(|cursor| Some((cursor, optional(cursor.unable_file()?, &mut images)?)))
        .collect();
    commands.entity(window).insert(icon.clone());
    commands.insert_resource(GamePointer {
        point: icon,
        contextual,
        unable,
    });

    // The pointer is the game's from here on, so egui must stop writing to the
    // same component. See the module note: this is the one line that fixes the
    // login box eating the cursor, and it is set here rather than in the plugin
    // build so it depends on the decode having worked rather than on
    // the diagnostics plugin happening to be registered first.
    egui.enable_cursor_icon_updates = false;
}

/// Put the game's pointer back if anything replaced it.
///
/// **The flag in [`load`] is the fix; this is the assertion behind it.** Every
/// system in the app can write `CursorIcon` on the window, one of them did, and
/// the failure it produced was silent and permanent — the pointer was simply
/// the wrong one from then on, with nothing logged and no count to notice it.
/// One component compare per frame is cheap insurance against the next thing
/// that does it.
///
/// A `Custom` icon is left alone, which is deliberate: the other forty-two
/// cursors are the game's too, so a future mouse-over test putting `Attack` on
/// the window must not be fought by this.
fn keep_the_game_pointer(
    mut commands: Commands,
    pointer: Option<Res<GamePointer>>,
    window: Query<(Entity, &CursorIcon), (With<PrimaryWindow>, Changed<CursorIcon>)>,
) {
    let Some(pointer) = pointer else {
        return;
    };
    for (window, icon) in &window {
        if matches!(icon, CursorIcon::Custom(_)) {
            continue;
        }
        warn!("something replaced the game's pointer with a system cursor — putting it back");
        commands.entity(window).insert(pointer.point.clone());
    }
}

/// **Which of the game's pointers is on the window**, in one place and in the
/// reference's own order of urgency.
///
/// ```text
/// a spell is waiting to be pointed  -> Cast / UnableCast        a MODE
/// the pointer is over a unit        -> Hovered::cursor          a HOVER
/// …or over a game object            -> HoveredObject::cursor    …the same
/// otherwise                         -> Point
/// ```
///
/// **The middle two are not a precedence.** They are one ray's answer split into
/// two resources, and exactly one of them is filled on any frame — see
/// [`crate::game::npc::object::HoveredObject`]. Which it is comes out of the
/// depth test in the pick, so a mob standing in front of an ore vein draws a
/// sword and the vein behind it draws nothing.
///
/// **One writer, deliberately.** Two systems each inserting `CursorIcon` on the
/// same window would fight for it a frame at a time, and the loser is invisible
/// — a flickering pointer with nothing logged. So the mode and the hover are
/// arms of one match rather than two passes, and the priority between them is
/// stated here rather than emerging from system order.
///
/// The mode outranks the hover because it is a *question the client asked*: a
/// spell waiting to be aimed must keep saying so over a hostile unit, and
/// `UnableCast` over one the spell cannot have is the only warning there is.
///
/// **Which cursor a *unit* deserves is not decided here**, and that is the
/// division this module is about: `vale_assets::look::cursor` owns the rule —
/// `UNIT_NPC_FLAGS` first, then attackability, which is the client's own order —
/// and `game::target::judge_the_hover` runs it once a frame. All that is left in
/// this function is the mode, the fallbacks and the write.
///
/// Written only on a change, for the reason [`hide_while_steering`] gives: the
/// component crosses to the windowing thread, and a cursor set sixty times a
/// second is sixty round trips to say the same thing. `Changed<CursorIcon>` is
/// not the guard — the *comparison* is, because this runs before
/// [`keep_the_game_pointer`] and that one reads the change detection.
fn choose_the_pointer(
    mut commands: Commands,
    pointer: Option<Res<GamePointer>>,
    targeting: Option<Res<crate::game::combat::action::SpellTargeting>>,
    hovered: Option<Res<crate::game::combat::target::Hovered>>,
    // …and the other population the same pick answers for — see
    // [`crate::game::npc::object::HoveredObject`]. Only one of the two is ever
    // filled, so the `or` below is a choice between an answer and nothing
    // rather than a precedence.
    object: Option<Res<crate::game::npc::object::HoveredObject>>,
    // **…and what the interface asked for**, which is the fourth input and the
    // only one that is not derived from the world — see
    // [`crate::game::combat::cursor::Cursor::asked`].
    carried: Option<Res<crate::game::combat::cursor::Cursor>>,
    window: Query<(Entity, &CursorIcon), With<PrimaryWindow>>,
) {
    let (Some(pointer), Some(targeting)) = (pointer, targeting) else {
        return;
    };
    // **`over_valid` decides between the two spell cursors and nothing else
    // decides anything**: a pointer over empty ground reads "unable", which is
    // right — clicking there stands the mode down rather than casting.
    let wanted = if targeting.is_targeting() {
        if targeting.over_valid {
            pointer.icon(Cursor::Cast)
        } else {
            pointer.refusing(Cursor::Cast)
        }
    } else if let Some((cursor, unable)) = carried.and_then(|c| c.asked) {
        // **What the interface asked for**, and it sits here rather than above
        // the mode because the reference puts it here: both sell hints refuse
        // to run at all while a spell is waiting, so a
        // cast cursor is never replaced by a purse. `ResetCursor` is the `None`
        // this arm is not reached on, which falls through to the hover — the
        // same order the reference leaves the pointer in.
        if unable {
            pointer.refusing(cursor)
        } else {
            pointer.icon(cursor)
        }
    } else {
        // The rule's answer, or the arrow when it has none. A cursor whose
        // bitmap would not decode also lands on the arrow — see
        // [`GamePointer::icon`].
        let over = hovered
            .and_then(|hovered| hovered.cursor)
            .or_else(|| object.and_then(|object| object.cursor));
        match over {
            Some(cursor) => pointer.icon(cursor),
            None => &pointer.point,
        }
    };
    for (window, icon) in &window {
        if icon != wanted {
            commands.entity(window).insert(wanted.clone());
        }
    }
}

/// Hide and lock the pointer for as long as a **world** gesture is running.
///
/// Either button, not just the right one: a left-drag orbits the character just
/// as a right-drag steers it, and a pointer that stayed put through one drifted
/// to the edge of the screen and sat there. The real client hides it on the way
/// down and puts it back where it was on the way up, which is what
/// `CursorGrabMode::Locked` buys — a released cursor otherwise reappears
/// wherever the drag left it.
///
/// **A press that landed on the interface keeps its pointer**, which is the
/// whole reason this reads [`MouseLook`] rather than the buttons: dragging a bag
/// icon or a slider must not blind the person doing it.
fn hide_while_steering(
    look: Res<crate::world::camera::MouseLook>,
    mut options: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let steering = look.active;
    for mut options in &mut options {
        // Written only on a change: these cross to the windowing thread, and a
        // grab mode set sixty times a second is sixty round trips to say
        // nothing.
        let grab = if steering {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        if options.visible == steering {
            options.visible = !steering;
        }
        if options.grab_mode != grab {
            options.grab_mode = grab;
        }
    }
}
