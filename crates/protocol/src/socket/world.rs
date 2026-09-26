//! World server session: the encrypted handshake and opcode traffic. Mirrors
//! vmangos `src/game/Protocol/WorldSocket.cpp` and `MangosSocketImpl.h`.
//!
//! Header framing (only headers are ciphered; bodies are plaintext):
//!   * server -> client: `size(u16 BE, = body + 2)` + `opcode(u16 LE)`  [4 bytes]
//!   * client -> server: `size(u16 BE, = body + 4)` + `opcode(u32 LE)`  [6 bytes]
//!
//! The size field counts the opcode, which is why the two directions differ by
//! two bytes: the opcode is u16 inbound and u32 outbound.

use crate::bytes::{Reader, Writer};
use crate::codes::WorldResult;
use crate::socket::crypt::HeaderCrypt;
use crate::socket::handler::{apply_packet, Incoming, PumpStats, Replies};
use crate::state::movement::MovementInfo;
use crate::state::objects::ObjectManager;
use crate::opcodes::Opcode;
use crate::state::query;
use crate::version::{self, ClientVersion};
use sha1::{Digest, Sha1};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A framed world packet: an opcode as it appeared on the wire, plus its body.
///
/// The opcode is kept as a raw `u32` rather than an [`Opcode`] because the
/// server legitimately sends opcodes this client has not implemented; turning
/// those into a hard error would break the session for no reason. Use
/// [`Packet::opcode`] to match on the ones we know.
#[derive(Debug, Clone)]
pub struct Packet {
    pub code: u32,
    pub body: Vec<u8>,
}

impl Packet {
    /// The opcode, if it is one we know about.
    pub fn opcode(&self) -> Option<Opcode> {
        Opcode::from_code(self.code)
    }

    pub fn is(&self, op: Opcode) -> bool {
        self.code == op.code()
    }

    /// Name for logging — falls back to the raw hex code when unknown.
    pub fn name(&self) -> String {
        Opcode::describe(self.code)
    }
}

/// **What has actually crossed the socket**, in bytes and in packets.
///
/// Counted at the two funnels — [`WorldSession::take_packet`] frames every
/// inbound packet and [`WorldSession::send`] writes every outbound one — rather
/// than beside the dispatch, because those are the only two places that see the
/// *wire* rather than the client's reading of it. The difference is not
/// academic: `recv_until` discards packets before they ever reach
/// `apply_packet`, and a header is four bytes that no body length includes.
///
/// The bytes are what this client framed and wrote, which is the payload and
/// its header and not the TCP or IP overhead beneath them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WireCounts {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub packets_in: u64,
    pub packets_out: u64,
}

/// One opcode's share of that traffic.
///
/// `handled` is meaningful for the inbound half only, and it is the number that
/// matters: an opcode arriving a thousand times with nothing reading it is a
/// whole subject this client is deaf to, and it looks identical from every other
/// counter to one that never arrives at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpcodeFlow {
    pub count: u32,
    pub bytes: u64,
    pub handled: bool,
}

/// **The wire, opcode by opcode**, as the debug panel reads it.
///
/// Published behind an `Arc` and rebuilt a few times a second rather than
/// copied into every status snapshot: the renderer asks for
/// [`crate::socket::session::SessionStatus`] once a frame, and a `BTreeMap` of a
/// hundred opcode names cloned a hundred times a second to learn nothing is
/// exactly the trade `LiveSession::world_ms` was split out to avoid.
///
/// **The names are resolved here and not while counting.** `Packet::name` is a
/// `format!` — it allocates a `String` per call — so keying the live tables by
/// name would have put an allocation on the path every packet in the session
/// takes. They are counted by raw opcode and named once per rebuild, which is
/// five times a second against a stream that can carry hundreds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Traffic {
    pub wire: WireCounts,
    /// Every opcode the dispatch saw, and whether anything read it.
    pub inbound: std::collections::BTreeMap<String, OpcodeFlow>,
    /// …and every one this client sent. `handled` is always true here; a
    /// packet we wrote is by definition one we meant to.
    pub outbound: std::collections::BTreeMap<String, OpcodeFlow>,
}

/// Name the rows of a table counted by raw opcode.
///
/// The one place a `Traffic` is built from the two live tables — see its own
/// note on why they are keyed by number.
pub fn named(counted: &std::collections::BTreeMap<u32, OpcodeFlow>) -> std::collections::BTreeMap<String, OpcodeFlow> {
    counted
        .iter()
        .map(|(code, flow)| (Opcode::describe(*code), *flow))
        .collect()
}

/// One packet kept whole, so that its body can be read a byte at a time.
///
/// The counters above say how much of each opcode crossed the wire. They cannot
/// say what any one packet contained, which is the question every parser bug in
/// this project has started from: a field taken at the wrong offset produces a
/// plausible value and no diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPacket {
    /// Position in the capture order, counting both directions. The two funnels
    /// run on the same thread, so this orders the stream exactly.
    pub sequence: u64,
    /// Milliseconds since the capture was armed.
    pub at_ms: u32,
    pub code: u32,
    pub name: String,
    /// Whether the server sent it. `false` is a packet this client wrote.
    pub inbound: bool,
    /// The body, truncated at [`CAPTURE_BODY_BYTES`].
    pub body: Vec<u8>,
    /// The body's true length, which may be longer than what was kept.
    pub length: usize,
}

impl CapturedPacket {
    /// Whether the body was cut short by [`CAPTURE_BODY_BYTES`].
    pub fn truncated(&self) -> bool {
        self.length > self.body.len()
    }
}

/// How many packets the capture ring holds. The oldest is dropped to make room.
///
/// A busy realm sends a few hundred packets a second, so this is a few seconds
/// of stream — long enough to hold the packet that preceded a visible fault and
/// short enough that a snapshot of it is not a measurable copy.
pub const CAPTURE_PACKETS: usize = 512;

/// How much of each body is kept.
///
/// `SMSG_UPDATE_OBJECT` runs to tens of kilobytes and the fields that matter in
/// a hand-read are near the front. Keeping the whole of every packet would make
/// the ring a megabyte and the snapshot a real cost.
pub const CAPTURE_BODY_BYTES: usize = 512;

/// The recent-packet ring, shared by the two funnels that fill it.
///
/// Both the inbound framing and the outbound write run on the session thread,
/// so the ordering here is the wire's. It is shared rather than kept per-funnel
/// because two rings would have to be merged, and there is no timestamp
/// precise enough to merge them by.
///
/// Disarmed, this costs one relaxed atomic load per packet. Armed, it costs a
/// lock, a `Vec` of at most [`CAPTURE_BODY_BYTES`], and a name lookup.
#[derive(Debug, Default)]
pub struct Capture {
    /// The fast path. Read once per packet in each direction.
    armed: std::sync::atomic::AtomicBool,
    inner: Mutex<CaptureRing>,
}

#[derive(Debug, Default)]
struct CaptureRing {
    packets: std::collections::VecDeque<CapturedPacket>,
    /// Packets seen since the capture was armed, which is what says whether the
    /// ring is a window onto a longer stream or the whole of it.
    seen: u64,
    /// When the capture was armed. `None` while it has never been.
    since: Option<Instant>,
}

impl Capture {
    pub fn armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Arm or disarm. Arming clears the ring, so a capture always begins empty
    /// and its sequence numbers start at zero; disarming keeps what was caught,
    /// so the packets that led to a fault can be read after the fact.
    pub fn arm(&self, on: bool) {
        if on {
            let mut ring = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            ring.packets.clear();
            ring.seen = 0;
            ring.since = Some(Instant::now());
        }
        self.armed.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Record one packet, in whichever direction.
    pub fn note(&self, code: u32, body: &[u8], inbound: bool) {
        if !self.armed() {
            return;
        }
        let mut ring = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let at_ms = ring
            .since
            .map_or(0, |at| at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
        let sequence = ring.seen;
        ring.seen += 1;
        if ring.packets.len() >= CAPTURE_PACKETS {
            ring.packets.pop_front();
        }
        ring.packets.push_back(CapturedPacket {
            sequence,
            at_ms,
            code,
            name: Opcode::describe(code),
            inbound,
            body: body[..body.len().min(CAPTURE_BODY_BYTES)].to_vec(),
            length: body.len(),
        });
    }

    /// What the ring holds, oldest first, with how many packets have passed
    /// through it since it was armed.
    pub fn snapshot(&self) -> CaptureSnapshot {
        let ring = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        CaptureSnapshot {
            armed: self.armed(),
            seen: ring.seen,
            packets: ring.packets.iter().cloned().collect(),
        }
    }
}

/// The capture as a reader sees it — published into
/// [`crate::socket::session::SessionStatus`] behind an `Arc`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureSnapshot {
    pub armed: bool,
    /// Packets seen since the capture was armed. Greater than `packets.len()`
    /// once the ring has wrapped, which is what says the list is a window.
    pub seen: u64,
    /// The ring's contents, oldest first.
    pub packets: Vec<CapturedPacket>,
}

/// A live, authenticated world session.
///
/// Reads are buffered rather than issued straight against the socket. A live
/// client has to poll — check for traffic, run a tick, send movement, repeat —
/// and a `read_exact` that times out half way through a packet body has already
/// consumed those bytes and cannot put them back. Framing out of an internal
/// buffer means a timeout is just "nothing new yet".
///
/// The header cipher makes this stateful: each 4-byte header must be decrypted
/// exactly once, in order. [`WorldSession::take_packet`] therefore remembers a
/// header it has already decrypted while it waits for the matching body.
pub struct WorldSession {
    stream: TcpStream,
    crypt: HeaderCrypt,
    /// Bytes received but not yet framed.
    rx: Vec<u8>,
    /// Read cursor into `rx`; the prefix before it has been consumed.
    rx_pos: usize,
    /// A header that has been decrypted while its body is still in flight,
    /// as `(opcode, body length)`.
    pending: Option<(u32, usize)>,
    /// Current socket read timeout, so it is only re-set when it changes —
    /// `set_read_timeout` is a syscall and this is on the hot path.
    timeout: Option<Duration>,
    /// Whether the socket is polled rather than blocking. See
    /// [`WorldSession::set_nonblocking`].
    nonblocking: bool,
    /// **What has crossed the socket**, counted at the two funnels — see
    /// [`WireCounts`], and [`WorldSession::wire`], which is the only reader.
    wire: WireCounts,
    /// …and the same thing per opcode, for the half this side writes. Keyed by
    /// raw opcode rather than by name — see [`Traffic`], where the reason is.
    /// The inbound half is counted at the dispatch instead, where "did anything
    /// read it" is knowable — see `handler::PumpStats::traffic`.
    sent: std::collections::BTreeMap<u32, OpcodeFlow>,
    /// What the handshake saw, in order.
    ///
    /// **A library does not print.** This used to be five `println!`s, which put
    /// `[world] server seed = 0x…` on the Bevy client's stdout at every login
    /// and gave the CLI no way to format or suppress it. The facts are worth
    /// keeping — the seed and the pre-login chatter are the first things to look
    /// at when a handshake stalls — so they are recorded and the caller decides
    /// what to do with them. Failures are not in here: those are `io::Error`
    /// and already carry their own message.
    handshake: Vec<String>,
    /// The recent-packet ring both funnels fill — see [`Capture`].
    ///
    /// Shared with the dispatch, which fills the inbound half from the same
    /// ring so that the two directions interleave in wire order. Disarmed by
    /// default: it is a diagnostic, and one relaxed atomic load per packet is
    /// what it costs while nobody is looking.
    capture: std::sync::Arc<Capture>,
}

/// Compact `rx` once the consumed prefix passes this. Small enough that the
/// buffer never grows unboundedly, large enough that the memmove is rare.
const RX_COMPACT_THRESHOLD: usize = 64 * 1024;

/// Gap between attempts when polling a non-blocking socket.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// **How long to wait for the world server to accept the connection**, and
/// then for it to finish the handshake. See `socket::auth`'s pair, which are
/// the same two numbers for the same two reasons — this is the second half of
/// the same login and it had the same hole.
///
/// The handshake budget covers the *whole* of it rather than one read: the
/// server legitimately sends pre-login chatter (`SMSG_ADDON_INFO` and friends)
/// before `SMSG_AUTH_RESPONSE`, so a per-read timeout would be renewed by
/// traffic that is not progress.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Open a socket to the world server that **cannot block for ever**, the same
/// shape as `socket::auth`'s `open_logon`.
///
/// The address is the one the *realm row* advertised, so it is as likely to be
/// unreachable as any other: `realmd.realmlist` naming a host this machine
/// cannot see is the ordinary private-server misconfiguration, and without a
/// connect timeout it costs the operating system's own SYN budget — 21 seconds
/// on Windows — before the screen says anything.
fn open_world(addr: &str) -> io::Result<TcpStream> {
    let mut last = None;
    for resolved in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&resolved, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true).ok();
                return Ok(stream);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("{addr} resolved to no addresses"),
        )
    }))
}

