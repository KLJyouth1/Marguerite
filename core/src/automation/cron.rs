//! cron 调度器
//!
//! 解析标准 5 字段 cron 表达式（分 时 日 月 周），计算下一次触发时间，
//! 并强制最小触发间隔为 10 分钟以防滥用。
//!
//! 支持语法：
//! - `*`            任意值
//! - `n`            单值
//! - `n-m`          范围（含端点）
//! - `n,m,o`        列表
//! - `*/n`          步长（在全部取值上按 n 步进）

use std::fmt;

use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Timelike, Utc};

/// 最小允许触发间隔（10 分钟）
pub const MIN_INTERVAL: Duration = Duration::minutes(10);

/// 单一 cron 字段的取值集合
#[derive(Debug, Clone)]
struct Field {
    values: Vec<u8>,
    star: bool,
}

impl Field {
    fn all(min: u8, max: u8) -> Self {
        Self {
            values: (min..=max).collect(),
            star: true,
        }
    }

    fn parse(s: &str, min: u8, max: u8) -> Result<Self> {
        if s == "*" {
            return Ok(Self::all(min, max));
        }
        let (body, step) = match s.split_once('/') {
            Some((b, st)) => (
                b,
                Some(
                    st.parse::<u8>()
                        .map_err(|_| anyhow!("步长解析失败: {st}"))?,
                ),
            ),
            None => (s, None),
        };
        let step = step.unwrap_or(1);
        if step == 0 {
            return Err(anyhow!("步长不能为 0"));
        }

        let mut values = Vec::new();
        if body == "*" {
            let mut v = min;
            while v <= max {
                values.push(v);
                v = v.saturating_add(step);
            }
        } else if body.contains(',') {
            if step != 1 {
                return Err(anyhow!("列表字段不支持步长: {s}"));
            }
            for part in body.split(',') {
                values.push(parse_scalar(part, min, max)?);
            }
        } else if let Some((lo, hi)) = body.split_once('-') {
            let lo = parse_scalar(lo, min, max)?;
            let hi = parse_scalar(hi, min, max)?;
            if lo > hi {
                return Err(anyhow!("范围下限大于上限: {s}"));
            }
            let mut v = lo;
            while v <= hi {
                values.push(v);
                v = v.saturating_add(step);
                if v < lo {
                    break;
                }
            }
        } else {
            if step != 1 {
                return Err(anyhow!("单值字段不支持步长: {s}"));
            }
            values.push(parse_scalar(body, min, max)?);
        }
        Ok(Self {
            values,
            star: false,
        })
    }

    fn matches(&self, v: u8) -> bool {
        self.values.contains(&v)
    }

    fn is_star(&self) -> bool {
        self.star
    }
}

fn parse_scalar(s: &str, min: u8, max: u8) -> Result<u8> {
    let v: u8 = s.parse().map_err(|_| anyhow!("字段值解析失败: {s}"))?;
    if v < min || v > max {
        return Err(anyhow!("字段值 {v} 超出范围 [{min},{max}]"));
    }
    Ok(v)
}

/// 已解析的 5 字段 cron 规格
#[derive(Debug, Clone)]
struct CronSpec {
    minute: Field,
    hour: Field,
    dom: Field,
    month: Field,
    dow: Field,
}

