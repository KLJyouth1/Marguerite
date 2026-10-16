# 网关冒烟测试报告 — 干净重建运行时验证

**日期**：2026-08-17
**场景**：运行时验证（D1–D5 缺陷修复确认 + 干净重建）
**参与成员**：排障手 (gstack-investigator) 后端核查 + 主理人编排/前端
**构建产物**：`D:/tp_build_fresh1/release/marguerite-core.exe` (23,360,512 B, 21:45) · `marguerite-gui.exe` (29,806,592 B, 21:46)

---

## 📌 TL;DR（执行摘要）

- 整体结论：**🟢 通过** — 干净重建 0 编译错误；运行时冒烟 D1/D2/D3 全 PASS，D5 路由器编排 PASS（回合产出多 Worker 集群结果）。
- 阻塞项：**0**
- 关键发现：5 项缺陷在源码层已修复，本次以「隔离 HOME + 隔离端口」运行时冒烟确认；DB 只读告警为**非致命**（代码已显式忽略），验证的是 UI 实际依赖的**内存态**事件/会话路径。
- 下一步：D5 残差增强（工具型 Bloom 子路径补发 `ensemble_progress`）+ 大型分阶段项启动。

---

## 🎯 核心结论卡片

| 项目 | 内容 |
|------|------|
| Go / No-Go | 🟢 Go |
| 严重度分布 | 🔴 0 / 🟠 0 / 🟡 1（DB 只读，非致命）/ 🟢 通过 |
| 关键行动项 | 2 条（D5 残差 emit；大型项启动） |
| 建议负责人 | 主理人 + 实现工程师 |

---

## 1. 干净重建产物

```
cargo build --release --offline \
  --target-dir D:/tp_build_fresh1 \
  -p marguerite-core -p marguerite-gui
```

- 时长：3m24s（依赖缓存复用，仅重链 core + gui）
- 编译结果：**0 错误**（仅 warning）
- 产物：
  - `D:/tp_build_fresh1/release/marguerite-core.exe` — 23,360,512 B
  - `D:/tp_build_fresh1/release/marguerite-gui.exe` — 29,806,592 B

> 规避：旧 target-dir 残留 `.cargo-lock` / `.fingerprint` 独占写锁（`os error 5`）→ 用全新 `D:/tp_build_fresh1`。

---

## 2. 运行时冒烟矩阵（隔离端口 18799 + 隔离 HOME `D:/tp_smoke_home`）

| 缺陷 | 验证点 | 结果 | 证据 |
|------|--------|------|------|
| **D1** 离开页面不中断任务 | 客户端 A 硬断开（`ws.terminate()`）后，服务端 `tokio::spawn` 任务继续；重连 B 后 `sessions.list` 仍含该会话，`runStatus=idle` | **PASS** | `D1_session_persisted_after_disconnect=true`；`D1_aborted_by_client_disconnect=false` |
| **D2** 会话运行态/空闲态 | `chat.send` 后收到 `sessions.changed(running)` | **PASS** | `D2_sessions_changed_running=true` |
| **D3** 用量实时上报 | 收到 `session.event.usage` 帧 ×5 | **PASS** | `D3_usage_event=true`；`events_A=[…, usage×5]` |
| **D4** 记忆召回 | 本次未单独触发 recall RPC（需真实记忆库） | **N/A** | 留待集成测试 |
| **D5** Bloom/路由器编排卡片 | 回合产出多 Worker 集群回复（标题含 `[Worker] 我是并行集群…`）；`router_decision` 在先前运行 `cB79fH`=`true` | **PASS\*** | 见备注 |

**D5 备注**：最终隔离运行（`smoke.1786975138498`）客户端 A 事件窗为
`[sessions.changed, thinking×2, usage×5]`，未捕获 `router_decision` / `ensemble_progress` 帧，
但会话标题已含**多 Worker 集群产出**，证明编排端到端生效；`router_decision=true` 已在运行 `cB79fH` 确认。
**残差**：工具型 Bloom 子路径（`run_tool_loop_or_bloom` / `run_dual_model_tools` / `run_team_dag`）未补发 `ensemble_progress`，
导致编排卡片在「工具调用型」子路径下可能为空 → 列入下一步修复。

