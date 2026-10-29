<template>
  <div class="sk-stage control-stage control-stage--spacious">
    <header class="sk-stage__header control-stage__header">
      <div class="sk-stage__title-block control-stage__title-block">
        <h1 class="sk-stage__title control-stage__title">{{ t('cronSkills.skillsView.title') }}</h1>
        <p class="sk-stage__subtitle control-stage__subtitle">{{ t('cronSkills.skillsView.subtitle') }}</p>
      </div>
      <div class="sk-stage__actions control-stage__actions">
        <div class="sk-search-wrap" :style="{ visibility: activeTab === 'skills' ? 'visible' : 'hidden' }">
          <span class="sk-search-icon">
            <Icon name="search" :size="16" />
          </span>
          <input
            v-model="filterText"
            class="sk-search-input"
            type="search"
            :placeholder="t('cronSkills.skillsView.filterPlaceholder')"
            autocomplete="off"
          />
        </div>
        <button class="btn btn--ghost" data-testid="skills-create-toggle" type="button" @click="showCreate = !showCreate">
          <Icon name="plus" :size="16" />
          <span>{{ t('plugins.createSkill') }}</span>
        </button>
        <button class="btn btn--ghost" data-testid="skills-overview" type="button" @click="skillsOverviewOpen = true">
          <Icon name="skills" :size="16" />
          <span>{{ t('cronSkills.skillsView.overviewTitle') }}</span>
        </button>
      </div>
    </header>

    <!-- 技能覆盖统计（skills.coverage）：总数 / 唯一 / 跳过 / 插件引用合并数 -->
    <section class="sk-coverage" data-testid="skills-coverage">
      <span class="sk-coverage__label">{{ t('skills.hub.coverageTitle') }}</span>
      <template v-if="coverage && coverage.ok">
        <span class="sk-coverage__pill">{{ t('skills.hub.coverageTotal', { count: coverageStats.total }) }}</span>
        <span class="sk-coverage__pill">{{ t('skills.hub.coverageUnique', { count: coverageStats.unique }) }}</span>
        <span class="sk-coverage__pill sk-coverage__pill--warn">{{ t('skills.hub.coverageSkipped', { count: coverageStats.skipped }) }}</span>
        <span class="sk-coverage__pill sk-coverage__pill--accent">{{ t('skills.hub.coverageMerged', { count: coverageStats.merged }) }}</span>
        <span v-if="coverage.plugins" class="sk-coverage__hint">
          {{ t('skills.hub.coveragePlugins', { refs: coverage.plugins.refsTotal ?? 0, resolved: coverage.plugins.resolved ?? 0, unresolved: coverage.plugins.unresolved ?? 0 }) }}
        </span>
      </template>
      <span v-else-if="coverageError" class="sk-coverage__hint sk-coverage__hint--err">{{ coverageError }}</span>
      <span v-else class="sk-coverage__hint">{{ t('skills.hub.coverageUnavailable') }}</span>
    </section>

    <!-- 已停用技能：真实改名后不在扫描结果里，单独回传以便重新启用/删除 -->
    <section v-if="disabledSkills.length" class="sk-disabled" data-testid="skills-disabled">
      <div class="sk-disabled__head">
        <span class="sk-coverage__label">{{ t('skills.manage.disabledTitle') }}</span>
        <span class="sk-coverage__hint">{{ t('skills.manage.disabledHint', { count: disabledSkills.length }) }}</span>
      </div>
      <div class="sk-disabled__list">
        <div v-for="s in disabledSkills" :key="s.name" class="sk-disabled__row">
          <code class="sk-disabled__name">{{ s.name }}</code>
          <span class="sk-disabled__path">{{ s.file_path || '' }}</span>
          <button class="btn btn--sm btn--ghost" :disabled="togglingName === s.name" @click="toggleSkill(s.name, true)">
            {{ t('skills.manage.enable') }}
          </button>
          <button class="btn btn--sm btn--ghost sk-disabled__danger" :disabled="deletingName === s.name" @click="deleteSkill(s.name)">
            {{ t('skills.manage.delete') }}
          </button>
        </div>
      </div>
    </section>

    <Transition name="modal">
      <div v-if="skillsOverviewOpen" class="sk-overview-modal" role="dialog" aria-modal="true" aria-labelledby="skills-overview-title" @click.self="skillsOverviewOpen = false">
        <section class="sk-overview-modal__panel">
          <header class="sk-overview-modal__head">
            <div><span class="sk-overview-modal__eyebrow">SKILLS OVERVIEW</span><h2 id="skills-overview-title">{{ t('cronSkills.skillsView.overviewTitle') }}</h2><p>{{ t('cronSkills.skillsView.overviewDesc') }}</p></div>
            <div class="sk-overview-modal__actions">
              <button class="btn btn--ghost" data-testid="skills-reload" type="button" :disabled="reloading" :aria-busy="reloading" @click="manualReload"><Icon name="refresh" :size="16" /><span>{{ reloading ? t('cronSkills.skillsView.refreshing') : t('cronSkills.skillsView.reload') }}</span></button>
              <button class="btn btn--ghost sk-overview-modal__close" type="button" :aria-label="t('common.close')" @click="skillsOverviewOpen = false"><Icon name="x" :size="18" /></button>
            </div>
          </header>
          <SkillsStats :tiles="statTiles" :active-key="statusFilter" :proposal-count="proposals.length" @select="selectStatusFromOverview" @show-proposals="showProposalsFromOverview" />
        </section>
      </div>
    </Transition>

    <div class="sk-tabs" role="tablist" :aria-label="t('cronSkills.skillsView.tabsLabel')">
      <button
        v-for="tab in hubTabs"
        :id="'sk-tab-' + tab.id"
        :key="tab.id"
        class="sk-tab"
        :class="{ 'is-active': activeTab === tab.id }"
        type="button"
        role="tab"
        :aria-selected="activeTab === tab.id"
        :aria-controls="tab.id === 'community' ? 'sk-panel-community' : 'sk-panel-skills'"
        @click="activeTab = tab.id"
      >
        <Icon :name="tab.icon" :size="16" />
        <span>{{ t(tab.labelKey) }}</span>
      </button>
    </div>

    <div v-show="activeTab !== 'community'" class="sk-panel" role="tabpanel" id="sk-panel-skills" aria-labelledby="sk-tab-skills">
      <div class="sk-installed">
        <section v-if="showCreate" class="sk-create">
          <h2 class="sk-create__title">{{ t('plugins.skill.formTitle') }}</h2>
          <div class="sk-create__form">
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.nameLabel') }}</span>
              <input v-model="createForm.name" class="sk-create__input" :placeholder="t('plugins.skill.namePlaceholder')" />
            </label>
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.descriptionLabel') }}</span>
              <input v-model="createForm.description" class="sk-create__input" :placeholder="t('plugins.skill.descriptionPlaceholder')" />
            </label>
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.triggersLabel') }}</span>
              <input v-model="createForm.triggers" class="sk-create__input" :placeholder="t('plugins.skill.triggersPlaceholder')" />
            </label>
            <label class="sk-create__field sk-create__field--full">
              <span>{{ t('plugins.skill.promptLabel') }}</span>
              <textarea v-model="createForm.prompt_template" class="sk-create__textarea" rows="5" :placeholder="t('plugins.skill.promptPlaceholder')"></textarea>
            </label>
            <div class="sk-create__actions">
              <button class="btn btn--ghost" type="button" @click="showCreate = false">{{ t('plugins.skill.cancel') }}</button>
              <button class="btn btn--primary" type="button" :disabled="creating" @click="createSkill">
                {{ creating ? t('plugins.skill.saving') : t('plugins.skill.save') }}
              </button>
            </div>
            <div v-if="createResult" class="sk-create__result" :class="{ 'is-error': !createOk }">{{ createResult }}</div>
          </div>
        </section>

        <section v-if="isDesktop" v-show="activeTab === 'tools'" class="sk-launch" :aria-label="t('cronSkills.skillsView.launchTitle')">
          <div class="sk-launch__head">
            <h3 class="sk-launch__title">{{ t('cronSkills.skillsView.launchTitle') }}</h3>
            <p class="sk-launch__desc">{{ t('cronSkills.skillsView.launchDesc') }}</p>
          </div>
          <div class="sk-launch__grid">
            <button
              type="button"
              class="sk-launch-card"
              :class="{ 'is-off': !labs.baota }"
              data-testid="launch-baota"
              @click="launchShortcut('baota')"
            >
              <span class="sk-launch-card__icon"><Icon name="terminal" :size="18" /></span>
              <span class="sk-launch-card__body">
                <strong>{{ t('cronSkills.skillsView.launchBaota') }}</strong>
                <span>{{ t('cronSkills.skillsView.launchBaotaDesc') }}</span>
              </span>
              <span class="sk-launch-card__state">{{ labs.baota ? t('cronSkills.skillsView.launchOn') : t('cronSkills.skillsView.launchOff') }}</span>
            </button>
            <button
              type="button"
              class="sk-launch-card"
              :class="{ 'is-off': !labs.desktopControl }"
              data-testid="launch-desktop-control"
              @click="launchShortcut('desktopControl')"
            >
              <span class="sk-launch-card__icon"><Icon name="keyboard" :size="18" /></span>
              <span class="sk-launch-card__body">
                <strong>{{ t('cronSkills.skillsView.launchDesktop') }}</strong>
                <span>{{ t('cronSkills.skillsView.launchDesktopDesc') }}</span>
              </span>
              <span class="sk-launch-card__state">{{ labs.desktopControl ? t('cronSkills.skillsView.launchOn') : t('cronSkills.skillsView.launchOff') }}</span>
            </button>
            <button
              type="button"
              class="sk-launch-card"
              :class="{ 'is-off': !labs.fpsAssistant }"
              data-testid="launch-fps"
              @click="launchShortcut('fpsAssistant')"
            >
              <span class="sk-launch-card__icon"><Icon name="gauge" :size="18" /></span>
              <span class="sk-launch-card__body">
                <strong>{{ t('cronSkills.skillsView.launchFps') }}</strong>
                <span>{{ t('cronSkills.skillsView.launchFpsDesc') }}</span>
              </span>
              <span class="sk-launch-card__state">{{ labs.fpsAssistant ? t('cronSkills.skillsView.launchOn') : t('cronSkills.skillsView.launchOff') }}</span>
            </button>
          </div>
        </section>

        <section v-show="activeTab === 'tools'" class="sk-tools" :aria-label="t('cronSkills.skillsView.toolsTitle')">
          <div class="sk-tools__head">
            <h3 class="sk-tools__title">{{ t('cronSkills.skillsView.toolsTitle') }}</h3>
            <p class="sk-tools__desc">{{ t('cronSkills.skillsView.toolsDesc') }}</p>
          </div>
          <div class="sk-tools__mcp">
            <span class="sk-tools__mcp-label">{{ t('cronSkills.skillsView.mcpTitle') }}</span>
            <template v-if="mcpServers.length">
              <span
                v-for="server in mcpServers"
                :key="server.name"
                class="sk-tools__mcp-chip"
                :title="mcpChipTitle(server)"
              >{{ server.name }}<span
                v-if="mcpToolCount(server.name) !== null"
                class="sk-tools__mcp-count"
              >{{ t('cronSkills.skillsView.mcpToolsCount', { count: mcpToolCount(server.name) }) }}</span></span>
              <button
                class="sk-tools__mcp-probe"
                type="button"
                data-testid="mcp-probe"
                :disabled="mcpProbing"
                @click="probeMcpTools"
              >
                {{ mcpProbing
                  ? t('cronSkills.skillsView.mcpProbing')
                  : t('cronSkills.skillsView.mcpProbe') }}
              </button>
            </template>
            <span v-else class="sk-tools__mcp-empty">{{ t('cronSkills.skillsView.mcpEmpty') }}</span>
            <span v-if="mcpProbeError" class="sk-tools__mcp-error" data-testid="mcp-probe-error">{{ mcpProbeError }}</span>
          </div>
          <div class="sk-tools__grid">
            <div v-if="toolGroups.baota.length" class="sk-tools-card" data-testid="tools-baota">
              <span class="sk-tools-card__icon"><Icon name="terminal" :size="18" /></span>
              <span class="sk-tools-card__body">
                <strong>{{ t('cronSkills.skillsView.toolsGroupBaota') }}</strong>
                <span>{{ t('cronSkills.skillsView.toolsCount', { count: toolGroups.baota.length }) }}</span>
              </span>
              <span class="sk-tools-card__names">{{ toolGroups.baota.map(tool => tool.name).join(', ') }}</span>
              <button
                class="sk-tools-card__cred"
                type="button"
                data-testid="baota-credentials"
                :title="t('cronSkills.skillsView.baotaCredentialsTitle')"
                @click="baotaCredOpen = true"
              >
                <Icon name="key" :size="13" />
                <span>{{ t('cronSkills.skillsView.baotaCredentials') }}</span>
              </button>
            </div>
            <div v-if="toolGroups.desktop.length" class="sk-tools-card" data-testid="tools-desktop">
              <span class="sk-tools-card__icon"><Icon name="keyboard" :size="18" /></span>
              <span class="sk-tools-card__body">
                <strong>{{ t('cronSkills.skillsView.toolsGroupDesktop') }}</strong>
                <span>{{ t('cronSkills.skillsView.toolsCount', { count: toolGroups.desktop.length }) }}</span>
              </span>
              <span class="sk-tools-card__names">{{ toolGroups.desktop.map(tool => tool.name).join(', ') }}</span>
            </div>
            <div v-if="toolGroups.rest.length" class="sk-tools-card">
              <span class="sk-tools-card__icon"><Icon name="skills" :size="18" /></span>
              <span class="sk-tools-card__body">
                <strong>{{ t('cronSkills.skillsView.toolsGroupOther') }}</strong>
                <span>{{ t('cronSkills.skillsView.toolsCount', { count: toolGroups.rest.length }) }}</span>
              </span>
              <span class="sk-tools-card__names">{{ toolGroups.rest.map(tool => tool.name).join(', ') }}</span>
            </div>
            <div v-if="toolsLoading && !toolsCatalog.length" class="sk-tools-card sk-tools-card--loading">
              <span class="sk-tools-card__body"><span>{{ t('cronSkills.skillsView.toolsLoading') }}</span></span>
            </div>
          </div>

          <!-- 工具启停（补丁层）：写 tool_patch.json，智能体下一轮生效，无需重启守护进程 -->
          <div v-if="toolsCatalog.length" class="sk-patch" data-testid="tools-patch">
            <div class="sk-patch__head">
              <span class="sk-patch__title">{{ t('cronSkills.skillsView.toolsPatchTitle') }}</span>
              <span class="sk-patch__hint">{{ t('cronSkills.skillsView.toolsPatchHint') }}</span>
            </div>
            <p v-if="patchError" class="sk-patch__error" data-testid="tools-patch-error">{{ patchError }}</p>
            <ul class="sk-patch__list">
              <li
                v-for="tool in toolsCatalog"
                :key="tool.name"
                class="sk-patch__row"
                :class="{ 'is-off': tool.enabled === false }"
              >
                <span class="sk-patch__name">{{ tool.name }}</span>
                <span class="sk-patch__desc" :title="tool.description">{{ tool.description }}</span>
                <button
                  class="sk-patch__toggle"
                  :class="{ 'is-off': tool.enabled === false }"
                  type="button"
                  :aria-pressed="tool.enabled !== false"
                  :disabled="patchBusy"
                  @click="toggleTool(tool)"
                >
                  {{ tool.enabled !== false
                    ? t('cronSkills.skillsView.toolsPatchEnable')
                    : t('cronSkills.skillsView.toolsPatchDisable') }}
                </button>
              </li>
            </ul>
            <p v-if="patchInactive.length" class="sk-patch__inactive" data-testid="tools-patch-inactive">
              {{ t('cronSkills.skillsView.toolsPatchInactive', { names: patchInactive.join(', ') }) }}
            </p>
          </div>
        </section>

        <details
          v-if="proposalsSettings.available"
          v-show="activeTab === 'proposals'"
          class="sk-group sk-group--ap-settings"
          :open="proposalsSettingsOn"
        >
          <summary class="sk-group__head">
            <span class="sk-group__caret">▾</span>
            <span class="sk-group__label">{{ t('cronSkills.autoPropose.title') }}</span>
            <span class="sk-group__count">{{ proposalsSettingsOn ? t('cronSkills.autoPropose.on') : t('cronSkills.autoPropose.off') }}</span>
            <span class="sk-group__meta">{{ t('cronSkills.autoPropose.meta') }}</span>
          </summary>
          <div class="sk-ap-settings">
            <label class="sk-ap-toggle">
              <ControlSwitch
                :checked="proposalsSettings.enabled"
                :aria-label="t('cronSkills.autoPropose.scheduledLabel')"
                @change="(v) => toggleAutoPropose('enabled', v)"
              />
              <span class="sk-ap-toggle__label">{{ t('cronSkills.autoPropose.scheduledLabel') }}</span>
              <i18n-t keypath="cronSkills.autoPropose.scheduledHint" tag="span" class="sk-ap-toggle__hint">
                <template #cron><code>{{ proposalsSettings.cron || '0 5 * * *' }}</code></template>
              </i18n-t>
            </label>
            <label class="sk-ap-toggle">
              <ControlSwitch
                :checked="proposalsSettings.on_dream_complete"
                :aria-label="t('cronSkills.autoPropose.dreamLabel')"
                @change="(v) => toggleAutoPropose('on_dream_complete', v)"
              />
              <span class="sk-ap-toggle__label">{{ t('cronSkills.autoPropose.dreamLabel') }}</span>
              <span class="sk-ap-toggle__hint">{{ t('cronSkills.autoPropose.dreamHint') }}</span>
            </label>
            <label class="sk-ap-toggle">
              <ControlSwitch
                :checked="proposalsSettings.auto_enable"
                :aria-label="t('cronSkills.autoPropose.autoEnableLabel')"
                @change="(v) => toggleAutoPropose('auto_enable', v)"
              />
              <span class="sk-ap-toggle__label">{{ t('cronSkills.autoPropose.autoEnableLabel') }}</span>
              <i18n-t keypath="cronSkills.autoPropose.autoEnableHint" tag="span" class="sk-ap-toggle__hint">
                <template #risk><code>{{ proposalsSettings.auto_enable_max_risk || 'low' }}</code></template>
              </i18n-t>
            </label>
            <label class="sk-ap-toggle">
              <span class="sk-ap-toggle__label">{{ t('cronSkills.autoPropose.riskCeilingLabel') }}</span>
              <select
                class="sk-ap-select"
                :value="proposalsSettings.auto_enable_max_risk || 'low'"
                @change="setAutoEnableRisk(($event.target as HTMLSelectElement).value)"
              >
                <option value="low">{{ t('cronSkills.autoPropose.riskLow') }}</option>
                <option value="medium">{{ t('cronSkills.autoPropose.riskMedium') }}</option>
                <option value="high">{{ t('cronSkills.autoPropose.riskHigh') }}</option>
              </select>
              <span class="sk-ap-toggle__hint">{{ t('cronSkills.autoPropose.riskCeilingHint') }}</span>
            </label>
          </div>
        </details>

        <PendingSkillProposals
          v-show="activeTab === 'proposals'"
          ref="proposalsPanelRef"
          :proposals="proposals"
          @show="openProposalDialog"
          @accept="acceptProposal"
          @reject="rejectProposal"
        />
        <AutoEnabledSkills v-show="activeTab === 'proposals'" :skills="autoEnabledSkills" @disable="disableAutoEnabled" />
        <SkillGroup
          v-show="activeTab === 'skills'"
          :title="t('cronSkills.skillsView.metaSkillsTitle')"
          :description="t('cronSkills.skillsView.metaSkillsDesc')"
          :skills="metaSkills"
          group-class="sk-group--meta"
          meta
          @open="openSkillDialog"
        />
        <SkillGroup
          v-for="layer in visibleLayerGroups"
          v-show="activeTab === 'skills'"
          :key="layer.key"
          :title="skillLayerLabel(layer.key)"
          :description="skillLayerHelp(layer.key)"
          :skills="layer.skills"
          @open="openSkillDialog"
        />

        <EmptyState
          v-if="installedEmpty"
          v-show="activeTab === 'skills'"
          icon="skills"
          :icon-size="36"
          test-id="skills-empty"
        >
          <p class="empty-panel__hint">
            <i18n-t v-if="filterText" keypath="cronSkills.skillsView.noMatchFilter">
              <template #filter><strong>{{ filterText }}</strong></template>
            </i18n-t>
            <template v-else>{{ emptyMessage }}</template>
          </p>
        </EmptyState>
      </div>
    </div>

    <div v-show="activeTab === 'community'" class="sk-panel" role="tabpanel" id="sk-panel-community" aria-labelledby="sk-tab-community">
      <SkillsRegistryPanel
        v-model:registry-query="registryQuery"
        v-model:github-url="githubUrl"
        :results="registryResults"
        :loading="registryLoading"
        :installing-id="installingId"
        @search="searchRegistry"
        @install-github="installGithub"
        @install="installSkill"
      />
    </div>

    <SkillDetailDialog
      :skill="selectedSkill"
      :proposal="selectedProposal"
      :loading-content="selectedSkillLoading"
      :content-error="selectedSkillError"
      :install-feedback="installFeedback"
      :installing-deps-id="installingDepsId"
      :uninstalling-name="deletingName || uninstallingName"
      :toggling-name="togglingName"
      @close="closeDialog"
      @install-deps="installDepsAndMaybeClose"
      @uninstall="uninstallSkillAndClose"
      @toggle="toggleSkillFromDetail"
      @edit="openSkillEditor"
    />

    <Transition name="modal">
      <div v-if="editOpen" class="sk-edit-modal" role="dialog" aria-modal="true" aria-labelledby="skill-edit-title" @click.self="editOpen = false">
        <section class="sk-edit-modal__panel">
          <header class="sk-edit-modal__head">
            <div>
              <span class="sk-overview-modal__eyebrow">SKILL</span>
              <h2 id="skill-edit-title">{{ t('skills.manage.editTitle', { name: editForm.name }) }}</h2>
            </div>
            <button class="btn btn--ghost sk-overview-modal__close" type="button" :aria-label="t('common.close')" @click="editOpen = false"><Icon name="x" :size="18" /></button>
          </header>
          <div class="sk-edit-modal__body">
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.descriptionLabel') }}</span>
              <input v-model="editForm.description" class="sk-create__input" />
            </label>
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.triggersLabel') }}</span>
              <input v-model="editForm.triggers" class="sk-create__input" :placeholder="t('plugins.skill.triggersPlaceholder')" />
            </label>
            <label class="sk-create__field">
              <span>{{ t('plugins.skill.promptLabel') }}</span>
              <textarea v-model="editForm.prompt_template" class="sk-create__textarea" rows="10"></textarea>
            </label>
            <div v-if="editMessage" class="sk-create__result" :class="{ 'is-error': !editOk }">{{ editMessage }}</div>
          </div>
          <footer class="sk-edit-modal__foot">
            <button class="btn btn--ghost" @click="editOpen = false">{{ t('plugins.skill.cancel') }}</button>
            <button class="btn btn--primary" :disabled="editSaving" @click="saveSkillEdit">
              {{ editSaving ? t('plugins.skill.saving') : t('plugins.skill.save') }}
            </button>
          </footer>
        </section>
      </div>
    </Transition>

    <BaotaCredentialsDialog
      :open="baotaCredOpen"
      @close="baotaCredOpen = false"
      @saved="loadTools()"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onActivated, onDeactivated, onUnmounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import Icon from '@/components/Icon.vue'
