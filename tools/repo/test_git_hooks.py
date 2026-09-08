import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]


class GitHookTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.tmp = Path(self.temp.name)
        self.home = self.tmp / "home"
        self.home.mkdir()
        self.main = self.tmp / "main"
        self.linked = self.tmp / "linked"
        self.fake_bin = self.tmp / "bin"
        self.fake_bin.mkdir()
        self.log = self.tmp / "calls.jsonl"
        for name in ("docker", "devcontainer"):
            script = self.fake_bin / name
            script.write_text("#!/bin/sh\nprintf 'unexpected container command: %s\\n' \"$0\" >&2\nexit 98\n")
            script.chmod(0o755)
        self._write_fake_pre_commit()
        self._init_repository()

    def _env(self, **updates: str) -> dict[str, str]:
        env = {
            name: value for name, value in os.environ.items()
            if not name.startswith(("GIT_", "ONDAS_"))
        }
        env.update(
            HOME=str(self.home), XDG_CONFIG_HOME=str(self.home / ".config"),
            GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
            PATH=f"{self.fake_bin}{os.pathsep}{env['PATH']}", FAKE_LOG=str(self.log),
        )
        env.update(updates)
        return env

    def _git(self, root: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *args], cwd=root, env=env or self._env(), text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True,
        )

    def _init_repository(self) -> None:
        self.main.mkdir()
        self._git(self.main, "init", "-q", "-b", "main")
        self._git(self.main, "config", "user.name", "Test User")
        self._git(self.main, "config", "user.email", "test@example.com")
        (self.main / "tools/repo").mkdir(parents=True)
        for relative in ("dev", "tools/repo/git-hook"):
            target = self.main / relative
            target.write_bytes((REPO_ROOT / relative).read_bytes())
            target.chmod(0o755)
        (self.main / ".devcontainer").mkdir()
        (self.main / ".devcontainer/devcontainer.json").write_text("{}\n")
        (self.main / "tracked").write_text("base\n")
        self._git(self.main, "add", ".")
        self._git(self.main, "commit", "-qm", "test: initial")
        self._git(self.main, "worktree", "add", "-qb", "linked", str(self.linked))

    def _write_fake_pre_commit(self) -> None:
        script = self.fake_bin / "pre-commit"
        script.write_text(
            """#!/usr/bin/env python3
import json, os, subprocess, sys
record = {"args": sys.argv[1:], "git_env": {k: v for k, v in os.environ.items() if k.startswith("GIT_")}}
if os.environ.get("PROBE_INDEX"):
    record["staged"] = subprocess.run(["git", "show", ":tracked"], check=True, capture_output=True, text=True).stdout
with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(record) + "\\n")
raise SystemExit(int(os.environ.get("FAKE_STATUS", "0")))
"""
        )
        script.chmod(0o755)

    def _install(self, root: Path) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(root / "dev"), "--install-hooks"], cwd=root, env=self._env(),
            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        )

    def _hooks_dir(self, root: Path) -> Path:
        return Path(self._git(root, "config", "--worktree", "--get", "core.hooksPath").stdout.strip())

    def _replace_dev(self, root: Path) -> None:
        installed = self._hooks_dir(root) / "dev"
        installed.write_text(
            """#!/usr/bin/env python3
import json, os, pathlib, sys
args = sys.argv[1:]
assert args.pop(0) == "--exec-only" and args.pop(0) == "env"
assignments = {}
while args and "=" in args[0]:
    key, value = args.pop(0).split("=", 1); assignments[key] = value
workspace = "/workspaces/" + pathlib.Path(os.environ["FAKE_ROOT"]).name
with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps({"dev_env": assignments, "args": args, "host_git_env": {
        k: v for k, v in os.environ.items() if k.startswith("GIT_")
    }}) + "\\n")
def host(value):
    return os.environ["FAKE_ROOT"] + value[len(workspace):] if value == workspace or value.startswith(workspace + "/") else value
env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
env.update({key: host(value) for key, value in assignments.items()})
os.execvpe(args[0], [host(value) for value in args], env)
"""
        )
        installed.chmod(0o755)

    def _run_hook(self, root: Path, **env_updates: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(self._hooks_dir(root) / "pre-commit")], cwd=root,
            env=self._env(FAKE_ROOT=str(root), **env_updates),
            text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        )

    def _calls(self) -> list[dict[str, object]]:
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_install_is_idempotent_and_worktree_local(self) -> None:
        for root in (self.main, self.linked):
            with self.subTest(root=root):
                result = self._install(root)
                self.assertEqual(result.returncode, 0, result.stderr)
                git_dir = Path(self._git(root, "rev-parse", "--absolute-git-dir").stdout.strip())
                hooks = self._hooks_dir(root)
                self.assertEqual(hooks, git_dir / "ondas-hooks")
                self.assertEqual(self._git(root, "config", "--get", "extensions.worktreeConfig").stdout.strip(), "true")
                self.assertFalse((hooks / "commit-msg").exists())
                for source, destination in (("dev", "dev"), ("tools/repo/git-hook", "pre-commit")):
                    self.assertEqual((hooks / destination).read_bytes(), (root / source).read_bytes())
                    self.assertFalse((hooks / destination).is_symlink())
                    self.assertTrue(os.access(hooks / destination, os.X_OK))
                result = self._install(root)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self._hooks_dir(root), hooks)
                if root == self.main:
                    result = subprocess.run(
                        ["git", "config", "--get", "core.hooksPath"], cwd=self.linked,
                        env=self._env(), capture_output=True, text=True,
                    )
                    self.assertEqual(result.returncode, 1, result.stdout)
        self.assertNotEqual(self._hooks_dir(self.main), self._hooks_dir(self.linked))

    def test_refuses_unrelated_effective_hooks_path(self) -> None:
        self._git(self.main, "config", "core.hooksPath", "/custom/shared-hooks")
        result = self._install(self.linked)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to replace", result.stderr)
        self.assertEqual(self._git(self.linked, "config", "--get", "core.hooksPath").stdout.strip(), "/custom/shared-hooks")
        self._git(self.main, "config", "--local", "--unset", "core.hooksPath")
        self.assertEqual(self._install(self.main).returncode, 0)
        self.assertEqual(self._install(self.linked).returncode, 0)
        main_hooks = self._hooks_dir(self.main)
        self._git(self.linked, "config", "--worktree", "core.hooksPath", "/custom/worktree-hooks")
        result = self._install(self.linked)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to replace", result.stderr)
        self.assertEqual(self._hooks_dir(self.linked), Path("/custom/worktree-hooks"))
        self.assertEqual(self._hooks_dir(self.main), main_hooks)

    def test_maps_main_and_linked_paths_and_preserves_exact_staged_index(self) -> None:
        for root in (self.main, self.linked):
            for alternate in (False, True):
                with self.subTest(root=root, alternate=alternate):
                    self.log.unlink(missing_ok=True)
                    result = self._install(root)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self._replace_dev(root)
                    common = Path(self._git(root, "rev-parse", "--path-format=absolute", "--git-common-dir").stdout.strip())
                    git_dir = Path(self._git(root, "rev-parse", "--absolute-git-dir").stdout.strip())
                    index = git_dir / "index"
                    updates = {}
                    if alternate:
                        index = common / "temporary index"
                        index.write_bytes((git_dir / "index").read_bytes())
                        updates["GIT_INDEX_FILE"] = str(index.relative_to(root)) if root == self.main else str(index)
                    staged = "alternate staged\n" if alternate else "default staged\n"
                    (root / "tracked").write_text(staged)
                    self._git(root, "add", "tracked", env=self._env(**updates))
                    (root / "tracked").write_text("unstaged working copy\n")
                    result = self._run_hook(
                        root, **updates, GIT_OBJECT_DIRECTORY=str(common / "objects"),
                        GIT_TRACE="0", PROBE_INDEX="1",
                    )
                    self.assertEqual(result.returncode, 0, result.stderr)
                    dev_call, call = self._calls()
                    workspace = f"/workspaces/{root.name}"
                    expected = {
                        "GIT_WORK_TREE": workspace,
                        "GIT_DIR": f"{workspace}/.git" if root == self.main else str(git_dir),
                        "GIT_COMMON_DIR": f"{workspace}/.git" if root == self.main else str(common),
                        "GIT_INDEX_FILE": f"{workspace}/{index.relative_to(root)}" if root == self.main else str(index),
                    }
                    self.assertEqual(dev_call["dev_env"], expected)
                    self.assertEqual(dev_call["host_git_env"], {})
                    self.assertEqual(call["staged"], staged)
                    self.assertEqual(call["git_env"]["GIT_INDEX_FILE"], str(index))
                    self.assertEqual(set(call["git_env"]), set(expected))
                    self.assertEqual(dev_call["args"], ["pre-commit", "run", "--hook-stage", "pre-commit"])
                    self.assertEqual(call["args"], ["run", "--hook-stage", "pre-commit"])

    def test_rejects_index_outside_mounted_roots(self) -> None:
        for root in (self.main, self.linked):
            with self.subTest(root=root):
                self.assertEqual(self._install(root).returncode, 0)
                self._replace_dev(root)
                result = self._run_hook(root, GIT_INDEX_FILE=str(self.tmp / "outside index"))
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("outside the mounted worktree", result.stderr)
                self.assertEqual(self._calls(), [])

    def test_installed_copies_survive_tracked_edits_and_return_failure(self) -> None:
        self.assertEqual(self._install(self.main).returncode, 0)
        hooks = self._hooks_dir(self.main)
        copies = {name: (hooks / name).read_bytes() for name in ("dev", "pre-commit")}
        for relative in ("dev", "tools/repo/git-hook"):
            (self.main / relative).write_text("#!/bin/sh\nexit 99\n")
        for name, contents in copies.items():
            self.assertEqual((hooks / name).read_bytes(), contents)
        self._replace_dev(self.main)
        result = self._run_hook(self.main, FAKE_STATUS="23")
        self.assertEqual(result.returncode, 23, result.stderr)
        self.assertEqual(self._calls()[1]["args"], ["run", "--hook-stage", "pre-commit"])

    def test_exec_only_never_starts_or_recreates_containers(self) -> None:
        # Capture the real launcher's labels instead of duplicating its fingerprint algorithm.
        mock = """#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
log_path = pathlib.Path(os.environ["FAKE_LOG"])
previous = [json.loads(line) for line in log_path.read_text().splitlines()] if log_path.exists() else []
with log_path.open("a") as log:
    log.write(json.dumps({"tool": name, "args": args}) + "\\n")
state = os.environ.get("FAKE_CONTAINER", "none")
if name == "devcontainer":
    raise SystemExit(0)
if args[0] == "info":
    raise SystemExit(0)
if args[0] == "ps":
    if state == "running" or (state == "stopped" and any("a" in arg for arg in args[1:] if arg.startswith("-") and not arg.startswith("--"))):
        print("test-container")
elif args[0] == "inspect":
    if "State.Running" in args[2]:
        print("true" if state == "running" else "false")
    else:
        assert "dev.ondas.config" in args[2], args
        launch = next(call for call in previous if call["tool"] == "devcontainer" and call["args"][0] == "up")
        print(next(arg.split("=", 1)[1] for arg in launch["args"] if arg.startswith("dev.ondas.config=")))
else:
    raise SystemExit("unexpected docker command: " + repr(args))
"""
        for name in ("docker", "devcontainer"):
            script = self.fake_bin / name
            script.write_text(mock)
            script.chmod(0o755)
        result = subprocess.run(
            [str(self.main / "dev"), "true"], cwd=self.main, env=self._env(),
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self._install(self.main).returncode, 0)
        installed_dev = self._hooks_dir(self.main) / "dev"
        (self.main / "dev").write_text("#!/bin/sh\nexit 99\n")
        for state in ("running", "none", "stopped"):
            with self.subTest(state=state):
                before = len(self._calls())
                result = subprocess.run(
                    [str(installed_dev), "--exec-only", "true"], cwd=self.main,
                    env=self._env(FAKE_CONTAINER=state), capture_output=True, text=True,
                )
                calls = self._calls()[before:]
                cli_calls = [call["args"][0] for call in calls if call["tool"] == "devcontainer"]
                if state == "running":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(cli_calls, ["exec"])
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertEqual(cli_calls, [])
                self.assertTrue(all(call["args"][0] in ("info", "ps", "inspect") for call in calls if call["tool"] == "docker"))


if __name__ == "__main__":
    unittest.main()
