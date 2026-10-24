# Refactoring contract

`scripts/check_refactor_contract.sh` prevents the CLI and core entry files from growing back into
single-file implementations. It verifies the current module boundaries, conservative line limits,
the single-binary integration-test layout, and the typed `PositionRecord` JSONL boundary.

The limits are regression guards, not style targets. New behavior should normally go into the
existing domain module. If a module approaches its limit, split it by responsibility and lower the
relevant limit where practical; do not raise a limit only to make CI pass.

Run it locally with:

```console
bash scripts/check_refactor_contract.sh
```

The broader `scripts/check_repository_contract.sh` runs this check as part of repository
validation.
