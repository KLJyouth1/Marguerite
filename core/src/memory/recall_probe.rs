//! 记忆检索通路的**常驻离线回归**（P0 交付物）。
//!
//! # 为什么存在
//!
//! S1 的判定性探针是在会话外的临时脚本里跑的，跑完就没了 —— 而它给出的结论
//! （「分层记忆通路打不过一个朴素关键词基线，且被绝对门饿死」）正是本轮改动的**唯一依据**。
//! 一次性的测量留不住：**没有常驻回归，下一句「我把打分改好了」又是一次无法反驳的自述。**
//!
//! 于是把探针搬进 `cargo test`：夹具在 [`fixtures/recall_probe.json`](super::recall_probe::FIXTURE)，
//! 语料入库、查询永不入库。三条硬口径：
//!
//! 1. **夹具必须自证。** `hard` 族的 `paraphrase` 查询与同族语料的**实词重合必须为 0**；
//!    不为 0 就是夹具退化（词面泄漏会让"语义泛化"看起来存在），本模块自己判失败，
//!    而不是让指标悄悄变好看。
//! 2. **同库同查询同 K**，五个通路并排测：
//!    `tiered`（本次改完的分层通路）、`loose`（朴素关键词基线，即"必须打平"的对象）、
//!    `legacy_gate`（被退休的旧规则：纯余弦 + 绝对门 0.35）、`substr`（感知层原来的整条子串）、
//!    `sensory_overlap`（D5 改完的感知判据）。
//! 3. **棘轮只许升不许降。** 下方 `MIN_*` 常量全部来自实测，留出余量；任何通路变差都会在这里红，
//!    而不是等到用户问「昨天还能召回，今天怎么空了」。
//! 4. **绝对下限与相对判据同时在场。** 绝对数会随指标口径过期（本轮就发生过一次，见
//!    [`ratchet_floors_hold`] 上方那张表），所以每条绝对线都配一条「不得比 `loose` 差」的
//!    相对线：召回、饿死、注入精度三项都各有一条。相对线不依赖语料的真实性，也不依赖标度。
//!
//! ⚠️ 语料是刻意构造的（真实记忆的词面重合率高于此），所以**绝对数字**会偏离线上；
//! 被钉住的是**通路之间的相对排序**与**饿死现象**——那两条不依赖语料的真实性。

use std::collections::HashSet;

use super::gateway::sensory_overlap;
use super::lexical::{content_words, shared_content_words};
use super::long_term::LocalEmbedding;
use super::scoring::{self, WEAK_EVIDENCE};
use super::short_term::{self, ShortTermMemory};
use super::tiered::{MemoryTierVariant, TieredMemory};

/// 离线夹具（15 族 × 5 条语料入库；60 条查询永不入库）。
pub(crate) const FIXTURE: &str = include_str!("fixtures/recall_probe.json");

/// 与线上 `inject` 链路口径一致的 K。
const TOP_K: usize = 5;

#[derive(serde::Deserialize)]
struct Fixture {
    families: Vec<Family>,
}

#[derive(serde::Deserialize)]
struct Family {
    id: String,
    lang: String,
    hard: bool,
    corpus: Vec<String>,
    queries: Vec<Query>,
}

#[derive(serde::Deserialize)]
struct Query {
    text: String,
    kind: String,
}

/// 入库的一条语料（`family` = 所属族序号，即 ground truth）。
struct Doc {
    family: usize,
    text: String,
}

/// 被测的五条通路。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Strategy {
    /// 改完的分层记忆：词面优先排序 + 零重合闸 + 相对截断（见 [`scoring`]）。
    Tiered,
    /// 朴素关键词基线：共享实词数降序，重合为 0 的不给。
    Loose,
    /// 退休的旧规则：纯余弦排序 + **绝对门** `cosine ≥ 0.35`。
    LegacyGate,
    /// 感知层原来的判据：整条查询当子串。
    Substr,
    /// D5 之后的感知判据：实词重合（与 `memory.sensory.search` 同一条代码）。
    SensoryOverlap,
}

impl Strategy {
    fn label(self) -> &'static str {
        match self {
            Strategy::Tiered => "tiered",
            Strategy::Loose => "loose",
            Strategy::LegacyGate => "legacy_gate",
            Strategy::Substr => "substr",
            Strategy::SensoryOverlap => "sensory",
        }
    }

    const ALL: [Strategy; 5] = [
        Strategy::Tiered,
        Strategy::Loose,
        Strategy::LegacyGate,
        Strategy::Substr,
        Strategy::SensoryOverlap,
    ];

    /// 返回本次查询的命中（doc 下标，已按该通路的排序截断到 TOP_K）。
    fn retrieve(self, query: &str, docs: &[Doc], base: &TieredMemory) -> Vec<usize> {
        match self {
            Strategy::Tiered => {
                // progressive_retrieve 会 touch()，每条查询都从同一份干净副本出发，
                // 免得「前一条查询的访问计数」影响后一条（旧实现正是这个毛病）。
                let mut m = base.clone();
                m.progressive_retrieve(query, TOP_K)
                    .into_iter()
                    // 长时层的 id 就是 doc 下标 + 1（见 build_library）。
                    .map(|h| (h.entry.id - 1) as usize)
                    .collect()
            }
            Strategy::Loose => {
                let qw = content_words(query);
                let mut scored: Vec<(usize, usize)> = docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (shared_content_words(&qw, &content_words(&d.text)), i))
                    .filter(|(overlap, _)| *overlap > 0)
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                scored.into_iter().take(TOP_K).map(|(_, i)| i).collect()
            }
            Strategy::LegacyGate => {
                let qv = LocalEmbedding::embed(query);
                let mut scored: Vec<(f64, usize)> = docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (LocalEmbedding::cosine(&qv, &LocalEmbedding::embed(&d.text)), i))
                    .filter(|(cosine, _)| *cosine >= WEAK_EVIDENCE)
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
                scored.into_iter().take(TOP_K).map(|(_, i)| i).collect()
            }
            Strategy::Substr => {
                let q = query.to_lowercase();
                docs.iter()
                    .enumerate()
                    .filter(|(_, d)| d.text.to_lowercase().contains(&q))
                    .take(TOP_K)
                    .map(|(i, _)| i)
                    .collect()
            }
            Strategy::SensoryOverlap => {
                // 与 gateway::sensory_search 完全同一条判据与排序（含「同分时新的在前」）。
                let mut scored: Vec<(usize, usize)> = docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| (sensory_overlap(query, &d.text), i))
                    .filter(|(overlap, _)| *overlap > 0)
                    .collect();
                scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                scored.into_iter().take(TOP_K).map(|(_, i)| i).collect()
            }
        }
    }
}

