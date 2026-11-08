#!/usr/bin/env python3
"""Handwritten unsafe Rust guard.

flutter_rust_bridge (v2) generated glue must contain unsafe FFI, so
`hermod-flutter-bridge` opts out of the workspace `forbid(unsafe_code)`
lint for its single generated file. This checker enforces the workspace
rule everywhere else: any `unsafe` keyword outside the generated file
fails the build.
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent.parent
FORBIDDEN_GENERATED = {"frb_generated.rs"}
CHANGED_EXTENSIONS = {".rs"}
DESCRIPTION = "handwritten unsafe Rust"
failures: list[str] = []

for crate in ("core", "cli", "flutter-bridge"):
    for source in (ROOT / crate / "src").rglob("*.rs"):
        if source.name in FORBIDDEN_GENERATED:
            continue
        text = source.read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), start=1):
            if "unsafe" in line and not line.lstrip().startswith("//"):
                failures.append(f"{source.relative_to(ROOT)}:{number}: {line.strip()}")

if failures:
    print(f"{DESCRIPTION} detected:")
    for failure in failures:
        print(f"  {failure}")
    sys.exit(1)
print("No handwritten unsafe Rust outside flutter_rust_bridge generated glue")
