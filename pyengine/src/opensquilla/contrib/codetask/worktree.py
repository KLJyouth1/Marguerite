"""Git-worktree isolation for parallel code-tasks ("task = worktree").

Each parallel task gets its own ``git worktree`` on its own branch, so N tasks
can run against the same repository with zero cross-contamination: separate
working trees, indexes, and HEADs, cheap (no full clone), and individually
reviewable. Lifecycle::

    create -> run (sub-agent works inside the worktree)
           -> collect_task_diff (review)
           -> finish --action merge | abandon
           -> cleanup (worktree + branch removed; abandon keeps a .patch)

Failure semantics:
- A failed task's worktree is NEVER auto-removed (keep the scene);
  ``collect_task_diff`` can still export the diff afterwards.
- ``finish --action abandon`` exports the diff to
  ``<worktrees_root>/abandoned/<task_id>.patch`` before deleting anything, and
  REFUSES to delete the branch if the export failed while the branch holds
  commits (no silent data loss).
- ``prune_task_worktrees`` reclaims registry entries whose worktree dir
  vanished (crash residue), mirroring ``git worktree prune``.

Layout (all under ``worktrees_root()``, never inside a source repo)::

    worktrees.json            # registry: task_id -> {repo, path, branch, base...}
    <task_id>/                # the worktree checkout (git worktree add target)
    abandoned/<task_id>.patch # exported diff of abandoned tasks

SECURITY: like the rest of code-task this is NOT an OS isolation boundary.
The boundary this module does enforce: task ids are a whitelisted charset
(no traversal), the worktree path is always ``worktrees_root/<task_id>``, and
every git call targets a repo verified via ``git rev-parse --git-dir`` using
an argv list (no shell, no injection surface).
"""

from __future__ import annotations

import json
import logging
import re
import subprocess
import uuid
from dataclasses import asdict, dataclass
from datetime import UTC, datetime
from pathlib import Path

from opensquilla.contrib.codetask import workspace
from opensquilla.contrib.codetask.config import (
    GIT_USER_EMAIL,
    GIT_USER_NAME,
    max_concurrent_worktrees,
    worktrees_root,
)

logger = logging.getLogger(__name__)

WORKTREE_BRANCH_PREFIX = "wtask/"
WORKTREE_STATE_FILE = "worktrees.json"
ABANDONED_DIRNAME = "abandoned"
MAX_ABANDONED_PATCH_BYTES = 8 * 1024 * 1024  # keep abandoned patches bounded

# task_id becomes a directory name AND a branch suffix: whitelist charset,
# explicitly reject traversal ("..") so it can never escape worktrees_root.
_TASK_ID_RE = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")


class WorktreeError(RuntimeError):
    """Worktree create/collect/finish failed (guards, git, or IO)."""


@dataclass
class TaskWorktree:
    """A task's worktree and the metadata needed to finish it later."""

    task_id: str
    repo: str  # source repo (main worktree), absolute
    path: str  # worktree checkout dir, absolute
    branch: str  # wtask/<task_id>
    base_ref: str  # requested base ("HEAD" or the --base value)
    base_commit: str  # resolved base commit at create time
    main_branch: str  # branch the source repo was on at create time
    created_at: str  # ISO-8601 UTC


@dataclass
class FinishResult:
    """Outcome of ``finish_task_worktree``."""

    task_id: str
    action: str  # merge | abandon
    ok: bool
    detail: str = ""
    patch_path: str | None = None  # abandoned diff kept for rollback
    merge_commit: str | None = None
    cleaned: bool = False


# ---------------------------------------------------------------------------
# git helpers
# ---------------------------------------------------------------------------


def _git(args: list[str], cwd: Path, timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        cwd=str(cwd),
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
    )


def _current_branch(path: Path) -> str:
    r = _git(["rev-parse", "--abbrev-ref", "HEAD"], path)
    return r.stdout.strip() if r.returncode == 0 else ""


