//! **The C functions `TaxiFrame.lua` calls** — twelve reads and three writes.
//!
//! ```text
//! NumTaxiNodes()               how many buttons the frame needs
//! TaxiNodeName(i)              TaxiNodePosition(i)   x, y in 0..1
//! TaxiNodeGetType(i)           CURRENT | REACHABLE | DISTANT | NONE
//! TaxiNodeCost(i)              GetNumRoutes(i)       hops in the route
//! TaxiGetSrcX(i, hop)   TaxiGetSrcY(i, hop)
//! TaxiGetDestX(i, hop)  TaxiGetDestY(i, hop)
//! SetTaxiMap(texture)          paint the continent's parchment onto a region
//!
//! TaxiNodeSetCurrent(i)  TakeTaxiNode(i)  CloseTaxiMap()  UnitOnTaxi(unit)
//! ```
//!
//! The split every panel keeps: the frame is `Interface\FrameXML\TaxiFrame.xml`,
//! the layout and the routing are [`vale_assets::tables::taxi`], the window is
//! [`crate::game::npc::taxi`], and this file is registration and arguments.
//!
//! ## Every index is one-based and the *hop* index is too
//!
//! `TaxiFrame_OnEvent` loops `for index = 1, num_nodes` and
//! `TaxiNodeOnButtonEnter` loops `for i = 1, NUM_TAXI_ROUTES` passing that `i`
//! as the second argument. Both are decremented on the way in, so a zero is
//! out of range at both ends
//! rather than meaning "the first".
//!
//! ## The four position reads answer a **number** for a bad index, not nil
//!
//! `TaxiNodeOnButtonEnter` multiplies each of them by the map's width before it
//! looks at anything, so a nil is `attempt to perform arithmetic on a nil value`
//! inside an `OnEnter` — which kills the tooltip for every node on the map. The
//! reference answers `0.0`, and so does this.
//!
//! ## `SetTaxiMap` is a write onto a region, and the only one in this family
//!
//! It takes the `<Texture>` and paints `Interface\TaxiFrame\TAXIMAP<map>.blp`
//! onto it — so it is shaped like [`super::super::api::portrait`]'s `SetPortraitTexture`
//! rather than like a read, and it is registered in the *scoped* half because
//! the map id it needs is the open window's.

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
///
/// `SetTaxiMap` is in this list and it is a write — the same exception
/// [`super::trainer::READS`] makes for `SelectTrainerService`, and for a
/// related reason: it needs the world at the moment of the call, so it cannot
/// be recorded and drained a system later.
pub const READS: [&str; 12] = [
    "GetNumRoutes",
    "NumTaxiNodes",
    "SetTaxiMap",
    "TaxiGetDestX",
    "TaxiGetDestY",
    "TaxiGetSrcX",
    "TaxiGetSrcY",
    "TaxiNodeCost",
    "TaxiNodeGetType",
    "TaxiNodeName",
    "TaxiNodePosition",
    "UnitOnTaxi",
];

