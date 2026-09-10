#!/usr/bin/env python3
"""Install locked fixture payloads without resetting existing checkouts."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PROVIDER = "kleverhq.ondas-fixtures"
REPOSITORY = "https://github.com/kleverhq/ondas-fixtures.git"


def install(root, lock=ROOT / "fixtures.lock.toml"):
    version = tomllib.loads(lock.read_text())["providers"][PROVIDER]
    tag = f"v{version}"
    directory = root / PROVIDER
    root.mkdir(parents=True, exist_ok=True)
    if not directory.exists():
        subprocess.run(["git", "clone", "--depth", "1", "--branch", tag,
                        REPOSITORY, str(directory)], check=True)
    head = subprocess.check_output(["git", "-C", str(directory), "rev-parse", "HEAD"], text=True).strip()
    pinned = subprocess.check_output(["git", "-C", str(directory), "rev-parse", f"refs/tags/{tag}^{{commit}}"], text=True).strip()
    if head != pinned:
        raise ValueError(f"{directory}: expected {tag}; refusing to change existing checkout")
    subprocess.run(["git", "-C", str(directory), "diff", "--exit-code", "HEAD", "--"], check=True)
    catalog = json.loads((directory / "catalog.json").read_text())
    if catalog["provider"] != PROVIDER or catalog["version"] != version:
        raise ValueError(f"{directory}: catalog does not match fixtures.lock.toml")
    subprocess.run([sys.executable, str(directory / "install.py")], check=True)


if __name__ == "__main__":
    try:
        location = os.environ.get("ONDAS_FIXTURES")
        if not location:
            raise ValueError("ONDAS_FIXTURES is required")
        install(Path(location).resolve())
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        print(f"fixtures-install: {error}", file=sys.stderr)
        sys.exit(1)
