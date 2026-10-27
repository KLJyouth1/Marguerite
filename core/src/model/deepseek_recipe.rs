//! `deepseek-recipe` 接入：把 Marguerite 的对话**按 DeepSeek 官方规范**渲染成
//! V4 / V4.1 的真实 prompt。
//!
//! # 为什么接它（2026-10-23）
//!
//! DeepSeek 在开源 DeepInfer（推理框架）的同一天开源了 `deepseek-recipe`
//! —— 一个 Rust 实现的「协议编解码」库：把 Messages / Chat Completions / Responses
//! 统一成 Conversation，再渲染成 V4/V4.1 的 prompt，并能反向解析输出
//! （正文 / 思考内容 / 工具调用）。
//!
//! 对桌面端而言，**DeepInfer 本身**（Linux + NVIDIA SM100 / 昇腾 NPU 的在线推理引擎）
//! 无法内嵌进 Windows 客户端；能落地的是 recipe 这一侧：
//!
//! 1. **自托管 DeepInfer 的接入正确性**：用户把 DeepInfer 包成 OpenAI 兼容服务后，
//!    我们发出去的 messages 到底被渲染成什么 prompt，此前完全是黑盒；
//! 2. **提示词漂移排查**：系统提示、思考开关、工具定义各自怎么进模板，
//!    出问题时只能猜。这里给出一条「渲染出来看一眼」的确定路径。
//!
//! # 边界（诚实说明）
//!
//! - `render_v41_prompt` 只做**渲染（请求侧）**；渲染结果仅用于诊断与自托管部署，
//!   **不会**替换发往官方 API 的请求体（官方 API 自己会做模板渲染，重复渲染会双重转义）；
//! - `parse_v41_response` / `estimate_tokens` 是 **2026-10-24 新增的诊断工具能力**，
//!   **不参与正常对话链路**：对话链路的响应解析仍由上游 OpenAI 兼容层
//!   （`model/wire.rs` + 思考内容字段）负责。它们的用途是自托管 DeepInfer /
//!   量化评测时，核对「发出去的 prompt」与「收回来的原始输出」是否与 DeepSeek 模板对齐；
//! - `deepseek-recipe-encoding 0.2.0` **只有 encode / render，没有 decode / 反解 API**
//!   （已核对 registry 源码：`PromptEncoding` 仅含 `render_conversation` / `encode`，
//!   导出类型只有 `RenderedPrompt`）。因此 `parse_v41_response` 是一个
//!   **按已知特殊标记切分的最小解析器**：不做正则或语义猜测，只认
//!   `deepseek-recipe-core` 声明的特殊标记拼写；
//! - 精确 token 计数需要配套词表，本模块不启用；`estimate_tokens` 只是**估算**。

use serde_json::{json, Value};

/// 把一段 OpenAI Chat Completions 形状的对话渲染成 DeepSeek V4.1 prompt。
///
/// - `messages`：`[{role, content}, ...]`（支持 `content` 为多段数组的形式，
///   由 recipe 负责把图片转成占位/预处理）；
/// - `model`：缺省 `DeepSeek-V4.1-Flash`（recipe 默认认得 V4-Pro / V4.1-Flash，
///   其它名字可用别名注册，这里透传给 recipe 的默认解析）。
pub fn render_v41_prompt(messages: &Value, model: Option<&str>) -> Result<String, String> {
    use deepseek_recipe::openai::ChatCompletionRequest;
    use deepseek_recipe::request::{ConversionOptions, ProtocolRequest};
    use deepseek_recipe_encoding::v4::dsv41::DeepseekV41Encoding;
    use deepseek_recipe_encoding::PromptEncoding;

    if !messages.is_array() {
        return Err("messages 必须是数组".to_string());
    }
    let body = serde_json::json!({
        "model": model.unwrap_or("DeepSeek-V4.1-Flash"),
        "messages": messages,
    });
    let req: ChatCompletionRequest =
        serde_json::from_value(body).map_err(|e| format!("请求体不符合 Chat Completions 形状: {e}"))?;
    let converted = req
        .convert(ConversionOptions::default())
        .map_err(|e| format!("协议转换失败: {e}"))?;
    let rendered = DeepseekV41Encoding::new().render_conversation(&converted.conversation);
    Ok(rendered.prompt)
}

