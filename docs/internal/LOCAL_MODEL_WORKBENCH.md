# 本地模型工作台（Local Model Workbench）

> 模块位置：`core/src/gateway/local_models.rs`
> WebUI 面板：`webui/src/views/LocalModelsView.vue`
> RPC 注册：`core/src/gateway/methods.rs:219-232`、`core/src/gateway/protocol.rs:217-230`

---

## 1. 概述

本地模型工作台是 Marguerite（Rust 核心 `marguerite-core` + Vue 3 webui）中用于管理**本机 LLM 推理**的能力集合：聚合多方下载源、依据宿主机真实硬件智能推荐、一键下载/部署/启停、训练与微调（finetune）、把本地模型标记为路由中的 `kind=Local` 档案，以及在模型目录上落地「shipped gate 六要素」准入徽章。

设计哲学（对齐 `local_models.rs:1-13` 模块注释）：

- **档案化 / 可热更新**：目录为数据表（`local_catalog()`），部署台账持久化 `local_models.json`，运行时增删不重编译。
- **诚实工程（honest engineering）**：真正能下载/启动依赖 Ollama CLI 是否可用；不可用时如实上报并提供官方命令/URL，**绝不假装「已就绪」**。
- **announce ≠ deployable（发布 ≠ 可部署）的 shipped-gate 纪律**：目录项自带 shipped gate 六要素，把「这是一个被宣布存在、且经得起放行校验的模型」与「它此刻真的能跑」区分开。徽章只描述模型本身的放行属性（开放权重、许可证、评测、定价、量化、可复现），**不代表该宿主已下载或已部署**。是否已部署由台账 `deployed`/`status` 字段表达。
- **训练也遵守诚实边界**：仅当探测到可用训练后端才真正派发子进程；否则如实标记 `needs_backend` 并给出配置指引，**绝不假装一次训练已经跑完**。

---

## 2. 架构

### 2.1 后端 RPC 一览（`models.local.*`）

全部在 `methods.rs:219-232` 注册、`protocol.rs:217-230` 声明，分发见 `methods.rs:835-848`：

| RPC | 处理函数（文件:行） | 作用 |
|-----|---------------------|------|
| `models.local.catalog` | `local_catalog_rpc` (`local_models.rs:736`) | 返回聚合下载源（ollama/huggingface/modelscope）与全部目录项（含 shipped gate 六要素）。 |
| `models.local.recommend` | `local_recommend` (`local_models.rs:750`) | 按用途 + 宿主规格（或回退 `host_system_spec`）智能推荐，返回档位/预算/候选。 |
| `models.local.list` | `local_list` (`local_models.rs:777`) | 列出已纳入管理的本地模型台账。 |
| `models.local.download` | `local_download` (`local_models.rs:783`) | 一键下载意图：Ollama 可用则后台 `ollama pull`，否则返回官方命令/URL，并写入台账。 |
| `models.local.deploy` | `local_deploy` (`local_models.rs:842`) | 把本地模型注册为路由档案（`kind=Local`），写入 `models.json`，并标记台账 `deployed=true`。 |
| `models.local.start` | `local_set_started(..., true)` (`local_models.rs:916`) | 启停开关（须先 deploy）。 |
| `models.local.stop` | `local_set_started(..., false)` (`local_models.rs:916`) | 停止运行态。 |
| `models.local.remove` | `local_remove` (`local_models.rs:940`) | 从台账移除，可选同时移除路由档案。 |
| `models.local.status` | `local_status` (`local_models.rs:977`) | 整体状态：Ollama 可用性 + 台账 + 路由中 `kind=Local` 档案。 |
| `models.local.system` | `local_system` (`local_models.rs:1008`) | 返回宿主真实系统规格（RAM/核心/GPU，`source:"host"`）。 |
| `models.local.finetune` | `local_finetune` (`local_models.rs:1023`) | 发起一次本地微调（诚实编排）。 |
| `models.local.finetune.list` | `local_finetune_list` (`local_models.rs:1139`) | 列出所有微调任务。 |
| `models.local.finetune.status` | `local_finetune_status` (`local_models.rs:1146`) | 单个微调任务状态。 |
| `models.local.finetune.cancel` | `local_finetune_cancel` (`local_models.rs:1160`) | 取消微调任务（已派发的子进程尽力 kill pid）。 |
| `models.gate.check` | `gate_check` (`shipped_gate.rs:187`) | 按目录 id 实时校验 shipped gate 六要素（权重/许可/评测/定价/可用性/兼容性），返回 `passed`/`blocking`/`findings`。宿主事实 `weightsPresent`、`runtimeSupports` 由调用方据真实部署环境提供。 |
| `models.profiles.add` | `models_profiles_add` (`models.rs:125`) | 新增模型档案；**前移 shipped gate**：当档案指向一个已声明权重的本地/第三方模型（`catalogId` 显式命中，或 `kind=Local` 且 `model`/`id` 命中目录项）时，必须过 gate 才允许注册到路由（阻断码 `gate.blocked`）。纯云 API 端点不参与 gate。 |

