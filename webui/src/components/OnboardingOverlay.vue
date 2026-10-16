<script setup lang="ts">
// OnboardingOverlay — first-run wizard that blocks the composer until a model
// provider is configured. It is the app-level safety net for users who land on
// a freshly installed gateway with no API key: instead of letting the first
// chat turn fail with an opaque "api_key_missing" error, this overlay guides
// them through provider selection → credential entry → verification in three
// short steps, with an escape hatch to the full Settings dialog.
//
// Architecture:
// - Uses the existing WebSocket RPC layer (useRpcStore), NOT Tauri IPC.
// - Reads readiness via `onboarding.status` (same RPC as SidebarSetupBanner).
// - Fetches provider catalog via `onboarding.catalog`.
// - Configures the chosen provider via `onboarding.provider.configure`.
// - Auto-refreshes when config saves invalidate readiness (onReadinessInvalidated).
// - Dismissal is persisted per-readiness-signature in localStorage, so it
//   re-arms when the readiness state changes (e.g. after a provider is removed).

import { computed, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import Icon from './Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useRpcCall } from '@/composables/useRpc'
import { useDialogA11y } from '@/composables/useDialogA11y'
import {
  optionalSessionRpcCallOptions,
} from '@/composables/chat/sessionBootstrapAdmission'
import {
  onReadinessInvalidated,
  useReadinessSummary,
  invalidateReadiness,
  type ReadinessStatus,
} from '@/composables/setup/useReadinessSummary'

const { t } = useI18n()
const router = useRouter()
const rpc = useRpcStore()

const modalRef = ref<HTMLElement | null>(null)

// ---------------------------------------------------------------------------
// Readiness check — drives overlay visibility.
// ---------------------------------------------------------------------------
const { data: status, loading, execute: refreshStatus } = useRpcCall<ReadinessStatus>(
  'onboarding.status',
  undefined,
  { callOptions: optionalSessionRpcCallOptions },
)
const { needsAction } = useReadinessSummary(status)

// Readiness signature for dismissal tracking. When the signature changes (e.g.
// a provider was removed, re-exposing a missing-env state), the dismissal is
// cleared and the overlay re-arms.
const readinessSignature = computed(() => JSON.stringify({
  n: status.value?.needsOnboarding,
  s: status.value?.llmSource,
}))

const DISMISS_KEY = 'marguerite.onboarding.dismissed'

function loadDismissedSig(): string | null {
  try { return localStorage.getItem(DISMISS_KEY) } catch { return null }
}
function saveDismissedSig(sig: string) {
  try { localStorage.setItem(DISMISS_KEY, sig) } catch { /* private mode */ }
}
function clearDismissed() {
  try { localStorage.removeItem(DISMISS_KEY) } catch { /* private mode */ }
}

const dismissedSig = ref<string | null>(loadDismissedSig())

// The overlay shows when: (1) RPC says action needed, (2) the dismissal
// signature doesn't match the current readiness, and (3) the gateway is
// connected (we don't block users who can't even reach the gateway — that's
// the SetupConnectionPanel's job).
const visible = computed(() =>
  rpc.isConnected
  && needsAction.value
  && dismissedSig.value !== readinessSignature.value,
)

// Escape key + Tab focus trap. Dismissing the overlay is a safe "come back
// later" — it doesn't lose data or skip a required step, so Escape = dismiss.
useDialogA11y(modalRef, visible, dismiss, {})

function dismiss() {
  dismissedSig.value = readinessSignature.value
  saveDismissedSig(readinessSignature.value)
}

// ---------------------------------------------------------------------------
// Cross-component readiness sync — re-fetch when a save hot-applies config.
// ---------------------------------------------------------------------------
let readinessPending = false

function flushReadiness() {
  if (readinessPending || loading.value) return
  readinessPending = true
  void refreshStatus().finally(() => { readinessPending = false })
}

const stopReadinessSync = onReadinessInvalidated(() => {
  readinessPending = true
  flushReadiness()
})

watch(
  [() => rpc.isConnected, loading],
  ([connected, busy]) => {
    if (connected && !busy) flushReadiness()
  },
  { flush: 'sync' },
)

onUnmounted(stopReadinessSync)

// ---------------------------------------------------------------------------
// Wizard state.
// ---------------------------------------------------------------------------
type Step = 'welcome' | 'provider' | 'credential' | 'verifying' | 'done'

const step = ref<Step>('welcome')

// Provider catalog (fetched lazily when the user reaches step 2).
interface ProviderSpec {
  providerId: string
  label?: string
  envKey?: string
  docsUrl?: string
  models?: Array<{ id: string; label?: string }>
}

interface OnboardingCatalog {
  providers?: ProviderSpec[]
}

const { data: catalog, loading: catalogLoading, execute: fetchCatalog } =
  useRpcCall<OnboardingCatalog>('onboarding.catalog', undefined, {
    callOptions: optionalSessionRpcCallOptions,
  })

const providers = computed(() => catalog.value?.providers ?? [])

const selectedProvider = ref<string>('')
const apiKey = ref('')
const apiKeyEnv = ref('')
const modelName = ref('')
const verifyError = ref('')
const probeOk = ref(false)

// Reset wizard state when overlay visibility changes.
watch(visible, (show) => {
  if (show) {
    step.value = 'welcome'
    selectedProvider.value = ''
    apiKey.value = ''
    apiKeyEnv.value = ''
    modelName.value = ''
    verifyError.value = ''
    probeOk.value = false
  }
})

function goToProviderStep() {
  step.value = 'provider'
  if (!catalog.value) void fetchCatalog().catch(() => { /* error in state */ })
}

function selectProvider(providerId: string) {
  selectedProvider.value = providerId
  // Pre-fill the env key hint from the provider spec.
  const spec = providers.value.find((p) => p.providerId === providerId)
  apiKeyEnv.value = spec?.envKey ?? ''
  // Pick the first model as default if available.
  modelName.value = spec?.models?.[0]?.id ?? ''
  step.value = 'credential'
}

function getProviderLabel(id: string): string {
  const spec = providers.value.find((p) => p.providerId === id)
  return spec?.label ?? id
}

async function verifyAndSave() {
  if (!selectedProvider.value) return
  step.value = 'verifying'
  verifyError.value = ''
  probeOk.value = false

  try {
    const payload: Record<string, unknown> = {
      provider: selectedProvider.value,
      model: modelName.value || undefined,
    }
    // api_key (pasted) and api_key_env are mutually exclusive — a pasted key wins.
    if (apiKey.value.trim()) {
      payload.api_key = apiKey.value.trim()
    } else if (apiKeyEnv.value.trim()) {
      payload.api_key_env = apiKeyEnv.value.trim()
    }

    await rpc.call('onboarding.provider.configure', payload)

    // Probe the provider to verify the credential actually works.
    try {
      const probeResult = await rpc.call<{ ok?: boolean; error?: string }>(
        'onboarding.provider.probe',
        { providerId: selectedProvider.value },
      )
      probeOk.value = probeResult?.ok === true
      if (!probeOk.value && probeResult?.error) {
        verifyError.value = probeResult.error
      }
    } catch {
      // Probe failed but config was saved — the provider is configured even if
      // the probe couldn't reach the endpoint (network, rate-limit, etc.).
      // Don't block the user; let them proceed and try a real chat.
      probeOk.value = true
    }

    // Invalidate readiness so all components (sidebar banner, this overlay)
    // re-fetch their snapshots and reflect the new state.
    invalidateReadiness()
    void refreshStatus()

    step.value = 'done'
  } catch (err: unknown) {
    step.value = 'credential'
    verifyError.value = err instanceof Error
      ? err.message
      : t('onboarding.wizard.errors.configure')
  }
}

function finishWizard() {
  // Clear the dismissal so the overlay won't reappear for this readiness state.
  clearDismissed()
  dismissedSig.value = null
  // If the user still needs action (e.g. probe failed), let the sidebar banner
  // handle it; otherwise the overlay stays gone on next readiness check.
  void refreshStatus()
}

function skipToSettings() {
  dismiss()
  void router.push('/settings/auto')
}

// Escape hatch: go to settings provider section directly.
function goToProviderSettings() {
  dismiss()
  void router.push('/settings/providers')
}
</script>

<template>
  <Teleport to="body">
    <Transition name="onboarding-fade">
      <div v-if="visible" class="onboarding-overlay" @click="skipToSettings">
        <div
          ref="modalRef"
          class="onboarding-modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="onboarding-title"
          @click.stop
        >
          <!-- Step 1: Welcome -->
          <template v-if="step === 'welcome'">
            <div class="onboarding-welcome">
              <div class="onboarding-brand">
                <img src="/marguerite-mark.svg" alt="" aria-hidden="true" class="onboarding-brand__mark" />
                <h2 id="onboarding-title" class="onboarding-title">{{ t('onboarding.wizard.welcomeTitle') }}</h2>
              </div>
              <p class="onboarding-subtitle">{{ t('onboarding.wizard.welcomeSubtitle') }}</p>
              <ul class="onboarding-features">
                <li>
                  <Icon name="shield" :size="16" />
                  <span>{{ t('onboarding.wizard.featureLocal') }}</span>
                </li>
                <li>
                  <Icon name="mode" :size="16" />
                  <span>{{ t('onboarding.wizard.featureMultiProvider') }}</span>
                </li>
                <li>
                  <Icon name="sparkles" :size="16" />
                  <span>{{ t('onboarding.wizard.featureAgent') }}</span>
                </li>
              </ul>
              <div class="onboarding-actions">
                <button type="button" class="btn btn--primary onboarding-cta" @click="goToProviderStep">
                  {{ t('onboarding.wizard.getStarted') }}
                </button>
                <button type="button" class="btn btn--ghost" @click="skipToSettings">
                  {{ t('onboarding.wizard.exploreSettings') }}
                </button>
              </div>
            </div>
          </template>

          <!-- Step 2: Provider selection -->
          <template v-else-if="step === 'provider'">
            <div class="onboarding-step-header">
              <button type="button" class="onboarding-back" @click="step = 'welcome'">
                <Icon name="chevronLeft" :size="14" />
                {{ t('onboarding.wizard.back') }}
              </button>
              <h2 id="onboarding-title" class="onboarding-title">{{ t('onboarding.wizard.chooseProvider') }}</h2>
            </div>
            <p class="onboarding-subtitle">{{ t('onboarding.wizard.chooseProviderHint') }}</p>
            <div v-if="catalogLoading" class="onboarding-loading">
              <span class="onboarding-spinner" aria-hidden="true"></span>
              <span>{{ t('onboarding.wizard.loadingProviders') }}</span>
            </div>
            <div v-else class="onboarding-providers">
              <button
                v-for="p in providers"
                :key="p.providerId"
                type="button"
                class="onboarding-provider-card"
                @click="selectProvider(p.providerId)"
              >
                <span class="onboarding-provider-card__label">{{ p.label || p.providerId }}</span>
                <span v-if="p.envKey" class="onboarding-provider-card__env">{{ p.envKey }}</span>
                <Icon name="chevronRight" :size="16" class="onboarding-provider-card__arrow" />
              </button>
            </div>
            <div class="onboarding-actions">
              <button type="button" class="btn btn--ghost" @click="goToProviderSettings">
                {{ t('onboarding.wizard.seeAllProviders') }}
              </button>
            </div>
          </template>

          <!-- Step 3: API Key input -->
          <template v-else-if="step === 'credential' || step === 'verifying'">
            <div class="onboarding-step-header">
              <button type="button" class="onboarding-back" @click="step = 'provider'" :disabled="step === 'verifying'">
                <Icon name="chevronLeft" :size="14" />
                {{ t('onboarding.wizard.back') }}
              </button>
              <h2 id="onboarding-title" class="onboarding-title">
                {{ t('onboarding.wizard.configureProvider', { provider: getProviderLabel(selectedProvider) }) }}
              </h2>
            </div>

            <div class="onboarding-form">
              <label class="onboarding-field">
                <span class="onboarding-field__label">{{ t('onboarding.wizard.apiKey') }}</span>
                <input
                  v-model="apiKey"
                  type="password"
                  class="onboarding-field__input"
                  :placeholder="apiKeyEnv ? t('onboarding.wizard.apiKeyOrEnvHint', { envKey: apiKeyEnv }) : t('onboarding.wizard.apiKeyPlaceholder')"
                  autocomplete="off"
                  spellcheck="false"
                  :disabled="step === 'verifying'"
                />
              </label>

              <details v-if="apiKeyEnv" class="onboarding-field-details">
                <summary>{{ t('onboarding.wizard.useEnvInstead') }}</summary>
                <label class="onboarding-field onboarding-field--env">
                  <span class="onboarding-field__label">{{ t('setup.common.apiKeyEnv') }}</span>
                  <input
                    v-model="apiKeyEnv"
                    type="text"
                    class="onboarding-field__input"
                    :placeholder="apiKeyEnv"
                    spellcheck="false"
                    :disabled="step === 'verifying' || Boolean(apiKey.trim())"
                  />
                </label>
              </details>

              <label class="onboarding-field">
                <span class="onboarding-field__label">{{ t('setup.common.model') }}</span>
                <input
                  v-model="modelName"
                  type="text"
                  class="onboarding-field__input"
                  :placeholder="t('onboarding.wizard.modelPlaceholder')"
                  spellcheck="false"
                  :disabled="step === 'verifying'"
                />
              </label>

              <p v-if="verifyError" class="onboarding-error" role="alert">
                <Icon name="alertTriangle" :size="14" />
                <span>{{ verifyError }}</span>
              </p>
            </div>

            <div class="onboarding-actions">
              <button
                type="button"
                class="btn btn--primary onboarding-cta"
                :disabled="step === 'verifying' || (!apiKey.trim() && !apiKeyEnv.trim())"
                @click="verifyAndSave"
              >
                <span v-if="step === 'verifying'" class="onboarding-spinner onboarding-spinner--sm" aria-hidden="true"></span>
                {{ step === 'verifying' ? t('onboarding.wizard.verifying') : t('onboarding.wizard.verifyAndSave') }}
              </button>
              <button type="button" class="btn btn--ghost" @click="skipToSettings" :disabled="step === 'verifying'">
                {{ t('onboarding.wizard.advancedSettings') }}
              </button>
            </div>
          </template>

          <!-- Step 4: Done -->
          <template v-else-if="step === 'done'">
            <div class="onboarding-done">
              <div class="onboarding-done__icon" :class="{ 'is-ok': probeOk }">
                <Icon :name="probeOk ? 'check' : 'alertTriangle'" :size="32" />
              </div>
              <h2 id="onboarding-title" class="onboarding-title">
                {{ probeOk ? t('onboarding.wizard.ready') : t('onboarding.wizard.savedButProbeFailed') }}
              </h2>
              <p class="onboarding-subtitle">
                {{ probeOk ? t('onboarding.wizard.readyHint') : t('onboarding.wizard.savedButProbeFailedHint') }}
              </p>
              <div class="onboarding-actions">
                <button type="button" class="btn btn--primary onboarding-cta" @click="finishWizard">
                  {{ t('onboarding.wizard.startUsing') }}
                </button>
                <button type="button" class="btn btn--ghost" @click="skipToSettings">
                  {{ t('onboarding.wizard.openSettings') }}
                </button>
              </div>
            </div>
          </template>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.onboarding-overlay {
  position: fixed;
  inset: 0;
  z-index: 1150;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--scrim);
  padding: var(--sp-4);
}

