//! 小游戏引擎（Game Hall）：纯逻辑、确定性、可序列化，供 TUI/Web 消费。

pub mod game2048;
pub mod snake;
pub mod tictactoe;

/// 玩家可见的游戏元信息。
#[derive(Debug, Clone, Copy)]
pub struct GameInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub controls: &'static str,
}

/// 已实装小游戏目录（单一事实源；TUI 槽位 / REPL `/games` / capabilities 均可消费）。
pub fn catalog() -> &'static [GameInfo] {
    &[
        GameInfo {
            id: "2048",
            name: "2048 数字合成",
            controls: "up/down/left/right",
        },
        GameInfo {
            id: "tictactoe",
            name: "井字棋 TicTacToe",
            controls: "0-8 落子",
        },
        GameInfo {
            id: "snake",
            name: "贪吃蛇 Snake",
            controls: "up/down/left/right",
        },
    ]
}

/// 执行一串动作并返回渲染后的棋盘 + 结果（游戏可玩性入口，TUI/Web/脚本均可消费）。
pub fn run_game(game_id: &str, moves: &[&str]) -> Result<String, String> {
    match game_id {
        "tictactoe" => {
            let mut g = tictactoe::TicTacToe::new();
            for m in moves {
                let cell: usize = m.parse().map_err(|_| format!("非法落子: {m}"))?;
                g.place(cell)?;
            }
            let status = match g.outcome() {
                tictactoe::Outcome::XWin => "X 获胜",
                tictactoe::Outcome::OWin => "O 获胜",
                tictactoe::Outcome::Draw => "平局",
                tictactoe::Outcome::Ongoing => "进行中",
            };
            let mut s = String::new();
            for r in 0..3 {
                for c in 0..3 {
                    let cell = r * 3 + c;
                    if let Some(p) = g.get(cell) {
                        let ps = p.to_string();
                        s.push_str(&ps);
                    } else {
                        s.push('.');
                    }
                    if c < 2 {
                        s.push(' ');
                    }
                }
                s.push('\n');
            }
            Ok(format!("{s}状态: {status}"))
        }
        "2048" => {
            let mut g = game2048::Game2048::new(0);
            for m in moves {
                let dir = match *m {
                    "left" => game2048::Direction::Left,
                    "right" => game2048::Direction::Right,
                    "up" => game2048::Direction::Up,
                    "down" => game2048::Direction::Down,
                    _ => return Err(format!("非法方向: {m}")),
                };
                g.step(dir);
            }
            let mut s = String::new();
            for r in 0..4 {
                for c in 0..4 {
                    s.push_str(&format!("{:>4}", g.get(r, c)));
                }
                s.push('\n');
            }
            Ok(format!("{s}得分: {}", g.score()))
        }
        "snake" => {
            let mut g = snake::Snake::new(10, 10, 0);
            let mut out = String::new();
            for m in moves {
                let dir = match *m {
                    "up" => snake::Direction::Up,
                    "down" => snake::Direction::Down,
                    "left" => snake::Direction::Left,
                    "right" => snake::Direction::Right,
                    _ => return Err(format!("非法方向: {m}")),
                };
                g.set_direction(dir);
                out.push_str(if g.step() == snake::Outcome::Alive {
                    "存活\n"
                } else {
                    "死亡\n"
                });
            }
            Ok(format!("{out}score: {}", g.score()))
        }
        _ => Err(format!("未知游戏: {game_id}（可用 /games 查看目录）")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_the_three_implemented_games() {
        let games = catalog();
        assert_eq!(games.len(), 3);
        let ids: Vec<&str> = games.iter().map(|g| g.id).collect();
        assert_eq!(ids, vec!["2048", "tictactoe", "snake"]);
        for g in games {
            assert!(!g.name.is_empty());
            assert!(!g.controls.is_empty());
        }
    }

    #[test]
    fn run_game_tictactoe_x_wins() {
        let out = run_game("tictactoe", &["0", "3", "1", "4", "2"]).unwrap();
        assert!(out.contains("X 获胜"), "got: {out}");
        assert!(out.contains('X'));
    }

    #[test]
    fn run_game_snake_alive_and_moves() {
        let out = run_game("snake", &["up", "left"]).unwrap();
        assert!(out.contains("存活"), "got: {out}");
    }

    #[test]
    fn run_game_2048_slides_and_scores() {
        let out = run_game("2048", &["left", "up"]).unwrap();
        assert!(out.contains("得分"), "got: {out}");
    }

    #[test]
    fn run_game_unknown_id_errors() {
        assert!(run_game("none", &[]).is_err());
        assert!(run_game("tictactoe", &["x"]).is_err());
    }
}
