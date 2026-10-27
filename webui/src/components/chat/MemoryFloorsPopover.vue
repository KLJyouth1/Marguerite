<template>
  <div ref="rootRef" class="mfloors">
    <button
      type="button"
      class="btn btn--icon btn--ghost mfloors__chip"
      :class="{ 'is-active': open }"
      :title="loadedOnce ? `记忆楼层：${enabledCount}/${total} 层开启` : '记忆楼层：读取中'"
      :aria-label="loadedOnce ? `记忆楼层：${enabledCount}/${total} 层开启` : '记忆楼层：读取中'"
      aria-haspopup="dialog"
      :aria-expanded="open ? 'true' : 'false'"
      data-testid="memory-floors-trigger"
      @click="toggleOpen"
    >
      <Icon name="boxes" :size="17" />
      <span class="mfloors__badge" aria-hidden="true">{{ loadedOnce ? enabledCount : '·' }}</span>
    </button>

    <section
      v-if="open"
      class="mfloors__popover"
      role="dialog"
      aria-label="记忆楼层"
      data-testid="memory-floors-panel"
      @keydown.esc.stop="close"
    >
      <header class="mfloors__head">
        <div>
          <strong>记忆楼层</strong>
          <small>7 层记忆 + 经验，共 8 层</small>
        </div>
        <button
          type="button"
          class="mfloors__close"
          aria-label="关闭"
          @click="close"
        >
          <Icon name="x" :size="14" />
        </button>
      </header>

      <p v-if="loading" class="mfloors__state">加载中…</p>
      <p v-else-if="error" class="mfloors__state mfloors__state--error" role="alert">
        {{ error }}
        <button type="button" class="btn" @click="void load()">重试</button>
      </p>

      <!-- 楼层从下往上叠：第 1 层在最底，经验层在最顶。
           `column-reverse` 让数组顺序（1→8）与视觉高度一致，不必在模板里反转。 -->
      <div v-else class="mfloors__stack" data-testid="memory-floors-stack">
        <button
          v-for="f in floors"
          :key="f.id"
          type="button"
          class="mfloor"
          :class="{ 'is-on': f.enabled }"
          :style="{ '--floor-index': String(f.floor) }"
          role="switch"
          :aria-checked="f.enabled ? 'true' : 'false'"
          :aria-label="`${f.name}：${f.enabled ? '已开启' : '已关闭'}`"
          :title="f.description"
          :disabled="saving"
          :data-testid="`memory-floor-${f.id}`"
          :data-enabled="f.enabled ? 'true' : 'false'"
          @click="void toggle(f)"
        >
          <span class="mfloor__no" aria-hidden="true">{{ f.floor }}</span>
          <span class="mfloor__body">
            <span class="mfloor__name">{{ f.name }}</span>
            <span class="mfloor__desc">{{ f.description }}</span>
          </span>
          <span class="mfloor__dot" aria-hidden="true" />
        </button>
      </div>

      <footer class="mfloors__foot">
        <button type="button" class="mfloors__all" :disabled="saving || loading" @click="void setAll(true)">
          全开
        </button>
        <button type="button" class="mfloors__all" :disabled="saving || loading" @click="void setAll(false)">
          全关
        </button>
        <span class="mfloors__status" role="status" data-testid="memory-floors-status">{{ status }}</span>
      </footer>
      <p class="mfloors__hint">
        关掉的层连检索都不会发起，也不会写进 prompt；下一次发送即刻生效。
      </p>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'

interface Floor {
  id: string
  floor: number
  name: string
  description: string
  backend: string
  enabled: boolean
}

interface FloorsPayload {
  ok?: boolean
  error?: string
  layers?: Floor[]
  enabledCount?: number
}

const rpc = useRpcStore()

const open = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const status = ref('')
const floors = ref<Floor[]>([])
const loadedOnce = ref(false)

const enabledCount = computed(() => floors.value.filter(f => f.enabled).length)
const total = computed(() => floors.value.length || 8)

const rootRef = ref<HTMLElement | null>(null)

function toggleOpen(): void {
  open.value = !open.value
}

function close(): void {
  open.value = false
}

