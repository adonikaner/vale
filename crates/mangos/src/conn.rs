//! The connection to the server's world database, and running statements on it.
//!
//! ## Where it is, and why that is an invented setting
//!
//! Every other thing this project reads has somewhere the real client already
//! puts it — the realm address is `realmlist.wtf`, the account is
//! `Config.wtf`'s. **The server's database is not one of those.** A 1.12 client
//! has no idea a database exists, has no field for one, and never will, so
//! there is no name registered by 5875 to store this under and this is a
//! mechanism of the editor's own. It is named here so nobody has to rediscover
//! that, the way `VALE_PASSWORD` is named in `vale_config`.
//!
//! Two variables, in order:
//!
//! ```text
//! VALE_WORLDDB   host;port;user;password;database — the connection itself
//! VALE_MANGOSD   a mangosd.conf, or the folder holding one, to read
//!                   `WorldDatabase.Info` out of
//! ```
//!
//! The second is the one to prefer: `WorldDatabase.Info` is where vmangos keeps
//! this already (`mangosd.conf:79` on the reference install), in exactly this
//! format, so pointing at the conf means there is still only one place the
//! answer lives. The first exists for a run that has no install to point at.
//!
//! **There is no default and no guess.** A folder that cannot be found is
//! reported as absent, and everything that needs a database says so and does
//! nothing — which is this plan's own rule for a machine that has not got one.
//! Guessing at `C:\MaNGOS` would be a connection attempt against whatever
//! happens to be there.
//!
//! ## Why this is not behind a Cargo feature
//!
//! A feature would mean two build configurations to keep working, and the
//! question it would be answering — *is there a database to talk to?* — is not
//! a compile-time question. It is answered at run time, once, by whether
//! [`Where::find`] returns anything and whether [`Db::open`] succeeds. The
//! editor greys the button either way.
//!
//! ## Everything runs in one transaction
//!
//! [`Db::run`] wraps its statements in `START TRANSACTION`/`COMMIT`. A save is
//! three statements per spell and a project can hold many, and a half-applied
//! project is the one state nothing here could describe: the SQL file would say
//! one thing, the rows another, and the undo written beside it would put back
//! rows that were never changed.
//!
//! **That holds only for a transactional engine, and the reference install is
//! not one.** `information_schema.tables` answers `MyISAM` for `item_template`,
//! `creature`, `creature_template`, `quest_template`, `spell_template` and
//! `npc_vendor` there, and MyISAM accepts `START TRANSACTION` and ignores it:
//! a statement that fails half way through a list leaves the ones before it
//! written. So nothing built on this may rely on the rollback. The editor's
//! applies are written so that every statement of an undo can be run against a
//! row it has already restored, and the undo is on disk before the first
//! statement runs — see `crates/editor/src/server/reconcile.rs`.

use mysql::prelude::Queryable;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where the world database is.
#[derive(Debug, Clone, PartialEq)]
pub struct Where {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Where {
    /// Parse vmangos' own `host;port;user;password;database`.
    ///
    /// The format is `Database::Initialize`'s and the separator is a semicolon;
    /// a password containing one cannot be expressed, which is vmangos' own
    /// limitation rather than this parser's.
    pub fn parse(info: &str) -> Option<Where> {
        let parts: Vec<&str> = info.trim().trim_matches('"').split(';').collect();
        let [host, port, user, password, database] = parts[..] else {
            return None;
        };
        Some(Where {
            host: host.trim().to_string(),
            port: port.trim().parse().ok()?,
            user: user.trim().to_string(),
            password: password.trim().to_string(),
            database: database.trim().to_string(),
        })
    }

    /// …out of a `mangosd.conf`, by its `WorldDatabase.Info` line.
    pub fn from_conf(path: impl AsRef<Path>) -> Option<Where> {
        Where::parse(&setting(path, "WorldDatabase.Info")?)
    }

    /// The two environment variables, in order — see the module comment.
    pub fn find() -> Option<Where> {
        if let Ok(info) = std::env::var("VALE_WORLDDB") {
            if let Some(found) = Where::parse(&info) {
                return Some(found);
            }
        }
        let at = PathBuf::from(std::env::var("VALE_MANGOSD").ok()?);
        let conf = match at.is_dir() {
            true => at.join("mangosd.conf"),
            false => at,
        };
        Where::from_conf(conf)
    }

    /// What to say when there is nowhere to connect to. A sentence rather than
    /// an error, because it is the ordinary state of a machine with no server
    /// on it.
    pub fn absent() -> String {
        "no world database: set VALE_MANGOSD to the server's mangosd.conf \
         (or VALE_WORLDDB to host;port;user;password;database)"
            .to_string()
    }

