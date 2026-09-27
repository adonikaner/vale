//! **Right-clicking something that is not a person** — the door, the chest, the
//! ore vein, the mailbox, the lever.
//!
//! A game object is the one population in the world that this client could
//! *draw* in full and could not touch at all: the pick in
//! [`crate::interface::target::hover`] skipped it outright with the comment
//! "a game object is usable and never selectable", and nothing ever made it
//! usable. So the Deadmines doors were solid, correctly posed, correctly lit —
//! and shut for ever.
//!
//! ## It is beside the NPC verbs because it is one of them
//!
//! This directory is "everything a right-click on somebody else opens", and a
//! chest opens the very loot window [`super::loot`] already draws, from the very
//! same `SMSG_LOOT_RESPONSE`. What changes is only who was clicked. The
//! directory's own note has been widened to say *something* as well as
//! *somebody*, which is the whole of the difference.
//!
//! ## A game object is hovered but never *selected*
//!
//! That distinction is why this carries its own [`HoveredObject`] rather than
//! widening [`crate::interface::target::Hovered`]: half a dozen things read
//! that resource as a unit — the tooltip's `SetUnit("mouseover")`, the spell
//! cursor's validity, `api::Units`' `"mouseover"` token — and a chest arriving
//! in it would be asked for its health. The **pick is still one walk**, and the
//! two resources partition its answer: at most one of them is filled on any
//! frame, and which one it is comes out of the depth test rather than out of a
//! precedence rule, so a mob standing in front of a mailbox is the thing the
//! click lands on.
//!
//! ## …and a chest is not opened by the packet that opens a door
//!
//! `CMSG_GAMEOBJ_USE` is the whole of the verb for a door, a lever, a chair, a
//! mailbox and a goober — and it does **nothing at all** for a chest, an ore
//! vein or a herb. Those are opened by *casting* at the object, and which spell
//! is decided by its lock. The rule and the server code behind it are in
//! `vale_assets::look::object`; [`use_the_object`] is the two-way switch.
//!
//! That is not a reading. `CMSG_GAMEOBJ_USE` was sent at a live herb node by a
//! character with 300 Herbalism standing on top of it, and nothing came back —
//! no loot, no state change, no refusal.
//!
//! ## …and a street sign is a game object that can only be looked at
//!
//! `GAMEOBJECT_TYPE_GENERIC` — the signposts, the plaques, the markers — is
//! refused by `HandleGameObjectUseOpcode` before any of its other checks, so
//! nothing can be *done* to one. What it has instead is a `floatingTooltip`
//! flag, and the plate it asks for **follows the pointer** rather than sitting
//! in the corner of the screen where a unit's goes.
//!
//! So [`HoveredObject`] answers two questions and not one:
//! [`HoveredObject::usable`] is whether a click sends anything, and
//! [`HoveredObject::hover`] is whether the pointer has something to say. The
//! first draft of this module had only the first, and the signs were filtered
//! out of the pick as scenery — which 468 of the 1,869 generics genuinely are,
//! and 1,197 of them are not.
//!
//! ## Nothing is predicted and there is nothing to wait for
//!
//! `CMSG_GAMEOBJ_USE` has no reply — see [`vale_protocol::play::object`],
//! where the four different paths an answer can come back down are listed. This
//! module sends and forgets; a door swings because `GAMEOBJECT_STATE` changed in
//! an update block, a vein opens a loot window because the server cast the
//! gathering spell on our behalf, and a locked anything says so through the
//! message table.

use bevy::prelude::*;

use crate::world::session::{ActiveSession, Session, WorldEntity};
use vale_assets::look::object::{self as object, Kind};

