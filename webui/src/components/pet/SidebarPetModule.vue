<template>
  <!-- 侧栏「鲸鱼娘」入口：紧凑交互卡片，替代原来的纯导航行。
       刻意不渲染 3D 舞台 / canvas / WebGL —— 页内桌宠由全局 DesktopPet 独占一个
       渲染上下文，侧栏再开一个会白烧第二条动画循环（见本次改造的纪律要求）。 -->
  <div class="sidebar-pet" :class="{ 'is-active': active }">
    <div class="sidebar-pet__head">
      <Icon name="pet" :size="16" />
      <span class="sidebar-pet__name">{{ t('pet.sidebar.title') }}</span>
      <span
        class="sidebar-pet__dot"
        :class="{ 'is-on': Boolean(status) }"
        aria-hidden="true"
      ></span>
    </div>

    <div class="sidebar-pet__stats">
      <span class="sidebar-pet__stat">
        <span class="sidebar-pet__k">{{ t('pet.sidebar.mood') }}</span>
        <span class="sidebar-pet__v">{{ moodText }}</span>
      </span>
      <span class="sidebar-pet__stat">
        <span class="sidebar-pet__k">{{ t('pet.sidebar.affinity') }}</span>
        <span class="sidebar-pet__v">{{ affectionText }}</span>
      </span>
    </div>

    <p v-if="statusNote" class="sidebar-pet__note">{{ statusNote }}</p>
    <p v-if="interactionNote" class="sidebar-pet__note">{{ interactionNote }}</p>
    <p v-if="actionError" class="sidebar-pet__note">{{ actionError }}</p>
    <p v-if="overlayNote" class="sidebar-pet__note">{{ overlayNote }}</p>

    <div class="sidebar-pet__actions">
      <button
        type="button"
        class="sidebar-pet__btn"
        :disabled="!canInteract"
        @click="interact"
      >
        {{ t('pet.sidebar.interact') }}
      </button>
      <button
        type="button"
        class="sidebar-pet__btn"
        :disabled="!overlayAvailable || overlayBusy"
        :title="overlayNote || undefined"
        @click="toggleOverlay"
      >
        {{ overlayOn ? t('pet.sidebar.hideOverlay') : t('pet.sidebar.showOverlay') }}
      </button>
      <button
        type="button"
        class="sidebar-pet__btn sidebar-pet__btn--full"
        :aria-current="active ? 'page' : undefined"
        @click="openFull"
      >
        {{ t('pet.sidebar.openFull') }}
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { usePetRpc, type PetStatus } from '@/rpc/petContract'
import {
  petOverlayAvailable,
  petOverlayPreferred,
  togglePetOverlay,
} from '@/composables/usePetOverlay'

defineProps<{ active?: boolean }>()
const emit = defineEmits<{ (e: 'navigate'): void }>()

const { t } = useI18n()
const router = useRouter()
const rpc = useRpcStore()
const petRpc = usePetRpc()

const status = ref<PetStatus | null>(null)
// 初始按「加载中」处理：避免首帧先闪一条「不可用」，再被立即成功的结果覆盖。
const loading = ref(true)
const loadError = ref('')
const busy = ref(false)
const actionError = ref('')

const overlayAvailable = petOverlayAvailable()
const overlayOn = ref(petOverlayPreferred())
const overlayBusy = ref(false)
const overlayError = ref('')

const statusReadable = computed(
  () => rpc.state === 'connected' && rpc.supportsMethod('pet.status'),
)
const canInteract = computed(
  () => rpc.state === 'connected' && rpc.supportsMethod('pet.triggerAction') && !busy.value,
)

const moodText = computed(() => status.value?.mood ?? '—')
const affectionText = computed(() =>
  typeof status.value?.affection === 'number' ? String(status.value.affection) : '—',
)

/** 读不到 pet.status 时如实说明「不可用 + 原因」，不弹 toast。 */
const statusNote = computed(() => {
  if (status.value) return ''
  if (loadError.value) return `${t('pet.sidebar.unavailable')} · ${loadError.value}`
  if (loading.value) return ''
  return `${t('pet.sidebar.unavailable')} · ${rpc.state === 'connected' ? 'pet.status' : rpc.state}`
})

const interactionNote = computed(() => {
  if (loading.value || !status.value) return ''
  if (rpc.state === 'connected' && rpc.supportsMethod('pet.triggerAction')) return ''
  return `${t('pet.sidebar.unavailable')} · pet.triggerAction`
})