impl CronSpec {
    fn parse(expr: &str) -> Result<Self> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(anyhow!(
                "cron 表达式需为 5 字段（分 时 日 月 周），收到 {} 字段: {expr}",
                parts.len()
            ));
        }
        let minute = Field::parse(parts[0], 0, 59)?;
        let hour = Field::parse(parts[1], 0, 23)?;
        let dom = Field::parse(parts[2], 1, 31)?;
        let month = Field::parse(parts[3], 1, 12)?;
        // 周字段允许 0-7（7 视同周日）
        let mut dow = Field::parse(parts[4], 0, 7)?;
        if dow.values.contains(&7) && !dow.values.contains(&0) {
            dow.values.push(0);
        }
        Ok(Self {
            minute,
            hour,
            dom,
            month,
            dow,
        })
    }

    /// 日/周组合匹配（Vixie cron 规则：两者都受限时为 OR，任一受限时为 AND）
    fn day_matches(&self, dt: &DateTime<Utc>) -> bool {
        let dom_star = self.dom.is_star();
        let dow_star = self.dow.is_star();
        let dom_ok = self.dom.matches(dt.day() as u8);
        let dow_ok = self.dow.matches(dt.weekday().num_days_from_sunday() as u8);
        match (dom_star, dow_star) {
            (true, true) => true,
            (true, false) => dow_ok,
            (false, true) => dom_ok,
            (false, false) => dom_ok || dow_ok,
        }
    }

    /// 当前时刻是否命中该 cron
    fn matches_now(&self, now: DateTime<Utc>) -> bool {
        self.minute.matches(now.minute() as u8)
            && self.hour.matches(now.hour() as u8)
            && self.day_matches(&now)
            && self.month.matches(now.month() as u8)
    }

    /// 计算 after 之后（严格大于）的下一个命中时刻
    fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        // 从 after 之后第一分钟的开始处起算
        let inc = after + Duration::minutes(1);
        let mut c = inc
            .date_naive()
            .and_hms_opt(inc.hour(), inc.minute(), 0)
            .map(|nt| DateTime::<Utc>::from_naive_utc_and_offset(nt, Utc))?;

        for _ in 0..1_000_000 {
            if c.year() > after.year() + 6 {
                return None;
            }
            if !self.month.matches(c.month() as u8) {
                c = advance_month(&self.month, c)?;
                continue;
            }
            if !self.day_matches(&c) {
                c = advance_day(c);
                continue;
            }
            if !self.hour.matches(c.hour() as u8) {
                c = advance_hour(&self.hour, c)?;
                continue;
            }
            if !self.minute.matches(c.minute() as u8) {
                c = advance_minute(&self.minute, c)?;
                continue;
            }
            return Some(c);
        }
        None
    }
}

fn advance_month(month_field: &Field, c: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut y = c.year();
    let mut mo = c.month();
    loop {
        mo += 1;
        if mo > 12 {
            mo = 1;
            y += 1;
        }
        if month_field.matches(mo as u8) {
            break;
        }
        if y > c.year() + 10 {
            return None;
        }
    }
    let nd = NaiveDate::from_ymd_opt(y, mo, 1)?;
    let nt = nd.and_hms_opt(0, 0, 0)?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(nt, Utc))
}

fn advance_day(c: DateTime<Utc>) -> DateTime<Utc> {
    let next = c + Duration::days(1);
    let nt = next.date_naive().and_hms_opt(0, 0, 0).unwrap();
    DateTime::<Utc>::from_naive_utc_and_offset(nt, Utc)
}

fn advance_hour(hour_field: &Field, c: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut cand = c + Duration::hours(1);
    loop {
        let nt = cand.date_naive().and_hms_opt(cand.hour(), 0, 0)?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(nt, Utc);
        if hour_field.matches(dt.hour() as u8) {
            return Some(dt);
        }
        cand += Duration::hours(1);
        if cand.year() > c.year() + 6 {
            return None;
        }
    }
}

fn advance_minute(min_field: &Field, c: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let mut cand = c + Duration::minutes(1);
    loop {
        let nt = cand
            .date_naive()
            .and_hms_opt(cand.hour(), cand.minute(), 0)?;
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(nt, Utc);
        if min_field.matches(dt.minute() as u8) {
            return Some(dt);
        }
        cand += Duration::minutes(1);
        if cand.year() > c.year() + 6 {
            return None;
        }
    }
}

/// cron 调度器
///
/// 持有已解析的表达式，提供 next_after / should_run。
/// 通过 [`CronScheduler::parse`] 构造，解析阶段即强制最小间隔约束。
#[derive(Debug, Clone)]
pub struct CronScheduler {
    spec: CronSpec,
}

impl CronScheduler {
    /// 解析 cron 表达式；若最小触发间隔小于 10 分钟则拒绝
    pub fn parse(expr: &str) -> Result<Self> {
        let spec = CronSpec::parse(expr)?;
        let sched = Self { spec };
        if let Some(gap) = sched.min_interval() {
            if gap < MIN_INTERVAL {
                return Err(anyhow!(
                    "cron 表达式最小触发间隔约 {} 分钟，小于平台下限 {} 分钟，已拒绝以防滥用",
                    gap.num_minutes(),
                    MIN_INTERVAL.num_minutes()
                ));
            }
        }
        Ok(sched)
    }

