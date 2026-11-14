#!/usr/bin/env python3
"""Handwritten unsafe Rust guard.

flutter_rust_bridge (v2) generated glue must contain unsafe FFI, so
`hermod-flutter-bridge` opts out of the workspace `forbid(unsafe_code)`
lint for its generated file. Audited native Windows FFI lives in explicitly
allowlisted bridge/CLI Windows files; all other handwritten source is checked.
"""
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]
FORBIDDEN_GENERATED = {"frb_generated.rs"}
CHANGED_EXTENSIONS = {".rs"}
DESCRIPTION = "handwritten unsafe Rust"
failures: list[str] = []

# Native Windows FFI is isolated in bridge/CLI Windows modules. Review every allowlisted
# file; all other handwritten source (including future platform modules) is checked.
NATIVE_FFI_ALLOWLIST = {
    Path("flutter-bridge/src/windows/acl.rs"),
    Path("flutter-bridge/src/windows/files.rs"),
    Path("cli/src/windows.rs"),
}

for crate in ("core", "cli", "flutter-bridge"):
    for source in (ROOT / crate / "src").rglob("*.rs"):
        if source.name in FORBIDDEN_GENERATED:
            continue
        if source.relative_to(ROOT) in NATIVE_FFI_ALLOWLIST:
            continue
        text = source.read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), start=1):
            if re.search(r"\bunsafe\b", line) and not line.lstrip().startswith("//"):
                failures.append(f"{source.relative_to(ROOT)}:{number}: {line.strip()}")

if failures:
    print(f"{DESCRIPTION} detected:")
    for failure in failures:
        print(f"  {failure}")
    sys.exit(1)
print("Unsafe boundary verified: generated FRB glue and explicitly audited native platform modules only")
