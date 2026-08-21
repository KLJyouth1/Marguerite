//! DSH 扩展的 6 个 Slot 实现（spec FR-A6 / T-1.7）。
//!
//! 提供 36 槽契约中以下槽位的 DSH 二创渲染器：
//! - `LeftSidebarFileTree`   → [`DshBetterSidebarSlot`]   强化文件树
//! - `RightAgentTasks`       → [`DshUsageSlot`]           7 天用量条形图
//! - `RightAgentStatus`      → [`DshPetWhaleSlot`]        鲸鱼娘桌宠精灵
//! - `BottomOutput`          → [`DshMiniGamesSlot`]       锚点小游戏入口
//! - `RightAgentPlanner`     → [`DshGenUiSlot`]           GenUI 示例卡片
//! - `RightAgentMemory`      → [`DshMarketSlot`]          插件市场列表
//!
//! 本模块不修改 [`SlotId`] 枚举或 [`Slot`] trait 定义本身，只新增独立
//! struct 作为实现。单测使用 `TestBackend` 40×15 缓冲区，断言渲染内容含
//! 标识串即可。

#[cfg(test)]
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
#[cfg(test)]
use ratatui::Terminal;

use crate::tui::workbench::slot::{Slot, SlotId};
use crate::tui::workbench::state::WorkbenchState;
use crate::workbench::pet::PetMood;
use crate::workbench::pet_v2::{ascii_sprite, PetStateV2};

// ───────────────────────────── 1. DshBetterSidebarSlot ─────────────────────────────

/// 强化版侧边栏（`LeftSidebarFileTree`）。
#[derive(Debug, Default)]
pub struct DshBetterSidebarSlot;

impl Slot for DshBetterSidebarSlot {
    fn id(&self) -> SlotId {
        SlotId::LeftSidebarFileTree
    }

