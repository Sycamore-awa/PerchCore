#!/usr/bin/env python3
"""BASE-02: isolated Rust/C ABI/Dart measurements with raw timings and process peak RSS."""

from __future__ import annotations

import argparse
import concurrent.futures
import ctypes
import datetime
import hashlib
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from make_corpus import build_corpus, check_corpus

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_optional(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def peak_rss():
    if sys.platform == "linux":
        status = read_optional("/proc/self/status")
        peak = next((int(line.split()[1]) * 1024 for line in (status or "").splitlines() if line.startswith("VmHWM:")), None)
        if peak is not None:
            return {"bytes": peak, "method": "/proc/self/status VmHWM",
                    "scope": "post-exec Python process: runtime, setup, warmups, operations, validation and cleanup"}
    try:
        import resource
        usage = resource.getrusage(resource.RUSAGE_SELF)
        if sys.platform not in ("linux", "darwin"):
            return {"bytes": None, "reason": "ru_maxrss units not specified for this platform"}
        return {
            "bytes": int(usage.ru_maxrss * (1024 if sys.platform == "linux" else 1)),
            "method": "getrusage(RUSAGE_SELF).ru_maxrss",
            "scope": "entire measured process: runtime, setup, warmups, operations, validation and cleanup",
        }
    except ImportError:
        return {"bytes": None, "reason": "resource.getrusage unavailable on this platform"}


def summarize(durations, work_bytes=None, operations=1):
    if not durations or any(value < 0 for value in durations):
        raise ValueError("nonempty nonnegative durations required")
    ordered = sorted(durations)
    # Nearest rank: no interpolation or invented tail samples.
    def percentile(fraction):
        return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]
    median = percentile(0.50)
    return {
        "count": len(ordered), "percentile_method": "nearest_rank",
        "p50_ns": median, "p90_ns": percentile(0.90), "p99_ns": percentile(0.99),
        "min_ns": ordered[0], "max_ns": ordered[-1], "mean_ns": statistics.mean(ordered),
        "operations_per_second_at_p50": operations * 1e9 / median if median else None,
        "work_bytes_per_iteration": work_bytes,
        "mib_per_second_at_p50": work_bytes * 1e9 / median / (1024 ** 2) if work_bytes is not None and median else None,
    }


