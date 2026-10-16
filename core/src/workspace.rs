//! 工作区解析与「可执行提示」（P0-1b）
//!
//! # 为什么需要这个模块
//!
//! 实测的失败链第 1 步是「会话 `project_path = null` —— 工作区未绑定」：
//! 智能体不知道文件该写到哪，于是**猜**相对路径，`str_replace_editor` 1 ms 即失败，
//! 随后进入每 2–4 秒一轮的死循环重试。真正缺的不是"再一个工作区变量"，而是两件事：
//!
//! 1. **一个权威解析器**：把「显式绑定 → 应用数据目录 → 进程 cwd」这三级回落
//!    收敛到一处。此前 `orchestration::default_workspace()` 与
//!    `gateway::sandbox_paths::sandbox_path_root()` 各写一半，谁都不完整。
//! 2. **可执行提示**：未绑定时必须告诉模型/用户「文件现在会落到哪、怎么改」。
//!    旧文案只有一句 `path escapes workspace: <rel>` —— 模型既不知道根在哪，
//!    也不知道怎么绑定，于是同一个错误重复到触发重复失败检测为止。
//!
//! # 设计约束
//!
//! - 纯解析 + 文案，**不做权限判定**（逃逸判定仍在 `tools::fs_path::safe_resolve`）。
//! - `guidance()` 的文案是给**模型**看的系统提示片段，必须自包含：根目录、当前
//!   绑定状态、以及绑定动作，缺一项就会重演「猜路径」。
//! - 未绑定**不是错误**：回落到应用数据目录是合法的默认行为，只是必须可见。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// 工作区来源：决定提示口吻（已绑定 = 陈述事实；未绑定 = 告知落地位置 + 绑定方法）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceSource {
    /// 会话/项目显式绑定（用户在 UI 选择了项目目录）。
    Bound,
    /// 未绑定 → 应用数据目录（`Config::data_dir()`）。不是用户项目。
    AppData,
    /// 数据目录不可用 → 进程当前工作目录。
    ProcessCwd,
    /// 显式指定了路径，但它此刻**不是目录**（已被移动/删除/从未创建）。
    /// 刻意不静默改指别处：注册表里所有工具都以这个根在跑，指引必须与工具
    /// 行为一致，否则模型按指引写的路径照样 1ms 失败。
    Missing,
}

impl WorkspaceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkspaceSource::Bound => "bound",
            WorkspaceSource::AppData => "app_data",
            WorkspaceSource::ProcessCwd => "process_cwd",
            WorkspaceSource::Missing => "missing",
        }
    }
}

/// 解析后的工作区（根目录 + 来源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub source: WorkspaceSource,
}

impl Workspace {
    /// 解析工作区根目录。
    ///
    /// 优先级：`explicit`（会话绑定的项目目录）> 应用数据目录 > 进程 cwd。
    /// `explicit` 指向一个**存在且是目录**的路径时才视为已绑定；若它此刻不是
    /// 目录（被删除/移动/拼错），仍以它作为根并标记 [`WorkspaceSource::Missing`]
    /// ——因为工具注册表已经按这个根在跑，改指别处只会让指引与工具行为不一致。
    pub fn resolve(explicit: Option<&Path>) -> Self {
        if let Some(p) = explicit {
            let trimmed = p.to_path_buf();
            if trimmed.is_dir() {
                // 刻意**不** canonicalize：Windows 上会带上 `\\?\` 前缀，
                // 那会让 guidance / 工具报错里出现一串用户认不出来的路径。
                // 逃逸判定由 `tools::fs_path::safe_resolve` 自己做（它内部会 canonicalize）。
                return Self {
                    root: trimmed,
                    source: WorkspaceSource::Bound,
                };
            }
            // 显式路径此刻不可用：保留它作为根（首次写入会自动创建），
            // 并由 guidance 说明「这不是默认目录、必要时请重新绑定」。
            return Self {
                root: trimmed,
                source: WorkspaceSource::Missing,
            };
        }
        Self {
            root: fallback().0,
            source: fallback().1,
        }
    }

    /// 是否已绑定真实项目目录。
    pub fn is_bound(&self) -> bool {
        matches!(self.source, WorkspaceSource::Bound)
    }

