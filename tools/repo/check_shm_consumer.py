#!/usr/bin/env python3
"""Build and launch a SHM consumer with explicit vendor dependency lookup."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
SOURCE = r"""
fn main() {
    let path = std::path::PathBuf::from(std::env::args_os().nth(1).expect("SHM directory"));
    std::env::set_current_dir(path.parent().unwrap()).unwrap();
    let mut wave = ondas::open_with(path.file_name().unwrap(), "shm-lib").unwrap();
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    assert_eq!(wave.format(), ondas::Format::Shm);
    let step = wave.hierarchy().signal("top.step").unwrap();
    for tick in [0, 3] {
        let sample = wave.sample(step, ondas::Time::from_ticks(tick)).unwrap();
        match sample.as_ref() {
            ondas::SampleRef::Value { value: ondas::ValueRef::Bits(bits), .. } =>
                assert_eq!(bits.to_string(), format!("{tick:032b}")),
            other => panic!("unexpected step sample: {other:?}"),
        }
    }
}
"""


def main():
    fixture = (ROOT / "fixtures/shm/shm0080-history-short/waveform.shm").resolve(strict=True)
    if not fixture.is_dir():
        raise ValueError("SHM input must be a database directory")
    scratch = ROOT / "tmp"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="shm-consumer-", dir=scratch) as temporary:
        crate = Path(temporary)
        (crate / "src").mkdir()
        (crate / "src/main.rs").write_text(SOURCE)
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "ondas-shm-consumer"\nversion = "0.0.0"\nedition = "2024"\n'
            '[dependencies]\nondas = { path = ' + json.dumps(str(ROOT))
            + ', features = ["shm-lib"] }\n'
        )
        shutil.copyfile(ROOT / "Cargo.lock", crate / "Cargo.lock")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(crate / "target")
        subprocess.run(["cargo", "build", "--manifest-path", str(crate / "Cargo.toml")],
                       env=env, check=True)
        env.pop("LD_LIBRARY_PATH", None)
        env.pop("XCELIUM_HOME", None)
        reader = (Path(os.environ["XCELIUM_HOME"]) / "tools/lib/64bit/libsst2.so").resolve(strict=True)
        dynamic = subprocess.run(["readelf", "-d", str(reader)], check=True,
                                 capture_output=True, text=True).stdout
        if "(SONAME)" in dynamic:
            runtime = crate / "runtime"
            runtime.mkdir()
            (runtime / "libsst2.so").symlink_to(reader)
            env["LD_LIBRARY_PATH"] = str(runtime)
        executable = crate / "target/debug/ondas-shm-consumer"
        loaded = subprocess.run(["ldd", str(executable)], env=env, check=True,
                                capture_output=True, text=True).stdout
        resolved = [Path(line.strip().split(" => ")[-1].rsplit(" (", 1)[0]).resolve()
                    for line in loaded.splitlines()]
        if reader not in resolved:
            raise ValueError(f"consumer did not resolve the selected SHM reader: {loaded}")
        subprocess.run([str(executable), str(fixture)],
                       env=env, check=True)
    print("Standalone SHM consumer passed with the selected reader and no Cargo loader paths.")


if __name__ == "__main__":
    main()
