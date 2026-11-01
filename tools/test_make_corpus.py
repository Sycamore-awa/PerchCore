"""Check the fixture CLI's failure and reproducibility behavior with stdlib unittest."""

import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

from make_sample_epub import build

SCRIPT = Path(__file__).with_name("make_corpus.py")


class CorpusCliTests(unittest.TestCase):
    def run_cli(self, destination, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--output", str(destination), *args],
            capture_output=True, text=True, check=False,
        )

    def test_check_detects_corruption_without_repair(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(self.run_cli(root).returncode, 0)
            source = root / "inputs/gbk.txt"
            source.write_bytes(b"corrupted")
            result = self.run_cli(root, "--check")
            self.assertEqual(result.returncode, 1)
            self.assertIn("content mismatch: inputs/gbk.txt", result.stderr)
            self.assertEqual(source.read_bytes(), b"corrupted")

    def test_check_detects_missing_and_unexpected_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertEqual(self.run_cli(root).returncode, 0)
            (root / "texts/utf8.decoded.txt").unlink()
            (root / "inputs/stale.txt").write_bytes(b"stale")
            result = self.run_cli(root, "--check")
            self.assertEqual(result.returncode, 1)
            self.assertIn("missing: texts/utf8.decoded.txt", result.stderr)
            self.assertIn("unexpected: inputs/stale.txt", result.stderr)

    def test_invalid_scale_does_not_write(self):
        with tempfile.TemporaryDirectory() as directory:
            destination = Path(directory) / "corpus"
            for args in [("--scale", "0"), ("--scale", "2"), ("--profile", "performance", "--scale", "-1")]:
                self.assertEqual(self.run_cli(destination, *args).returncode, 2)
                self.assertFalse(destination.exists())

    def test_epub_example_has_fixed_zip_metadata_and_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory) / "a.epub", Path(directory) / "b.epub"
            build(first)
            build(second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                self.assertEqual(archive.namelist()[0], "mimetype")
                self.assertEqual(archive.read("mimetype"), b"application/epub+zip")
                for entry in archive.infolist():
                    self.assertEqual(entry.date_time, (2026, 10, 16, 0, 0, 0))
                    self.assertEqual(entry.compress_type, zipfile.ZIP_STORED)
                    self.assertEqual(entry.create_system, 3)
                    self.assertEqual(entry.external_attr >> 16, 0o100644)


if __name__ == "__main__":
    unittest.main()
