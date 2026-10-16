from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
from collections.abc import Callable, Iterator
from pathlib import Path

import pytest

_REPO_ROOT = Path(__file__).resolve().parents[1]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

_PYTEST_STATE_ROOT = Path(tempfile.gettempdir()) / f"opensquilla-pytest-{os.getpid()}"

os.environ.setdefault("OPENSQUILLA_STATE_DIR", str(_PYTEST_STATE_ROOT / "state"))
os.environ.setdefault("OPENSQUILLA_LOG_DIR", str(_PYTEST_STATE_ROOT / "logs"))
os.environ.setdefault("OPENSQUILLA_TURN_CALL_LOG", "0")
os.environ.setdefault("OPENSQUILLA_TEST_PROFILE_LOCK_ROOT", "1")
os.environ.setdefault(
    "OPENSQUILLA_USER_STATE_DIR",
    str(_PYTEST_STATE_ROOT / "profile-lock-state"),
)

# `pythonpath = ["src"]` (pyproject.toml) only patches *this* interpreter's
# ``sys.path``; it is not inherited by the fresh interpreters that the recovery
# contract tests spawn (`subprocess.run([sys.executable, "-m",
# "opensquilla.cli.main", ...])`). The sidecar is deliberately never installed
# (see AGENTS.md section 2), so those children used to die with "No module named
# 'opensquilla'" -- the tests then failed on the import instead of on their
# assertion, and on Windows the parent's write to the already-dead child's stdin
# surfaced as a bare `OSError [Errno 22]`. Exporting the source root makes the
# children import the same tree this suite is testing; tests that simulate a
# *packaged* install pop ``PYTHONPATH`` explicitly, so they keep their meaning.
_SRC_ROOT = _REPO_ROOT / "src"
_pythonpath_entries = [
    entry for entry in os.environ.get("PYTHONPATH", "").split(os.pathsep) if entry
]
if str(_SRC_ROOT) not in _pythonpath_entries:
    os.environ["PYTHONPATH"] = os.pathsep.join([str(_SRC_ROOT), *_pythonpath_entries])

_PROVIDER_ENV_KEYS = (
    "AIHUBMIX_API_KEY",
    "ANTHROPIC_API_KEY",
    "ARK_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "BAILIAN_API_KEY",
    "BOCHA_SEARCH_API_KEY",
    "BRAVE_API_KEY",
    "BRAVE_SEARCH_API_KEY",
    "BYTEPLUS_API_KEY",
    "CUSTOM_LLM_API_KEY",
    "DASHSCOPE_API_KEY",
    "DEEPSEEK_API_KEY",
    "EXA_API_KEY",
    "FIRECRAWL_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GROQ_API_KEY",
    "IQS_SEARCH_API_KEY",
    "KIMI_CODING_API_KEY",
    "LITELLM_API_KEY",
    "MIMO_API_KEY",
    "MINIMAX_API_KEY",
    "MINIMAX_CN_API_KEY",
    "MINIMAX_CODING_API_KEY",
    "MISTRAL_API_KEY",
    "MOONSHOT_API_KEY",
    "OLLAMA_API_KEY",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "QIANFAN_API_KEY",
    "SILICONFLOW_API_KEY",
    "TAVILY_API_KEY",
    "TENCENT_TOKEN_PLAN_API_KEY",
    "TENCENT_TOKENHUB_API_KEY",
    "TENCENT_TOKENHUB_INTL_API_KEY",
    "TOKENRHYTHM_API_KEY",
    "VOLCENGINE_API_KEY",
    "VOLC_ARK_API_KEY",
    "ZAI_API_KEY",
)

_LIVE_MARKERS = (
    "llm",
    "llm_smoke",
    "llm_costly",
    "llm_tools",
    "llm_embedding",
    "llm_reasoning",
    "llm_gateway",
    "llm_image",
    "llm_router_acc",
    "live_channel",
    "live_search",
)


