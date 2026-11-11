//! The TCP connections of a session, encrypted.
//!
//! [`Listener`] runs on the host: it accepts connections and numbers each one as
//! a user, from 1. [`Connection`] is a joining editor's. Each connection is a
//! reader thread and a writer thread over the one socket, and the editor's
//! frame loop reaches them through channels: [`Listener::poll`] and
//! [`Connection::poll`] never block, and a send only queues the message.
//! Nagle's algorithm is off, since a session sends many small messages that
//! should not wait for each other.
//!
//! ## The handshake
//!
//! Each side sends [`HELLO`] and a fresh X25519 public key as the first bytes
//! of the connection. The shared secret gives one key per direction and the
//! connection's binding, which the password check is bound to
//! ([`crate::auth`]). A key whose exchange gives no shared secret (a low-order
//! point) closes the connection.
//!
//! ## A frame
//!
//! A message is encoded as [`crate::wire`] encodes it, deflated when that
//! makes it smaller (a first byte says which), and sealed with
//! ChaCha20-Poly1305 under the direction's key. The nonce is the frame's
//! number in its direction, so a frame dropped, repeated or reordered fails
//! to open and closes the connection. On the socket a frame is its length,
//! four bytes little-endian, then the sealed bytes.
//!
//! The encryption keeps the edits and the shared world database's login from
//! anyone who reads the network. Without a password a machine between the two
//! editors can still open a connection to each and pass the edits on; with
//! one it cannot answer the challenge.

use crate::wire::{UserId, MAX_FRAME};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufWriter, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a joining editor waits for the host to answer the connection, and
/// how long either side waits for the other's key.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The first bytes each side sends, before its key: this transport's version.
pub const HELLO: &[u8; 4] = b"VCX1";

/// A message is deflated when it is at least this long.
const DEFLATE_FROM: usize = 256;

/// A sealed frame is its body plus the flag byte and the 16-byte tag.
const SEAL_OVERHEAD: usize = 17;

/// What happened on the host's listener.
#[derive(Debug)]
pub enum Event<M> {
    /// A connection, from this address, with this binding; see
    /// [`crate::auth`].
    Connected(UserId, SocketAddr, [u8; 32]),
    Message(UserId, M),
    /// The connection is gone, and why.
    Closed(UserId, String),
}

/// One accepted connection: where its writer thread takes messages from, and
/// the socket, kept to shut it down.
struct Link<O> {
    send: Sender<O>,
    stream: TcpStream,
}

/// The host's listener. Dropping it closes every connection.
pub struct Listener<I, O> {
    addr: SocketAddr,
    /// Behind a lock so the listener is `Sync`; only [`Self::poll`] takes it.
    events: Mutex<Receiver<Event<I>>>,
    links: Arc<Mutex<HashMap<UserId, Link<O>>>>,
    stop: Arc<AtomicBool>,
}

impl<I, O> Listener<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    /// Listen on `addr`, `0.0.0.0:port` for every interface. Port 0 takes any
    /// free port; [`Self::addr`] says which.
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Listener<I, O>> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let (events_in, events) = channel();
        let links: Arc<Mutex<HashMap<UserId, Link<O>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (accept_links, accept_stop) = (links.clone(), stop.clone());
        std::thread::Builder::new().name("collab-accept".into()).spawn(move || {
            let mut next: UserId = 1;
            for stream in listener.incoming() {
                if accept_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let user = next;
                next += 1;
                // The handshake runs on a thread of its own, so a connection
                // that sends nothing does not hold up the next.
                let (events, links) = (events_in.clone(), accept_links.clone());
                let _ = std::thread::Builder::new().name(format!("collab-open-{user}")).spawn(move || {
                    match open_link::<I, O>(user, stream, events.clone(), links.clone()) {
                        Ok((link, from, binding)) => {
                            links.lock().expect("links").insert(user, link);
                            let _ = events.send(Event::Connected(user, from, binding));
                        }
                        Err(e) => {
                            let _ = events.send(Event::Closed(user, e.to_string()));
                        }
                    }
                });
            }
        })?;
        Ok(Listener { addr, events: Mutex::new(events), links, stop })
    }

    /// The address and port it is listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Everything that has happened since the last call.
    pub fn poll(&self) -> Vec<Event<I>> {
        self.events.lock().expect("events").try_iter().collect()
    }

    /// Queue a message for one user. A user whose connection is gone is
    /// skipped; its [`Event::Closed`] has been or will be reported.
    pub fn send(&self, user: UserId, message: O) {
        if let Some(link) = self.links.lock().expect("links").get(&user) {
            let _ = link.send.send(message);
        }
    }

    /// A sender for one user's connection, for a thread that writes to it
    /// without the listener: the files a joining editor fetches.
    pub fn sender(&self, user: UserId) -> Option<Sender<O>> {
        self.links.lock().expect("links").get(&user).map(|link| link.send.clone())
    }

    /// Close one user's connection once its queued messages are written.
    pub fn close(&self, user: UserId) {
        // Dropping the sender ends the writer thread after the queue, and the
        // writer shuts the socket down as it ends.
        self.links.lock().expect("links").remove(&user);
    }
}

