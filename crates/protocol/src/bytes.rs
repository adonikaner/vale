//! Minimal little-endian byte reader/writer for wire packets.
//!
//! WoW packet bodies are little-endian; byte-array fields (SRP numbers, the
//! session key) are already in the order the server hashed them, so we pass
//! them through verbatim. Only the two packet *headers* are big-endian on the
//! size field, and those are handled in `world.rs` / `auth.rs` directly.

use std::io::{self, Read};
use std::net::TcpStream;

/// A cursor over an owned byte buffer with typed little-endian reads.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn u8(&mut self) -> u8 {
        let v = self.buf[self.pos];
        self.pos += 1;
        v
    }

    pub fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        v
    }

    pub fn u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&self.buf[self.pos..self.pos + 4]);
        self.pos += 4;
        u32::from_le_bytes(b)
    }

    pub fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }

    pub fn u64(&mut self) -> u64 {
        let lo = self.u32() as u64;
        let hi = self.u32() as u64;
        (hi << 32) | lo
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        let out = self.buf[self.pos..self.pos + n].to_vec();
        self.pos += n;
        out
    }

    /// Step over `n` bytes without copying them.
    ///
    /// For the runs of fields a packet states and this client does not read —
    /// the sub-damage array in front of `SMSG_ATTACKERSTATEUPDATE`'s victim
    /// state, say. `bytes` would do the same job and allocate a `Vec` to throw
    /// away.
    pub fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.buf.len());
    }

    /// Are there at least `n` bytes left?
    ///
    /// The typed readers above index directly and therefore panic past the end.
    /// That is fine for packets whose layout we have verified, but
    /// `SMSG_UPDATE_OBJECT` is variable-length and gated by flags, so any
    /// misreading there would desynchronise and run off the end. Callers in
    /// `update.rs` check this before every field group and stop cleanly instead.
    pub fn has(&self, n: usize) -> bool {
        self.remaining() >= n
    }

    /// A "packed" GUID: a one-byte mask, then only the non-zero bytes.
    ///
    /// Bit `i` of the mask means byte `i` of the little-endian u64 is present.
    /// Used for most object references from 1.9 onwards — but note that
    /// `UPDATETYPE_MOVEMENT` blocks still carry a plain u64, so do not assume
    /// every GUID on the wire is packed.
    pub fn packed_guid(&mut self) -> u64 {
        if !self.has(1) {
            return 0;
        }
        let mask = self.u8();
        let mut guid = 0u64;
        for i in 0..8 {
            if mask & (1 << i) != 0 {
                if !self.has(1) {
                    break;
                }
                guid |= (self.u8() as u64) << (i * 8);
            }
        }
        guid
    }

    /// Null-terminated ASCII string (WoW CString).
    pub fn cstring(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned();
        if self.pos < self.buf.len() {
            self.pos += 1; // consume the null terminator
        }
        s
    }
}

/// A growable little-endian packet body writer.
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    /// A "packed" GUID — the mirror of [`Reader::packed_guid`].
    ///
    /// Note that this is *not* interchangeable with a plain u64 on the wire:
    /// `CMSG_SET_ACTIVE_MOVER` takes the raw form while most other references
    /// take the packed one, and the server reads exactly what it declared.
    pub fn packed_guid(&mut self, guid: u64) -> &mut Self {
        let bytes = guid.to_le_bytes();
        let mut mask = 0u8;
        for (i, b) in bytes.iter().enumerate() {
            if *b != 0 {
                mask |= 1 << i;
            }
        }
        self.buf.push(mask);
        for b in bytes.iter() {
            if *b != 0 {
                self.buf.push(*b);
            }
        }
        self
    }

    /// Null-terminated ASCII string.
    pub fn cstring(&mut self, s: &str) -> &mut Self {
        self.buf.extend_from_slice(s.as_bytes());
        self.buf.push(0);
        self
    }
}

/// Read exactly `n` bytes from a stream (blocking), or error on EOF.
pub fn read_exact(stream: &mut TcpStream, n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Lowercase hex, for debug logging.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
