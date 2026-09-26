//! SRP6 client for the WoW 1.12 logon handshake.
//!
//! Mirrors vmangos `src/shared/Crypto/Authentication/SRP6.cpp`. All WoW SRP
//! numbers travel as little-endian byte arrays, so we convert with
//! `from_bytes_le` / `to_bytes_le` throughout. Constants:
//!   N = 894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7
//!   g = 7 ,  k = 3
//!
//! Proof order (must match SRP6::CalculateProof):
//!   M1 = SHA1( (H(N) xor H(g)) | H(user) | s | A | B | K )
//!
//! Every one of those operands is a `BigNumber` on the server, and hashing a
//! `BigNumber` feeds it at its **minimal** byte length, not the width it
//! occupies on the wire. See [`to_le_minimal`] — getting that wrong makes
//! roughly one login in sixty fail as a wrong password.

use num_bigint::BigUint;
use rand::RngCore;
use sha1::{Digest, Sha1};

/// N as a big-endian hex string (as written in vmangos source).
const N_HEX: &str = "894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7";

fn sha1(chunks: &[&[u8]]) -> [u8; 20] {
    let mut h = Sha1::new();
    for c in chunks {
        h.update(c);
    }
    let d = h.finalize(); // GenericArray<u8, 20>; deref to &[u8] and copy
    let mut out = [0u8; 20];
    out.copy_from_slice(&d);
    out
}

/// Client integrity ("crc") proof for CMD_AUTH_LOGON_PROOF.
///
/// vmangos `AuthSocket::VerifyVersion` computes SHA1(A | integrityHash), where
/// integrityHash is the `allowed_clients.integrity_hash` row matching
/// (build, os, platform). The retail client derives that hash from its own
/// binaries; we send the published known-good value for our build so a server
/// with `StrictVersionCheck = 1` accepts us.
pub fn version_proof(a_pub: &[u8; 32], integrity_hash: &[u8; 20]) -> [u8; 20] {
    sha1(&[a_pub, integrity_hash])
}

/// Left-pad/truncate a little-endian byte array to exactly `len` bytes.
fn to_le_fixed(n: &BigUint, len: usize) -> Vec<u8> {
    let mut v = n.to_bytes_le();
    v.resize(len, 0); // little-endian: zero-extend the high end
    v.truncate(len);
    v
}

/// Trim a little-endian value to the length vmangos would hash it at.
///
/// **This is not cosmetic and it is not optional.** Every SRP number is a
/// `BigNumber` server-side, and `SHA1::Generator::UpdateData(BigNumber const&)`
/// feeds it `AsByteArray()` — which defaults to `GetNumBytes()`, the *minimal*
/// length. So when `A`, `B`, `s` or `K` happens to have a zero most-significant
/// byte, the server hashes 31 bytes where the packet carried 32, and any client
/// that hashes the padded form computes a different `u` and a different `M1`.
///
/// The failure is intermittent — roughly one login in sixty across the four
/// values — and realmd reports it as `WOW_FAIL_UNKNOWN_ACCOUNT` with
/// "tried to login with wrong password!" in its log, which points squarely at
/// the credentials and not at a byte count.
fn to_le_minimal(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|b| *b != 0)
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

pub struct SrpResult {
    /// Client public ephemeral A (32-byte LE), sent in the logon proof.
    pub a_pub: [u8; 32],
    /// Client proof M1 (20 bytes), sent in the logon proof.
    pub m1: [u8; 20],
    /// 40-byte session key K, shared with the world server.
    pub session_key: [u8; 40],
}

