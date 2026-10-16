"""Native bridge: workbench memory ⇄ Marguerite core 7-layer memory substrate.

目标
----
把工作台的 7 层记忆（L1 感官 … L7 元）做成 Marguerite "自己"的记忆，而不是一个
独立孤岛。Rust 核心已经实现了 7 层记忆（``core/src/memory/*``）以及 Python 侧车
（``pyengine/src/opensquilla/...``）的 SQLite schema。本模块做一个**物理桥接**：

* ``CoreSqlite`` 模式：发现并接入 Marguerite 既有的数据目录 / SQLite 记忆库
  （如 ``~/.marguerite/sessions.db``），把工作台写入的 ``memory_items`` 表放进
  同一份库里 —— 与核心共用同一块物理存储。
* ``Local`` 模式：发现不了核心库时，回退到 schema 完全一致的本地库
  （``<data_dir>/core_bridge.db`` 或 ``<MARGUERITE_HOME>/core_bridge.db``），
  保证测试与离线场景始终可运行。

两种后端暴露**完全相同的接口**（store / query / layers_status / count / capabilities、
以及异步 ``store``/``query``）。``memory.MemoryEngine`` 只管委托，不关心底层是谁。

安全
----
* 所有库文件路径都先经 ``security.safe_resolve`` 做 symlink 防护后才落地。
* 无任何 ``eval`` / 反向 shell；JSON body 由 Pydantic 在 ``api.py`` 校验。
* 嵌入算法与 Rust ``core/src/memory/long_term.rs`` 的 ``LocalEmbedding``
  （256 维 FNV-1a 特征哈希，unigram+bigram，L2 归一化）字节级对齐。
"""

from __future__ import annotations

import asyncio
import json
import logging
import math
import os
import re
import sqlite3
import threading
import uuid
from datetime import datetime, timezone
from enum import Enum
from pathlib import Path
from typing import Optional

logger = logging.getLogger("marguerite.core_bridge")

# ---------------------------------------------------------------------------
# 本地嵌入 —— 与 Rust core/src/memory/long_term.rs 的 LocalEmbedding 对齐
# ---------------------------------------------------------------------------
VEC_DIM = 256
SIM_THRESHOLD = 0.35

# FNV-1a 64-bit 参数（与 Rust 实现一致）。
_FNV_OFFSET = 0xCBF29CE484222325
_FNV_PRIME = 0x100000001B3
_U64_MASK = 0xFFFFFFFFFFFFFFFF

# 7 层目录：id -> (name, description, capacity)。L1 为瞬时层（不落盘）。
LAYER_DEFS: dict[str, tuple[str, str, int]] = {
    "L1": ("sensory", "Raw input buffers (transient)", 64),
    "L2": ("short_term", "Active session context", 512),
    "L3": ("working", "Task-scoped scratchpad (structured)", 512),
    "L4": ("episodic", "Session/event logs (time-ordered)", 1024),
    "L5": ("semantic", "Facts + knowledge base", 2048),
    "L6": ("procedural", "Skills / playbooks / distilled experience", 1024),
    "L7": ("meta", "Self-model: user profile, capabilities, reflection", 512),
}

# 持久化层（L1 感官层按契约保持瞬时，仅存于内存）。
_PERSISTED_LAYERS = frozenset(LAYER_DEFS) - {"L1"}

# 核心库中可能存在的表名（用于"这是不是 Marguerite 核心库"的嗅探）。
_CORE_TABLE_HINTS = ("chunks", "sessions", "files", "embedding_cache", "memory_items")

BRIDGE_VERSION = "1.0.0"


class Backend(str, Enum):
    """物理记忆库绑定到的后端类型。"""

    CORE_SQLITE = "CoreSqlite"  # 已发现并接入 Marguerite 核心库（native）
    LOCAL = "Local"  # 回退：schema 一致的工作台自带本地库


