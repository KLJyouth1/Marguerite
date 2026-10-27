//! 技能加载器
//!
//! 从本地文件系统或远程 URL 加载技能定义。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use super::registry::{Skill, SkillSource};
use anyhow::{bail, Context, Result};
use tracing::info;

use crate::security::skill_scan::SkillInstallScanner;

/// 目录型技能扫描的最大深度（root = 0，子目录 1..=6）
const MAX_SCAN_DEPTH: usize = 6;
/// 单次扫描最多处理的候选文件数（防止 438+ 目录把启动拖垮）；超出记 `scan-cap`
const MAX_SCAN_FILES: usize = 1200;
/// 单个技能文件大小上限（512KB）
const MAX_SKILL_FILE_BYTES: u64 = 512 * 1024;
/// `SKILL.md` 正文注入 prompt 的字符上限（超出截断并留标记）
const MAX_PROMPT_CHARS: usize = 4000;
/// 索引扫描（`skills.list` / `list_skills` / `search_skills`）保留的正文长度。
///
/// 三级加载里的第 1 级是「元数据常驻」：列表类调用只需要名称、描述和开头几句
/// 判断「这是不是我要的技能」。正文完整版由 `use_skill` 按名字重新加载
/// （`SkillLoader::load` → 全量 4000 字），因此这里可以安全地压到几百字，
/// 让一次列表扫描的字符串占用从「数千字 × N」降到「数百字 × N」。
const MAX_INDEX_PROMPT_CHARS: usize = 320;

/// 被跳过的技能文件（`reason` 是稳定错误码，供前端聚合）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    /// 相对技能根目录的路径（统一用 `/` 分隔）
    pub path: String,
    /// 跳过原因：missing-frontmatter | invalid-frontmatter | empty-body |
    /// too-large | read-error | parse-error | duplicate-name | scan-cap
    pub reason: String,
}

/// 递归扫描报告（确定性：同一目录树 → 同一报告）
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    /// 技能根目录
    pub root: PathBuf,
    /// 访问到的目录数（不含 root 本身）
    pub directories: usize,
    /// 实际处理的候选文件数（yaml + SKILL.md + 被跳过者）
    pub files_scanned: usize,
    /// 成功加载的顶层 YAML 数
    pub yaml: usize,
    /// 成功加载的 SKILL.md 数
    pub skill_md: usize,
    /// 跳过明细（内部保留全量；`skills.coverage` 只展示前 50 条）
    pub skipped: Vec<SkippedFile>,
    /// 跳过总数（真实计数，不受展示上限影响）
    pub skipped_total: usize,
}

/// 一次扫描的结果
#[derive(Debug, Clone, Default)]
pub struct SkillScan {
    /// 去重后（按名字先到先得）的技能列表
    pub skills: Vec<Skill>,
    /// 扫描报告
    pub report: ScanReport,
}

impl ScanReport {
    fn skip(&mut self, path: &Path, root: &Path, reason: &str) {
        self.skipped_total += 1;
        self.skipped.push(SkippedFile {
            path: relative_display(path, root),
            reason: reason.to_string(),
        });
    }
}

/// 相对 root 的展示路径：统一 `/`（Windows 反斜杠会让前端/日志难读）
fn relative_display(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

/// 从 YAML frontmatter 映射里取字段（键名精确匹配）
fn fm_lookup<'a>(map: &'a serde_yaml::Mapping, key: &str) -> Option<&'a serde_yaml::Value> {
    map.iter()
        .find(|(k, _)| k.as_str() == Some(key))
        .map(|(_, v)| v)
}

fn fm_string(map: &serde_yaml::Mapping, key: &str) -> Option<String> {
    fm_lookup(map, key)
        .and_then(serde_yaml::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `allowed-tools`: 逗号分隔字符串 → tools；也接受 YAML 数组
fn fm_tools(map: &serde_yaml::Mapping) -> Vec<String> {
    match fm_lookup(map, "allowed-tools") {
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// `intent` 可为数组或单个字符串
fn fm_intent(map: &serde_yaml::Mapping) -> Vec<String> {
    match fm_lookup(map, "intent") {
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// 解析 `SKILL.md`：`---` frontmatter + 正文（正文截断到 [`MAX_PROMPT_CHARS`]）
///
/// `pub(crate)`：插件技能桥接（`skills/plugin_bridge.rs`）复用同一解析器，
/// 避免出现「插件里的 SKILL.md 与目录里的 SKILL.md 两套解析规则」。
pub(crate) fn parse_skill_md(path: &Path, content: &str, root: &Path) -> Result<Skill> {
    parse_skill_md_capped(path, content, root, MAX_PROMPT_CHARS)
}

/// 带正文上限的解析：`cap` 决定保留多少正文字符（索引扫描用小值）。
pub(crate) fn parse_skill_md_capped(
    path: &Path,
    content: &str,
    root: &Path,
    cap: usize,
) -> Result<Skill> {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);

    let mut offset = 0usize;
    let mut front = String::new();
    let mut closed = false;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if start == 0 {
            if trimmed != "---" {
                bail!("缺少 frontmatter（首行必须是 ---）");
            }
            continue;
        }
        if trimmed == "---" {
            closed = true;
            break;
        }
        front.push_str(line);
    }
    if !closed {
        bail!("frontmatter 未闭合");
    }

    let value: serde_yaml::Value =
        serde_yaml::from_str(&front).context("frontmatter 不是合法 YAML")?;
    let map = value
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("frontmatter 必须是映射"))?;
    let name = fm_string(map, "name").ok_or_else(|| anyhow::anyhow!("frontmatter 缺少 name"))?;

    let body = text.get(offset..).unwrap_or("");
    if body.trim().is_empty() {
        bail!("正文为空");
    }

    let total_chars = body.chars().count();
    let prompt_template = if total_chars > cap {
        let head: String = body.chars().take(cap).collect();
        format!(
            "{head}\n\n[...truncated: {} chars; source: {}]",
            total_chars - cap,
            relative_display(path, root)
        )
    } else {
        body.to_string()
    };

    Ok(Skill {
        name,
        description: fm_string(map, "description").unwrap_or_default(),
        version: fm_string(map, "version").unwrap_or_else(|| "0.0.0".to_string()),
        author: fm_string(map, "author"),
        intent: fm_intent(map),
        tools: fm_tools(map),
        prompt_template,
        dependencies: Vec::new(),
        source: SkillSource::Builtin,
        rating: 0.0,
        uses: 0,
        rank: super::registry::ranks::SYSTEM,
    })
}

/// 把 `SKILL.md` 解析错误归类为稳定的跳过原因码
fn md_skip_reason(err: &anyhow::Error) -> &'static str {
    let msg = err.to_string();
    if msg.contains("缺少 frontmatter") || msg.contains("frontmatter 未闭合") {
        "missing-frontmatter"
    } else if msg.contains("frontmatter") {
        // 非法 YAML / 非映射 / 缺 name
        "invalid-frontmatter"
    } else if msg.contains("正文为空") {
        "empty-body"
    } else {
        "parse-error"
    }
}

/// 递归收集候选文件（排序后返回，保证确定性）
fn collect_entries(
    dir: &Path,
    depth: usize,
    root: &Path,
    report: &mut ScanReport,
    dirs: &mut usize,
    out: &mut Vec<PathBuf>,
    is_root: bool,
) -> Result<()> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            if is_root {
                return Err(e).with_context(|| format!("读取技能目录失败: {}", dir.display()));
            }
            // 子目录不可读：如实记账，不中断整棵树的扫描
            report.skip(dir, root, "read-error");
            return Ok(());
        }
    };

    let mut entries: Vec<PathBuf> = Vec::with_capacity(16);
    for entry in read_dir {
        match entry {
            Ok(e) => entries.push(e.path()),
            Err(_) => report.skip(dir, root, "read-error"),
        }
    }
    // 稳定排序：同输入同输出（不依赖文件系统枚举顺序）
    entries.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

    for path in entries {
        if path.is_dir() {
            // 停用标记：目录整体改名为 `*.disabled`，扫描时直接跳过。
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".disabled"))
            {
                report.skip(&path, root, "disabled");
                continue;
            }
            if depth < MAX_SCAN_DEPTH {
                *dirs += 1;
                collect_entries(&path, depth + 1, root, report, dirs, out, false)?;
            }
            continue;
        }
        let is_yaml = depth == 0
            && path
                .extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml");
        let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
        if is_yaml || is_skill_md {
            out.push(path);
        }
    }
    Ok(())
}


