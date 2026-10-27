<template>
  <div class="ext-panel">
    <header class="ext-panel__head">
      <h3 class="ext-panel__title">{{ t(`settings.ext.${sectionId}.title`) }}</h3>
      <p class="ext-panel__desc">{{ t(`settings.ext.${sectionId}.desc`) }}</p>
    </header>

    <!-- 宠物：桌宠开关 + 皮肤选择 + 预览提示 -->
    <section v-if="sectionId === 'pet'" class="ext-card">
      <label class="control-row" for="pet-desktop-toggle">
        <span class="control-label">{{ t('settings.ext.pet.toggleLabel') }}</span>
        <input
          id="pet-desktop-toggle"
          v-model="petEnabled"
          type="checkbox"
          class="control-input"
          role="switch"
          @change="persistPetEnabled"
        />
      </label>
      <p class="ext-hint">{{ t('settings.ext.pet.hint') }}</p>

      <!-- 桌面悬浮窗：真壳的独立置顶窗口（pet-overlay）。页内桌宠与它是两件事，
           以前只有页内开关、没有任何入口能打开悬浮窗（用户反馈「桌宠不显示」）。 -->
      <label class="control-row" for="pet-overlay-toggle">
        <span class="control-label">{{ t('settings.ext.pet.overlayLabel') }}</span>
        <input
          id="pet-overlay-toggle"
          v-model="petOverlayOn"
          type="checkbox"
          class="control-input"
          role="switch"
          :disabled="!petOverlaySupported"
          @change="persistPetOverlay"
        />
      </label>
      <p class="ext-hint">{{ petOverlaySupported ? t('settings.ext.pet.overlayHint') : t('settings.ext.pet.overlayUnavailable') }}</p>
      <p v-if="petOverlayError" class="ext-hint ext-hint--warn">{{ t('settings.ext.pet.overlayFailed', { error: petOverlayError }) }}</p>

      <h4 class="ext-subtitle">{{ t('settings.ext.pet.skinTitle') }}</h4>
      <div class="pet-skin-grid">
        <button
          v-for="s in petSkins"
          :key="s.id"
          class="pet-skin-opt"
          :class="{ 'is-active': s.id === currentPetSkin }"
          @click="selectPetSkin(s.id)"
        >
          <div class="pet-skin-opt__preview">
            <img v-if="s.kind === 'image'" :src="s.src" :alt="s.name" draggable="false" />
            <span v-else class="pet-skin-opt__emoji">🐱</span>
          </div>
          <span class="pet-skin-opt__name">{{ s.name }}</span>
        </button>
      </div>

      <RouterLink class="btn btn--ghost" :to="{ path: '/pet' }">
        {{ t('settings.ext.pet.openPreview') }}
      </RouterLink>
    </section>

    <!-- 插件市场：market.* RPC（网关后端提供；不可用时容错显示） -->
    <section v-else-if="sectionId === 'pluginMarket'" class="ext-card">
      <div v-if="marketError" class="ext-error">{{ marketError }}</div>
      <ul v-else-if="marketItems.length" class="ext-list">
        <li v-for="item in marketItems" :key="item.name" class="ext-list__row">
          <div class="ext-list__main">
            <strong>{{ item.name }}</strong>
            <span class="ext-list__ver">v{{ item.version }}</span>
          </div>
          <span class="ext-list__desc">{{ item.description }}</span>
        </li>
      </ul>
      <p v-else class="ext-hint">{{ t('settings.ext.pluginMarket.loading') }}</p>
    </section>

    <!-- 企业协同 · SSH：直连 / 隧道两种方式，不引导安装第三方组网工具 -->
    <section v-else-if="sectionId === 'collabSsh'" class="ext-card">
      <p class="ext-hint">{{ t('settings.ext.collabSsh.localNote') }}</p>

      <h4 class="ext-subtitle">{{ t('settings.ext.collabSsh.vpnTitle') }}</h4>
      <ol class="ext-steps">
        <li>{{ t('settings.ext.collabSsh.step1') }}</li>
        <li>{{ t('settings.ext.collabSsh.step2') }}</li>
        <li>{{ t('settings.ext.collabSsh.step3') }}</li>
        <li>{{ t('settings.ext.collabSsh.step4') }}</li>
        <li>{{ t('settings.ext.collabSsh.step5') }}</li>
      </ol>
      <div class="ext-cmdrow">
        <code class="ext-cmd">{{ sshSampleCommand }}</code>
        <button type="button" class="btn btn--ghost btn--sm" @click="copySshCommand">
          {{ copied ? t('settings.ext.collabSsh.copied') : t('settings.ext.collabSsh.copy') }}
        </button>
      </div>

      <h4 class="ext-subtitle">{{ t('settings.ext.collabSsh.securityTitle') }}</h4>
      <ul class="ext-security">
        <li>{{ t('settings.ext.collabSsh.sec1') }}</li>
        <li>{{ t('settings.ext.collabSsh.sec2') }}</li>
        <li>{{ t('settings.ext.collabSsh.sec3') }}</li>
      </ul>
    </section>

    <!-- 通用：说明 + 打开入口 -->
    <section v-else class="ext-card">
      <p class="ext-hint">{{ t(`settings.ext.${sectionId}.hint`) }}</p>
      <div class="ext-actions">
        <RouterLink
          v-for="cta in ctas"
          :key="cta.path"
          class="btn btn--ghost"
          :to="{ path: cta.path }"
        >{{ cta.label }}</RouterLink>
      </div>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRpcStore } from '@/stores/rpc'
