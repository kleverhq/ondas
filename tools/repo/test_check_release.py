"""Release metadata checks need no network, SDK or registry credentials."""

from pathlib import Path
import tempfile
import unittest

from check_release import release_notes


class ReleaseTests(unittest.TestCase):
    def test_release_metadata_and_notes(self):
        manifest = '[package]\nname = "ondas"\nversion = "1.0.0"\n'
        lock = '[[package]]\nname = "ondas"\nversion = "1.0.0"\n'
        changelog = (
            "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- Future work.\n\n"
            "## [1.0.0] - 2026-09-21\n\n### Added\n\n- Stable API.\n\n"
            "## [0.1.0] - 2026-01-01\n\n- Older notes.\n"
            "[1.0.0]: https://example.com/release\n"
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, text in [("Cargo.toml", manifest), ("Cargo.lock", lock),
                               ("CHANGELOG.md", changelog)]:
                (root / name).write_text(text)
            expected = "### Added\n\n- Stable API.\n"
            self.assertEqual(release_notes(root), expected)
            self.assertEqual(release_notes(root, "v1.0.0"), expected)
            for tag in ["v0.1.0", "1.0.0", "v1.0.0-rc.1", "v01.0.0", ""]:
                with self.subTest(tag=tag), self.assertRaises(ValueError):
                    release_notes(root, tag)
            for name, original, invalid in [
                ("Cargo.toml", manifest, manifest.replace("1.0.0", "1.0.0-rc.1")),
                ("Cargo.lock", lock, lock.replace("1.0.0", "0.1.0")),
                ("CHANGELOG.md", changelog, changelog.replace("[1.0.0] -", "[2.0.0] -")),
                ("CHANGELOG.md", changelog, changelog.replace("2026-09-21", "2026-02-30")),
                ("CHANGELOG.md", changelog, changelog.replace("- Stable API.", "")),
                ("CHANGELOG.md", changelog, changelog + "\n## [1.0.0] - 2026-09-21\n"),
            ]:
                with self.subTest(file=name, invalid=invalid):
                    (root / name).write_text(invalid)
                    with self.assertRaises(ValueError):
                        release_notes(root)
                    (root / name).write_text(original)
            (root / "CHANGELOG.md").write_text(
                "## [1.0.0] - 2026-09-21\n\n" + expected
                + "\n[1.0.0]: https://example.com/release\n"
            )
            self.assertEqual(release_notes(root), expected)


if __name__ == "__main__":
    unittest.main()