    /// 面向模型的系统提示片段（自包含：根目录 + 状态 + 绑定方法）。
    ///
    /// 这不是装饰文案：它是「猜路径 → 1 ms 失败 → 死循环」这条链的第一道防线。
    pub fn guidance(&self) -> String {
        let root = self.root.display().to_string();
        match self.source {
            WorkspaceSource::Bound => format!(
                "【工作区】{root}（已绑定项目目录）。\n\
                 - 所有文件工具的工作区根就是这个目录：相对路径相对它解析；\
                 绝对路径必须位于它内部，否则会被拒绝。\n\
                 - 交付产物请写到这个目录下，并用工作区内相对路径回读。",
            ),
            WorkspaceSource::AppData | WorkspaceSource::ProcessCwd => format!(
                "【工作区】{root}（未绑定项目目录，当前使用{desc}）。\n\
                 - 文件会写入这个目录；这不是用户的项目目录，只是默认落点。\n\
                 - 如果任务面向某个真实项目，请先让用户在界面上把这个会话绑定到\
                 项目目录（会话的 projectPath 指向它），再按绑定后的根读写；\
                 在绑定前，请使用上面的绝对路径，**不要猜测**其它目录。",
                desc = match self.source {
                    WorkspaceSource::AppData => "应用数据目录",
                    _ => "进程当前工作目录",
                },
            ),
            WorkspaceSource::Missing => format!(
                "【工作区】{root}（已指定，但该目录当前不存在：可能被移动、删除或从未创建）。\n\
                 - 所有文件工具仍以这个目录为工作区根：首次写入时会自动创建它。\n\
                 - 请只在这个目录内读写；如果它对应的项目已经不在原处，\
                 请让用户重新绑定项目目录后再继续，**不要猜测**其它目录顶替。",
            ),
        }
    }

    /// 工具失败时附带的路径提示：把「根在哪 + 该怎么写」说清楚。
    pub fn path_hint(&self) -> String {
        format!(
            "工作区根：{}；请改用该目录内的相对路径",
            self.root.display()
        )
    }

    /// 状态视图（供 `workspace.status` RPC 与 doctor 项使用）。
    pub fn status_json(&self) -> Value {
        json!({
            "ok": true,
            "root": self.root.display().to_string(),
            "source": self.source.as_str(),
            "bound": self.is_bound(),
            "guidance": self.guidance(),
        })
    }
}

/// 默认根与来源（无显式绑定时）：应用数据目录优先，进程 cwd 兜底。
///
/// 一次算出「根 + 来源」两件事：分成两个函数会让 `Config::data_dir()` 被
/// 求值两次，且存在两次之间环境变化的极小窗口（根与来源不一致）。
fn fallback() -> (PathBuf, WorkspaceSource) {
    let data = crate::persistence::config::Config::data_dir();
    if data.is_dir() {
        (data, WorkspaceSource::AppData)
    } else {
        (
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            WorkspaceSource::ProcessCwd,
        )
    }
}

