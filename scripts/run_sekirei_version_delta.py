#!/usr/bin/env python3
"""Mine fixed-node search deltas between two immutable Sekirei revisions."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import shutil
import statistics
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_logged(
    name: str,
    command: list[str],
    work_dir: Path,
    *,
    env: dict[str, str] | None = None,
) -> dict[str, object]:
    log = work_dir / f"{name}.log"
    started = time.monotonic()
    with log.open("wb") as output:
        result = subprocess.run(
            command,
            cwd=work_dir,
            env=env,
            stdout=output,
            stderr=subprocess.STDOUT,
            check=False,
        )
    portable_command = []
    for argument in command:
        portable = argument
        for base, marker in ((ROOT, "<shogiesa>"), (work_dir, "<work>")):
            base_text = str(base)
            if portable == base_text:
                portable = marker
                break
            if portable.startswith(f"{base_text}{os.sep}"):
                portable = f"{marker}/{portable[len(base_text) + 1:]}"
                break
        portable_command.append(portable)
    record: dict[str, object] = {
        "name": name,
        "command": portable_command,
        "wall_time_seconds": round(time.monotonic() - started, 6),
        "exit_code": result.returncode,
    }
    if result.returncode != 0:
        record["log_tail"] = log.read_text(errors="replace")[-6000:]
        raise RuntimeError(json.dumps(record, indent=2))
    return record


def archive_ref(repository: Path, ref: str, destination: Path) -> str:
    commit = subprocess.run(
        ["git", "-C", str(repository), "rev-parse", f"{ref}^{{commit}}"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.strip()
    archive = subprocess.run(
        ["git", "-C", str(repository), "archive", ref],
        check=True,
        stdout=subprocess.PIPE,
    ).stdout
    destination.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
        tar.extractall(destination, filter="data")
    return commit


def jsonl_count(path: Path) -> int:
    with path.open(encoding="utf-8") as source:
        return sum(1 for line in source if line.strip())


def stage_corpus(source: Path, destination: Path, max_games: int) -> list[dict[str, str]]:
    if source.is_file():
        candidates = [source]
        root = source.parent
    else:
        candidates = sorted(
            path
            for path in source.rglob("*")
            if path.is_file() and path.suffix.lower() in {".csa", ".kif", ".ki2"}
        )[:max_games]
        root = source
    if not candidates:
        raise RuntimeError(f"no supported game records found under {source}")
    destination.mkdir(parents=True)
    selected = []
    for index, path in enumerate(candidates):
        staged = destination / f"{index:04d}{path.suffix.lower()}"
        shutil.copy2(path, staged)
        selected.append(
            {
                "path": str(path.relative_to(root)),
                "sha256": sha256_file(path),
            }
        )
    return selected


def percentile(values: list[int], fraction: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, int((len(ordered) - 1) * fraction))
    return ordered[index]


def analyse_delta(path: Path, baseline_name: str, candidate_name: str) -> dict[str, object]:
    records = 0
    paired = 0
    bestmove_disagreements = 0
    cp_pairs = 0
    cp_deltas: list[int] = []
    mate_or_unpaired_scores = 0
    with path.open(encoding="utf-8") as source:
        for raw in source:
            if not raw.strip():
                continue
            records += 1
            record = json.loads(raw)
            observations = {item["engine"]: item for item in record.get("observations", [])}
            baseline = observations.get(baseline_name)
            candidate = observations.get(candidate_name)
            if baseline is None or candidate is None:
                continue
            paired += 1
            if baseline.get("bestmove") != candidate.get("bestmove"):
                bestmove_disagreements += 1
            baseline_score = baseline.get("score", {})
            candidate_score = candidate.get("score", {})
            if baseline_score.get("kind") == "cp" and candidate_score.get("kind") == "cp":
                cp_pairs += 1
                cp_deltas.append(
                    abs(int(candidate_score["value"]) - int(baseline_score["value"]))
                )
            else:
                mate_or_unpaired_scores += 1
    return {
        "records": records,
        "paired_records": paired,
        "bestmove_disagreements": bestmove_disagreements,
        "bestmove_disagreement_rate": (
            bestmove_disagreements / paired if paired else None
        ),
        "cp_pairs": cp_pairs,
        "mate_or_unpaired_scores": mate_or_unpaired_scores,
        "absolute_cp_delta": {
            "mean": statistics.fmean(cp_deltas) if cp_deltas else None,
            "median": statistics.median(cp_deltas) if cp_deltas else None,
            "p90": percentile(cp_deltas, 0.90),
            "p95": percentile(cp_deltas, 0.95),
            "max": max(cp_deltas) if cp_deltas else None,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sekirei-dir", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--shogiesa", type=Path, default=ROOT / "target/release/shogiesa")
    parser.add_argument("--baseline-ref", default="v0.3.65")
    parser.add_argument("--candidate-ref", default="v0.3.66")
    parser.add_argument("--nodes", type=int, default=10_000)
    parser.add_argument("--positions", type=int, default=256)
    parser.add_argument("--mine-count", type=int, default=64)
    parser.add_argument("--max-games", type=int, default=12)
    parser.add_argument("--mined-out", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--keep-work-dir", type=Path)
    args = parser.parse_args()
    if (
        args.nodes <= 0
        or args.positions <= 0
        or args.mine_count <= 0
        or args.max_games <= 0
    ):
        parser.error(
            "--nodes, --positions, --mine-count, and --max-games must be greater than zero"
        )
    sekirei_dir = args.sekirei_dir.resolve()
    corpus = args.corpus.resolve()
    shogiesa = args.shogiesa.resolve()
    if not (sekirei_dir / ".git").is_dir():
        parser.error(f"not a Sekirei git repository: {sekirei_dir}")
    if not corpus.exists():
        parser.error(f"corpus does not exist: {corpus}")
    if not shogiesa.is_file():
        parser.error(f"shogiesa binary does not exist: {shogiesa}")

    dirty = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.splitlines()
    if dirty:
        parser.error("shogiesa working tree must be clean before an external measurement")

    temporary = None
    if args.keep_work_dir:
        work_dir = args.keep_work_dir.resolve()
        work_dir.mkdir(parents=True, exist_ok=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="shogiesa-sekirei-delta-")
        work_dir = Path(temporary.name)

    baseline_src = work_dir / "baseline-src"
    candidate_src = work_dir / "candidate-src"
    baseline_commit = archive_ref(sekirei_dir, args.baseline_ref, baseline_src)
    candidate_commit = archive_ref(sekirei_dir, args.candidate_ref, candidate_src)
    baseline_engine_name = f"sekirei-{args.baseline_ref}"
    candidate_engine_name = f"sekirei-{args.candidate_ref}"
    target_dir = work_dir / "cargo-target"
    build_env = os.environ.copy()
    build_env["CARGO_TARGET_DIR"] = str(target_dir)
    steps: list[dict[str, object]] = []

    steps.append(
        run_logged(
            "build-baseline",
            ["cargo", "build", "--locked", "--release", "-p", "sekirei"],
            baseline_src,
            env=build_env,
        )
    )
    baseline_binary = work_dir / "sekirei-baseline"
    shutil.copy2(target_dir / "release/sekirei", baseline_binary)
    steps.append(
        run_logged(
            "build-candidate",
            ["cargo", "build", "--locked", "--release", "-p", "sekirei"],
            candidate_src,
            env=build_env,
        )
    )
    candidate_binary = work_dir / "sekirei-candidate"
    shutil.copy2(target_dir / "release/sekirei", candidate_binary)

    extracted = work_dir / "extracted.jsonl"
    sampled = work_dir / "sampled.jsonl"
    baseline_labeled = work_dir / "baseline.jsonl"
    candidate_labeled = work_dir / "candidate.jsonl"
    merged = work_dir / "merged.jsonl"
    stable = work_dir / "stable.jsonl"
    mined = work_dir / "mined.jsonl"
    baseline_manifest = work_dir / "baseline-manifest.json"
    candidate_manifest = work_dir / "candidate-manifest.json"
    staged_corpus = work_dir / "corpus"
    selected_inputs = stage_corpus(corpus, staged_corpus, args.max_games)

    steps.append(
        run_logged(
            "extract",
            [
                str(shogiesa),
                "extract",
                "--input",
                str(staged_corpus),
                "--recursive",
                "--min-ply",
                "20",
                "--max-ply",
                "160",
                "--every-n-plies",
                "4",
                "--dedup",
                "--out",
                str(extracted),
            ],
            work_dir,
        )
    )
    extracted_count = jsonl_count(extracted)
    if extracted_count == 0:
        raise RuntimeError("corpus yielded zero positions; refusing an empty measurement")
    steps.append(
        run_logged(
            "sample",
            [
                str(shogiesa),
                "sample",
                "--input",
                str(extracted),
                "--count",
                str(args.positions),
                "--seed",
                "366",
                "--out",
                str(sampled),
            ],
            work_dir,
        )
    )
    sampled_count = jsonl_count(sampled)
    if sampled_count == 0:
        raise RuntimeError("sampling yielded zero positions; refusing an empty measurement")

    def label_command(
        engine: Path, engine_name: str, output: Path, manifest: Path
    ) -> list[str]:
        return [
            str(shogiesa),
            "label",
            "--input",
            str(sampled),
            "--engine",
            str(engine),
            "--engine-name",
            engine_name,
            "--nodes",
            str(args.nodes),
            "--timeout-ms",
            "60000",
            "--jobs",
            "1",
            "--engine-option",
            "Threads=1",
            "--engine-option",
            "SpecTopN=0",
            "--usi-strict",
            "--preserve-order",
            "--manifest",
            str(manifest),
            "--out",
            str(output),
        ]

    steps.append(
        run_logged(
            "label-baseline",
            label_command(
                baseline_binary,
                baseline_engine_name,
                baseline_labeled,
                baseline_manifest,
            ),
            work_dir,
        )
    )
    steps.append(
        run_logged(
            "label-candidate",
            label_command(
                candidate_binary,
                candidate_engine_name,
                candidate_labeled,
                candidate_manifest,
            ),
            work_dir,
        )
    )
    steps.append(
        run_logged(
            "merge",
            [
                str(shogiesa),
                "merge-observations",
                "--primary",
                str(baseline_labeled),
                "--secondary",
                str(candidate_labeled),
                "--out",
                str(merged),
            ],
            work_dir,
        )
    )
    steps.append(
        run_logged(
            "stability",
            [str(shogiesa), "stability", "--input", str(merged), "--out", str(stable)],
            work_dir,
        )
    )
    mine_count = min(args.mine_count, jsonl_count(stable))
    steps.append(
        run_logged(
            "mine-uncertain",
            [
                str(shogiesa),
                "select",
                "--strategy",
                "uncertain",
                "--input",
                str(stable),
                "--count",
                str(mine_count),
                "--seed",
                "366",
                "--out",
                str(mined),
            ],
            work_dir,
        )
    )

    artifact = {
        "schema": "shogiesa.sekirei-version-delta.v1",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "complete",
        "claim_boundary": "fixed-node material-evaluator diagnostic; no playing-strength claim",
        "shogiesa": {
            "commit": subprocess.run(
                ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip(),
            "binary_sha256": sha256_file(shogiesa),
        },
        "sekirei": {
            "baseline": {
                "ref": args.baseline_ref,
                "commit": baseline_commit,
                "binary_sha256": sha256_file(baseline_binary),
            },
            "candidate": {
                "ref": args.candidate_ref,
                "commit": candidate_commit,
                "binary_sha256": sha256_file(candidate_binary),
            },
            "engine_options": {"Threads": 1, "SpecTopN": 0},
            "nodes": args.nodes,
            "evaluator": "built-in material",
        },
        "corpus": {
            "path": (
                str(corpus.relative_to(sekirei_dir))
                if corpus.is_relative_to(sekirei_dir)
                else str(corpus)
            ),
            "selected_inputs": selected_inputs,
            "extracted_records": extracted_count,
            "sampled_records": sampled_count,
            "sampled_sha256": sha256_file(sampled),
        },
        "delta": analyse_delta(stable, baseline_engine_name, candidate_engine_name),
        "mined": {
            "records": jsonl_count(mined),
            "sha256": sha256_file(mined),
        },
        "manifests": {
            "baseline_sha256": sha256_file(baseline_manifest),
            "candidate_sha256": sha256_file(candidate_manifest),
        },
        "steps": steps,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(artifact, indent=2) + "\n", encoding="utf-8")
    if args.mined_out is not None:
        args.mined_out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(mined, args.mined_out)
    print(json.dumps(artifact["delta"], indent=2))
    print(f"artifact: {args.out}")
    if args.mined_out is not None:
        print(f"mined positions: {args.mined_out}")
    if temporary is not None:
        temporary.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
