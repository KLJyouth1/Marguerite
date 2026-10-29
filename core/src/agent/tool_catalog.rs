//! 能力目录（capability catalog）：把「本机到底有什么可用」写进系统提示。
//!
//! ## 为什么需要这个模块
//!
//! 用户实测反馈：智能体不知道工作台内置了真实的浏览器、终端（PowerShell/cmd/bash
//! 真 PTY）、编辑器工作台、TUI、CLI、桌面窗口，也不知道有没有连接 MCP、有哪些技能。
//! 结果就是——明明有真终端，模型却用 `shell_exec` 反复拼命令；明明有浏览器驾驶舱，
//! 模型却说"我无法打开网页"；技能索引里有几十个技能，模型只会 `list_skills` 试探。
//!
//! 提示里只写"共有 N 个工具：a, b, c"是不够的：**名字不携带语义**（`web_video`
//! 到底是录屏还是生成视频？），而且工具之外的**表面（surface）**完全没被描述。
//!
//! ## KV-cache 纪律
//!
//! 本目录属于**会话级静态内容**：同一会话内技能索引与内置表面清单不变，只有工具
//! 可见面（ToolDisclosure 逐轮扩展）会变。因此：
//! - 表面清单 / 技能索引 / 决策规则 → 放在**稳定段**（不随轮次抖动）；
//! - 逐轮变化的"本轮工具名清单"仍由 `tool_guide_text` 负责，本模块只补**分组视图**
//!   并在末尾标注"完整参数见 function schema"，避免把 schema 正文塞进提示。
//!
//! 技能索引走进程级缓存（按 skills 根目录为键）：`scan_index` 要扫几百个 `SKILL.md`，
//! 每轮重扫会让首 token 延迟随轮次线性增长。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use crate::model::llm::ToolDef;

/// 工具分组：`(组名, 该组工具名)`。
///
/// 这是**展示口径**，不是准入控制——没在这里登记的工具会落到"其他"组，不会被隐藏。
/// 新增工具时补一行即可（数据驱动，不改逻辑）。
const TOOL_GROUPS: &[(&str, &[&str])] = &[
    (
        "文件与代码",
        &[
            "read_file",
            "write_file",
            "append_file",
            "edit_file",
            "str_replace_editor",
            "apply_patch",
            "list_dir",
            "find_files",
            "glob_file_search",
            "grep",
            "codebase_search",
            "code_intel",
            "file_info",
            "copy_file",
            "move_file",
            "delete_file",
            "delete_file_permanent",
            "restore_from_trash",
            "empty_trash",
            "make_dir",
            "count_lines",
            "diff",
            "git",
        ],
    ),
    (
        "执行与终端",
        &["shell_exec", "code_exec", "terminal"],
    ),
    (
        "浏览器与网络",
        &[
            "browser",
            "web_fetch",
            "http_request",
            "web_search",
            "web_research",
        ],
    ),
    (
        "桌面与 GUI",
        &[
            "computer_use",
            "system_control",
            "window_manage",
            "clipboard",
            "process_manage",
            "desktop_launch",
            "desktop_action",
            "office_automation",
        ],
    ),
    (
        "媒体与多模态",
        &[
            "media",
            "image_generate",
            "video_generate",
            "code_video",
            "web_video",
            "manim_video",
            "audio_transcribe",
            "audio_speak",
            "blender",
            "vision",
        ],
    ),
    (
        "记忆与知识",
        &[
            "memory_save",
            "memory_search",
            "memory_list",
            "kb_search",
            "kb_ingest",
            "distill_query",
            "json_query",
        ],
    ),
    (
        "协作与计划",
        &[
            "todo_write",
            "research",
            "automation",
            "auto_record",
            "auto_replay",
            "ssh_hosts",
            "ssh_exec",
        ],
    ),
    (
        "技能与发现",
        &["list_skills", "use_skill", "create_skill", "discover_tools"],
    ),
];

