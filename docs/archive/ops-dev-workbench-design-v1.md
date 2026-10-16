# Marguerite「服务器运维 + 智能编程」一体化工作台设计方案 v1

> 基于现有 Marguerite 架构（Rust Core Gateway v3 + Vue.js WebUI + btpanel-mcp）扩展设计。

---

## 一、整体架构与模块划分

### 1.1 现有基础设施盘点

| 层次 | 现有模块 | 复用程度 |
|------|----------|----------|
| RPC 协议 | `gateway/protocol.rs` (PROTOCOL_VERSION=3, WS/JSON-RPC) | ✅ 直接复用 |
| 方法路由 | `gateway/methods.rs` (dispatchSync) | ✅ 扩展新方法族 |
| 认证 | `gateway/auth.rs` + `collab/auth.rs` (本地 token) | ⚠️ 需升级为多角色 |
| 审计 | `automation/audit.rs` | ✅ 直接复用 |
| 远程运维 | `collab/collab_ssh_probe/exec` | ✅ 扩展批量/文件管理 |
| 宝塔 | `gateway/baota.rs` + `btpanel-mcp/` (Python MCP) | ✅ 直接复用 |
| 代码索引 | `codeintel/index.rs` + `codeintel/search.rs` | ✅ 扩展语义检索 |
| 缓存 | `cache/mod.rs` + `cache/semantic.rs` | ✅ 扩展分层 |
| Agent 运行时 | `agent/runtime.rs` + `agent/subagent.rs` | ✅ 直接复用 |
| 任务拆解 | `plan/runner.rs` (run_autonomous) | ⚠️ 需加步数上限 |
| 模板 | `chain/templates.rs` | ✅ 扩展运维/编程模板 |
| 前端工作台 | `webui/src/workbench/` (registry/runtime/store) | ✅ 扩展新面板 |

### 1.2 新增/扩展模块架构图

