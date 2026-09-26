//! Result codes returned by the two servers.
//!
//! Kept as enums rather than loose `u8` constants so that a stray number can
//! never be silently compared against the wrong namespace: realmd and the world
//! server both answer with a single byte, but they use *different, overlapping*
//! code tables. `4` means UNKNOWN_ACCOUNT to realmd and FAILED_TO_CONNECT to the
//! world server.
//!
//! Sources:
//!   * [`LogonResult`]  — vmangos `src/realmd/AuthCodes.h`, `enum AuthResult`
//!   * [`WorldResult`]  — vmangos `src/game/SharedDefines.h`, `enum ResponseCodes`
//!
//! …and one thing vmangos cannot answer, because it never crosses the wire:
//! **what the player is told**. Both tables index a set of `GlueStrings.lua`
//! keys, and which key belongs to which code is the *client's* rule — see
//! [`Refusal::glue_string_key`], which follows the client and is not a
//! reconstruction.

/// Second byte of a realmd `CMD_AUTH_LOGON_CHALLENGE` / `CMD_AUTH_LOGON_PROOF`
/// reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogonResult {
    Success = 0x00,
    Unknown0 = 0x01,
    Unknown1 = 0x02,
    Banned = 0x03,
    UnknownAccount = 0x04,
    IncorrectPassword = 0x05,
    AlreadyOnline = 0x06,
    NoTime = 0x07,
    DbBusy = 0x08,
    VersionInvalid = 0x09,
    VersionUpdate = 0x0A,
    InvalidServer = 0x0B,
    Suspended = 0x0C,
    NoAccess = 0x0D,
    SuccessSurvey = 0x0E,
    ParentalControl = 0x0F,
    Disconnected = 0xFF,
}

impl LogonResult {
    pub fn from_code(code: u8) -> Option<Self> {
        use LogonResult::*;
        Some(match code {
            0x00 => Success,
            0x01 => Unknown0,
            0x02 => Unknown1,
            0x03 => Banned,
            0x04 => UnknownAccount,
            0x05 => IncorrectPassword,
            0x06 => AlreadyOnline,
            0x07 => NoTime,
            0x08 => DbBusy,
            0x09 => VersionInvalid,
            0x0A => VersionUpdate,
            0x0B => InvalidServer,
            0x0C => Suspended,
            0x0D => NoAccess,
            0x0E => SuccessSurvey,
            0x0F => ParentalControl,
            0xFF => Disconnected,
            _ => return None,
        })
    }