    /// 计算 after 之后下一次触发时刻
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.spec.next_after(after)
    }

    /// 当前时刻是否应触发
    pub fn should_run(&self, now: DateTime<Utc>) -> bool {
        self.spec.matches_now(now)
    }

    /// 计算连续多次触发之间的最小间隔（用于间隔约束校验）
    fn min_interval(&self) -> Option<Duration> {
        let now = Utc::now();
        let mut cur = now;
        let mut prev: Option<DateTime<Utc>> = None;
        let mut min_gap = Duration::days(365 * 10);
        for _ in 0..6 {
            match self.spec.next_after(cur) {
                Some(next) => {
                    if let Some(p) = prev {
                        let gap = next - p;
                        if gap < min_gap {
                            min_gap = gap;
                        }
                    }
                    prev = Some(next);
                    cur = next;
                }
                None => break,
            }
        }
        if prev.is_some() {
            Some(min_gap)
        } else {
            None
        }
    }
}

impl fmt::Display for CronScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CronScheduler")
    }
}

/// 定时任务类型。
pub enum JobKind {
    /// 自演化任务：到点触发 `HookEvent::SelfEvolve` 闭环（评测→变异→门控）。
    SelfEvolve,
    /// 具名任务（如外部脚本/命令标识）。
    Named(String),
}

impl JobKind {
    /// 稳定字符串标识。
    pub fn as_str(&self) -> &str {
        match self {
            JobKind::SelfEvolve => "self_evolve",
            JobKind::Named(n) => n.as_str(),
        }
    }
}

/// 一条带调度计划的定时任务。
pub struct CronJob {
    /// 调度计划（已强制最小间隔约束）
    pub schedule: CronScheduler,
    /// 任务类型
    pub kind: JobKind,
}

impl CronJob {
    /// 注册一个自演化定时任务（表达式须满足最小间隔）。
    pub fn self_evolve(expr: &str) -> Result<Self> {
        Ok(Self {
            schedule: CronScheduler::parse(expr)?,
            kind: JobKind::SelfEvolve,
        })
    }

    /// 注册一个具名定时任务。
    pub fn named(expr: &str, name: &str) -> Result<Self> {
        Ok(Self {
            schedule: CronScheduler::parse(expr)?,
            kind: JobKind::Named(name.to_string()),
        })
    }

    /// 当前时刻是否应触发该任务。
    pub fn should_run(&self, now: DateTime<Utc>) -> bool {
        self.schedule.should_run(now)
    }
}

/// 定时任务注册表：持有多条 `CronJob`，按当前时刻筛选到期任务。
#[derive(Default)]
pub struct JobRegistry {
    jobs: Vec<CronJob>,
}

impl JobRegistry {
    /// 空注册表。
    pub fn new() -> Self {
        Self { jobs: Vec::new() }
    }

    /// 注册一条任务。
    pub fn register(&mut self, job: CronJob) {
        self.jobs.push(job);
    }

    /// 当前时刻到期的任务（按注册顺序）。
    pub fn due_jobs(&self, now: DateTime<Utc>) -> Vec<&CronJob> {
        self.jobs.iter().filter(|j| j.should_run(now)).collect()
    }

