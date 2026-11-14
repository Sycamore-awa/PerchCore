#!/usr/bin/env python3
"""Benchmark STORE-01 fresh-process artifact recovery at library scale."""

from __future__ import annotations

import argparse
import datetime
import json
import math
import os
import platform
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import uuid

from bench import filesystem, read_optional, sha256

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()
NAMESPACE = uuid.UUID("625e638c-1310-4b6a-bd54-839a5889df0a")


def parse_counts(value):
    try:
        counts = [int(item) for item in value.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError("counts must be comma-separated integers") from error
    if not counts or any(item < 1 for item in counts) or counts != sorted(set(counts)):
        raise argparse.ArgumentTypeError("counts must be distinct positive integers in ascending order")
    return counts


def summarize(values):
    if not values or any(value < 0 for value in values):
        raise ValueError("nonempty nonnegative values required")
    ordered = sorted(values)

    def percentile(fraction):
        return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]

    return {
        "count": len(ordered),
        "percentile_method": "nearest_rank",
        "p50_ns": percentile(0.50),
        "p90_ns": percentile(0.90),
        "p95_ns": percentile(0.95),
        "p99_ns": percentile(0.99),
        "min_ns": ordered[0],
        "max_ns": ordered[-1],
        "mean_ns": statistics.mean(ordered),
    }