/// **The game object under the pointer this frame**, or nothing.
///
/// Written by [`crate::interface::target::hover`], which does the one ray
/// walk both this and `Hovered` are the answer to — see the module note for why
/// they are two resources and not one.
#[derive(Resource, Default)]
pub struct HoveredObject {
    pub guid: Option<u64>,
    pub entity: Option<Entity>,
    /// **Whether a click here would be sent at all**, which is
    /// [`Kind::usable`]'s answer — the great majority of game objects in the
    /// world are invisible zone markers and scenery.
    ///
    /// Beside the guid rather than re-derived at the click, on the same terms as
    /// `Hovered::attackable`: what the pointer says and what the button does are
    /// one answer, computed once a frame.
    pub usable: bool,
    /// …and which of the game's pointers belongs over it — the pick over an ore
    /// vein, the leaf over a herb, the envelope over a mailbox. `None` is the
    /// arrow, and it is also what an unusable object gets.
    pub cursor: Option<vale_assets::look::cursor::Cursor>,
    /// **What the plate calls it** — the template's own name, which is the
    /// whole of a game object's tooltip in 1.12.
    ///
    /// Empty until `CMSG_GAMEOBJECT_QUERY` comes back, which is the same round
    /// trip a creature's name costs and is handled the same way: the plate is
    /// raised on the change, and the change includes the name arriving.
    pub name: String,
    /// …and **the two lines under the name**, already judged — see
    /// [`LockPlate`].
    pub plate: LockPlate,
    /// **The spell a click would cast at it**, for the two types that are opened
    /// that way — see [`vale_assets::look::object::opener`]. `None` for
    /// everything a click merely *uses*, which is every other type.
    ///
    /// Resolved on the hover rather than at the press for the reason the cursor
    /// is: the answer needs the whole spell catalogue and the character's own
    /// book, and the click must not be the thing that goes looking.
    pub cast: Option<u32>,
    /// **What hovering it is worth**, which is not what clicking it is worth —
    /// see [`vale_assets::look::object::hover_of`]. `floating` puts the plate
    /// at the pointer instead of the screen's corner, and it is the whole of
    /// what makes a street sign different; `highlight` lights the model.
    pub hover: vale_assets::look::object::Hover,
    /// **What kind of thing it is**, which is not derivable from anything else
    /// here: the two flags above say what a click is *worth*, not what it
    /// opens. Carried because one type is answered by the client rather than by
    /// the server — a mailbox opens a window with no packet behind it — and the
    /// press has to be able to tell.
    pub kind: Kind,
}

impl HoveredObject {
    /// Write this frame's judgement, comparing first.
    ///
    /// A `ResMut` dereferenced every frame marks the resource changed every
    /// frame, and the world tooltip fires on the change — so the compares are
    /// what keep the plate from being torn down and rebuilt sixty times a
    /// second. Same reason `target::judge_the_hover` compares.
    #[allow(clippy::too_many_arguments)]
    fn judged(
        &mut self,
        kind: Kind,
        usable: bool,
        cursor: Option<vale_assets::look::cursor::Cursor>,
        name: &str,
        plate: LockPlate,
        cast: Option<u32>,
        hover: vale_assets::look::object::Hover,
    ) {
        if self.hover != hover {
            self.hover = hover;
        }
        if self.kind != kind {
            self.kind = kind;
        }
        if self.usable != usable {
            self.usable = usable;
        }
        if self.cursor != cursor {
            self.cursor = cursor;
        }
        if self.name != name {
            self.name = name.to_string();
        }
        if self.plate != plate {
            self.plate = plate;
        }
        if self.cast != cast {
            self.cast = cast;
        }
    }

    /// …and the same for "the pointer is over nothing", which is every frame of
    /// a session spent looking at the ground. The guid and the entity are the
    /// pick's to write and are deliberately untouched.
    fn forget_the_judgement(&mut self) {
        self.judged(
            Kind::Generic,
            false,
            None,
            "",
            LockPlate::default(),
            None,
            Default::default(),
        );
    }

    fn clear(&mut self) {
        self.guid = None;
        self.entity = None;
        self.usable = false;
        self.cursor = None;
        self.name.clear();
        self.plate = LockPlate::default();
        self.cast = None;
        self.hover = Default::default();
        self.kind = Kind::Generic;
    }
}