impl<I, O> Drop for Listener<I, O> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // The accept thread is blocked in `accept`; one connection wakes it,
        // and the thread's end closes the listening socket so the port can be
        // bound again. A listener on every interface (`0.0.0.0`) is woken on
        // loopback, since Windows refuses a connection to the unspecified
        // address and the thread would keep the port for the life of the
        // process.
        let mut wake = self.addr;
        if wake.ip().is_unspecified() {
            wake.set_ip(match wake {
                SocketAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
                SocketAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
            });
        }
        let _ = TcpStream::connect_timeout(&wake, Duration::from_millis(200));
        for (_, link) in self.links.lock().expect("links").drain() {
            let _ = link.stream.shutdown(Shutdown::Both);
        }
    }
}

/// One direction's sealing: its key and the number of the next frame.
struct Sealer {
    cipher: ChaCha20Poly1305,
    next: u64,
}

impl Sealer {
    fn new(key: [u8; 32]) -> Sealer {
        Sealer { cipher: ChaCha20Poly1305::new(Key::from_slice(&key)), next: 0 }
    }

    /// The nonce of the next frame, which moves the count on.
    fn nonce(&mut self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.next.to_le_bytes());
        self.next += 1;
        nonce
    }

    fn seal(&mut self, plain: &[u8]) -> std::io::Result<Vec<u8>> {
        let nonce = self.nonce();
        self.cipher.encrypt(Nonce::from_slice(&nonce), plain).map_err(|_| std::io::Error::other("a frame could not be sealed"))
    }

    fn open(&mut self, sealed: &[u8]) -> std::io::Result<Vec<u8>> {
        let nonce = self.nonce();
        self.cipher
            .decrypt(Nonce::from_slice(&nonce), sealed)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a frame did not open: it was changed, repeated or out of order"))
    }
}

/// Exchange keys over a new connection. `joining` is the side that opened
/// it. Answers the sealer for frames sent, the opener for frames received,
/// and the connection's binding. See *The handshake*.
fn handshake(stream: &mut TcpStream, joining: bool) -> std::io::Result<(Sealer, Sealer, [u8; 32])> {
    let secret = x25519_dalek::StaticSecret::from(rand::random::<[u8; 32]>());
    let public = x25519_dalek::PublicKey::from(&secret);
    let mut hello = HELLO.to_vec();
    hello.extend_from_slice(public.as_bytes());
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.write_all(&hello)?;
    let mut theirs = [0u8; 36];
    stream.read_exact(&mut theirs)?;
    stream.set_read_timeout(None)?;
    if &theirs[..4] != HELLO {
        return Err(std::io::Error::other("the other end is not a session of this editor's version"));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&theirs[4..]);
    let their = x25519_dalek::PublicKey::from(key);
    let shared = secret.diffie_hellman(&their);
    if !shared.was_contributory() {
        return Err(std::io::Error::other("the other end's key gives no shared secret"));
    }
    let (joiner, host) = match joining {
        true => (public.to_bytes(), their.to_bytes()),
        false => (their.to_bytes(), public.to_bytes()),
    };
    let mut both = joiner.to_vec();
    both.extend_from_slice(&host);
    let binding = blake3::derive_key("vale-collab 2026-11-10 connection binding", &both);
    let mut material = shared.as_bytes().to_vec();
    material.extend_from_slice(&both);
    let to_host = blake3::derive_key("vale-collab 2026-11-10 joiner to host", &material);
    let to_joiner = blake3::derive_key("vale-collab 2026-11-10 host to joiner", &material);
    Ok(match joining {
        true => (Sealer::new(to_host), Sealer::new(to_joiner), binding),
        false => (Sealer::new(to_joiner), Sealer::new(to_host), binding),
    })
}

