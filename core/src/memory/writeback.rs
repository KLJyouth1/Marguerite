//! 记忆**写回**：跨项目层的**唯一写入口**（闸门 [`promote`] 的执行者）。
//!
//! # 为什么需要它
//!
//! `assistant.brief.get` 读的是 [`crate::memory::cross_project::cross_project_search`]，
//! 但写入侧此前只有读：跨项目记忆来自**其他 AI 工具**写的 Markdown，本引擎从不往回写 ——
//! 「越用越懂你」只有前半句，会读、不会学。同时 [`promote`] 里的准入/取代规则没有任何
//! 真实写路径在使用（闸门建好了，却没有门）。
//!
//! 本模块把这两件事接起来：**先过闸门，再落盘**，并且是全仓唯一的写入口。
//!
//! # 三条口径（读代码前先读这三条）
//!
//! 1. **准入**：一律先走 [`promote::decide`]（阈值只在 `promote` 定义一次，这里不再复制）。
//!    [`Verdict::Drop`] 直接返回，**一个字节都不写**；`Promote` 写跨项目层；
//!    `Keep` 写项目局部层。
//! 2. **分层口径**：现有跨项目 store 是 `~/.trae-cn/memory/projects/<project>/*.md`，
//!    那是**别的工具的文件**，里面没有层级概念，硬塞一层标签等于污染别人的格式。
//!    所以本模块另建一份事实库 `Config::config_dir()/memory/cross_project_facts.jsonl`，
//!    用记录上的 `tier` 字段区分两层：`cross_project`（Promote）/ `project_local`（Keep）。
//!    检索侧（[`cross_project_search`] / 召回侧）**两层都回**，结果里 `kind` 标 `fact` ——
//!    「值得写但不值得跨项目」的事实仍然学得到，只是不带跨项目权重。
//!    [`cross_project_search`]: crate::memory::cross_project::cross_project_search
//! 3. **同 key 冲突**：key = `source + "::" + 归一化文本`（去首尾/重复空白、转小写）。
//!    重写同一 key 时旧记录**不删**，只标 `superseded_by = 新 id`（与
//!    [`promote::WriteGate`] 同一语义），检索只回未被取代的那条：既不让两条值同时
//!    进上下文（「该信哪个」不该丢给模型猜），又留下「它什么时候被改过」的追溯。
//!
//! 写入失败（磁盘不可写）返回 `written = false` 并带原因：`verdict` 如实反映**闸门决策**，
//! `written` 表示**是否真的落盘**，两者不混为一谈。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::memory::promote::{self, Entry, Verdict};
use crate::memory::tiered::MemoryTierVariant;

/// 事实库文件名（`Config::config_dir()/memory/<此名>`）。
pub const FACT_FILE: &str = "cross_project_facts.jsonl";

/// 事实库最多保留多少条记录（含被取代的留痕）。
///
/// 事实库是**追加型**的：同 key 重写会留一条历史。没有上限就等于「用得越久、文件越大、
/// 每次写入重写得越慢」，而写入路径是每轮对话都会走的。
/// 超限先淘汰最旧的**已被取代**记录（纯留痕），不够再淘汰最旧的活跃记录。
const MAX_RECORDS: usize = 1024;

/// 重要度起步分：低于 [`promote::KEEP_THRESHOLD`]，只有信号才能把一条抬过线。
const BASE_IMPORTANCE: f64 = 0.20;
/// 够长才可能是「一句完整事实」而不是「嗯」「继续」。
const LENGTH_BONUS: f64 = 0.15;
/// 「一句完整事实」的字符数下界。
const DURABLE_MIN_CHARS: usize = 24;
/// 具体标记（数字/路径/URL/`key=value`/反引号标识符）的加分：
/// 这类内容**能被别的项目直接复用**，是跨项目层真正想要的东西。
const CONCRETE_BONUS: f64 = 0.20;
/// 显式要求记住的口径词（与 [`promote::EXPLICIT_BOOST`] 配套；全部小写，匹配前统一小写）。
const EXPLICIT_CUES: &[&str] = &[
    "记住",
    "记得",
    "以后都",
    "以后一律",
    "始终",
    "一律",
    "必须",
    "不允许",
    "不要再",
    "remember",
    "always",
    "never",
    "must",
];
/// key 的字符上限（key 进 JSONL，不设上限等于让一条长文当主键）。
const MAX_KEY_CHARS: usize = 200;