```
┌─────────────────────────────────────────────────────────────────┐
│                     WebUI (Vue 3 + Vite)                        │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐    │
│  │ OpsPanel │ │ DevPanel │ │ TeamPanel│ │ ContextPiplineView│    │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────────┬─────────┘    │
│       │            │            │                 │              │
│  ─────┴────────────┴────────────┴─────────────────┴─────        │
│                  Workbench Layout + Store                       │
│                        │ rpc.ts (WS)                            │
└────────────────────────┼────────────────────────────────────────┘
                         │ PROTOCOL v3
┌────────────────────────┼────────────────────────────────────────┐
│              Rust Core Gateway (127.0.0.1:18790)                │
│                        │                                        │
│  ┌─────────────────────┼──────────────────────────────────┐    │
│  │  methods.rs dispatch                                     │    │
│  │  ├── ops.*          ← 新增：运维方法族                    │    │
│  │  ├── dev.*          ← 新增：智能编程方法族                │    │
│  │  ├── team.*         ← 扩展：RBAC                         │    │
│  │  ├── baota.*        ← 已有：宝塔凭据                     │    │
│  │  ├── collab.*       ← 已有：协作会话                     │    │
│  │  └── cache.*        ← 扩展：分层缓存                     │    │
│  └──────────────────────────────────────────────────────────┘    │
│                                                                  │
│  ┌──── core/src/ops/ ─────────────────────────────────────────┐  │
│  │  host.rs        → 主机池管理 (CRUD, 批量选择, 健康检查)     │  │
│  │  session.rs     → SSH 会话池 (多路复用, 连接池, 超时)       │  │
│  │  file.rs        → 远程文件管理 (浏览/上传/下载/编辑)        │  │
│  │  process.rs     → 进程管理 (ps/kill/top)                   │  │
│  │  log.rs         → 日志流 (tail -f, 关键词过滤, 告警)       │  │
│  │  firewall.rs    → 防火墙规则 (iptables/ufw/firewalld)     │  │
│  │  ssl.rs         → SSL 证书管理 (申请/续期/部署)            │  │
│  │  cron.rs        → 计划任务 CRUD (经宝塔 API)               │  │
│  │  site.rs        → 站点管理 (增删/PHP版本/反向代理)          │  │
│  │  db.rs          → 数据库管理 (MySQL/PostgreSQL CRUD)       │  │
│  │  batch.rs       → 批量操作编排 (并行, 限速, 进度)           │  │
│  │  rollback.rs    → 操作回滚 (快照 + 撤销脚本)               │  │
│  │  bta.rs         → 宝塔 API 封装层 (经 btpanel-mcp)         │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──── core/src/dev/ ─────────────────────────────────────────┐  │
│  │  pipeline.rs    → 上下文压缩管线 (分层/模板/检索)           │  │
│  │  intent.rs      → 意图路由 (classify → route → decompose)  │  │
│  │  decompose.rs   → 多步任务拆解 (plan → execute → verify)   │  │
│  │  template.rs    → 输出模板 (结构化 JSON/Markdown 契约)     │  │
│  │  hitcache.rs    → 命中缓存 (语义匹配 + 精确去重)            │  │
│  │  selfcheck.rs   → 失败自检与重试 (compile/test → repair)   │  │
│  │  index.rs       → 代码库索引 (AST → 向量 → BM25 混合)      │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──── core/src/rbac/ ────────────────────────────────────────┐  │
│  │  roles.rs       → 角色/权限模型 (admin/dev/guest)          │  │
│  │  policy.rs      → 方法级 ACL (method → min_role 映射)     │  │
│  │  audit.rs       → 操作审计 (who/what/when/result)          │  │
│  │  versioning.rs  → 脚本/配置版本管理 (快照 + diff + 回滚)    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──── core/src/security/ ────────────────────────────────────┐  │
│  │  vault.rs       → 凭据加密存储 (AES-256-GCM)              │  │
│  │  confirm.rs     → 危险操作二次确认 (预览 diff + 人工确认)   │  │
│  │  rate_limit.rs  → 并发/速率限制 (令牌桶, 每主机/每方法)    │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──── 已有复用层 ────────────────────────────────────────────┐  │
│  │  agent/          → AI Agent 运行时 (工具调用循环)            │  │
│  │  plan/           → 多步计划执行 (run_autonomous)             │  │
│  │  btpanel-mcp/    → 宝塔 MCP (Python, stdio)                 │  │
│  │  collab/         → 协作会话 (SSH probe/exec, CRDT)           │  │
│  │  codeintel/      → 代码索引 (已有, 扩展向量)                  │  │
│  └───────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### 1.3 模块职责与依赖关系

| 模块 | 职责 | 依赖 |
|------|------|------|
| `ops/` | 远程主机全生命周期运维 | `collab/`(SSH), `security/`(加密/限流), `rbac/`(权限) |
| `dev/` | 智能编程上下文管理 | `agent/`(LLM), `codeintel/`(检索), `cache/`(缓存), `plan/`(拆解) |
| `rbac/` | 认证+授权+审计 | `persistence/`(存储), `security/`(加密) |
| `security/` | 凭据保险库+确认+限流 | 无(底层设施) |

---

## 二、关键数据流与接口约定

### 2.1 运维操作数据流（以"批量重启站点"为例）

```
用户 → OpsPanel.vue
  → rpc req: ops.site.batch_restart {hosts: [...], site_ids: [...]}
    → methods.rs dispatch → ops::batch::execute()
      → rbac::policy.check("ops.site.batch_restart", user_role)  // 权限检查
        → admin 才允许 batch_restart
      → security::confirm::require_approval()                    // 危险操作确认
        → 生成预览 diff + 影响范围 → 发送 event "ops.confirm.required"
        → 等待用户二次确认 (WS event "ops.confirm.approved")
      → security::rate_limit::acquire()                          // 并发控制
        → 令牌桶: max_concurrent_per_host = 3, global = 10
      → ops::session::get_or_create_pool(host_ids)               // SSH 连接池
      → ops::rollback::snapshot(hosts, targets)                  // 回滚快照
      → ops::bta::batch_call("site.restart", targets)            // 经宝塔 API
        → btpanel-mcp (Python subprocess) → BT Panel API
      → ops::rollback::register_undo(undo_script)                // 注册回滚脚本
      → event "ops.progress" (逐主机推送进度)
      → event "ops.result" (汇总结果)
    → ops::audit::log(actor, action, targets, result)             // 审计
  → res: {ok, results: [{host, site, success, msg, rollback_id}]}