def _branch_exists(repo: Path, branch: str) -> bool:
    r = _git(["rev-parse", "--verify", "--quiet", f"refs/heads/{branch}"], repo)
    return r.returncode == 0


def _validate_repo(repo: str | Path) -> Path:
    """Resolve and verify that ``repo`` is a git repository (main worktree)."""
    p = Path(repo).expanduser()
    if not p.exists():
        raise WorktreeError(f"repo path does not exist: {p}")
    p = p.resolve()
    if not p.is_dir():
        raise WorktreeError(f"repo path is not a directory: {p}")
    r = _git(["rev-parse", "--git-dir"], p)
    if r.returncode != 0:
        raise WorktreeError(f"not a git repository: {p}")
    return p


def _validate_task_id(task_id: str) -> str:
    tid = task_id.strip()
    if not _TASK_ID_RE.match(tid) or ".." in tid:
        raise WorktreeError(
            f"invalid task_id {task_id!r}: expected 1-64 chars of "
            "[a-z0-9._-] (no '..', no path separators)"
        )
    return tid


# ---------------------------------------------------------------------------
# registry (worktrees.json) — domain record for rescue/merge metadata
# ---------------------------------------------------------------------------


def _state_file() -> Path:
    return worktrees_root() / WORKTREE_STATE_FILE