/// 已知特殊标记在解析器里的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    /// 序列开始：只记录，不切分消息。
    Bos,
    /// 序列结束：只记录，不切分消息。
    Eos,
    /// 角色标记：切分出一条消息。
    Role(&'static str),
    /// 思考内容开始。
    ThinkingStart,
    /// 思考内容结束。
    ThinkingEnd,
    /// 其它已知控制标记（提醒 / DSML / 图像 / 定位）：只记录，拼写保留在正文里。
    Control,
}

/// 已知特殊标记的拼写表。
///
/// 与 `deepseek-recipe-core 0.2.0` 的 `SpecialToken::default_str()` 逐字对齐
/// （registry 源码：`deepseek-recipe-core-0.2.0/src/special_tokens.rs`）。
/// `deepseek-recipe-encoding` 只做 encode、不导出反解 API，故这里镜像一份官方拼写；
/// 同时接受 ASCII 竖线写法，方便从日志 / 文档里手工粘贴。
const KNOWN_TOKENS: &[(&str, TokenKind)] = &[
    ("<｜begin▁of▁sentence｜>", TokenKind::Bos),
    ("<|begin▁of▁sentence|>", TokenKind::Bos),
    // 粘贴来源常见把 `▁` 打成 `_`（终端/博客渲染差异），一并识别为边界标记；
    // 否则它们会被当成未知标记留在正文里，污染诊断结果。
    ("<｜begin_of_sentence｜>", TokenKind::Bos),
    ("<|begin_of_sentence|>", TokenKind::Bos),
    ("<｜end▁of▁sentence｜>", TokenKind::Eos),
    ("<|end▁of▁sentence|>", TokenKind::Eos),
    ("<｜end_of_sentence｜>", TokenKind::Eos),
    ("<|end_of_sentence|>", TokenKind::Eos),
    ("<｜System｜>", TokenKind::Role("system")),
    ("<|System|>", TokenKind::Role("system")),
    ("<｜User｜>", TokenKind::Role("user")),
    ("<|User|>", TokenKind::Role("user")),
    ("<｜Assistant｜>", TokenKind::Role("assistant")),
    ("<|Assistant|>", TokenKind::Role("assistant")),
    ("<think>", TokenKind::ThinkingStart),
    ("</think>", TokenKind::ThinkingEnd),
    ("<｜latest_reminder｜>", TokenKind::Control),
    ("<|latest_reminder|>", TokenKind::Control),
    ("<｜DSML｜", TokenKind::Control),
    ("<|DSML|", TokenKind::Control),
    ("<｜image｜>", TokenKind::Control),
    ("<|image|>", TokenKind::Control),
    ("<｜ref｜>", TokenKind::Control),
    ("<|ref|>", TokenKind::Control),
    ("<｜/ref｜>", TokenKind::Control),
    ("<|/ref|>", TokenKind::Control),
    ("<｜box｜>", TokenKind::Control),
    ("<|box|>", TokenKind::Control),
    ("<｜/box｜>", TokenKind::Control),
    ("<|/box|>", TokenKind::Control),
    ("<｜point｜>", TokenKind::Control),
    ("<|point|>", TokenKind::Control),
    ("<｜/point｜>", TokenKind::Control),
    ("<|/point|>", TokenKind::Control),
    ("<｜polygon｜>", TokenKind::Control),
    ("<|polygon|>", TokenKind::Control),
    ("<｜/polygon｜>", TokenKind::Control),
    ("<|/polygon|>", TokenKind::Control),
];

/// 当前正在累积的一段输出。
struct Segment {
    /// 当前段的角色；`None` = 还没遇到任何 role 标记（V4 的 system 前导文本也在此）。
    role: Option<&'static str>,
    /// 正文（assistant 段不含思考内容）。
    content: String,
    /// 思考内容（仅 assistant 段单独收集）。
    reasoning: String,
    /// 是否位于 `<think>...</think>` 之间。
    in_think: bool,
}

impl Segment {
    fn new() -> Self {
        Self {
            role: None,
            content: String::new(),
            reasoning: String::new(),
            in_think: false,
        }
    }

    fn push(&mut self, text: &str) {
        // 只有 assistant 段把思考内容拆出去；其它角色即使出现 `<think>` 也按正文保留，
        // 避免把 system/user 文本静默丢掉。
        if self.in_think && self.role == Some("assistant") {
            self.reasoning.push_str(text);
        } else {
            self.content.push_str(text);
        }
    }
}