.onboarding-modal {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-modal);
  width: 90%;
  max-width: 480px;
  max-height: 85vh;
  overflow-y: auto;
  padding: var(--sp-6);
  box-shadow: 0 8px 32px color-mix(in srgb, var(--scrim) 32%, transparent);
}

.onboarding-brand {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  margin-bottom: var(--sp-4);
}

.onboarding-brand__mark {
  width: 48px;
  height: 48px;
}

.onboarding-title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 700;
  text-align: center;
}

.onboarding-subtitle {
  margin: 0 0 var(--sp-4);
  color: var(--text-muted);
  font-size: var(--fs-sm);
  line-height: 1.5;
  text-align: center;
}

.onboarding-features {
  list-style: none;
  margin: 0 0 var(--sp-5);
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}

.onboarding-features li {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  font-size: var(--fs-sm);
  color: var(--text-dim);
}

.onboarding-features li :deep(svg),
.onboarding-features li :deep([class*="icon"]) {
  color: var(--accent);
  flex: none;
}

.onboarding-actions {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  align-items: stretch;
  margin-top: var(--sp-4);
}

.onboarding-cta {
  min-height: 40px;
}

.onboarding-step-header {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  margin-bottom: var(--sp-3);
}

.onboarding-back {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: none;
  border: 0;
  color: var(--text-muted);
  font: inherit;
  font-size: var(--fs-sm);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: var(--radius-xs);
}