import EmptyState from '@/components/EmptyState.vue'
import ControlSwitch from '@/components/ControlSwitch.vue'
import AutoEnabledSkills from '@/components/skills/AutoEnabledSkills.vue'
import BaotaCredentialsDialog from '@/components/skills/BaotaCredentialsDialog.vue'
import PendingSkillProposals from '@/components/skills/PendingSkillProposals.vue'
import SkillDetailDialog from '@/components/skills/SkillDetailDialog.vue'
import SkillGroup from '@/components/skills/SkillGroup.vue'
import SkillsRegistryPanel from '@/components/skills/SkillsRegistryPanel.vue'
import SkillsStats from '@/components/skills/SkillsStats.vue'
import { useSkillProposals } from '@/composables/skills/useSkillProposals'
import { useSkillDetailController } from '@/composables/skills/useSkillDetailController'
import { useSkillRegistry } from '@/composables/skills/useSkillRegistry'
import { skillLayerHelp, skillLayerLabel, useSkillsCatalog } from '@/composables/skills/useSkillsCatalog'
import { useToasts } from '@/composables/useToasts'
import { useConfirm } from '@/composables/useConfirm'
import { applyDisabled, computeNextDisabled, disabledNames } from '@/utils/tools/toolPatch'
import { refreshToolSources } from '@/composables/useToolSources'
import { usePlatform } from '@/platform'
import { useRpcStore } from '@/stores/rpc'
import type { IconName } from '@/utils/icons'
import type { Proposal, Skill } from '@/types/skills'

