# 启动动画素材

## 为什么这里有 HEVC 原件而 `public/` 里是转码版

`bg-v2-hevc-source.mp4` 是收到的**原始素材**（HEVC / H.265，1280×720，24fps，30.04s）。

**它不能直接用于启动动画**：WebView2（Chromium 内核）默认**不含 HEVC 解码器**
（专利许可原因，依赖系统 HEVC 扩展 + 硬件解码）。实测表现是 `<video>` 立刻 error、
视频永不显示 —— 也就是"启动动画只有字幕、没有视频"。

因此 `webui/public/splash/` 下放的是转码后的两份**所有 Chromium 都能解**的格式：

| 文件 | 编码 | 用途 |
|---|---|---|
| `bg-v2-h264.mp4` | H.264 Main / yuv420p / faststart | 主选（WebView2 必解） |
| `bg-v2.webm` | VP9 | 备选（体积相近，Chromium 原生支持） |

原件留在这里仅为**可溯源**，不进构建产物（`public/` 之外不会被 Vite 复制）。

## 重新生成

```bash
cd webui/public/splash
ffmpeg -y -i ../../media/splash/bg-v2-hevc-source.mp4 \
  -c:v libx264 -profile:v main -level 4.0 -pix_fmt yuv420p \
  -crf 26 -preset slow -movflags +faststart -an bg-v2-h264.mp4

ffmpeg -y -i ../../media/splash/bg-v2-hevc-source.mp4 \
  -c:v libvpx-vp9 -crf 34 -b:v 0 -row-mt 1 -an bg-v2.webm
```

## 改素材时务必核对

换了新素材后，先用 `ffprobe` 确认编码**不是** `hevc`：

```bash
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name -of csv=p=0 <<file>>
```

`hevc` → 必须转码，否则启动动画不会有画面。
