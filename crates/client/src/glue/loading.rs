//! **The loading screen: when it is up, and what is on it.**
//!
//! Not a frame, not a widget and not a `GlueXML` file — 1.12's loading screen is
//! the client's own full-screen draw, four vertices and a bar, and nothing in
//! `Interface\` mentions it. So it is state here and paint in
//! [`crate::ui::loading`], on the same split every other subject in this
//! directory keeps. The *rules* — which picture a map gets, where the bar sits —
//! are one crate down in [`vale_assets::tables::loading`], where they are checked by
//! `vale loading` with no window open.
//!
//! ## The two moments, and only one of them is on the wire
//!
//! ```text
//! entering the world   Screen::Characters -> Screen::Entering   Session::entering_map
//! a far teleport       SMSG_TRANSFER_PENDING                    its own map id
//! ```
//!
//! The second is the whole reason `SMSG_TRANSFER_PENDING` exists. vmangos sends
//! it from `Player::ExecuteTeleportFar` **immediately before removing the
//! character from the old map**, with `// send transfer packet to display load
//! screen` on the line above, and it carries nothing but the destination. Read
//! the map change off `SMSG_NEW_WORLD` instead and the screen goes up *after*
//! the world it was supposed to hide has already gone.
//!
//! A same-map teleport — `.tele` across a continent, a summon — sends
//! `MSG_MOVE_TELEPORT_ACK` and no transfer at all, and gets no loading screen,
//! which is the reference's behaviour and not an omission here.
//!
//! ## What it comes down on, which is this client's own answer
//!
//! The reference blocks: the client spins its IO queue until the
//! building under the character is in, and the loading screen is what the player
//! sees while it does. This client cannot block its own frame — see the note on
//! [`crate::world::session::Standing`] — so the
//! screen instead stays up until the streamer has caught up:
//!
//! ```text
//! 1. there is a world, and it is the one we were told we were going to
//! 2. the terrain pass is streaming that map, not the one we left
//! 3. every tile of its 3x3 has arrived and been handed over
//! 4. no building on them is still being read or staged
//! ```
//!
//! Steps 2 and 4 are the two that are easy to leave out and both are visible
//! when they are. Without 2 there is a single frame, on the far side of a
//! teleport, where the *old* map's nine tiles are complete and the screen drops
//! on them. Without 4 the ground is there and Stormwind assembles itself in
//! front of the player, which is the picture half of the falling-through-the-
//! floor report.
//!
//! ## …and it always comes down
//!
//! [`GIVE_UP_SECS`] is the one thing here that is a policy rather than a
//! measurement, and it exists because every other condition above is somebody
//! else's: a client that cannot reach its archives, or a map with no ADTs at
//! all, would otherwise sit behind a parchment for ever with no way to say so.
//! Giving up logs a warning and shows the world, which is the honest failure —
//! the player sees whatever did load, rather than nothing.
//!
//! **The bar is the ground and the tail is the buildings.** Its fraction is the
//! share of the 3x3 that has arrived, which is one number that cannot lie; the
//! last condition holds a full bar for as long as the city takes. That is not
//! the reference's arithmetic — 1.12 blends three counters through an easing —
//! and it is deliberately not dressed up as it.

use crate::interface::events::PlayerLeavingWorld;
use crate::assets::GameAssets;
use crate::render::terrain::LoadedTiles;
use crate::render::wmos::WmoCache;
use crate::world::session::{Screen, Session, WorldStatus};
use vale_assets::tables::loading::LoadingScreens;
use bevy::prelude::*;
use std::sync::Arc;

/// How long the screen may stay up before it is taken down regardless.
///
/// **Taste, and the only number in this module that is.** Nothing measured says
/// what it should be; what it is for is that every condition
/// [`settle`] waits on belongs to another pass, so a fault in any of them would
/// otherwise be a client that never draws the world again and never says why.
/// Thirty seconds is several times the slowest login this project has measured
/// (a cold city is nine tiles at ~10 MiB of upload apiece, which the streamer
/// spends a few seconds on) and short enough that a person notices it end.
const GIVE_UP_SECS: f32 = 30.0;

