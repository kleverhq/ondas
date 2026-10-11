#!/usr/bin/env python3
"""Build and directly launch a separate FSDB consumer, without Cargo loader paths."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = r"""
mod fsdb_compatibility;

fn main() {
    let path = std::env::args_os().nth(1).expect("FSDB path");
    let path = std::path::Path::new(&path);
    let Some(mut wave) = fsdb_compatibility::open(ondas::open_with(path, "fsdb-lib"), path) else {
        std::process::exit(77);
    };
    assert_eq!(wave.format(), ondas::Format::Fsdb);
    let signal = wave.hierarchy().signals()
        .find(|s| s.encoding() != ondas::Encoding::Unsupported).unwrap();
    wave.sample(signal, ondas::Time::from_ticks(0)).unwrap();
}
"""


def main():
    fixture = (
        ROOT / "fixtures/fsdb/fsdb0005-compare-xz/waveform.fsdb"
    ).resolve(strict=True)
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="fsdb-consumer-", dir=scratch) as temporary:
        crate = Path(temporary)
        (crate / "src").mkdir()
        (crate / "src/main.rs").write_text(SOURCE)
        shutil.copyfile(ROOT / "tests/support/fsdb_compatibility.rs",
                        crate / "src/fsdb_compatibility.rs")
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "ondas-fsdb-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
            '[dependencies]\nondas = { path = ' + json.dumps(str(ROOT))
            + ', features = ["fsdb-lib"] }\n'
        )
        # Retain the repository's dependency versions; Cargo adds the smoke crate.
        shutil.copyfile(ROOT / "Cargo.lock", crate / "Cargo.lock")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(crate / "target")
        subprocess.run(["cargo", "build", "--manifest-path", str(crate / "Cargo.toml")],
                       env=env, check=True)
        env.pop("LD_LIBRARY_PATH", None)
        env.pop("VERDI_HOME", None)
        result = subprocess.run(
            [str(crate / "target/debug/ondas-fsdb-consumer"), str(fixture)], env=env,
        )
        if result.returncode == 77:
            print("Standalone FSDB consumer started; file skipped for Reader version.")
        else:
            result.check_returncode()
            print("Standalone FSDB consumer passed without Cargo loader environment.")


if __name__ == "__main__":
    main()