/// 一个「档」=（语种类别, 查询种类）。与探针原表的六档一一对应。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Bucket {
    /// `en` / `zh` / `hard`
    lang: &'static str,
    /// `lexical` / `paraphrase`
    kind: &'static str,
}

impl Bucket {
    fn label(self) -> String {
        format!("{}·{}", self.lang, self.kind)
    }
}

fn bucket_of(family: &Family, kind: &str) -> Bucket {
    let lang = if family.hard {
        "hard"
    } else if family.lang == "zh" {
        "zh"
    } else {
        "en"
    };
    // paraphrase2 是第二份改述，与 paraphrase 同类，合并统计（分母太小会让单条查询左右结果）。
    let kind = if kind.starts_with("paraphrase") { "paraphrase" } else { "lexical" };
    Bucket { lang, kind }
}

#[derive(Copy, Clone, Debug, Default)]
struct Cells {
    queries: usize,
    hit: usize,
    starved: usize,
    /// 「注入的行里属于本查询的族」的比例累加，**只对有命中的查询**计入。
    prec_sum: f64,
    /// `prec_sum` 的分母。饿死的查询不稀释精确率——它们由 `starved` 单独计量，
    /// 混进这个分母会让「什么都不注入」看起来和「注入得很准」一样好。
    scored: usize,
    /// K 个槽位里被**真证据**填掉多少（对全部查询，分母 `TOP_K × queries`）。
    /// 这条刻意与 `prec` 分开：一个通路「注入得准」和「把预算填满」是两件事，
    /// 只看前者会奖励「只投一行」、只看后者会奖励「无脑投满 5 行」。
    fill_sum: f64,
}

impl Cells {
    fn hit_rate(self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            self.hit as f64 / self.queries as f64
        }
    }
    /// 注入精度：实际进上下文的那些行里，属于本查询族的比例。
    fn prec(self) -> f64 {
        if self.scored == 0 {
            0.0
        } else {
            self.prec_sum / self.scored as f64
        }
    }
    /// 预算利用率：`TOP_K` 个槽位被真证据填掉多少。
    fn fill(self) -> f64 {
        if self.queries == 0 {
            0.0
        } else {
            self.fill_sum / (self.queries * TOP_K) as f64
        }
    }
    fn merge(&mut self, other: &Cells) {
        self.queries += other.queries;
        self.hit += other.hit;
        self.starved += other.starved;
        self.prec_sum += other.prec_sum;
        self.scored += other.scored;
        self.fill_sum += other.fill_sum;
    }
}