### 2.2 数据流

```
                         models.local.system  ─┐
                                               ├─► recommend_models(intent, ram, gpu, os)
  WebUI / CLI ──RPC──► gateway::methods ──────┤
                                               ├─► local_catalog()（热扩展目录）
                                               └─► LocalModelStore / LocalFinetuneStore（JSON 台账）

  deploy ──► ModelSelector::upsert(ModelProfile{kind=Local}) ──► models.json（路由可见）
  start/stop ──► ManagedLocalModel.started ──► 仅标记；本地推理进程由用户/Ollama 服务管理
```

### 2.3 持久化文件

- `local_models.json`：位于 `Config::config_dir()` 同目录（即 `models.json` 同目录），由 `LocalModelStore` 读写（`local_models.rs:296-316`）。结构：`{ managed: BTreeMap<id, ManagedLocalModel> }`。
- `local_finetunes.json`：位于 `Config::config_dir()`，由 `LocalFinetuneStore` 读写（`local_models.rs:605-624`）。结构：`{ jobs: BTreeMap<id, FinetuneJob> }`。

### 2.4 部署即注册 `kind=Local` 路由档案

`local_deploy`（`local_models.rs:842-913`）会：

1. 在 `LocalModelStore` 中标记 `deployed=true`、`status="ready"`、`profile_id="local-<id>"`；
2. 通过 `ModelSelector::load()` 拿到注册表，构造 `ModelProfile` 并 `upsert`：
   - `id = "local-<id>"`，`label = "本地 · <label>"`；
   - `provider` 默认 `"ollama"`，`base_url` 默认 `"http://localhost:11434/v1"`（均可由参数覆盖）；
   - `model` 取 `ollama_tag`（无 Ollama 标签时取目录 `id`）；
   - **`kind = ProfileKind::Local`**（`local_models.rs:894`）；
   - `builtin=false`，`temperature=0.7`，`max_tokens=4096`。
3. `sel.save()` 落盘到 `models.json`。

此后该档案出现在 `models.local.status` 的 `local_profiles` 中，**路由可见、可在模型选择器中切换为主模型**。

> **关于 copilot（Bloom 副模型）**：`local_deploy` 本身**不会**自动把本地模型设为 copilot。注册完成后，可通过标准模型管理命令将其标记为 Bloom 副模型——`marguerite model copilot local-<id>`（`cli/program.rs:1206-1215`，底层 `ModelSelector::use_copilot`）。即「可选地标记为 copilot」是部署后的独立操作，而非 deploy 的隐式副作用。

---

## 3. 真实宿主探测（`host_system_spec`）

旧实现用浏览器 `navigator` 猜值（`deviceMemory` 封顶 8 GB、`hardwareConcurrency`、`webgl` 探 GPU），与运行 Marguerite 的真实机器脱节。本模块改为在**后端**用 `host_system_spec()`（`local_models.rs:548-559`）读取运行 Marguerite 的宿主真实规格，并在 `local_recommend` 中回退使用（`local_models.rs:750-774`），取代旧硬编码 `ram=16/gpu=false`。

`HostSpec`（`local_models.rs:458-469`）字段：`os`、`ram_gb`、`cores`、`gpu`、`gpu_name`、`source`（恒为 `"host"`）。

### 3.1 内存（RAM）按操作系统探测（零新增依赖）

`detect_ram_bytes()`（`local_models.rs:472-518`）：