.onboarding-back:hover:not(:disabled) {
  background: color-mix(in srgb, var(--text) 8%, transparent);
  color: var(--text);
}

.onboarding-back:disabled {
  opacity: 0.4;
  cursor: default;
}

.onboarding-loading {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  color: var(--text-muted);
  font-size: var(--fs-sm);
  padding: var(--sp-4) 0;
}

.onboarding-spinner {
  display: inline-block;
  width: 16px;
  height: 16px;
  border: 2px solid color-mix(in srgb, var(--text) 16%, transparent);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: onboarding-spin 0.6s linear infinite;
}

.onboarding-spinner--sm {
  width: 12px;
  height: 12px;
  border-width: 1.5px;
}

@keyframes onboarding-spin {
  to { transform: rotate(360deg); }
}

.onboarding-providers {
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}

.onboarding-provider-card {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--bg-elevated, transparent);
  cursor: pointer;
  text-align: left;
  font: inherit;
  color: var(--text);
  transition: border-color var(--dur-fast, 120ms), background var(--dur-fast, 120ms);
}

.onboarding-provider-card:hover {
  border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  background: color-mix(in srgb, var(--accent) 8%, transparent);
}

.onboarding-provider-card__label {
  flex: 1;
  font-weight: 600;
  font-size: var(--fs-sm);
}