/// 内置应用表面（工作台**真实存在**的界面，不是概念）。
///
/// 每项写清"是什么 / 什么时候该用 / 与工具的关系"。这段是用户反复抱怨的核心：
/// 模型此前只知道有工具，不知道有这些**可被它驱动**的表面。
const BUILTIN_SURFACES: &[(&str, &str)] = &[
    (
        "内置浏览器（浏览器驾驶舱）",
        "工作台内嵌的真实 Chromium 浏览器会话：可导航、点击、输入、截图、取无障碍树。\
         需要\"看网页真实渲染\"或复现用户描述的前端问题时用它（`browser` 工具驱动同一实现）；\
         只是读一篇文章用 `web_fetch` 就够，不要为此起浏览器。",
    ),
    (
        "终端（真 PTY）",
        "PowerShell / cmd / pwsh / bash / sh 均可选，运行在工作区根目录、支持 resize 与真实退出码。\
         探查环境、跑构建/测试、看进程用它；**一次性的短命令用 `shell_exec` 更省事**，\
         需要交互式会话（连续多条命令、需要保持 cwd/环境）才开终端。",
    ),
    (
        "编辑器工作台",
        "带语法高亮与 diff 的代码编辑器面板，用户能看到你改了什么。\
         写/改代码走文件工具（`write_file`/`edit_file`/`apply_patch`），\
         需要用户直接查看或手改时，产物会在编辑器里打开。",
    ),
    (
        "产物中心",
        "跨会话汇总所有产物（文档/图片/视频/压缩包），会校验文件是否真的还在磁盘上。\
         交付类任务（PPT/报告/视频）完成后，产物会自动登记到这里，不需要你手写清单。",
    ),
    (
        "团队看板",
        "主智能体 / 子代理 / 专家 / 蜂群 / 团队成员的实时状态与动向。\
         派活之后用户在这里看进度；你不需要自己汇报进度到看板，编排层会自动写。",
    ),
    (
        "TUI 与 CLI",
        "同一引擎的命令行形态：`marguerite`（一次性指令）与 `marguerite tui`（终端界面）。\
         需要无人值守批量执行、或在没有桌面时复现问题时用它；它不是\"另一个模型\"，工具面相同。",
    ),
    (
        "桌面窗口（GUI）",
        "主窗口、桌宠悬浮窗、任务悬浮球。这些是 Tauri 窗口，可用 `computer_use` 操控，\
         但**优先用应用自己的 RPC/工具**（更稳、可审计），图形坐标点击是兜底手段。",
    ),
];

/// 内置表面的紧凑行（子代理 / 团队用）：`(名称, 一句话, 对应工具名)`。
///
/// 名称必须与 [`BUILTIN_SURFACES`] 逐项一致（有回归测试咬合），只是把长段描述压成
/// 一句话 + 工具名，让紧凑目录不牺牲「有哪些面」这条核心信息。无直接工具的面留空串。
const BUILTIN_SURFACES_COMPACT: &[(&str, &str, &str)] = &[
    ("内置浏览器（浏览器驾驶舱）", "真实 Chromium 会话：导航/点击/输入/截图", "browser"),
    ("终端（真 PTY）", "pwsh/cmd/bash 交互会话；短命令用 shell_exec", "terminal、shell_exec"),
    ("编辑器工作台", "用户可实时查看与手改的面板", "write_file、edit_file、apply_patch"),
    ("产物中心", "交付产物自动登记并校验落盘", ""),
    ("团队看板", "子代理/成员进度实时状态", ""),
    ("TUI 与 CLI", "同引擎命令行形态，工具面相同", ""),
    ("桌面窗口（GUI）", "Tauri 窗口；优先 RPC/工具，坐标点击兜底", "computer_use"),
];

