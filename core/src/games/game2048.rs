//! 2048 数字合成游戏引擎 — 纯逻辑、确定性、可序列化。
//!
//! 落地 \`dsh.games.2048\` 能力：4×4 棋盘、方向键滑动、合并（每格每步至多合并一次）、计分、
//! 随机生成新块（2/4）、无空位且无可合并时结束。渲染层只消费本引擎状态；规则由测试全量覆盖。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone)]
pub struct Game2048 {
    grid: [[u32; 4]; 4],
    score: u32,
    rng: u64,
}

impl Game2048 {
    pub fn new(seed: u64) -> Self {
        let mut g = Self {
            grid: [[0; 4]; 4],
            score: 0,
            rng: seed,
        };
        g.spawn();
        g.spawn();
        g
    }

    #[cfg(test)]
    pub fn from_cells(grid: [[u32; 4]; 4]) -> Self {
        Self {
            grid,
            score: 0,
            rng: 0,
        }
    }

    pub fn get(&self, row: usize, col: usize) -> u32 {
        self.grid[row][col]
    }
    pub fn score(&self) -> u32 {
        self.score
    }

    fn next_rng(&mut self) -> u64 {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.rng
    }

    pub fn spawn(&mut self) {
        let mut empties: Vec<(usize, usize)> = Vec::new();
        for r in 0..4 {
            for c in 0..4 {
                if self.grid[r][c] == 0 {
                    empties.push((r, c));
                }
            }
        }
        if empties.is_empty() {
            return;
        }
        let idx = (self.next_rng() % empties.len() as u64) as usize;
        let (r, c) = empties[idx];
        let val = if self.next_rng().is_multiple_of(10) {
            4
        } else {
            2
        };
        self.grid[r][c] = val;
    }

    fn slide_line_left(line: [u32; 4]) -> ([u32; 4], u32, bool) {
        let arr: Vec<u32> = line.iter().copied().filter(|&v| v != 0).collect();
        let mut out: Vec<u32> = Vec::with_capacity(4);
        let mut score = 0u32;
        let mut i = 0;
        while i < arr.len() {
            if i + 1 < arr.len() && arr[i] == arr[i + 1] {
                let merged = arr[i] * 2;
                out.push(merged);
                score += merged;
                i += 2;
            } else {
                out.push(arr[i]);
                i += 1;
            }
        }
        while out.len() < 4 {
            out.push(0);
        }
        let mut res = [0u32; 4];
        for (k, v) in out.iter().enumerate() {
            res[k] = *v;
        }
        let changed = res != line;
        (res, score, changed)
    }

    fn slide_line_right(line: [u32; 4]) -> ([u32; 4], u32, bool) {
        let mut rev = [0u32; 4];
        for k in 0..4 {
            rev[k] = line[3 - k];
        }
        let (r, score, changed) = Self::slide_line_left(rev);
        let mut res = [0u32; 4];
        for k in 0..4 {
            res[k] = r[3 - k];
        }
        (res, score, changed)
    }

    pub fn slide(&mut self, dir: Direction) -> bool {
        let mut changed = false;
        let mut score_add = 0u32;
        for i in 0..4 {
            let (nr, s, c) = match dir {
                Direction::Left => Self::slide_line_left(self.grid[i]),
                Direction::Right => Self::slide_line_right(self.grid[i]),
                Direction::Up | Direction::Down => {
                    let col: [u32; 4] = [
                        self.grid[0][i],
                        self.grid[1][i],
                        self.grid[2][i],
                        self.grid[3][i],
                    ];
                    let (nr, s, c) = if dir == Direction::Up {
                        Self::slide_line_left(col)
                    } else {
                        Self::slide_line_right(col)
                    };
                    (nr, s, c)
                }
            };
            match dir {
                Direction::Left | Direction::Right => self.grid[i] = nr,
                Direction::Up | Direction::Down => {
                    for (row, v) in nr.iter().enumerate() {
                        self.grid[row][i] = *v;
                    }
                }
            }
            score_add += s;
            changed |= c;
        }
        self.score += score_add;
        changed
    }