    /// Host and database, for a status line. **Never the password.**
    pub fn line(&self) -> String {
        format!("{}@{}:{}/{}", self.user, self.host, self.port, self.database)
    }
}

/// An open connection.
pub struct Db {
    conn: mysql::Conn,
    at: Where,
}

impl Db {
    pub fn open(at: &Where) -> Result<Db, String> {
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some(at.host.clone()))
            .tcp_port(at.port)
            .user(Some(at.user.clone()))
            .pass(Some(at.password.clone()))
            .db_name(Some(at.database.clone()));
        let conn = mysql::Conn::new(opts).map_err(|e| format!("{}: {e}", at.line()))?;
        Ok(Db { conn, at: at.clone() })
    }

    /// …or nothing, when there is nowhere to connect to.
    pub fn find() -> Result<Db, String> {
        let at = Where::find().ok_or_else(Where::absent)?;
        Db::open(&at)
    }

    pub fn at(&self) -> &Where {
        &self.at
    }

    /// **Run every statement, or none of them.**
    ///
    /// Returns how many rows were affected in total, which is the number worth
    /// reporting: three statements per spell of which two are `INSERT IGNORE`
    /// means the count is not the statement count and never will be.
    pub fn run(&mut self, statements: &[String]) -> Result<u64, String> {
        if statements.is_empty() {
            return Ok(0);
        }
        let mut tx = self
            .conn
            .start_transaction(mysql::TxOpts::default())
            .map_err(|e| e.to_string())?;
        let mut affected = 0;
        for statement in statements {
            tx.query_drop(statement)
                .map_err(|e| format!("{e}\n  in: {}", head(statement)))?;
            affected += tx.affected_rows();
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(affected)
    }

    /// One row as `column -> value`, with `None` for a SQL `NULL`.
    ///
    /// Everything comes back as text because that is what a statement needs it
    /// as; MySQL parses its own output back to the same value, and going
    /// through a typed round trip would add a place for a float to change in
    /// the fourth decimal.
    ///
    /// The first row of [`Db::rows`], and it fails where that does.
    pub fn row(&mut self, sql: &str) -> Result<Option<HashMap<String, Option<String>>>, String> {
        Ok(self.rows(sql)?.into_iter().next())
    }

    /// **Every row a statement returns**, in the order the server sent them.
    ///
    /// The same reading as [`Db::row`], repeated: text throughout, with `None`
    /// for a SQL `NULL`. One map per row is more allocation than a positional
    /// reading would be, and it is what keeps a caller from depending on the
    /// column order of a `SELECT *` — which for `creature_template` is 80
    /// columns and is the thing a schema transcription gets wrong.
    ///
    /// **A row that fails to arrive, or a value that will not read as text, is
    /// an error**, never a missing row or a `NULL`. These reads are what an
    /// undo is written from: a value read as `NULL` would be put back as one,
    /// and a row skipped would be a row a removal takes and its undo does not
    /// restore.
    pub fn rows(&mut self, sql: &str) -> Result<Vec<HashMap<String, Option<String>>>, String> {
        let mut out = Vec::new();
        let result = self
            .conn
            .query_iter(sql)
            .map_err(|e| format!("{e}\n  in: {}", head(sql)))?;
        for row in result {
            let row = row.map_err(|e| format!("{e}\n  in: {}", head(sql)))?;
            let columns = row.columns();
            let mut one = HashMap::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                let name = column.name_str().to_string();
                let value: Option<String> = match row.get_opt(index) {
                    Some(Ok(value)) => value,
                    Some(Err(e)) => {
                        return Err(format!(
                            "{name} could not be read as text ({e})\n  in: {}",
                            head(sql)
                        ))
                    }
                    None => {
                        return Err(format!("{name} is missing from the row\n  in: {}", head(sql)))
                    }
                };
                one.insert(name, value);
            }
            out.push(one);
        }
        Ok(out)
    }
}

