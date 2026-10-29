<template>
  <div v-if="latest" class="handoff-notice" data-testid="handoff-notice">
    <Icon name="fileText" :size="15" class="handoff-notice__icon" aria-hidden="true" />
    <div class="handoff-notice__body">
      <p class="handoff-notice__title">{{ t('chat.handoff.noticeTitle') }}</p>
      <code class="handoff-notice__file">{{ latest.name }}</code>
    </div>
    <div class="handoff-notice__actions">
      <button
        type="button"
        class="btn btn--sm btn--primary"
        data-testid="handoff-notice-use"
        @click="emit('use', latest.name)"
      >{{ t('chat.handoff.noticeUse') }}</button>
      <button
        type="button"
        class="btn btn--sm btn--ghost"
        data-testid="handoff-notice-open"
        @click="emit('open', latest.path)"
      >{{ t('chat.handoff.noticeOpen') }}</button>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 工作区交接单承接提示（docs/plans/2026-10-27-final-remediation-plan.md A2）。
 *
 * 新会话落地页检测当前工作区 `handoff/` 目录的最新交接单。发现即提示：
 * 一键把承接指令填入输入框（人审后发送——决策权在人），或跳编辑器查看原文。
 * 设计纪律：检测失败/无交接单时**完全静默**——这是提示卡，不是错误面；
 * 绝不在落地页制造噪音。
 */
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import type { SandboxPathEntry } from '@/types/rpc'

const props = defineProps<{
  /** 当前会话绑定的工作区根（绝对路径）；空串表示未绑定，不检测。 */
  basePath: string
}>()

const emit = defineEmits<{
  /** 用户点击"引用到输入框"，参数为交接单文件名（handoff/ 内）。 */
  (e: 'use', fileName: string): void
  /** 用户点击"在编辑器打开"，参数为交接单绝对路径。 */
  (e: 'open', fullPath: string): void
}>()

const rpc = useRpcStore()
const { t } = useI18n()

const latest = ref<SandboxPathEntry | null>(null)
let seq = 0

async function detect(base: string): Promise<void> {
  const ticket = ++seq
  latest.value = null
  const root = String(base ?? '').trim()
  if (!root) return
  const dir = `${root.replace(/[\\/]+$/, '')}/handoff`
  try {
    if (!rpc.isConnected) await rpc.waitForConnection(8_000)
    const res = await rpc.call<{ entries?: SandboxPathEntry[] }>('sandbox.path.list', { path: dir })
    if (ticket !== seq) return // 期间工作区已切换，丢弃过期结果
    const files = (res?.entries ?? []).filter(
      e => e && e.kind === 'file' && !e.hidden && /\.md$/i.test(e.name),
    )
    // 文件名是 handoff 时间戳（ISO 排序安全），字典序最大即最新。
    files.sort((a, b) => (a.name < b.name ? 1 : a.name > b.name ? -1 : 0))
    latest.value = files[0] ?? null
  } catch {
    // 目录不存在/读取失败：静默，不给落地页添噪音。
    if (ticket === seq) latest.value = null
  }
}

watch(() => props.basePath, (base) => { void detect(base) }, { immediate: true })
</script>

<style scoped>
.handoff-notice {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  max-width: 560px;
  margin: 0 auto var(--sp-3);
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid var(--hairline);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent) 6%, var(--card));
}
.handoff-notice__icon { color: var(--accent); flex-shrink: 0; }
.handoff-notice__body { flex: 1; min-width: 0; }
.handoff-notice__title { margin: 0; font-size: var(--fs-xs); color: var(--text); }
.handoff-notice__file {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-dim);
}
.handoff-notice__actions { display: flex; gap: var(--sp-1); flex-shrink: 0; }
</style>