    fn render(&self, frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
        let title = Line::from(Span::styled(
            "[DSH Better Sidebar]",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        let lines: Vec<Line> = vec![
            title,
            Line::from("marguerite/"),
            Line::from("  ├── core/"),
            Line::from("  │   ├── src/"),
            Line::from("  │   │   └── lib.rs"),
            Line::from("  │   └── Cargo.toml"),
            Line::from("  ├── cli/"),
            Line::from("  │   └── src/"),
            Line::from("  └── README.md"),
            Line::from(""),
            Line::from(""),
            Line::from(""),
            Line::from(""),
            Line::from("[ Browser ][ Editor ]"),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 2. DshUsageSlot ─────────────────────────────

/// 每日用量面板（`RightAgentTasks`）。
#[derive(Debug, Default)]
pub struct DshUsageSlot;

fn bar_line(day: &str, pct: u8) -> String {
    let filled = (pct as usize).min(100) / 10;
    let empty = 10usize.saturating_sub(filled);
    format!(
        "{day} {}{} {pct:>3}%",
        "█".repeat(filled),
        "░".repeat(empty)
    )
}

impl Slot for DshUsageSlot {
    fn id(&self) -> SlotId {
        SlotId::RightAgentTasks
    }

    fn render(&self, frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
        let title = Line::from(vec![
            Span::styled(
                "Daily Usage ",
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            ),
            Span::styled("(TOKEN)", Style::default().fg(Color::Yellow)),
        ]);
        // 注意：40x15 TestBackend 下，块边框占 2（内 13 行），必须紧凑。
        let lines: Vec<Line> = vec![
            title,
            Line::from(bar_line("Mon", 42)),
            Line::from(bar_line("Tue", 65)),
            Line::from(bar_line("Wed", 30)),
            Line::from(bar_line("Thu", 80)),
            Line::from(bar_line("Fri", 55)),
            Line::from(bar_line("Sat", 20)),
            Line::from(bar_line("Sun", 10)),
            Line::from(""),
            Line::from("Total: 12,345 TOKENS"),
            Line::from("Monthly: -.--"),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 3. DshPetWhaleSlot ─────────────────────────────

/// 鲸鱼娘桌宠精灵面板（`RightAgentStatus`）。
#[derive(Debug, Default)]
pub struct DshPetWhaleSlot;

impl DshPetWhaleSlot {
    /// 从 state 里的共享 PetV2 构造状态行（affinity / state / energy）。
    /// 回退链：PetV2 → PetState → default (idle, energy=80, affinity=0)。
    fn status_line(state: &WorkbenchState) -> String {
        if let Some(pet) = state.pet_v2.as_ref() {
            let mood = pet_state_v2_to_label(pet.state);
            format!(
                "Mood: {mood} / affinity: {} / pets: {} / feeds: {} / energy: {}",
                pet.affinity, pet.pets, pet.feeds, pet.energy
            )
        } else {
            // fallback：沿用旧版 PetState（若 WorkbenchState 未来嵌入该字段），否则 idle/80
            let (m, e) = (PetMood::Idle, 80);
            format!("Mood: {} / affinity: 0 / energy: {}", pet_mood_label(m), e)
        }
    }

    /// 优先 PetV2::ascii_sprite 输出 6 帧精灵（每条 32 列宽，由 pet_v2 模块保证），
    /// 缺失时回退原静态 6 行（向后兼容）。
    fn sprite_lines(state: &WorkbenchState) -> Vec<Line<'static>> {
        if let Some(pet) = state.pet_v2.as_ref() {
            ascii_sprite(pet.state)
                .into_iter()
                .map(|l| Line::from(l.to_string()))
                .collect()
        } else {
            vec![
                Line::from("      .---.       "),
                Line::from("     /     \\      "),
                Line::from("    |(o) (o)|     "),
                Line::from("    \\   ~   /     "),
                Line::from("     \\_____/      "),
                Line::from("      /| |\\       "),
            ]
        }
    }
}

fn pet_state_v2_to_label(s: PetStateV2) -> &'static str {
    match s {
        PetStateV2::Idle => "idle",
        PetStateV2::Thinking => "thinking",
        PetStateV2::Running => "running",
        PetStateV2::Sleeping => "sleeping",
        PetStateV2::Eating => "eating",
        PetStateV2::Petted => "petted",
    }
}

fn pet_mood_label(m: PetMood) -> &'static str {
    match m {
        PetMood::Idle => "idle",
        PetMood::Happy => "happy",
        PetMood::Sleepy => "sleepy",
        PetMood::Working => "working",
    }
}

impl Slot for DshPetWhaleSlot {
    fn id(&self) -> SlotId {
        SlotId::RightAgentStatus
    }

    fn render(&self, frame: &mut Frame, area: Rect, state: &WorkbenchState) {
        // Review Advisory-B 落地：不再硬编码 `None`；直接从 WorkbenchState.pet_v2 读取
        // PetV2 共享状态（affinity / pets / feeds / energy / 6 帧 state）。
        // 当 pet_v2 = None（罕见）时，status_line + sprite_lines 内部已提供向后兼容 fallback。
        let status_line = Self::status_line(state);
        let sprite_lines = Self::sprite_lines(state);

        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "🐋 Whale (DSH)",
            Style::default()
                .fg(Color::Rgb(0, 180, 220))
                .add_modifier(Modifier::BOLD),
        ))];
        lines.extend(sprite_lines);
        lines.push(Line::from(""));
        lines.push(Line::from(status_line));
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(0, 130, 180)));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 4. DshMiniGamesSlot ─────────────────────────────

/// 锚点小游戏入口（`BottomOutput`）。
#[derive(Debug, Default)]
pub struct DshMiniGamesSlot;

impl Slot for DshMiniGamesSlot {
    fn id(&self) -> SlotId {
        SlotId::BottomOutput
    }

    fn render(&self, frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
        let lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "Mini Games (DSH)",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("  2048           ↑↓←→ 方向键"),
            Line::from("  TicTacToe      小键盘 1-9"),
            Line::from("  Snake          WASD"),
            Line::from(""),
            Line::from("  （输入对应按键进入游戏）"),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 5. DshGenUiSlot ─────────────────────────────

/// GenUI 示例卡片（`RightAgentPlanner`）。
#[derive(Debug, Default)]
pub struct DshGenUiSlot;

impl Slot for DshGenUiSlot {
    fn id(&self) -> SlotId {
        SlotId::RightAgentPlanner
    }

    fn render(&self, frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
        let lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "GenUI (DSH)",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(" ┌─DSH GenUI Demo────────────┐ "),
            Line::from(" │ 这是一个示例卡片，由       │ "),
            Line::from(" │ GenUI 渲染器绘制。         │ "),
            Line::from(" │                           │ "),
            Line::from(" │   [ A ]   [ B ]   [ C ]   │ "),
            Line::from(" └───────────────────────────┘ "),
        ];
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 6. DshMarketSlot ─────────────────────────────

/// 插件市场列表（`RightAgentMemory`）。
#[derive(Debug, Default)]
pub struct DshMarketSlot;

/// 演示用插件清单（8 份），B 阶段替换为读 manifests 常量。
const MARKET_PLUGINS: &[(&str, &str)] = &[
    ("dsh-harness-core", "v0.1.0"),
    ("dsh-harness-hooks", "v0.1.2"),
    ("dsh-genui-core", "v0.2.0"),
    ("dsh-deep-whale", "v0.3.1"),
    ("dsh-pet-games", "v0.0.4"),
    ("dsh-usage-meter", "v0.1.1"),
    ("dsh-sidepanel", "v0.0.9"),
    ("dsh-router", "v0.2.0"),
];

impl Slot for DshMarketSlot {
    fn id(&self) -> SlotId {
        SlotId::RightAgentMemory
    }

    fn render(&self, frame: &mut Frame, area: Rect, _state: &WorkbenchState) {
        let mut lines: Vec<Line> = vec![Line::from(Span::styled(
            "Market (DSH)",
            Style::default()
                .fg(Color::Rgb(255, 160, 0))
                .add_modifier(Modifier::BOLD),
        ))];
        for (name, ver) in MARKET_PLUGINS {
            lines.push(Line::from(format!("  - {name:<24} {ver}")));
        }
        lines.push(Line::from(""));
        lines.push(Line::from("/market install <name>"));

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(255, 160, 0)));
        let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
        frame.render_widget(para, area);
    }
}

// ───────────────────────────── 单测（6 条） ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::workbench::state::WorkbenchState;

    fn make_state() -> WorkbenchState {
        WorkbenchState::new("ut".into(), "m".into())
    }

    fn render_into(slot: &dyn Slot, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("Terminal 构造不应失败");
        let state = make_state();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, w, h);
                slot.render(f, area, &state);
            })
            .expect("draw 不应失败");
        // ratatui 0.28：Buffer 无 Display/ToString；按 cell.symbol 逐格拼接断言字符串。
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::with_capacity((area.width as usize + 1) * area.height as usize);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn dsh_better_sidebar_contains_identifier() {
        let buf = render_into(&DshBetterSidebarSlot::default(), 40, 15);
        assert!(
            buf.contains("[DSH Better Sidebar]"),
            "缓冲区应包含标题标识串，实际前 500 字节：{}",
            &buf.chars().take(500).collect::<String>()
        );
    }