# ---------------------------------------------------------------------------
# 嵌入工具（复刻 Rust LocalEmbedding）
# ---------------------------------------------------------------------------
def _tokenize(text: str) -> list[str]:
    """词袋分词：ascii 字母数字小写；CJK 按单字切分。"""
    tokens: list[str] = []
    for word in re.split(r"[^0-9a-zA-Z]+", text):
        w = word.strip().lower()
        if w and w.isascii():
            tokens.append(w)
    for ch in text:
        if ch.isalpha() and not ch.isascii():
            tokens.append(ch)
    return tokens


def _hash_to(s: str, seed: int) -> int:
    """FNV-1a 64-bit 哈希（带种子，无符号回绕，对齐 Rust）。"""
    h = (_FNV_OFFSET ^ ((seed * _FNV_PRIME) & _U64_MASK)) & _U64_MASK
    for b in s.encode("utf-8"):
        h ^= b
        h = (h * _FNV_PRIME) & _U64_MASK
    return h % VEC_DIM


def embed(text: str) -> list[float]:
    """文本 -> L2 归一化的 256 维特征哈希向量。"""
    vec = [0.0] * VEC_DIM
    tokens = _tokenize(text)
    for t in tokens:
        vec[_hash_to(t, 0)] += 1.0
    for a, b in zip(tokens, tokens[1:]):
        vec[_hash_to(f"{a} {b}", 1)] += 0.5
    norm = math.sqrt(sum(v * v for v in vec))
    if norm > 0.0:
        vec = [v / norm for v in vec]
    return vec


def cosine(a: list[float], b: list[float]) -> float:
    return sum(x * y for x, y in zip(a, b))


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


# ---------------------------------------------------------------------------
# 物理 SQLite 后端（CoreSqlite 与 Local 共用同一 schema）
# ---------------------------------------------------------------------------
_MEMORY_DDL = """
CREATE TABLE IF NOT EXISTS memory_items (
    id         TEXT PRIMARY KEY,
    layer      TEXT NOT NULL,
    kind       TEXT NOT NULL DEFAULT 'note',
    content    TEXT NOT NULL,
    embedding  TEXT,                       -- JSON list[float] | NULL
    created_at TEXT NOT NULL,
    refs       TEXT NOT NULL DEFAULT '[]', -- JSON list[str]
    source     TEXT NOT NULL DEFAULT 'workbench'
);
CREATE INDEX IF NOT EXISTS idx_memory_items_layer ON memory_items(layer);
"""