/// 一次测量：`(档, 通路) -> 指标`，外加全库汇总。
struct Report {
    cells: Vec<(Bucket, &'static str, Cells)>,
    total: Vec<(&'static str, Cells)>,
}

impl Report {
    fn get(&self, lang: &'static str, kind: &'static str, strategy: &str) -> Cells {
        let bucket = Bucket { lang, kind };
        self.cells
            .iter()
            .find(|(b, s, _)| *b == bucket && *s == strategy)
            .map(|(_, _, c)| *c)
            .unwrap_or_default()
    }

    fn total(&self, strategy: &str) -> Cells {
        self.total
            .iter()
            .find(|(s, _)| *s == strategy)
            .map(|(_, c)| *c)
            .unwrap_or_default()
    }

    /// 同一语种（跨 lexical/paraphrase）的汇总：规格的「hard 档」判据就是这个口径。
    fn agg(&self, lang: &'static str, strategy: &str) -> Cells {
        let mut acc = Cells::default();
        for (b, s, c) in &self.cells {
            if b.lang == lang && *s == strategy {
                acc.merge(c);
            }
        }
        acc
    }

    /// 六档里 `tiered` 与 `loose` 的 hit@5 差值（正=tiered 更强）。
    fn tiered_minus_loose(&self, lang: &'static str, kind: &'static str) -> f64 {
        self.get(lang, kind, "tiered").hit_rate() - self.get(lang, kind, "loose").hit_rate()
    }
}

fn load_fixture() -> Fixture {
    serde_json::from_str(FIXTURE).expect("夹具 recall_probe.json 必须可解析")
}

/// 测量结果里出现过的档位（去重、稳定序）。
///
/// 三处按档遍历的判据都必须用这里，**不要手写清单**：曾经有一份手写的
/// `[("lexical", en/zh/hard), ("paraphrase", en/hard)]` 漏掉了 `zh·paraphrase`，
/// 于是绝对地板清空整档中文改述时，基准对照一声不响。
fn buckets_in(report: &Report) -> Vec<Bucket> {
    let mut out: Vec<Bucket> = Vec::new();
    for (b, _, _) in &report.cells {
        if !out.contains(b) {
            out.push(*b);
        }
    }
    out.sort();
    out
}

/// 把所有语料按族顺序装进一个 TieredMemory（长时层）；同时返回 doc 表。
/// doc 下标 = 插入顺序，`entry.id = 下标 + 1`。
fn build_library(fixture: &Fixture) -> (Vec<Doc>, TieredMemory) {
    let mut docs = Vec::new();
    let mut m = TieredMemory::new();
    for (fi, fam) in fixture.families.iter().enumerate() {
        for text in &fam.corpus {
            docs.push(Doc { family: fi, text: text.clone() });
            m.insert(MemoryTierVariant::LongTerm, text.clone(), fi.to_string());
        }
    }
    (docs, m)
}

fn measure(fixture: &Fixture) -> Report {
    let (docs, base) = build_library(fixture);
    let mut cells: Vec<(Bucket, &'static str, Cells)> = Vec::new();
    let mut totals: Vec<(&'static str, Cells)> = Vec::new();

    let mut buckets: Vec<Bucket> = Vec::new();
    for fam in &fixture.families {
        for q in &fam.queries {
            let b = bucket_of(fam, &q.kind);
            if !buckets.contains(&b) {
                buckets.push(b);
            }
        }
    }
    buckets.sort();

    for strategy in Strategy::ALL {
        let mut total = Cells::default();
        for bucket in &buckets {
            let mut c = Cells::default();
            for (fi, fam) in fixture.families.iter().enumerate() {
                for q in &fam.queries {
                    if bucket_of(fam, &q.kind) != *bucket {
                        continue;
                    }
                    let hits = strategy.retrieve(&q.text, &docs, &base);
                    let own = hits.iter().filter(|i| docs[**i].family == fi).count();
                    c.queries += 1;
                    if own > 0 {
                        c.hit += 1;
                    }
                    if hits.is_empty() {
                        c.starved += 1;
                    } else {
                        c.prec_sum += own as f64 / hits.len() as f64;
                        c.scored += 1;
                    }
                    c.fill_sum += own as f64;
                }
            }
            cells.push((*bucket, strategy.label(), c));
            total.merge(&c);
        }
        totals.push((strategy.label(), total));
    }
    Report { cells, total: totals }
}

// ───────────────────────────── 自证 ─────────────────────────────

/// 夹具必须自证：`hard` 族的改述与同族语料**零实词重合**。
/// 一旦有人为了让指标好看而"顺手"把关键词加回改述里，这里先红。
#[test]
fn hard_paraphrases_share_no_content_words_with_their_own_corpus() {
    let fixture = load_fixture();
    let mut checked = 0usize;
    let mut overlaps: Vec<usize> = Vec::new();
    for fam in fixture.families.iter().filter(|f| f.hard) {
        for q in fam.queries.iter().filter(|q| q.kind.starts_with("paraphrase")) {
            let qw = content_words(&q.text);
            assert!(!qw.is_empty(), "改述 {:?} 一个实词都没有，夹具本身没意义", q.text);
            for line in &fam.corpus {
                let o = shared_content_words(&qw, &content_words(line));
                overlaps.push(o);
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 20,
        "hard 档改述覆盖太少（{checked} 对），夹具不足以自证"
    );
    let bad: Vec<usize> = overlaps.iter().copied().filter(|o| *o > 0).collect();
    assert!(
        bad.is_empty(),
        "hard 族改述与同族语料出现词面泄漏：{} 对重合 >0（max={}）。零重合是本夹具的存在前提。",
        bad.len(),
        overlaps.iter().copied().max().unwrap_or(0)
    );
}

/// 没有任何查询逐字躺在语料里 —— 否则 hit@5 测的是「字符串相等」而不是检索。
#[test]
fn no_query_is_also_in_the_corpus() {
    let fixture = load_fixture();
    let corpus: HashSet<String> = fixture
        .families
        .iter()
        .flat_map(|f| f.corpus.iter())
        .map(|t| t.to_lowercase())
        .collect();
    let mut offenders = Vec::new();
    for fam in &fixture.families {
        for q in &fam.queries {
            if corpus.contains(&q.text.to_lowercase()) {
                offenders.push(q.text.clone());
            }
        }
    }
    assert!(offenders.is_empty(), "以下查询与语料完全相同：{offenders:?}");
}

/// 每条 `lexical` 查询必须至少与同族一条语料共享实词 —— 否则该档无从命中，指标是假的。
#[test]
fn every_lexical_query_overlaps_its_own_family() {
    let fixture = load_fixture();
    let mut offenders = Vec::new();
    for fam in &fixture.families {
        for q in fam.queries.iter().filter(|q| q.kind == "lexical") {
            let qw = content_words(&q.text);
            let best = fam
                .corpus
                .iter()
                .map(|c| shared_content_words(&qw, &content_words(c)))
                .max()
                .unwrap_or(0);
            if best == 0 {
                offenders.push((fam.id.clone(), q.text.clone()));
            }
        }
    }
    assert!(offenders.is_empty(), "lexical 查询与同族零重合（档位失去意义）：{offenders:?}");
}

/// 长时池与 flyhash 索引必须一致：检索已不再依赖它，但观测面与 P1 的去重都要靠它。
#[test]
fn library_and_flyhash_index_stay_in_lockstep() {
    let fixture = load_fixture();
    let (docs, base) = build_library(&fixture);
    assert_eq!(docs.len(), 75, "夹具应为 15 族 × 5 条");
    assert_eq!(base.len(MemoryTierVariant::LongTerm), docs.len());
    assert_eq!(
        base.fly_len(),
        base.len(MemoryTierVariant::LongTerm),
        "flyhash 索引条数必须与长时池一致"
    );
    assert!(
        base.max_per_tier() > docs.len(),
        "夹具不该触发 FIFO 淘汰，否则指标测的是淘汰而不是检索"
    );
}

// ───────────────────────────── 指标 ─────────────────────────────

/// 打印六档 × 五通路对照表（`cargo test -p marguerite-core --lib memory::recall_probe -- --nocapture`）。
#[test]
fn print_offline_probe_table() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    eprintln!("{}", render_table(&report));
}

/// 表格列宽：单元格是「命中率·饿死数·注入精度·预算利用率」，24 列够放。
const COL: usize = 24;

fn cell(c: Cells) -> String {
    format!(
        "{:.0}%·饿{}·p{:.2}·f{:.2}",
        c.hit_rate() * 100.0,
        c.starved,
        c.prec(),
        c.fill()
    )
}

fn render_table(report: &Report) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let seen = buckets_in(report);
    let header: String = Strategy::ALL
        .iter()
        .map(|s| format!("{:>COL$}", s.label()))
        .collect::<Vec<_>>()
        .join("");
    let _ = writeln!(out, "{:<COL$}{}", "档(命中·饿死·精度p·填满f)", header);
    for kind in ["lexical", "paraphrase"] {
        for lang in ["en", "zh", "hard"] {
            let b = Bucket { lang, kind };
            if !seen.contains(&b) {
                continue;
            }
            let n = report.get(lang, kind, "loose").queries;
            let _ = write!(out, "{:<COL$}", format!("{} n={}", b.label(), n));
            for s in Strategy::ALL {
                let _ = write!(out, "{:>COL$}", cell(report.get(lang, kind, s.label())));
            }
            let _ = writeln!(out);
        }
    }
    let _ = write!(out, "{:<COL$}", "全库");
    for s in Strategy::ALL {
        let _ = write!(out, "{:>COL$}", cell(report.total(s.label())));
    }
    let _ = writeln!(out);
    out
}

/// 逐查询找出「`loose` 命中而 `tiered` 漏掉」的那些，并给出 tiered 视角下的真实分数与位次。
///
/// 失败信息必须**可直接行动**：只说「75% < 83%」的回归测试，下一次还是要从头查。
fn tiered_misses_vs_loose(fixture: &Fixture) -> Vec<String> {
    let (docs, base) = build_library(fixture);
    let mut out = Vec::new();
    for (fi, fam) in fixture.families.iter().enumerate() {
        for q in &fam.queries {
            let tiered = Strategy::Tiered.retrieve(&q.text, &docs, &base);
            let loose = Strategy::Loose.retrieve(&q.text, &docs, &base);
            let own = |ids: &[usize]| ids.iter().any(|i| docs[*i].family == fi);
            if own(&loose) && !own(&tiered) {
                // 重算 tiered 的完整排序（不截断、不限 K），看目标族到底排第几、差多少
                let qw = content_words(&q.text);
                let qv = LocalEmbedding::embed(&q.text);
                let mut ranked: Vec<(f64, usize, usize, f64)> = docs
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        let cosine = LocalEmbedding::cosine(&qv, &LocalEmbedding::embed(&d.text));
                        let overlap = shared_content_words(&qw, &content_words(&d.text));
                        (super::scoring::fuse(overlap, scoring::coverage(qw.len(), overlap), cosine), i, overlap, cosine)
                    })
                    .collect();
                ranked.sort_by(|a, b| {
                    scoring::lexical_first(a.2, a.0, b.2, b.0).then(a.1.cmp(&b.1))
                });
                let cutoff = super::scoring::cutoff(ranked.first().map(|r| r.0).unwrap_or(0.0));
                let worst_kept = ranked.iter().filter(|r| r.0 >= cutoff).count();
                let target = ranked
                    .iter()
                    .position(|r| docs[r.1].family == fi)
                    .map(|p| format!("目标族位次={} rank={:.3} overlap={} cosine={:.3}", p + 1, ranked[p].0, ranked[p].2, ranked[p].3))
                    .unwrap_or_else(|| "目标族不在排序里".to_string());
                let winners = ranked
                    .iter()
                    .take(5)
                    .map(|r| format!("族{}({:.3}/词{})", docs[r.1].family, r.0, r.2))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push(format!(
                    "[{}|{}] {} … cutoff={:.3} 过线数={}；前5={}；{}",
                    fam.id,
                    q.kind,
                    truncate(&q.text, 60),
                    cutoff,
                    worst_kept,
                    winners,
                    target
                ));
            }
        }
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n).collect();
        t.push('…');
        t
    }
}

/// 一次基准对照：`tiered` 不得比 `loose` 弱 —— 这是「换打分」这件事的全部意义。
/// 实测曾为 en 41.7% vs 70.8%、zh 40% vs 100%；改完后必须收敛到打平或更好。
///
/// ⚠️ 档位**从测量结果里枚举**，不是手写清单。这里原来是一份手写的
/// `[("lexical", [en,zh,hard]), ("paraphrase", [en,hard])]`，漏了 `zh·paraphrase` ——
/// 于是绝对地板把整档中文改述清空（100% → 70%）时，这条基准对照**一声不响**。
/// 回归网自己漏档，比被测代码漏档更糟：它让人以为「网兜住了」。
#[test]
fn tiered_is_never_weaker_than_the_naive_keyword_baseline() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let mut lags = Vec::new();
    let mut compared = 0usize;
    for bucket in &buckets_in(&report) {
        if report.get(bucket.lang, bucket.kind, "loose").queries == 0 {
            continue;
        }
        compared += 1;
        let delta = report.tiered_minus_loose(bucket.lang, bucket.kind);
        if delta < -0.001 {
            lags.push(format!(
                "{}: tiered {:.1}% < loose {:.1}%",
                bucket.label(),
                report.get(bucket.lang, bucket.kind, "tiered").hit_rate() * 100.0,
                report.get(bucket.lang, bucket.kind, "loose").hit_rate() * 100.0
            ));
        }
    }
    assert!(
        compared >= 6,
        "基准对照只比了 {compared} 档（夹具应有 6 档）：网在空转，红也不会响"
    );
    assert!(
        lags.is_empty(),
        "分层通路仍弱于朴素关键词基线：{}\n{}\n漏掉的查询：\n{}",
        lags.join("; "),
        render_table(&report),
        tiered_misses_vs_loose(&fixture).join("\n")
    );
}