impl WorldSession {
    /// Connect, perform the encrypted handshake, and return once the server has
    /// answered `SMSG_AUTH_RESPONSE` with `AUTH_OK`.
    pub fn connect(
        world_addr: &str,
        account: &str,
        session_key: [u8; 40],
    ) -> io::Result<WorldSession> {
        Self::connect_as(
            world_addr,
            account,
            session_key,
            &version::WOW_1_12_1_WIN_X86,
        )
    }

    pub fn connect_as(
        world_addr: &str,
        account: &str,
        session_key: [u8; 40],
        client: &ClientVersion,
    ) -> io::Result<WorldSession> {
        Self::connect_within(world_addr, account, session_key, client, HANDSHAKE_TIMEOUT)
    }

    /// …with the handshake budget stated, which is [`connect_as`] plus the one
    /// parameter a test needs: reproducing "the server accepted and said
    /// nothing" at fifteen seconds a go is not a unit test.
    ///
    /// [`connect_as`]: Self::connect_as
    pub fn connect_within(
        world_addr: &str,
        account: &str,
        session_key: [u8; 40],
        client: &ClientVersion,
        budget: Duration,
    ) -> io::Result<WorldSession> {
        let stream = open_world(world_addr)?;
        let mut session = WorldSession {
            stream,
            crypt: HeaderCrypt::new(),
            rx: Vec::new(),
            rx_pos: 0,
            pending: None,
            timeout: None,
            nonblocking: false,
            wire: WireCounts::default(),
            sent: std::collections::BTreeMap::new(),
            handshake: Vec::new(),
            capture: std::sync::Arc::new(Capture::default()),
        };

        // **The handshake is on a clock**, because every read below is a
        // blocking one and a server that accepts the socket and then goes quiet
        // would park this call for ever — see `socket::auth`'s `REPLY_TIMEOUT`,
        // which is the same bug one round trip earlier and the one that was
        // actually reported.
        let deadline = Instant::now() + budget;

        // ---- SMSG_AUTH_CHALLENGE (plaintext) — grab the server seed -------
        let challenge = session.recv_by(deadline)?;
        if !challenge.is(Opcode::SMSG_AUTH_CHALLENGE) {
            return Err(err(format!(
                "expected SMSG_AUTH_CHALLENGE, got {}",
                challenge.name()
            )));
        }
        let server_seed = Reader::new(&challenge.body).u32();
        session.handshake.push(format!("server seed {server_seed:#010x}"));

        // ---- CMSG_AUTH_SESSION (plaintext header) -------------------------
        let account_upper = account.to_uppercase();
        let client_seed: u32 = rand::random();
        let digest =
            session_digest(&account_upper, client_seed, server_seed, &session_key);

        // No addon block: vmangos validates auth before parsing addons, so its
        // absence does not block login. If the server ever drops the socket
        // immediately after this packet, a missing (zlib) addon block is the
        // first thing to suspect.
        session.send(
            Opcode::CMSG_AUTH_SESSION,
            &auth_session_body(client.build, &account_upper, client_seed, &digest),
        )?;

        // Encryption turns on immediately after the plaintext auth session.
        session.crypt.enable(session_key);

        // ---- wait for SMSG_AUTH_RESPONSE = AUTH_OK ------------------------
        loop {
            let pkt = session.recv_by(deadline)?;
            if !pkt.is(Opcode::SMSG_AUTH_RESPONSE) {
                // e.g. SMSG_ADDON_INFO or other pre-login chatter.
                session
                    .handshake
                    .push(format!("pre-login {} — {} bytes", pkt.name(), pkt.body.len()));
                continue;
            }
            let code = *pkt.body.first().ok_or_else(|| {
                err("SMSG_AUTH_RESPONSE had an empty body".to_string())
            })?;
            match WorldResult::from_code(code) {
                Some(WorldResult::Ok) => {
                    session.handshake.push("SMSG_AUTH_RESPONSE = Ok".into());
                    break;
                }
                Some(WorldResult::WaitQueue) => {
                    session.handshake.push("in login queue, waiting".into());
                }
                // **The byte travels with the sentence** — see
                // [`crate::codes::Refusal`]. A world refusal indexes its own
                // `AUTH_*` key, which is a different table from realmd's and is
                // exactly why the two codes are separate enums.
                other => {
                    return Err(crate::codes::Refusal::World(code).into_error(format!(
                        "auth response {} — not Ok",
                        other
                            .map(|r| format!("{r:?}"))
                            .unwrap_or_else(|| WorldResult::describe(code))
                    )));
                }
            }
        }

        Ok(session)
    }

    /// Frame and send a packet, encrypting the 6-byte client header.
    pub fn send(&mut self, opcode: Opcode, body: &[u8]) -> io::Result<()> {
        let size = (body.len() + 4) as u16; // opcode(4) + body
        let mut header = Vec::with_capacity(6);
        header.extend_from_slice(&size.to_be_bytes()); // big-endian size
        header.extend_from_slice(&opcode.code().to_le_bytes()); // little-endian opcode
        self.crypt.encrypt_send(&mut header);

        let mut out = header;
        out.extend_from_slice(body);
        // **Counted before the write rather than after it.** A short write is
        // retried inside `write_all` and a failed one takes the session down,
        // so "what this client asked the socket to carry" is the only number
        // with a single unambiguous moment — and it is the one a net read-out
        // is about.
        self.wire.packets_out += 1;
        self.wire.bytes_out += out.len() as u64;
        self.capture.note(opcode.code(), body, false);
        let flow = self.sent.entry(opcode.code()).or_insert(OpcodeFlow {
            count: 0,
            bytes: 0,
            handled: true,
        });
        flow.count += 1;
        flow.bytes += out.len() as u64;
        self.write_all(&out)
    }