@pytest.fixture(autouse=True)
def _isolate_provider_credentials(
    monkeypatch: pytest.MonkeyPatch,
    request: pytest.FixtureRequest,
) -> None:
    """Keep default tests offline even when local env files have API keys."""
    if any(request.node.get_closest_marker(marker) for marker in _LIVE_MARKERS):
        return
    for env_key in _PROVIDER_ENV_KEYS:
        # An explicit empty value prevents build_services.load_env() from
        # rehydrating credentials from a repository or profile .env file.
        monkeypatch.setenv(env_key, "")


@pytest.fixture(autouse=True)
def _reset_channels_reconciler_singleton():
    """Clear the channels-reconcile bridge between tests.

    Gateway boot registers a live reconciler in a module-level singleton;
    without this reset, a boot-running test leaks its closure into later
    channel CRUD tests, which then reconcile against a foreign gateway.
    """
    from opensquilla.gateway.channels_bridge import reset_channels_reconciler

    reset_channels_reconciler()
    yield
    reset_channels_reconciler()


@pytest.fixture(autouse=True)
def _undo_leaked_cli_structlog_default():
    """Revert the CLI structlog default when a test leaves it behind.

    The CLI entry callback installs a process-wide structlog default (stderr
    output, WARNING+ filter; ``observability/cli_logging.py``). Tests that
    invoke the Typer app would otherwise leak that filter into later tests
    that capture info-level structlog events. Only the CLI default is
    reverted; any other configuration a test installs is left for that test's
    own teardown.
    """
    import structlog

    from opensquilla.observability.cli_logging import is_cli_default_active

    was_configured = structlog.is_configured()
    old_config = structlog.get_config()
    was_cli_default = is_cli_default_active()
    yield
    if is_cli_default_active() and not was_cli_default:
        if was_configured:
            structlog.configure(**old_config)
        else:
            structlog.reset_defaults()


@pytest.fixture(scope="session")
def _migrated_db_template(
    tmp_path_factory: pytest.TempPathFactory,
) -> Iterator[Path]:
    """Build one pristine latest-schema SQLite database per pytest session.

    This fixture is only for ordinary tests that require the current schema.
    Migration, rollback, schema-ahead, audit, and lock tests must continue to
    create their databases from scratch and call the migrator explicitly.
    """
    import hashlib
    import sqlite3

    from opensquilla.persistence.migrator import apply_pending

    template = tmp_path_factory.mktemp("migrated-db-template") / "template.db"
    applied = apply_pending(str(template), _REPO_ROOT / "migrations")
    assert applied, "fresh migrated test template did not apply any migrations"

    connection = sqlite3.connect(template)
    try:
        assert connection.execute("PRAGMA quick_check").fetchone() == ("ok",)
    finally:
        connection.close()

    sidecars = tuple(Path(f"{template}{suffix}") for suffix in ("-wal", "-shm"))
    assert not any(path.exists() for path in sidecars)
    pristine_digest = hashlib.sha256(template.read_bytes()).digest()

    yield template

    assert hashlib.sha256(template.read_bytes()).digest() == pristine_digest
    assert not any(path.exists() for path in sidecars)


@pytest.fixture
def migrated_db_factory(
    tmp_path: Path,
    _migrated_db_template: Path,
) -> Callable[[str | None], Path]:
    """Return a factory that copies the pristine schema into isolated test DBs."""
    import itertools
    import shutil

    sequence = itertools.count()

    def copy_template(filename: str | None = None) -> Path:
        destination = tmp_path / (filename or f"test-{next(sequence)}.db")
        shutil.copyfile(_migrated_db_template, destination)
        return destination

    return copy_template


@pytest.fixture
def migrated_db(migrated_db_factory: Callable[[str | None], Path]) -> Path:
    """Return an isolated latest-schema database for one test."""
    return migrated_db_factory(None)