    pub fn step(&mut self, dir: Direction) -> bool {
        let moved = self.slide(dir);
        if moved {
            self.spawn();
        }
        moved
    }

    pub fn game_over(&self) -> bool {
        for r in 0..4 {
            for c in 0..4 {
                if self.grid[r][c] == 0 {
                    return false;
                }
                if r + 1 < 4 && self.grid[r][c] == self.grid[r + 1][c] {
                    return false;
                }
                if c + 1 < 4 && self.grid[r][c] == self.grid[r][c + 1] {
                    return false;
                }
            }
        }
        true
    }

    pub fn can_move(&self, dir: Direction) -> bool {
        let mut probe = self.clone();
        probe.slide(dir)
    }

    pub fn reset(&mut self) {
        self.grid = [[0; 4]; 4];
        self.score = 0;
        self.spawn();
        self.spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slide_left_merges_pair() {
        let mut g = Game2048::from_cells([[2, 2, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert!(g.slide(Direction::Left));
        assert_eq!(g.get(0, 0), 4);
        for c in 1..4 {
            assert_eq!(g.get(0, c), 0);
        }
        assert_eq!(g.score(), 4);
    }

    #[test]
    fn slide_right_pushes_to_right_edge() {
        let mut g = Game2048::from_cells([[0, 0, 2, 2], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert!(g.slide(Direction::Right));
        assert_eq!(g.get(0, 3), 4);
        assert_eq!(g.get(0, 2), 0);
        assert_eq!(g.score(), 4);
    }

    #[test]
    fn each_tile_merges_once_per_step() {
        let mut g = Game2048::from_cells([[2, 2, 2, 2], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert!(g.slide(Direction::Left));
        assert_eq!((g.get(0, 0), g.get(0, 1)), (4, 4));
        assert_eq!(g.score(), 8);
    }

    #[test]
    fn slide_up_merges_column() {
        let mut g = Game2048::from_cells([[2, 0, 0, 0], [2, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert!(g.slide(Direction::Up));
        assert_eq!(g.get(0, 0), 4);
        assert_eq!(g.score(), 4);
    }

    #[test]
    fn slide_with_no_change_returns_false() {
        let mut g = Game2048::from_cells([[2, 4, 8, 16], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]]);
        assert!(!g.slide(Direction::Left));
        assert_eq!(g.score(), 0);
    }

    #[test]
    fn game_over_when_full_and_no_merge() {
        let g = Game2048::from_cells([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 2]]);
        assert!(g.game_over());
    }

    #[test]
    fn non_game_over_when_a_merge_available() {
        let g = Game2048::from_cells([[2, 4, 2, 4], [4, 2, 4, 2], [2, 4, 2, 4], [4, 2, 4, 4]]);
        assert!(!g.game_over());
    }

    #[test]
    fn new_spawns_exactly_two_tiles() {
        let g = Game2048::new(7);
        let count = (0..4)
            .flat_map(|r| (0..4).map(move |c| (r, c)))
            .filter(|&(r, c)| g.get(r, c) != 0)
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn deterministic_spawn_for_same_seed() {
        let a = Game2048::new(42);
        let b = Game2048::new(42);
        let mut ga = Vec::new();
        for r in 0..4 {
            for c in 0..4 {
                ga.push(a.get(r, c));
            }
        }
        let mut gb = Vec::new();
        for r in 0..4 {
            for c in 0..4 {
                gb.push(b.get(r, c));
            }
        }
        assert_eq!(ga, gb);
    }

    #[test]
    fn reset_clears_and_repopulates() {
        let mut g = Game2048::new(1);
        g.slide(Direction::Left);
        g.reset();
        assert_eq!(g.score(), 0);
        let count = (0..4)
            .flat_map(|r| (0..4).map(move |c| (r, c)))
            .filter(|&(r, c)| g.get(r, c) != 0)
            .count();
        assert_eq!(count, 2);
    }
}