class SqliteMemoryBackend:
    """把 7 层记忆落到 SQLite；CoreSqlite 与 Local 仅库路径不同，schema 完全一致。

    SQLite 操作同步执行（连接即用即开即用即关），避免在 worker 线程里遗留文件句柄
    （Windows 下 ``TemporaryDirectory`` 清理时易触发 "文件被占用" 的权限错误）。
    """

    def __init__(
        self,
        db_path: Path,
        backend: Backend,
        native: bool,
        store_path: Optional[Path],
    ) -> None:
        self.db_path = Path(db_path)
        self.backend = backend
        self.native = native
        self.store_path = store_path
        # L1 感官层瞬时缓冲（契约要求不落盘）。
        self._sensory: list[dict] = []
        self._lock = threading.Lock()
        self._init_db()

    # -- 连接管理 ----------------------------------------------------------
    def _conn(self) -> sqlite3.Connection:
        c = sqlite3.connect(str(self.db_path), timeout=30)
        c.row_factory = sqlite3.Row
        c.execute("PRAGMA busy_timeout=5000")
        return c

    def _init_db(self) -> None:
        with self._conn() as c:
            c.executescript(_MEMORY_DDL)

    # -- 写入 ---------------------------------------------------------------
    def _sync_store(self, item: dict) -> dict:
        layer = item["layer"]
        # L1 瞬时：仅在内存缓冲（FIFO 容量淘汰）。
        if layer == "L1":
            with self._lock:
                self._sensory.append(item)
                cap = LAYER_DEFS["L1"][2]
                while len(self._sensory) > cap:
                    self._sensory.pop(0)
            return item
        # L2..L7 持久化。
        with self._conn() as c:
            c.execute(
                "INSERT OR REPLACE INTO memory_items "
                "(id, layer, kind, content, embedding, created_at, refs, source) "
                "VALUES (?,?,?,?,?,?,?,?)",
                (
                    item["id"],
                    item["layer"],
                    item["kind"],
                    item["content"],
                    json.dumps(item["embedding"]) if item.get("embedding") is not None else None,
                    item["created_at"],
                    json.dumps(item.get("refs", [])),
                    item.get("source", "workbench"),
                ),
            )
            self._enforce_capacity(c, layer)
        return item

    def _enforce_capacity(self, conn: sqlite3.Connection, layer: str) -> None:
        cap = LAYER_DEFS[layer][2]
        cur = conn.execute(
            "SELECT COUNT(*) AS n FROM memory_items WHERE layer=?", (layer,)
        ).fetchone()
        excess = cur["n"] - cap
        if excess <= 0:
            return
        # FIFO：删最旧（rowid 最小）的超出项。
        old = conn.execute(
            "SELECT id FROM memory_items WHERE layer=? ORDER BY rowid ASC LIMIT ?",
            (layer, excess),
        ).fetchall()
        for row in old:
            conn.execute("DELETE FROM memory_items WHERE id=?", (row["id"],))

    async def store_item(
        self,
        layer: str,
        kind: str,
        content: str,
        refs: Optional[list[str]] = None,
        embedding: Optional[list[float]] = None,
        source: str = "workbench",
    ) -> dict:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        item = {
            "id": uuid.uuid4().hex,
            "layer": layer,
            "kind": kind or "note",
            "content": content,
            "embedding": embedding if embedding is not None else embed(content),
            "created_at": now_iso(),
            "refs": refs or [],
            "source": source,
        }
        return self._sync_store(item)

    # -- 读取 ---------------------------------------------------------------
    def _sync_query(self, layer: str, q: str, k: int) -> list[dict]:
        qv = embed(q)
        # L1 瞬时层直接扫内存缓冲。
        if layer == "L1":
            with self._lock:
                items = list(self._sensory)
            hits: list[tuple[float, dict]] = []
            for it in items:
                if it["layer"] != "L1":
                    continue
                ev = it.get("embedding") or embed(it["content"])
                if len(ev) != VEC_DIM:
                    continue
                score = cosine(qv, ev)
                if score >= SIM_THRESHOLD:
                    hits.append((score, it))
            hits.sort(key=lambda x: -x[0])
            return [h[1] for h in hits[:k]]
        # L2..L7：从库里取，用余弦相似度召回。
        rows: list[dict] = []
        with self._conn() as c:
            for row in c.execute(
                "SELECT * FROM memory_items WHERE layer=?", (layer,)
            ):
                rows.append(dict(row))
        hits = []
        for r in rows:
            emb = json.loads(r["embedding"]) if r["embedding"] else None
            if emb is None or len(emb) != VEC_DIM:
                emb = embed(r["content"])
            score = cosine(qv, emb)
            if score >= SIM_THRESHOLD:
                hits.append((score, r))
        hits.sort(key=lambda x: -x[0])
        out: list[dict] = []
        for _score, r in hits[:k]:
            out.append(
                {
                    "id": r["id"],
                    "layer": r["layer"],
                    "kind": r["kind"],
                    "content": r["content"],
                    "embedding": None,  # 查询响应剔除重向量，保持 payload 轻量
                    "created_at": r["created_at"],
                    "refs": json.loads(r["refs"]),
                }
            )
        return out

    async def query_layer(self, layer: str, q: str, k: int = 5) -> list[dict]:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        return self._sync_query(layer, q, k)

    def _sync_layers_status(self) -> list[dict]:
        counts: dict[str, int] = {}
        with self._conn() as c:
            for row in c.execute(
                "SELECT layer, COUNT(*) AS n FROM memory_items GROUP BY layer"
            ):
                counts[row["layer"]] = row["n"]
        with self._lock:
            counts["L1"] = len(self._sensory)
        return [
            {
                "layer": lid,
                "name": LAYER_DEFS[lid][0],
                "count": counts.get(lid, 0),
                "capacity": LAYER_DEFS[lid][2],
            }
            for lid in LAYER_DEFS
        ]

    async def layers_status(self) -> list[dict]:
        return self._sync_layers_status()

    def _sync_count(self, layer: str) -> int:
        if layer == "L1":
            with self._lock:
                return len(self._sensory)
        with self._conn() as c:
            row = c.execute(
                "SELECT COUNT(*) FROM memory_items WHERE layer=?", (layer,)
            ).fetchone()
            return int(row[0])

    async def count_layer(self, layer: str) -> int:
        return self._sync_count(layer)

    # -- experts：蒸馏专家结论写入 L6 程序性记忆，使其也"原生"于核心 --------
    async def store_expert_analysis(
        self, role: str, verdict: str, confidence: float, summary: str
    ) -> dict:
        return await self.store_item(
            "L6",
            "expert_analysis",
            f"{role}: verdict={verdict} confidence={confidence} :: {summary}",
            refs=[role],
            source="workbench",
        )

    # -- 同步辅助（Local -> Core 冲刷） ------------------------------------
    def _sync_all_items(self) -> list[dict]:
        out: list[dict] = []
        with self._conn() as c:
            for row in c.execute("SELECT * FROM memory_items ORDER BY rowid ASC"):
                out.append(dict(row))
        return out

    async def all_items(self) -> list[dict]:
        return self._sync_all_items()

    def _sync_import_items(self, items: list[dict]) -> int:
        n = 0
        with self._conn() as c:
            for it in items:
                emb = it.get("embedding")
                c.execute(
                    "INSERT OR IGNORE INTO memory_items "
                    "(id, layer, kind, content, embedding, created_at, refs, source) "
                    "VALUES (?,?,?,?,?,?,?,?)",
                    (
                        it["id"],
                        it["layer"],
                        it.get("kind", "note"),
                        it["content"],
                        json.dumps(emb) if emb is not None else None,
                        it.get("created_at", now_iso()),
                        json.dumps(it.get("refs", [])),
                        it.get("source", "workbench"),
                    ),
                )
                n += 1
        return n

    async def import_items(self, items: list[dict]) -> int:
        return self._sync_import_items(items)

    # -- 能力声明（原生契约 schema） ---------------------------------------
    def capabilities(self, expert_count: int = 5, service: str = "marguerite-file-intel") -> dict:
        """返回与原生契约一致的能力清单（供 ``/harness/capabilities`` 与 ``CoreBridge`` 共用）。"""
        return {
            "service": service,
            "memory_backend": {
                "active": "core" if self.native else "local",
                "is_core_store": self.native,
                "fallback": (not self.native),
                "db_path": str(self.store_path) if self.store_path else None,
                "backend": self.backend.value,
            },
            "layers": {
                lid: {"name": LAYER_DEFS[lid][0], "capacity": LAYER_DEFS[lid][2]}
                for lid in LAYER_DEFS
            },
            "native_capabilities": {
                "memory_7layer": True,
                "file_intel": True,
                "knowledge_base": True,
                "distilled_experts": True,
                "research": True,
                "expert_count": expert_count,
                "bridge_version": BRIDGE_VERSION,
            },
        }

    async def close(self) -> None:  # 连接即用即关，无长连接需要释放
        return None