import { PET_SKINS, getCurrentSkin, setCurrentSkin } from '@/components/pet/skins'
import {
  petOverlayAvailable,
  petOverlayPreferred,
  setPetOverlayEnabled,
} from '@/composables/usePetOverlay'

const props = defineProps<{ sectionId: string }>()
const { t } = useI18n()
const rpc = useRpcStore()

const PET_ENABLED_KEY = 'pet.desktopEnabled'

// 与 DesktopPet 同一判据：缺省开启，只有显式关过（'false'）才算关。
// 两处若判据不一致，开关会显示「开」而桌宠不出现（或反之）——用户看到的就是坏掉。
const petEnabled = ref(localStorage.getItem(PET_ENABLED_KEY) !== 'false')
const petSkins = PET_SKINS
const currentPetSkin = ref(getCurrentSkin().id)
// 桌面悬浮窗：能力与状态都取真实值（Web 端置灰，不假装可用）
const petOverlaySupported = petOverlayAvailable()
const petOverlayOn = ref(petOverlaySupported && petOverlayPreferred())
const petOverlayError = ref('')
async function persistPetOverlay() {
  petOverlayError.value = ''
  const result = await setPetOverlayEnabled(petOverlayOn.value)
  if (!result.ok) {
    petOverlayOn.value = false
    petOverlayError.value = result.error ?? 'unknown'
  }
}
function persistPetEnabled() {
  localStorage.setItem(PET_ENABLED_KEY, String(petEnabled.value))
  window.dispatchEvent(new StorageEvent('storage', { key: PET_ENABLED_KEY }))
}
function selectPetSkin(id: string) {
  setCurrentSkin(id)
  currentPetSkin.value = id
}

interface MarketItem { name: string; version: string; description: string; kind: string }
const marketItems = ref<MarketItem[]>([])
const marketError = ref('')

const CTA_MAP: Record<string, { path: string; labelKey: string }[]> = {
  plugins: [{ path: '/plugins', labelKey: 'open' }],
  agentPresets: [{ path: '/agents', labelKey: 'openAgents' }],
  webUiPlugins: [{ path: '/plugins', labelKey: 'open' }],
  skinCenter: [{ path: '/settings/appearance', labelKey: 'openAppearance' }],
  communityPlugins: [{ path: '/plugins', labelKey: 'open' }],
  graph: [{ path: '/graph', labelKey: 'open' }],
  collabSsh: [],
}
const ctas = computed(() =>
  (CTA_MAP[props.sectionId] ?? []).map(c => ({ path: c.path, label: t(`settings.ext.${props.sectionId}.${c.labelKey}`) })),
)

// ---- Collab & SSH：示例命令 + 复制回执（纯前端能力，不伪造后端 SSH RPC）----
const sshSampleCommand = computed(() => t('settings.ext.collabSsh.sampleComment'))
const copied = ref(false)
let copiedTimer: ReturnType<typeof setTimeout> | null = null
async function copySshCommand() {
  try {
    await navigator.clipboard.writeText(sshSampleCommand.value)
    copied.value = true
    if (copiedTimer) clearTimeout(copiedTimer)
    copiedTimer = setTimeout(() => { copied.value = false }, 1500)
  } catch {
    /* 剪贴板权限被拒时静默保留原文案（无敏感数据） */
  }
}

