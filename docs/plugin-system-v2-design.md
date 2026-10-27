# 插件系统 v2 设计补充

> 2026-10-24。本文只描述**现状事实、真实缺口与实施顺序**，不修改任何并发中的代码。
> 目标读者：后续实现者与评审者。

## 1. 现状（代码事实）

插件相关能力目前分成**两个互不统一的层**：

| 层 | 入口 | 数据模型 | 观测面 |
|---|---|---|---|
| 声明式装配层 | `PluginManifest` + `PatchLayer` + `PluginSlot` | 清单（name/version/capabilities/hooks/skills/activation/permissions…） | `plugins.status`（`gateway/plugin_status.rs`）→ `manifests / slots / report / summary` |
| DI 运行层 | `plugins/di.rs` | `PluginRegistryEntry { name, version, entry, enabled, depends_on, source }` + `PluginRegistryFile` | **无 RPC**；仅 `plugins.list` 间接暴露 |

- 发现：`discover_symlinks(plugins_root, include_local_fallback)` 扫描 `plugins_root/managed` 的
  symlink（BepInEx drop-in 风格），非 symlink 作为 local fallback（`di.rs:302`）。
- 装配：`assemble()` 读 `plugin_manifest_dir()` 的清单 + `patches_dir()` 的 patch，
  `apply_layers()` 装配进 `PluginSlot`；失败**响亮 warn 但降级**（`plugin_status.rs:63`）。
- 加载：`PluginLoader::load_enabled` 按注册表拓扑注入 DI；`reload_plugin` 是
  「整体 unload → 清空 applied TypeId → 重新 load_enabled」（`di.rs:645`）。
- 红线（不可绕过）：
  1. 禁止全上下文重注入：清单含 `system_prompt` / `full_context` 即进 `context_leak_suspects`；
  2. 禁止中途替换工具面：`guarded_provide` 对已 applied 的 TypeId 返回 Err，
     只有显式 `reload_plugin` 例外。
- 前端：`PluginsView.vue` 同时调用 `plugins.list` 与 `plugins.status`。

## 2. 真实缺口（按优先级）

1. **`plugins.list` 名不副实**：`gateway/fleet.rs:126 plugins_list()` 实际返回
   `domain::capabilities::registry()` 的**能力目录**（name/description/group/tier/maturity），
   不是已安装插件。用户看到的是「能力」而非「插件」，与 `plugins.status` 的清单是两套东西。
2. **两个注册表不统一**：DI 注册表（enabled/depends_on/entry）与声明式清单（assembled/slots）
   没有任何关联字段；一个插件可能在清单里已装配、在注册表里不存在，反之亦然，且**不可观测**。
3. **热启停无对外入口**：`disable_plugin` / `reload_plugin` 已实现，但 `SUPPORTED_METHODS`
   只有 `plugins.list` / `plugins.status`，UI 无法真正启停。
4. **插件技能与技能加载器未汇合**：`PluginSlot` 已把清单里的 skill 槽装进统一槽位
   （`slot.rs` 的 tool/skill 统一路由），但 `SkillLoader`（prompt 注入侧）只扫 `skills/`
   目录，**不消费**插件的 skill 槽；两条技能通道各自为政，用户看到的技能列表与插件声明不一致。
5. **损坏链接/缺失入口/依赖环没有统一报告**：`discover_symlinks` 对坏链只 warn 后跳过；
   `apply_layers` 的环/缺失依赖进 `report.errors`，但注册表侧的问题不在这份报告里。
6. **没有覆盖率/健康快照**：技能侧已有 `skills.coverage`（递归发现 + 跳过原因），
   插件侧没有对应物，排障只能看日志。

## 3. 目标与非目标

**目标**
- 一套**统一插件清单**（合并注册表条目 + 声明式清单 + 市场来源），可查询、可解释。
- 只读观测面 `plugins.coverage`：发现数 / 已装配 / 启用 / 停用 / 坏链 / 缺失入口 /
  依赖问题 / 红线嫌疑 / 跳过原因。
