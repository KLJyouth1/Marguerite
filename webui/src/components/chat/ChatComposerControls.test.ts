import { describe, expect, it } from 'vitest'
import runModeSource from './ChatComposerRunMode.vue?raw'
import modelRoutingSource from './ChatComposerModelRouting.vue?raw'
import runModePanelSource from './RunModePanel.vue?raw'
import composerSource from './ChatComposer.vue?raw'
import viewSource from '../../views/ChatView.vue?raw'
import runModeControllerSource from '../../composables/chat/useChatRunModeController.ts?raw'
import appearanceSource from '../settings/SettingsAppearancePanel.vue?raw'
import slashSource from '../../composables/chat/useChatSlashCommands.ts?raw'
import zhHans from '../../locales/zh-Hans.json'

describe('ChatComposer control hierarchy', () => {
  it('keeps legacy execution mode choices out of the composer settings panel', () => {
    expect(composerSource).not.toContain('ChatComposerSettings')
    expect(composerSource).not.toContain('chat.composer.executionMode')
    expect(composerSource).not.toContain('composer-execution-mode')
    expect(composerSource).not.toContain('setElevatedMode')
  })

  it('threads the shield run-mode control through ChatComposer and ChatView', () => {
    // 三个维度现已合并进统一面板 RunModePanel（沙箱/协作/模型路由）
    expect(composerSource).toContain('RunModePanel')
    expect(runModePanelSource).toContain('ChatComposerRunMode')
    expect(runModePanelSource).toContain('<Icon name="shield"')
    expect(composerSource).toContain(':run-mode="runMode"')
    expect(composerSource).toContain(':safe-setup-available="safeSetupAvailable"')
    expect(composerSource).toContain('@set-run-mode="emit(\'setRunMode\', $event)"')
    expect(composerSource).toContain('setRunMode: [mode: SandboxRunMode]')

    // 会话锁定经 RunModePanel 透传给沙箱子组件
    expect(composerSource).toContain(':run-mode-locked="runModeLocked"')
    expect(composerSource).toContain(':run-mode-lock-message="runModeLockMessage"')
    expect(runModePanelSource).toContain('run-mode-panel-lock-tip')

    expect(viewSource).toContain(':run-mode="runMode"')
    expect(viewSource).toContain(':allowed-run-modes="composerAllowedRunModes"')
    expect(viewSource).toContain(':safe-setup-available="composerSafeSetupAvailable"')
    expect(viewSource).toContain('@set-run-mode="setComposerRunMode"')
    expect(viewSource).toContain('useChatRunModePreference')
    expect(viewSource).toContain('setGlobalRunMode')
    expect(viewSource).toContain('useChatRunModeController(viewState,')
    expect(viewSource).toContain('setComposerRunMode,')
    expect(runModeControllerSource).toContain('async function setComposerRunMode(mode: SandboxRunMode)')
    expect(runModeControllerSource).toContain('await setGlobalRunMode(mode)')
    expect(viewSource).toContain(':run-mode-locked="runModeLocked"')
    expect(viewSource).toContain('|| activeRunModeLock.value !== null')
    expect(zhHans.chat.composer.runModeLocked).toBe('运行中，当前会话无法修改')
  })

  it('shares the first-time Safe setup dialog with the composer', () => {
    expect(viewSource).toContain('<SandboxSetupDialog')
    expect(viewSource).toContain(':open="composerSandboxSetupOpen"')
    expect(viewSource).toContain('@confirm="void confirmComposerSandboxSetup()"')
    expect(viewSource).toContain('@background="runComposerSandboxSetupInBackground"')
    expect(viewSource).toContain('useSandboxSetupStore')
    expect(runModeControllerSource).toContain('sandboxSetupStore.startSafeSetup()')
    expect(viewSource).not.toContain('completeComposerSafeSetup')
    expect(runModeControllerSource).toContain('composerRunModeSelectionAction')
  })

  it('offers exactly Safe and Full from the shield popover', () => {
    expect(runModeSource).toContain("value: 'safe'")
    expect(runModeSource).toContain("value: 'full'")
    expect(runModeSource).not.toContain("value: 'standard'")
    expect(runModeSource).not.toContain("value: 'trusted'")
    expect(runModeSource).not.toContain("value: 'on'")
    expect(runModeSource).not.toContain("value: 'bypass'")
    expect(zhHans.chat.composer.runModeSafeDesc)
      .toBe('在沙箱中运行，并遵循你的安全规则。')
  })

  it('offers Safe from the composer only after sandbox setup is ready', () => {
    expect(viewSource).toContain("status.state !== 'ready'")
    expect(viewSource).toContain("allowedRunModes.value.filter((mode) => mode !== 'safe')")
  })

  it('moves visual effects to Appearance settings', () => {
    expect(appearanceSource).toContain('settings.appearance.visualEffectsLabel')
    expect(appearanceSource).toContain('name="appearance_visual_effects"')
    expect(appearanceSource).toContain('@change="setRouterVisualEffectsEnabled"')
    expect(composerSource).not.toContain('setVisualEffectsEnabled')
  })

  it('keeps Coding mode command-first while exposing its active global state', () => {
    expect(composerSource).toContain('v-if="codingModeEnabled"')
    expect(composerSource).toContain('chat-coding-mode-chip')
    expect(composerSource).toContain("emit('setCodingModeEnabled', false)")
    expect(slashSource).toContain("action === 'coding.mode'")
    expect(slashSource).toContain('const enabled = !options.codingModeEnabled.value')
    expect(viewSource).toContain('codingModeEnabled,')
    expect(viewSource).toContain('codingModeSettingsBusy,')
    expect(viewSource).toContain('setCodingModeEnabled,')
    expect(viewSource).toContain('@set-coding-mode-enabled="setComposerCodingModeEnabled"')
  })

  it('completes Slash candidates without executing them from the suggestion menu', () => {
    expect(viewSource).toContain('@click="completeSlashCmd(cmd)"')
    expect(viewSource).not.toContain('@click="selectSlashCmd(cmd)"')
    expect(slashSource).toContain('function completeSlashCmd')
    expect(slashSource).toContain('function activateSlashCmd')
  })
})