/// The shortest time the screen is shown for once it is up.
///
/// Also taste. A teleport whose destination tiles are all still cached settles
/// on the frame after it is raised, and a full-screen picture that appears and
/// disappears inside two frames reads as a flash of corruption rather than as a
/// loading screen. The reference has no equivalent because it is never that
/// fast: it is re-reading the map from a CD-era disk.
const MINIMUM_SECS: f32 = 0.5;

/// **`SMSG_TRANSFER_PENDING`, forwarded off the session's event queue** by
/// [`crate::interface::action`]'s drain — the destination map, which is the packet's whole
/// content.
#[derive(Message, Debug, Clone, Copy)]
pub struct TransferPending(pub u32);

/// Whether the world is hidden, and behind what.
///
/// The painter reads exactly two things off this — [`LoadingScreen::picture`]
/// and [`LoadingScreen::progress`] — and holds no opinion about either.
#[derive(Resource, Default)]
pub struct LoadingScreen {
    up: Option<Raised>,
    /// The two DBCs, taken once on the first raise. Not at startup: opening the
    /// archive chain is ~19 files and the login screen should paint first, which
    /// is the same argument `messages::load_strings` makes.
    screens: Option<Arc<LoadingScreens>>,
}

/// A screen that is up.
struct Raised {
    /// The `Interface\` path being drawn, already through the archive check —
    /// see [`vale_assets::tables::loading::LoadingScreens::picture_in`].
    picture: String,
    /// Which map we are waiting to be standing on, or `None` for a login, where
    /// arriving anywhere at all is arriving.
    ///
    /// A destination that is *already* the current map — the one shape of
    /// transfer that happens without a map change, a character standing on a
    /// boat — passes this test immediately and is held only by
    /// [`MINIMUM_SECS`]. Stated rather than special-cased: nothing else on the
    /// wire distinguishes it, and a boat is `SMSG_MONSTER_MOVE_TRANSPORT`'s
    /// subject rather than this module's.
    destination: Option<u32>,
    /// `Time::elapsed_secs` when it went up.
    raised_at: f32,
    /// How full the bar is, `0..=1`.
    progress: f32,
}

impl LoadingScreen {
    /// The picture to draw over everything, or `None` for "the world is
    /// visible".
    pub fn picture(&self) -> Option<&str> {
        self.up.as_ref().map(|up| up.picture.as_str())
    }

    /// How full the bar is. Meaningless while [`Self::picture`] is `None`.
    pub fn progress(&self) -> f32 {
        self.up.as_ref().map_or(0.0, |up| up.progress)
    }
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadingScreen>()
            .add_message::<TransferPending>()
            // **`raise` before `settle`, and both in `GameSet`.** A transfer
            // raised and settled in one frame would be a screen nobody ever
            // saw; the order makes the earliest a screen can come down the
            // frame *after* the one it went up on, which `MINIMUM_SECS` then
            // widens. And the set puts both after the world has been polled,
            // so `settle` reads this frame's map rather than last frame's.
            .add_systems(Update, (raise, settle).chain().in_set(crate::interface::GameSet));
    }
}

/// Put the screen up: at a login, and at a far teleport.
fn raise(
    time: Res<Time>,
    assets: Res<GameAssets>,
    session: Res<Session>,
    mut screen: ResMut<LoadingScreen>,
    mut transfers: MessageReader<TransferPending>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
) {
    // **Leaving the world takes the screen with it.** The one shape of stuck
    // screen [`settle`] cannot reason its way out of: it is waiting for a map
    // that a logout has just made unreachable, so without this it sits over the
    // character-select screen until [`GIVE_UP_SECS`]. Read even when nothing is
    // up, because an unread queue is a message delivered late.
    if leaving.read().count() > 0 {
        screen.up = None;
    }

    let destination: Option<u32> = match (transfers.read().last().copied(), session.screen()) {
        // A transfer wins over a login: the two cannot both be happening, and
        // the packet is the more recent statement if they somehow were.
        (Some(TransferPending(map)), _) => Some(map),
        // Entering the world, which is the one raise with no packet behind it —
        // and `None`, because arriving anywhere at all is arriving.
        (None, Screen::Entering) if screen.up.is_none() => None,
        _ => return,
    };

    // The map whose parchment to show. For a login that is the character's own,
    // recorded at the press because the row it came from is consumed before
    // there is a session to ask — see `Session::entering_map`.
    let map = destination.or(session.entering_map).unwrap_or(0);
    let screens = screen
        .screens
        .get_or_insert_with(|| assets.loading_screens())
        .clone();
    // …through the archive, which is the client's fifth fallback path and the
    // one a table cannot answer. One archive lookup per raise.
    let picture = screens
        .picture_in(map, |path| {
            assets
                .with_archive(|archive| Ok(archive.read(path).is_ok()))
                .unwrap_or(false)
        })
        .to_string();

    info!("loading screen up for map {map}: {picture}");
    screen.up = Some(Raised {
        picture,
        destination,
        raised_at: time.elapsed_secs(),
        progress: 0.0,
    });
}

