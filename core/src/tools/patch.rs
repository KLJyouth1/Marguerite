//! 工具面 patch 层（热启停，免重启）。
//!
//! 对应 dsh-market 的 `cordis.patch.yml` 思路：**基础定义不动，补丁层单独落盘**，
//! 运行时把补丁叠加到基础配置上。这里补丁的对象是「工具面」——被禁用的工具从
//! 智能体可见的工具集中消失（而不是只在 UI 上打个灰标）。
//!
//! ## 为什么这样就能「热」生效
//!
//! 工具面在 `Orchestrator` 构造时装配（`orchestration::Orchestrator::new_*` 内的
//! `register_builtin_tools` + MCP + 技能工具），而网关聊天路径**每轮请求都新建
//! 一个 Orchestrator**（`gateway/chat.rs`）。因此补丁文件在装配点被读取一次，
//! 写盘后**下一个轮次**即生效——无需重启守护进程，也无需热重载运行时。
//!
//! ## 诚实边界（写进契约，不靠文档口头承诺）
//!
//! - 补丁**只影响之后新建的编排器**：正在执行中的轮次不会中途被剥夺工具。
//!   这是刻意的（中途替换工具面会破坏 KV 前缀与工具循环一致性），RPC 响应里
//!   显式返回 `appliesTo: "next_turn"`，不谎称即时生效。
//! - 补丁文件损坏时**绝不静默清空**：清空等于把用户禁用的工具悄悄放回来。
//!   读取失败时保留进程内最后一次成功读取的补丁，并把错误如实上抛。
//! - 未知工具名不写入补丁（否则补丁会记录一堆永远不存在的名字，看起来"生效了"
//!   其实什么也没发生）。由调用方（RPC）校验后只提交已知名。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// 补丁文件路径覆盖（测试重定向到临时目录用，避免污染真实数据目录）。
pub const ENV_TOOL_PATCH_FILE: &str = "MARGUERITE_TOOL_PATCH_FILE";

/// 补丁文件版本。结构变更时递增；不认识的版本按「不可读」处理（回退 last-good）。
pub const TOOL_PATCH_VERSION: u32 = 1;

/// 工具面补丁：当前被禁用的工具名集合。
///
/// `disabled` 用 `BTreeSet` 保证序列化顺序稳定（补丁文件 diff 可读、可复现）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPatch {
    #[serde(default)]
    pub version: u32,
    /// 最后一次写盘时间（毫秒）；只读来源（文件缺失）时为 0。
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub disabled: BTreeSet<String>,
}

impl ToolPatch {
    /// 空补丁（无任何工具被禁用）。
    pub fn empty() -> Self {
        Self {
            version: TOOL_PATCH_VERSION,
            updated_at: 0,
            disabled: BTreeSet::new(),
        }
    }

    /// 该工具是否被补丁禁用。
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.contains(name)
    }

    /// 被禁用的工具名（有序，供 RPC / UI 展示）。
    pub fn disabled_names(&self) -> Vec<String> {
        self.disabled.iter().cloned().collect()
    }
}

/// 补丁文件路径：环境变量覆盖 > 应用数据目录。
pub fn patch_path() -> PathBuf {
    if let Ok(p) = std::env::var(ENV_TOOL_PATCH_FILE) {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    crate::persistence::config::Config::data_dir().join("tool_patch.json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 规范化工具名：去空白；空名丢弃（防止把 `""` 写进补丁）。
fn normalize_names<I: IntoIterator<Item = String>>(names: I) -> BTreeSet<String> {
    names
        .into_iter()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// 从指定路径读取补丁（无缓存，纯 IO + 校验）。
///
/// - 文件不存在 → 空补丁（首次使用的正常状态，不是错误）；
/// - JSON 非法 / 版本不认识 → `Err`（**不**退化成空补丁，避免静默放回被禁工具）。
pub fn read_patch_at(path: &Path) -> Result<ToolPatch, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ToolPatch::empty()),
        Err(e) => return Err(format!("读取工具补丁失败（{}）：{e}", path.display())),
    };
    let mut patch: ToolPatch = serde_json::from_str(&text)
        .map_err(|e| format!("工具补丁 JSON 非法（{}）：{e}", path.display()))?;
    if patch.version != TOOL_PATCH_VERSION {
        let seen = if patch.version == 0 {
            "缺少 version 字段".to_string()
        } else {
            format!("文件为 {}", patch.version)
        };
        return Err(format!(
            "工具补丁版本不支持（{seen}），本程序为 {TOOL_PATCH_VERSION}"
        ));
    }
    // 规范化一次，避免手工编辑的文件把空白名带进来。
    patch.disabled = normalize_names(patch.disabled.into_iter().collect::<Vec<_>>());
    Ok(patch)
}

/// 原子写入补丁（临时文件 + rename），返回写入后的补丁。
pub fn write_patch_at(path: &Path, disabled: &BTreeSet<String>) -> Result<ToolPatch, String> {
    let patch = ToolPatch {
        version: TOOL_PATCH_VERSION,
        updated_at: now_ms(),
        disabled: disabled.clone(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建工具补丁目录失败（{}）：{e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&patch)
        .map_err(|e| format!("序列化工具补丁失败：{e}"))?;
    // 先写临时文件再原子替换：读者永远看不到半份 JSON。
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text).map_err(|e| format!("写入工具补丁失败（{}）：{e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换工具补丁失败（{}）：{e}", path.display())
    })?;
    Ok(patch)
}

/// 进程内补丁缓存：保留**最后一次成功读取**的补丁，供损坏时回退。
#[derive(Debug, Default)]
struct PatchCache {
    loaded: bool,
    last_good: ToolPatch,
    /// 最近一次读取失败的原因（读取成功后清空）。
    error: Option<String>,
}

fn cache() -> &'static Mutex<PatchCache> {
    static C: OnceLock<Mutex<PatchCache>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(PatchCache::default()))
}