def command_output(command):
    return subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def build(binary):
    started = time.perf_counter_ns()
    result = subprocess.run(
        ["cargo", "build", "--release", "--locked", "-p", "perch-jobs", "--example", "bench_recovery"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    )
    return {"command": "cargo build --release --locked -p perch-jobs --example bench_recovery",
            "elapsed_ns": time.perf_counter_ns() - started,
            "stdout": result.stdout, "stderr": result.stderr, "binary": str(binary)}


def ensure_performance_corpus(path, scale):
    subprocess.run(
        [sys.executable, "tools/make_corpus.py", "--profile", "performance", "--scale", str(scale), "--output", str(path)],
        cwd=ROOT, check=True,
    )


def manifest_sample(corpus, sample_id):
    manifest = json.loads((corpus / "manifest.json").read_text(encoding="utf-8"))
    sample = next((item for item in manifest["samples"] if item["id"] == sample_id), None)
    if sample is None or not sample.get("indexed"):
        raise ValueError(f"indexed sample {sample_id!r} missing from {corpus}")
    return manifest, sample


def prepare_template(binary, corpus, sample_id, root, label):
    manifest, sample = manifest_sample(corpus, sample_id)
    output = root / f"template-{label}"
    book = uuid.uuid5(NAMESPACE, f"template/{label}")
    child = subprocess.run(
        [str(binary), "prepare", str((corpus / sample["path"]).resolve()), sample["format"], str(output), str(book)],
        cwd=ROOT, capture_output=True, text=True, check=True, timeout=300,
    )
    stats = json.loads(child.stdout)
    return output / str(book), {
        "label": label, "sample": sample_id, "format": sample["format"],
        "corpus_profile": manifest["profile"], "corpus_scale": manifest["scale"],
        "source_bytes": (corpus / sample["path"]).stat().st_size,
        "source_sha256": sha256(corpus / sample["path"]), **stats,
    }


def copy_template(source, destination):
    shutil.copytree(source, destination)


def make_library(root, profile, count, templates):
    library = root / f"library-{profile}-{count}"
    library.mkdir()
    for index in range(count):
        book = uuid.uuid5(NAMESPACE, f"{profile}/{count}/{index}")
        copy_template(templates[index % len(templates)], library / str(book))
    sizes = {name: 0 for name in ("state.json", "refinement.json", "index.bin", "text.json")}
    for directory in library.iterdir():
        for name in sizes:
            sizes[name] += (directory / name).stat().st_size
    return library, sizes


def run_worker(binary, library, recovered, ready, indexing, timeout):
    started = time.perf_counter_ns()
    child = subprocess.run(
        [str(binary), "measure", str(library), str(recovered), str(ready), str(indexing)],
        cwd=ROOT, capture_output=True, text=True, check=True, timeout=timeout,
    )
    value = json.loads(child.stdout)
    value["process_elapsed_ns"] = time.perf_counter_ns() - started
    return value


def measure_case(binary, library, profile, count, corruption, iterations, warmups, timeout, sizes):
    expected_indexing = 1 if corruption else 0
    expected_ready = count - expected_indexing
    warmup_results = [run_worker(binary, library, count, expected_ready, expected_indexing, timeout)
                      for _ in range(warmups)]
    runs = [run_worker(binary, library, count, expected_ready, expected_indexing, timeout)
            for _ in range(iterations)]
    durations = [item["duration_ns"] for item in runs]
    rss = [item["rss_bytes"] for item in runs if item["rss_bytes"] is not None]
    peaks = [item["peak_rss_bytes"] for item in runs if item["peak_rss_bytes"] is not None]
    return {
        "id": f"{profile}/{count}/" + ("one-corrupt-index" if corruption else "healthy"),
        "profile": profile, "books": count,
        "corruption": "one index.bin overwritten with invalid bytes" if corruption else None,
        "expected": {"recovered": count, "ready": expected_ready, "indexing": expected_indexing},
        "artifact_bytes": {**sizes, "total": sum(sizes.values())},
        "durations_ns": durations, "summary": summarize(durations),
        "rss_bytes": rss, "peak_rss_bytes": peaks,
        "maximum_recorded_rss_bytes": max([*rss, *peaks], default=None),
        "process_elapsed_ns": [item["process_elapsed_ns"] for item in runs],
        "warmups": warmup_results,
        "validation": {key: runs[-1][key] for key in
                       ("recovered", "ready", "indexing", "registered", "failed", "corrupt_index_reasons")},
    }


def render_markdown(report, output):
    lines = [
        "# STORE-01 恢复冷启动基准", "",
        f"原始记录：[{output.name}]({output.name})。", "",
        f"每项使用全新 release Rust 进程，只计时 `PathStore::new`；"
        f"{report['measurement']['iterations']} 次测量、{report['measurement']['warmups']} 次预热。",
        "书库由真实 TXT/EPUB 管线产物复制得到，生成、复制、进程启动和结果校验不在恢复计时内。",
        "系统页缓存未驱逐，因此“冷启动”只表示全新进程的首次恢复，不表示冷块设备 I/O。",
        "RSS 是 worker 整个进程的 VmRSS/VmHWM 最大记录值，不是 SDK 独占分配。", "",
        "| 书库 | 本数 | 状态 | 产物 MiB | p50 ms | p95 ms | p99 ms | ms/本(p50) | 最大 RSS MiB | 恢复结果 |",
        "|---|---:|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for case in report["cases"]:
        summary = case["summary"]
        state = "1 本索引损坏" if case["corruption"] else "健康"
        rss = case["maximum_recorded_rss_bytes"]
        rss_mib = f"{rss / 1048576:.3f}" if rss is not None else "n/a"
        result = case["validation"]
        lines.append(
            f"| {case['profile']} | {case['books']} | {state} | {case['artifact_bytes']['total'] / 1048576:.3f} | "
            f"{summary['p50_ns'] / 1e6:.3f} | {summary['p95_ns'] / 1e6:.3f} | {summary['p99_ns'] / 1e6:.3f} | "
            f"{summary['p50_ns'] / 1e6 / case['books']:.4f} | "
            f"{rss_mib} | "
            f"ready={result['ready']}, indexing={result['indexing']} |"
        )
    lines += ["", "## 口径", "",
              "- `mixed-small`：正确性集的 UTF-8 TXT 与 EPUB3 Ready 产物交替组成。",
              f"- `mixed-scale-{report['measurement']['performance_scale']}`：性能集的 long-line TXT 与 EPUB3 Ready 产物交替组成。",
              "- 损坏案例仅覆盖一本书的 `index.bin`；该书稳定降级到 indexing，其余书保持 Ready。",
              "- 报告不设自动性能门槛；宿主 Linux 数据用于判断是否值得引入 manifest/延迟审计，不代表手机预算。"]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--counts", type=parse_counts, default=parse_counts("1,10,100,1000"))
    parser.add_argument("--large-counts", type=parse_counts, default=parse_counts("1,10,100"))
    parser.add_argument("--iterations", type=int, default=20)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--performance-scale", type=int, default=64)
    parser.add_argument("--timeout-seconds", type=int, default=300)
    parser.add_argument("--output", type=Path, default=ROOT / "target/store01/recovery.json")
    parser.add_argument("--scratch-root", type=Path, default=ROOT / "target/store01")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/bench_recovery")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    if args.iterations < 1 or args.warmups < 0 or args.performance_scale < 1 or args.timeout_seconds < 1:
        parser.error("iterations, performance scale and timeout must be positive; warmups must be nonnegative")

    binary = args.binary.resolve()
    builds = [] if args.skip_build else [build(binary)]
    if not binary.is_file():
        parser.error(f"benchmark binary not found: {binary}")
    args.scratch_root.mkdir(parents=True, exist_ok=True)
    performance_corpus = args.scratch_root.resolve() / f"corpus-{args.performance_scale}"
    ensure_performance_corpus(performance_corpus, args.performance_scale)

    cpu = read_optional("/proc/cpuinfo")
    cpu_model = next((line.split(":", 1)[1].strip() for line in (cpu or "").splitlines()
                      if line.startswith("model name")), None)
    report = {
        "version": 1, "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "environment": {"os": platform.platform(), "machine": platform.machine(), "cpu": cpu_model,
                        "logical_cpus": os.cpu_count(), "python": platform.python_version(),
                        "rust": command_output(["rustc", "--version"]),
                        "git_commit": command_output(["git", "rev-parse", "HEAD"]),
                        "dirty_workspace": bool(command_output(["git", "status", "--porcelain"])),
                        "memory": read_optional("/proc/meminfo")},
        "measurement": {"iterations": args.iterations, "warmups": args.warmups,
                        "counts": args.counts, "large_counts": args.large_counts,
                        "performance_scale": args.performance_scale,
                        "timer": "Rust std::time::Instant around PathStore::new only",
                        "process": "fresh release Rust exec for every warmup and measured run",
                        "cache": "OS page cache uncontrolled; this is fresh-process recovery, not cold block-device I/O",
                        "rss": "Linux /proc/self/status VmRSS and VmHWM after recovery; whole worker process"},
        "artifacts": {"binary": str(binary), "binary_sha256": sha256(binary),
                      "runner_sha256": sha256(SCRIPT),
                      "worker_sha256": sha256(ROOT / "crates/perch-jobs/examples/bench_recovery.rs"),
                      "store_sha256": sha256(ROOT / "crates/perch-jobs/src/store.rs"),
                      "cargo_lock_sha256": sha256(ROOT / "Cargo.lock")},
        "filesystems": {"scratch": filesystem(args.scratch_root.resolve()),
                        "correctness_corpus": filesystem(ROOT / "examples/data/corpus")},
        "builds": builds, "templates": [], "cases": [],
    }

    with tempfile.TemporaryDirectory(prefix="recovery-", dir=args.scratch_root) as temporary:
        temporary = Path(temporary)
        small_txt, small_txt_stats = prepare_template(binary, ROOT / "examples/data/corpus", "utf8", temporary, "small-txt")
        small_epub, small_epub_stats = prepare_template(binary, ROOT / "examples/data/corpus", "epub3", temporary, "small-epub")
        large_txt, large_txt_stats = prepare_template(binary, performance_corpus, "long-line", temporary, "large-txt")
        large_epub, large_epub_stats = prepare_template(binary, performance_corpus, "epub3", temporary, "large-epub")
        report["templates"] = [small_txt_stats, small_epub_stats, large_txt_stats, large_epub_stats]
        profiles = [
            ("mixed-small", args.counts, [small_txt, small_epub]),
            (f"mixed-scale-{args.performance_scale}", args.large_counts, [large_txt, large_epub]),
        ]
        for profile, counts, templates in profiles:
            for count in counts:
                print(f"Preparing {profile} with {count} books", file=sys.stderr, flush=True)
                library, sizes = make_library(temporary, profile, count, templates)
                print(f"Measuring {profile} with {count} books", file=sys.stderr, flush=True)
                report["cases"].append(measure_case(
                    binary, library, profile, count, False, args.iterations, args.warmups,
                    args.timeout_seconds, sizes))
                if profile == "mixed-small" and count == max(args.counts):
                    corrupt = sorted(library.iterdir())[0] / "index.bin"
                    old_size = corrupt.stat().st_size
                    corrupt.write_bytes(b"broken recovery benchmark index")
                    sizes["index.bin"] += corrupt.stat().st_size - old_size
                    print(f"Measuring {profile} with one corrupt index", file=sys.stderr, flush=True)
                    report["cases"].append(measure_case(
                        binary, library, profile, count, True, args.iterations, args.warmups,
                        args.timeout_seconds, sizes))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.output.with_suffix(".md").write_text(render_markdown(report, args.output), encoding="utf-8")
    print(f"Recorded {len(report['cases'])} cases to {args.output}")


if __name__ == "__main__":
    main()
