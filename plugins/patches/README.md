# plugins/patches —— 插件补丁层（P0 S2）

> ⚠️ **本目录当前不参与实际装配（apply 见后续切片），仅做装载与校验。**
>
> 也就是说：这里放一份 `*.patch.json`，Marguerite 启动时**只会解析、校验、排序、打一行摘要日志**，
> **不会**把它真正装到任何槽位上。真正「装配进运行期组合」的能力是后续切片（S4）的职责。
> 正因为 apply 尚未实现，本目录**刻意不放任何真实的 `*.patch.json` 样例文件**，以免误导
> —— 样例文件只存在于测试的临时目录里（见 `core/tests/plugin_patch_gate.rs`）。

## 这个目录是什么

Marguerite 的插件底座长期是「**有骨架、无装配**」：

- `plugins/manifests/*.plugin.json` 是**安装态**声明（一个插件提供了什么能力 / 钩子 / 技能）；
- 但没有任何代码把清单**装配**进运行期组合 —— 即「装了不动」在旧版本里是**完全静默**的。

补丁层（patch layer）借鉴 DSH 的 `cordis.patch.yml` 形态，把两件事解耦：

| 概念 | 载体 | 含义 |
| --- | --- | --- |
| **安装** | `plugins/manifests/<name>.plugin.json` | 这个插件**存在**、提供什么能力 |
| **装配** | `plugins/patches/<xxx>.patch.json` | 这个插件**如何**被装进组合 |

一份 patch 文件只描述「往哪个槽位插什么 / 覆盖哪个配置 / 停用哪个条目」，不描述插件本体。

## 目录解析顺序

运行期按以下顺序找本目录，**命中即用**，全部不存在则视为「未找到 patch 目录」：

1. 环境变量 `MARGUERITE_PATCHES_DIR`（须为已存在的目录）；
2. `cwd/plugins/patches`；
3. 可执行文件**同目录**的 `plugins/patches`；
4. 可执行文件**上一级目录**的 `plugins/patches`；
5. 可执行文件同目录的 `resources/plugins/patches`（安装版资源布局）。

实现见 `core/src/plugins/patch.rs::patches_dir()`。

## 文件字段（`*.patch.json`）

文件名必须以 `.patch.json` 结尾；其余文件（如本 `README.md`）会被忽略。

```jsonc
{
  // [必填] schema 标识，必须严格等于 "marguerite.patch/v1"
  "schema": "marguerite.patch/v1",

  // [必填] 目标插件名，必须命中 plugins/manifests/<name>.plugin.json 的 name
  "target": "dsh-modlens",

  // [选填，默认 0] 装载层级，小者先装载
  "layer": 10,

  // [选填，默认 ["desktop"]] 生效的 profile 列表
  "profile": ["desktop"],

  // [选填，默认 []] 依赖的其它 patch target（同样是插件名）
  "requires": ["dsh-harness-core"],

  // [选填，默认 []] 装配操作序列
  "ops": [
    { "op": "insert", "slot": "tools", "id": "modlens.recognize", "order": 5, "label": "识图" },
    { "op": "set",    "key": "ui.theme", "value": "dark" },
    { "op": "disable", "id": "legacy.recognize" }
  ]
}
```

### `schema`

取值有且只有 **`"marguerite.patch/v1"`**。写成别的值会在校验阶段产生
`patch.bad_schema` 诊断（**不会被静默忽略**）。

### 三种 `op`

`op` 为标签字段（内部标签枚举），三种取值对应三种操作：

| `op` | 字段 | 含义 |
| --- | --- | --- |
| `insert` | `slot`（槽位名）、`id`（条目 id，必填非空）、`order`（选填，默认 0，小者先）、`label`（选填） | 往指定槽位插入一个条目 |
| `set` | `key`（配置键）、`value`（任意 JSON 值） | 覆盖某个配置键 |
| `disable` | `id`（条目 id） | 停用某个已装配条目 |

> `insert` 的 `id` 若为空（或纯空白）会产生 `patch.empty_id` 诊断。

## 排序语义（`order_layers`）

装载顺序由三级规则决定，**完全确定**（不依赖任何哈希容器的迭代序）：

1. **`layer` 升序** —— 数值小的先装载；
2. **同层按 `target` 字典序** —— 保证同层内顺序与文件系统 / 目录枚举顺序无关；
3. **`requires` 拓扑排序** —— 被依赖者的 target 一定排在使用者之前（即使它的 `layer` 更大）。

失败是**响亮**的，不静默降级：

- `requires` 指向一个**不存在的 target** → 整批 `Err`；
- `requires` 之间**成环** → 整批 `Err`（并在错误信息里列出未能放置的 target）。

## 诊断码

校验（`PatchLayer::validate`）与摘要（`summarize`）会产出如下诊断，启动日志逐条打印：

| 诊断码 | 触发条件 |
| --- | --- |
| `patch.bad_schema` | `schema` 不等于 `"marguerite.patch/v1"` |
| `patch.orphan_target` | `target` 没命中任何已装载的 manifest 名 |
| `patch.empty_id` | `insert` 的 `id` 为空（或纯空白） |
| `patch.missing_requirement` | `requires` 里有一项不是任何已知 manifest 名 |

## 本切片（P0 S2）做了什么 / 没做什么

**做了**（只读，不产生副作用）：

- `PatchLayer::load_dir` —— 扫目录、解析，**解析失败带文件名报错**，按文件名排序保证稳定；
- `PatchLayer::validate` —— 单层四类校验（上表）；
- `patch::order_layers` —— 三级排序 + 环 / 缺失依赖检测；
- `patch::summarize` —— 总数 + 按 profile 计数 + 汇总诊断；
- `patch::patches_dir` —— 目录解析；
- GUI（`gui/src-tauri/src/lib.rs`）在 `setup` 中装载一次并打印一行摘要；
  装载失败**不会**让启动失败（仅 `eprintln!` 告警）。

**没做**（后续切片）：

- **没有** apply：不往 `PluginSlot` / `PluginRegistry` 里塞任何东西；
- **没有**新增 RPC / 不改 `GatewayState`；
- **没有**改动 `core/src/gateway/server.rs`。

## 相关代码

| 位置 | 作用 |
| --- | --- |
| `core/src/plugins/patch.rs` | 补丁层本体（数据结构 + 装载 + 校验 + 排序 + 摘要） |
| `core/tests/plugin_patch_gate.rs` | 集成门禁（用临时目录自造输入，不依赖本目录有文件） |
| `plugins/manifests/*.plugin.json` | 安装态清单（`target` 的取值来源） |
