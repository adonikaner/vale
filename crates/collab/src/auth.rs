//! The password check.
//!
//! The host sends a random nonce; the joining editor answers with a hash of
//! the nonce and its name, keyed by a key derived from the password. The
//! password itself is never sent, and an answer is good for one nonce only.
//!
//! The nonce is bound to the connection first ([`bound`]): hashed with the
//! connection's binding, which both ends work out from the keys they
//! exchanged (`crate::net`). A machine between the two that opened a
//! connection to each would hold two bindings, so an answer it passes on
//! does not check, and it cannot answer itself without the password.

/// The key a password stands for.
pub fn key(password: &str) -> [u8; 32] {
    blake3::derive_key("vale-collab 2026-11-08 session password", password.as_bytes())
}

/// The answer to a challenge: `nonce` and `name`, hashed under `key`.
pub fn proof(key: &[u8; 32], nonce: &[u8; 32], name: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(key);
    hasher.update(nonce);
    hasher.update(name.as_bytes());
    *hasher.finalize().as_bytes()
}

/// Whether `answer` is the proof for this key, nonce and name. Compared in
/// constant time, through [`blake3::Hash`]'s equality.
pub fn check(key: &[u8; 32], nonce: &[u8; 32], name: &str, answer: &[u8; 32]) -> bool {
    blake3::Hash::from_bytes(proof(key, nonce, name)) == blake3::Hash::from_bytes(*answer)
}

/// A challenge's nonce bound to one connection. See the module comment.
pub fn bound(nonce: &[u8; 32], binding: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("vale-collab 2026-11-10 bound challenge");
    hasher.update(nonce);
    hasher.update(binding);
    *hasher.finalize().as_bytes()
}

/// A fresh nonce.
pub fn nonce() -> [u8; 32] {
    rand::random()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The right password passes; a wrong password, another nonce or another
    /// name does not.
    #[test]
    fn only_the_right_password_for_this_nonce_and_name_passes() {
        let nonce = [7; 32];
        let answer = proof(&key("hunter2"), &nonce, "Alex");
        assert!(check(&key("hunter2"), &nonce, "Alex", &answer));
        assert!(!check(&key("hunter3"), &nonce, "Alex", &answer));
        assert!(!check(&key("hunter2"), &[8; 32], "Alex", &answer));
        assert!(!check(&key("hunter2"), &nonce, "Sam", &answer));
        let here = bound(&nonce, &[1; 32]);
        assert!(!check(&key("hunter2"), &bound(&nonce, &[2; 32]), "Alex", &proof(&key("hunter2"), &here, "Alex")));
    }
}
