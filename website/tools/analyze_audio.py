"""Beat / section analysis for the promo soundtrack (numpy + ffmpeg only).

  1. decode to mono 22.05 kHz float PCM
  2. spectral-flux onset envelope
  3. onsets by adaptive peak picking -> median inter-onset interval
  4. beat grid: try 1x / 2x / 0.5x of the IOI-derived tempo, keep the phase
     that collects the most onset energy
  5. strong beats (top energy) and coarse RMS sections

Usage: python scripts/promo/analyze_audio.py <audio> <out.json>
"""

from __future__ import annotations

import json
import subprocess
import sys

import numpy as np

SR = 22050
FRAME = 1024
HOP = 256
FPS = SR / HOP


def decode(path: str) -> np.ndarray:
    proc = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", path, "-ac", "1", "-ar", str(SR), "-f", "f32le", "-"],
        capture_output=True,
        check=True,
    )
    return np.frombuffer(proc.stdout, dtype=np.float32)


def onset_envelope(samples: np.ndarray) -> np.ndarray:
    if samples.size < FRAME:
        return np.zeros(1, dtype=np.float32)

    count = 1 + (samples.size - FRAME) // HOP
    window = np.hanning(FRAME).astype(np.float32)
    frames = np.lib.stride_tricks.as_strided(
        samples,
        shape=(count, FRAME),
        strides=(samples.strides[0] * HOP, samples.strides[0]),
        writeable=False,
    )
    spectra = np.log1p(np.abs(np.fft.rfft(frames * window, axis=1)) * 10.0)
    flux = np.maximum(np.diff(spectra, axis=0), 0.0).sum(axis=1)
    flux = np.concatenate([[0.0], flux]).astype(np.float32)

    # adaptive threshold: subtract a slow local mean, keep positive residue
    kernel = np.ones(48, dtype=np.float32) / 48.0
    local = np.convolve(flux, kernel, mode="same")
    env = np.maximum(flux / (local + 1e-4) - 1.0, 0.0).astype(np.float32)
    return env


def pick_onsets(env: np.ndarray) -> np.ndarray:
    if env.size < 3:
        return np.array([], dtype=int)
    threshold = float(np.mean(env) + 0.35 * np.std(env))
    peaks = []
    for i in range(1, env.size - 1):
        if env[i] < threshold:
            continue
        if env[i] < env[i - 1] or env[i] < env[i + 1]:
            continue
        if peaks and (i - peaks[-1]) < int(FPS * 0.09):  # 90 ms refractory
            if env[i] > env[peaks[-1]]:
                peaks[-1] = i
            continue
        peaks.append(i)
    return np.array(peaks, dtype=int)


def snap_to_pulse(t: float, pulse: float, phase: float) -> float:
    """Nearest point on the pulse grid (used by the site to cut on the beat)."""
    if pulse <= 0:
        return t
    k = round((t - phase) / pulse)
    return round(max(0.0, phase + k * pulse), 3)


def tempo_candidates(onsets: np.ndarray) -> list[float]:
    if onsets.size < 8:
        return [112.0, 56.0, 168.0]

    intervals = np.diff(onsets) / FPS
    intervals = intervals[(intervals > 0.12) & (intervals < 2.0)]
    if intervals.size == 0:
        return [112.0, 56.0, 168.0]

    median = float(np.median(intervals))
    base = 60.0 / median
    # Snap to a musical range, then consider the common half / double readings.
    while base < 70:
        base *= 2
    while base > 180:
        base /= 2
    return [base, base / 2, base * 2]


def best_grid(env: np.ndarray, bpm: float) -> tuple[float, list[dict]]:
    period = 60.0 / bpm * FPS
    if period < 2:
        return 0.0, []

    offsets = np.arange(0.0, period, 0.2)
    best_offset, best_score = 0.0, -1.0
    for offset in offsets:
        idx = np.round(np.arange(offset, env.size, period)).astype(int)
        idx = idx[(idx >= 0) & (idx < env.size)]
        score = float(env[idx].sum()) if idx.size else 0.0
        if score > best_score:
            best_offset, best_score = float(offset), score

    beats = []
    for idx in np.round(np.arange(best_offset, env.size, period)).astype(int):
        if 0 <= idx < env.size:
            beats.append({"t": round(idx * HOP / SR, 3), "strength": round(float(env[idx]), 3)})
    return best_score, beats


