//! 全仓唯一的文本脱敏实现（S6）。
//!
//! # 为什么收敛成一处
//!
//! 修复前Rust 侧有6 处各自为政的脱敏（`doctor/appliance.rs`、
//! `gateway/system.rs`、`ops/command.rs`、`cache/semantic.rs` …），
//! 形态与覆盖范围各不相同——其中只有 doctor 那一版覆盖了裸令牌，
//! 而它**不认识 `key=value` 赋值**。诊断包要把侧车日志、配置、
//! 退出证据打包给外部支持方，收集点可以有十处，**脱敏只能有一个收口**。
//! 因此本模块是唯一实现，[`crate::doctor::appliance::redact_secrets`]
//! 已改为委托至此。
//!
//! Python 侧对应实现：
//! `pyengine/src/opensquilla/observability/redact.py::scrub_text`。
//! 两者按同一份规则维护；Rust 侧**不能**用 `regex` crate 的
//! look-around（`regex` 不支持回溯断言），故下面对应处的注释说明了
//! 为什么那些断言在 Rust 里可以省掉。
//!
//! # 安全边界
//!
//! 脱敏是**分享前**的防线，不是加密：它降低误贴到 issue 的概率，
//! 不能替代「密钥不进日志」。`~/.marguerite/.env` 才是密钥唯一的合法落盘位置。

use std::sync::OnceLock;

use regex::Regex;

/// 脱敏后的统一占位符。
///
/// 与 Python 侧 `redact.py::_REDACTED` 同名同形：两侧产出的产物可以互相解读。
pub const REDACTED: &str = "[redacted]";

/// 家目录在产物中的占位符。
pub const USER_HOME_PLACEHOLDER: &str = "[USER]";

/// 形如密钥的赋值键名。
///
/// - 前半段是**整体命中**的固定键（`api_key` / `token` / `password` …）。
/// - 后半段是**后缀**分支（`foo_token` / `bar_secret`）：刻意**不含**裸 `_key`
///   后缀，否则 `session_key`、`cache_key` 这类良性标识符会在诊断报告里被抹成
///   一片 `[redacted]`，报告随之失去可读性（而可读性正是脱敏之外的第二目的）。
///
/// 为什么不加 Python 版那条 `(?<![a-z0-9_])` 负向后顾断言：那条断言是为
/// **CPython 回溯引擎**兜底的——`*` 前缀会在词元run 上被逐字符重扫，
/// MB 级base64/hex 日志尾会退化到二次复杂度。Rust 的 `regex` 是NFA/DFA
/// 混合引擎、保证线性时间，同样的输入不会退化；并且缺少锚定时
/// `[A-Za-z0-9_]*` 从run 起点贪婪匹配即可覆盖同一段文本，**匹配结果不变**。
/// 去掉它换来的是更简单、可读性更好的模式。
const SECRET_KEY_PATTERN: &str = r#"(?i:api[_-]?key|token|secret|password|
    secret[_-]?access[_-]?key|secret[_-]?key|secret|app[_-]?secret|
    signing[_-]?secret|private[_-]?key|verification[_-]?token|encrypt[_-]?key|
    encoding[_-]?aes[_-]?key|
    [a-z0-9_]*(?:_token|_secret|_password|_api_key))"#;

/// `Authorization: <scheme> <credential>` 整段脱敏。
///
/// **必须排在赋值规则之前**。赋值规则的值字符类在空白处停：
/// 对 `Authorization: Bearer xxx` 它会把 `Bearer` 当成整个值吃掉，留下裸的
/// `xxx`——而此时 `bearer_re` 赖以定位方案词的锚点已被替换掉，再也匹配不上，
/// 真正的令牌原样外泄（实测泄漏，见 `no_leak_via_assignment_half_eat`）。
/// 这里整段处理并**保留方案词**（`Basic` / `Bearer` / `Digest` …）：
/// 排障时需要知道用的是哪种鉴权，只抹掉凭据本身。
fn authorization_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // 单行书写：本模式**不带** `(?x)`，多行缩进会变成字面量空格/换行。
        Regex::new(
            r#"(?i)(?P<pre>["']?authorization["']?[\x20\t]*[=:][\x20\t]*)(?P<scheme>[A-Za-z][A-Za-z0-9-]*[\x20\t]+)?(?P<val>"[^"\n]*"|'[^'\n]*'|[^\n"',]+)"#,
        )
        .expect("authorization 脱敏正则必须可编译（模式为常量）")
    })
}

