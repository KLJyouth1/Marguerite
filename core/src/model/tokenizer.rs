//! DeepSeek 词表精确 token 计数（可选能力，缺失时诚实降级）
//!
//! # 为什么需要
//!
//! 上下文预算、压缩触发线、`model.deepseek.render` 的返回都需要 token 数。
//! 全仓此前的口径是**估算**（`context::session::estimate_tokens`：ASCII 约 4 字符
//! 1 token、CJK 约 1.6 token/字）。估算是保守的、够用的，但它不是「真实分词结果」——
//! 做量化评测、对齐自托管推理服务时，差 10% 就足以让结论失真。
//!
//! 本模块用 HuggingFace `tokenizers` 加载 **DeepSeek 官方词表**（`tokenizer.json`），
//! 给出真正的 BPE 分词计数。
//!
//! # 词表从哪来（不入安装包）
//!
//! 词表约 7.5 MB，属模型资产而非代码资产，因此**不打进安装包**，按以下顺序解析：
//!
//! 1. 环境变量 `MARGUERITE_TOKENIZER`（指向任意 `tokenizer.json`）
//! 2. 托管目录 `%LOCALAPPDATA%\Marguerite\tokenizers\deepseek-v3.json`
//! 3. 安装目录同级 `tokenizers/deepseek-v3.json`（运维手工放置）
//!
//! 三者都没有 → `count_exact` 返回 `Err`，调用方继续用估算值，并在返回体里
//! 明确标注「精确计数不可用 + 原因」，**不假装精确**。
//!
//! # 诚实边界
//!
//! - 只做「文本 → token 数」；不做 chat 模板包裹（模板由 `deepseek_recipe` 负责）。
//! - 词表版本与模型版本强相关：拿 V3 词表去数 V4 文本在特殊标记上会有偏差，
//!   所以状态接口会把实际使用的文件路径一并回传，便于核对。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{json, Value};

/// 词表候选文件名（托管目录与安装目录都用它）。
const VOCAB_FILE: &str = "deepseek-v3.json";

/// 词表只加载一次（7.5 MB 解析有一定成本，且失败原因不会随请求变化）。
static CACHE: OnceLock<Result<tokenizers::Tokenizer, String>> = OnceLock::new();

/// 纯函数：按优先级挑选词表路径（参数全部注入，便于单测）。
pub fn pick_tokenizer_path(
    env: Option<&Path>,
    managed_dir: Option<&Path>,
    exe_dir: &Path,
) -> Option<PathBuf> {
    if let Some(p) = env.filter(|p| p.is_file()) {
        return Some(p.to_path_buf());
    }
    if let Some(dir) = managed_dir {
        let cand = dir.join(VOCAB_FILE);
        if cand.is_file() {
            return Some(cand);
        }
    }
    if !exe_dir.as_os_str().is_empty() {
        let cand = exe_dir.join("tokenizers").join(VOCAB_FILE);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 解析当前环境下的词表路径（无则 `None`）。
pub fn resolve_path() -> Option<PathBuf> {
    let env = std::env::var_os("MARGUERITE_TOKENIZER").map(PathBuf::from);
    let managed = std::env::var_os("LOCALAPPDATA")
        .map(|b| PathBuf::from(b).join("Marguerite").join("tokenizers"));
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    pick_tokenizer_path(env.as_deref(), managed.as_deref(), &exe_dir)
}

fn tokenizer() -> &'static Result<tokenizers::Tokenizer, String> {
    CACHE.get_or_init(|| {
        let Some(path) = resolve_path() else {
            return Err(format!(
                "未找到 DeepSeek 词表（可放 %LOCALAPPDATA%\\Marguerite\\tokenizers\\{VOCAB_FILE}，\
                 或设置环境变量 MARGUERITE_TOKENIZER 指向 tokenizer.json）"
            ));
        };
        tokenizers::Tokenizer::from_file(&path)
            .map_err(|e| format!("加载词表失败（{}）：{e}", path.display()))
    })
}

/// 精确分词计数。
///
/// `add_special_tokens = false`：这里的文本是**已经渲染好的整段 prompt**，
/// 特殊 token 由调用方按模板自行写入，重复添加会多算。
pub fn count_exact(text: &str) -> Result<u64, String> {
    let tk = tokenizer().as_ref().map_err(|e| e.clone())?;
    let enc = tk
        .encode(text, false)
        .map_err(|e| format!("分词失败：{e}"))?;
    Ok(enc.len() as u64)
}

/// 词表状态（供设置页 / 诊断用：是否可用、用的哪份文件）。
pub fn status() -> Value {
    let path = resolve_path();
    match tokenizer().as_ref() {
        Ok(_) => json!({
            "available": true,
            "path": path.map(|p| p.display().to_string()),
            "vocabFile": VOCAB_FILE,
        }),
        Err(e) => json!({
            "available": false,
            "path": path.map(|p| p.display().to_string()),
            "vocabFile": VOCAB_FILE,
            "error": e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmp(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("mq-tok-{}-{tag}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn env_override_beats_managed_and_exe() {
        let root = tmp("env");
        let envp = root.join("custom.json");
        fs::write(&envp, b"{}").unwrap();
        let managed = root.join("managed");
        fs::create_dir_all(&managed).unwrap();
        fs::write(managed.join(VOCAB_FILE), b"{}").unwrap();
        let exe = root.join("app");
        fs::create_dir_all(exe.join("tokenizers")).unwrap();
        fs::write(exe.join("tokenizers").join(VOCAB_FILE), b"{}").unwrap();

        assert_eq!(
            pick_tokenizer_path(Some(&envp), Some(&managed), &exe).as_deref(),
            Some(envp.as_path()),
            "环境变量必须最优先"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn managed_dir_beats_install_dir() {
        let root = tmp("managed");
        let managed = root.join("managed");
        fs::create_dir_all(&managed).unwrap();
        let managed_vocab = managed.join(VOCAB_FILE);
        fs::write(&managed_vocab, b"{}").unwrap();
        let exe = root.join("app");
        fs::create_dir_all(exe.join("tokenizers")).unwrap();
        fs::write(exe.join("tokenizers").join(VOCAB_FILE), b"{}").unwrap();

        assert_eq!(
            pick_tokenizer_path(None, Some(&managed), &exe).as_deref(),
            Some(managed_vocab.as_path())
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn returns_none_without_vocab() {
        let root = tmp("none");
        let exe = root.join("app");
        fs::create_dir_all(&exe).unwrap();
        assert!(pick_tokenizer_path(None, Some(&root.join("absent")), &exe).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    /// 真词表存在时验证精确计数；不存在时跳过（CI 不装词表）。
    ///
    /// 断言用「同一文本的精确值 ≤ 估算值的 2 倍」这种松约束：这里要证明的是
    /// **真的跑通了分词**，不是钉死某个具体数字（词表版本会变）。
    #[test]
    fn exact_count_works_when_vocab_present() {
        if resolve_path().is_none() {
            return;
        }
        let text = "hello world, 你好世界";
        let exact = count_exact(text).expect("词表在就应能分词");
        assert!(exact > 0, "非空文本的 token 数必须大于 0");
        assert_eq!(count_exact("").expect("空串也应可分词"), 0, "空串应为 0 token");
        let estimate = crate::context::session::estimate_tokens(text);
        assert!(
            exact <= estimate * 2,
            "精确值 {exact} 与估算值 {estimate} 差异过大，可能词表不匹配"
        );
    }
}