/// 当前会话绑定的项目目录 → 解析成工作区；None 表示未绑定。
///
/// 统一的入口：网关的 chat / team / plan 三条 dispatch 路径都应经此函数取根，
/// 避免各处重复实现「有绑定用绑定、没有用 data_dir」。
pub fn resolve_for_session(project_path: Option<&str>) -> Workspace {
    let explicit = project_path
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Path::new);
    Workspace::resolve(explicit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mgt_ws_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn explicit_existing_dir_is_bound() {
        let d = temp_dir("bound");
        let ws = Workspace::resolve(Some(&d));
        assert!(ws.is_bound(), "显式绑定且目录存在必须视为已绑定");
        assert_eq!(ws.source, WorkspaceSource::Bound);
        assert!(ws.root.is_dir());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_explicit_dir_keeps_the_declared_root_and_says_so() {
        let ghost = std::env::temp_dir().join("mgt_ws_definitely_missing_dir");
        let _ = std::fs::remove_dir_all(&ghost);
        let ws = Workspace::resolve(Some(&ghost));
        assert!(!ws.is_bound(), "目录不存在不得谎称已绑定");
        assert_eq!(
            ws.root, ghost,
            "必须保留用户指定的根——工具注册表正是按它跑的，改指别处会让指引与工具行为不一致"
        );
        assert_eq!(ws.source, WorkspaceSource::Missing);
        let g = ws.guidance();
        assert!(g.contains("不存在"), "必须说明目录当前不存在");
        assert!(g.contains("重新绑定"), "必须给出可执行的下一步");
        assert!(g.contains("不要猜测"), "必须堵住'猜一个目录顶替'这条路");
    }

    #[test]
    fn none_resolves_to_an_existing_root() {
        let ws = Workspace::resolve(None);
        assert!(ws.root.is_dir(), "默认工作区根必须存在");
        assert!(!ws.is_bound());
    }

    #[test]
    fn guidance_is_self_contained_for_both_states() {
        let d = temp_dir("guidance");
        let bound = Workspace::resolve(Some(&d));
        let g = bound.guidance();
        assert!(g.contains(&d.display().to_string()), "必须写出根目录");
        assert!(g.contains("已绑定"), "已绑定状态要说明");
        assert!(!g.contains("不要猜测"), "已绑定时不要求绑定动作");

        let unbound = Workspace::resolve(None);
        let g2 = unbound.guidance();
        assert!(g2.contains(&unbound.root.display().to_string()));
        assert!(g2.contains("未绑定"), "未绑定状态要说明");
        assert!(
            g2.contains("不要猜测"),
            "未绑定时必须给出可执行动作，否则模型继续猜路径"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 长会话里同一装配必须产出同一段指引（KV-cache 前缀稳定性的前提）。
    #[test]
    fn guidance_is_byte_stable_for_the_same_root() {
        let d = temp_dir("stable");
        let a = Workspace::resolve(Some(&d));
        let b = Workspace::resolve(Some(&d));
        assert_eq!(a.guidance(), b.guidance());
        assert_eq!(a.path_hint(), b.path_hint());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 护栏：指引里反引号包住的 RPC 名必须**真实存在**。
    ///
    /// 本仓已因 `tools.catalog` 宣称 `file_read` / `terminal_exec` / `kb_search`
    /// 等编造工具名吃过教训（前端徽标恒不显示、模型一调就失败）。给模型的系统
    /// 提示同理：写一个不存在的方法名，等于教模型去调一个必然失败的接口。
    /// 该用例即挡住"提示词漂移到不存在的 API"。
    #[test]
    fn guidance_names_no_fabricated_rpc() {
        fn phantom_rpcs(g: &str) -> Vec<String> {
            let mut out = Vec::new();
            let mut rest = g;
            while let Some(open) = rest.find('`') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('`') else { break };
                let name = &after[..close];
                // 只校验形如 `a.b` 的 RPC 名；纯代码片段（如 `x`）不在协议清单里，跳过。
                if name.contains('.')
                    && !crate::gateway::protocol::SUPPORTED_METHODS.contains(&name)
                {
                    out.push(name.to_string());
                }
                rest = &after[close + 1..];
            }
            out
        }

        let d = temp_dir("phantom");
        for ws in [
            Workspace::resolve(Some(&d)),
            Workspace::resolve(None),
            Workspace::resolve(Some(&std::env::temp_dir().join("mgt_ws_phantom_missing"))),
        ] {
            let bad = phantom_rpcs(&ws.guidance());
            assert!(
                bad.is_empty(),
                "指引引用了不存在的 RPC：{bad:?}（完整指引：{}）",
                ws.guidance()
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn path_hint_names_the_root() {
        let d = temp_dir("hint");
        let ws = Workspace::resolve(Some(&d));
        assert!(ws.path_hint().contains(&d.display().to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn status_json_is_honest_about_binding() {
        let d = temp_dir("status");
        let v = Workspace::resolve(Some(&d)).status_json();
        assert_eq!(v["bound"], json!(true));
        assert_eq!(v["source"], json!("bound"));
        assert_eq!(v["ok"], json!(true));
        assert!(v["guidance"].as_str().is_some_and(|s| !s.is_empty()));

        let ghost = std::env::temp_dir().join("mgt_ws_status_missing");
        let _ = std::fs::remove_dir_all(&ghost);
        let v2 = Workspace::resolve(Some(&ghost)).status_json();
        assert_eq!(v2["bound"], json!(false));
        assert_eq!(v2["source"], json!("missing"));
        assert_eq!(v2["root"], json!(ghost.display().to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn resolve_for_session_ignores_blank_paths() {
        let ws = resolve_for_session(Some("   "));
        assert!(!ws.is_bound(), "空白字符串不得当成绑定");
        let ws2 = resolve_for_session(None);
        assert_eq!(ws.source, ws2.source);
    }
}
