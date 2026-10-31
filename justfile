# PerchCore: run from this repository root.
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

ci: fmt-check lint test

rust-example:
    cargo run -p perch-core --example inspect -- examples/data/sample.txt 文字

sample-epub:
    python3 tools/make_sample_epub.py target/sample.epub

bench-register:
    cargo run -p perch-import --release --example bench_register

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