/// 能力快照：会话级稳定的那部分（技能索引）。
#[derive(Debug, Clone, Default)]
pub struct CapabilitySnapshot {
    /// 技能索引（Level-1：name + description 摘要）
    pub skills: Vec<(String, String)>,
    /// 扫描到的技能总数（去重后）
    pub skill_total: usize,
    /// 是否因上限被截断（诚实标注）
    pub skill_truncated: bool,
}

/// 技能索引上限：目录里有 300+ 个 SKILL.md 时，全量塞进提示会挤掉真正的任务上下文。
/// 只列前 N 个（按名字排序，稳定），并在末尾如实写"另有 M 个未列出，用 list_skills 查看"。
const MAX_SKILLS_IN_PROMPT: usize = 60;
/// 单条技能描述的展示上限（Level-1 只用于意图匹配，不需要长文）
const MAX_SKILL_DESC_CHARS: usize = 96;

/// 进程级技能索引缓存（键 = skills 根目录的规范化字符串）。
///
/// 同一进程内 skills 目录不会在会话中途搬家；`skills.reload` 后由调用方
/// 用 [`invalidate_skill_cache`] 主动失效，避免读到旧索引。
fn skill_cache() -> &'static Mutex<BTreeMap<String, Arc<CapabilitySnapshot>>> {
    static C: OnceLock<Mutex<BTreeMap<String, Arc<CapabilitySnapshot>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 进程级技能目录（由编排层装配时登记一次）。
///
/// 子代理 / 团队成员拿不到应用 `Config`（它们只持有 ModelRouter 与 ToolRegistry），
/// 所以用这个只写一次的登记点把技能目录传下去；未登记时返回空快照（诚实降级，
/// 不会编造技能名）。
static REGISTERED_SKILLS_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

/// 登记技能根目录（编排层启动时调用；幂等，后到的调用被忽略）。
pub fn register_skills_dir(dir: std::path::PathBuf) {
    let _ = REGISTERED_SKILLS_DIR.set(dir);
}

/// 取已登记技能目录的快照（未登记 → 空快照）。
pub fn global_snapshot() -> Arc<CapabilitySnapshot> {
    match REGISTERED_SKILLS_DIR.get() {
        Some(dir) => snapshot(dir),
        None => Arc::new(CapabilitySnapshot::default()),
    }
}

/// 失效技能索引缓存（`skills.reload` / `skills.save` 之后必须调用）。
pub fn invalidate_skill_cache() {
    if let Ok(mut m) = skill_cache().lock() {
        m.clear();
    }
}

/// 取技能索引快照（带缓存）。读取失败返回空快照——**绝不 panic**，
/// 也绝不在提示里编造技能名。
pub fn snapshot(skills_dir: &Path) -> Arc<CapabilitySnapshot> {
    let key = skills_dir.to_string_lossy().to_string();
    if let Ok(m) = skill_cache().lock() {
        if let Some(hit) = m.get(&key) {
            return hit.clone();
        }
    }
    let snap = Arc::new(scan_skills(skills_dir));
    if let Ok(mut m) = skill_cache().lock() {
        m.insert(key, snap.clone());
    }
    snap
}

fn scan_skills(skills_dir: &Path) -> CapabilitySnapshot {
    let loader = crate::skills::loader::SkillLoader::new(skills_dir);
    let scan = match loader.scan_index() {
        Ok(s) => s,
        Err(_) => return CapabilitySnapshot::default(),
    };
    let total = scan.skills.len();
    let mut items: Vec<(String, String)> = scan
        .skills
        .iter()
        .filter_map(|s| {
            let name = s.name.trim();
            if name.is_empty() {
                return None;
            }
            let desc: String = s
                .description
                .trim()
                .chars()
                .take(MAX_SKILL_DESC_CHARS)
                .collect();
            Some((name.to_string(), desc))
        })
        .collect();
    // 名字排序：提示文本字节级稳定，否则整段前缀缓存每轮失效。
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let truncated = items.len() > MAX_SKILLS_IN_PROMPT;
    items.truncate(MAX_SKILLS_IN_PROMPT);
    CapabilitySnapshot {
        skills: items,
        skill_total: total,
        skill_truncated: truncated,
    }
}

