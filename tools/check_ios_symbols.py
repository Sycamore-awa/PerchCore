#!/usr/bin/env python3
"""Assert that the iOS static library exports the public C ABI."""

from pathlib import Path
import os
import shutil
import subprocess
import sys


def find_nm() -> str:
    override = os.environ.get("PERCH_NM_BIN")
    if override:
        return override
    # A distro LLVM can be too old for bitcode in Rust's static archives.
    # Prefer the matching llvm-tools-preview component when it is installed.
    sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    version = subprocess.check_output(["rustc", "-vV"], text=True)
    host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
    binary = "llvm-nm.exe" if os.name == "nt" else "llvm-nm"
    rust_nm = Path(sysroot) / "lib/rustlib" / host / "bin" / binary
    if rust_nm.is_file():
        return str(rust_nm)
    system_nm = shutil.which("llvm-nm")
    if system_nm:
        return system_nm
    candidates = sorted(Path("/usr/lib").glob("llvm-*/bin/llvm-nm"))
    if candidates:
        return str(candidates[-1])
    raise SystemExit("Install Rust's matching tools: rustup component add llvm-tools-preview")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: check_ios_symbols.py <libperch_ffi.a>")
    nm = find_nm()
    print(f"Using llvm-nm: {nm}")
    result = subprocess.run(
        [nm, "--defined-only", "-g", sys.argv[1]],
        capture_output=True,
        text=True,
    )
    if result.returncode:
        print(result.stderr, file=sys.stderr)
        raise SystemExit("llvm-nm failed; install llvm-tools-preview for the active Rust toolchain")
    symbols = {
        fields[-1]
        for line in result.stdout.splitlines()
        if len(fields := line.split()) >= 2 and fields[-2] == "T"
    }
    for symbol in ("_perch_call", "_perch_string_free"):
        if symbol not in symbols:
            raise SystemExit(f"missing global export: {symbol}")
        print(f"export verified: {symbol}")


if __name__ == "__main__":
    main()