/// 一次写回的结果（**如实**：`verdict` 是闸门决策，`written` 是是否真的落盘）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberOutcome {
    /// 是否真的落盘（`false` 有两种可能：被闸门挡下，或磁盘写入失败，看 `reason`）
    pub written: bool,
    /// 闸门结论的字符串形态：`"promote"` / `"keep"` / `"drop"`
    pub verdict: &'static str,
    /// 冲突键（`source::归一化文本`）
    pub key: String,
    /// 同 key 冲突时被取代的旧记录（`key#id`）；没有冲突就是 `None`
    pub superseded: Option<String>,
    /// 人话原因（进日志 / 观测面 / 测试断言）
    pub reason: String,
}

/// 事实库里的一条记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactRecord {
    /// 自增 id（写入顺序；检索结果里的 `line` 就是它）
    pub id: u64,
    /// 冲突键：`source::归一化文本`
    pub key: String,
    /// 事实所属项目
    pub project: String,
    /// 写回来源（如 `tiered` / 工具名）
    pub source: String,
    /// 事实正文
    pub text: String,
    /// `cross_project`（Promote）/ `project_local`（Keep）
    pub tier: String,
    /// 闸门的准入分（重要度 + 显式口径词加分，即 `promote::gate_score` 的结果）。
    /// 落盘留存，便于事后解释「它为什么在跨项目层」。
    pub importance: f64,
    /// 写入时刻（unix 毫秒，由调用方传入，不读墙钟）
    pub created_at_ms: u64,
    /// 同 key 冲突时被哪条新记录取代；有值即「留痕但不参与检索」
    pub superseded_by: Option<u64>,
}

/// 事实库路径：`Config::config_dir()/memory/cross_project_facts.jsonl`。
///
/// 与 `memory::synapse` 同一定位（`Config` 目录下的运行期数据）；
/// 单测用 `Config::set_config_dir_override` 把整个目录换成临时目录即可隔离，
/// 不需要（也不允许）碰真实用户目录。
pub fn store_path() -> PathBuf {
    crate::persistence::config::Config::config_dir()
        .join("memory")
        .join(FACT_FILE)
}

/// 闸门结论 → 对外的字符串口径。
pub fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Promote => "promote",
        Verdict::Keep => "keep",
        Verdict::Drop => "drop",
    }
}

/// 结论 → 落盘层级（`None` 表示不落盘）。
fn tier_label(verdict: Verdict) -> Option<&'static str> {
    match verdict {
        Verdict::Promote => Some("cross_project"),
        Verdict::Keep => Some("project_local"),
        Verdict::Drop => None,
    }
}

