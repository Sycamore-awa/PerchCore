#!/usr/bin/env python3
"""Assert that the iOS static library exports the public C ABI."""

from pathlib import Path
import os
import shutil
import subprocess
import sys


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: check_ios_symbols.py <libperch_ffi.a>")
    nm = os.environ.get("PERCH_NM_BIN") or shutil.which("llvm-nm")
    if nm is None:
        candidates = sorted(Path("/usr/lib").glob("llvm-*/bin/llvm-nm"))
        nm = str(candidates[-1]) if candidates else None
    if nm is None:
        raise SystemExit("llvm-nm is required; GNU nm cannot read Mach-O archives")
    result = subprocess.run(
        [nm, "--defined-only", "-g", sys.argv[1]],
        check=True,
        capture_output=True,
        text=True,
    )
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