interface SkillReloadError {
  name?: string
  path?: string
  message?: string
  kept_previous?: boolean
}

interface SkillReloadResult {
  success: boolean
  changed: boolean
  partial: boolean
  generation: number
  added?: string[]
  removed?: string[]
  modified?: string[]
  errors?: SkillReloadError[]
}

const { t } = useI18n()
const skillsOverviewOpen = ref(false)
const baotaCredOpen = ref(false)
const { pushToast } = useToasts()
const { confirm } = useConfirm()
const rpc = useRpcStore()
const router = useRouter()
const isDesktop = usePlatform().capabilities.isDesktop
type SkillsTabId = 'skills' | 'tools' | 'proposals' | 'community'
const activeTab = ref<SkillsTabId>('skills')
const hubTabs: Array<{ id: SkillsTabId; labelKey: string; icon: IconName }> = [
  { id: 'skills', labelKey: 'skills.hub.tabSkills', icon: 'skills' },
  { id: 'tools', labelKey: 'skills.hub.tabTools', icon: 'terminal' },
  { id: 'proposals', labelKey: 'skills.hub.tabProposals', icon: 'listChecks' },
  { id: 'community', labelKey: 'skills.hub.tabCommunity', icon: 'download' },
]
const reloading = ref(false)
const selectedProposal = ref<Proposal | null>(null)
const proposalsPanelRef = ref<InstanceType<typeof PendingSkillProposals> | null>(null)

// 创建技能（原 PluginsView 里的表单迁移到此，插件页不再承担技能创建）。
const showCreate = ref(false)
const creating = ref(false)
const createResult = ref('')
const createOk = ref(false)
const createForm = reactive({ name: '', description: '', prompt_template: '', triggers: '' })