```

### 2.2 智能编程数据流（以"修复 Bug"为例）

```
用户 → ChatView / DevPanel
  → rpc req: chat.send {session_id, message: "修复用户登录 NPE"}
    → methods.rs → chat::handle()
      → dev::intent::classify(message)                           // 意图路由
        → intent = "BugFix", complexity = "Medium"
      → dev::hitcache::lookup(intent, message_hash)              // 缓存复用
        → cache hit? → 直接返回 {source: "cache", answer, tokens_saved: N}
        → cache miss → 继续
      → dev::pipeline::build_context(intent, session_id)         // 分层上下文
        → L0: 系统提示 + 角色指令 (固定, ~500 tok)
        → L1: 项目摘要 + 目录树 (会话级, 按需刷新, ~800 tok)
        → L2: 意图相关文件片段 (codeintel::search 检索, ~2000 tok)
        → L3: 对话历史 (压缩摘要 + 最近 K=3 轮原文, ~1000 tok)
        → 总上下文 ≤ max_context_tokens (LLM 配置)
      → dev::decompose::plan(intent, context)                    // 多步拆解
        → 步骤: [1.定位文件, 2.分析根因, 3.生成补丁, 4.验证]
      → 逐步执行 (agent::tool_call 循环)
        → 每步后 dev::selfcheck::verify(step_result)
          → compile_check? test_pass? → 通过 → 下一步
          → 失败 → 生成修复提示 → 重试 (max_retry = 2)
          → 重试耗尽 → 标记步骤失败 → 继续或终止
      → dev::template::format_output(result)                     // 模板化输出
        → 结构化: {summary, diff, explanation, test_evidence}
      → dev::hitcache::store(intent, message_hash, output)       // 写入缓存
      → cache::token_usage::track()                               // Token 统计
  → res: {ok, answer, token_usage, plan_steps, cache_status}
```

### 2.3 协作数据流（以"共享 SSH 会话"为例）

```
管理员 → TeamPanel
  → rpc req: team.share_session {session_id, target_role: "developer"}
    → rbac::policy.check("team.share_session", "admin")
    → collab::session::create_share(session_id, role)
      → 生成 share_token (短期有效)
    → collab::audit::log(actor, "session.share", ...)
  → 开发者 → TerminalView
    → rpc req: collab.join {share_token}
      → rbac::policy.check("collab.join", "developer")
      → collab::presence::join(session_id, user_id)
      → event "collab.presence.update" → 广播在线列表
    → rpc req: collab.exec {session_id, command}
      → rbac::policy.check("collab.exec", "developer")
      → 危险命令检测 (rm -rf /, shutdown) → 拦截 + 审计
      → SSH 执行 → 结果返回 → 审计