@pytest.fixture(scope="session")
def isolated_core_wheel(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """Build the core wheel from an isolated source tree with a synthetic UI.

    A source checkout intentionally has no generated Vue ``dist`` tree, while
    standard wheel builds fail closed without a verified artifact. Packaging
    contract tests share this minimal artifact so they continue to test the
    real Hatch wheel selection without requiring a frontend build or mutating
    the checkout under test.
    """

    import hashlib
    import json
    import shutil
    import subprocess

    from scripts.verify_webui_artifact import MANIFEST_NAME, source_fingerprint

    if shutil.which("uv") is None:
        pytest.skip("uv not on PATH")

    temp_root = tmp_path_factory.mktemp("isolated-core-wheel")
    build_root = temp_root / "source"
    build_root.mkdir()
    def ignored(source: str, names: list[str]) -> set[str]:
        source_path = Path(source).resolve()
        generated = {
            name
            for name in names
            if name in {"node_modules", "coverage", "test-results", "__pycache__"}
            or name.endswith(".pyc")
        }
        if source_path == (_REPO_ROOT / "src/opensquilla/gateway/static").resolve():
            generated.add("dist")
        if source_path == (_REPO_ROOT / "opensquilla-webui").resolve():
            generated.add("dist")
        return generated

    for directory in ("src", "migrations", "opensquilla-webui", "scripts"):
        shutil.copytree(_REPO_ROOT / directory, build_root / directory, ignore=ignored)
    for filename in (
        ".gitignore",
        "LICENSE",
        "README.md",
        "hatch_build.py",
        "pyproject.toml",
    ):
        shutil.copy2(_REPO_ROOT / filename, build_root / filename)

    dist = build_root / "src" / "opensquilla" / "gateway" / "static" / "dist"
    assets = dist / "assets"
    assets.mkdir(parents=True)
    (assets / "packaging-probe.js").write_bytes(b"window.__opensquillaPackagingProbe = true;\n")
    (assets / "packaging-probe.css").write_bytes(b"body{}\n")
    (dist / "index.html").write_text(
        """<!doctype html>
<link rel="stylesheet" href="./assets/packaging-probe.css">
<script type="module" src="./assets/packaging-probe.js"></script>
""",
        encoding="utf-8",
    )

    records = []
    for path in sorted(dist.rglob("*")):
        if not path.is_file() or path.name == MANIFEST_NAME:
            continue
        content = path.read_bytes()
        records.append(
            {
                "path": path.relative_to(dist).as_posix(),
                "size": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            }
        )
    manifest = {
        "schemaVersion": 1,
        "sourceFingerprint": source_fingerprint(build_root / "opensquilla-webui"),
        "files": records,
    }
    (dist / MANIFEST_NAME).write_text(
        f"{json.dumps(manifest, indent=2)}\n",
        encoding="utf-8",
    )

    wheel_dir = temp_root / "wheel"
    result = subprocess.run(
        ["uv", "build", "--out-dir", str(wheel_dir)],
        cwd=build_root,
        capture_output=True,
        text=True,
        timeout=300,
    )
    assert result.returncode == 0, f"uv build failed: {result.stderr}"
    wheels = list(wheel_dir.glob("opensquilla-*.whl"))
    assert len(wheels) == 1, f"Expected 1 wheel, got {wheels}"
    return wheels[0]


# ---------------------------------------------------------------------------
# Un-migrated developer scaffolding — skipped, but never silently "passed"
# ---------------------------------------------------------------------------
# This tree is the product half of a larger upstream repository. A handful of
# test modules exercise developer/release tooling that was never migrated here
# (`pyengine/scripts/*.py`, `pyengine/.github/scripts/*.py`), or that lives in
# the optional dev-dependency group. Importing them aborts collection, so a bare
# `pytest` used to open with 19 ERRORs that read like product breakage.
#
# They are deliberately neither deleted nor hidden:
#   * not deleted — these modules are the specification for tooling that may
#     still land, and deleting them would destroy that specification;
#   * not hidden — every skip is echoed in the terminal summary below, so a
#     green run never implies these were verified (see AGENTS.md section 8:
#     "skipped != passed").
# Each group names the exact prerequisite it needs. The moment that
# prerequisite exists the modules are collected again with no edit to this
# file, so the list is self-healing rather than an ever-growing ignore file.
#
# `kind` is how the prerequisite is probed:
#   "path"   -> a filesystem path relative to pyengine/
#   "module" -> an importable module name
_TESTS_ROOT = Path(__file__).resolve().parent

_SCAFFOLDING_GROUPS: tuple[tuple[str, str, tuple[str, ...]], ...] = (
    ("path", "scripts/live_harness_security.py", (
        "live/test_multi_provider_matrix_live.py",
        "live/test_tokenrhythm_catalog_live.py",
        "test_live_multi_provider_matrix.py",
    )),
    ("path", ".github/scripts/check_ci_results.py", (
        "test_ci/test_ci_result_gate.py",
    )),
    ("path", ".github/scripts/build_windows_test_durations.py", (
        "test_ci/test_windows_duration_governance.py",
    )),
    ("path", ".github/scripts/windows_test_shards.py", (
        "test_ci/test_windows_test_shards.py",
    )),
    ("path", "scripts/live_mixed_provider_gateway.py", (
        "test_live_mixed_provider_gateway.py",
    )),
    ("path", "scripts/live_provider_profile_gateway_e2e.py", (
        "test_live_provider_profile_gateway_e2e.py",
    )),
    ("path", "scripts/live_provider_profile_smoke.py", (
        "test_live_provider_profile_smoke.py",
    )),
    ("path", "scripts/live_tokenrhythm_billing_audit.py", (
        "test_live_tokenrhythm_billing_audit.py",
    )),
    ("path", "scripts/live_openrouter_prompt_cache_smoke.py", (
        "test_openrouter_prompt_cache_smoke.py",
    )),
    ("module", "scripts.verify_webui_artifact", (
        "test_ci/test_migrations_packaged.py",
        "test_scripts/test_verify_webui_artifact.py",
    )),
    ("module", "scripts.compare_meta_skill_openclaw", (
        "test_meta_skill_openclaw_comparison.py",
        "test_meta_skill_openclaw_lifestyle_comparison.py",
    )),
    ("module", "scripts.refresh_models_dev_snapshot", (
        "test_scripts/test_refresh_models_dev_snapshot.py",
    )),
    ("module", "scripts.release_channel_manifest", (
        "test_scripts/test_release_channel_manifest.py",
    )),
    # Declared in pyproject [dependency-groups]; install them to run these.
    ("module", "syrupy", (
        "test_tools/test_dispatch_corpus_snapshots.py",
    )),
    ("module", "laboratory", (
        "test_tools/test_dispatch_equivalence.py",
    )),
    # Also in [project.optional-dependencies] dev -- `pip install -e ".[dev]"`.
    ("module", "mypy", (
        "test_ci/test_mypy_partition_gate.py",
    )),
    # --- R60: the remaining never-migrated developer/release scaffolding ------
    # Same class as the entries above: these modules read a tool that this
    # product half of the repository never received. Measured with
    # `pytest <module> --tb=no` (R60); each module listed here is blocked *in
    # full* on the named path:
    #   test_scripts   73 failed / 0 passed   (12 modules, all blocked)
    #   test_desktop  118 failed / 1 passed   (3 modules; the single pass is
    #                                          lost with the module it shares)
    #   integration     8 failed / 0 passed   (1 module in the TUI lab tree)
    #   root level     20 failed / 0 passed   (5 modules)
    # Mixed modules (some tests pass, some read a missing upstream file) are
    # deliberately **not** listed -- ignoring them would drop real green tests,
    # and they stay visible instead of being quietly relabelled.
    ("path", "scripts/experiments/analyze_dashscope_payload_parity.py", (
        "test_scripts/test_analyze_dashscope_payload_parity.py",
    )),
    ("path", "scripts/experiments/analyze_dashscope_payload_risk.py", (
        "test_scripts/test_analyze_dashscope_payload_risk.py",
    )),
    ("path", "scripts/experiments/analyze_tool_compression.py", (
        "test_scripts/test_analyze_tool_compression.py",
    )),
    ("path", "scripts/build_wheelhouse_zip.py", (
        "test_scripts/test_build_wheelhouse_zip.py",
    )),
    ("path", "scripts/experiments/check_docker_image_lock.py", (
        "test_scripts/test_check_docker_image_lock.py",
    )),
    ("path", "scripts/experiments/check_treatment_delivery.py", (
        "test_scripts/test_check_treatment_delivery.py",
    )),
    ("path", "scripts/live_meta_skill_creator_e2e.py", (
        "test_scripts/test_live_meta_skill_creator_e2e.py",
    )),
    ("module", "scripts.live_meta_soft_activation_e2e", (
        "test_scripts/test_live_meta_soft_activation_e2e.py",
    )),
    ("path", "scripts/meta_skill_validation_matrix.py", (
        "test_scripts/test_meta_skill_validation_matrix.py",
    )),
    ("path", "scripts/meta_trigger_accuracy.py", (
        "test_scripts/test_meta_trigger_accuracy_script.py",
    )),
    ("path", "scripts/experiments/replay_finalize_gate.py", (
        "test_scripts/test_replay_finalize_gate.py",
    )),
    ("path", "scripts/smoke_v4_phase3_router.py", (
        "test_scripts/test_router_live_script_defaults.py",
    )),
    # The desktop shell here is the Tauri app in gui/; the Electron tree is gone.
    ("path", "desktop/electron/src/main.ts", (
        "test_desktop/test_electron_startup_contract.py",
    )),
    ("path", "desktop/electron/scripts/gateway-entry.py", (
        "test_desktop/test_gateway_tls_runtime.py",
    )),
    ("path", "desktop/electron/package-lock.json", (
        "test_desktop/test_electron_dependency_security.py",
    )),
    ("path", "scripts/tui_real_terminal_lab.py", (
        "integration/cli/tui_real_terminal/test_lab_script.py",
    )),
    ("path", ".github/scripts/issue_link_sync.py", (
        "test_github_issue_link_sync.py",
    )),
    ("path", ".github/workflows/mirror-release-to-oss.yml", (
        "test_release_oss_mirror_workflow.py",
    )),
    ("path", "README.md", (
        "test_readme_links.py",
        # Also needs docs/features/tui-product-contract.md; README.md is the
        # first file it reads, and neither was migrated.
        "test_tui_default_docs.py",
    )),
    # 3 failed / 0 passed: the whole module asserts upstream meta-skill authoring
    # docs (`META_SKILL_GUIDE.md`, `docs/authoring/meta-skills.md`, and the file
    # named here), none of which was migrated.
    ("path", "docs/features/meta-skill-user-guide.md", (
        "test_skills/test_meta_skill_authoring_docs.py",
    )),
)

# ---------------------------------------------------------------------------
# Inherited upstream contracts -- a *different* repository's CI/release tests
# ---------------------------------------------------------------------------
# `pyengine/` was absorbed from an upstream Python project, and `tests/test_ci/`
# came with it. Several of those modules assert that project's *own*
# `.github/workflows/*.yml`, its `.github/scripts/*` helpers, its
# `scripts/classify-ci-changes.sh`, and its `opensquilla-webui/` console
# directory. None of that was migrated, and it never will be: Marguerite's CI is
# `.github/workflows/{ci,release}.yml` with different job names, and its console
# is `webui/`.
#
# Measured (R24, with PYTHONUTF8=1 so the Windows-cp936 `read_text()` defect is
# excluded): `pytest tests/test_ci` -> 94 failed / 41 passed, of which **84 are
# `test_workflows.py` alone** (it reads `.github/workflows/ci.yml` relative to the
# CWD, i.e. `pyengine/.github/...`, which does not exist). These four entries
# account for 93 of those 94.
#
# ⚠️ Unlike `_SCAFFOLDING_GROUPS`, these do **not** self-heal: restoring the
# absent path is necessary but not sufficient, because the assertions name jobs
# and artefacts this repository deliberately does not have. They are listed so a
# contributor running `pytest` gets a named reason instead of ~93 red lines --
# **not** to make the suite look green.
_INHERITED_GROUPS: tuple[tuple[str, str, tuple[str, ...]], ...] = (
    ("path", ".github/workflows/ci.yml", (
        "test_ci/test_workflows.py",
    )),
    ("path", ".github/scripts/validate_pr_body.py", (
        "test_ci/test_pr_body_lint.py",
    )),
    ("path", "scripts/update_router_artifact_manifest.py", (
        "test_ci/test_router_artifact_manifest.py",
    )),
    # Asserts the upstream console directory; this repo's console is `webui/`.
    ("path", "../opensquilla-webui", (
        "test_ci/test_bgm_asset_hygiene.py",
    )),
)

# tests-relative module path -> (kind, target, reason)
_SCAFFOLDING_INDEX: dict[str, tuple[str, str, str]] = {
    module: (kind, target, "absent")
    for kind, target, modules in _SCAFFOLDING_GROUPS
    for module in modules
} | {
    module: (kind, target, "inherited")
    for kind, target, modules in _INHERITED_GROUPS
    for module in modules
}

_skipped_scaffolding: dict[str, tuple[str, str]] = {}


def _scaffolding_present(kind: str, target: str) -> bool:
    if kind == "path":
        return (_REPO_ROOT / target).exists()
    try:
        return importlib.util.find_spec(target) is not None
    except (ImportError, ValueError):
        # A missing parent package raises ModuleNotFoundError (an ImportError);
        # a malformed dotted name raises ValueError. Both mean "not available".
        return False


def _in_requested_scope(config: pytest.Config, path: Path) -> bool:
    """Is ``path`` inside one of the paths this run was actually asked to collect?

    Needed because pytest evaluates the *top-level entries of the argument's
    parent directory* while descending only into the argument itself. Running
    `pytest tests/unit` therefore hands the hook every module sitting directly in
    `tests/` -- they are genuinely not collected, but reporting them as "skipped"
    for a `tests/unit` run would misstate what that run ignored.
    """
    args = [arg for arg in config.args if not arg.startswith("-")]
    if not args:
        return True
    base = Path(config.invocation_params.dir)
    resolved = path.resolve()
    for raw in args:
        candidate = Path(raw)
        if not candidate.is_absolute():
            candidate = base / candidate
        try:
            candidate = candidate.resolve()
        except OSError:
            continue
        if candidate == resolved or candidate in resolved.parents:
            return True
    return False


def pytest_ignore_collect(collection_path: Path, config: pytest.Config) -> bool | None:
    """Ignore test modules whose scaffolding is absent from this tree.

    Returns ``None`` (never ``False``) for everything else: this hook is
    ``firstresult``, so ``False`` would override other plugins' decisions.
    """
    if not _SCAFFOLDING_INDEX or collection_path.suffix != ".py":
        return None
    try:
        relative = collection_path.resolve().relative_to(_TESTS_ROOT).as_posix()
    except ValueError:
        return None
    prerequisite = _SCAFFOLDING_INDEX.get(relative)
    if prerequisite is None:
        return None
    kind, target, reason = prerequisite
    if _scaffolding_present(kind, target):
        return None
    if _in_requested_scope(config, collection_path):
        _skipped_scaffolding[relative] = (target, reason)
    return True


def pytest_terminal_summary(terminalreporter, exitstatus, config) -> None:
    """Make the scaffolding skips visible in the summary, never as a pass."""
    if not _skipped_scaffolding:
        return
    inherited = [m for m, (_, r) in _skipped_scaffolding.items() if r == "inherited"]
    absent = len(_skipped_scaffolding) - len(inherited)
    terminalreporter.write_sep(
        "-",
        f"{len(_skipped_scaffolding)} test module(s) SKIPPED (not verified): "
        f"{absent} missing prerequisite, {len(inherited)} inherited upstream contract",
    )
    for module in sorted(_skipped_scaffolding):
        target, reason = _skipped_scaffolding[module]
        tag = "inherited, will NOT self-heal" if reason == "inherited" else "needs"
        terminalreporter.write_line(f"  tests/{module}  ->  {tag} {target}")
    terminalreporter.write_line("  see AGENTS.md section 2 for the rationale")
