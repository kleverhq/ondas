"""Controlled point/range adapter checks; real SDK qualification is separate."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class WdbPointTests(unittest.TestCase):
    def test_selection_lookup_cold_load_and_context_lifecycle(self):
        with tempfile.TemporaryDirectory(prefix="ondas-wdb-point-") as temporary:
            for bit_index in (0, 1):
                with self.subTest(bit_index=bit_index):
                    executable = Path(temporary) / f"point-{bit_index}"
                    subprocess.run([
                        "g++", "-std=c++17", "-O2", "-ffunction-sections", "-fdata-sections",
                        f"-DONDAS_WDB_BIT_INDEX={bit_index}", '-DONDAS_WDB_DATA="unused"',
                        '-DONDAS_WDB_READER="unused"', '-DONDAS_WDB_KERNEL="unused"',
                        str(ROOT / "tools/repo/wdb_point_test.cpp"),
                        "-Wl,--gc-sections", "-ldl", "-o", str(executable),
                    ], check=True)
                    subprocess.run([str(executable)], check=True)
                    # A fresh process without the deletion ABI must fail before creating trees.
                    subprocess.run([str(executable), "missing-delete"], check=True)