    /// `write_all`, but tolerant of a non-blocking socket.
    ///
    /// Client packets are tens of bytes against a socket buffer measured in
    /// tens of kilobytes, so a short write is close to impossible — but
    /// `write_all` on a non-blocking socket turns one into a hard error, and a
    /// half-written *ciphered header* would desynchronise the stream for good.
    fn write_all(&mut self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            match self.stream.write(buf) {
                Ok(0) => return Err(err("world socket refused a write".into())),
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if is_timeout(&e) => std::thread::sleep(POLL_INTERVAL),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Bytes buffered but not yet framed.
    fn buffered(&self) -> usize {
        self.rx.len() - self.rx_pos
    }

    /// Frame one packet out of the receive buffer, if a whole one is there.
    fn take_packet(&mut self) -> Option<Packet> {
        if self.pending.is_none() {
            if self.buffered() < 4 {
                return None;
            }
            let mut header = self.rx[self.rx_pos..self.rx_pos + 4].to_vec();
            self.rx_pos += 4;
            self.crypt.decrypt_recv(&mut header);
            // size is big-endian and counts the opcode (2) + body.
            let size = u16::from_be_bytes([header[0], header[1]]) as usize;
            let code = u16::from_le_bytes([header[2], header[3]]) as u32;
            self.pending = Some((code, size.saturating_sub(2)));
        }

        let (code, body_len) = self.pending?;
        if self.buffered() < body_len {
            return None;
        }
        let body = self.rx[self.rx_pos..self.rx_pos + body_len].to_vec();
        self.rx_pos += body_len;
        self.pending = None;
        // **The one place every inbound packet is framed**, which is why the
        // wire counters live here rather than beside the dispatch: `recv_until`
        // throws packets away before `apply_packet` ever sees them, and the
        // four-byte header is not in any body length.
        self.wire.packets_in += 1;
        self.wire.bytes_in += (body.len() + 4) as u64;
        // Captured here rather than at the dispatch for the same reason the
        // counters are: `recv_until` discards packets before `apply_packet`
        // sees them, and a capture that missed those could not show what the
        // server sent during a handshake or a teleport.
        self.capture.note(code, &body, true);
        Some(Packet { code, body })
    }

    /// What has crossed this socket — see [`WireCounts`].
    pub fn wire(&self) -> WireCounts {
        self.wire
    }

    /// …and the outbound half of it opcode by opcode, named. Built rather than
    /// borrowed because the one caller is publishing it into a snapshot behind
    /// a lock, and because the names are resolved on the way out — see
    /// [`Traffic`].
    pub fn sent(&self) -> std::collections::BTreeMap<String, OpcodeFlow> {
        named(&self.sent)
    }

    /// The recent-packet ring, for the dispatch to fill the inbound half of and
    /// for the session loop to arm and read — see [`Capture`].
    pub fn capture(&self) -> std::sync::Arc<Capture> {
        std::sync::Arc::clone(&self.capture)
    }

    /// Switch between a blocking socket and a polled one.
    ///
    /// **Windows needs this.** A blocking `recv` with `SO_RCVTIMEO` that times
    /// out repeatedly — which is exactly what a 20 Hz client loop does on a
    /// quiet socket — eventually fails with `WSA_IO_PENDING` (os error 997) and
    /// takes the session down with it. Winsock only really supports that option
    /// for the occasional timeout, not as a polling mechanism.
    ///
    /// In non-blocking mode [`WorldSession::fill`] does the waiting itself, so
    /// `next_packet`'s contract is unchanged either way.
    pub fn set_nonblocking(&mut self, nonblocking: bool) -> io::Result<()> {
        if nonblocking {
            // Belt and braces: leaving a timeout set on a non-blocking socket
            // has no effect, but clearing it removes the option that caused the
            // trouble in the first place.
            self.stream.set_read_timeout(None)?;
            self.timeout = None;
        }
        self.stream.set_nonblocking(nonblocking)?;
        self.nonblocking = nonblocking;
        Ok(())
    }

    /// Pull whatever the socket has into the receive buffer.
    ///
    /// `wait` of `None` blocks. Returns the number of bytes read; zero means
    /// the wait elapsed with nothing to show for it, which is not an error —
    /// a quiet world server is the normal state.
    fn fill(&mut self, wait: Option<Duration>) -> io::Result<usize> {
        // Reclaim the consumed prefix rather than growing forever.
        if self.rx_pos > 0 && (self.rx_pos >= RX_COMPACT_THRESHOLD || self.rx_pos == self.rx.len())
        {
            self.rx.drain(..self.rx_pos);
            self.rx_pos = 0;
        }

        if self.nonblocking {
            return self.poll_until(wait);
        }

        if self.timeout != wait {
            // A zero timeout means "block forever" to the OS, so anything this
            // small is rounded up rather than silently turned into a hang.
            let effective = wait.map(|d| d.max(Duration::from_millis(1)));
            self.stream.set_read_timeout(effective)?;
            self.timeout = wait;
        }
        self.read_once()
    }

    /// Retry a non-blocking read until something arrives or `wait` runs out.
    fn poll_until(&mut self, wait: Option<Duration>) -> io::Result<usize> {
        let deadline = wait.map(|w| Instant::now() + w);
        loop {
            match self.read_once() {
                Ok(0) => {}
                other => return other,
            }
            match deadline {
                Some(d) if Instant::now() >= d => return Ok(0),
                _ => {}
            }
            // Short enough that a packet is noticed within a frame, long
            // enough not to spin a core on an idle socket.
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// One read attempt. `Ok(0)` means "nothing available", never end-of-file —
    /// a real EOF is an error, because the world stream does not end politely.
    fn read_once(&mut self) -> io::Result<usize> {
        let mut buf = [0u8; 16 * 1024];
        match self.stream.read(&mut buf) {
            Ok(0) => Err(err("world server closed the connection".into())),
            Ok(n) => {
                self.rx.extend_from_slice(&buf[..n]);
                Ok(n)
            }
            Err(e) if is_timeout(&e) => Ok(0),
            Err(e) => Err(e),
        }
    }

    /// Read one framed packet, blocking until it arrives.
    pub fn recv(&mut self) -> io::Result<Packet> {
        loop {
            if let Some(p) = self.take_packet() {
                return Ok(p);
            }
            self.fill(None)?;
        }
    }

    /// The next packet, or [`io::ErrorKind::TimedOut`] once `deadline` passes.
    ///
    /// **Only the handshake uses this**, and the deadline is the whole
    /// handshake rather than one read — see [`HANDSHAKE_TIMEOUT`]. A live
    /// session wants [`Self::next_packet`], whose quiet socket is an answer
    /// rather than a failure.
    fn recv_by(&mut self, deadline: Instant) -> io::Result<Packet> {
        loop {
            if let Some(p) = self.take_packet() {
                return Ok(p);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the world server accepted the connection and did not finish the handshake",
                ));
            }
            self.fill(Some(left))?;
        }
    }

    /// The next packet, waiting at most `wait` for one to turn up.
    ///
    /// `Ok(None)` means the socket was quiet — the expected outcome most times
    /// a live loop asks. Anything already buffered is returned without waiting.
    pub fn next_packet(&mut self, wait: Duration) -> io::Result<Option<Packet>> {
        if let Some(p) = self.take_packet() {
            return Ok(Some(p));
        }
        if self.fill(Some(wait))? == 0 {
            return Ok(None);
        }
        Ok(self.take_packet())
    }

    /// Read packets until one matching `want` arrives, **discarding the rest**.
    ///
    /// The discarding is the point to be careful about: those packets do not
    /// reach [`crate::socket::handler::apply_packet`] and are counted nowhere. That is
    /// tolerable only because the one caller is [`Self::char_enum`], which runs
    /// before `CMSG_PLAYER_LOGIN` — the client is not in the world yet, so
    /// nothing dropped here describes anything it is tracking. **Do not reach
    /// for this once the world burst has started**; it is a hole, and a packet
    /// swallowed there would be invisible.
    ///
    /// What was skipped is recorded rather than printed, on the same terms as
    /// the handshake: it is the first thing to look at when this hangs.
    pub fn recv_until(&mut self, want: Opcode) -> io::Result<Packet> {
        loop {
            let pkt = self.recv()?;
            if pkt.is(want) {
                return Ok(pkt);
            }
            self.handshake.push(format!(
                "waiting for {want}, skipped {} — {} bytes",
                pkt.name(),
                pkt.body.len()
            ));
        }
    }

    /// …and the same thing with a deadline on it, for a caller that is holding
    /// a **frame** open rather than a task.
    ///
    /// [`Self::recv_until`] blocks for ever, which is right for the two
    /// exchanges that run on the task pool and wrong for the one that runs on
    /// the render thread: `CMSG_CHAR_CREATE` is answered from a database write
    /// and the window is stopped until it comes back. See
    /// `crate::play::charcreate::create_character`, which says why that one is not a
    /// task.
    ///
    /// The elapsed budget is the *whole* wait rather than per packet, so a
    /// server that keeps sending something else cannot extend it indefinitely.
    pub fn recv_until_within(&mut self, want: Opcode, wait: Duration) -> io::Result<Packet> {
        match self.try_recv_until_within(want, wait)? {
            Some(pkt) => Ok(pkt),
            None => Err(err(format!("timed out waiting for {want}"))),
        }
    }

    /// …and the same wait again for a caller to whom **silence is an answer**.
    ///
    /// `Ok(None)` is the deadline passing; an `Err` is the socket. The two are
    /// one `io::Error` in [`Self::recv_until_within`] and that is right for
    /// every exchange the server always answers — but not for every exchange.
    /// `HandleCharDeleteOpcode` has **three** paths that `return` without
    /// sending anything at all (a character still loaded, a guid it cannot find,
    /// a guid belonging to another account), so a delete that goes quiet is the
    /// server declining rather than the connection going away, and treating it
    /// as the latter throws away a working character screen. See
    /// [`crate::play::charcreate::delete_character`].
    pub fn try_recv_until_within(
        &mut self,
        want: Opcode,
        wait: Duration,
    ) -> io::Result<Option<Packet>> {
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let Some(pkt) = self.next_packet(left)? else {
                continue;
            };
            if pkt.is(want) {
                return Ok(Some(pkt));
            }
            self.handshake.push(format!(
                "waiting for {want}, skipped {} — {} bytes",
                pkt.name(),
                pkt.body.len()
            ));
        }
    }

    /// Ask for the account's characters.
    pub fn char_enum(&mut self) -> io::Result<Vec<CharListEntry>> {
        self.send(Opcode::CMSG_CHAR_ENUM, &[])?;
        let pkt = self.recv_until(Opcode::SMSG_CHAR_ENUM)?;
        Ok(parse_char_enum(&pkt.body))
    }

    /// **Which world server this actually is**, as the socket sees it.
    ///
    /// Not the configured string: a realmlist line and a realm-list entry can
    /// name one machine two ways — `localhost` against `127.0.0.1` — and
    /// anything keyed by the *name* would then keep two of whatever it is
    /// keying. `None` if the socket cannot say, which
    /// on a live connection it always can.
    pub fn peer_address(&self) -> Option<String> {
        self.stream.peer_addr().ok().map(|a| a.to_string())
    }

    /// What the handshake saw, for a caller that wants to print it.
    pub fn handshake_log(&self) -> &[String] {
        &self.handshake
    }

    /// Enter the world as a character. The server replies with a burst of
    /// packets, the important ones being `SMSG_UPDATE_OBJECT` describing the
    /// player and everything nearby.
    pub fn player_login(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_PLAYER_LOGIN, &player_login_body(guid))
    }

    /// Send everything a handler queued while the world lock was held.
    ///
    /// See [`crate::socket::handler`]: a reply is *collected* rather than written,
    /// because a socket write under that lock blocks every reader on network
    /// latency. This is where it actually goes out, and the caller has released
    /// the lock by then.
    pub fn flush(&mut self, replies: &mut Replies) -> io::Result<()> {
        for (opcode, body) in replies.take() {
            self.send(opcode, &body)?;
        }
        Ok(())
    }

    /// Read packets for up to `budget`, folding object updates into `world`.
    ///
    /// Returns once the server goes quiet for `idle_timeout` or the budget
    /// expires — the world stream never "ends", so a caller-supplied stopping
    /// condition is mandatory.
    ///
    /// **A snapshot still answers the packets that must be answered.** It has no
    /// [`crate::socket::handler::LocalState`] — there is no simulation here to resync —
    /// but a teleport left unacknowledged holds the *session*, not just the
    /// position, so the replies go out. Before the dispatch was unified this
    /// path did not handle those packets at all.
    pub fn pump(
        &mut self,
        world: &mut ObjectManager,
        budget: Duration,
        idle_timeout: Duration,
    ) -> io::Result<PumpStats> {
        let deadline = Instant::now() + budget;
        let mut stats = PumpStats::default();
        let mut replies = Replies::default();

        while Instant::now() < deadline {
            // A quiet socket is the normal exit, not a failure.
            let Some(pkt) = self.next_packet(idle_timeout)? else {
                break;
            };
            apply_packet(
                &mut Incoming {
                    world,
                    stats: &mut stats,
                    replies: &mut replies,
                    local: None,
                },
                &pkt,
            );
            self.flush(&mut replies)?;
        }
        Ok(stats)
    }

