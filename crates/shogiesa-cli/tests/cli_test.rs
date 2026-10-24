use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use assert_cmd::{Command, cargo::cargo_bin};
use predicates::prelude::*;
use tempfile::{NamedTempFile, TempDir, TempPath};

/// Reserve a temporary output path without keeping its file handle open. Windows cannot
/// atomically replace a destination while another process still holds the `NamedTempFile`
/// handle, so command outputs use this helper while writable input fixtures keep using
/// `NamedTempFile` directly.
struct ClosedTempFile(TempPath);

impl ClosedTempFile {
    fn path(&self) -> &Path {
        self.0.as_ref()
    }
}

fn closed_temp_file() -> ClosedTempFile {
    ClosedTempFile(NamedTempFile::new().unwrap().into_temp_path())
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Golden reports are canonical LF text. Normalize fixture checkout line endings so tests compare
/// CLI output, rather than the developer's Git `core.autocrlf` setting.
fn golden(name: &str) -> String {
    std::fs::read_to_string(fixture(name))
        .unwrap()
        .replace("\r\n", "\n")
}

fn shogiesa() -> Command {
    Command::cargo_bin("shogiesa").unwrap()
}

// ponytail: `fake-usi-engine` lives in a sibling crate, so plain `cargo test`
// only builds its unit-test harness, not the plain bin CARGO_BIN_EXE_ needs.
// Build it explicitly once, then reuse assert_cmd's normal lookup.
fn fake_usi_engine_bin() -> std::path::PathBuf {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = std::process::Command::new(cargo)
            .args(["build", "-p", "fake-usi-engine"])
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "failed to build fake-usi-engine");
    });
    cargo_bin("fake-usi-engine")
}

fn source_paths_in_order(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            value["source"]["path"].as_str().unwrap().to_string()
        })
        .collect()
}

#[path = "cli/adapters.rs"]
mod adapters;
#[path = "cli/diagnostics.rs"]
mod diagnostics;
#[path = "cli/distribution.rs"]
mod distribution;
#[path = "cli/extract.rs"]
mod extract;
#[path = "cli/ingest_extra.rs"]
mod ingest_extra;
#[path = "cli/label.rs"]
mod label;
#[path = "cli/regressions.rs"]
mod regressions;
#[path = "cli/workflows.rs"]
mod workflows;

use ingest_extra::position_at_ply;
use workflows::{
    make_labeled_jsonl, obs, obs_with_engine, obs_with_margin, position, position_white_to_move,
    position_with_path, source_record,
};
