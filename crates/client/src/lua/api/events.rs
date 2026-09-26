//! **The world's news, delivered to the interface.**
//!
//! One system. It takes everything [`crate::game::events`] wrote, and for each
//! one calls every frame that said `RegisterEvent` for it — which is the other
//! half of the loop the binding dispatch is the first half of:
//!
//! ```text
//! a key goes down    ->  a <Binding> body  ->  a verb   ->  the client acts
//! the client learns  ->  an event name     ->  OnEvent  ->  the interface redraws
//! ```
//!
//! Until this existed, `game::events` was nine well-named message types that
//! nothing in Lua could hear. That was the one joint the widget tree genuinely
//! needs, and it is this file.
//!
//! ## Ordering: after the whole of `game/`, and what that costs
//!
//! The dispatch runs **after** [`crate::game::GameSet`], so an event written
//! anywhere in that directory is delivered in the same frame it was written. The
//! price is at the other end: a verb an `OnEvent` handler calls is announced as a
//! [`crate::game::bindings::BindingPressed`] *after* the systems that read those
//! have run, so it takes effect on the next frame.
//!
//! That is a stated one-frame lag and not a bug waiting to be found. It is also
//! the right trade: nothing in the shipped FrameXML calls a verb from an event
//! handler at all (`ActionButton_OnEvent` and `TargetFrame_OnEvent` only redraw),
//! whereas *every* handler wants the news to be this frame's. The alternative —
//! dispatching inside `GameSet` — would delay the news instead, which is the half
//! everything depends on.

use bevy::prelude::*;

use super::super::api::LuaWorld;
use super::super::host::LuaHost;
use crate::game::bindings::BindingPressed;
use crate::game::events::GameEventReaders;

pub struct EventsPlugin;

impl Plugin for EventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, dispatch.after(crate::game::GameSet));
    }
}

/// Hand every event written this frame to the frames listening for it.
///
/// `pub(super)` so that [`super::update`] can order itself after it — a frame
/// told about the world this frame should animate from the state that left it in.
#[allow(clippy::too_many_arguments)]
pub(in crate::lua) fn dispatch(
    host: Option<NonSendMut<LuaHost>>,
    mut events: GameEventReaders,
    world: LuaWorld,
    mut pressed: MessageWriter<BindingPressed>,
) {
    let Some(mut host) = host else { return };
    // **Drained whether or not anything is listening.** A reader that only looks
    // when a frame exists would deliver a backlog of stale news the moment the
    // first frame registered — every target change since login, in order.
    let news = events.drain();
    if news.is_empty() {
        return;
    }
    let live = world.live();
    // **One scope for the whole frame's news** — see [`LuaHost::fire_events`],
    // where the arithmetic is. This loop used to open one per event, which made
    // the interface's per-frame cost scale with how much the world had to say
    // and is the whole of why a party of four cost what it did.
    let batch: Vec<(&str, &[crate::game::events::EventArg])> =
        news.iter().map(|(event, args)| (*event, args.as_slice())).collect();
    for binding in host.fire_events(&batch, &live) {
        pressed.write(BindingPressed(binding));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::events::{PlayerTargetChanged, SpellcastStart, UiErrorMessage};
    use crate::lua::api::tests::Stub;

    /// A world with just enough in it to run this one system.
    fn app() -> App {
        let mut app = App::new();
        crate::game::events::register(&mut app);
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .add_message::<BindingPressed>()
            .add_systems(Update, dispatch);
        app
    }

    /// Load a chunk into the app's host, the way the XML loader will.
    fn load(app: &mut App, source: &str) {
        let world = Stub::default();
        let host = app.world_mut().get_non_send_mut::<LuaHost>().expect("host");
        host.run(&world, |lua| lua.load(source).exec())
            .expect("the chunk loads");
    }

    fn global(app: &mut App, expression: &str) -> String {
        let world = Stub::default();
        let host = app.world_mut().get_non_send_mut::<LuaHost>().expect("host");
        host.run(&world, |lua| {
            let value: mlua::Value = lua.load(expression).eval()?;
            Ok(format!("{value:?}"))
        })
        .expect("the expression runs")
    }

    /// **The whole loop, in one system**: an event written by `game/` reaches a
    /// frame that registered for it, in the same frame, with the game's own
    /// arguments in `arg1`.
    ///
    /// This is the test that would have caught the wiring being absent, which is
    /// the state the round before this one left the client in: nine well-named
    /// message types that nothing in Lua could hear, with no error anywhere.
    #[test]
    fn an_event_written_this_frame_reaches_the_frame_that_registered() {
        let mut app = app();
        load(
            &mut app,
            r#"
            heard = {};
            f = CreateFrame("Frame", "CastingBarFrame");
            f:RegisterEvent("SPELLCAST_START");
            f:SetScript("OnEvent", function()
                table.insert(heard, event .. " " .. arg1 .. " " .. arg2);
            end);
            "#,
        );
        app.world_mut().write_message(SpellcastStart {
            name: "Fireball".to_string(),
            duration_ms: 3500,
        });
        app.update();
        assert_eq!(
            global(&mut app, "return heard[1]"),
            r#"String("SPELLCAST_START Fireball 3500")"#
        );
    }

    /// **An event nothing registered for is dropped rather than queued**, which
    /// is what stops a frame created an hour into a session being handed every
    /// target change since login the moment it registers.
    #[test]
    fn news_nobody_asked_for_is_not_kept() {
        let mut app = app();
        app.world_mut().write_message(PlayerTargetChanged);
        app.world_mut()
            .write_message(UiErrorMessage("You are too far away!".to_string()));
        app.update();

        load(
            &mut app,
            r#"
            heard = 0;
            f = CreateFrame("Frame", "Late");
            f:RegisterEvent("PLAYER_TARGET_CHANGED");
            f:SetScript("OnEvent", function() heard = heard + 1; end);
            "#,
        );
        app.update();
        assert_eq!(global(&mut app, "return heard"), "Integer(0)");
        // …and the next one really does arrive, so the drain is not simply broken.
        app.world_mut().write_message(PlayerTargetChanged);
        app.update();
        assert_eq!(global(&mut app, "return heard"), "Integer(1)");
    }

    // **A read inside a handler sees the live world** is covered by
    // `host::tests::an_event_reaches_the_frame_that_registered_for_it`, over an
    // `Answers` stub rather than a spawned `WorldEntity` — which is the whole
    // reason [`super::super::api::Answers`] is a trait. What the two tests above
    // add is that the *real* `Units` param resolves and the system schedules,
    // which is the half a stub cannot check.
}
