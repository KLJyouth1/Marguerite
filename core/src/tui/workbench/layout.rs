//! 五 region 布局引擎 + 折叠规则 + 响应式
//!
//! 验收（B1）：宽度 `>= 100` 全五栏；`80..=99` 折叠右栏；`< 80` 折叠双栏退化为单栏。

use ratatui::layout::{Constraint, Layout, Rect};

/// 工作台区域枚举（用于索引与调试）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchRegion {
    /// 左活动栏（图标列）
    Activity,
    /// 左侧边栏
    Sidebar,
    /// 中工作区
    Main,
    /// 右 AI 面板
    Rail,
    /// 底部状态栏
    Status,
}

/// 折叠状态（手动优先于响应式）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CollapseState {
    /// 侧边栏是否折叠
    pub sidebar: bool,
    /// 右栏是否折叠
    pub rail: bool,
}

/// 按宽度自适应折叠
pub fn compute_collapse(width: u16) -> CollapseState {
    if width >= 100 {
        CollapseState::default()
    } else if width >= 80 {
        CollapseState {
            sidebar: false,
            rail: true,
        }
    } else {
        CollapseState {
            sidebar: true,
            rail: true,
        }
    }
}

/// 布局矩形（折叠时对应 region 为 `None`）
#[derive(Debug, Clone, Copy)]
pub struct WorkbenchLayout {
    /// 活动栏（始终存在，宽 2，全高）
    pub activity: Rect,
    /// 顶栏（始终存在，高 1，位于活动栏右侧、内容区上方；承载 mode + 模型选择器同构显示）
    pub topbar: Rect,
    /// 侧边栏（折叠时 None）
    pub sidebar: Option<Rect>,
    /// 主区（始终存在）
    pub main: Rect,
    /// 右栏（折叠时 None）
    pub rail: Option<Rect>,
    /// 状态栏（始终存在，全宽底栏）
    pub status: Rect,
}

/// 计算五 region 矩形（活动栏全高 + 顶栏横跨内容区上方 + 内容区三栏）
pub fn split(area: Rect, collapse: &CollapseState) -> WorkbenchLayout {
    // 垂直：上=主体行，下=状态栏（高 1）
    let v = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let body = v[0];
    let status = v[1];

    // 主体水平：活动栏(2, 全高) + 内容区(Min)
    let bh = Layout::horizontal([Constraint::Length(2), Constraint::Min(1)]).split(body);
    let activity = bh[0];
    let content = bh[1];

    // 内容区垂直：顶栏(1) + 工作区(Min)
    let cv = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(content);
    let topbar = cv[0];
    let work = cv[1];

    // 工作区水平：侧边栏(22)? + 主区(Min40) + 右栏(30)?
    let mut h: Vec<Constraint> = Vec::new();
    if !collapse.sidebar {
        h.push(Constraint::Length(22)); // 侧边栏
    }
    h.push(Constraint::Min(40)); // 主区
    if !collapse.rail {
        h.push(Constraint::Length(34)); // 右栏（含 Deploy/Collab 两新增 tab）
    }
    let hr = Layout::horizontal(h).split(work);

    let mut idx = 0usize;
    let sidebar = if !collapse.sidebar {
        let r = hr[idx];
        idx += 1;
        Some(r)
    } else {
        None
    };
    let main = hr[idx];
    idx += 1;
    let rail = if !collapse.rail {
        let r = hr[idx];
        Some(r)
    } else {
        None
    };

    WorkbenchLayout {
        activity,
        topbar,
        sidebar,
        main,
        rail,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_collapse_wide_shows_all() {
        let c = compute_collapse(120);
        assert!(!c.sidebar && !c.rail);
    }

    #[test]
    fn test_compute_collapse_mid_folds_rail() {
        let c = compute_collapse(90);
        assert!(!c.sidebar && c.rail);
    }

    #[test]
    fn test_compute_collapse_narrow_folds_both() {
        let c = compute_collapse(70);
        assert!(c.sidebar && c.rail);
    }

    #[test]
    fn test_split_full_five_regions() {
        let area = Rect::new(0, 0, 120, 40);
        let lay = split(area, &CollapseState::default());
        assert!(lay.sidebar.is_some());
        assert!(lay.rail.is_some());
        // 活动栏宽度应为 2
        assert_eq!(lay.activity.width, 2);
        // 状态栏全宽
        assert_eq!(lay.status.width, 120);
        assert_eq!(lay.status.height, 1);
    }

    #[test]
    fn test_split_narrow_collapses_both() {
        let area = Rect::new(0, 0, 70, 40);
        let lay = split(area, &compute_collapse(70));
        assert!(lay.sidebar.is_none());
        assert!(lay.rail.is_none());
        assert_eq!(lay.activity.width, 2);
        assert_eq!(lay.status.width, 70);
    }

    #[test]
    fn test_split_mid_collapses_rail_only() {
        let area = Rect::new(0, 0, 90, 40);
        let lay = split(area, &compute_collapse(90));
        assert!(lay.sidebar.is_some());
        assert!(lay.rail.is_none());
    }

    #[test]
    fn test_split_topbar_present_and_full_width_of_content() {
        let area = Rect::new(0, 0, 120, 40);
        let lay = split(area, &CollapseState::default());
        // 顶栏高 1，位于活动栏右侧、横跨内容区上方
        assert_eq!(lay.topbar.height, 1);
        assert_eq!(lay.topbar.y, 0);
        assert_eq!(lay.topbar.width, area.width - 2); // 减去活动栏宽度
                                                      // 主区在顶栏下方
        assert_eq!(lay.main.y, lay.topbar.y + 1);
    }
}