/// Write one message as a sealed frame. See *A frame*.
fn write_sealed<T: Serialize>(to: &mut impl Write, sealer: &mut Sealer, message: &T) -> std::io::Result<()> {
    let body = crate::wire::encode(message)?;
    if body.len() > MAX_FRAME {
        return Err(std::io::Error::other(format!("a {} byte message is over the frame limit", body.len())));
    }
    let mut plain = Vec::with_capacity(body.len() + 1);
    plain.push(0);
    if body.len() >= DEFLATE_FROM {
        let mut deflated = flate2::write::DeflateEncoder::new(vec![1u8], flate2::Compression::fast());
        deflated.write_all(&body)?;
        let deflated = deflated.finish()?;
        if deflated.len() < body.len() {
            plain = deflated;
        }
    }
    if plain.len() == 1 {
        plain.extend_from_slice(&body);
    }
    let sealed = sealer.seal(&plain)?;
    to.write_all(&(sealed.len() as u32).to_le_bytes())?;
    to.write_all(&sealed)?;
    to.flush()
}

/// Read one sealed frame and decode its message. An end of stream before
/// the length is [`std::io::ErrorKind::UnexpectedEof`].
fn read_sealed<T: DeserializeOwned>(from: &mut impl Read, opener: &mut Sealer) -> std::io::Result<T> {
    let mut length = [0u8; 4];
    from.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME + SEAL_OVERHEAD {
        return Err(std::io::Error::other(format!("a frame of {length} bytes is over the limit")));
    }
    let mut sealed = vec![0u8; length];
    from.read_exact(&mut sealed)?;
    let plain = opener.open(&sealed)?;
    let (flag, rest) = plain.split_first().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "an empty frame"))?;
    let body = match flag {
        0 => rest.to_vec(),
        1 => {
            let mut body = Vec::new();
            flate2::read::DeflateDecoder::new(rest).take(MAX_FRAME as u64 + 1).read_to_end(&mut body)?;
            if body.len() > MAX_FRAME {
                return Err(std::io::Error::other("a frame inflates past the limit"));
            }
            body
        }
        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "a frame of an unknown form")),
    };
    crate::wire::decode(&body)
}

/// Exchange keys over one accepted connection and start its reader and
/// writer threads.
fn open_link<I, O>(
    user: UserId,
    mut stream: TcpStream,
    events: Sender<Event<I>>,
    links: Arc<Mutex<HashMap<UserId, Link<O>>>>,
) -> std::io::Result<(Link<O>, SocketAddr, [u8; 32])>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    stream.set_nodelay(true)?;
    let from = stream.peer_addr()?;
    let (sealer, mut opener, binding) = handshake(&mut stream, false)?;
    let mut reading = stream.try_clone()?;
    let writing = stream.try_clone()?;
    let (send, queue) = channel::<O>();
    std::thread::Builder::new().name(format!("collab-read-{user}")).spawn(move || loop {
        match read_sealed::<I>(&mut reading, &mut opener) {
            Ok(message) => {
                if events.send(Event::Message(user, message)).is_err() {
                    break;
                }
            }
            Err(e) => {
                links.lock().expect("links").remove(&user);
                let _ = events.send(Event::Closed(user, e.to_string()));
                break;
            }
        }
    })?;
    std::thread::Builder::new().name(format!("collab-write-{user}")).spawn(move || write_all(writing, queue, sealer))?;
    Ok((Link { send, stream }, from, binding))
}

/// Write every queued message until the queue is dropped or a write fails,
/// then shut the socket down.
fn write_all<O: Serialize>(stream: TcpStream, queue: Receiver<O>, mut sealer: Sealer) {
    let mut writer = BufWriter::new(&stream);
    for message in queue {
        if write_sealed(&mut writer, &mut sealer, &message).is_err() {
            break;
        }
    }
    drop(writer);
    let _ = stream.shutdown(Shutdown::Both);
}

/// A joining editor's connection to the host. Dropping it closes it.
pub struct Connection<I, O> {
    /// Behind a lock so the connection is `Sync`; only [`Self::poll`] takes
    /// it.
    received: Mutex<Receiver<Result<I, String>>>,
    send: Option<Sender<O>>,
    binding: [u8; 32],
}