    /// Ask what a creature entry actually is. The answer arrives asynchronously
    /// as `SMSG_CREATURE_QUERY_RESPONSE` and is picked up by [`Self::pump`],
    /// so send a batch and then pump once rather than blocking per entry.
    pub fn query_creature(&mut self, entry: u32, guid: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_CREATURE_QUERY,
            &query::creature_query_body(entry, guid),
        )
    }

    /// Resolve every unit entry currently known but unnamed.
    ///
    /// Returns how many queries were sent; the caller must pump afterwards to
    /// collect the replies.
    pub fn request_unknown_creatures(&mut self, world: &ObjectManager) -> io::Result<usize> {
        let pending = world.unresolved_creature_entries();
        for (entry, guid) in &pending {
            self.query_creature(*entry, *guid)?;
        }
        Ok(pending.len())
    }

    /// The same for the items players are wearing, whose *appearance* only the
    /// server can name — `Item.dbc` is not in the 1.12 archives.
    pub fn request_unknown_items(&mut self, world: &ObjectManager) -> io::Result<usize> {
        let pending = world.unresolved_item_entries();
        for entry in &pending {
            self.send(
                Opcode::CMSG_ITEM_QUERY_SINGLE,
                &query::item_query_body(*entry, 0),
            )?;
        }
        Ok(pending.len())
    }

    /// The same for the *players* in view, who are named by GUID rather than by
    /// entry — they have no template to share.
    pub fn request_unknown_players(&mut self, world: &ObjectManager) -> io::Result<usize> {
        let pending = world.unresolved_player_guids();
        for guid in &pending {
            self.send(Opcode::CMSG_NAME_QUERY, &query::name_query_body(*guid))?;
        }
        Ok(pending.len())
    }

    /// The same for game objects — chests, doors, mailboxes, campfires.
    pub fn request_unknown_gameobjects(&mut self, world: &ObjectManager) -> io::Result<usize> {
        let pending = world.unresolved_gameobject_entries();
        for (entry, guid) in &pending {
            self.send(
                Opcode::CMSG_GAMEOBJECT_QUERY,
                &query::gameobject_query_body(*entry, *guid),
            )?;
        }
        Ok(pending.len())
    }

    /// Round-trip a keepalive.
    pub fn ping(&mut self, sequence: u32, latency: u32) -> io::Result<()> {
        self.send(Opcode::CMSG_PING, &ping_body(sequence, latency))
    }

    /// Tell the server which unit this client is driving.
    ///
    /// **Movement does not work without this.** `Player::GetConfirmedMover`
    /// opens with `if (!m_session->GetClientMoverGuid()) return nullptr;` — the
    /// comment beside it reads "no mover client side, is this a fake client?" —
    /// and `HandleMovementOpcodes` returns immediately on a null mover. Every
    /// `MSG_MOVE_*` is then dropped in silence: no error, no kick, nothing moves.
    ///
    /// The GUID is a plain u64 here, not the packed form (`recvData >> guid`
    /// on an `ObjectGuid` reads `uint64` in 1.12).
    pub fn set_active_mover(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_SET_ACTIVE_MOVER, &plain_guid_body(guid))
    }

    /// Send one `MSG_MOVE_*` packet.
    ///
    /// Unlike the server's own relay of these packets, the client's copy has no
    /// GUID prefix — the server already knows who is talking.
    pub fn send_movement(&mut self, opcode: Opcode, info: &MovementInfo) -> io::Result<()> {
        self.send(opcode, &info.to_bytes())
    }

    /// Start swinging at a unit, and stop again.
    ///
    /// `HandleAttackSwingOpcode` is `recv_data >> guid` on an `ObjectGuid`, so a
    /// plain u64 and not the packed form — the same asymmetry as
    /// `CMSG_SET_ACTIVE_MOVER` and `CMSG_NAME_QUERY`. No selection is needed
    /// first; the handler resolves the GUID against the map itself.
    ///
    /// This exists to make the *server* fight back. A creature that has a victim
    /// is the only way to see the packets an attacking unit generates — a chase
    /// spline, `UPDATEFLAG_MELEE_ATTACKING`, `SMSG_ATTACKERSTATEUPDATE` — and
    /// they cannot be observed by walking around.
    pub fn attack_swing(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_ATTACKSWING, &plain_guid_body(guid))
    }

    /// **`CMSG_RESET_INSTANCES`** — an empty body, and the opcode is the whole
    /// message.
    ///
    /// `HandleResetInstancesOpcode` reads nothing at all: it resets the group's
    /// bindings when the caller leads one and the player's otherwise. There is
    /// no acknowledgement; the only thing that can come back is
    /// `SMSG_INSTANCE_RESET_FAILED`.
    pub fn reset_instances(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_RESET_INSTANCES, &[])
    }

    pub fn attack_stop(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_ATTACKSTOP, &[])
    }

    /// **Stop the ranged auto-repeat** — Auto Shot, a wand's Shoot.
    ///
    /// An empty body: `HandleCancelAutoRepeatSpellOpcode` reads nothing at all
    /// and is `GetMover()->InterruptSpell(CURRENT_AUTOREPEAT_SPELL)`, so the
    /// opcode *is* the message, the same way each of the five
    /// `SMSG_ATTACKSWING_*` refusals is.
    ///
    /// **There is no matching "start".** One auto-repeat spell is begun by an
    /// ordinary `CMSG_CAST_SPELL` — `Spell::prepare` sees
    /// `IsAutoRepeatRangedSpell()` and files it in `CURRENT_AUTOREPEAT_SPELL`
    /// instead of casting it, and `Unit::_UpdateAutoRepeatSpell` then fires a
    /// triggered copy on the ranged attack timer for as long as it stands. So
    /// the loop's two ends are asymmetric on the wire: a cast starts it and
    /// this stops it, and without this a client can only ever turn Auto Shot
    /// **on**.
    pub fn cancel_auto_repeat(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_CANCEL_AUTO_REPEAT_SPELL, &[])
    }

    /// **Draw or put away the weapons** — and the direction of this packet is
    /// the whole point of it.
    ///
    /// `UNIT_FIELD_BYTES_2` byte 0 reads like a field the server owns. It is
    /// not: `HandleSetSheathedOpcode` is the *only* thing in vmangos that writes
    /// a `Player`'s, so for our own character the update field is an **echo of
    /// this packet** and never its source. A client that waits for the server to
    /// draw its weapon waits for ever, which is exactly how this client came to
    /// fight everything in the game with its fists. `vale_assets::look::sheath` is
    /// where the decision is made, and when.
    ///
    /// `u32 sheathed`, and vmangos drops the packet silently for anything at or
    /// above [`MAX_SHEATH_STATE`] — so an out-of-range value is not refused, it
    /// just leaves the two ends disagreeing. Clamped by the caller.
    pub fn set_sheathed(&mut self, state: u32) -> io::Result<()> {
        self.send(Opcode::CMSG_SETSHEATHED, &set_sheathed_body(state))
    }

    /// Tell the server which unit is selected — **a plain u64, like the two
    /// above it**.
    ///
    /// This does not *make* a selection. The client's own target is decided the
    /// instant the player clicks and the ring appears with no round trip; what
    /// this buys is everything the server does with knowing: it writes the guid
    /// into our `UNIT_FIELD_TARGET` so other people see who we are looking at,
    /// makes the target's faction visible on the reputation list, drops combo
    /// points when a rogue or druid switches, and re-aims an auto-shot.
    /// `HandleSetSelectionOpcode` reads `recv_data >> guid` on an `ObjectGuid`.
    ///
    /// Zero is a legitimate value and means "nothing selected".
    pub fn set_selection(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_SET_SELECTION, &plain_guid_body(guid))
    }

    /// Cast a spell, at a unit or at nobody.
    ///
    /// **The target block is a mask and the mask may be empty.** See
    /// [`crate::play::spells::CastTarget`]: a spell that names its own targets — every
    /// self-buff in the game — is sent with `TARGET_FLAG_SELF`, which is zero
    /// and carries no guid, and shipping the current selection with one instead
    /// is how Ice Armor comes back "Invalid target".
    ///
    /// The server answers `SMSG_CAST_RESULT` either way, and only then the
    /// ordinary `SMSG_SPELL_START`/`SMSG_SPELL_GO` that everybody else's cast
    /// produces.
    pub fn cast_spell(
        &mut self,
        spell_id: u32,
        target: crate::play::spells::CastTarget,
    ) -> io::Result<()> {
        self.send(
            Opcode::CMSG_CAST_SPELL,
            &crate::play::spells::cast_spell_body(spell_id, target),
        )
    }

    /// Stop a cast that is still winding up. `u32 spellId` and nothing else.
    pub fn cancel_cast(&mut self, spell_id: u32) -> io::Result<()> {
        self.send(
            Opcode::CMSG_CANCEL_CAST,
            &crate::play::spells::cancel_cast_body(spell_id),
        )
    }

    /// Drop a buff we are carrying — `BuffButton_OnClick`'s right-click.
    ///
    /// See [`crate::play::spells::cancel_aura_body`] for why this takes a spell id
    /// where every other aura packet is slot-keyed, and for what the server does
    /// with one it will not honour (nothing, silently).
    pub fn cancel_aura(&mut self, spell_id: u32) -> io::Result<()> {
        self.send(
            Opcode::CMSG_CANCEL_AURA,
            &crate::play::spells::cancel_aura_body(spell_id),
        )
    }

    /// **Remember what is on this button** — `CMSG_SET_ACTION_BUTTON`.
    ///
    /// The one packet in the game whose whole content is a client decision: the
    /// bar is the client's state and this asks the server to store it for the
    /// next login. It is answered with nothing at all, so there is no echo to
    /// wait for and no failure to report — see
    /// [`crate::play::spells::set_action_button_body`], and note that a *removal* is
    /// this same packet with a zero word rather than a different one.
    ///
    /// The slot is **zero-based**, matching `SMSG_ACTION_BUTTONS`' own indexing.
    pub fn set_action_button(&mut self, slot: u8, packed: u32) -> io::Result<()> {
        self.send(
            Opcode::CMSG_SET_ACTION_BUTTON,
            &crate::play::spells::set_action_button_body(slot, packed),
        )
    }

    /// **Remember which extra action bars are on** — `CMSG_SET_ACTIONBAR_TOGGLES`.
    ///
    /// The same shape as [`Self::set_action_button`] and for the same reason —
    /// the layout is the client's and the server is only being told — but with
    /// one difference worth knowing: this one *does* come back, as
    /// `PLAYER_FIELD_BYTES` byte 2 in the next values block. See
    /// [`crate::play::spells::set_actionbar_toggles_body`].
    pub fn set_actionbar_toggles(&mut self, mask: u8) -> io::Result<()> {
        self.send(
            Opcode::CMSG_SET_ACTIONBAR_TOGGLES,
            &crate::play::spells::set_actionbar_toggles_body(mask),
        )
    }

    /// **Right-click something that is carried** — `CMSG_USE_ITEM`.
    ///
    /// The pair is the *server's* numbering, not the interface's; see
    /// [`crate::play::items::server_container_slot`], which is the one place the two
    /// are crossed. `spell_index` names which of the prototype's five spell
    /// blocks fires — see [`crate::play::items::use_item_body`].
    pub fn use_item(
        &mut self,
        bag: u8,
        slot: u8,
        spell_index: u8,
        target: crate::play::spells::CastTarget,
    ) -> io::Result<()> {
        self.send(
            Opcode::CMSG_USE_ITEM,
            &crate::play::items::use_item_body(bag, slot, spell_index, target),
        )
    }

    /// **The quest conversation**, one method for eight opcodes.
    ///
    /// The opcode is chosen here rather than at the caller for the reason
    /// `move_item` gives: which one to send is arithmetic on the verb, and a
    /// caller that picked would be a second place that had to know the mapping.
    pub fn quest(&mut self, verb: crate::socket::session::QuestVerb) -> io::Result<()> {
        use crate::socket::session::QuestVerb as V;
        let (opcode, body) = match verb {
            V::Status(guid) => (
                Opcode::CMSG_QUESTGIVER_STATUS_QUERY,
                crate::play::quest::guid_body(guid),
            ),
            V::Hello(guid) => (Opcode::CMSG_QUESTGIVER_HELLO, crate::play::quest::guid_body(guid)),
            V::Details { guid, quest_id } => (
                Opcode::CMSG_QUESTGIVER_QUERY_QUEST,
                crate::play::quest::quest_body(guid, quest_id),
            ),
            V::Accept { guid, quest_id } => (
                Opcode::CMSG_QUESTGIVER_ACCEPT_QUEST,
                crate::play::quest::quest_body(guid, quest_id),
            ),
            V::Complete { guid, quest_id } => (
                Opcode::CMSG_QUESTGIVER_COMPLETE_QUEST,
                crate::play::quest::quest_body(guid, quest_id),
            ),
            V::RequestReward { guid, quest_id } => (
                Opcode::CMSG_QUESTGIVER_REQUEST_REWARD,
                crate::play::quest::quest_body(guid, quest_id),
            ),
            V::ChooseReward {
                guid,
                quest_id,
                choice,
            } => (
                Opcode::CMSG_QUESTGIVER_CHOOSE_REWARD,
                crate::play::quest::choose_reward_body(guid, quest_id, choice),
            ),
            V::Query(quest_id) => (
                Opcode::CMSG_QUEST_QUERY,
                crate::play::quest::quest_query_body(quest_id),
            ),
            V::Abandon(slot) => (
                Opcode::CMSG_QUESTLOG_REMOVE_QUEST,
                crate::play::quest::abandon_body(slot),
            ),
        };
        self.send(opcode, &body)
    }

    /// **Use a game object** — `CMSG_GAMEOBJ_USE`, one guid, no reply.
    ///
    /// See [`crate::play::object`] for what comes of it, which is never an
    /// answer to this packet: a door changes its update field, a chest opens a
    /// loot window, a vein starts a gathering cast, and a locked anything says
    /// so through the message table.
    pub fn use_object(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_GAMEOBJ_USE, &crate::play::object::use_body(guid))
    }

    /// **The gossip and vendor conversation**, one method for seven opcodes —
    /// the same shape as [`WorldSession::quest`] and for the same reason.
    pub fn npc(&mut self, verb: crate::socket::session::NpcVerb) -> io::Result<()> {
        use crate::socket::session::NpcVerb as V;
        let (opcode, body) = match verb {
            V::GossipHello(guid) => (Opcode::CMSG_GOSSIP_HELLO, crate::play::gossip::guid_body(guid)),
            V::GossipSelect { guid, option } => (
                Opcode::CMSG_GOSSIP_SELECT_OPTION,
                crate::play::gossip::select_option_body(guid, option),
            ),
            V::TextQuery { text_id, guid } => (
                Opcode::CMSG_NPC_TEXT_QUERY,
                crate::play::gossip::npc_text_query_body(text_id, guid),
            ),
            V::BinderActivate(guid) => (
                Opcode::CMSG_BINDER_ACTIVATE,
                crate::play::bindpoint::binder_activate_body(guid),
            ),
            V::PageTextQuery { page_id, guid } => (
                Opcode::CMSG_PAGE_TEXT_QUERY,
                crate::play::pagetext::page_text_query_body(page_id, guid),
            ),
            V::ListInventory(guid) => {
                (Opcode::CMSG_LIST_INVENTORY, crate::play::gossip::guid_body(guid))
            }
            V::Buy {
                vendor,
                entry,
                count,
            } => (
                Opcode::CMSG_BUY_ITEM,
                crate::play::gossip::buy_item_body(vendor, entry, count),
            ),
            V::Sell {
                vendor,
                item,
                count,
            } => (
                Opcode::CMSG_SELL_ITEM,
                crate::play::gossip::sell_item_body(vendor, item, count),
            ),
            V::Buyback { vendor, wire_slot } => (
                Opcode::CMSG_BUYBACK_ITEM,
                crate::play::gossip::buyback_body(vendor, wire_slot),
            ),
            V::Repair { vendor, item } => (
                Opcode::CMSG_REPAIR_ITEM,
                crate::play::gossip::repair_item_body(vendor, item),
            ),
            // …and the trainer's two, which share this family's bare-guid hello.
            V::TrainerList(guid) => {
                (Opcode::CMSG_TRAINER_LIST, crate::play::gossip::guid_body(guid))
            }
            V::TrainerBuy { trainer, spell } => (
                Opcode::CMSG_TRAINER_BUY_SPELL,
                crate::play::trainer::buy_spell_body(trainer, spell),
            ),
            // …and the stable master's five, four of which are the bare guid
            // again and two of which add a pet number. See
            // [`crate::play::stable`].
            V::StableList(guid) => (
                Opcode::MSG_LIST_STABLED_PETS,
                crate::play::stable::list_stabled_pets_body(guid),
            ),
            V::StablePet(guid) => (
                Opcode::CMSG_STABLE_PET,
                crate::play::stable::stable_pet_body(guid),
            ),
            V::UnstablePet { stable, pet_number } => (
                Opcode::CMSG_UNSTABLE_PET,
                crate::play::stable::unstable_pet_body(stable, pet_number),
            ),
            V::StableSwap { stable, pet_number } => (
                Opcode::CMSG_STABLE_SWAP_PET,
                crate::play::stable::swap_stable_pet_body(stable, pet_number),
            ),
            V::BuyStableSlot(guid) => (
                Opcode::CMSG_BUY_STABLE_SLOT,
                crate::play::stable::buy_stable_slot_body(guid),
            ),
            // …and the banker's two, both the bare guid. See [`crate::play::bank`].
            V::BankerActivate(guid) => (
                Opcode::CMSG_BANKER_ACTIVATE,
                crate::play::bank::banker_activate_body(guid),
            ),
            V::BuyBankSlot(guid) => (
                Opcode::CMSG_BUY_BANK_SLOT,
                crate::play::bank::buy_bank_slot_body(guid),
            ),
        };
        self.send(opcode, &body)
    }

    /// **Everything a player can tell a pet**, one method for nine opcodes —
    /// the same shape as [`WorldSession::npc`].
    ///
    /// By reference rather than by value because two of the nine carry a heap
    /// allocation: a rename carries a name and a set-action carries a list.
    pub fn pet(&mut self, verb: &crate::socket::session::PetVerb) -> io::Result<()> {
        use crate::play::pet;
        use crate::socket::session::PetVerb as V;
        let (opcode, body) = match verb {
            V::Action { pet: guid, data, target } => (
                Opcode::CMSG_PET_ACTION,
                pet::pet_action_body(*guid, *data, *target),
            ),
            V::NameQuery { pet_number, pet: guid } => (
                Opcode::CMSG_PET_NAME_QUERY,
                pet::pet_name_query_body(*pet_number, *guid),
            ),
            V::SetAction { pet: guid, moves } => (
                Opcode::CMSG_PET_SET_ACTION,
                pet::pet_set_action_body(*guid, moves),
            ),
            V::Autocast { pet: guid, spell_id, on } => (
                Opcode::CMSG_PET_SPELL_AUTOCAST,
                pet::pet_spell_autocast_body(*guid, *spell_id, *on),
            ),
            // Three opcodes and one body — see [`crate::play::pet::pet_guid_body`].
            V::StopAttack(guid) => (Opcode::CMSG_PET_STOP_ATTACK, pet::pet_guid_body(*guid)),
            V::Abandon(guid) => (Opcode::CMSG_PET_ABANDON, pet::pet_guid_body(*guid)),
            V::Unlearn(guid) => (Opcode::CMSG_PET_UNLEARN, pet::pet_guid_body(*guid)),
            V::Rename { pet: guid, name } => (
                Opcode::CMSG_PET_RENAME,
                pet::pet_rename_body(*guid, name),
            ),
            V::CancelAura { pet: guid, spell_id } => (
                Opcode::CMSG_PET_CANCEL_AURA,
                pet::pet_cancel_aura_body(*guid, *spell_id),
            ),
        };
        self.send(opcode, &body)
    }

    /// **The flight master's three verbs** — see
    /// [`crate::socket::session::TaxiVerb`] and [`crate::play::taxi`], which owns the bodies.
    ///
    /// By reference for the same reason [`WorldSession::party`] is: the express
    /// flight carries a route.
    pub fn taxi(&mut self, verb: &crate::socket::session::TaxiVerb) -> io::Result<()> {
        use crate::socket::session::TaxiVerb as V;
        let (opcode, body) = match verb {
            // Both openers are the bare guid every hello in this family is.
            V::QueryNodes(guid) => (
                Opcode::CMSG_TAXIQUERYAVAILABLENODES,
                crate::play::gossip::guid_body(*guid),
            ),
            V::NodeStatus(guid) => (
                Opcode::CMSG_TAXINODE_STATUS_QUERY,
                crate::play::gossip::guid_body(*guid),
            ),
            V::Activate { guid, from, to } => (
                Opcode::CMSG_ACTIVATETAXI,
                crate::play::taxi::activate_taxi_body(*guid, *from, *to),
            ),
            V::ActivateExpress { guid, cost, nodes } => (
                Opcode::CMSG_ACTIVATETAXIEXPRESS,
                crate::play::taxi::activate_taxi_express_body(*guid, *cost, nodes),
            ),
        };
        self.send(opcode, &body)
    }

    /// **The group's verbs, party and raid together** — see
    /// [`crate::socket::session::PartyVerb`] and [`crate::play::group`], which
    /// owns the bodies.
    ///
    /// By reference rather than by value, which is this family's one difference
    /// from the seven above it: four of the verbs carry a name.
    pub fn party(&mut self, verb: &crate::socket::session::PartyVerb) -> io::Result<()> {
        use crate::socket::session::PartyVerb as V;
        let (opcode, body) = match verb {
            V::Invite(name) => (Opcode::CMSG_GROUP_INVITE, crate::play::group::invite_body(name)),
            V::Accept => (Opcode::CMSG_GROUP_ACCEPT, crate::play::group::EMPTY.to_vec()),
            V::Decline => (Opcode::CMSG_GROUP_DECLINE, crate::play::group::EMPTY.to_vec()),
            // **Leaving and disbanding are one opcode** — see [`V::Leave`].
            V::Leave => (Opcode::CMSG_GROUP_DISBAND, crate::play::group::EMPTY.to_vec()),
            V::Uninvite(name) => (
                Opcode::CMSG_GROUP_UNINVITE,
                crate::play::group::uninvite_body(name),
            ),
            V::UninviteGuid(guid) => (
                Opcode::CMSG_GROUP_UNINVITE_GUID,
                crate::play::group::guid_body(*guid),
            ),
            V::SetLeader(guid) => (
                Opcode::CMSG_GROUP_SET_LEADER,
                crate::play::group::guid_body(*guid),
            ),
            V::RequestStats(guid) => (
                Opcode::CMSG_REQUEST_PARTY_MEMBER_STATS,
                crate::play::group::request_stats_body(*guid),
            ),
            // **Empty, like the three above it** — see [`V::ConvertToRaid`].
            V::ConvertToRaid => (
                Opcode::CMSG_GROUP_RAID_CONVERT,
                crate::play::group::EMPTY.to_vec(),
            ),
            V::ChangeSubGroup { name, subgroup } => (
                Opcode::CMSG_GROUP_CHANGE_SUB_GROUP,
                crate::play::group::change_sub_group_body(name, *subgroup),
            ),
            V::SwapSubGroup { name, swap_with } => (
                Opcode::CMSG_GROUP_SWAP_SUB_GROUP,
                crate::play::group::swap_sub_group_body(name, swap_with),
            ),
            V::SetAssistant { guid, assistant } => (
                Opcode::CMSG_GROUP_ASSISTANT_LEADER,
                crate::play::group::assistant_leader_body(*guid, *assistant),
            ),
            // **One opcode, told apart by its length** — see
            // [`crate::play::group::ReadyCheck`].
            V::StartReadyCheck => (
                Opcode::MSG_RAID_READY_CHECK,
                crate::play::group::EMPTY.to_vec(),
            ),
            V::AnswerReadyCheck(ready) => (
                Opcode::MSG_RAID_READY_CHECK,
                crate::play::group::ready_check_answer_body(*ready),
            ),
        };
        self.send(opcode, &body)
    }

    /// **The reputation panel's three** — see
    /// [`crate::socket::session::ReputationVerb`] and
    /// [`crate::play::reputation`], which owns the bodies.
    pub fn reputation(
        &mut self,
        verb: crate::socket::session::ReputationVerb,
    ) -> io::Result<()> {
        use crate::socket::session::ReputationVerb as V;
        let (opcode, body) = match verb {
            V::AtWar { reputation_list_id, at_war } => (
                Opcode::CMSG_SET_FACTION_ATWAR,
                crate::play::reputation::set_faction_at_war_body(reputation_list_id, at_war),
            ),
            V::Inactive { reputation_list_id, inactive } => (
                Opcode::CMSG_SET_FACTION_INACTIVE,
                crate::play::reputation::set_faction_inactive_body(reputation_list_id, inactive),
            ),
            V::Watched { reputation_list_id } => (
                Opcode::CMSG_SET_WATCHED_FACTION,
                crate::play::reputation::set_watched_faction_body(reputation_list_id),
            ),
        };
        self.send(opcode, &body)
    }

    /// **The social panel's six** — see
    /// [`crate::socket::session::SocialVerb`].
    ///
    /// By reference rather than by value because three of the six carry a string
    /// and one carries a whole search, and this is the one place they are turned
    /// into bytes.
    pub fn social(&mut self, verb: &crate::socket::session::SocialVerb) -> io::Result<()> {
        use crate::socket::session::SocialVerb as V;
        let (opcode, body) = match verb {
            V::List => (Opcode::CMSG_FRIEND_LIST, crate::play::social::friend_list_body()),
            V::AddFriend(name) => (
                Opcode::CMSG_ADD_FRIEND,
                crate::play::social::add_friend_body(name),
            ),
            V::DelFriend(guid) => (
                Opcode::CMSG_DEL_FRIEND,
                crate::play::social::del_friend_body(*guid),
            ),
            V::AddIgnore(name) => (
                Opcode::CMSG_ADD_IGNORE,
                crate::play::social::add_ignore_body(name),
            ),
            V::DelIgnore(guid) => (
                Opcode::CMSG_DEL_IGNORE,
                crate::play::social::del_ignore_body(*guid),
            ),
            V::Who(request) => (Opcode::CMSG_WHO, crate::play::social::who_body(request)),
        };
        self.send(opcode, &body)
    }

    /// **The chat frame's sixteen channel verbs** — see
    /// [`crate::socket::session::ChannelVerb`] and [`crate::play::channels`],
    /// which owns the bodies. Eight of them are one layout, name and player.
    pub fn channel(&mut self, verb: &crate::socket::session::ChannelVerb) -> io::Result<()> {
        use crate::play::channels as c;
        use crate::socket::session::ChannelVerb as V;
        let (opcode, body) = match verb {
            V::Join { name, password } => (Opcode::CMSG_JOIN_CHANNEL, c::join_body(name, password)),
            V::Leave(name) => (Opcode::CMSG_LEAVE_CHANNEL, c::leave_body(name)),
            V::List(name) => (Opcode::CMSG_CHANNEL_LIST, c::list_body(name)),
            V::Password { name, password } => {
                (Opcode::CMSG_CHANNEL_PASSWORD, c::password_body(name, password))
            }
            V::SetOwner { name, player } => {
                (Opcode::CMSG_CHANNEL_SET_OWNER, c::set_owner_body(name, player))
            }
            V::Owner(name) => (Opcode::CMSG_CHANNEL_OWNER, c::owner_body(name)),
            V::Moderator { name, player } => {
                (Opcode::CMSG_CHANNEL_MODERATOR, c::player_body(name, player))
            }
            V::Unmoderator { name, player } => {
                (Opcode::CMSG_CHANNEL_UNMODERATOR, c::player_body(name, player))
            }
            V::Mute { name, player } => (Opcode::CMSG_CHANNEL_MUTE, c::player_body(name, player)),
            V::Unmute { name, player } => {
                (Opcode::CMSG_CHANNEL_UNMUTE, c::player_body(name, player))
            }
            V::Invite { name, player } => {
                (Opcode::CMSG_CHANNEL_INVITE, c::player_body(name, player))
            }
            V::Kick { name, player } => (Opcode::CMSG_CHANNEL_KICK, c::player_body(name, player)),
            V::Ban { name, player } => (Opcode::CMSG_CHANNEL_BAN, c::player_body(name, player)),
            V::Unban { name, player } => (Opcode::CMSG_CHANNEL_UNBAN, c::player_body(name, player)),
            V::Announcements(name) => (Opcode::CMSG_CHANNEL_ANNOUNCEMENTS, c::toggle_body(name)),
            V::Moderate(name) => (Opcode::CMSG_CHANNEL_MODERATE, c::toggle_body(name)),
        };
        self.send(opcode, &body)
    }

    /// **The mail window's nine** — see [`crate::socket::session::MailVerb`] and
    /// [`crate::play::mail`], which owns the bodies.
    ///
    /// By reference because a send carries three strings. Five of the nine are
    /// the same twelve bytes under five different opcodes, which is the whole
    /// reason the match below looks repetitive: the opcode *is* the verb.
    /// **The trade window's ten verbs**, one method for ten opcodes — the
    /// same shape as [`WorldSession::mail`]. See [`crate::play::trade`].
    pub fn trade(&mut self, verb: crate::socket::session::TradeVerb) -> io::Result<()> {
        use crate::play::trade as t;
        use crate::socket::session::TradeVerb as V;
        let (opcode, body) = match verb {
            V::Initiate(guid) => (Opcode::CMSG_INITIATE_TRADE, t::initiate_trade_body(guid)),
            V::Begin => (Opcode::CMSG_BEGIN_TRADE, Vec::new()),
            V::Busy => (Opcode::CMSG_BUSY_TRADE, Vec::new()),
            V::Ignore => (Opcode::CMSG_IGNORE_TRADE, Vec::new()),
            V::Accept => (Opcode::CMSG_ACCEPT_TRADE, t::accept_trade_body()),
            V::Unaccept => (Opcode::CMSG_UNACCEPT_TRADE, Vec::new()),
            V::Cancel => (Opcode::CMSG_CANCEL_TRADE, Vec::new()),
            V::SetItem {
                trade_slot,
                bag,
                slot,
            } => (
                Opcode::CMSG_SET_TRADE_ITEM,
                t::set_trade_item_body(trade_slot, bag, slot),
            ),
            V::ClearItem { trade_slot } => {
                (Opcode::CMSG_CLEAR_TRADE_ITEM, t::clear_trade_item_body(trade_slot))
            }
            V::SetGold(copper) => (Opcode::CMSG_SET_TRADE_GOLD, t::set_trade_gold_body(copper)),
        };
        self.send(opcode, &body)
    }

    pub fn mail(&mut self, verb: &crate::socket::session::MailVerb) -> io::Result<()> {
        use crate::socket::session::MailVerb as V;
        let letter = |mailbox: u64, mail_id: u32| crate::play::mail::mail_id_body(mailbox, mail_id);
        let (opcode, body) = match verb {
            V::List(mailbox) => (
                Opcode::CMSG_GET_MAIL_LIST,
                crate::play::mail::get_mail_list_body(*mailbox),
            ),
            V::MarkAsRead { mailbox, mail_id } => {
                (Opcode::CMSG_MAIL_MARK_AS_READ, letter(*mailbox, *mail_id))
            }
            V::TakeMoney { mailbox, mail_id } => {
                (Opcode::CMSG_MAIL_TAKE_MONEY, letter(*mailbox, *mail_id))
            }
            V::TakeItem { mailbox, mail_id } => {
                (Opcode::CMSG_MAIL_TAKE_ITEM, letter(*mailbox, *mail_id))
            }
            V::Return { mailbox, mail_id } => (
                Opcode::CMSG_MAIL_RETURN_TO_SENDER,
                letter(*mailbox, *mail_id),
            ),
            V::Delete { mailbox, mail_id } => {
                (Opcode::CMSG_MAIL_DELETE, letter(*mailbox, *mail_id))
            }
            V::TakeText {
                mailbox,
                mail_id,
                template_id,
            } => (
                Opcode::CMSG_MAIL_CREATE_TEXT_ITEM,
                crate::play::mail::create_text_item_body(*mailbox, *mail_id, *template_id),
            ),
            V::TextQuery {
                item_text_id,
                mail_id,
            } => (
                Opcode::CMSG_ITEM_TEXT_QUERY,
                crate::play::mail::item_text_query_body(*item_text_id, *mail_id),
            ),
            V::Send(mail) => (
                Opcode::CMSG_SEND_MAIL,
                crate::play::mail::send_mail_body(mail),
            ),
            V::NextTime => (
                Opcode::MSG_QUERY_NEXT_MAIL_TIME,
                crate::play::mail::next_mail_time_body(),
            ),
        };
        self.send(opcode, &body)
    }

    /// **Spend a talent point** — `CMSG_LEARN_TALENT`.
    ///
    /// `rank` is **zero-based**, and the whole of what is checked before it is
    /// sent is that the talent is not already maxed —
    /// `vale_assets::tables::talent::TalentTree::learn` owns that gate,
    /// which is the client's own single test. Everything else is
    /// `Player::LearnTalent`'s, and a refusal comes back as silence: see
    /// [`crate::play::talents`].
    pub fn learn_talent(&mut self, talent_id: u32, rank: u32) -> io::Result<()> {
        self.send(
            Opcode::CMSG_LEARN_TALENT,
            &crate::play::talents::learn_talent_body(talent_id, rank),
        )
    }

    /// **Open the window on a body** — `CMSG_LOOT`.
    ///
    /// What comes back is `SMSG_LOOT_RESPONSE`, which is also how the server
    /// refuses; see [`crate::play::loot`].
    pub fn loot(&mut self, guid: u64) -> io::Result<()> {
        self.send(Opcode::CMSG_LOOT, &crate::play::loot::loot_body(guid))
    }

    /// …and close it — `CMSG_LOOT_RELEASE`.
    ///
    /// The guid is a formality: `HandleLootReleaseOpcode` reads it and throws it
    /// away in favour of its own record. The window shuts when
    /// `SMSG_LOOT_RELEASE_RESPONSE` comes back, not here.
    pub fn loot_release(&mut self, guid: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_LOOT_RELEASE,
            &crate::play::loot::loot_release_body(guid),
        )
    }

    /// …take one row — `CMSG_AUTOSTORE_LOOT_ITEM`, by the **server's** index.
    pub fn loot_item(&mut self, index: u8) -> io::Result<()> {
        self.send(
            Opcode::CMSG_AUTOSTORE_LOOT_ITEM,
            &crate::play::loot::autostore_loot_item_body(index),
        )
    }

    /// …and the coins — `CMSG_LOOT_MONEY`, which has no body at all.
    pub fn loot_money(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_LOOT_MONEY, &[])
    }

    /// …and vote in a group roll — `CMSG_LOOT_ROLL`.
    ///
    /// **Named by the body and the slot**, never by the `rollID` the interface
    /// uses: that id is a counter this client keeps and the server has never
    /// heard of it. See [`crate::play::lootroll`].
    pub fn loot_roll(
        &mut self,
        guid: u64,
        item_slot: u32,
        vote: crate::play::lootroll::RollVote,
    ) -> io::Result<()> {
        self.send(
            Opcode::CMSG_LOOT_ROLL,
            &crate::play::lootroll::loot_roll_body(guid, item_slot, vote),
        )
    }

    /// …and right-click something that is *worn* rather than used —
    /// `CMSG_AUTOEQUIP_ITEM`, which is the other half of the same click.
    pub fn auto_equip_item(&mut self, bag: u8, slot: u8) -> io::Result<()> {
        self.send(
            Opcode::CMSG_AUTOEQUIP_ITEM,
            &crate::play::items::auto_equip_body(bag, slot),
        )
    }

    /// **Open something that holds loot** — `CMSG_OPEN_ITEM`. See
    /// [`crate::play::items::open_item_body`], where the four refusals are.
    pub fn open_item(&mut self, bag: u8, slot: u8) -> io::Result<()> {
        self.send(
            Opcode::CMSG_OPEN_ITEM,
            &crate::play::items::open_item_body(bag, slot),
        )
    }

    /// **Across the bank counter** — `CMSG_AUTOSTORE_BANK_ITEM` for a source
    /// in the bank and `CMSG_AUTOBANK_ITEM` for one in the bags, which is the
    /// same fork `HandleAutoStoreBankItemOpcode` makes on the server. See
    /// [`crate::play::bank`].
    pub fn bank_item(&mut self, bag: u8, slot: u8) -> io::Result<()> {
        let opcode = if crate::play::items::is_bank_position(bag, slot) {
            Opcode::CMSG_AUTOSTORE_BANK_ITEM
        } else {
            Opcode::CMSG_AUTOBANK_ITEM
        };
        self.send(opcode, &crate::play::bank::bank_move_body(bag, slot))
    }

    /// …and the third thing a right-click on a bag square can be: ammo is
    /// *loaded*, not worn — `CMSG_SET_AMMO`. See
    /// [`crate::play::items::set_ammo_body`].
    pub fn set_ammo(&mut self, entry: u32) -> io::Result<()> {
        self.send(Opcode::CMSG_SET_AMMO, &crate::play::items::set_ammo_body(entry))
    }

    /// **Move an item**, in whichever of the five shapes the two ends call for.
    ///
    /// The opcode is chosen here rather than at the caller because the choice is
    /// arithmetic on the arguments — see [`crate::socket::session::Command::MoveItem`],
    /// which is where the table is:
    ///
    /// ```text
    /// no destination            CMSG_DESTROYITEM
    /// a count                   CMSG_SPLIT_ITEM
    /// a destination bag only    CMSG_AUTOSTORE_BAG_ITEM
    /// both ends in the player   CMSG_SWAP_INV_ITEM
    /// otherwise                 CMSG_SWAP_ITEM
    /// ```
    ///
    /// **The two swap bodies are the opposite way round on the wire**, which is
    /// the trap this function exists to make unrepeatable: `CMSG_SWAP_INV_ITEM`
    /// is source-then-destination and `CMSG_SWAP_ITEM` is
    /// destination-then-source, and getting it wrong moves the wrong item to the
    /// wrong place rather than erroring.
    pub fn move_item(
        &mut self,
        src_bag: u8,
        src_slot: u8,
        dst: Option<(u8, u8)>,
        count: Option<u8>,
    ) -> io::Result<()> {
        use crate::play::items::{SERVER_BAG_NONE, SERVER_SLOT_ANY};
        let Some((dst_bag, dst_slot)) = dst else {
            return self.send(
                Opcode::CMSG_DESTROYITEM,
                &crate::play::items::destroy_item_body(src_bag, src_slot, count.unwrap_or(0)),
            );
        };
        if let Some(count) = count {
            return self.send(
                Opcode::CMSG_SPLIT_ITEM,
                &crate::play::items::split_item_body(src_bag, src_slot, dst_bag, dst_slot, count),
            );
        }
        if dst_slot == SERVER_SLOT_ANY {
            return self.send(
                Opcode::CMSG_AUTOSTORE_BAG_ITEM,
                &crate::play::items::autostore_bag_item_body(src_bag, src_slot, dst_bag),
            );
        }
        if src_bag == SERVER_BAG_NONE && dst_bag == SERVER_BAG_NONE {
            return self.send(
                Opcode::CMSG_SWAP_INV_ITEM,
                &crate::play::items::swap_inv_item_body(src_slot, dst_slot),
            );
        }
        self.send(
            Opcode::CMSG_SWAP_ITEM,
            &crate::play::items::swap_item_body(dst_bag, dst_slot, src_bag, src_slot),
        )
    }

    /// **Ask to leave** — `CMSG_LOGOUT_REQUEST`, and **the body is empty**.
    ///
    /// What comes back is `SMSG_LOGOUT_RESPONSE`, which may refuse; see
    /// [`crate::play::logout`] for the three reasons and for why nothing here starts
    /// a timer of its own.
    pub fn logout_request(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_LOGOUT_REQUEST, &[])
    }

    /// …and take it back — `CMSG_LOGOUT_CANCEL`, empty as well.
    ///
    /// Worth sending even when this client thinks nothing is pending:
    /// `HandleLogoutCancelOpcode` is what stands the character back up and
    /// clears `UNIT_FLAG_STUNNED`, and a client that skipped it because its own
    /// record had lapsed would leave the character rooted where it sat.
    pub fn logout_cancel(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_LOGOUT_CANCEL, &[])
    }

    /// **Release the spirit** — `CMSG_REPOP_REQUEST`, and the body is empty.
    ///
    /// It is empty *on purpose*: `HandleRepopRequestOpcode`'s first line is a
    /// commented-out `read_skip<uint8>()` with the note "client crash" beside
    /// it, so the 5875 client sends nothing here and a byte would desynchronise
    /// the stream.
    pub fn repop_request(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_REPOP_REQUEST, &[])
    }

    /// **Where is the body?** — `MSG_CORPSE_QUERY`, empty, answered under the
    /// same opcode.
    pub fn corpse_query(&mut self) -> io::Result<()> {
        self.send(Opcode::MSG_CORPSE_QUERY, &[])
    }

    /// **Stand up on the body** — `CMSG_RECLAIM_CORPSE`, carrying our own guid,
    /// which the server reads and then never uses. See
    /// [`crate::play::death::reclaim_corpse_body`].
    pub fn reclaim_corpse(&mut self, guid: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_RECLAIM_CORPSE,
            &crate::play::death::reclaim_corpse_body(guid),
        )
    }

    /// Answer a resurrection offer — `CMSG_RESURRECT_RESPONSE`.
    pub fn resurrect_response(&mut self, caster: u64, accept: bool) -> io::Result<()> {
        self.send(
            Opcode::CMSG_RESURRECT_RESPONSE,
            &crate::play::death::resurrect_response_body(caster, accept),
        )
    }

    /// Answer a duel — `CMSG_DUEL_ACCEPTED` or `CMSG_DUEL_CANCELLED`.
    pub fn duel_answer(&mut self, arbiter: u64, accept: bool) -> io::Result<()> {
        let opcode = if accept {
            Opcode::CMSG_DUEL_ACCEPTED
        } else {
            Opcode::CMSG_DUEL_CANCELLED
        };
        self.send(opcode, &crate::play::duel::duel_answer_body(arbiter))
    }

    /// Accept a summon — `CMSG_SUMMON_RESPONSE`.
    pub fn summon_response(&mut self, summoner: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_SUMMON_RESPONSE,
            &crate::play::summon::summon_response_body(summoner),
        )
    }

    /// Ask for the played time — `CMSG_PLAYED_TIME`, bodiless.
    pub fn request_played_time(&mut self) -> io::Result<()> {
        self.send(Opcode::CMSG_PLAYED_TIME, &[])
    }

    /// Take the spirit healer's offer — `CMSG_SPIRIT_HEALER_ACTIVATE`.
    pub fn spirit_healer_activate(&mut self, healer: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_SPIRIT_HEALER_ACTIVATE,
            &crate::play::death::spirit_healer_activate_body(healer),
        )
    }

    /// Play a text emote — `/dance`, `/wave`, `/bow`.
    ///
    /// **This exists for the same reason [`Self::attack_swing`] does: to make
    /// the server say something it otherwise only says about other people.**
    /// `SMSG_EMOTE` is the one packet in the game that comes close to naming an
    /// animation, and without a way to provoke one, "this client does not
    /// animate emotes" and "nobody emoted" are the same empty screen.
    ///
    /// See [`text_emote_body`] for what the three fields are.
    pub fn text_emote(&mut self, text_emote: u32, emote_num: u32, target: u64) -> io::Result<()> {
        self.send(
            Opcode::CMSG_TEXT_EMOTE,
            &crate::play::emotetext::text_emote_body(text_emote, emote_num, target),
        )
    }

    /// Say something — or, with a leading `.`, ask the server to do something.
    ///
    /// The language is a parameter and not a default because the server refuses
    /// both `LANG_UNIVERSAL` and any language the character has not learned, and
    /// both refusals are silent. See [`crate::play::chat`].
    pub fn say(
        &mut self,
        kind: crate::play::chat::ChatType,
        language: crate::play::chat::Language,
        target: Option<&str>,
        text: &str,
    ) -> io::Result<()> {
        self.send(
            Opcode::CMSG_MESSAGECHAT,
            &crate::play::chat::message_body(kind, language, target, text),
        )
    }
}