async function createSkill() {
  if (!createForm.name.trim() || !createForm.description.trim() || !createForm.prompt_template.trim()) {
    createOk.value = false
    createResult.value = t('plugins.skill.requiredFields')
    return
  }
  creating.value = true
  createResult.value = ''
  try {
    const res = await rpc.call<{ success: boolean; message: string }>('skills.create', {
      name: createForm.name.trim(),
      description: createForm.description.trim(),
      prompt_template: createForm.prompt_template,
      triggers: createForm.triggers.split(',').map(s => s.trim()).filter(Boolean),
    })
    createOk.value = res.success
    createResult.value = res.message
    if (res.success) {
      createForm.name = ''
      createForm.description = ''
      createForm.prompt_template = ''
      createForm.triggers = ''
      showCreate.value = false
      await loadData()
    }
  } catch (e) {
    createOk.value = false
    createResult.value = t('plugins.skill.createFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    creating.value = false
  }
}

// 顶部覆盖率（skills.coverage）：loaded 是去重后的唯一技能数；
// yaml + skillMd 是扫描成功文件总数（去重前）；mergedTotal 含插件引用合并后的总数。
interface SkillsCoverage {
  ok: boolean
  loaded?: number
  yaml?: number
  skillMd?: number
  skippedTotal?: number
  mergedTotal?: number
  plugins?: { refsTotal?: number; resolved?: number; unresolved?: number }
  error?: string
  code?: string
}
const coverage = ref<SkillsCoverage | null>(null)
const coverageError = ref('')
const coverageStats = computed(() => ({
  total: (coverage.value?.yaml ?? 0) + (coverage.value?.skillMd ?? 0),
  unique: coverage.value?.loaded ?? 0,
  skipped: coverage.value?.skippedTotal ?? 0,
  merged: coverage.value?.mergedTotal ?? 0,
}))

async function loadCoverage() {
  coverageError.value = ''
  try {
    await rpc.waitForConnection()
    const res = await rpc.call<SkillsCoverage>('skills.coverage', {})
    coverage.value = res ?? null
    if (res?.ok === false) coverageError.value = res.error || t('skills.hub.coverageUnavailable')
  } catch (e) {
    coverage.value = null
    coverageError.value = e instanceof Error ? e.message : String(e)
  }
}

// 快捷入口：与设置 › 高级 › Labs 共享同一组 localStorage 开关。直接读键（与
// SettingsAdvancedPanel 完全一致的约定），不引入 chat 依赖。桌面工具箱在此
// 一键唤起：未启用时给引导 toast，已启用时把预填任务交到会话（自动发送）。
const BAOTA_KEY = 'marguerite.labs.baota'
const DESKTOP_CONTROL_KEY = 'marguerite.labs.desktopControl'
const FPS_ASSISTANT_KEY = 'marguerite.labs.fpsAssistant'
type LaunchKey = 'baota' | 'desktopControl' | 'fpsAssistant'
const labs = ref<Record<LaunchKey, boolean>>({
  baota: readLabs(BAOTA_KEY),
  desktopControl: readLabs(DESKTOP_CONTROL_KEY),
  fpsAssistant: readLabs(FPS_ASSISTANT_KEY),
})
function readLabs(key: string): boolean {
  try { return localStorage.getItem(key) === '1' } catch { return false }
}
const LAUNCH_PREFILLS: Record<LaunchKey, string> = {
  baota: '打开宝塔面板运维工具箱：查看当前服务器概况（站点、数据库、SSL、计划任务与容器）。',
  desktopControl: '使用桌面级 Computer Use（system_control）做一次自检：读取屏幕尺寸与当前光标位置，并简述可用操作。',
  fpsAssistant: '启动 FPS 第一人称射击操作助手（skills/fps）：按操作流程帮我校准鼠标灵敏度、压枪与跟枪参数。',
}
const LAUNCH_LABELS: Record<LaunchKey, string> = {
  baota: '宝塔面板运维工具箱',
  desktopControl: '桌面级 Computer Use',
  fpsAssistant: 'FPS 第一人称射击助手',
}
function launchShortcut(key: LaunchKey) {
  if (!labs.value[key]) {
    pushToast(t('cronSkills.skillsView.launchNeedsEnable', { lab: LAUNCH_LABELS[key] }), { tone: 'warn' })
    return
  }
  // 与会话 Hub「开始任务」相同的交接契约：预填并自动发送。
  router.push({ path: '/chat/new', state: { prefill: LAUNCH_PREFILLS[key], autosend: true } })
}

// 工具与能力：网关注册的内置工具目录（tools.catalog）+ 已配置的 MCP 服务器
// （config.get → mcp.servers）。宝塔 baota.*（内置）与 baota_*（btpanel-mcp）
// 与 Computer Use（system_control）单独成卡展示，其余工具折叠为明细。
interface CatalogTool {
  name: string
  description: string
  source?: string
  server?: string
  /** 工具面 patch 层状态；缺省（旧后端）视为启用 */
  enabled?: boolean
}
interface McpServerRow { name: string; command?: string; url?: string }
const toolsLoading = ref(false)
const toolsCatalog = ref<CatalogTool[]>([])
const mcpServers = ref<McpServerRow[]>([])
// 工具启停（补丁层）：patchBusy 防连点，patchError 如实展示后端错误，
// patchInactive 是「补丁里写了、但当前工具面没有」的名字（打错字时不静默）。
const patchBusy = ref(false)
const patchError = ref('')
const patchInactive = ref<string[]>([])
function isBaotaTool(name: string): boolean {
  return name.startsWith('baota.') || name.startsWith('baota_')
}
const toolGroups = computed(() => {
  const baota = toolsCatalog.value.filter(t => isBaotaTool(t.name))
  const desktop = toolsCatalog.value.filter(t => t.name === 'system_control')
  const rest = toolsCatalog.value.filter(
    t => !isBaotaTool(t.name) && t.name !== 'system_control',
  )
  return { baota, desktop, rest }
})
async function loadTools() {
  if (toolsLoading.value) return
  toolsLoading.value = true
  try {
    await rpc.waitForConnection()
    const catalog = await rpc.call<{ items?: CatalogTool[] }>('tools.catalog')
    toolsCatalog.value = catalog?.items || []
    const cfg = await rpc.call<{ mcp?: { servers?: McpServerRow[] } }>('config.get')
    mcpServers.value = cfg?.mcp?.servers || []
    await loadToolPatch()
  } catch {
    // 工具目录是补充信息：加载失败保持空态，不打扰主流程
  } finally {
    toolsLoading.value = false
  }
}

/** 读取补丁状态：`inactive` 与读取错误都要如实展示，不做静默兜底。 */
async function loadToolPatch() {
  try {
    const res = await rpc.call<{ ok?: boolean; inactive?: string[]; error?: string | null }>(
      'tools.patch.get',
    )
    patchInactive.value = res?.inactive || []
    patchError.value = res?.ok === false ? String(res?.error || '') : ''
  } catch {
    // 补丁状态是补充信息，读取失败不影响工具目录展示
  }
}

// ── MCP 工具清单 ──────────────────────────────────────────────────────────
// `tools.catalog` 只覆盖本地工具，MCP 工具要连服务器才知道。这里是**显式探测**
// （不带 cachedOnly），与聊天渲染路径的「只读缓存」区分开：渲染不产生网络副作用。
interface McpServerTools { name: string; ok: boolean; count?: number | null; error?: string | null }
const mcpProbing = ref(false)
const mcpServerTools = ref<McpServerTools[]>([])
const mcpProbeError = ref('')

async function probeMcpTools() {
  if (mcpProbing.value) return
  mcpProbing.value = true
  mcpProbeError.value = ''
  try {
    const res = await rpc.call<{ ok?: boolean; servers?: McpServerTools[] }>('mcp.tools.list')
    mcpServerTools.value = res?.servers || []
    // 失败的服务器如实列出（一个连不上不影响其它服务器，但绝不静默）
    const bad = mcpServerTools.value.filter(s => !s.ok)
    if (bad.length) mcpProbeError.value = bad.map(s => `${s.name}: ${s.error || ''}`).join('；')
    // 探测成功后让聊天里的工具来源徽标立即可用（否则要刷新页面才生效）
    await refreshToolSources()
  } catch (e) {
    mcpProbeError.value = String(e)
  } finally {
    mcpProbing.value = false
  }
}

/** 某服务器提供的工具数；未探测 / 探测失败时返回 null（不显示，也不填 0 冒充）。 */
function mcpToolCount(name: string): number | null {
  const hit = mcpServerTools.value.find(s => s.name === name)
  if (!hit || !hit.ok || typeof hit.count !== 'number') return null
  return hit.count
}

function mcpChipTitle(server: McpServerRow): string {
  const hit = mcpServerTools.value.find(s => s.name === server.name)
  if (hit && !hit.ok) return String(hit.error || '')
  return server.url || server.command || ''
}

/** 切换单个工具的启停：整体覆盖写补丁，成功后本地同步（不做乐观更新，避免与后端不一致）。 */
async function toggleTool(tool: CatalogTool) {
  if (patchBusy.value) return
  patchBusy.value = true
  try {
    const nextEnabled = tool.enabled === false
    const disabled = computeNextDisabled(disabledNames(toolsCatalog.value), tool.name, nextEnabled)
    const res = await rpc.call<{
      ok?: boolean
      disabled?: string[]
      inactive?: string[]
      error?: string
    }>('tools.patch.set', { disabled })
    if (res?.ok === false) {
      patchError.value = String(res.error || '')
      return
    }
    patchError.value = ''
    patchInactive.value = res?.inactive || []
    toolsCatalog.value = applyDisabled(toolsCatalog.value, res?.disabled || [])
  } catch (e) {
    patchError.value = String(e)
  } finally {
    patchBusy.value = false
  }
}

let loadData: () => Promise<boolean>

const proposalsModel = useSkillProposals(rpc, async () => { await loadData() })
const {
  proposals,
  autoEnabledSkills,
  proposalsSettings,
  proposalsSettingsOn,
  loadProposals,
  toggleAutoPropose,
  setAutoEnableRisk,
  showProposal,
  acceptProposal,
  rejectProposal,
  disableAutoEnabled,
} = proposalsModel

const catalog = useSkillsCatalog(rpc, {
  proposals,
  autoEnabledSkills,
  proposalsSettings,
  loadProposals,
})

const {
  filterText,
  statusFilter,
  metaSkills,
  visibleLayerGroups,
  installedEmpty,
  emptyMessage,
  statTiles,
  disabledSkills = ref([]),
  setStatusFilter,
} = catalog

loadData = catalog.loadData

function reloadSummary(result: SkillReloadResult): string {
  return t('cronSkills.skillsView.reloadSummary', {
    added: result.added?.length || 0,
    removed: result.removed?.length || 0,
    modified: result.modified?.length || 0,
  })
}

async function manualReload() {
  if (reloading.value) return
  reloading.value = true
  try {
    await rpc.waitForConnection()
    const result = await rpc.call<SkillReloadResult>('skills.reload')
    // Always redraw from the catalog the Gateway is actually serving. On a
    // failed publish this is the prior last-known-good generation.
    const listed = await loadData()
    if (listed === false) {
      throw new Error(t('cronSkills.skillsView.reloadListFailed'))
    }

    if (!result.success) {
      const error = result.errors?.[0]?.message || t('cronSkills.skillsView.reloadUnknownError')
      pushToast(t('cronSkills.skillsView.reloadFailed', { error }), { tone: 'danger' })
    } else if (result.partial) {
      pushToast(t('cronSkills.skillsView.reloadPartial', {
        generation: result.generation,
        summary: reloadSummary(result),
        errors: result.errors?.length || 0,
      }), { tone: 'warn' })
    } else if (!result.changed) {
      pushToast(t('cronSkills.skillsView.reloadNoChanges', {
        generation: result.generation,
      }))
    } else {
      pushToast(t('cronSkills.skillsView.reloadSuccess', {
        generation: result.generation,
        summary: reloadSummary(result),
      }), { tone: 'ok' })
    }
  } catch (err) {
    pushToast(t('cronSkills.skillsView.reloadFailed', {
      error: (err as Error).message,
    }), { tone: 'danger' })
  } finally {
    reloading.value = false
  }
}

const registry = useSkillRegistry(rpc, loadData)
const {
  registryQuery,
  githubUrl,
  registryResults,
  registryLoading,
  installingId,
  installingDepsId,
  uninstallingName,
  searchRegistry,
  installGithub,
  installSkill,
  installDeps,
} = registry

const skillDetail = useSkillDetailController({ rpc, installDeps })
const {
  selectedSkill,
  selectedSkillLoading,
  selectedSkillError,
  installFeedback,
  openSkill,
  closeSkill,
  installCurrentDependencies,
} = skillDetail

// This view is kept-alive (route meta.keepAlive), so the data fetch is bound on
// activation rather than mount — onMounted/onUnmounted only fire on first mount /
// cache eviction, not when navigating away and back. onActivated also runs on
// first display, so it covers the initial load while also refreshing on revisit.
// There are no subscriptions or polls in this view's data stack, so teardown is a
// no-op, but it is kept idempotent and wired to both onDeactivated and onUnmounted
// to match the reference pattern and guard against future additions.
let unsubs: Array<() => void> = []

function teardownLive() {
  unsubs.forEach(unsub => unsub())
  unsubs = []
  closeDialog()
}

onActivated(() => {
  void loadData()
  void loadTools()
  void loadCoverage()
})

onDeactivated(teardownLive)
onUnmounted(teardownLive)

function scrollToProposals() {
  proposalsPanelRef.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
}

function selectStatusFromOverview(key: string) {
  skillsOverviewOpen.value = false
  selectStatusFilter(key)
}

async function showProposalsFromOverview() {
  skillsOverviewOpen.value = false
  await showProposalsFromStats()
}
function selectStatusFilter(key: string) {
  activeTab.value = 'skills'
  setStatusFilter(key)
}

async function showProposalsFromStats() {
  activeTab.value = 'proposals'
  await nextTick()
  scrollToProposals()
}

async function openSkillDialog(skill: Skill) {
  selectedProposal.value = null
  await openSkill(skill)
}

async function openProposalDialog(proposalId: string) {
  const proposal = await showProposal(proposalId)
  if (!proposal) return
  closeSkill()
  selectedProposal.value = proposal
}

function closeDialog() {
  closeSkill()
  selectedProposal.value = null
}

async function installDepsAndMaybeClose(name: string, installId: string) {
  await installCurrentDependencies(name, installId)
}

const togglingName = ref('')
const deletingName = ref('')

/** 启停：真实改名 + 持久索引；停用后技能从列表消失，进入「已停用」区。 */
async function toggleSkill(name: string, enabled: boolean) {
  if (togglingName.value) return
  togglingName.value = name
  try {
    const res = await rpc.call<{ ok?: boolean; message?: string }>('skills.setEnabled', { name, enabled })
    if (res?.ok === false) {
      pushToast(res.message || t('skills.manage.toggleFailed'), { tone: 'danger' })
      return
    }
    pushToast(t(enabled ? 'skills.manage.enabledNotice' : 'skills.manage.disabledNotice', { name }), { tone: 'ok' })
    await loadData()
    await loadCoverage()
    if (!enabled) closeDialog()
  } catch (e) {
    pushToast(t('skills.manage.toggleFailedDetail', { error: (e as Error).message }), { tone: 'danger' })
  } finally {
    togglingName.value = ''
  }
}

function toggleSkillFromDetail(name: string, enabled: boolean) {
  void toggleSkill(name, enabled)
}

/** 删除：优先用 skills.uninstall（覆盖 YAML 与 SKILL.md 目录），删除前确认。 */
async function deleteSkill(name: string) {
  if (deletingName.value) return
  const approved = await confirm({
    title: t('skills.manage.deleteTitle', { name }),
    body: t('skills.manage.deleteBody', { name }),
    primaryLabel: t('skills.manage.deleteConfirm'),
  })
  if (!approved) return
  deletingName.value = name
  try {
    const res = await rpc.call<{ success?: boolean; message?: string }>('skills.uninstall', { name })
    if (res?.success === false) {
      pushToast(res.message || t('skills.manage.deleteFailed'), { tone: 'danger' })
      return
    }
    pushToast(t('skills.manage.deletedNotice', { name }), { tone: 'ok' })
    await loadData()
    await loadCoverage()
    if (selectedSkill.value?.name === name) closeDialog()
  } catch (e) {
    pushToast(t('skills.manage.deleteFailedDetail', { error: (e as Error).message }), { tone: 'danger' })
  } finally {
    deletingName.value = ''
  }
}

async function uninstallSkillAndClose(name: string) {
  await deleteSkill(name)
}

// ── 编辑 YAML 技能 ──────────────────────────────────────────────────────
const editOpen = ref(false)
const editSaving = ref(false)
const editMessage = ref('')
const editOk = ref(true)
const editForm = reactive({ name: '', description: '', triggers: '', prompt_template: '' })

function openSkillEditor(skill: Skill) {
  editForm.name = skill.name
  editForm.description = skill.description || ''
  editForm.triggers = (skill.triggers || []).join(', ')
  editForm.prompt_template = skill.content || ''
  editMessage.value = ''
  editOk.value = true
  editOpen.value = true
}

async function saveSkillEdit() {
  if (editSaving.value) return
  if (!editForm.name || !editForm.description.trim() || !editForm.prompt_template.trim()) {
    editOk.value = false
    editMessage.value = t('plugins.skill.requiredFields')
    return
  }
  editSaving.value = true
  editMessage.value = ''
  try {
    const res = await rpc.call<{ success?: boolean; message?: string }>('skills.update', {
      name: editForm.name,
      description: editForm.description.trim(),
      prompt_template: editForm.prompt_template,
      triggers: editForm.triggers.split(',').map(s => s.trim()).filter(Boolean),
    })
    editOk.value = res?.success !== false
    editMessage.value = res?.message || (editOk.value ? t('skills.manage.saved') : t('skills.manage.saveFailed'))
    if (editOk.value) {
      editOpen.value = false
      closeDialog()
      await loadData()
      await loadCoverage()
    }
  } catch (e) {
    editOk.value = false
    editMessage.value = (e as Error).message
  } finally {
    editSaving.value = false
  }
}
</script>

<style>
/* Compact skills overview */
.sk-overview-modal {
  align-items: center;
  background: var(--scrim);
  display: flex;
  inset: 0;
  justify-content: center;
  padding: 24px;
  position: fixed;
  z-index: 1101; /* Layer: modal-dialog */
}

.sk-overview-modal__panel {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--elev-3);
  max-width: 960px;
  padding: 22px;
  width: 100%;
}