    /// What the failure actually means for *this* client, including the
    /// non-obvious ones. Wrong guesses here cost real debugging time.
    pub fn explain(self) -> &'static str {
        use LogonResult::*;
        match self {
            Success | SuccessSurvey => "authenticated",
            UnknownAccount => "no such account (create it: `account create <user> <pass>`)",
            IncorrectPassword => "wrong password",
            Banned | Suspended => "account is banned or suspended",
            AlreadyOnline => "account already logged in; the world session may still be open",
            VersionInvalid => {
                // Reached only *after* SRP6 succeeds, so it is never a password
                // problem. See `srp::version_proof`.
                "client integrity check failed — the crc_hash did not match \
                 SHA1(A | allowed_clients.integrity_hash) for this build/os/platform"
            }
            VersionUpdate => "server wants a different client build",
            NoTime => "account has no playtime left",
            DbBusy => "server database busy; retry",
            InvalidServer | NoAccess | Unknown0 | Unknown1 => "server refused the connection",
            ParentalControl => "blocked by parental controls",
            Disconnected => "disconnected",
        }
    }

    pub fn is_success(self) -> bool {
        matches!(self, LogonResult::Success | LogonResult::SuccessSurvey)
    }

    /// **The `GlueStrings.lua` key the 1.12 client shows for this code**, or
    /// `None` for a code that is not a failure at all.
    ///
    /// Taken from the client rather than reasoned about, because the reasonable
    /// answer is wrong twice. The client lets `0x00` and `0x0e` through as
    /// successes and maps every other code onto one of twenty-one messages:
    /// `LOGIN_OK`, the six SRP messages, the three unknown-account variants,
    /// `LOGIN_INCORRECT_PASSWORD`, `LOGIN_FAILED`, `LOGIN_SERVER_DOWN`,
    /// `LOGIN_BANNED`, `LOGIN_BADVERSION`, `LOGIN_ALREADYONLINE`,
    /// `LOGIN_NOTIME`, `LOGIN_DBBUSY`, `LOGIN_SUSPENDED`,
    /// `LOGIN_PARENTALCONTROL` and `DISCONNECTED`.
    ///
    /// The two surprises, both of which a careful guess gets wrong:
    ///
    /// * **A wrong password says the same thing as an unknown account.** The
    ///   client handles `0x04` and `0x05` *identically*, so
    ///   `LOGIN_INCORRECT_PASSWORD` — which is in the file, and which is the
    ///   obvious answer — is never shown for a realmd refusal at all. It is
    ///   reachable only from the PIN branch of that case.
    /// * **`VersionUpdate` is not an error message.** The client reports it as
    ///   login *state* 6 and result 0, which is the patch-download path; it puts
    ///   no dialog up.
    ///
    /// Everything the client does not name — `0x01`, `0x02`, `InvalidServer`,
    /// `NoAccess` — falls to its default, `LOGIN_FAILED`.
    pub fn glue_string_key(self) -> Option<&'static str> {
        use LogonResult::*;
        Some(match self {
            Success | SuccessSurvey => return None,
            // The download path. No dialog: see the second bullet above.
            VersionUpdate => return None,
            Banned => "LOGIN_BANNED",
            UnknownAccount | IncorrectPassword => "LOGIN_UNKNOWN_ACCOUNT",
            AlreadyOnline => "LOGIN_ALREADYONLINE",
            NoTime => "LOGIN_NOTIME",
            DbBusy => "LOGIN_DBBUSY",
            VersionInvalid => "LOGIN_BADVERSION",
            Suspended => "LOGIN_SUSPENDED",
            ParentalControl => "LOGIN_PARENTALCONTROL",
            Disconnected => "DISCONNECTED",
            Unknown0 | Unknown1 | InvalidServer | NoAccess => "LOGIN_FAILED",
        })
    }

    /// Render an arbitrary wire byte for logging.
    pub fn describe(code: u8) -> String {
        match Self::from_code(code) {
            Some(r) => format!("{r:?} — {}", r.explain()),
            None => format!("UNKNOWN({code})"),
        }
    }
}

/// Body byte of `SMSG_AUTH_RESPONSE`.
///
/// vmangos declares these in one large implicit-value `enum ResponseCodes`, so
/// the numbers below are the positional values from that enum (`AUTH_OK` is the
/// 13th entry, hence 12). Only the `AUTH_*` range is modelled; the surrounding
/// `RESPONSE_*` / `CSTATUS_*` / `REALM_LIST_*` entries never appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WorldResult {
    Ok = 12,
    Failed = 13,
    Reject = 14,
    BadServerProof = 15,
    Unavailable = 16,
    SystemError = 17,
    BillingError = 18,
    BillingExpired = 19,
    VersionMismatch = 20,
    UnknownAccount = 21,
    IncorrectPassword = 22,
    SessionExpired = 23,
    ServerShuttingDown = 24,
    AlreadyLoggingIn = 25,
    LoginServerNotFound = 26,
    WaitQueue = 27,
    Banned = 28,
    AlreadyOnline = 29,
    NoTime = 30,
    DbBusy = 31,
    Suspended = 32,
    ParentalControl = 33,
}

impl WorldResult {
    pub fn from_code(code: u8) -> Option<Self> {
        use WorldResult::*;
        Some(match code {
            12 => Ok,
            13 => Failed,
            14 => Reject,
            15 => BadServerProof,
            16 => Unavailable,
            17 => SystemError,
            18 => BillingError,
            19 => BillingExpired,
            20 => VersionMismatch,
            21 => UnknownAccount,
            22 => IncorrectPassword,
            23 => SessionExpired,
            24 => ServerShuttingDown,
            25 => AlreadyLoggingIn,
            26 => LoginServerNotFound,
            27 => WaitQueue,
            28 => Banned,
            29 => AlreadyOnline,
            30 => NoTime,
            31 => DbBusy,
            32 => Suspended,
            33 => ParentalControl,
            _ => return None,
        })
    }

    pub fn describe(code: u8) -> String {
        match Self::from_code(code) {
            Some(r) => format!("{r:?}"),
            None => format!("UNKNOWN({code})"),
        }
    }

