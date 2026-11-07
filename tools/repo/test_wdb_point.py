"""Controlled point/range adapter checks; real SDK qualification is separate."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def compile_probe(output, definitions):
    subprocess.run([
        "g++", "-std=c++17", "-O2", "-ffunction-sections", "-fdata-sections",
        *definitions, '-DONDAS_WDB_DATA="unused"',
        '-DONDAS_WDB_READER="unused"', '-DONDAS_WDB_KERNEL="unused"',
        str(ROOT / "tools/repo/wdb_point_test.cpp"),
        "-Wl,--gc-sections", "-Wl,--export-dynamic", "-ldl", "-o", str(output),
    ], check=True)


class WdbPointTests(unittest.TestCase):
    def test_selection_lookup_cold_load_and_context_lifecycle(self):
        with tempfile.TemporaryDirectory(prefix="ondas-wdb-point-") as temporary:
            for bit_index in (0, 1):
                with self.subTest(bit_index=bit_index):
                    executable = Path(temporary) / f"point-{bit_index}"
                    compile_probe(executable, [f"-DONDAS_WDB_BIT_INDEX={bit_index}"])
                    subprocess.run([str(executable)], check=True)
                    # A fresh process without the deletion ABI must fail before creating trees.
                    subprocess.run([str(executable), "missing-delete"], check=True)
                    subprocess.run([str(executable), "parallel"], check=True)
                    subprocess.run([str(executable), "missing-capability"], check=True)
                    subprocess.run([str(executable), "omission"], check=True)
                    if bit_index == 0:
                        subprocess.run([str(executable), "task-failure"], check=True)

            # Deterministic hardware budget: 0/1 keep the serial scan; 2 reaches the real
            # worker/task paths. The controlled thread seam proves one worker was created before
            # the startup failure.
            for budget in (0, 1, 2):
                with self.subTest(budget=budget):
                    normal = Path(temporary) / f"point-budget-{budget}"
                    compile_probe(normal, [
                        "-DONDAS_WDB_BIT_INDEX=0",
                        f"-DONDAS_WDB_TEST_HARDWARE_CONCURRENCY={budget}",
                    ])
                    subprocess.run([str(normal), "parallel"], check=True)
                    subprocess.run([str(normal), "missing-capability"], check=True)
                    subprocess.run([str(normal), "omission"], check=True)
                    subprocess.run([str(normal), "task-failure"], check=True)
                    fault = Path(temporary) / f"point-start-{budget}"
                    compile_probe(fault, [
                        "-DONDAS_WDB_BIT_INDEX=0",
                        f"-DONDAS_WDB_TEST_HARDWARE_CONCURRENCY={budget}",
                        "-DONDAS_WDB_TEST_THREAD_LIMIT=1",
                    ])
                    subprocess.run([str(fault), "start-failure"], check=True)