const overlayNote = computed(() => {
  // 非桌面壳：如实说明不可用的原因，不显示「已开启」的假状态。
  if (!overlayAvailable) return `${t('pet.sidebar.unavailable')} · desktop-shell-required`
  if (overlayError.value) return `${t('pet.sidebar.unavailable')} · ${overlayError.value}`
  return ''
})

async function loadStatus(): Promise<void> {
  loading.value = true
  loadError.value = ''
  try {
    if (!statusReadable.value) throw new Error(rpc.state === 'connected' ? 'pet.status' : rpc.state)
    status.value = await petRpc.status()
  } catch (e) {
    status.value = null
    loadError.value = e instanceof Error ? e.message : String(e)
  } finally {
    loading.value = false
  }
}

async function interact(): Promise<void> {
  if (!canInteract.value) return
  busy.value = true
  actionError.value = ''
  try {
    const res = await petRpc.triggerAction('pet')
    if (!res.ok) throw new Error(res.error || 'pet.triggerAction')
    if (status.value) {
      status.value = {
        ...status.value,
        mood: res.mood ?? status.value.mood,
        affection: typeof res.affection === 'number' ? res.affection : status.value.affection,
      }
    } else {
      void loadStatus()
    }
  } catch (e) {
    actionError.value = `${t('pet.sidebar.unavailable')} · ${e instanceof Error ? e.message : String(e)}`
  } finally {
    busy.value = false
  }
}

async function toggleOverlay(): Promise<void> {
  if (!overlayAvailable || overlayBusy.value) return
  overlayBusy.value = true
  overlayError.value = ''
  const res = await togglePetOverlay()
  if (res.ok) overlayOn.value = res.visible
  else overlayError.value = res.error || 'petOverlayToggle'
  overlayBusy.value = false
}

/** 「打开完整页面」= 原来的 /pet 导航入口：push 后把点击交给侧栏的 handleNavClick。 */
function openFull(): void {
  void router.push('/pet')
  emit('navigate')
}

watch(
  () => rpc.state,
  (state) => {
    if (state === 'connected') void loadStatus()
  },
)

onMounted(() => {
  void loadStatus()
})
</script>

<style scoped>
.sidebar-pet {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  margin: var(--sp-1) var(--sp-2) var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border: 1px solid color-mix(in srgb, var(--border) 75%, transparent);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--sidebar-item-hover) 55%, transparent);
  color: var(--sidebar-text-strong);
}

.sidebar-pet.is-active {
  background: var(--sidebar-item-active);
  box-shadow: inset 2px 0 0 var(--accent-secondary);
}

.sidebar-pet__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  min-width: 0;
}

.sidebar-pet__name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  font-weight: 600;
}

.sidebar-pet__dot {
  flex: none;
  width: 7px;
  height: 7px;
  border-radius: var(--radius-full);
  background: var(--text-dim);
  opacity: 0.5;
}

.sidebar-pet__dot.is-on {
  background: var(--ok);
  opacity: 1;
}

.sidebar-pet__stats {
  display: flex;
  gap: var(--sp-3);
  min-width: 0;
}

.sidebar-pet__stat {
  display: flex;
  align-items: baseline;
  gap: var(--sp-1);
  min-width: 0;
}

.sidebar-pet__k {
  font-size: var(--fs-xs);
  color: var(--text-dim);
}

.sidebar-pet__v {
  font-size: var(--fs-xs);
  color: var(--text);
  font-variant-numeric: tabular-nums;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sidebar-pet__note {
  margin: 0;
  font-size: var(--fs-xs);
  line-height: 1.35;
  color: var(--text-dim);
  overflow-wrap: anywhere;
}

.sidebar-pet__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1);
}

.sidebar-pet__btn {
  flex: 1 1 auto;
  min-width: 0;
  min-height: 26px;
  padding: 2px var(--sp-2);
  border: 1px solid color-mix(in srgb, var(--border) 80%, transparent);
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--sidebar-text-strong);
  font-family: var(--font-sans);
  font-size: var(--fs-xs);
  cursor: pointer;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  transition: transform var(--dur-fast) linear, opacity var(--dur-fast) linear;
}

.sidebar-pet__btn:hover:not(:disabled),
.sidebar-pet__btn:focus-visible:not(:disabled) {
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  border-color: color-mix(in srgb, var(--accent) 45%, transparent);
}

.sidebar-pet__btn:active:not(:disabled) {
  transform: scale(0.98);
}

.sidebar-pet__btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.sidebar-pet__btn--full {
  flex-basis: 100%;
  color: var(--accent);
}

@media (prefers-reduced-motion: reduce) {
  .sidebar-pet__btn {
    transition: none;
  }
  .sidebar-pet__btn:active:not(:disabled) {
    transform: none;
  }
}
</style>