function eventInsideRoot(event: PointerEvent): boolean {
  const root = rootRef.value
  if (!root) return false
  const path = typeof event.composedPath === 'function' ? event.composedPath() : []
  if (path.includes(root)) return true
  return event.target instanceof Node && root.contains(event.target)
}

function onOutsidePointerDown(event: PointerEvent): void {
  if (open.value && !eventInsideRoot(event)) open.value = false
}

watch(open, (isOpen) => {
  if (isOpen) {
    document.addEventListener('pointerdown', onOutsidePointerDown, true)
    // 首次打开才拉取：组件常驻在输入框里，每次展开都打一次 RPC 没必要。
    if (!loadedOnce.value) void load()
  } else {
    document.removeEventListener('pointerdown', onOutsidePointerDown, true)
  }
})

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onOutsidePointerDown, true)
})

// 挂载即预取：徽标是「一眼看状态」的入口，只有打开弹窗才拉取会让它长期显示 0，
// 巡检实测（2026-10-24）因此出现「徽标 0、弹窗里 8 层全开」的自相矛盾。
//
// RPC 可能还没连上（功能页冷加载竞态）：先直接试一次；失败时如果 `methods`
// 随后就绪（连接建立），再自动补一次，而不是让徽标一直停在占位符。
onMounted(() => {
  void load()
  const stop = watch(
    () => rpc.methods.length,
    (n) => {
      if (n > 0 && !loadedOnce.value) {
        stop()
        void load()
      }
    },
  )
})