// ---------------------------------------------------------------------------
// Outbound bodies
//
// **Every packet body this client sends is built by a free function**, and the
// `WorldSession` methods above are one-line conveniences over
// `send(opcode, &body)`. There used to be two conventions — bodies inlined into
// eight methods with a `Writer`, and four `*_body()` functions in `query.rs` —
// and both were used inside a single loop in `resolve_names`, so the next one
// was a coin flip.
//
// It is not only tidiness. A body welded to a socket cannot be unit-tested, and
// **the acks cannot be built by a method at all**: `crate::socket::handler` composes
// them with no socket to hand, because a reply is queued under the world lock
// and sent after it is released. The rule is that a body lives beside the thing
// it is about — movement bodies in `movement.rs` next to their parsers, query
// bodies in `query.rs`, and the session's own here.
// ---------------------------------------------------------------------------

/// `CMSG_AUTH_SESSION`: the build we claim to be, the account, our seed and the
/// digest proving we hold the session key.
fn auth_session_body(build: u16, account_upper: &str, client_seed: u32, digest: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(build as u32)
        .u32(0) // serverId
        .cstring(account_upper)
        .u32(client_seed)
        .bytes(digest);
    w.buf
}

/// `CMSG_PLAYER_LOGIN`: the guid, low word first.
fn player_login_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32((guid & 0xFFFF_FFFF) as u32).u32((guid >> 32) as u32);
    w.buf
}