/// 当前生效的补丁（带最近一次读取错误）。
///
/// 每次调用都会重新读盘（工具面每轮重建，读一个几 KB 的 JSON 可忽略），因此
/// **外部编辑补丁文件同样生效**；读失败时回退到进程内最后一次成功值。
pub fn current_with_error() -> (ToolPatch, Option<String>) {
    let path = patch_path();
    let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
    match read_patch_at(&path) {
        Ok(patch) => {
            g.loaded = true;
            g.last_good = patch.clone();
            g.error = None;
            (patch, None)
        }
        Err(err) => {
            if !g.loaded {
                // 从未成功读过：没有可信基线，只能以空补丁起步，但错误照实上报。
                g.loaded = true;
                g.last_good = ToolPatch::empty();
            }
            g.error = Some(err.clone());
            (g.last_good.clone(), Some(err))
        }
    }
}

/// 当前生效的补丁（忽略读取错误，供装配点使用）。
pub fn current() -> ToolPatch {
    current_with_error().0
}

/// 覆盖写补丁并刷新缓存，返回写入后的补丁。
pub fn set_disabled(names: Vec<String>) -> Result<ToolPatch, String> {
    let disabled = normalize_names(names);
    let patch = write_patch_at(&patch_path(), &disabled)?;
    let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
    g.loaded = true;
    g.last_good = patch.clone();
    g.error = None;
    Ok(patch)
}

/// 清空补丁缓存（单测隔离用：避免用例之间共享进程内 last-good）。
#[cfg(test)]
pub fn reset_cache_for_test() {
    let mut g = cache().lock().unwrap_or_else(|e| e.into_inner());
    *g = PatchCache::default();
}

