# Marguerite 系统 L3 防护与沙箱安全审计报告

| 项 | 值 |
|----|----|
| 审计对象 | Marguerite（Rust core + Python 引擎 + 宝塔面板工具集） |
| 审计模式 | Comprehensive（全量静态审计） |
| 审计日期 | 2026-08-11 |
| 审计范围 | `core/src/`：sandbox（mod/whitelist/policy/audit/rpc/penetration_tests）、security（scanner_l2/scanner_l3/command_guard/exec/os_sandbox/ssrf_guard/mod）、tools（baota/builtin/fs_path）、gateway（auth/server）、terminal（mod）、sandbox（rpc） |
| 方法论 | OWASP Top 10（A01–A10）+ STRIDE 威胁建模 |
| 执行阶段 | 全 14 阶段；环境无 Rust 工具链（rustc not found），Phase 13 主动验证以源码审阅 + 既有测试断言为证据，未做编译期验证 |
| 修正记录 | 2026-08-11 补遗（一）：新增 **F-07（高）**——L3 `GuardrailScanner` 未接入生产执行路径（幻象防护）。2026-08-11 补遗（二）：F-07 扩至 **L2+L3 整条静态安全链**——`MalwareScanner`(L2) 同样仅定义+测试、零生产调用（`scanner_l2.rs` 仅 `security/mod.rs:41,67` 测试引用；`builtin.rs` 零匹配）。感谢 gstack-product-reviewer 代码级取证与指正。 |

---

## 一、执行摘要（Executive Summary）

系统采用**分层纵深防御**：L1（命令级语义拦截 `command_guard` / `ssrf_guard` / `exec` 沙箱）、L2（静态代码/内容扫描 `scanner_l2`）、L3（操作护栏 / 权限边界 `scanner_l3`）。核心模块普遍具备**安全默认**与**失败封闭（fail-closed）**设计。**但经复核确认：L3 扫描器逻辑完整且自带测试，却未接入任何生产工具执行路径（见 F-07），ACP 对外宣称 `L3 enforced=true` 不成立——属"幻象防护"。**

经主动验证，识别 **7 项风险**：**高危 1 项（F-07）、中危 4 项（F-01~F-04）、低危 2 项（F-05~F-06）**。最值得关注的 4 个点：
1. **★ L2+L3 整条静态安全链未接线（幻象防护）**——`MalwareScanner`(L2) 与 `GuardrailScanner`(L3) 逻辑就绪、单测齐备、ACP 声明 `enforced:true`，但运行时均不被调用；实际仅 L1（CommandGuard/SsrfGuard/沙箱）真正生效
2. **终端行模式不经沙箱护栏直接 shell 执行**（纵深防御不一致，绕过 L1/L3）
3. **网关 WebSocket 握手对本地无 token 连接放行**（token 未强制）
4. **宝塔面板 HTTP 客户端禁用 TLS 证书校验**（MITM 风险）

**整体安全态势：C 级**——架构与 L1/L2 实现良好，但 L3 声明与实现脱节构成严重信任缺口，须在发布前修复（将 F-07 列为 P0 阻塞项）；建议同步推进 F-03（TLS）、F-01（终端对齐）、F-02（网关强制认证）。

---

## 二、L3 防护核查结论与证据

### 2.1 L3 操作护栏（`core/src/security/scanner_l3.rs`）

| 证据项 | 位置 | 结论 |
|--------|------|------|
| `GuardrailScanner::check(op, target, cwd)` | line 121 | 主入口 |
| `SYSTEM_PREFIXES`（`/etc`、`/usr`、`/System`、`C:\Windows` 等） | line 94 | 系统目录强保护 |
| `CREDENTIAL_DIRS`（`.ssh` / `.aws` / `.gnupg`） | line 100 | 凭证目录 **一律 deny**（line 141-142） |
| `ESCAPE_PATTERNS`（`chroot` / `mount` / `setuid` / `pkexec` / `docker run --privileged` 等） | line 200 | 沙箱逃逸意图 → **deny**（line 125-127） |
| `GuardrailVerdict::{allow / approve / deny}` | line 53-83 | 写 `.git` 需审批、跨工作区/越界需审批、相对路径放行（line 148-183） |