    /// **The `GlueStrings.lua` key for this code**, or `None` for `Ok` and for
    /// the queue, which is a wait rather than a refusal.
    ///
    /// A flat array rather than a switch this time: the client keeps 34 strings
    /// indexed by the response code outright, `RESPONSE_SUCCESS` at 0
    /// through `AUTH_PARENTAL_CONTROL` at 33, with `AUTH_OK` landing at exactly
    /// 12 — which is the check that says the array is the enum. So every key
    /// here is its own variant's name, and that is a measurement rather than a
    /// convention this client chose.
    ///
    /// Five of them are shown through `OKAY_WITH_URL` rather than `OKAY` in the
    /// reference, with a companion `AUTH_*_URL` key as the dialog's `data`
    /// (banned, no time, db busy, suspended, parental control).
    /// This client does not open a browser — see `lua::glue`'s `LaunchURL` —
    /// so it shows the same text on a plain `OKAY`.
    pub fn glue_string_key(self) -> Option<&'static str> {
        use WorldResult::*;
        Some(match self {
            Ok | WaitQueue => return None,
            Failed => "AUTH_FAILED",
            Reject => "AUTH_REJECT",
            BadServerProof => "AUTH_BAD_SERVER_PROOF",
            Unavailable => "AUTH_UNAVAILABLE",
            SystemError => "AUTH_SYSTEM_ERROR",
            BillingError => "AUTH_BILLING_ERROR",
            BillingExpired => "AUTH_BILLING_EXPIRED",
            VersionMismatch => "AUTH_VERSION_MISMATCH",
            UnknownAccount => "AUTH_UNKNOWN_ACCOUNT",
            IncorrectPassword => "AUTH_INCORRECT_PASSWORD",
            SessionExpired => "AUTH_SESSION_EXPIRED",
            ServerShuttingDown => "AUTH_SERVER_SHUTTING_DOWN",
            AlreadyLoggingIn => "AUTH_ALREADY_LOGGING_IN",
            LoginServerNotFound => "AUTH_LOGIN_SERVER_NOT_FOUND",
            Banned => "AUTH_BANNED",
            AlreadyOnline => "AUTH_ALREADY_ONLINE",
            NoTime => "AUTH_NO_TIME",
            DbBusy => "AUTH_DB_BUSY",
            Suspended => "AUTH_SUSPENDED",
            ParentalControl => "AUTH_PARENTAL_CONTROL",
        })
    }
}