/// `key = value` / `key: value` / `"key": "value"` 赋值形态。
///
/// 形状说明（逐条都有代价，换来的是不误伤、不漏读）：
///
/// - 分隔符用 `[\x20\t]*` 而**不是** `\s*`：裸标签 `password:\n` 若用 `\s*`，
///   会把分隔符一路吃到下一行、把下一行首词当成密码吞掉。
///   ⚠️ 这里必须写 `\x20`（空格的十六进制转义）而不是字面空格：本模式带
///   `(?x)` 标志，而 Rust 的 `(?x)` **连字符类内部的空白也一并忽略** ——
///   写 `[ \t]` 会被静默降级成 `[\t]`，于是 `api_key: sk-xxx` 这种
///   **空格分隔的赋值完全不脱敏**（`config.yaml` 恰是这一形态）。
///   这是实测踩到的坑：`token=abc` 命中而 `token: abc` 不命中。
/// - 值的字符类排除 `[`：占位符 `[redacted]` 自身**不得**再次匹配，
///   否则二次脱敏会产出 `[redacted]]`。这条与Python 侧
///   `(?P<value>\[redacted\]...)` 分支是同一个目的，两种引擎两种写法。
/// - 值的字符类同时排除引号：带引号的值交给前面的引号分支整体匹配
///   （`"a b c"` 含空格，不会被逐字符分支截半）。
fn assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r#"(?x)
            (?P<pre>["']?{key}["']?[\x20\t]*[=:][\x20\t]*)
            (?P<val>"[^"\n]*"|'[^'\n]*'|[^\s"',;)\]\[]+)"#,
            key = SECRET_KEY_PATTERN
        ))
        .expect("assignment 脱敏正则必须可编译（模式为常量，见 SECRET_KEY_PATTERN）")
    })
}

/// 键名本身是否属于「值必须整体抹掉」的敏感键（JSON 树用；文本用 [`scrub_text`]）。
///
/// 文本脱敏靠 `key=value` 的**整体形态**，而结构化数据里键与值是两个独立标量，
/// 赋值正则看不到它们的关系 —— 所以 JSON 先按键名判定，值再交给 [`scrub_text`]。
///
/// 判定两步：先按 [`SECRET_KEY_PATTERN`] 整体匹配（`api_key` / `token` /
/// `x_api_token` …），再用「去掉非字母数字后的小写形态」兜住驼峰与连字符写法
/// （`apiKey` / `x-api-key` / `clientSecret`）。第二步刻意**强于** Python 侧
/// `safety/secret_redaction.py::is_secret_key`：那张表只做字面包含关系，
/// `accessToken` / `privateKey` 会漏 —— 而审批卡片是展示面，多抹一个键只是少一条
/// 信息，漏抹一个键就是把凭据送进浏览器。
pub fn is_secret_key(key: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(&format!(r"(?i)^(?:{SECRET_KEY_PATTERN})$"))
            .expect("敏感键名正则必须可编译（模式为常量）")
    });
    if re.is_match(key) {
        return true;
    }
    let compact: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        compact.as_str(),
        "token"
            | "apikey"
            | "xapikey"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "bearertoken"
            | "secret"
            | "clientsecret"
            | "privatekey"
            | "secretkey"
            | "credential"
            | "credentials"
            | "password"
            | "authorization"
    )
}