/// One drawn node, as the frame reads it — every accessor's answer in one
/// shape, so the board is walked once per row rather than once per question.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaxiNodeLine {
    pub name: String,
    /// The literal `TaxiButtonTypes` is indexed with.
    pub kind: &'static str,
    pub at: [f32; 2],
    pub cost: u32,
    pub hops: usize,
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let index = |n: Option<i64>| -> Option<usize> { usize::try_from(n?).ok().filter(|i| *i > 0) };
    let line = move |n: Option<i64>| index(n).and_then(|i| answers.taxi_node(i));

    globals.set(
        "NumTaxiNodes",
        scope.create_function(move |_, ()| Ok(answers.taxi_nodes()))?,
    )?;
    globals.set(
        "TaxiNodeName",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(line(n).map(|row| row.name).unwrap_or_default())
        })?,
    )?;
    globals.set(
        "TaxiNodePosition",
        scope.create_function(move |_, n: Option<i64>| {
            let at = line(n).map_or([0.0, 0.0], |row| row.at);
            Ok((at[0], at[1]))
        })?,
    )?;
    globals.set(
        "TaxiNodeGetType",
        scope.create_function(move |_, n: Option<i64>| {
            // **`"NONE"` for a row that does not exist**, which is what the
            // frame hides a button for — and the only answer that keeps
            // `TaxiButtonTypes[type].file` from indexing nil.
            Ok(line(n).map_or("NONE", |row| row.kind))
        })?,
    )?;
    globals.set(
        "TaxiNodeCost",
        scope.create_function(move |_, n: Option<i64>| Ok(line(n).map_or(0, |row| row.cost)))?,
    )?;
    globals.set(
        "GetNumRoutes",
        scope.create_function(move |_, n: Option<i64>| Ok(line(n).map_or(0, |row| row.hops)))?,
    )?;
    // The four line ends, which differ only in which of the two points and
    // which of its two axes they answer.
    for (name, source, axis) in [
        ("TaxiGetSrcX", true, 0),
        ("TaxiGetSrcY", true, 1),
        ("TaxiGetDestX", false, 0),
        ("TaxiGetDestY", false, 1),
    ] {
        globals.set(
            name,
            scope.create_function(move |_, (n, hop): (Option<i64>, Option<i64>)| {
                let ends = index(n).zip(index(hop)).and_then(|(row, hop)| {
                    answers.taxi_hop(row, hop)
                });
                // See the module note: a number for a bad index, never nil.
                Ok(ends.map_or(0.0, |(src, dest)| {
                    if source { src[axis] } else { dest[axis] }
                }))
            })?,
        )?;
    }
    // **Not about the window at all** — see [`TaxiAnswers::unit_on_taxi`]. It
    // is here rather than among the unit reads because its subject is this
    // one, and `UIParent.lua` asks it about the player while no map is open:
    // being on a flight is what suppresses the "you cannot do that now"
    // dialogs on the escape menu.
    globals.set(
        "UnitOnTaxi",
        scope.create_function(move |_, token: Option<String>| {
            Ok(super::super::api::one_or_nil(
                answers.unit_on_taxi(&token.unwrap_or_default()),
            ))
        })?,
    )?;
    // **The one write in the scoped half.** `TaxiFrame_OnEvent` calls it before
    // it places any button, so a window whose art this does not paint draws its
    // nodes over the frame's own bare paper.
    globals.set(
        "SetTaxiMap",
        scope.create_function(move |lua, target: mlua::Value| {
            // A region or its name, the same two shapes `SetPortraitTexture`
            // takes — and neither is an error, because this runs inside an
            // `OnEvent`.
            let region = match target {
                mlua::Value::Table(region) => Some(region),
                mlua::Value::String(name) => lua.globals().get(name.to_string_lossy())?,
                _ => None,
            };
            let (Some(region), Some(art)) = (region, answers.taxi_map_art()) else {
                return Ok(());
            };
            super::super::widgets::regions::set_texture_path(&region, Some(&art))
        })?,
    )?;
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::game::npc::taxi::TaxiPress>>>;

