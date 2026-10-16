"""Unit tests for opensquilla.contrib.codetask.worktree (real git, local)."""

import json
import shutil
import subprocess
from pathlib import Path

import pytest
from typer.testing import CliRunner

from opensquilla.cli.codetask_cmd import codetask_app
from opensquilla.contrib.codetask import worktree as wt
from opensquilla.contrib.codetask.worktree import WorktreeError

cli_runner = CliRunner()


def _git(args, cwd):
    return subprocess.run(["git", *args], cwd=str(cwd), capture_output=True, text=True)


@pytest.fixture
def wt_root(tmp_path, monkeypatch):
    root = tmp_path / "wt-root"
    monkeypatch.setenv("OPENSQUILLA_CODETASK_WORKTREES_DIR", str(root))
    return root


@pytest.fixture
def source_repo(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "mod.py").write_text("def f():\n    return 1\n", encoding="utf-8")
    (src / "README.md").write_text("hello\n", encoding="utf-8")
    _git(["init", "-q", "-b", "main"], src)
    _git(["config", "user.email", "t@t"], src)
    _git(["config", "user.name", "t"], src)
    _git(["add", "-A"], src)
    _git(["commit", "-q", "-m", "init"], src)
    return src


def _commit_in(path, files: dict[str, str], message: str):
    for name, content in files.items():
        target = path / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
    _git(["add", "-A"], path)
    _git(["commit", "-q", "-m", message], path)


# ---------------------------------------------------------------------------
# isolation
# ---------------------------------------------------------------------------


def test_create_isolates_from_main_worktree(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, slug="fix")
    wt_path = Path(record.path)

    # Everything the task does inside the worktree must be invisible to the source repo.
    (wt_path / "mod.py").write_text("def f():\n    return 2\n", encoding="utf-8")
    (wt_path / "new_file.py").write_text("X = 1\n", encoding="utf-8")

    status = _git(["status", "--porcelain"], source_repo)
    assert status.stdout.strip() == ""
    assert not (source_repo / "new_file.py").exists()
    assert (source_repo / "mod.py").read_text(encoding="utf-8") == "def f():\n    return 1\n"
    # The worktree is registered and git knows it. (git prints POSIX paths
    # even on Windows — normalize before comparing.)
    listed = _git(["worktree", "list", "--porcelain"], source_repo)
    worktree_lines = [ln for ln in listed.stdout.splitlines() if ln.startswith("worktree ")]
    assert record.path.replace("\\", "/") in listed.stdout
    assert len(worktree_lines) == 2  # source repo + the new task worktree
    registry = json.loads((wt_root / "worktrees.json").read_text(encoding="utf-8"))
    assert record.task_id in registry["worktrees"]


def test_create_from_explicit_base_ref(wt_root, source_repo):
    base = _git(["rev-parse", "HEAD"], source_repo).stdout.strip()
    _git(["tag", "v1"], source_repo)
    _commit_in(source_repo, {"mod.py": "def f():\n    return 99\n"}, "second")
    head = _git(["rev-parse", "HEAD"], source_repo).stdout.strip()
    assert base != head

    record = wt.create_task_worktree(source_repo, base_ref="v1", slug="from-tag")
    assert record.base_ref == "v1"
    assert record.base_commit == base
    # The worktree checked out the base, not HEAD.
    assert Path(record.path, "mod.py").read_text(encoding="utf-8") == "def f():\n    return 1\n"


def test_create_rejects_non_repo(wt_root, tmp_path):
    plain = tmp_path / "plain"
    plain.mkdir()
    with pytest.raises(WorktreeError, match="not a git repository"):
        wt.create_task_worktree(plain)


def test_create_rejects_residue_directory(wt_root, source_repo):
    (wt_root / "fix-1").mkdir(parents=True)
    with pytest.raises(WorktreeError, match="already exists"):
        wt.create_task_worktree(source_repo, task_id="fix-1")


def test_duplicate_task_id_rejected(wt_root, source_repo):
    wt.create_task_worktree(source_repo, task_id="dup", slug="dup")
    with pytest.raises(WorktreeError, match="already registered"):
        wt.create_task_worktree(source_repo, task_id="dup")