- 设计上为 `plugins.setEnabled` 与 `plugins.reload` 预留语义（不在本轮实现）。

**非目标（本轮）**
- 不改 `di.rs` 的注册表写入与 `reload_plugin` 语义（并发 WIP，且红线敏感）。
- 不做第三方 DLL/WASM 的沙箱隔离。
- 不改 `plugins.list` 的既有返回字段（前端 `PluginsView` 依赖它）。

## 4. 统一模型

```text
PluginRecord {
  id: String,                 // 注册表 name / 清单 name（同名视为同一插件）
  version: String,
  sources: ["manifest" | "registry" | "market"],
  enabled: bool,              // 注册表 enabled
  assembled: bool,            // 清单是否真装配进 PluginSlot
  entry: Option<String>,      // 原生入口
  activation: "restart" | "hot",
  slots: [String],            // 该清单产出的槽位
  skills: [String],           // 清单声明的技能
  dependsOn: [String],
  errors: [String],           // 坏链 / 缺入口 / 环 / 清单解析失败
  redline: "none" | "context-leak"
}
```

合并规则（确定性）：
1. 以 `id` 为键，清单与注册表取并集；
2. `sources` 去重并按字典序；
3. 任一来源报错都进 `errors`，**不静默丢弃**；
4. `redline` 一旦为 `context-leak`，`enabled` 视为 `false`（启动时必须拒绝启用或 require-ack）。

## 5. RPC 契约（新增）

`plugins.coverage` → 只读，不触发装配

```json
{
  "ok": true,
  "root": "<plugins_root>",
  "registry": { "entries": 3, "enabled": 2, "disabled": 1, "missingEntries": 0, "brokenLinks": 0 },
  "managed":  { "discovered": 5, "symlinks": 4, "localFallback": 1 },
  "manifests": { "total": 16, "assembled": 14, "patches": 7, "errors": 0 },
  "redlines": { "contextLeakSuspects": ["..."] },
  "plugins": [ PluginRecord, ... ],
  "gaps": ["registry-only:x", "manifest-only:y"],
  "errors": [ { "path": "...", "reason": "broken-symlink" } ]
}
```

- 全部数字来自真实扫描/装配快照，无数据即 0，不编造百分比。
- 错误三段式：不可用 → `{ ok:false, error, code:"PLUGINS_UNAVAILABLE" }`。

## 6. 生命周期（目标态）

```text
discover(managed + registry)
  → validate(manifest JSON / 依赖 / 红线)
  → assemble(PluginSlot + PatchLayer)
  → load(DI, 拓扑 + guarded_provide)
  → observe(plugins.status + plugins.coverage)
  → reload/disable(受红线约束)
```

约束：
- 发现阶段**只读**，不得因观测触发写入或热重载；
- `enabled=false` 的插件可以出现在 coverage 中，但不得进入 `load_enabled`；
- 红线嫌疑插件在 coverage 中必须显式标红，且不得被静默启用。

## 7. 兼容与迁移

- 保留 `plugins.list` / `plugins.status` 的字段与语义，**不改 wire 结构**；
- 新 RPC 只做增量；前端新页面/面板订阅新 RPC，不替换旧调用；
- 若后续统一清单，先加字段（`sources` / `assembled` / `errors`），再迁移 UI，
  最后才考虑把 `plugins.list` 改名为 `capabilities.list`（需要单独评审）。

## 8. 测试计划

1. 纯函数：合并规则（并集/去重/确定性排序）、红线降级、errors 聚合。
2. 临时目录：正常 symlink、坏链、缺失 entry、清单解析失败、依赖环、
   registry-only / manifest-only 两类 gap。
3. 端到端：`plugins.coverage` 在真实 `plugins_root` 上返回计数，且**不触发**
   `reload_plugin`（用装配快照的 `assembled` 前后不变断言）。
