#!/usr/bin/env python3

from __future__ import annotations

import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from report_sekirei_gate import GateArtifactError, build_report, validate_gate_artifact


FIXTURE = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "fixtures"
    / "sekirei_ab_gate_result_v1.json"
)


class SekireiGateReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.payload = json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_valid_completed_result_is_preserved(self) -> None:
        summary = validate_gate_artifact(self.payload)
        self.assertEqual(summary["status"], "completed")
        self.assertEqual(summary["games_played"], 4)
        self.assertTrue(summary["usable_for_gate"])

    def test_rejects_unknown_schema(self) -> None:
        self.payload["schema_version"] = "sekirei.ab_gate_result.v2"
        with self.assertRaisesRegex(GateArtifactError, "schema_version"):
            validate_gate_artifact(self.payload)

    def test_rejects_inconsistent_game_counts(self) -> None:
        self.payload["result"]["draws"] = 3
        with self.assertRaisesRegex(GateArtifactError, "does not sum"):
            validate_gate_artifact(self.payload)

    def test_rejects_false_complete_partial_result(self) -> None:
        self.payload["terminal_state"] = "partial"
        self.payload["status"] = "error"
        with self.assertRaisesRegex(GateArtifactError, "evidence.complete"):
            validate_gate_artifact(self.payload)

    def test_rejects_status_that_disagrees_with_terminal_state(self) -> None:
        self.payload["status"] = "inconclusive"
        with self.assertRaisesRegex(GateArtifactError, "status disagrees"):
            validate_gate_artifact(self.payload)

    def test_rejects_complete_result_without_binary_identity(self) -> None:
        self.payload["evidence"]["engines"]["a"].pop("sha256")
        with self.assertRaisesRegex(GateArtifactError, "engines.a.sha256"):
            validate_gate_artifact(self.payload)

    def test_build_report_does_not_recompute_upstream_verdict(self) -> None:
        payload = copy.deepcopy(self.payload)
        payload["elo"] = 999.0
        payload["result"]["elo"]["estimate"] = 999.0
        with tempfile.TemporaryDirectory() as raw_dir:
            path = Path(raw_dir) / "gate.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            report = build_report([path])
        self.assertEqual(report["summary"]["upstream_statuses"], ["completed"])
        self.assertIn("pass-through", report["claim_boundary"])


if __name__ == "__main__":
    unittest.main()