- **Linux**：读 `/proc/meminfo` 的 `MemTotal`（`local_models.rs:473-484`）。
- **macOS**：`sysctl -n hw.memsize`（`local_models.rs:485-496`）。
- **Windows**：通过 **PowerShell CIM** `(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory`（`local_models.rs:497-516`）。**这里刻意不用 `windows` crate**——`Win32_System_Memory` 的 `GlobalMemoryStatusEx/MEMORYSTATUSEX` 在当前依赖版本（0.62.2）feature 下解析不到，故回退 PowerShell，依赖无关更稳（详见 2026-08-27 实现日志）。
- 兜底：探测失败返回保守值 `16 GiB`（`local_models.rs:517`）。

### 3.2 CPU 核心数

`detect_cores()`（`local_models.rs:521-523`）使用 `std::thread::available_parallelism()`（stable since 1.59，零依赖），失败兜底 4。

### 3.3 GPU 探测

`detect_gpu()`（`local_models.rs:526-545`）按顺序 PATH 探测：

- `nvidia-smi --query-gpu=name --format=csv,noheader`
- `rocm-smi --showproductname`

命中即返回 `(true, 显卡名)`；都不在 PATH 则 `(false, "")`。**诚实探测、不臆测**：没有就如实报「未检测到」。

> 这一套真实探测**取代**了前端 `navigator.deviceMemory`（封顶 8 G）、`webgl` 探针等浏览器近似，使「根据用户系统配置智能推荐」真正落到宿主硬件。

---

## 4. 聚合下载源与 shipped gate 六要素

### 4.1 聚合下载源

`local_catalog_rpc`（`local_models.rs:736-746`）返回 `sources` 数组，覆盖三类源：

| name | url | one_click |
|------|-----|-----------|
| ollama | https://ollama.com/library | true（后台 `ollama pull`） |
| huggingface | https://huggingface.co | false（返回仓库 URL 供手动下载） |
| modelscope | https://modelscope.cn | false（返回仓库 URL 供手动下载） |

目录项结构见 `LocalModelEntry`（`local_models.rs:28-54`），含 `ollama_tag`、`hf_repo`、`msc_repo` 三类拉取标识。

### 4.2 shipped gate 六要素（目录项放行徽章）

每个目录项带六个放行维度，定义在 `LocalModelEntry` 上（**实际字段名，来自代码**）：

- `weights: bool` — 开放权重（权重）
- `license: String` — 许可证（许可证）
- `eval: String` — 评测结论（评测）
- `pricing: String` — 定价（定价，目录默认值 `"本地免费（自付电费）"`）
- `quantized: bool` — 是否量化（量化）
- `reproducible: bool` — 是否可复现（可复现）

`to_json`（`local_models.rs:93-114`）将它们聚合进 `shipped_gate` 子对象返回给前端。WebUI 的 `gateBadges()`（`LocalModelsView.vue:232-242`）把它们渲染为六枚徽章：权重 / 许可 / 评测 / 定价 / 量化 / 可复现。

> **纪律重申（announce ≠ deployable）**：六要素描述「这个模型本身是否经得起放行」，并**不代表**用户本机已下载/已部署。是否已部署由台账 `deployed`、`status` 表达；目录项与台账是两层概念。`catalog_has_shipped_gate_six_elements` 测试（`local_models.rs:1208`）强制每个目录项六要素齐备（权重/量化/可复现为真，许可证/评测/定价非空，且必须有 HF 源）。

### 4.3 运行时 shipped gate 校验与注册前移

目录徽章（`4.2`）只描述「模型本身放行属性」。真正的放行校验在运行时由 `shipped_gate::gate_local_model`（`shipped_gate.rs:100`）计算，按本机宿主事实判定：

- `weights_present`：权重文件**实际已下载/存在**（而非仅「宣布」）——`false` 时 `Weights` 判 `Fail`（宣布 ≠ 可部署）。
- `runtime_supports`：本地运行时支持该模型格式/架构——`false` 时 `Compatibility` 判 `Fail`。

六要素 `Weights / License / Eval / Pricing / Availability / Compatibility` 中，任一 `Fail` 即 `passed=false`，禁止接入路由。`Warn`（如许可证未知、缺评测）不阻断，仅提示。