    #[test]
    fn dsh_usage_contains_identifier() {
        let buf = render_into(&DshUsageSlot::default(), 40, 15);
        assert!(
            buf.contains("Daily Usage"),
            "缓冲区应包含 'Daily Usage'，实际：{buf}"
        );
        assert!(
            buf.contains("TOKEN"),
            "缓冲区应包含 'TOKEN'，实际：{buf}"
        );
        assert!(
            buf.contains("Total: 12,345 TOKENS"),
            "缓冲区应包含 TOKENS 合计行"
        );
    }

    #[test]
    fn dsh_pet_whale_contains_identifier() {
        let buf = render_into(&DshPetWhaleSlot::default(), 40, 15);
        assert!(
            buf.contains("Whale") || buf.contains("🐋"),
            "缓冲区应包含 Whale/🐋 标题标识"
        );
        assert!(
            buf.contains("Mood:"),
            "缓冲区应包含 Mood 行，实际：{buf}"
        );
        assert!(
            buf.contains("energy: 80") || buf.contains("energy: 100"),
            "缓冲区应显示能量值（默认 80 或其他合法值）"
        );
    }

    #[test]
    fn dsh_mini_games_contains_identifier() {
        let buf = render_into(&DshMiniGamesSlot::default(), 40, 15);
        assert!(
            buf.contains("Mini Games (DSH)"),
            "缓冲区应包含标题标识串"
        );
        assert!(buf.contains("2048"), "应列出 2048");
        assert!(buf.contains("TicTacToe"), "应列出 TicTacToe");
        assert!(buf.contains("Snake"), "应列出 Snake");
    }

    #[test]
    fn dsh_genui_contains_identifier() {
        let buf = render_into(&DshGenUiSlot::default(), 40, 15);
        assert!(
            buf.contains("GenUI (DSH)"),
            "缓冲区应包含标题标识串"
        );
        assert!(buf.contains("[ A ]"), "应包含按钮 [ A ]");
        assert!(buf.contains("[ B ]"), "应包含按钮 [ B ]");
        assert!(buf.contains("[ C ]"), "应包含按钮 [ C ]");
    }

    #[test]
    fn dsh_market_contains_identifier() {
        let buf = render_into(&DshMarketSlot::default(), 40, 15);
        assert!(
            buf.contains("Market (DSH)"),
            "缓冲区应包含标题标识串"
        );
        assert!(
            buf.contains("dsh-harness-core"),
            "应列出 dsh-harness-core 插件"
        );
        assert!(
            buf.contains("dsh-router"),
            "应列出 dsh-router 插件"
        );
        assert!(
            buf.contains("/market install"),
            "底部应给出 install 提示"
        );
    }
}
