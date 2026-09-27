//! **The C functions `PetStable.lua` calls** — seven reads, four writes.
//!
//! ```text
//! GetNumStableSlots()        slots bought      GetNumStablePets()   rows held
//! GetSelectedStablePet()     which stall is ticked, or -1
//! GetStablePetInfo(i)        icon, name, level, family, loyalty
//! GetStablePetFoodTypes(i)   the diet, as a vararg BuildListString joins
//! GetNextStableSlotCost()    copper, straight into MoneyFrame_Update
//! SetPetStablePaperdoll(f)   point a <PlayerModel> at the selected pet
//!
//! ClickStablePet(i)          PickupStablePet(i)
//! BuyStableSlot()            ClosePetStables()
//! ```
//!
//! The split every panel keeps: the wire is
//! [`vale_protocol::play::stable`], the window is
//! [`crate::interface::stable`], the family and diet and slot price are
//! [`vale_assets::tables::pet`], and this file is registration and
//! arguments.
//!
//! ## Every one of these was a stub, and the stubs were not wrong
//!
//! `GetSelectedStablePet` answered `-1`, `GetStablePetInfo` answered nil, the
//! three counts and the cost answered `0`, and `ClickStablePet` did nothing.
//! Read as a *state* rather than as a hole, that is exactly a hunter standing
//! at a stable master with nothing stabled and no slots bought — which is why
//! the panel drew correctly and nothing reported a fault. See
//! [`super::super::api::stubs`], where they were and what each was chosen to
//! be; the values here are the same ones whenever the window is shut.
//!
//! ## `ClickStablePet` returns whether the panel should redraw
//!
//! `PetStableSlotTemplate`'s `OnClick` is `if ( ClickStablePet(this:GetID()) )
//! then PetStable_Update(); end`, so a falsey answer is a click that changes
//! nothing. This answers **1 whenever a stable is open**: the tick moves to the
//! pressed stall whatever the wire does, and the window's own
//! `PET_STABLE_UPDATE` follows the packet a round trip later. Answering only on
//! a packet being sent would leave the tick behind on the one press that sends
//! nothing — slot 0 with no pet out, which is the state a fresh hunter's panel
//! opens in.
//!
//! ## `GetStablePetInfo` is five values and only three are in the packet
//!
//! The icon and the family come from `CreatureFamily.dbc` by way of the pet's
//! creature entry, which is a `CMSG_CREATURE_QUERY` round trip —
//! [`crate::interface::stable::resolve`] is what raises `PET_STABLE_UPDATE`
//! again when it lands. Until then both are **empty strings rather than nil**,
//! because `PetStable_Update` concatenates them into a label
//! (`name.." "..format(UNIT_LEVEL_TEMPLATE, level).." "..family`) and a nil
//! there is an error that takes the whole panel down, not a blank word.
//!
//! ## `SetPetStablePaperdoll` draws all three stalls, and two of them are not
//! ## units
//!
//! The frame is a `<PlayerModel>` and [`crate::render::paperdoll`] drew one by
//! resolving a *unit token*, which the pet that is out has (`"pet"`) and a
//! **stabled** pet does not: it has no guid, no entity and no token, only a
//! creature entry. So for two rounds this pointed the frame at `"pet"` for the
//! current stall and cleared it otherwise — the reference's picture in the one
//! case a hunter sees most and an empty rectangle in the other two.
//!
//! What closed it is a second way in to that pass:
//! [`crate::render::paperdoll::DISPLAY_ID_PREFIX`], a subject named by display
//! id rather than by token. The entry is the stable packet's own and the
//! display id behind it is `SMSG_CREATURE_QUERY_RESPONSE`'s — the same template
//! cache the family, the icon and the diet are already read from, so a stabled
//! pet draws as soon as the query that names it lands and an empty rectangle
//! for the frame or two before that.

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
///
/// **`SetPetStablePaperdoll` is in this list and it is a write** — it changes a
/// frame, and it has to have done so before the call returns, exactly as
/// `SelectTrainerService` does. See [`install`].
pub const READS: [&str; 7] = [
    "GetNextStableSlotCost",
    "GetNumStablePets",
    "GetNumStableSlots",
    "GetSelectedStablePet",
    "GetStablePetFoodTypes",
    "GetStablePetInfo",
    "SetPetStablePaperdoll",
];

/// One stall, as the panel reads it — `GetStablePetInfo`'s five values in the
/// order it returns them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StableLine {
    /// `CreatureFamily.dbc`'s own icon path, whole. Empty until the creature
    /// query lands — see the module note.
    pub icon: String,
    pub name: String,
    pub level: u32,
    /// "Wolf", "Cat". Empty on the same terms as the icon.
    pub family: String,
    /// The `PetLoyalty.dbc` label, already in words.
    pub loyalty: String,
}