# ---------------------------------------------------------------------------
# diff collection
# ---------------------------------------------------------------------------


def test_collect_diff_captures_committed_and_uncommitted(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="t1", slug="t")
    wt_path = Path(record.path)

    _commit_in(wt_path, {"feature.py": "VALUE = 1\n"}, "add feature")
    (wt_path / "mod.py").write_text("def f():\n    return 5\n", encoding="utf-8")  # uncommitted

    files_changed, diffstat, patch = wt.collect_task_diff(source_repo, "t1")
    assert files_changed == 2
    assert "feature.py" in patch and "mod.py" in patch
    assert "VALUE = 1" in patch and "return 5" in patch
    assert diffstat.strip()


def test_rescue_collect_after_worktree_loss(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="t2", slug="t")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"rescued.py": "SAFE = 1\n"}, "work worth rescuing")

    # Simulate a crash: the worktree dir is gone (externally removed), but git
    # kept the branch.
    shutil.rmtree(wt_path)
    _git(["worktree", "prune"], source_repo)

    files_changed, diffstat, patch = wt.collect_task_diff(source_repo, "t2")
    assert files_changed == 1
    assert "rescued.py" in patch and "SAFE = 1" in patch

    # And the work can still be merged (rescue-merge).
    result = wt.finish_task_worktree(source_repo, "t2", action="merge")
    assert result.ok and "rescue" in result.detail
    assert (source_repo / "rescued.py").exists()


def test_rescue_impossible_when_branch_also_gone(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="t3", slug="t")
    # Simulate the worst crash residue: worktree dir gone AND branch deleted.
    shutil.rmtree(Path(record.path))
    _git(["worktree", "prune"], source_repo)
    _git(["branch", "-D", "wtask/t3"], source_repo)
    with pytest.raises(WorktreeError, match="both gone"):
        wt.collect_task_diff(source_repo, "t3")


# ---------------------------------------------------------------------------
# merge closed loop + guards
# ---------------------------------------------------------------------------


def test_merge_closed_loop(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="m1", slug="m")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"feature.py": "VALUE = 1\n"}, "add feature")

    result = wt.finish_task_worktree(source_repo, "m1", action="merge")
    assert result.ok
    assert result.merge_commit

    # Source repo now contains the work, via a --no-ff merge that names the task.
    assert (source_repo / "feature.py").exists()
    log = _git(["log", "--oneline", "--merges", "-n", "1"], source_repo)
    assert "worktree(m1)" in log.stdout

    # Cleanup left no residue: worktree gone, branch gone, registry empty.
    # (git prints POSIX paths even on Windows — normalize before comparing.)
    listed = _git(["worktree", "list", "--porcelain"], source_repo)
    worktree_lines = [ln for ln in listed.stdout.splitlines() if ln.startswith("worktree ")]
    assert len(worktree_lines) == 1  # only the source repo's main worktree
    assert Path(source_repo).as_posix() in listed.stdout
    assert record.path.replace("\\", "/") not in listed.stdout
    assert "wtask/m1" not in _git(["branch", "--list"], source_repo).stdout
    assert wt.list_task_worktrees(source_repo) == []


def test_merge_refuses_dirty_main_worktree(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="m2", slug="m")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"feature.py": "V = 1\n"}, "work")

    (source_repo / "mod.py").write_text("def f():\n    return 42\n", encoding="utf-8")  # dirty!
    with pytest.raises(WorktreeError, match="uncommitted changes"):
        wt.finish_task_worktree(source_repo, "m2", action="merge")

    # Scene preserved: worktree still there, user change untouched.
    assert Path(record.path).exists()
    assert [t["task_id"] for t in wt.list_task_worktrees(source_repo)] == ["m2"]

    # Clean up the user's scratch and retry: now it merges.
    _git(["checkout", "--", "mod.py"], source_repo)
    result = wt.finish_task_worktree(source_repo, "m2", action="merge")
    assert result.ok


