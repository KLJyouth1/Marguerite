//! 意图预览与高风险判定（M2-C1 Intent Preview）
//!
//! 对高影响动作（删除/重置/覆盖/终止/批量等）做结构化意图识别：
//! - 纯函数：不持有状态，输入 `(工具名, 参数 JSON)`，输出结构化 [`ToolIntention`]
//! - 风险分级：`High`（破坏性/不可逆）/ `Medium`（一般写）/ `Low`（只读）
//! - 判定规则精确到工具名 + 参数特征，所有判定路径由表驱动单测 100% 覆盖
//!
//! 消费方：
//! - [`crate::chain::executor`] 在审批确认分支解析意图，`high_risk` 命中的审批
//!   提级为 [`RiskLevel::High`] 并随 Approval 事件注入 `intent` 字段；
//! - WebUI ApprovalCard / GUI ConfirmCard 用 `intent` 渲染「操作类型 / 目标对象 /
//!   影响范围 / 预估副作用」结构化确认卡。

use serde::{Deserialize, Serialize};

use super::gateway::RiskLevel;

/// 操作类型（确认卡头部文案与 C5 信任类目分组的依据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    /// 删除 / 清空 / 移除
    Delete,
    /// 重置 / 抹除 / 清库
    Reset,
    /// 覆盖式写入（truncate overwrite）
    Overwrite,
    /// 终止进程 / 会话 / 服务
    Terminate,
    /// 命令 / 代码执行
    Execute,
    /// 出网访问 / 外部请求
    Network,
    /// 一般写入（创建 / 修改 / 追加）
    Write,
    /// 只读查询
    Read,
    /// 未识别（fail-closed 按 Medium 确认）
    Unknown,
}

impl IntentKind {
    /// 确认卡展示用的中文标签
    pub fn label(self) -> &'static str {
        match self {
            IntentKind::Delete => "删除",
            IntentKind::Reset => "重置",
            IntentKind::Overwrite => "覆盖",
            IntentKind::Terminate => "终止",
            IntentKind::Execute => "执行",
            IntentKind::Network => "网络访问",
            IntentKind::Write => "写入",
            IntentKind::Read => "读取",
            IntentKind::Unknown => "未识别操作",
        }
    }
}

/// 结构化意图预览（随 Approval 事件透传给双端确认卡）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolIntention {
    /// 操作类型
    pub kind: IntentKind,
    /// 是否高风险（拦截判定核心；high_risk 命中 → RiskLevel::High）
    pub high_risk: bool,
    /// 是否不可逆（无法撤销/恢复现场）
    pub irreversible: bool,
    /// 是否破坏性（对既有数据/状态有破坏）
    pub destructive: bool,
    /// 目标对象（从参数提取：path / command / host / name 等）
    pub target: Option<String>,
    /// 影响范围描述
    pub scope: String,
    /// 预估副作用清单
    pub side_effects: Vec<String>,
    /// 确认原因（展示在卡片上的拦截说明）
    pub confirmation: String,
    /// 判定风险等级
    pub risk: RiskLevel,
}

impl ToolIntention {
    /// 是否属高影响类目（供 C1 拦截展示与 C5 信任分组复用）
    pub fn impactful(&self) -> bool {
        matches!(
            self.kind,
            IntentKind::Delete
                | IntentKind::Reset
                | IntentKind::Overwrite
                | IntentKind::Terminate
        )
    }

    /// 目标对象展示值
    pub fn target_label(&self) -> &str {
        self.target.as_deref().unwrap_or("—")
    }
}

// ───────────────────────────── 特征判定表 ─────────────────────────────

/// 删除类工具名（小写后缀匹配；`contains` 覆盖 `fs.delete`、`file_delete` 等变体）
const DELETE_TOOLS: &[&str] = &[
    "delete",
    "remove",
    "rm",
    "trash",
    "purge",
    "unlink",
    "wipe",
    "del",
    "fs_delete",
    "file_delete",
];