/// **What the stable panel may ask.**
pub trait StableAnswers {
    /// `GetNumStableSlots()` — slots **bought**, 0..2.
    fn stable_slots(&self) -> u32;
    /// `GetNumStablePets()` — rows the packet carried, the current pet
    /// included.
    fn stable_pets(&self) -> u32;
    /// `GetSelectedStablePet()` — **-1 when nothing is picked**, which is the
    /// value `PetStable_Update`'s own first branch tests for.
    fn selected_stable_pet(&self) -> i32;
    /// `GetStablePetInfo(i)` — `None` for an empty stall, which the panel draws
    /// as `EMPTY_STABLE_SLOT`.
    fn stable_pet_info(&self, panel_slot: u8) -> Option<StableLine>;
    /// `GetStablePetFoodTypes(i)` — a **vararg**, joined by `BuildListString`.
    fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String>;
    /// `GetNextStableSlotCost()` — copper, and `0` once every slot is bought.
    fn next_stable_slot_cost(&self) -> u32;
    /// Which subject `SetPetStablePaperdoll` should point a frame at, if any.
    ///
    /// A **unit token** for the pet that is out and a
    /// [`crate::render::paperdoll::DISPLAY_ID_PREFIX`] id for a stabled one,
    /// which has no token to be named by — see the module note.
    fn stable_paperdoll_unit(&self) -> Option<String>;
}

impl StableAnswers for super::super::api::Live<'_, '_, '_> {
    fn stable_slots(&self) -> u32 {
        u32::from(self.stable.slots())
    }

    fn stable_pets(&self) -> u32 {
        self.stable.pet_count() as u32
    }

    fn selected_stable_pet(&self) -> i32 {
        i32::from(self.stable.selected())
    }

    fn stable_pet_info(&self, panel_slot: u8) -> Option<StableLine> {
        let pet = self.stable.pet(panel_slot)?;
        let tables = self.tables.as_deref();
        let family = self.stable_family(pet.entry);
        Some(StableLine {
            icon: family
                .as_ref()
                .map(|f| f.icon.clone())
                .unwrap_or_default(),
            name: pet.name.clone(),
            level: pet.level,
            family: family.map(|f| f.name.clone()).unwrap_or_default(),
            loyalty: tables
                .and_then(|t| t.pet().loyalty(pet.loyalty))
                .unwrap_or_default()
                .to_string(),
        })
    }

    fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String> {
        let Some(pet) = self.stable.pet(panel_slot) else {
            return Vec::new();
        };
        let Some(family) = self.stable_family_id(pet.entry) else {
            return Vec::new();
        };
        self.tables.as_deref().map_or_else(Vec::new, |tables| {
            tables
                .pet()
                .foods_for(family)
                .into_iter()
                .map(str::to_string)
                .collect()
        })
    }

    fn next_stable_slot_cost(&self) -> u32 {
        self.tables
            .as_deref()
            .map_or(0, |tables| tables.pet().stable_slot_cost(self.stable_slots()))
    }

    /// **Both stalls now**: the current pet by its token, and a stabled one by
    /// its display id — see the module note.
    fn stable_paperdoll_unit(&self) -> Option<String> {
        let selected = self.selected_stable_pet();
        if selected == 0 {
            // Stall 0 is the pet that is *out*, and it is a unit like any other.
            return self
                .units
                .exists(crate::interface::api::UnitId::Pet)
                .then(|| "pet".to_string());
        }
        // …and every other stall is a creature that is not in the world, which
        // the paper-doll pass reaches by display id alone.
        let entry = self.stable_pet_entry(u8::try_from(selected).ok()?)?;
        let display = self.stable_display_id(entry)?;
        Some(format!(
            "{}{display}",
            crate::render::paperdoll::DISPLAY_ID_PREFIX
        ))
    }
}