/// key 用的归一化：去首尾与重复空白、转小写。
///
/// **只做这一件事**：任何更激进的「语义归一」（去标点、抽数字）都会让两条不同的事实
/// 撞成一个 key，从而互相取代 —— 那是静默丢数据，比不归一糟得多。
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// 项目名清洗：去控制字符、限长；空则回落 `default`（项目名会进检索结果的展示字段）。
fn normalize_project(project: &str) -> String {
    let cleaned: String = project
        .trim()
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(64)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

/// 来源清洗（同上，限长更短）。
fn normalize_source(source: &str) -> String {
    let cleaned: String = source
        .trim()
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(32)
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

/// 冲突键：`source::归一化文本`（按字符截断，绝不切在非字符边界上）。
fn build_key(source: &str, text: &str) -> String {
    let norm = normalize_text(text);
    let norm: String = norm.chars().take(MAX_KEY_CHARS).collect();
    format!("{source}::{norm}")
}

/// 文本里是否出现显式「要求记住」的口径词（返回命中的那个词，供原因文案用）。
fn explicit_cue(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    EXPLICIT_CUES.iter().copied().find(|cue| lower.contains(cue))
}

/// 是否含**可复用**的具体标记：数字、路径分隔符、`::`、`=`、URL、反引号标识符。
fn has_concrete_marker(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        || text.contains('/')
        || text.contains('\\')
        || text.contains("::")
        || text.contains('=')
        || text.contains('`')
        || text.contains("http")
}

/// 事实重要度（确定性：不读墙钟、不读配置，只看文本本身）。
///
/// 信号只留三个，每个都能向用户解释：① 是否够长（一句完整陈述）；② 是否含具体标记
/// （数字/路径/命令 —— 别处能直接复用的东西）；③ 是否显式要求记住。
/// 三项都命中的上限是 `0.20 + 0.15 + 0.20 = 0.55`，再加 [`promote::EXPLICIT_BOOST`]
/// 正好越过 `PROMOTE_THRESHOLD`：**只有「够长 + 具体 + 显式」才进跨项目层**。
fn importance_of(text: &str) -> f64 {
    let mut score = BASE_IMPORTANCE;
    if text.chars().count() >= DURABLE_MIN_CHARS {
        score += LENGTH_BONUS;
    }
    if has_concrete_marker(text) {
        score += CONCRETE_BONUS;
    }
    score.min(1.0)
}

/// 是否允许落盘。
///
/// 单测默认**不写**：配置目录与凭据目录同源（AGENTS.md §4），未显式注入
/// `Config::set_config_dir_override` 的用例一律跳过落盘，免得 `cargo test` 在
/// 真实用户目录里留下一堆测试文本。生产构建恒为 `true`（这道门只在 `cfg(test)` 下存在）。
#[cfg(test)]
fn writes_allowed() -> bool {
    crate::persistence::config::Config::current_config_dir_override().is_some()
}

/// 是否允许落盘（生产：恒允许）。
#[cfg(not(test))]
fn writes_allowed() -> bool {
    true
}

/// **唯一写入口**：先过闸门，再决定写哪层、是否取代旧值。
///
/// - `project`：事实所属项目（进检索结果的展示字段；空串回落 `default`）
/// - `text`：事实正文（`Drop` 时一个字节都不写）
/// - `source`：写回来源（如 `tiered` / 工具名），与归一化文本一起构成冲突键
/// - `now_ms`：**决策时刻**（unix 毫秒），不读墙钟 —— 测试要能钉住时间
pub fn remember_fact(project: &str, text: &str, source: &str, now_ms: u64) -> RememberOutcome {
    let project = normalize_project(project);
    let source = normalize_source(source);
    let text = text.trim();
    let key = build_key(&source, text);
    let importance = importance_of(text);
    let cue = explicit_cue(text);
    let explicit = cue.is_some();
    let at = (now_ms / 1000) as i64;

    let entry = Entry {
        key: Some(key.clone()),
        text: text.to_string(),
        importance,
        last_hit: at,
        explicit,
        at,
        tier: MemoryTierVariant::ShortTerm,
    };
    let verdict = promote::decide(&entry);
    let score = promote::gate_score(&entry);

    let Some(tier) = tier_label(verdict) else {
        return RememberOutcome {
            written: false,
            verdict: verdict_label(verdict),
            key,
            superseded: None,
            reason: format!(
                "准入分 {score:.2} 低于写入门 {:.2}：不落盘",
                promote::KEEP_THRESHOLD
            ),
        };
    };

    if !writes_allowed() {
        return RememberOutcome {
            written: false,
            verdict: verdict_label(verdict),
            key,
            superseded: None,
            reason: format!("准入分 {score:.2} 过了写入门；单测未隔离配置目录，跳过落盘"),
        };
    }

    let path = store_path();
    let mut records = load_from(&path);
    let id = records.iter().map(|r| r.id).max().unwrap_or(0) + 1;

    // 同 key 冲突：旧记录不删，只标 superseded_by（与 `promote::WriteGate` 同一语义）。
    let mut superseded = None;
    for old in records.iter_mut().rev() {
        if old.superseded_by.is_none() && old.key == key {
            old.superseded_by = Some(id);
            superseded = Some(format!("{}#{}", old.key, old.id));
            break;
        }
    }

    records.push(FactRecord {
        id,
        key: key.clone(),
        project: project.clone(),
        source: source.clone(),
        text: text.to_string(),
        tier: tier.to_string(),
        importance: score,
        created_at_ms: now_ms,
        superseded_by: None,
    });
    prune(&mut records);

    let mut reason = match verdict {
        Verdict::Promote => format!(
            "准入分 {score:.2} ≥ {:.2}：写入跨项目层（id={id}）",
            promote::PROMOTE_THRESHOLD
        ),
        _ => format!(
            "准入分 {score:.2} 在 [{:.2}, {:.2}) 区间：写入项目局部层（id={id}）",
            promote::KEEP_THRESHOLD,
            promote::PROMOTE_THRESHOLD
        ),
    };
    if let Some(cue) = cue {
        reason.push_str(&format!("；命中显式口径词「{cue}」"));
    }
    if let Some(old) = &superseded {
        reason.push_str(&format!("；同 key 旧记录 {old} 已标记取代"));
    }

    match save_to(&path, &records) {
        Ok(()) => RememberOutcome {
            written: true,
            verdict: verdict_label(verdict),
            key,
            superseded,
            reason,
        },
        Err(e) => RememberOutcome {
            written: false,
            verdict: verdict_label(verdict),
            key,
            superseded: None,
            reason: format!("{reason}；但落盘失败：{e}"),
        },
    }
}

/// 当前**有效**（未被取代）的事实记录；供检索侧读。
///
/// 读失败/文件不存在都回空表：事实库是增强，读不到不该让检索报错。
pub fn active_records() -> Vec<FactRecord> {
    load_from(&store_path())
        .into_iter()
        .filter(|r| r.superseded_by.is_none())
        .collect()
}

/// 按 id 取一条**仍有效**的记录（供渐进披露第 2 步展开）。
pub fn record_by_id(id: u64) -> Option<FactRecord> {
    load_from(&store_path())
        .into_iter()
        .find(|r| r.id == id && r.superseded_by.is_none())
}

/// 用户主动「忘掉」一条写回事实（R-02 可撤销性的最后一环）。
///
/// **是硬删除，不是打墓碑**：用户说「忘掉这条」时，留一条 `superseded_by` 记录
/// 意味着正文仍留在磁盘上 —— 那与「忘掉」相悖。
///
/// 返回 `Ok(true)` 表示确实删了；`Ok(false)` 表示没有这条 id（幂等：重试删除不报错）。
/// 落盘失败返回 `Err` 且**不改动磁盘** —— 调用方据此告诉用户删除没生效。
pub fn forget_fact(id: u64) -> Result<bool, String> {
    let path = store_path();
    let records = load_from(&path);
    // 先把日志要用的字段拷出来：`records` 随后会被 `into_iter()` 移走。
    let Some((key, project)) = records
        .iter()
        .find(|r| r.id == id)
        .map(|r| (r.key.clone(), r.project.clone()))
    else {
        return Ok(false);
    };
    let keep: Vec<FactRecord> = records.into_iter().filter(|r| r.id != id).collect();
    save_to(&path, &keep).map_err(|e| format!("落盘失败: {e}"))?;
    tracing::info!(
        target: "memory.writeback",
        id,
        key = %key,
        project = %project,
        "用户删除了一条写回事实"
    );
    Ok(true)
}

/// 读整个事实库（含被取代的留痕）。坏行跳过而不是整体失败：
/// 一条被手改坏的记录不该让整个记忆库消失。
fn load_from(path: &Path) -> Vec<FactRecord> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| serde_json::from_str::<FactRecord>(line.trim()).ok())
        .collect()
}

