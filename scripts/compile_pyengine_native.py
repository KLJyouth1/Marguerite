#!/usr/bin/env python3
"""Turn the staged Python engine into machine code, in place, before packaging.

Why this exists: the Rust core ships as native code, but the sidecar shipped as
1,017 plaintext `.py` / 17.1 MiB of first-party source inside `bundle.resources`
(measured 2026-10-28 on the staged tree behind the published 0.2.17 installer).
Every user therefore received a readable copy of the engine. This script compiles
each module with Cython and links it with MSVC against the **bundled** runtime's
headers and import library, so the `.pyd` matches the interpreter that actually
ships, then removes the source it replaced.

The tree it operates on is the *staged* copy under
`gui/src-tauri/resources/pyengine`, never `pyengine/` in the repository —
development keeps editing readable source; only the installer payload is hardened.

Two behaviors were proven here before compiling anything, because the whole design
depends on them (`pkgutil` channel discovery and 20+ `inspect.signature` sites):
a compiled submodule is still enumerated by `pkgutil.iter_modules`, and Cython 3.3
keeps `inspect.signature` working, including defaults, `*args`/`**kwargs` and
keyword-only parameters.

Not everything may become native, and each exclusion is derived, not guessed:
  - `__init__.py`            package stubs: `__path__`, `importlib.resources.files`
  - `cli/main.py`            the sidecar is started as `python -m opensquilla.cli.main`
                             (`core/src/pyengine/sidecar.rs`), and runpy cannot execute
                             an extension module
  - `skills/bundled/*/scripts/*.py`  invoked as files (`uv run python scripts/lint.py`)
  - self/sibling source readers, found by scanning each module for
    `Path(__file__).read_text()`, `with_name("*.py")`, `glob("*.py")`
  - modules that fail to transpile or compile, recorded with the toolchain's own message

`pyengine/migrations/V*.py` is deliberately untouched: `persistence/migrator.py:76`
and `recovery/engine.py:572` enumerate migrations with `glob("V*.py")` and hand the
source to yoyo.

Exit codes follow the repository convention: 0 success, 1 real failure, 2 cannot
verify (missing toolchain or runtime) — a `[SKIP]` is never a pass.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

for stream in (sys.stdout, sys.stderr):
    if hasattr(stream, "reconfigure"):
        stream.reconfigure(errors="backslashreplace")

REPO_ROOT = Path(__file__).resolve().parent.parent
STAGED = REPO_ROOT / "gui" / "src-tauri" / "resources"
PKG_SUBPATH = Path("pyengine") / "src" / "opensquilla"
MANIFEST_SUBPATH = Path("pyengine") / "NATIVE.json"
# The staged package and the repository source are the same relative layout, so one
# constant defines both. `check-shipped-resources.py` imports these two names rather
# than spelling the path again — a stamp computed over a different tree than the one
# being verified would fail every build and teach people to ignore the guard.
ENGINE_SOURCE_ROOT = REPO_ROOT / PKG_SUBPATH

# --- hazard patterns that make a module unsafe to compile -------------------
SELF_SOURCE = re.compile(r"Path\(\s*__file__\s*\)\s*(?:\.resolve\(\))?\s*\.\s*(read_text|read_bytes)")
SIBLING_SOURCE = re.compile(r"with_name\(\s*[\"']([^\"']+\.py)[\"']\s*\)")
GLOB_SOURCE = re.compile(r"\.(glob|rglob|iglob)\(\s*[\"'][^\"']*\.py[\"']")

RUNPY_ENTRY = "cli/main.py"
# `binding=True` makes compiled methods `cython_function_or_method` objects, which
# implement the descriptor protocol like real Python functions. Without it they are
# bare `cyfunction`s, and pydantic v2 refuses to build the class: the 2026-10-28
# verification pass found 554 modules unimportable, all reporting
# `A non-annotated attribute was detected: to_public_dict = <cyfunction
# SandboxPolicy.to_public_dict ...>` — raised once in the class that defines the
# method and cascading to every importer of it.
CYTHON_FLAGS = ["-3", "-X", "binding=True", "-X", "annotation_typing=False"]
# `annotation_typing=False` 是 2026-10-29 实测补的：Cython 默认按注解做类型校验，
# 而 typer 的**遗留默认值写法**恰好把 `typer.Option(...)`（OptionInfo）放在
# `days: int` 这类注解上。编译后模块在 import 期即抛
# `TypeError: Expected int, got OptionInfo`（实测：cli/bundle_cmd.py:52、
# cli/codetask_cmd.py:71），侧车 `-m opensquilla.cli.main` 直接起不来 ——
# 表现是网关降级为纯 Rust、config.get/onboarding.* 全 501。
# 关掉注解校验只影响编译期类型收窄（优化与运行时类型检查），不改语义；
# 真正的类型契约仍由 pytest 与导入校验兜底。
# Modules that glob `*.py` from disk but whose glob target provably lies outside
# the compiled tree, so their own file type cannot affect discovery. Each entry was
# read, not inferred: `persistence/migrator.py:103` derives
# `Path(__file__).resolve().parents[3] / "migrations"` = `pyengine/migrations`,
# which this script never touches (and which `recovery/engine.py:572` enumerates as
# `V*.py` regardless). Anything else that globs `.py` falls through to the
# conservative rule and is reported, so a new shape cannot silently join this list.
SAFE_SOURCE_GLOBS = {
    "migration/opensquilla_home.py": "globs pyengine/migrations (parents[3]), outside this tree",
    "persistence/migrator.py": "globs pyengine/migrations (parents[3]), outside this tree",
    "recovery/engine.py": "globs pyengine/migrations (parents[3]), outside this tree",
}
MSVC_VCVARS_CANDIDATES = [
    r"C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Auxiliary/Build/vcvars64.bat",
    r"C:/Program Files/Microsoft Visual Studio/2022/Community/VC/Auxiliary/Build/vcvars64.bat",
    r"C:/Program Files/Microsoft Visual Studio/2022/Enterprise/VC/Auxiliary/Build/vcvars64.bat",
]
VSWHERE = Path(r"C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe")


def fail(message: str, code: int = 1) -> int:
    print(f"[FAIL] {message}", file=sys.stderr)
    return code


def cannot_verify(message: str) -> int:
    return fail(message, 2)


def env_lookup(env: dict[str, str], name: str) -> str:
    """Windows env blocks are case-insensitive; `set` output is not."""
    for key, value in env.items():
        if key.upper() == name.upper():
            return value
    return ""


def env_prepend(env: dict[str, str], name: str, value: str) -> None:
    """Prepend to an env var, reusing whatever spelling the block already uses."""
    actual = next((key for key in env if key.upper() == name.upper()), name)
    env[actual] = os.pathsep.join([value, env.get(actual, "")]).rstrip(os.pathsep)


def run(cmd, *, env=None, shell=False, timeout=600, cwd=None):
    proc = subprocess.run(
        cmd,
        shell=shell,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        env=env,
        cwd=cwd,
        timeout=timeout,
    )
    out = (proc.stdout + proc.stderr).decode("utf-8", "replace")
    return proc.returncode, out


def find_vcvars() -> Path | None:
    if VSWHERE.is_file():
        rc, out = run([str(VSWHERE), "-latest", "-products", "*", "-property", "installationPath"])
        if rc == 0:
            root = out.strip().splitlines()[0] if out.strip() else ""
            if root:
                candidate = Path(root) / "VC" / "Auxiliary" / "Build" / "vcvars64.bat"
                if candidate.is_file():
                    return candidate
    for candidate in MSVC_VCVARS_CANDIDATES:
        if Path(candidate).is_file():
            return Path(candidate)
    return None


def msvc_env(vcvars: Path) -> tuple[dict[str, str], dict[str, str]]:
    """Capture vcvars64's environment and tool paths once.

    Re-invoking `call vcvars64.bat` per module costs several seconds each; with
    ~900 modules that alone would dominate the build.

    `cl`/`link` are returned as absolute paths on purpose: CreateProcessW resolves
    the application name against the *calling* process's PATH, so passing vcvars'
    PATH in `env=` alone makes `["cl", ...]` fail with WinError 2.
    """
    rc, out = run(f'call "{vcvars}" >nul && set', shell=True, timeout=300)
    if rc != 0:
        raise RuntimeError(f"vcvars64 failed rc={rc}: {out[-400:]}")
    env = dict(os.environ)
    for line in out.splitlines():
        if "=" in line:
            key, _, value = line.partition("=")
            env[key] = value
    tools: dict[str, str] = {}
    for directory in env_lookup(env, "PATH").split(os.pathsep):
        for tool in ("cl", "link"):
            if tool in tools or not directory:
                continue
            candidate = Path(directory) / f"{tool}.exe"
            if candidate.is_file():
                tools[tool] = str(candidate)
    missing = [tool for tool in ("cl", "link") if tool not in tools]
    if missing:
        raise RuntimeError(f"vcvars64 PATH has no {'/'.join(missing)}.exe — compiler not installed")
    return env, tools


def cython_env(root: Path) -> tuple[Path, str]:
    """A throwaway venv holding Cython — the shipped runtime must not gain a compiler."""
    venv = Path(tempfile.gettempdir()) / "marguerite-cython-env"
    cython = venv / "Scripts" / "cython.exe"
    if not cython.is_file():
        venv.parent.mkdir(parents=True, exist_ok=True)
        if venv.exists():
            shutil.rmtree(venv, ignore_errors=True)
        uv = shutil.which("uv")
        bootstrap = (
            [uv, "venv", "--python", sys.executable, str(venv)]
            if uv
            else [sys.executable, "-m", "venv", str(venv)]
        )
        rc, out = run(bootstrap, timeout=600)
        if rc:
            raise RuntimeError(f"venv creation failed: {out[-400:]}")
        install = (
            [uv, "pip", "install", "--python", str(venv / "Scripts" / "python.exe"), "cython"]
            if uv
            else [str(venv / "Scripts" / "python.exe"), "-m", "pip", "install", "cython"]
        )
        rc, out = run(install, timeout=900)
        if rc:
            raise RuntimeError(f"cython install failed: {out[-400:]}")
    rc, out = run([str(cython), "--version"])
    return cython, out.strip() or "unknown"


def source_hazards(text: str) -> list[str]:
    reasons = []
    if SELF_SOURCE.search(text):
        reasons.append("reads its own __file__ as text at runtime")
    for match in SIBLING_SOURCE.finditer(text):
        reasons.append(f"reads sibling source {match.group(1)} via with_name()")
    for match in GLOB_SOURCE.finditer(text):
        reasons.append(f"enumerates {match.group(1)}('*.py') on disk")
    return reasons


def classify(pkg: Path) -> tuple[list[Path], dict[str, str], dict[str, str]]:
    """Split the staged modules into compile targets, declared exclusions and notes.

    Exclusion keys are suffix-free module paths (`engine/runtime`), the same
    canonical form `scripts/check-shipped-resources.py` compares against.
    """
    targets: list[Path] = []
    excluded: dict[str, str] = {}
    notes: dict[str, str] = {}
    for path in sorted(pkg.rglob("*.py")):
        rel = path.relative_to(pkg).as_posix()
        key = rel[: -len(".py")]
        if path.name == "__init__.py":
            excluded[key] = "package stub: __path__ / importlib.resources"
            continue
        if rel == RUNPY_ENTRY:
            excluded[key] = "runpy -m entry point (core/src/pyengine/sidecar.rs)"
            continue
        parts = path.relative_to(pkg).parts
        if len(parts) > 3 and parts[0] == "skills" and parts[1] == "bundled" and parts[-2] == "scripts":
            excluded[key] = "bundled skill script, executed as a file"
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            excluded[key] = f"unreadable source ({error})"
            continue
        hazards = source_hazards(text)
        if hazards:
            globs_only = all(reason.startswith("enumerates ") for reason in hazards)
            if globs_only and rel in SAFE_SOURCE_GLOBS:
                notes[key] = SAFE_SOURCE_GLOBS[rel]
                targets.append(path)
                continue
            excluded[key] = "; ".join(sorted(set(hazards)))
            continue
        targets.append(path)
    return targets, excluded, notes


def build_module(
    path: Path,
    pkg: Path,
    cython: Path,
    work: Path,
    import_lib: str,
    env: dict[str, str],
    tools: dict[str, str],
) -> tuple[str, Path | None, str]:
    """One module -> C -> .pyd. Returns (key, pyd or None, error line)."""
    key = path.relative_to(pkg).as_posix()[: -len(".py")]
    mirror = work / "src" / f"{key}.py"
    mirror.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(path, mirror)
    c_file = mirror.with_suffix(".c")
    # `--module-name` is not cosmetic. Without it Cython names the module after the
    # file stem, so every class in `opensquilla/session/models.py` gets
    # `__module__ == "models"`. Pydantic resolves `from __future__ import annotations`
    # strings through `sys.modules[cls.__module__].__dict__`, finds nothing under
    # that name, and leaves every field annotation an unresolved ForwardRef —
    # measured 2026-10-29: sqlmodel then dies at import with
    # `TypeError: issubclass() arg 1 must be a class`, and models whose fields
    # survive raise `PydanticUndefinedAnnotation` on first use. The bare stem
    # also breaks `relative import` resolution for the same reason.
    # The name must be the dotted path *including* the top-level package
    # (`pkg` is `<...>/src/opensquilla`, so `key` alone would only say
    # `session.models`). A path component that is not an identifier
    # (`squilla_router/models/v4.2_phase3_inference/...`) cannot be spelled as a
    # module name at all — those directories are unimportable to begin with, so
    # they keep the historical file-stem naming instead of failing outright.
    parts = [pkg.name, *key.split("/")]
    name_args = (
        ["--module-name", ".".join(parts)] if all(p.isidentifier() for p in parts) else []
    )
    rc, out = run(
        [
            str(cython),
            *CYTHON_FLAGS,
            *name_args,
            "-o",
            str(c_file),
            str(mirror),
        ],
        env=env,
        timeout=600,
    )
    if rc or not c_file.is_file():
        return key, None, f"cython: {tail(out)}"
    # Every cl/link argument is a path relative to work/src and every directory is
    # carried in INCLUDE/LIB: subprocess.list2cmdline cannot pass `/I"dir with space"`
    # through cl's own option parser, and a quoted token there fails as
    # `C1083: cannot open include file 'Python.h'`.
    # Flat output names keyed by the full module path, too — `gateway/config` and
    # `sandbox/config` share a stem, and colliding in one directory would hand the
    # second module's binary to the first's destination.
    flat = key.replace("/", "_")
    source_arg = key.replace("/", "\\") + ".c"
    rc, out = run(
        [
            tools["cl"],
            "/nologo",
            "/O2",
            "/MD",
            "/utf-8",
            "/c",
            f"/Fo{flat}.obj",
            source_arg,
        ],
        env=env,
        cwd=work / "src",
        timeout=900,
    )
    if rc:
        return key, None, f"cl: {tail(out)}"
    rc, out = run(
        [
            tools["link"],
            "/nologo",
            "/DLL",
            f"/OUT:{flat}.pyd",
            f"{flat}.obj",
            import_lib,
        ],
        env=env,
        cwd=work / "src",
        timeout=600,
    )
    pyd = work / "src" / f"{flat}.pyd"
    if rc or not pyd.is_file():
        return key, None, f"link: {tail(out)}"
    return key, pyd, ""


def tail(text: str, lines: int = 6) -> str:
    kept = [ln for ln in text.splitlines() if ln.strip()]
    return " | ".join(kept[-lines:])[-500:]


def source_fingerprint(root: Path) -> str:
    """Content hash of the engine sources this tree was compiled from.

    Measured on 2026-10-28: `gui/src-tauri/build.rs` re-stages `resources/pyengine`
    on *every* cargo invocation, and the IDE's periodic `cargo check` runs it with
    its own environment — so it wiped a native tree 23 seconds after it landed. The
    build script now leaves such a tree alone when `NATIVE.json` is present, which
    creates the opposite hazard: a native tree that outlives its sources. This
    fingerprint is what makes that detectable — `check-shipped-resources.py`
    compares it against the repo's `pyengine/src` and fails when they diverge.
    """
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*.py")):
        if "__pycache__" in path.parts:
            continue
        digest.update(path.relative_to(root).as_posix().encode("utf-8"))
        digest.update(b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()[:16]


CHILD_VERIFY_SCRIPT = r"""
import importlib, json, sys