/// 退休的旧规则必须** measurable 地**更差 —— 这条断言不是为了好看：
/// 谁把绝对门重新接回打分，这里立刻红。
#[test]
fn the_retired_absolute_gate_is_measurably_worse() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let legacy = report.total("legacy_gate");
    let tiered = report.total("tiered");
    assert!(
        tiered.queries > 0 && legacy.queries == tiered.queries,
        "两条通路必须同一批查询：{} vs {}",
        legacy.queries,
        tiered.queries
    );
    assert!(
        legacy.starved > tiered.starved,
        "旧绝对门饿死 {} 条 vs 新规则 {} 条：若前者不再更多，说明绝对门又回来了",
        legacy.starved,
        tiered.starved
    );
    assert!(
        tiered.hit_rate() > legacy.hit_rate(),
        "新规则 hit@5 {:.1}% 未超过旧规则 {:.1}%",
        tiered.hit_rate() * 100.0,
        legacy.hit_rate() * 100.0
    );
}

/// 整条子串判据（感知层原来的写法）实测接近全空 —— 把它钉成反向自证：
/// D5 的实词重合必须显著强于它，否则「修了」只是话术。
#[test]
fn sensory_overlap_beats_the_whole_query_substring_by_a_margin() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let old = report.total("substr");
    let new = report.total("sensory");
    assert!(
        new.hit_rate() > old.hit_rate(),
        "感知层实词重合 {:.1}% 没有超过整条子串 {:.1}%",
        new.hit_rate() * 100.0,
        old.hit_rate() * 100.0
    );
    // 规格 §3 D5 的验收线（原路 0.0% → ≥60%）
    assert!(
        new.hit_rate() >= 0.60,
        "感知层 hit@5 {:.1}% 低于 D5 验收线 60%",
        new.hit_rate() * 100.0
    );
}

// ───────────────────────── 验收线 + 棘轮 ─────────────────────────
//
// 下面每个数字都是 **2026-10-29 实测**（`--nocapture` 打印的同一张表），留出余量后写死：
//
// | 档 | tiered hit@5 | 饿死 | 注入精度 p | 预算利用 f |
// |---|---|---|---|---|
// | en·lexical (6) | 100% | 0 | 0.94 | 0.47 |
// | zh·lexical (5) | 100% | 0 | 0.92 | 0.40 |
// | hard·lexical (4) | 100% | 0 | 1.00 | 0.45 |
// | en·paraphrase (12) | 83% | 1 | 0.60 | 0.27 |
// | zh·paraphrase (10) | 100% | 0 | 0.53 | 0.34 |
// | hard·paraphrase (8) | 0% | 4 | 0.00 | 0.00 |
// | 全库 (45) | 77.8% | 5 | 0.65 | 0.29 |
//
// 同表基线：`loose` 全库 78%·饿5·**p0.56**·f0.40；`legacy_gate` 49%·饿20·p0.69·f0.15。
//
// # 这两列各自防什么
//
// - **p（注入精度）**：实际进上下文的那些行里，属于本查询族的比例。防「为了召回把无关文本倒进
//   被缓存的 System 前缀」。
// - **f（预算利用）**：5 个槽位被真证据填掉多少。防「为了把 p 做高而什么都不注入」。
//
// ⚠️ **精度口径在本轮被纠正过一次**，所以新旧数字不可直接比。原先两条共用同一个累加
// `own / TOP_K`：分母固定为 5，于是「只投 1 行且投对」得 0.20，「投满 5 行、错 3 行」得 0.40 ——
// 那条指标把**饿死**和**投错**混成一个数，结构性地奖励注满预算。当时据此写的下限
// （全库 hit 85%、hard·paraphrase 40%、饿死 ≤1）全部是靠零重合哈希碰撞凑出来的：
// `hard·paraphrase` 那 50% 命中的同档精度只有 0.12（每投十行错八行），而「饿死 0」是
// 靠把无关行也塞进去实现的。**这两个数不是成绩，是缺陷的读数。**
// 拆成 p/f 两列后，同一份行为的第一次诚实测量就是上表：与朴素关键词基线**打平召回**
// （78% vs 78%）、**更高精度**（0.65 vs 0.56）、**更少注入量**（f 0.29 vs 0.40）——
// 相对截断买到的正是「不拿弱证据凑数」，这在旧口径下会被记成退步。
//
// ⚠️ 改这些常量只允许**上调**。下调一次就等于把「本次修复」抹掉一半，
// 而下一个人看不见任何信号 —— 这正是本轮要消灭的那类「静默回退」。

