//! World packet-header cipher for vanilla (1.12).
//!
//! Mirrors vmangos `AuthCrypt`. The server encrypts its 4-byte outgoing
//! headers with `EncryptSend` and decrypts our 6-byte headers with
//! `DecryptRecv`. On the client the roles swap:
//!
//!   * to SEND a client header, we run the inverse of the server's
//!     `DecryptRecv`, which is exactly its `EncryptSend` formula:
//!         x = (plain ^ key[i]) + prev ; prev = x
//!   * to RECV a server header, we run its `DecryptRecv`:
//!         x = (enc - prev) ^ key[i]  ; prev = enc(original)
//!
//! Both directions share the same 40-byte key K but keep independent (i, j)
//! state. Encryption is enabled only *after* the unencrypted CMSG_AUTH_SESSION.

pub struct HeaderCrypt {
    key: [u8; 40],
    send_i: usize,
    send_j: u8,
    recv_i: usize,
    recv_j: u8,
    enabled: bool,
}

impl HeaderCrypt {
    pub fn new() -> Self {
        HeaderCrypt {
            key: [0u8; 40],
            send_i: 0,
            send_j: 0,
            recv_i: 0,
            recv_j: 0,
            enabled: false,
        }
    }

    /// Turn on encryption with the SRP session key (called right after we send
    /// the plaintext CMSG_AUTH_SESSION).
    pub fn enable(&mut self, session_key: [u8; 40]) {
        self.key = session_key;
        self.send_i = 0;
        self.send_j = 0;
        self.recv_i = 0;
        self.recv_j = 0;
        self.enabled = true;
    }

    /// Encrypt an outgoing 6-byte client header in place.
    pub fn encrypt_send(&mut self, data: &mut [u8]) {
        if !self.enabled {
            return;
        }
        for b in data.iter_mut() {
            self.send_i %= self.key.len();
            let x = (*b ^ self.key[self.send_i]).wrapping_add(self.send_j);
            self.send_i += 1;
            self.send_j = x;
            *b = x;
        }
    }

    /// Decrypt an incoming 4-byte server header in place.
    pub fn decrypt_recv(&mut self, data: &mut [u8]) {
        if !self.enabled {
            return;
        }
        for b in data.iter_mut() {
            self.recv_i %= self.key.len();
            let orig = *b;
            let x = (orig.wrapping_sub(self.recv_j)) ^ self.key[self.recv_i];
            self.recv_i += 1;
            self.recv_j = orig;
            *b = x;
        }
    }
}
