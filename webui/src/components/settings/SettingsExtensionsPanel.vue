<template>
  <div class="ext-panel">
    <header class="ext-panel__head">
      <h3 class="ext-panel__title">{{ t(`settings.ext.${sectionId}.title`) }}</h3>
      <p class="ext-panel__desc">{{ t(`settings.ext.${sectionId}.desc`) }}</p>
    </header>

    <!-- 宠物：桌宠开关 + 预览提示 -->
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

const props = defineProps<{ sectionId: string }>()
const { t } = useI18n()
const rpc = useRpcStore()

const PET_ENABLED_KEY = 'pet.desktopEnabled'

const petEnabled = ref(localStorage.getItem(PET_ENABLED_KEY) === 'true')
function persistPetEnabled() {
  localStorage.setItem(PET_ENABLED_KEY, String(petEnabled.value))
  window.dispatchEvent(new StorageEvent('storage', { key: PET_ENABLED_KEY }))
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
</style>