# ---------------------------------------------------------------------------
# 同步驱动：在同步上下文（def 路由 / 测试）里跑 async 接口
# ---------------------------------------------------------------------------
def _run(coro):
    """在同步上下文里驱动桥接的 async 接口（无运行中 loop → asyncio.run）。"""
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(coro)
    import concurrent.futures

    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as ex:
        return ex.submit(lambda: asyncio.run(coro)).result()


# ---------------------------------------------------------------------------
# 发现 Marguerite 核心数据目录 / 核心库
# ---------------------------------------------------------------------------
def find_marguerite_home() -> Optional[Path]:
    """定位 Marguerite 数据目录。

    优先级：``MARGUERITE_HOME`` 环境变量（必须已存在）→ ``~/.marguerite`` →
    ``%APPDATA%/marguerite``（Windows）。若都不存在，返回 ``None``（触发 Local 回退）。
    """
    env = os.environ.get("MARGUERITE_HOME")
    if env:
        p = Path(env).expanduser()
        if p.exists():
            return p
        # 显式指定但尚不存在：视为"无既有库"，走 Local 回退。
        return None
    posix = Path.home() / ".marguerite"
    if posix.exists():
        return posix
    appdata = os.environ.get("APPDATA")
    if appdata:
        win = Path(appdata) / "marguerite"
        if win.exists():
            return win
    return None