/// **One setting out of a vmangos conf**, by its key.
///
/// The format is `key = value`, one a line, `#` for a comment. Case-insensitive
/// on the key, because the confs ship the keys in mixed case and a person
/// editing one does not keep to it.
pub fn setting(path: impl AsRef<Path>, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// **The content patch the server runs at**, when nothing says otherwise.
///
/// vmangos' own default and the reference install's. It decides which row of
/// `creature_template` — and of every other table keyed by `(entry, patch)` —
/// the server loads, so an edit written under the wrong one is an edit the
/// running server never reads. See [`crate::creature`].
pub const DEFAULT_WOW_PATCH: u32 = 10;

/// …and what `mangosd.conf` says it is.
///
/// `None` when there is no conf to read or it does not carry the key, and the
/// caller falls back to [`DEFAULT_WOW_PATCH`]. There is no way to ask the
/// running server: no packet and no GM command states it.
pub fn wow_patch_from_conf(path: impl AsRef<Path>) -> Option<u32> {
    setting(path, "WowPatch")?.trim_matches('"').trim().parse().ok()
}

/// The first line of a statement, for an error that must not carry 3 KB of SQL.
fn head(statement: &str) -> String {
    let first: String = statement.chars().take(90).collect();
    match first.len() < statement.len() {
        true => format!("{first}…"),
        false => first,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vmangos' own format, as `mangosd.conf` on the reference install spells
    /// it — quotes included, since the conf writes them.
    #[test]
    fn the_connection_string_is_vmangos_own_five_fields() {
        let at = Where::parse("\"127.0.0.1;3306;mangos;secret;mangos\"").expect("five fields");
        assert_eq!(at.host, "127.0.0.1");
        assert_eq!(at.port, 3306);
        assert_eq!(at.user, "mangos");
        assert_eq!(at.password, "secret");
        assert_eq!(at.database, "mangos");
    }

    #[test]
    fn anything_that_is_not_five_fields_is_not_a_connection() {
        for bad in ["", "127.0.0.1", "127.0.0.1;3306;u;p", "a;b;c;d;e;f", "h;nope;u;p;d"] {
            assert!(Where::parse(bad).is_none(), "{bad:?}");
        }
    }

    /// **The password is never in the line a panel or a log shows.** It is in
    /// the conf and in the environment and nowhere else this crate prints.
    #[test]
    fn the_status_line_does_not_carry_the_password() {
        let at = Where::parse("127.0.0.1;3306;mangos;hunter2;mangos").expect("five fields");
        assert!(!at.line().contains("hunter2"));
        assert_eq!(at.line(), "mangos@127.0.0.1:3306/mangos");
    }

    #[test]
    fn a_conf_is_read_by_its_key_and_comments_are_not() {
        let dir = std::env::temp_dir().join("vale-mangos-conf-test");
        std::fs::create_dir_all(&dir).expect("a temp folder");
        let conf = dir.join("mangosd.conf");
        std::fs::write(
            &conf,
            "# WorldDatabase.Info = \"commented;1;out;x;y\"\n\
             LoginDatabase.Info = \"127.0.0.1;3306;root;root;realmd\"\n\
             WorldDatabase.Info              = \"127.0.0.1;3306;root;root;mangos\"\n",
        )
        .expect("a temp file");
        let at = Where::from_conf(&conf).expect("the world line");
        assert_eq!(at.database, "mangos");
        let _ = std::fs::remove_file(&conf);
    }

    #[test]
    fn a_conf_without_the_key_is_nothing_rather_than_a_guess() {
        let dir = std::env::temp_dir().join("vale-mangos-conf-test");
        std::fs::create_dir_all(&dir).expect("a temp folder");
        let conf = dir.join("empty.conf");
        std::fs::write(&conf, "MaxPingTime = 30\n").expect("a temp file");
        assert!(Where::from_conf(&conf).is_none());
        let _ = std::fs::remove_file(&conf);
    }

    /// **The content patch is read from the same conf the connection is**, and
    /// its absence is the default rather than a failure: an install that never
    /// set `WowPatch` is running vmangos' own.
    #[test]
    fn the_content_patch_comes_out_of_the_conf_or_falls_back() {
        let dir = std::env::temp_dir().join("vale-mangos-conf-test");
        std::fs::create_dir_all(&dir).expect("a temp folder");
        let conf = dir.join("patch.conf");
        std::fs::write(&conf, "# WowPatch = 3\nWowPatch = 10\n").expect("a temp file");
        assert_eq!(wow_patch_from_conf(&conf), Some(10));
        assert_eq!(setting(&conf, "wowpatch").as_deref(), Some("10"));
        assert_eq!(setting(&conf, "MaxPingTime"), None);
        let _ = std::fs::remove_file(&conf);
        assert_eq!(wow_patch_from_conf(dir.join("not-there.conf")), None);
        assert_eq!(DEFAULT_WOW_PATCH, 10);
    }

    #[test]
    fn a_long_statement_is_cut_before_it_reaches_an_error_message() {
        let long = "x".repeat(4000);
        assert!(head(&long).len() < 120);
        assert_eq!(head("short"), "short");
    }
}
