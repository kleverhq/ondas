"""Check the actual private native decoder without vendor libraries or fixtures."""
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ShmValueTests(unittest.TestCase):
    def test_quoted_values_and_malformed_transport(self):
        with tempfile.TemporaryDirectory(prefix="ondas-shm-value-") as temporary:
            executable = Path(temporary) / "decode"
            subprocess.run([
                "g++", "-std=c++17", "-Wall", "-Wextra", "-Werror",
                "-I", str(ROOT / "native"),
                str(ROOT / "tools/repo/shm_value_test.cpp"), "-o", str(executable),
            ], check=True)
            subprocess.run([str(executable)], check=True)


if __name__ == "__main__":
    unittest.main()