/// **Every `ResponseCodes` value, as the `GlueStrings.lua` key it is shown
/// through** — transcribed from the client's own table.
///
/// [`WorldResult`] above models the `AUTH_*` window of this enum because that is
/// the only part `SMSG_AUTH_RESPONSE` can carry. `SMSG_CHAR_CREATE` answers with
/// a byte from *two* further windows — `CHAR_CREATE_*` at 45..=55 and
/// `CHAR_NAME_*` at 69..=82, because a name the server will not take is refused
/// by the name checker rather than by the creator — and an enum spanning both
/// would be a third table to keep in step with this one for no gain. So this is
/// the flat array, which is also **exactly the mechanism the client uses**: 84
/// string pointers indexed by the response code outright, with `AUTH_OK` landing
/// at 12 and `CHAR_CREATE_SUCCESS` at 46, which is the check that says the array
/// is the enum.
///
/// Every key here is its own variant's name, and that is a measurement rather
/// than a convention this repo chose — the alternative, composing a key from a
/// variant name in Rust, is what produces keys the shipped file does not carry.
const RESPONSE_KEYS: [&str; 83] = [
    "RESPONSE_SUCCESS",
    "RESPONSE_FAILURE",
    "RESPONSE_CANCELLED",
    "RESPONSE_DISCONNECTED",
    "RESPONSE_FAILED_TO_CONNECT",
    "RESPONSE_CONNECTED",
    "RESPONSE_VERSION_MISMATCH",
    "CSTATUS_CONNECTING",
    "CSTATUS_NEGOTIATING_SECURITY",
    "CSTATUS_NEGOTIATION_COMPLETE",
    "CSTATUS_NEGOTIATION_FAILED",
    "CSTATUS_AUTHENTICATING",
    "AUTH_OK",
    "AUTH_FAILED",
    "AUTH_REJECT",
    "AUTH_BAD_SERVER_PROOF",
    "AUTH_UNAVAILABLE",
    "AUTH_SYSTEM_ERROR",
    "AUTH_BILLING_ERROR",
    "AUTH_BILLING_EXPIRED",
    "AUTH_VERSION_MISMATCH",
    "AUTH_UNKNOWN_ACCOUNT",
    "AUTH_INCORRECT_PASSWORD",
    "AUTH_SESSION_EXPIRED",
    "AUTH_SERVER_SHUTTING_DOWN",
    "AUTH_ALREADY_LOGGING_IN",
    "AUTH_LOGIN_SERVER_NOT_FOUND",
    "AUTH_WAIT_QUEUE",
    "AUTH_BANNED",
    "AUTH_ALREADY_ONLINE",
    "AUTH_NO_TIME",
    "AUTH_DB_BUSY",
    "AUTH_SUSPENDED",
    "AUTH_PARENTAL_CONTROL",
    "REALM_LIST_IN_PROGRESS",
    "REALM_LIST_SUCCESS",
    "REALM_LIST_FAILED",
    "REALM_LIST_INVALID",
    "REALM_LIST_REALM_NOT_FOUND",
    "ACCOUNT_CREATE_IN_PROGRESS",
    "ACCOUNT_CREATE_SUCCESS",
    "ACCOUNT_CREATE_FAILED",
    "CHAR_LIST_RETRIEVING",
    "CHAR_LIST_RETRIEVED",
    "CHAR_LIST_FAILED",
    "CHAR_CREATE_IN_PROGRESS",
    "CHAR_CREATE_SUCCESS",
    "CHAR_CREATE_ERROR",
    "CHAR_CREATE_FAILED",
    "CHAR_CREATE_NAME_IN_USE",
    "CHAR_CREATE_DISABLED",
    "CHAR_CREATE_PVP_TEAMS_VIOLATION",
    "CHAR_CREATE_SERVER_LIMIT",
    "CHAR_CREATE_ACCOUNT_LIMIT",
    "CHAR_CREATE_SERVER_QUEUE",
    "CHAR_CREATE_ONLY_EXISTING",
    "CHAR_DELETE_IN_PROGRESS",
    "CHAR_DELETE_SUCCESS",
    "CHAR_DELETE_FAILED",
    "CHAR_DELETE_FAILED_LOCKED_FOR_TRANSFER",
    "CHAR_LOGIN_IN_PROGRESS",
    "CHAR_LOGIN_SUCCESS",
    "CHAR_LOGIN_NO_WORLD",
    "CHAR_LOGIN_DUPLICATE_CHARACTER",
    "CHAR_LOGIN_NO_INSTANCES",
    "CHAR_LOGIN_FAILED",
    "CHAR_LOGIN_DISABLED",
    "CHAR_LOGIN_NO_CHARACTER",
    "CHAR_LOGIN_LOCKED_FOR_TRANSFER",
    "CHAR_NAME_NO_NAME",
    "CHAR_NAME_TOO_SHORT",
    "CHAR_NAME_TOO_LONG",
    "CHAR_NAME_INVALID_CHARACTER",
    "CHAR_NAME_MIXED_LANGUAGES",
    "CHAR_NAME_PROFANE",
    "CHAR_NAME_RESERVED",
    "CHAR_NAME_INVALID_APOSTROPHE",
    "CHAR_NAME_MULTIPLE_APOSTROPHES",
    "CHAR_NAME_THREE_CONSECUTIVE",
    "CHAR_NAME_INVALID_SPACE",
    "CHAR_NAME_CONSECUTIVE_SPACES",
    "CHAR_NAME_FAILURE",
    "CHAR_NAME_SUCCESS",
];

/// **`CHAR_CREATE_SUCCESS`** — the one value of `SMSG_CHAR_CREATE` that is not a
/// refusal, and the only index into [`RESPONSE_KEYS`] this client compares
/// against. Named rather than written as 46 for the reason every other code in
/// this file is: the number means nothing without the table it indexes.
pub const CHAR_CREATE_SUCCESS: u8 = 46;

/// **`CHAR_DELETE_SUCCESS`** — the same thing eleven entries on, for
/// `SMSG_CHAR_DELETE`. The `CHAR_DELETE_*` window is 56..=59 and only this one
/// value means the character is gone; vmangos sends `CHAR_DELETE_FAILED` for a
/// guild leader and **nothing at all** for its other three refusals, which is
/// [`crate::play::charcreate::delete_character`]'s problem rather than this table's.
pub const CHAR_DELETE_SUCCESS: u8 = 57;