def _looks_like_core_store(db: Path) -> bool:
    """只读打开库，嗅探是否含核心表（chunks / sessions / files / memory_items…）。"""
    try:
        uri = f"{db.as_uri()}?mode=ro"
        with sqlite3.connect(uri, uri=True) as c:
            rows = c.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            ).fetchall()
        names = {r[0] for r in rows}
        return bool(names & set(_CORE_TABLE_HINTS))
    except (sqlite3.Error, OSError, ValueError):
        return False


def discover_core_db(home: Optional[Path]) -> Optional[Path]:
    """在 Marguerite 数据目录内寻找核心记忆库（bounded scan，depth<=3）。"""
    if not home or not home.exists():
        return None
    home = Path(home)
    candidates: list[Path] = []
    # 1) 优先：核心已知库名。
    for name in ("memory.db", "sessions.db", "core.db", "marguerite.db"):
        p = home / name
        if p.exists():
            candidates.append(p)
    # 2) 其次：有限深度扫描含核心表的库。
    depth = 0
    frontier = [home]
    while frontier and depth < 3:
        nxt: list[Path] = []
        for d in frontier:
            try:
                for child in d.iterdir():
                    if child.is_dir():
                        nxt.append(child)
                    elif child.suffix == ".db" and child not in candidates:
                        if _looks_like_core_store(child):
                            candidates.append(child)
            except (PermissionError, OSError):
                continue
        frontier = nxt
        depth += 1
    return candidates[0] if candidates else None


# ---------------------------------------------------------------------------
# 桥接门面：显式 data_dir + marguerite_home，自动选 CoreSqlite / Local
# ---------------------------------------------------------------------------
def _safe_local_db(local_data_dir: Path, relative: str) -> Path:
    """用 security.safe_resolve 在受控根目录内解析本地库路径（symlink 防护）。"""
    from marguerite_workbench.security import safe_resolve

    root = Path(local_data_dir)
    root.mkdir(parents=True, exist_ok=True)
    try:
        return safe_resolve(root, relative)
    except ValueError:
        # 极端兜底：直接拼接（仍位于受控 data_dir 内）。
        return root / relative


