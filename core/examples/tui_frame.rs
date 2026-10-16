//! Render one real TUI frame to ANSI on stdout — used to produce the terminal
//! screenshot in the docs.
//!
//! ```text
//! cargo run -p marguerite-core --example tui_frame > frame.ansi
//! ```
//!
//! Why an example and not a terminal capture: this goes through ratatui's
//! `TestBackend`, the same path `core/src/tui/**` tests use, so it needs no
//! pty, no real terminal, and no interactive session — which is what makes the
//! screenshot reproducible on any machine (including CI).
//!
//! Each cell is re-emitted with its own fg/bg/modifier as SGR sequences, so the
//! output can be rendered by any terminal-to-image converter.
//!
//! ⚠️ The frame is built from `Config::default()`; it never reads your real
//! state directory, and the stream is written to stdout only — no files.

use marguerite_core::persistence::config::Config;
use marguerite_core::tui::workbench::WorkbenchApp;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 38;

fn fg_sgr(c: Color) -> String {
    match c {
        Color::Reset => "39".into(),
        Color::Black => "30".into(),
        Color::Red => "31".into(),
        Color::Green => "32".into(),
        Color::Yellow => "33".into(),
        Color::Blue => "34".into(),
        Color::Magenta => "35".into(),
        Color::Cyan => "36".into(),
        Color::Gray => "37".into(),
        Color::DarkGray => "90".into(),
        Color::LightRed => "91".into(),
        Color::LightGreen => "92".into(),
        Color::LightYellow => "93".into(),
        Color::LightBlue => "94".into(),
        Color::LightMagenta => "95".into(),
        Color::LightCyan => "96".into(),
        Color::White => "97".into(),
        Color::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        Color::Indexed(i) => format!("38;5;{i}"),
    }
}

fn bg_sgr(c: Color) -> String {
    match c {
        Color::Reset => "49".into(),
        Color::Black => "40".into(),
        Color::Red => "41".into(),
        Color::Green => "42".into(),
        Color::Yellow => "43".into(),
        Color::Blue => "44".into(),
        Color::Magenta => "45".into(),
        Color::Cyan => "46".into(),
        Color::Gray => "47".into(),
        Color::DarkGray => "100".into(),
        Color::LightRed => "101".into(),
        Color::LightGreen => "102".into(),
        Color::LightYellow => "103".into(),
        Color::LightBlue => "104".into(),
        Color::LightMagenta => "105".into(),
        Color::LightCyan => "106".into(),
        Color::White => "107".into(),
        Color::Rgb(r, g, b) => format!("48;2;{r};{g};{b}"),
        Color::Indexed(i) => format!("48;5;{i}"),
    }
}

fn mod_sgr(m: Modifier) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::SLOW_BLINK, "5"),
        (Modifier::RAPID_BLINK, "6"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            parts.push(code);
        }
    }
    parts.join(";")
}

fn main() {
    let app = WorkbenchApp::from_config(&Config::default()).expect("default workbench config");
    let mut term = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("test backend");
    term.draw(|f| app.render(f)).expect("render frame");

    let buf = term.backend().buffer();
    let w = buf.area.width as usize;
    let h = buf.area.height as usize;
    let content = buf.content();

    let mut out = String::new();
    for y in 0..h {
        let mut last: Option<(Color, Color, Modifier)> = None;
        // Advance by the glyph's DISPLAY width, not by one cell per iteration.
        // Rendering text in ratatui blanks the continuation cell of a wide glyph
        // with a space (it does not set `skip`), but a real terminal advances two
        // columns for the glyph itself -- emitting that blank would shift every
        // CJK run one column to the right.
        let mut x = 0usize;
        while x < w {
            let cell = &content[y * w + x];
            let sym = cell.symbol();
            let width = sym.width().max(1);
            let style = (cell.fg, cell.bg, cell.modifier);
            if last != Some(style) {
                out.push_str("\u{1b}[");
                out.push_str(&fg_sgr(cell.fg));
                out.push(';');
                out.push_str(&bg_sgr(cell.bg));
                let mods = mod_sgr(cell.modifier);
                if !mods.is_empty() {
                    out.push(';');
                    out.push_str(&mods);
                }
                out.push('m');
                last = Some(style);
            }
            out.push_str(sym);
            x += width;
        }
        out.push_str("\u{1b}[0m\n");
    }
    print!("{out}");
}