async function loadMarket() {
  if (props.sectionId !== 'pluginMarket') return
  try {
    const res = await rpc.call<{ items?: MarketItem[] }>('market.list')
    marketItems.value = res.items ?? []
    if (!marketItems.value.length) marketError.value = t('settings.ext.pluginMarket.empty')
  } catch {
    marketError.value = t('settings.ext.pluginMarket.unavailable')
  }
}

watch(() => props.sectionId, loadMarket)
onMounted(loadMarket)
</script>

<style scoped>
.ext-panel { display: flex; flex-direction: column; gap: var(--sp-4); }
.ext-panel__head { display: flex; flex-direction: column; gap: var(--sp-1); }
.ext-panel__title { font-size: var(--fs-lg); font-weight: 700; margin: 0; }
.ext-panel__desc { color: var(--text-muted); font-size: var(--fs-sm); margin: 0; }
.ext-card {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
}
.control-row { align-items: center; display: flex; gap: var(--sp-3); justify-content: space-between; }
.control-label { font-size: var(--fs-sm); font-weight: 600; }
.ext-hint { color: var(--text-muted); font-size: var(--fs-sm); margin: 0; }
.ext-hint--warn { color: var(--warn); }
.ext-actions { display: flex; flex-wrap: wrap; gap: var(--sp-2); }
.ext-error { color: var(--danger); font-size: var(--fs-sm); }
.ext-list { display: flex; flex-direction: column; gap: var(--sp-2); list-style: none; margin: 0; padding: 0; }
.ext-list__row {
  align-items: baseline;
  border-bottom: 1px solid var(--border);
  display: flex;
  gap: var(--sp-3);
  justify-content: space-between;
  padding-bottom: var(--sp-2);
}
.ext-list__main { display: flex; gap: var(--sp-2); }
.ext-list__ver { color: var(--text-dim); font-size: var(--fs-xs); }
.ext-list__desc { color: var(--text-muted); font-size: var(--fs-sm); text-align: right; }
.ext-subtitle { font-size: var(--fs-sm); font-weight: 700; margin: var(--sp-1) 0 0; }
.ext-steps { color: var(--text-muted); display: flex; flex-direction: column; font-size: var(--fs-sm); gap: 6px; margin: 0; padding-left: 18px; }
.ext-link { color: var(--accent); text-decoration: none; }
.ext-link:hover { text-decoration: underline; }

.pet-skin-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(90px, 1fr)); gap: 8px; margin: var(--sp-2) 0 var(--sp-3); }
.pet-skin-opt {
  display: flex; flex-direction: column; align-items: center; gap: 4px;
  padding: 8px 4px; background: var(--bg); border: 2px solid var(--border);
  border-radius: var(--radius-sm); cursor: pointer; transition: all var(--dur-fast) var(--ease-out);
}
.pet-skin-opt:hover { border-color: color-mix(in srgb, var(--accent) 40%, var(--border)); }
.pet-skin-opt.is-active { border-color: var(--accent); box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 15%, transparent); }
.pet-skin-opt__preview { width: 50px; height: 56px; display: flex; align-items: center; justify-content: center; border-radius: var(--radius-xs); overflow: hidden; }
.pet-skin-opt__preview img { max-width: 100%; max-height: 100%; object-fit: contain; }
.pet-skin-opt__emoji { font-size: 28px; }
.pet-skin-opt__name { font-size: 11px; color: var(--text); text-align: center; }
.ext-cmdrow { align-items: center; background: color-mix(in srgb, var(--text) 6%, transparent); border-radius: var(--radius-sm); display: flex; gap: var(--sp-2); justify-content: space-between; padding: 8px 10px; }
.ext-cmd { font-family: var(--font-mono, monospace); font-size: var(--fs-xs); word-break: break-all; }
.ext-security { color: var(--text-muted); display: flex; flex-direction: column; font-size: var(--fs-xs); gap: 4px; margin: 0; padding-left: 16px; }
</style>
