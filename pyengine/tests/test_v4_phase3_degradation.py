"""V4 Phase 3 router must degrade loudly, not silently.

Regression for the ML300 incident where missing ML dependencies routed
every turn to the default tier with only per-turn debug metadata as
evidence.

Extended for the weights-not-fetched class: the shipped bundle's weights are
Git LFS pointers, which used to be misreported as a generic
``router_runtime_unavailable`` (and logged as ``init_failed``) because the
size check ran before the pointer check.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

from opensquilla.router_runtime_diagnostics import (
    REPAIRABLE_ROUTER_DEGRADATIONS,
    ROUTER_ASSETS_MISSING,
    ROUTER_PYTHON_DEPENDENCY_MISSING,
    ROUTER_WEIGHTS_NOT_FETCHED,
    classify_router_runtime_error,
    router_runtime_hint,
)
from opensquilla.squilla_router import v4_phase3 as v4mod
from opensquilla.squilla_router.v4_phase3 import (
    V4Phase3Strategy,
    default_bundle_dir,
    inspect_bundle,
    scan_manifest,
)

_LFS_POINTER = b"version https://git-lfs.github.com/spec/v1\noid sha256:deadbeef\nsize 39684725\n"


def _make_unavailable_classifier(tmp_path):
    # Empty bundle dir → _validate_bundle fails → unavailable (no raise
    # because require_router_runtime defaults to False).
    return V4Phase3Strategy(bundle_dir=tmp_path / "missing-bundle")


class _RecordingLog:
    """Minimal structlog stand-in that records ``(event, kwargs)`` pairs.

    2026-10-22：增加 ``levels`` —— 只记 event 名无法断言"可一条命令修复的降级
    不该记 error"。启动日志的 ERROR 计数是用户判断"系统是否坏了"的主要信号，
    所以级别本身就是契约的一部分，必须可测。
    """

    def __init__(self) -> None:
        self.events: list[tuple[str, dict]] = []
        self.levels: list[str] = []

    def _record(self, event: str, **kwargs) -> None:
        self.events.append((event, kwargs))
        self.levels.append(self._level)

    def error(self, event: str, **kwargs) -> None:
        self._level = "error"
        self._record(event, **kwargs)

    def warning(self, event: str, **kwargs) -> None:
        self._level = "warning"
        self._record(event, **kwargs)

    def info(self, event: str, **kwargs) -> None:
        self._level = "info"
        self._record(event, **kwargs)

    def debug(self, event: str, **kwargs) -> None:
        self._level = "debug"
        self._record(event, **kwargs)

    @property
    def names(self) -> list[str]:
        return [event for event, _ in self.events]


def _write_bundle(bundle: Path, files: dict[str, bytes]) -> None:
    (bundle / "runtime_src").mkdir(parents=True, exist_ok=True)
    (bundle / "router.runtime.yaml").write_text("v4: {}\n", encoding="utf-8")
    entries = []
    for rel, payload in files.items():
        target = bundle / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)
        entries.append({"path": rel, "size_bytes": len(payload)})
    (bundle / "artifact_manifest.json").write_text(
        json.dumps({"files": entries}), encoding="utf-8"
    )


def test_unavailable_classifier_reports_source(tmp_path):
    clf = _make_unavailable_classifier(tmp_path)
    tier, confidence, source, meta = asyncio.run(
        clf.classify("hello", valid_tiers=["c1", "c2"])
    )
    assert source == "v4_unavailable"
    assert confidence == 0.0


def test_degradation_warns_once(tmp_path):
    clf = _make_unavailable_classifier(tmp_path)
    assert clf._degraded_warned is False
    asyncio.run(clf.classify("first", valid_tiers=["c1"]))
    assert clf._degraded_warned is True
    # Second call must not reset or re-warn; flag stays set.
    asyncio.run(clf.classify("second", valid_tiers=["c1"]))
    assert clf._degraded_warned is True


def test_require_router_runtime_fails_fast(tmp_path):
    with pytest.raises(RuntimeError):
        V4Phase3Strategy(
            bundle_dir=tmp_path / "missing-bundle",
            require_router_runtime=True,
        )


def test_require_router_runtime_reports_lfs_pointer_assets(tmp_path):
    bundle = tmp_path / "bundle"
    (bundle / "runtime_src").mkdir(parents=True)
    (bundle / "router.runtime.yaml").write_text("v4: {}\n", encoding="utf-8")

    pointer = b"version https://git-lfs.github.com/spec/v1\n"
    asset = bundle / "features" / "tfidf.pkl"
    asset.parent.mkdir(parents=True)
    asset.write_bytes(pointer)
    (bundle / "artifact_manifest.json").write_text(
        json.dumps({"files": [{"path": "features/tfidf.pkl", "size_bytes": len(pointer)}]}),
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match="Git LFS pointer"):
        V4Phase3Strategy(bundle_dir=bundle, require_router_runtime=True)


# ── weights-not-fetched class ────────────────────────────────────────────────


def test_pointer_detection_precedes_size_check(tmp_path):
    """A pointer is always smaller than its artifact.

    Checking size first (the original order) reported
    ``size 133 != manifest 23902398`` — true, but it hid the one-command fix.
    """

    bundle = tmp_path / "bundle"
    _write_bundle(bundle, {"bge_onnx/model.onnx": _LFS_POINTER})

    scan = scan_manifest(bundle)
    assert scan.pointer_assets == ("bge_onnx/model.onnx",)
    assert all("Git LFS pointer" in p for p in scan.problems), scan.problems
    assert not any("size " in p and "!=" in p for p in scan.problems), (
        "pointer must be diagnosed as a pointer, not as size drift"
    )


def test_inspect_bundle_classifies_pointer_bundle_as_weights_not_fetched(tmp_path):
    bundle = tmp_path / "bundle"
    _write_bundle(
        bundle,
        {
            "lgbm_main.bin": _LFS_POINTER,
            "features/tfidf.pkl": _LFS_POINTER,
            "version.json": b'{"version": "v4.2"}',
        },
    )

    verdict = inspect_bundle(bundle)
    assert verdict["available"] is False
    assert verdict["errorKind"] == ROUTER_WEIGHTS_NOT_FETCHED
    assert verdict["pointerAssets"] == ["lgbm_main.bin", "features/tfidf.pkl"]
    assert verdict["checkedAssets"] == 1  # version.json is the only real byte file
    assert verdict["modelVersion"] == "v4.2"
    assert "git lfs pull" in verdict["hint"]
    assert classify_router_runtime_error(verdict["problems"][0]) == ROUTER_WEIGHTS_NOT_FETCHED


def test_inspect_bundle_reports_available_for_a_complete_bundle(tmp_path):
    bundle = tmp_path / "bundle"
    _write_bundle(
        bundle,
        {
            "lgbm_main.bin": b"\x00real-weights\x01",
            "features/tfidf.pkl": b"\x00real-pickle\x01",
        },
    )

    verdict = inspect_bundle(bundle)
    assert verdict["available"] is True
    assert verdict["errorKind"] is None
    assert verdict["hint"] == ""
    assert verdict["problemCount"] == 0
    assert verdict["checkedAssets"] == 2


def test_inspect_bundle_flags_a_missing_bundle(tmp_path):
    verdict = inspect_bundle(tmp_path / "nope")
    assert verdict["present"] is False
    assert verdict["errorKind"] == ROUTER_ASSETS_MISSING
    assert verdict["hint"] == router_runtime_hint(ROUTER_ASSETS_MISSING)


def test_weights_not_fetched_logs_a_distinct_event(tmp_path, monkeypatch):
    """The roadmap anchor: startup logs must not carry ``init_failed`` for this."""

    recorder = _RecordingLog()
    monkeypatch.setattr(v4mod, "log", recorder)

    bundle = tmp_path / "bundle"
    _write_bundle(bundle, {"lgbm_main.bin": _LFS_POINTER})
    strategy = V4Phase3Strategy(bundle_dir=bundle)

    assert recorder.names == ["v4_phase3.weights_not_fetched"], recorder.names
    assert strategy.status()["errorKind"] == ROUTER_WEIGHTS_NOT_FETCHED
    assert strategy.status()["available"] is False


def test_repairable_degradations_are_not_logged_as_errors():
    """「一条命令可修」的降级必须记 warning，不能记 error。

    真实场景（2026-10-22 实测安装版启动日志）：

        [ERROR] opensquilla.squilla_router.v4_phase3: v4_phase3.init_failed
                error="No module named 'lightgbm'" ...
        [WARNING] OpenSquilla router fallback active: ...
                OpenSquilla can still run with safe router fallback ...

    同一次启动里，ERROR 行说"初始化失败"，紧跟着的 WARNING 行说"仍可正常运行" ——
    自相矛盾，用户只记住那 12 条 ERROR。而 ``RouterBundleDegraded`` 的 docstring
    早已写明 ML 路由是可选增强、规则路由仍服务每一轮。

    判据：``REPAIRABLE_ROUTER_DEGRADATIONS`` 里的种类 → warning；
    其余（真损坏，如 bundle 布局缺失）→ 仍 error。
    """

    assert ROUTER_WEIGHTS_NOT_FETCHED in REPAIRABLE_ROUTER_DEGRADATIONS
    assert ROUTER_PYTHON_DEPENDENCY_MISSING in REPAIRABLE_ROUTER_DEGRADATIONS
    # 真损坏必须留在外面，否则会把"安装坏了"也降级成 warning
    assert ROUTER_ASSETS_MISSING not in REPAIRABLE_ROUTER_DEGRADATIONS


def test_python_dependency_missing_logs_deps_missing_warning(tmp_path, monkeypatch):
    """``lightgbm`` 缺失 → event=deps_missing、级别=warning（而非 init_failed/error）。"""

    recorder = _RecordingLog()
    monkeypatch.setattr(v4mod, "log", recorder)

    bundle = tmp_path / "bundle"
    _write_bundle(bundle, {"lgbm_main.bin": b"not-a-real-model"})

    # 让 _init_runtime 抛出 ImportError，走 classify → router_python_dependency_missing
    def _raise_import_error(*_a, **_kw):
        raise ImportError("No module named 'lightgbm'")

    monkeypatch.setattr(V4Phase3Strategy, "_init_runtime", _raise_import_error)
    strategy = V4Phase3Strategy(bundle_dir=bundle)

    assert strategy.status()["errorKind"] == ROUTER_PYTHON_DEPENDENCY_MISSING
    assert recorder.names == ["v4_phase3.deps_missing"], recorder.names
    assert recorder.levels == ["warning"], recorder.levels
    # 提示必须告诉用户怎么修（否则 warning 也只是噪音）
    assert "recommended" in recorder.events[0][1]["hint"]


def test_unrepairable_failure_still_logs_error(tmp_path, monkeypatch):
    """真损坏（bundle 布局缺失）必须仍是 init_failed + error —— 不许被"降噪"带走。"""

    recorder = _RecordingLog()
    monkeypatch.setattr(v4mod, "log", recorder)

    def _raise_runtime(*_a, **_kw):
        raise RuntimeError("bundle layout is missing")

    monkeypatch.setattr(V4Phase3Strategy, "_init_runtime", _raise_runtime)
    V4Phase3Strategy(bundle_dir=tmp_path / "whatever")

    # 分类结果取决于 classify_router_runtime_error；关键是它不能落进可修复集合
    from opensquilla.router_runtime_diagnostics import REPAIRABLE_ROUTER_DEGRADATIONS

    kind = recorder.events[0][1]["runtime_error_kind"]
    assert kind not in REPAIRABLE_ROUTER_DEGRADATIONS
    assert recorder.levels == ["error"], recorder.levels


def test_status_surfaces_kind_and_hint(tmp_path):
    strategy = _make_unavailable_classifier(tmp_path)
    status = strategy.status()
    assert status["available"] is False
    assert status["errorKind"] == ROUTER_ASSETS_MISSING
    assert status["bundleDir"] == str(tmp_path / "missing-bundle")
    assert status["hint"] == router_runtime_hint(ROUTER_ASSETS_MISSING)


def test_inspect_bundle_agrees_with_a_direct_scan_of_the_shipped_bundle():
    """Guards against the verdict drifting from the bytes actually on disk.

    Written to hold in both states: with LFS pointers un-pulled (today) it
    must say ``weights_not_fetched``; after ``git lfs pull`` it must say
    ``available``. It never silently skips a mismatch.
    """

    bundle = default_bundle_dir()
    if not (bundle / "artifact_manifest.json").is_file():
        pytest.skip("shipped bundle absent from this checkout")

    scan = scan_manifest(bundle)
    verdict = inspect_bundle(bundle)

    assert verdict["available"] is (not scan.problems)
    if scan.pointer_assets:
        assert verdict["errorKind"] == ROUTER_WEIGHTS_NOT_FETCHED
        assert verdict["pointerAssets"] == list(scan.pointer_assets[:5])
        assert verdict["checkedAssets"] == scan.checked
    elif not scan.problems:
        assert verdict["errorKind"] is None