class CAbi:
    def __init__(self, library):
        self.library = ctypes.CDLL(str(library))
        self.call = self.library.perch_call
        self.call.argtypes = [ctypes.c_char_p, ctypes.c_size_t, ctypes.c_char_p, ctypes.c_size_t]
        self.call.restype = ctypes.c_void_p
        self.free = self.library.perch_string_free
        self.free.argtypes = [ctypes.c_void_p]
        self.free.restype = None

    def invoke(self, method, arguments):
        method_bytes = method.encode("utf-8")
        args = json.dumps(arguments, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        pointer = self.call(method_bytes, len(method_bytes), args, len(args))
        if not pointer:
            raise RuntimeError("perch_call returned NULL")
        try:
            response = ctypes.string_at(pointer)
            value = json.loads(response)
        finally:
            self.free(pointer)
        if "error" in value:
            raise RuntimeError(json.dumps(value["error"], ensure_ascii=False))
        return value, len(method_bytes) + len(args), len(response)


def ffi_worker(config):
    core = CAbi(config["library"])
    stage = config["stage"]
    source = Path(config["path"]).read_bytes()
    decoded = Path(config["decoded_path"]).read_bytes().decode("utf-8") if config["decoded_path"] else None
    indexed = Path(config["indexed_path"]).read_bytes().decode("utf-8") if config["indexed_path"] else None
    handle = None
    executor = None
    slice_expected = None
    request_durations = []
    searchable_chars = len(indexed) if indexed is not None else 0
    if stage in ("txt.slice", "txt.slice.concurrent"):
        handle = core.invoke("txt.open", {"path": config["path"]})[0]["handle"]
        start = config["slice_start"]
        slice_expected = decoded[start:start + config["slice_chars"]]
        if stage.endswith("concurrent"):
            executor = concurrent.futures.ThreadPoolExecutor(max_workers=2)
    elif stage == "book.search":
        opened = core.invoke("book.open", {"bytes_hex": source.hex(), "format": config["format"], "chunk_chars": config["chunk_chars"]})[0]
        handle = opened["handle"]
        searchable_chars = opened["charCount"]
    elif stage == "book.load":
        setup = dict(config, stage="save", iterations=1, warmups=0)
        setup_path = Path(config["scratch"]) / "setup.json"
        setup_path.write_text(json.dumps(setup), encoding="utf-8")
        subprocess.run([config["rust_binary"], str(setup_path)], check=True, capture_output=True, timeout=config["timeout_seconds"])

    def slice_call():
        started = time.perf_counter_ns()
        result = core.invoke("txt.slice", {"handle": handle, "start": config["slice_start"], "chars": config["slice_chars"]})
        return result, time.perf_counter_ns() - started

    def operation():
        if stage == "txt.slice.concurrent":
            futures = [executor.submit(slice_call) for _ in range(2)]
            results = [future.result() for future in futures]
            return results
        if stage == "txt.slice":
            return core.invoke(stage, {"handle": handle, "start": config["slice_start"], "chars": config["slice_chars"]})
        if stage in ("version", "txt.open", "pdf.analyze"):
            return core.invoke(stage, {} if stage == "version" else {"path": config["path"]})
        if stage == "book.search":
            return core.invoke(stage, {"handle": handle, "query": config["query"], "limit": 20})
        if stage == "book.load":
            return core.invoke(stage, {"index_path": str(Path(config["scratch"]) / "index.bin"), "text_path": str(Path(config["scratch"]) / "text.json")})
        if stage in ("book.open", "jobs.refine", "txt.analyze"):
            arguments = {"bytes_hex": source.hex(), "format": config["format"], "chunk_chars": config["chunk_chars"]}
            if stage == "jobs.refine":
                arguments["max_chars"] = config["max_chars"]
            return core.invoke(stage, arguments)
        raise ValueError(f"unknown FFI stage: {stage}")

    def validate(result):
        if stage == "txt.slice.concurrent":
            for (value, _, _), _ in result:
                if value["text"] != slice_expected:
                    raise ValueError("concurrent slice differs from corpus")
            return {"requests_per_iteration": 2, "slice_chars": len(slice_expected),
                    "request_bytes": sum(item[0][1] for item in result), "response_bytes": sum(item[0][2] for item in result)}
        value, request_bytes, response_bytes = result
        stats = {"request_bytes": request_bytes, "response_bytes": response_bytes}
        if stage == "version":
            if not value.get("core"):
                raise ValueError("unexpected core version")
        elif stage == "txt.open":
            if value["chars"] != len(decoded) or value["hadErrors"]:
                raise ValueError("TXT open differs from corpus")
            stats.update(chars=value["chars"], encoding=value["encoding"])
        elif stage == "txt.slice":
            if value["text"] != slice_expected:
                raise ValueError("slice differs from corpus")
            stats.update(chars=value["chars"], utf8_bytes=len(value["text"].encode("utf-8")))
        elif stage == "txt.analyze":
            if value["text"] != decoded or value["had_decode_errors"]:
                raise ValueError("TXT analysis differs from corpus")
            stats.update(chars=value["chars"])
        elif stage in ("book.open", "book.load"):
            if not value["truncated"] and value["charCount"] != len(indexed):
                raise ValueError("book character count differs from corpus")
            stats.update(chars=value["charCount"], truncated=value["truncated"])
            if stage == "book.load":
                stats["artifact_bytes"] = sum((Path(config["scratch"]) / name).stat().st_size for name in ("index.bin", "text.json"))
        elif stage == "book.search":
            ranges = [[hit["charStart"], hit["charEnd"]] for hit in value["hits"]]
            expected_ranges = [span for span in config["expected_ranges"] if span[1] <= searchable_chars]
            if ranges != expected_ranges[:20]:
                raise ValueError("FFI search differs from corpus")
            stats.update(hits=len(ranges), result_limit=20, searchable_chars=searchable_chars,
                         truncated=searchable_chars < len(indexed))
        elif stage == "jobs.refine":
            if value["charCount"] <= 0:
                raise ValueError("empty refinement")
            stats.update(visible_chars=value["charCount"])
        elif stage == "pdf.analyze":
            if value["pageCount"] != 3 or len(value["outline"]) != 2:
                raise ValueError("PDF analysis differs from corpus")
            stats.update(pages=value["pageCount"])
        return stats

    durations = []
    stats = {}
    try:
        for iteration in range(config["warmups"] + config["iterations"]):
            started = time.perf_counter_ns()
            result = operation()
            elapsed = time.perf_counter_ns() - started
            stats = validate(result)
            if stage in ("txt.open", "book.open", "book.load"):
                core.invoke("txt.close" if stage == "txt.open" else "book.close", {"handle": result[0]["handle"]})
            if iteration >= config["warmups"]:
                durations.append(elapsed)
                if stage == "txt.slice.concurrent":
                    request_durations.extend(item[1] for item in result)
    finally:
        if executor:
            executor.shutdown(wait=True)
        if handle is not None:
            core.invoke("txt.close" if stage.startswith("txt.") else "book.close", {"handle": handle})
    return {"durations_ns": durations, "request_durations_ns": request_durations,
            "stats": stats, "peak_rss": peak_rss()}


def worker(config):
    if config["client"] == "ctypes":
        return ffi_worker(config)
    if config["client"] == "rust":
        command = [config["rust_binary"], config["config_path"]]
    else:
        command = [config["dart_binary"], config["config_path"]]
    result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=config["timeout_seconds"])
    measured = json.loads(result.stdout)
    if "peak_rss" not in measured:
        measured["peak_rss"] = {"bytes": None, "reason": "worker did not report its own process peak RSS"}
    return measured


