// ponytail: empty lib target so other crates can depend on this bin-only
// crate as a dev-dependency — Cargo ignores path deps with no lib target,
// which silently breaks `CARGO_BIN_EXE_fake-usi-engine` for assert_cmd.
