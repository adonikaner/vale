//! `vale dbc` — every table's shape, and one table field by field.
//!
//! Bare, it is the census and the round trip: every entry under
//! `DBFilesClient\`, its shape, and whether the editable container in
//! `vale-edit` writes it back byte for byte. That is the only check a
//! lossless writer can have, and it is a headless check over the archives, so it
//! belongs here rather than in the editor — the same argument `vale bake`
//! is here on.

use crate::common::*;
use vale_config::Config;
use vale_edit::dbc::DbcFile;

/// Dump a DBC table's shape and a few records, guessing each field's type.
///
/// Field *meanings* are not in the file, so the only honest way to find out
/// which column of `CreatureDisplayInfo` is the model id is to look at the
/// numbers. A value that resolves to a plausible string in the string block is
/// almost certainly a string; a small float near 1.0 is a scale.
pub fn cmd_dbc(cfg: &Config, table: &str, id: Option<u32>) -> Result<(), String> {
    use vale_assets::tables::dbc::{dbc_path, Dbc};

    let mut assets = open_assets(cfg)?;
    let path = dbc_path(table);
    let raw = assets.read(&path).map_err(|e| e.to_string())?;
    let dbc = Dbc::parse(&raw).map_err(|e| e.to_string())?;
    println!(
        "{path}: {} records x {} fields",
        dbc.record_count, dbc.field_count
    );

    // Either one record by id (field 0 is the id in every table) or the first
    // few, which is enough to see the shape.
    let rows: Vec<usize> = match id {
        Some(id) => (0..dbc.record_count)
            .filter(|&r| dbc.u32_at(r, 0) == Some(id))
            .collect(),
        None => (0..dbc.record_count.min(3)).collect(),
    };
    if rows.is_empty() {
        return Err(format!("no record with id {id:?} in {table}"));
    }

    for record in rows {
        println!("\n  record {record}:");
        for field in 0..dbc.field_count {
            let raw = dbc.u32_at(record, field).unwrap_or(0);
            let as_f32 = f32::from_bits(raw);
            let text = dbc.string_at(record, field).unwrap_or_default();
            let mut guesses = vec![format!("u32 {raw}")];
            if raw != 0 && as_f32.is_finite() && as_f32.abs() > 1.0e-4 && as_f32.abs() < 1.0e6 {
                guesses.push(format!("f32 {as_f32:.3}"));
            }
            if !text.is_empty() && text.is_ascii() {
                guesses.push(format!("str {text:?}"));
            }
            println!("    [{field:>2}] {}", guesses.join("  |  "));
        }
    }
    Ok(())
}

/// Every table in the archives: its shape, and whether it survives a round trip
/// through [`DbcFile`].
///
/// Four numbers are reported for the population rather than for each table,
/// because each is an assumption something downstream makes and none of them is
/// stated by the format:
///
/// * how many entries are **not tables at all** — zero-byte files under a `.dbc`
///   name, which the client asks for and gets nothing from;
/// * how many have a `recordSize` that is not `fieldCount * 4`, which is what
///   every field-by-index reader in this repository assumes;
/// * how many have a string block that does not open with a NUL, which is what
///   makes offset 0 the empty string;
/// * how many do not write back byte for byte, which must be none.
pub fn cmd_dbc_all(cfg: &Config) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let mut paths = assets.list_prefix("DBFilesClient\\");
    paths.sort();
    if paths.is_empty() {
        return Err("no DBFilesClient\\ entries in the archives".to_string());
    }

    let mut empty = Vec::new();
    let mut unreadable = Vec::new();
    let mut odd_record = Vec::new();
    let mut odd_strings = Vec::new();
    let mut lossy = Vec::new();
    let mut tables = 0usize;
    let mut records = 0usize;
    let mut bytes = 0usize;

    for path in &paths {
        let name = path.rsplit('\\').next().unwrap_or(path);
        let Ok(raw) = assets.read(path) else {
            unreadable.push(name.to_string());
            continue;
        };
        if raw.is_empty() {
            empty.push(name.to_string());
            println!("  {name:<40} empty");
            continue;
        }
        let table = match DbcFile::parse(&raw) {
            Ok(table) => table,
            Err(e) => {
                unreadable.push(format!("{name}: {e}"));
                println!("  {name:<40} {e}");
                continue;
            }
        };

        let written = table.write();
        let same = written == raw;
        if !same {
            lossy.push(name.to_string());
        }
        if table.record_size() != table.field_count() * 4 {
            odd_record.push(name.to_string());
        }
        if table.string_size() > 0 && table.text_at(0).as_deref() != Some("") {
            odd_strings.push(name.to_string());
        }

        tables += 1;
        records += table.record_count();
        bytes += raw.len();
        println!(
            "  {name:<40} {:>6} x {:>3}  record {:>4}  strings {:>7}  {}",
            table.record_count(),
            table.field_count(),
            table.record_size(),
            table.string_size(),
            if same { "same" } else { "DIFFERS" },
        );
    }

    println!(
        "\n{} entries: {tables} tables, {} records, {} KiB",
        paths.len(),
        records,
        bytes / 1024
    );
    println!("  empty files:          {:>3}  {}", empty.len(), empty.join(" "));
    println!(
        "  would not parse:      {:>3}  {}",
        unreadable.len(),
        unreadable.join(" ")
    );
    println!(
        "  recordSize != 4n:     {:>3}  {}",
        odd_record.len(),
        odd_record.join(" ")
    );
    println!(
        "  strings not NUL-led:  {:>3}  {}",
        odd_strings.len(),
        odd_strings.join(" ")
    );
    println!("  did not round trip:   {:>3}  {}", lossy.len(), lossy.join(" "));

    if lossy.is_empty() {
        Ok(())
    } else {
        Err(format!("{} tables did not write back", lossy.len()))
    }
}