/// **The two lines the plate draws under a game object's name**, judged against
/// this character rather than merely read off the table.
///
/// The reference composes both in C — 1.12 has no `GameTooltip:SetGameObject`
/// — so this is that function's output rather than a state the interface could
/// have built for itself. In the reference's own order: the word "Locked" when
/// the object's `GO_FLAG_LOCKED` bit is set, then at most one "Requires …"
/// line out of **column 0** of the lock.
///
/// **Each line carries its own colour, and that is the point of the type.**
/// Both were drawn plain before this, so a vein a miner was thirty points short
/// of read exactly like one they could mine — which is the one thing the line
/// exists to say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockPlate {
    /// `GO_FLAG_LOCKED` — the word `LOCKED` out of `GlobalStrings.lua`, in
    /// [`Self::locked_colour`].
    pub locked: bool,
    /// Red when there is no way in, the gathering-difficulty colour when the
    /// way in is a spell, green when it is a key already in the bags.
    pub locked_colour: [f32; 3],
    /// Which of `GlobalStrings.lua`'s three lock sentences the second line is,
    /// or `None` for no second line — which is the common answer.
    ///
    /// **All three read `"Requires %s"`**; what differs is the colour and
    /// which authority fills the `%s`. `LOCKED_WITH_SPELL_KNOWN` and
    /// `LOCKED_WITH_SPELL` take a `LockType.dbc` name, and `LOCKED_WITH_ITEM`
    /// takes an item's.
    pub requires_key: Option<&'static str>,
    /// …the `%s`. Empty is a line that is not drawn: an item key whose name has
    /// not come back from `CMSG_ITEM_QUERY_SINGLE` yet is skipped outright in
    /// the reference too.
    pub requires: String,
    /// …and its colour.
    pub requires_colour: [f32; 3],
}

/// **A click landed on a game object**, and what kind it was.
///
/// Raised beside the packet rather than instead of it, for the one type whose
/// window the *client* opens: a mailbox. See [`super::mail`], and
/// `vale_protocol::play::mail`'s note on why `CMSG_GAMEOBJ_USE` reaches an
/// empty arm there.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectUsed {
    pub guid: u64,
    pub kind: Kind,
}

/// The systems this module owns.
pub struct ObjectPlugin;

impl Plugin for ObjectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HoveredObject>().add_message::<ObjectUsed>().add_systems(
            Update,
            (judge_the_object, use_the_object, forget)
                .chain()
                // **After the pick has run**, which is what fills the guid this
                // reads — and after the whole target chain rather than after
                // `hover` alone, because the click below must not fire in the
                // same frame a spell cursor is taking the click instead.
                .after(super::target::TargetSet)
                .in_set(super::GameSet),
        );
    }
}

