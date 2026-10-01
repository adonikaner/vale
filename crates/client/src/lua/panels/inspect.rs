//! The inspect window's C functions: who may be inspected, and the honor tab.
//!
//! ```text
//! CanInspect(unit)              1 when the unit may be inspected, else nil,
//!                               with the reason printed
//! NotifyInspect(unit)           send CMSG_INSPECT, and inspect this player
//! ClearInspectPlayer()          inspect nobody
//! HasInspectHonorData()         1 when the honor tab's answer has arrived
//! RequestInspectHonorData()     ask for it, once per player
//! GetInspectHonorData()         the twelve numbers the honor tab shows
//! GetInspectPVPRankProgress()   progress toward the next rank, 0..1
//! ```
//!
//! The rule behind `CanInspect` is [`vale_assets::look::inspect`]. The three
//! verbs are queued and acted on by [`crate::interface::inspect`], and so is
//! `CanInspect`'s refusal line. The window's gear and model come from the
//! inspected unit's own fields and need nothing here.

use std::cell::RefCell;
use std::rc::Rc;

use vale_assets::look::inspect::Target;
use vale_protocol::play::inspect::InspectHonor;

use super::super::api::Answers;
use crate::interface::inspect::InspectPress;

pub type Queue = Rc<RefCell<Vec<InspectPress>>>;

/// The reads, for the interface census.
pub const READS: [&str; 4] = [
    "CanInspect",
    "GetInspectHonorData",
    "GetInspectPVPRankProgress",
    "HasInspectHonorData",
];

/// What the inspect functions need from the world.
pub trait InspectAnswers {
    /// What `CanInspect` tests about the unit `token` names, or `None` when it
    /// names nobody.
    fn inspect_target(&self, _token: &str) -> Option<Target> {
        None
    }
    /// The inspected player's honor tab, once it has arrived.
    fn inspect_honor(&self) -> Option<InspectHonor> {
        None
    }
}

impl InspectAnswers for super::super::api::Live<'_, '_, '_> {
    fn inspect_target(&self, token: &str) -> Option<Target> {
        use crate::interface::api::UnitId;
        let (unit, at) = self.units.placed(UnitId::parse(token)?)?;
        let (me, here) = self.units.placed(UnitId::Player)?;
        let factions = self.tables.as_ref().and_then(|tables| tables.factions());
        let charmed = unit.charmed_by.is_some_and(|g| g != 0) || me.charmed_by.is_some_and(|g| g != 0);
        Some(Target {
            player: unit.kind == vale_protocol::state::update::ObjectType::Player,
            own: unit.guid == me.guid,
            same_side: vale_assets::look::inspect::same_side(me.faction, unit.faction, charmed, |t| {
                factions.and_then(|f| f.group(t))
            }),
            distance_sq: at.translation.distance_squared(here.translation),
        })
    }

    fn inspect_honor(&self) -> Option<InspectHonor> {
        self.inspect.honor().copied()
    }
}

/// The three verbs, which need nothing from the world at the call. The queue
/// is also kept in the interpreter's app data, where `CanInspect`, a scoped
/// read, finds it to queue its refusal line.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();
    lua.set_app_data(Rc::clone(queue));
    let push = |name: &str, press: fn(Option<String>) -> Option<InspectPress>| -> mlua::Result<()> {
        let queue = Rc::clone(queue);
        let f = lua.create_function(move |_, token: Option<String>| {
            if let Some(press) = press(token) {
                queue.borrow_mut().push(press);
            }
            Ok(())
        })?;
        globals.set(name, f)
    };
    push("NotifyInspect", |token| {
        token.filter(|t| !t.is_empty()).map(InspectPress::Notify)
    })?;
    push("ClearInspectPlayer", |_| Some(InspectPress::Clear))?;
    push("RequestInspectHonorData", |_| Some(InspectPress::RequestHonor))?;
    Ok(())
}

pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    globals.set(
        "CanInspect",
        scope.create_function(move |lua, token: Option<String>| {
            let token = token.unwrap_or_default();
            let target = answers.inspect_target(&token.to_ascii_lowercase());
            let (allowed, refusal) =
                vale_assets::look::inspect::can_inspect(target.as_ref(), !token.is_empty());
            if let (Some(key), Some(queue)) = (refusal, lua.app_data_ref::<Queue>()) {
                queue.borrow_mut().push(InspectPress::Refused(key));
            }
            Ok(allowed.then_some(1))
        })?,
    )?;
    globals.set(
        "HasInspectHonorData",
        scope.create_function(move |_, ()| Ok(answers.inspect_honor().is_some().then_some(1)))?,
    )?;
    // The twelve values in the order `InspectHonorFrame_Update` takes them.
    // An empty tab, before the answer, is twelve zeroes.
    globals.set(
        "GetInspectHonorData",
        scope.create_function(move |_, ()| {
            let h = answers.inspect_honor().unwrap_or_default();
            Ok(honor_data(&h))
        })?,
    )?;
    // The rank bar's byte over 255.
    globals.set(
        "GetInspectPVPRankProgress",
        scope.create_function(move |_, ()| {
            let h = answers.inspect_honor().unwrap_or_default();
            Ok(f64::from(h.rank_progress) / 255.0)
        })?,
    )?;
    Ok(())
}

/// `GetInspectHonorData`'s twelve values: today's honorable and dishonorable
/// kills, yesterday's kills and honor, this week's kills and honor, last
/// week's kills, honor and standing, the lifetime honorable and dishonorable
/// kills, and the highest rank.
fn honor_data(h: &InspectHonor) -> mlua::Variadic<u32> {
    mlua::Variadic::from_iter([
        u32::from(h.today_kills),
        u32::from(h.today_dishonorable),
        u32::from(h.yesterday_kills),
        h.yesterday_honor,
        u32::from(h.this_week_kills),
        h.this_week_honor,
        u32::from(h.last_week_kills),
        h.last_week_honor,
        h.last_week_standing,
        h.lifetime_kills,
        h.lifetime_dishonorable,
        u32::from(h.highest_rank),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The values come out in the order `InspectHonorFrame_Update` assigns
    /// them, with distinct numbers so that a swap is caught.
    #[test]
    fn the_honor_data_is_in_the_frames_order() {
        let h = InspectHonor {
            guid: 1,
            highest_rank: 12,
            today_kills: 1,
            today_dishonorable: 2,
            yesterday_kills: 3,
            yesterday_honor: 4,
            this_week_kills: 5,
            this_week_honor: 6,
            last_week_kills: 7,
            last_week_honor: 8,
            last_week_standing: 9,
            lifetime_kills: 10,
            lifetime_dishonorable: 11,
            rank_progress: 0,
        };
        let values: Vec<u32> = honor_data(&h).into_iter().collect();
        assert_eq!(values, (1..=12).collect::<Vec<u32>>());
    }
}