/// `tiered` 各档 hit@5 的下限（实测值留 7–10 个百分点余量）。
///
/// `hard·paraphrase` **故意不在这里**：夹具保证它与同族语料零实词重合，而
/// [`scoring::LOCAL_HASH_IS_SEMANTIC`] 为假时零重合就是 0 分，于是这一档按构造只能是 0%。
/// 给它写任何正数下限，等于给「靠哈希碰撞蒙对」留后门；它由
/// [`the_zero_overlap_gate_is_what_decides_hard_paraphrase`] 双向钉住。
const TIERED_MIN_HIT: &[(&str, &str, f64)] = &[
    ("en", "lexical", 0.90),
    ("zh", "lexical", 0.90),
    ("hard", "lexical", 0.90),
    ("en", "paraphrase", 0.80),
    ("zh", "paraphrase", 0.90),
];

/// 饿死的唯一合法理由：这条查询在库裡**确实**没有任何词面证据。
///
/// 所以判据不是「饿死要少」，而是「**不得比朴素关键词基线多饿**」——基线找不到的是
/// 语料问题，我们找不到才是规则问题。这条断言有牙：把 [`scoring::keeps`] 里那个
/// 已删除的 `HARD_FLOOR` 加回来（它按比例设线、专杀长查询），中文改述档立刻多饿一片。
#[test]
fn tiered_never_starves_beyond_where_the_keyword_baseline_starves() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let mut worse = Vec::new();
    for b in &buckets_in(&report) {
        let t = report.get(b.lang, b.kind, "tiered");
        let l = report.get(b.lang, b.kind, "loose");
        if t.starved > l.starved {
            worse.push(format!(
                "{}: tiered 饿死 {} 条 > 基线 {} 条",
                b.label(),
                t.starved,
                l.starved
            ));
        }
    }
    assert!(
        worse.is_empty(),
        "分层通路饿死了本可以召回的查询：{}\n{}",
        worse.join("; "),
        render_table(&report)
    );
}

/// 规格 §5 P0 的验收线：`en` ≥65%、`zh` ≥90%、hard 档不得比基线多饿、hard 档精度不低于基线。
///
/// ⚠️ 原规格写的「hard 档饿死 ≤3」已被本轮实测**撤回**：那个数假设零重合改述能靠余弦救回，
/// 而救回的代价是精度 0.12。改称「不得比基线多饿」是同一个意图的诚实版本（数值上 4 条，
/// 与基线完全相等——见上表的 `hard·paraphrase` 行）。规格已同步更正。
#[test]
fn p0_acceptance_numbers_from_the_spec_hold() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let en_lex = report.get("en", "lexical", "tiered");
    let zh_lex = report.get("zh", "lexical", "tiered");
    assert!(
        en_lex.hit_rate() >= 0.65,
        "P0 验收：en hit@5 {:.1}% < 65%",
        en_lex.hit_rate() * 100.0
    );
    assert!(
        zh_lex.hit_rate() >= 0.90,
        "P0 验收：zh hit@5 {:.1}% < 90%",
        zh_lex.hit_rate() * 100.0
    );
    let hard = report.agg("hard", "tiered");
    let hard_base = report.agg("hard", "loose");
    assert!(
        hard.starved <= hard_base.starved,
        "P0 验收：hard 档饿死 {} 条 > 基线 {} 条",
        hard.starved,
        hard_base.starved
    );
    assert!(
        hard.prec() + 1e-9 >= hard_base.prec(),
        "P0 验收：hard 档注入精度 {:.3} < 基线 {:.3}（不得为召回牺牲到基线以下）",
        hard.prec(),
        hard_base.prec()
    );
}

/// 零重合闸是 `hard·paraphrase` 的唯一决定者 —— **双向** tripwire。
///
/// 这一档的查询与同族语料零实词重合（由
/// [`hard_paraphrases_share_no_content_words_with_their_own_corpus`] 钉死），所以：
/// - 闸关着（默认后端无语义）时必须 **0%**：出现任何命中，说明有人把零重合候选放回上下文了；
/// - 闸打开（接了真嵌入模型）时必须 **>0%**：否则那个后端什么也没买到，白付延迟。
///
/// ⚠️ 翻 [`scoring::LOCAL_HASH_IS_SEMANTIC`] 时会在这里红 —— 那是设计好的：
/// 它要求改的人先重测本模块并更新棘轮，而不是让「接了语义」这件事静默改变检索行为。
#[test]
fn the_zero_overlap_gate_is_what_decides_hard_paraphrase() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let t = report.get("hard", "paraphrase", "tiered");
    assert!(
        t.queries >= 8,
        "hard·paraphrase 档只有 {} 条查询，不足以判这条闸",
        t.queries
    );
    if scoring::LOCAL_HASH_IS_SEMANTIC {
        assert!(
            t.hit_rate() > 0.0,
            "语义闸已打开，但 hard·paraphrase 仍 0%：这个嵌入后端没有带来任何召回"
        );
    } else {
        assert_eq!(
            t.hit_rate(),
            0.0,
            "零词面重合的改述拿到了 {:.1}% 命中——有候选绕过了 [`scoring::fuse`] 的零重合闸",
            t.hit_rate() * 100.0
        );
    }
}

