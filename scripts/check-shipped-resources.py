#!/usr/bin/env python3
"""What actually lands in the installer, checked before it is built.

`scripts/package_full.cmd` stages three trees into `gui/src-tauri` and
`tauri.conf.json` puts all three into `bundle.resources`. `webui/dist` has its
own verifier, but the staged copies are plain `xcopy` output: a stale tree, or a
file the frontend verifier never sees, ships anyway. Measured on the staged tree
behind the published 0.2.17 MSI: 103 `*.map` files in each console copy, 23 MB of
internal `docs/`, the repository's own `AGENTS.md`, and 599 `__pycache__` dirs.

Exit codes follow the repository convention: 0 clean, 1 violations, 2 cannot
verify (a `[SKIP]` is never a pass).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TAURI_ROOT = REPO_ROOT / "gui" / "src-tauri"

# One definition of "what the engine sources hash to", shared with the compiler that
# writes the stamp. If this import ever breaks, `repo_source_fingerprint()` returns
# None and the native rule fails closed — a guard that cannot check is not a pass.
sys.path.insert(0, str(Path(__file__).resolve().parent))
try:
    from compile_pyengine_native import ENGINE_SOURCE_ROOT, source_fingerprint
except ImportError:
    ENGINE_SOURCE_ROOT = None
    source_fingerprint = None

# The three trees `bundle.resources` declares, plus the pyengine Control UI copy.
STAGED_ROOTS = [
    TAURI_ROOT / "webui" / "dist",
    TAURI_ROOT / "resources",
    TAURI_ROOT / "skills",
]

SOURCE_MAP_SUFFIXES = {".map"}
SOURCE_MAP_KEYS = {".js", ".mjs", ".json", ".css", ".html"}
SOURCE_MAP_PAYLOAD = re.compile(rb"sourcesContent")

# Contributor/agent-facing files at the resources root. `AGENTS.md` is the worst
# of them: it quotes the leaked provider key and the whole internal gate story,
# and the published 0.2.17 MSI put 134 KB of it on every user's disk.
BANNED_RESOURCE_ROOT_FILES = {
    "AGENTS.md",
    "Cargo.lock",
    "config.yaml",
    "rust-toolchain.toml",
}

# `core/src/doctor/appliance.rs` requires a `docs` directory to exist, so the
# tree stays — but only these self-contained user manuals ship. Everything else
# under docs/ is phase reports, acceptance docs, QA screenshots and research.
SHIPPED_DOCS = {
    "cli-reference.md",
    "commands-and-modes-reference.md",
    "goal.md",
    "heavy-runtime-guide.md",
    "mcp.md",
    "memory.md",
    "ollama-local-models-e2e.md",
    "overview.md",
    "skills.md",
}

BANNED_PATH_NAMES = {"__pycache__", ".pytest_cache", ".DS_Store"}
BANNED_SUFFIXES = {".pyc", ".pyo"}

# The bundled interpreter is the sidecar's first-choice runtime
# (`core/src/pyengine/sidecar.rs`: `<exe>/resources/python/python.exe`). It is not
# in git — `scripts/fetch_python_runtime.ps1` materializes it — so nothing else
# fails when it is missing: the installer installs, the app opens, and the Python
# engine silently degrades to a Rust-only capability subset. Measured 2026-10-28:
# `package_full.cmd` step 4 wiped the whole `resources/` tree on every run, which
# deleted it, and no step re-created it.
REQUIRED_INTERPRETER = ("python", "python.exe")

# ---------------------------------------------------------------------------
# Native payload: the Python engine must not ship as readable source.
#
# The Rust core is already machine code. The sidecar was not: measured 2026-10-28
# on the staged tree behind the published installer, 1,017 plaintext `.py` /
# 17.1 MiB of first-party source sat in `bundle.resources`, i.e. every user got a
# readable copy of the engine. `scripts/compile_pyengine_native.py` turns those
# into cp312 `.pyd` and writes `pyengine/NATIVE.json` recording what it compiled
# and what it deliberately left as source. This rule is the other half: a manifest
# that claims a module is native while the `.py` is still on disk, or a plaintext
# module nobody declared, is a violation. No manifest at all means the installer
# is shipping source, which is the thing being prevented — so fail closed.
# ---------------------------------------------------------------------------
NATIVE_MANIFEST = ("pyengine", "NATIVE.json")
PYENGINE_PKG = ("pyengine", "src", "opensquilla")
NATIVE_SUFFIXES = (".pyd", ".so")
# `NATIVE.json` keys both sides by the suffix-free module path — `engine/runtime`,
# never `engine/runtime.py` or `engine/runtime.cp312-win_amd64.pyd` — so one
# canonical spelling is comparable in both directions.


def module_key(rel: Path) -> str:
    """`gateway/config.cp312-win_amd64.pyd` -> `gateway/config`.

    Split on the first dot of the **file name**, not of the path: the ABI tag only
    ever lives in the name, while a directory may legitimately contain one
    (`squilla_router/models/v4.2_phase3_inference/...`). Splitting the whole path
    truncated that key to `squilla_router/models/v4` and every module under such a
    directory reported as undeclared once they started compiling.
    """
    stem = rel.name.split(".")[0]
    parent = rel.parent.as_posix()
    return stem if parent in ("", ".") else f"{parent}/{stem}"


def staged_native(pkg: Path, module: str) -> bool:
    for suffix in NATIVE_SUFFIXES:
        if pkg.joinpath(f"{module}{suffix}").is_file():
            return True
        if list(pkg.glob(f"{module}.*{suffix}")):
            return True
    return False


def source_map_violations(root: Path, rel_prefix: str) -> list[str]:
    found: list[str] = []
    for kind, path in walk(root):
        rel = f"{rel_prefix}/{path.relative_to(root).as_posix()}"
        if kind == "symlink":
            found.append(f"{rel}: symlink in a staged tree")
            continue
        if path.suffix.lower() in SOURCE_MAP_SUFFIXES:
            found.append(f"{rel}: source map shipped to users")
            continue
        if path.suffix.lower() in SOURCE_MAP_KEYS:
            try:
                body = path.read_bytes()
            except OSError as error:  # unreadable = cannot verify, not clean
                found.append(f"{rel}: unreadable ({error})")
                continue
            if SOURCE_MAP_PAYLOAD.search(body):
                found.append(f"{rel}: inlined source map payload")
    return found


def internal_content_violations(resources: Path) -> list[str]:
    found: list[str] = []
    for _, path in walk(resources):
        rel = path.relative_to(resources)
        name = rel.as_posix()
        if len(rel.parts) == 1 and name in BANNED_RESOURCE_ROOT_FILES:
            found.append(f"resources/{name}: contributor-facing file shipped to users")
        if rel.parts[:1] == ("docs",):
            doc = Path(*rel.parts[1:]).as_posix()
            if doc not in SHIPPED_DOCS:
                found.append(f"resources/docs/{doc}: internal documentation shipped")
    return found


def machine_state_violations(root: Path, rel_prefix: str) -> list[str]:
    found: list[str] = []
    for _, path in walk(root):
        parts = path.relative_to(root).parts
        # `resources/python/` is a vendored interpreter distribution (13,085 of
        # the 13,105 `.pyc` files live there). Its caches are upstream's build
        # output, not this project's machine state.
        if rel_prefix == "resources" and parts[:1] == ("python",):
            continue
        if any(part in BANNED_PATH_NAMES for part in parts[:-1]):
            continue  # counted once at the directory level below
        if path.name in BANNED_PATH_NAMES:
            found.append(f"{rel_prefix}/{path.relative_to(root).as_posix()}: machine state")
        elif path.suffix.lower() in BANNED_SUFFIXES:
            found.append(f"{rel_prefix}/{path.relative_to(root).as_posix()}: bytecode cache")
    for directory in sorted(root.rglob("*")):
        if rel_prefix == "resources" and directory.relative_to(root).parts[:1] == ("python",):
            continue
        if directory.is_dir() and directory.name in BANNED_PATH_NAMES:
            found.append(f"{rel_prefix}/{directory.relative_to(root).as_posix()}/: cache directory")
    return found


def walk(root: Path):
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            yield ("symlink", path)
        elif path.is_file():
            yield ("file", path)


def runtime_violations(resources: Path) -> list[str]:
    interpreter = resources.joinpath(*REQUIRED_INTERPRETER)
    if interpreter.is_file():
        return []
    return [
        f"resources/{'/'.join(REQUIRED_INTERPRETER)}: bundled interpreter missing — "
        "the sidecar silently degrades to Rust-only. Run scripts/fetch_python_runtime.ps1."
    ]


def repo_source_fingerprint() -> str | None:
    """Content digest of the engine sources as they stand in the repository.

    Returns None when it cannot be computed, which the caller must treat as
    cannot-verify rather than as a match.
    """
    if source_fingerprint is None or ENGINE_SOURCE_ROOT is None:
        return None
    if not ENGINE_SOURCE_ROOT.is_dir():
        return None
    return source_fingerprint(ENGINE_SOURCE_ROOT)


def native_violations(resources: Path) -> list[str]:
    """Cross-check `pyengine/NATIVE.json` against the files actually staged."""
    import json

    pkg = resources.joinpath(*PYENGINE_PKG)
    if not pkg.is_dir():
        return []  # no Python engine in this tree at all

    manifest_path = resources.joinpath(*NATIVE_MANIFEST)
    if not manifest_path.is_file():
        plaintext = sum(1 for _ in pkg.rglob("*.py"))
        native = sum(1 for _ in pkg.rglob("*") if _.suffix.lower() in NATIVE_SUFFIXES)
        if not plaintext and not native:
            return []  # engine absent from this tree — nothing to protect
        return [
            f"resources/{'/'.join(NATIVE_MANIFEST)}: missing while {plaintext} plaintext "
            f".py / {native} native are staged — the installer ships readable engine "
            "source. Run scripts/compile_pyengine_native.py before packaging."
        ]

    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return [f"resources/{'/'.join(NATIVE_MANIFEST)}: unreadable ({error})"]

    compiled = manifest.get("compiled")
    excluded = manifest.get("excluded")
    if not isinstance(compiled, list) or not isinstance(excluded, dict):
        return [
            f"resources/{'/'.join(NATIVE_MANIFEST)}: schema invalid — needs "
            "'compiled' (list) and 'excluded' (path -> reason object)"
        ]

    found: list[str] = []
    compiled_set = {str(item).replace("\\", "/") for item in compiled}

    # `build.rs` stops re-staging `resources/pyengine` as soon as `NATIVE.json`
    # exists (it must — the IDE's periodic `cargo check` wiped a finished native
    # tree 23 seconds after it landed on 2026-10-28). The marker therefore creates
    # the opposite hazard: a hardened tree that outlives the sources it was
    # compiled from, which the installer would ship as if it were current. The
    # stamped digest is what makes that observable instead of silent.
    if manifest.get("building"):
        return [
            f"resources/{'/'.join(NATIVE_MANIFEST)}: a native build claimed this tree "
            "and was never finalized — the compile died mid-flight. Re-run "
            "scripts/compile_pyengine_native.py; a half-converted tree must not ship."
        ]

    stamp = manifest.get("sourceFingerprint")
    if not isinstance(stamp, str) or not stamp:
        found.append(
            f"resources/{'/'.join(NATIVE_MANIFEST)}: no sourceFingerprint — the "
            "manifest cannot prove it describes the sources that are in the "
            "repository. Re-run scripts/compile_pyengine_native.py."
        )
    else:
        current = repo_source_fingerprint()
        if current is None:
            found.append(
                f"resources/{'/'.join(NATIVE_MANIFEST)}: stamped {stamp}, but the "
                "repository engine sources are unavailable here, so the stamp cannot "
                "be verified — this is cannot-verify, not a pass."
            )
        elif current != stamp:
            found.append(
                f"resources/{'/'.join(NATIVE_MANIFEST)}: stale — the native tree was "
                f"compiled from sources digest {stamp}, but pyengine/src/opensquilla "
                f"is now {current}. Re-run scripts/compile_pyengine_native.py."
            )

    # A module the toolchain refused is still plaintext, but the compiler wrote
    # down why. That is a disclosed gap, so it is declared — the summary prints
    # the count so a spike cannot hide inside the accepted set.
    failed = manifest.get("compileFailures")
    declared = set(excluded)
    if isinstance(failed, dict):
        declared |= {str(key).replace("\\", "/") for key in failed}
    for path in sorted(pkg.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(pkg).as_posix()
        if path.suffix.lower() == ".py":
            # The manifest keys modules without a suffix (`engine/runtime`), so a
            # staged `engine/runtime.py` must be compared by the same key.
            key = rel[: -len(".py")]
            if key in compiled_set:
                found.append(
                    f"resources/{'/'.join(PYENGINE_PKG)}/{rel}: declared compiled but "
                    "the plaintext source is still staged"
                )
            elif key not in declared:
                found.append(
                    f"resources/{'/'.join(PYENGINE_PKG)}/{rel}: own plaintext module "
                    "shipped to users, absent from the native manifest"
                )
        elif path.suffix.lower() in NATIVE_SUFFIXES:
            key = module_key(path.relative_to(pkg))
            if key not in compiled_set:
                found.append(
                    f"resources/{'/'.join(PYENGINE_PKG)}/{rel}: native module not declared "
                    "in the manifest"
                )

    for rel in sorted(compiled_set):
        if not staged_native(pkg, rel):
            found.append(
                f"resources/{'/'.join(PYENGINE_PKG)}/{rel}: manifest claims compiled, "
                "no .pyd/.so is staged"
            )
    return found


def native_summary(roots: list[tuple[str, Path]]) -> list[str]:
    """Disclose the composition of the engine payload on the passing path.

    "No violations" alone would let a release ship with almost nothing compiled:
    the rules accept any plaintext the manifest explains, so the size of the
    remaining plaintext has to be printed, not merely be checkable.
    """
    import json

    lines: list[str] = []
    for _, root in roots:
        manifest_path = root.joinpath(*NATIVE_MANIFEST)
        if not manifest_path.is_file():
            continue
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        compiled = manifest.get("compiled")
        failures = manifest.get("compileFailures")
        excluded = manifest.get("excluded")
        if not isinstance(compiled, list):
            continue
        pkg = root.joinpath(*PYENGINE_PKG)
        plaintext = sum(1 for _ in pkg.rglob("*.py"))
        bytes_left = sum(f.stat().st_size for f in pkg.rglob("*.py"))
        lines.append(
            f"  native: {len(compiled)} compiled, {len(excluded) if isinstance(excluded, dict) else 0} "
            f"declared, {len(failures) if isinstance(failures, dict) else 0} build failure(s); "
            f"{plaintext} .py still staged ({bytes_left:,} bytes)"
        )
    return lines


def violations(roots: list[tuple[str, Path]]) -> list[str]:
    found: list[str] = []
    for prefix, root in roots:
        if not root.is_dir():
            continue
        found.extend(source_map_violations(root, prefix))
        found.extend(machine_state_violations(root, prefix))
        if prefix == "resources":
            resources = root
            found.extend(internal_content_violations(resources))
            found.extend(runtime_violations(resources))
            found.extend(native_violations(resources))
    return found


def self_test() -> int:
    """Prove every rule actually fires. A hygiene guard that has never rejected
    anything is indistinguishable from one that cannot reject anything."""
    import shutil
    import tempfile

    checks: list[tuple[str, bool, bool, list[str], list[str]]] = []
    with tempfile.TemporaryDirectory(prefix="shipped-resources-selftest-") as tmp:
        base = Path(tmp) / "src-tauri"
        resources = base / "resources"
        (resources / "docs").mkdir(parents=True)
        (resources / "CHANGELOG.md").write_text("# ok\n", encoding="utf-8")
        (resources / "AGENTS.md").write_text("# internal\n", encoding="utf-8")
        (resources / "docs" / "overview.md").write_text("# manual\n", encoding="utf-8")
        (resources / "docs" / "M3_产品化_验收报告.md").write_text("# internal\n", encoding="utf-8")
        dist = resources / "pyengine" / "src" / "opensquilla" / "gateway" / "static" / "dist"
        dist.mkdir(parents=True)
        (dist / "index.html").write_text("<html></html>", encoding="utf-8")
        (dist / "app.js").write_text("console.log(1)\n//# sourceMappingURL=app.js.map\n", encoding="utf-8")
        (dist / "app.js.map").write_text('{"sourcesContent":["const a = 1"]}', encoding="utf-8")
        (dist / "bundle.js").write_text('var x;//# sourceMap={"sourcesContent":["secret"]}\n', encoding="utf-8")
        cache = resources / "pyengine" / "src" / "opensquilla" / "__pycache__"
        cache.mkdir(parents=True)
        (cache / "boot.cpython-312.pyc").write_bytes(b"\x00" * 8)
        # A stray `.pyc` outside a cache dir is the only shape that reaches the
        # "bytecode cache" reason: entries under `__pycache__` are deliberately
        # reported once, at directory level, not per file.
        (resources / "pyengine" / "leaked.pyc").write_bytes(b"\x00" * 8)

        # 1. Everything dirty: each rule must name its own cause.
        dirty = violations([("resources", resources)])
        expectations = {
            "source map shipped to users": "app.js.map",
            "inlined source map payload": "bundle.js",
            "contributor-facing file shipped": "AGENTS.md",
            "internal documentation shipped": "M3_",
            "cache directory": "__pycache__",
            "bytecode cache": "leaked.pyc",
            "bundled interpreter missing": "python.exe",
        }
        for reason, needle in expectations.items():
            hits = [item for item in dirty if reason in item and needle in item]
            checks.append((f"detects {reason}", True, bool(hits), hits, dirty))

        # 2. Clean tree: no rule may fire.
        shutil.rmtree(cache)
        (resources / "pyengine" / "leaked.pyc").unlink()
        for extra in (dist / "app.js.map", dist / "bundle.js"):
            extra.unlink()
        (dist / "app.js").write_text("console.log(1)\n", encoding="utf-8")
        (resources / "AGENTS.md").unlink()
        (resources / "docs" / "M3_产品化_验收报告.md").unlink()
        (resources / "python").mkdir()
        (resources / "python" / "python.exe").write_bytes(b"MZ")
        clean = violations([("resources", resources)])
        checks.append(("clean tree reports nothing", False, not clean, [], clean))

        # 3. Native payload: each failure mode of the anti-decompilation rule.
        import json

        engine = resources / "pyengine" / "src" / "opensquilla" / "engine"
        engine.mkdir(parents=True)
        (engine / "runtime.py").write_text("def f():\n    return 1\n", encoding="utf-8")
        (engine / "__init__.py").write_text("", encoding="utf-8")
        native = [v for v in violations([("resources", resources)]) if "NATIVE.json" in v]
        checks.append(("detects missing native manifest", True, bool(native), native, native))

        manifest = resources / "pyengine" / "NATIVE.json"
        current = repo_source_fingerprint()
        # (a) declared compiled, but the .py is still there and no .pyd exists.
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime"],
                    "excluded": {"engine/__init__": "package stub"},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )
        reported = violations([("resources", resources)])
        stale = [v for v in reported if "declared compiled but the plaintext" in v]
        checks.append(("detects source left beside a compiled claim", True, bool(stale), stale, reported))
        missing_bin = [v for v in reported if "claims compiled, no .pyd/.so" in v]
        checks.append(("detects compiled claim without a binary", True, bool(missing_bin), missing_bin, reported))

        # (b) source gone, binary still missing.
        (engine / "runtime.py").unlink()
        reported = [v for v in violations([("resources", resources)]) if "claims compiled" in v]
        checks.append(
            ("still detects the missing binary alone", True, bool(reported), reported, reported)
        )

        # (c) the honest end state: .pyd present, .py gone, __init__ excluded.
        (engine / "runtime.pyd").write_bytes(b"MZ")
        clean = violations([("resources", resources)])
        checks.append(("native tree reports nothing", False, not clean, [], clean))

        # (c2) a directory name may contain a dot. `module_key` strips ABI tags from
        # the *file name*; splitting the whole path at the first dot turned
        # `v4.2_probe/leaf` into `v4` and reported a correctly declared module as
        # undeclared (2026-10-29: 13 modules under `v4.2_phase3_inference`).
        dotted = resources / "pyengine" / "src" / "opensquilla" / "v4.2_probe"
        dotted.mkdir(parents=True)
        (dotted / "leaf.pyd").write_bytes(b"MZ")
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime", "v4.2_probe/leaf"],
                    "excluded": {"engine/__init__": "package stub"},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )
        dotted_found = [v for v in violations([("resources", resources)]) if "leaf.pyd" in v]
        checks.append(
            (
                "accepts a compiled module under a dotted directory",
                False,
                not dotted_found,
                [],
                dotted_found,
            )
        )
        shutil.rmtree(dotted)
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime"],
                    "excluded": {"engine/__init__": "package stub"},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )

        # (d) one undeclared plaintext module smuggled back in.
        (engine / "extra.py").write_text("SECRET = 1\n", encoding="utf-8")
        undeclared = [
            v for v in violations([("resources", resources)]) if "absent from the native manifest" in v
        ]
        checks.append(("detects undeclared plaintext module", True, bool(undeclared), undeclared, undeclared))

        # (e) the same module, now *declared* as a toolchain failure. It is still
        # plaintext, but the compiler recorded why, so this is a disclosed gap and
        # not a silent one — the rule must stop firing for it.
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime"],
                    "excluded": {"engine/__init__": "package stub"},
                    "compileFailures": {"engine/extra": "cython: unsupported syntax at line 3"},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )
        after_declare = [v for v in violations([("resources", resources)]) if "extra.py" in v]
        checks.append(
            (
                "accepts a disclosed compile failure as declared",
                False,
                not after_declare,
                [],
                after_declare,
            )
        )

        # (f) `build.rs` stops re-staging the moment `NATIVE.json` exists, so the
        # native tree can outlive the sources it was compiled from. The manifest's
        # fingerprint is what turns that silent lie into a violation.
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime"],
                    "excluded": {"engine/__init__": "package stub"},
                    "compileFailures": {"engine/extra": "cython: unsupported syntax at line 3"},
                    "sourceFingerprint": "0123456789abcdef",
                }
            ),
            encoding="utf-8",
        )
        stale_tree = [v for v in violations([("resources", resources)]) if "stale" in v]
        checks.append(
            (
                "detects a native tree older than its sources",
                True,
                bool(stale_tree),
                stale_tree,
                violations([("resources", resources)]),
            )
        )

        # (g) the same tree, honestly stamped: the only difference is the digest.
        manifest.write_text(
            json.dumps(
                {
                    "compiled": ["engine/runtime"],
                    "excluded": {"engine/__init__": "package stub"},
                    "compileFailures": {"engine/extra": "cython: unsupported syntax at line 3"},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )
        fresh = violations([("resources", resources)])
        checks.append(
            ("accepts a native tree stamped with the current sources", False, not fresh, [], fresh)
        )

        # (h) a manifest that predates stamping proves nothing about its tree.
        manifest.write_text(
            json.dumps({"compiled": ["engine/runtime"], "excluded": {"engine/__init__": "package stub"}}),
            encoding="utf-8",
        )
        unstamped = [v for v in violations([("resources", resources)]) if "sourceFingerprint" in v]
        checks.append(
            ("detects a manifest with no source fingerprint", True, bool(unstamped), unstamped, unstamped)
        )

        # (i) a compile that died mid-flight leaves its claim marker behind, and the
        # half-converted tree must never ship.
        manifest.write_text(
            json.dumps(
                {
                    "building": True,
                    "compiled": [],
                    "excluded": {},
                    "sourceFingerprint": current,
                }
            ),
            encoding="utf-8",
        )
        inflight = [v for v in violations([("resources", resources)]) if "never finalized" in v]
        checks.append(
            ("detects an in-flight native build claim", True, bool(inflight), inflight, inflight)
        )

    failed = 0
    for label, should_fire, fired, hits, actual in checks:
        if should_fire and not fired:
            print(
                f"[SELFTEST FAIL] {label}: rule never fired "
                f"({len(actual)} violation(s) reported, none matching)",
                file=sys.stderr,
            )
            failed += 1
        elif not should_fire and actual:
            print(f"[SELFTEST FAIL] {label}: {len(actual)} unexpected violation(s)", file=sys.stderr)
            for item in actual[:5]:
                print(f"    - {item}", file=sys.stderr)
            failed += 1
        else:
            print(f"[SELFTEST OK]   {label}")
    return 1 if failed else 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()

    # `--root` scans a materialized tree instead of the staged one. `gui/src-tauri/build.rs`
    # re-stages pyengine and the Control UI copy during `cargo tauri build`, *after* any
    # pre-build check, so only the tree under `target/release/resources` is evidence of
    # what the installer actually carried.
    extra_roots = [Path(value) for flag, value in zip(argv, argv[1:]) if flag == "--root"]
    roots: list[tuple[str, Path]] = [(root.relative_to(TAURI_ROOT).as_posix(), root) for root in STAGED_ROOTS]
    roots.extend(("resources", root) for root in extra_roots)

    present = [root for _, root in roots if root.is_dir()]
    if not present:
        print(
            "check-shipped-resources: NO staged bundle resources found under "
            f"{TAURI_ROOT.relative_to(REPO_ROOT)} — nothing to verify. "
            "Run scripts/package_full.cmd first (release gate only).",
            file=sys.stderr,
        )
        return 2

    found = violations(roots)
    if found:
        by_reason: dict[str, int] = {}
        for item in found:
            reason = item.split(": ", 1)[1]
            by_reason[reason] = by_reason.get(reason, 0) + 1
        print(f"check-shipped-resources: {len(found)} violation(s):", file=sys.stderr)
        for reason, count in sorted(by_reason.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:6d}  {reason}", file=sys.stderr)
        examples = [item for item in found if "source map" not in item.split(": ", 1)[1]]
        for item in examples[:20]:
            print(f"  - {item}", file=sys.stderr)
        if len(examples) > 20:
            print(f"  ... {len(examples) - 20} more non-map violations", file=sys.stderr)
        return 1

    print(
        "check-shipped-resources: staged bundle resources carry no shipped source "
        f"({len(present)} tree(s) checked)"
    )
    for line in native_summary(roots):
        print(line)
    return 0


if __name__ == "__main__":
    # Violation paths carry CJK filenames; a cp936 console cannot encode them, and a
    # reporter that crashes mid-report would turn a real finding into a silent pass.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(errors="backslashreplace")
    sys.exit(main(sys.argv[1:]))