/// **What the pointer is over, judged** — is it worth clicking, and which
/// bitmap goes on the cursor.
///
/// The twin of `target::judge_the_hover` and it exists for the same reason: the
/// pointer's picture and what the button does are read off one answer, so they
/// cannot disagree.
///
/// Asked every frame rather than on the change, also for that function's reason:
/// the world moves under a still pointer. A chest is emptied, a door is opened
/// by somebody else, and — the case that actually bites — the template arrives a
/// round trip after the object does, so a vein hovered the instant it streams in
/// is an unusable nothing that becomes a mining node a frame or two later.
fn judge_the_object(
    assets: Res<crate::assets::GameAssets>,
    objects: Query<&WorldEntity>,
    // …and the character's own half of every lock question — see
    // [`Character`], which is three reads bundled because this system is at
    // the parameter limit without them.
    me: Character,
    mut hovered: ResMut<HoveredObject>,
) {
    // **Nothing under the pointer is the only early return**, and that is the
    // shape rather than the tidiness. This body used to be a `||` closure
    // returning `Option<_>` with a `?` on every fallible step, and one of those
    // steps was `kind.opened_by_spell()` — which is legitimately `false` for a
    // door, a chair, a mailbox and a lever, so `?` threw the *whole* answer away
    // for every game object in the game except a chest. The symptom was that
    // hovering did nothing at all: no cursor, no plate, no click, on everything
    // but an ore vein. A `?` is only safe where `None` means "no answer"; here
    // most of these mean "no, and that is the answer".
    let Some(object) = hovered.entity.and_then(|entity| objects.get(entity).ok()) else {
        hovered.forget_the_judgement();
        return;
    };
    let kind = Kind::of(object.object_kind);
    // **Three live fields, not the template.** The template says what a thing
    // is; these say what it is doing, and they are what separates a chest this
    // character may open from one that is somebody else's kill. See
    // [`vale_assets::look::object::interactable`].
    let locked = object.object_flags & object::go_flags::LOCKED != 0;
    let state = object.object_state.unwrap_or(0);
    let usable = object::interactable(kind, object.object_flags, object.object_dyn_flags);
    // **The tables are optional and their absence is not "unusable".**
    // Without `Lock.dbc` every object reads as unlocked, which is the plain
    // hand over an ore vein — a worse pointer, not a dead one. Deciding
    // usability off the flags above rather than off a table is what keeps that
    // true.
    let tables = assets.display_tables().ok();
    let cursor = usable
        .then(|| {
            let tables = tables.as_ref()?;
            object::over_object(kind, tables.locks(), object.object_lock)
        })
        .flatten();
    // **Only for a type that is opened that way** — so the lock on a door costs
    // nothing here and the world lock is not taken for a mailbox.
    let cast = if kind.opened_by_spell() {
        (|| {
            let tables = tables.as_ref()?;
            let catalogue = tables.spellbook()?;
            let known = me.known_spells();
            object::opener(
                kind,
                tables.locks(),
                object.object_lock,
                catalogue,
                |spell| known.contains(&spell),
            )
        })()
    } else {
        None
    };
    // **"Locked" needs the flag and nothing else**, so a missing table costs
    // the colour and the requirement line but never the word itself.
    let plate = match tables.as_ref() {
        Some(tables) => lock_plate(tables, &me, object, state, locked),
        None => LockPlate {
            locked,
            locked_colour: object::difficulty::IMPOSSIBLE,
            ..LockPlate::default()
        },
    };
    // Compared rather than written, so the change flag stays meaningful for the
    // tooltip, which fires on it.
    hovered.judged(
        kind,
        usable,
        cursor,
        &object.name,
        plate,
        cast,
        object.object_hover,
    );
}

/// **What this character brings to a lock**, bundled.
///
/// Three reads that answer one question — which open-lock spells are known,
/// what the character's rank in each is, and what is in the bags — and a system
/// taking them separately is a system past Bevy's sixteen parameters.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Character<'w, 's> {
    units: super::api::Units<'w, 's>,
    inventory: Res<'w, super::items::Inventory>,
    session: Res<'w, Session>,
}

impl Character<'_, '_> {
    /// **The spells this character knows**, copied out from under the world
    /// lock.
    ///
    /// A copy per call and deliberately not cached: the spellbook is behind the
    /// session's mutex, and holding a borrow of it across the rest of a frame
    /// is the shape `resolve_names` exists to avoid. It is asked for only when
    /// the pointer is over something with a lock, which is a handful of frames
    /// in a session.
    fn known_spells(&self) -> Vec<u32> {
        let Some(ActiveSession { live, .. }) = self.session.active.as_ref() else {
            return Vec::new();
        };
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.spellbook.known.iter().copied().collect()
    }

