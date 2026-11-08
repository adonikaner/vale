//! The password check.
//!
//! The host sends a random nonce; the joining editor answers with a hash of
//! the nonce and its name, keyed by a key derived from the password. The
//! password itself is never sent, and an answer is good for one nonce only.
//! This keeps out an editor that does not know the password. It does not
//! encrypt the session: see the crate comment.

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
    }
}