**注册前移（蓝图纪律落地）**：`models.profiles.add`（`models.rs:125`）在写入 `models.json` 之前即执行该 gate——只要新增档案指向一个已声明权重的本地/第三方模型（显式 `catalogId`，或 `kind=Local` 且 `model`/`id` 命中聚合目录条目），未过 gate 直接拒绝（码 `gate.blocked`），并把校验结论回传前端。纯云 API 端点（OpenAI 兼容等，无权重声明）**不参与** gate——我们无法诚实评估其「权重是否落地」，故不臆测、不阻断。

前端 `LocalModelsView.vue` 提供「校验 shipped gate」按钮（调用 `models.gate.check`，按全局开关 `权重已下载 / 运行时支持` 实时展示六要素结论），「校验全部 gate」可批量校验整目录。`models.local.deploy`（`local_models.rs:842`）保留 `requireGate` 入参，作为部署路径的等价校验。

此外，WebUI「引擎健康 · 模型」页（`EngineHealthView.vue`）的**手写「新增模型档案」表单**也已接入：可选 `kind=local`，勾选 `权重已实际下载 / 本机运行时支持`、`强制 gate 校验 / 强制跳过 gate`，并填 `catalogId`（精确命中聚合目录）。选 `kind=local` 即复用本次前移的 gate——`models.profiles.add` 据 `model`/`id` 命中目录项后执行放行校验，`gate.blocked` 时前端透明回显错误与错误码。base_url 同时兼容前端 camelCase `baseUrl`、旧调用方 `baseUri`、持久化 `base_url` 三种写法（修复 provider=custom 时恒报「缺少 base_url」的潜伏 bug）。

---

## 5. 智能推荐

`recommend_models(intent, ram_gb, has_gpu, os)`（`local_models.rs:373-451`）：

1. **档位分桶** `tier_for`（`local_models.rs:349-359`）：
   - `edge`：RAM < 8G
   - `laptop`：8–15G
   - `workstation`：16–31G
   - `server`：≥ 32G **或** 检测到 GPU
2. **预算上限** `max_billions_for`（`local_models.rs:362-370`，Q4_K_M 经验约 0.6 GB/10 亿参数）：edge→4B、laptop→14B、workstation→32B、server→106B。
3. **参数规模解析**：从 `id`/`label` 启发式抽取最大整数段（含 `T` 按 ×1000 折算），例如 `qwen3.8-27b` 须取 max=27 而非首段 3（`local_models.rs:380-405`）。
4. **过滤 + 打分**（`local_models.rs:410-425`）：
   - 规模超出档位预算 → **硬过滤**（`billions(e) <= max_b`）；
   - 用途命中 +20，通用兜底 `general` +6，规模适配 +10；
   - 内存惩罚用 `min_ram_gb.saturating_sub(ram_gb)`（u32 截断，避免负数加成导致小模型反超推理模型，`local_models.rs:422`）。
5. 取 Top 4，附 `match_score` 与 `reason`（按档位给出中文说明）。

`local_recommend` 在未显式传 `ramGB/gpu/os` 时回退 `host_system_spec()`（`local_models.rs:750-774`）——这正是「取代浏览器猜值」的落点。WebUI 在 `runRecommend`（`LocalModelsView.vue:256-266`）把系统卡探测到的值带上，切换用途下拉即重新推荐。

---

## 6. 一键下载 / 部署 / 启停

### 6.1 生命周期状态机（台账 `ManagedLocalModel.status`）

```
                 download                  deploy                start        stop
  ┌──────────┐ ──────────► ┌────────────┐ ───────► ┌──────────┐ ────► ┌──────────┐ ──► ┌──────────┐
  │not_installed│          │ downloading │         │  ready    │      │ running  │     │ stopped  │
  └──────────┘ ◄────────── └────────────┘ ◄─────── └──────────┘ ◄──── └──────────┘     └──────────┘
   (Ollama 不可用时           (Ollama 可用            (deploy 后置                (started 标记切换)
    直接落此态)              后台 pull)              deployed=true)
```

