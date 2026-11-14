#!/usr/bin/env python3
"""Validate offline API contracts and replay the same calls through real clients."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import sys
import tempfile
import threading

from jsonschema import Draft202012Validator, FormatChecker
from referencing import Registry, Resource
from referencing.exceptions import NoSuchResource

from api_catalog import COMMON_ID, REQUEST_ID, RESPONSE_ID, ROOT, SCHEMA_VERSION, artifacts
from api_examples import BOOK, LOC_BOOK, LOC_TEXT

CONTEXT_ENTRY_ERRORS = {"CONTEXT_BAD_HANDLE", "CONTEXT_METHOD_SCOPE"}
FROZEN_V1_SCHEMAS = {
    "common": "50b6487c41a2a8c03fe5fce1e1671a0627d27e2df3c95e7b1f6d2fbc963e3ca7",
    "request": "85be3a7baf47128cdd8bebb46743bc54481ca91a9e31a37e98d2182fd1e0c2d5",
    "response": "79dd0f66dcbb1ac3b7bced1a4f9fc005d52a1399699c32d1b359d0ca3070d028",
}
FROZEN_V1_2_FILES = {
    "common.schema.json": "2a2c4961771eaf010c5023d2796a8ba7647dab2620521fe22c752a711e5268c7",
    "request.schema.json": "13ff7a70daf178841177e81f75b80041b4f12514a0bb9978b57712db5177c19f",
    "response.schema.json": "b66519e6836444bf076de1776bc54ca2c93c1bfbd011c14c714a612175bdceee",
    "examples.json": "f72834921ce2148626d9cda95735dea9fbe2fd65447f504a39940f03722a198f",
    "responses.json": "89398a459f4e2d97499fdba1c4398eb0159a9fe4be45ba8aad2ba5c40efff20f",
}
FROZEN_V1_3_FILES = {
    "common.schema.json": "419312d1c8b7a26a61311ce02d8434e597d1b3cd1b5cabbfd13c78a08532d1ff",
    "request.schema.json": "8936c1abf5df6874cebce3e04c07238f9f7ad9a55bb34998715cfb76cfac3567",
    "response.schema.json": "10119d3167d6575c3d64e3e97ec689ea6564db7187660e551713ac967244810a",
    "examples.json": "9fc2cea996f32387c89f42902f3306dd3690b72def359845be37dc4cc2ea3b33",
    "responses.json": "2e536e679eeda8bcdd92792604f51c61924c48a7b3821a0d78df09ff8d2be9e2",
}


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def no_network(uri):
    raise NoSuchResource(ref=uri)


def validators(version=SCHEMA_VERSION):
    registry = Registry(retrieve=no_network)
    schemas = {}
    for name in ("common", "request", "response"):
        schema = read_json(ROOT / "api" / version / f"{name}.schema.json")
        Draft202012Validator.check_schema(schema)
        registry = registry.with_resource(schema["$id"], Resource.from_contents(schema))
        schemas[schema["$id"]] = schema
    return {ident: Draft202012Validator(schema, registry=registry, format_checker=FormatChecker())
            for ident, schema in schemas.items()}


def validate(validator, value, label):
    errors = list(validator.iter_errors(value))
    if errors:
        raise AssertionError(f"{label}: {errors[0].message[:250]}")


def resolve(value, variables, results):
    if isinstance(value, dict):
        if set(value) == {"$var"}:
            return variables[value["$var"]]
        if set(value) == {"$ref"}:
            parts = value["$ref"].split(".")
            result = results[parts[0]]
            for key in parts[1:]:
                result = result[int(key)] if isinstance(result, list) else result[key]
            return result
        return {key: resolve(part, variables, results) for key, part in value.items()}
    if isinstance(value, list):
        return [resolve(part, variables, results) for part in value]
    return value


def subset(actual, expected, label):
    if isinstance(expected, dict):
        assert isinstance(actual, dict), f"{label}: expected object, got {actual!r}"
        for key, value in expected.items():
            assert key in actual, f"{label}: missing {key}"
            subset(actual[key], value, f"{label}.{key}")
    else:
        assert actual == expected, f"{label}: {actual!r} != {expected!r}"


def variables(directory):
    corpus = ROOT / "examples/data/corpus/inputs"
    return {"source": str(directory / "中文窗口.txt"), "missing": str(directory / "missing.txt"),
            "out": str(directory / "artifacts"), "out2": str(directory / "second-pipeline"),
            "context_out_first": str(directory / "context-first"),
            "context_out_second": str(directory / "context-second"),
            "context_out_extra": str(directory / "context-extra"),
            "bad_policy_dir": str(directory / "bad-policy"), "blocked": str(directory / "blocked"),
            "index": str(directory / "artifacts" / BOOK / "index.bin"),
            "text_json": str(directory / "artifacts" / BOOK / "text.json"),
            "future_json": str(directory / "future.json"),
            "mismatch_json": str(directory / "mismatch.json"),
            "loc_source": str(directory / "source-locations.txt"),
            "loc_out": str(directory / "location-artifacts"),
            "loc_index": str(directory / "location-artifacts" / LOC_BOOK / "index.bin"),
            "loc_text_json": str(directory / "location-artifacts" / LOC_BOOK / "text.json"),
            **{variant: str(directory / f"{variant}.json") for variant in ("missing_map", "future_map", "bad_map", "null_map")},
            "legacy_index": str(directory / "legacy-index.bin"),
            "legacy_text": str(ROOT / "examples/data/compatibility/text-v0.json"),
            "gbk": str(corpus / "gbk.txt"), "pdf": str(corpus / "pdf-outline.pdf"),
            "epub": str(corpus / "epub3.epub"), "big5": str(corpus / "big5.txt"),
            "long_line": str(corpus / "long-line.txt"),
            "epub_hex": (corpus / "epub3.epub").read_bytes().hex(),
            "pdf_hex": (corpus / "pdf-outline.pdf").read_bytes().hex()}


def normalized(result, context):
    # Errors are compared by code; messages depend on paths, OS, and toolchain.
    if isinstance(result, dict) and "error" in result:
        return {"error": {"code": result["error"]["code"], "message": "<diagnostic>"}}
    if isinstance(result, dict):
        return {key: sorted(normalized(value, context), key=lambda item: item["book"])
                if key == "recovered" else normalized(value, context) for key, value in result.items()}
    if isinstance(result, list):
        return [normalized(value, context) for value in result]
    if isinstance(result, str):
        for name, value in sorted(context.items(), key=lambda item: len(item[1]), reverse=True):
            if name not in ("epub_hex", "pdf_hex"):
                result = result.replace(value, f"${{{name}}}")
    return result


def method_names():
    source = (ROOT / "crates/perch-ffi/src/lib.rs").read_text(encoding="utf-8")
    source = source.split("fn call(", 1)[1].split("/// 解析可选的", 1)[0]
    return set(re.findall(r'^ {8}"([^"]+)" =>', source, flags=re.MULTILINE))


def check_offline(require_responses=True):
    for name, content in artifacts().items():
        assert (ROOT / name).read_text(encoding="utf-8") == content, f"Generated drift: {name}"
    validation = validators()
    catalog = read_json(ROOT / "api/catalog.json")["methods"]
    names = {entry["name"] for entry in catalog}
    assert len(names) == len(catalog) == 35
    assert names == method_names(), f"FFI/catalog method drift: {names ^ method_names()}"
    fixtures = read_json(ROOT / "api/examples.json")
    calls = [call for call in fixtures["calls"] if "method" in call]
    identifiers = {call["id"] for call in calls}
    assert len(identifiers) == len(calls), "Duplicate example IDs"
    assert names == {call["method"] for call in calls if not call.get("error")}, "Missing successful method example"
    if not require_responses:
        return validation
    responses = read_json(ROOT / "api/responses.json")
    assert set(responses) == identifiers, "Response examples drift from requests"
    context = variables(Path("/offline/perch"))
    results = {}
    for call in calls:
        args = resolve(call["args"], context, results)
        request = {"method": call["method"], "args": args}
        accepted = validation[REQUEST_ID].is_valid(request)
        assert accepted == call["canonical"], f"{call['id']}: canonical request classification wrong"
        response = responses[call["id"]]
        if call["method"] in names:
            validate(validation[RESPONSE_ID], {"method": call["method"], "result": response}, call["id"])
        else:
            error_validator = validation[COMMON_ID].evolve(schema={"$ref": f"{COMMON_ID}#/$defs/Error"})
            validate(error_validator, response, call["id"])
        if call.get("error"):
            assert response["error"]["code"] == call["error"]
        else:
            assert "error" not in response, call["id"]
            subset(response, call.get("expect", {}), call["id"])
        results[call["id"]] = response
    for name, expected_hash in FROZEN_V1_SCHEMAS.items():
        path = ROOT / "api" / f"{name}.schema.json"
        assert hashlib.sha256(path.read_bytes()).hexdigest() == expected_hash, f"Frozen v1 schema drift: {name}"
        legacy = read_json(path)
        Draft202012Validator.check_schema(legacy)
    check_path_parity(responses)
    check_frozen_v1_2()
    check_frozen_v1_3()
    check_location_parity(responses)
    print(f"Offline contracts: {len(names)} methods / 3 current + 9 frozen Draft 2020-12 schemas / {len(calls)} request-response examples passed")
    return validation


def check_path_parity(responses):
    for legacy, path in [("refine", "refine_path"), ("refine_epub", "refine_epub_path"),
                         ("book_open", "book_path"), ("book_epub_open", "book_epub_path"),
                         ("book_search", "book_path_search"), ("book_epub_search", "book_epub_path_search"),
                         ("txt_analyze", "inspect_path")]:
        before = {key: value for key, value in responses[legacy].items() if key not in ("handle", "text")}
        after = {key: value for key, value in responses[path].items() if key not in ("handle", "text")}
        assert before == after, f"Path/bytes result drift: {path}"
    assert "text" not in responses["inspect_large"] and "handle" not in responses["inspect_large"]
    assert responses["inspect_large"]["chars"] > 20000
    assert len(json.dumps(responses["inspect_large"])) < 256


def check_location_parity(responses):
    for name in ("loc_txt_path", "loc_loaded"):
        assert responses[name] == responses["loc_txt"], f"Source location drift: {name}"
    for name in ("loc_load_search", "loc_missing_map_search", "loc_future_map_search", "loc_bad_map_search", "loc_null_map_search"):
        assert responses[name] == responses["loc_txt_search"], f"Mapping error blocked or changed search: {name}"
    for name, expected in [("loc_txt", "乙"), ("loc_txt_emoji", "😀"), ("loc_txt_combining", "é"), ("loc_txt_collapsed", "\r\n\r\n")]:
        location = responses[name]
        start, end = location["start"], location["end"]
        assert LOC_TEXT[start["charOffset"]:end["charOffset"]] == expected
        assert LOC_TEXT.encode()[start["byteOffset"]:end["byteOffset"]].decode() == expected
    assert responses["loc_txt_read"]["text"] == "乙"
    corpus = ROOT / "examples/data/corpus/texts"
    for encoding in ("gbk", "big5"):
        decoded = (corpus / f"{encoding}.decoded.txt").read_bytes().decode()
        start, end = responses[f"loc_{encoding}"]["start"], responses[f"loc_{encoding}"]["end"]
        assert decoded[start["charOffset"]:end["charOffset"]] == "中文"
        assert decoded.encode()[start["byteOffset"]:end["byteOffset"]].decode() == "中文"


def check_frozen_v1_2():
    check_frozen("v1.2", FROZEN_V1_2_FILES, 161)


def check_frozen_v1_3():
    check_frozen("v1.3", FROZEN_V1_3_FILES, 189)


def check_frozen(version, hashes, count):
    directory = ROOT / "api" / version
    for name, expected_hash in hashes.items():
        assert hashlib.sha256((directory / name).read_bytes()).hexdigest() == expected_hash, f"Frozen {version} drift: {name}"
    validation = validators(version)
    request_validator = validation[f"urn:perchcore:api:{version}:request"]
    response_validator = validation[f"urn:perchcore:api:{version}:response"]
    fixtures = read_json(directory / "examples.json")
    responses = read_json(directory / "responses.json")
    current = read_json(ROOT / "api/responses.json")
    assert len(responses) == count
    context, results = variables(Path("/offline/perch")), {}
    for call in fixtures["calls"]:
        if "method" not in call:
            continue
        args = resolve(call["args"], context, results)
        assert request_validator.is_valid({"method": call["method"], "args": args}) == call["canonical"], call["id"]
        response = responses[call["id"]]
        expected = json.loads(json.dumps(response))
        if call["id"] == "compatibility":
            expected["api"]["minor"] = current[call["id"]]["api"]["minor"]
        assert current[call["id"]] == expected, f"Legacy response drift: {call['id']}"
        if call["method"] != "unknown.method":
            validate(response_validator, {"method": call["method"], "result": response}, call["id"])
        subset(response, {"error": {"code": call["error"]}} if call.get("error") else call.get("expect", {}), call["id"])
        results[call["id"]] = response


def run_command(command, cwd=ROOT, env=None):
    result = subprocess.run(command, cwd=cwd, env=env, text=True, encoding="utf-8",
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=240)
    if result.returncode:
        raise RuntimeError(f"Command failed: {command}\n{result.stdout[-5000:]}")
    return result.stdout


def library_path():
    filename = "perch_ffi.dll" if sys.platform == "win32" else (
        "libperch_ffi.dylib" if sys.platform == "darwin" else "libperch_ffi.so")
    return ROOT / "target/release" / filename


def build_clients(clients, library, skip_build):
    if not skip_build:
        run_command(["cargo", "build", "-p", "perch-ffi", "--release", "--locked"])
    assert library.is_file(), f"Missing shared library: {library}"
    commands = {}
    env = os.environ.copy()
    env["PATH"] = str(library.parent) + os.pathsep + env["PATH"]
    if "c" in clients:
        linked_library = library
        if sys.platform == "win32":
            candidates = [library.with_suffix(".dll.lib"), library.with_name("libperch_ffi.dll.a")]
            linked_library = next((candidate for candidate in candidates if candidate.exists()), library)
        build = ROOT / "target/api-c"
        run_command(["cmake", "-S", str(ROOT / "examples/c"), "-B", str(build),
                     f"-DPERCH_CORE_LIBRARY={linked_library}"])
        run_command(["cmake", "--build", str(build), "--config", "Release"])
        run_command(["ctest", "--test-dir", str(build), "-C", "Release", "--output-on-failure"], env=env)
        filename = "perch-replay.exe" if sys.platform == "win32" else "perch-replay"
        executable = next(path for path in (build / filename, build / "Release" / filename) if path.exists())
        commands["c"] = ([str(executable)], ROOT)
    if "dart" in clients:
        package = ROOT / "bindings/dart/perch_core"
        run_command(["dart", "pub", "get"], cwd=package)
        # AOT verifies the same deployment mode as native desktop applications.
        executable = ROOT / "target" / ("api-dart.exe" if sys.platform == "win32" else "api-dart")
        run_command(["dart", "compile", "exe", "tool/api_replay.dart", "-o", str(executable)], cwd=package)
        commands["dart"] = ([str(executable), str(library)], ROOT)
    return commands, env


class Client:
    def __init__(self, command, cwd, env, log):
        self.stderr = log.open("w", encoding="utf-8")
        self.process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=self.stderr,
                                        text=True, encoding="utf-8", bufsize=1)
        self.lines = queue.Queue()
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()
        self.log = log

    def _read(self):
        try:
            for line in self.process.stdout:
                self.lines.put(line)
        finally:
            self.lines.put(None)

    def invoke(self, method, args, context=None):
        prefix = "" if context is None else str(context)
        self.process.stdin.write(prefix + "\t" + method + "\t" + json.dumps(args, ensure_ascii=False) + "\n")
        self.process.stdin.flush()
        try:
            line = self.lines.get(timeout=30)
        except queue.Empty as error:
            raise RuntimeError(f"Client timeout at {method}; stderr: {self.log}") from error
        if line is None:
            raise RuntimeError(f"Client exited at {method}; stderr: {self.log.read_text(encoding='utf-8')}")
        return json.loads(line)

    def close(self, success):
        self.process.stdin.close()
        try:
            status = self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            status = self.process.wait()
        self.thread.join(timeout=1)
        self.process.stdout.close()
        self.stderr.close()
        if success:
            assert status == 0, f"Client exit {status}; stderr: {self.log}"


def action(step, context):
    if step["action"] == "mismatch_text":
        value = read_json(Path(context["text_json"]))
        value["text"] = "different content"
        Path(context["mismatch_json"]).write_text(json.dumps(value), encoding="utf-8")
    elif step["action"] == "future_text":
        value = read_json(Path(context["text_json"]))
        value["version"] = 2
        Path(context["future_json"]).write_text(json.dumps(value), encoding="utf-8")
    elif step["action"] == "block_book_dir":
        (Path(context["out"]) / step["book"]).write_bytes(b"block directory creation")
    elif step["action"] == "corrupt_index":
        (Path(context["out"]) / step["book"] / "index.bin").write_bytes(b"broken index")
    elif step["action"] == "source_map_variants":
        original = read_json(Path(context["loc_text_json"]))
        assert original["sourceMap"]["version"] == 1
        for variant in ("missing_map", "future_map", "bad_map", "null_map"):
            value = json.loads(json.dumps(original))
            if variant == "missing_map":
                value.pop("sourceMap")
            elif variant == "future_map":
                value["sourceMap"]["version"] = 2
            elif variant == "bad_map":
                value["sourceMap"]["runs"] = []
            else:
                value["sourceMap"] = None
            Path(context[variant]).write_text(json.dumps(value), encoding="utf-8")
    else:
        raise AssertionError(f"Unknown fixture action: {step}")


def replay(clients, library, skip_build, record):
    validation = check_offline(require_responses=not record)
    catalog = {entry["name"]: entry for entry in read_json(ROOT / "api/catalog.json")["methods"]}
    fixture = read_json(ROOT / "api/examples.json")
    commands, env = build_clients(clients, library, skip_build)
    golden = {} if record else read_json(ROOT / "api/responses.json")
    observed = {}
    states, processes = {}, {}
    success = False
    with tempfile.TemporaryDirectory(prefix="perch-api-") as temporary:
        try:
            for name, (command, cwd) in commands.items():
                directory = Path(temporary) / name
                directory.mkdir()
                context = variables(directory)
                Path(context["source"]).write_bytes(fixture["source_text"].encode())
                Path(context["loc_source"]).write_bytes(fixture["location_text"].encode())
                legacy_hex = (ROOT / "examples/data/compatibility/index-v1.hex").read_text(encoding="utf-8")
                Path(context["legacy_index"]).write_bytes(bytes.fromhex(legacy_hex))
                Path(context["blocked"]).write_bytes(b"not a directory")
                states[name] = (context, {})
                processes[name] = Client(command, cwd, env, directory / "stderr.log")
            for step in fixture["calls"]:
                if "action" in step:
                    for context, _ in states.values():
                        action(step, context)
                    continue
                outcomes = {}
                for name, process in processes.items():
                    context, results = states[name]
                    args = resolve(step["args"], context, results)
                    call_context = resolve(step["context"], context, results) if "context" in step else None
                    accepted = validation[REQUEST_ID].is_valid({"method": step["method"], "args": args})
                    assert accepted == step["canonical"], f"{step['id']}: request classification wrong"
                    result = process.invoke(step["method"], args, call_context)
                    if step["method"] in catalog:
                        validate(validation[RESPONSE_ID], {"method": step["method"], "result": result}, step["id"])
                    if step.get("error"):
                        assert result.get("error", {}).get("code") == step["error"], f"{step['id']}: {result}"
                        assert isinstance(result["error"]["message"], str) and result["error"]["message"]
                        if step["method"] in catalog:
                            assert (step["error"] in catalog[step["method"]]["errors"] or
                                    step["error"] in CONTEXT_ENTRY_ERRORS), f"Unlisted error: {step}"
                    else:
                        assert "error" not in result, f"{step['id']}: {result}"
                        subset(result, step.get("expect", {}), step["id"])
                    results[step["id"]] = result
                    outcomes[name] = normalized(result, context)
                first = next(iter(outcomes.values()))
                for name, result in outcomes.items():
                    assert result == first, f"C/Dart mismatch at {step['id']} ({name}): {result} != {first}"
                if not record:
                    assert first == golden[step["id"]], f"Golden contract drift at {step['id']}: {first} != {golden[step['id']]}"
                observed[step["id"]] = first
            check_path_parity(observed)
            success = True
        finally:
            for process in processes.values():
                process.close(success)
    if record:
        assert set(clients) == {"c", "dart"}, "Record requires both real C and Dart clients"
        (ROOT / "api/responses.json").write_text(json.dumps(observed, ensure_ascii=False, indent=2) + "\n",
                                                encoding="utf-8", newline="\n")
        check_offline()
    print(f"Real {' + '.join(clients)} ABI: {len(observed)} calls per client / canonical requests, schema responses, assertions and snapshots passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay", action="store_true", help="Build persistent clients and run real ABI calls")
    parser.add_argument("--clients", default="c,dart", help="c,dart or c (platform smoke)")
    parser.add_argument("--library", type=Path, help="Existing shared library (default target/release)")
    parser.add_argument("--skip-build", action="store_true", help="Reuse the Rust library; still compile clients")
    parser.add_argument("--record", action="store_true", help="Explicitly replace response fixtures after BOTH clients agree")
    args = parser.parse_args()
    clients = args.clients.split(",")
    if len(set(clients)) != len(clients) or not set(clients) <= {"c", "dart"}:
        parser.error("--clients must be c,dart, c, or dart")
    if args.record and (not args.replay or set(clients) != {"c", "dart"}):
        parser.error("--record requires --replay with both c,dart clients")
    if args.replay:
        replay(clients, (args.library or library_path()).resolve(), args.skip_build, args.record)
    else:
        check_offline()


if __name__ == "__main__":
    main()