**结论（实现层）**：L3 的 `GuardrailScanner` 逻辑完整、单元/集成测试齐备，关键系统目录与凭证目录的匹配规则正确（见 F-07 修正：运行时不生效）。

**⚠️ 修正（接线层）—— 见 F-07**：经 grep 全量核查，`GuardrailScanner::check` / `check_all` 的**唯一调用点是 `security/mod.rs:50,70` 的集成测试**与 `scanner_l3.rs` 内部的单元测试；在 `tools/builtin.rs`、`tools/baota.rs`、`serve/acp/session.rs` 等**生产执行路径中零调用**。`grep GuardrailScanner|OpKind|guardrail` 在 `builtin.rs` 零匹配（仅有 `ssrf.check` 与 `SyntaxChecker`）。对比 `CommandGuard`/`SsrfGuard` 已真正接入 `shell_exec`/`web_fetch`/`http_request`，L3 扫描器是**"代码完整但未接线"**状态。

**边界说明**：`check()` 仅对 `target` 做匹配；即便接线，若调用方传入相对路径或经 shell 包装的命令，仍需 L1 兜底。当前问题在于 L3 根本未被调用。

### 2.2 分层集成（**L2 + L3 整条静态链均未挂入运行时**）

`security/mod.rs` 的 `security_layered_chain_blocks_malicious_pipeline`（line 38）仅在**测试内手动组合** L2（`MalwareScanner::new().scan_text`，line 41）+ L3（`GuardrailScanner::new().check`，line 50/70）。生产 dispatcher 从未串联该链。经全量 grep 复核，运行时接线现状为：

| 层 | 扫描器 | 生产调用点 | 状态 |
|----|--------|-----------|------|
| L1 | `CommandGuard` / `SsrfGuard` / 沙箱 | `builtin.rs:584`(shell_exec)、`:735/:772`(web_fetch/http_request) | ✅ 已接线 |
| L2 | `MalwareScanner`（`scanner_l2.rs`） | **零**（`builtin.rs` 零匹配；仅 `security/mod.rs:41,67` 测试引用） | ❌ 未接线（幻象） |
| L3 | `GuardrailScanner`（`scanner_l3.rs`） | **零**（`builtin.rs` 零匹配；仅 `security/mod.rs:50,70` 测试引用） | ❌ 未接线（幻象） |

`session/rollback.rs:391` 的 `scan_workspace` 为工作区文件快照枚举（返回 path/size/perm/sha256），**与恶意代码扫描无关**，不计入 L2 接线。

ACP 接口 `m_permissions_list`（`serve/acp/session.rs:333-334`）声明 `{"id":"L2","name":"malware_static_scan","enforced":true}` 与 `{"id":"L3","name":"operation_guardrail","enforced":true}`——**均与实际运行时事实不符**。即对外宣称的"分层护栏（layered_guardrails）"模型中，仅 L1 真实生效，L2/L3 为**虚假保证**（见 F-07，高危）。

---

## 三、沙箱终端 / 文件系统 / 代码执行 安全边界

### 3.1 沙箱终端判定流水线（`sandbox/mod.rs` + `whitelist.rs`）

| 证据项 | 位置 | 结论 |
|--------|------|------|
| `Sandbox::execute()` | line 179 | 先 `check()` → `tokio::time::timeout`（line 193）→ 输出截断 `cap()` |
| 判定层（Deny/ReadOnly/Write） | line 221-238 | `CommandClass::Deny` → deny；`Write` 经 `mode_gate()` |
| 白名单三分法（DENY/READONLY/WRITE） | `whitelist.rs` | `rm/mv/chmod/kill/sudo/bash/sh/curl/wget/ssh` 等在 DENY；**未知命令默认 Deny** |
| `classify_full()` | `whitelist.rs` | git 子命令级 + 重定向提升写 + `sed -i` 就地编辑复核 |
| `has_inline_payload()` | `whitelist.rs` | 检测 `python -c` / `node -e` 等解释器内联载荷 |
| `validate_cwd()` | `sandbox/mod.rs` line 301 | canonicalize 后 `starts_with(workspace)` |
| `mode_gate()` | `sandbox/mod.rs` line 286 | Plan 模式拒写；Agent 非交互默认拒；Bloom 放行 |
| 资源限制 | `sandbox/mod.rs` line 39 | 超时默认 60s、输出上限 1MB、最大子进程 8 |