/// 裸`Authorization: Bearer <token>` 形态。
///
/// 保留 `Bearer ` 方案词：诊断报告需要知道**用的是哪种鉴权**，
/// 被抹掉就成了无信息量的 `[redacted]`。
fn bearer_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(?P<pre>\bbearer[ \t]+)[A-Za-z0-9._~+/=-]+")
            .expect("bearer 脱敏正则必须可编译（模式为常量）")
    })
}

/// 全局 distinctive 的裸令牌前缀。
///
/// 覆盖 provider / channel 报错里**没有 `key=` 结构**的回显形态
/// （`Incorrect API key sk-... provided`）——这类消息会直接流进
/// 侧车 stderr 与 `turn_errors`。每条分支都锚在词元run 边界并设长度下限，
/// 使普通散文（`skill`、`risk-free`、`eyJustSaying`）不会误命中；
/// 过度遮蔽无害，泄漏令牌尾部才有害。
fn bare_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            \b(?:
                sk-[A-Za-z0-9_-]{16,}                # OpenAI 风格（含 sk-proj- / sk-ant-）
              | sk_tr_[A-Za-z0-9_-]{16,}            # TokenRhythm（下划线而非连字符）
              | xox[abposr]-[A-Za-z0-9-]{10,}       # Slack bot/user/app/legacy
              | gh[pousr]_[A-Za-z0-9_]{16,}         # GitHub classic（ghp/gho/ghu/ghs/ghr）
              | github_pat_[A-Za-z0-9_]{16,}        # GitHub fine-grained PAT
              | AKIA[0-9A-Z]{16}                    # AWS access key id
              | eyJ[A-Za-z0-9_-]{5,}(?:\.[A-Za-z0-9_-]{8,}){2,}  # JWT 状点分base64url
              | AIza[0-9A-Za-z_-]{20,}              # Google API key
            )",
        )
        .expect("裸令牌脱敏正则必须可编译（模式为常量）")
    })
}

/// URL 内嵌凭据：`scheme://user:pass@host`。
///
/// 只抹userinfo，保留 scheme 与 host：诊断报告里「连的是哪个端点」是排障关键，
/// 被抹掉就只剩一句「请求失败」。
fn url_credential_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?P<pre>[a-z][a-z0-9+.\-]*://)[^/\s:@]+:[^/\s@]+@")
            .expect("URL 凭据脱敏正则必须可编译（模式为常量）")
    })
}

/// Slack incoming-webhook：凭据在 path 里而非 userinfo。
///
/// 路径字符类排除 `[`，使已脱敏的 `services/[redacted]` 不会二次匹配。
fn slack_webhook_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?P<pre>\bhooks\.slack\.com/services/)[A-Za-z0-9/_-]+")
            .expect("Slack webhook 脱敏正则必须可编译（模式为常量）")
    })
}

/// 当前用户的家目录字符串（`None` = 取不到 / 无意义）。
///
/// 用**纯字符串替换**而非正则：Windows 上家目录含 `\` 与盘符，
/// 写成正则既要转义又要考虑大小写，徒增出错面；替换则天然只影响
/// 恰好等于家目录的那一段。
fn home_dir_string() -> Option<String> {
    let home = dirs::home_dir()?;
    let text = home.to_string_lossy().to_string();
    if text.is_empty() || text == "/" || text == "\\" {
        None
    } else {
        Some(text)
    }
}