describe('ChatComposer model routing contract', () => {
  it('keeps model-routing choices out of the generic composer settings panel', () => {
    expect(composerSource).not.toContain('label="Squilla Router"')
    expect(composerSource).not.toContain('label="LLM Ensemble"')
    expect(composerSource).not.toContain('routerEnabled: boolean')
    expect(composerSource).not.toContain('llmEnsembleEnabled: boolean')
  })

  it('threads the independent model-routing control through ChatComposer and ChatView', () => {
    expect(composerSource).toContain('RunModePanel')
    expect(runModePanelSource).toContain('ChatComposerModelRouting')
    expect(composerSource).toContain(':model-routing-mode="modelRoutingMode"')
    expect(composerSource).toContain(':model-routing-settings-busy="modelRoutingSettingsBusy"')
    expect(composerSource).toContain('@set-model-routing-mode="emit(\'setModelRoutingMode\', $event)"')
    expect(composerSource).toContain('modelRoutingMode: ModelRoutingMode')
    expect(composerSource).toContain('setModelRoutingMode: [mode: ModelRoutingMode]')

    expect(viewSource).toContain(':model-routing-mode="modelRoutingMode"')
    expect(viewSource).toContain(':model-routing-settings-busy="modelRoutingSettingsBusy"')
    expect(viewSource).toContain('@set-model-routing-mode="setComposerModelRoutingMode"')
    expect(viewSource).toContain('setComposerModelRoutingMode,')
    expect(runModeControllerSource).toContain('async function setComposerModelRoutingMode(mode: ModelRoutingMode)')
    expect(runModeControllerSource).toContain('await setModelRoutingMode(mode)')
  })

  it('offers exactly the three mutually-exclusive model-routing modes', () => {
    expect(modelRoutingSource).toContain("value: 'off'")
    expect(modelRoutingSource).toContain("value: 'squilla_router'")
    expect(modelRoutingSource).toContain("value: 'llm_ensemble'")
    expect(modelRoutingSource).not.toContain('setRouterEnabled')
    expect(modelRoutingSource).not.toContain('setLlmEnsembleEnabled')
  })
})