/// 技能扫描器进程级单例：构造会编译 ~20 条正则，成本较高，故 `LazyLock` 复用，
/// 避免 `load_all` 逐个技能加载时重复编译（与 `preflight::Preflight` 的 `LazyLock` 同思路）。
static SKILL_SCANNER: LazyLock<SkillInstallScanner> = LazyLock::new(SkillInstallScanner::new);

/// Windows 保留设备名：`CON.yaml` 之类的文件在 Windows 上会打开设备而不是文件，
/// 写入/删除都会产生不可预期行为，必须在命名阶段直接拒绝。
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// 严格校验技能名（用于**定位已有文件**的路径拼装）。
///
/// 技能名同时充当文件名，因此只允许 `[A-Za-z0-9_-]`。任何分隔符、盘符、`..`、
/// 控制字符都会被拒绝 —— 这是「技能名 → 文件路径」这一步的唯一防线：
/// 远程 YAML 的 `name` 字段与 `skills.uninstall` 的入参都来自不可信输入，
/// 一旦直接 `join`，`../../.ssh/authorized_keys` 之类就能越出技能目录。
pub fn sanitize_skill_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("技能名不可为空");
    }
    if name.len() > 100 {
        bail!("技能名过长（最多 100 字节）");
    }
    if name == "." || name == ".." {
        bail!("非法技能名: {name}");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("技能名只允许字母、数字、'-' 与 '_': {name}");
    }
    if WINDOWS_RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        bail!("技能名与系统保留设备名冲突: {name}");
    }
    Ok(name.to_string())
}

/// 把展示名映射为合法文件 stem（用于**创建**新技能时的宽松处理）。
///
/// 与 [`sanitize_skill_name`] 的区别：这里允许用户给中文/带空格的展示名，
/// 非法字符统一折叠为单个 `-`；折叠后若没有任何有效字符则报错，绝不产生空名字。
pub fn slugify_skill_name(name: &str) -> Result<String> {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        bail!("技能名需包含至少一个英文字母、数字或下划线");
    }
    sanitize_skill_name(&out)
}

/// 把任意文本渲染成 YAML 双引号标量。
///
/// 直接拼接 `description: {用户输入}` 时，输入里的换行 / `:` / `#` / 引号
/// 会改变 YAML 结构（甚至注入额外字段），所以统一走转义。
pub fn yaml_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 技能加载器
///
/// 负责从 YAML 文件加载技能定义，支持本地文件和远程 URL。
pub struct SkillLoader {
    /// 技能文件目录
    skills_dir: PathBuf,
}

/// 一次技能启停的结果（`changed=false` 表示幂等，没有真的改文件）。
#[derive(Debug, Clone)]
pub struct SkillToggleOutcome {
    pub name: String,
    pub enabled: bool,
    pub changed: bool,
    pub path: Option<PathBuf>,
}

/// 去掉停用后缀（`x.yaml.disabled` → `x.yaml`，`dir.disabled` → `dir`）。
fn strip_disabled_suffix(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    text.strip_suffix(".disabled").map(PathBuf::from)
}

/// 用于反序列化 YAML 的中间结构（不含 source 字段）
#[derive(serde::Deserialize)]
struct SkillData {
    name: String,
    description: String,
    version: String,
    author: Option<String>,
    #[serde(default)]
    intent: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
    prompt_template: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    rating: f64,
    #[serde(default)]
    uses: u32,
    #[serde(default)]
    rank: Option<u32>,
}

impl SkillLoader {
    /// 创建新的技能加载器
    pub fn new(skills_dir: &Path) -> Self {
        Self {
            skills_dir: skills_dir.to_path_buf(),
        }
    }

    /// 加载所有技能（保持既有签名）
    ///
    /// 委托 [`SkillLoader::scan_all`]：顶层 `*.yaml|*.yml` 行为不变，
    /// 同时把任意深度（≤6）的目录型 `SKILL.md` 一并接入产品。
    pub fn load_all(&self) -> Result<Vec<Skill>> {
        Ok(self.scan_all()?.skills)
    }

    /// 递归扫描：顶层 YAML + 目录型 `SKILL.md`，返回技能与**确定性**报告。
    ///
    /// 规则（顺序敏感，保证同输入同输出）：
    /// 1. 收集候选 → 按路径字符串排序；
    /// 2. 逐文件处理（上限 [`MAX_SCAN_FILES`]，超出记 `scan-cap`）；
    /// 3. 按名字去重：先到先得，重复者记 `duplicate-name`。
    pub fn scan_all(&self) -> Result<SkillScan> {
        self.scan_with(MAX_PROMPT_CHARS, true)
    }

    /// 索引扫描（三级加载的第 1 级）：正文只保留 [`MAX_INDEX_PROMPT_CHARS`]。
    ///
    /// 供 `skills.list` / `list_skills` / `search_skills` 使用：这些路径只需要
    /// 「名字 + 描述 + 开头几句」来匹配意图，正文由 `use_skill` 触发时再全量加载。
    pub fn scan_index(&self) -> Result<SkillScan> {
        self.scan_with(MAX_INDEX_PROMPT_CHARS, false)
    }

