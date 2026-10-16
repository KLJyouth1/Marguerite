"""7-layer memory engine — 现为 Marguerite 原生桥接的委托层。

保留既有公共符号（``MemoryEngine`` 及 ``file_intel`` / ``kb`` 复用的 ``embed`` /
``cosine`` / ``VEC_DIM`` / ``SIM_THRESHOLD`` / ``LAYER_DEFS``），使既有 import 不
失效；全部持久化与 7 层模型现由 ``core_bridge.CoreBridge`` 拥有：

* ``CoreBridge`` 会优先挂载 Marguerite 核心 store（``CoreSqlite``），否则降级到
  自带 ``core_bridge.db``（``Local``）—— 即「物理桥接」。
* ``MemoryEngine`` 仅把同步路由的调用转发给 ``CoreBridge`` 的同步封装
  （store_sync / query_sync / layers_status_sync / count_sync）。

嵌入算法（embed / cosine / VEC_DIM / SIM_THRESHOLD）从 ``core_bridge`` 再导出，
保证 KB 检索与记忆检索处于同一语义空间（与 Rust 核心一致）。
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

from marguerite_workbench.core_bridge import (
    Backend, CoreBridge, LAYER_DEFS, SIM_THRESHOLD, VEC_DIM,
    cosine, embed, now_iso,
)

logger = logging.getLogger("marguerite.memory")

# 重新导出 KB / file_intel 依赖的符号（保持兼容，不破坏既有 import）。
__all__ = [
    "MemoryEngine", "LAYER_DEFS", "VEC_DIM", "SIM_THRESHOLD",
    "embed", "cosine", "now_iso", "Backend", "CoreBridge",
]

# 持久化层集合（L1 sensory 仅驻内存），供外部引用。
_PERSISTED_LAYERS = frozenset(LAYER_DEFS) - {"L1"}


class MemoryEngine:
    """7 层记忆引擎 —— 委托给 ``CoreBridge`` 的物理桥接实现。"""

    def __init__(self, path: str | Path) -> None:
        # ``path`` 是历史 JSONL 路径；取其父目录作为桥接的数据目录。
        legacy_path = Path(path)
        data_dir = legacy_path.parent
        self.bridge = CoreBridge(data_dir=data_dir)
        self.legacy_path = legacy_path
        logger.info(
            "memory bridge ready: backend=%s native=%s store=%s",
            self.bridge.active_backend().value,
            self.bridge.is_native(),
            self.bridge.db_path,
        )

    # -- 后端透传 ----------------------------------------------------------
    def active_backend(self) -> Backend:
        return self.bridge.active_backend()

    def is_native(self) -> bool:
        return self.bridge.is_native()

    # -- 写 -----------------------------------------------------------------
    def store(
        self,
        layer: str,
        kind: str,
        content: str,
        refs: Optional[list[str]] = None,
        embedding: Optional[list[float]] = None,
    ) -> dict:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        return self.bridge.store_sync(layer, kind, content, refs, embedding)

    # -- 读 -----------------------------------------------------------------
    def query(self, layer: str, q: str, k: int = 5) -> list[dict]:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        return self.bridge.query_sync(layer, q, k)

    def layers_status(self) -> list[dict]:
        return self.bridge.layers_status_sync()

    def count(self, layer: str) -> int:
        return self.bridge.count_sync(layer)

    # -- 桥接辅助（供端点调用） --------------------------------------------
    def capabilities(self, expert_count: int = 5) -> dict:
        return self.bridge.capabilities(expert_count=expert_count)

    async def sync_to_core(self) -> dict:
        """Local → Core 冲刷：仅当存在可达的 Marguerite 核心库时生效。"""
        if self.bridge.is_native():
            return {
                "status": "native",
                "synced": 0,
                "detail": "already bound to Marguerite core store",
                "target": str(self.bridge.db_path),
            }
        from marguerite_workbench.core_bridge import (
            Backend, SqliteMemoryBackend, discover_core_db, find_marguerite_home,
        )

        home = find_marguerite_home()
        core_db = discover_core_db(home) if home else None
        if core_db is None:
            return {
                "status": "no_core_store",
                "synced": 0,
                "detail": "no reachable Marguerite core store; nothing to flush",
            }
        core = SqliteMemoryBackend(core_db, Backend.CORE_SQLITE, True, core_db)
        items = await self.bridge._backend.all_items()
        n = await core.import_items(items)
        return {"status": "synced", "synced": n, "target": str(core_db)}

    async def close(self) -> None:
        await self.bridge.close()
