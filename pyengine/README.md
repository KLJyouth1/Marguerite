# OpenSquilla

Marguerite 桌面端的 **Python 侧车运行时**（microkernel agent runtime）。
由 Rust 核心网关（`core/src/pyengine/sidecar.rs`）以子进程方式拉起，
经 WebSocket 向桌面端提供会话、工具、记忆、路由与多渠道消息能力。

> 本文件是 `pyproject.toml` 里 `readme = "README.md"` 的落点。
> **缺它会让 `pip install -e .` 在 `hatchling` 读元数据阶段直接失败**
> （`AttributeError: ... readme`），进而导致这个包根本无法被安装、
> 开发环境不可复现。2026-10-22 实测确认并补上。

## 安装（开发环境）

```bash
cd pyengine
python -m venv .venv                 # 若 .venv 已存在可跳过
./.venv/Scripts/python.exe -m ensurepip --upgrade
./.venv/Scripts/python.exe -m pip install -e ".[dev,memory]"
```

Windows 下虚拟环境在 `.venv\Scripts\`，Linux/macOS 在 `.venv/bin/`。

**为什么是 `-e ".[dev,memory]"` 而不是 `-e .`**：
`dev` 带 pytest/mypy/ruff（跑测试与静态检查必需），`memory` 带 tiktoken/jieba
（分层记忆的召回与分词）。只装核心依赖时，相关测试会因缺包被跳过而不是失败，
容易误判成"测试通过"。

**已知坑（2026-10-22 实测）**：

- `pip install -e .` 曾因本文件缺失而失败
  （`hatchling` 读 `readme` 时抛 `AttributeError`）——**不要删本文件**。
- `jieba` 的 sdist 偶发**哈希不匹配**（本地缓存损坏）。此时加 `--no-cache-dir` 强制重下：
  ```bash
  ./.venv/Scripts/python.exe -m pip install --no-cache-dir "jieba>=0.42"
  ```
  若强制重下后仍不匹配，说明下载链路被中间代理改写，**应停止安装并排查代理**，
  不要用 `--no-deps` 或跳过校验绕过。

## 运行

```bash
# 网关（桌面端实际拉起的入口）
./.venv/Scripts/gateway.exe

# CLI
./.venv/Scripts/opensquilla.exe
```

## 测试

```bash
./.venv/Scripts/python.exe -m pytest tests/ -q
```

只跑单个文件（更快，且不依赖全部测试依赖）：

```bash
./.venv/Scripts/python.exe -m pytest tests/test_v4_phase3_degradation.py -q
```

## 依赖分层

| 层 | 声明位置 | 说明 |
|---|---|---|
| 核心依赖（41 项） | `pyproject.toml` → `[project].dependencies` | 网关与运行时必需 |
| `dev` | `[project.optional-dependencies].dev` | pytest / pytest-asyncio / mypy / ruff |
| `memory` | 同上 | tiktoken / jieba（分层记忆召回与分词） |
| `recommended` | 同上 | lightgbm / onnxruntime / scikit-learn 等。**桌面安装包不含内嵌运行时，缺这些属预期降级，不是安装损坏** |
| 其它 extras | `mcp` / `msg` / `matrix` / `document-extras` / `swebench` | 按需启用 |

全部装齐（核心 + 开发 + 记忆）：

```bash
./.venv/Scripts/python.exe -m pip install -e ".[dev,memory]"
```

## 目录结构（顶层）

| 目录 | 职责 |
|---|---|
| `src/opensquilla/gateway/` | HTTP/WS 网关、RPC 方法、静态控制台 |
| `src/opensquilla/agent/` | 智能体运行时、子代理、工具回路 |
| `src/opensquilla/squilla_router/` | 模型路由（含 v4_phase3 策略） |
| `src/opensquilla/session/` | 会话存储 |
| `src/opensquilla/memory/` | 分层记忆 |
| `src/opensquilla/tools/` | 内置工具实现 |
| `tests/` | pytest 套件 |

## 许可

Apache-2.0，见 `LICENSE`。
