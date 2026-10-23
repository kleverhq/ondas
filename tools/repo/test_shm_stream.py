"""Directed merge/decoder tests; these do not qualify a vendor runtime."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ShmStreamTests(unittest.TestCase):
    def test_tick_bursts_order_errors_and_cursor_cleanup(self):
        with tempfile.TemporaryDirectory(prefix="ondas-shm-stream-") as temporary:
            executable = Path(temporary) / "stream"
            subprocess.run([
                "g++", "-std=c++17", "-O2", "-Wall", "-Wextra", "-Werror",
                "-Wno-subobject-linkage", str(ROOT / "tools/repo/shm_stream_test.cpp"),
                "-o", str(executable),
            ], check=True)
            subprocess.run([str(executable)], check=True)