def command_text(command):
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True, check=True)
    return (result.stdout + result.stderr).strip()


def filesystem(path):
    mounts = read_optional("/proc/self/mounts")
    if mounts is None:
        return {"path": str(path), "type": None, "reason": "mount metadata unavailable"}
    matches = []
    for line in mounts.splitlines():
        fields = line.split()
        mount = fields[1].replace(r"\040", " ")
        if str(path) == mount or str(path).startswith(mount.rstrip("/") + "/"):
            matches.append((len(mount), {"path": str(path), "device": fields[0], "mount": mount,
                                       "type": fields[2], "options": fields[3]}))
    return max(matches, key=lambda match: match[0])[1] if matches else {"path": str(path), "type": None}


def environment():
    cpuinfo = read_optional("/proc/cpuinfo")
    cpu = next((line.split(":", 1)[1].strip() for line in (cpuinfo or "").splitlines() if line.startswith("model name")), None)
    memory = read_optional("/proc/meminfo")
    memory_kib = next((int(line.split()[1]) for line in (memory or "").splitlines() if line.startswith("MemTotal:")), None)
    return {
        "os": platform.platform(), "architecture": platform.machine(),
        "cpu": cpu or platform.processor() or None, "logical_cpus": os.cpu_count(),
        "cpu_affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "memory_bytes": memory_kib * 1024 if memory_kib is not None else None,
        "cgroup_memory_max": read_optional("/sys/fs/cgroup/memory.max"),
        "cpu_governor": read_optional("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        "nvme_model": read_optional("/sys/block/nvme0n1/device/model"),
        "python": sys.version, "rustc": command_text(["rustc", "-Vv"]),
        "cargo": command_text(["cargo", "--version"]),
        "git_commit": command_text(["git", "-C", str(ROOT), "rev-parse", "HEAD"]),
        "working_tree_dirty": bool(command_text(["git", "-C", str(ROOT), "status", "--porcelain"])),
        "load_average": list(os.getloadavg()) if hasattr(os, "getloadavg") else None,
    }


def stage_list(client, sample):
    if client == "rust":
        if sample["format"] == "pdf":
            return ["register", "open"]
        stages = ["register", "open", "refine", "extract", "index", "query.hit", "query.miss", "save", "reopen", "pipeline"]
        return stages + (["decode", "slice"] if sample["format"] == "txt" else [])
    if client == "dart-worker":
        return ["txt.slice"] if sample["format"] == "txt" else []
    if sample["format"] == "pdf":
        return ["pdf.analyze"] if client == "ctypes" else []
    stages = ["book.open", "book.search"]
    if sample["format"] == "txt":
        stages += ["txt.open", "txt.slice", "txt.analyze"]
        if client == "ctypes":
            stages += ["txt.slice.concurrent"]
    if client == "ctypes":
        stages += ["jobs.refine", "book.load"]
    return stages


def work_bytes(config, sample, result):
    stage = config["stage"]
    stats = result["stats"]
    if stage == "register":
        return 2 * min(sample["bytes"], 65536)
    if stage in ("slice", "txt.slice", "txt.slice.concurrent"):
        return stats.get("utf8_bytes", stats.get("response_bytes"))
    if stage == "index":
        return sample["indexed"]["utf8_bytes"] if not stats.get("truncated") else None
    if stage == "save":
        return stats["index_bytes"] + stats["text_bytes"]
    if stage in ("reopen", "book.load"):
        return stats.get("artifact_bytes")
    if stage in ("query.hit", "query.miss", "book.search", "version"):
        return None
    return sample["bytes"]


def markdown(report):
    lines = ["# PerchCore BASE-02 measurement", "",
             f'Generated: {report["created_at"]}', "",
             f'Corpus: `{report["corpus"]["profile"]}`, scale={report["corpus"]["scale"]}; iterations={report["measurement"]["iterations"]}; warmups={report["measurement"]["warmups"]}.', "",
             "Cache: OS cache uncontrolled; setup/validation and warmups read the same input. No cold-cache claim.", "",
             "Peak RSS covers the entire isolated process, including setup, validation and runtime. Values from different language runtimes include different overhead.", "",
             "| Client | Sample | Stage | p50 ms | p90 ms | p99 ms | Peak RSS MiB |", "|---|---|---|---:|---:|---:|---:|"]
    for case in report["cases"]:
        if case["status"] != "ok":
            lines.append(f'| {case["client"]} | {case["sample_id"]} | {case["stage"]} | failed | — | — | — |')
            continue
        summary = case["summary"]
        peak = case["peak_rss"]["bytes"]
        peak_text = f"{peak / 1024**2:.2f}" if peak is not None else "unavailable"
        lines.append(f'| {case["client"]} | {case["sample_id"]} | {case["stage"]} | {summary["p50_ns"]/1e6:.4f} | {summary["p90_ns"]/1e6:.4f} | {summary["p99_ns"]/1e6:.4f} | {peak_text} |')
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, default=ROOT / "examples/data/corpus")
    parser.add_argument("--output", type=Path, default=ROOT / "target/base02/report.json")
    parser.add_argument("--iterations", type=int, default=30)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument("--clients", default="rust,ctypes", help="comma-separated rust,ctypes,dart,dart-worker")
    parser.add_argument("--samples", help="comma-separated corpus IDs")
    parser.add_argument("--stages", help="comma-separated stage names")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--clean-build", action="store_true", help="require a new/empty target-dir and record clean compilation separately")
    parser.add_argument("--target-dir", type=Path, default=ROOT / "target")
    parser.add_argument("--scratch-root", type=Path, help="artifact filesystem; default: target-dir/base02/scratch")
    parser.add_argument("--timeout-seconds", type=int, default=120)
    args = parser.parse_args()
    clients = args.clients.split(",")
    if not clients or set(clients) - {"rust", "ctypes", "dart", "dart-worker"} or len(clients) != len(set(clients)):
        parser.error("unknown client")
    if args.iterations < 1 or args.warmups < 0 or args.timeout_seconds < 1:
        parser.error("iterations and timeout must be positive; warmups must be nonnegative")
    corpus = args.corpus.resolve()
    manifest = json.loads((corpus / "manifest.json").read_bytes())
    if manifest["version"] != 1 or manifest["generator_version"] != 1:
        parser.error("unsupported corpus version")
    problems = check_corpus(corpus, build_corpus(manifest["profile"], manifest["scale"]))
    if problems:
        parser.error("corpus verification failed: " + "; ".join(problems))
    samples = [sample for sample in manifest["samples"] if not args.samples or sample["id"] in args.samples.split(",")]
    if args.samples and set(args.samples.split(",")) - {sample["id"] for sample in samples}:
        parser.error("unknown sample ID")
    if not samples:
        parser.error("no matching samples")
    cases = [(client, sample, stage) for client in clients for sample in samples for stage in stage_list(client, sample)
             if not args.stages or stage in args.stages.split(",")]
    if not args.stages or "version" in args.stages.split(","):
        cases += [(client, samples[0], "version") for client in clients if client != "rust"]
    if not cases:
        parser.error("no matching cases")
    target = args.target_dir.resolve()
    if args.clean_build and (args.skip_build or (target.exists() and any(target.iterdir()))):
        parser.error("--clean-build requires builds enabled and a new or empty --target-dir")
    scratch_root = args.scratch_root.resolve() if args.scratch_root else target / "base02/scratch"
    extension = ".exe" if os.name == "nt" else ""
    rust_binary = target / f"release/examples/bench_core{extension}"
    library = target / "release" / ("perch_ffi.dll" if os.name == "nt" else "libperch_ffi.dylib" if sys.platform == "darwin" else "libperch_ffi.so")
    dart_binary = target / f"base02/bench_dart{extension}"
    builds = []

    def build(name, command, cwd=ROOT):
        print(f"Building {name} (outside runtime measurements)", file=sys.stderr, flush=True)
        started = time.perf_counter_ns()
        result = subprocess.run(command, cwd=cwd, capture_output=True, text=True, check=True)
        builds.append({"name": name, "command": command, "elapsed_ns": time.perf_counter_ns() - started,
                       "cache": "new Cargo target-dir; registry/download caches may be warm" if args.clean_build else "existing Cargo/Dart caches allowed; not a clean first build",
                       "log": result.stdout + result.stderr})

    if not args.skip_build:
        build("Rust benchmark", ["cargo", "build", "--release", "--locked", "--target-dir", str(target), "-p", "perch-core", "--example", "bench_core"])
        if set(clients) - {"rust"}:
            build("C ABI library", ["cargo", "build", "--release", "--locked", "--target-dir", str(target), "-p", "perch-ffi"])
        if set(clients) & {"dart", "dart-worker"}:
            dart_binary.parent.mkdir(parents=True, exist_ok=True)
            build("Dart dependencies", ["dart", "pub", "get", "--enforce-lockfile"], ROOT / "bindings/dart/perch_core")
            build("Dart AOT worker", ["dart", "compile", "exe", "tool/bench_ffi.dart", "-o", str(dart_binary)], ROOT / "bindings/dart/perch_core")
    artifacts = {"rust_binary": rust_binary}
    if set(clients) - {"rust"}:
        artifacts["library"] = library
    if set(clients) & {"dart", "dart-worker"}:
        artifacts["dart_binary"] = dart_binary
    for artifact in artifacts.values():
        if not artifact.is_file():
            parser.error(f"missing compiled artifact: {artifact}")
    report = {
        "version": 1, "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "environment": environment(), "builds": builds,
        "first_clean_build_elapsed_ns": sum(item["elapsed_ns"] for item in builds) if args.clean_build else None,
        "first_clean_build_note": "sum of recorded build steps; fresh Cargo target-dir, dependency download caches may be warm; Dart pub/cache state recorded in build logs" if args.clean_build else "not measured; incremental build timings are separate from operation timings",
        "artifacts": {key: {"path": str(value), "sha256": sha256(value)} for key, value in artifacts.items()},
        "sources": {path: sha256(ROOT / path) for path in ["Cargo.lock", "tools/bench.py", "crates/perch-core/examples/bench_core.rs"]},
        "corpus": {"path": str(corpus), "manifest_sha256": sha256(corpus / "manifest.json"),
                   "profile": manifest["profile"], "scale": manifest["scale"], "samples": samples},
        "measurement": {"iterations": args.iterations, "warmups": args.warmups, "clock": "Rust Instant / Python perf_counter_ns / Dart Stopwatch",
                        "cache": "uncontrolled OS page cache; fixture verification, setup and warmups read input; no cold-cache measurement",
                        "max_chars": 5_000_000, "chunk_chars": 4096, "slice_chars": 800,
                        "save_durability": "write + rename; no fsync, matching current SDK behavior",
                        "timeout_seconds": args.timeout_seconds,
                        "input_filesystem": filesystem(corpus), "artifact_filesystem": filesystem(scratch_root)},
        "unavailable": {"cold_cache": "system caches were not evicted", "mobile_devices": "not measured",
                        "energy": "not measured"},
        "cases": [],
    }
    if not args.clean_build:
        report["unavailable"]["clean_first_build"] = "not measured"
    if set(clients) & {"dart", "dart-worker"}:
        report["environment"]["dart"] = command_text(["dart", "--version"])
        report["sources"]["bindings/dart/perch_core/tool/bench_ffi.dart"] = sha256(ROOT / "bindings/dart/perch_core/tool/bench_ffi.dart")
    scratch_root.mkdir(parents=True, exist_ok=True)
    for number, (client, sample, stage) in enumerate(cases, 1):
        query = sample.get("queries", [{"query": "", "ranges": []}])[0]
        with tempfile.TemporaryDirectory(prefix="perch-base02-", dir=scratch_root) as scratch:
            config_path = Path(scratch) / "case.json"
            config = {"client": client, "stage": stage, "path": str(corpus / sample["path"]), "format": sample["format"],
                      "decoded_path": str(corpus / sample["decoded"]["path"]) if "decoded" in sample else None,
                      "indexed_path": str(corpus / sample["indexed"]["path"]) if "indexed" in sample else None,
                      "iterations": args.iterations, "warmups": args.warmups, "scratch": scratch,
                      "query": query["query"] if stage != "query.miss" else "未出现的检索词",
                      "expected_ranges": query["ranges"] if stage != "query.miss" else [],
                      "max_chars": 5_000_000, "chunk_chars": 4096,
                      "slice_start": max(0, sample.get("decoded", {}).get("chars", 0) - 800), "slice_chars": 800,
                      "rust_binary": str(rust_binary), "library": str(library), "dart_binary": str(dart_binary),
                      "config_path": str(config_path), "timeout_seconds": args.timeout_seconds}
            config_path.write_text(json.dumps(config, ensure_ascii=False), encoding="utf-8")
            case = {"client": client, "sample_id": sample["id"], "stage": stage,
                    "query": config["query"] if stage.startswith("query.") or stage == "book.search" else None,
                    "slice_start": config["slice_start"] if "slice" in stage else None}
            started = time.perf_counter_ns()
            try:
                child = subprocess.run([sys.executable, str(SCRIPT), "--worker", str(config_path)], capture_output=True,
                                       text=True, check=True, timeout=args.timeout_seconds + 10)
                result = json.loads(child.stdout)
                if len(result["durations_ns"]) != args.iterations:
                    raise ValueError("worker returned wrong number of timings")
                case.update(result, status="ok")
                case["summary"] = summarize(result["durations_ns"], work_bytes(config, sample, result),
                                            2 if stage == "txt.slice.concurrent" else 1)
                if result.get("request_durations_ns"):
                    case["request_summary"] = summarize(result["request_durations_ns"])
                message = f'p50={case["summary"]["p50_ns"]/1e6:.4f}ms'
            except (subprocess.SubprocessError, ValueError) as error:
                case.update(status="failed", error=str(error), stderr=getattr(error, "stderr", None))
                message = "FAILED"
            case["process_elapsed_ns"] = time.perf_counter_ns() - started
            report["cases"].append(case)
            print(f"[{number}/{len(cases)}] {client} {sample['id']} {stage}: {message}", file=sys.stderr, flush=True)
    report["environment"]["load_average_after"] = list(os.getloadavg()) if hasattr(os, "getloadavg") else None
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
    args.output.with_suffix(".md").write_text(markdown(report), encoding="utf-8", newline="\n")
    print(f"Wrote {args.output} and {args.output.with_suffix('.md')}")
    return int(any(case["status"] != "ok" for case in report["cases"]))


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--worker":
        config = json.loads(Path(sys.argv[2]).read_bytes())
        print(json.dumps(worker(config), ensure_ascii=False))
    else:
        try:
            raise SystemExit(main())
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            print(f"benchmark failed: {error}", file=sys.stderr)
            if isinstance(error, subprocess.CalledProcessError):
                print(error.stderr, file=sys.stderr)
            raise SystemExit(1)