.sk-overview-modal__head {
  align-items: flex-start;
  display: flex;
  gap: 20px;
  justify-content: space-between;
  margin-bottom: 18px;
}

.sk-overview-modal__eyebrow {
  color: var(--accent);
  font-size: 10px;
  font-weight: 750;
  letter-spacing: .13em;
}

.sk-overview-modal__head h2 {
  font-size: 1.125rem;
  margin: 4px 0 0;
}

.sk-overview-modal__head p {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  margin: 5px 0 0;
}

.sk-overview-modal__actions {
  align-items: center;
  display: flex;
  gap: 6px;
}
.sk-overview-modal__close {
  padding: 6px;
}

@media (max-width: 700px) {
  .sk-overview-modal { align-items: flex-end; padding: 0; }
  .sk-overview-modal__panel { border-bottom-left-radius: 0; border-bottom-right-radius: 0; max-height: 88vh; overflow: auto; padding: 18px; }
}

/* Search */
.sk-search-wrap {
  position: relative;
  display: flex;
  align-items: center;
}
.sk-search-icon {
  position: absolute;
  left: 10px;
  color: var(--text-dim);
  pointer-events: none;
  display: inline-flex;
  align-items: center;
}
.sk-search-input {
  padding: 8px 12px 8px 34px;
  font-size: var(--fs-sm);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text);
  outline: none;
  min-width: 200px;
  transition: border-color var(--transition), box-shadow var(--transition);
}
/* base.css resets text inputs via input:not([type="radio"]):not([type="checkbox"])
   — specificity (0,2,1), which outranks the .sk-search-input class and drops the
   leading-icon clearance (and elevated fill), letting the search/download icon
   overlap the placeholder. Re-assert just those two properties at matching reach;
   the :not() mirror clears the base reset without touching the --lg width rule. */
.sk-search-input:not([type="radio"]):not([type="checkbox"]) {
  padding: 8px 12px 8px 34px;
  background: var(--bg-elevated);
}
.sk-search-input:focus {
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}
.sk-search-wrap--lg .sk-search-input {
  min-width: 320px;
}

