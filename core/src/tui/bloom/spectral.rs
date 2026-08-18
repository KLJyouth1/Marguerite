//! 音频频谱分析（Bloom 音乐粒子的能量源）
//!
//! 纯数学、零依赖、零音频设备依赖：给定一帧 `f32` 时域样本，输出
//! 低/中/高频带能量 + RMS + 谱质心。FFT 为手写迭代 radix-2 Cooley-Tukey，
//! 保证在 CI / 无头环境可编译、可单测（不链接任何音频后端）。

/// 复数（轻量本地实现，避免引入 num-complex 依赖）
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cx {
    re: f32,
    im: f32,
}

impl Cx {
    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }
    fn add(self, o: Cx) -> Cx {
        Cx::new(self.re + o.re, self.im + o.im)
    }
    fn sub(self, o: Cx) -> Cx {
        Cx::new(self.re - o.re, self.im - o.im)
    }
    fn mul(self, o: Cx) -> Cx {
        Cx::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
}

/// 迭代 radix-2 FFT（输入长度必须为 2 的幂）
fn fft(input: &[Cx]) -> Vec<Cx> {
    let n = input.len();
    debug_assert!(n.is_power_of_two() && n > 0);
    // 位反转置换
    let mut a: Vec<Cx> = input.to_vec();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    // Danielson-Lanczos 蝶形
    let mut len = 2usize;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let wlen = Cx::new(ang.cos(), ang.sin());
        let mut i = 0usize;
        while i < n {
            let mut w = Cx::new(1.0, 0.0);
            for k in 0..(len / 2) {
                let u = a[i + k];
                let v = a[i + k + len / 2].mul(w);
                a[i + k] = u.add(v);
                a[i + k + len / 2] = u.sub(v);
                w = w.mul(wlen);
            }
            i += len;
        }
        len <<= 1;
    }
    a
}

/// 频谱带特征
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralBands {
    /// 低频带能量（0..1 归一）
    pub low: f32,
    /// 中频带能量
    pub mid: f32,
    /// 高频带能量
    pub high: f32,
    /// 均方根（响度）
    pub rms: f32,
    /// 谱质心（亮度，Hz）
    pub centroid: f32,
}

impl SpectralBands {
    /// 全零（静音帧）
    pub fn silent() -> Self {
        Self {
            low: 0.0,
            mid: 0.0,
            high: 0.0,
            rms: 0.0,
            centroid: 0.0,
        }
    }
}

fn next_pow2(n: usize) -> usize {
    let mut p = 1;
    while p < n {
        p <<= 1;
    }
    p
}