/// 重置 / 抹除类工具名
const RESET_TOOLS: &[&str] = &[
    "reset",
    "factory_reset",
    "reinit",
    "reinitialize",
    "wipe",
    "format",
    "clear_all",
    "reset_all",
    "vault_reset",
    "session_reset",
];

/// 终止类工具名
const TERMINATE_TOOLS: &[&str] = &[
    "terminate",
    "kill",
    "stop",
    "shutdown",
    "taskkill",
    "kill_process",
    "stop_service",
    "cancel",
];

/// 执行类工具名
const EXEC_TOOLS: &[&str] = &[
    "exec",
    "run",
    "run_command",
    "command",
    "terminal",
    "shell",
    "exec_command",
    "computer_use",
    "desktop",
    "sandbox_exec",
    "eval",
    "execute",
];

/// 出网类工具名
const NETWORK_TOOLS: &[&str] = &[
    "fetch",
    "http",
    "request",
    "network",
    "web",
    "url",
    "download",
    "post",
    "web_search",
    "browse",
    "open_url",
];

/// 写入类工具名
const WRITE_TOOLS: &[&str] = &[
    "write",
    "write_file",
    "fs_write",
    "file_write",
    "edit_file",
    "append",
    "create",
    "mkdir",
    "mkdirs",
    "mkfile",
    "mv",
    "move",
    "rename",
    "copy",
    "cp",
    "update_file",
    "overwrite",
];

/// 只读类工具名（list/get/read/status/search/query 等）
const READ_TOOLS: &[&str] = &[
    "read",
    "read_file",
    "fs_read",
    "file_read",
    "list",
    "ls",
    "get",
    "status",
    "search",
    "query",
    "peek",
    "info",
    "summary",
    "stats",
    "usage",
    "inspect",
    "check",
];

/// 默认转回收站的工具（精确全名白名单）。
///
/// T1 起 `delete_file` 的默认语义是「删除转回收站、可恢复」，永久删除必须显式
/// 走 `delete_file_permanent`。其余 delete 类工具（memory_delete / skill_delete /
/// 各后端自定义删除动词）删除对象不是用户工作区文件，无回收站兜底 —— 一律维持
/// 不可逆的保守判定：宁可过度确认，不可弱化确认。
const TRASH_DEFAULT_TOOLS: &[&str] = &["delete_file"];

/// 命令字符串中的高风险 token（命中 → Execute/删除 相关命令提级 High）
const HIGH_RISK_COMMAND_TOKENS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "rm -f",
    "rm -r",
    "--recursive",
    "-Recurse",
    "remove-item",
    "rd /s",
    "rmdir /s",
    "del /f",
    "del /s",
    "format-volume",
    "format ",
    "mkfs",
    "fdisk",
    "diskpart",
    "shutdown",
    "taskkill /f",
    "kill -9",
    "cipher /w",
    "--purge",
    "--delete",
    "--force",
    " -f ",
];

fn norm_tool(tool: &str) -> String {
    tool.trim().to_lowercase()
}

fn has_token(name: &str, table: &[&str]) -> bool {
    // 按命名空间分隔符分拆出语义段：`file_delete`→[file, delete]；`fs.delete`→[fs, delete]
    let segs: Vec<&str> = name
        .split(['.', ':', '/', '_', '-'])
        .filter(|s| !s.is_empty())
        .collect();
    table.iter().any(|cand| {
        // 段精确命中（覆盖命名空间变体，避免 "rm" 误中 "terminate" 之类短词子串）
        if segs.contains(cand) {
            return true;
        }
        // 长词（≥4 字符）宽容子串匹配：覆盖无分隔符连写变体（如 fsdelete、readstatus）
        cand.len() >= 4 && name.contains(cand)
    })
}

