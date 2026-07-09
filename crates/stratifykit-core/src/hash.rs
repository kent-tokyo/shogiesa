/// Deterministic hash of `(seed, s)` -- the tie-breaking/spreading mechanism bounded top-K
/// sampling uses to pick "which items" deterministically. Each part is hashed with an explicit
/// length prefix so a naive concatenation can't collide across the seed/string boundary.
pub fn seeded_hash(seed: u64, s: &str) -> u64 {
    let mut h = blake3::Hasher::new();
    for part in [&seed.to_le_bytes()[..], s.as_bytes()] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }
    u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_and_input_are_deterministic() {
        assert_eq!(seeded_hash(42, "abc"), seeded_hash(42, "abc"));
    }

    #[test]
    fn different_seeds_spread_the_same_input() {
        assert_ne!(seeded_hash(1, "abc"), seeded_hash(2, "abc"));
    }

    #[test]
    fn different_inputs_spread_the_same_seed() {
        assert_ne!(seeded_hash(1, "ab"), seeded_hash(1, "ab_extra"));
    }

    // Why hard-coded, not just "assert it matches itself": blake3's digest for a fixed input is
    // stable forever by spec, so a literal expected value is a real regression guard, catching a
    // future accidental change to the hashing scheme -- ported verbatim from the pre-extraction
    // implementation in shogiesa-cli to prove this crate's algorithm is byte-for-byte identical.
    #[test]
    fn seeded_hash_is_a_stable_golden_value() {
        assert_eq!(seeded_hash(7, "startpos"), 13402537162744184401);
    }
}
