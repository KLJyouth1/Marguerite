//! 快捷键表 + 冲突检测（B8）

/// 快捷键作用域
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 全局
    Global,
    /// 聊天输入
    ChatInput,
    /// 右栏
    Rail,
    /// 侧边栏
    Sidebar,
    /// leader 和弦（Ctrl+X 后）
    Leader,
}

/// 快捷键动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// 折叠侧边栏
    ToggleSidebar,
    /// 折叠右栏
    ToggleRail,
    /// 主区 Chat
    MainChat,
    /// 主区 Code
    MainCode,
    /// 主区 Terminal
    MainTerminal,
    /// 聚焦右栏
    FocusRail,
    /// 新会话
    NewSession,
    /// 会话列表
    SessionList,
    /// 退出
    Quit,
    /// 帮助
    ShowHelp,
    /// 命令面板
    OpenPalette,
    /// 全屏查看
    Fullscreen,
    /// 压缩上下文
    Compact,
    /// 历史上翻（召回更早命令）
    HistoryPrev,
    /// 历史下翻（召回更新命令）
    HistoryNext,
    /// 反向搜索历史（Ctrl+R）
    HistorySearch,
}

/// 快捷键绑定
#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    /// 按键描述
    pub keys: &'static str,
    /// 动作
    pub action: ShortcutAction,
    /// 作用域
    pub scope: Scope,
}

/// 全部快捷键（含 Ctrl+X leader 和弦）
pub fn builtin_shortcuts() -> Vec<Shortcut> {
    use Scope::*;
    use ShortcutAction::*;
    vec![
        Shortcut {
            keys: "Ctrl+B",
            action: ToggleSidebar,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+Shift+B",
            action: ToggleRail,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+1",
            action: MainChat,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+2",
            action: MainCode,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+3",
            action: MainTerminal,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+U",
            action: FocusRail,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+N",
            action: NewSession,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+L",
            action: SessionList,
            scope: Global,
        },
        Shortcut {
            keys: "Ctrl+C",
            action: Quit,
            scope: Global,
        },
        Shortcut {
            keys: "Alt+V",
            action: Fullscreen,
            scope: Global,
        },
        Shortcut {
            keys: "Esc",
            action: ShowHelp,
            scope: Global,
        },
        Shortcut {
            keys: "Tab",
            action: OpenPalette,
            scope: Global,
        },
        // leader 和弦（独立 scope，不与 Global 冲突）
        Shortcut {
            keys: "Ctrl+X c",
            action: Compact,
            scope: Leader,
        },
        Shortcut {
            keys: "Ctrl+X n",
            action: NewSession,
            scope: Leader,
        },
        Shortcut {
            keys: "Ctrl+X t",
            action: Fullscreen,
            scope: Leader,
        },
        Shortcut {
            keys: "Ctrl+X e",
            action: OpenPalette,
            scope: Leader,
        },
        Shortcut {
            keys: "Ctrl+X q",
            action: Quit,
            scope: Leader,
        },
        // 命令历史（Phase 4 / AC-08，ChatInput 作用域）
        Shortcut {
            keys: "Up",
            action: HistoryPrev,
            scope: ChatInput,
        },
        Shortcut {
            keys: "Down",
            action: HistoryNext,
            scope: ChatInput,
        },
        Shortcut {
            keys: "Ctrl+R",
            action: HistorySearch,
            scope: ChatInput,
        },
    ]
}

/// 启动期冲突检测：同 (keys, scope) 重复绑定 → 返回冲突对
pub fn detect_conflicts(table: &[Shortcut]) -> Vec<(&Shortcut, &Shortcut)> {
    let mut out = Vec::new();
    for i in 0..table.len() {
        for j in (i + 1)..table.len() {
            if table[i].keys == table[j].keys && table[i].scope == table[j].scope {
                out.push((&table[i], &table[j]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_shortcuts_no_conflict() {
        let table = builtin_shortcuts();
        let conflicts = detect_conflicts(&table);
        assert!(
            conflicts.is_empty(),
            "内置快捷键不应有冲突: {:?}",
            conflicts
        );
    }

    #[test]
    fn test_detect_conflicts_finds_duplicate() {
        let mut table = builtin_shortcuts();
        // 注入一个与 Global Ctrl+B 重复的绑定
        table.push(Shortcut {
            keys: "Ctrl+B",
            action: ShortcutAction::FocusRail,
            scope: Scope::Global,
        });
        let conflicts = detect_conflicts(&table);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0.keys, "Ctrl+B");
    }

    #[test]
    fn test_leader_scope_does_not_conflict_with_global() {
        // leader 和弦与全局同名键不冲突：leader 是独立 scope，不与 Global 同键判冲突
        // （设计注：builtin_shortcuts 中 Leader 入口 Ctrl+X 为前缀和弦，不与 Global 单键绑定冲突）
        let table = vec![
            Shortcut {
                keys: "Ctrl+X c",
                action: ShortcutAction::Compact,
                scope: Scope::Leader,
            },
            Shortcut {
                keys: "Ctrl+X c",
                action: ShortcutAction::Compact,
                scope: Scope::Global,
            },
        ];
        assert_eq!(
            detect_conflicts(&table).len(),
            0,
            "leader 与 global 同键不应判冲突"
        );
    }
}
