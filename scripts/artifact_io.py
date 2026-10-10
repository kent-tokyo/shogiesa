#!/usr/bin/env python3
"""Small, dependency-free helpers for safe measurement artifact output."""

from __future__ import annotations

import json
import os
import shutil
import tempfile
from pathlib import Path
from typing import Any, Iterable


PathRole = tuple[str, Path]


def _path_key(path: Path) -> Path:
    path = path.expanduser()
    if path.is_symlink():
        return path.resolve(strict=True)
    if path.exists():
        return path.resolve(strict=True)
    return path.parent.resolve(strict=False) / path.name


def paths_collide(left: Path, right: Path) -> bool:
    """Compare existing file identity, falling back to normalized future paths."""

    if (left.exists() or left.is_symlink()) and (right.exists() or right.is_symlink()):
        try:
            return os.path.samefile(left, right)
        except OSError as error:
            raise ValueError(f"cannot compare path identity for {left} and {right}: {error}") from error
    return _path_key(left) == _path_key(right)


def validate_path_roles(read_paths: Iterable[PathRole], write_paths: Iterable[PathRole]) -> None:
    """Reject read/write aliases and pairwise-colliding output paths."""

    reads = list(read_paths)
    writes = list(write_paths)
    for read_name, read_path in reads:
        for write_name, write_path in writes:
            if paths_collide(read_path, write_path):
                raise ValueError(f"{read_name} must not be the same path as {write_name}")
    for index, (left_name, left_path) in enumerate(writes):
        for right_name, right_path in writes[index + 1 :]:
            if paths_collide(left_path, right_path):
                raise ValueError(f"{left_name} must not be the same path as {right_name}")


def _output_destination(path: Path) -> Path:
    path = path.expanduser()
    if path.is_symlink():
        return path.resolve(strict=True)
    return path


def _atomic_replace(path: Path, write_temp: Any) -> None:
    destination = _output_destination(path)
    if destination.is_dir():
        raise IsADirectoryError(f"artifact destination is a directory: {path}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temp_name = tempfile.mkstemp(
        prefix=f".{destination.name}.", dir=destination.parent
    )
    temp_path = Path(temp_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            write_temp(output)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp_path, destination)
    except BaseException:
        temp_path.unlink(missing_ok=True)
        raise


def atomic_write_text(path: Path, text: str) -> None:
    encoded = text.encode("utf-8")
    _atomic_replace(path, lambda output: output.write(encoded))


def atomic_write_json(path: Path, value: Any) -> None:
    atomic_write_text(path, json.dumps(value, indent=2) + "\n")


def atomic_copy(source: Path, destination: Path) -> None:
    def copy(output: Any) -> None:
        with source.open("rb") as input_file:
            shutil.copyfileobj(input_file, output, length=1024 * 1024)

    _atomic_replace(destination, copy)