/// 在 `rest` 开头匹配已知特殊标记；没有则返回 `None`。
fn match_known_token(rest: &str) -> Option<(&'static str, TokenKind)> {
    KNOWN_TOKENS
        .iter()
        .find(|(spelling, _)| rest.starts_with(spelling))
        .map(|(spelling, kind)| (*spelling, *kind))
}

/// 若 `rest` 以「未知特殊标记」开头，返回它的字节长度。
///
/// 形态固定为 `<` + 竖线（`|` 或 `｜`）+ 任意内容 + 同一竖线 + `>`，最长 64 字节。
/// 只做字符扫描，不用正则、不做模糊猜测：形状或长度不符就不当成标记。
fn unknown_marker_len(rest: &str) -> Option<usize> {
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first != '<' {
        return None;
    }
    let (_, bar) = chars.next()?;
    if bar != '|' && bar != '｜' {
        return None;
    }
    for (idx, ch) in chars {
        // 已知标记最长 20 余字节；超过 64 字节仍未闭合就不当成标记，避免吞掉正文。
        if idx > 64 {
            return None;
        }
        if ch == bar {
            let after = idx + ch.len_utf8();
            if rest[after..].starts_with('>') {
                return Some(after + 1);
            }
        }
    }
    None
}

/// 记录出现过的标记（按首次出现顺序去重）。
fn record_token(spelling: &str, tokens_found: &mut Vec<String>) {
    if !tokens_found.iter().any(|t| t == spelling) {
        tokens_found.push(spelling.to_string());
    }
}

/// 收尾当前段：裁剪首尾空白，assistant 段拆分 reasoning / answer，非 assistant
/// 段把思考文本并回正文。
fn flush_segment(
    seg: &mut Segment,
    messages: &mut Vec<Value>,
    reasoning_parts: &mut Vec<String>,
    answer_parts: &mut Vec<String>,
) {
    let role = seg.role.unwrap_or("assistant");
    let mut content = std::mem::take(&mut seg.content);
    let mut reasoning = std::mem::take(&mut seg.reasoning);
    seg.in_think = false;

    if role != "assistant" {
        // 非 assistant 段不拆思考：按原顺序并回正文（content 在前、reasoning 在后
        // 的拼接只发生在极端输入里，正常模板不会出现）。
        content.push_str(&reasoning);
        reasoning.clear();
    }

    let content = content.trim().to_string();
    let reasoning = reasoning.trim().to_string();
    if content.is_empty() && reasoning.is_empty() {
        return;
    }

    if role == "assistant" {
        if !reasoning.is_empty() {
            reasoning_parts.push(reasoning.clone());
        }
        if !content.is_empty() {
            answer_parts.push(content.clone());
        }
    }

    messages.push(json!({ "role": role, "content": content }));
}

