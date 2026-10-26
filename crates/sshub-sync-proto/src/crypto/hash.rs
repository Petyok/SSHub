//! BLAKE3 helpers: the op-chain and channel-binding hash, and context-separated
//! key derivation (one fixed context string per purpose).

/// Hash of op-chain links and channel bindings.
pub fn chain_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// BLAKE3 `derive_key` with a hard-coded context string.
pub fn derive_key(context: &str, ikm: &[u8]) -> [u8; 32] {
    blake3::derive_key(context, ikm)
}

/// Extendable output of `derive_key`: `out.len()` bytes under one context.
pub fn derive_xof(context: &str, ikm: &[u8], out: &mut [u8]) {
    let mut h = blake3::Hasher::new_derive_key(context);
    h.update(ikm);
    h.finalize_xof().fill(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_known_answer_for_empty_input() {
        assert_eq!(
            hex::encode(chain_hash(b"")),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn xof_prefix_equals_derive_key() {
        let mut out = [0u8; 128];
        derive_xof("sshub/test", b"ikm", &mut out);
        assert_eq!(&out[..32], &derive_key("sshub/test", b"ikm"));
    }
}