/// 棘轮：逐档 hit@5 下限 + 全库饿死/精度/预算利用下限 + 不低于基线的精度。
#[test]
fn ratchet_floors_hold() {
    let fixture = load_fixture();
    let report = measure(&fixture);
    let mut broken = Vec::new();
    for (lang, kind, min) in TIERED_MIN_HIT {
        let c = report.get(lang, kind, "tiered");
        if c.hit_rate() < *min {
            broken.push(format!(
                "tiered {}·{}: {:.1}% < 下限 {:.1}%",
                lang,
                kind,
                c.hit_rate() * 100.0,
                min * 100.0
            ));
        }
    }
    let total = report.total("tiered");
    let base = report.total("loose");
    if total.hit_rate() < 0.75 {
        broken.push(format!(
            "tiered 全库: {:.1}% < 下限 75%",
            total.hit_rate() * 100.0
        ));
    }
    if total.prec() < 0.60 {
        broken.push(format!(
            "tiered 全库注入精度 {:.3} < 下限 0.60（实测 0.65）",
            total.prec()
        ));
    }
    if total.fill() < 0.25 {
        broken.push(format!(
            "tiered 全库预算利用 {:.3} < 下限 0.25（实测 0.29）：靠「什么都不注入」刷高精度就是这条在挡",
            total.fill()
        ));
    }
    // 相对判据不随指标口径过期：召回打平基线的前提下，注入的每一行必须更准。
    if total.hit_rate() < base.hit_rate() - 0.001 {
        broken.push(format!(
            "tiered 全库 {:.1}% 低于朴素关键词基线 {:.1}%",
            total.hit_rate() * 100.0,
            base.hit_rate() * 100.0
        ));
    }
    if total.prec() + 1e-9 < base.prec() {
        broken.push(format!(
            "tiered 注入精度 {:.3} 低于朴素关键词基线 {:.3}",
            total.prec(),
            base.prec()
        ));
    }
    assert!(
        broken.is_empty(),
        "检索质量棘轮被打破：\n{}\n{}",
        broken.join("\n"),
        render_table(&report)
    );
}

/// 只看**零词面重合**的配对，把余弦分布拆成「同族改述」与「异族无关」两堆。
///
/// 这是 [`scoring::LOCAL_HASH_IS_SEMANTIC`] 的证据来源：改述句彼此「本该语义相近」，
/// 无关句彼此「本该语义远离」，一个真有语义的通道必须能把两堆分开。
fn zero_overlap_cosines(fixture: &Fixture) -> (Vec<f64>, Vec<f64>) {
    let (docs, _base) = build_library(fixture);
    let (mut same, mut cross) = (Vec::new(), Vec::new());
    for (fi, fam) in fixture.families.iter().enumerate() {
        for q in fam
            .queries
            .iter()
            .filter(|q| q.kind.starts_with("paraphrase"))
        {
            let qv = LocalEmbedding::embed(&q.text);
            let qw = content_words(&q.text);
            for d in docs.iter() {
                let overlap = shared_content_words(&qw, &content_words(&d.text));
                if overlap > 0 {
                    continue; // 只看零重合对：有词面证据时根本不需要语义这一路
                }
                let cosine = LocalEmbedding::cosine(&qv, &LocalEmbedding::embed(&d.text));
                if d.family == fi {
                    same.push(cosine);
                } else {
                    cross.push(cosine);
                }
            }
        }
    }
    (same, cross)
}

fn quantile(v: &[f64], num: usize, den: usize) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((s.len() - 1) * num / den).min(s.len() - 1);
    s[idx]
}

/// **常驻测量**（不是指标，是判据）：默认的本地特征哈希**没有**语义。
///
/// 2026-10-29 实测（75 条语料 / 30 条零重合改述查询）：
/// - 同族改述 n=28 min=0.014 **p50=0.137** p90=0.431 max=0.474
/// - 异族无关 n=2095 min=0.000 **p50=0.104 p90=0.247** max=0.528，
///   其中 **207/2095（9.9%）的无关对余弦 ≥0.25**
///
/// 所以这里断言的是「**分不开**」：噪声尾部越过信号中位，信号中位连弱证据线都不到 ——
/// **不存在任何一个阈值**能把「相关」和「无关」切开，因此零重合的候选不配单独进上下文
/// （[`scoring::LOCAL_HASH_IS_SEMANTIC`] 为假，见 [`scoring`] 模块头「规则一」第三版）。
///
/// ⚠️ 接上真正的嵌入模型后这条会**变红**，那是设计好的闸：红 = 「语义通道现在真有意义了，
/// 去翻常量、重测 [`ratchet_floors_hold`] 的棘轮」，而不是「代码坏了」。
/// **只翻常量、不换后端**同样会红（常量与测量必须对得上），所以这条路没有「改一行就放行」的捷径。
#[test]
fn the_local_hash_channel_carries_no_semantic_signal() {
    let fixture = load_fixture();
    let (same, cross) = zero_overlap_cosines(&fixture);
    let fmt = |v: &[f64]| -> String {
        format!(
            "n={} min={:.3} p50={:.3} p90={:.3} max={:.3}",
            v.len(),
            quantile(v, 0, 1),
            quantile(v, 1, 2),
            quantile(v, 9, 10),
            quantile(v, 1, 1)
        )
    };
    eprintln!("零重合·同族改述 余弦: {}", fmt(&same));
    eprintln!("零重合·异族无关 余弦: {}", fmt(&cross));

    assert!(
        same.len() >= 20 && cross.len() >= 500,
        "样本量不足以支撑这条判据：同族 {} 条、异族 {} 条（夹具退化了）",
        same.len(),
        cross.len()
    );
    let same_p50 = quantile(&same, 1, 2);
    let cross_p90 = quantile(&cross, 9, 10);
    if scoring::LOCAL_HASH_IS_SEMANTIC {
        // 常量声称「零重合的余弦可以单独进上下文」。这句话必须由**同一批测量**证实：
        // 真改述的中位数要越过无关噪声的 p90，否则那个 `true` 是凭空签字。
        // 没有这道闸，改一行常量就能让下面所有断言继续绿、同时把碰撞当语义。
        assert!(
            same_p50 > cross_p90,
            "LOCAL_HASH_IS_SEMANTIC=true，但实测仍然分不开：同族 p50={:.3} ≤ 无关 p90={:.3}。\n\
             → 要么把常量改回 false，要么先换掉 LocalEmbedding 的实现再重测这张表。",
            same_p50,
            cross_p90
        );
    }
    assert!(
        cross_p90 > same_p50,
        "无关噪声的 p90({:.3}) 已不再越过真改述的 p50({:.3})：语义通道可能真的有意义了。\n\
         → 翻 scoring::LOCAL_HASH_IS_SEMANTIC，并按新的分布重测棘轮。",
        cross_p90,
        same_p50
    );
    assert!(
        same_p50 < WEAK_EVIDENCE,
        "真改述的余弦中位数已到 {:.3}（≥ 弱证据线 {:.2}）：语义通道可能真的有意义了。\n\
         → 翻 scoring::LOCAL_HASH_IS_SEMANTIC，并按新的分布重测棘轮。",
        same_p50,
        WEAK_EVIDENCE
    );
    // 反向自证：不是「整个通道一片死寂所以分开」——无关那堆里确实有成批的高分碰撞
    let noisy = cross.iter().filter(|c| **c >= 0.25).count();
    assert!(
        noisy * 50 >= cross.len(),
        "无关对里余弦 ≥0.25 的只剩 {}/{}：碰撞噪声几乎消失，判据该重写了",
        noisy,
        cross.len()
    );
}