/// 把工具名按分组归类；未登记的工具进"其他"。返回稳定顺序（组内按名字排序）。
fn grouped(tools: &[ToolDef]) -> Vec<(&'static str, Vec<&str>)> {
    let mut out: Vec<(&'static str, Vec<&str>)> = Vec::new();
    let mut used: Vec<&str> = Vec::new();
    for (group, names) in TOOL_GROUPS {
        let mut hit: Vec<&str> = tools
            .iter()
            .filter(|t| names.contains(&t.name.as_str()))
            .map(|t| t.name.as_str())
            .collect();
        hit.sort_unstable();
        for n in &hit {
            used.push(n);
        }
        if !hit.is_empty() {
            out.push((group, hit));
        }
    }
    let mut rest: Vec<&str> = tools
        .iter()
        .map(|t| t.name.as_str())
        .filter(|n| !used.contains(n))
        .collect();
    rest.sort_unstable();
    if !rest.is_empty() {
        out.push(("其他（含 MCP 远端工具）", rest));
    }
    out
}

/// 完整能力目录（主智能体 / lead 用）。
///
/// 只在**非直答轮**注入（直答轮工具面为空，见 `tool_guide_text` 的空面分支）。
pub fn catalog_text(tools: &[ToolDef], snap: &CapabilitySnapshot) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("## 本机可用能力与选择规则（会话级，稳定）\n\n");

    // ── A. 内置表面 ──
    out.push_str("### A. 内置应用表面（真实存在、可被你驱动）\n");
    for (name, desc) in BUILTIN_SURFACES {
        out.push_str(&format!("- **{name}**：{desc}\n"));
    }

    // ── B. 工具分组 ──
    out.push_str("\n### B. 本轮可见工具（按用途分组，完整参数看 function schema）\n");
    if tools.is_empty() {
        out.push_str("- 本轮没有暴露任何工具（直答轮或当前模式封禁），不要声称调用过工具。\n");
    } else {
        for (group, names) in grouped(tools) {
            out.push_str(&format!("- **{group}**：{}\n", names.join("、")));
        }
    }

    // ── C. 技能索引 ──
    out.push_str("\n### C. 技能索引（Level-1 元数据；命中后用 `use_skill` 加载正文）\n");
    if snap.skills.is_empty() {
        out.push_str("- 当前未发现技能（技能目录为空或不可读）。需要时用 `list_skills` 确认，不要编造技能名。\n");
    } else {
        for (name, desc) in &snap.skills {
            if desc.is_empty() {
                out.push_str(&format!("- `{name}`\n"));
            } else {
                out.push_str(&format!("- `{name}` — {desc}\n"));
            }
        }
        if snap.skill_truncated {
            out.push_str(&format!(
                "- （共 {} 个技能，此处只列前 {} 个；其余用 `list_skills` 查看）\n",
                snap.skill_total, MAX_SKILLS_IN_PROMPT
            ));
        }
    }

    // ── D. MCP ──
    out.push_str("\n### D. MCP 远端工具\n");
    out.push_str(
        "- 已连接的 MCP 服务，其工具会出现在上面「其他（含 MCP 远端工具）」组里，名字通常带服务前缀。\n\
         - 如果那里没有带前缀的工具，说明**当前没有连接 MCP 服务**——如实告诉用户去「插件」页配置，\
         不要假装调用过 MCP。\n",
    );

    // ── E. 决策规则 ──
    out.push_str("\n### E. 选择规则（先判层级，再选具体工具）\n");
    out.push_str(
        "1. **能用内置表面就不用坐标点击**：写文件/跑命令/开网页走对应工具，`computer_use` 只做兜底。\n\
         2. **技能优先于临场发明**：索引里已有技能覆盖该任务时（命名与描述命中），先 `use_skill` 加载再动手；\
         没有匹配才自己写流程。\n\
         3. **MCP 优先于手写等价实现**：需要外部系统（GitHub、Notion、数据库等）且已连接对应 MCP 时用它。\n\
         4. **一次性命令用 `shell_exec`，交互式会话用终端**；不要为了「显得在做事」开一堆终端。\n\
         5. **静态度/观点/改写/讲解 → 直接回答**，不调工具；实时事实（天气/股价/最新文档）→ 联网检索一次即可。\n\
         6. **证据够了就停**：拿到足以支撑结论的输出后立即作答，不要继续堆工具调用。\n\
         7. **能力缺失要如实说**：不在工具面、不在技能索引、也没有 MCP → 明说缺什么、去哪里配。\n",
    );
    out
}

