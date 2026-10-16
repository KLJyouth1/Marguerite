"""Control UI dist resolution: packaged path vs Tauri fallback.

Regression for the 2026-10-12 desktop defect (found via a user's log panel):

    control_ui.webui_assets_missing
    dist_dir='...\\resources\\pyengine\\src\\opensquilla\\gateway\\static\\dist'

Tauri installed the built Vue console at ``<install>/webui/dist``, but the gateway only
looked at ``resources/pyengine/src/opensquilla/gateway/static/dist``. The packaging scripts
copy the console to the Tauri location and never to the gateway's, so the desktop Control UI
served an "assets are unavailable" notice instead of the console.

These tests pin the resolution order so the fallback cannot silently regress.
"""

from __future__ import annotations

from pathlib import Path

from opensquilla.gateway import control_ui


def test_install_root_maps_module_path_to_install_root() -> None:
    fake = Path("C:/App/Marguerite/resources/pyengine/src/opensquilla/gateway/control_ui.py")
    assert control_ui._install_root_from(fake) == Path("C:/App/Marguerite")


def test_install_root_returns_none_when_layout_too_shallow() -> None:
    # A path with fewer than 6 parent segments after resolution has no install
    # root. Use a root-anchored short path so the assertion is CWD-independent on
    # Windows: a relative "a/b/c.py" resolves against the CWD (deep enough that
    # parents[5] exists and returns a drive root instead of None).
    assert control_ui._install_root_from(Path("/a/b/c.py")) is None


def test_resolve_prefers_packaged_static_dist(tmp_path, monkeypatch) -> None:
    """规范位置（打包器复制来的）优先。"""
    static_dir = tmp_path / "static"
    dist = static_dir / "dist"
    dist.mkdir(parents=True)
    monkeypatch.setattr(control_ui, "_STATIC_DIR", static_dir)
    assert control_ui._resolve_dist_dir() == dist


def test_resolve_falls_back_to_tauri_webui_dist(tmp_path, monkeypatch) -> None:
    """核心回归：static/dist 缺失、Tauri 落点存在 → 必须命中回退路径。"""
    static_dir = tmp_path / "static"
    static_dir.mkdir(parents=True)
    module_dir = tmp_path / "resources" / "pyengine" / "src" / "opensquilla" / "gateway"
    module_dir.mkdir(parents=True)
    module_file = module_dir / "control_ui.py"
    module_file.write_text("# probe\n", encoding="utf-8")
    tauri_dist = tmp_path / "webui" / "dist"
    tauri_dist.mkdir(parents=True)

    monkeypatch.setattr(control_ui, "_STATIC_DIR", static_dir)
    monkeypatch.setattr(control_ui, "__file__", str(module_file))
    assert control_ui._resolve_dist_dir() == tauri_dist.resolve()


def test_resolve_returns_primary_when_no_candidate_exists(tmp_path, monkeypatch) -> None:
    """都不命中时返回规范路径，由调用方输出可操作诊断（不抛异常）。"""
    static_dir = tmp_path / "static"
    static_dir.mkdir(parents=True)
    module_file = tmp_path / "x.py"  # 层数不足 → 无回退候选
    module_file.write_text("", encoding="utf-8")
    monkeypatch.setattr(control_ui, "_STATIC_DIR", static_dir)
    monkeypatch.setattr(control_ui, "__file__", str(module_file))
    assert control_ui._resolve_dist_dir() == static_dir / "dist"