/// `CMSG_PING`: a sequence number the server echoes, and our current latency.
fn ping_body(sequence: u32, latency: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(sequence).u32(latency);
    w.buf
}

/// A body that is one **plain** u64 guid: `CMSG_SET_ACTIVE_MOVER`,
/// `CMSG_ATTACKSWING` and `CMSG_SET_SELECTION`.
///
/// Both read `recvData >> guid` on an `ObjectGuid`, which is `uint64` in 1.12 —
/// **not** the packed form the same guid takes inside an update block. GUIDs are
/// packed except where they are not, and these are two of the places they are
/// not; sending the packed form here is a body the server reads as garbage.
pub(crate) fn plain_guid_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `MAX_SHEATH_STATE` — vmangos' bound on the value in `CMSG_SETSHEATHED`.
///
/// A constant rather than a literal for the usual reason, and one specific one:
/// `HandleSetSheathedOpcode` **returns without answering** for anything at or
/// above it. There is no refusal packet, so a client that sends 3 has simply
/// desynchronised itself from the server with nothing on the wire to say so.
pub const MAX_SHEATH_STATE: u8 = 3;

/// Body for `CMSG_SETSHEATHED`: one `u32`, and that is the whole packet.
///
/// A `u32` for a value whose whole range is 0..2 is worth stating, because the
/// obvious guess is the `u8` the update field holds — and vmangos reads
/// `recv_data >> sheathed` into a `uint32`, so a one-byte body under-runs and is
/// dropped with no error at either end.
pub fn set_sheathed_body(state: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(state);
    w.buf
}

