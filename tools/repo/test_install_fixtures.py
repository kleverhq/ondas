import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import install_fixtures as installer


class FixtureInstallTests(unittest.TestCase):
    def test_pinned_clone_and_cached_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root / "lock.toml"
            lock.write_text('[providers]\n"kleverhq.ondas-fixtures" = "4.1.2"\n')
            target = root / installer.PROVIDER

            def run(command, **kwargs):
                if command[1] == "clone":
                    self.assertIn("v4.1.2", command)
                    target.mkdir()
                    (target / "catalog.json").write_text(json.dumps({"provider": installer.PROVIDER, "version": "4.1.2"}))

            with patch.object(installer.subprocess, "run", side_effect=run) as execute, patch.object(installer.subprocess, "check_output", return_value="commit\n"):
                installer.install(root, lock)
                self.assertEqual(execute.call_args_list[-1].args[0][-1], str(target / "install.py"))
                execute.reset_mock()
                installer.install(root, lock)
                self.assertFalse(any(call.args[0][1] == "clone" for call in execute.call_args_list))
                self.assertEqual(execute.call_count, 2)  # clean tree + installer, even on cache hit

            with patch.object(installer.subprocess, "check_output", side_effect=["other", "pinned"]), patch.object(installer.subprocess, "run") as execute:
                with self.assertRaisesRegex(ValueError, "refusing"):
                    installer.install(root, lock)
                execute.assert_not_called()

            (target / "catalog.json").write_text(json.dumps({"provider": installer.PROVIDER, "version": "wrong"}))
            with patch.object(installer.subprocess, "check_output", return_value="commit"), patch.object(installer.subprocess, "run") as execute:
                with self.assertRaisesRegex(ValueError, "catalog"):
                    installer.install(root, lock)
                self.assertEqual(execute.call_count, 1)
