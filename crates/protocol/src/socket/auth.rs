//! Realmd (logon server, default port 3724): SRP6 challenge/proof and the realm
//! list. Mirrors vmangos `src/realmd/AuthSocket.cpp`.
//!
//! The logon server uses a flat, un-encrypted, command-tagged framing — no
//! world-style headers. Every packet begins with a one-byte [`LogonCmd`].

use crate::bytes::{hex, read_exact, Reader, Writer};
use crate::codes::LogonResult;
use crate::socket::srp;
use crate::version::{self, ClientVersion};
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// First byte of every realmd packet in either direction.
/// vmangos `src/realmd/AuthCodes.h`, `enum eAuthCmd`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogonCmd {
    LogonChallenge = 0x00,
    LogonProof = 0x01,
    ReconnectChallenge = 0x02,
    ReconnectProof = 0x03,
    RealmList = 0x10,
}

impl LogonCmd {
    pub fn code(self) -> u8 {
        self as u8
    }
}

/// The realmd protocol version byte the client puts in the challenge. Not a
/// result code — realmd ignores it, but the retail client sends 3.
const CHALLENGE_PROTOCOL_VERSION: u8 = 3;

#[derive(Debug, Clone)]
pub struct Realm {
    pub name: String,
    /// "host:port" as advertised by the server, and the only word on where the
    /// world server is: a row here that this machine cannot reach is fixed in
    /// `realmd.realmlist`, not worked around on this side.
    pub address: String,
    pub population: f32,
    pub num_chars: u8,
}

pub struct AuthOutcome {
    /// The 40-byte SRP6 session key `K`, shared with the world server.
    pub session_key: [u8; 40],
    pub realms: Vec<Realm>,
    /// What the logon saw, in order, for a caller that wants to print it.
    ///
    /// **A library does not print.** These were `println!`s, which put `[auth]
    /// computed A=…` on the renderer's stdout at every login and gave no caller
    /// a way to format or suppress them. Failures are not in here — those are
    /// `io::Error` and carry their own message — but the server's `M2`
    /// *mismatch* is, because it is the one thing that is not fatal and still
    /// worth saying: we hold a usable session key, and the peer did not prove it
    /// knew the password.
    pub log: Vec<String>,
}

/// **Realmd's own port**, for a `realmlist.wtf` line that names a bare host —
/// which is how every private server's instructions spell it. The shipped
/// `realmList` default carries the port explicitly
/// (`us.logon.worldofwarcraft.com:3724`), so both forms reach here.
pub const LOGON_PORT: u16 = 3724;

/// `host` as a socket address, with [`LOGON_PORT`] filled in when it has no
/// port of its own.
pub fn logon_address(host: &str) -> String {
    match host.contains(':') {
        true => host.to_string(),
        false => format!("{host}:{LOGON_PORT}"),
    }
}

/// **How long to wait for realmd to accept the connection.**
///
/// A host that is down refuses at once and a host that is not there takes the
/// operating system's own SYN timeout — 21 seconds on Windows, over a minute
/// on Linux — which is a long time to sit on "Connecting to server…" for an
/// answer that is not coming. See [`REPLY_TIMEOUT`] for the half that matters
/// more.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// **…and how long to wait for it to say anything once it has.**
///
/// This is the one that was missing, and its absence is a whole class of bug
/// rather than a slow login. Every read in this file is a blocking
/// `read_exact`, so a realmd that accepts the connection and then goes quiet —
/// throttling, a wedged worker, a half-open connection a NAT dropped — parks
/// the caller **for ever**. The caller is a task on Bevy's compute pool with no
/// await point in it, so cancelling cannot unpark it either: the thread is gone
/// for the rest of the session, and enough of those starve the pool that also
/// streams terrain. That is the "endless connecting to server" loop, and it
/// gets worse each time the player presses Cancel and tries again.
///
/// Fifteen seconds is a choice, not a measurement: long enough that a busy
/// server on a real link is never cut off mid-handshake, short enough that a
/// wedged one gives the screen back while the player is still looking at it.
/// The reference has no equivalent to point at — 5875 runs its logon on its own
/// select loop and gives up on a *state machine* rather than a read.
///
/// Whatever it fires on becomes `LOGIN_SERVER_DOWN`, because
/// `session::logon_blocking` maps every `io::Error` here to that key — which is
/// the same sentence the reference shows when the socket will not open at all,
/// and the honest one for a server that opened it and then said nothing.
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// Open a socket to realmd that **cannot block for ever** — see
/// [`REPLY_TIMEOUT`], which is the whole reason this is not a
/// `TcpStream::connect`.
///
/// `connect_timeout` takes a resolved `SocketAddr` rather than a string, so the
/// name is resolved here and each address is tried in turn: a host with an
/// unreachable AAAA and a working A record is otherwise a five-second stall
/// before a login that would have worked. The last error is the one reported,
/// which is the same rule `TcpStream::connect` follows.
fn open_logon(host: &str) -> io::Result<TcpStream> {
    let address = logon_address(host);
    let mut last = None;
    for addr in address.to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
                stream.set_write_timeout(Some(REPLY_TIMEOUT))?;
                stream.set_nodelay(true).ok();
                return Ok(stream);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("{address} resolved to no addresses"),
        )
    }))
}

