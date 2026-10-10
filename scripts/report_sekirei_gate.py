#!/usr/bin/env python3
"""Validate Sekirei A/B gate artifacts and emit a provenance-preserving report."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from artifact_io import atomic_write_json, validate_path_roles


SCHEMA = "sekirei.ab_gate_result.v1"
COMPLETE_STATES = {"completed", "sprt_stopped", "inconclusive"}
TERMINAL_STATES = COMPLETE_STATES | {"failed", "partial"}
STATUSES = {"completed", "pass", "fail", "inconclusive", "error"}


class GateArtifactError(ValueError):
    """Raised when an upstream result violates its machine-readable contract."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_object(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise GateArtifactError(f"{field} must be an object")
    return value


def require_int(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise GateArtifactError(f"{field} must be a non-negative integer")
    return value


def require_number_or_none(value: Any, field: str) -> float | int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise GateArtifactError(f"{field} must be a number or null")
    return value


def require_file_identity(value: Any, field: str, complete: bool) -> dict[str, Any]:
    identity = require_object(value, field)
    if complete:
        if identity.get("exists") is not True:
            raise GateArtifactError(f"{field}.exists must be true for complete evidence")
        if not isinstance(identity.get("sha256"), str) or not identity["sha256"]:
            raise GateArtifactError(f"{field}.sha256 is required for complete evidence")
    return identity


def validate_gate_artifact(payload: Any) -> dict[str, Any]:
    root = require_object(payload, "root")
    if root.get("schema_version") != SCHEMA:
        raise GateArtifactError(f"schema_version must be {SCHEMA!r}")

    terminal_state = root.get("terminal_state")
    status = root.get("status")
    if terminal_state not in TERMINAL_STATES:
        raise GateArtifactError(f"unsupported terminal_state: {terminal_state!r}")
    if status not in STATUSES:
        raise GateArtifactError(f"unsupported status: {status!r}")
    allowed_statuses = {
        "completed": {"completed"},
        "sprt_stopped": {"pass", "fail"},
        "inconclusive": {"inconclusive"},
        "failed": {"error"},
        "partial": {"error"},
    }
    if status not in allowed_statuses[terminal_state]:
        raise GateArtifactError("status disagrees with terminal_state")

    configuration = require_object(root.get("configuration"), "configuration")
    games_limit = require_int(configuration.get("games_limit"), "configuration.games_limit")
    if games_limit == 0:
        raise GateArtifactError("configuration.games_limit must be positive")

    result = require_object(root.get("result"), "result")
    wins = require_int(result.get("wins"), "result.wins")
    losses = require_int(result.get("losses"), "result.losses")
    draws = require_int(result.get("draws"), "result.draws")
    games_played = require_int(result.get("games_played"), "result.games_played")
    if wins + losses + draws != games_played:
        raise GateArtifactError("result W/D/L does not sum to games_played")
    if games_played > games_limit:
        raise GateArtifactError("result.games_played exceeds configuration.games_limit")

    evidence = require_object(root.get("evidence"), "evidence")
    complete = evidence.get("complete")
    if not isinstance(complete, bool):
        raise GateArtifactError("evidence.complete must be a boolean")
    process_exit_code = require_int(
        evidence.get("process_exit_code"), "evidence.process_exit_code"
    )
    expected_complete = terminal_state in COMPLETE_STATES
    if complete != expected_complete:
        raise GateArtifactError("evidence.complete disagrees with terminal_state")
    if complete and process_exit_code != 0:
        raise GateArtifactError("complete evidence must have process_exit_code 0")
    if terminal_state in {"completed", "inconclusive"} and games_played != games_limit:
        raise GateArtifactError(f"{terminal_state} evidence must reach games_limit")
    if terminal_state == "sprt_stopped" and not (0 < games_played <= games_limit):
        raise GateArtifactError("sprt_stopped evidence must contain at least one game")
    elo = require_object(result.get("elo"), "result.elo")
    estimate = require_number_or_none(elo.get("estimate"), "result.elo.estimate")
    margin = require_number_or_none(elo.get("margin"), "result.elo.margin")
    if root.get("elo") != estimate or root.get("ci") != margin:
        raise GateArtifactError("top-level elo/ci must match result.elo")

    runner = require_object(evidence.get("runner"), "evidence.runner")
    if complete:
        if not isinstance(runner.get("version"), str) or not runner["version"]:
            raise GateArtifactError("evidence.runner.version is required for complete evidence")
        if not isinstance(runner.get("commit"), str) or not runner["commit"]:
            raise GateArtifactError("evidence.runner.commit is required for complete evidence")
        if runner.get("dirty") is not False:
            raise GateArtifactError("complete gate evidence requires a clean runner")
    require_file_identity(evidence.get("match_binary"), "evidence.match_binary", complete)
    engines = require_object(evidence.get("engines"), "evidence.engines")
    require_file_identity(engines.get("a"), "evidence.engines.a", complete)
    require_file_identity(engines.get("b"), "evidence.engines.b", complete)
    openings = require_file_identity(evidence.get("openings"), "evidence.openings", complete)
    if complete and require_int(openings.get("position_count"), "evidence.openings.position_count") == 0:
        raise GateArtifactError("complete gate evidence requires at least one opening")
    evaluation = require_object(evidence.get("evaluation"), "evidence.evaluation")
    if evaluation.get("mode") == "halfkp":
        require_file_identity(evaluation.get("file"), "evidence.evaluation.file", complete)
    command = evidence.get("command")
    if not isinstance(command, list) or not all(isinstance(item, str) for item in command):
        raise GateArtifactError("evidence.command must be a string array")

    return {
        "status": status,
        "terminal_state": terminal_state,
        "mode": root.get("mode"),
        "games_limit": games_limit,
        "games_played": games_played,
        "wins": wins,
        "losses": losses,
        "draws": draws,
        "elo": estimate,
        "ci": margin,
        "usable_for_gate": complete,
        "runner": runner,
        "engines": engines,
        "evaluation": evaluation,
        "openings": openings,
        "command": command,
    }


def repository_identity(root: Path) -> dict[str, Any]:
    identity: dict[str, Any] = {}
    try:
        identity["commit"] = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        identity["dirty"] = bool(
            subprocess.run(
                ["git", "-C", str(root), "status", "--porcelain"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
        )
    except (OSError, subprocess.CalledProcessError):
        identity.update({"commit": None, "dirty": None})
    return identity


def build_report(inputs: list[Path]) -> dict[str, Any]:
    runs = []
    for path in inputs:
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise GateArtifactError(f"cannot read {path}: {error}") from error
        summary = validate_gate_artifact(payload)
        summary["source"] = {"path": str(path), "sha256": sha256_file(path)}
        runs.append(summary)

    root = Path(__file__).resolve().parent.parent
    return {
        "schema": "shogiesa.sekirei-gate-report.v1",
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "claim_boundary": (
            "upstream Sekirei gate evidence and verdict pass-through; "
            "no independent Elo or playing-strength claim"
        ),
        "shogiesa": repository_identity(root),
        "environment": {"platform": platform.platform(), "python": platform.python_version()},
        "runs": runs,
        "summary": {
            "run_count": len(runs),
            "usable_run_count": sum(1 for run in runs if run["usable_for_gate"]),
            "all_evidence_complete": all(run["usable_for_gate"] for run in runs),
            "upstream_statuses": [run["status"] for run in runs],
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, action="append", required=True)
    parser.add_argument("--out", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        validate_path_roles(
            [("--input", path) for path in args.input],
            [("--out", args.out)],
        )
        report = build_report(args.input)
        atomic_write_json(args.out, report)
    except (GateArtifactError, OSError, ValueError) as error:
        raise SystemExit(f"error: {error}") from error
    print(f"Sekirei gate report: {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
