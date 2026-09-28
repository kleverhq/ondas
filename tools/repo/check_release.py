#!/usr/bin/env python3
"""Validate release metadata and print the current version's changelog notes."""

import argparse
from datetime import date
from pathlib import Path
import re
import sys
import tomllib


def release_notes(root: Path, tag: str | None = None) -> str:
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    package = manifest["package"]
    version = package["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("package.version must be stable X.Y.Z")
    if tag is not None and tag != f"v{version}":
        raise ValueError(f"release tag must be v{version}, got {tag!r}")

    lock = tomllib.loads((root / "Cargo.lock").read_text())
    versions = [p["version"] for p in lock["package"] if p["name"] == package["name"]]
    if versions != [version]:
        raise ValueError("Cargo.lock package version does not match Cargo.toml")

    changelog = (root / "CHANGELOG.md").read_text()
    headings = list(re.finditer(
        rf"^## \[{re.escape(version)}\] - (\d{{4}}-\d{{2}}-\d{{2}})$",
        changelog, re.MULTILINE,
    ))
    if len(headings) != 1:
        raise ValueError(f"expected one changelog heading: ## [{version}] - YYYY-MM-DD")
    heading = headings[0]
    date.fromisoformat(heading[1])
    notes = re.split(r"^## |^\[[^\]\n]+\]:", changelog[heading.end():],
                     maxsplit=1, flags=re.MULTILINE)[0].strip()
    if not notes or not any(line.strip() and not line.startswith("#")
                            for line in notes.splitlines()):
        raise ValueError(f"changelog notes for {version} are empty")
    return notes + "\n"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", help="Require this exact stable vX.Y.Z tag")
    args = parser.parse_args()
    try:
        print(release_notes(Path.cwd(), args.tag), end="")
    except (OSError, ValueError, KeyError) as error:
        sys.exit(f"error: release: {error}")


if __name__ == "__main__":
    main()
