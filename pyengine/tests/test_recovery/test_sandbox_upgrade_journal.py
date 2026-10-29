from __future__ import annotations

import json
import os
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from opensquilla.sandbox.upgrade_migration import (
    SandboxUpgradeCoordinator,
    _resolve_windows_powershell,
    inspect_sandbox_upgrade,
)


def test_interrupted_prepared_journal_retries_to_commit(tmp_path: Path) -> None:
    (tmp_path / "config.toml").write_text(
        '[sandbox]\nrun_mode = "standard"\n',
        encoding="utf-8",
    )
    coordinator = SandboxUpgradeCoordinator(tmp_path)
    coordinator.snapshot_path.mkdir()
    (coordinator.snapshot_path / "manifest.json").write_text(
        '{"stores":[]}',
        encoding="utf-8",
    )
    coordinator.journal_path.write_text(
        json.dumps(
            {
                "migrationVersion": 2,
                "status": "prepared",
                "stores": ["config.toml"],
                "snapshot": str(coordinator.snapshot_path),
            }
        ),
        encoding="utf-8",
    )

    report = coordinator.run()

    assert report.ok is True
    assert report.status == "committed"
    assert inspect_sandbox_upgrade(tmp_path).ok is True


def test_invalid_journal_requires_manual_recovery_without_rollback(
    tmp_path: Path,
) -> None:
    journal = tmp_path / ".sandbox-upgrade-v2.json"
    journal.write_text('{"migrationVersion":999}', encoding="utf-8")

    report = inspect_sandbox_upgrade(tmp_path)

    assert report.ok is False
    assert report.status == "manual_recovery_required"
    assert journal.exists()


def test_concurrent_direct_update_migrations_serialize_on_profile_lock(
    tmp_path: Path,
) -> None:
    (tmp_path / "config.toml").write_text(
        '[sandbox]\nrun_mode = "trusted"\n',
        encoding="utf-8",
    )

    with ThreadPoolExecutor(max_workers=2) as pool:
        reports = list(
            pool.map(
                lambda _: SandboxUpgradeCoordinator(tmp_path).run(),
                range(2),
            )
        )

    assert all(report.ok and report.status == "committed" for report in reports)
    assert 'run_mode = "safe"' in (tmp_path / "config.toml").read_text(
        encoding="utf-8"
    )


@pytest.mark.skipif(os.name != "nt", reason="Windows ACL hardening path")
def test_boot_survives_path_without_windows_powershell(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """PATH 不含 `System32\\WindowsPowerShell\\v1.0` 时迁移仍必须成功。

    真机事故：裸 `powershell` 无法解析 → `FileNotFoundError: [WinError 2]`
    → `migration_failed_manual_recovery_required` → 整个 Python 侧车起不来，
    overview/tools/memory/knowledge 全部数据真空（2026-10-28 隔离网关实测）。
    """

    (tmp_path / "config.toml").write_text(
        '[sandbox]\nrun_mode = "standard"\n',
        encoding="utf-8",
    )
    # 只留一个空目录：CreateProcess 的默认搜索路径不覆盖 WindowsPowerShell。
    monkeypatch.setenv("PATH", str(tmp_path / "empty-path"))
    _resolve_windows_powershell.cache_clear()
    try:
        resolved = _resolve_windows_powershell()
        assert os.path.isabs(resolved)
        assert os.path.isfile(resolved)

        report = SandboxUpgradeCoordinator(tmp_path).run()
        assert report.ok is True, report.error
        assert report.status == "committed"
    finally:
        _resolve_windows_powershell.cache_clear()