    /// **The spell this character would open one `LockType` with, and their
    /// rank in the skill line it belongs to** — which is the half of
    /// [`vale_assets::look::object::can_open`] that is in no file.
    ///
    /// The skill line is the *spell's* rather than the lock type's, which is
    /// the reference's own route (it goes through the spell record) and is not
    /// a detail: `LockType.dbc` row 3 is "Mining" and the **skill line** of that
    /// name is 186, two unrelated small integers one join apart.
    fn opens(
        &self,
        tables: &vale_assets::tables::dbc::DisplayTables,
        known: &[u32],
        lock_type: u32,
    ) -> Option<(u32, u32)> {
        let catalogue = tables.spellbook()?;
        let spell = catalogue
            .open_lock_spells(lock_type)
            .iter()
            .copied()
            .find(|spell| known.contains(spell))?;
        let (race, class) = self
            .units
            .race_class_ids(super::api::UnitId::Player)
            .unwrap_or((0, 0));
        // **A spell with no `SkillLineAbility` row answers line 0, and that is
        // rank 0 rather than "cannot".** `Opening` belongs to no skill line at
        // all and an `Open`-type lock wants rank 0, so the two meet at "anybody
        // may" — which is right for every quest goober in the game.
        let line = tables
            .skills()
            .map_or(0, |skills| skills.line_of(spell, race as u8, class as u8));
        let have = self
            .units
            .skills(super::api::UnitId::Player)
            .and_then(|skills| {
                skills
                    .iter()
                    .find(|skill| u32::from(skill.id) == line)
                    .map(|skill| u32::from(skill.value))
            })
            .unwrap_or(0);
        Some((spell, have))
    }
}

/// The two lines, composed — [`LockPlate`], one branch of the reference at a time.
fn lock_plate(
    tables: &vale_assets::tables::dbc::DisplayTables,
    me: &Character,
    object: &WorldEntity,
    state: u8,
    locked: bool,
) -> LockPlate {
    let locks = tables.locks();
    // **Nothing to say about a thing with no lock that is not flagged**, which
    // is the overwhelming majority of the world. Asked first so that hovering a
    // signpost costs neither a spellbook copy nor a table walk.
    if !locked && locks.first_slot(object.object_lock).is_none() {
        return LockPlate::default();
    }
    let known = me.known_spells();
    let mut plate = LockPlate {
        locked,
        // **Red unless something says otherwise** — the reference starts from
        // red before it asks, and both the other arms are overwrites.
        locked_colour: object::difficulty::IMPOSSIBLE,
        ..LockPlate::default()
    };
    if locked {
        let way = object::can_open(
            locks,
            object.object_lock,
            state,
            locked,
            object.object_level,
            |lock_type| me.opens(tables, &known, lock_type),
            |entry| me.inventory.carried.find_entry(entry).is_some(),
        );
        plate.locked_colour = match way {
            // A way in that is a spell is graded by the rank; one that is a key
            // already in the bags is flatly green.
            Some(way) if way.spell.is_some() => object::difficulty_colour(way.have, way.need),
            Some(_) => object::difficulty::EASY,
            None => object::difficulty::IMPOSSIBLE,
        };
    }
    // **Column 0 and no other** — see
    // [`vale_assets::tables::lock::Locks::first_slot`], which is where the
    // measurement and the bug it stops are written down.
    match object::requirement_of(locks, object.object_lock, state, locked) {
        Some(object::Requirement::Skill { lock_type, rank }) => {
            let Some(name) = locks.lock_type_name(lock_type) else {
                return plate;
            };
            let need = if rank == 0 {
                object.object_level.saturating_mul(5)
            } else {
                rank
            };
            match me.opens(tables, &known, lock_type) {
                // **Known: the sentence is the same and the colour is the
                // answer.** `LOCKED_WITH_SPELL_KNOWN` in the
                // gathering-difficulty ramp against this slot's own rank, which
                // is what makes the line say whether *you* can do it.
                Some((_, have)) => {
                    plate.requires = name.to_string();
                    plate.requires_key = Some("LOCKED_WITH_SPELL_KNOWN");
                    plate.requires_colour = object::difficulty_colour(have, need);
                }
                // **Unknown: flat red, and only when the word "Locked" is not
                // already on the plate.** The reference returns outright on the
                // flag, so a locked strongbox says "Locked" and stops rather
                // than saying the same thing twice in two colours.
                None => {
                    if locked {
                        return plate;
                    }
                    plate.requires = name.to_string();
                    plate.requires_key = Some("LOCKED_WITH_SPELL");
                    plate.requires_colour = REFUSED;
                }
            }
        }
        Some(object::Requirement::Item(entry)) => {
            // **A key whose name has not arrived draws no line**, which is the
            // reference's own early exit rather than a gap: the
            // name is one `CMSG_ITEM_QUERY_SINGLE` away and the plate is
            // rebuilt when it lands.
            let Some(template) = me.inventory.template(entry) else {
                return plate;
            };
            plate.requires = template.name.clone();
            plate.requires_key = Some("LOCKED_WITH_ITEM");
            plate.requires_colour = PLAIN;
        }
        None => {}
    }
    plate
}