/// 把补丁叠加到工具注册表：移除被禁用的本地工具。
///
/// 返回**实际被移除**的名字，而不是补丁全集——补丁里可能残留当前根本不存在的
/// 工具名（换了版本、工具改名），只回显补丁内容会让调用方误以为那些工具被停用了。
///
/// 只作用于本地工具：MCP 远端工具与技能工具不归本补丁管（它们的生命周期在各自
/// 的服务器/技能目录里），混在一起会让「禁用」语义含混。
pub fn apply_to_registry(
    registry: &mut crate::mcp::registry::ToolRegistry,
    patch: &ToolPatch,
) -> Vec<String> {
    let mut removed = Vec::new();
    for name in &patch.disabled {
        if registry.remove_local(name) {
            removed.push(name.clone());
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 环境变量是**进程级**的：本模块内触碰 `ENV_TOOL_PATCH_FILE` 的用例必须串行，
    /// 否则会互相改掉对方的路径。另外这些用例一律只用**不存在的工具名**，
    /// 这样即使环境变量在并行执行的其它用例（如 `Orchestrator::new`）眼里短暂可见，
    /// 也绝不可能改变真实工具面——本轮已实测过一次这类串扰误伤。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn tmp_path(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "mgt_tool_patch_{}_{}_{}.json",
            std::process::id(),
            tag,
            seq
        ))
    }

    #[test]
    fn missing_file_is_empty_patch_not_an_error() {
        let p = tmp_path("missing");
        let patch = read_patch_at(&p).expect("文件缺失应是空补丁而非错误");
        assert!(patch.disabled.is_empty());
        assert_eq!(patch.version, TOOL_PATCH_VERSION);
    }

    #[test]
    fn write_then_read_round_trips_and_leaves_no_tmp_file() {
        let p = tmp_path("roundtrip");
        let mut disabled = BTreeSet::new();
        disabled.insert("shell".to_string());
        disabled.insert("computer_use".to_string());
        let written = write_patch_at(&p, &disabled).expect("写入应成功");
        assert!(written.updated_at > 0, "写盘须记录时间");

        let read = read_patch_at(&p).expect("读回应成功");
        assert_eq!(read.disabled, disabled);
        assert!(
            !p.with_extension("json.tmp").exists(),
            "原子替换后不得残留临时文件"
        );
        let _ = std::fs::remove_file(&p);
    }

    /// 补丁文件损坏**绝不静默清空**——清空等于把用户禁用的工具悄悄放回来。
    #[test]
    fn corrupt_file_is_an_error_not_a_silent_empty_patch() {
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{ not json").unwrap();
        let err = read_patch_at(&p).expect_err("坏 JSON 必须报错");
        assert!(err.contains("JSON 非法"), "错误须可诊断，实际：{err}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let p = tmp_path("version");
        std::fs::write(
            &p,
            r#"{"version":99,"updated_at":0,"disabled":["shell"]}"#,
        )
        .unwrap();
        let err = read_patch_at(&p).expect_err("不认识的版本必须拒绝");
        assert!(err.contains("版本不支持"), "实际：{err}");
        let _ = std::fs::remove_file(&p);
    }

    /// 读失败时回退到进程内 last-good（而不是放回被禁工具）。
    #[test]
    fn current_falls_back_to_last_good_when_file_breaks() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_cache_for_test();
        let p = tmp_path("fallback");
        std::env::set_var(ENV_TOOL_PATCH_FILE, &p);
        // 刻意用不存在的工具名：环境变量在并行用例眼里短暂可见，也不能改变真实工具面。
        let fake = "zz_not_a_real_tool";

        set_disabled(vec![fake.to_string()]).expect("写盘应成功");
        let (patch, err) = current_with_error();
        assert!(err.is_none(), "健康文件不应报错");
        assert!(patch.is_disabled(fake));

        // 把文件改坏：必须仍认为该工具被禁用，并如实报错
        std::fs::write(&p, "garbage").unwrap();
        let (patch, err) = current_with_error();
        assert!(err.is_some(), "坏文件必须上抛错误");
        assert!(
            patch.is_disabled(fake),
            "坏文件不得把已禁用工具放回来（last-good 回退）"
        );

        std::env::remove_var(ENV_TOOL_PATCH_FILE);
        reset_cache_for_test();
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn blank_names_are_dropped_on_write() {
        let p = tmp_path("blank");
        let patch = write_patch_at(
            &p,
            &normalize_names(vec!["  shell  ".to_string(), "   ".to_string(), String::new()]),
        )
        .expect("写入应成功");
        assert_eq!(patch.disabled_names(), vec!["shell".to_string()]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn patch_path_honours_env_override() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let p = tmp_path("env");
        std::env::set_var(ENV_TOOL_PATCH_FILE, &p);
        assert_eq!(patch_path(), p);
        std::env::remove_var(ENV_TOOL_PATCH_FILE);
        assert!(patch_path().ends_with("tool_patch.json"));
    }

    /// 端到端：补丁叠加到**真实**内置工具注册表后，被禁工具确实从工具面消失，
    /// 未禁用的工具一个都不少；补丁里不存在的名字如实计入「未生效」。
    #[test]
    fn patch_removes_real_builtin_tool_from_surface() {
        let dir = std::env::temp_dir().join(format!("mgt_tool_patch_ws_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut reg = crate::mcp::registry::ToolRegistry::new();
        crate::tools::register_builtin_tools(&mut reg, &dir, None, None, None)
            .expect("内置工具注册应成功");

        let before = reg.list_tools();
        assert!(before.contains(&"shell_exec".to_string()), "基线应含 shell_exec");
        let baseline = before.len();

        let mut patch = ToolPatch::empty();
        patch.disabled.insert("shell_exec".to_string());
        // 补丁里塞一个不存在的名字：不得谎称它生效了
        patch.disabled.insert("no_such_tool_xyz".to_string());

        let removed = apply_to_registry(&mut reg, &patch);
        assert_eq!(removed, vec!["shell_exec".to_string()], "只应报告真正被移除的工具");

        let after = reg.list_tools();
        assert!(
            !after.contains(&"shell_exec".to_string()),
            "被禁工具不得再出现在工具面"
        );
        assert_eq!(after.len(), baseline - 1, "只应少这一个工具");
    }
}
