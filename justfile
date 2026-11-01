# PerchCore: run from this repository root.
api_python := if os() == "windows" { "target/api-tools/Scripts/python.exe" } else { "target/api-tools/bin/python" }

default:
    @just --list

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

test:
    cargo test --workspace --all-features --locked

core-lib:
    cargo build -p perch-ffi --release --locked

ci: corpus-check api-check fmt-check lint test

# Install the development-only schema validator into a local virtual environment.
api-setup:
    python3 -m venv target/api-tools
    {{api_python}} -m pip install -r tools/requirements-api.txt

api-generate:
    python3 tools/api_catalog.py

api-check:
    python3 tools/api_catalog.py --check
    {{api_python}} tools/check_api.py
    {{api_python}} -m unittest discover -s tools -p 'api_contract_tests.py'

api-replay:
    {{api_python}} tools/check_api.py --replay

rust-example:
    cargo run -p perch-core --example inspect -- examples/data/sample.txt 文字

sample-epub:
    python3 tools/make_sample_epub.py target/sample.epub

corpus:
    python3 tools/make_corpus.py

corpus-check:
    python3 tools/make_corpus.py --check
    python3 -m unittest discover -s tools -p 'test_*.py'

corpus-performance scale="256":
    python3 tools/make_corpus.py --profile performance --scale {{scale}}

bench-register:
    cargo run -p perch-import --release --example bench_register

bench corpus="examples/data/corpus":
    python3 tools/bench.py --corpus {{corpus}}

bench-all corpus="examples/data/corpus":
    python3 tools/bench.py --corpus {{corpus}} --clients rust,ctypes,dart,dart-worker

bench-performance scale="64":
    python3 tools/make_corpus.py --profile performance --scale {{scale}} --output target/performance-corpus
    python3 tools/bench.py --corpus target/performance-corpus --output target/base02/performance.json

c-example: core-lib
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
      Darwin) extension=dylib ;;
      Linux) extension=so ;;
      *) echo "Use examples/c/CMakeLists.txt with your platform import library" >&2; exit 1 ;;
    esac
    cmake -S examples/c -B target/c-example -DPERCH_CORE_LIBRARY="${PWD}/target/release/libperch_ffi.${extension}"
    cmake --build target/c-example
    ctest --test-dir target/c-example --output-on-failure

ffi-smoke: core-lib
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
      Darwin) library=libperch_ffi.dylib ;;
      Linux) library=libperch_ffi.so ;;
      *) library=perch_ffi.dll ;;
    esac
    cd bindings/dart/perch_core
    dart pub get
    dart analyze
    dart run tool/ffi_smoke.dart "../../../target/release/${library}"

dart-example: core-lib
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
      Darwin) library=libperch_ffi.dylib ;;
      Linux) library=libperch_ffi.so ;;
      *) library=perch_ffi.dll ;;
    esac
    cd bindings/dart/perch_core
    dart pub get
    dart run example/main.dart "../../../target/release/${library}" ../../../examples/data/sample.txt

check-ios:
    cargo check --workspace --all-features --locked --target aarch64-apple-ios

check-ios-sim:
    cargo check --workspace --all-features --locked --target aarch64-apple-ios-sim
    cargo check --workspace --all-features --locked --target x86_64-apple-ios

build-ios-lib:
    cargo rustc -p perch-ffi --release --locked --target aarch64-apple-ios --crate-type staticlib

ios-symbols: build-ios-lib
    python3 tools/check_ios_symbols.py target/aarch64-apple-ios/release/libperch_ffi.a