/// 全量重写（无 BOM）：事实库规模受 [`MAX_RECORDS`] 约束，重写成本可预期。
fn save_to(path: &Path, records: &[FactRecord]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = String::new();
    for record in records {
        let line = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    std::fs::write(path, buf)
}

/// 超限淘汰：先淘汰最旧的**已被取代**记录（纯留痕），不够再淘汰最旧的活跃记录。
fn prune(records: &mut Vec<FactRecord>) {
    if records.len() <= MAX_RECORDS {
        return;
    }
    let excess = records.len() - MAX_RECORDS;
    let mut victims: Vec<usize> = (0..records.len())
        .filter(|i| records[*i].superseded_by.is_some())
        .take(excess)
        .collect();
    if victims.len() < excess {
        victims.extend(
            (0..records.len())
                .filter(|i| records[*i].superseded_by.is_none())
                .take(excess - victims.len()),
        );
    }
    victims.sort_unstable();
    victims.dedup();
    for i in victims.into_iter().rev() {
        records.remove(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::config::Config;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// 每个用例一个独立配置目录（线程局部覆盖），绝不碰真实用户目录。
    fn isolated_dir(name: &str) -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mgt-writeback-{name}-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Config::set_config_dir_override(dir.clone());
        dir
    }

    fn raw_lines(path: &Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// 低分事实：闸门判 `Drop`，**文件里不能多出任何记录**。
    #[test]
    fn a_low_score_fact_is_dropped_and_never_reaches_the_disk() {
        let dir = isolated_dir("drop");
        let path = store_path();
        let outcome = remember_fact("proj-a", "嗯", "tiered", 1_000);
        assert_eq!(outcome.verdict, "drop", "两个字的闲聊属于噪声档: {outcome:?}");
        assert!(!outcome.written, "被挡下的条目不得声称写入");
        assert!(outcome.superseded.is_none());
        assert!(outcome.reason.contains("不落盘"), "{}", outcome.reason);
        assert!(
            !path.exists(),
            "Drop 不得创建事实库文件：{}",
            path.display()
        );
        assert!(raw_lines(&path).is_empty());
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    /// 高分事实：`Promote` → 跨项目层，且**能被 `cross_project_search` 检索到**
    /// （读侧此前只有 MD 文件，写回的事实若读不到，等于没写）。
    #[test]
    fn a_promoted_fact_is_searchable_through_cross_project_search() {
        let dir = isolated_dir("promote_search");
        let text = "记住：生产发布只用 canary，不许直接全量（脚本在 /deploy/canary.sh）";
        let outcome = remember_fact("marguerite", text, "tiered", 1_700_000_000_000);
        assert_eq!(outcome.verdict, "promote", "{outcome:?}");
        assert!(outcome.written, "{outcome:?}");

        let hits = crate::memory::cross_project::cross_project_search(&json!({
            "query": "生产发布 canary /deploy/canary.sh",
            "limit": 12,
        }));
        let items = hits["items"].as_array().expect("items 必须是数组");
        let hit = items
            .iter()
            .find(|i| i["file"] == FACT_FILE)
            .unwrap_or_else(|| panic!("写回的事实必须能被检索到：{hits}"));
        assert_eq!(hit["kind"], "fact");
        assert_eq!(hit["project"], "marguerite");
        assert!(
            hit["excerpt"].as_str().unwrap_or("").contains("canary"),
            "excerpt 必须含事实内容：{hit}"
        );

        // 展开（渐进披露第 2 步）同样要能用，否则命中行只读得到摘要。
        let line = hit["line"].as_u64().expect("line 是记录 id");
        let expanded = crate::memory::cross_project::cross_project_expand(&json!({
            "project": "marguerite",
            "file": FACT_FILE,
            "line": line,
        }));
        assert_eq!(expanded["expanded"], true, "{expanded}");
        assert!(
            expanded["hit"].as_str().unwrap_or("").contains("canary"),
            "{expanded}"
        );
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    /// 同 key 重写：旧记录被 supersede（不删），检索只回新值。
    #[test]
    fn rewriting_the_same_fact_supersedes_the_old_record_and_search_returns_one_value() {
        let dir = isolated_dir("supersede");
        let text = "记住：本地集成测试统一用 C:\\tgt 作为 target 目录";
        let first = remember_fact("marguerite", text, "tiered", 1_700_000_000_000);
        let second = remember_fact("marguerite", text, "tiered", 1_700_000_060_000);
        assert!(first.written && second.written, "{first:?} / {second:?}");
        assert_eq!(first.superseded, None, "首次写入没有旧值可取代");
        assert_eq!(
            second.superseded,
            Some(format!("{}#{}", first.key, 1)),
            "第二次写入必须标出被取代的旧记录"
        );

        let records: Vec<FactRecord> = raw_lines(&store_path())
            .iter()
            .map(|l| serde_json::from_str(l).expect("每行都是一条记录"))
            .collect();
        assert_eq!(records.len(), 2, "旧记录不删，只留痕");
        assert_eq!(records[0].superseded_by, Some(records[1].id));
        assert_eq!(records[1].superseded_by, None);

        let hits = crate::memory::cross_project::cross_project_search(&json!({
            "query": "集成测试 target 目录 C:\\tgt",
            "limit": 12,
        }));
        let ours: Vec<&serde_json::Value> = hits["items"]
            .as_array()
            .expect("items 必须是数组")
            .iter()
            .filter(|i| i["file"] == FACT_FILE)
            .collect();
        assert_eq!(ours.len(), 1, "同 key 只能回一条：{hits}");
        assert_eq!(
            ours[0]["line"].as_u64(),
            Some(records[1].id),
            "回的必须是新记录"
        );
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    /// `verdict` 与实际落盘必须一致：drop 无记录、keep 落项目局部层、promote 落跨项目层。
    #[test]
    fn the_reported_verdict_matches_what_is_actually_on_the_disk() {
        let dir = isolated_dir("verdict");
        let dropped = remember_fact("proj", "好的", "tiered", 1_000);
        let kept = remember_fact(
            "proj",
            "这个项目里长期维护着一份不便公开的口径说明文档，写在内部 wiki 上",
            "tiered",
            2_000,
        );
        let promoted = remember_fact(
            "proj",
            "记住：打包脚本必须先跑 check-version-sync.py 再出包（/scripts/package.sh）",
            "tiered",
            3_000,
        );
        assert_eq!(
            (dropped.verdict, kept.verdict, promoted.verdict),
            ("drop", "keep", "promote"),
            "{dropped:?} / {kept:?} / {promoted:?}"
        );
        assert!(!dropped.written, "{dropped:?}");

        let records: Vec<FactRecord> = raw_lines(&store_path())
            .iter()
            .map(|l| serde_json::from_str(l).expect("每行都是一条记录"))
            .collect();
        assert_eq!(records.len(), 2, "只有 keep + promote 真的落盘");
        assert!(
            !records.iter().any(|r| r.key == dropped.key),
            "被判 Drop 的条目不得出现在库里"
        );
        let kept_rec = records
            .iter()
            .find(|r| r.key == kept.key)
            .expect("keep 必须落盘");
        assert_eq!(kept_rec.tier, "project_local", "Keep 落项目局部层");
        let promoted_rec = records
            .iter()
            .find(|r| r.key == promoted.key)
            .expect("promote 必须落盘");
        assert_eq!(promoted_rec.tier, "cross_project", "Promote 落跨项目层");
        assert!(
            promoted_rec.importance >= promote::PROMOTE_THRESHOLD,
            "落盘的重要度必须与结论自洽：{}",
            promoted_rec.importance
        );
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    /// 归一化只做「空白 + 大小写」：两条内容不同的事实绝不能撞成一个 key。
    #[test]
    fn normalization_collapses_whitespace_without_merging_distinct_facts() {
        assert_eq!(normalize_text("  Foo   Bar \n"), "foo bar");
        assert_ne!(
            build_key("tiered", "部署用 canary"),
            build_key("tiered", "部署用 canary2"),
            "不同事实不得共用一个 key（否则会互相取代，静默丢数据）"
        );
        assert_ne!(
            build_key("tiered", "同一句话"),
            build_key("other", "同一句话"),
            "不同来源不得共用一个 key"
        );
    }

    /// R-02：用户删除写回事实 —— 真删正文（磁盘上不得残留），且幂等。
    #[test]
    fn forget_fact_removes_text_from_disk_and_is_idempotent() {
        let dir = isolated_dir("forget");
        let text = "记住：所有回答先给结论";
        let outcome = remember_fact("app", text, "test", 1_700_000_000_000);
        assert!(outcome.written, "前置：事实必须写进去：{outcome:?}");
        let id = active_records()
            .into_iter()
            .find(|r| r.text == text)
            .expect("刚写入的事实应可读回")
            .id;

        assert_eq!(forget_fact(id), Ok(true));
        assert!(
            active_records().iter().all(|r| r.id != id),
            "删除后不得再出现在有效事实里"
        );
        let raw = std::fs::read_to_string(store_path()).unwrap_or_default();
        assert!(
            !raw.contains(text),
            "硬删除必须把正文从磁盘上抹掉（打墓碑会留下正文）：{raw}"
        );

        // 幂等：再删一次返回 false，不报错、不破坏文件。
        assert_eq!(forget_fact(id), Ok(false));
        // 不存在的 id 同理。
        assert_eq!(forget_fact(9_999_999), Ok(false));
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }

    /// 删除一条不得连带删掉别的（事实库是全量重写，最容易在这里写错）。
    #[test]
    fn forget_fact_keeps_other_records_intact() {
        let dir = isolated_dir("forget_keep");
        remember_fact("app", "记住第一条：用 Rust 写核心", "test", 1_700_000_000_000);
        remember_fact("app", "记住第二条：前端只用 Vue3 控制台", "test", 1_700_000_001_000);
        let all = active_records();
        assert!(all.len() >= 2, "前置：两条都应落盘，实际 {all:?}");
        let victim = all[0].id;
        let survivor = all[1].clone();

        assert_eq!(forget_fact(victim), Ok(true));
        let after = active_records();
        assert!(
            after.iter().any(|r| r.id == survivor.id && r.text == survivor.text),
            "删除一条不得连带删掉其它记录"
        );
        assert!(after.iter().all(|r| r.id != victim));
        std::fs::remove_dir_all(&dir).unwrap_or(());
    }
}
