#!/usr/bin/env python3
"""Keep Linux release executables runnable on Ubuntu 22.04 (docs/ci-runners.md)."""
import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

# Ubuntu 22.04's libc. Newer builders must never silently raise this floor.
FLOOR = (2, 35)
FLOOR_TEXT = ".".join(map(str, FLOOR))


def incompatible_versions(report):
    """Version requirements, not the builder's installed libc, determine compatibility."""
    incompatible = set()
    needs = False
    for line in report.splitlines():
        if line.startswith("Version "):
            needs = line.startswith("Version needs section")
        if not needs:
            continue
        for name in re.findall(r"\bName:\s+(GLIBC_[A-Za-z0-9_.]+)", line):
            version = re.fullmatch(r"GLIBC_(\d+(?:\.\d+)+)", name)
            # ABI tags (e.g. GLIBC_ABI_DT_RELR, added in 2.36) and private versions
            # are not an Ubuntu 22.04 public ABI. Unknown tags must not pass silently.
            if version is None or tuple(map(int, version[1].split("."))) > FLOOR:
                incompatible.add(name)
    return sorted(incompatible)


def check_binaries(binaries):
    """Raise an error if any ELF file exceeds the Linux release baseline."""
    readelf = shutil.which("readelf")
    if readelf is None:
        raise ValueError("readelf is required (install binutils)")
    errors = []
    for binary in binaries:
        result = subprocess.run([readelf, "--wide", "--version-info", "--", binary], text=True, capture_output=True,
                                env={**os.environ, "LC_ALL": "C"})
        if result.returncode:
            errors.append(result.stderr.strip() or f"{binary}: readelf failed with status {result.returncode}")
            continue
        incompatible = incompatible_versions(result.stdout)
        if incompatible:
            errors.append(f"{binary}: requires {', '.join(incompatible)}; "
                          f"Linux releases must support glibc {FLOOR_TEXT} (Ubuntu 22.04)")
        else:
            print(f"{binary}: glibc requirements fit Ubuntu 22.04")
    if errors:
        raise ValueError("\n".join(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binaries", type=Path, nargs="+")
    args = parser.parse_args()
    try:
        check_binaries(args.binaries)
    except ValueError as error:
        sys.exit(str(error))


if __name__ == "__main__":
    main()
