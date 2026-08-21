//! 德州扑克游戏引擎（Game Hall / 赌 Token）
//!
//! 纯逻辑、无 IO、确定性（可注入种子 RNG）、全类型可 `serde` 序列化。
//! 覆盖：牌堆/发牌、盲注、四轮下注（fold/check/call/raise/all-in）、边池、摊牌比牌、
//! Token 经济。房间模型传输无关——跨机联机需 relay 服务（本切片落地本地多人对战 +
//! AI 座位，`PokerHub` 作为进程内房间管理器）。
//!
//! # 安全边界
//! - 所有金额以 `i64` 表示 Token，禁止负数下注；
//! - `apply_action` 做合法性校验（轮到谁、下注额是否合法、是否 all-in）；
//! - 房间状态可整体序列化给前端，但**前端渲染前必须过 `genui::validate`**（防注入）。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use serde::{Deserialize, Serialize};

// ───────────────────────────── 牌 ─────────────────────────────

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Suit {
    Spades = 0,
    Hearts = 1,
    Diamonds = 2,
    Clubs = 3,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Rank {
    Two = 0,
    Three = 1,
    Four = 2,
    Five = 3,
    Six = 4,
    Seven = 5,
    Eight = 6,
    Nine = 7,
    Ten = 8,
    Jack = 9,
    Queen = 10,
    King = 11,
    Ace = 12,
}

impl Rank {
    /// 展示用短名（字母/数字，非 emoji，符合 P0 图标红线）
    pub fn label(&self) -> &'static str {
        match self {
            Rank::Two => "2",
            Rank::Three => "3",
            Rank::Four => "4",
            Rank::Five => "5",
            Rank::Six => "6",
            Rank::Seven => "7",
            Rank::Eight => "8",
            Rank::Nine => "9",
            Rank::Ten => "T",
            Rank::Jack => "J",
            Rank::Queen => "Q",
            Rank::King => "K",
            Rank::Ace => "A",
        }
    }
}

impl Suit {
    /// 花色符号（字母，非 emoji）
    pub fn symbol(&self) -> &'static str {
        match self {
            Suit::Spades => "s",
            Suit::Hearts => "h",
            Suit::Diamonds => "d",
            Suit::Clubs => "c",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }
    /// 0..51 唯一编号
    pub fn id(&self) -> u8 {
        (self.rank as u8) * 4 + (self.suit as u8)
    }
    /// 展示字符串，如 "As"（黑桃 A）/"Td"（方块 T）
    pub fn label(&self) -> String {
        format!("{}{}", self.rank.label(), self.suit.symbol())
    }
}

/// 确定性线性同余 RNG（无外部依赖，单测可复现）
#[derive(Debug, Clone)]
pub struct LcgRng {
    state: u64,
}

impl LcgRng {
    pub fn new(seed: u64) -> Self {
        let s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
        Self { state: s }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
}

#[derive(Debug, Clone)]
pub struct Deck {
    cards: Vec<Card>,
}

impl Deck {
    pub fn standard() -> Self {
        let ranks = [
            Rank::Two,
            Rank::Three,
            Rank::Four,
            Rank::Five,
            Rank::Six,
            Rank::Seven,
            Rank::Eight,
            Rank::Nine,
            Rank::Ten,
            Rank::Jack,
            Rank::Queen,
            Rank::King,
            Rank::Ace,
        ];
        let mut cards = Vec::with_capacity(52);
        for suit in [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs] {
            for &rank in &ranks {
                cards.push(Card::new(rank, suit));
            }
        }
        Self { cards }
    }

    /// Fisher–Yates 洗牌（确定性，取决于 rng）
    pub fn shuffle(&mut self, rng: &mut LcgRng) {
        let n = self.cards.len();
        for i in (1..n).rev() {
            let j = rng.below((i as u64) + 1) as usize;
            self.cards.swap(i, j);
        }
    }

    pub fn deal(&mut self) -> Option<Card> {
        self.cards.pop()
    }

    pub fn len(&self) -> usize {
        self.cards.len()
    }
}

// ───────────────────────────── 牌型评估 ─────────────────────────────

/// 手牌值：`category` 8=同花顺 .. 0=高牌；`tie` 为降序的 5 个断点（0..12，Two..Ace）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandValue {
    pub category: u8,
    pub tie: [u8; 5],
}