/// **The red a requirement nothing this character has can meet is drawn in** —
/// `0xffff0000`, which is pure red and is *not*
/// [`vale_assets::look::object::difficulty::IMPOSSIBLE`]'s `0xffff2020`.
///
/// Two reds one byte apart on two lines of the same plate, and the difference
/// is which question was asked: the ramp's red is "your rank is short", and
/// this one is "there is no rank that would do".
const REFUSED: [f32; 3] = [1.0, 0.0, 0.0];

/// …and the white a key's name takes — `0xffffffff`. A key is
/// a thing you either have or have not, so there is nothing to grade.
const PLAIN: [f32; 3] = [1.0, 1.0, 1.0];


/// **Use it** — either button, on the same three gesture tests the unit click
/// makes.
///
/// **Both buttons, and that is a reading.** The right button is certain: it is
/// the one this client already routes every other world interaction through, and
/// the reference opens a door with it. The left is the reading, and the argument
/// is the pointer: a hand, a pick or an envelope is drawn over the thing, and a
/// pointer that says "do something here" and then does nothing on the primary
/// button is a pointer that lies. There is also nothing for the left button to
/// do instead — a game object cannot be selected — so the alternative is a
/// gesture that is silently inert.
///
/// **A left click over a game object must not clear the selection**, which is
/// the one thing that would otherwise happen: `select_on_click` clears on empty
/// ground, and as far as it is concerned a chest *is* empty ground. That is
/// handled where it belongs, in `target::select_on_click`, which reads this
/// resource for exactly that test.
fn use_the_object(
    buttons: Res<ButtonInput<MouseButton>>,
    look: Res<crate::world::camera::MouseLook>,
    targeting: Res<super::action::SpellTargeting>,
    hovered: Res<HoveredObject>,
    session: Res<Session>,
    mut used: MessageWriter<ObjectUsed>,
    // …and the two the reference refuses a click with before the packet — see
    // the branches at the foot of the body.
    objects: Query<&WorldEntity>,
    mut errors: super::messages::UiErrors,
) {
    // The same gesture tests both unit clicks make: the finger came up, the
    // press landed on the world, and the pointer did not travel far enough to
    // make this a camera move. Plus the fourth the right button needs — with the
    // other button also down this is autorun.
    let left = buttons.just_released(MouseButton::Left) && !buttons.pressed(MouseButton::Right);
    let right = buttons.just_released(MouseButton::Right) && !buttons.pressed(MouseButton::Left);
    if (!left && !right) || !look.on_world || look.dragged {
        return;
    }
    // **A waiting spell cursor takes the click instead**, on both buttons: the
    // left one is the cast and the right one is this client's camera. Neither
    // should open a chest.
    if targeting.is_targeting() {
        return;
    }
    let Some(guid) = hovered.guid.filter(|_| hovered.usable) else {
        return;
    };
    let Some(ActiveSession { live, .. }) = session.active.as_ref() else {
        return;
    };
    // **Two refusals the reference makes before the packet**, both about
    // what the thing is *doing* rather than what it
    // is. Neither is recoverable from the server's side: a destroyed object and
    // a locked one both answer `CMSG_GAMEOBJ_USE` with silence, which is the
    // "clicking it does nothing" half of the report.
    if let Some(object) = hovered.entity.and_then(|entity| objects.get(entity).ok()) {
        if object.object_state == Some(object::STATE_DESTROYED) {
            errors.key("ERR_USE_DESTROYED");
            return;
        }
        // **Locked, with no way in at all** — which is the plate's own "Locked"
        // line in red, said out loud. `GO_FLAG_LOCKED` is the whole of the
        // gate, so an ore vein a miner is short on is **not** refused here: a
        // vein is not flagged locked, and the server's own
        // `ERR_USE_LOCKED_WITH_SPELL_S` is a better sentence than anything this
        // side can invent. See [`LockPlate`].
        //
        // A door says so in its own words: the reference's door handler is
        // constructed with error 220 where the base class takes 219.
        if hovered.plate.locked && hovered.plate.locked_colour == object::difficulty::IMPOSSIBLE {
            errors.key(if hovered.kind == Kind::Door {
                "ERR_DOOR_LOCKED"
            } else {
                "ERR_USE_LOCKED"
            });
            return;
        }
    }
    // **Two verbs, and which one is the object's own** — see the module note.
    // A chest, an ore vein and a herb are a *cast* at the thing; everything else
    // is the use packet. `cast` is `None` for every type that is not opened that
    // way, and also for one that is when the character has no spell for its lock
    // — a rogue's lockbox with no rogue. That last case sends nothing, which is
    // the honest answer: the use packet would reach the arm that does nothing.
    match hovered.cast {
        Some(spell) => live.cast(spell, vale_protocol::play::spells::CastTarget::Object(guid)),
        None => live.use_object(guid),
    }
    // **…and the click itself, for the one kind the server has no answer to.**
    // See [`ObjectUsed`]. Raised for every kind rather than only for a mailbox,
    // because a message that is filtered by its reader stays a fact about the
    // gesture rather than a fact about mail.
    used.write(ObjectUsed {
        guid,
        kind: hovered.kind,
    });
}

