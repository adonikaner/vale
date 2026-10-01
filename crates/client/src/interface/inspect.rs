//! The client's side of `InspectFrame`: whom the character is inspecting, and
//! that player's honor tab.
//!
//! `Blizzard_InspectUI` draws the gear and the model from the inspected unit's
//! own fields, which every client in view already has, so the only state the
//! window needs from this module is the honor tab's. The 1.12.1 client keeps
//! three things for it, and so does [`Inspect`]:
//!
//! * the inspected guid, which `NotifyInspect` sets and `ClearInspectPlayer`
//!   clears. A new guid drops the honor data and the outstanding request.
//! * whether an honor request is outstanding, so `RequestInspectHonorData`
//!   sends `MSG_INSPECT_HONOR_STATS` once per guid.
//! * the answer, kept only when it is for the inspected guid, at which point
//!   `INSPECT_HONOR_UPDATE` is raised.
//!
//! See [`crate::lua::panels::inspect`] for the C functions over this, and
//! [`vale_protocol::play::inspect`] for the packets.

use bevy::prelude::*;

use vale_protocol::play::inspect::InspectHonor;

use crate::interface::api::{UnitId, Units};
use crate::interface::events::InspectHonorUpdate;
use crate::interface::messages::TableLine;
use crate::world::session::Session;

/// `MSG_INSPECT_HONOR_STATS` from the server, routed by `world::incoming`.
#[derive(Message, Debug, Clone, Copy)]
pub struct InspectAnswer(pub InspectHonor);

/// What the inspect window's C functions ask for, queued by
/// [`crate::lua::panels::inspect`] and acted on here.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum InspectPress {
    /// `NotifyInspect(unit)`: send `CMSG_INSPECT` and make this the inspected
    /// player.
    Notify(String),
    /// `ClearInspectPlayer()`.
    Clear,
    /// `RequestInspectHonorData()`.
    RequestHonor,
    /// A `CanInspect` refusal, printed through the message table.
    Refused(&'static str),
}

/// Whom the character is inspecting, and the honor tab's answer.
#[derive(Resource, Default, Debug)]
pub struct Inspect {
    guid: Option<u64>,
    honor: Option<InspectHonor>,
    pending: bool,
}

impl Inspect {
    /// The inspected player's honor, once it has arrived.
    pub fn honor(&self) -> Option<&InspectHonor> {
        self.honor.as_ref()
    }

    /// Make `guid` the inspected player. A change drops the honor data and the
    /// outstanding request, as the 1.12.1 client does; the same guid again
    /// keeps both.
    fn set_guid(&mut self, guid: Option<u64>) {
        if self.guid != guid {
            self.guid = guid;
            self.honor = None;
            self.pending = false;
        }
    }

    /// The guid to ask for honor about, if a request should go out: there is an
    /// inspected player, no answer yet and no request outstanding. Marks the
    /// request outstanding.
    fn want_honor(&mut self) -> Option<u64> {
        if self.honor.is_some() || self.pending {
            return None;
        }
        let guid = self.guid?;
        self.pending = true;
        Some(guid)
    }

    /// Keep an answer if it is for the inspected player. Returns whether it was
    /// kept.
    fn answered(&mut self, honor: InspectHonor) -> bool {
        if self.guid != Some(honor.guid) {
            return false;
        }
        self.honor = Some(honor);
        self.pending = false;
        true
    }
}

pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<InspectAnswer>()
            .add_message::<InspectPress>()
            .init_resource::<Inspect>()
            .add_systems(Update, (answers, presses, act).chain().in_set(super::GameSet));
    }
}

/// Keep the server's honor answer and tell the window.
fn answers(
    mut incoming: MessageReader<InspectAnswer>,
    mut inspect: ResMut<Inspect>,
    mut update: MessageWriter<InspectHonorUpdate>,
) {
    for InspectAnswer(honor) in incoming.read() {
        if inspect.answered(*honor) {
            update.write(InspectHonorUpdate);
        }
    }
}

/// Drain the Lua host's queue into messages.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<InspectPress>) {
    let Some(mut host) = host else { return };
    for press in host.take_inspect_presses() {
        out.write(press);
    }
}

/// Send what the window asked for, and print a refusal.
fn act(
    mut presses: MessageReader<InspectPress>,
    mut inspect: ResMut<Inspect>,
    session: Res<Session>,
    units: Units,
    mut say: MessageWriter<TableLine>,
) {
    for press in presses.read() {
        match press {
            InspectPress::Notify(token) => {
                let Some(guid) = UnitId::parse(token)
                    .and_then(|id| units.get(id))
                    .map(|unit| unit.guid)
                else {
                    continue;
                };
                if let Some(active) = session.active.as_ref() {
                    active.live.inspect(guid);
                }
                inspect.set_guid(Some(guid));
            }
            InspectPress::Clear => inspect.set_guid(None),
            InspectPress::RequestHonor => {
                if let (Some(guid), Some(active)) = (inspect.want_honor(), session.active.as_ref()) {
                    active.live.inspect_honor(guid);
                }
            }
            InspectPress::Refused(key) => {
                say.write(TableLine(key));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn honor(guid: u64) -> InspectHonor {
        InspectHonor {
            guid,
            lifetime_kills: 12,
            ..Default::default()
        }
    }

    /// One request per inspected player, an answer for anyone else is
    /// dropped, and a new player starts again.
    #[test]
    fn the_honor_tab_asks_once_per_player() {
        let mut inspect = Inspect::default();
        assert_eq!(inspect.want_honor(), None, "nobody is being inspected");
        inspect.set_guid(Some(7));
        assert_eq!(inspect.want_honor(), Some(7));
        assert_eq!(inspect.want_honor(), None, "one request outstanding");
        assert!(!inspect.answered(honor(8)), "an answer about someone else");
        assert!(inspect.answered(honor(7)));
        assert_eq!(inspect.honor().map(|h| h.lifetime_kills), Some(12));
        assert_eq!(inspect.want_honor(), None, "already answered");
        inspect.set_guid(Some(7));
        assert!(inspect.honor().is_some(), "the same player keeps the answer");
        inspect.set_guid(Some(9));
        assert!(inspect.honor().is_none(), "a new player drops it");
        assert_eq!(inspect.want_honor(), Some(9));
        inspect.set_guid(None);
        assert_eq!(inspect.want_honor(), None);
    }
}