/// 脱敏任意文本：五类敏感形态 + 家目录归一。
///
/// # 五类覆盖
///
/// 1. `Bearer <token>`（含 `authorization` 头的裸值）
/// 2. `key=value` / `key: value` / `"key": "value"` 赋值（键名见 [`SECRET_KEY_PATTERN`]）
/// 3. 裸令牌前缀（`sk-` / `sk_tr_` / `xox?-` / `gh?_` / `github_pat_` / `AKIA` / `eyJ…` / `AIza`）
/// 4. URL 内嵌凭据（`scheme://user:pass@host`）与 Slack incoming-webhook path
/// 5. 家目录 → `[USER]`
///
/// # 幂等性
///
/// 对任意 `t`，`scrub_text(scrub_text(t)) == scrub_text(t)`。这不是锦上添花：
/// 诊断包可能被反复收集（先脱敏落盘、再打包），二次脱敏若产出
/// `[redacted]]`、`[USER][USER]` 之类，既污染产物又说明规则有副作用。
/// 幂等由两处构造保证：赋值分支排除 `[`、webhook path 分支排除 `[`。
///
/// # 顺序
///
/// authorization → 赋值 → bearer → 裸令牌 → URL → webhook → 家目录。
/// **authorization 必须先于赋值**：赋值会把 `Bearer` 当成值吃掉、毁掉
/// bearer 规则的锚点，令牌随之裸奔（详见 [`authorization_re`]）。
/// 赋值在裸令牌之前：`api_key=sk-xxxx` 会被整体吃成
/// `api_key=[redacted]`；反过来则先留下 `api_key=` 再被裸令牌规则处理，
/// 产物形态依赖规则顺序而不稳定。家目录放最后：它是纯字符串替换，
/// 放在最前会让后续正则看到被换掉的路径。
pub fn scrub_text(text: &str) -> String {
    let mut out = authorization_re()
        .replace_all(
            text,
            format!("${{pre}}${{scheme}}{REDACTED}").as_str(),
        )
        .into_owned();
    out = assignment_re()
        .replace_all(&out, format!("${{pre}}{REDACTED}").as_str())
        .into_owned();
    out = bearer_re()
        .replace_all(&out, format!("${{pre}}{REDACTED}").as_str())
        .into_owned();
    out = bare_token_re().replace_all(&out, REDACTED).into_owned();
    out = url_credential_re()
        .replace_all(&out, format!("${{pre}}{REDACTED}@").as_str())
        .into_owned();
    out = slack_webhook_re()
        .replace_all(&out, format!("${{pre}}{REDACTED}").as_str())
        .into_owned();
    if let Some(home) = home_dir_string() {
        out = out.replace(home.as_str(), USER_HOME_PLACEHOLDER);
        // JSON / Rust `Debug` 会把路径里的 `\` 转义成 `\\`，而
        // `home_dir_string()` 给的是单反斜杠形态 —— 只替换原形态会漏掉
        // `"config_dir": "C:\\Users\\<用户名>"` 这类转义路径，用户名随之外泄
        // （实测泄漏）。正斜杠形态也一并归一，日志里两种写法都常见。
        let escaped = home.replace('\\', "\\\\");
        if escaped != home {
            out = out.replace(escaped.as_str(), USER_HOME_PLACEHOLDER);
        }
        let forward = home.replace('\\', "/");
        if forward != home {
            out = out.replace(forward.as_str(), USER_HOME_PLACEHOLDER);
        }
    }
    out
}