/// Let go when the world does — the twin of `target::forget`, and it is needed
/// for the same reason: an `Entity` outlives the despawn that frees it, and a
/// pointer judged against a handle from the last session is a cursor drawn off a
/// stale lookup.
fn forget(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut hovered: ResMut<HoveredObject>,
) {
    if leaving.read().next().is_some() {
        hovered.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::state::update::ObjectType;

    /// One frame of [`judge_the_object`] over a single hovered game object.
    ///
    /// Deliberately built with **no archives and no session** — `GameAssets`
    /// over an empty path answers `Err` to `display_tables()` — because that is
    /// the harshest case and the one the regression below is about: everything
    /// that needs a table has to come back empty *without* taking the rest of
    /// the answer with it.
    fn judged(object_kind: u32) -> HoveredObject {
        judged_with(object_kind, 0, 0)
    }

    /// …and the same with the object's own two live flag words, which is what
    /// [`vale_assets::look::object::interactable`] reads.
    fn judged_with(object_kind: u32, object_flags: u32, object_dyn_flags: u32) -> HoveredObject {
        let mut app = App::new();
        app.init_resource::<HoveredObject>()
            .init_resource::<Session>()
            .init_resource::<crate::interface::items::Inventory>()
            // …and the five `Units` reads, none of which the judgement needs
            // unless a lock table answers — which with no archives it never does.
            .init_resource::<crate::interface::target::Selection>()
            .init_resource::<crate::interface::target::Hovered>()
            .init_resource::<crate::interface::gossip::NpcUnit>()
            .init_resource::<crate::interface::reputation::PlayerStanding>()
            .init_resource::<crate::interface::party::Party>()
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .add_systems(Update, judge_the_object);
        let entity = app
            .world_mut()
            .spawn(WorldEntity {
                guid: 0xF110_0000_0000_0001,
                kind: ObjectType::GameObject,
                name: "Mailbox".to_string(),
                object_kind,
                object_flags,
                object_dyn_flags,
                ..WorldEntity::default()
            })
            .id();
        app.world_mut().resource_mut::<HoveredObject>().entity = Some(entity);
        app.update();
        std::mem::take(&mut *app.world_mut().resource_mut::<HoveredObject>())
    }

    /// **The type is not the whole answer, and three live bits are the rest.**
    ///
    /// Every case below is a chest with an ordinary template — the same type,
    /// the same name, the same lock — so a judgement made off the type alone
    /// put the interact hand on all five and sent a packet the server drops.
    /// See [`vale_assets::look::object::interactable`].
    #[test]
    fn a_chest_the_server_has_switched_off_is_not_usable() {
        use vale_assets::look::object::{go_dyn_flags, go_flags};
        assert!(judged_with(3, 0, 0).usable, "the plain case");
        assert!(!judged_with(3, go_flags::IN_USE, 0).usable);
        assert!(!judged_with(3, go_flags::NO_INTERACT, 0).usable);
        assert!(!judged_with(3, go_flags::INTERACT_COND, 0).usable);
        assert!(judged_with(3, go_flags::INTERACT_COND, go_dyn_flags::ACTIVATE).usable);
        // …and a locked one is still clicked: the server answers in words.
        assert!(judged_with(3, go_flags::LOCKED, 0).usable);
        // **The cursor follows it**, which is the half the player sees first.
        // (Only the negative half is checkable with no archives: the pointer's
        // picture comes off `Lock.dbc`, so the positive case has no cursor here
        // either.)
        assert!(judged_with(3, go_flags::NO_INTERACT, 0).cursor.is_none());
    }

    /// **"Locked" goes on the plate off the flag and off nothing else**, and it
    /// is red when there is no way in — which with no archives is every time.
    #[test]
    fn the_locked_flag_is_the_whole_of_the_locked_line() {
        use vale_assets::look::object::{difficulty, go_flags};
        assert!(!judged_with(3, 0, 0).plate.locked);
        let locked = judged_with(3, go_flags::LOCKED, 0).plate;
        assert!(locked.locked);
        assert_eq!(locked.locked_colour, difficulty::IMPOSSIBLE);
        assert_eq!(
            locked.requires_key, None,
            "…and with the word Locked already up, the reference stops there"
        );
    }

    /// **A door is not a chest, and asking whether it is one must not throw the
    /// whole judgement away.**
    ///
    /// This is the round's own reported bug, and it deserves a test rather than
    /// a comment because the shape that caused it is so ordinary: the body was a
    /// closure returning `Option<_>` in which every fallible step was written
    /// `…?`, and one of those steps was `kind.opened_by_spell()` — which is
    /// legitimately `false` for a door, a chair, a lever and a mailbox. So
    /// `judge()` answered `None` for every game object in the game except a
    /// chest, `unwrap_or_default()` wrote `usable = false` and an empty name,
    /// and hovering did nothing at all: no cursor, no plate, no click.
    ///
    /// The assertion that catches it is **`usable`**, because it is the one
    /// answer that depends on nothing but the type — no archives, no session, no
    /// lock table. If a later edit reintroduces an early return, this is false.
    #[test]
    fn a_type_that_is_not_opened_by_a_spell_is_still_judged() {
        // 19 is `Mailbox`: usable, and emphatically not a chest.
        let mailbox = judged(19);
        assert!(
            mailbox.usable,
            "a mailbox is usable — a `?` on `opened_by_spell` is what made it not"
        );
        assert_eq!(mailbox.name, "Mailbox", "…and it is named, from the same answer");
        assert_eq!(mailbox.cast, None, "…and there is nothing to cast at it");

        // 0 is `Door` and 7 is `Chair`, on the same terms.
        for kind in [0, 1, 7, 9, 10] {
            assert!(judged(kind).usable, "kind {kind} is usable");
        }
        // …and the scenery still is not: 5 is `GENERIC`, 8 is `SPELL_FOCUS`.
        for kind in [5, 8, 11, 15] {
            assert!(!judged(kind).usable, "kind {kind} is not");
        }
    }
}