/// Perform the full logon: challenge -> proof -> realm list.
pub fn login(host: &str, account: &str, password: &str) -> io::Result<AuthOutcome> {
    login_as(host, account, password, &version::WOW_1_12_1_WIN_X86)
}

/// Log on while presenting a specific client build. Split out from [`login`] so
/// that supporting another build is a data change, not a code change.
pub fn login_as(
    host: &str,
    account: &str,
    password: &str,
    client: &ClientVersion,
) -> io::Result<AuthOutcome> {
    let mut stream = open_logon(host)?;

    // ---- CMD_AUTH_LOGON_CHALLENGE ----------------------------------------
    let account_upper = account.to_uppercase();
    let mut body = Writer::new();
    body.u8(CHALLENGE_PROTOCOL_VERSION)
        .u8(0) // size low byte  (patched below)
        .u8(0) // size high byte (patched below)
        .bytes(b"WoW\0")
        .u8(client.version.0)
        .u8(client.version.1)
        .u8(client.version.2)
        .u16(client.build)
        .bytes(&client.platform)
        .bytes(&client.os)
        .bytes(&client.locale)
        .u32(0) // timezone bias
        .u32(0x0100_007F) // client IP (127.0.0.1); unused by the server
        .u8(account_upper.len() as u8)
        .bytes(account_upper.as_bytes());
    // `size` counts every byte after itself: the leading protocol-version byte
    // and the two size bytes are excluded.
    let size = (body.buf.len() - 3) as u16;
    body.buf[1] = (size & 0xff) as u8;
    body.buf[2] = (size >> 8) as u8;

    let mut pkt = vec![LogonCmd::LogonChallenge.code()];
    pkt.extend_from_slice(&body.buf);
    stream.write_all(&pkt)?;

    // ---- challenge response ----------------------------------------------
    // cmd(1) unk(1) result(1) [ B[32] g_len g N_len N s[32]
    //                           versionChallenge[16] securityFlags(1) ]
    let head = read_exact(&mut stream, 3)?;
    expect_cmd(head[0], LogonCmd::LogonChallenge)?;
    check_result(head[2], "logon challenge")?;

    let b_pub = read_exact(&mut stream, 32)?;
    let g_len = read_exact(&mut stream, 1)?[0] as usize;
    let g = read_exact(&mut stream, g_len)?;
    let n_len = read_exact(&mut stream, 1)?[0] as usize;
    let n = read_exact(&mut stream, n_len)?;
    let salt = read_exact(&mut stream, 32)?;
    let _version_challenge = read_exact(&mut stream, 16)?;
    let security_flags = read_exact(&mut stream, 1)?[0];
    if security_flags != 0 {
        return Err(err(format!(
            "account requires 2FA/PIN (securityFlags={security_flags:#x}); not supported yet"
        )));
    }

    // ---- SRP6 client math -------------------------------------------------
    let srp = srp::compute(account, password, &b_pub, g[0], &n, &salt);
    let mut log = vec![
        format!("computed A={}", hex(&srp.a_pub)),
        format!("computed M1={}", hex(&srp.m1)),
    ];

    // ---- CMD_AUTH_LOGON_PROOF --------------------------------------------
    let mut proof = vec![LogonCmd::LogonProof.code()];
    proof.extend_from_slice(&srp.a_pub); // A[32]
    proof.extend_from_slice(&srp.m1); // M1[20]
    // crc_hash: the client-integrity proof. Sending zeros here is rejected with
    // VersionInvalid whenever the server runs StrictVersionCheck = 1 and has a
    // non-empty integrity hash for our build. See `version::ClientVersion`.
    proof.extend_from_slice(&srp::version_proof(&srp.a_pub, &client.integrity_hash));
    proof.push(0); // number_of_keys
    proof.push(0); // securityFlags
    stream.write_all(&proof)?;

    // ---- proof response ---------------------------------------------------
    // cmd(1) result(1); on success M2[20] surveyId(u32).
    let ph = read_exact(&mut stream, 2)?;
    expect_cmd(ph[0], LogonCmd::LogonProof)?;
    check_result(ph[1], "logon proof")?;

    let m2 = read_exact(&mut stream, 20)?;
    let _survey_id = read_exact(&mut stream, 4)?;
    let expected = srp::expected_m2(&srp.a_pub, &srp.m1, &srp.session_key);
    log.push(if m2 == expected {
        "server M2 verified — mutual auth OK".to_string()
    } else {
        // Not fatal: we already hold a usable session key. But a mismatch means
        // the peer did not prove knowledge of the password, so say so loudly.
        format!("WARNING server M2 mismatch (got {})", hex(&m2))
    });

    // ---- CMD_REALM_LIST ---------------------------------------------------
    stream.write_all(&[LogonCmd::RealmList.code(), 0, 0, 0, 0])?;
    let realms = read_realm_list(&mut stream)?;

    Ok(AuthOutcome {
        session_key: srp.session_key,
        realms,
        log,
    })
}

