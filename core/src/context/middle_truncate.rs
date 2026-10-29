//! 工具输出的「中间截断」（middle truncation）——保留头尾，砍掉中段。
//!
//! # 为什么要在**进上下文之前**截断
//!
//! 工具输出（`cat` 一个大日志、`grep` 整个仓库、`cargo build` 的几百行警告）是上下文
//! 膨胀的头号来源。一旦它进了对话历史，**每轮都要重新付费**：
//! - prompt cache 按**前缀逐 token 一致**命中，历史里塞进一段 200KB 日志后，
//!   这段之后的缓存全部失效；
//! - 窗口压力直接抬升压缩触发频率，压缩又会丢真信息。
//!
//! 所以要在「工具刚返回 → 拼进 history」这一步截，而不是等压缩器事后处理。
//!
//! # 为什么是**中间**截断，而不是尾部截断
//!
//! 工具输出的信息分布极度不均：
//! - **头部**：命令回显、表头、首个错误、文件开头 —— 判断"出了什么事"必须看；
//! - **尾部**：汇总行（`test result: ok. 12 passed`）、exit 状态、最后的错误 —— 判断"成不成"必须看；
//! - **中段**：重复的日志行、成百上千条相同的 warning —— 90% 的体积、几乎零信息增量。
//!
//! 尾部截断（`text[..limit]`）恰好砍掉最有用的汇总行，是这类实现最常见的错误。
//!
//! # 边界约定（可测试的硬约束）
//! - 按**字符**（不是字节）计数，避免把 UTF-8 多字节字符切成半个；
//! - 省略的字符数写进标记，模型能判断"这里缺了多少"，而不是以为输出就到这儿；
//! - 小于阈值时**原样返回**（`None` 表示没截断），调用方据此决定是否打日志；
//! - 原文不丢：调用方在截断前已经把完整输出落到会话台账（`append_tool_activity`），
//!   标记里提示"完整输出在会话台账"，让模型知道还能要。

/// 默认头部保留字符数（20 000 ≈ 命令回显 + 前几条错误）。
pub const DEFAULT_HEAD_CHARS: usize = 20_000;
/// 默认尾部保留字符数（4 000 ≈ 汇总/exit/最后一条错误）。
pub const DEFAULT_TAIL_CHARS: usize = 4_000;

/// 截断结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncated {
    /// 截断后的文本（含省略标记）；未截断时等于原文。
    pub text: String,
    /// 被省略的字符数；`None` = 没截断。
    pub omitted_chars: Option<usize>,
}

impl Truncated {
    /// 是否发生了截断。
    pub fn was_truncated(&self) -> bool {
        self.omitted_chars.is_some()
    }
}

/// 截断标记的默认「全文去哪儿找」提示：指向会话工具活动台账。
pub const DEFAULT_RETRIEVE_HINT: &str =
    "完整输出已记录在本次会话的工具活动台账中，如需细节请读取该工具产物";

/// 保留头 `head` 个字符 + 尾 `tail` 个字符，中段替换成省略标记。
///
/// 当 `head + tail >= 总字符数` 时原样返回（不制造"省略了 0 个字符"这种噪声标记）。
pub fn middle_truncate(text: &str, head: usize, tail: usize) -> Truncated {
    middle_truncate_with_hint(text, head, tail, DEFAULT_RETRIEVE_HINT)
}

/// 同 [`middle_truncate`]，但省略标记里的「去哪儿找全文」由调用方给。
///
/// 入口截流把原文真的落了盘时，提示必须带上**真能用的取回指引**（指纹），
/// 而不是一句读不到的台账地址 —— 那是 R54「文档承诺了、代码没兑现」的形状。
pub fn middle_truncate_with_hint(
    text: &str,
    head: usize,
    tail: usize,
    retrieve_hint: &str,
) -> Truncated {
    let total = text.chars().count();
    if head + tail >= total {
        return Truncated {
            text: text.to_string(),
            omitted_chars: None,
        };
    }
    let omitted = total - head - tail;
    // 收集时按字符边界推进：`chars().take(n)` 天然不会切碎多字节字符。
    let head_part: String = text.chars().take(head).collect();
    let tail_part: String = text.chars().skip(total - tail).collect();
    // 标记要能让模型**看出这是省略**，并知道去哪儿找全文；不要写成"…"就完事。
    let marker = format!("\n\n…[中间省略 {omitted} 个字符；{retrieve_hint}]…\n\n");
    Truncated {
        text: format!("{head_part}{marker}{tail_part}"),
        omitted_chars: Some(omitted),
    }
}

/// 按默认阈值截断（头部 20k + 尾部 4k 字符）。
pub fn middle_truncate_default(text: &str) -> Truncated {
    middle_truncate(text, DEFAULT_HEAD_CHARS, DEFAULT_TAIL_CHARS)
}

/// 入口截流总开关（配置键 `context.entry_truncation`）。
///
/// 默认**开**，与 D11 已上线的现网行为一致；这是紧急关闭阀，不是灰度开关。
/// 配置读失败也按「开」处理：关掉它会悄悄放大 prompt（与 `memory.*` 的
/// fail-open 方向相反 —— 那一类改变的是增强，这一类的改变直接动模型输入）。
///
/// 单测不读磁盘：`Config::load()` 在文件缺失时会**创建默认配置**，用例里调用
/// 就是 AGENTS.md R64 那类「单测写进用户家目录」的事故。
pub fn entry_truncation_enabled() -> bool {
    #[cfg(test)]
    {
        true
    }
    #[cfg(not(test))]
    {
        crate::persistence::config::Config::load()
            .map(|c| c.context.entry_truncation)
            .unwrap_or(true)
    }
}