```

### 2.4 核心 API 方法族

#### 运维方法族 (`ops.*`)

| 方法 | 角色 | 描述 |
|------|------|------|
| `ops.host.list` | dev+ | 列出主机池（含状态/标签/凭据引用） |
| `ops.host.add` | admin | 添加主机（host/port/user/auth_ref） |
| `ops.host.remove` | admin | 移除主机 |
| `ops.host.health` | dev+ | 批量健康检查（SSH ping / CPU / Mem / Disk） |
| `ops.session.open` | dev+ | 打开远程 SSH 会话（复用连接池） |
| `ops.session.exec` | dev+ | 在远程会话执行命令（限流+审计） |
| `ops.file.list` | dev+ | 远程目录列表 |
| `ops.file.read` | dev+ | 远程文件读取（支持范围） |
| `ops.file.write` | admin | 远程文件写入（需确认+快照） |
| `ops.file.upload` | dev+ | 上传文件（SFTP） |
| `ops.file.download` | dev+ | 下载文件（SFTP） |
| `ops.process.list` | dev+ | 远程进程列表（ps/top） |
| `ops.process.kill` | admin | 终止进程（需确认） |
| `ops.log.tail` | dev+ | 日志流（tail -f，WebSocket 推送） |
| `ops.log.search` | dev+ | 日志关键词搜索 |
| `ops.site.list` | dev+ | 宝塔站点列表 |
| `ops.site.create` | admin | 创建站点（域名/PHP/路径） |
| `ops.site.delete` | admin | 删除站点（需确认+回滚） |
| `ops.site.restart` | admin | 重启站点 |
| `ops.db.list` | dev+ | 数据库列表 |
| `ops.db.backup` | admin | 数据库备份（需确认） |
| `ops.ssl.list` | dev+ | SSL 证书列表（含到期日） |
| `ops.ssl.deploy` | admin | 部署 SSL 证书 |
| `ops.cron.list` | dev+ | 计划任务列表 |
| `ops.cron.create` | admin | 创建计划任务 |
| `ops.firewall.rules` | dev+ | 防火墙规则列表 |
| `ops.firewall.rule.add` | admin | 添加防火墙规则（需确认） |
| `ops.batch.exec` | admin | 批量操作编排（并行+限速+进度） |
| `ops.rollback.exec` | admin | 执行回滚（by rollback_id） |
| `ops.confirm.approve` | admin | 批准待确认操作 |

#### 智能编程方法族 (`dev.*`)

| 方法 | 角色 | 描述 |
|------|------|------|
| `dev.intent.classify` | dev+ | 意图分类（离线规则，无 LLM 消耗） |
| `dev.context.build` | dev+ | 构建分层上下文（返回 token 分配方案） |
| `dev.decompose.plan` | dev+ | 多步任务拆解（LLM 一次调用） |
| `dev.execute.step` | dev+ | 执行单步（工具调用 → 自检 → 缓存） |
| `dev.hitcache.query` | dev+ | 查询命中缓存 |
| `dev.hitcache.stats` | dev+ | 缓存统计（命中率/节省 Token 数） |
| `dev.template.render` | dev+ | 按模板格式化输出 |
| `dev.selfcheck.run` | dev+ | 对生成代码运行自检（编译/测试） |
| `dev.index.build` | dev+ | 构建代码库索引 |
| `dev.index.search` | dev+ | 语义+BM25 混合搜索 |

#### 协作方法族 (`team.*` 扩展)

| 方法 | 角色 | 描述 |
|------|------|------|
| `team.users.list` | admin | 用户列表（角色/权限/最近活跃） |
| `team.users.create` | admin | 创建用户（role + 凭据） |
| `team.users.update_role` | admin | 修改用户角色 |
| `team.share.create` | admin | 创建共享链接（会话/脚本/配置） |
| `team.version.list` | dev+ | 版本历史 |
| `team.version.diff` | dev+ | 版本间 diff |
| `team.version.rollback` | admin | 回滚到指定版本 |
| `team.audit.log` | admin | 操作审计日志查询 |

### 2.5 凭据加密存储设计

```
$1marguerite/
  vault.enc                    ← AES-256-GCM 加密的凭据保险库
    master_key_hash            ← PBKDF2(用户口令, salt, 600_000 iters)
    encrypted_data             ← AES-256-GCM(plaintext, nonce, aad)
    plaintext 内含:
      hosts:
        host_id:
          host: "192.168.1.100"
          port: 22
          user: "root"
          auth_type: "password" | "key"
          password_enc: "..."
          private_key_enc: "..."
      baota:
        base_url: "..."
        username: "..."
        password_enc: "..."
      llm_keys:
        provider_name: "api_key_enc"
