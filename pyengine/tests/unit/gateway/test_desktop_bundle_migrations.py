"""R61 守卫：桌面安装包必须带上 pyengine 的 `migrations/`。

现场（真机，管理员解包 MSI 后以安装布局启动）：打包进来的引擎**能**被找到并启动
（`resources/pyengine/src/opensquilla/cli/main.py`），但随即抛
`RuntimeError: opensquilla migrations directory not found` 退出 —— 因为
`resolve_migrations_dir()` 的第三个候选是 `Path(__file__).resolve().parents[3]/"migrations"`，
安装后即 `<install>/resources/pyengine/migrations`，而打包脚本只复制了 `pyengine/src`。
于是桌面端只剩纯 Rust 能力子集。

本用例把「打包脚本必须复制 migrations」钉成契约，并证明候选路径与打包位置一致。
"""

from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
PACKAGE_SCRIPT = REPO_ROOT / "scripts" / "package_full.cmd"
PACKAGE_SCRIPT_LIGHT = REPO_ROOT / "scripts" / "package_app.cmd"
MIGRATIONS = REPO_ROOT / "pyengine" / "migrations"


def test_repo_migrations_dir_exists_with_versioned_files() -> None:
    assert MIGRATIONS.is_dir(), f"仓库里必须有 {MIGRATIONS}"
    assert list(MIGRATIONS.glob("V*.py")), (
        "migrations 目录必须含 V*.py（resolve_migrations_dir 的判据）"
    )


def test_repo_layout_candidate_matches_where_the_bundle_puts_it() -> None:
    """证明 `migrator.py` 的仓库候选 = 打包脚本复制的 `pyengine/migrations`。"""
    migrator = REPO_ROOT / "pyengine" / "src" / "opensquilla" / "persistence" / "migrator.py"
    candidate = migrator.resolve().parents[3] / "migrations"
    assert candidate == MIGRATIONS.resolve(), (
        f"候选路径与打包位置必须一致：{candidate} != {MIGRATIONS}"
    )
    assert list(candidate.glob("V*.py")), "候选路径必须真的含迁移文件"


def test_packaging_script_copies_migrations_into_the_bundle() -> None:
    text = PACKAGE_SCRIPT.read_text(encoding="utf-8")
    assert "pyengine\\migrations" in text, (
        "package_full.cmd 必须把 pyengine\\migrations 复制进 resources\\pyengine\\"
        "（否则安装版引擎起不来）"
    )
    # 反向自证：脚本仍保留 src 的复制（别为了过本用例删掉它）
    assert "pyengine\\src" in text, "package_full.cmd 仍必须复制 pyengine\\src"


def test_light_packaging_script_declares_where_full_resources_come_from() -> None:
    """轻量脚本不复制资源，必须把「资源从哪来」写清楚，避免有人当它是全量打包。"""
    text = PACKAGE_SCRIPT_LIGHT.read_text(encoding="utf-8")
    assert "package_full" in text or "resources" in text, (
        "package_app.cmd 必须指明全量资源由 package_full.cmd（或手工同步）提供"
    )