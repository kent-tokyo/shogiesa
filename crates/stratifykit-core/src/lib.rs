//! Domain-agnostic distribution-control primitives: bounded top-K sampling, group-aware quota
//! fill, and coverage classification. No knowledge of any particular record type -- callers
//! supply plain closures (`bucket_key_fn`, `group_key_fn`) to map their own domain's rows onto
//! the generic `FeatureKey`/`BucketKey`/`GroupKey` string concepts used here.

mod coverage;
mod hash;
mod heap;
mod quota;
mod sampling;

pub use coverage::{BucketStatus, bucket_floor, classify_bucket, mean_of};
pub use hash::seeded_hash;
pub use heap::{HeapEntry, TotalF32, push_bounded};
pub use quota::QuotaSpec;
pub use sampling::{GroupAwareFillResult, group_aware_fill, reservoir_sample};

/// A single dimension's value for one record (e.g. a phase name, a side name) -- callers compose
/// these into a [`BucketKey`] however their domain defines "bucket".
pub type FeatureKey = String;
/// A composite bucket identifier a caller's sampling/coverage quota is keyed on.
pub type BucketKey = String;
/// Identifies which correlated group (e.g. one source game) a record belongs to, for
/// group-aware sampling that keeps one group from consuming an entire bucket's quota.
pub type GroupKey = String;

#[cfg(test)]
mod tests {
    /// This crate's whole reason to exist is being usable by a future non-shogi consumer
    /// (masstrust/quietset-style products, per its README) -- a `shogiesa-*` dependency creeping
    /// into `Cargo.toml` would silently break that promise. `include_str!` reads the file at
    /// compile time, so this fails the moment such a dependency is added, not just when someone
    /// remembers to check.
    #[test]
    fn cargo_toml_has_no_shogiesa_dependency() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            !manifest.contains("shogiesa"),
            "stratifykit-core must stay zero-shogi-vocabulary; found a `shogiesa` reference in Cargo.toml"
        );
    }
}