impl PartialOrd for HandValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HandValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.category
            .cmp(&other.category)
            .then_with(|| self.tie.cmp(&other.tie))
    }
}

/// 找 5 连顺的最高张（0..12），处理 A2345 轮子（最高张为 Five=3）
fn straight_high(counts: &[u8; 13]) -> Option<u8> {
    for top in (4..=12u8).rev() {
        let ok = (top - 4..=top).all(|r| counts[r as usize] > 0);
        if ok {
            return Some(top);
        }
    }
    // 轮子 A-2-3-4-5：A(12) 当 1 用
    if counts[12] > 0 && counts[0] > 0 && counts[1] > 0 && counts[2] > 0 && counts[3] > 0 {
        return Some(3); // Five
    }
    None
}

/// 评估 5~7 张牌，返回可比的手牌值
pub fn evaluate(cards: &[Card]) -> HandValue {
    let mut rank_count = [0u8; 13];
    let mut suit_count = [0u8; 4];
    for c in cards {
        rank_count[c.rank as usize] += 1;
        suit_count[c.suit as usize] += 1;
    }

    let flush_suit = (0..4usize).find(|&s| suit_count[s] >= 5);

    if let Some(suit) = flush_suit {
        let mut fc = [0u8; 13];
        for c in cards {
            if c.suit as usize == suit {
                fc[c.rank as usize] += 1;
            }
        }
        if let Some(h) = straight_high(&fc) {
            return HandValue { category: 8, tie: [h, 0, 0, 0, 0] };
        }
        let mut ranks: Vec<u8> = cards
            .iter()
            .filter(|c| c.suit as usize == suit)
            .map(|c| c.rank as u8)
            .collect();
        ranks.sort_unstable_by(|a, b| b.cmp(a));
        return HandValue {
            category: 5,
            tie: [ranks[0], ranks[1], ranks[2], ranks[3], ranks[4]],
        };
    }

    if let Some(h) = straight_high(&rank_count) {
        return HandValue { category: 4, tie: [h, 0, 0, 0, 0] };
    }

    let mut groups: Vec<(u8, u8)> = rank_count
        .iter()
        .enumerate()
        .filter(|(_, &n)| n > 0)
        .map(|(r, &n)| (n, r as u8))
        .collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    let g = |i: usize| -> u8 { groups.get(i).map(|x| x.1).unwrap_or(0) };
    let gs = |i: usize| -> u8 { groups.get(i).map(|x| x.0).unwrap_or(0) };

    match (gs(0), gs(1)) {
        (4, _) => {
            let mut kickers: Vec<u8> = groups.iter().filter(|x| x.0 != 4).map(|x| x.1).collect();
            kickers.sort_unstable_by(|a, b| b.cmp(a));
            HandValue {
                category: 7,
                tie: [g(0), kickers.first().copied().unwrap_or(0), 0, 0, 0],
            }
        }
        (3, 2) | (3, 3) => {
            let pair = groups.iter().find(|x| x.0 >= 2 && x.1 != g(0)).map(|x| x.1).unwrap_or(0);
            HandValue { category: 6, tie: [g(0), pair, 0, 0, 0] }
        }
        (3, _) => {
            let mut kickers: Vec<u8> = groups.iter().filter(|x| x.0 == 1).map(|x| x.1).collect();
            kickers.sort_unstable_by(|a, b| b.cmp(a));
            HandValue {
                category: 3,
                tie: [
                    g(0),
                    kickers.first().copied().unwrap_or(0),
                    kickers.get(1).copied().unwrap_or(0),
                    0,
                    0,
                ],
            }
        }
        (2, 2) => {
            let mut kickers: Vec<u8> = groups.iter().filter(|x| x.0 == 1).map(|x| x.1).collect();
            kickers.sort_unstable_by(|a, b| b.cmp(a));
            HandValue {
                category: 2,
                tie: [g(0), g(1), kickers.first().copied().unwrap_or(0), 0, 0],
            }
        }
        (2, _) => {
            let mut kickers: Vec<u8> = groups.iter().filter(|x| x.0 == 1).map(|x| x.1).collect();
            kickers.sort_unstable_by(|a, b| b.cmp(a));
            HandValue {
                category: 1,
                tie: [
                    g(0),
                    kickers.first().copied().unwrap_or(0),
                    kickers.get(1).copied().unwrap_or(0),
                    kickers.get(2).copied().unwrap_or(0),
                    0,
                ],
            }
        }
        _ => {
            let mut ranks: Vec<u8> = groups.iter().map(|x| x.1).collect();
            ranks.sort_unstable_by(|a, b| b.cmp(a));
            let mut tie = [0u8; 5];
            for i in 0..5 {
                tie[i] = ranks.get(i).copied().unwrap_or(0);
            }
            HandValue { category: 0, tie }
        }
    }
}

