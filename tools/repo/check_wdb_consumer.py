#!/usr/bin/env python3
"""Check a compatible WDB consumer outside Cargo's loader environment."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = r"""
fn main() {
    let path = std::env::args_os().nth(1).expect("WDB path");
    if std::env::args().nth(2).as_deref() == Some("guard-hit") {
        // A clean lifetime leaves the idle owner; the next logical open must still run the
        // runtime selection guards instead of trusting the retained native reader.
        drop(ondas::open_with(&path, "wdb-lib").unwrap());
        // SAFETY: this check is a single-threaded standalone consumer and owns its environment.
        unsafe { std::env::set_var("RDI_DATADIR", std::env::args_os().nth(3).expect("wrong data path")) };
        let error = ondas::open_with(&path, "wdb-lib")
            .err()
            .expect("wrong data selection on a retained owner");
        assert!(matches!(&error, ondas::Error::UnsupportedFeature { message, .. }
            if message.contains("RDI_DATADIR")));
        println!("WDB data selection rejected on a retained owner");
        return;
    }
    if std::env::args().nth(2).as_deref() == Some("guard") {
        let error = ondas::open_with(&path, "wdb-lib").err().expect("wrong data selection");
        assert!(matches!(&error, ondas::Error::UnsupportedFeature { message, .. }
            if message.contains("RDI_DATADIR")));
        println!("WDB data selection rejected");
        return;
    }
    let mut wave = ondas::open_with(&path, "wdb-lib").unwrap();
    let metadata = ondas::read_metadata(&path).unwrap();
    assert_eq!(metadata.time_span(), wave.metadata().time_span());
    let signal = wave.hierarchy().signal("tesbench.sel").unwrap();
    for (tick, expected) in [(0, "uu"), (10_000, "00"), (20_000, "01")] {
        let ondas::Sample::Value { value: ondas::Value::Bits(bits), .. } =
            wave.sample(signal, ondas::Time::from_ticks(tick)).unwrap() else {
                panic!("expected recorded bits");
            };
        assert_eq!(bits.as_ref().to_string(), expected);
    }
    let mut count = 0;
    let _ = wave.scan(&[signal], ondas::TimeRange::all(), |_| {
        count += 1;
        std::ops::ControlFlow::<()>::Continue(())
    }).unwrap();
    assert_eq!(count, 76);
    println!("compatible WDB queries passed");
}
"""


def main():
    fixture = (ROOT / "fixtures/wdb/wdb0091-tesbench-behav/waveform.wdb").resolve(strict=True)
    sdk = Path(os.environ["XILINX_VIVADO"]).resolve(strict=True)
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wdb-consumer-", dir=scratch) as temporary:
        crate = Path(temporary)
        (crate / "src").mkdir()
        (crate / "src/main.rs").write_text(SOURCE)
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "ondas-wdb-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
            '[dependencies]\nondas = { path = ' + json.dumps(str(ROOT))
            + ', features = ["wdb-lib"] }\n'
        )
        shutil.copyfile(ROOT / "Cargo.lock", crate / "Cargo.lock")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(crate / "target")
        subprocess.run(["cargo", "build", "--offline", "--manifest-path", str(crate / "Cargo.toml")],
                       env=env, check=True)
        # Supply the selected SDK explicitly; inherit no Cargo-added loader paths. An isolated
        # TMPDIR makes the vendor backing file of this process observable after a normal exit.
        env["LD_LIBRARY_PATH"] = str(sdk / "lib/lnx64.o")
        env["RDI_DATADIR"] = str(sdk / "data")
        env.pop("XILINX_VIVADO", None)
        env.pop("XILINX_VIVADO_HOMES", None)
        temp_scope = Path(tempfile.mkdtemp(prefix="wdb-consumer-tmp-", dir=scratch))
        env["TMPDIR"] = str(temp_scope)
        result = subprocess.run(
            [str(crate / "target/debug/ondas-wdb-consumer"), str(fixture)],
            env=env, cwd=crate, capture_output=True, text=True, timeout=30,
        )
        if result.returncode:
            print(result.stderr, end="")
        result.check_returncode()
        assert result.stdout == "compatible WDB queries passed\n", result.stdout
        # A normal single-thread exit must release a parked reader exactly like a reader that is
        # dropped inside the process: no vendor backing file is left behind.
        leftovers = sorted(entry.name for entry in temp_scope.iterdir())
        assert not leftovers, f"normal exit left vendor temporary files behind: {leftovers}"
        # Keep this scope for the remaining subprocesses, which still inherit TMPDIR. The outer
        # temporary directory removes it after all of them finish.
        env["RDI_DATADIR"] = str(crate / "absent-data")
        newer = (ROOT / "fixtures/wdb/wdb0002-reg-test-behav/waveform.wdb").resolve(strict=True)
        guard = subprocess.run(
            [str(crate / "target/debug/ondas-wdb-consumer"), str(newer), "guard"],
            env=env, cwd=crate, capture_output=True, text=True, timeout=30,
        )
        guard.check_returncode()
        assert guard.stdout == "WDB data selection rejected\n", guard.stdout
        env["RDI_DATADIR"] = str(sdk / "data")
        retained = subprocess.run(
            [str(crate / "target/debug/ondas-wdb-consumer"), str(fixture), "guard-hit",
             str(crate / "absent-data")],
            env=env, cwd=crate, capture_output=True, text=True, timeout=30,
        )
        if retained.returncode:
            print(retained.stderr, end="")
        retained.check_returncode()
        assert retained.stdout == "WDB data selection rejected on a retained owner\n", retained.stdout
        print("Standalone WDB consumer read compatible observations and closed successfully.")


if __name__ == "__main__":
    main()