- `not_installed`：Ollama 不可用 / 尚未下载（`local_models.rs:806`）。
- `downloading`：Ollama 可用且 `ollama_tag` 非空时，后台 fire-and-forget `ollama pull`（**测试态 `#[cfg(not(test))]` 不真实拉取**，`local_models.rs:794-802`）。
- `ready`：经 `deploy` 后（`local_models.rs:877`）。
- `running` / `stopped`：由 `local_set_started` 切换 `started` 布尔（`local_models.rs:916-937`）。

### 6.2 启停与移除

- **start/stop**：`local_set_started` 要求先 `deploy`，否则返回 `not_deployed`（或 `not_managed`），绝不静默启动（`local_models.rs:919-924`，测试 `start_requires_deploy_first` 锁定）。注意：start/stop **只切换台账标记**，本地推理进程由用户/Ollama 服务实际管理（`local_models.rs:931-935`）。
- **remove**：`local_remove`（`local_models.rs:940-974`）从台账删条目，并（默认 `removeProfile=true`）尝试从 `models.json` 移除路由档案。守卫逻辑：仅当「注册表仅剩 ≤1 个且全部非 `Local`」时才拒绝删唯一档案，本地档案允许删空（`local_models.rs:958-962`）。

---

## 7. 训练与微调（诚实编排）

设计边界（`local_models.rs:561-571`）：任务台账持久化 `local_finetunes.json`；**仅当检测到可用训练后端才真正派发子进程，否则如实标记 `needs_backend` 并给出配置指引，绝不假装已训练**。

### 7.1 任务台账

- `FinetuneJob`（`local_models.rs:574-596`）：`base_id`、`base_label`、`method`(lora/qlora/full)、`dataset_path`、`output_dir`、`epochs`、`learning_rate`、`status`(queued/running/done/failed/cancelled)、`backend`、`pid`、`needs_backend`、`error`、时间戳。
- `LocalFinetuneStore`（`local_models.rs:599-624`）持久化到 `local_finetunes.json`。

### 7.2 后端探测

`detect_finetune_backend()`（`local_models.rs:627-643`）：

1. **用户模板优先**：读取环境变量 `MGT_FINETUNE_COMMAND`，非空即用。模板含占位符 `{base}` `{dataset}` `{out}` `{epochs}` `{lr}` `{method}`（`local_models.rs:656-662` 替换）。
2. 否则 PATH 探测已知工具：`llama-finetune` / `axolotl` / `unsloth`（`local_models.rs:634-641`）。

### 7.3 派发（诚实）

`local_finetune`（`local_models.rs:1023-1136`）逻辑：

- 缺 `baseId`/`dataset` → 直接返回错误（`finetune.no_base` / `finetune.no_dataset`）。
- 检测到**自定义模板**（非 `auto:`）→ `spawn_finetune` 派发后台子进程，记录 `pid`、`status="running"`、`needs_backend=false`（派发失败则 `status="failed"`）。
- 否则（`None` 或仅 `auto:` 工具名）→ `status="queued"`、`needs_backend=true`、给出配置指引（`local_models.rs:1082-1095`）。**绝不假装已训练**。
- `spawn_finetune` / `kill_pid` 均 `#[cfg(not(test))]`——**测试态绝不真实派发/杀进程**（`local_models.rs:646-696`、`671-685`），避免下载/训练副作用。

### 7.4 取消

`local_finetune_cancel`（`local_models.rs:1160-1182`）：若 `pid != 0` 则 `#[cfg(not(test))]` 调 `kill_pid`（Windows `taskkill /PID /F`，其余 `kill -TERM`），并置 `status="cancelled"`。

WebUI 对应面板 `LocalModelsView.vue:119-167`（基底模型下拉=目录+已管理、数据集路径、方法、轮数、学习率、发起/取消；任务列表显示状态与「待配置后端」提示）。

---

## 8. 成本与定价观测

工作台系统卡显示「成本与时段」（WebUI `LocalModelsView.vue:28-40`），数据来自 `usage.cost` RPC（注册见 `methods.rs:916-917`、`protocol.rs:242`）。

`usage/cost`（`usage/mod.rs:363` 起）聚合 token 台账估算美元成本，并给出**分时段定价**：

