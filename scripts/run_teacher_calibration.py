#!/usr/bin/env python3
"""Run a reproducible USI teacher/search/quality calibration matrix."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.dont_write_bytecode = True


ROOT = Path(__file__).resolve().parent.parent
NAME_RE = re.compile(r"^[A-Za-z0-9_.-]+$")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def jsonl_count(path: Path) -> int:
    with path.open(encoding="utf-8") as source:
        return sum(1 for line in source if line.strip())


def parse_positive_csv(value: str) -> list[int]:
    parts = [item.strip() for item in value.split(",")]
    if not parts or any(not item for item in parts):
        raise argparse.ArgumentTypeError(
            "expected comma-separated positive integers without empty entries"
        )
    try:
        values = [int(item) for item in parts]
    except ValueError as error:
        raise argparse.ArgumentTypeError("expected comma-separated integers") from error
    if not values or any(item <= 0 for item in values) or len(values) != len(set(values)):
        raise argparse.ArgumentTypeError("values must be distinct positive integers")
    return values


def parse_engine_option(value: str) -> str:
    if "=" not in value:
        raise argparse.ArgumentTypeError("engine option must use NAME=VALUE")
    name, _ = value.split("=", 1)
    if not name.strip():
        raise argparse.ArgumentTypeError("engine option NAME must not be empty")
    if name.lower() == "multipv":
        raise argparse.ArgumentTypeError(
            "configure MultiPV with --multipv so every matrix case has one identity"
        )
    return value


def parse_teacher(value: str) -> tuple[str, Path]:
    if "=" not in value:
        raise argparse.ArgumentTypeError("teacher must use NAME=PATH")
    name, raw_path = value.split("=", 1)
    if not NAME_RE.fullmatch(name):
        raise argparse.ArgumentTypeError(
            "teacher NAME may contain only letters, digits, dot, underscore, and hyphen"
        )
    if not raw_path:
        raise argparse.ArgumentTypeError("teacher PATH must not be empty")
    return name, Path(raw_path)


def summarize_probe_output(stdout: str, stderr: str, require_weight_ack: bool) -> dict[str, object]:
    lines = [line.strip() for line in stdout.splitlines() if line.strip()]
    if "usiok" not in lines:
        raise RuntimeError("teacher preflight did not receive usiok")
    if "readyok" not in lines:
        raise RuntimeError("teacher preflight did not receive readyok")
    failures = [line for line in lines if "weight load failed" in line]
    acknowledgements = [line for line in lines if "NNUE weights loaded from" in line]
    if failures:
        raise RuntimeError(f"teacher preflight reported {failures[0]}")
    if require_weight_ack and len(acknowledgements) != 1:
        raise RuntimeError(
            "teacher preflight requires exactly one NNUE weight-load acknowledgement"
        )
    formats = [
        line.removeprefix("info string evaluator format ")
        for line in lines
        if line.startswith("info string evaluator format ")
    ]
    names = [line.removeprefix("id name ") for line in lines if line.startswith("id name ")]
    transcript = f"{stdout}\n<stderr>\n{stderr}".encode()
    return {
        "usiok": True,
        "readyok": True,
        "engine_name": names[0] if names else None,
        "weight_load_acknowledged": bool(acknowledgements),
        "evaluator_format": formats[0] if formats else None,
        "transcript_sha256": hashlib.sha256(transcript).hexdigest(),
    }


def probe_teacher(
    engine: Path,
    options: list[str],
    timeout_ms: int,
    require_weight_ack: bool,
) -> dict[str, object]:
    commands = ["usi"]
    for option in options:
        name, value = option.split("=", 1)
        commands.append(f"setoption name {name} value {value}")
    commands.extend(["isready", "quit"])
    try:
        result = subprocess.run(
            [str(engine)],
            input="\n".join(commands) + "\n",
            capture_output=True,
            text=True,
            timeout=max(5.0, timeout_ms / 1000),
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(f"teacher preflight timed out: {engine}") from error
    if result.returncode != 0:
        raise RuntimeError(f"teacher preflight exited {result.returncode}: {engine}")
    return summarize_probe_output(result.stdout, result.stderr, require_weight_ack)


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
    work_dir: Path,
    *,
    cwd: Path = ROOT,
) -> dict[str, object]:
    log = work_dir / f"{name}.log"
    started = time.monotonic()
    with log.open("wb") as output:
        result = subprocess.run(
            command,
            cwd=cwd,
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


def record_key(record: dict[str, object]) -> str:
    return json.dumps(
        [record.get("sfen"), record.get("source")],
        sort_keys=True,
        separators=(",", ":"),
    )


def load_case(path: Path, engine_name: str) -> tuple[int, dict[str, dict[str, object]]]:
    total = 0
    seen: set[str] = set()
    observations: dict[str, dict[str, object]] = {}
    with path.open(encoding="utf-8") as source:
        for raw in source:
            if not raw.strip():
                continue
            total += 1
            record = json.loads(raw)
            key = record_key(record)
            if key in seen:
                raise RuntimeError(f"duplicate record identity in {path}: {key}")
            seen.add(key)
            matches = [
                item
                for item in record.get("observations", [])
                if item.get("engine") == engine_name
            ]
            if len(matches) > 1:
                raise RuntimeError(f"duplicate observation for {engine_name}")
            if matches:
                observations[key] = matches[0]
    return total, observations


def count_existing_observations(path: Path) -> int:
    count = 0
    with path.open(encoding="utf-8") as source:
        for raw in source:
            if raw.strip():
                record = json.loads(raw)
                count += len(record.get("observations", []))
    return count


def write_json_atomic(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, raw_temporary = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(raw_temporary)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    except BaseException:
        temporary.unlink(missing_ok=True)
        raise


def percentile(values: list[float], fraction: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = max(0, min(len(ordered) - 1, math.ceil(fraction * len(ordered)) - 1))
    return ordered[index]


def score_cp(observation: dict[str, object]) -> int | None:
    score = observation.get("score")
    if not isinstance(score, dict) or score.get("kind") != "cp":
        return None
    value = score.get("value")
    return int(value) if isinstance(value, int) else None


def is_special_bestmove(observation: dict[str, object]) -> bool:
    return observation.get("bestmove_kind") is not None or observation.get("bestmove") in {
        "resign",
        "win",
        "none",
    }


def summarize_case(
    total: int, observations: dict[str, dict[str, object]]
) -> dict[str, object]:
    values = list(observations.values())
    exact = sum(item.get("score_bound", "exact") == "exact" for item in values)
    cp_count = sum(score_cp(item) is not None for item in values)
    mate_count = sum(
        isinstance(item.get("score"), dict) and item["score"].get("kind") == "mate"
        for item in values
    )
    margins = [
        int(item["policy_margin_cp"])
        for item in values
        if isinstance(item.get("policy_margin_cp"), int)
    ]
    underreached = 0
    for item in values:
        requested = item.get("requested_depth")
        actual = item.get("depth")
        score = item.get("score")
        is_mate = isinstance(score, dict) and score.get("kind") == "mate"
        if isinstance(requested, int) and isinstance(actual, int) and actual < requested and not is_mate:
            underreached += 1
    return {
        "input_records": total,
        "labeled_records": len(values),
        "coverage_fraction": len(values) / total if total else None,
        "cp_scores": cp_count,
        "mate_scores": mate_count,
        "exact_scores": exact,
        "non_exact_scores": len(values) - exact,
        "timeout_salvaged": sum(bool(item.get("was_timeout_salvaged")) for item in values),
        "requested_depth_underreached": underreached,
        "special_bestmoves": sum(is_special_bestmove(item) for item in values),
        "candidate_coverage": sum(bool(item.get("candidates")) for item in values),
        "policy_margin_coverage": len(margins),
        "policy_margin_cp": {
            "mean": statistics.fmean(margins) if margins else None,
            "median": statistics.median(margins) if margins else None,
            "p10": percentile([float(value) for value in margins], 0.10),
        },
        "engine_versions": sorted(
            {
                str(item["engine_version"])
                for item in values
                if item.get("engine_version") is not None
            }
        ),
    }


def summarize_ensemble(
    total: int,
    observation_maps: list[dict[str, dict[str, object]]],
) -> dict[str, object]:
    if not observation_maps:
        raise ValueError("at least one observation map is required")
    common_keys = set(observation_maps[0])
    for mapping in observation_maps[1:]:
        common_keys.intersection_update(mapping)
    comparable = 0
    agreement = 0
    cp_spreads: list[float] = []
    for key in sorted(common_keys):
        observations = [mapping[key] for mapping in observation_maps]
        ordinary = [item for item in observations if not is_special_bestmove(item)]
        if len(ordinary) >= 2:
            comparable += 1
            if len({str(item.get("bestmove")) for item in ordinary}) == 1:
                agreement += 1
        cps = [value for value in (score_cp(item) for item in observations) if value is not None]
        if len(cps) >= 2:
            cp_spreads.append(float(max(cps) - min(cps)))
    return {
        "input_records": total,
        "teacher_count": len(observation_maps),
        "all_teachers_labeled": len(common_keys),
        "all_teachers_coverage_fraction": len(common_keys) / total if total else None,
        "bestmove_comparable": comparable,
        "bestmove_agreements": agreement,
        "bestmove_disagreements": comparable - agreement,
        "bestmove_agreement_fraction": agreement / comparable if comparable else None,
        "cp_spread": {
            "pairs": len(cp_spreads),
            "mean": statistics.fmean(cp_spreads) if cp_spreads else None,
            "median": statistics.median(cp_spreads) if cp_spreads else None,
            "p95": percentile(cp_spreads, 0.95),
            "max": max(cp_spreads) if cp_spreads else None,
        },
    }


QUALITY_PROFILES: dict[str, list[str]] = {
    "exact": ["--require-exact-score", "--exclude-timeout-salvaged"],
    "stable150": [
        "--require-exact-score",
        "--exclude-timeout-salvaged",
        "--max-score-swing-cp",
        "150",
    ],
    "consensus150": [
        "--require-exact-score",
        "--exclude-timeout-salvaged",
        "--max-score-swing-cp",
        "150",
        "--require-engine-agreement",
        "--max-engine-score-swing-cp",
        "150",
    ],
    "policy50": [
        "--require-exact-score",
        "--exclude-timeout-salvaged",
        "--max-score-swing-cp",
        "150",
        "--require-engine-agreement",
        "--max-engine-score-swing-cp",
        "150",
        "--require-policy-margin",
        "--min-policy-margin-cp",
        "50",
    ],
}


def manifest_summary(path: Path) -> dict[str, object]:
    manifest = json.loads(path.read_text(encoding="utf-8"))
    return {
        "records_kept": manifest.get("records_kept"),
        "records_dropped": manifest.get("records_dropped"),
        "drop_reasons": manifest.get("drop_reasons", {}),
        "manifest_sha256": sha256_file(path),
    }


def merge_cases(
    shogiesa: Path,
    case_paths: list[Path],
    destination: Path,
    stem: str,
    work_dir: Path,
    steps: list[dict[str, object]],
) -> None:
    shutil.copy2(case_paths[0], destination)
    for index, secondary in enumerate(case_paths[1:], start=2):
        merged = work_dir / f"{stem}-merge-{index}.jsonl"
        steps.append(
            run_step(
                f"{stem}-merge-{index}",
                [
                    str(shogiesa),
                    "merge-observations",
                    "--primary",
                    str(destination),
                    "--secondary",
                    str(secondary),
                    "--out",
                    str(merged),
                ],
                work_dir,
            )
        )
        os.replace(merged, destination)


def self_test() -> int:
    total = 3
    first = {
        "a": {
            "score": {"kind": "cp", "value": 20},
            "score_bound": "exact",
            "bestmove": "7g7f",
            "policy_margin_cp": 80,
            "candidates": [{"multipv": 1}],
        },
        "b": {
            "score": {"kind": "cp", "value": -10},
            "score_bound": "lowerbound",
            "bestmove": "2g2f",
        },
    }
    second = {
        "a": {
            "score": {"kind": "cp", "value": 30},
            "score_bound": "exact",
            "bestmove": "7g7f",
        },
        "b": {
            "score": {"kind": "cp", "value": 40},
            "score_bound": "exact",
            "bestmove": "8h2b+",
        },
        "c": {
            "score": {"kind": "mate", "moves": 3},
            "score_bound": "exact",
            "bestmove": "5a5b",
        },
    }
    case = summarize_case(total, first)
    assert case["labeled_records"] == 2
    assert case["coverage_fraction"] == 2 / 3
    assert case["exact_scores"] == 1
    assert case["non_exact_scores"] == 1
    assert case["policy_margin_coverage"] == 1
    ensemble = summarize_ensemble(total, [first, second])
    assert ensemble["all_teachers_labeled"] == 2
    assert ensemble["bestmove_agreements"] == 1
    assert ensemble["bestmove_disagreements"] == 1
    assert ensemble["cp_spread"]["median"] == 30.0
    assert ensemble["cp_spread"]["p95"] == 50.0
    assert parse_positive_csv("4,8") == [4, 8]
    for malformed in ("", "4,", ",4", "4,,8"):
        try:
            parse_positive_csv(malformed)
        except argparse.ArgumentTypeError:
            pass
        else:
            raise AssertionError(f"accepted malformed positive CSV: {malformed!r}")
    assert parse_engine_option("Threads=1") == "Threads=1"
    probe = summarize_probe_output(
        "id name Sekirei\nusiok\ninfo string NNUE weights loaded from weights.bin\n"
        "info string evaluator format sekirei-nnue-output-v1\nreadyok\n",
        "",
        True,
    )
    assert probe["weight_load_acknowledged"] is True
    assert probe["evaluator_format"] == "sekirei-nnue-output-v1"
    try:
        summarize_probe_output("usiok\nreadyok\n", "", True)
    except RuntimeError:
        pass
    else:
        raise AssertionError("accepted a missing NNUE weight-load acknowledgement")
    print("teacher calibration self-test: PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true", help="test pure metric aggregation")
    parser.add_argument("--input", type=Path, help="unlabeled PositionRecord JSONL")
    parser.add_argument(
        "--shogiesa",
        type=Path,
        default=ROOT / "target/release/shogiesa",
        help="prebuilt shogiesa binary",
    )
    parser.add_argument(
        "--teacher",
        action="append",
        type=parse_teacher,
        default=[],
        help="ordered teacher identity as NAME=PATH; repeat for ensembles",
    )
    parser.add_argument(
        "--weight",
        action="append",
        type=parse_teacher,
        default=[],
        help="optional teacher weight identity as NAME=PATH",
    )
    parser.add_argument(
        "--depths",
        type=parse_positive_csv,
        default=parse_positive_csv("4,8"),
        help="comma-separated depth limits (default: 4,8)",
    )
    parser.add_argument("--nodes", type=parse_positive_csv, help="comma-separated node limits")
    parser.add_argument(
        "--multipv",
        type=parse_positive_csv,
        default=parse_positive_csv("1,3"),
        help="comma-separated MultiPV values (default: 1,3)",
    )
    parser.add_argument("--positions", type=int, default=256, help="sample size")
    parser.add_argument("--seed", type=int, default=367, help="sample seed")
    parser.add_argument("--timeout-ms", type=int, default=60_000)
    parser.add_argument(
        "--engine-option",
        action="append",
        type=parse_engine_option,
        default=[],
        help="USI NAME=VALUE option applied to every teacher",
    )
    parser.add_argument("--out", type=Path, help="output artifact JSON")
    parser.add_argument("--keep-work-dir", type=Path, help="retain intermediate files here")
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="allow a dirty tree but mark the artifact candidate",
    )
    parser.add_argument(
        "--require-weight-ack",
        action="store_true",
        help="fail unless every teacher acknowledges loading the declared EvalFile",
    )
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.input is None or args.out is None or not args.teacher:
        parser.error("--input, --out, and at least one --teacher NAME=PATH are required")
    if args.positions <= 0 or args.timeout_ms <= 0:
        parser.error("--positions and --timeout-ms must be greater than zero")
    teacher_names = [name for name, _ in args.teacher]
    if len(teacher_names) != len(set(teacher_names)):
        parser.error("teacher names must be unique")
    option_names = [option.split("=", 1)[0].lower() for option in args.engine_option]
    if len(option_names) != len(set(option_names)):
        parser.error("engine option names must be unique")

    source = args.input.resolve()
    shogiesa = args.shogiesa.resolve()
    teachers = [(name, path.resolve()) for name, path in args.teacher]
    weights = {name: path.resolve() for name, path in args.weight}
    if len(weights) != len(args.weight):
        parser.error("weight names must be unique")
    unknown_weights = sorted(set(weights) - set(teacher_names))
    if unknown_weights:
        parser.error(f"weights reference unknown teachers: {', '.join(unknown_weights)}")
    if not source.is_file() or not shogiesa.is_file():
        parser.error("input and shogiesa binary must exist")
    if not os.access(shogiesa, os.X_OK):
        parser.error(f"shogiesa binary is not executable: {shogiesa}")
    for name, path in teachers:
        if not path.is_file():
            parser.error(f"teacher binary does not exist: {name}={path}")
        if not os.access(path, os.X_OK):
            parser.error(f"teacher binary is not executable: {name}={path}")
    for name, path in weights.items():
        if not path.is_file():
            parser.error(f"weight file does not exist: {name}={path}")
    if args.require_weight_ack:
        missing_weights = sorted(set(teacher_names) - set(weights))
        if missing_weights:
            parser.error(
                "--require-weight-ack needs --weight for every teacher: "
                + ", ".join(missing_weights)
            )
        eval_options = [
            option.split("=", 1)[1]
            for option in args.engine_option
            if option.split("=", 1)[0].lower() == "evalfile"
        ]
        if len(eval_options) != 1:
            parser.error("--require-weight-ack needs exactly one --engine-option EvalFile=PATH")
        eval_path = Path(eval_options[0]).expanduser().resolve()
        if not eval_path.is_file():
            parser.error(f"EvalFile does not exist: {eval_path}")
        eval_hash = sha256_file(eval_path)
        mismatched = sorted(
            name for name, path in weights.items() if sha256_file(path) != eval_hash
        )
        if mismatched:
            parser.error(
                "EvalFile bytes do not match declared teacher weights: "
                + ", ".join(mismatched)
            )

    dirty_paths = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.splitlines()
    if dirty_paths and not args.allow_dirty:
        parser.error("shogiesa working tree must be clean; use --allow-dirty only for candidate runs")

    temporary = None
    if args.keep_work_dir:
        work_dir = args.keep_work_dir.resolve()
        work_dir.mkdir(parents=True, exist_ok=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="shogiesa-teacher-calibration-")
        work_dir = Path(temporary.name)

    teacher_preflight = [
        {
            "name": name,
            **probe_teacher(path, args.engine_option, args.timeout_ms, args.require_weight_ack),
        }
        for name, path in teachers
    ]
    steps: list[dict[str, object]] = []
    steps.append(
        run_step(
            "validate-input",
            [str(shogiesa), "validate", "--strict", "--input", str(source)],
            work_dir,
        )
    )
    existing_observations = count_existing_observations(source)
    if existing_observations:
        raise RuntimeError(
            "input must contain only unlabeled positions; "
            f"found {existing_observations} existing observations"
        )
    sampled = work_dir / "sampled.jsonl"
    steps.append(
        run_step(
            "sample",
            [
                str(shogiesa),
                "sample",
                "--input",
                str(source),
                "--count",
                str(args.positions),
                "--seed",
                str(args.seed),
                "--out",
                str(sampled),
            ],
            work_dir,
        )
    )
    sampled_count = jsonl_count(sampled)
    if sampled_count == 0:
        raise RuntimeError("sampling yielded zero positions")
    steps.append(
        run_step(
            "validate-sample",
            [str(shogiesa), "validate", "--strict", "--input", str(sampled)],
            work_dir,
        )
    )
    limits = [("depth", value) for value in args.depths]
    if args.nodes:
        limits.extend(("nodes", value) for value in args.nodes)
    case_records: list[dict[str, object]] = []
    case_paths: dict[tuple[str, str, int, int], Path] = {}
    case_maps: dict[tuple[str, str, int, int], dict[str, dict[str, object]]] = {}
    for teacher_name, engine in teachers:
        for limit_kind, limit_value in limits:
            for multipv in args.multipv:
                case_name = f"{teacher_name}__{limit_kind}{limit_value}__mpv{multipv}"
                stem = case_name.replace(".", "-")
                output = work_dir / f"{stem}.jsonl"
                manifest = work_dir / f"{stem}-manifest.json"
                command = [
                    str(shogiesa),
                    "label",
                    "--input",
                    str(sampled),
                    "--engine",
                    str(engine),
                    "--engine-name",
                    case_name,
                    "--timeout-ms",
                    str(args.timeout_ms),
                    "--jobs",
                    "1",
                    "--multipv",
                    str(multipv),
                    "--usi-strict",
                    "--preserve-order",
                    "--manifest",
                    str(manifest),
                    "--out",
                    str(output),
                ]
                command.extend(
                    ["--depths", str(limit_value)]
                    if limit_kind == "depth"
                    else ["--nodes", str(limit_value)]
                )
                for option in args.engine_option:
                    command.extend(["--engine-option", option])
                if teacher_name in weights:
                    command.extend(["--weight-file", str(weights[teacher_name])])
                steps.append(run_step(f"label-{stem}", command, work_dir))
                total, mapping = load_case(output, case_name)
                if total != sampled_count:
                    raise RuntimeError(f"case {case_name} changed record count")
                metrics = summarize_case(total, mapping)
                if metrics["labeled_records"] == 0:
                    raise RuntimeError(
                        f"case {case_name} produced zero observations; refusing an empty calibration"
                    )
                key = (teacher_name, limit_kind, limit_value, multipv)
                case_paths[key] = output
                case_maps[key] = mapping
                case_records.append(
                    {
                        "name": case_name,
                        "teacher": teacher_name,
                        "limit_kind": limit_kind,
                        "limit_value": limit_value,
                        "multipv": multipv,
                        "output_sha256": sha256_file(output),
                        "manifest_sha256": sha256_file(manifest),
                        "metrics": metrics,
                    }
                )

    cohorts: list[dict[str, object]] = []
    for limit_kind, limit_value in limits:
        for multipv in args.multipv:
            for teacher_count in range(1, len(teachers) + 1):
                selected_names = teacher_names[:teacher_count]
                keys = [
                    (name, limit_kind, limit_value, multipv) for name in selected_names
                ]
                stem = f"cohort-{limit_kind}{limit_value}-mpv{multipv}-teachers{teacher_count}"
                merged = work_dir / f"{stem}.jsonl"
                merge_cases(
                    shogiesa,
                    [case_paths[key] for key in keys],
                    merged,
                    stem,
                    work_dir,
                    steps,
                )
                stable = work_dir / f"{stem}-stable.jsonl"
                steps.append(
                    run_step(
                        f"{stem}-stability",
                        [
                            str(shogiesa),
                            "stability",
                            "--input",
                            str(merged),
                            "--out",
                            str(stable),
                        ],
                        work_dir,
                    )
                )
                profiles: dict[str, object] = {}
                for profile_name, profile_args in QUALITY_PROFILES.items():
                    manifest = work_dir / f"{stem}-{profile_name}-manifest.json"
                    steps.append(
                        run_step(
                            f"{stem}-filter-{profile_name}",
                            [
                                str(shogiesa),
                                "filter",
                                "--input",
                                str(stable),
                                "--dry-run",
                                "--manifest",
                                str(manifest),
                                *profile_args,
                            ],
                            work_dir,
                        )
                    )
                    profiles[profile_name] = manifest_summary(manifest)
                cohorts.append(
                    {
                        "limit_kind": limit_kind,
                        "limit_value": limit_value,
                        "multipv": multipv,
                        "teachers": selected_names,
                        "metrics": summarize_ensemble(
                            sampled_count, [case_maps[key] for key in keys]
                        ),
                        "quality_profiles": profiles,
                        "stable_sha256": sha256_file(stable),
                    }
                )

    commit = subprocess.run(
        ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.strip()
    artifact = {
        "schema": "shogiesa.teacher-calibration.v1",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": "candidate" if dirty_paths else "complete",
        "claim_boundary": (
            "parameter-matrix diagnostics only; no training-effect or playing-strength claim"
        ),
        "shogiesa": {
            "commit": commit,
            "dirty": bool(dirty_paths),
            "dirty_paths": dirty_paths,
            "binary_sha256": sha256_file(shogiesa),
        },
        "input": {
            "path": str(source),
            "sha256": sha256_file(source),
            "sample_seed": args.seed,
            "requested_sample_records": args.positions,
            "sampled_records": sampled_count,
            "sampled_sha256": sha256_file(sampled),
        },
        "teachers": [
            {
                "name": name,
                "binary_sha256": sha256_file(path),
                "weight_sha256": sha256_file(weights[name]) if name in weights else None,
            }
            for name, path in teachers
        ],
        "teacher_preflight": teacher_preflight,
        "environment": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "processor": platform.processor(),
            "cpu_count": os.cpu_count(),
        },
        "matrix": {
            "depths": args.depths,
            "nodes": args.nodes or [],
            "multipv": args.multipv,
            "teacher_prefix_order": teacher_names,
            "engine_options": args.engine_option,
            "timeout_ms": args.timeout_ms,
            "quality_profiles": QUALITY_PROFILES,
        },
        "cases": case_records,
        "cohorts": cohorts,
        "steps": steps,
    }
    write_json_atomic(args.out, artifact)
    print(
        json.dumps(
            {
                "status": artifact["status"],
                "cases": len(case_records),
                "cohorts": len(cohorts),
                "sampled_records": sampled_count,
            },
            indent=2,
        )
    )
    print(f"artifact: {args.out}")
    if temporary is not None:
        temporary.cleanup()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (json.JSONDecodeError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1) from None