/// Read the realm-list reply (pre-2.0.3 / 1.12 layout).
fn read_realm_list(stream: &mut TcpStream) -> io::Result<Vec<Realm>> {
    let head = read_exact(stream, 3)?;
    expect_cmd(head[0], LogonCmd::RealmList)?;
    let size = u16::from_le_bytes([head[1], head[2]]) as usize;
    let body = read_exact(stream, size)?;
    let mut r = Reader::new(&body);

    let _unused = r.u32();
    let count = r.u8();
    let mut realms = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let _icon = r.u32(); // realm type
        let _flags = r.u8();
        let name = r.cstring();
        let address = r.cstring();
        let population = r.f32();
        let num_chars = r.u8();
        let _category = r.u8();
        let _unk = r.u8();
        realms.push(Realm {
            name,
            address,
            population,
            num_chars,
        });
    }
    Ok(realms)
}

fn expect_cmd(got: u8, want: LogonCmd) -> io::Result<()> {
    if got == want.code() {
        Ok(())
    } else {
        Err(err(format!(
            "expected {want:?} ({:#04x}), got {got:#04x}",
            want.code()
        )))
    }
}

/// Turn a realmd result byte into an error carrying what it actually means.
///
/// **The byte travels with the sentence.** A caller that only logs wants the
/// `Display`; the login screen wants to say what the *game* says, which is a
/// `GlueStrings.lua` key indexed by this code — see [`crate::codes::Refusal`],
/// which is what the error's inner error is.
fn check_result(code: u8, stage: &str) -> io::Result<()> {
    match LogonResult::from_code(code) {
        Some(r) if r.is_success() => Ok(()),
        Some(r) => Err(crate::codes::Refusal::Logon(code).into_error(format!(
            "{stage} rejected: {r:?} ({code}) — {}",
            r.explain()
        ))),
        None => Err(crate::codes::Refusal::Logon(code)
            .into_error(format!("{stage} rejected with unknown code {code}"))),
    }
}

fn err(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn a_bare_host_gets_realmds_own_port_and_one_with_a_port_keeps_it() {
        assert_eq!(logon_address("127.0.0.1"), "127.0.0.1:3724");
        assert_eq!(logon_address("127.0.0.1:9000"), "127.0.0.1:9000");
        assert_eq!(
            logon_address("us.logon.worldofwarcraft.com:3724"),
            "us.logon.worldofwarcraft.com:3724"
        );
    }

    /// **The socket comes back with both timeouts on it**, which is the whole
    /// of the fix: every read in this file is a blocking `read_exact`, so a
    /// realmd that accepts the connection and then says nothing used to park
    /// the caller for ever. Asserted on the socket rather than by waiting,
    /// because waiting is fifteen seconds.
    #[test]
    fn the_logon_socket_cannot_block_for_ever() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().unwrap().port();
        // Deliberately never accepted and never answered — the backlog takes
        // the connection, which is exactly the case this is about.
        let stream = open_logon(&format!("127.0.0.1:{port}")).expect("connect");
        assert_eq!(stream.read_timeout().unwrap(), Some(REPLY_TIMEOUT));
        assert_eq!(stream.write_timeout().unwrap(), Some(REPLY_TIMEOUT));
    }

    /// …and a port with nothing on it fails rather than hanging, with the
    /// error the caller turns into `LOGIN_SERVER_DOWN`.
    #[test]
    fn a_logon_port_with_nothing_on_it_fails_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(open_logon(&format!("127.0.0.1:{port}")).is_err());
    }
}
