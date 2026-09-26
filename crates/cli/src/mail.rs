//! `vale mail` — **the paper a letter is written on**, checked against the
//! archives, and the two client-side rules over it.
//!
//! Nothing about the mailbox arrives in a file except this: the inbox is a
//! packet and the letter's words are a query. What `Stationery.dbc` decides is
//! everything a *fresh* character sees in the Send tab — which paper is
//! offered, what it costs, what the letter's icon is, and what the parchment
//! behind the words is — and every one of those is a join that can be wrong
//! without failing.
//!
//! Three things are checked, and the third is the one worth the command:
//!
//! * **every background resolves in the archive chain**, both halves of it.
//!   `Interface\Stationery\<texture>1.blp` and `…2.blp` are built by string
//!   concatenation in `MailFrame.lua`, so a texture column read one field over
//!   would produce a plausible stem that names no file — and the panel draws a
//!   blank parchment rather than raising.
//! * **the flag column is `& 1` and exactly one row has the bit.** That single
//!   number is what `GetNumStationeries()` answers for a character carrying
//!   nothing, so reading the wrong column offers five rows or none, both of
//!   which look deliberate.
//! * **the postage arithmetic**, traced for each row: 30 copper plus the row's
//!   own item price when it is not carried. The item prices are not in the
//!   archives at all — `Item.dbc` is not shipped and the client learns them by
//!   `CMSG_ITEM_QUERY_SINGLE` — so what is printed is the rule with the price
//!   left as the unknown it is, which is the honest shape.
//!
//! ```text
//! vale mail          the five rows, their files, the offered rule, the postage
//! vale mail 41       …one row traced, with both background halves resolved
//! ```

use crate::common::*;
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::stationery::MailTables;
use vale_config::Config;
use vale_protocol::play::mail::{GM_STATIONERY, POSTAGE};

pub fn cmd_mail(cfg: &Config, which: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let mut read = |table: &str| assets.read(&dbc_path(table)).ok().unwrap_or_default();
    let stationery = read("Stationery");
    let package = read("Package");
    let template = read("MailTemplate");
    let tables = MailTables::parse(&stationery, &package, &template);

    if tables.stationery().is_empty() {
        return Err("Stationery.dbc named no paper at all — is it in the archives?".into());
    }

    match which {
        Some(id) => one(&mut assets, &tables, id),
        None => all(&mut assets, &tables),
    }
}

fn all(assets: &mut vale_assets::Assets, tables: &MailTables) -> Result<(), String> {
    println!(
        "Stationery.dbc: {} row(s); MailTemplate.dbc: {} row(s)",
        tables.stationery().len(),
        templates_in(tables),
    );
    println!(
        "\n  {:<5} {:<8} {:<20} {:<7} {:<9} background",
        "id", "item", "texture", "flags", "offered"
    );
    let mut missing = 0;
    for row in tables.stationery() {
        let (left, right) = row.background();
        let present = assets.read(&format!("{left}.blp")).is_ok()
            && assets.read(&format!("{right}.blp")).is_ok();
        if !present {
            missing += 1;
        }
        println!(
            "  {:<5} {:<8} {:<20} {:<7} {:<9} {}",
            row.id,
            row.item,
            row.texture,
            row.flags,
            if row.always_available() { "always" } else { "if held" },
            if present { "ok" } else { "MISSING" },
        );
    }

    // --- the rule that decides the panel ------------------------------------
    let bare: Vec<u32> = tables.offered(|_| false).iter().map(|row| row.id).collect();
    let held: Vec<u32> = tables.offered(|_| true).iter().map(|row| row.id).collect();
    println!(
        "\noffered to a character carrying nothing: {bare:?}\n\
         offered to one carrying every stationery item: {held:?}"
    );
    if bare.len() != 1 {
        return Err(format!(
            "expected exactly one always-available paper; got {bare:?} — the flag column is `& 1`, \
             so this is the column being read one field over"
        ));
    }
    let gm = tables.stationery_by_id(GM_STATIONERY);
    match gm {
        Some(row) if row.always_available() => {
            return Err("the GM stationery is flagged as always available, which it is not".into())
        }
        Some(_) => println!("the GM row ({GM_STATIONERY}) is present and is never offered — correct"),
        None => println!("no GM row: a GM's letter will draw as an ordinary one"),
    }

    // --- the postage --------------------------------------------------------
    println!(
        "\npostage = {POSTAGE} copper + the selected paper's own item price, and the second \
         half is skipped\n         when the character is already carrying one. The \
         item prices are not in\n         the archives — the client learns them by \
         CMSG_ITEM_QUERY_SINGLE — so the price is\n         {POSTAGE} until the template answers."
    );

    if missing > 0 {
        return Err(format!(
            "{missing} of {} backgrounds are not in the archives",
            tables.stationery().len()
        ));
    }
    println!("\nevery background resolves, both halves.");
    Ok(())
}

fn one(assets: &mut vale_assets::Assets, tables: &MailTables, id: u32) -> Result<(), String> {
    let Some(row) = tables.stationery_by_id(id) else {
        let ids: Vec<u32> = tables.stationery().iter().map(|row| row.id).collect();
        return Err(format!("no stationery {id}; the table has {ids:?}"));
    };
    println!("stationery {}", row.id);
    println!("  item entry      {}", row.item);
    println!("  texture stem    {}", row.texture);
    println!(
        "  flags           {} ({})",
        row.flags,
        if row.always_available() {
            "bit 0 set: offered to everybody"
        } else {
            "bit 0 clear: offered only while its item is carried"
        }
    );
    if row.id == GM_STATIONERY {
        println!("  **a GM's paper** — a letter on it can never be replied to or kept as an item");
    }
    let (left, right) = row.background();
    for half in [&left, &right] {
        let path = format!("{half}.blp");
        match assets.read(&path) {
            Ok(bytes) => println!("  {path}: {} bytes", bytes.len()),
            Err(e) => return Err(format!("{path}: {e}")),
        }
    }
    println!(
        "\n  the letter's icon is this row's *item*'s icon, not its texture column\n  \
         — so it is blank until CMSG_ITEM_QUERY_SINGLE answers for {}.",
        row.item
    );
    println!("  postage on it: {POSTAGE} + item {} price, or {POSTAGE} while it is carried.", row.item);
    Ok(())
}

/// How many templates the table holds — a walk rather than a length, because
/// [`MailTables`] deliberately exposes lookups rather than the vector.
fn templates_in(tables: &MailTables) -> usize {
    (0..2000).filter(|id| tables.template(*id).is_some()).count()
}
