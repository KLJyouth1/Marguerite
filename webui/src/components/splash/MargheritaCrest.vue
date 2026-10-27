<script setup lang="ts">
/**
 * Margherita 徽记 —— 启动动画的**原创**字形与标记。
 *
 * # 设计溯源（三个真实地标，不是凭空装饰）
 *
 * 1. **Rifugio Capanna Regina Margherita**（玛格丽特皇后小屋，海拔 4554 m，
 *    位于 Monte Rosa 的 Punta Gnifetti，欧洲最高山屋，1893 年建成）：
 *    它的剪影最容易被记住的特征是**屋脊顶端的十字架**与**三面石砌的三角轮廓**。
 *    → 本徽记的 `十字` 与 `三角基座` 直接取自它。
 *
 * 2. **Margherita of Savoy**（玛格丽特皇后，意大利王后，1851–1926）：
 *    Savoy 王室纹章的标志性母题是 **Savoy knot**（萨伏依结，一种对称的
 *    8 字形绳结，寓意"不可解"。萨伏依结的几何本质是**一条不断开的对称曲线**。
 *    → 本徽记环绕山峰的 `对称双弧` 即该绳结的极简化表达（保留"不断开"的拓扑）。
 *
 * 3. **Monte Margherita**（玛格丽特峰）：以皇后命名的山峰，其山脊是**锯齿状**的。
 *    → 徽记底部 `锯齿山脊线` 取自它。
 *
 * 三者在几何上刚好可以合成一个符号：**锯齿托住山、山里立十字、绳结环绕**。
 * 这是原创组合，不是任何现有品牌的复刻。
 *
 * # 为什么是"字形"而不是"字体文件"
 *
 * 造一套完整字体（含中文回退、多字重、字距表）是数周工作量，且本项目的文案
 * 只有 "MARGUERITE" 一个词。业界做法（LV / YSL / Chanel）是**单字母 monogram
 * + 精心排印的其余字母**。所以这里：
 *   · 首字母 **M** 是手绘 SVG 路径（双峰轮廓 —— 既是 M 也是山）；
 *   · 其余字母用 token 化的排印（极宽字距 + 细字重），与 monogram 同一视觉语言。
 *
 * 这样交付的是**真正原创且可维护**的标记，而不是把系统字体换个名字假装原创。
 */
withDefaults(
  defineProps<{
    /** 渲染尺寸（正方形边长，px） */
    size?: number
    /** 是否显示底部的锯齿山脊（极小尺寸下可关掉，避免糊成一团） */
    ridge?: boolean
  }>(),
  { size: 96, ridge: true },
)
</script>

<template>
  <svg
    class="margherita-crest"
    :width="size"
    :height="size"
    viewBox="0 0 120 120"
    role="img"
    aria-label="Margherita"
    fill="none"
  >
    <!-- ── Savoy knot 的极简对称双弧：环绕山峰，两端刻意"开口"表示绳结未打紧 ── -->
    <path
      class="crest-knot"
      d="M28 62 C10 52 10 30 30 26 C44 23 52 34 60 44"
      stroke-linecap="round"
    />
    <path
      class="crest-knot"
      d="M92 62 C110 52 110 30 90 26 C76 23 68 34 60 44"
      stroke-linecap="round"
    />

    <!-- ── 三角基座（Capanna Regina Margherita 的三面石砌轮廓） + M 形双峰 ──
         M 与山的同构是这枚徽记的核心：同一个轮廓读作字母 M，也读作双峰山脊。 -->
    <path
      class="crest-massif"
      d="M22 92 L44 50 L56 68 L72 30 L98 92"
      stroke-linecap="round"
      stroke-linejoin="round"
    />

    <!-- ── 最高峰上的十字（小屋屋脊） ── -->
    <path class="crest-cross" d="M72 30 L72 12" stroke-linecap="round" />
    <path class="crest-cross" d="M66 17 L78 17" stroke-linecap="round" />

    <!-- ── 底部锯齿山脊（Monte Margherita） ── -->
    <path
      v-if="ridge"
      class="crest-ridge"
      d="M22 100 L30 94 L38 100 L46 94 L54 100 L62 94 L70 100 L78 94 L86 100 L94 94 L98 100"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
  </svg>
</template>

<style scoped>
/* 全部走 token / color-mix：裸色值会被 check-webui-colors 判违规，
   也会在浅色主题下失去对比度。 */
.margherita-crest {
  display: block;
}

.crest-massif {
  stroke: var(--text);
  stroke-width: 3.2;
  opacity: 0.95;
}

.crest-cross {
  stroke: var(--accent);
  stroke-width: 3;
}

.crest-knot {
  stroke: color-mix(in srgb, var(--accent-secondary, var(--accent)) 78%, transparent);
  stroke-width: 2;
  opacity: 0.85;
}

.crest-ridge {
  stroke: color-mix(in srgb, var(--text) 45%, transparent);
  stroke-width: 1.6;
}
</style>