/// The `GlueStrings.lua` key for any `ResponseCodes` byte, or `None` for one the
/// client's own array does not reach.
///
/// **`None` is a real answer**: the array is 83 long and a byte past it is a
/// code this build does not have, which shows no dialog at all rather than an
/// invented one — the same rule `assets::strings` keeps for a missing key.
pub fn response_key(code: u8) -> Option<&'static str> {
    RESPONSE_KEYS.get(usize::from(code)).copied()
}

/// **A refusal that came back with a code**, kept whole so the client can say
/// what the game says rather than print a sentence this repo made up.
///
/// Two variants because there are two tables and `4` means two different things
/// depending on which socket answered — the same reason [`LogonResult`] and
/// [`WorldResult`] are separate enums. This is what travels: the failure paths
/// hand back an `io::Error` whose inner error is one of these, so a caller that
/// only wants to log gets the `Display` and a caller that wants to *show*
/// something asks for [`Refusal::glue_string_key`].
///
/// **A wire byte no table names is data, not an error** — the code is kept
/// verbatim and `glue_string_key` answers the same default the client itself
/// does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// realmd's, on :3724 — a `CMD_AUTH_LOGON_CHALLENGE` or `_PROOF` reply.
    Logon(u8),
    /// The world server's, on :8085 — `SMSG_AUTH_RESPONSE`.
    World(u8),
}

impl Refusal {
    /// The `GlueStrings.lua` key to show, or `None` when this code is not
    /// something the player is told about.
    ///
    /// **An unrecognised realmd byte is `LOGIN_FAILED`** (the reference's own
    /// default), and an unrecognised world byte is `AUTH_FAILED` — which is
    /// this client's choice rather than a measurement, because the reference
    /// indexes its array without a bounds check and the codes below 12 are
    /// connection *states* rather than refusals.
    pub fn glue_string_key(self) -> Option<&'static str> {
        match self {
            Refusal::Logon(code) => LogonResult::from_code(code)
                .map_or(Some("LOGIN_FAILED"), LogonResult::glue_string_key),
            Refusal::World(code) => WorldResult::from_code(code)
                .map_or(Some("AUTH_FAILED"), WorldResult::glue_string_key),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Logon(code) => write!(f, "{}", LogonResult::describe(*code)),
            Refusal::World(code) => write!(f, "{}", WorldResult::describe(*code)),
        }
    }
}

impl std::error::Error for Refusal {}

impl Refusal {
    /// Recover a refusal from an [`std::io::Error`] one of the handshakes
    /// produced, or `None` for an ordinary socket failure.
    ///
    /// The failure paths stay `io::Result` — every caller of `auth::login` and
    /// `WorldSession::connect` wants one, and two of the three only ever print
    /// it — so the code travels as the error's *inner* error rather than as a
    /// second return type. This is the one place that knows to look.
    pub fn of(error: &std::io::Error) -> Option<Refusal> {
        error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Rejected>())
            .map(|rejected| rejected.refusal)
    }

    /// …and the constructor for the other end: an `io::Error` that carries both
    /// a sentence and this code.
    pub fn into_error(self, message: String) -> std::io::Error {
        std::io::Error::other(Rejected {
            refusal: self,
            message,
        })
    }
}