    fn scan_with(&self, cap: usize, verbose: bool) -> Result<SkillScan> {
        let root = self.skills_dir.clone();
        let mut report = ScanReport {
            root: root.clone(),
            ..Default::default()
        };

        if !root.exists() {
            info!("技能目录不存在: {}", root.display());
            return Ok(SkillScan::default());
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut directories = 0usize;
        collect_entries(
            &root,
            0,
            &root,
            &mut report,
            &mut directories,
            &mut candidates,
            true,
        )?;
        report.directories = directories;
        candidates.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));

        let mut skills: Vec<Skill> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        for (idx, path) in candidates.iter().enumerate() {
            if idx >= MAX_SCAN_FILES {
                report.skip(path, &root, "scan-cap");
                continue;
            }
            report.files_scanned += 1;

            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => {
                    report.skip(path, &root, "read-error");
                    continue;
                }
            };
            if meta.len() > MAX_SKILL_FILE_BYTES {
                report.skip(path, &root, "too-large");
                continue;
            }

            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            let skill = if is_skill_md {
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => {
                        report.skip(path, &root, "read-error");
                        continue;
                    }
                };
                match parse_skill_md_capped(path, &content, &root, cap) {
                    Ok(s) => s,
                    Err(e) => {
                        info!("跳过目录型技能 {}: {}", path.display(), e);
                        report.skip(path, &root, md_skip_reason(&e));
                        continue;
                    }
                }
            } else {
                match self.load_from_path(path) {
                    Ok(s) => s,
                    Err(e) => {
                        info!("跳过技能文件 {}: {}", path.display(), e);
                        report.skip(path, &root, "parse-error");
                        continue;
                    }
                }
            };

            if !seen.insert(skill.name.clone()) {
                report.skip(path, &root, "duplicate-name");
                continue;
            }
            if is_skill_md {
                report.skill_md += 1;
            } else {
                report.yaml += 1;
            }
            if verbose {
                info!("加载技能: {} ({})", skill.name, path.display());
            }
            skills.push(skill);
        }

        Ok(SkillScan {
            skills,
            report,
        })
    }

    /// 停用索引文件路径（隐藏文件，不被技能扫描识别）。
    fn disabled_index_path(&self) -> PathBuf {
        self.skills_dir.join(".disabled-index.json")
    }

    /// 读取停用索引：`技能名 → 相对 skills_dir 的停用路径`。
    fn disabled_index(&self) -> HashMap<String, String> {
        let Ok(text) = std::fs::read_to_string(self.disabled_index_path()) else {
            return HashMap::new();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return HashMap::new();
        };
        value
            .get("entries")
            .and_then(serde_json::Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 写停用索引（原子写；失败返回错误，由调用方回滚文件改名）。
    fn save_disabled_index(&self, entries: &HashMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.skills_dir)?;
        let value = serde_json::json!({
            "version": 1,
            "entries": entries,
            "updatedAt": chrono::Utc::now().to_rfc3339(),
        });
        let path = self.disabled_index_path();
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&value)?)
            .with_context(|| format!("写入停用索引失败: {}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .with_context(|| format!("提交停用索引失败: {}", path.display()))?;
        Ok(())
    }

    /// 该技能是否已被停用（只读索引，不扫描文件）。
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled_index().contains_key(name)
    }

    /// 当前停用的技能（名字 + 落盘路径），供 UI 反向启用/删除。
    pub fn disabled_skills(&self) -> Vec<(String, String)> {
        let mut items: Vec<(String, String)> = self
            .disabled_index()
            .into_iter()
            .map(|(name, rel)| {
                let path = self
                    .resolve_indexed_path(&rel)
                    .map(|p| p.display().to_string())
                    .unwrap_or(rel);
                (name, path)
            })
            .collect();
        items.sort();
        items
    }

    /// 启停一个技能（真实改名 + 索引持久化，失败回滚）。
    ///
    /// - YAML 技能：`<name>.yaml` ↔ `<name>.yaml.disabled`
    /// - 目录技能（SKILL.md）：技能目录 ↔ `<dir>.disabled`
    ///
    /// 停用后扫描器不会再看到它，因此 `skills.list` / 模型侧技能目录都会同步消失；
    /// 重新启用后下一次扫描恢复。
    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<SkillToggleOutcome> {
        let name = sanitize_skill_name(name)?;
        let mut index = self.disabled_index();
        if enabled {
            let Some(rel) = index.get(&name).cloned() else {
                // 没有停用记录：已启用是幂等成功，但不能把「不存在」当成功。
                if self.try_skill_path(&name).map(|p| p.exists()).unwrap_or(false) {
                    let path = self.try_skill_path(&name).ok();
                    return Ok(SkillToggleOutcome {
                        name,
                        enabled: true,
                        changed: false,
                        path,
                    });
                }
                bail!("技能不存在或未处于停用状态: {name}");
            };
            let current = self.resolve_indexed_path(&rel)?;
            let restored = strip_disabled_suffix(&current)
                .ok_or_else(|| anyhow::anyhow!("停用索引路径非法: {rel}"))?;
            if !current.exists() {
                index.remove(&name);
                let _ = self.save_disabled_index(&index);
                bail!("停用文件不存在（已清理索引）: {}", current.display());
            }
            std::fs::rename(&current, &restored)
                .with_context(|| format!("恢复技能失败: {}", current.display()))?;
            index.remove(&name);
            if let Err(e) = self.save_disabled_index(&index) {
                let _ = std::fs::rename(&restored, &current);
                return Err(e);
            }
            return Ok(SkillToggleOutcome {
                name,
                enabled: true,
                changed: true,
                path: Some(restored),
            });
        }

        if index.contains_key(&name) {
            let path = index
                .get(&name)
                .and_then(|rel| self.resolve_indexed_path(rel).ok());
            return Ok(SkillToggleOutcome {
                name,
                enabled: false,
                changed: false,
                path,
            });
        }
        let Some(target) = self.find_skill_artifact(&name) else {
            bail!("技能不存在: {name}");
        };
        let disabled = PathBuf::from(format!("{}.disabled", target.display()));
        std::fs::rename(&target, &disabled)
            .with_context(|| format!("停用技能失败: {}", target.display()))?;
        let rel = disabled
            .strip_prefix(&self.skills_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| disabled.to_string_lossy().to_string());
        index.insert(name.clone(), rel);
        if let Err(e) = self.save_disabled_index(&index) {
            let _ = std::fs::rename(&disabled, &target);
            return Err(e);
        }
        Ok(SkillToggleOutcome {
            name,
            enabled: false,
            changed: true,
            path: Some(disabled),
        })
    }

    /// 索引里的相对路径 → 绝对路径（拒绝 `..` 越界）。
    fn resolve_indexed_path(&self, rel: &str) -> Result<PathBuf> {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute()
            || rel_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            bail!("停用索引路径越界: {rel}");
        }
        Ok(self.skills_dir.join(rel_path))
    }

    /// 找到技能的真实落盘产物：顶层 YAML 或 SKILL.md 所在目录。
    fn find_skill_artifact(&self, name: &str) -> Option<PathBuf> {
        if let Ok(path) = self.try_skill_path(name) {
            if path.exists() {
                return Some(path);
            }
        }
        // 目录型技能：`Skill.source` 是 `Builtin`（不携带路径），只能递归扫描
        // SKILL.md 并逐个解析 frontmatter 里的 name 来定位真实目录。
        let mut report = ScanReport::default();
        let mut dirs = 0usize;
        let mut candidates: Vec<PathBuf> = Vec::new();
        collect_entries(
            &self.skills_dir,
            0,
            &self.skills_dir,
            &mut report,
            &mut dirs,
            &mut candidates,
            true,
        )
        .ok()?;
        for path in candidates {
            if path.file_name().is_none_or(|n| n != "SKILL.md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(skill) = parse_skill_md(&path, &content, &self.skills_dir) else {
                continue;
            };
            if skill.name == name || skill.name.eq_ignore_ascii_case(name) {
                return path.parent().map(Path::to_path_buf);
            }
        }
        None
    }

    /// 删除技能的真实产物（YAML 文件或 SKILL.md 目录），拒绝目录越界。
    pub fn remove_skill(&self, name: &str) -> Result<PathBuf> {
        let name = sanitize_skill_name(name)?;
        let mut index = self.disabled_index();
        let target = if let Some(rel) = index.get(&name) {
            let path = self.resolve_indexed_path(rel)?;
            if !path.exists() {
                index.remove(&name);
                let _ = self.save_disabled_index(&index);
                bail!("技能停用文件不存在: {}", path.display());
            }
            strip_disabled_suffix(&path).ok_or_else(|| anyhow::anyhow!("停用索引路径非法"))?
        } else {
            self.find_skill_artifact(&name)
                .ok_or_else(|| anyhow::anyhow!("技能不存在: {name}"))?
        };
        // 路径必须仍在技能目录内（canonicalize 后比对），否则拒绝删除。
        let root = self
            .skills_dir
            .canonicalize()
            .with_context(|| format!("技能目录不可解析: {}", self.skills_dir.display()))?;
        let canonical = target
            .canonicalize()
            .with_context(|| format!("技能路径不可解析: {}", target.display()))?;
        if !canonical.starts_with(&root) {
            bail!("拒绝删除技能目录之外的路径: {}", target.display());
        }
        if canonical.is_dir() {
            std::fs::remove_dir_all(&canonical)
                .with_context(|| format!("删除技能目录失败: {}", canonical.display()))?;
        } else {
            std::fs::remove_file(&canonical)
                .with_context(|| format!("删除技能文件失败: {}", canonical.display()))?;
        }
        if index.remove(&name).is_some() {
            let _ = self.save_disabled_index(&index);
        }
        Ok(canonical)
    }

    /// 加载单个技能
    ///
    /// 先走既有的「顶层 `<name>.yaml`」路径；失败（文件名不存在 / 名字含
    /// `:` 等命名空间分隔符）时回退到递归扫描，按名字匹配（先精确、再不区分大小写）。
    pub fn load(&self, name: &str) -> Result<Skill> {
        if self.is_disabled(name) {
            bail!("技能已停用: {name}");
        }
        if let Ok(path) = self.try_skill_path(name) {
            if path.exists() {
                // 真实解析错误原样上抛，不伪装成「不存在」
                return self.load_from_path(&path);
            }
        }

        if let Ok(scan) = self.scan_all() {
            if let Some(s) = scan.skills.iter().find(|s| s.name == name) {
                return Ok(s.clone());
            }
            if let Some(s) = scan
                .skills
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
            {
                return Ok(s.clone());
            }
        }

        match self.try_skill_path(name) {
            Ok(path) => bail!("技能文件不存在: {}", path.display()),
            Err(e) => Err(e),
        }
    }

    /// 从绝对/相对路径加载单个技能文件（供 register-batch 等 CLI 场景做单文件解析）。
    ///
    /// 与 `load(name)` 等价：会基于文件位置打上 Builtin / Local 标记，并自动推断 rank
    /// （未显式指定 rank 时按 source 落入 SYSTEM/USER/COMMUNITY 等级）。
    pub fn load_single(&self, path: &Path) -> Result<Skill> {
        if !path.exists() {
            bail!("技能文件不存在: {}", path.display());
        }
        self.load_from_path(path)
    }

    /// 从 URL 加载技能（远程安装）
    ///
    /// 注意：当前实现仅设置 Remote 标记，不执行实际 HTTP 请求。
    pub fn load_from_url(&self, url: &str) -> Result<Skill> {
        // 从 URL 中推断技能名称
        let name = url
            .rsplit('/')
            .next()
            .and_then(|s| s.strip_suffix(".yaml").or_else(|| s.strip_suffix(".yml")))
            .unwrap_or("unknown");

        let skill = Skill {
            name: name.to_string(),
            description: format!("远程技能: {}", url),
            version: "0.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: String::new(),
            dependencies: vec![],
            source: SkillSource::Remote {
                url: url.to_string(),
            },
            rating: 0.0,
            uses: 0,
            rank: super::registry::ranks::COMMUNITY,
        };

        Ok(skill)
    }

    /// 从远程 URL 下载技能 YAML 并注册到本地技能目录
    ///
    /// 执行真实 HTTP 请求获取 YAML 内容，先经 L2 恶意扫描（见 `SkillInstallScanner`），
    /// 命中即拒绝落盘，否则解析并保存到技能目录。
    pub async fn download_and_register(&self, url: &str) -> Result<Skill> {
        let client = reqwest::Client::new();
        let resp = client
            .get(url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .with_context(|| format!("下载技能失败: {url}"))?;
        if !resp.status().is_success() {
            bail!("下载技能失败: HTTP {}", resp.status());
        }
        let content = resp.text().await?;

        // 解析一次以取技能名称（URL 无可靠 name 字段时兜底）
        let data: SkillData =
            serde_yaml::from_str(&content).with_context(|| format!("解析技能 YAML 失败: {url}"))?;
        // 远端 `name` 是**不可信输入**，且会被拼成文件名：这里先严格校验，
        // 非法名字直接拒绝（不做静默改名，避免覆盖同名技能）。
        let name = if data.name.trim().is_empty() {
            let stem = url
                .rsplit('/')
                .next()
                .and_then(|s| s.strip_suffix(".yaml").or_else(|| s.strip_suffix(".yml")))
                .unwrap_or("skill");
            slugify_skill_name(stem)?
        } else {
            sanitize_skill_name(&data.name)
                .with_context(|| format!("远端技能名非法（可能包含路径分隔符或 `..`）: {url}"))?
        };

        self.install_yaml(&name, &content)
            .with_context(|| format!("技能安装被安全层拦截: {url}"))
    }

    /// 安装本地 YAML 内容到技能目录：先经 L2 恶意扫描，命中即拒绝，否则落盘。
    ///
    /// 这是"技能安装前恶意扫描"的统一入口——所有进入工作区的技能文件（无论来自
    /// 远程下载还是本地导入）在调用 `std::fs::write` 落盘前都必须经过此函数。命中
    /// 危险模式（任意 `Critical` 或累计 risk_score ≥ 120）时拒绝安装，绝不写盘。
    pub fn install_yaml(&self, name: &str, content: &str) -> Result<Skill> {
        // —— 名称净化：`name` 来自远端 YAML 的 `name` 字段或用户输入，必须落在技能目录内 ——
        // 先于扫描执行：即使内容无害，`../../x` 这样的名字也不允许落盘。
        let name = sanitize_skill_name(name)?;

        // —— L2：落盘前扫描技能内容（含 manifest / prompt_template / dependencies）——
        let verdict = SKILL_SCANNER.scan(content);
        if verdict.blocked {
            crate::security::skill_scan::log_skill_block("install", &name, None, &verdict);
            bail!(
                "技能安装被安全层拒绝（{}）：{}",
                verdict.code(),
                verdict.reason
            );
        }

        let data: SkillData =
            serde_yaml::from_str(content).with_context(|| "解析技能 YAML 失败".to_string())?;

        // 扫描通过后才落盘
        std::fs::create_dir_all(&self.skills_dir)?;
        let dest = self.skills_dir.join(format!("{name}.yaml"));
        std::fs::write(&dest, content)?;

        // 日志必须在 `name` 被 move 进 `Skill` **之前**打：写在构造之后就是
        // 「借用已移走的值」（E0382），整套 `--lib` 直接编译不过。
        info!("技能已安装并通过安全扫描: {name}");
        let skill = Skill {
            name,
            description: data.description,
            version: data.version,
            author: data.author,
            intent: data.intent,
            tools: data.tools,
            prompt_template: data.prompt_template,
            dependencies: data.dependencies,
            source: SkillSource::Local { path: dest },
            rating: data.rating,
            uses: data.uses,
            rank: data.rank.unwrap_or(super::registry::ranks::USER),
        };
        Ok(skill)
    }

    /// 验证技能签名（基本版）
    ///
    /// 检查 source 字段的合法性。
    pub fn verify_signature(&self, skill: &Skill) -> bool {
        match &skill.source {
            SkillSource::Builtin => true,
            SkillSource::Local { path } => path.exists(),
            SkillSource::Remote { url } => {
                url.starts_with("http://") || url.starts_with("https://")
            }
        }
    }

    /// 获取技能文件路径（名称非法时返回错误，绝不拼出技能目录外的路径）
    ///
    /// 根据技能名称拼接对应的 YAML 文件路径。
    pub fn try_skill_path(&self, name: &str) -> Result<PathBuf> {
        let name = sanitize_skill_name(name)?;
        Ok(self.skills_dir.join(format!("{name}.yaml")))
    }

    /// 从指定路径加载技能
    fn load_from_path(&self, path: &Path) -> Result<Skill> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("读取技能文件失败: {}", path.display()))?;

        // 运行时防篡改：已安装 / 本地导入的技能在加载执行前再扫一遍 L2。
        // 覆盖两类场景：① 绕过 `install_yaml` 直接丢入 skills_dir 的未审查 YAML；
        // ② 安装后被篡改（落盘内容被改写）的技能文件。命中即拒绝加载，绝不执行。
        let verdict = SKILL_SCANNER.scan(&content);
        if verdict.blocked {
            let audit_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown");
            crate::security::skill_scan::log_skill_block("load", audit_name, Some(path), &verdict);
            bail!(
                "技能加载被安全层拒绝（{}）：{}",
                verdict.code(),
                verdict.reason
            );
        }

        let data: SkillData = serde_yaml::from_str(&content)
            .with_context(|| format!("解析技能文件失败: {}", path.display()))?;

        let source = if path.starts_with(&self.skills_dir) {
            // 相对于技能目录的文件标记为 Builtin（如果来自内置目录）
            SkillSource::Builtin
        } else {
            SkillSource::Local {
                path: path.to_path_buf(),
            }
        };

        let rank = data.rank.unwrap_or(match &source {
            SkillSource::Builtin => super::registry::ranks::SYSTEM,
            SkillSource::Local { .. } => super::registry::ranks::USER,
            SkillSource::Remote { .. } => super::registry::ranks::COMMUNITY,
        });

        let skill = Skill {
            name: data.name,
            description: data.description,
            version: data.version,
            author: data.author,
            intent: data.intent,
            tools: data.tools,
            prompt_template: data.prompt_template,
            dependencies: data.dependencies,
            source,
            rating: data.rating,
            uses: data.uses,
            rank,
        };

        Ok(skill)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    fn create_test_skill_yaml(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{}.yaml", name));
        let content = format!(
            r#"name: {}
description: "测试技能: {}"
version: "1.0.0"
intent:
  - test
  - 测试
tools: []
prompt_template: "模板: {{text}}"
"#,
            name, name
        );
        fs::write(&path, content).unwrap();
        path
    }

    // ── 目录型技能（SKILL.md）扫描 ─────────────────────────────────────────

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marguerite_loader_{tag}_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_skill_md(root: &Path, rel_dir: &str, name: &str, body: &str) -> PathBuf {
        let dir = root.join(rel_dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("SKILL.md");
        let content = format!(
            "---\nname: {name}\ndescription: 测试技能\nauthor: tester\nversion: \"1.2.3\"\nallowed-tools: file_read, terminal_exec\nintent:\n  - code\n  - 测试\n---\n\n{body}\n"
        );
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn nested_skill_md_discovered_up_to_depth_six() {
        let root = temp_root("nested");
        write_skill_md(&root, "a/b/c", "deep-skill", "正文");
        // 深度 7：超出上限，必须不可见
        write_skill_md(&root, "d1/d2/d3/d4/d5/d6/d7", "too-deep", "正文");
        // 大小写不匹配：SKILL.md 是大小写敏感的
        let dir = root.join("lower");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("skill.md"), "---\nname: lower\n---\n正文").unwrap();

        let loader = SkillLoader::new(&root);
        let scan = loader.scan_all().unwrap();
        let names: Vec<&str> = scan.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["deep-skill"], "只应加载深度 ≤6 的 SKILL.md");
        assert_eq!(scan.report.skill_md, 1);
        assert!(scan.report.directories >= 3);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skill_md_frontmatter_fields_are_parsed() {
        let root = temp_root("fm");
        write_skill_md(&root, "pkg", "fm-skill", "请执行任务");
        let loader = SkillLoader::new(&root);
        let scan = loader.scan_all().unwrap();
        assert_eq!(scan.skills.len(), 1);
        let s = &scan.skills[0];
        assert_eq!(s.name, "fm-skill");
        assert_eq!(s.description, "测试技能");
        assert_eq!(s.version, "1.2.3");
        assert_eq!(s.author.as_deref(), Some("tester"));
        assert_eq!(s.tools, vec!["file_read", "terminal_exec"]);
        assert_eq!(s.intent, vec!["code", "测试"]);
        assert_eq!(s.prompt_template.trim(), "请执行任务");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn skill_md_body_is_truncated_with_source_marker() {
        let root = temp_root("trunc");
        let body = "x".repeat(5000);
        write_skill_md(&root, "big", "big-skill", &body);
        let loader = SkillLoader::new(&root);
        let scan = loader.scan_all().unwrap();
        let p = &scan.skills[0].prompt_template;
        assert!(
            p.contains("[...truncated: ")
                && p.contains(" chars; source: big/SKILL.md]"),
            "截断必须留正文字符数 + 相对路径标记: 尾部 = {}",
            &p[p.len().saturating_sub(80)..]
        );
        assert!(p.chars().count() < 4200, "注入 prompt 的正文必须被压到 4k 级");
        fs::remove_dir_all(&root).unwrap();
    }

    /// 索引扫描（列表/搜索路径）必须只保留正文摘要，而全量扫描仍给完整正文。
    ///
    /// 这条是三级加载的回归护栏：如果哪天有人把 `scan_index` 改回全量，
    /// 359 个技能的列表调用会重新把数 MB 正文读进内存。
    #[test]
    fn index_scan_caps_body_while_full_scan_keeps_it() {
        let root = temp_root("index-cap");
        let body = "y".repeat(3000);
        write_skill_md(&root, "pkg", "cap-skill", &body);
        let loader = SkillLoader::new(&root);

        let full = loader.scan_all().unwrap();
        let index = loader.scan_index().unwrap();
        assert_eq!(full.skills.len(), index.skills.len());
        assert_eq!(full.skills[0].name, index.skills[0].name);

        let full_len = full.skills[0].prompt_template.chars().count();
        let index_len = index.skills[0].prompt_template.chars().count();
        assert!(
            index_len < MAX_INDEX_PROMPT_CHARS + 120,
            "索引正文应压到 ~{MAX_INDEX_PROMPT_CHARS} 字，实际 {index_len}"
        );
        assert!(index_len < full_len, "索引正文必须短于全量正文");
        assert!(index.skills[0].prompt_template.contains("[...truncated: "));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn duplicate_names_keep_path_first_and_are_reported() {
        let root = temp_root("dup");
        write_skill_md(&root, "a", "dup", "A");
        write_skill_md(&root, "b", "dup", "B");
        let loader = SkillLoader::new(&root);
        let scan = loader.scan_all().unwrap();
        assert_eq!(scan.skills.len(), 1);
        assert_eq!(scan.skills[0].prompt_template.trim(), "A", "按路径排序先到先得");
        assert_eq!(scan.report.skipped_total, 1);
        assert_eq!(scan.report.skipped[0].reason, "duplicate-name");
        assert_eq!(scan.report.skipped[0].path, "b/SKILL.md");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn broken_frontmatter_is_skipped_with_reason() {
        let root = temp_root("badfm");
        let cases = [
            ("nofm", "没有 frontmatter 的正文", "missing-frontmatter"),
            ("badyaml", "---\nname: [unclosed\n---\n正文", "invalid-frontmatter"),
            ("emptybody", "---\nname: empty\n---\n\n", "empty-body"),
        ];
        for (dir, content, _reason) in cases {
            let d = root.join(dir);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("SKILL.md"), content).unwrap();
        }
        let loader = SkillLoader::new(&root);
        let scan = loader.scan_all().unwrap();
        assert!(scan.skills.is_empty(), "坏 frontmatter 一律不得加载");
        assert_eq!(scan.report.skipped_total, 3);
        let reasons: Vec<&str> = scan
            .report
            .skipped
            .iter()
            .map(|s| s.reason.as_str())
            .collect();
        assert!(reasons.contains(&"missing-frontmatter"), "{reasons:?}");
        assert!(reasons.contains(&"invalid-frontmatter"), "{reasons:?}");
        assert!(reasons.contains(&"empty-body"), "{reasons:?}");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn oversized_skill_md_is_skipped() {
        let root = temp_root("large");
        write_skill_md(&root, "huge", "huge", &"x".repeat(600 * 1024));
        let scan = SkillLoader::new(&root).scan_all().unwrap();
        assert!(scan.skills.is_empty());
        assert_eq!(scan.report.skipped[0].reason, "too-large");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn top_level_yaml_still_loads_and_counts() {
        let root = temp_root("yaml");
        create_test_skill_yaml(&root, "legacy");
        write_skill_md(&root, "a", "md-skill", "正文");
        let scan = SkillLoader::new(&root).scan_all().unwrap();
        assert_eq!(scan.skills.len(), 2);
        assert_eq!(scan.report.yaml, 1);
        assert_eq!(scan.report.skill_md, 1);
        assert_eq!(scan.report.files_scanned, 2);
        assert_eq!(scan.report.directories, 1);
        assert_eq!(scan.report.skipped_total, 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_falls_back_to_skill_md_by_exact_then_ci_name() {
        let root = temp_root("loadfb");
        write_skill_md(&root, "pkg", "ns:skill", "正文");
        let loader = SkillLoader::new(&root);

        // 名字含 `:` → try_skill_path 拒绝 → 回退扫描精确命中
        let s = loader.load("ns:skill").expect("应回退到 SKILL.md");
        assert_eq!(s.name, "ns:skill");
        // 再不区分大小写命中
        let s2 = loader.load("NS:SKILL").expect("大小写不敏感回退");
        assert_eq!(s2.name, "ns:skill");
        // 仍无 → 原错误
        assert!(loader.load("nope").is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn scan_is_deterministic_across_runs() {
        let root = temp_root("determ");
        create_test_skill_yaml(&root, "z_yaml");
        write_skill_md(&root, "m/1", "m-one", "正文");
        write_skill_md(&root, "a/2", "a-two", "正文");
        write_skill_md(&root, "b/3", "a-two", "重复");
        let loader = SkillLoader::new(&root);
        let one = loader.scan_all().unwrap();
        let two = loader.scan_all().unwrap();
        let names = |s: &SkillScan| -> Vec<String> {
            s.skills.iter().map(|x| x.name.clone()).collect()
        };
        assert_eq!(names(&one), names(&two), "同输入必须同输出");
        assert_eq!(one.report.skipped, two.report.skipped);
        assert_eq!(one.report.files_scanned, two.report.files_scanned);
        assert_eq!(one.report.skipped_total, 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn load_all_keeps_signature_and_returns_same_as_scan() {
        let root = temp_root("loadall");
        create_test_skill_yaml(&root, "a_yaml");
        write_skill_md(&root, "x", "x-skill", "正文");
        let loader = SkillLoader::new(&root);
        let all = loader.load_all().unwrap();
        let scan = loader.scan_all().unwrap();
        assert_eq!(all.len(), scan.skills.len());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn test_load_single_skill() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_single");
        fs::create_dir_all(&dir).unwrap();
        create_test_skill_yaml(&dir, "test_skill");

        let loader = SkillLoader::new(&dir);
        let skill = loader.load("test_skill").unwrap();
        assert_eq!(skill.name, "test_skill");
        assert_eq!(skill.version, "1.0.0");
        assert!(skill.intent.contains(&"test".to_string()));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_nonexistent() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_nonexistent");
        let loader = SkillLoader::new(&dir);
        assert!(loader.load("nonexistent").is_err());
    }

    #[test]
    fn test_load_all() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_all");
        fs::create_dir_all(&dir).unwrap();
        create_test_skill_yaml(&dir, "skill_a");
        create_test_skill_yaml(&dir, "skill_b");

        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert_eq!(skills.len(), 2);

        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill_a"));
        assert!(names.contains(&"skill_b"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_all_empty_dir() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_empty");
        fs::create_dir_all(&dir).unwrap();

        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_all_nonexistent_dir() {
        let dir = std::env::temp_dir().join("marguerite_test_loader_nodir");
        let loader = SkillLoader::new(&dir);
        let skills = loader.load_all().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_get_skill_path() {
        let dir = Path::new("/tmp/skills");
        let loader = SkillLoader::new(dir);
        let path = loader.try_skill_path("my_skill").unwrap();
        assert_eq!(path, Path::new("/tmp/skills/my_skill.yaml"));
    }

    /// 技能名是文件名：任何能越出技能目录的名字都必须被拒绝。
    #[test]
    fn skill_name_rejects_traversal_and_separators() {
        for bad in [
            "",
            ".",
            "..",
            "../evil",
            "..\\evil",
            "../../.ssh/authorized_keys",
            "/etc/passwd",
            "C:\\Windows\\System32\\x",
            "a/b",
            "a\\b",
            "a b",
            "a\0b",
            "con",
            "NUL",
            "com1",
        ] {
            assert!(
                sanitize_skill_name(bad).is_err(),
                "非法技能名应被拒绝: {bad:?}"
            );
        }
        for good in ["my-skill", "my_skill", "Skill123", "a"] {
            assert_eq!(sanitize_skill_name(good).unwrap(), good);
        }
    }

    /// 远程安装时，远端 YAML 里的恶意 `name` 绝不能导致越界落盘。
    #[test]
    fn install_yaml_rejects_traversal_name_and_writes_nothing() {
        let base = std::env::temp_dir().join("marguerite_test_install_traversal");
        let dir = base.join("skills");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&dir).unwrap();
        let loader = SkillLoader::new(&dir);

        let yaml = "name: ../../pwned\ndescription: x\nversion: \"1.0.0\"\nprompt_template: |\n  hi\n";
        let res = loader.install_yaml("../../pwned", yaml);
        assert!(res.is_err(), "穿越型技能名必须被拒绝");
        assert!(
            !base.join("pwned.yaml").exists(),
            "绝不能在技能目录之外落盘"
        );
        assert!(!dir.join("..").join("pwned.yaml").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    /// `slugify`（创建技能用）对中文/空格做折叠，但不允许折叠出空名字。
    #[test]
    fn slugify_skill_name_folds_display_names() {
        assert_eq!(slugify_skill_name("My Skill").unwrap(), "My-Skill");
        assert!(slugify_skill_name("代码审查").is_err());
        assert!(slugify_skill_name("  ").is_err());
        assert_eq!(slugify_skill_name("../evil").unwrap(), "evil");
    }

    /// `yaml_quote` 必须让换行/引号不破坏 YAML 结构。
    #[test]
    fn yaml_quote_escapes_structural_chars() {
        assert_eq!(yaml_quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(yaml_quote("a\nb"), "\"a\\nb\"");
        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&format!("k: {}\n", yaml_quote("x: y # z"))).unwrap();
        assert_eq!(parsed["k"].as_str().unwrap(), "x: y # z");
    }

    #[test]
    fn test_verify_signature_builtin() {
        let dir = std::env::temp_dir().join("marguerite_test_verify");
        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "test".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Builtin,
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));
    }

    #[test]
    fn test_verify_signature_local() {
        let dir = std::env::temp_dir().join("marguerite_test_verify_local");
        fs::create_dir_all(&dir).unwrap();
        let yaml_path = create_test_skill_yaml(&dir, "local_skill");

        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "local_skill".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Local {
                path: yaml_path.clone(),
            },
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));

        // 不存在的路径应验证失败
        let skill_bad = Skill {
            source: SkillSource::Local {
                path: Path::new("/nonexistent/skill.yaml").to_path_buf(),
            },
            ..skill
        };
        assert!(!loader.verify_signature(&skill_bad));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_verify_signature_remote() {
        let dir = std::env::temp_dir().join("marguerite_test_verify_remote");
        let loader = SkillLoader::new(&dir);

        let skill = Skill {
            name: "test".to_string(),
            description: "test".to_string(),
            version: "1.0.0".to_string(),
            author: None,
            intent: vec![],
            tools: vec![],
            prompt_template: "".to_string(),
            dependencies: vec![],
            source: SkillSource::Remote {
                url: "https://example.com/skill.yaml".to_string(),
            },
            rating: 0.0,
            uses: 0,
            rank: 300,
        };
        assert!(loader.verify_signature(&skill));

        // 非 http(s) URL 应验证失败
        let skill_bad = Skill {
            source: SkillSource::Remote {
                url: "ftp://example.com/skill.yaml".to_string(),
            },
            ..skill
        };
        assert!(!loader.verify_signature(&skill_bad));
    }

    #[test]
    fn test_load_from_url() {
        let dir = std::env::temp_dir().join("marguerite_test_url");
        let loader = SkillLoader::new(&dir);

        let skill = loader
            .load_from_url("https://example.com/skills/my_skill.yaml")
            .unwrap();
        assert_eq!(skill.name, "my_skill");
        assert!(matches!(skill.source, SkillSource::Remote { .. }));
    }

    /// 简易 mock HTTP 服务器：单次响应
    async fn mock_http_server(status: u16, body: &str) -> String {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            loop {
                if let Ok((mut socket, _)) = listener.accept().await {
                    let _ = crate::tests::mock_http::read_request_full(&mut socket).await;
                    let reason = if status >= 500 { "Server Error" } else { "OK" };
                    let header = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/x-yaml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                    let _ = socket.flush().await;
                }
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn test_download_and_register_success() {
        let dir = std::env::temp_dir().join("marguerite_test_download");
        let _ = std::fs::remove_dir_all(&dir);
        let yaml = r#"name: remote_skill
description: "远程技能测试"
version: "0.1.0"
prompt_template: "模板: {{text}}"
"#;
        let url = mock_http_server(200, yaml).await;
        let loader = SkillLoader::new(&dir);

        let skill = loader.download_and_register(&url).await.unwrap();
        assert_eq!(skill.name, "remote_skill");
        assert_eq!(skill.version, "0.1.0");
        assert!(matches!(skill.source, SkillSource::Local { .. }));
        // 已保存到技能目录
        let saved = loader.load("remote_skill").unwrap();
        assert_eq!(saved.name, "remote_skill");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_download_and_register_http_error() {
        let dir = std::env::temp_dir().join("marguerite_test_download_err");
        let url = mock_http_server(404, "not found").await;
        let loader = SkillLoader::new(&dir);
        assert!(loader.download_and_register(&url).await.is_err());
    }

    #[tokio::test]
    async fn test_download_and_register_invalid_yaml() {
        let dir = std::env::temp_dir().join("marguerite_test_download_bad");
        let url = mock_http_server(200, "not: [valid: yaml").await;
        let loader = SkillLoader::new(&dir);
        assert!(loader.download_and_register(&url).await.is_err());
    }

    /// 技能安装前恶意扫描：内嵌 eval 的 YAML 应被拒绝，且绝不能落盘。
    #[test]
    fn install_yaml_blocks_malicious_skill_and_writes_nothing() {
        let dir = std::env::temp_dir().join("marguerite_test_install_malicious");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let loader = SkillLoader::new(&dir);

        let evil = "name: evil\nversion: \"0.1.0\"\nprompt_template: |\n  result = eval(user_input)\n  return result\n";
        let res = loader.install_yaml("evil", evil);
        assert!(res.is_err(), "含 eval 的技能安装应被拒绝");
        let msg = format!("{res:?}");
        assert!(
            msg.contains(crate::security::skill_scan::SKILL_MALWARE_BLOCKED),
            "拒绝消息应带错误码: {msg}"
        );

        // 关键不变量：拒绝后工作区不得出现该文件（绝不写盘）
        assert!(!dir.join("evil.yaml").exists(), "被拦截的技能绝不应落盘");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 良性技能 YAML 应正常安装并落盘。
    #[test]
    fn install_yaml_allows_benign_skill() {
        let dir = std::env::temp_dir().join("marguerite_test_install_benign");
        let _ = std::fs::remove_dir_all(&dir);
        let loader = SkillLoader::new(&dir);

        let good = "name: adder\ndescription: 求和技能\nversion: \"1.0.0\"\nprompt_template: |\n  请对给定数字求和并返回结果。\n";
        let skill = loader.install_yaml("adder", good).unwrap();
        assert_eq!(skill.name, "adder");
        assert!(dir.join("adder.yaml").exists(), "良性技能应已落盘");

        // 已安装的技能可被 load 读出
        let reloaded = loader.load("adder").unwrap();
        assert_eq!(reloaded.name, "adder");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本地导入的恶意技能在加载执行前被 L2 扫描拒绝（运行时防篡改）。
    ///
    /// 模拟"绕过 install_yaml、直接把恶意 YAML 丢进 skills_dir"的场景，
    /// `load()` → `load_from_path` 应在解析前扫描并拒绝。
    #[test]
    fn load_rejects_malicious_skill_on_path() {
        let dir = std::env::temp_dir().join("marguerite_test_load_malicious");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 直接写盘，绕过 install_yaml 的落盘前扫描（模拟本地导入 / 落盘后被篡改）
        let evil = "name: evil\nversion: \"0.1.0\"\nprompt_template: |\n  result = eval(user_input)\n  return result\n";
        std::fs::write(dir.join("evil.yaml"), evil).unwrap();

        let loader = SkillLoader::new(&dir);
        assert!(
            loader.load("evil").is_err(),
            "本地导入的恶意技能加载应被安全层拒绝"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 停用/启用 YAML 技能：文件真实改名、索引持久化、扫描与 load 都看不到它。
    #[test]
    fn yaml_skill_toggle_renames_and_restores() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_skill_toggle_yaml_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("toggle-me.yaml"),
            "name: toggle-me\ndescription: d\nversion: \"1.0.0\"\nprompt_template: |\n  正文\n",
        )
        .unwrap();
        let loader = SkillLoader::new(&dir);
        let scan = loader.scan_all().unwrap();
        assert!(
            scan.skills.iter().any(|s| s.name == "toggle-me"),
            "技能未被扫描到，skipped={:?}",
            scan.report.skipped
        );

        let off = loader.set_enabled("toggle-me", false).unwrap();
        assert!(off.changed);
        assert!(!dir.join("toggle-me.yaml").exists());
        assert!(dir.join("toggle-me.yaml.disabled").exists());
        assert!(loader.is_disabled("toggle-me"));
        assert!(!loader.scan_all().unwrap().skills.iter().any(|s| s.name == "toggle-me"));
        assert!(loader.load("toggle-me").is_err());
        assert_eq!(loader.disabled_skills().len(), 1);

        let on = loader.set_enabled("toggle-me", true).unwrap();
        assert!(on.changed);
        assert!(dir.join("toggle-me.yaml").exists());
        assert!(!loader.is_disabled("toggle-me"));
        assert!(loader.load("toggle-me").is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 停用/启用目录型技能：整目录改名为 *.disabled，扫描跳过。
    #[test]
    fn directory_skill_toggle_renames_and_restores() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_skill_toggle_dir_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let skill_dir = dir.join("pkg");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: dir-skill\ndescription: 测试技能\nauthor: tester\nversion: \"1.2.3\"\n---\n\n正文\n",
        )
        .unwrap();
        let loader = SkillLoader::new(&dir);
        let scan = loader.scan_all().unwrap();
        assert!(
            scan.skills.iter().any(|s| s.name == "dir-skill"),
            "目录技能未被扫描到，skipped={:?}",
            scan.report.skipped
        );

        loader.set_enabled("dir-skill", false).unwrap();
        assert!(dir.join("pkg.disabled").exists());
        assert!(!dir.join("pkg").exists());
        assert!(!loader.scan_all().unwrap().skills.iter().any(|s| s.name == "dir-skill"));

        loader.set_enabled("dir-skill", true).unwrap();
        assert!(dir.join("pkg").exists());
        assert!(loader.scan_all().unwrap().skills.iter().any(|s| s.name == "dir-skill"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 删除技能：YAML 与 SKILL.md 目录都必须被移除，且越界路径被拒绝。
    #[test]
    fn remove_skill_handles_file_and_directory() {
        let dir = std::env::temp_dir().join(format!(
            "marguerite_skill_remove_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        std::fs::write(
            dir.join("file-skill.yaml"),
            "name: file-skill\ndescription: d\nversion: \"1.0.0\"\nprompt_template: |\n  正文\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("pkg").join("SKILL.md"),
            "---\nname: dir-skill\ndescription: 测试技能\nauthor: tester\nversion: \"1.2.3\"\n---\n\n正文\n",
        )
        .unwrap();
        let loader = SkillLoader::new(&dir);

        loader.remove_skill("file-skill").unwrap();
        assert!(!dir.join("file-skill.yaml").exists());
        loader.remove_skill("dir-skill").unwrap();
        assert!(!dir.join("pkg").exists());
        assert!(loader.remove_skill("nope").is_err());
        assert!(loader.remove_skill("../escape").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