/* Quick launch: desktop toolbox cards, one row on the installed tab. */
.sk-launch {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
.sk-launch__head {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--sp-2) var(--sp-3);
}
.sk-launch__title {
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
  line-height: 20px;
  margin: 0;
}
.sk-launch__desc {
  color: var(--text-dim);
  font-size: 12px;
  line-height: 20px;
  margin: 0;
}
.sk-launch__grid {
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: repeat(3, minmax(0, 1fr));
}
.sk-launch-card {
  align-items: center;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  color: var(--text);
  cursor: pointer;
  display: grid;
  font: inherit;
  gap: var(--sp-2);
  grid-template-columns: auto minmax(0, 1fr) auto;
  min-height: 64px;
  padding: var(--sp-3);
  text-align: left;
  transition: border-color var(--dur-fast) var(--ease-out), box-shadow var(--dur-fast) var(--ease-out);
}
.sk-launch-card:hover {
  border-color: var(--border-strong, var(--border));
  box-shadow: var(--elev-1);
}
.sk-launch-card:focus-visible {
  box-shadow: var(--focus-ring);
  outline: 0;
}
.sk-launch-card.is-off {
  opacity: 0.62;
}
.sk-launch-card__icon {
  align-items: center;
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  border-radius: var(--radius-md);
  color: var(--accent);
  display: inline-flex;
  height: 34px;
  justify-content: center;
  width: 34px;
}
.sk-launch-card.is-off .sk-launch-card__icon {
  color: var(--text-dim);
  background: var(--bg-elevated);
}
.sk-launch-card__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.sk-launch-card__body strong {
  font-size: var(--fs-sm);
  font-weight: 650;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sk-launch-card__body span {
  color: var(--text-muted);
  font-size: 11px;
  line-height: 18px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sk-launch-card__state {
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--text-dim);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
  padding: 2px 8px;
  white-space: nowrap;
}
.sk-launch-card:not(.is-off) .sk-launch-card__state {
  border-color: color-mix(in srgb, var(--ok) 45%, var(--border));
  color: var(--ok);
}
@media (max-width: 920px) {
  .sk-launch__grid { grid-template-columns: minmax(0, 1fr); }
}

/* Tools & capabilities: registered tool catalog + configured MCP servers. */
.sk-tools {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
.sk-tools__head {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: var(--sp-2) var(--sp-3);
}
.sk-tools__title {
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
  line-height: 20px;
  margin: 0;
}
.sk-tools__desc {
  color: var(--text-dim);
  font-size: 12px;
  line-height: 20px;
  margin: 0;
}
.sk-tools__mcp {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-1) var(--sp-2);
  padding: var(--sp-2) var(--sp-3);
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.sk-tools__mcp-label {
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
}
.sk-tools__mcp-chip {
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--text);
  font-family: var(--font-mono);
  font-size: 11px;
  padding: 2px 9px;
}
.sk-tools__mcp-count {
  color: var(--text-dim);
  margin-left: 5px;
}
.sk-tools__mcp-probe {
  background: transparent;
  border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
  border-radius: var(--radius-full);
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  font-size: 11px;
  padding: 2px 10px;
  transition: border-color var(--dur-fast) var(--ease-out);
}
.sk-tools__mcp-probe:hover {
  border-color: var(--accent);
}
.sk-tools__mcp-probe:focus-visible {
  box-shadow: var(--focus-ring);
  outline: 0;
}
.sk-tools__mcp-probe:disabled {
  cursor: default;
  opacity: 0.6;
}
.sk-tools__mcp-error {
  color: var(--warn);
  font-size: 11px;
  line-height: 18px;
  width: 100%;
}
.sk-tools__mcp-empty {
  color: var(--text-muted);
  font-size: 12px;
  line-height: 20px;
}
.sk-tools__grid {
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: repeat(3, minmax(0, 1fr));
}
.sk-tools-card {
  align-items: center;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: auto minmax(0, 1fr);
  min-height: 64px;
  padding: var(--sp-3);
}
.sk-tools-card__icon {
  align-items: center;
  background: color-mix(in srgb, var(--accent) 10%, transparent);
  border-radius: var(--radius-md);
  color: var(--accent);
  display: inline-flex;
  height: 34px;
  justify-content: center;
  width: 34px;
}
.sk-tools-card__body {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.sk-tools-card__body strong {
  font-size: var(--fs-sm);
  font-weight: 650;
}
.sk-tools-card__body span {
  color: var(--text-muted);
  font-size: 11px;
  line-height: 18px;
}
.sk-tools-card__names {
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: 10px;
  grid-column: 1 / -1;
  line-height: 16px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sk-tools-card--loading {
  justify-content: center;
}
.sk-tools-card__cred {
  align-items: center;
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 30%, var(--border));
  border-radius: var(--radius-full);
  color: var(--accent);
  cursor: pointer;
  display: inline-flex;
  font: inherit;
  font-size: 11px;
  font-weight: 650;
  gap: 5px;
  grid-column: 1 / -1;
  justify-content: center;
  justify-self: end;
  line-height: 18px;
  padding: 3px 12px;
  transition: border-color var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out);
}
.sk-tools-card__cred:hover {
  border-color: var(--accent);
  color: var(--accent);
}
.sk-tools-card__cred:focus-visible {
  box-shadow: var(--focus-ring);
  outline: 0;
}
.sk-tools-card--loading .sk-tools-card__body span {
  color: var(--text-dim);
}
@media (max-width: 920px) {
  .sk-tools__grid { grid-template-columns: minmax(0, 1fr); }
}

/* 工具启停（补丁层）：每行一个工具 + 一个开关 */
.sk-patch {
  border-top: 1px solid var(--border);
  margin-top: var(--sp-3);
  padding-top: var(--sp-3);
}
.sk-patch__head {
  align-items: baseline;
  display: flex;
  flex-wrap: wrap;
  gap: var(--sp-2);
  margin-bottom: var(--sp-2);
}
.sk-patch__title {
  font-size: var(--fs-sm);
  font-weight: 650;
}
.sk-patch__hint {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.sk-patch__error {
  color: var(--danger);
  font-size: var(--fs-xs);
  margin: 0 0 var(--sp-2);
}
.sk-patch__list {
  display: grid;
  gap: 2px;
  list-style: none;
  margin: 0;
  max-height: 260px;
  overflow-y: auto;
  padding: 0;
}
.sk-patch__row {
  align-items: center;
  border-radius: var(--radius-xs);
  display: grid;
  gap: var(--sp-2);
  grid-template-columns: minmax(96px, max-content) minmax(0, 1fr) auto;
  padding: 3px 6px;
}
.sk-patch__row.is-off {
  background: color-mix(in srgb, var(--warn) 8%, transparent);
}
.sk-patch__name {
  color: var(--text);
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sk-patch__desc {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sk-patch__toggle {
  background: transparent;
  border: 1px solid color-mix(in srgb, var(--ok) 50%, var(--border));
  border-radius: var(--radius-pill);
  color: var(--ok);
  cursor: pointer;
  font: inherit;
  font-size: var(--fs-xs);
  line-height: 18px;
  padding: 1px 10px;
  transition: border-color var(--dur-fast) var(--ease-out), color var(--dur-fast) var(--ease-out);
}
.sk-patch__toggle.is-off {
  border-color: color-mix(in srgb, var(--warn) 50%, var(--border));
  color: var(--warn);
}
.sk-patch__toggle:focus-visible {
  box-shadow: var(--focus-ring);
  outline: 0;
}
.sk-patch__toggle:disabled {
  cursor: default;
  opacity: 0.6;
}
.sk-patch__inactive {
  color: var(--warn);
  font-size: var(--fs-xs);
  margin: var(--sp-2) 0 0;
}

/* Tabs */
.sk-tabs {
  display: flex;
  gap: 0;
  border-bottom: 1px solid var(--border);
}
.sk-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 10px 18px;
  background: transparent;
  border: 0;
  border-bottom: 2px solid transparent;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-muted);
  cursor: pointer;
  transition: color var(--transition), border-color var(--transition);
}
.sk-tab.is-active {
  color: var(--accent);
  border-bottom-color: var(--accent);
}
.sk-tab:hover:not(.is-active) {
  color: var(--text);
}

/* Panels */
.sk-panel {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}

/* Groups */
.sk-group {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  overflow: hidden;
}
.sk-group--meta {
  border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
}
.sk-group--proposals {
  border-color: color-mix(in srgb, var(--warn) 30%, var(--border));
}
.sk-group--ap-settings {
  border-color: color-mix(in srgb, var(--accent) 20%, var(--border));
}
.sk-group__head {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  cursor: pointer;
  user-select: none;
  background: var(--bg-elevated);
  font-size: var(--fs-sm);
}
.sk-group__caret {
  color: var(--text-dim);
  font-size: 10px;
  transition: transform var(--dur-base) var(--ease-standard);
}
.sk-group[open] .sk-group__caret {
  transform: rotate(180deg);
}
.sk-group__label {
  font-weight: 600;
  color: var(--text);
}
.sk-group__count {
  font-size: var(--fs-xs);
  color: var(--text-dim);
  background: var(--bg);
  padding: 1px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
}
.sk-group__meta {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  margin-left: auto;
}

/* Grid */
.sk-grid {
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}
.sk-card__head {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.sk-card__dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.sk-card__dot.is-ready {
  background: var(--ok);
}
.sk-card__dot.is-needs {
  background: var(--warn-fill);
}
.sk-card__dot.is-unverified {
  background: var(--text-dim);
}
.sk-card__dot.is-provider-check {
  background: var(--text-dim);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--text-dim) 18%, transparent);
}
.sk-card__emoji {
  font-size: 14px;
  line-height: 1;
}
.sk-card__name {
  font-weight: 600;
  font-size: var(--fs-sm);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.sk-card__kind-badge {
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
  flex-shrink: 0;
}
.sk-card__desc {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  line-height: 1.4;
}
.sk-card__deps {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.sk-card__dep {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  font-family: var(--font-mono);
  font-size: 9px;
  padding: 1px 5px;
}
.sk-card__dep--missing {
  border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
  color: var(--warn);
}
.sk-card__dep--advisory {
  border-style: dashed;
  color: var(--text-muted);
}
.sk-card__provider-status {
  align-self: flex-start;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 1px 6px;
  border: 1px solid color-mix(in srgb, var(--text-dim) 40%, var(--border));
  border-radius: var(--radius-sm);
  color: var(--text-dim);
  background: var(--bg-elevated);
  font-size: 10px;
  font-weight: 600;
}
.sk-card__sub-row {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  margin-top: 2px;
}
.sk-card__sub-label {
  font-size: 10px;
  font-weight: 600;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.06em;
  margin-right: 2px;
}
.sk-card__sub-chip {
  font-size: 10px;
  padding: 1px 6px;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text-muted);
}
.sk-card__sub-chip--more {
  background: transparent;
  border-style: dashed;
}

/* Proposals list */
.sk-proposals-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}
.sk-proposal-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-3);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  flex-wrap: wrap;
}
.sk-proposal-row__head {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  min-width: 0;
}
.sk-proposal-row__id {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--text);
  background: var(--bg-elevated);
  padding: 2px 6px;
  border-radius: var(--radius-sm);
}
.sk-proposal-row__actions {
  display: flex;
  gap: 6px;
  flex-shrink: 0;
}
.sk-prop-chip {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: var(--radius-sm);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  color: var(--text-muted);
}
.sk-prop-chip--ok {
  border-color: color-mix(in srgb, var(--ok) 40%, var(--border));
  color: var(--ok);
}
.sk-prop-chip--warn {
  border-color: color-mix(in srgb, var(--warn) 40%, var(--border));
  color: var(--warn);
}
.sk-prop-chip--auto {
  border-style: dashed;
  color: var(--accent);
}
.sk-prop-hash {
  font-family: var(--font-mono);
  font-size: 10px;
  color: var(--text-dim);
}