/// 把模型返回的 DeepSeek V4 / V4.1 原始输出解析成结构化 JSON。
///
/// # 输入
///
/// 含 `<｜begin▁of▁sentence｜>` / `<｜User｜>` / `<｜Assistant｜>` /
/// `<｜end▁of▁sentence｜>` 等标记的原始文本（ASCII 竖线写法同样接受）。
///
/// # 输出
///
/// ```json
/// {"messages":[{"role":"system|user|assistant","content":"…"}],
///  "reasoning":"…","answer":"…","tokensFound":["…"]}
/// ```
///
/// - `messages`：按 role 标记切出的对话段；`content` 已去掉结构标记与思考内容，
///   首尾空白被裁剪。没有任何 role 标记时，整段文本按 `assistant` 处理；
/// - `reasoning`：assistant 段中 `<think>…</think>` 内的文本，多段以空行拼接；
/// - `answer`：assistant 段中思考内容之外的正文，多段以空行拼接；
/// - `tokensFound`：出现过的特殊标记（含未知标记），按首次出现顺序去重，
///   保留原始拼写。未知标记不会丢弃，而是原样留在 `content` / `reasoning` 里。
///
/// # 边界
///
/// 这是**诊断工具**，不是对话链路的解析器：不做 tokenizer 级还原，也不解析
/// DSML 工具调用的语义（标记会被记录，标签正文按原文保留）。
pub fn parse_v41_response(raw: &str) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Err("raw 为空，没有可解析的内容".to_string());
    }

    let mut messages: Vec<Value> = Vec::new();
    let mut tokens_found: Vec<String> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut answer_parts: Vec<String> = Vec::new();
    let mut seg = Segment::new();

    let mut i = 0usize;
    while i < raw.len() {
        let rest = &raw[i..];

        if let Some((spelling, kind)) = match_known_token(rest) {
            record_token(spelling, &mut tokens_found);
            match kind {
                TokenKind::Bos | TokenKind::Eos => {}
                TokenKind::ThinkingStart => seg.in_think = true,
                TokenKind::ThinkingEnd => seg.in_think = false,
                // 非结构标记：记录但保留原文（不静默丢掉图像 / 定位 / DSML 信息）。
                TokenKind::Control => seg.push(spelling),
                TokenKind::Role(role) => {
                    if seg.role.is_some() {
                        flush_segment(
                            &mut seg,
                            &mut messages,
                            &mut reasoning_parts,
                            &mut answer_parts,
                        );
                    } else if !seg.content.is_empty() || !seg.reasoning.is_empty() {
                        // V4 模板把 system 正文直接贴在 BOS 之后（没有 <｜System｜>）；
                        // 一旦出现 role 标记，说明这段前导文本就是 system 段。
                        seg.role = Some("system");
                        flush_segment(
                            &mut seg,
                            &mut messages,
                            &mut reasoning_parts,
                            &mut answer_parts,
                        );
                    }
                    seg.role = Some(role);
                    seg.in_think = false;
                }
            }
            i += spelling.len();
            continue;
        }

        if let Some(len) = unknown_marker_len(rest) {
            let marker = &rest[..len];
            record_token(marker, &mut tokens_found);
            // 未知标记保留原文，同时记入 tokensFound，便于核对模板是否漂移。
            seg.push(marker);
            i += len;
            continue;
        }

        let ch = rest.chars().next().expect("rest 非空");
        let len = ch.len_utf8();
        seg.push(&rest[..len]);
        i += len;
    }

    flush_segment(
        &mut seg,
        &mut messages,
        &mut reasoning_parts,
        &mut answer_parts,
    );

    Ok(json!({
        "messages": messages,
        "reasoning": reasoning_parts.join("\n\n"),
        "answer": answer_parts.join("\n\n"),
        "tokensFound": tokens_found,
    }))
}