/// **How full the bar is and whether the world is there**, as arithmetic over
/// four plain numbers.
///
/// A free function rather than four lines inside [`settle`] for the reason
/// [`crate::lua::widgets::draw`]'s assertions are unit tests rather than screenshots:
/// every interesting claim about the loading screen is about *these four* —
/// zero wanted tiles is not completion, a city still staging is not completion,
/// and the bar is a share of what is outstanding — and none of them needs a
/// session, a socket or a render world to state.
///
/// `arrived` is conditions 1 and 2 of the module comment already answered,
/// because both are about identity rather than progress.
fn reckon(arrived: bool, wanted: usize, settling: usize, buildings: usize) -> (f32, bool) {
    if !arrived || wanted == 0 {
        // **Not a full bar.** Zero wanted tiles is "the streamer has not
        // chosen a 3x3 yet", which is the state every login begins in; reading
        // it as "nothing left to do" would drop the screen on frame one.
        return (0.0, false);
    }
    let landed = wanted - settling.min(wanted);
    (
        landed as f32 / wanted as f32,
        settling == 0 && buildings == 0,
    )
}

/// Fill the bar, and take the screen down when the world behind it is there.
///
/// The four conditions and why each is in the list are in the module comment;
/// the last two are [`reckon`].
fn settle(
    time: Res<Time>,
    session: Res<Session>,
    status: Res<WorldStatus>,
    tiles: Res<LoadedTiles>,
    wmos: Res<WmoCache>,
    // …and the geometry a map with *no* tiles has instead, which the two
    // numbers above cannot see: on a WMO-only map the streamer's nine ADTs all
    // fail to read at once, so `settling` is zero from the first frame and this
    // screen would come down on an empty room. See
    // [`crate::render::globalwmo::GlobalBuilding::settling`].
    global: Res<crate::render::globalwmo::GlobalBuilding>,
    mut screen: ResMut<LoadingScreen>,
) {
    let now = time.elapsed_secs();
    let Some(up) = screen.up.as_mut() else {
        return;
    };
    let waited = now - up.raised_at;

    // There is a world, it is the one we were told we were going to, and the
    // terrain pass is streaming *that* map — see [`LoadedTiles::map`], which
    // exists for exactly the one frame where the last of those is false and the
    // other two are true.
    let arrived = session
        .active
        .as_ref()
        .is_some_and(|active| up.destination.is_none_or(|map| active.map_id == map))
        && status.in_world
        && tiles.map() == Some(status.map_name.as_str());

    let (wanted, settling) = (tiles.wanted(), tiles.settling());
    // **The building count carries both populations**, because that is the one
    // `reckon` already treats as "there is more of this world to come" without
    // it having to be a tile. A map with a global WMO is reading its WDT, then
    // reading the building, and neither is a tile.
    let buildings = wmos.busy() + global.settling();
    let (progress, ready) = reckon(arrived, wanted, settling, buildings);
    up.progress = progress;

    if waited >= GIVE_UP_SECS {
        warn!(
            "loading screen giving up after {GIVE_UP_SECS:.0}s — {settling} of {wanted} tile(s) \
             and {buildings} building(s) still outstanding"
        );
        screen.up = None;
        return;
    }
    if ready && waited >= MINIMUM_SECS {
        info!("loading screen down after {waited:.1}s");
        screen.up = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The state every login begins in is not completion.** `wanted == 0`
    /// means the terrain pass has not chosen a 3x3 yet, and a client that read
    /// it as "nothing outstanding" would take the screen down on the first
    /// frame of every login and every teleport — which is indistinguishable
    /// from having no loading screen at all.
    #[test]
    fn a_world_that_has_not_started_streaming_is_not_a_loaded_one() {
        assert_eq!(reckon(true, 0, 0, 0), (0.0, false));
        // …and neither is a *finished* 3x3 on the map we have just left, which
        // is the frame `LoadedTiles::map` exists for.
        assert_eq!(reckon(false, 9, 0, 0), (0.0, false));
    }

    /// **The one number the bar is**: the share of the 3x3 that has landed,
    /// counted as what is *not* outstanding — see [`LoadedTiles::settling`],
    /// which is why a tile whose ADT will not parse cannot hold it below full.
    #[test]
    fn the_bar_is_the_share_of_the_tiles_that_have_landed() {
        assert_eq!(reckon(true, 9, 9, 0).0, 0.0);
        assert!((reckon(true, 9, 3, 0).0 - 6.0 / 9.0).abs() < 1e-6);
        assert_eq!(reckon(true, 9, 0, 0).0, 1.0);
        // A 3x3 clipped by the edge of the map is four tiles, not nine, and a
        // full bar is still a full bar.
        assert_eq!(reckon(true, 4, 0, 0), (1.0, true));
    }

    /// **A full bar is not a finished load while the city is still arriving.**
    /// The condition that is easy to leave out and visible when it is: the
    /// ground is there and Stormwind assembles itself in front of the player.
    #[test]
    fn the_buildings_hold_the_screen_after_the_ground_has_landed() {
        assert_eq!(reckon(true, 9, 0, 5), (1.0, false));
        assert_eq!(reckon(true, 9, 0, 0), (1.0, true));
    }

    /// A world with the resources [`settle`] reads and nothing else — no
    /// archives, no render plugins, no socket. `Session::active` is `None`
    /// here, which is exactly the state a screen waiting on a world that never
    /// arrives is in.
    fn harness() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<Session>()
            .init_resource::<WorldStatus>()
            .init_resource::<LoadedTiles>()
            .init_resource::<WmoCache>()
            // …and the other population, for the maps with no tiles at all.
            .init_resource::<crate::render::globalwmo::GlobalBuilding>()
            .init_resource::<LoadingScreen>()
            .add_systems(Update, settle);
        app
    }

    /// Put a screen up by hand, since [`raise`] needs an archive chain.
    fn show(app: &mut App, raised_at: f32) {
        app.world_mut().resource_mut::<LoadingScreen>().up = Some(Raised {
            picture: "Interface\\Glues\\loading".to_string(),
            destination: None,
            raised_at,
            progress: 0.0,
        });
    }

    /// **It always comes down**, which is the one promise here that does not
    /// depend on any other pass keeping its own — see [`GIVE_UP_SECS`]. A
    /// screen over a world that never arrives is a client with no way to say
    /// what went wrong.
    #[test]
    fn a_world_that_never_arrives_still_gives_the_player_the_screen_back() {
        let mut app = harness();

        // Freshly up, over nothing at all: it stays.
        show(&mut app, 0.0);
        app.update();
        assert!(app.world().resource::<LoadingScreen>().picture().is_some());
        assert_eq!(app.world().resource::<LoadingScreen>().progress(), 0.0);

        // …and past its deadline it does not.
        show(&mut app, -GIVE_UP_SECS - 1.0);
        app.update();
        assert!(
            app.world().resource::<LoadingScreen>().picture().is_none(),
            "the screen outlived its own deadline"
        );
    }

    /// The deadline has to be well clear of the floor, or a screen that is
    /// merely slow to raise is one that gives up before it can ever come down
    /// cleanly. Cheap to state and easy to break by tuning one of them.
    #[test]
    fn the_minimum_is_far_inside_the_deadline() {
        const { assert!(MINIMUM_SECS * 10.0 < GIVE_UP_SECS) };
    }
}
