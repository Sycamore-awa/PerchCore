"""Measurement statistics and failure behavior, independent of benchmark speed."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from bench import ROOT, peak_rss, summarize


class BenchmarkTests(unittest.TestCase):
    def test_nearest_rank_percentiles_and_throughput(self):
        summary = summarize(list(range(30, 0, -1)), work_bytes=1024 ** 2, operations=2)
        self.assertEqual(summary["p50_ns"], 15)
        self.assertEqual(summary["p90_ns"], 27)
        self.assertEqual(summary["p99_ns"], 30)
        self.assertEqual(summary["operations_per_second_at_p50"], 2e9 / 15)
        self.assertEqual(summary["mib_per_second_at_p50"], 1e9 / 15)

    def test_one_sample_and_zero_duration_do_not_invent_values(self):
        summary = summarize([0])
        self.assertEqual(summary["p50_ns"], 0)
        self.assertEqual(summary["p99_ns"], 0)
        self.assertIsNone(summary["operations_per_second_at_p50"])
        self.assertIsNone(summary["mib_per_second_at_p50"])
        self.assertIsNone(summary["work_bytes_per_iteration"])

    def test_invalid_durations_rejected(self):
        for durations in [[], [-1], [10, -1]]:
            with self.assertRaises(ValueError):
                summarize(durations)

    def test_linux_memory_uses_post_exec_high_water_mark(self):
        with patch("bench.sys.platform", "linux"), patch("bench.read_optional", return_value="Name: test\nVmHWM:\t1234 kB\nVmRSS:\t1 kB\n"):
            measured = peak_rss()
        self.assertEqual(measured["bytes"], 1234 * 1024)
        self.assertEqual(measured["method"], "/proc/self/status VmHWM")

    def test_invalid_options_do_not_build_or_write_report(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "report.json"
            for arguments in [["--iterations", "0"], ["--clients", "rust,rust"], ["--samples", "missing"], ["--stages", "missing"]]:
                result = subprocess.run([sys.executable, str(ROOT / "tools/bench.py"), "--output", str(output), *arguments],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertNotIn("Building", result.stderr)
                self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