// ───────────── 常驻测量：FTS5 的连接语义该选哪个 ─────────────
//
// 规格 §D4 的验收线是「`zh lexical` 的 fts5 hit@5 从 0.0% 到 ≥80%」，而上面那张表里
// 原本**没有 FTS 档**：这条线此前无人测量，`match_expression` 用 AND 是我自己拍的。
// 拍的那一遍在真实库上留了证据：`"技术方案"` 命中 5 条、`"技术方案设计"` 命中 **0** 条 ——
// 因为中文分词产出「单字 + 相邻双字」，AND 链要求**每个**双字都在文档里相邻出现，
// 于是 AND-over-bigram 事实上是**短语匹配**：自然语言查询多带一个词就全丢。
//
// 这段回答的问题与它的答案一起常驻，因为选定之后**真正的风险是漂移**：
// `Production` 档直接调用线上的 [`short_term::match_expression`]，所以线上口径一旦改动
// （换连接符、去掉实词过滤、改截断），下面那张表里它那一行就跟着动，
// 而 `d4_fts_join_mode_measurement` 末尾的棘轮断言会当场拦住。
// 跑法：
// `cargo test -p marguerite-core --lib memory::recall_probe::d4_fts_join_mode_measurement -- --exact --nocapture`

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum JoinMode {
    /// 首版拍出来的口径（AND + 全词元）—— 实测全库 0 命中，留在这里是为了拦住「改回去」。
    And,
    /// OR + 全词元：召回够了，但停用词碰撞把注入精度拉到 0.40 一档。
    Or,
    /// AND 不中再 OR 兜底。
    AndThenOr,
    /// OR + 实词（保留中文单字）。
    OrContent,
    /// **线上口径**：`short_term::match_expression` 本身（OR + 实词 + 去中文单字）。
    Production,
}

impl JoinMode {
    fn label(self) -> &'static str {
        match self {
            JoinMode::And => "and",
            JoinMode::Or => "or",
            JoinMode::AndThenOr => "and_then_or",
            JoinMode::OrContent => "or_content",
            JoinMode::Production => "production",
        }
    }
    const ALL: [JoinMode; 5] = [
        JoinMode::And,
        JoinMode::Or,
        JoinMode::AndThenOr,
        JoinMode::OrContent,
        JoinMode::Production,
    ];
}

/// 夹具语料的 FTS5 库（临时目录，Drop 时删除；**绝不碰真实 data_dir**）。
struct FtsStore {
    mem: ShortTermMemory,
    dir: std::path::PathBuf,
}