    /// 注册表内任务数（可观测）。
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s)
            .single()
            .expect("valid datetime")
    }

    #[test]
    fn parse_every_2h_next_is_2h_later() {
        let s = CronScheduler::parse("0 */2 * * *").expect("valid");
        // 14:00 是命中时刻，next_after 严格大于 → 16:00
        let next = s.next_after(dt(2026, 1, 1, 14, 0, 0)).expect("has next");
        assert_eq!(next, dt(2026, 1, 1, 16, 0, 0));
        // 15:00 之后也应得到 16:00
        let next2 = s.next_after(dt(2026, 1, 1, 15, 0, 0)).expect("has next");
        assert_eq!(next2, dt(2026, 1, 1, 16, 0, 0));
    }

    #[test]
    fn reject_every_minute() {
        assert!(CronScheduler::parse("* * * * *").is_err());
    }

    #[test]
    fn reject_every_5_min() {
        assert!(CronScheduler::parse("*/5 * * * *").is_err());
    }

    #[test]
    fn accept_every_10_min() {
        assert!(CronScheduler::parse("*/10 * * * *").is_ok());
    }

    #[test]
    fn should_run_true_and_false() {
        let s = CronScheduler::parse("0 * * * *").expect("valid");
        assert!(s.should_run(dt(2026, 1, 1, 10, 0, 0)));
        assert!(!s.should_run(dt(2026, 1, 1, 10, 30, 0)));
        // 09:00 同为 minute-0，应命中（原断言 !should_run 写错，与上一行自相矛盾）
        assert!(s.should_run(dt(2026, 1, 1, 9, 0, 0)));
    }

    #[test]
    fn next_midnight() {
        let s = CronScheduler::parse("0 0 * * *").expect("valid");
        let next = s.next_after(dt(2026, 1, 1, 10, 0, 0)).expect("has next");
        assert_eq!(next, dt(2026, 1, 2, 0, 0, 0));
    }

    #[test]
    fn next_weekday_930() {
        // 2026-01-03 是周六
        let s = CronScheduler::parse("30 9 * * 1-5").expect("valid");
        let next = s.next_after(dt(2026, 1, 3, 12, 0, 0)).expect("has next");
        assert_eq!(next, dt(2026, 1, 5, 9, 30, 0));
    }

    #[test]
    fn next_weekly_sunday() {
        // 2026-01-01 是周四，下一个周日 2026-01-04
        let s = CronScheduler::parse("0 0 * * 0").expect("valid");
        let next = s.next_after(dt(2026, 1, 1, 10, 0, 0)).expect("has next");
        assert_eq!(next, dt(2026, 1, 4, 0, 0, 0));
    }

    #[test]
    fn reject_out_of_range() {
        assert!(CronScheduler::parse("99 * * * *").is_err());
        assert!(CronScheduler::parse("0 99 * * *").is_err());
    }

    #[test]
    fn reject_wrong_field_count() {
        assert!(CronScheduler::parse("0 * * *").is_err());
        assert!(CronScheduler::parse("0 * * * * *").is_err());
        assert!(CronScheduler::parse("").is_err());
    }

    #[test]
    fn list_field_matches() {
        let s = CronScheduler::parse("1,15 * * * *").expect("valid");
        assert!(s.should_run(dt(2026, 1, 1, 8, 1, 0)));
        assert!(s.should_run(dt(2026, 1, 1, 8, 15, 0)));
        assert!(!s.should_run(dt(2026, 1, 1, 8, 2, 0)));
        let next = s.next_after(dt(2026, 1, 1, 8, 0, 0)).expect("has next");
        assert_eq!(next, dt(2026, 1, 1, 8, 1, 0));
    }

    #[test]
    fn range_field_step() {
        // 每 2 小时：0,2,4,...22
        let s = CronScheduler::parse("0 0-22/2 * * *").expect("valid");
        assert!(s.should_run(dt(2026, 1, 1, 4, 0, 0)));
        assert!(!s.should_run(dt(2026, 1, 1, 5, 0, 0)));
    }

    #[test]
    fn dom_dow_or_rule() {
        // 每月 1 号 或 周一 0:00
        let s = CronScheduler::parse("0 0 1 * 1").expect("valid");
        // 2026-01-05 是周一 → 命中（dow）
        assert!(s.should_run(dt(2026, 1, 5, 0, 0, 0)));
        // 2026-01-01 是 1 号 → 命中（dom）
        assert!(s.should_run(dt(2026, 1, 1, 0, 0, 0)));
        // 2026-01-02 周五非 1 号 → 不命中
        assert!(!s.should_run(dt(2026, 1, 2, 0, 0, 0)));
    }

    // ---- 自演化定时任务注册 ----

    #[test]
    fn job_registry_self_evolve_due_at_hour() {
        let mut reg = JobRegistry::new();
        reg.register(CronJob::self_evolve("0 * * * *").expect("valid"));
        assert_eq!(reg.len(), 1);
        let now = dt(2026, 1, 1, 10, 0, 0);
        assert_eq!(reg.due_jobs(now).len(), 1, "整点应到期");
        let off = dt(2026, 1, 1, 10, 30, 0);
        assert_eq!(reg.due_jobs(off).len(), 0, "非整点不应到期");
    }

    #[test]
    fn self_evolve_job_rejects_too_frequent() {
        assert!(
            CronJob::self_evolve("* * * * *").is_err(),
            "每分钟自演化须被拒绝（最小间隔约束）"
        );
    }

    #[test]
    fn job_kind_str_roundtrip() {
        let e = CronJob::self_evolve("0 0 * * *").unwrap();
        assert_eq!(e.kind.as_str(), "self_evolve");
        let n = CronJob::named("0 0 * * *", "backup").unwrap();
        assert_eq!(n.kind.as_str(), "backup");
    }
}