**结论**：沙箱护栏实现完整，fail-closed 倾向明确。边界缺口：
- `validate_cwd()` 在 canonicalize **失败**时回退原值（line 302-303），为潜在边界（见 F-05）；
- 符号链接 TOCTOU 因 canonicalize 已解析大部分，风险低；
- **终端行模式不经此流水线**（见 3.4）。

### 3.2 OS 级沙箱（`core/src/security/os_sandbox.rs`）

`IsolationLevel::{Full/Partial/Unsupported}`，fail-closed。Linux Landlock、macOS Seatbelt（`sandbox_init` FFI）；**Windows 诚实返回 Unsupported**，不伪造隔离、命令级兜底。**降级策略安全**。

### 3.3 文件系统安全解析（`tools/fs_path.rs` + `tools/builtin.rs`）

`safe_resolve(root, rel)`：组件栈归一化拒绝 `..` 逃逸 + canonicalize 前缀校验。`delete_file` / `make_dir` / `move_file` / `shell_exec` 均经此路径。

`web_fetch` / `http_request` 经 `SsrfGuard`（`ssrf_guard.rs`）：拦截私有/回环/链路本地/CGNAT/云元数据（169.254.169.254）、非 http(s)、DNS 重绑定防护。**网络与文件边界实现到位**。

### 3.4 终端行模式绕过沙箱护栏 ★ 中危（F-01）

| 证据项 | 位置 |
|--------|------|
| `TerminalSession::run_command()` | `terminal/mod.rs` line 152-169 |
| `Command::new(&self.shell).arg("-c").arg(cmd)`（Win `cmd /C`） | line 153-160 |
| 仅 `strip_ansi` 处理输出 | line 169 |
| 仅 `cd` 内建被拦截持久化 cwd | line 116-127 |

**结论**：交互式终端是独立的、用户已认证的执行面，但相对于工具侧强制沙箱，**终端不经 `CommandGuard` / `Sandbox` 白名单 / `scanner_l3` / `safe_resolve`，也不落盘审计（`audit.rs`）**，造成纵深防御不一致。本地已认证用户可经终端执行任意命令（含被沙箱 DENY 的 `rm -rf`、`sudo` 等），绕过 L1/L3。

**风险等级：中（Medium）**。攻击需本地访问 + 已认证会话；但"沙箱"卖点对外宣称时，终端未对齐会误导安全预期。**修复建议**：终端默认以 `WorkspaceWrite` 沙箱策略运行，或在文档显式标注"终端不受沙箱约束"并将所有提交写入审计日志。

---

## 四、宝塔面板 `baota.*` 工具集风险（`core/src/tools/baota.rs`）

14 个工具注册于 `register_baota_tools`（line 214）：`system_info / network_info / exec_shell / list_sites / get_site_info / create_site / set_site_status / list_files / read_file / write_file / list_databases / create_database / list_ftp / create_ftp`。

### 4.1 凭据管理

| 证据项 | 位置 | 结论 |
|--------|------|------|
| 凭据来源 `BAOTA_PANEL_URL / USERNAME / PASSWORD / SECRET_KEY` | line 68-71 | 环境变量，`unwrap_or_default()` 缺省空串 |
| `ensure_login()` | line 134-151 | 纯 Rust 无法稳定复刻面板签名 → **返回 Err**；需 `baota_bridge cli.py login` 生成 state 或配置 `BAOTA_COOKIE_JSON` |
| 网关 token 落盘 | `gateway/auth.rs` | token 存 `~/.marguerite/gateway-token` |

**结论**：凭据基线合理（环境变量 + 不落盘明文文件），但面板登录态强依赖外部 bridge——纯 core 当前无法独立登录（设计取舍，可接受，见 F-06）。

### 4.2 TLS 证书校验禁用 ★ 中-高（F-03）

