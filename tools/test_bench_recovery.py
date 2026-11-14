import argparse
import hashlib
import json
from pathlib import Path
import unittest

import bench_recovery


class RecoveryBenchmarkTests(unittest.TestCase):
    def test_counts_require_sorted_distinct_positive_values(self):
        self.assertEqual(bench_recovery.parse_counts("1,10,100"), [1, 10, 100])
        for value in ("", "0,1", "2,1", "1,1", "one"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                bench_recovery.parse_counts(value)

    def test_summary_uses_nearest_rank_and_includes_p95(self):
        summary = bench_recovery.summarize(list(range(1, 21)))
        self.assertEqual(summary["p50_ns"], 10)
        self.assertEqual(summary["p90_ns"], 18)
        self.assertEqual(summary["p95_ns"], 19)
        self.assertEqual(summary["p99_ns"], 20)

    def test_summary_rejects_empty_or_negative_values(self):
        for values in ([], [1, -1]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                bench_recovery.summarize(values)

    def test_markdown_renders_unavailable_rss(self):
        report = {
            "measurement": {"iterations": 1, "warmups": 0, "performance_scale": 64},
            "cases": [{
                "profile": "mixed-small", "books": 1, "corruption": None,
                "artifact_bytes": {"total": 1024},
                "summary": {"p50_ns": 1, "p95_ns": 2, "p99_ns": 3},
                "maximum_recorded_rss_bytes": None,
                "validation": {"ready": 1, "indexing": 0},
            }],
        }
        rendered = bench_recovery.render_markdown(report, Path("report.json"))
        self.assertIn("| n/a | ready=1, indexing=0 |", rendered)

    def test_archived_report_matches_measured_sources(self):
        root = Path(__file__).resolve().parents[1]
        report = json.loads((root / "docs/benchmarks/2026-11-14-linux-recovery.json").read_text())
        sources = {
            "runner_sha256": root / "tools/bench_recovery.py",
            "worker_sha256": root / "crates/perch-jobs/examples/bench_recovery.rs",
            "store_sha256": root / "crates/perch-jobs/src/store.rs",
            "cargo_lock_sha256": root / "Cargo.lock",
        }
        for field, path in sources.items():
            with self.subTest(field=field):
                actual = hashlib.sha256(path.read_bytes()).hexdigest()
                self.assertEqual(report["artifacts"][field], actual)


if __name__ == "__main__":
    unittest.main()