names = json.loads(open(sys.argv[1], encoding="utf-8").read())
sink = open(sys.argv[2], "a", encoding="utf-8")
known = set(json.loads(open(sys.argv[3], encoding="utf-8").read()))


def flat(value):
    return " ".join(str(value).split())


def owner_of(tb, fallback):
    # Attribution is by the frame's module __name__, never by co_filename: a
    # compiled Cython frame reports only the *basename* of the source it was
    # mirrored from ("policy_models.py", co_name "init policy_models"), so a path
    # match silently failed and every cascade importer looked self-blamed (546
    # demotions on 2026-10-28, measured). The innermost engine frame is the module
    # whose own body raised; the importers above it are collateral.
    best = None
    while tb is not None:
        module = tb.tb_frame.f_globals.get("__name__")
        if isinstance(module, str) and module in known:
            best = module
        tb = tb.tb_next
    return best or fallback


for name in names:
    try:
        importlib.import_module(name)
    except BaseException as error:
        sink.write("%s\t%s\t%s: %s\n" % (name, owner_of(error.__traceback__, name),
                                         type(error).__name__, flat(error)))
        sink.flush()
"""


def verify_pass(
    python: Path, root: Path, names: list[str], known: list[str], work: Path, label: str
) -> dict[str, tuple[str, str]]:
    """Import `names` with `root` on PYTHONPATH; map failing module -> (culprit, message).

    `known` is the set of module names whose frames may be blamed for a failure —
    the caller passes every module in the tree, not just the ones being imported,
    because a compiled module can blow up inside a sibling it imported.

    The list travels in a file, not argv: 883 dotted names overflow the 32 KiB
    CreateProcess command line and this pass died with
    ``FileNotFoundError: [WinError 206]`` on 2026-10-28 after an 18-minute build.
    Each failure is appended as it happens so a timeout still leaves evidence about
    the modules that were checked.
    """
    list_file = work / f"verify-{label}-list.json"
    bad_file = work / f"verify-{label}-bad.tsv"
    known_file = work / f"verify-{label}-known.json"
    list_file.write_text(json.dumps(names), encoding="utf-8")
    known_file.write_text(json.dumps(known), encoding="utf-8")
    bad_file.write_text("", encoding="utf-8")
    env = dict(os.environ)
    env["PYTHONPATH"] = str(root)
    env["PYTHONIOENCODING"] = "utf-8"
    # Importing the plaintext leftovers would otherwise drop `__pycache__` into the
    # tree that becomes the installer payload — check-shipped-resources flags those
    # as violations, because a shipped cache directory is both dead weight and a
    # copy of the very source the native build is meant to stop carrying.
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    try:
        proc = subprocess.run(
            [
                str(python),
                "-c",
                CHILD_VERIFY_SCRIPT,
                str(list_file),
                str(bad_file),
                str(known_file),
            ],
            capture_output=True,
            stdin=subprocess.DEVNULL,
            env=env,
            timeout=1800,
        )
        code, out = proc.returncode, (proc.stdout + proc.stderr).decode("utf-8", "replace")
    except subprocess.TimeoutExpired as error:
        partial = (error.stdout or b"") + (error.stderr or b"")
        code, out = 124, partial.decode("utf-8", "replace")
    bad: dict[str, tuple[str, str]] = {}
    for line in bad_file.read_text(encoding="utf-8", errors="replace").splitlines():
        name, _, rest = line.partition("\t")
        culprit, _, message = rest.partition("\t")
        if name:
            bad[name] = (culprit, message)
    if code:
        bad["<harness>"] = ("<harness>", f"rc={code} after {len(bad)} failure(s): {tail(out)}")
    return bad


def stabilize_native(
    staged: Path, python: Path, compiled: list[str], pkg: Path, work: Path, max_rounds: int = 8
) -> tuple[list[str], dict[str, str], list[str], list[str]]:
    """Import-verify, and demote whichever module is *itself* to blame, until stable.

    Cython and pydantic v2 are structurally incompatible for models that define
    methods: pydantic's `default_ignored_types()` only forgives `FunctionType`, and
    a compiled method is a `cyfunction`, so class creation raises
    `A non-annotated attribute was detected` (reproduced on a six-line fixture, and
    `-X binding=True` does not change it). Rather than hand-maintain a list of
    "modules that look pydantic-ish", the tree is judged by running it: a module
    that imports fine as source but raises natively is kept as source, and the
    hundreds of importers that only failed in cascade are left compiled.

    Returns (still-native keys, demoted key -> reason, pre-existing failures,
    regressions that could not be attributed to a self-blamed module).
    """
    key_of = {f"opensquilla.{key.replace('/', '.')}": key for key in compiled}
    all_names = sorted(key_of)
    native = list(compiled)
    demoted: dict[str, str] = {}
    baseline_failed: set[str] = set()
    for _round in range(max_rounds):
        names = [f"opensquilla.{key.replace('/', '.')}" for key in native]
        bad = verify_pass(python, staged / "pyengine" / "src", names, all_names, work, "native")
        if not bad:
            return native, demoted, [], []
        if not baseline_failed:
            baseline = verify_pass(
                python, REPO_ROOT / "pyengine" / "src", all_names, all_names, work, "source"
            )
            baseline_failed = set(baseline)
        failing = {n: v for n, v in bad.items() if n not in baseline_failed}
        pre_existing = [
            f"{n}: {message}" for n, (_, message) in sorted(bad.items()) if n in baseline_failed
        ]
        roots = {n: m for n, (c, m) in failing.items() if c == n}
        if not roots:
            return (
                native,
                demoted,
                pre_existing,
                [f"{n}: {message}" for n, (_, message) in sorted(failing.items())],
            )
        unremovable: list[str] = []
        for name, message in sorted(roots.items()):
            key = key_of[name]
            pkg.joinpath(f"{key}.pyd").unlink(missing_ok=True)
            if not pkg.joinpath(f"{key}.py").is_file():
                unremovable.append(f"{name}: demoted but its .py is gone ({message})")
                continue
            demoted[key] = message
        native = [k for k in native if k not in demoted]
        if unremovable:
            return native, demoted, [], unremovable
    return native, demoted, [], [f"did not stabilize after {max_rounds} rounds"]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=str(STAGED), help="staged resources directory")
    parser.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 4) - 1))
    parser.add_argument("--limit", type=int, default=0, help="compile at most N modules (rehearsal)")
    parser.add_argument("--only", default="", help="substring filter on module path (rehearsal)")
    parser.add_argument("--keep-source", action="store_true", help="do not delete replaced .py")
    parser.add_argument("--dry-run", action="store_true", help="classify and report, write nothing")
    parser.add_argument(
        "--skip-verify", action="store_true", help="skip the bundled-interpreter import pass"
    )
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="reuse the .pyd already staged; only build the ones that are missing",
    )
    args = parser.parse_args(argv)

    staged = Path(args.root).resolve()
    pkg = staged / PKG_SUBPATH
    if not pkg.is_dir():
        return cannot_verify(f"{pkg} is absent — run scripts/package_full.cmd staging first")

    python = staged / "python" / "python.exe"
    if not python.is_file():
        return cannot_verify(
            f"{python} is absent — the native build must link against the shipping "
            "interpreter; run scripts/fetch_python_runtime.ps1"
        )
    rc, out = run([str(python), "-c", "import sys; print('%d.%d' % sys.version_info[:2])"])
    if rc:
        return cannot_verify(f"bundled interpreter not runnable: {tail(out)}")
    version = out.strip().splitlines()[-1]
    import_lib = f"python{version.replace('.', '')}.lib"
    include = staged / "python" / "include"
    libs = staged / "python" / "libs"
    if not (include / "Python.h").is_file():
        return cannot_verify(f"{include / 'Python.h'} missing — cannot target the bundled ABI")
    if not (libs / import_lib).is_file():
        return cannot_verify(f"{libs / import_lib} missing — cannot link against the bundled ABI")
    if not ENGINE_SOURCE_ROOT.is_dir():
        return cannot_verify(
            f"{ENGINE_SOURCE_ROOT} is absent — the manifest stamps the sources it was "
            "compiled from, and without them the stamp cannot be computed or verified"
        )
    fingerprint = source_fingerprint(ENGINE_SOURCE_ROOT)

    try:
        cython, cython_version = cython_env(staged)
        vcvars = find_vcvars()
        if vcvars is None:
            return cannot_verify("no vcvars64.bat found — install Visual Studio 2022 Build Tools")
        env, tools = msvc_env(vcvars)
        env_prepend(env, "INCLUDE", str(include))
        env_prepend(env, "LIB", str(libs))
    except RuntimeError as error:
        return cannot_verify(str(error))

    targets, excluded, notes = classify(pkg)
    if args.only:
        targets = [t for t in targets if args.only in t.relative_to(pkg).as_posix()]
    if args.limit:
        targets = targets[: args.limit]

    def key_of(path: Path) -> str:
        return path.relative_to(pkg).as_posix().rsplit(".", 1)[0]

    # --skip-build: trust what is already in the staged tree and only pay for the
    # missing modules. Needed because the 2026-10-28 full pass compiled all 883
    # targets in 18 minutes and then died in the *verification* harness, so a
    # re-run had to reuse the artifacts rather than rebuild them.
    reused: list[str] = []
    if args.skip_build:
        allowed = {key_of(t) for t in targets}
        staged_keys = {key_of(p) for p in pkg.rglob("*.pyd")}
        reused = sorted(allowed & staged_keys)
        targets = [t for t in targets if key_of(t) not in reused]

    print(f"bundled interpreter : {version} ({import_lib})")
    print(f"cython              : {cython_version}")
    print(f"compiler            : {vcvars}")
    print(f"modules to compile  : {len(targets)}")
    if args.skip_build:
        print(f"reused from staging : {len(reused)}")
    print(f"declared exclusions : {len(excluded)}")
    for rel, reason in sorted(excluded.items()):
        if not reason.startswith("package stub"):
            print(f"  - {rel}: {reason}")
    for rel, reason in sorted(notes.items()):
        print(f"  + {rel}: compiled despite globbing .py — {reason}")
    if args.dry_run:
        print("[dry-run] nothing written")
        return 0

    # Claim the tree *before* the first .pyd lands. `gui/src-tauri/build.rs` skips
    # its unconditional re-stage when `NATIVE.json` exists, and without this marker
    # the ~18-minute window between "staged" and "manifest written" is exactly when
    # the IDE's periodic `cargo check` deleted a finished native tree (23 seconds
    # after it landed, measured 2026-10-28). A run that dies here leaves
    # `building: true` behind on purpose: `check-shipped-resources.py` then refuses
    # to package a half-converted tree, and the next full compile replaces it.
    manifest = staged / MANIFEST_SUBPATH
    if manifest.is_file() and '"building": true' in manifest.read_text(encoding="utf-8"):
        print("[note] a previous compile claimed this tree and never finished; this run replaces the claim")
    if not (args.only or args.limit):
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_bytes(
            json.dumps(
                {
                    "schema": 1,
                    "building": True,
                    "python": version,
                    "sourceFingerprint": fingerprint,
                    "compiled": [],
                    "excluded": {},
                },
                ensure_ascii=False,
                indent=1,
            ).encode("utf-8")
        )

    work = Path(tempfile.mkdtemp(prefix="marguerite-native-"))
    compiled: list[str] = list(reused)
    failures: list[tuple[str, str]] = []
    manifest_written = False
    started = time.monotonic()
    try:
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [
                pool.submit(
                    build_module,
                    path,
                    pkg,
                    cython,
                    work,
                    import_lib,
                    env,
                    tools,
                )
                for path in targets
            ]
            for index, future in enumerate(concurrent.futures.as_completed(futures), 1):
                key, pyd, error = future.result()
                if pyd is None:
                    failures.append((key, error))
                    continue
                shutil.copy2(pyd, pkg.joinpath(f"{key}.pyd"))
                compiled.append(key)
                if index % 100 == 0 or index == len(futures):
                    elapsed = time.monotonic() - started
                    print(f"  built {index}/{len(futures)} ({elapsed:.0f}s)")

        # One parallel pass is not reproducible. With ~900 Cython processes there
        # is always a handful that die with rc != 0 and **empty** output, in one
        # run and not the next: two consecutive full builds on 2026-10-29 failed
        # on 47 and 69 modules with only 19 names in common, and the differing
        # ones were plain `cython: ` with no message. A silent death is a resource
        # artifact, not a verdict about the source, so a failure is only believed
        # after it fails again when it is the only thing running.
        if failures:
            print(f"[note] retrying {len(failures)} failed module(s) one at a time")
            by_key = {key_of(t): t for t in targets}
            retried: list[tuple[str, str]] = []
            recovered = 0
            for key, error in failures:
                path = by_key.get(key)
                if path is None:
                    retried.append((key, error))
                    continue
                again_key, pyd, again_error = build_module(
                    path, pkg, cython, work, import_lib, env, tools
                )
                if pyd is None:
                    retried.append((again_key, again_error))
                    continue
                shutil.copy2(pyd, pkg.joinpath(f"{again_key}.pyd"))
                compiled.append(again_key)
                recovered += 1
            failures = retried
            print(
                f"[note] retry recovered {recovered}; {len(failures)} still failing"
            )

        # Import-verify before destroying any source: with both present the .pyd
        # wins, so this exercises exactly the shipped artifact. Modules that the
        # toolchain cannot compile into an importable binary are demoted back to
        # source here, on runtime evidence, and stay demoted in the manifest.
        regression: list[str] = []
        pre_existing: list[str] = []
        demoted: dict[str, str] = {}
        if not args.skip_verify:
            native, demoted, pre_existing, regression = stabilize_native(
                staged, python, sorted(compiled), pkg, work
            )
            compiled = native

        if not args.keep_source:
            for key in compiled:
                pkg.joinpath(f"{key}.py").unlink(missing_ok=True)

        if args.only or args.limit:
            print(
                "[note] partial build (--only/--limit): NATIVE.json left untouched -- "
                "the guard only accepts a manifest describing the whole tree"
            )
        else:
            manifest_written = True
            manifest.write_bytes(
                json.dumps(
                    {
                        "schema": 1,
                        "python": version,
                        "sourceFingerprint": fingerprint,
                        "importLib": import_lib,

                        "cython": cython_version,
                        "compiled": sorted(compiled),
                        "excluded": dict(sorted(excluded.items())),
                        "safeSourceGlobNotes": dict(sorted(notes.items())),
                        "compileFailures": {
                            **{key: msg for key, msg in failures},
                            **demoted,
                        },
                    },
                    ensure_ascii=False,
                    indent=1,
                ).encode("utf-8")
            )
    finally:
        shutil.rmtree(work, ignore_errors=True)

    source_bytes = sum(p.stat().st_size for p in pkg.rglob("*.py"))
    print(
        f"\ncompiled {len(compiled)} native module(s); "
        f"{len(list(pkg.rglob('*.py')))} .py still staged "
        f"({source_bytes:,} plaintext bytes remain)"
    )
    if pre_existing:
        print(f"[note] {len(pre_existing)} module(s) already unimportable as source (not caused here)")
        for item in pre_existing[:5]:
            print(f"  - {item}")
    if failures:
        print(f"[WARN] {len(failures)} module(s) failed to build and remain plaintext:")
        for rel, message in failures[:10]:
            print(f"  - {rel}: {message}")
    if demoted:
        print(
            f"[note] {len(demoted)} compiled module(s) would not import and were "
            "demoted back to source (see compileFailures):"
        )
        for rel, message in sorted(demoted.items())[:10]:
            print(f"  - {rel}: {message[:160]}")
    if regression:
        print(f"[FAIL] {len(regression)} native module(s) do not import:", file=sys.stderr)
        for item in regression[:20]:
            print(f"  - {item}", file=sys.stderr)
        return 1
    if manifest_written:
        print(f"[OK] manifest -> {manifest.relative_to(REPO_ROOT).as_posix()}")
    else:
        print("[OK] rehearsal complete (no manifest written)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
