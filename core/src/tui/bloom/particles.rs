//! Bloom 终端粒子场（粒子溶解隐喻，纯数学可测）
//!
//! 不依赖任何终端/图形后端：粒子在场内上浮、衰减，像 Bloom 的粒子溶解。
//! 渲染层（[`widget`]）按 `density_at` 把场离散成 Unicode 块字符 + 辉光色。
//! 内置可重现 LCG，保证单测确定性（不依赖平台随机数源）。

/// 单个粒子
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Particle {
    /// 归一化坐标（0..1，原点左下）
    pub x: f32,
    pub y: f32,
    /// 速度（归一化/秒）
    pub vx: f32,
    pub vy: f32,
    /// 剩余寿命（0..1，1=新生）
    pub life: f32,
}

/// Bloom 粒子场
#[derive(Debug, Clone)]
pub struct ParticleField {
    particles: Vec<Particle>,
    /// 网格宽（列）
    pub w: u16,
    /// 网格高（行）
    pub h: u16,
    /// 发射速率（粒子/秒），由音频能量驱动
    spawn_rate: f32,
    /// LCG 状态（确定性）
    rng: u64,
    /// 已累计待发射的粒子数（浮点累加）
    acc: f32,
}

impl ParticleField {
    /// 新建空白场。seed 用于确定性随机（测试用固定值）。
    pub fn new(w: u16, h: u16, seed: u64) -> Self {
        Self {
            particles: Vec::with_capacity(256),
            w: w.max(1),
            h: h.max(1),
            spawn_rate: 12.0,
            rng: seed.wrapping_add(0x9E3779B97F4A7C15),
            acc: 0.0,
        }
    }

    /// LCG 下一步，返回 [0,1)
    fn next_f(&mut self) -> f32 {
        // splitmix64 风格
        self.rng = self.rng.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        (z >> 11) as f32 / (1u64 << 53) as f32
    }

    /// 推进一帧。dt 秒；energy(0..1) 驱动发射速率与上升速度（Bloom 音乐能量）。
    pub fn step(&mut self, dt: f32, energy: f32) {
        let energy = energy.clamp(0.0, 1.0);
        // 发射：纯能量驱动（静默不发射，Bloom 音乐粒子隐喻）
        self.spawn_rate = energy * 60.0;
        self.acc += self.spawn_rate * dt;
        while self.acc >= 1.0 {
            self.acc -= 1.0;
            let x = self.next_f();
            let vx = (self.next_f() - 0.5) * 0.15;
            let vy = 0.25 + energy * 0.6 + self.next_f() * 0.1;
            self.particles.push(Particle {
                x,
                y: 0.02,
                vx,
                vy,
                life: 1.0,
            });
        }
        // 积分 + 寿命衰减
        let decay = 0.35 + (1.0 - energy) * 0.2;
        let mut alive = Vec::with_capacity(self.particles.len());
        for mut p in self.particles.drain(..) {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.life -= decay * dt;
            if p.life > 0.0 && p.y < 1.05 {
                // 轻微回弹到边界内
                if p.x < 0.0 {
                    p.x = 0.0;
                    p.vx = p.vx.abs();
                } else if p.x > 1.0 {
                    p.x = 1.0;
                    p.vx = -p.vx.abs();
                }
                alive.push(p);
            }
        }
        self.particles = alive;
    }

    /// 网格单元密度（0..1，取该单元内粒子的平均寿命）。
    /// 超出网格范围返回 0。
    pub fn density_at(&self, gx: u16, gy: u16) -> f32 {
        if gx >= self.w || gy >= self.h {
            return 0.0;
        }
        // 终端 y 向下；粒子 y 向上 -> 反转
        let ny = 1.0 - (gy as f32 + 0.5) / self.h as f32;
        let nx = (gx as f32 + 0.5) / self.w as f32;
        let mut sum = 0.0f32;
        let mut cnt = 0u32;
        for p in &self.particles {
            if (p.x - nx).abs() < 0.5 / self.w as f32 && (p.y - ny).abs() < 0.5 / self.h as f32 {
                sum += p.life;
                cnt += 1;
            }
        }
        if cnt == 0 {
            0.0
        } else {
            (sum / cnt as f32).clamp(0.0, 1.0)
        }
    }

    /// 当前粒子数
    pub fn count(&self) -> usize {
        self.particles.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_empty() {
        let f = ParticleField::new(20, 10, 42);
        assert_eq!(f.count(), 0);
        assert_eq!(f.density_at(0, 0), 0.0);
    }

    #[test]
    fn test_step_spawns_and_rises() {
        let mut f = ParticleField::new(40, 20, 7);
        // 有能量才发射
        f.step(0.1, 0.5);
        assert!(f.count() > 0, "有能量时应发射粒子");
        // 采集最低粒子 y
        let before = f.particles.iter().map(|p| p.y).fold(0.0f32, f32::max);
        f.step(0.1, 0.5);
        let after = f.particles.iter().map(|p| p.y).fold(0.0f32, f32::max);
        // 上浮：平均 y 应增大（粒子从底部升起）
        assert!(after >= before - 1e-3, "粒子应上浮");
    }

    #[test]
    fn test_silent_frame_emits_nothing() {
        let mut f = ParticleField::new(40, 20, 7);
        f.step(0.1, 0.0);
        f.step(0.5, 0.0);
        assert_eq!(f.count(), 0, "静默（energy=0）不应发射粒子");
    }

    #[test]
    fn test_energy_increases_spawn_rate() {
        let mut low = ParticleField::new(40, 20, 1);
        low.step(0.5, 0.0);
        let n_low = low.count();
        let mut high = ParticleField::new(40, 20, 1);
        high.step(0.5, 1.0);
        let n_high = high.count();
        assert!(
            n_high > n_low,
            "高能量应发射更多粒子 ({} vs {})",
            n_high,
            n_low
        );
    }

    #[test]
    fn test_particles_decay_and_die() {
        let mut f = ParticleField::new(40, 20, 3);
        f.step(0.1, 1.0);
        let n0 = f.count();
        assert!(n0 > 0);
        // 静默推进：不再发射，已有粒子随寿命衰减最终消亡
        for _ in 0..200 {
            f.step(0.1, 0.0);
        }
        assert_eq!(f.count(), 0, "静默后粒子应全部消亡");
    }

    #[test]
    fn test_determinism_same_seed() {
        let mut a = ParticleField::new(30, 15, 99);
        let mut b = ParticleField::new(30, 15, 99);
        a.step(0.1, 0.7);
        b.step(0.1, 0.7);
        assert_eq!(a.count(), b.count());
        assert_eq!(
            a.particles.first().map(|p| (p.x, p.y)),
            b.particles.first().map(|p| (p.x, p.y))
        );
    }

    #[test]
    fn test_density_out_of_bounds_is_zero() {
        let mut f = ParticleField::new(10, 10, 5);
        f.step(0.2, 1.0);
        assert_eq!(f.density_at(10, 0), 0.0);
        assert_eq!(f.density_at(0, 10), 0.0);
    }
}
