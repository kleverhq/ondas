#!/usr/bin/env python3
"""Check downstream FSDB linking and stdout without Cargo loader paths."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = r"""
mod fsdb_compatibility;
use std::io::Write;

fn main() {
    let path = std::env::args_os().nth(1).expect("FSDB path");
    let path = std::path::Path::new(&path);
    let mode = std::env::args().nth(2).expect("check mode");
    std::thread::scope(|threads| {
        threads.spawn(|| {
            for index in 0..32 {
                std::io::stdout().lock()
                    .write_all(format!("app stdout {index}\n").as_bytes()).unwrap();
                std::io::stderr().lock()
                    .write_all(format!("app stderr {index}\n").as_bytes()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        if mode == "metadata" {
            ondas::read_metadata(path).unwrap();
            return;
        }
        if mode == "error" {
            assert!(ondas::open(path).is_err());
            assert!(ondas::open_with(path, "fsdb-lib").is_err());
            assert!(ondas::read_metadata(path).is_err());
            return;
        }
        let result = if mode == "probe" {
            ondas::open(path)
        } else {
            ondas::open_with(path, "fsdb-lib")
        };
        let Some(mut wave) = fsdb_compatibility::open(result, path) else {
            std::process::exit(77);
        };
        assert_eq!(wave.format(), ondas::Format::Fsdb);
        let signal = wave.hierarchy().signals()
            .find(|s| s.encoding() != ondas::Encoding::Unsupported).unwrap();
        wave.sample(signal, ondas::Time::from_ticks(0)).unwrap();
        let mut count = 0;
        let result = wave.scan(&[signal], ondas::TimeRange::all(), |_| {
            count += 1;
            std::ops::ControlFlow::<()>::Continue(())
        }).unwrap();
        assert_eq!(result, std::ops::ControlFlow::Continue(()));
        assert!(count > 0);
    });
    println!("app done");
}
"""


def main():
    fixture = (
        ROOT / "fixtures/fsdb/fsdb0005-compare-xz/waveform.fsdb"
    ).resolve(strict=True)
    compatible = (
        ROOT / "fixtures/fsdb/fsdb0033-sti-dac/waveform.fsdb"
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
        for name in ("NO_DUMP_LOGDIR", "NOVAS_NO_DUMP_LOGDIR", "FSDB_ENV_SKIP_COPYRIGHT"):
            env.pop(name, None)
        empty = crate / "empty.fsdb"
        empty.touch()
        cases = [(fixture, "open", True), (compatible, "metadata", False),
                 (compatible, "probe", False), (compatible, "open", False),
                 (empty, "error", False)]
        expected = "".join(f"app stdout {index}\n" for index in range(32)) + "app done\n"
        for path, mode, allow_skip in cases:
            result = subprocess.run(
                [str(crate / "target/debug/ondas-fsdb-consumer"), str(path), mode],
                env=env, cwd=crate, capture_output=True, text=True, timeout=30,
            )
            if result.returncode == 77 and allow_skip:
                print("Standalone FSDB consumer started; file skipped for Reader version.")
                continue
            if result.returncode:
                print(result.stderr, end="")
            result.check_returncode()
            assert result.stdout == expected, (mode, result.stdout, result.stderr)
            for index in range(32):
                assert result.stderr.count(f"app stderr {index}\n") == 1, (mode, index)
        print("Standalone FSDB consumer passed with clean stdout and app output preserved.")


if __name__ == "__main__":
    main()