- `estimated_cost_usd`：估算总成本（`usage/mod.rs:383`）。
- `price_input_per_mtok` / `price_output_per_mtok`：输入/输出单价（`usage/mod.rs:386-389`）。
- `period`：`peak` / `off-peak`。`current_period()`（`usage/mod.rs:350-360`）以本地时间 **23:00–07:59 为 off-peak**，其余为 peak（DeepSeek 式分时段定价启发）。
- `advice` / `schedule`：峰值时段建议把批量/长任务排程到 off-peak 执行以降本。

WebUI 据此高亮当前时段（off-peak 绿色、peak 橙色）并显示建议文案。

---

## 9. 使用方式

### 9.1 WebUI：`LocalModelsView`

`webui/src/views/LocalModelsView.vue`（`onMounted` → `refreshAll`，`LocalModelsView.vue:321`、`314-319`）：
- 顶部三张系统卡：系统配置（真实探测）、成本与时段、已部署本地模型（路由可见）。
- 智能推荐区：用途下拉（general/coding/reasoning/chat/multimodal）+ 候选卡片「下载 / 部署到路由」。
- 聚合下载目录：shipped gate 六要素徽章 + 每项的「下载 / 部署 / 启动|停止 / 移除」。
- 训练与微调区：表单 + 任务列表（含「待配置后端」提示）。

### 9.2 CLI：`marguerite model ...`

模型管理子命令（`cli/program.rs:1159-1246`）作用于 `models.json` 注册表，工作台部署出的 `local-<id>` 档案同样可在此操作：

- `marguerite model list` — 列出档案（`*` 主模型、`◇` Bloom 副模型），含 `kind=Local` 的本地档案。
- `marguerite model current` — 显示主模型 / Bloom 副模型。
- `marguerite model use local-<id>` — 把本地档案切为主模型（`ModelSelector::use_active`）。
- `marguerite model copilot local-<id>` — 把本地档案标记为 Bloom 副模型（`use_copilot`）；`off` 关闭。
- `marguerite model remove local-<id>` — 删除路由档案（等价于 RPC `models.local.remove` 的 `removeProfile` 路径）。
- `marguerite model path` — 打印 `models.json` 路径（即本地台账同目录）。

> 注：本地工作台的「下载/部署/启停/微调」主入口是 **RPC + WebUI**，CLI `model` 子命令负责注册表层面的查看与切换（含已部署的本地档案）。

### 9.3 配置旋钮

- **`MGT_FINETUNE_COMMAND`**（环境变量）：训练后端命令模板，占位符 `{base}` `{dataset}` `{out}` `{epochs}` `{lr}` `{method}`（`local_models.rs:627-633`、`656-662`）。设置后 `models.local.finetune` 才会真正派发训练。
- **本地台账路径**：`local_models.json` / `local_finetunes.json` 均位于 `Config::config_dir()`（与 `models.json` 同目录，`local_models.rs:296-299`、`605-607`）。测试通过 `Config::set_config_dir_override` 重定向到临时目录（`local_models.rs:1192-1205`）。

---

## 10. 测试与验证

### 10.1 Rust 测试模块（`local_models.rs` 内 `mod tests`，`local_models.rs:1184-1374`）

关键测试：

- `catalog_has_shipped_gate_six_elements` — 每个目录项六要素齐备。
- `recommendation_respects_tier_budget` — 边缘档位不推 >4B、服务器/GPU 推 30B+ MoE（含 `qwen3-30b-a3b`）。
- `intent_steers_recommendation` — 编码用途应含 `qwen2.5-coder-14b`。
- `deploy_registers_local_profile_and_marks_source` — deploy 注册 `kind=Local` 档案、remove 清理台账+档案。
- `download_records_intent_without_side_effect` — 下载意图写入台账（测试态不真实拉取）。
- `start_requires_deploy_first` — 未部署不得启动。
- `host_system_spec_returns_real_values` / `local_system_rpc_returns_host_spec` — 真实探测与 RPC 一致，`source=="host"`。
- `recommend_without_params_uses_host_spec` — 不传参回退宿主探测不 panic。
- `finetune_without_backend_is_queued_and_flagged` — 无后端时诚实 `queued` + `needs_backend=true`，cancel 置 `cancelled`。
- `finetune_rejects_missing_inputs` — 缺 `baseId`/`dataset` 返回对应错误码。