/// 7 张（2 底牌 + 5 公共牌）选最优 5 张评估
pub fn evaluate_7(hole: [Card; 2], board: &[Card]) -> HandValue {
    let mut all: Vec<Card> = board.to_vec();
    all.push(hole[0]);
    all.push(hole[1]);
    evaluate(&all)
}

/// 牌型中文名（用于 UI 展示）
pub fn hand_category_name(category: u8) -> &'static str {
    match category {
        8 => "同花顺",
        7 => "四条",
        6 => "葫芦",
        5 => "同花",
        4 => "顺子",
        3 => "三条",
        2 => "两对",
        1 => "一对",
        _ => "高牌",
    }
}

// ───────────────────────────── 引擎 ─────────────────────────────

pub type Seat = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    Fold,
    Check,
    Call,
    Raise,
    AllIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
    Showdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub seat: Seat,
    pub name: String,
    /// 剩余 Token
    pub tokens: i64,
    /// 本手累计投入（含盲注，用于边池）
    pub committed: i64,
    /// 本街已投入（每街重置，用于判断是否还需下注）
    pub street_committed: i64,
    pub folded: bool,
    pub all_in: bool,
    pub hole: [Option<Card>; 2],
    pub is_human: bool,
}

impl Player {
    fn new(seat: Seat, name: String, tokens: i64, is_human: bool) -> Self {
        Self {
            seat,
            name,
            tokens,
            committed: 0,
            street_committed: 0,
            folded: false,
            all_in: false,
            hole: [None, None],
            is_human,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pot {
    pub amount: i64,
    pub eligible: Vec<Seat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PokerRoom {
    pub id: String,
    pub name: String,
    pub players: Vec<Player>,
    pub board: Vec<Card>,
    /// 预留的公共牌（start_hand 时从 deck 抽出，逐街发放）
    pub pending_board: Vec<Card>,
    pub street: Street,
    pub dealer_seat: Seat,
    pub current_seat: Seat,
    pub small_blind: i64,
    pub big_blind: i64,
    pub street_high: i64,
    pub min_raise: i64,
    pub pots: Vec<Pot>,
    pub started: bool,
    pub hand_over: bool,
    pub winners: Vec<(Seat, i64)>,
    pub last_action: String,
}

impl PokerRoom {
    pub fn new(id: String, name: String, small_blind: i64, big_blind: i64) -> Self {
        Self {
            id,
            name,
            players: Vec::new(),
            board: Vec::new(),
            pending_board: Vec::new(),
            street: Street::Preflop,
            dealer_seat: 0,
            current_seat: 0,
            small_blind,
            big_blind,
            street_high: 0,
            min_raise: big_blind,
            pots: Vec::new(),
            started: false,
            hand_over: false,
            winners: Vec::new(),
            last_action: String::new(),
        }
    }

    pub fn add_player(&mut self, name: String, tokens: i64, is_human: bool) -> Seat {
        let seat = self.players.len() as Seat;
        self.players.push(Player::new(seat, name, tokens, is_human));
        seat
    }

    fn active(&self, p: &Player) -> bool {
        !p.folded && !p.all_in && p.tokens > 0
    }

    fn not_folded(&self, p: &Player) -> bool {
        !p.folded
    }

    /// 发牌并开局：清空上局状态、盲注、发底牌、预留公共牌、定位行动
    pub fn start_hand(&mut self, rng: &mut LcgRng) -> Result<(), String> {
        if self.players.len() < 2 {
            return Err("至少需要 2 名玩家".into());
        }
        let mut deck = Deck::standard();
        deck.shuffle(rng);

        for p in &mut self.players {
            p.hole = [None, None];
            p.committed = 0;
            p.street_committed = 0;
            p.folded = false;
            p.all_in = false;
        }
        self.board.clear();
        self.pending_board.clear();
        self.pots.clear();
        self.winners.clear();
        self.street = Street::Preflop;
        self.hand_over = false;
        self.last_action = "发牌".to_string();

        // 发两张底牌
        for i in 0..2 {
            for p in &mut self.players {
                p.hole[i] = deck.deal();
            }
        }
        // 预留 5 张公共牌（flop3 + turn1 + river1）
        for _ in 0..5 {
            self.pending_board.push(deck.deal().unwrap());
        }

        // 盲注：庄家左手两家
        let n = self.players.len() as Seat;
        let sb_seat = (self.dealer_seat + 1) % n;
        let bb_seat = (self.dealer_seat + 2) % n;

        {
            let p = &mut self.players[sb_seat as usize];
            let pay = self.small_blind.min(p.tokens);
            p.tokens -= pay;
            p.committed = pay;
            p.street_committed = pay;
            if p.tokens == 0 {
                p.all_in = true;
            }
        }
        {
            let p = &mut self.players[bb_seat as usize];
            let pay = self.big_blind.min(p.tokens);
            p.tokens -= pay;
            p.committed = pay;
            p.street_committed = pay;
            if p.tokens == 0 {
                p.all_in = true;
            }
        }

        self.street_high = self.big_blind;
        self.min_raise = self.big_blind;
        self.current_seat = (bb_seat + 1) % n;
        self.started = true;
        Ok(())
    }

    /// 玩家行动。`amount` 为 raise 的目标总额（本街投入）。
    pub fn apply_action(
        &mut self,
        seat: Seat,
        action: ActionKind,
        amount: Option<i64>,
    ) -> Result<(), String> {
        if self.hand_over {
            return Err("本手已结束".into());
        }
        if seat != self.current_seat {
            return Err(format!("现在轮到座位 {}，不是 {}", self.current_seat, seat));
        }
        let idx = seat as usize;
        if self.players[idx].folded {
            return Err("该玩家已弃牌".into());
        }

        let to_call = self.street_high - self.players[idx].street_committed;

        match action {
            ActionKind::Fold => {
                self.players[idx].folded = true;
                self.last_action = format!("{} 弃牌", self.players[idx].name);
            }
            ActionKind::Check => {
                if to_call > 0 {
                    return Err("有待跟注额，不能过牌".into());
                }
                self.last_action = format!("{} 过牌", self.players[idx].name);
            }
            ActionKind::Call => {
                let pay = to_call.min(self.players[idx].tokens);
                self.players[idx].tokens -= pay;
                self.players[idx].committed += pay;
                self.players[idx].street_committed += pay;
                if self.players[idx].tokens == 0 {
                    self.players[idx].all_in = true;
                }
                self.last_action = format!("{} 跟注 {}", self.players[idx].name, pay);
            }
            ActionKind::Raise => {
                let target = amount.unwrap_or(0);
                if target <= self.street_high {
                    return Err(format!("加注额须高于当前注 {}", self.street_high));
                }
                let pay = (target - self.players[idx].street_committed).min(self.players[idx].tokens);
                if pay <= 0 {
                    return Err("无效加注".into());
                }
                self.players[idx].tokens -= pay;
                self.players[idx].committed += pay;
                self.players[idx].street_committed += pay;
                self.street_high = self.players[idx].street_committed;
                if self.players[idx].tokens == 0 {
                    self.players[idx].all_in = true;
                }
                self.last_action = format!(
                    "{} 加注到 {}",
                    self.players[idx].name, self.players[idx].street_committed
                );
            }
            ActionKind::AllIn => {
                let pay = self.players[idx].tokens;
                self.players[idx].tokens = 0;
                self.players[idx].committed += pay;
                self.players[idx].street_committed += pay;
                self.players[idx].all_in = true;
                if self.players[idx].street_committed > self.street_high {
                    self.street_high = self.players[idx].street_committed;
                }
                self.last_action = format!(
                    "{} 全下 {}",
                    self.players[idx].name, self.players[idx].committed
                );
            }
        }

        self.advance();
        Ok(())
    }

    fn advance(&mut self) {
        let alive: Vec<usize> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.folded)
            .map(|(i, _)| i)
            .collect();
        if alive.len() == 1 {
            self.finish_hand();
            return;
        }

        let betting_done = self
            .players
            .iter()
            .filter(|p| self.not_folded(p))
            .all(|p| p.all_in || p.street_committed == self.street_high);

        if betting_done {
            // 未弃牌且未全下（还能行动）的玩家数
            let can_bet = alive.iter().filter(|&&i| !self.players[i].all_in).count();
            if can_bet <= 1 {
                // 无人能再下注 → 直接发完剩余公共牌并摊牌（全下跑板）
                self.run_out_board();
            } else {
                match self.street {
                    Street::Preflop => self.deal_street(3, Street::Flop),
                    Street::Flop => self.deal_street(1, Street::Turn),
                    Street::Turn => self.deal_street(1, Street::River),
                    Street::River => self.to_showdown(),
                    Street::Showdown => {}
                }
            }
        } else {
            self.next_active_seat();
        }
    }

    /// 全下跑板：无人能下注时，逐街发完公共牌并进入摊牌
    fn run_out_board(&mut self) {
        while self.street != Street::Showdown {
            match self.street {
                Street::Preflop => self.deal_street(3, Street::Flop),
                Street::Flop => self.deal_street(1, Street::Turn),
                Street::Turn => self.deal_street(1, Street::River),
                Street::River => {
                    self.to_showdown();
                    break;
                }
                Street::Showdown => break,
            }
        }
    }

    fn next_active_seat(&mut self) {
        let n = self.players.len() as Seat;
        if n == 0 {
            return;
        }
        for _ in 0..n {
            self.current_seat = (self.current_seat + 1) % n;
            let p = &self.players[self.current_seat as usize];
            if self.active(p) {
                return;
            }
        }
    }

    fn deal_street(&mut self, count: usize, street: Street) {
        for _ in 0..count {
            if let Some(c) = self.pending_board.pop() {
                self.board.push(c);
            }
        }
        self.street = street;
        for p in &mut self.players {
            p.street_committed = 0;
        }
        self.street_high = 0;
        self.current_seat = self.first_active_after_dealer();
    }

    fn first_active_after_dealer(&self) -> Seat {
        let n = self.players.len() as Seat;
        for step in 1..=n {
            let s = (self.dealer_seat + step) % n;
            let p = &self.players[s as usize];
            if self.active(p) {
                return s;
            }
        }
        0
    }

    fn to_showdown(&mut self) {
        self.street = Street::Showdown;
        let alive: Vec<usize> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.folded)
            .map(|(i, _)| i)
            .collect();

        if alive.len() == 1 {
            self.finish_hand();
            return;
        }

        self.pots = self.compute_side_pots(&alive);

        let mut winnings: Vec<(Seat, i64)> = vec![(0, 0); self.players.len()];
        for pot in &self.pots {
            let mut best: Option<(HandValue, Seat)> = None;
            for &s in &pot.eligible {
                let p = &self.players[s as usize];
                if p.hole[0].is_none() || p.hole[1].is_none() {
                    continue;
                }
                let hv = evaluate_7([p.hole[0].unwrap(), p.hole[1].unwrap()], &self.board);
                match best {
                    None => best = Some((hv, s)),
                    Some((bhv, _)) => {
                        if hv > bhv {
                            best = Some((hv, s));
                        }
                    }
                }
            }
            if let Some((_, ws)) = best {
                winnings[ws as usize].1 += pot.amount;
            }
        }

        for (seat, amount) in winnings.iter().enumerate() {
            if amount.1 > 0 {
                self.players[seat].tokens += amount.1;
            }
        }
        self.winners = winnings
            .iter()
            .enumerate()
            .filter(|(_, (_, a))| *a > 0)
            .map(|(_, (s, a))| (*s, *a))
            .collect();
        self.last_action = "摊牌".to_string();
        self.hand_over = true;
    }

    fn compute_side_pots(&self, alive: &[usize]) -> Vec<Pot> {
        let mut levels: Vec<i64> = alive.iter().map(|&i| self.players[i].committed).collect();
        levels.sort_unstable();
        levels.dedup();

        let mut pots = Vec::new();
        let mut prev = 0i64;
        for &level in &levels {
            let diff = level - prev;
            if diff <= 0 {
                continue;
            }
            let eligible: Vec<Seat> = alive
                .iter()
                .filter(|&&i| self.players[i].committed >= level)
                .map(|&i| i as Seat)
                .collect();
            let contributors = alive
                .iter()
                .filter(|&&i| self.players[i].committed >= level)
                .count() as i64;
            let amount = diff * contributors;
            if amount > 0 {
                pots.push(Pot { amount, eligible });
            }
            prev = level;
        }
        pots
    }

    fn finish_hand(&mut self) {
        let total: i64 = self.players.iter().map(|p| p.committed).sum();
        if let Some(ws) = self.players.iter().position(|p| !p.folded).map(|i| i as Seat) {
            self.players[ws as usize].tokens += total;
            self.winners = vec![(ws, total)];
            self.last_action = format!("{} 赢得底池", self.players[ws as usize].name);
        }
        self.hand_over = true;
        self.street = Street::Showdown;
    }
}

// ───────────────────────────── 房间管理器 ─────────────────────────────

/// 进程内房间管理器：多人本地对战（多个 human/AI 座位同坐一桌）。
/// 跨机联机需在此之上叠加 relay 传输层（本切片不落地网络）。
#[derive(Debug, Default)]
pub struct PokerHub {
    pub rooms: HashMap<String, PokerRoom>,
    pub rng_seed: u64,
}

impl PokerHub {
    pub fn new() -> Self {
        Self {
            rooms: HashMap::new(),
            rng_seed: 0xC0FFEE,
        }
    }

    pub fn create_room(&mut self, name: String, small_blind: i64, big_blind: i64) -> String {
        let id = format!("room_{}", self.rooms.len() + 1);
        let room = PokerRoom::new(id.clone(), name, small_blind, big_blind);
        self.rooms.insert(id.clone(), room);
        id
    }

    pub fn join_room(
        &mut self,
        room_id: &str,
        name: String,
        tokens: i64,
        is_human: bool,
    ) -> Result<Seat, String> {
        let room = self.rooms.get_mut(room_id).ok_or("房间不存在")?;
        if room.started {
            return Err("牌局已开始，不能加入".into());
        }
        Ok(room.add_player(name, tokens, is_human))
    }

    pub fn start_room(&mut self, room_id: &str) -> Result<(), String> {
        let room = self.rooms.get_mut(room_id).ok_or("房间不存在")?;
        let mut rng = LcgRng::new(self.rng_seed);
        self.rng_seed = self.rng_seed.wrapping_add(0x1234_5678);
        room.start_hand(&mut rng)
    }

    pub fn act(
        &mut self,
        room_id: &str,
        seat: Seat,
        action: ActionKind,
        amount: Option<i64>,
    ) -> Result<(), String> {
        let room = self.rooms.get_mut(room_id).ok_or("房间不存在")?;
        room.apply_action(seat, action, amount)
    }

    pub fn room(&self, room_id: &str) -> Option<&PokerRoom> {
        self.rooms.get(room_id)
    }
}

// ───────────────────────────── 全局大厅 ─────────────────────────────

static GLOBAL_HUB: OnceLock<Arc<Mutex<PokerHub>>> = OnceLock::new();

/// 进程级全局扑克大厅：供网关 RPC（`poker.*`）与 GUI（Tauri AppState）共享同一实例，
/// 避免两套房间状态。仿 `usage::LEDGER` 的 OnceLock 单例模式。
pub fn global_hub() -> &'static Arc<Mutex<PokerHub>> {
    GLOBAL_HUB.get_or_init(|| Arc::new(Mutex::new(PokerHub::new())))
}

// ───────────────────────────── 测试 ─────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cards(s: &str) -> Vec<Card> {
        s.split_whitespace()
            .map(|tok| {
                let mut c = tok.chars();
                let r = c.next().unwrap();
                let su = c.next().unwrap();
                let rank = match r {
                    '2' => Rank::Two,
                    '3' => Rank::Three,
                    '4' => Rank::Four,
                    '5' => Rank::Five,
                    '6' => Rank::Six,
                    '7' => Rank::Seven,
                    '8' => Rank::Eight,
                    '9' => Rank::Nine,
                    'T' => Rank::Ten,
                    'J' => Rank::Jack,
                    'Q' => Rank::Queen,
                    'K' => Rank::King,
                    'A' => Rank::Ace,
                    _ => panic!("bad rank {}", r),
                };
                let suit = match su {
                    's' => Suit::Spades,
                    'h' => Suit::Hearts,
                    'd' => Suit::Diamonds,
                    'c' => Suit::Clubs,
                    _ => panic!("bad suit {}", su),
                };
                Card::new(rank, suit)
            })
            .collect()
    }

    #[test]
    fn deck_is_52_and_shuffle_deterministic() {
        let mut d1 = Deck::standard();
        let mut d2 = Deck::standard();
        assert_eq!(d1.len(), 52);
        let mut r1 = LcgRng::new(42);
        let mut r2 = LcgRng::new(42);
        d1.shuffle(&mut r1);
        d2.shuffle(&mut r2);
        assert_eq!(d1.cards, d2.cards);
        assert_eq!(d1.len(), 52);
    }

    #[test]
    fn evaluate_categories_order() {
        let royal = cards("As Ks Qs Js Ts");
        let sf = cards("9h 8h 7h 6h 5h");
        let quads = cards("Qc Qd Qh Qs 2s");
        let boat = cards("Qc Qd Qh 2s 2d");
        let flush = cards("Ah Kh 9h 6h 3h");
        let straight = cards("9h 8d 7c 6s 5h");
        let trips = cards("Qc Qd Qh 9s 2d");
        let two_pair = cards("Qc Qd 9h 9s 2d");
        let pair = cards("Qc Qd 9h 7s 2d");
        let high = cards("Ah Qd 9h 7s 2d");

        assert_eq!(evaluate(&royal).category, 8);
        assert_eq!(evaluate(&sf).category, 8);
        assert!(evaluate(&royal) > evaluate(&sf));
        assert!(evaluate(&sf) > evaluate(&quads));
        assert!(evaluate(&quads) > evaluate(&boat));
        assert!(evaluate(&boat) > evaluate(&flush));
        assert!(evaluate(&flush) > evaluate(&straight));
        assert!(evaluate(&straight) > evaluate(&trips));
        assert!(evaluate(&trips) > evaluate(&two_pair));
        assert!(evaluate(&two_pair) > evaluate(&pair));
        assert!(evaluate(&pair) > evaluate(&high));
    }

    #[test]
    fn wheel_straight_detected() {
        let wheel = cards("Ah 2d 3c 4s 5h");
        assert_eq!(evaluate(&wheel).category, 4);
        assert_eq!(evaluate(&wheel).tie[0], 3);
    }

    #[test]
    fn seven_card_best_five() {
        let hole_arr = cards("As Ks");
        let hole = [hole_arr[0], hole_arr[1]];
        let board = cards("Qs Js Ts 2d 7c");
        assert_eq!(evaluate_7(hole, &board).category, 8);
    }

    #[test]
    fn engine_blind_and_deal() {
        let mut hub = PokerHub::new();
        let rid = hub.create_room("t".into(), 1, 2);
        hub.join_room(&rid, "A".into(), 100, true).unwrap();
        hub.join_room(&rid, "B".into(), 100, false).unwrap();
        hub.join_room(&rid, "C".into(), 100, false).unwrap();
        hub.start_room(&rid).unwrap();
        let room = hub.room(&rid).unwrap();
        assert!(room.players.iter().all(|p| p.hole[0].is_some() && p.hole[1].is_some()));
        assert_eq!(room.players[1].committed, 1);
        assert_eq!(room.players[2].committed, 2);
        assert_eq!(room.street_high, 2);
        assert_eq!(room.pending_board.len(), 5);
    }

    #[test]
    fn engine_showdown_board_dealt() {
        let mut hub = PokerHub::new();
        let rid = hub.create_room("t".into(), 1, 2);
        hub.join_room(&rid, "A".into(), 100, true).unwrap();
        hub.join_room(&rid, "B".into(), 100, false).unwrap();
        hub.start_room(&rid).unwrap();
        // 2 人桌：庄=0 → 小盲=1、大盲=0，翻前第一家是座位 1
        // 让 2 人快速全下 → 摊牌，验证 board 发满 5 张
        hub.act(&rid, 1, ActionKind::AllIn, None).unwrap();
        hub.act(&rid, 0, ActionKind::AllIn, None).unwrap();
        let room = hub.room(&rid).unwrap();
        assert!(room.hand_over);
        assert_eq!(room.board.len(), 5);
    }
}