impl FtsStore {
    fn with_corpus(docs: &[Doc]) -> Self {
        let dir = std::env::temp_dir().join(format!("mgt-fts-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("临时目录必须能建");
        let mut mem = ShortTermMemory::new(&dir.join("short_term.db"))
            .with_max_sessions(docs.len() + 16);
        mem.init().expect("FTS 夹具库初始化失败");
        for (i, d) in docs.iter().enumerate() {
            mem.store_session(&i.to_string(), &d.text, &[])
                .expect("语料入库失败");
        }
        Self { mem, dir }
    }

    fn search(&self, expression: &str) -> Vec<usize> {
        if expression.is_empty() {
            // 与线上同一条口径：没有词元就没有条件，绝不退化成「匹配全部」。
            return Vec::new();
        }
        self.mem
            .search_match(expression, TOP_K)
            .expect("检索失败")
            .into_iter()
            .map(|s| s.session_id.parse::<usize>().expect("session_id 就是 doc 下标"))
            .collect()
    }
}

impl Drop for FtsStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fts_retrieve(store: &FtsStore, query: &str, mode: JoinMode) -> Vec<usize> {
    // 全部档位都走 short_term 里的那几条管道，「测的」与「跑的」只差连接符与词元口径，
    // 不会同时差两层（否则这张表测的就不是线上行为）。
    let all = short_term::query_terms(query);
    match mode {
        JoinMode::And => store.search(&short_term::to_expression(&all, " AND ")),
        JoinMode::Or => store.search(&short_term::to_expression(&all, " OR ")),
        JoinMode::AndThenOr => {
            let mut ids = store.search(&short_term::to_expression(&all, " AND "));
            if ids.len() < TOP_K {
                for id in store.search(&short_term::to_expression(&all, " OR ")) {
                    if ids.len() >= TOP_K {
                        break;
                    }
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            ids
        }
        JoinMode::OrContent => {
            let kept = short_term::content_terms(query, true);
            let sel: &[String] = if kept.is_empty() { &all } else { &kept };
            store.search(&short_term::to_expression(sel, " OR "))
        }
        JoinMode::Production => short_term::match_expression(query)
            .map(|expression| store.search(&expression))
            .unwrap_or_default(),
    }
}

/// §D4 选定口径（= 线上 `match_expression`）的**棘轮下限**，全部来自实测并留了余量。
///
/// 为什么这里需要断言而不只是打印：这张表跑完如果不钉住，下一次改动（换连接符、
/// 去掉实词过滤、把中文单字放回去）会让线上口径静默变差，而「我测过」这句话
/// 在下一个会话里无法反驳 —— 与 [`ratchet_floors_hold`] 是同一条教训。
const D4_MIN_HIT: f64 = 0.72; // 实测 0.756
const D4_MIN_PREC: f64 = 0.60; // 实测 0.663
const D4_MAX_STARVED: usize = 8; // 实测 6 / 45 条
/// 规格的验收线：`zh lexical` 的 fts5 hit@5 从 0.0% 提到 ≥80%（实测 100%）。
const D4_ZH_LEXICAL_MIN_HIT: f64 = 0.80;
/// 「实词 + 去中文单字」相对「全词元 OR」必须买到的精度差（实测 0.663 vs 0.409）。
const D4_PREC_MARGIN_OVER_ALL_TOKENS: f64 = 0.10;

#[test]
fn d4_fts_join_mode_measurement() {
    use std::fmt::Write as _;
    let fixture = load_fixture();
    let (docs, _base) = build_library(&fixture);
    let store = FtsStore::with_corpus(&docs);
    let mut buckets: Vec<Bucket> = Vec::new();
    for fam in &fixture.families {
        for q in &fam.queries {
            let b = bucket_of(fam, &q.kind);
            if !buckets.contains(&b) {
                buckets.push(b);
            }
        }
    }
    buckets.sort();

    let mut out = String::new();
    let baseline = measure(&fixture);
    let _ = writeln!(
        out,
        "FTS5 连接语义测量（{} 条语料 / {} 档 / K={TOP_K}）",
        docs.len(),
        buckets.len()
    );
    // 参照系：同一个夹具、同一批查询、同一个 K 下，已有两条通路的读数。
    // 没有这两行，下面的数字只是数字——「hit 88.9%」是好是坏，取决于跟谁比。
    for name in ["loose", "tiered"] {
        let _ = writeln!(out, "\n参照 {name}:");
        for bucket in &buckets {
            let c = baseline.get(bucket.lang, bucket.kind, name);
            let _ = writeln!(
                out,
                "  {:<16} n={:<3} hit={:.1}% starved={} prec={:.3} fill={:.3}",
                format!("{}-{}", bucket.lang, bucket.kind),
                c.queries,
                c.hit_rate() * 100.0,
                c.starved,
                c.prec(),
                c.fill()
            );
        }
        let t = baseline.total(name);
        let _ = writeln!(
            out,
            "  {:<16} n={:<3} hit={:.1}% starved={} prec={:.3} fill={:.3}",
            "TOTAL",
            t.queries,
            t.hit_rate() * 100.0,
            t.starved,
            t.prec(),
            t.fill()
        );
    }
    let mut misses: Vec<String> = Vec::new();
    let mut mode_totals: Vec<(&'static str, Cells)> = Vec::new();
    let mut production_cells: Vec<(Bucket, Cells)> = Vec::new();
    for mode in JoinMode::ALL {
        let mut total = Cells::default();
        let _ = writeln!(out, "\n== mode {} ==", mode.label());
        for bucket in &buckets {
            let mut c = Cells::default();
            for (fi, fam) in fixture.families.iter().enumerate() {
                for q in &fam.queries {
                    if bucket_of(fam, &q.kind) != *bucket {
                        continue;
                    }
                    let hits = fts_retrieve(&store, &q.text, mode);
                    let own = hits.iter().filter(|i| docs[**i].family == fi).count();
                    c.queries += 1;
                    if own > 0 {
                        c.hit += 1;
                    } else {
                        misses.push(format!(
                            "[{}|{}|{}] {}",
                            mode.label(),
                            fam.id,
                            q.kind,
                            truncate(&q.text, 70)
                        ));
                    }
                    if hits.is_empty() {
                        c.starved += 1;
                    } else {
                        c.prec_sum += own as f64 / hits.len() as f64;
                        c.scored += 1;
                    }
                    c.fill_sum += own as f64;
                }
            }
            let _ = writeln!(
                out,
                "{:<16} n={:<3} hit={:.1}% starved={} prec={:.3} fill={:.3}",
                format!("{}-{}", bucket.lang, bucket.kind),
                c.queries,
                c.hit_rate() * 100.0,
                c.starved,
                c.prec(),
                c.fill()
            );
            if mode == JoinMode::Production {
                production_cells.push((*bucket, c));
            }
            total.merge(&c);
        }
        let _ = writeln!(
            out,
            "{:<16} n={:<3} hit={:.1}% starved={} prec={:.3} fill={:.3}",
            "TOTAL",
            total.queries,
            total.hit_rate() * 100.0,
            total.starved,
            total.prec(),
            total.fill()
        );
        mode_totals.push((mode.label(), total));
    }
    let _ = writeln!(out, "\n== 未命中明细 ==\n{}", misses.join("\n"));
    let path = std::env::temp_dir().join("d4_fts_join_modes.txt");
    std::fs::write(&path, &out).expect("报告写入失败");
    eprintln!("FTS join-mode report -> {}", path.display());

    let row = |name: &str| -> Cells {
        mode_totals
            .iter()
            .find(|(label, _)| *label == name)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("档位 {name} 没有被测量（JoinMode::ALL 少了一条）"))
    };
    let production = row("production");
    // 断言的是「线上口径」，不是六条里随便哪条：`Production` 档调的就是
    // `short_term::match_expression`，所以线上改一行，这里就会跟着红。
    assert_eq!(
        production.queries,
        row("and").queries,
        "每条查询都必须在每个档位下各测一次"
    );
    assert!(
        production.hit_rate() >= D4_MIN_HIT,
        "线上口径 hit@5 = {:.1}%（下限 {:.1}%）：\n
         → 词元管道或连接符被动过了，去读 d4_fts_join_modes.txt 的 production 段",
        production.hit_rate() * 100.0,
        D4_MIN_HIT * 100.0
    );
    assert!(
        production.prec() >= D4_MIN_PREC,
        "线上口径注入精度 = {:.3}（下限 {:.2}）：\
         中文单字或停用词大概又被放回查询里了",
        production.prec(),
        D4_MIN_PREC
    );
    assert!(
        production.starved <= D4_MAX_STARVED,
        "线上口径饿死 {} 条（上限 {}）：饿死是「什么都不注入」，\n
         比注入错东西更难被发现",
        production.starved,
        D4_MAX_STARVED
    );

    // 规格的验收线：中文词面查询从「0 命中」变成「≥80%」。
    let zh_lexical = production_cells
        .iter()
        .find(|(b, _)| b.lang == "zh" && b.kind == "lexical")
        .map(|(_, c)| *c)
        .expect("zh-lexical 档必须被测量");
    assert!(
        zh_lexical.hit_rate() >= D4_ZH_LEXICAL_MIN_HIT,
        "§D4 验收线：zh lexical 的 fts5 hit@5 从 0.0% 提到 ≥{:.0}%，实测 {:.1}%。\n
         → 中文又检索不回来了（先查 normalize_for_index 与分词口径是否漂移）",
        D4_ZH_LEXICAL_MIN_HIT * 100.0,
        zh_lexical.hit_rate() * 100.0
    );

    // 三条「口径之间」的相对线：不依赖语料的真实性，绝对数过期了它们仍然成立。
    assert!(
        row("and").hit_rate() <= 0.05,
        "AND + 全词元 现在有 {:.1}% 命中。这条断言存在的意义就是「AND 是死口径」\
         （实测 0/45）；如果它真的能命中了，说明连接符该重新选 —— 去重测这张表，别直接改下限",
        row("and").hit_rate() * 100.0
    );
    assert!(
        production.prec() >= row("or").prec() + D4_PREC_MARGIN_OVER_ALL_TOKENS,
        "实词过滤失效：线上精度 {:.3} 相对全词元 OR {:.3} 的差不足 {:.2}；\
         全词元 OR 的「命中」里有一批是停用词碰撞（hard·paraphrase 精度 0.125）",
        production.prec(),
        row("or").prec(),
        D4_PREC_MARGIN_OVER_ALL_TOKENS
    );
    assert!(
        production.prec() > row("or_content").prec(),
        "去中文单字失效：线上精度 {:.3} ≤ 保留单字的实词口径 {:.3}（实测 0.663 vs 0.547）",
        production.prec(),
        row("or_content").prec()
    );
}
