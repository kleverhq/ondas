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
        # Supply the selected SDK explicitly; inherit no Cargo-added loader paths.
        env["LD_LIBRARY_PATH"] = str(sdk / "lib/lnx64.o")
        env["RDI_DATADIR"] = str(sdk / "data")
        env.pop("XILINX_VIVADO", None)
        env.pop("XILINX_VIVADO_HOMES", None)
        result = subprocess.run(
            [str(crate / "target/debug/ondas-wdb-consumer"), str(fixture)],
            env=env, cwd=crate, capture_output=True, text=True, timeout=30,
        )
        if result.returncode:
            print(result.stderr, end="")
        result.check_returncode()
        assert result.stdout == "compatible WDB queries passed\n", result.stdout
        env["RDI_DATADIR"] = str(crate / "absent-data")
        newer = (ROOT / "fixtures/wdb/wdb0002-reg-test-behav/waveform.wdb").resolve(strict=True)
        guard = subprocess.run(
            [str(crate / "target/debug/ondas-wdb-consumer"), str(newer), "guard"],
            env=env, cwd=crate, capture_output=True, text=True, timeout=30,
        )
        guard.check_returncode()
        assert guard.stdout == "WDB data selection rejected\n", guard.stdout
        print("Standalone WDB consumer read compatible observations and closed successfully.")


if __name__ == "__main__":
    main()
