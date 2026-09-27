//! **The C functions `TradeFrame.lua` and the `TRADE` popup call** — six
//! reads, ten writes.
//!
//! ```text
//! GetTradePlayerItemInfo(i)   name, texture, count, usable, enchant   (our side)
//! GetTradeTargetItemInfo(i)   name, texture, count, quality, usable, enchant
//! GetTradePlayerItemLink(i)   GetTradeTargetItemLink(i)
//! GetPlayerTradeMoney()       GetTargetTradeMoney()
//!
//! InitiateTrade(unit)   BeginTrade()   CancelTrade()   CloseTrade()
//! AcceptTrade()   CancelTradeAccept()   ClickTradeButton(i)
//! ClickTargetTradeButton(i)   SetTradeMoney(copper)   AddTradeMoney()
//! ```
//!
//! The split every panel keeps: the wire is [`vale_protocol::play::trade`],
//! the window is [`crate::interface::trade`], and this file is
//! registration and arguments.
//!
//! ## Both `*ItemInfo` reads are off the server's own statement
//!
//! Neither side's offer is modelled locally — see the window's note — so
//! `GetTradePlayerItemInfo` reads the echo of our offer exactly as
//! `GetTradeTargetItemInfo` reads the partner's. The name and quality come
//! from the item template, which for a partner's item is a
//! `CMSG_ITEM_QUERY_SINGLE` the handler asked for as the offer arrived; until
//! it lands the row draws its icon (the display id is in the packet) with no
//! name, and `TRADE_TARGET_ITEM_CHANGED` is not re-raised — the reference
//! has the same round trip and draws the same blank.
//!
//! ## Two of the ten are answered and do nothing
//!
//! `ClickTargetTradeButton` is the far side's square, which is a link and a
//! plate and never a packet. `AddTradeMoney` puts *cursor* money in the box,
//! and this client has no cursor money (`GetCursorMoney` is a stub answering
//! 0), so there is nothing to add.

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 6] = [
    "GetPlayerTradeMoney",
    "GetTargetTradeMoney",
    "GetTradePlayerItemInfo",
    "GetTradePlayerItemLink",
    "GetTradeTargetItemInfo",
    "GetTradeTargetItemLink",
];

/// One square, as either read returns it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradeLine {
    pub entry: u32,
    /// Empty until the template lands — see the module note.
    pub name: String,
    pub texture: Option<String>,
    pub count: u32,
    /// -1 until the template lands, as `GetInventoryItemQuality` answers.
    pub quality: i32,
    /// The spell aimed at the non-traded square, in words, or `None`.
    pub enchant: Option<String>,
    /// **Whether the character may hold this**, which is what decides the red
    /// square — see [`crate::world::proficiency`], which is the only
    /// thing that can answer it and is one packet per item class.
    ///
    /// `true` for everything the server has said nothing about, which is every
    /// class but weapons and armour and is `Proficiencies::allows`' own rule; it
    /// is also `true` until the template lands, because the class is on the
    /// template and refusing what has not arrived would flash every square red
    /// for a round trip.
    pub usable: bool,
}

/// **What the trade panel may ask.** Every method has a default that is a
/// shut window, which is what the headless harnesses see.
pub trait TradeAnswers {
    /// One square of one side, 1..7, or `None` for an empty one.
    fn trade_item(&self, theirs: bool, id: u8) -> Option<TradeLine> {
        let _ = (theirs, id);
        None
    }
    /// `GetPlayerTradeMoney()` / `GetTargetTradeMoney()`, in copper.
    fn trade_money(&self, theirs: bool) -> u32 {
        let _ = theirs;
        0
    }
}

impl TradeAnswers for super::super::api::Live<'_, '_, '_> {
    fn trade_item(&self, theirs: bool, id: u8) -> Option<TradeLine> {
        let offer = self.trade.offer(theirs)?;
        let item = offer.items.get(usize::from(id.checked_sub(1)?))?.as_ref()?;
        let template = self.session_template(item.entry);
        let enchant = (usize::from(id) == vale_protocol::play::trade::TRADE_SLOT_COUNT
            && offer.spell != 0)
            .then(|| {
                self.tables
                    .as_deref()
                    .and_then(|t| t.spellbook())
                    .and_then(|book| book.info(offer.spell))
                    .map(|spell| spell.name)
            })
            .flatten();
        // **Asked of the template, and permissive without one** — see the
        // field, and `Proficiencies::allows`, whose own note is that a class
        // the server has never spoken about is allowed.
        let usable = template.as_ref().is_none_or(|t| {
            self.proficiency.0.allows(t.class as u8, t.subclass as u8)
        });
        Some(TradeLine {
            entry: item.entry,
            name: template.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
            texture: self
                .tables
                .as_ref()
                .and_then(|t| t.item_icon(item.display_id)),
            count: item.count,
            quality: template.map_or(-1, |t| t.quality as i32),
            enchant,
            usable,
        })
    }

    fn trade_money(&self, theirs: bool) -> u32 {
        self.trade.offer(theirs).map_or(0, |offer| offer.money)
    }
}

pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let slot = |n: Option<i64>| -> Option<u8> { u8::try_from(n?).ok() };

    // `local name, texture, numItems, isUsable, enchantment = GetTradePlayerItemInfo(id)`.
    // Five nils for an empty square: `TradeFrame_UpdatePlayerItem` writes the
    // name and the texture straight into the button, and `SetItemButtonCount`
    // treats a nil count as none.
    globals.set(
        "GetTradePlayerItemInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let Some(line) = slot(n).and_then(|i| answers.trade_item(false, i)) else {
                return Ok((None, None, None, None, None));
            };
            Ok((
                Some(line.name),
                line.texture,
                Some(line.count),
                Some(i64::from(line.usable)),
                line.enchant,
            ))
        })?,
    )?;
    // …and the far side's, with the quality between the count and the usable
    // flag. **Usable is the proficiency answer**, which is what
    // `TradeFrame_UpdateTargetItem` colours the square by — see
    // [`TradeLine::usable`]. It is not the *whole* of the reference's test: a
    // level or a class requirement also greys a square, and those want the
    // requirement lines the item plate already composes and are owed beside
    // them. Proficiency is the half that is on the wire and in no file.
    globals.set(
        "GetTradeTargetItemInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let Some(line) = slot(n).and_then(|i| answers.trade_item(true, i)) else {
                return Ok((None, None, None, None, None, None));
            };
            Ok((
                Some(line.name),
                line.texture,
                Some(line.count),
                Some(line.quality),
                Some(i64::from(line.usable)),
                line.enchant,
            ))
        })?,
    )?;
    for (name, theirs) in [("GetTradePlayerItemLink", false), ("GetTradeTargetItemLink", true)] {
        globals.set(
            name,
            scope.create_function(move |_, n: Option<i64>| {
                Ok(slot(n)
                    .and_then(|i| answers.trade_item(theirs, i))
                    .filter(|line| !line.name.is_empty())
                    .map(|line| {
                        super::container::item_link(line.entry, line.quality.max(0) as u32, &line.name)
                    }))
            })?,
        )?;
    }
    globals.set(
        "GetPlayerTradeMoney",
        scope.create_function(move |_, ()| Ok(answers.trade_money(false)))?,
    )?;
    globals.set(
        "GetTargetTradeMoney",
        scope.create_function(move |_, ()| Ok(answers.trade_money(true)))?,
    )?;
    Ok(())
}

pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::trade::TradePress>>>;

pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::trade::TradePress as P;
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
    push!("InitiateTrade", Option<String>, |unit| unit
        .filter(|u| !u.is_empty())
        .map(P::Initiate));
    push!("BeginTrade", Option<i64>, |_a| Some(P::Begin));
    push!("CancelTrade", Option<i64>, |_a| Some(P::Cancel));
    push!("CloseTrade", Option<i64>, |_a| Some(P::Close));
    push!("AcceptTrade", Option<i64>, |_a| Some(P::Accept));
    push!("CancelTradeAccept", Option<i64>, |_a| Some(P::Unaccept));
    push!("ClickTradeButton", Option<i64>, |n| n
        .and_then(|n| u8::try_from(n).ok())
        .map(P::ClickSlot));
    push!("SetTradeMoney", Option<f64>, |copper| copper
        .map(|c| P::SetMoney(c.max(0.0) as u32)));
    // **The partner's square, which is the enchant door** — see the window's
    // own arm, and the module note above it. The button's number is deliberately
    // dropped: the client reads the spell cursor before it reads the number.
    push!("ClickTargetTradeButton", Option<i64>, |_a| Some(P::ClickTheirSlot));
    // …and the one that does nothing — see the module note.
    push!("AddTradeMoney", Option<i64>, |_a| None::<P>);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_read_list_is_sorted_and_unique() {
        let mut sorted = super::READS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.as_slice(), super::READS.as_slice());
    }

    /// **The ten verbs land on the queue with their arguments**, and the one
    /// that does nothing puts nothing on it.
    #[test]
    fn the_verbs_record_what_the_panel_pressed() {
        use crate::interface::trade::TradePress as P;
        let lua = mlua::Lua::new();
        let queue: super::Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        super::register(&lua, &queue).expect("registers");
        lua.load(
            r#"InitiateTrade("target"); BeginTrade(); ClickTradeButton(3); SetTradeMoney(1234);
               ClickTargetTradeButton(2); AddTradeMoney(); AcceptTrade(); CancelTradeAccept();
               CloseTrade(); CancelTrade(); InitiateTrade("")"#,
        )
        .exec()
        .expect("runs");
        assert_eq!(
            *queue.borrow(),
            vec![
                P::Initiate("target".into()),
                P::Begin,
                P::ClickSlot(3),
                P::SetMoney(1234),
                // **The partner's square records regardless of its number**,
                // which is the reference's own reading order — see the push.
                P::ClickTheirSlot,
                P::Accept,
                P::Unaccept,
                P::Close,
                P::Cancel,
            ]
        );
    }
}
