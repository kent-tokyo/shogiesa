# Release checklist

Use this checklist for each release candidate. Record commands and artifacts from the same clean
checkout. A blocked or unavailable check remains blocked; it is not a pass.

The local wrapper is:

```bash
bash scripts/release_readiness.sh
```

## Candidate validation

- [ ] workspace and publishable crate versions match the intended tag
- [ ] `bash scripts/check_repository_contract.sh`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo package --workspace --locked`
- [ ] `git diff --check` and the release checkout is clean
- [ ] malformed CSA/KIF/JSONL and pack-corruption fixtures pass
- [ ] Linux, macOS, Windows, audit, and CodeQL checks pass for the release commit

## Contract and claims

- [ ] README examples match current CLI help
- [ ] JSONL schema and pack format match code and fixtures
- [ ] CHANGELOG covers user-visible changes and compatibility breaks
- [ ] manifests preserve the available input/output, seed, engine, weight, and option identities
- [ ] throughput, RSS, training effect, interoperability, and Elo are labeled unmeasured unless a
      dated artifact records the setup and result

## Publication

- [ ] annotated tag points to the validated commit and is pushed
- [ ] GitHub Release is published from that tag
- [ ] every publishable workspace crate is present on crates.io at the same version and is not
      yanked
- [ ] release links and the dated validation log are updated after publication

## Current published release

The corresponding checks for `v0.11.1` are complete. Evidence is in the
[v0.11.1 validation log](release_validation_2026-10-10_v0.11.1.md), the
[GitHub Release](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.1), and the nine
crates.io package records. `main` may contain later unreleased changes.

Release validation proves build, test, packaging, and publication status. It does not prove that
shogiesa is fastest, improves training, or changes engine strength.
