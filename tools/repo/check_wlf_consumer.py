#!/usr/bin/env python3
"""Build and directly launch a separate WLF consumer, without Cargo loader paths."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = r"""
fn main() {
    let path = std::env::args_os().nth(1).expect("WLF path");
    let mut wave = ondas::open_with(path, "wlf-lib").unwrap();
    assert_eq!(wave.format(), ondas::Format::Wlf);
    let signal = wave.hierarchy().signals()
        .find(|s| s.encoding() != ondas::Encoding::Unsupported).unwrap();
    wave.sample(signal, ondas::Time::from_ticks(0)).unwrap();
}
"""


def main():
    fixture = (
        ROOT / "fixtures/wlf/wlf0038-count4b-3-2-vsim/waveform.wlf"
    ).resolve(strict=True)
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wlf-consumer-", dir=scratch) as temporary:
        crate = Path(temporary)
        (crate / "src").mkdir()
        (crate / "src/main.rs").write_text(SOURCE)
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "ondas-wlf-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
            '[dependencies]\nondas = { path = ' + json.dumps(str(ROOT))
            + ', features = ["wlf-lib"] }\n'
        )
        # Retain the repository's dependency versions; Cargo adds the smoke crate.
        shutil.copyfile(ROOT / "Cargo.lock", crate / "Cargo.lock")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(crate / "target")
        subprocess.run(["cargo", "build", "--manifest-path", str(crate / "Cargo.toml")],
                       env=env, check=True)
        binary = crate / "target/debug/ondas-wlf-consumer"
        dependencies = subprocess.check_output(["readelf", "-d", str(binary)], text=True)
        vendor = str((Path(env["QUESTA_HOME"]) / "linux_x86_64/libwlf.so").resolve(strict=True))
        if f"Shared library: [{vendor}]" not in dependencies:
            raise RuntimeError("WLF vendor dependency did not propagate to the consumer")
        if "libondas_wlf" in dependencies:
            raise RuntimeError("consumer requires a separately installed Ondas shim")
        env.pop("LD_LIBRARY_PATH", None)
        env.pop("QUESTA_HOME", None)
        subprocess.run([str(binary), str(fixture)],
                       env=env, check=True)
    print("Standalone WLF consumer passed without Cargo loader environment.")


if __name__ == "__main__":
    main()