/// 产物级硬排除：这些名字**永不**进入任何对外产物。
///
/// 与 Python 侧 `bundle.py::_is_excluded` 同名单同理由，另加
/// `skill_block.jsonl`——安全审计台账里存的是**用户输入原文片段**，
/// 它不是故障证据，把它塞进 bug 报告等于把用户的口述隐私一起外发。
///
/// 这是写入路径上的最后一道闸：即便调用方 future 性地漏掉一处判断，
/// 收口处的 [`crate::observability::bundle`] 仍会拒写。
pub fn is_excluded_artifact(entry_name: &str) -> bool {
    let base = entry_name.rsplit('/').next().unwrap_or(entry_name);
    base == "desktop-credential.json"
        || base == ".env"
        || base.starts_with(".env.")
        || base.ends_with("-raw.jsonl")
        || base == "skill_block.jsonl"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 明显是假的凭据串：`FAKEfakeFAKE` 段的意图是让任何扫描器都不会
    /// 把它误当成真密钥而报警。
    const FAKE_SK: &str = "sk-FAKEfakeFAKEfakeFAKE1234";
    const FAKE_BEARER: &str = "Bearer abcdefghijklmnopqrstuvwx";

    #[test]
    fn masks_key_value_assignment() {
        let out = scrub_text(&format!("api_key={FAKE_SK}"));
        assert!(!out.contains(FAKE_SK), "赋值形态必须脱敏: {out}");
        assert_eq!(out, format!("api_key={REDACTED}"));
    }

    #[test]
    fn masks_bare_provider_token_without_key_equals() {
        // provider 报错原文形态：没有 key= 结构，只有裸前缀
        let out = scrub_text(&format!("Incorrect API key {FAKE_SK} provided"));
        assert!(!out.contains(FAKE_SK), "裸令牌必须脱敏: {out}");
        assert!(out.contains("Incorrect API key"), "上下文须保留: {out}");
    }

    #[test]
    fn masks_bearer_and_keeps_scheme_word() {
        let out = scrub_text(FAKE_BEARER);
        assert!(!out.contains("abcdefghijklmnopqrstuvwx"), "{out}");
        assert_eq!(out, format!("Bearer {REDACTED}"));
    }

    #[test]
    fn masks_url_userinfo_but_keeps_host() {
        let out = scrub_text("gateway said https://alice:hunter2PLACEHOLDER@api.internal:8443/v1");
        assert!(!out.contains("hunter2PLACEHOLDER"), "{out}");
        assert!(
            out.contains("api.internal:8443/v1"),
            "host 必须保留（排障需要）: {out}"
        );
    }

    #[test]
    fn masks_slack_webhook_path() {
        let out = scrub_text("POST https://hooks.slack.com/services/T0000/B1111/XXXXfakeXXXX");
        assert!(!out.contains("XXXXfakeXXXX"), "{out}");
        assert!(out.contains("hooks.slack.com/services/"), "{out}");
    }

    #[test]
    fn normalizes_home_directory() {
        let Some(home) = home_dir_string() else {
            // 取不到家目录的环境不做断言（脱敏在该环境下退化为不替换）
            return;
        };
        let out = scrub_text(&format!("reading {home}/.marguerite/config.yaml"));
        assert!(!out.contains(home.as_str()), "家目录必须被替换: {out}");
        assert!(out.contains(USER_HOME_PLACEHOLDER), "{out}");
    }

    #[test]
    fn is_idempotent() {
        //幂等性是诊断包可反复收集的前提，见 scrub_text 文档
        let raw = format!(
            "api_key={FAKE_SK} token=ghp_FAKEfakeFAKEfakeFAKE1234 {FAKE_BEARER} \
             https://u:pFAKEfake@host/x\nIncorrect API key {FAKE_SK} provided"
        );
        let once = scrub_text(&raw);
        let twice = scrub_text(&once);
        assert_eq!(once, twice, "二次脱敏必须是无操作");
        assert!(!once.contains(FAKE_SK));
    }

    #[test]
    fn does_not_swallow_next_line_first_word() {
        // 回归：分隔符若用 `\s*`，`password:` 后的换行会被吃进分隔符，
        // 下一行首词（这里是 stopHere）就被当成密码抹掉。
        let out = scrub_text("password:\nstopHere is the next sentence");
        assert!(
            out.contains("stopHere"),
            "不得吞掉下一行首词: {out}"
        );
    }

    #[test]
    fn keeps_quoted_value_with_spaces_intact() {
        let out = scrub_text(r#"password: "correct horse battery""#);
        assert!(!out.contains("correct horse"), "{out}");
        assert_eq!(out, format!("password: {REDACTED}"));
    }

    #[test]
    fn no_leak_via_assignment_half_eat() {
        // 回归（实测泄漏）：赋值规则会把 `Bearer` 当成整个值吃掉，
        // 毁掉 bearer 规则的锚点，令牌随之裸奔。
        let token = "abcdefghijklmnopqrstuvwx";
        let out = scrub_text(&format!("Authorization: Bearer {token}"));
        assert!(!out.contains(token), "方案词后的令牌不得残留: {out}");
        assert!(out.contains("Bearer"), "方案词应保留供排障: {out}");

        // `Basic` 这类 bearer 规则不认的方案同样不能漏
        let basic = "dXNlcjpwYXNzd29yZA==";
        let out = scrub_text(&format!("authorization: Basic {basic}"));
        assert!(!out.contains(basic), "Basic 凭据不得残留: {out}");
    }

    #[test]
    fn redacts_escaped_and_forward_home_paths() {
        // 回归（实测泄漏）：JSON 把 `\` 转义成 `\\`，只替换单反斜杠形态
        // 会漏掉 `"config_dir": "C:\\Users\\<用户>"`，用户名随之外泄。
        let Some(home) = home_dir_string() else {
            return; // 取不到家目录的环境（如某些 CI）跳过
        };
        let leaf = std::path::Path::new(&home)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if leaf.is_empty() {
            return;
        }
        for shape in [
            home.clone(),                        // 原形态
            home.replace('\\', "\\\\"),          // JSON / Debug 转义
            home.replace('\\', "/"),             // 正斜杠
        ] {
            let out = scrub_text(&format!("path={shape}/sub/file"));
            assert!(
                !out.contains(&leaf),
                "家目录用户名不得残留（形态 {shape}）: {out}"
            );
            assert!(out.contains(USER_HOME_PLACEHOLDER), "{out}");
        }
        // 幂等：二次脱敏不得叠加占位符
        let once = scrub_text(&format!("path={home}"));
        assert_eq!(scrub_text(&once), once);
    }

    #[test]
    fn keeps_benign_identifiers_readable() {
        // 过度脱敏会让诊断报告失去价值：session_key / cache_key 不是密钥
        let out = scrub_text("session_key=abc123 cache_key=def456 trace_id=xyz");
        assert!(out.contains("session_key=abc123"), "{out}");
        assert!(out.contains("cache_key=def456"), "{out}");
        assert!(out.contains("trace_id=xyz"), "{out}");
    }

    #[test]
    fn leaves_ordinary_prose_alone() {
        // 长度下限 + 词元边界锚定：普通词不得被误判成令牌
        let out = scrub_text("the risk-free skill list is ready; eyJustSaying is not a jwt");
        assert_eq!(out, "the risk-free skill list is ready; eyJustSaying is not a jwt");
    }

    #[test]
    fn handles_multibyte_utf8_without_corruption() {
        let raw = format!("密码：中文内容 🚀 api_key={FAKE_SK} 结束");
        let out = scrub_text(&raw);
        assert!(out.contains("中文内容 🚀"), "多字节内容必须原样保留: {out}");
        assert!(!out.contains(FAKE_SK), "{out}");
    }

    #[test]
    fn handles_large_tail_without_degenerating() {
        // 覆盖 Python 版那条负向后顾断言所防的场景：MB 级base64 run。
        // Rust `regex` 保证线性时间，此处的作用是把「有人把模式改回
        // 需要回溯的形式」这类退化当场暴露出来（测试超时即失败），
        // 而不是让实机导出在生产日志上跑几小时。
        let mut raw = String::with_capacity(512 * 1024);
        for _ in 0..4096 {
            raw.push_str("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo=");
        }
        raw.push('\n');
        raw.push_str(&format!("api_key={FAKE_SK}\n"));
        let out = scrub_text(&raw);
        assert!(!out.contains(FAKE_SK), "大块文本尾部仍须脱敏");
        assert!(out.contains("QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo="));
    }

    #[test]
    fn exclusion_list_matches_python_side() {
        for name in [
            "logs/desktop-credential.json",
            "config/.env",
            "config/.env.production",
            "sessions/run-raw.jsonl",
            "audit/skill_block.jsonl",
        ] {
            assert!(is_excluded_artifact(name), "{name} 必须被硬排除");
        }
        for name in ["config/config.yaml", "logs/sidecar-61750.err.log", "manifest.json"] {
            assert!(!is_excluded_artifact(name), "{name} 不应被排除");
        }
    }
}