fn str_arg(args: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let obj = args.as_object()?;
    for key in keys {
        if let Some(val) = obj.get(*key) {
            if let Some(s) = val.as_str() {
                if !s.trim().is_empty() {
                    return Some(s.trim().to_string());
                }
            } else if !val.is_string() && (val.is_number() || val.is_boolean()) {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn flag(args: &serde_json::Value, keys: &[&str]) -> bool {
    let Some(obj) = args.as_object() else {
        return false;
    };
    keys.iter().any(|key| match obj.get(*key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => {
            let low = s.to_lowercase();
            low == "true" || low == "yes" || low == "1" || low == "force"
        }
        Some(serde_json::Value::Number(n)) => n.as_u64() == Some(1) || n.as_i64() == Some(1),
        _ => false,
    })
}

/// 参数中是否存在通配批量信号（glob / 多路径 / batch）
fn batch_marker(args: &serde_json::Value) -> Option<usize> {
    let obj = args.as_object()?;
    if flag(args, &["batch", "bulk"]) {
        return Some(2);
    }
    if flag(args, &["recursive", "recurse", "deep", "all"]) {
        return Some(2);
    }
    if let Some(serde_json::Value::Array(items)) = obj.get("paths").or_else(|| obj.get("targets"))
    {
        if items.len() > 1 {
            return Some(items.len());
        }
    }
    if let Some(glob) = str_arg(args, &["glob", "pattern"]) {
        if glob.contains('*') || glob.contains('?') {
            return Some(1);
        }
    }
    None
}

/// 命令参数命令串是否含破坏性 token
fn command_is_destructive(command: &str) -> bool {
    let low = command.to_lowercase();
    HIGH_RISK_COMMAND_TOKENS
        .iter()
        .any(|tok| low.contains(&tok.to_lowercase()))
}

/// 对 (工具名, 参数) 做结构化意图识别。
///
/// 纯函数，无副作用；同输入恒同输出（便于单测与审计）。
pub fn analyze(tool: &str, args: &serde_json::Value) -> ToolIntention {
    let name = norm_tool(tool);
    // 工具名末端段（剥离命名空间前缀，如 `fs.delete` → `delete`）
    let tail = name.rsplit(['.', ':', '/']).next().unwrap_or(&name);
    let tail = tail.trim().to_string();

    let path_arg = str_arg(args, &["path", "file", "target", "name", "id", "key"]);
    let command_arg = str_arg(args, &["command", "cmd", "code", "script", "text"]);
    let host_arg = str_arg(args, &["host", "url", "uri", "endpoint"]);
    let recursive = flag(args, &["recursive", "recurse", "deep", "all"]);
    let force = flag(args, &["force", "overwrite", "truncate", "replace"]) || {
        command_arg
            .as_deref()
            .map(command_is_destructive)
            .unwrap_or(false)
    };
    let batch = batch_marker(args);

    let kind;
    let mut irreversible = false;
    let mut destructive = false;
    let mut high = false;
    let mut target = None;
    let mut scope;
    let mut side_effects: Vec<String> = Vec::new();

    // ── 删除类 ──
    if has_token(&tail, DELETE_TOOLS) {
        kind = IntentKind::Delete;
        destructive = true;
        irreversible = true;
        high = true;
        target = path_arg.clone();
        // 可逆信号：
        // - 参数显式 to_trash/trash/recycle/restore/dry_run；
        // - 工具名是回收站恢复动词（restore/recover + trash/recycle 同时出现），
        //   与 vault_restore/session_restore 这类「恢复出厂设置」语义区分开；
        // - 白名单工具（delete_file）且未带 permanent 信号。
        // 永久删除必须显式：permanent 类参数或 `*_permanent` 工具名 → 不可逆。
        let trash_restore_tool = (tail.contains("restore") || tail.contains("recover"))
            && (tail.contains("trash") || tail.contains("recycle"));
        let permanent = flag(
            args,
            &["permanent", "skip_trash", "no_trash", "hard_delete", "force_delete"],
        ) || tail.contains("permanent");
        if !permanent
            && (flag(args, &["to_trash", "trash", "recycle", "restore", "dry_run"])
                || trash_restore_tool
                || TRASH_DEFAULT_TOOLS.contains(&name.as_str()))
        {
            irreversible = false;
            side_effects.push("目标将移入回收站，可恢复".to_string());
        } else if permanent {
            side_effects.push("永久删除，无回收站兜底，不可恢复".to_string());
        } else {
            side_effects.push("目标数据将被移除，无法恢复".to_string());
        }
        scope = match &target {
            Some(t) => format!("作用于对象：{t}"),
            None => "作用于删除工具给定的目标".to_string(),
        };
        if let Some(n) = batch {
            scope.push_str(&format!("（批量 {} 项）", if n > 1 { n.to_string() } else { "未知".into() }));
            side_effects.push("批量删除可能波及多个无关目标".to_string());
        }
    }
    // ── 重置/抹除类 ──
    else if has_token(&tail, RESET_TOOLS) {
        kind = IntentKind::Reset;
        destructive = true;
        irreversible = true;
        high = true;
        target = path_arg;
        scope = "将重置/抹除目标状态或数据".to_string();
        side_effects.push("既有配置/数据将被清空，无法恢复".to_string());
        if let Some(n) = batch {
            side_effects.push(format!("将重置批量目标（{} 项）", if n > 1 { n.to_string() } else { "未知".into() } ));
        }
    }
    // ── 终止类 ──
    else if has_token(&tail, TERMINATE_TOOLS) {
        kind = IntentKind::Terminate;
        high = true;
        irreversible = true;
        target = path_arg.or_else(|| str_arg(args, &["pid", "process", "session_id", "session_key", "name"]));
        scope = "将终止目标进程/会话/服务".to_string();
        side_effects.push("被终止对象立即停止，运行中数据可能丢失".to_string());
    }
    // ── 执行类 ──
    else if has_token(&tail, EXEC_TOOLS) {
        kind = IntentKind::Execute;
        target = command_arg.clone();
        scope = command_arg
            .clone()
            .map(|c| format!("将执行命令：{c}"))
            .unwrap_or_else(|| "将执行指定命令/代码".to_string());
        if let Some(cmd) = &command_arg {
            if command_is_destructive(cmd) {
                destructive = true;
                irreversible = true;
                high = true;
                side_effects.push("命令含破坏性操作（删除/格式化/强制参数）".to_string());
            } else {
                side_effects.push("命令将直接在本机执行".to_string());
            }
        } else {
            side_effects.push("将直接执行命令".to_string());
        }
        if let Some(n) = batch {
            high = true;
            side_effects.push(format!("批量执行 {} 项命令", if n > 1 { n.to_string() } else { "未知".into() }));
        }
        // 附带文件系统操作 flag（force/recursive）的终端命令视为破坏性
        if (force || recursive) && !destructive {
            destructive = true;
            irreversible = true;
            high = true;
            side_effects.push("携带强制/递归参数，可能覆盖已有结果".to_string());
        }
        if high {
            scope.push_str("（高危）");
        }
    }
    // ── 出网类 ──
    else if has_token(&tail, NETWORK_TOOLS) {
        kind = IntentKind::Network;
        target = host_arg.clone().or(command_arg);
        scope = host_arg
            .clone()
            .map(|h| format!("将发起网络访问：{h}"))
            .unwrap_or_else(|| "将发起外部网络请求".to_string());
        side_effects.push("将产生对外网络流量与外部依赖".to_string());
    }
    // ── 写入类 ──
    else if has_token(&tail, WRITE_TOOLS) {
        target = path_arg;
        // 工具名本身为截断/覆盖语义，或参数携带覆盖信号 → 覆盖类
        if tail == "truncate" || tail == "overwrite" || flag(args, &["overwrite", "force", "truncate", "replace"]) {
            kind = IntentKind::Overwrite;
            destructive = true;
            irreversible = true;
            high = true;
            scope = target
                .clone()
                .map(|t| format!("将覆盖写入目标：{t}"))
                .unwrap_or_else(|| "将覆盖写入目标".to_string());
            side_effects.push("目标原有内容将被替换，无法恢复".to_string());
        } else {
            kind = IntentKind::Write;
            scope = target
                .clone()
                .map(|t| format!("将写入/创建目标：{t}"))
                .unwrap_or_else(|| "将写入/创建目标".to_string());
            side_effects.push("将修改本地文件/记忆状态".to_string());
        }
    }
    // ── 只读类 ──
    else if has_token(&tail, READ_TOOLS) || tail.ends_with("list") || tail.ends_with("status") {
        kind = IntentKind::Read;
        scope = "只读查询，不修改任何状态".to_string();
    }
    // ── 未知（fail-closed 保守确认） ──
    else {
        kind = IntentKind::Unknown;
        scope = "操作类型未识别，按需确认".to_string();
    }

    // 风险/确认原因收口
    let risk = if high {
        RiskLevel::High
    } else if matches!(
        kind,
        IntentKind::Write | IntentKind::Execute | IntentKind::Network | IntentKind::Unknown
    ) {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    let impactful = matches!(
        kind,
        IntentKind::Delete
            | IntentKind::Reset
            | IntentKind::Overwrite
            | IntentKind::Terminate
    );
    let confirmation = if impactful {
        format!("{}操作：{}。{}", kind.label(), scope, side_effects.join("；"))
    } else if high {
        format!("高影响操作：{}", scope)
    } else {
        format!("{}操作：{}", kind.label(), scope)
    };

    ToolIntention {
        kind,
        high_risk: high,
        irreversible,
        destructive,
        target,
        scope,
        side_effects,
        confirmation,
        risk,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Case {
        tool: &'static str,
        args: serde_json::Value,
        kind: IntentKind,
        high: bool,
        irreversible: bool,
    }

    fn run(cases: &[Case]) {
        for c in cases {
            let it = analyze(c.tool, &c.args);
            assert_eq!(
                it.kind, c.kind,
                "tool={:?} args={:?}: kind 不符 (got {:?})",
                c.tool, c.args, it.kind
            );
            assert_eq!(
                it.high_risk, c.high,
                "tool={:?} args={:?}: high_risk 不符 (got {:?}, bool={})",
                c.tool, c.args, it.risk, it.high_risk
            );
            assert_eq!(
                it.irreversible, c.irreversible,
                "tool={:?} args={:?}: irreversible 不符",
                c.tool, c.args
            );
        }
    }

    #[test]
    fn delete_detection() {
        run(&[
            Case { tool: "fs_delete", args: json!({"path": "/tmp/a.txt"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "file.delete", args: json!({"target": "D:/x/report.txt"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "delete", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "remove", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "fs_delete", args: json!({"path": "/tmp/a.txt", "to_trash": true}), kind: IntentKind::Delete, high: true, irreversible: false },
        ]);
    }

    /// T1 删除转回收站：白名单 `delete_file` 默认可逆；永久删除必须显式。
    #[test]
    fn trash_default_vs_explicit_permanent() {
        run(&[
            // delete_file 默认转回收站 → 可逆
            Case { tool: "delete_file", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: false },
            // 永久删除：显式动词 / 显式参数 → 不可逆
            Case { tool: "delete_file_permanent", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "delete_file", args: json!({"path": "x", "permanent": true}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "fs_delete", args: json!({"path": "x", "force_delete": true}), kind: IntentKind::Delete, high: true, irreversible: true },
            // 回收站恢复动词 → 可逆
            Case { tool: "restore_from_trash", args: json!({"id": "3"}), kind: IntentKind::Delete, high: true, irreversible: false },
            // 非白名单删除动词维持不可逆保守判定（不可弱化）
            Case { tool: "memory_delete", args: json!({"path": "m.md"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "skill_delete", args: json!({"name": "s"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "unlink", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: true },
            Case { tool: "purge", args: json!({"path": "x"}), kind: IntentKind::Delete, high: true, irreversible: true },
        ]);
        let it = analyze("delete_file", &json!({"path": "x"}));
        assert!(it.side_effects.iter().any(|s| s.contains("可恢复")));
        let it = analyze("delete_file_permanent", &json!({"path": "x"}));
        assert!(it.side_effects.iter().any(|s| s.contains("永久删除")));
    }

    #[test]
    fn delete_batch_flagged() {
        let it = analyze("fs_delete", &json!({"paths": ["a", "b", "c"]}));
        assert_eq!(it.kind, IntentKind::Delete);
        assert!(it.high_risk);
        assert!(it.scope.contains("批量"));
        let it2 = analyze("delete", &json!({"path": "x", "recursive": true}));
        assert!(it2.high_risk);
        assert!(it2.scope.contains("批量"));
    }

    #[test]
    fn reset_detection() {
        run(&[
            Case { tool: "reset", args: json!({"name": "kb"}), kind: IntentKind::Reset, high: true, irreversible: true },
            Case { tool: "vault_reset", args: json!({}), kind: IntentKind::Reset, high: true, irreversible: true },
            Case { tool: "factory_reset", args: json!({"confirm": true}), kind: IntentKind::Reset, high: true, irreversible: true },
            Case { tool: "format", args: json!({"path": "E:"}), kind: IntentKind::Reset, high: true, irreversible: true },
        ]);
    }

    #[test]
    fn terminate_detection() {
        run(&[
            Case { tool: "kill", args: json!({"pid": 4321}), kind: IntentKind::Terminate, high: true, irreversible: true },
            Case { tool: "terminate", args: json!({"session_key": "s1"}), kind: IntentKind::Terminate, high: true, irreversible: true },
            Case { tool: "taskkill", args: json!({"process": "wechat.exe"}), kind: IntentKind::Terminate, high: true, irreversible: true },
            Case { tool: "stop_service", args: json!({"name": "mysql"}), kind: IntentKind::Terminate, high: true, irreversible: true },
        ]);
    }

    #[test]
    fn exec_destructive_command() {
        run(&[
            Case { tool: "exec", args: json!({"command": "rm -rf /"}), kind: IntentKind::Execute, high: true, irreversible: true },
            Case { tool: "exec", args: json!({"command": "del /f /s *.tmp"}), kind: IntentKind::Execute, high: true, irreversible: true },
            Case { tool: "exec", args: json!({"command": "Remove-Item -Recurse D:\\old"}), kind: IntentKind::Execute, high: true, irreversible: true },
            Case { tool: "exec", args: json!({"command": "echo hello"}), kind: IntentKind::Execute, high: false, irreversible: false },
            Case { tool: "terminal", args: json!({"command": "ls -la"}), kind: IntentKind::Execute, high: false, irreversible: false },
        ]);
    }

    #[test]
    fn exec_batch_raised() {
        let it = analyze("exec", &json!({"command": "run_tests.sh", "batch": true}));
        assert_eq!(it.kind, IntentKind::Execute);
        assert!(it.high_risk);
        assert!(it.scope.contains("高危"));
    }

    #[test]
    fn network_detection() {
        run(&[
            Case { tool: "fetch", args: json!({"url": "https://api.xxx.com"}), kind: IntentKind::Network, high: false, irreversible: false },
            Case { tool: "http_request", args: json!({"host": "1.2.3.4"}), kind: IntentKind::Network, high: false, irreversible: false },
            Case { tool: "open_url", args: json!({"url": "https://x"}), kind: IntentKind::Network, high: false, irreversible: false },
        ]);
    }

    #[test]
    fn overwrite_vs_write() {
        run(&[
            Case { tool: "fs_write", args: json!({"path": "a.md", "content": "x"}), kind: IntentKind::Write, high: false, irreversible: false },
            Case { tool: "write_file", args: json!({"path": "a.md", "content": "x", "overwrite": true}), kind: IntentKind::Overwrite, high: true, irreversible: true },
            Case { tool: "edit_file", args: json!({"path": "a.rs", "content": "x", "force": true}), kind: IntentKind::Overwrite, high: true, irreversible: true },
            Case { tool: "fs_write", args: json!({"path": "a.md", "content": "x", "truncate": true}), kind: IntentKind::Overwrite, high: true, irreversible: true },
            Case { tool: "append", args: json!({"path": "log.txt", "text": "line"}), kind: IntentKind::Write, high: false, irreversible: false },
        ]);
    }

    #[test]
    fn read_and_low_risk() {
        run(&[
            Case { tool: "read_file", args: json!({"path": "a.md"}), kind: IntentKind::Read, high: false, irreversible: false },
            Case { tool: "fs_read", args: json!({"path": "x"}), kind: IntentKind::Read, high: false, irreversible: false },
            Case { tool: "kb_search", args: json!({"query": "q"}), kind: IntentKind::Read, high: false, irreversible: false },
        ]);
    }

    #[test]
    fn read_like_suffix() {
        let it = analyze("session_list", &json!({}));
        assert_eq!(it.kind, IntentKind::Read);
        assert!(!it.high_risk);
        let it = analyze("pet_status", &json!({}));
        assert_eq!(it.kind, IntentKind::Read);
    }

    #[test]
    fn unknown_is_conservative_medium() {
        let it = analyze("mystery_tool", &json!({"foo": 1}));
        assert_eq!(it.kind, IntentKind::Unknown);
        assert!(!it.high_risk);
        assert_eq!(it.risk, RiskLevel::Medium);
    }

    #[test]
    fn case_insensitive_tool_name() {
        let it = analyze("FS_DELETE", &json!({"path": "x"}));
        assert_eq!(it.kind, IntentKind::Delete);
        assert!(it.high_risk);
    }

    #[test]
    fn namespace_prefixed_name_tail() {
        let it = analyze("filesystem.delete_path", &json!({"path": "x"}));
        assert_eq!(it.kind, IntentKind::Delete);
        assert!(it.high_risk);
    }

    #[test]
    fn impactful_kind_membership() {
        for k in [IntentKind::Delete, IntentKind::Reset, IntentKind::Overwrite, IntentKind::Terminate] {
            let mut it = analyze("delete", &json!({"path": "x"}));
            it.kind = k;
            assert!(it.impactful());
        }
        let it = analyze("fs_read", &json!({"path": "x"}));
        assert!(!it.impactful());
    }

    #[test]
    fn serialization_shape_for_frontend() {
        let it = analyze("delete", &json!({"path": "/tmp/secret.md"}));
        let v = serde_json::to_value(&it).unwrap();
        assert_eq!(v["kind"], "delete");
        assert_eq!(v["high_risk"], true);
        assert_eq!(v["irreversible"], true);
        assert_eq!(v["target"], "/tmp/secret.md");
        assert!(v["scope"].as_str().unwrap().contains("secret.md"));
        assert!(!v["side_effects"].as_array().unwrap().is_empty());
    }

    /// 穷举：所有高危 token 都被命令判定识别（每 token 独立命中）
    #[test]
    fn all_high_risk_tokens_detected() {
        for tok in HIGH_RISK_COMMAND_TOKENS {
            let it = analyze("exec", &json!({"command": format!("echo before; {tok}; echo after") }));
            assert!(
                it.high_risk,
                "高危 token 未被识别: {tok:?} (got kind={:?})",
                it.kind
            );
        }
    }

    /// FAIL-CLOSED 保证：破坏性工具集合与枚举集合一致（防止误删集合条目导致漏判）
    #[test]
    fn delete_tool_table_matches_tailnames() {
        for t in DELETE_TOOLS {
            let it = analyze(t, &json!({"path": "x"}));
            assert_eq!(
                it.kind,
                IntentKind::Delete,
                "DELETE_TOOLS 条目 {} 未判为 Delete",
                t
            );
        }
    }
}