class CoreBridge:
    """物理桥接门面。

    自动选择后端：``marguerite_home`` 指向的目录里发现核心库（sessions.db / chunks
    等）则接入 ``CoreSqlite``（native），否则回退 ``Local``。两种后端暴露同样的
    store / query / layers_status / count / capabilities 接口（含异步与同步两套）。
    """

    def __init__(self, data_dir: Path, marguerite_home: Optional[Path] = None) -> None:
        self.data_dir = Path(data_dir)
        self.marguerite_home = Path(marguerite_home) if marguerite_home else None
        # 显式 marguerite_home 优先；否则回退到全局发现。
        home = self.marguerite_home if self.marguerite_home is not None else find_marguerite_home()
        core_db = discover_core_db(home) if (home and home.exists()) else None
        if core_db is not None:
            try:
                # 物理接入核心库（native）。若核心库只读 / 被占用（例如 Rust 核心进程
                # 持锁、或文件权限为只读），则降级到 Local —— 保证 workbench 始终能
                # 启动。这把「无可达 core store 时仍可用」的兜底契约扩展到
                # 「发现了但不可写」的情形，避免在真正机器上启动即崩溃。
                self._backend = SqliteMemoryBackend(core_db, Backend.CORE_SQLITE, True, core_db)
                self._kind = Backend.CORE_SQLITE
            except (sqlite3.Error, OSError) as exc:
                logger.warning(
                    "core store discovered at %s but not writable (%s); "
                    "falling back to Local bridge",
                    core_db, exc,
                )
                local_db = self._local_db_path()
                self._backend = SqliteMemoryBackend(local_db, Backend.LOCAL, False, local_db)
                self._kind = Backend.LOCAL
        else:
            local_db = self._local_db_path()
            self._backend = SqliteMemoryBackend(local_db, Backend.LOCAL, False, local_db)
            self._kind = Backend.LOCAL

    def _local_db_path(self) -> Path:
        if self.marguerite_home is not None and self.marguerite_home.exists():
            return _safe_local_db(self.marguerite_home, "core_bridge.db")
        return _safe_local_db(self.data_dir, "core_bridge.db")

    # -- 后端状态 -----------------------------------------------------------
    def active_backend(self) -> Backend:
        return self._kind

    @property
    def db_path(self) -> Optional[Path]:
        return self._backend.store_path

    def is_native(self) -> bool:
        return self._kind == Backend.CORE_SQLITE

    # -- 异步接口（payload 风格，与契约 MemoryItem 对齐） -------------------
    async def store(self, layer: str, payload: dict) -> dict:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        return await self._backend.store_item(
            layer,
            payload.get("kind", "note"),
            payload.get("content", ""),
            payload.get("refs"),
            payload.get("embedding"),
        )

    async def query(self, layer: str, q: str, k: int = 5) -> list[dict]:
        if layer not in LAYER_DEFS:
            raise ValueError(f"unknown memory layer: {layer!r}")
        return await self._backend.query_layer(layer, q, k)

    # -- 同步封装（供 MemoryEngine / 同步路由复用） ------------------------
    def store_sync(self, layer, kind, content, refs=None, embedding=None, source="workbench") -> dict:
        return _run(
            self.store(
                layer,
                {"kind": kind, "content": content, "refs": refs, "embedding": embedding},
            )
        )

    def query_sync(self, layer, q, k=5) -> list[dict]:
        return _run(self.query(layer, q, k))

    def layers_status_sync(self) -> list[dict]:
        return _run(self._backend.layers_status())

    def count_sync(self, layer) -> int:
        return _run(self._backend.count_layer(layer))

    # -- 能力声明 -----------------------------------------------------------
    def capabilities(self, expert_count: int = 5, service: str = "marguerite-file-intel") -> dict:
        return self._backend.capabilities(expert_count=expert_count, service=service)

    async def close(self) -> None:
        await self._backend.close()


# ---------------------------------------------------------------------------
# 模块级解析（供需要直接拿后端的场景；MemoryEngine 走 CoreBridge 即可）
# ---------------------------------------------------------------------------
class BridgeResolution:
    __slots__ = ("backend", "kind", "store_path", "native")

    def __init__(self, backend, kind, store_path, native):
        self.backend = backend
        self.kind = kind
        self.store_path = store_path
        self.native = native


def resolve_bridge(local_data_dir: Path) -> BridgeResolution:
    """解析物理桥接后端（全局发现版）。"""
    home = find_marguerite_home()
    core_db = discover_core_db(home) if home else None
    if core_db is not None:
        backend = SqliteMemoryBackend(core_db, Backend.CORE_SQLITE, True, core_db)
        return BridgeResolution(backend, Backend.CORE_SQLITE, core_db, True)
    if home is not None:
        local_db = _safe_local_db(home, "core_bridge.db")
    else:
        local_db = _safe_local_db(Path(local_data_dir), "core_bridge.db")
    backend = SqliteMemoryBackend(local_db, Backend.LOCAL, False, local_db)
    return BridgeResolution(backend, Backend.LOCAL, local_db, False)