/// 窗口化 + FFT + 分带能量。
///
/// `sample_rate` 用于把频域 bin 映射到真实频率（默认 44.1kHz）。
/// 输入不足时填零到 2 的幂；超过 cap（默认 4096）时截断。
pub fn analyze_bands(samples: &[f32], sample_rate: Option<u32>) -> SpectralBands {
    if samples.is_empty() {
        return SpectralBands::silent();
    }
    let sr = sample_rate.unwrap_or(44_100) as f32;
    let cap = 4096usize;
    let take = samples.len().min(cap);
    let n = next_pow2(take);
    // Hann 窗 + 填零
    let mut buf = vec![Cx::new(0.0, 0.0); n];
    for (i, s) in samples.iter().take(take).enumerate() {
        let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (take as f32 - 1.0)).cos();
        buf[i] = Cx::new(s * w, 0.0);
    }
    let spec = fft(&buf);
    // 幅度谱（半谱）
    let half = n / 2;
    let mut mag = vec![0.0f32; half];
    let mut total = 0.0f32;
    let mut weighted = 0.0f32;
    for (k, m) in mag.iter_mut().enumerate() {
        let re = spec[k].re;
        let im = spec[k].im;
        let amp = (re * re + im * im).sqrt();
        // 幅度归一（除以 N/2）
        let norm = amp / (half as f32);
        *m = norm;
        let freq = k as f32 * sr / n as f32;
        total += norm;
        weighted += norm * freq;
    }
    let rms = (samples.iter().take(take).map(|s| s * s).sum::<f32>() / take as f32).sqrt();
    let centroid = if total > 1e-6 { weighted / total } else { 0.0 };

    // 分带积分（Hz 边界）
    let band = |lo: f32, hi: f32| -> f32 {
        let mut e = 0.0f32;
        for (k, &m) in mag.iter().enumerate() {
            let f = k as f32 * sr / n as f32;
            if f >= lo && f < hi {
                e += m;
            }
        }
        e
    };
    let low = band(20.0, 250.0);
    let mid = band(250.0, 2000.0);
    let high = band(2000.0, 8000.0);
    // 归一（相对总能量）
    let sum = (low + mid + high).max(1e-9);
    SpectralBands {
        low: low / sum,
        mid: mid / sum,
        high: high / sum,
        rms,
        centroid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn test_next_pow2() {
        assert_eq!(next_pow2(1), 1);
        assert_eq!(next_pow2(2), 2);
        assert_eq!(next_pow2(3), 4);
        assert_eq!(next_pow2(1000), 1024);
    }

    #[test]
    fn test_fft_impulse_peak_at_dc() {
        // 单位脉冲：FFT 全为 1（DC 分量），幅度谱在 k=0 最大
        let inp: Vec<Cx> = (0..8)
            .map(|i| Cx::new(if i == 0 { 1.0 } else { 0.0 }, 0.0))
            .collect();
        let out = fft(&inp);
        let mut max_k = 0;
        let mut max_v = 0.0f32;
        for (k, c) in out.iter().enumerate() {
            let v = (c.re * c.re + c.im * c.im).sqrt();
            if v > max_v {
                max_v = v;
                max_k = k;
            }
        }
        assert_eq!(max_k, 0);
    }

    #[test]
    fn test_fft_sine_lands_in_expected_bin() {
        // 8kHz 采样，1024 点，1kHz 正弦 -> 期望峰在 bin 128 附近
        let sr = 8000u32;
        let n = 1024usize;
        let sig = sine(1000.0, sr, n);
        let b = analyze_bands(&sig, Some(sr));
        let bin_hz = sr as f32 / n as f32; // 7.8125 Hz/bin
        let expected = (1000.0 / bin_hz).round() as usize;
        // 用幅度谱找峰
        let cap = n.min(4096);
        let nn = next_pow2(cap);
        let mut buf = vec![Cx::new(0.0, 0.0); nn];
        for (i, s) in sig.iter().take(cap).enumerate() {
            let w = 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (cap as f32 - 1.0)).cos();
            buf[i] = Cx::new(s * w, 0.0);
        }
        let spec = fft(&buf);
        let half = nn / 2;
        let mut peak_k = 0;
        let mut peak_v = 0.0f32;
        for (k, bin) in spec.iter().enumerate().skip(1).take(half - 1) {
            let v = (bin.re * bin.re + bin.im * bin.im).sqrt();
            if v > peak_v {
                peak_v = v;
                peak_k = k;
            }
        }
        assert!(
            (peak_k as i32 - expected as i32).abs() <= 2,
            "峰值 bin {} 应接近 {}",
            peak_k,
            expected
        );
        // 有能量
        assert!(b.rms > 0.5, "正弦 RMS 应接近 1/√2");
    }

    #[test]
    fn test_silent_frame_is_zero() {
        let b = analyze_bands(&[], None);
        assert_eq!(b, SpectralBands::silent());
        let zeros = vec![0.0f32; 512];
        let b2 = analyze_bands(&zeros, None);
        assert_eq!(b2.rms, 0.0);
        assert_eq!(b2.centroid, 0.0);
    }

    #[test]
    fn test_bands_normalized_sum_approx_one() {
        let sig = sine(440.0, 44_100, 2048);
        let b = analyze_bands(&sig, None);
        let s = b.low + b.mid + b.high;
        assert!((s - 1.0).abs() < 1e-3, "分带能量之和应≈1，实际 {}", s);
        // 440Hz 落在中频带(250-2000Hz)，mid 应最强
        assert!(b.mid >= b.low && b.mid >= b.high, "440Hz 应落在中频带");
    }
}
