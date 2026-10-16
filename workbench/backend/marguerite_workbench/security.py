"""Path-safety boundary, mirroring Marguerite's ``core/src/tools/fs_path.rs``.

``safe_resolve`` turns an arbitrary (relative *or* absolute) path into a concrete
path that is **guaranteed** to live inside the workspace root.  It rejects:

* ``..`` traversal that would escape the root,
* absolute paths that point outside the root,
* symlink escapes (the real target of any symlink must stay inside root).

This is the single choke-point every file operation in the workbench passes
through — there is no other way to turn user-supplied path strings into real
filesystem access.
"""

from __future__ import annotations

from pathlib import Path

# Anti R5: named constants instead of bare literals.
SYMLINK_ESCAPE_MSG = "path escapes workspace via symlink: {!r}"
TRAVERSAL_ESCAPE_MSG = "path escapes workspace: {!r}"
EMPTY_PATH_MSG = "empty path is not allowed"


def safe_resolve(root: str | Path, rel: str | Path) -> Path:
    """Resolve ``rel`` against ``root`` and assert it stays inside ``root``.

    Returns the resolved :class:`pathlib.Path` (symlinks followed).  Raises
    ``ValueError`` when the path would escape the workspace.
    """
    if rel is None or str(rel).strip() == "":
        raise ValueError(EMPTY_PATH_MSG)

    root_p = Path(root).resolve()
    rel_p = Path(rel)

    # Absolute user paths are allowed ONLY when they already fall inside the
    # workspace root (mirrors the Rust component-stack normalisation where an
    # absolute rel collapses to itself and is then prefix-checked).
    if rel_p.is_absolute():
        candidate = rel_p.resolve()
    else:
        candidate = (root_p / rel_p).resolve()

    # Follow symlinks so a symlinked directory/file whose *real* target lives
    # outside the root is still caught as an escape.
    candidate = candidate.resolve()

    try:
        # Raises ValueError if candidate is not within root_p.
        candidate.relative_to(root_p)
    except ValueError:
        # Distinguish the two escape classes for clearer diagnostics.
        if candidate.is_symlink() or any(p.is_symlink() for p in _iter_parents(candidate)):
            raise ValueError(SYMLINK_ESCAPE_MSG.format(str(rel)))
        raise ValueError(TRAVERSAL_ESCAPE_MSG.format(str(rel)))

    return candidate


def _iter_parents(path: Path):
    """Yield each ancestor directory of ``path`` (exclusive of the root)."""
    for parent in path.parents:
        yield parent