/* Auto-propose settings */
.sk-ap-settings {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-3) var(--sp-4) var(--sp-4);
}
.sk-ap-toggle {
  display: flex;
  align-items: flex-start;
  gap: var(--sp-2);
  flex-wrap: wrap;
  cursor: pointer;
}
.sk-ap-toggle input[type="checkbox"] {
  margin-top: 2px;
  accent-color: var(--accent);
}
.sk-ap-toggle__label {
  font-weight: 600;
  font-size: var(--fs-sm);
  color: var(--text);
}
.sk-ap-toggle__hint {
  font-size: var(--fs-xs);
  color: var(--text-muted);
  width: 100%;
  margin-left: 44px;
}
.sk-ap-select {
  padding: 4px 8px;
  font-size: var(--fs-sm);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  outline: none;
}
.sk-ap-select:focus {
  border-color: var(--accent);
}

/* Registry */
.sk-registry {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.sk-registry__head,
.sk-github-install {
  display: flex;
  align-items: center;
  gap: var(--sp-2) 0;
  flex-wrap: wrap;
}
.sk-registry__results {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--sp-4);
  min-height: 120px;
}
.sk-registry__hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: var(--sp-5);
  color: var(--text-muted);
  text-align: center;
}
.sk-registry__hint-icon {
  color: var(--text-dim);
}
.sk-registry__loading {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: var(--sp-5);
  color: var(--text-muted);
}
.sk-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid var(--border);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: sk-spin 0.8s linear infinite;
}
.sk-registry__name {
  font-weight: 600;
}
.sk-registry__desc {
  color: var(--text-muted);
  font-size: var(--fs-xs);
  max-width: 300px;
}

/* Dialog */
.sk-dialog {
  position: fixed;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  color: var(--text);
  max-width: 640px;
  width: 90vw;
  max-height: 85vh;
  overflow: hidden;
  padding: 0;
  margin: 0;
}
.sk-dialog::backdrop {
  background: var(--scrim);
}
.sk-detail {
  display: flex;
  flex-direction: column;
  font-family: var(--font-sans);
  font-size: 13px;
  line-height: 20px;
  max-height: 85vh;
}
.sk-detail__header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--sp-3);
  padding: var(--sp-4);
  border-bottom: 1px solid var(--border);
}
.sk-detail__head-left {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  flex-wrap: wrap;
  flex: 1 1 auto;
  min-width: 0;
}
.sk-detail__emoji {
  font-size: 18px;
  line-height: 1;
}
.sk-detail__name {
  font-family: inherit;
  font-size: 16px;
  font-weight: 600;
  overflow-wrap: anywhere;
  line-height: 22px;
}
.sk-detail__chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  min-width: 0;
}
.sk-detail__body {
  padding: var(--sp-4);
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.sk-detail__desc {
  margin: 0;
  color: var(--text-muted);
  font-family: inherit;
  font-size: 13px;
  line-height: 22px;
}
.sk-detail__section {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
}
.sk-detail__section-title {
  font-family: inherit;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.04em;
  line-height: 20px;
  color: var(--text-dim);
}
.sk-detail__sub-list {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.sk-detail__missing {
  margin: 0;
  padding-left: var(--sp-4);
  font-family: inherit;
  font-size: 13px;
  line-height: 22px;
  color: var(--text-muted);
}
.sk-detail__missing li {
  margin-bottom: 6px;
}
.sk-detail__missing code {
  font-family: var(--font-mono);
  font-size: 13px;
  line-height: 22px;
}
.sk-detail__declared {
  margin-top: 0;
}
.sk-detail__dependency-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 6px;
}
.sk-detail__dependency-stat {
  align-items: center;
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 7px 4px;
  text-align: center;
}
.sk-detail__dependency-stat strong {
  color: var(--text);
  font-family: var(--font-mono);
  font-size: 13px;
  line-height: 20px;
}
.sk-detail__dependency-stat span {
  color: var(--text-dim);
  font-family: inherit;
  font-size: 12px;
  line-height: 18px;
}
.sk-detail__dependency-stat.is-missing {
  border-color: color-mix(in srgb, var(--warn) 45%, var(--border));
}
.sk-detail__dependency-stat.is-missing strong {
  color: var(--warn);
}
.sk-detail__advisory-note {
  color: var(--text-muted);
  font-family: inherit;
  font-size: 13px;
  line-height: 22px;
  margin: 0;
}
.sk-detail__install-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-2);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  font-family: inherit;
  font-size: 13px;
  line-height: 20px;
}
.sk-detail__install-row--toolchain {
  align-items: flex-start;
  flex-direction: column;
}
.sk-detail__toolchain-guidance {
  line-height: 1.45;
}
.sk-detail__link {
  color: var(--accent);
  text-decoration: none;
  font-family: inherit;
  font-size: 13px;
  line-height: 20px;
}
.sk-detail__link:hover {
  text-decoration: underline;
}
.sk-detail__content-state {
  padding: var(--sp-3);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text-muted);
  font-family: inherit;
  font-size: 13px;
  line-height: 20px;
}
.sk-detail__content-state--error {
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 35%, var(--border));
}
.sk-detail__content-state--warn {
  border-color: color-mix(in srgb, var(--warn) 35%, var(--border));
  color: var(--warn);
}
.sk-detail__foot {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: var(--sp-2);
  padding: var(--sp-3) var(--sp-4);
  border-top: 1px solid var(--border);
  flex-wrap: wrap;
}
.sk-detail__path {
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 18px;
}

.sk-iconbtn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  background: transparent;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  color: var(--text-muted);
  cursor: pointer;
  flex: 0 0 32px;
  transition: color var(--transition), border-color var(--transition);
}
.sk-iconbtn:hover {
  color: var(--text);
  border-color: var(--border-focus);
}

/* Chips */
.sk-chip {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  font-family: inherit;
  font-size: 12px;
  font-weight: 600;
  line-height: 18px;
  border: 1px solid var(--border);
  background: var(--bg-elevated);
  color: var(--text-muted);
}
.sk-chip--ok {
  border-color: color-mix(in srgb, var(--ok) 40%, var(--border));
  color: var(--ok);
}
.sk-chip--warn {
  border-color: color-mix(in srgb, var(--warn) 40%, var(--border));
  color: var(--warn);
}
.sk-chip--unverified {
  border-color: color-mix(in srgb, var(--text-dim) 40%, var(--border));
  color: var(--text-dim);
}
.sk-chip--sub {
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  color: var(--accent);
}
.sk-chip--trigger {
  font-family: var(--font-mono);
  font-size: 12px;
  background: var(--bg);
}

/* Audit grid */
.sk-audit-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px 16px;
  font-size: var(--fs-sm);
}
.sk-audit-grid__wide {
  grid-column: 1 / -1;
}
.sk-audit-grid span {
  color: var(--text-dim);
  font-size: var(--fs-xs);
}
.sk-audit-grid strong {
  color: var(--text);
  font-weight: 600;
}
.sk-audit-grid code {
  font-family: var(--font-mono);
  font-size: 11px;
  background: var(--bg-elevated);
  padding: 1px 4px;
  border-radius: var(--radius-sm);
  margin-right: 4px;
}
.sk-audit-empty {
  font-size: var(--fs-sm);
  color: var(--text-muted);
  padding: var(--sp-2);
}

/* Preformatted */
.sk-detail__pre {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  padding: var(--sp-3);
  font-family: var(--font-mono);
  font-size: 12px;
  line-height: 20px;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-word;
  margin: 0;
  color: var(--text-muted);
}