/// **工具输出回灌上下文前的唯一入口**（三处调用点共用：`agent/runtime.rs`、
/// `agent/stream_loop.rs`、`agent/multi_strategy.rs`）。
///
/// 为什么要收成一个函数：这三条路径是**同一件事的三份实现**（阻塞回路 / 流式回路 /
/// 多策略回路）。本仓已经吃过"改一处漏两处"的亏（双实现漂移），所以截断策略必须
/// 只有一份实现 —— 将来调阈值也只有一个地方要改。
///
/// 截断时把原文留底（[`super::spool`]）并**把指纹写进标记**：这样「完整输出还在」
/// 才是模型可验证的事实，而不只是一句读不到的承诺。
///
/// 截断时打 `info!`：这是**用户可见状态**路径。不打日志的话，"模型说输出只有 20 行"
/// 会被当成工具缺陷去查，而真相是我们自己截的。
pub fn truncate_for_context(tool: &str, output: &str) -> String {
    if !entry_truncation_enabled() {
        return output.to_string();
    }
    let probe = middle_truncate_default(output);
    let Some(omitted) = probe.omitted_chars else {
        return probe.text;
    };
    let hint = match super::spool::spool(output) {
        Some(fp) => format!("原文全文可用 tool_output_retrieve(\"{fp}\") 取回"),
        None => "原文留底不可用（超过留底上限或磁盘不可写）".to_string(),
    };
    let truncated = middle_truncate_with_hint(output, DEFAULT_HEAD_CHARS, DEFAULT_TAIL_CHARS, &hint);
    tracing::info!(
        tool = %tool,
        omitted_chars = omitted,
        kept_chars = truncated.text.chars().count(),
        "工具输出超过上下文预算，已中间截断（原文已留底，可按指纹取回）"
    );
    truncated.text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_when_under_budget() {
        let text = "hello 世界";
        let out = middle_truncate(text, 10, 10);
        assert_eq!(out.text, text);
        assert!(!out.was_truncated());
    }

    #[test]
    fn keeps_head_and_tail_and_reports_omitted_count() {
        let text: String = (0..100).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
        let out = middle_truncate(&text, 10, 10);
        assert!(out.was_truncated());
        assert_eq!(out.omitted_chars, Some(80));
        // 头 10 个与尾 10 个必须逐字符保留
        assert!(out.text.starts_with(&text[..10]));
        assert!(out.text.ends_with(&text[text.len() - 10..]));
        // 标记里必须出现"完整输出"与省略数量，否则模型无从判断信息缺失
        assert!(out.text.contains("省略 80 个字符"));
        assert!(out.text.contains("完整输出"));
    }

    #[test]
    fn counts_chars_not_bytes_so_multibyte_is_never_split() {
        // 全是 3 字节的中文字符：按字符数截断后，结果必须是合法 UTF-8 且首尾字符完整
        let text = "中".repeat(50);
        let out = middle_truncate(&text, 5, 5);
        assert_eq!(out.omitted_chars, Some(40));
        assert!(out.text.starts_with("中中中中中"));
        assert!(out.text.ends_with("中中中中中"));
    }

    #[test]
    fn boundary_totals_are_not_truncated() {
        let text = "x".repeat(20);
        // 刚好等于总长：不截断（避免"省略 0 个字符"的噪声）
        assert!(!middle_truncate(&text, 10, 10).was_truncated());
        // 少一个字符：截断，省略 1
        assert_eq!(middle_truncate(&text, 10, 9).omitted_chars, Some(1));
    }

    #[test]
    fn tail_of_zero_still_works() {
        let text = "abcdefghij";
        let out = middle_truncate(text, 4, 0);
        assert_eq!(out.omitted_chars, Some(6));
        assert!(out.text.starts_with("abcd"));
        assert!(!out.text.ends_with('j'));
    }

    #[test]
    fn truncate_for_context_hands_out_a_fingerprint_that_really_retrieves() {
        let raw = format!(
            "{}{}{}",
            "H".repeat(30_000),
            "M".repeat(30_000),
            "T".repeat(30_000)
        );
        let text = truncate_for_context("big_out", &raw);
        assert!(text.len() < raw.len());
        assert!(text.starts_with('H'));
        assert!(text.ends_with('T'));
        assert!(text.contains("中间省略"), "不得静默丢弃：{}", &text[..200]);
        let fp = text
            .split("tool_output_retrieve(\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .expect("标记必须给出可取回的指纹，而不是一句读不到的台账地址");
        assert_eq!(
            crate::context::spool::retrieve(fp).as_deref(),
            Some(raw.as_str()),
            "标记里的指纹必须真能逐字节取回原文"
        );
    }

    #[test]
    fn under_budget_output_is_untouched() {
        let raw = "一行普通输出";
        assert_eq!(truncate_for_context("echo", raw), raw);
    }
}
