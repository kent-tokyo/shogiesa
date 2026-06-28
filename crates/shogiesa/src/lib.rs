//! **将棋の餌。** Shogi training-data feed for NNUE engines.
//!
//! This is the meta crate for shogiesa. It re-exports the core library crates.
//!
//! # Crates
//!
//! | Crate | Purpose |
//! |---|---|
//! | [`shogiesa-core`](https://crates.io/crates/shogiesa-core) | Shared domain types (`PositionRecord`, `Score`, `Sfen`, …) |
//! | [`shogiesa-csa`](https://crates.io/crates/shogiesa-csa)   | CSA game record ingestion → SFEN extraction |
//! | [`shogiesa-usi`](https://crates.io/crates/shogiesa-usi)   | USI engine communication for position labeling |
//! | [`shogiesa-cli`](https://crates.io/crates/shogiesa-cli)   | CLI binary (`shogiesa extract / label / report / validate`) |
//!
//! # Install the CLI
//!
//! ```bash
//! cargo install shogiesa-cli
//! ```

pub use shogiesa_core as core;
pub use shogiesa_csa as csa;
pub use shogiesa_usi as usi;