```rust
// baota.rs line 102-106
http: reqwest::Client::builder()
    .danger_accept_invalid_certs(true) // 面板多为自签证书
    .timeout(std::time::Duration::from_secs(20))
    .build()
```

**位置**：`baota.rs` line 103。**所有**宝塔 HTTP 调用**禁用 TLS 证书校验**。面板虽多为自签证书，但一旦 `BAOTA_PANEL_URL` 经网络可达，攻击者可 MITM：窃取凭据、篡改返回（line 130 `serde_json::from_str` 直接解析面板响应）、注入命令。

**风险等级：中-高（Medium-High）**。若面板走公网/内网 IP，MITM 影响严重。**修复建议**：增加 `BAOTA_TLS_VERIFY` 开关，默认 `true`；自签场景由用户显式 opt-in 并告警。

### 4.3 命令注入与权限校验

`baota.exec_shell`（line 252-274）：`cmd` 取自参数 → `d.insert("shell", cmd)` → `g.post("/ajax?action=GetTaskSpeed", d)`（line 269-270）。**`cmd` 作为表单字段透传至面板服务端执行，不经本地 OS shell，故本地命令注入不适用**；风险由面板侧权限/沙箱承担。

权限校验（**良好**）：

| 证据项 | 位置 | 结论 |
|--------|------|------|
| `require_ready()` | line 185-196 | 未配置报错；写操作且 `dry_run` → 阻止（line 192） |
| `check_confirm()` | line 199-204 | `confirm_exec` 时需 `confirm=true` 字段 |
| `from_env()` 默认 `dry_run` / `confirm_exec = true` | line 72-73 | 安全默认，有单测断言（line 568-569） |

**结论**：权限校验与安全默认达到良好水平。实发执行路径需 `dry_run=false` + `confirm=true` 双条件，且 `ensure_login()` 在纯 core 下返回 Err 进一步收敛风险。**建议文档明确**："`baota.exec_shell` 等价于在面板服务器以面板进程权限执行命令"。

---

## 五、OWASP Top 10 / STRIDE 威胁建模

### 5.1 OWASP Top 10 映射

| 类别 | 对应发现 | 说明 |
|------|----------|------|
| A01 失效访问控制 | F-01、F-02 | 终端绕过沙箱；网关无 token 本地直连 |
| A02 密码学失效 | F-03 | 宝塔 TLS 校验禁用 |
| A03 注入 | F-01 | 终端 shell 直执行；`baota.exec_shell` 面板侧执行 |
| A05 安全配置错误 | F-04、**F-07** | 网关静态文件穿越仅字符串检测；**ACP 虚假宣称 L3 `enforced:true`（幻象防护）** |
| A09 日志与监控失效 | F-01、**F-07** | 终端路径未落盘审计；**已声明的安全控制实际未运行，且无告警** |
| A04/A06/A07/A08/A10 | — | 经核查未命中（依赖管理、认证流程、完整性、SSRF 均到位） |

### 5.2 STRIDE 映射

| 维度 | 评估 |
|------|------|
| **Spoofing（伪装）** | 网关 token 恒定时间比对（`auth::verify`）良好；但无 token 放行（F-02）削弱本地认证强度 |
| **Tampering（篡改）** | TLS 禁用（F-03）致传输完整性失效 |
| **Repudiation（抵赖）** | 终端无审计（F-01）削弱事后追责 |
| **Information Disclosure（信息泄露）** | 静态文件穿越（F-04）；凭据以环境变量传递（F-01 低） |
| **Denial of Service（拒绝服务）** | 沙箱超时/输出截断到位；终端无限制（F-01 隐性 DoS） |
| **Elevation of Privilege（提权）** | 终端绕过沙箱即提权面（F-01） |

---

## 六、风险等级清单与修复建议