/// Body for `CMSG_TEXT_EMOTE`: `u32 textEmote`, `u32 emoteNum`, plain `u64`
/// target guid.
///
/// **The id is an `EmotesText.dbc` row, not an `Emotes.dbc` one.**
/// `HandleTextEmoteOpcode` looks it up there and takes the row's `textid`,
/// which is what it broadcasts in `SMSG_EMOTE` — so the id going out and the id
/// coming back are from two different tables and are not equal. 34 is
/// `TEXTEMOTE_DANCE` and comes back as `Emotes` row 10, `STATE_DANCE`.
///
/// `emoteNum` selects between the several phrasings a text emote has ("you
/// dance", "you dance with X") and does not touch the animation. The target is
/// looked up on the map and may be zero, which is an emote at nobody.
fn is_timeout(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// `digest = SHA1(account | u32(0) | clientSeed | serverSeed | K)`
fn session_digest(
    account_upper: &str,
    client_seed: u32,
    server_seed: u32,
    session_key: &[u8; 40],
) -> [u8; 20] {
    let mut sha = Sha1::new();
    sha.update(account_upper.as_bytes());
    sha.update(0u32.to_le_bytes());
    sha.update(client_seed.to_le_bytes());
    sha.update(server_seed.to_le_bytes());
    sha.update(session_key);
    let d = sha.finalize();
    let mut digest = [0u8; 20];
    digest.copy_from_slice(&d);
    digest
}

#[derive(Debug, Clone)]
pub struct CharListEntry {
    pub guid: u64,
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    pub level: u8,
    /// `skin, face, hairStyle, hairColour, facialHair` — the five bytes
    /// `PLAYER_BYTES` and `PLAYER_BYTES_2` carry once there *is* a world.
    ///
    /// **Read rather than skipped, because character select draws the
    /// character.** Everything else about the plinth is derivable (the model is
    /// the race's, the wardrobe is below), but a face and a hairstyle are not:
    /// without them every character on the list is skin 0, face 0, bald. Kept as
    /// the five raw bytes rather than as `vale_assets::look::character::Appearance`
    /// because this crate does not depend on that one — the client assembles it
    /// in one line.
    pub appearance: [u8; 5],
    pub zone: u32,
    pub map: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `characters.character_flags`, verbatim — see [`CharListEntry::GHOST`].
    pub flags: u32,
    /// **What the character is wearing**, as `(ItemDisplayInfo id,
    /// InventoryType)` per slot, zeroes where the slot is empty.
    ///
    /// The one place in this protocol where a wardrobe arrives *already
    /// resolved*: `Player::BuildEnumData` writes `proto->DisplayInfoID` and
    /// `proto->InventoryType` outright, where `PLAYER_VISIBLE_ITEM_n_0` in the
    /// world carries an item **entry** that costs a `CMSG_ITEM_QUERY_SINGLE`
    /// round trip. So character select can dress the character on the frame the
    /// list arrives.
    ///
    /// What it does **not** carry is the item's class, subclass and sheath
    /// type — the three fields that decide where a *put-away* weapon hangs.
    /// Those live in the item template, `Item.dbc` is not in the 1.12 archives,
    /// and no packet on this screen can fetch them.
    ///
    /// **That used to be recorded here as the reason the plinth's character has
    /// empty hands, and the inference was wrong.** Nothing on character select
    /// is ever put away: the reference's dressing loop hands its weapon attacher
    /// a sheath type of 0 and a clear "put away" flag, so both hands take a hand
    /// point, and the only thing it asks about the item is
    /// `inventoryType == 14`, which is right here. The ranged slot (index 17) is
    /// skipped outright. See `vale_assets::tables::item::Weapon::from_char_enum`.
    ///
    /// **The index is the equipment slot and it is load-bearing**: 15 is the
    /// main hand and 16 the off hand, and no inventory type distinguishes them —
    /// a one-hand sword is `INVTYPE_WEAPON` in either. So this stays a
    /// positional list with its empty slots in it rather than being compacted.
    pub equipment: Vec<(u32, u8)>,
}

impl CharListEntry {
    /// `CHARACTER_FLAG_GHOST` — the character is dead, and the row reads
    /// `<name> (Ghost)`. `GetCharacterInfo`'s eighth return value.
    pub const GHOST: u32 = 0x0000_2000;
    /// `CHARACTER_FLAG_HIDE_HELM` / `_HIDE_CLOAK` — the two "do not draw this
    /// slot" ticks from the interface's own options, which the *server* stores
    /// and which decide what stands on the plinth.
    pub const HIDE_HELM: u32 = 0x0000_0400;
    pub const HIDE_CLOAK: u32 = 0x0000_0800;

    pub fn is_ghost(&self) -> bool {
        self.flags & Self::GHOST != 0
    }
    pub fn hides_helm(&self) -> bool {
        self.flags & Self::HIDE_HELM != 0
    }
    pub fn hides_cloak(&self) -> bool {
        self.flags & Self::HIDE_CLOAK != 0
    }
}

/// Number of equipment slots serialised per character in `SMSG_CHAR_ENUM`.
///
/// vmangos `Player::BuildEnumData` loops `slot < INVENTORY_SLOT_BAG_START + 1`,
/// i.e. 19 equipment slots plus the first bag.
///
/// Each is **five bytes** — `u32 displayId` then `u8 inventoryType` — and there
/// is deliberately no per-slot enchantment field in 1.12; that is added in 2.4.
/// Assuming otherwise overruns the packet by 80 bytes per character and
/// desynchronises every entry after the first, which is what
/// `a_second_character_parses_after_the_first` is for.
const CHAR_ENUM_EQUIPMENT_SLOTS: usize = 20;

/// Parse `SMSG_CHAR_ENUM`. Only the pet block is skipped now, and it must still
/// be *consumed exactly* or the next character misparses.
///
/// **The appearance and the equipment used to be skipped too**, and that was the
/// whole reason character select showed an empty plinth: the five appearance
/// bytes and the twenty `(display id, inventory type)` pairs are the only
/// statement anywhere of what a character on this screen looks like. Reading
/// them costs nothing — they were already being stepped over.
fn parse_char_enum(body: &[u8]) -> Vec<CharListEntry> {
    let mut r = Reader::new(body);
    let count = r.u8();
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let guid = {
            let lo = r.u32() as u64;
            let hi = r.u32() as u64;
            (hi << 32) | lo
        };
        let name = r.cstring();
        let race = r.u8();
        let class = r.u8();
        let gender = r.u8();
        // skin, face, hair style, hair colour, facial hair — in that order, and
        // the order is the one `Player::BuildEnumData` writes them in.
        let appearance = [r.u8(), r.u8(), r.u8(), r.u8(), r.u8()];
        let level = r.u8();
        let zone = r.u32();
        let map = r.u32();
        let x = r.f32();
        let y = r.f32();
        let z = r.f32();
        let _guild_id = r.u32();
        let flags = r.u32();
        let _first_login = r.u8();
        let _pet_display_id = r.u32();
        let _pet_level = r.u32();
        let _pet_family = r.u32();
        let equipment = (0..CHAR_ENUM_EQUIPMENT_SLOTS)
            .map(|_| (r.u32(), r.u8()))
            .collect();

        out.push(CharListEntry {
            guid,
            name,
            race,
            class,
            gender,
            appearance,
            level,
            zone,
            map,
            x,
            y,
            z,
            flags,
            equipment,
        });
    }
    out
}