.onboarding-provider-card__env {
  font-size: 11px;
  color: var(--text-muted);
  font-family: var(--font-mono, monospace);
}

.onboarding-provider-card__arrow {
  color: var(--text-muted);
  flex: none;
}

.onboarding-form {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  margin-bottom: var(--sp-3);
}

.onboarding-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.onboarding-field__label {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-muted);
}

.onboarding-field__input {
  padding: 8px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg);
  color: var(--text);
  font: inherit;
  font-size: var(--fs-sm);
}

.onboarding-field__input:focus {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 1px;
  border-color: var(--accent);
}

.onboarding-field__input:disabled {
  opacity: 0.5;
}

.onboarding-field-details {
  margin-top: -4px;
}

.onboarding-field-details summary {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 0;
}

.onboarding-field--env {
  margin-top: var(--sp-2);
}

.onboarding-error {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  color: var(--danger);
  font-size: var(--fs-xs);
  line-height: 1.4;
  margin: 0;
}

.onboarding-done {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-4) 0;
}

.onboarding-done__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
}

.onboarding-done__icon.is-ok {
  color: var(--ok);
  background: color-mix(in srgb, var(--ok) 12%, transparent);
}

.onboarding-fade-enter-active,
.onboarding-fade-leave-active {
  transition: opacity var(--dur-base, 200ms);
}

.onboarding-fade-enter-from,
.onboarding-fade-leave-to {
  opacity: 0;
}
</style>