/// Token **估算**（不是精确计数）。
///
/// 直接复用仓库既有口径 [`crate::context::session::estimate_tokens`]，不新造第二套规则：
/// CJK / 全角约 1.6 token/字、拉丁字符约 4 字符/token、空白约 0.3 token，
/// 向上取整且下界为 1（空串也返回 1）。
///
/// 精确 token 计数需要 DeepSeek 词表，本模块不启用；返回值只能用于量级 / 预算判断，
/// 不能当计费依据。
pub fn estimate_tokens(text: &str) -> u64 {
    crate::context::session::estimate_tokens(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实渲染：至少验证「用户原话进入了 prompt」且「结构体模板被套上」。
    ///
    /// 不断言具体模板文本（那是上游版本细节，断言死它会在升级时误报），
    /// 只断言用户可验证的两件事。
    #[test]
    fn renders_user_message_into_a_prompt() {
        let msgs = serde_json::json!([
            {"role": "system", "content": "你是严工。"},
            {"role": "user", "content": "帮我查一下缓存命中率"}
        ]);
        let prompt = render_v41_prompt(&msgs, Some("DeepSeek-V4.1-Flash")).expect("应渲染成功");
        assert!(prompt.contains("帮我查一下缓存命中率"), "用户原话必须出现在 prompt: {prompt}");
        assert!(prompt.contains("你是严工"), "系统提示必须进入 prompt: {prompt}");
        assert!(prompt.len() > 20, "prompt 不应退化成空壳");
    }

    #[test]
    fn rejects_non_array_messages() {
        assert!(render_v41_prompt(&serde_json::json!({"role": "user"}), None).is_err());
        assert!(render_v41_prompt(&serde_json::json!("hi"), None).is_err());
    }

    #[test]
    fn rejects_malformed_message_objects() {
        // role 缺失 → recipe 的转换应报错，而不是静默丢消息
        let bad = serde_json::json!([{ "content": "没有 role" }]);
        assert!(render_v41_prompt(&bad, None).is_err());
    }

    fn tokens_of(v: &Value) -> Vec<String> {
        v["tokensFound"]
            .as_array()
            .expect("tokensFound 应为数组")
            .iter()
            .map(|t| t.as_str().expect("标记应为字符串").to_string())
            .collect()
    }

    /// 正常输出：role 标记切分出 system / user / assistant 三段。
    #[test]
    fn parses_plain_v41_output() {
        let raw = concat!(
            "<｜begin▁of▁sentence｜>",
            "<｜System｜>你是严工。",
            "<｜User｜>帮我查缓存命中率",
            "<｜Assistant｜>命中率是 92%。",
            "<｜end▁of▁sentence｜>",
        );
        let v = parse_v41_response(raw).expect("应解析成功");
        let messages = v["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages.len(), 3, "应按 role 标记切出三段: {v}");
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "你是严工。");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"], "帮我查缓存命中率");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "命中率是 92%。");
        assert_eq!(v["reasoning"], "");
        assert_eq!(v["answer"], "命中率是 92%。");

        let tokens = tokens_of(&v);
        for expected in [
            "<｜begin▁of▁sentence｜>",
            "<｜System｜>",
            "<｜User｜>",
            "<｜Assistant｜>",
            "<｜end▁of▁sentence｜>",
        ] {
            assert!(tokens.iter().any(|t| t == expected), "缺少标记 {expected}: {v}");
        }
    }

    /// 粘贴来源常见把 `▁` 打成 `_`：边界标记仍应被识别并剔除，不能污染正文。
    #[test]
    fn strips_underscore_spelled_boundary_markers() {
        let raw = "<|begin_of_sentence|>你是严工。<|User|>hi<|Assistant|>meow<|end_of_sentence|>";
        let v = parse_v41_response(raw).expect("应解析成功");
        let messages = v["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages[0]["content"], "你是严工。", "BOS 不应进入正文: {v}");
        assert_eq!(messages[2]["content"], "meow", "EOS 不应进入正文: {v}");
        let tokens = tokens_of(&v);
        assert!(tokens.iter().any(|t| t == "<|begin_of_sentence|>"));
        assert!(tokens.iter().any(|t| t == "<|end_of_sentence|>"));
    }

    /// 含 reasoning 的 assistant 段：思考内容单独抽出，正文不含思考。
    #[test]
    fn extracts_reasoning_from_assistant_turn() {
        let raw = "<｜User｜>1+1=?<｜Assistant｜><think>先做加法</think>等于 2。<｜end▁of▁sentence｜>";
        let v = parse_v41_response(raw).expect("应解析成功");
        let messages = v["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "等于 2。", "思考内容不应混入正文: {v}");
        assert_eq!(v["reasoning"], "先做加法");
        assert_eq!(v["answer"], "等于 2。");
        let tokens = tokens_of(&v);
        assert!(tokens.iter().any(|t| t == "<think>"));
        assert!(tokens.iter().any(|t| t == "</think>"));
    }

    /// 未知标记：记入 tokensFound，且原样保留在正文里。
    #[test]
    fn keeps_unknown_special_tokens_verbatim() {
        let raw = "<｜User｜>看看 <|Bogus|> 这个标记<｜Assistant｜>好<｜end▁of▁sentence｜>";
        let v = parse_v41_response(raw).expect("应解析成功");
        let messages = v["messages"].as_array().expect("messages 应为数组");
        assert_eq!(
            messages[0]["content"], "看看 <|Bogus|> 这个标记",
            "未知标记必须原样保留: {v}"
        );
        assert!(
            tokens_of(&v).iter().any(|t| t == "<|Bogus|>"),
            "未知标记应记入 tokensFound: {v}"
        );
    }

    /// 估算函数：纯 ASCII / 纯中文 / 空串（沿用仓库既有口径，非精确计数）。
    #[test]
    fn estimates_tokens_for_ascii_cjk_and_empty() {
        // 11 个字符：10 个非空白 × 0.25 + 1 个空白 × 0.3 = 2.8 → 向上取整 3
        assert_eq!(estimate_tokens("hello world"), 3);
        // 4 个汉字 × 1.6 = 6.4 → 向上取整 7
        assert_eq!(estimate_tokens("你好世界"), 7);
        // 空串沿用仓库既有下界（至少 1 个 token），不是精确值
        assert_eq!(estimate_tokens(""), 1);
    }

    /// 空输入拒绝；同时接受 ASCII 竖线写法的标记。
    #[test]
    fn rejects_empty_raw_and_accepts_ascii_pipe_spelling() {
        assert!(parse_v41_response("").is_err());
        assert!(parse_v41_response("  \n ").is_err());
        let v = parse_v41_response("<|User|>问<|Assistant|>答").expect("ASCII 写法应可解析");
        let messages = v["messages"].as_array().expect("messages 应为数组");
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "问");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "答");
    }
}