/// Run the client side of SRP6 given the challenge fields from the server.
///
/// * `account` / `password` — will be upper-cased (WoW hashes the upper form).
/// * `b_pub`, `salt` — the 32-byte LE `B` and 32-byte LE salt from the server.
/// * `g_byte`, `n_bytes` — the generator and modulus the server advertised
///   (we assert they equal the known constants; the client trusts its own N/g).
pub fn compute(
    account: &str,
    password: &str,
    b_pub: &[u8],
    g_byte: u8,
    n_bytes: &[u8],
    salt: &[u8],
) -> SrpResult {
    let user = account.to_uppercase();
    let pass = password.to_uppercase();

    let n = BigUint::parse_bytes(N_HEX.as_bytes(), 16).unwrap();
    let g = BigUint::from(7u32);
    let k = BigUint::from(3u32);

    // Sanity: the server should be using the same parameters we hard-code.
    debug_assert_eq!(g_byte, 7, "unexpected SRP generator from server");
    debug_assert_eq!(
        BigUint::from_bytes_le(n_bytes),
        n,
        "unexpected SRP modulus from server"
    );

    let b = BigUint::from_bytes_le(b_pub);

    // x = H( salt | H(USER:PASS) ). The salt is also a BigNumber server-side,
    // though in practice a no-op: `SetRand` calls `BN_rand(..., top = 0)`,
    // which forces the top bit, so a server-generated salt is always a full
    // 32 bytes.
    let identity_hash = sha1(&[format!("{}:{}", user, pass).as_bytes()]);
    let x_hash = sha1(&[to_le_minimal(salt), &identity_hash]);
    let x = BigUint::from_bytes_le(&x_hash);

    // Client ephemeral: a random, A = g^a mod N. vmangos uses b = 19 bytes.
    let mut a_bytes = [0u8; 19];
    rand::thread_rng().fill_bytes(&mut a_bytes);
    let a = BigUint::from_bytes_le(&a_bytes);
    let a_pub_big = g.modpow(&a, &n);
    let a_pub_le = to_le_fixed(&a_pub_big, 32);

    // u = H(A | B), both at their minimal length — see `to_le_minimal`.
    let u = BigUint::from_bytes_le(&sha1(&[
        to_le_minimal(&a_pub_le),
        to_le_minimal(b_pub),
    ]));

    // Verifier v = g^x mod N, then S = (B - k*v)^(a + u*x) mod N.
    let v = g.modpow(&x, &n);
    let kv = (&k * &v) % &n;
    // Ensure a non-negative base before modpow: (B mod N + N - kv mod N) mod N.
    let base = ((&b % &n) + &n - (&kv % &n)) % &n;
    let exp = &a + &u * &x;
    let s = base.modpow(&exp, &n);

    // Interleave S -> 40-byte session key K (SRP6::HashSessionKey).
    let session_key = interleave(&to_le_fixed(&s, 32));

    // M1 = H( (H(N) xor H(g)) | H(USER) | s | A | B | K )
    let hn = sha1(&[&to_le_fixed(&n, 32)]);
    let hg = sha1(&[&[g_byte]]);
    let mut xored = [0u8; 20];
    for i in 0..20 {
        xored[i] = hn[i] ^ hg[i];
    }
    let huser = sha1(&[user.as_bytes()]);
    let m1 = sha1(&[
        &xored,
        &huser,
        to_le_minimal(salt),
        to_le_minimal(&a_pub_le),
        to_le_minimal(b_pub),
        to_le_minimal(&session_key),
    ]);

    let mut a_pub = [0u8; 32];
    a_pub.copy_from_slice(&a_pub_le);
    SrpResult {
        a_pub,
        m1,
        session_key,
    }
}

/// SRP6::HashSessionKey: split the 32-byte S into even/odd bytes, SHA1 each
/// 16-byte half, then interleave the two digests into a 40-byte key.
fn interleave(s: &[u8]) -> [u8; 40] {
    let mut even = [0u8; 16];
    let mut odd = [0u8; 16];
    for i in 0..16 {
        even[i] = s[i * 2];
        odd[i] = s[i * 2 + 1];
    }
    let he = sha1(&[&even]);
    let ho = sha1(&[&odd]);
    let mut k = [0u8; 40];
    for i in 0..20 {
        k[i * 2] = he[i];
        k[i * 2 + 1] = ho[i];
    }
    k
}

/// Optional: verify the server's M2 = H(A | M1 | K).
///
/// `SRP6::Finalize` hashes `A` and `K` as BigNumbers, so they are trimmed for
/// the same reason as in [`compute`].
pub fn expected_m2(a_pub: &[u8], m1: &[u8], session_key: &[u8]) -> [u8; 20] {
    sha1(&[to_le_minimal(a_pub), m1, to_le_minimal(session_key)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_length_drops_only_high_zero_bytes() {
        // Little-endian, so the high end is the *tail*.
        assert_eq!(to_le_minimal(&[0x34, 0x12, 0x00, 0x00]), &[0x34, 0x12]);
        // A zero in the middle or at the low end is significant.
        assert_eq!(to_le_minimal(&[0x00, 0x12, 0x00, 0x01]), &[0x00, 0x12, 0x00, 0x01]);
        assert_eq!(to_le_minimal(&[0x01, 0x00, 0x00, 0x00]), &[0x01]);
    }

    #[test]
    fn an_all_zero_value_hashes_as_nothing() {
        // `BN_num_bytes` of zero is 0, so vmangos appends no bytes at all.
        // Emitting a single zero byte instead would silently change the digest.
        assert!(to_le_minimal(&[0u8; 32]).is_empty());
    }

    #[test]
    fn a_full_width_value_is_untouched() {
        let full = [0xFFu8; 32];
        assert_eq!(to_le_minimal(&full).len(), 32);
    }
}
