#!/usr/bin/env python3
"""Run a bounded four-arm, three-seed Sekirei training ablation."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.dont_write_bytecode = True

from artifact_io import atomic_write_json, validate_path_roles
from run_sekirei_version_delta import archive_ref, jsonl_count, sha256_file, stage_corpus


ROOT = Path(__file__).resolve().parent.parent


def portable_command(command: list[str], work_dir: Path) -> list[str]:
    result = []
    for argument in command:
        value = argument
        for base, marker in ((ROOT, "<shogiesa>"), (work_dir, "<work>")):
            text = str(base)
            if value == text:
                value = marker
                break
            if value.startswith(f"{text}{os.sep}"):
                value = f"{marker}/{value[len(text) + 1:]}"
                break
        result.append(value)
    return result


def run_step(
    name: str,
    command: list[str],
    cwd: Path,
    work_dir: Path,
    *,
    env: dict[str, str] | None = None,
) -> dict[str, object]:
    log = work_dir / f"{name}.log"
    started = time.monotonic()
    with log.open("wb") as output:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            stdout=output,
            stderr=subprocess.STDOUT,
            check=False,
        )
    record: dict[str, object] = {
        "name": name,
        "command": portable_command(command, work_dir),
        "wall_time_seconds": round(time.monotonic() - started, 6),
        "exit_code": result.returncode,
    }
    if result.returncode != 0:
        record["log_tail"] = log.read_text(errors="replace")[-6000:]
        raise RuntimeError(json.dumps(record, indent=2))
    return record


def sample_command(shogiesa: Path, source: Path, count: int, seed: int, out: Path) -> list[str]:
    return [
        str(shogiesa),
        "sample",
        "--input",
        str(source),
        "--count",
        str(count),
        "--seed",
        str(seed),
        "--out",
        str(out),
    ]


def parse_seeds(value: str) -> list[int]:
    try:
        seeds = [int(item) for item in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("seeds must be comma-separated integers") from error
    if len(seeds) != 3 or len(set(seeds)) != 3:
        raise argparse.ArgumentTypeError("exactly three distinct seeds are required")
    return seeds


def mean_stdev(values: list[float]) -> dict[str, float]:
    return {
        "mean": statistics.fmean(values),
        "stdev": statistics.stdev(values) if len(values) > 1 else 0.0,
        "min": min(values),
        "max": max(values),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sekirei-dir", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--shogiesa", type=Path, default=ROOT / "target/release/shogiesa")
    parser.add_argument("--baseline-ref", default="v0.3.67")
    parser.add_argument("--candidate-ref", default="v0.3.68")
    parser.add_argument("--max-games", type=int, default=24)
    parser.add_argument("--positions", type=int, default=512)
    parser.add_argument("--arm-size", type=int, default=64)
    parser.add_argument("--diagnostic-nodes", type=int, default=2_000)
    parser.add_argument("--teacher-depth", type=int, default=1)
    parser.add_argument("--seeds", type=parse_seeds, default=parse_seeds("101,202,303"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--keep-work-dir", type=Path)
    args = parser.parse_args()
    for name in ("max_games", "positions", "arm_size", "diagnostic_nodes", "teacher_depth"):
        if getattr(args, name) <= 0:
            parser.error(f"--{name.replace('_', '-')} must be greater than zero")

    sekirei_dir = args.sekirei_dir.resolve()
    corpus = args.corpus.resolve()
    shogiesa = args.shogiesa.resolve()
    if not (sekirei_dir / ".git").is_dir():
        parser.error(f"not a Sekirei git repository: {sekirei_dir}")
    if not corpus.exists() or not shogiesa.is_file():
        parser.error("corpus and release shogiesa binary must exist")
    try:
        validate_path_roles(
            [("--corpus", corpus), ("--shogiesa", shogiesa)],
            [("--out", args.out)],
        )
    except ValueError as error:
        parser.error(str(error))
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
        temporary = tempfile.TemporaryDirectory(prefix="shogiesa-learning-ablation-")
        work_dir = Path(temporary.name)

    baseline_src = work_dir / "baseline-src"
    candidate_src = work_dir / "candidate-src"
    baseline_commit = archive_ref(sekirei_dir, args.baseline_ref, baseline_src)
    candidate_commit = archive_ref(sekirei_dir, args.candidate_ref, candidate_src)
    target_dir = work_dir / "cargo-target"
    build_env = os.environ.copy()
    build_env["CARGO_TARGET_DIR"] = str(target_dir)
    steps: list[dict[str, object]] = []
    steps.append(
        run_step(
            "build-baseline",
            ["cargo", "build", "--locked", "--release", "-p", "sekirei"],
            baseline_src,
            work_dir,
            env=build_env,
        )
    )
    baseline_engine = work_dir / "sekirei-baseline"
    shutil.copy2(target_dir / "release/sekirei", baseline_engine)
    steps.append(
        run_step(
            "build-candidate-and-trainer",
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "-p",
                "sekirei",
                "-p",
                "sekirei-train",
            ],
            candidate_src,
            work_dir,
            env=build_env,
        )
    )
    candidate_engine = work_dir / "sekirei-candidate"
    trainer = work_dir / "train"
    shutil.copy2(target_dir / "release/sekirei", candidate_engine)
    shutil.copy2(target_dir / "release/train", trainer)

    staged_corpus = work_dir / "corpus"
    selected_inputs = stage_corpus(corpus, staged_corpus, args.max_games)
    extracted = work_dir / "extracted.jsonl"
    sampled = work_dir / "sampled.jsonl"
    baseline_labeled = work_dir / "baseline.jsonl"
    candidate_labeled = work_dir / "candidate.jsonl"
    merged = work_dir / "merged.jsonl"
    stable = work_dir / "stable.jsonl"
    train = work_dir / "train.jsonl"
    valid = work_dir / "valid.jsonl"
    test = work_dir / "test.jsonl"
    steps.append(
        run_step(
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
            ROOT,
            work_dir,
        )
    )
    if jsonl_count(extracted) == 0:
        raise RuntimeError("corpus yielded zero positions")
    steps.append(run_step("sample", sample_command(shogiesa, extracted, args.positions, 366, sampled), ROOT, work_dir))

    def label(engine: Path, name: str, out: Path) -> list[str]:
        return [
            str(shogiesa),
            "label",
            "--input",
            str(sampled),
            "--engine",
            str(engine),
            "--engine-name",
            name,
            "--nodes",
            str(args.diagnostic_nodes),
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
            "--out",
            str(out),
        ]

    baseline_name = f"sekirei-{args.baseline_ref}"
    candidate_name = f"sekirei-{args.candidate_ref}"
    steps.append(run_step("label-baseline", label(baseline_engine, baseline_name, baseline_labeled), ROOT, work_dir))
    steps.append(run_step("label-candidate", label(candidate_engine, candidate_name, candidate_labeled), ROOT, work_dir))
    steps.append(
        run_step(
            "merge",
            [str(shogiesa), "merge-observations", "--primary", str(baseline_labeled), "--secondary", str(candidate_labeled), "--out", str(merged)],
            ROOT,
            work_dir,
        )
    )
    steps.append(run_step("stability", [str(shogiesa), "stability", "--input", str(merged), "--out", str(stable)], ROOT, work_dir))
    steps.append(
        run_step(
            "split",
            [
                str(shogiesa),
                "split",
                "--input",
                str(stable),
                "--train",
                str(train),
                "--valid",
                str(valid),
                "--test",
                str(test),
                "--valid-frac",
                "0.2",
                "--test-frac",
                "0.2",
                "--seed",
                "366",
            ],
            ROOT,
            work_dir,
        )
    )
    split_counts = {name: jsonl_count(path) for name, path in (("train", train), ("valid", valid), ("test", test))}
    if min(split_counts.values()) == 0:
        raise RuntimeError(f"source split produced an empty partition: {split_counts}")

    filtered_candidates = work_dir / "filtered-candidates.jsonl"
    mined_candidates = work_dir / "mined-candidates.jsonl"
    balanced_candidates = work_dir / "balanced-candidates.jsonl"
    filter_manifest = work_dir / "filter-manifest.json"
    balance_manifest = work_dir / "balance-manifest.json"
    steps.append(
        run_step(
            "filter-arm",
            [
                str(shogiesa),
                "filter",
                "--input",
                str(train),
                "--require-engine-agreement",
                "--max-engine-score-swing-cp",
                "150",
                "--exclude-mate",
                "--manifest",
                str(filter_manifest),
                "--out",
                str(filtered_candidates),
            ],
            ROOT,
            work_dir,
        )
    )
    steps.append(
        run_step(
            "mine-arm",
            [str(shogiesa), "select", "--strategy", "uncertain", "--input", str(train), "--count", str(min(args.arm_size, split_counts["train"])), "--seed", "366", "--out", str(mined_candidates)],
            ROOT,
            work_dir,
        )
    )
    steps.append(
        run_step(
            "balance-arm",
            [str(shogiesa), "balance", "--input", str(train), "--by", "phase", "--manifest", str(balance_manifest), "--out", str(balanced_candidates)],
            ROOT,
            work_dir,
        )
    )
    candidate_paths = {
        "baseline": train,
        "filtered": filtered_candidates,
        "mined": mined_candidates,
        "balanced": balanced_candidates,
    }
    candidate_counts = {name: jsonl_count(path) for name, path in candidate_paths.items()}
    arm_size = min(args.arm_size, *candidate_counts.values())
    if arm_size < 16:
        raise RuntimeError(f"too few common arm positions ({arm_size}): {candidate_counts}")
    arms: dict[str, Path] = {}
    for index, (name, source) in enumerate(candidate_paths.items()):
        out = work_dir / f"arm-{name}.jsonl"
        steps.append(run_step(f"sample-arm-{name}", sample_command(shogiesa, source, arm_size, 366 + index, out), ROOT, work_dir))
        arms[name] = out

    runs: list[dict[str, object]] = []
    valid_line = re.compile(r"valid: loss_raw=([0-9.eE+-]+)\s+loss_weighted=([0-9.eE+-]+)")
    for arm_name, arm_path in arms.items():
        for seed in args.seeds:
            run_dir = work_dir / f"run-{arm_name}-{seed}"
            checkpoint_dir = run_dir / "checkpoints"
            checkpoint_dir.mkdir(parents=True)
            output = run_dir / "weights.bin"
            command = [
                str(trainer),
                "--positions",
                str(arm_path),
                "--validation-positions",
                str(valid),
                "--strict-positions",
                "--output",
                str(output),
                "--checkpoint-dir",
                str(checkpoint_dir),
                "--epochs",
                "1",
                "--label-depth",
                str(args.teacher_depth),
                "--teacher-eval",
                "material",
                "--lr-schedule",
                "constant",
                "--init-seed",
                str(seed),
                "--split-seed",
                "366",
                "--shuffle-seed",
                str(seed),
            ]
            step = run_step(f"train-{arm_name}-{seed}", command, candidate_src, work_dir)
            log_text = (work_dir / f"train-{arm_name}-{seed}.log").read_text(errors="replace")
            match = valid_line.search(log_text)
            meta_path = checkpoint_dir / "weights.epoch1.meta.json"
            if match is None or not meta_path.is_file():
                raise RuntimeError(f"missing validation metrics for {arm_name} seed {seed}")
            meta = json.loads(meta_path.read_text())
            runs.append(
                {
                    "arm": arm_name,
                    "seed": seed,
                    "wall_time_seconds": step["wall_time_seconds"],
                    "valid_loss_raw": float(match.group(1)),
                    "valid_loss_weighted": float(match.group(2)),
                    "valid_cp_mse": meta["valid_cp_mse"],
                    "valid_output_std": meta["valid_output_std"],
                    "valid_output_range": meta["valid_output_range"],
                    "train_count": meta["train_count"],
                    "valid_count": meta["valid_count"],
                    "param_update_norm": meta["param_update_norm"],
                    "checkpoint_hash": meta["checkpoint_hash"],
                }
            )
            shutil.rmtree(run_dir)

    summary = {}
    for arm_name in arms:
        arm_runs = [run for run in runs if run["arm"] == arm_name]
        summary[arm_name] = {
            "valid_cp_mse": mean_stdev([float(run["valid_cp_mse"]) for run in arm_runs]),
            "valid_loss_raw": mean_stdev([float(run["valid_loss_raw"]) for run in arm_runs]),
        }

    artifact = {
        "schema": "shogiesa.sekirei-learning-ablation.v1",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "complete",
        "claim_boundary": "small-corpus one-epoch diagnostic; no playing-strength or generalization claim",
        "shogiesa": {
            "commit": subprocess.run(["git", "-C", str(ROOT), "rev-parse", "HEAD"], check=True, stdout=subprocess.PIPE, text=True).stdout.strip(),
            "binary_sha256": sha256_file(shogiesa),
        },
        "sekirei": {
            "baseline": {"ref": args.baseline_ref, "commit": baseline_commit, "binary_sha256": sha256_file(baseline_engine)},
            "candidate": {"ref": args.candidate_ref, "commit": candidate_commit, "binary_sha256": sha256_file(candidate_engine)},
            "trainer_sha256": sha256_file(trainer),
            "diagnostic_nodes": args.diagnostic_nodes,
            "teacher_depth": args.teacher_depth,
            "engine_options": {"Threads": 1, "SpecTopN": 0},
        },
        "corpus": {
            "path": str(corpus.relative_to(sekirei_dir)) if corpus.is_relative_to(sekirei_dir) else str(corpus),
            "selected_inputs": selected_inputs,
            "extracted_records": jsonl_count(extracted),
            "sampled_records": jsonl_count(sampled),
            "sampled_sha256": sha256_file(sampled),
            "split_counts": split_counts,
            "validation_sha256": sha256_file(valid),
            "test_sha256": sha256_file(test),
        },
        "arms": {
            name: {
                "candidate_records": candidate_counts[name],
                "training_records": arm_size,
                "sha256": sha256_file(path),
            }
            for name, path in arms.items()
        },
        "seeds": args.seeds,
        "runs": runs,
        "summary": summary,
        "manifests": {
            "filter_sha256": sha256_file(filter_manifest),
            "balance_sha256": sha256_file(balance_manifest),
        },
        "preparation_steps": steps,
    }
    atomic_write_json(args.out, artifact)
    print(json.dumps(summary, indent=2))
    print(f"artifact: {args.out}")
    if temporary is not None:
        temporary.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