fn err(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
}

/// Backwards-compatible entry point: connect and immediately enumerate
/// characters.
pub fn enter(
    world_addr: &str,
    account: &str,
    session_key: [u8; 40],
) -> io::Result<Vec<CharListEntry>> {
    let mut session = WorldSession::connect(world_addr, account, session_key)?;
    session.char_enum()
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    /// **Disarmed, the ring records nothing at all**, which is what makes it
    /// affordable on the framing path: the whole cost is the atomic load in
    /// [`Capture::armed`].
    #[test]
    fn a_disarmed_capture_records_nothing() {
        let capture = Capture::default();
        assert!(!capture.armed());
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[1, 2, 3], true);
        let snapshot = capture.snapshot();
        assert!(snapshot.packets.is_empty());
        assert_eq!(snapshot.seen, 0);
        assert!(!snapshot.armed);
    }

    /// **Both directions land in one ring, in the order they were noted**, which
    /// is the whole reason there is one ring rather than two: the two funnels
    /// run on the same thread and no timestamp is fine enough to merge them by.
    #[test]
    fn the_two_directions_interleave_in_wire_order() {
        let capture = Capture::default();
        capture.arm(true);
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[1], true);
        capture.note(Opcode::CMSG_PING.code(), &[2], false);
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[3], true);

        let snapshot = capture.snapshot();
        assert!(snapshot.armed);
        assert_eq!(snapshot.seen, 3);
        let order: Vec<(u64, bool, u8)> = snapshot
            .packets
            .iter()
            .map(|p| (p.sequence, p.inbound, p.body[0]))
            .collect();
        assert_eq!(order, [(0, true, 1), (1, false, 2), (2, true, 3)]);
        // …and the names are resolved on the way in, so a reader needs no
        // opcode table of its own.
        assert_eq!(snapshot.packets[1].name, "CMSG_PING");
    }

    /// **The ring drops its oldest and says so.** `seen` outrunning the kept
    /// count is what tells a reader the list is a window onto a longer capture
    /// rather than the whole of it.
    #[test]
    fn the_ring_is_bounded_and_reports_what_it_dropped() {
        let capture = Capture::default();
        capture.arm(true);
        for _ in 0..(CAPTURE_PACKETS + 40) {
            capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[0], true);
        }
        let snapshot = capture.snapshot();
        assert_eq!(snapshot.packets.len(), CAPTURE_PACKETS);
        assert_eq!(snapshot.seen as usize, CAPTURE_PACKETS + 40);
        // The oldest forty are gone, so the first sequence number kept is 40.
        assert_eq!(snapshot.packets[0].sequence, 40);
    }

    /// **A long body is cut and the true length is kept beside it.**
    /// `SMSG_UPDATE_OBJECT` runs to tens of kilobytes; keeping every byte would
    /// make the ring a megabyte and its snapshot a real cost.
    #[test]
    fn a_long_body_is_truncated_and_says_its_real_length() {
        let capture = Capture::default();
        capture.arm(true);
        let body = vec![7u8; CAPTURE_BODY_BYTES * 3];
        capture.note(Opcode::SMSG_UPDATE_OBJECT.code(), &body, true);

        let packet = capture.snapshot().packets.remove(0);
        assert_eq!(packet.body.len(), CAPTURE_BODY_BYTES);
        assert_eq!(packet.length, CAPTURE_BODY_BYTES * 3);
        assert!(packet.truncated());

        // …and a short one is not marked truncated, which is what the byte
        // view's own note is gated on.
        capture.note(Opcode::CMSG_PING.code(), &[1, 2], false);
        let short = capture.snapshot().packets.pop().expect("the second packet");
        assert_eq!(short.length, 2);
        assert!(!short.truncated());
    }

    /// **Arming clears the ring; disarming does not.** A capture always begins
    /// empty, so its sequence numbers start at zero and cannot be confused with
    /// a previous run's — and the packets that preceded a fault are exactly the
    /// ones wanted after the box is unticked.
    #[test]
    fn arming_clears_the_ring_and_disarming_keeps_it() {
        let capture = Capture::default();
        capture.arm(true);
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[1], true);
        capture.arm(false);

        let snapshot = capture.snapshot();
        assert!(!snapshot.armed, "disarmed");
        assert_eq!(snapshot.packets.len(), 1, "…and the packet is still readable");

        // Anything noted while disarmed is dropped rather than appended.
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[2], true);
        assert_eq!(capture.snapshot().packets.len(), 1);

        // Re-arming starts a fresh capture from sequence zero.
        capture.arm(true);
        assert!(capture.snapshot().packets.is_empty());
        capture.note(Opcode::SMSG_MONSTER_MOVE.code(), &[3], true);
        let snapshot = capture.snapshot();
        assert_eq!(snapshot.seen, 1);
        assert_eq!(snapshot.packets[0].sequence, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// **A world server that accepts the socket and says nothing gives up**,
    /// which is the second half of the "endless connecting to server" bug: the
    /// handshake below `connect_as` is four blocking reads, and without a
    /// deadline the whole login parks on a pool thread that dropping the task
    /// cannot even stop.
    ///
    /// The budget is a parameter for exactly this — see
    /// [`WorldSession::connect_within`] — because reproducing it at the shipped
    /// fifteen seconds is not a unit test.
    #[test]
    fn a_world_server_that_never_answers_is_given_up_on() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().unwrap().port();
        // Accepted by the backlog and never answered, which is the shape a
        // wedged or throttling server presents.
        let started = Instant::now();
        let outcome = WorldSession::connect_within(
            &format!("127.0.0.1:{port}"),
            "TEST",
            [0u8; 40],
            &version::WOW_1_12_1_WIN_X86,
            Duration::from_millis(200),
        );
        let error = outcome.err().expect("a silent server is not a session");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it gave up in {:?}, which is not giving up",
            started.elapsed()
        );
    }

    /// One character, byte for byte as `Player::BuildEnumData` writes one.
    fn character_bytes(w: &mut Writer, guid: u32, name: &str, flags: u32) {
        w.u32(guid).u32(0); // guid
        w.buf.extend_from_slice(name.as_bytes());
        w.u8(0);
        w.u8(1).u8(8).u8(1); // human, warlock, female
        w.u8(3).u8(5).u8(9).u8(2).u8(4); // skin, face, hair, colour, facial hair
        w.u8(60);
        w.u32(12).u32(0); // zone, map
        w.f32(-8900.0).f32(-130.0).f32(80.0);
        w.u32(0); // guild
        w.u32(flags);
        w.u8(0); // first login
        w.u32(0).u32(0).u32(0); // pet
        for slot in 0..CHAR_ENUM_EQUIPMENT_SLOTS {
            // Slot 0 is the head, slot 4 the chest; the rest are empty, which is
            // the commonest shape and the one a reader must not choke on.
            match slot {
                0 => w.u32(21_549).u8(1),
                4 => w.u32(31_051).u8(5),
                _ => w.u32(0).u8(0),
            };
        }
    }

    /// **The three blocks this parser used to step over**: the five appearance
    /// bytes, the flag word and the twenty-slot wardrobe.
    ///
    /// All three are what character select *draws*, so a misread here is a
    /// plinth with the wrong face, no gear, or a `(Ghost)` that never appears.
    /// The body is assembled rather than captured because a capture would pin
    /// one account's characters; what wants pinning is the layout.
    #[test]
    fn a_character_row_carries_its_appearance_its_flags_and_its_wardrobe() {
        let mut w = Writer::new();
        w.u8(1); // one character
        character_bytes(
            &mut w,
            7,
            "Alden",
            CharListEntry::GHOST | CharListEntry::HIDE_HELM,
        );

        let rows = parse_char_enum(&w.buf);
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.name, "Alden");
        assert_eq!(row.appearance, [3, 5, 9, 2, 4], "skin, face, hair, colour, beard");
        assert!(row.is_ghost() && row.hides_helm() && !row.hides_cloak());
        assert_eq!(row.equipment.len(), CHAR_ENUM_EQUIPMENT_SLOTS);
        assert_eq!(row.equipment[0], (21_549, 1));
        assert_eq!(row.equipment[4], (31_051, 5));
        assert_eq!(row.equipment[19], (0, 0), "the last slot is the first bag");
    }

    /// **The second character in the list is where a block-size error shows.**
    ///
    /// A per-character body read one byte long or short leaves every entry after
    /// the first misaligned — the classic version of this is the 2.4 enchant
    /// field, which is 80 bytes a character and turns the rest of the list into
    /// noise. An account with one character never notices.
    #[test]
    fn a_second_character_parses_after_the_first() {
        let mut w = Writer::new();
        w.u8(2);
        character_bytes(&mut w, 7, "Alden", CharListEntry::GHOST);
        character_bytes(&mut w, 9, "Bram", 0);

        let rows = parse_char_enum(&w.buf);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].name, "Bram");
        assert_eq!(rows[1].guid, 9);
        assert!(!rows[1].is_ghost(), "the first row's flags did not bleed");
        assert_eq!(rows[1].appearance, [3, 5, 9, 2, 4]);
        assert_eq!(rows[1].equipment[4], (31_051, 5));
    }

    /// **`CMSG_SETSHEATHED` is a `u32`, not the byte the update field holds.**
    ///
    /// The obvious mistake is the interesting one: `UNIT_FIELD_BYTES_2` byte 0
    /// is where this value ends up, so writing one byte reads as correct — and
    /// vmangos' `recv_data >> sheathed` under-runs on a 1-byte body and drops
    /// the packet with nothing said at either end. The failure mode is a weapon
    /// that other players never see drawn.
    #[test]
    fn the_sheath_body_is_four_bytes() {
        assert_eq!(set_sheathed_body(1), vec![1, 0, 0, 0]);
        assert_eq!(set_sheathed_body(0).len(), 4);
    }

    #[test]
    fn unknown_opcodes_are_tolerated() {
        let pkt = Packet {
            code: 0xFFFF,
            body: vec![],
        };
        assert!(pkt.opcode().is_none());
        assert!(pkt.name().contains("UNKNOWN"));
    }
}