impl<I, O> Connection<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    /// Connect to the host at `addr` (`192.168.1.20:7878`, or a name), and
    /// exchange keys.
    pub fn open(addr: &str) -> std::io::Result<Connection<I, O>> {
        let mut last = std::io::Error::other(format!("{addr} names no address"));
        for at in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&at, CONNECT_TIMEOUT) {
                Ok(stream) => return Connection::over(stream),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn over(mut stream: TcpStream) -> std::io::Result<Connection<I, O>> {
        stream.set_nodelay(true)?;
        let (sealer, mut opener, binding) = handshake(&mut stream, true)?;
        let mut reading = stream.try_clone()?;
        let writing = stream;
        let (received_in, received) = channel();
        std::thread::Builder::new().name("collab-read".into()).spawn(move || loop {
            match read_sealed::<I>(&mut reading, &mut opener) {
                Ok(message) => {
                    if received_in.send(Ok(message)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = received_in.send(Err(e.to_string()));
                    break;
                }
            }
        })?;
        let (send, queue) = channel::<O>();
        std::thread::Builder::new().name("collab-write".into()).spawn(move || write_all(writing, queue, sealer))?;
        Ok(Connection { received: Mutex::new(received), send: Some(send), binding })
    }

    /// The connection's binding. See [`crate::auth`].
    pub fn binding(&self) -> [u8; 32] {
        self.binding
    }

    /// Every message received since the last call. An `Err` is the
    /// connection ending, and is the last item.
    pub fn poll(&self) -> Vec<Result<I, String>> {
        self.received.lock().expect("received").try_iter().collect()
    }

    /// Queue a message for the host.
    pub fn send(&self, message: O) {
        if let Some(send) = &self.send {
            let _ = send.send(message);
        }
    }
}

impl<I, O> Drop for Connection<I, O> {
    fn drop(&mut self) {
        // Dropping the sender ends the writer thread once the queue is
        // written, and its shutdown of the socket ends the reader thread.
        self.send = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A port a session on every interface listened on can be listened on
    /// again once the session ends.
    #[test]
    fn an_ended_listener_frees_its_port() {
        let first = Listener::<u32, u32>::bind("0.0.0.0:0").expect("a free port");
        let port = first.addr().port();
        let joined = Connection::<u32, u32>::open(&format!("127.0.0.1:{port}")).expect("a connection");
        drop(first);
        drop(joined);
        // The accept thread closes the socket as it ends, just after the drop.
        let again = (0..50).find_map(|_| {
            let bound = Listener::<u32, u32>::bind(("0.0.0.0", port)).ok();
            if bound.is_none() {
                std::thread::sleep(Duration::from_millis(20));
            }
            bound
        });
        assert!(again.is_some(), "port {port} is still held");
    }

    /// Messages cross both ways, a long one deflated, and both ends hold one
    /// binding.
    #[test]
    fn messages_cross_sealed_both_ways() {
        let listener = Listener::<String, String>::bind("127.0.0.1:0").expect("a free port");
        let joined = Connection::<String, String>::open(&listener.addr().to_string()).expect("a connection");
        let long = "terrain ".repeat(1000);
        joined.send(long.clone());
        let mut got = Vec::new();
        let mut binding = None;
        for _ in 0..200 {
            for event in listener.poll() {
                match event {
                    Event::Connected(user, _, bound) => {
                        binding = Some(bound);
                        listener.send(user, "welcome".to_string());
                    }
                    Event::Message(_, message) => got.push(message),
                    Event::Closed(_, why) => panic!("closed: {why}"),
                }
            }
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(got, vec![long]);
        assert_eq!(binding, Some(joined.binding()));
        let answer = (0..200).find_map(|_| {
            std::thread::sleep(Duration::from_millis(10));
            joined.poll().into_iter().next()
        });
        assert_eq!(answer, Some(Ok("welcome".to_string())));
    }

    /// A sealed frame opens once, in order; a repeated one does not.
    #[test]
    fn a_repeated_frame_does_not_open() {
        let (mut sealer, mut opener) = (Sealer::new([5; 32]), Sealer::new([5; 32]));
        let mut wire = Vec::new();
        write_sealed(&mut wire, &mut sealer, &"one".to_string()).unwrap();
        let first: String = read_sealed(&mut wire.as_slice(), &mut opener).unwrap();
        assert_eq!(first, "one");
        assert!(read_sealed::<String>(&mut wire.as_slice(), &mut opener).is_err());
    }
}