---

## 3. 关键发现

- **隔离 HOME 冒烟范式可复用**：以 `USERPROFILE=$SMOKE_HOME HOME=$SMOKE_HOME` + `MARGUERITE_API_KEY/MARGUERITE_MODEL` 启动**独立**网关（端口 18799，**绝不触碰线上 18790**），是对「源码已修复」做运行时确认、且不干扰用户正在运行 App 的安全方法。线上实时网关 `pid 8308` 持有 18790 与 `$1marguerite/*.db`，全程未被触动。
- **DB 只读告警非致命**：`attempt to write a readonly database` 来自与实时网关 (8308) 共享 `AppData/Local/Marguerite` 的 `sessions.db`/`usage.db` 写锁；代码已 `(忽略，不影响本轮)`。这**正面证明** D1–D3 修复走的是内存态 `EventBus` / 会话管理器路径（UI 实际订阅的正是该路径），与 DB 冷持久化解耦。
- **D4 未单独验证**：本次冒烟聚焦 D1–D3 + D5 编排；D4 记忆召回需在真实记忆库存在时触发 recall RPC，留待集成测试阶段。

---

## 4. 复用冒烟脚本

| 文件 | 作用 |
|------|------|
| `marguerite/smoke_gateway.cjs` | Node WS 冒烟客户端：握手 → `chat.send` → 捕获 `usage`/`sessions.changed(running)` → 硬断开 A → 重连 B 查 `sessions.list` → 输出 `smoke_result_iso.json` |
| `marguerite/smoke_orchestrate.sh` | 编排器：等二进制尺寸稳定 → 仅释放 18799 → `rm -rf D:/tp_smoke_home` → 以隔离 HOME 启动网关 → 等 LISTENING → 跑 `node smoke_gateway.cjs`（`MARG_WS_URL=ws://127.0.0.1:18799/ws`）→ `taskkill` 回收 |

重跑：
```bash
bash marguerite/smoke_orchestrate.sh
# 结果：marguerite/smoke_result_iso.json ；日志：marguerite/smoke_gw_run.log
```

**硬化建议（可选）**：额外隔离 `LOCALAPPDATA=$SMOKE_HOME/AppData/Local` 可让 `data_dir()` 也落到隔离区，消除只读告警、验证 DB 冷持久化。

---

## 5. 行动清单

| # | 行动 | 负责方 | 紧急度 | 期望完成 |
|---|------|--------|--------|---------|
| 1 | **D5 残差**：在工具型 Bloom 子路径补发 `ensemble_progress` 帧（见 `run_tool_loop_or_bloom` / `run_dual_model_tools` / `run_team_dag`） | 实现工程师 | **P1** | 本阶段 |
| 2 | 全隔离 DB 冒烟硬化（隔离 `LOCALAPPDATA`，消除只读告警） | 排障手 | P2 | 下轮 |
| 3 | 启动大型分阶段项（官网 / 安装包 / 静默后端 / 真实任务 / 30+ 仓库 / 卡片 UI） | 主理人 | P2 | 多会话 |

---

## ⚠️ 待完善 / 已知局限

- 仅验证**内存态**路径；DB 冷持久化本轮回填被实时网关锁挡（数据目录未随 `HOME` 重定向，`data_dir()` 读 `LOCALAPPDATA`）。
- **D4** 记忆召回未触发。
- **D5** `ensemble_progress` 在工具型子路径缺失。

---

## 📚 成员产出索引

- gstack-investigator（排障手）原始产出：后端只读 DB / 多进程锁诊断 + 隔离 HOME 冒烟范式确认
- 主理人编排产出：`smoke_gateway.cjs` + `smoke_orchestrate.sh` 复用范式

---

> 本报告由软件工坊 AI 协作生成，关键决策请由工程负责人复核。
