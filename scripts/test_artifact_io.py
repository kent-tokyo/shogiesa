#!/usr/bin/env python3

from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from artifact_io import atomic_copy, atomic_write_text, paths_collide, validate_path_roles


class ArtifactIoTests(unittest.TestCase):
    def test_existing_hardlink_and_symlink_aliases_collide(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            directory = Path(raw_dir)
            original = directory / "original"
            hardlink = directory / "hardlink"
            symlink = directory / "symlink"
            original.write_text("data", encoding="utf-8")
            os.link(original, hardlink)

            self.assertTrue(paths_collide(original, hardlink))
            try:
                symlink.symlink_to(original)
            except OSError:
                return
            self.assertTrue(paths_collide(original, symlink))

    def test_validate_path_roles_rejects_read_write_and_write_write_aliases(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            directory = Path(raw_dir)
            source = directory / "source"
            source.write_text("data", encoding="utf-8")
            alias = directory / "alias"
            os.link(source, alias)

            with self.assertRaisesRegex(ValueError, "--input must not"):
                validate_path_roles([("--input", source)], [("--out", alias)])
            with self.assertRaisesRegex(ValueError, "--out must not"):
                validate_path_roles([], [("--out", source), ("--mined-out", alias)])

    def test_failed_replace_preserves_existing_artifact_and_cleans_temp(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            directory = Path(raw_dir)
            destination = directory / "artifact.json"
            destination.write_text("previous\n", encoding="utf-8")

            with mock.patch("artifact_io.os.replace", side_effect=OSError("injected")):
                with self.assertRaisesRegex(OSError, "injected"):
                    atomic_write_text(destination, "replacement\n")

            self.assertEqual(destination.read_text(encoding="utf-8"), "previous\n")
            self.assertEqual([path.name for path in directory.iterdir()], ["artifact.json"])

    def test_atomic_write_follows_existing_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            directory = Path(raw_dir)
            target = directory / "target.json"
            link = directory / "artifact.json"
            target.write_text("previous\n", encoding="utf-8")
            try:
                link.symlink_to(target)
            except OSError as error:
                self.skipTest(f"symlink unavailable: {error}")

            atomic_write_text(link, "replacement\n")

            self.assertTrue(link.is_symlink())
            self.assertEqual(target.read_text(encoding="utf-8"), "replacement\n")

    def test_atomic_copy_replaces_only_after_complete_copy(self) -> None:
        with tempfile.TemporaryDirectory() as raw_dir:
            directory = Path(raw_dir)
            source = directory / "source.jsonl"
            destination = directory / "destination.jsonl"
            source.write_bytes(b"complete dataset\n")
            destination.write_bytes(b"previous dataset\n")

            atomic_copy(source, destination)

            self.assertEqual(destination.read_bytes(), source.read_bytes())


if __name__ == "__main__":
    unittest.main()