| ID | 标题 | 分类 | 严重度 | 位置 | 修复建议 |
|----|------|------|--------|------|----------|
| **F-07** | ★ **L2+L3 整条静态安全链未接入运行时（幻象防护）**：`MalwareScanner`(L2) 与 `GuardrailScanner`(L3) 逻辑/单测/ACP 声明就绪，但工具执行路径零调用；仅 L1 真正生效；ACP 虚假宣称 L2/L3 `enforced:true` | A05/A09/STRIDE-Repudiation | **高** | L3：`scanner_l3.rs`（仅 `security/mod.rs:50,70` 测试调用）；L2：`scanner_l2.rs`（仅 `security/mod.rs:41,67` 测试调用，`builtin.rs` 零匹配）；声明点 `serve/acp/session.rs:333`(L2) / `:334`(L3) | **接线**：(a) L3——工具预执行钩子调用 `GuardrailScanner::check_all`（`builtin.rs` 的 write_file/delete_file/make_dir/shell_exec，`baota.rs` 的 write_file/exec_shell）；(b) L2——代码落盘/执行前调用 `MalwareScanner::scan_text`/`scan_file`（堵 eval/反弹 shell/危险模式）。**声明一致性**：接线前 ACP `enforced` 先置 `false`，通过集成测试后再置 `true`；新增"接线"集成测试防回归。**复测口径（正向）**：写 `/etc/passwd`、沙箱逃逸模式应被 `check_all` 在工具预执行阶段 Deny；**负向**：工作区内普通写文件 + 良性代码全放行；**声明一致性**：`acp/session.rs` 的 `enforced` 须与接线状态同步 |
| **F-01** | 终端行模式绕过沙箱护栏（不经 L1/L3、不审计） | A01/A03/A09/STRIDE-EoP | **中** | `terminal/mod.rs:152` | 终端以 `WorkspaceWrite` 沙箱策略运行；或显式标注不受沙箱约束并落盘审计 |
| **F-02** | 网关 WS 握手对本地无 token 连接放行 | A01/STRIDE-Spoofing | **中** | `gateway/server.rs:346` | 默认强制 token；提供显式 opt-out 开关并告警 |
| **F-03** | 宝塔面板 HTTP 客户端禁用 TLS 校验 | A02/STRIDE-Tampering | **中-高** | `baota.rs:103` | 增加 `BAOTA_TLS_VERIFY` 默认 `true`；自签场景显式 opt-in |
| **F-04** | 网关静态文件穿越仅字符串 `..` 检测、未 canonicalize | A05/STRIDE-InfoDisclosure | **低-中** | `gateway/server.rs:473` | 加 `canonicalize` + 前缀校验；禁止符号链接出界 |
| **F-05** | `validate_cwd` canonicalize 失败时回退原值 | 边界 | **低** | `sandbox/mod.rs:302` | canonicalize 失败时直接 deny |
| **F-06** | 面板登录态强依赖外部 bridge（纯 core 无法独立登录） | 信息 | **低** | `baota.rs:134` | 文档说明 + 提供登录状态自检 |

---

## 七、安全态势评分与修复路线

**评分分布**：严重 0 / 高 1（F-07）/ 中 4（F-01~F-04）/ 低 2（F-05~F-06）/ 信息 0 → **整体 C 级**

**修复优先级**：
- **P0（发布前阻塞）**：**F-07**（L2+**L3** 整条静态链接线 + ACP `enforced` 先降为 `false`，通过集成测试后再置 `true`）；F-03（若 `BAOTA_PANEL_URL` 走网络）
- **P1（本迭代）**：F-01（终端对齐沙箱 + 审计）、F-02（网关强制本地 token）
- **P2（下迭代）**：F-04（canonicalize 落地）、F-05（canonicalize 失败 deny）

---

## 八、审计局限与说明

1. 环境无 Rust 工具链（rustc not found），未能执行 `cargo test` / `cargo build` 做编译期与运行时验证；Phase 13 主动验证以源码审阅 + 既有测试断言（如 `baota.rs:568`、`gateway/server.rs:521`）为证据。
2. 项目非 git 仓库（`fatal: not a git repository`），未做版本历史密钥考古；`.env` 含 `MARGUERITE_API_KEY` 已被 `.gitignore` 忽略（低风险）。
3. 渗透测试 `penetration_tests.rs`（P1-P12：rm -rf 拒绝、`./rm` 伪装、sudo、curl|sh、python -c、git push、cwd 逃逸、未知工具、输出截断、超时）已覆盖 L1 护栏，设计有效。
