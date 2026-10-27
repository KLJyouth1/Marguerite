#!/usr/bin/env node
/**
 * 启动动画素材编解码守卫。
 *
 * # 为什么需要它（2026-10-22 实测事故）
 *
 * `SplashView` 引用 `public/splash/bg-v2.mp4`，产物里文件明明存在（指纹也对得上），
 * 但**视频从不播放**，用户只看到极光和字幕。
 *
 * 根因：该文件是 **HEVC / H.265（`hvc1`）**，而 WebView2（Chromium 内核）默认
 * **不含 HEVC 解码器**（专利许可，依赖系统 HEVC 扩展 + 硬件解码）。`<video>`
 * 立刻 error → 回退极光。
 *
 * 这类缺陷**所有常规门禁都抓不到**：文件存在、大小非零、路径正确、构建成功、
 * 指纹一致 —— 唯独解不出来。所以必须直接检查**编码**。
 *
 * 判据：
 *   1. `public/splash/` 下每个 `.mp4` 不得是 HEVC/`hvc1`/`hev1`；
 *   2. 至少存在一份 H.264（`avc1`）—— 那是 WebView2 的保底；
 *   3. `SplashView` 的首选 source 必须指向 H.264 文件；
 *   4. `public/music/` 下的 `.mp4`（背景 MV）同规则：不得 HEVC。
 *      2026-10-23 增补：新增 Seedance 生成的 30s 背景视频，与启动动画是
 *      **同一类失败模式**（文件在、大小对、路径对，就是解不出来），
 *      守卫必须一起覆盖，否则下一次换素材又会静默黑屏。
 *
 * 纯 Node、零依赖：读 MP4 的 `ftyp` / 编解码 fourcc 直接扫字节，
 * 不依赖 ffprobe 是否装在构建机上。
 */

import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '..')
const splashDir = join(root, 'public', 'splash')
const musicDir = join(root, 'public', 'music')
const splashView = join(root, 'src', 'views', 'SplashView.vue')

const failures = []

/** 扫描字节流里的编解码 fourcc。 */
function detectCodecs(buf) {
  const found = new Set()
  for (const cc of ['avc1', 'hvc1', 'hev1', 'vp09', 'av01']) {
    if (buf.includes(Buffer.from(cc))) found.add(cc)
  }
  return found
}

if (!existsSync(splashDir)) {
  console.log('[check-splash-media] public/splash 不存在，跳过')
  process.exit(0)
}

const videos = readdirSync(splashDir).filter((f) => /\.(mp4|webm|mov)$/i.test(f))
if (videos.length === 0) {
  failures.push('public/splash 下没有任何视频文件 —— 启动动画会只剩极光')
}

let hasH264 = false
for (const name of videos) {
  const buf = readFileSync(join(splashDir, name))
  const codecs = detectCodecs(buf)
  if (name.toLowerCase().endsWith('.webm')) {
    // WebM 用 VP8/VP9，Chromium 原生支持；不在此处判定
    continue
  }
  if (codecs.has('hvc1') || codecs.has('hev1')) {
    failures.push(
      `${name} 是 HEVC/H.265 —— WebView2 默认解不了，启动动画会没有画面。` +
        `请转成 H.264：ffmpeg -i ${name} -c:v libx264 -profile:v main -pix_fmt yuv420p -crf 26 ${name.replace(/\.mp4$/i, '-h264.mp4')}`,
    )
  }
  if (codecs.has('avc1')) hasH264 = true
}

if (!hasH264) {
  failures.push(
    'public/splash 下没有 H.264（avc1）视频 —— WebView2 的保底格式缺失。' +
      '即使有 WebM，旧版 WebView2 也可能只有 H.264。',
  )
}

// 3. 首选 source 必须是 H.264
if (existsSync(splashView)) {
  const src = readFileSync(splashView, 'utf8')
  const m = src.match(/BG_VIDEO_SOURCES\s*=\s*\[([\s\S]*?)\]/)
  if (!m) {
    failures.push('SplashView 里找不到 BG_VIDEO_SOURCES —— 多源声明缺失')
  } else {
    const block = m[1]
    const first = block.match(/src:\s*'([^']+)'/)
    if (!first) {
      failures.push('BG_VIDEO_SOURCES 里没有 src')
    } else if (!/h264/i.test(first[1])) {
      failures.push(
        `BG_VIDEO_SOURCES 首选源是 ${first[1]} —— 必须把 H.264 放在第一位，` +
          '否则浏览器可能先挑到解不了的格式',
      )
    }
  }
}

console.log('=== 启动动画素材编解码守卫 ===')
console.log(`  扫描 ${videos.length} 个视频，H.264 存在=${hasH264}`)

// 4. 背景 MV（public/music/*.mp4）同规则：不得 HEVC。
//    它与启动动画共用同一个 WebView2 解码器，失败表现也一致（静默黑屏）。
let musicVideos = 0
if (existsSync(musicDir)) {
  musicVideos = readdirSync(musicDir).filter((f) => /\.mp4$/i.test(f)).length
  for (const name of readdirSync(musicDir).filter((f) => /\.mp4$/i.test(f))) {
    const buf = readFileSync(join(musicDir, name))
    const codecs = detectCodecs(buf)
    if (codecs.has('hvc1') || codecs.has('hev1')) {
      failures.push(
        `public/music/${name} 是 HEVC/H.265 —— WebView2 默认解不了，音乐背景视频会静默黑屏。` +
          `请转成 H.264：ffmpeg -i ${name} -c:v libx264 -pix_fmt yuv420p -crf 24 ${name}`,
      )
    }
  }
  console.log(`  背景 MV：${musicVideos} 个（music/*.mp4）`)
}

if (failures.length) {
  for (const f of failures) console.error(`[FAIL] ${f}`)
  console.error(`\n未通过：${failures.length} 条`)
  process.exit(1)
}
console.log('[PASS] 无 HEVC 视频；H.264 保底存在；首选源为 H.264。')