4. 并发安全：测试使用独立临时目录，绝不复用用户 `plugins_root`。

## 9. 实施顺序

- **P0（已完成，2026-10-24）**：`plugins.coverage` + `plugins.setEnabled` + `plugins.reload`
  （`gateway/plugins_runtime.rs` / `gateway/plugins_rpc.rs`）＋ 10 条单测 + 前端只读区块。
- **P1（已完成）**：`PluginsView` 新增「热启停与覆盖率」区块（启停开关 / 重载 / 红线禁用 /
  来源缺口 / 诚实边界说明），6 语言键齐全。
- **P2（已完成）**：`plugins.status` 的 `manifests[].enabled` 与 `summary.disabled_plugins`
  反映注册表停用；停用插件立即从 `slots` 视图移除（热启停可观测）。
- **P3（未做）**：插件声明 skills 桥接（`PluginManifest.skills` → `SkillLoader` 增量发现）。

## 9.1 已落地实现（与本文设计的差异，以代码为准）

| 设计项 | 实现 | 说明 |
|---|---|---|
| `plugins.coverage` | ✅ `plugins_rpc::plugins_coverage` | 额外返回 `registryPath` / `skippedTotal` / `lastError` |
| `plugins.setEnabled` | ✅ `set_enabled` | 额外保证：落盘失败 → 回滚内存；重载失败 → 回滚标记并回写 |
| `plugins.reload` | ✅ `reload` | 走 `PluginLoader::reload_plugin`（红线例外仅限 reload） |
| `PluginRecord.effectiveEnabled` | ✅ | `enabled && !redline`，前端据此禁用开关 |
| `nativeLoaded` | ✅ 恒为 0 | 当前构建是 `NoopDynamicLoader`，字段诚实反映 |
| 整体重载原语 | ✅ `PluginLoader::reload_all` | 同时清空**插件专用**容器，避免「停用了但服务还在」 |

**未实现且不假装**：真实 DLL/WASM 动态加载；`plugins.setEnabled` 对 manifest-only 条目
返回 `PLUGIN_NOT_FOUND`（因为注册表里没有它，前端对这种行只显示「未安装到注册表」）。

## 9.2 红线守卫（已测试）

- `redline_plugin_cannot_be_enabled`：清单含 `system_prompt` → `PLUGIN_REDLINE_BLOCKED`；
- `toggle_persists_and_changes_load_order`：启停真实落盘，重开注册表可见；
- `unknown_plugin_is_not_found` / `reload_unknown_is_not_found`：错误码可机器校验；
- `coverage_merges_registry_and_manifests_with_gaps`：两源合并 + 缺口分类；
- `coverage_without_manifests_is_honest`：目录缺失 → 计数 0 + `patches.error` 可读；
- `set_enabled_rejects_empty_name_and_missing_bool`：参数校验先于运行时访问。

## 10. 风险与红线

- 注册表写入走 `persistence::write_json_atomic`（另一会话已落地原子写），本模块只调
  `PluginRegistryFile::save`，不自己拼写盘逻辑。
- `plugins.status` 的装配快照是 `OnceLock`，**重启生效**；coverage 的 `assembled` 也来自
  该快照，不是实时重装配 —— 因此 coverage 明确**不触发**装配。
- 用户可写状态（`managed/` + `registry.json`）落在 `Config::data_dir()/plugins`，
  与只读的 `resources/plugins/manifests` 分离，安装包升级不会冲掉用户启停状态。
- 不引入外部网络；不下载插件；观测路径不写盘。
- 「启用」必须先过红线检查，失败可读（`PLUGIN_REDLINE_BLOCKED`），不允许假成功。

---

参考：`core/src/plugins/di.rs`、`core/src/plugins/manifest.rs`、`core/src/gateway/plugin_status.rs`、
`core/src/gateway/fleet.rs`、`webui/src/views/PluginsView.vue`、`AGENTS.md §3.1`。
