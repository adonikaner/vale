//! The TCP connections of a session.
//!
//! [`Listener`] runs on the host: it accepts connections and numbers each one as
//! a user, from 1. [`Connection`] is a joining editor's. Each connection is a
//! reader thread and a writer thread over the one socket, and the editor's
//! frame loop reaches them through channels: [`Listener::poll`] and
//! [`Connection::poll`] never block, and a send only queues the message.
//!
//! Messages are framed by [`crate::wire::write_frame`]. Nagle's algorithm is
//! off, since a session sends many small messages that should not wait for
//! each other.

use crate::wire::{read_frame, write_frame, UserId};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::io::BufWriter;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a joining editor waits for the host to answer the connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// What happened on the host's listener.
#[derive(Debug)]
pub enum Event<M> {
    Connected(UserId, SocketAddr),
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
                match open_link::<I, O>(user, stream, events_in.clone(), accept_links.clone()) {
                    Ok((link, from)) => {
                        accept_links.lock().expect("links").insert(user, link);
                        let _ = events_in.send(Event::Connected(user, from));
                    }
                    Err(e) => {
                        let _ = events_in.send(Event::Closed(user, e.to_string()));
                    }
                }
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
        // The accept thread is blocked in `accept`; one connection wakes it.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
        for (_, link) in self.links.lock().expect("links").drain() {
            let _ = link.stream.shutdown(Shutdown::Both);
        }
    }
}

/// Start the reader and writer threads of one accepted connection.
fn open_link<I, O>(
    user: UserId,
    stream: TcpStream,
    events: Sender<Event<I>>,
    links: Arc<Mutex<HashMap<UserId, Link<O>>>>,
) -> std::io::Result<(Link<O>, SocketAddr)>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    stream.set_nodelay(true)?;
    let from = stream.peer_addr()?;
    let mut reading = stream.try_clone()?;
    let writing = stream.try_clone()?;
    let (send, queue) = channel::<O>();
    std::thread::Builder::new().name(format!("collab-read-{user}")).spawn(move || loop {
        match read_frame::<I>(&mut reading) {
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
    std::thread::Builder::new().name(format!("collab-write-{user}")).spawn(move || write_all(writing, queue))?;
    Ok((Link { send, stream }, from))
}

/// Write every queued message until the queue is dropped or a write fails,
/// then shut the socket down.
fn write_all<O: Serialize>(stream: TcpStream, queue: Receiver<O>) {
    let mut writer = BufWriter::new(&stream);
    for message in queue {
        if write_frame(&mut writer, &message).is_err() {
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
}

impl<I, O> Connection<I, O>
where
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    /// Connect to the host at `addr` (`192.168.1.20:7878`, or a name).
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

    fn over(stream: TcpStream) -> std::io::Result<Connection<I, O>> {
        stream.set_nodelay(true)?;
        let mut reading = stream.try_clone()?;
        let writing = stream;
        let (received_in, received) = channel();
        std::thread::Builder::new().name("collab-read".into()).spawn(move || loop {
            match read_frame::<I>(&mut reading) {
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
        std::thread::Builder::new().name("collab-write".into()).spawn(move || write_all(writing, queue))?;
        Ok(Connection { received: Mutex::new(received), send: Some(send) })
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