/// 紧凑能力目录（子代理 / 专家 / 团队成员 / 蜂群 worker 用）。
///
/// 这些智能体的上下文预算更紧（要放子任务描述与中间结果），所以只给：
/// 内置面每名一行（名称 + 一句话 + 工具名，不复制主面长文本）+
/// 组名 + 每组代表工具（≤5）+ 技能名的前缀列表 + 三句决策规则。
pub fn compact_catalog_text(tools: &[ToolDef], snap: &CapabilitySnapshot) -> String {
    let mut out = String::with_capacity(768);
    out.push_str("## 你的可用能力（与会话内其它智能体同一套工具面）\n");
    // 内置面缺失时子代理会退化成「写脚本盲控电脑」，所以这一段不能省；
    // 省的是描述长度（每名一句话），不是覆盖面。
    out.push_str("- 内置应用表面（真实存在、可被驱动）：\n");
    for (name, brief, surface_tools) in BUILTIN_SURFACES_COMPACT {
        if surface_tools.is_empty() {
            out.push_str(&format!("  - {name}｜{brief}\n"));
        } else {
            out.push_str(&format!("  - {name}｜{brief}（工具：{surface_tools}）\n"));
        }
    }
    let groups = grouped(tools);
    if groups.is_empty() {
        out.push_str("- 本轮没有工具：直接基于已知信息完成子任务，不要声称调用过工具。\n");
    } else {
        for (group, names) in groups {
            let shown: Vec<&str> = names.iter().take(5).copied().collect();
            let more = names.len().saturating_sub(shown.len());
            let tail = if more > 0 {
                format!("（另有 {more} 个）")
            } else {
                String::new()
            };
            out.push_str(&format!("- {group}：{}{tail}\n", shown.join("、")));
        }
    }
    if !snap.skills.is_empty() {
        let names: Vec<&str> = snap.skills.iter().take(12).map(|(n, _)| n.as_str()).collect();
        out.push_str(&format!("- 技能（前 12 个）：{}\n", names.join("、")));
    }
    out.push_str(
        "规则：内置浏览器/终端/编辑器都是真实能力，能直接用就不要说做不到；\
         技能命中先 `use_skill`；外部系统优先 MCP；证据够就停；缺能力如实上报给委派你的智能体。\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, desc: &str) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn catalog_describes_builtin_surfaces_even_with_no_tools() {
        let text = catalog_text(&[], &CapabilitySnapshot::default());
        // 用户抱怨的核心：内置浏览器/终端/编辑器/CLI/TUI 必须被写进提示
        for needle in ["内置浏览器", "终端（真 PTY）", "编辑器工作台", "TUI", "CLI", "桌面窗口"] {
            assert!(text.contains(needle), "能力目录缺少「{needle}」：\n{text}");
        }
        assert!(text.contains("本轮没有暴露任何工具"), "空工具面必须显式说明");
    }

    #[test]
    fn catalog_groups_known_tools_and_puts_unknown_into_mcp_bucket() {
        let tools = vec![
            tool("read_file", "读文件"),
            tool("shell_exec", "执行命令"),
            tool("browser", "浏览器"),
            tool("gh__create_issue", "MCP: 建 issue"),
        ];
        let text = catalog_text(&tools, &CapabilitySnapshot::default());
        assert!(text.contains("**文件与代码**：read_file"), "{text}");
        assert!(text.contains("**执行与终端**：shell_exec"), "{text}");
        assert!(text.contains("**浏览器与网络**：browser"), "{text}");
        assert!(
            text.contains("其他（含 MCP 远端工具）\"") || text.contains("其他（含 MCP 远端工具）**：gh__create_issue"),
            "未登记工具必须落进 MCP/其他桶：{text}"
        );
    }

    #[test]
    fn catalog_lists_skill_index_and_honest_truncation() {
        let snap = CapabilitySnapshot {
            skills: vec![
                ("alpha".to_string(), "做 A 类事".to_string()),
                ("beta".to_string(), String::new()),
            ],
            skill_total: 120,
            skill_truncated: true,
        };
        let text = catalog_text(&[], &snap);
        assert!(text.contains("`alpha` — 做 A 类事"), "{text}");
        assert!(text.contains("`beta`"), "{text}");
        assert!(text.contains("共 120 个技能"), "截断必须如实标注：{text}");
    }

    #[test]
    fn compact_catalog_is_shorter_and_keeps_decision_rules() {
        let tools = vec![
            tool("read_file", "读"),
            tool("write_file", "写"),
            tool("shell_exec", "执行"),
            tool("browser", "浏览"),
            tool("computer_use", "桌面"),
        ];
        let snap = CapabilitySnapshot {
            skills: vec![("s1".to_string(), "d".to_string())],
            skill_total: 1,
            skill_truncated: false,
        };
        let full = catalog_text(&tools, &snap);
        let compact = compact_catalog_text(&tools, &snap);
        assert!(
            compact.len() < full.len() / 2,
            "紧凑版应显著更短：full={} compact={}",
            full.len(),
            compact.len()
        );
        assert!(compact.contains("内置浏览器"), "紧凑版也要说明内置表面是真实能力");
        assert!(compact.contains("use_skill"));
    }

    #[test]
    fn compact_catalog_carries_builtin_surfaces_without_long_text() {
        let text = compact_catalog_text(&[], &CapabilitySnapshot::default());
        // 七个面一个都不能少：子代理此前只知道有工具、不知道有这些表面。
        for needle in [
            "内置浏览器（浏览器驾驶舱）",
            "终端（真 PTY）",
            "编辑器工作台",
            "产物中心",
            "团队看板",
            "TUI 与 CLI",
            "桌面窗口（GUI）",
        ] {
            assert!(text.contains(needle), "紧凑目录缺少内置面「{needle}」：\n{text}");
        }
        // 每名要带对应工具名（compact 的本意：一句话 + 工具名）。
        assert!(text.contains("（工具：browser）"), "{text}");
        assert!(text.contains("terminal、shell_exec"), "{text}");
        // 但不能把主面长文本复制过来。
        assert!(
            !text.contains("复现用户描述的前端问题"),
            "紧凑版不得复制主面长描述"
        );
    }

    #[test]
    fn compact_surface_table_names_match_full_surface_table() {
        let full: Vec<&str> = BUILTIN_SURFACES.iter().map(|(n, _)| *n).collect();
        let compact: Vec<&str> = BUILTIN_SURFACES_COMPACT.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(full, compact, "两张表面表的名称与顺序必须逐项一致");
    }

    #[test]
    fn skill_snapshot_of_missing_dir_is_empty_not_panic() {
        let snap = snapshot(Path::new("Z:\\definitely-missing-skills-dir"));
        assert!(snap.skills.is_empty());
        assert_eq!(snap.skill_total, 0);
    }
}