/// Register the three writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::game::npc::taxi::TaxiPress as P;
    let globals = lua.globals();
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
    let index = |n: Option<i64>| usize::try_from(n?).ok().filter(|i| *i > 0);
    push!("TakeTaxiNode", Option<i64>, |n| index(n).map(P::Take));
    // **`TaxiNodeSetCurrent` records and changes nothing observable**, which is
    // not a stub: the client rebuilds the line list of the node it names, and
    // that list is derived on demand here ([`vale_assets::tables::taxi::Board::hop`])
    // rather than cached — so there is nothing for it to rebuild. It is kept
    // because `TaxiNodeOnButtonEnter` calls it before it reads the lines, and a
    // missing global there aborts the tooltip.
    push!("TaxiNodeSetCurrent", Option<i64>, |n| index(n)
        .map(P::SetCurrent));
    push!("CloseTaxiMap", Option<i64>, |_a| Some(P::Close));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// The whole read surface against the stub board, in the shapes the frame
    /// uses them in.
    #[test]
    fn the_reads_answer_the_shapes_the_frame_indexes_with() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return NumTaxiNodes()"), "Integer(2)");
        assert_eq!(
            eval(&world, "return TaxiNodeName(1)"),
            r#"String("Stormwind")"#
        );
        assert_eq!(
            eval(&world, "return TaxiNodeGetType(1) .. TaxiNodeGetType(2)"),
            r#"String("CURRENTREACHABLE")"#
        );
        // `TaxiFrame_OnEvent` multiplies both by the map's size.
        assert_eq!(
            eval(&world, "local x, y = TaxiNodePosition(2); return x * 100"),
            "Integer(50)"
        );
        assert_eq!(eval(&world, "return TaxiNodeCost(2)"), "Integer(110)");
        assert_eq!(eval(&world, "return GetNumRoutes(2)"), "Integer(1)");
    }

    /// **A row past the end is a number and a word, never nil** — the tooltip
    /// and the button loop both do arithmetic on the answer.
    #[test]
    fn a_bad_index_answers_zero_and_none() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return TaxiNodeGetType(99)"), r#"String("NONE")"#);
        assert_eq!(eval(&world, "return TaxiNodeCost(99)"), "Integer(0)");
        assert_eq!(eval(&world, "return GetNumRoutes(0)"), "Integer(0)");
        // Zero and not nil: 1.12's Lua has one number type, and what the frame
        // needs is that the arithmetic in `TaxiNodeOnButtonEnter` works at all.
        assert_eq!(eval(&world, "return TaxiGetSrcX(99, 1)"), "Integer(0)");
        assert_eq!(
            eval(&world, "return TaxiGetDestY(2, 0)"),
            "Integer(0)",
            "the hop index is one-based too"
        );
        assert_eq!(
            eval(&world, "local x, y = TaxiNodePosition(99); return x + y"),
            "Integer(0)"
        );
    }

    /// The four line ends pick the two points and the two axes apart — a
    /// transposition here draws every route as a diagonal to the corner.
    #[test]
    fn the_four_line_ends_are_two_points_and_two_axes() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return TaxiGetSrcX(2, 1)"), "Number(0.25)");
        assert_eq!(eval(&world, "return TaxiGetSrcY(2, 1)"), "Number(0.75)");
        assert_eq!(eval(&world, "return TaxiGetDestX(2, 1)"), "Number(0.5)");
        assert_eq!(eval(&world, "return TaxiGetDestY(2, 1)"), "Number(0.125)");
    }

    /// `SetTaxiMap` paints the continent's parchment onto the region it is
    /// handed — by object or by name, like every other region write.
    ///
    /// Its own harness rather than [`eval`], because it is the one read in this
    /// file that needs the **object model**: `api::tests::eval` installs the
    /// reads onto a bare state and `CreateFrame` is not one of them.
    #[test]
    fn set_taxi_map_paints_the_continents_parchment() {
        let world = Stub::default();
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        lua.load(r#"f = CreateFrame("Frame"); map = f:CreateTexture("TaxiMap")"#)
            .exec()
            .expect("a region to paint");
        let painted: String = lua
            .scope(|scope| {
                install(&lua, scope, &world)?;
                // Both shapes the first argument may take, in one chunk: the
                // region itself and its global name.
                lua.load("SetTaxiMap(map); SetTaxiMap('TaxiMap'); return map:GetTexture()")
                    .eval()
            })
            .expect("the chunk runs");
        assert_eq!(painted, vale_assets::tables::taxi::map_art(0));
    }

    /// The writes record one-based and drop nonsense.
    #[test]
    fn the_writes_record_in_call_order() {
        use crate::game::npc::taxi::TaxiPress as P;
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load("TaxiNodeSetCurrent(3); TakeTaxiNode(3); TakeTaxiNode(0); CloseTaxiMap()")
            .exec()
            .expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![P::SetCurrent(3), P::Take(3), P::Close]
        );
    }
}

/// **What the interface may ask about the flight map.**
///
/// Beside its own registration, like every other subject trait in this
/// directory — see [`super::super::api::Answers`], which is the sum of them.
pub trait TaxiAnswers {
    /// `NumTaxiNodes()` — how many buttons the frame needs.
    fn taxi_nodes(&self) -> usize;
    /// Everything about one drawn node, one-based.
    fn taxi_node(&self, row: usize) -> Option<TaxiNodeLine>;
    /// The endpoints of one hop of a node's route, both one-based, each in
    /// `0..1` across the map art.
    fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])>;
    /// `SetTaxiMap`'s file — `None` with no window open.
    fn taxi_map_art(&self) -> Option<String>;
    /// `UnitOnTaxi(unit)` — is this unit in the air on a flight path?
    fn unit_on_taxi(&self, token: &str) -> bool;
}

impl TaxiAnswers for super::super::api::Live<'_, '_, '_> {
    fn taxi_nodes(&self) -> usize {
        self.taxi.board().map_or(0, |board| board.rows.len())
    }

    fn taxi_node(&self, row: usize) -> Option<TaxiNodeLine> {
        let board = self.taxi.board()?;
        let node = board.row(row)?;
        Some(TaxiNodeLine {
            name: node.name.clone(),
            kind: node.kind.word(),
            at: node.at,
            cost: board.cost(row),
            hops: board.hops(row),
        })
    }

    fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
        self.taxi.board()?.hop(row, hop)
    }

    fn taxi_map_art(&self) -> Option<String> {
        self.taxi.map_art()
    }

    fn unit_on_taxi(&self, token: &str) -> bool {
        // **Off the unit's own flags and not off the open window**, which is
        // the difference that matters: a character who logged out mid-flight
        // and back in is on a taxi and has never seen a taxi map. See
        // [`crate::game::npc::taxi::on_taxi`].
        super::super::api::Live::id(token)
            .and_then(|id| self.units.get(id))
            .is_some_and(crate::game::npc::taxi::on_taxi)
    }
}