def _load_registry() -> dict[str, dict]:
    """Load the registry; a missing/corrupt file degrades to an empty table.

    The git worktrees themselves and their branches survive registry loss, so
    a corrupt table must not wedge create/finish.
    """
    f = _state_file()
    if not f.exists():
        return {}
    try:
        data = json.loads(f.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        logger.warning("worktree registry unreadable (%s); treating as empty", exc)
        return {}
    entries = data.get("worktrees") if isinstance(data, dict) else None
    return entries if isinstance(entries, dict) else {}


def _save_registry(entries: dict[str, dict]) -> None:
    root = worktrees_root()
    root.mkdir(parents=True, exist_ok=True)
    tmp = root / (WORKTREE_STATE_FILE + ".tmp")
    tmp.write_text(
        json.dumps({"version": 1, "worktrees": entries}, indent=2, ensure_ascii=False),
        encoding="utf-8",
    )
    tmp.replace(_state_file())  # atomic within the same volume


def _entry_to_worktree(entry: dict) -> TaskWorktree:
    fields = TaskWorktree.__dataclass_fields__
    return TaskWorktree(**{k: entry[k] for k in fields if k in entry})


# ---------------------------------------------------------------------------
# create
# ---------------------------------------------------------------------------


def _generate_task_id(slug: str) -> str:
    from opensquilla.contrib.codetask.config import slugify

    base = slugify(slug)[:24] or "task"
    return f"{base}-{uuid.uuid4().hex[:8]}"


def create_task_worktree(
    repo: str | Path,
    *,
    task_id: str | None = None,
    base_ref: str | None = None,
    slug: str = "task",
) -> TaskWorktree:
    """Create an isolated git worktree + branch for a parallel task.

    Guards (fail-closed): repo is a git dir; task_id charset (path-traversal
    proof); per-repo concurrency cap; task_id not already registered; worktree
    dir free; base ref resolves. The main worktree does NOT need to be clean
    (we branch from a commit, not from the working state).
    """
    repo_path = _validate_repo(repo)
    entries = _load_registry()

    active = sum(1 for e in entries.values() if e.get("repo") == str(repo_path))
    cap = max_concurrent_worktrees()
    if active >= cap:
        raise WorktreeError(
            f"concurrent worktree limit reached ({cap}) for {repo_path}; "
            "finish or prune existing tasks first"
        )

    tid = _validate_task_id(task_id or _generate_task_id(slug))
    if tid in entries:
        raise WorktreeError(
            f"task_id {tid!r} already registered for {repo_path}; "
            "pick another id, or finish/prune the old task"
        )

    base = base_ref or "HEAD"
    r = _git(["rev-parse", "--verify", f"{base}^{{commit}}"], repo_path)
    if r.returncode != 0:
        raise WorktreeError(f"base ref does not resolve to a commit: {base!r}")
    base_commit = r.stdout.strip()

    branch = f"{WORKTREE_BRANCH_PREFIX}{tid}"
    attempt = 2
    while _branch_exists(repo_path, branch):
        branch = f"{WORKTREE_BRANCH_PREFIX}{tid}-{attempt}"
        attempt += 1
        if attempt > 100:
            raise WorktreeError("could not find a free branch name for task")

    path = worktrees_root() / tid
    if path.exists():
        raise WorktreeError(
            f"worktree path already exists: {path} (residue of an earlier "
            "task; prune or pick another task id)"
        )

    add = _git(
        ["worktree", "add", "-b", branch, str(path), base_commit],
        repo_path,
        timeout=300,
    )
    if add.returncode != 0:
        raise WorktreeError(f"git worktree add failed: {(add.stderr or '').strip()[-400:]}")

    # Local identity only — never touches the user's global git config.
    _git(["config", "user.email", GIT_USER_EMAIL], path)
    _git(["config", "user.name", GIT_USER_NAME], path)

    wt = TaskWorktree(
        task_id=tid,
        repo=str(repo_path),
        path=str(path),
        branch=branch,
        base_ref=base,
        base_commit=base_commit,
        main_branch=_current_branch(repo_path) or base_commit,
        created_at=datetime.now(UTC).isoformat(),
    )
    entries[tid] = {**asdict(wt), "state": "active"}
    _save_registry(entries)
    logger.info(
        "Created task worktree %s @ %s (branch %s, base %s)",
        tid,
        path,
        branch,
        base_commit[:12],
    )
    return wt


# ---------------------------------------------------------------------------
# read / collect
# ---------------------------------------------------------------------------


def get_task_worktree(repo: str | Path, task_id: str) -> TaskWorktree:
    """Fetch a registered task worktree (does not require the dir to exist)."""
    repo_path = _validate_repo(repo)
    tid = _validate_task_id(task_id)
    entry = _load_registry().get(tid)
    if entry is None or entry.get("repo") != str(repo_path):
        raise WorktreeError(f"unknown task_id {tid!r} for repo {repo_path}")
    return _entry_to_worktree(entry)


def collect_task_diff(repo: str | Path, task_id: str) -> tuple[int, str, str]:
    """Return (files_changed, diffstat, patch) for a task, vs its base commit.

    Live mode (worktree dir present) reuses ``workspace.collect_change`` —
    ``git add -A`` then diff, so uncommitted AND committed work is captured.
    Rescue mode (dir gone, branch alive) diffs ``base_commit..branch``
    directly: a crashed or externally-removed task is still recoverable.
    """
    wt = get_task_worktree(repo, task_id)
    repo_path = Path(wt.repo)
    if Path(wt.path).exists():
        return workspace.collect_change(Path(wt.path), wt.base_commit)

    if not _branch_exists(repo_path, wt.branch):
        raise WorktreeError(
            f"worktree {wt.path} and branch {wt.branch} are both gone; nothing left to collect"
        )
    r = _git(["diff", "--binary", "--no-color", wt.base_commit, wt.branch], repo_path)
    if r.returncode != 0:
        raise WorktreeError(f"rescue diff failed: {(r.stderr or '').strip()[-300:]}")
    patch = r.stdout
    stat = _git(["diff", "--stat", wt.base_commit, wt.branch], repo_path)
    names = _git(["diff", "--name-only", wt.base_commit, wt.branch], repo_path)
    diffstat = stat.stdout.strip() if stat.returncode == 0 else ""
    files_changed = len([n for n in names.stdout.splitlines() if n.strip()])
    return files_changed, diffstat, patch


# ---------------------------------------------------------------------------
# finish: merge / abandon
# ---------------------------------------------------------------------------


def _branch_has_new_commits(repo: Path, base_commit: str, branch: str) -> bool:
    r = _git(["rev-list", "--count", f"{base_commit}..{branch}"], repo)
    try:
        return int(r.stdout.strip() or "0") > 0
    except ValueError:
        return False


def _cleanup_worktree(repo_path: Path, wt: TaskWorktree) -> bool:
    """Remove the worktree dir (force, prune-retry) and delete its branch.

    Registry entry is dropped regardless; returns whether the dir removal
    actually succeeded.
    """
    ok = True
    if Path(wt.path).exists():
        r = _git(["worktree", "remove", "--force", str(wt.path)], repo_path)
        if r.returncode != 0:
            # A stale/locked worktree (dead process handle) needs a prune
            # first; one retry, then report honestly.
            _git(["worktree", "prune"], repo_path)
            r = _git(["worktree", "remove", "--force", str(wt.path)], repo_path)
        ok = r.returncode == 0
        if not ok:
            logger.warning(
                "worktree remove failed for %s: %s", wt.path, (r.stderr or "").strip()[-200:]
            )
    _git(["branch", "-D", wt.branch], repo_path)  # already-deleted branch: ignore rc
    entries = _load_registry()
    entries.pop(wt.task_id, None)
    _save_registry(entries)
    return ok


def _merge_worktree(repo_path: Path, wt: TaskWorktree, cleanup: bool) -> FinishResult:
    # Guard A: the main worktree must still be on the branch it was on at
    # create time — git refuses merging into a branch checked out elsewhere,
    # and we must never silently merge onto a different branch.
    current = _current_branch(repo_path)
    if current != wt.main_branch:
        raise WorktreeError(
            f"source repo is on branch {current!r}, expected {wt.main_branch!r} "
            "(the branch at task-create time); switch back and retry"
        )
    # Guard B: never merge over the user's uncommitted work.
    if workspace.is_dirty(repo_path):
        raise WorktreeError(
            "source repo has uncommitted changes; commit or stash them "
            "before merging a task worktree"
        )
    if not _branch_exists(repo_path, wt.branch):
        raise WorktreeError(
            f"branch {wt.branch} is gone; use collect_task_diff to recover "
            "whatever remains and merge manually"
        )

    live = Path(wt.path).exists()
    # The merge itself works from the branch alone; a lost worktree dir
    # (rescue-merge) is fine because the branch has no checkout anymore.
    m = _git(
        ["merge", "--no-ff", wt.branch, "-m", f"worktree({wt.task_id}): merge task branch"],
        repo_path,
        timeout=600,
    )
    if m.returncode != 0:
        raise WorktreeError(
            f"git merge failed (task worktree preserved): "
            f"{(m.stderr or m.stdout or '').strip()[-400:]}"
        )
    merge_commit = _git(["rev-parse", "HEAD"], repo_path).stdout.strip()
    cleaned = False
    if cleanup:
        cleaned = _cleanup_worktree(repo_path, wt)
    detail = "merged via live worktree" if live else "merged via rescue (worktree dir was gone)"
    return FinishResult(
        task_id=wt.task_id,
        action="merge",
        ok=True,
        detail=detail,
        merge_commit=merge_commit,
        cleaned=cleaned,
    )


def _abandon_worktree(repo_path: Path, wt: TaskWorktree) -> FinishResult:
    # Export the diff FIRST, fail-closed on loss: if the branch holds commits
    # we could not save, refuse to delete anything (keep the scene).
    patch_path: Path | None = None
    has_commits = _branch_has_new_commits(repo_path, wt.base_commit, wt.branch)
    try:
        _, _, patch = collect_task_diff(repo_path, wt.task_id)
        if patch.strip():
            dest_dir = worktrees_root() / ABANDONED_DIRNAME
            dest_dir.mkdir(parents=True, exist_ok=True)
            candidate = dest_dir / f"{wt.task_id}.patch"
            if len(patch.encode("utf-8")) > MAX_ABANDONED_PATCH_BYTES:
                if has_commits:
                    raise WorktreeError(
                        "abandoned diff exceeds "
                        f"{MAX_ABANDONED_PATCH_BYTES} bytes and was not saved; "
                        "branch and worktree kept — export manually before abandoning"
                    )
                logger.warning("abandoned diff for %s too large to keep", wt.task_id)
            else:
                candidate.write_text(patch, encoding="utf-8")
                patch_path = candidate
        elif has_commits:
            # Commits on the branch but no textual diff: pathological (e.g.
            # merge-only branch) — keep everything rather than lose history.
            raise WorktreeError(
                "branch has commits but produced an empty diff; "
                "branch and worktree kept — inspect manually before abandoning"
            )
    except WorktreeError:
        if has_commits:
            raise  # never abandon away commits we could not export
        logger.warning("abandon diff export failed for %s (no commits to lose)", wt.task_id)

    cleaned = _cleanup_worktree(repo_path, wt)
    detail = (
        f"worktree and branch removed; diff kept at {patch_path}"
        if patch_path
        else "worktree and branch removed (nothing to keep)"
    )
    return FinishResult(
        task_id=wt.task_id,
        action="abandon",
        ok=True,
        detail=detail,
        patch_path=str(patch_path) if patch_path else None,
        cleaned=cleaned,
    )


def finish_task_worktree(
    repo: str | Path,
    task_id: str,
    *,
    action: str = "merge",
    cleanup: bool = True,
) -> FinishResult:
    """Finish a task: merge its branch into the main line, or abandon it.

    merge  — guarded (on-branch + clean main worktree), --no-ff, conflicts
             abort with the worktree preserved; cleanup removes worktree +
             branch afterwards (unless cleanup=False).
    abandon — exports the diff to abandoned/<task_id>.patch first, then
             removes worktree + branch. Refuses if commits could not be
             exported (no silent data loss).
    """
    if action not in ("merge", "abandon"):
        raise WorktreeError(f"unknown action {action!r}; expected 'merge' or 'abandon'")
    repo_path = _validate_repo(repo)
    wt = get_task_worktree(repo_path, task_id)
    if action == "merge":
        return _merge_worktree(repo_path, wt, cleanup=cleanup)
    return _abandon_worktree(repo_path, wt)


# ---------------------------------------------------------------------------
# list / prune
# ---------------------------------------------------------------------------


def list_task_worktrees(repo: str | Path) -> list[dict]:
    """Registered tasks for a repo, with live status, oldest first."""
    repo_path = _validate_repo(repo)
    out: list[dict] = []
    for tid, e in _load_registry().items():
        if e.get("repo") != str(repo_path):
            continue
        out.append(
            {
                "task_id": tid,
                "branch": e.get("branch"),
                "base_commit": (e.get("base_commit") or "")[:12],
                "state": e.get("state", "active"),
                "worktree_present": bool(e.get("path")) and Path(str(e["path"])).exists(),
                "path": e.get("path"),
                "main_branch": e.get("main_branch"),
                "created_at": e.get("created_at"),
            }
        )
    out.sort(key=lambda d: str(d.get("created_at") or ""))
    return out


def prune_task_worktrees(repo: str | Path) -> dict:
    """Reclaim crash residue: git-level prune + drop registry rows whose
    worktree dir no longer exists. Branches are left alone (they are the
    rescue surface)."""
    repo_path = _validate_repo(repo)
    _git(["worktree", "prune"], repo_path)
    entries = _load_registry()
    dropped = [
        tid
        for tid, e in entries.items()
        if e.get("repo") == str(repo_path) and not (e.get("path") and Path(str(e["path"])).exists())
    ]
    for tid in dropped:
        entries.pop(tid, None)
    _save_registry(entries)
    remaining = [tid for tid, e in entries.items() if e.get("repo") == str(repo_path)]
    return {"pruned": dropped, "remaining": remaining}
