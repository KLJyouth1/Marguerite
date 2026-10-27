# Built-in music catalog — generator notes

## What this is

`generate-music.mjs` renders the four MP3 tracks shipped in
`public/music/` (the built-in background-music catalog). Every note is
procedural: oscillators, envelopes and a feedback-delay "reverb" computed in
plain JavaScript. **No audio samples, no third-party recordings, no licensed
material of any kind is used.**

## Provenance

- **Generated:** 2026-10-20 by the Marguerite project on Windows with
  ffmpeg 8.1.1 (LAME MP3 encoder).
- **Author:** Marguerite project authorship, dedicated to the public domain.
- **License:** CC0 1.0 Universal (Public Domain Dedication). You may copy,
  modify, distribute and perform the work, even for commercial purposes, all
  without asking permission. See `https://creativecommons.org/publicdomain/zero/1.0/`.

## Pipeline

1. Each track is synthesized into a 44.1 kHz stereo float buffer:
   drums (sine-pitch kicks, noise hats/snare), additive saw/triangle bass and
   plucks, detuned-sine pads, plus an optional vinyl-noise bed.
2. The master bus applies a 0.22–0.34 s feedback delay, a `tanh` soft clip
   and peak normalisation to −1 dBFS.
3. The buffer is written as a 16-bit PCM WAV, then encoded by ffmpeg:
   `libmp3lame, 112 kbps, 44.1 kHz, stereo`.
4. Temp WAVs are cleaned up; the script prints sha256 + size per track.

Deterministic: a fixed mulberry32 seed per track means re-running produces
byte-identical files. To re-render (e.g. after editing the script):

```
cd webui
node scripts/generate-music.mjs
```

Then refresh `public/music/playlist.json` durations if they change.

## Current catalog

| File | Length | Bitrate |
|---|---|---|
| dusk-pad.mp3 | 75 s | 112 kbps |
| neon-arp.mp3 | 80 s | 112 kbps |
| lofi-drift.mp3 | 80 s | 112 kbps |
| focus-pulse.mp3 | 85 s | 112 kbps |

Total ~4.5 MB. If you ship these in a build, no attribution is required, but
keeping this file next to the script documents the lineage.