/* Empty state 的样式已收口到 `components/EmptyState.vue`；
   过滤词强调仍由 slot 内的 `.empty-panel__hint strong` 承载，故保留该规则。 */
.empty-panel__hint strong {
  color: var(--text);
}

/* Utility */
.sk-dim {
  color: var(--text-dim);
}
.sk-mono {
  font-family: var(--font-mono);
}

/* Animations */
@keyframes sk-fade-up {
  from { opacity: 0; transform: translateY(6px); }
  to { opacity: 1; transform: translateY(0); }
}
@keyframes sk-spin {
  to { transform: rotate(360deg); }
}

/* Responsive */
@media (max-width: 720px) {
  .sk-dialog {
    width: calc(100vw - 24px);
    max-height: calc(100dvh - 24px);
  }
  .sk-detail {
    max-height: calc(100dvh - 24px);
  }
  .sk-detail__header {
    align-items: flex-start;
    padding: var(--sp-3);
  }
  .sk-detail__chips {
    flex-basis: 100%;
  }
  .sk-detail__body {
    padding: var(--sp-3);
  }
  .sk-stage__header {
    flex-direction: column;
    align-items: stretch;
  }
  .sk-stage__actions {
    width: 100%;
  }
  .sk-search-input,
  .sk-search-wrap--lg .sk-search-input {
    min-width: 0;
    width: 100%;
  }
  .sk-grid {
    grid-template-columns: 1fr;
  }
  .sk-detail__dependency-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .sk-proposal-row {
    flex-direction: column;
    align-items: flex-start;
  }
}

/* Skill catalog groups read as open sections, not cards inside cards. */
.sk-group--skills {
  background: transparent;
  border: 0;
  border-radius: 0;
  overflow: visible;
}
.sk-group--skills.sk-group--meta {
  border: 0;
}
.sk-group--skills > .sk-group__head {
  border-bottom: 0;
  padding: 14px 2px 12px;
}
.sk-group--skills > .sk-grid {
  padding: 16px 0 18px;
}

/* Skills page typography and alignment contract. */
.sk-stage {
  font-family: var(--font-sans);
  font-size: 13px;
  line-height: 1.5;
}
.sk-stage button,
.sk-stage input,
.sk-stage select {
  font-family: inherit;
}
.sk-stage__header,
.sk-stage__actions,
.sk-search-wrap,
.sk-tab,
.sk-group__head {
  align-items: center;
}
.sk-stage__title {
  font-family: var(--font-display);
  font-size: 24px;
  font-weight: 700;
  line-height: 1.25;
}
.sk-stage__subtitle {
  font-size: 13px;
  line-height: 1.6;
}
.sk-stage__actions .btn,
.sk-tab,
.sk-search-input {
  font-size: 13px;
  line-height: 20px;
}
.sk-stage__actions .btn,
.sk-tab {
  align-items: center;
  display: inline-flex;
  gap: 7px;
}
.sk-stage__actions .btn > .icon,
.sk-tab > .icon,
.sk-search-icon > .icon {
  align-items: center;
  display: inline-flex;
  height: 18px;
  justify-content: center;
  line-height: 0;
  width: 18px;
}
.sk-stage__actions .btn > .icon svg,
.sk-tab > .icon svg,
.sk-search-icon > .icon svg {
  display: block;
}
.sk-search-input {
  height: 38px;
}
.sk-search-icon {
  height: 18px;
  justify-content: center;
  line-height: 0;
  width: 18px;
}
.sk-group--skills > .sk-group__head {
  min-height: 52px;
}
.sk-group__label {
  font-size: 14px;
  font-weight: 650;
  line-height: 20px;
}
.sk-group__count {
  align-items: center;
  display: inline-flex;
  font-size: 11px;
  height: 22px;
  justify-content: center;
  line-height: 1;
  min-width: 24px;
  padding: 0 7px;
}
.sk-group__meta {
  font-size: 12px;
  line-height: 20px;
}

/* Shared typography contract for every skill-card surface. */
.sk-card,
.sk-tile,
.sk-stat,
.sk-proposal-row,
.sk-registry__results {
  font-family: var(--font-sans);
}
.sk-card__name,
.sk-registry__name {
  font-size: 13px;
  font-weight: 650;
  line-height: 20px;
}
.sk-card__desc,
.sk-registry__desc {
  font-size: 13px;
  line-height: 22px;
}
.sk-card__desc {
  min-height: 44px;
}
.sk-card__dep,
.sk-card__sub-label,
.sk-card__sub-chip,
.sk-prop-chip,
.sk-prop-hash {
  font-size: 11px;
  line-height: 18px;
}
.sk-proposal-row__id {
  font-size: 12px;
  line-height: 18px;
}
.sk-stat__label,
.sk-stat__hint {
  font-family: inherit;
  font-size: 12px;
  line-height: 18px;
}
.sk-detail h3 {
  font-family: inherit;
  font-size: 16px;
  line-height: 22px;
  margin: 0;
}
.sk-detail h4 {
  color: var(--text-dim);
  font-family: inherit;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: .04em;
  line-height: 20px;
  margin: 0;
}
/* Coverage summary (skills.coverage) */
.sk-coverage {
  align-items: center;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: var(--sp-3);
  padding: var(--sp-2) var(--sp-3);
}
.sk-coverage__label {
  color: var(--text-dim);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: .05em;
  text-transform: uppercase;
}
.sk-coverage__pill {
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  border-radius: var(--radius-full);
  color: var(--text-muted);
  font-size: 11px;
  padding: 2px 9px;
}
.sk-coverage__pill--warn {
  border-color: color-mix(in srgb, var(--warn) 40%, var(--border));
  color: var(--warn);
}
.sk-coverage__pill--accent {
  border-color: color-mix(in srgb, var(--accent) 30%, var(--border));
  color: var(--accent);
}
.sk-coverage__hint { color: var(--text-dim); font-size: 11px; }
.sk-coverage__hint--err { color: var(--danger); }

/* Create skill form (moved from PluginsView) */
.sk-create {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--sp-3) var(--sp-4);
}
.sk-create__title { font-size: 14px; font-weight: 650; margin: 0 0 var(--sp-3); }
.sk-create__form { display: grid; gap: var(--sp-2) var(--sp-3); grid-template-columns: repeat(2, minmax(0, 1fr)); }
.sk-create__field { display: flex; flex-direction: column; gap: 4px; color: var(--text-dim); font-size: 11px; }
.sk-create__field--full { grid-column: 1 / -1; }
.sk-create__input,
.sk-create__textarea {
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-family: inherit;
  font-size: 12px;
  outline: none;
  padding: 7px 10px;
}
.sk-create__textarea { font-family: var(--font-mono); resize: vertical; }
.sk-create__input:focus,
.sk-create__textarea:focus { border-color: var(--accent); }
.sk-create__actions { display: flex; gap: 8px; grid-column: 1 / -1; justify-content: flex-end; }
.sk-create__result {
  background: color-mix(in srgb, var(--ok) 10%, var(--card));
  border-radius: var(--radius-sm);
  color: var(--ok);
  font-size: 12px;
  grid-column: 1 / -1;
  padding: 8px 12px;
}
.sk-create__result.is-error {
  background: color-mix(in srgb, var(--danger) 10%, var(--card));
  color: var(--danger);
}
@media (max-width: 720px) {
  .sk-create__form { grid-template-columns: minmax(0, 1fr); }
}

/* 已停用技能 */
.sk-disabled {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  margin-top: var(--sp-3);
  padding: var(--sp-2) var(--sp-3);
}
.sk-disabled__head { align-items: center; display: flex; flex-wrap: wrap; gap: 8px; }
.sk-disabled__list { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; }
.sk-disabled__row {
  align-items: center;
  border-bottom: 1px solid var(--border);
  display: grid;
  gap: 8px;
  grid-template-columns: minmax(140px, max-content) minmax(0, 1fr) auto auto;
  padding: 5px 0;
}
.sk-disabled__row:last-child { border-bottom: 0; }
.sk-disabled__name { font-family: var(--font-mono); font-size: 12px; }
.sk-disabled__path { color: var(--text-dim); font-family: var(--font-mono); font-size: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sk-disabled__danger { color: var(--danger); }

/* 技能编辑弹窗 */
.sk-edit-modal {
  align-items: center;
  background: var(--scrim);
  display: flex;
  inset: 0;
  justify-content: center;
  padding: 24px;
  position: fixed;
  z-index: 1101;
}
.sk-edit-modal__panel {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  box-shadow: var(--elev-3);
  display: flex;
  flex-direction: column;
  max-height: 88vh;
  max-width: 720px;
  width: 100%;
}
.sk-edit-modal__head {
  align-items: flex-start;
  border-bottom: 1px solid var(--border);
  display: flex;
  gap: 12px;
  justify-content: space-between;
  padding: 18px 20px;
}
.sk-edit-modal__head h2 { font-size: 16px; margin: 4px 0 0; }
.sk-edit-modal__body {
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  overflow: auto;
  padding: 16px 20px;
}
.sk-edit-modal__foot {
  border-top: 1px solid var(--border);
  display: flex;
  gap: 8px;
  justify-content: flex-end;
  padding: 12px 20px;
}
.sk-detail__foot-actions {
  align-items: center;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  justify-content: flex-end;
}
@media (max-width: 720px) {
  .sk-edit-modal { align-items: flex-end; padding: 0; }
  .sk-edit-modal__panel { border-bottom-left-radius: 0; border-bottom-right-radius: 0; max-height: 92vh; }
  .sk-disabled__row { grid-template-columns: minmax(0, 1fr) auto auto; }
  .sk-disabled__path { display: none; }
}
</style>