async function load(): Promise<void> {
  loading.value = true
  error.value = ''
  try {
    if (rpc.methods.length > 0 && !rpc.supportsMethod('memory.layers.switches.get')) {
      error.value = '当前内核不支持记忆楼层开关（memory.layers.switches.get）'
      return
    }
    const res = await rpc.call<FloorsPayload>('memory.layers.switches.get')
    if (res?.ok === false || !res?.layers) {
      error.value = res?.error || '读取记忆楼层失败'
      return
    }
    floors.value = res.layers
    loadedOnce.value = true
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

/// 单层切换：乐观更新 + 失败回滚。
///
/// 为什么乐观：开关是本地的、往返要几十毫秒，等 RPC 回来再动会让点击"粘手"。
/// 为什么必须回滚：写配置可能失败（文件锁/只读），不回滚就是"界面说关了、实际还开着"，
/// 而记忆是否注入几乎无法从界面上验证 —— 这种假状态最难排查。
async function toggle(f: Floor): Promise<void> {
  const next = !f.enabled
  f.enabled = next
  await persist({ [f.id]: next }, () => { f.enabled = !next })
}

async function setAll(value: boolean): Promise<void> {
  const snapshot = floors.value.map(f => ({ id: f.id, enabled: f.enabled }))
  floors.value = floors.value.map(f => ({ ...f, enabled: value }))
  await persist(
    Object.fromEntries(snapshot.map(s => [s.id, value])),
    () => {
      for (const s of snapshot) {
        const hit = floors.value.find(f => f.id === s.id)
        if (hit) hit.enabled = s.enabled
      }
    },
  )
}

async function persist(patch: Record<string, boolean>, rollback: () => void): Promise<void> {
  saving.value = true
  status.value = ''
  try {
    const res = await rpc.call<FloorsPayload & { applied?: string[] }>(
      'memory.layers.switches.set',
      { layers: patch },
    )
    if (res?.ok === false) {
      rollback()
      status.value = res.error || '保存失败'
      return
    }
    if (res?.layers) floors.value = res.layers
    status.value = '已保存'
  } catch (e) {
    rollback()
    status.value = e instanceof Error ? e.message : String(e)
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.mfloors {
  position: relative;
  display: inline-flex;
  align-items: center;
  flex: 0 0 auto;
}

.mfloors__chip {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-height: 34px;
  padding: 0 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
}

.mfloors__chip:hover,
.mfloors__chip.is-active {
  color: var(--text);
  border-color: var(--border-strong);
  background: var(--bg-hover);
}

.mfloors__badge {
  min-width: 14px;
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
  color: var(--accent);
}

.mfloors__popover {
  position: absolute;
  bottom: calc(100% + var(--sp-2));
  left: 0;
  z-index: 40;
  width: 320px;
  /* 锚在输入框上方：高度还必须受视口剩余空间约束，否则窗口较矮时弹窗会顶出屏幕。 */
  max-height: min(620px, calc(100vh - 120px));
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  padding: var(--sp-3);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  background: var(--bg-elevated);
  box-shadow: var(--shadow-lg);
}

.mfloors__head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--sp-2);
}

.mfloors__head strong {
  font-size: var(--fs-sm);
}

.mfloors__head small {
  display: block;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.mfloors__close {
  border: 0;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  padding: 2px;
}

.mfloors__state {
  margin: 0;
  color: var(--text-muted);
  font-size: var(--fs-xs);
}

.mfloors__state--error {
  color: var(--text);
}

/* 楼层堆叠：column-reverse 让数组第 1 项落在最底，经验层在最顶。 */
.mfloors__stack {
  display: flex;
  flex-direction: column-reverse;
  gap: 3px;
  /* 关键：flex 子项的默认 min-height:auto 会阻止它在父级 max-height 内收缩，
     导致内容溢到弹窗外面（巡检实测：经验层 top=-36px，被视口裁掉）。
     min-height:0 + flex:1 才让这里真正成为滚动容器。 */
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  scrollbar-gutter: stable;
  padding-block: 2px;
}

.mfloor {
  position: relative;
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: 100%;
  /* 越高的楼层略窄，形成"塔"的透视；第 1 层最宽。 */
  margin-inline: calc((var(--floor-index) - 1) * 2px);
  padding: 5px var(--sp-2);
  text-align: start;
  border: 1px dashed var(--border);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  opacity: 0.55;
  transition: opacity var(--dur-fast) var(--ease-standard),
    background-color var(--dur-fast) var(--ease-standard),
    border-color var(--dur-fast) var(--ease-standard);
}

/* 开启 = 一层透明方框叠加：淡色填充 + 实线边框 + 还原亮度。 */
.mfloor.is-on {
  opacity: 1;
  border-style: solid;
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--text);
}

.mfloor:disabled {
  cursor: default;
}

.mfloor__no {
  flex: 0 0 18px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-sm);
  background: var(--bg-surface-2);
  color: var(--text-dim);
  font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
}

.mfloor.is-on .mfloor__no {
  background: color-mix(in srgb, var(--accent) 26%, transparent);
  color: var(--text);
}

.mfloor__body {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}

.mfloor__name {
  font-size: var(--fs-xs);
  font-weight: 600;
}

.mfloor__desc {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  line-height: 1.35;
  /* 单行截断：8 层要在一屏内全见，两行描述会把弹窗顶出可视区；
     完整说明在按钮的 title 上，悬停可读。 */
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.mfloor__dot {
  flex: 0 0 8px;
  width: 8px;
  height: 8px;
  border-radius: var(--radius-full);
  border: 1px solid var(--border-strong);
  background: transparent;
}

.mfloor.is-on .mfloor__dot {
  border-color: var(--accent);
  background: var(--accent);
}

.mfloors__foot {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
}

.mfloors__all {
  padding: 3px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-pill);
  background: transparent;
  color: var(--text-muted);
  font-size: var(--fs-xs);
  cursor: pointer;
}

.mfloors__all:hover:not(:disabled) {
  color: var(--text);
  border-color: var(--border-strong);
}

.mfloors__status {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}

.mfloors__hint {
  margin: 0;
  color: var(--text-dim);
  font-size: var(--fs-xs);
  line-height: 1.4;
}

/* 矮窗口（常见 680–760px 高）：
   弹窗向上展开，空间本来就不够 8 层「名称 + 描述」，硬撑会把弹窗顶进顶栏
   （实测 top=6.5px，关闭按钮被顶栏徽标盖住点不到）。
   这一档只保留楼层名（完整描述在按钮 title 上），换取 8 层一屏全见。 */
@media (max-height: 780px) {
  .mfloor__desc {
    display: none;
  }

  .mfloor {
    padding: 4px var(--sp-2);
    min-height: 30px;
  }
}
</style>
