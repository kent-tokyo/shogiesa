#!/usr/bin/env python3
"""Measure local shogiesa streaming commands on a deterministic synthetic JSONL corpus."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from artifact_io import atomic_write_json, validate_path_roles


ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def git_output(*args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(ROOT), *args],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip()


def make_dataset(path: Path, records: int) -> None:
    templates = [
        json.loads(line)
        for line in FIXTURE.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    if not templates:
        raise RuntimeError(f"fixture has no records: {FIXTURE}")

    with path.open("w", encoding="utf-8", newline="\n") as output:
        for index in range(records):
            record = copy.deepcopy(templates[index % len(templates)])
            sfen = record["sfen"].split()
            sfen[-1] = str(index + 1)
            record["sfen"] = " ".join(sfen)
            source = record["source"]
            source["path"] = f"synthetic/game-{index // 200:06}.csa"
            source["ply"] = index % 200 + 1
            source["root_id"] = source["path"]
            source["variation_id"] = None
            source["branch_from_ply"] = None
            output.write(
                json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n"
            )


def rss_bytes(pid: int) -> int | None:
    result = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(pid)],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        check=False,
    )
    value = result.stdout.strip()
    return int(value) * 1024 if value.isdigit() else None


def fd_count(pid: int) -> tuple[int | None, str]:
    proc_fd = Path(f"/proc/{pid}/fd")
    if proc_fd.is_dir():
        try:
            return len(list(proc_fd.iterdir())), "procfs"
        except OSError:
            pass
    if shutil.which("lsof"):
        result = subprocess.run(
            ["lsof", "-a", "-p", str(pid), "-F", "f"],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            check=False,
        )
        if result.returncode == 0:
            return (
                sum(
                    line.startswith("f") and line[1:].isdigit()
                    for line in result.stdout.splitlines()
                ),
                "lsof",
            )
    return None, "unavailable"


def run_measured(name: str, command: list[str], work_dir: Path) -> dict[str, object]:
    stdout_path = work_dir / f"{name}.stdout"
    stderr_path = work_dir / f"{name}.stderr"
    peak_rss = 0
    max_fds = 0
    samples = 0
    fd_method = "unavailable"
    started = time.monotonic()
    with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
        process = subprocess.Popen(command, cwd=ROOT, stdout=stdout, stderr=stderr)
        while process.poll() is None:
            rss = rss_bytes(process.pid)
            fds, method = fd_count(process.pid)
            if rss is not None:
                peak_rss = max(peak_rss, rss)
            if fds is not None:
                max_fds = max(max_fds, fds)
                fd_method = method
            samples += 1
            time.sleep(0.01)
        return_code = process.returncode
    wall_time = time.monotonic() - started
    result: dict[str, object] = {
        "name": name,
        "command": command,
        "exit_code": return_code,
        "wall_time_seconds": round(wall_time, 6),
        "peak_rss_bytes": peak_rss or None,
        "max_open_fds": max_fds or None,
        "fd_measurement": fd_method,
        "samples": samples,
        "stdout_bytes": stdout_path.stat().st_size,
        "stderr_bytes": stderr_path.stat().st_size,
    }
    if return_code != 0:
        result["stderr_tail"] = stderr_path.read_text(errors="replace")[-4000:]
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--records", type=int, default=100_000)
    parser.add_argument("--shogiesa", type=Path, default=ROOT / "target/release/shogiesa")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--keep-work-dir", type=Path)
    args = parser.parse_args()
    if args.records <= 0:
        parser.error("--records must be greater than zero")
    binary = args.shogiesa.resolve()
    if not binary.is_file():
        parser.error(f"shogiesa binary not found: {binary}; build it before measuring")
    try:
        validate_path_roles(
            [("fixture", FIXTURE), ("--shogiesa", binary)],
            [("--out", args.out)],
        )
    except ValueError as error:
        parser.error(str(error))

    temporary = None
    if args.keep_work_dir:
        work_dir = args.keep_work_dir.resolve()
        work_dir.mkdir(parents=True, exist_ok=True)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="shogiesa-resource-")
        work_dir = Path(temporary.name)

    input_path = work_dir / "input.jsonl"
    pack_path = work_dir / "dataset.shgpk"
    unpacked_path = work_dir / "unpacked.jsonl"
    disk_before = shutil.disk_usage(work_dir).free
    generation_started = time.monotonic()
    make_dataset(input_path, args.records)
    generation_seconds = time.monotonic() - generation_started
    input_bytes = input_path.stat().st_size
    input_sha256 = sha256_file(input_path)

    commands_before_unpack = [
        ("validate", [str(binary), "validate", "--strict", "--input", str(input_path)]),
        ("report", [str(binary), "report", "--input", str(input_path)]),
        (
            "pack",
            [str(binary), "pack", "--input", str(input_path), "--out", str(pack_path)],
        ),
    ]
    measurements = [
        run_measured(name, command, work_dir) for name, command in commands_before_unpack
    ]
    input_removed_before_unpack = False
    if all(item["exit_code"] == 0 for item in measurements) and pack_path.exists():
        # At 1M records the JSONL, pack, and unpacked JSONL together need more than 2 GiB.
        # validate/report/pack have already consumed and hashed the input, so remove that
        # regenerable synthetic file before measuring unpack. This keeps the peak footprint near
        # max(input + pack, pack + unpack) without changing any measured command.
        input_path.unlink()
        input_removed_before_unpack = True
    measurements.append(
        run_measured(
            "unpack",
            [
                str(binary),
                "unpack",
                "--input",
                str(pack_path),
                "--out",
                str(unpacked_path),
            ],
            work_dir,
        )
    )
    disk_after = shutil.disk_usage(work_dir).free
    status = "pass" if all(item["exit_code"] == 0 for item in measurements) else "fail"
    unpacked_records = (
        sum(1 for line in unpacked_path.open(encoding="utf-8") if line.strip())
        if unpacked_path.exists()
        else None
    )
    status_porcelain = subprocess.run(
        ["git", "-C", str(ROOT), "status", "--porcelain"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    ).stdout.rstrip("\n")
    artifact = {
        "schema": "shogiesa.resource-baseline.v1",
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "status": status,
        "repository": {
            "commit": git_output("rev-parse", "HEAD"),
            "working_tree_dirty": bool(status_porcelain),
            "changed_paths": [line[3:] for line in status_porcelain.splitlines()],
        },
        "binary": {
            "path": str(binary),
            "sha256": sha256_file(binary),
            "version": subprocess.run(
                [str(binary), "--version"],
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip(),
        },
        "environment": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "logical_cpus": os.cpu_count(),
            "sampling_interval_seconds": 0.01,
            "disk_free_before_bytes": disk_before,
            "disk_free_after_bytes": disk_after,
            "disk_consumed_bytes": max(0, disk_before - disk_after),
        },
        "dataset": {
            "records": args.records,
            "generation_seconds": round(generation_seconds, 6),
            "input_bytes": input_bytes,
            "input_sha256": input_sha256,
            "input_removed_before_unpack": input_removed_before_unpack,
            "fixture_sha256": sha256_file(FIXTURE),
            "pack_bytes": pack_path.stat().st_size if pack_path.exists() else None,
            "pack_sha256": sha256_file(pack_path) if pack_path.exists() else None,
            "unpacked_bytes": unpacked_path.stat().st_size if unpacked_path.exists() else None,
            "unpacked_sha256": sha256_file(unpacked_path) if unpacked_path.exists() else None,
            "unpacked_records": unpacked_records,
        },
        "measurements": measurements,
        "limits": [
            "Synthetic records vary source provenance and SFEN move count but do not model a real corpus distribution.",
            "RSS and FD values are sampled and can miss a short-lived peak.",
            "Generation time is recorded separately and excluded from command wall times.",
            "The regenerable synthetic input is removed after pack and before unpack to bound peak disk use.",
        ],
    }
    atomic_write_json(args.out, artifact)
    print(f"resource baseline: {status.upper()}")
    print(f"  records: {args.records}")
    print(f"  artifact: {args.out}")
    for item in measurements:
        print(
            f"  {item['name']}: wall={item['wall_time_seconds']}s "
            f"rss={item['peak_rss_bytes']} fd={item['max_open_fds']}"
        )
    if temporary is not None:
        temporary.cleanup()
    return 0 if status == "pass" and unpacked_records == args.records else 1


if __name__ == "__main__":
    raise SystemExit(main())