impl super::super::api::Live<'_, '_, '_> {
    /// The pet family id behind a creature entry, out of the template cache the
    /// stable window filled — `None` while the query is in flight.
    ///
    /// **Read-only**: the *asking* is [`crate::interface::stable`]'s, which is
    /// the one place that knows the window opened. A read that queued its own
    /// query would re-queue it from inside a Lua call every time the panel
    /// redrew.
    fn stable_family_id(&self, entry: u32) -> Option<u32> {
        let world = self.world.as_ref()?;
        let world = world.lock().ok()?;
        let family = world.creatures.get(&entry)?.pet_family;
        (family != 0).then_some(family)
    }

    fn stable_family(&self, entry: u32) -> Option<vale_assets::tables::pet::Family> {
        let id = self.stable_family_id(entry)?;
        self.tables.as_deref()?.pet().family(id).cloned()
    }

    /// The creature entry in a stall, or `None` for an empty one.
    fn stable_pet_entry(&self, panel_slot: u8) -> Option<u32> {
        self.stable.pet(panel_slot).map(|pet| pet.entry)
    }

    /// …and the display id behind it, out of the same template cache
    /// [`Self::stable_family_id`] reads — `None` while the query is in flight,
    /// which draws an empty rectangle for a frame or two rather than the wrong
    /// creature.
    fn stable_display_id(&self, entry: u32) -> Option<u32> {
        let world = self.world.as_ref()?;
        let world = world.lock().ok()?;
        let display = world.creatures.get(&entry)?.display_id;
        (display != 0).then_some(display)
    }
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    // **The panel's slot ids are 0..2 and its loop runs `1..NUM_PET_STABLE_SLOTS`
    // as well as passing a literal 0**, so a negative or oversized id is a
    // question about a stall that does not exist rather than an error.
    let slot = |n: Option<i64>| -> Option<u8> { u8::try_from(n?).ok() };

    globals.set(
        "GetNumStableSlots",
        scope.create_function(move |_, ()| Ok(answers.stable_slots()))?,
    )?;
    globals.set(
        "GetNumStablePets",
        scope.create_function(move |_, ()| Ok(answers.stable_pets()))?,
    )?;
    globals.set(
        "GetSelectedStablePet",
        scope.create_function(move |_, ()| Ok(answers.selected_stable_pet()))?,
    )?;
    // **Five values, and the panel unpacks all five in one statement.** A
    // shorter answer leaves `family` nil and the label concatenation below it
    // is then an error rather than a blank word — see the module note.
    globals.set(
        "GetStablePetInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let Some(line) = slot(n).and_then(|i| answers.stable_pet_info(i)) else {
                return Ok((None, None, None, None, None));
            };
            Ok((
                Some(line.icon),
                Some(line.name),
                Some(line.level),
                Some(line.family),
                Some(line.loyalty),
            ))
        })?,
    )?;
    // A vararg, like `GetPetFoodTypes` beside it: `BuildListString` walks
    // `arg.n`, so an empty diet must be *no values* rather than one nil.
    globals.set(
        "GetStablePetFoodTypes",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(mlua::Variadic::from(
                slot(n)
                    .map(|i| answers.stable_pet_food_types(i))
                    .unwrap_or_default(),
            ))
        })?,
    )?;
    globals.set(
        "GetNextStableSlotCost",
        scope.create_function(move |_, ()| Ok(answers.next_stable_slot_cost()))?,
    )?;
    // **Scoped rather than recorded**, because it changes a frame and the
    // caller draws that frame on its very next line: `PetStable_Update` calls
    // this and then `PetStableModel:Show()`. The same argument
    // `SelectTrainerService` is in [`super::trainer::install`] for.
    globals.set(
        "SetPetStablePaperdoll",
        scope.create_function(move |lua, frame: Option<mlua::Table>| {
            let Some(frame) = frame else { return Ok(()) };
            let subject = answers.stable_paperdoll_unit();
            super::super::widgets::model::point_at_unit(lua, &frame, subject.as_deref())
        })?,
    )?;
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::stable::StablePress>>>;

/// Register the four writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::stable::StablePress as P;
    let globals = lua.globals();
    let slot = |n: Option<i64>| -> Option<u8> { u8::try_from(n?).ok() };

    // **`ClickStablePet` answers 1 while a stable is open** — see the module
    // note, which is about the one press that sends no packet.
    let click = {
        let queue = std::rc::Rc::clone(queue);
        lua.create_function(move |_, n: Option<i64>| {
            let Some(id) = slot(n) else { return Ok(None) };
            queue.borrow_mut().push(P::Click(id));
            Ok(Some(1))
        })?
    };
    globals.set("ClickStablePet", click)?;

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = std::rc::Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(press) = $body {
                    queue.borrow_mut().push(press);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }
    push!("PickupStablePet", Option<i64>, |n| slot(n).map(P::Pickup));
    push!("BuyStableSlot", Option<i64>, |_a| Some(P::BuySlot));
    push!("ClosePetStables", Option<i64>, |_a| Some(P::Close));
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The read list is sorted and each name is registered exactly once — the
    /// same shape every panel's list is checked in, and what keeps
    /// `vale framexml`'s count honest.
    #[test]
    fn the_read_list_is_sorted_and_unique() {
        let mut sorted = super::READS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.as_slice(), super::READS.as_slice());
    }
}