测试采用 `OnceLock<Mutex<PathBuf>>` + `lock_home()`（`local_models.rs:1189-1205`）串行化对 `models.json`/`local_models.json` 的写，避免并发覆写用户目录。

成本相关：`usage/mod.rs` 测试 `cost_estimates_positive_and_period_valid`（`usage/mod.rs:412`）。

### 10.2 WebUI 覆盖

`LocalModelsView.vue` 通过 `vitest` 随 webui 套件运行；2026-08-27 实现日志记录：`vue-tsc --noEmit` 0 错误，`vitest run --no-cache` = 300 文件 / 3125 passed（其中含 PSI 解包等回归，工作台面板本身随整体套件验证）。

### 10.3 干净 CI 纪律（重要）

本特性曾因**增量编译缓存掩盖 lib 级编译错误**而出现「全绿」假象（`marguerite-core` 干净 `cargo check` 一度无法编译，详见 2026-08-27 日志）。验证协议：

1. 先 `cargo clean -p marguerite-core`（怀疑掩盖时），再 `cargo check --workspace`；
2. 跑 `cargo test --workspace`（而非单模块），避免复用旧 `.rlib`；
3. `cargo check -p marguerite-core --tests` 清测试目标警告（`#[must_use]` 等只在 `--tests` 下暴露）。

落地验证结果（2026-08-27，B 特性闭环）：`cargo clean -p marguerite-core` → `cargo check` 抓到 3 个真编译错误并逐个修 → `cargo test -p marguerite-core` = **2209 passed / 0 failed / 17 ignored**（+5 新测试）；`cargo check -p marguerite-core` 0 warning；webui 3125 passed。

---

## 11. 已知限制与后续

以下仅列代码中可见的限制，未见的均标 TBD，不臆造：

- **GPU 探测仅做存在性 + 名称**：`detect_gpu` 只返回是否检测到与显卡名，未探测显存容量/算力，故推荐分桶只用 `has_gpu` 布尔（`local_models.rs:349`）。显存感知推荐为后续 TBD。
- **下载为「意图 + 后台 pull」**：`local_download` 在 Ollama 可用时 fire-and-forget `ollama pull`，台账 `status` 置 `downloading` 但**不轮询拉取进度/完成态**，成功后不会自动回写为 `ready`（就绪态主要由 `deploy` 驱动）。进度回写为 TBD。
- **start/stop 仅切台账标记**：本地推理进程由用户/Ollama 服务实际管理，工作台不负责拉起/杀掉推理服务（`local_models.rs:931-935`）。
- **shipped gate 校验器已落地（蓝图 C）**：目录项六要素除展示/自测外，现已由独立校验器强制校验——见 `gateway/shipped_gate.rs`：纯函数 `gate_local_model(entry, weights_present, runtime_supports)` 逐要素给出 `Pass/Warn/Fail`，出现 `Fail` 即 `passed=false`（Warn 仅提示不阻断）；RPC `models.gate.check` 按目录 id 校验；`local_deploy` 默认仅作提示（响应带 `gate` 字段），传 `requireGate=true` 时未过 gate 返回 `local_model.gate_blocked` 拒绝接入路由。注意本工作台目录的六要素徽章字段是 `weights/license/eval/pricing/quantized/reproducible`（描述模型放行属性），而校验器按蓝图六要素 **权重/许可证/评测/定价/可用性/兼容性** 核对（可用性←是否有下载源，兼容性←`runtime_supports` 传入），两者语义一致、口径互补。
- **路由规则已数据驱动（A 延伸）**：`router.rs` 的 `default_routing_rules` 与故障转移拓扑 `provider_sibling_model`/`provider_tiers` 已外置为 `core/src/model/routing_rules.json`（含 21 条路由规则 + failover 拓扑），编译期内嵌、零行为变化，新增/调整档位只改数据文件。旧「硬编码旧版模型名」已消除。
- **训练后端未内置**：`spawn_finetune` 仅按模板/探测到的可执行文件派发，不内置任何训练实现；无后端时任务停留在 `queued` + `needs_backend`，需用户配置 `MGT_FINETUNE_COMMAND` 或安装工具。
- **`pricing` 硬编码为「本地免费（自付电费）」**（`local_models.rs:87`），未接入按量/第三方定价的动态来源。
