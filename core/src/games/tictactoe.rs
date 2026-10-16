//! 井字棋（TicTacToe）游戏引擎 — 纯逻辑、无 IO、确定性。
//!
//! 落地 `dsh.games.tictactoe` 能力：3×3 棋盘、自动轮换落子、胜负/平局判定、悔棋一步。
//! TUI/Web 渲染层只消费本引擎状态，不重复实现规则；引擎规则由 tests 全量覆盖。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Player {
    X,
    O,
}

impl Player {
    pub fn other(self) -> Player {
        match self {
            Player::X => Player::O,
            Player::O => Player::X,
        }
    }
}

impl fmt::Display for Player {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Player::X => "X",
            Player::O => "O",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    XWin,
    OWin,
    Draw,
    Ongoing,
}

#[derive(Debug, Clone)]
pub struct TicTacToe {
    cells: [Option<Player>; 9],
    history: Vec<(usize, Player)>,
}

impl Default for TicTacToe {
    fn default() -> Self {
        Self::new()
    }
}

impl TicTacToe {
    pub fn new() -> Self {
        Self {
            cells: [None; 9],
            history: Vec::new(),
        }
    }

    pub fn is_valid_cell(cell: usize) -> bool {
        cell <= 8
    }

    pub fn turn(&self) -> Player {
        if self.history.len().is_multiple_of(2) {
            Player::X
        } else {
            Player::O
        }
    }

    pub fn place(&mut self, cell: usize) -> Result<Player, String> {
        if !Self::is_valid_cell(cell) {
            return Err(format!("格子越界: {cell}（应为 0..=8）"));
        }
        if self.is_over() {
            return Err("棋局已结束".to_string());
        }
        if self.cells[cell].is_some() {
            return Err(format!("格子 {cell} 已被占用"));
        }
        let player = self.turn();
        self.cells[cell] = Some(player);
        self.history.push((cell, player));
        Ok(player)
    }

    pub fn place_as(&mut self, cell: usize, player: Player) -> Result<(), String> {
        if player != self.turn() {
            return Err(format!("未轮到 {player}，应为 {}", self.turn()));
        }
        self.place(cell).map(|_| ())
    }

    pub fn get(&self, cell: usize) -> Option<Player> {
        self.cells.get(cell).copied().flatten()
    }

    pub fn winner(&self) -> Option<Player> {
        const LINES: [[usize; 3]; 8] = [
            [0, 1, 2],
            [3, 4, 5],
            [6, 7, 8],
            [0, 3, 6],
            [1, 4, 7],
            [2, 5, 8],
            [0, 4, 8],
            [2, 4, 6],
        ];
        for line in LINES {
            if let (Some(a), Some(b), Some(c)) = (
                self.cells[line[0]],
                self.cells[line[1]],
                self.cells[line[2]],
            ) {
                if a == b && b == c {
                    return Some(a);
                }
            }
        }
        None
    }

    pub fn outcome(&self) -> Outcome {
        if let Some(w) = self.winner() {
            return match w {
                Player::X => Outcome::XWin,
                Player::O => Outcome::OWin,
            };
        }
        if self.history.len() == 9 {
            return Outcome::Draw;
        }
        Outcome::Ongoing
    }

    pub fn is_draw(&self) -> bool {
        matches!(self.outcome(), Outcome::Draw)
    }
    pub fn is_over(&self) -> bool {
        !matches!(self.outcome(), Outcome::Ongoing)
    }

    pub fn undo(&mut self) -> bool {
        if let Some((cell, _)) = self.history.pop() {
            self.cells[cell] = None;
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.cells = [None; 9];
        self.history.clear();
    }
    pub fn history(&self) -> &[(usize, Player)] {
        &self.history
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_empty_and_x_turn() {
        let g = TicTacToe::new();
        assert_eq!(g.turn(), Player::X);
        assert_eq!(g.outcome(), Outcome::Ongoing);
        assert!(!g.is_over());
        for i in 0..9 {
            assert_eq!(g.get(i), None);
        }
    }

    #[test]
    fn x_wins_top_row() {
        let mut g = TicTacToe::new();
        g.place(0).unwrap();
        g.place(3).unwrap();
        g.place(1).unwrap();
        g.place(4).unwrap();
        g.place(2).unwrap();
        assert_eq!(g.outcome(), Outcome::XWin);
        assert_eq!(g.winner(), Some(Player::X));
        assert!(g.is_over());
    }

    #[test]
    fn o_wins_main_diagonal() {
        let mut g = TicTacToe::new();
        g.place(1).unwrap();
        g.place(0).unwrap();
        g.place(3).unwrap();
        g.place(4).unwrap();
        g.place(5).unwrap();
        g.place(8).unwrap();
        assert_eq!(g.outcome(), Outcome::OWin);
    }

    #[test]
    fn full_board_no_line_is_draw() {
        // 经典平局布局（X 先手，5 X + 4 O），无任何连线
        let moves = [
            (0, Player::X),
            (2, Player::O),
            (1, Player::X),
            (3, Player::O),
            (5, Player::X),
            (4, Player::O),
            (6, Player::X),
            (7, Player::O),
            (8, Player::X),
        ];
        let mut g = TicTacToe::new();
        for (cell, p) in moves {
            g.place_as(cell, p).unwrap();
        }
        assert_eq!(g.outcome(), Outcome::Draw);
        assert!(g.is_draw());
    }

    #[test]
    fn out_of_range_and_occupied_and_wrong_turn_rejected() {
        let mut g = TicTacToe::new();
        assert!(g.place(9).is_err());
        g.place(0).unwrap();
        assert!(g.place(0).is_err(), "占位应拒绝");
        assert!(g.place_as(1, Player::X).is_err(), "未轮到 X 应拒绝");
        assert!(g.place_as(1, Player::O).is_ok());
    }

    #[test]
    fn undo_restores_previous_state_and_turn() {
        let mut g = TicTacToe::new();
        g.place(0).unwrap();
        g.place(4).unwrap();
        assert_eq!(g.turn(), Player::X);
        assert!(g.undo()); // 撤 O
        assert_eq!(g.history().len(), 1);
        assert_eq!(g.get(4), None);
        assert_eq!(g.turn(), Player::O);
        assert!(g.undo()); // 撤 X
        assert_eq!(g.history().len(), 0);
        assert_eq!(g.get(0), None);
        assert_eq!(g.turn(), Player::X);
        assert!(!g.undo()); // 无可悔 -> false
    }

    #[test]
    fn reset_clears_board() {
        let mut g = TicTacToe::new();
        g.place(0).unwrap();
        g.place(1).unwrap();
        g.reset();
        assert!(g.history().is_empty());
        assert_eq!(g.outcome(), Outcome::Ongoing);
        assert_eq!(g.turn(), Player::X);
    }

    #[test]
    fn cannot_place_after_game_over() {
        let mut g = TicTacToe::new();
        g.place(0).unwrap();
        g.place(3).unwrap();
        g.place(1).unwrap();
        g.place(4).unwrap();
        g.place(2).unwrap();
        assert!(g.is_over());
        assert!(g.place(6).is_err(), "棋局结束后应拒绝落子");
    }
}