def test_merge_refuses_wrong_branch(wt_root, source_repo):
    wt.create_task_worktree(source_repo, task_id="m3", slug="m")
    _git(["checkout", "-q", "-b", "other"], source_repo)
    with pytest.raises(WorktreeError, match="expected 'main'"):
        wt.finish_task_worktree(source_repo, "m3", action="merge")


def test_merge_conflict_preserves_scene(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="m4", slug="m")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"mod.py": "def f():\n    return 'task-side'\n"}, "task side")

    # A conflicting change lands on the main line (still clean, still on main).
    _commit_in(source_repo, {"mod.py": "def f():\n    return 'main-side'\n"}, "main side")

    with pytest.raises(WorktreeError, match="git merge failed"):
        wt.finish_task_worktree(source_repo, "m4", action="merge")

    # Nothing deleted; the merge state is abortable by the operator.
    assert Path(record.path).exists()
    assert [t["task_id"] for t in wt.list_task_worktrees(source_repo)] == ["m4"]
    _git(["merge", "--abort"], source_repo)


# ---------------------------------------------------------------------------
# abandon
# ---------------------------------------------------------------------------


def test_abandon_keeps_diff_and_cleans(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="a1", slug="a")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"experiment.py": "TRIAL = 1\n"}, "trial")
    (wt_path / "mod.py").write_text("def f():\n    return 3\n", encoding="utf-8")

    result = wt.finish_task_worktree(source_repo, "a1", action="abandon")
    assert result.ok
    assert result.patch_path
    patch = Path(result.patch_path).read_text(encoding="utf-8")
    assert "experiment.py" in patch and "TRIAL = 1" in patch

    # Worktree + branch removed, registry empty.
    assert not Path(record.path).exists()
    assert "wtask/a1" not in _git(["branch", "--list"], source_repo).stdout
    assert wt.list_task_worktrees(source_repo) == []
    # The abandoned dir sits under the worktrees root, not in the source repo.
    assert not (source_repo / "experiment.py").exists()


def test_abandon_without_work_removes_cleanly(wt_root, source_repo):
    wt.create_task_worktree(source_repo, task_id="a2", slug="a")
    result = wt.finish_task_worktree(source_repo, "a2", action="abandon")
    assert result.ok and result.patch_path is None
    assert not (wt_root / "a2").exists()
    assert not (wt_root / "abandoned" / "a2.patch").exists()


def test_abandon_refuses_to_lose_unexportable_commits(wt_root, source_repo, monkeypatch):
    record = wt.create_task_worktree(source_repo, task_id="a3", slug="a")
    wt_path = Path(record.path)
    _commit_in(wt_path, {"x.py": "X = 1\n"}, "real work")

    # Force diff export to fail while the branch holds commits.
    def _boom(*_a, **_k):
        raise WorktreeError("simulated export failure")

    monkeypatch.setattr(wt, "collect_task_diff", _boom)
    with pytest.raises(WorktreeError, match="export failure"):
        wt.finish_task_worktree(source_repo, "a3", action="abandon")

    # Nothing was deleted: the scene is intact.
    assert Path(record.path).exists()
    assert "wtask/a3" in _git(["branch", "--list"], source_repo).stdout


# ---------------------------------------------------------------------------
# governance & security
# ---------------------------------------------------------------------------


def test_concurrency_limit_enforced(wt_root, source_repo, monkeypatch):
    monkeypatch.setenv("OPENSQUILLA_CODETASK_MAX_WORKTREES", "2")
    wt.create_task_worktree(source_repo, task_id="c1")
    wt.create_task_worktree(source_repo, task_id="c2")
    with pytest.raises(WorktreeError, match="limit reached"):
        wt.create_task_worktree(source_repo, task_id="c3")


@pytest.mark.parametrize("bad_id", ["../evil", "a/b", "with space", "..", "UPPER", "a\\b"])
def test_task_id_traversal_and_charset_rejected(wt_root, source_repo, bad_id):
    with pytest.raises(WorktreeError, match="invalid task_id"):
        wt.create_task_worktree(source_repo, task_id=bad_id)


def test_empty_task_id_autogenerates(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, slug="auto", task_id="")
    assert record.task_id.startswith("auto-")
    assert len(record.task_id) > len("auto-")