/// The payload of a refusal's `io::Error`: the sentence a log wants and the code
/// a dialog wants, in one value.
#[derive(Debug)]
struct Rejected {
    refusal: Refusal,
    message: String,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Rejected {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.refusal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_wire_values_match_vmangos() {
        // These two are load-bearing: the world handshake is judged by them.
        assert_eq!(WorldResult::Ok as u8, 12);
        assert_eq!(WorldResult::WaitQueue as u8, 27);
        assert_eq!(LogonResult::VersionInvalid as u8, 9);
    }

    #[test]
    fn unknown_codes_do_not_panic() {
        assert!(LogonResult::from_code(200).is_none());
        assert!(WorldResult::from_code(0).is_none());
        assert!(WorldResult::describe(99).contains("UNKNOWN"));
    }

    /// **The two the reference gets right and a guess gets wrong**, which is the
    /// whole reason [`LogonResult::glue_string_key`] follows the client rather
    /// than the enum's own names.
    #[test]
    fn a_wrong_password_says_what_an_unknown_account_says() {
        // The client handles `0x04` and `0x05` identically, so
        // `LOGIN_INCORRECT_PASSWORD` is never shown for a realmd refusal at all.
        assert_eq!(
            LogonResult::IncorrectPassword.glue_string_key(),
            Some("LOGIN_UNKNOWN_ACCOUNT")
        );
        assert_eq!(
            LogonResult::UnknownAccount.glue_string_key(),
            LogonResult::IncorrectPassword.glue_string_key()
        );
        // …and a version *update* puts no dialog up: it is the patch
        // download, not an error.
        assert_eq!(LogonResult::VersionUpdate.glue_string_key(), None);
        assert_eq!(
            LogonResult::VersionInvalid.glue_string_key(),
            Some("LOGIN_BADVERSION"),
            "the integrity refusal is the one that does"
        );
    }

    /// A success is not a message, and everything the client does not name
    /// falls to its default.
    #[test]
    fn only_a_refusal_has_something_to_say() {
        assert_eq!(LogonResult::Success.glue_string_key(), None);
        assert_eq!(LogonResult::SuccessSurvey.glue_string_key(), None);
        assert_eq!(WorldResult::Ok.glue_string_key(), None);
        // The queue is a wait, and the reference draws it as a *status* line
        // rather than as a refusal.
        assert_eq!(WorldResult::WaitQueue.glue_string_key(), None);

        assert_eq!(
            Refusal::Logon(0x0b).glue_string_key(),
            Some("LOGIN_FAILED"),
            "InvalidServer is in the enum and not in the table"
        );
        assert_eq!(Refusal::Logon(200).glue_string_key(), Some("LOGIN_FAILED"));
        assert_eq!(Refusal::World(200).glue_string_key(), Some("AUTH_FAILED"));
    }

    /// **The flat table and the `AUTH_*` enum are the same table**, which is the
    /// check that stops a transcription of 83 strings from drifting.
    ///
    /// [`RESPONSE_KEYS`] was transcribed from the client's own table and
    /// [`WorldResult::glue_string_key`] from the same table separately; they
    /// overlap at 12..=33 and every entry has to agree. The two exceptions are the two this file already documents as *not* refusals
    /// — `AUTH_OK` and the queue — which the enum answers `None` for and the
    /// array still names, because the array is indexed by code and not by
    /// whether there is anything to say.
    #[test]
    fn the_flat_response_table_agrees_with_the_auth_enum() {
        for code in 12..=33u8 {
            let from_enum = WorldResult::from_code(code)
                .expect("every code in the window is a variant")
                .glue_string_key();
            let from_array = response_key(code);
            match from_enum {
                Some(key) => assert_eq!(Some(key), from_array, "code {code} disagrees"),
                // The two that are not refusals still occupy their slot.
                None => assert!(
                    matches!(from_array, Some("AUTH_OK" | "AUTH_WAIT_QUEUE")),
                    "code {code} answers no key and is not one of the two that may"
                ),
            }
        }
        // …and the two landmarks that say the array is the enum rather than
        // something that merely starts the same way.
        assert_eq!(response_key(12), Some("AUTH_OK"));
        assert_eq!(response_key(CHAR_CREATE_SUCCESS), Some("CHAR_CREATE_SUCCESS"));
        assert_eq!(response_key(82), Some("CHAR_NAME_SUCCESS"));
        assert_eq!(response_key(83), None, "the array ends at 82");
        assert_eq!(response_key(255), None);
    }

    /// **The code survives the `io::Error`**, which is what lets the failure
    /// paths keep their `io::Result` while the login screen still gets to show
    /// the game's own sentence.
    #[test]
    fn a_refusal_travels_inside_the_error_it_is_reported_as() {
        let error = Refusal::Logon(0x04).into_error("logon proof rejected".into());
        assert_eq!(error.to_string(), "logon proof rejected");
        assert_eq!(Refusal::of(&error), Some(Refusal::Logon(0x04)));

        // An ordinary socket failure carries no code, and asking must not be an
        // error in itself.
        let plain = std::io::Error::other("connection refused");
        assert_eq!(Refusal::of(&plain), None);
    }
}
