//! 贪吃蛇（Snake）游戏引擎 — 纯逻辑、确定性、可序列化。
//!
//! 落地 `dsh.games.snake` 能力：蛇身移动、转向（禁 180° 掉头）、食物增长、墙体/自身碰撞、计分。
//! 渲染层只消费本引擎状态；规则由测试全量覆盖。

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn opposite(self) -> Direction {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Alive,
    Dead,
}

#[derive(Debug, Clone)]
pub struct Snake {
    width: i32,
    height: i32,
    body: VecDeque<Point>,
    dir: Direction,
    food: Point,
    rng: u64,
}

impl Snake {
    pub fn new(width: i32, height: i32, seed: u64) -> Self {
        let cx = width / 2;
        let cy = height / 2;
        let mut body = VecDeque::new();
        body.push_back(Point { x: cx, y: cy });
        body.push_back(Point { x: cx - 1, y: cy });
        body.push_back(Point { x: cx - 2, y: cy });
        let mut s = Self {
            width,
            height,
            body,
            dir: Direction::Right,
            food: Point { x: 0, y: 0 },
            rng: seed,
        };
        s.spawn_food();
        s
    }

    pub fn head(&self) -> Point {
        *self.body.front().expect("snake 身体非空")
    }
    pub fn direction(&self) -> Direction {
        self.dir
    }
    pub fn len(&self) -> usize {
        self.body.len()
    }
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }
    pub fn score(&self) -> usize {
        self.body.len() - 3
    }
    pub fn food(&self) -> Point {
        self.food
    }
    pub fn body(&self) -> &VecDeque<Point> {
        &self.body
    }

    fn next_rng(&mut self) -> u64 {
        self.rng = self
            .rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.rng
    }

    fn spawn_food(&mut self) {
        loop {
            let x = (self.next_rng() % self.width as u64) as i32;
            let y = (self.next_rng() % self.height as u64) as i32;
            let p = Point { x, y };
            if !self.body.contains(&p) {
                self.food = p;
                return;
            }
        }
    }

    pub fn place_food(&mut self, x: i32, y: i32) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            let p = Point { x, y };
            if !self.body.contains(&p) {
                self.food = p;
            }
        }
    }

    pub fn set_direction(&mut self, d: Direction) {
        if d != self.dir.opposite() {
            self.dir = d;
        }
    }

    pub fn step(&mut self) -> Outcome {
        let h = self.head();
        let nh = match self.dir {
            Direction::Up => Point { x: h.x, y: h.y - 1 },
            Direction::Down => Point { x: h.x, y: h.y + 1 },
            Direction::Left => Point { x: h.x - 1, y: h.y },
            Direction::Right => Point { x: h.x + 1, y: h.y },
        };
        if nh.x < 0 || nh.y < 0 || nh.x >= self.width || nh.y >= self.height {
            return Outcome::Dead;
        }
        let ate = nh == self.food;
        self.body.push_front(nh);
        if ate {
            self.spawn_food();
        } else {
            self.body.pop_back();
        }
        let head = self.head();
        let mut rest = self.body.iter().skip(1);
        if rest.any(|p| *p == head) {
            return Outcome::Dead;
        }
        Outcome::Alive
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_head_center_len_3_right() {
        let s = Snake::new(10, 10, 1);
        assert_eq!(s.head(), Point { x: 5, y: 5 });
        assert_eq!(s.len(), 3);
        assert_eq!(s.direction(), Direction::Right);
        assert_eq!(s.score(), 0);
        assert!(s.food() != s.head());
    }

    #[test]
    fn move_advances_head_and_tail_follows() {
        let mut s = Snake::new(10, 10, 1);
        let h = s.head();
        assert_eq!(s.step(), Outcome::Alive);
        assert_eq!(s.head(), Point { x: h.x + 1, y: h.y });
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn eat_food_grows_and_scores() {
        let mut s = Snake::new(10, 10, 1);
        let h = s.head();
        s.place_food(h.x + 1, h.y);
        assert_eq!(s.step(), Outcome::Alive);
        assert_eq!(s.len(), 4);
        assert_eq!(s.score(), 1);
    }

    #[test]
    fn wall_collision_is_dead() {
        let mut s = Snake::new(5, 5, 1);
        s.place_food(0, 0);
        assert_eq!(s.step(), Outcome::Alive);
        assert_eq!(s.step(), Outcome::Alive);
        assert_eq!(s.step(), Outcome::Dead);
    }

    #[test]
    fn self_collision_is_dead() {
        let mut s = Snake::new(10, 20, 1);
        s.place_food(s.head().x + 1, s.head().y);
        s.step();
        s.place_food(s.head().x + 1, s.head().y);
        s.step();
        s.set_direction(Direction::Up);
        assert_eq!(s.step(), Outcome::Alive);
        s.set_direction(Direction::Left);
        assert_eq!(s.step(), Outcome::Alive);
        s.set_direction(Direction::Down);
        assert_eq!(s.step(), Outcome::Dead);
    }

    #[test]
    fn reverse_direction_is_blocked() {
        let mut s = Snake::new(10, 10, 1);
        s.set_direction(Direction::Left);
        assert_eq!(s.direction(), Direction::Right);
        s.set_direction(Direction::Up);
        assert_eq!(s.direction(), Direction::Up);
    }

    #[test]
    fn deterministic_for_same_seed() {
        let a = Snake::new(12, 12, 42);
        let b = Snake::new(12, 12, 42);
        assert_eq!(a.food(), b.food());
    }
}
