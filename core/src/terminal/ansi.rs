//! 剥离 ANSI 转义序列（CSI / OSC / 其他 ESC 引导序列），保留可见文本。
//!
//! 算法：
//! - CSI（`ESC [`）：跳过 `[` 后直到终止字节（`0x40..=0x7E`，即 `@A-Z[\]^_`a-z`），
//!   参数字节（数字/`;`）低于 `@` 故不被误判为终止；
//! - OSC（`ESC ]`）：跳过直到 BEL（`0x07`）或 ST（`ESC \`）；
//! - 其它 ESC 序列：跳过紧随的一个字节。
//!   对 `ESC[m` / `ESC[31;1m` / `ESC]0;titleBEL` 等均安全。
//!
//! 独立成模块是为了让 `terminal::backend`（子模块）和 `terminal::TerminalSession`
//!（父模块里的结构体）都能引用同一实现，避免跨模块循环依赖。
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1B}' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next(); // 消费 '['
                for n in chars.by_ref() {
                    if ('@'..='~').contains(&n) {
                        break; // 终止字节（如 m）
                    }
                }
            }
            Some(']') => {
                chars.next(); // 消费 ']'
                for n in chars.by_ref() {
                    if n == '\u{07}' || n == '\u{1B}' {
                        break; // BEL 或 ST 起始
                    }
                }
            }
            Some(_) | None => {
                chars.next(); // 消费紧随的一个字节（若有）
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_csi_color() {
        assert_eq!(strip_ansi("\u{1B}[31mred\u{1B}[0m"), "red");
    }

    #[test]
    fn removes_multi_param() {
        assert_eq!(
            strip_ansi("\u{1B}[1;32;40mhello\u{1B}[0m world"),
            "hello world"
        );
    }

    #[test]
    fn keeps_plain_text() {
        assert_eq!(strip_ansi("no escapes here"), "no escapes here");
    }

    #[test]
    fn handles_os_title() {
        let input = "before\u{1B}]0;my title\u{07}after";
        assert_eq!(strip_ansi(input), "beforeafter");
    }

    #[test]
    fn handles_lone_esc() {
        assert_eq!(strip_ansi("ab\u{1B}"), "ab");
    }
}