def rms_sections(samples: np.ndarray, fps: float = 4.0) -> list[dict]:
    win = int(SR / fps)
    count = samples.size // win
    if count < 4 or win < 1:
        return []

    blocks = samples[: count * win].reshape(count, win)
    rms = np.sqrt((blocks ** 2).mean(axis=1))
    norm = rms / (rms.max() + 1e-9)

    sections = [{"t": 0.0, "rms": round(float(norm[0]), 3)}]
    last_t = 0.0
    for i in range(1, count):
        delta = abs(float(norm[i]) - float(norm[i - 1]))
        t = i / fps
        if delta > 0.22 and (t - last_t) >= 2.0:
            sections.append({"t": round(t, 2), "rms": round(float(norm[i]), 3)})
            last_t = t
    return sections


def main() -> int:
    if len(sys.argv) < 3:
        print(__doc__)
        return 2

    audio, out = sys.argv[1], sys.argv[2]
    samples = decode(audio)
    env = onset_envelope(samples)
    onsets = pick_onsets(env)

    candidates = tempo_candidates(onsets)
    scored = []
    for bpm in candidates:
        score, beats = best_grid(env, bpm)
        scored.append((score, bpm, beats))
    scored.sort(key=lambda item: -item[0])
    best_score, bpm, beats = scored[0]

    # Raw sum of onset energy always favours the fastest grid (every 16th hits
    # something). Score per beat instead, over a real tempo sweep: that is what
    # actually separates "the beat" from its subdivisions on this track.
    sweep = []
    for candidate_bpm in np.arange(50.0, 180.5, 0.5):
        score, grid = best_grid(env, float(candidate_bpm))
        if not grid:
            continue
        sweep.append((score / len(grid), float(candidate_bpm), score, grid))
    sweep.sort(key=lambda item: -item[0])
    musical_score_per_beat, musical_bpm, musical_raw, musical_beats = sweep[0]

    beat_seconds = 60.0 / musical_bpm
    beat_phase = (musical_beats[0]["t"] % beat_seconds) if musical_beats else 0.0

    strong = sorted(beats, key=lambda b: -b["strength"])
    strong = strong[: max(8, len(beats) // 4)]
    strong.sort(key=lambda b: b["t"])

    pulse = 60.0 / bpm
    phase = (beats[0]["t"] % pulse) if beats else 0.0
    onsets_out = [
        {"t": round(float(i) * HOP / SR, 3), "strength": round(float(env[i]), 3)}
        for i in onsets
    ]
    onsets_out.sort(key=lambda o: o["t"])

    duration = samples.size / SR
    payload = {
        "source": audio,
        "duration": round(float(duration), 3),
        "sampleRate": SR,
        "bpm": round(float(bpm), 2),
        "pulseSeconds": round(pulse, 4),
        "pulsePhase": round(float(phase), 4),
        "beatBpm": round(float(musical_bpm), 2),
        "beatSeconds": round(float(beat_seconds), 4),
        "beatPhase": round(float(beat_phase), 4),
        "barSeconds": round(float(beat_seconds * 4), 4),
        "musicalScore": round(float(musical_score_per_beat), 3),
        "candidates": [
            {"bpm": round(float(b), 2), "score": round(float(s), 2), "beats": len(bs)}
            for s, b, bs in scored
        ],
        "onsetCount": int(onsets.size),
        "medianIoi": round(float(np.median(np.diff(onsets) / FPS)), 3) if onsets.size > 2 else None,
        "onsets": onsets_out,
        "beats": beats,
        "strongBeats": strong,
        "sections": rms_sections(samples),
    }
    with open(out, "w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)

    # 同源静态站点（含 file:// 校验）用 <script> 读节拍，fetch 会被 CORS 拦。
    # 只带页面真正使用的字段，避免把一个 27 KB 的分析结果塞进首屏。
    if len(sys.argv) > 3:
        js_path = sys.argv[3]
        slim_keys = ("duration", "beatBpm", "beatSeconds", "beatPhase", "barSeconds", "strongBeats")
        slim = {key: payload[key] for key in slim_keys}
        with open(js_path, "w", encoding="utf-8") as handle:
            handle.write("/* 由 tools/analyze_audio.py 生成，勿手改 */\n")
            handle.write("window.MARGUERITE_BEATS = ")
            json.dump(slim, handle, ensure_ascii=False, separators=(",", ":"))
            handle.write(";\n")

    print(
        f"duration={duration:.2f}s bpm={bpm:.2f} beats={len(beats)} "
        f"onsets={onsets.size} sections={len(payload['sections'])}"
    )
    print("candidates:", ", ".join(f"{c['bpm']}@{c['score']}" for c in payload["candidates"]))
    print("strong:", ", ".join(f"{b['t']:.2f}" for b in strong[:20]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