```

### 2.6 Token 消耗降低策略

| 策略 | 实现位置 | 预期节省 |
|------|----------|----------|
| **分层上下文** | `dev/pipeline.rs` | L0 固定(~500 tok)，L1 会话级缓存(~800 tok)，L2 按需检索(~2000 tok)，L3 滑动窗口(~1000 tok)。总上下文 4300 tok vs 全量 15K+ tok = **70%+ 减少** |
| **意图路由** | `dev/intent.rs` | 纯离线规则分类，不消耗 LLM Token。路由正确率 >90%（简单类直接走模板） |
| **模板化输出** | `dev/template.rs` | 每次输出按预定义 JSON Schema 生成，避免 LLM 自由发挥产生冗余文本。减少 30-50% 输出 Token |
| **缓存复用** | `dev/hitcache.rs` | 语义相似度 > 0.95 的相同意图 → 直接返回缓存结果。首次命中后重复问题 **0 Token** |
| **代码索引** | `dev/index.rs` + `codeintel/` | 只检索相关文件片段（top-K=5），而非全量代码。每次查询节省 3-10K Token |
| **失败自检** | `dev/selfcheck.rs` | 编译/测试失败时生成精确修复提示，避免无效重试。减少 50%+ 无效 LLM 调用 |
| **步数上限** | `plan/runner.rs` | 每个子步骤 max_llm_rounds = 5，防止无限循环。硬性保证 |

---

## 三、分阶段实施路径

### Phase 1：基础设施（RBAC + 加密 + 确认）

**工期估算**：3-5 天

| 任务 | 文件 | 验证方式 |
|------|------|----------|
| 实现角色/权限模型 | `core/src/rbac/roles.rs` | 单元测试：各角色能否访问各方法族 |
 | 实现 AES-256-GCM 保险库 | `core/src/security/vault.rs` | 单元测试：加密→解密 round-trip + 篡改检测 |
 | 实现危险操作确认 | `core/src/security/confirm.rs` | 集成测试：未确认→拒绝；确认→执行 |
 | 实现速率限制 | `core/src/security/rate_limit.rs` | 单元测试：超限→429 |
 | 扩展 `methods.rs` dispatch | 在现有 dispatch 中插入 RBAC 检查 | 集成测试：guest 访问 admin 方法 → 403 |
 | WebUI：TeamPanel | `webui/src/views/TeamPanel.vue` | 手动：创建用户/切换角色/查看审计 |

**风险点**：
- 密钥管理复杂度 → 简化为本地单用户口令解锁 vault
- 向后兼容 → 新增方法默认 `min_role = developer`，已有方法不加权限（渐进收紧）

---

### Phase 2：远程运维核心

**工期估算**：5-8 天

| 任务 | 文件 | 验证方式 |
|------|------|----------|
 | 主机池管理 | `core/src/ops/host.rs` | 单元 + 集成（SSH ping 多主机） |
 | SSH 连接池 | `core/src/ops/session.rs` | 并发测试：10 并发 exec 不泄漏连接 |
 | 远程文件管理 | `core/src/ops/file.rs` | 集成：read/write/upload/download round-trip |
 | 进程管理 | `core/src/ops/process.rs` | 集成：ps → 找到目标 pid → kill → 确认消失 |
 | 日志流 | `core/src/ops/log.rs` | 集成：tail -f /var/log/syslog，WS 收到新行 |
 | 批量操作 | `core/src/ops/batch.rs` | 集成：3 主机并行 exec，限速 2/s |
 | 回滚 | `core/src/ops/rollback.rs` | 集成：写入文件 → 回滚 → 内容恢复 |
 | WebUI：OpsPanel | `webui/src/views/OpsPanel.vue` | 手动：主机管理/文件浏览/日志查看 |

**风险点**：
- SSH 连接稳定性 → 加心跳 + 自动重连（已有 collab/collab_ssh_probe 基础）
- 跨平台命令差异 → 统一用 POSIX 命令，非 POSIX 主机给出警告
- 大文件传输 → 限制单文件上限（默认 50MB），分块传输

---

### Phase 3：宝塔集成

**工期估算**：3-5 天

| 任务 | 文件 | 验证方式 |
|------|------|----------|
 | 宝塔 API 封装 | `core/src/ops/bta.rs` | 集成：连接宝塔 → 获取站点列表 |
 | 站点 CRUD | 同上 | 集成：创建/修改/删除站点 + 回滚 |
 | 数据库管理 | 同上 | 集成：列表/备份/还原 |
 | SSL 管理 | 同上 | 集成：列表/部署/续期 |
 | 计划任务 | 同上 | 集成：CRUD 计划任务 |
 | 防火墙 | 同上 | 集成：规则列表/添加/删除 |
 | WebUI 集成 | `webui/src/views/OpsPanel.vue` 扩展 | 手动：宝塔面板操作全流程 |

**风险点**：
- 宝塔 API 版本差异 → 定义适配层，按宝塔版本选择 API 路径
- SSL 证书自动续期 → Let's Encrypt DNS 验证需要 API 密钥，非默认实现

---

### Phase 4：智能编程工作台

**工期估算**：5-8 天

| 任务 | 文件 | 验证方式 |
|------|------|----------|
 | 意图路由 | `core/src/dev/intent.rs` | 单元：50 条测试消息分类准确率 > 90% |
 | 分层上下文 | `core/src/dev/pipeline.rs` | 单元：各层 token 计数正确；集成：总 context ≤ max |
 | 代码索引（向量） | `core/src/dev/index.rs` | 集成：索引项目 → 语义搜索返回相关文件 |
 | 缓存复用 | `core/src/dev/hitcache.rs` | 单元：相同意图 → cache hit；集成：节省 Token 统计 |
 | 多步拆解 | `core/src/dev/decompose.rs` | 单元：复杂任务正确拆解为 3-7 步 |
 | 失败自检 | `core/src/dev/selfcheck.rs` | 集成：生成代码 → compile check → fail → repair |
 | 模板化输出 | `core/src/dev/template.rs` | 单元：输出符合 JSON Schema |
 | 步数上限 | 修改 `plan/runner.rs` | 集成：复杂任务不超过 max_rounds |
 | WebUI：DevPanel | `webui/src/views/DevPanel.vue` | 手动：意图路由/上下文查看/缓存统计 |

**风险点**：
- 向量索引质量 → 混合 BM25 + 向量，可用 `cache/semantic.rs` 已有实现
- 自检可能误判 → 提供手动覆盖开关
- 缓存失效 → TTL（默认 1h）+ 意图哈希去重

---

### Phase 5：集成测试与交付

**工期估算**：3-4 天

| 任务 | 验证方式 |
|------|----------|
 | 端到端场景：管理员批量部署站点 | 全流程：选主机→确认→执行→回滚→审计 |
 | 端到端场景：开发者智能修复 Bug | 全流程：意图→索引→拆解→执行→自检→输出 |
 | 端到端场景：访客查看日志 | 权限隔离：仅能看到允许的主机+日志 |
 | 安全审计：凭据泄露检测 | 尝试通过各 API 获取明文密码 → 全部拒绝 |
 | 压力测试：10 并发 × 5 主机 × 3 操作 | 无死锁/内存泄漏/超时雪崩 |
 | 性能基线：Token 消耗对比 | 同任务 before/after Token 统计 |

---

### 总工期：约 19-30 天（单人全职）

### 关键假设

1. **服务器系统**：Linux (Ubuntu 20.04+/CentOS 7+)，SSH 可达
2. **宝塔版本**：7.x+，API 可用
3. **团队规模**：≤ 20 人，角色以 admin/dev/guest 三级足够
4. **代码库**：≤ 10 万行，索引时间 < 5 分钟
5. **LLM 提供商**：已有配置（OpenAI/DeepSeek/GLM 等任一可用）
6. **前端框架**：继续使用 Vue 3 + Vite（与现有 webui 一致）

