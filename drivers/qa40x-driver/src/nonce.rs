//! A per-call test value for the register-0 write/read-back handshake.

use std::sync::atomic::{AtomicU32, Ordering};

/// Only needs to differ between calls (a stale reply must not pass as a fresh
/// one), so a clock-seeded xorshift is enough — no RNG dependency.
pub(crate) fn connection_nonce() -> u32 {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
        .unwrap_or(0x9E37_79B9);
    let mut x = t ^ COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed) ^ 0xA5A5_5A5A;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_nonces_differ() {
        let a = connection_nonce();
        let b = connection_nonce();
        assert_ne!(a, b);
    }
}