def test_prune_reclaims_crash_residue(wt_root, source_repo):
    record = wt.create_task_worktree(source_repo, task_id="p1", slug="p")
    # Crash residue: the dir vanished but the registry row survived.
    shutil.rmtree(Path(record.path))
    assert [t["task_id"] for t in wt.list_task_worktrees(source_repo)] == ["p1"]

    summary = wt.prune_task_worktrees(source_repo)
    assert summary["pruned"] == ["p1"]
    assert summary["remaining"] == []
    assert wt.list_task_worktrees(source_repo) == []


def test_registry_corruption_does_not_wedge_create(wt_root, source_repo):
    (wt_root).mkdir(parents=True, exist_ok=True)
    (wt_root / "worktrees.json").write_text("{not json", encoding="utf-8")
    record = wt.create_task_worktree(source_repo, task_id="r1")
    assert record.task_id == "r1"
    assert [t["task_id"] for t in wt.list_task_worktrees(source_repo)] == ["r1"]


def test_finish_unknown_action_rejected(wt_root, source_repo):
    wt.create_task_worktree(source_repo, task_id="u1")
    with pytest.raises(WorktreeError, match="unknown action"):
        wt.finish_task_worktree(source_repo, "u1", action="cherry-pick")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def test_cli_create_diff_finish_roundtrip(wt_root, source_repo):
    created = cli_runner.invoke(
        codetask_app,
        ["worktree", "create", "--repo", str(source_repo), "--task-id", "cli1", "--slug", "cli"],
    )
    assert created.exit_code == 0, created.output
    payload = json.loads(created.stdout)
    assert payload["task_id"] == "cli1"
    assert payload["branch"] == "wtask/cli1"

    # Real work inside the worktree, then collect via CLI.
    _commit_in(Path(payload["path"]), {"cli_feature.py": "F = 1\n"}, "work")
    patch_file = source_repo.parent / "cli1.patch"
    collected = cli_runner.invoke(
        codetask_app,
        [
            "worktree",
            "diff",
            "--repo",
            str(source_repo),
            "--task-id",
            "cli1",
            "--out",
            str(patch_file),
        ],
    )
    assert collected.exit_code == 0, collected.output
    assert json.loads(collected.stdout)["files_changed"] == 1
    assert "cli_feature.py" in patch_file.read_text(encoding="utf-8")

    finished = cli_runner.invoke(
        codetask_app,
        [
            "worktree",
            "finish",
            "--repo",
            str(source_repo),
            "--task-id",
            "cli1",
            "--action",
            "merge",
        ],
    )
    assert finished.exit_code == 0, finished.output
    result = json.loads(finished.stdout)
    assert result["ok"] and result["merge_commit"]
    assert (source_repo / "cli_feature.py").exists()

    listed = cli_runner.invoke(codetask_app, ["worktree", "list", "--repo", str(source_repo)])
    assert listed.exit_code == 0
    assert json.loads(listed.stdout)["tasks"] == []


def test_cli_finish_conflict_exits_nonzero(wt_root, source_repo):
    created = cli_runner.invoke(
        codetask_app,
        ["worktree", "create", "--repo", str(source_repo), "--task-id", "cli2"],
    )
    assert created.exit_code == 0, created.output
    payload = json.loads(created.stdout)
    _commit_in(Path(payload["path"]), {"mod.py": "def f():\n    return 'task'\n"}, "t")
    _commit_in(source_repo, {"mod.py": "def f():\n    return 'main'\n"}, "m")

    finished = cli_runner.invoke(
        codetask_app,
        [
            "worktree",
            "finish",
            "--repo",
            str(source_repo),
            "--task-id",
            "cli2",
            "--action",
            "merge",
        ],
    )
    assert finished.exit_code == 1
    assert "git merge failed" in finished.output
    _git(["merge", "--abort"], source_repo)


def test_cli_create_bad_task_id_exits_nonzero(wt_root, source_repo):
    result = cli_runner.invoke(
        codetask_app,
        ["worktree", "create", "--repo", str(source_repo), "--task-id", "../evil"],
    )
    assert result.exit_code == 1
    assert "invalid task_id" in result.output
