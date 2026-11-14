#!/usr/bin/env python3
"""LOC-01: artifact growth, C ABI opens/locates and isolated process RSS.

The load comparison uses the same index and body, with/without the optional map.
It does not reconstruct an older implementation of source extraction.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import platform
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid

from bench import CAbi, filesystem, peak_rss, read_optional, sha256, summarize
from make_corpus import build_corpus, check_corpus

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()
MODES = ("book.open.path", "book.load.legacy", "book.load.mapped", "book.locate")


def rss():
    status = read_optional("/proc/self/status") if sys.platform == "linux" else None
    return next((int(line.split()[1]) * 1024 for line in (status or "").splitlines()
                 if line.startswith("VmRSS:")), None)


def worker(config):
    core = CAbi(config["library"])
    mode = config["mode"]
    baseline_rss = rss()
    locate_handle = None
    locate_range = None
    if mode == "book.locate":
        locate_handle = core.invoke("book.load", {"index_path": config["index"], "text_path": config["mapped"]})[0]["handle"]
        hits = core.invoke("book.search", {"handle": locate_handle, "query": config["query"], "limit": 1000000})[0]["hits"]
        if not hits:
            raise ValueError("benchmark query must hit")
        locate_range = {"handle": locate_handle, "char_start": hits[-1]["charStart"], "char_end": hits[-1]["charEnd"]}
    setup_rss = rss()

    def operation():
        if mode == "book.open.path":
            return core.invoke("book.open", {"path": config["source"], "format": config["format"]})
        if mode == "book.locate":
            return core.invoke("book.locate", locate_range)
        return core.invoke("book.load", {"index_path": config["index"],
            "text_path": config["mapped"] if mode == "book.load.mapped" else config["legacy"]})

    durations, rss_samples = [], []
    request_bytes = response_bytes = None
    for iteration in range(config["warmups"] + config["iterations"]):
        started = time.perf_counter_ns()
        value, request_bytes, response_bytes = operation()
        elapsed = time.perf_counter_ns() - started
        rss_samples.append(rss())
        if mode == "book.locate":
            if value["start"]["kind"] != config["format"] or value["end"]["kind"] != config["format"]:
                raise ValueError("location format mismatch")
        else:
            if {key: value[key] for key in config["expected"]} != config["expected"]:
                raise ValueError("open statistics differ from prepared artifacts")
            core.invoke("book.close", {"handle": value["handle"]})
        if iteration >= config["warmups"]:
            durations.append(elapsed)
    if locate_handle is not None:
        core.invoke("book.close", {"handle": locate_handle})
    final_rss = rss()
    kernel_peak = peak_rss()
    recorded_rss = [value for value in [baseline_rss, setup_rss, *rss_samples,
                    final_rss, kernel_peak["bytes"]] if value is not None]
    return {"durations_ns": durations, "summary": summarize(durations),
            "request_bytes": request_bytes, "response_bytes": response_bytes,
            "rss_before_setup_bytes": baseline_rss, "rss_after_setup_bytes": setup_rss,
            "rss_after_first_operation_bytes": rss_samples[0],
            "rss_after_operations_bytes": rss_samples, "rss_after_cleanup_bytes": final_rss,
            "peak_rss": kernel_peak,
            "maximum_recorded_rss": {
                "bytes": max(recorded_rss) if recorded_rss else None,
                "method": "max(sampled /proc/self/status VmRSS, final VmHWM)"
                    if sys.platform == "linux" else kernel_peak.get("method"),
                "scope": "whole worker process: runtime, setup, warmups, operations, validation and cleanup",
                "caveat": "Linux RSS counters are approximate/asynchronous; samples do not guarantee all transient peaks",
            }}


def prepare(core, corpus, sample, directory):
    pipeline = core.invoke("pipeline.open", {"out_dir": str(directory)})[0]["handle"]
    book = str(uuid.uuid4())
    source = (corpus / sample["path"]).resolve()
    try:
        core.invoke("pipeline.enqueue", {"handle": pipeline, "book": book, "path": str(source), "format": sample["format"]})
        for expected in ("refined", "indexed"):
            if core.invoke("pipeline.step", {"handle": pipeline, "now_ms": 0})[0]["outcome"] != expected:
                raise ValueError("preparation did not complete")
    finally:
        core.invoke("pipeline.close", {"handle": pipeline})
    index, mapped = directory / book / "index.bin", directory / book / "text.json"
    mapped_bytes = mapped.read_bytes()
    body = json.loads(mapped_bytes)
    source_map = body.pop("sourceMap")
    legacy = directory / book / "legacy-text.json"
    legacy.write_bytes(json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode())
    opened = core.invoke("book.load", {"index_path": str(index), "text_path": str(mapped)})[0]
    core.invoke("book.close", {"handle": opened["handle"]})
    return {"source": str(source), "format": sample["format"], "query": sample["queries"][0]["query"],
            "index": str(index), "mapped": str(mapped), "legacy": str(legacy),
            "expected": {key: value for key, value in opened.items() if key != "handle"}}, {
        "source_bytes": source.stat().st_size, "source_sha256": sha256(source),
        "index_bytes": index.stat().st_size, "legacy_text_bytes": legacy.stat().st_size,
        "mapped_text_bytes": len(mapped_bytes), "added_bytes": len(mapped_bytes) - legacy.stat().st_size,
        "run_count": len(source_map["runs"]), "statistics": {key: value for key, value in opened.items() if key != "handle"},
    }


def command_output(command):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=True)
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, action="append")
    parser.add_argument("--samples", default="utf8,gbk,big5,epub2,epub3,long-line,cross-shard")
    parser.add_argument("--iterations", type=int, default=30)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--output", type=Path, default=ROOT / "target/loc01/source-map.json")
    extension = "dll" if sys.platform == "win32" else "dylib" if sys.platform == "darwin" else "so"
    library_name = "perch_ffi.dll" if sys.platform == "win32" else f"libperch_ffi.{extension}"
    parser.add_argument("--library", type=Path, default=ROOT / "target/release" / library_name)
    args = parser.parse_args()
    if args.iterations < 1 or args.warmups < 0:
        parser.error("iterations must be positive and warmups nonnegative")
    selected = args.samples.split(",")
    corpora = [path.resolve() for path in args.corpus or [ROOT / "examples/data/corpus"]]
    manifests = []
    for corpus in corpora:
        manifest = json.loads((corpus / "manifest.json").read_text(encoding="utf-8"))
        if manifest["version"] != 1 or manifest["generator_version"] != 1:
            parser.error("unsupported corpus version")
        problems = check_corpus(corpus, build_corpus(manifest["profile"], manifest["scale"]))
        if problems:
            parser.error("corpus verification failed: " + "; ".join(problems))
        available = {sample["id"] for sample in manifest["samples"] if sample.get("indexed")}
        if len(set(selected)) != len(selected) or not set(selected) <= available:
            parser.error("samples must be distinct indexed corpus IDs")
        manifests.append((corpus, manifest))
    library = args.library.resolve()
    core = CAbi(library)
    if core.invoke("compatibility", {})[0]["api"] != {"major": 1, "minor": 4}:
        parser.error("build the API v1.4 release library before measuring")
    cpu = read_optional("/proc/cpuinfo")
    cpu_model = next((line.split(":", 1)[1].strip() for line in (cpu or "").splitlines() if line.startswith("model name")), None)
    report = {"version": 1, "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "environment": {"os": platform.platform(), "machine": platform.machine(), "cpu": cpu_model,
            "logical_cpus": os.cpu_count(), "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
            "memory": read_optional("/proc/meminfo"), "rust": command_output(["rustc", "--version"]),
            "python": platform.python_version(), "git_commit": command_output(["git", "rev-parse", "HEAD"]),
            "dirty_workspace": bool(command_output(["git", "status", "--porcelain"]))},
        "artifacts": {"library_sha256": sha256(library), "cargo_lock_sha256": sha256(ROOT / "Cargo.lock"),
            "runner_sha256": sha256(SCRIPT), "source_map_sha256": sha256(ROOT / "crates/perch-jobs/src/source_map.rs"),
            "source_sha256": {str(path.relative_to(ROOT)): sha256(path) for path in [
                ROOT / "crates/perch-jobs/src/lib.rs", ROOT / "crates/perch-jobs/src/pipeline.rs",
                ROOT / "crates/perch-jobs/src/store.rs", ROOT / "crates/perch-ffi/src/lib.rs",
                ROOT / "tools/bench.py", ROOT / "tools/make_corpus.py"]}},
        "measurement": {"iterations": args.iterations, "warmups": args.warmups,
            "timer": "time.perf_counter_ns; JSON/C ABI/file I/O included; close/validation excluded",
            "cache": "page cache uncontrolled; artifacts prepared before workers; warmups included",
            "rss": "fresh exec per sample/mode; whole Python+C ABI process, not SDK-exclusive allocations; "
                "raw VmRSS samples and final VmHWM retained; maximum_recorded_rss uses their maximum; "
                "Linux counters are approximate/asynchronous and do not guarantee all transient peaks",
            "load_comparison": "identical index/body, optional sourceMap present versus omitted; current implementation in both",
            "open_comparison": "book.open.path measured for the new implementation only; no reconstructed pre-LOC baseline"},
        "filesystems": {"artifacts": filesystem(ROOT / "target"), "inputs": [filesystem(path) for path in corpora]},
        "corpora": [{"profile": manifest["profile"], "scale": manifest["scale"], "manifest_sha256": sha256(corpus / "manifest.json")}
            for corpus, manifest in manifests], "samples": [], "cases": []}
    with tempfile.TemporaryDirectory(prefix="perch-loc01-", dir=ROOT / "target") as temporary:
        for corpus, manifest in manifests:
            for sample in manifest["samples"]:
                if sample["id"] not in selected:
                    continue
                ident = f"{manifest['profile']}-{manifest['scale']}/{sample['id']}"
                directory = Path(temporary) / ident
                config, sizes = prepare(core, corpus, sample, directory)
                report["samples"].append({"id": ident, **sizes})
                for mode in MODES:
                    print(f"Measuring {ident} {mode}", file=sys.stderr, flush=True)
                    configuration = {**config, "mode": mode, "library": str(library), "iterations": args.iterations, "warmups": args.warmups}
                    config_path = directory / "worker.json"
                    config_path.write_text(json.dumps(configuration), encoding="utf-8")
                    child = subprocess.run([sys.executable, str(SCRIPT), "--worker", str(config_path)],
                        capture_output=True, text=True, check=True, timeout=240)
                    report["cases"].append({"id": ident, "mode": mode, **json.loads(child.stdout)})
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    lines = ["# LOC-01 源映射测量", "", f"原始记录：[{args.output.name}]({args.output.name})。", "",
        "同一 release 库通过 C ABI 测量。重开比较使用相同 index/body，仅去掉可选 sourceMap；",
        "路径抽取打开仅记录新实现，不将其解释为改动前后比较。",
        f"共 {len(report['cases'])} 项，每项独立进程、{args.iterations} 次计时、{args.warmups} 次预热，系统页缓存未控制。",
        "RSS 包含 Python、Rust、准备和校验。保留每轮 VmRSS 与最终 VmHWM 原值，表中取最大记录值。",
        "Linux 计数存在近似/异步更新，可能出现最终 VmHWM 小于先前 VmRSS，不能保证捕获所有瞬时峰值。",
        "进程间差值不能解释为 SDK 独占分配量。", "",
        "| 样本 | index 字节 | 原 text 字节 | 带映射 text 字节 | 增量字节 | 区间数 |", "|---|---:|---:|---:|---:|---:|"]
    for sample in report["samples"]:
        lines.append(f"| {sample['id']} | {sample['index_bytes']} | {sample['legacy_text_bytes']} | {sample['mapped_text_bytes']} | {sample['added_bytes']} | {sample['run_count']} |")
    lines += ["", "| 样本 | 方法 | p50 ms | p90 ms | p99 ms | 首次操作后 RSS MiB | 最大记录 RSS MiB |", "|---|---|---:|---:|---:|---:|---:|"]
    for case in report["cases"]:
        summary = case["summary"]
        def mib(value):
            return f"{value / 1048576:.3f}" if value is not None else "未测"
        lines.append(f"| {case['id']} | {case['mode']} | {summary['p50_ns'] / 1e6:.4f} | {summary['p90_ns'] / 1e6:.4f} | {summary['p99_ns'] / 1e6:.4f} | {mib(case['rss_after_first_operation_bytes'])} | {mib(case['maximum_recorded_rss']['bytes'])} |")
    args.output.with_suffix(".md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Recorded {len(report['cases'])} cases to {args.output}")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--worker":
        print(json.dumps(worker(json.loads(Path(sys.argv[2]).read_text(encoding="utf-8")))))
    else:
        main()
