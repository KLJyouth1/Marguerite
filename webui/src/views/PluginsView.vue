<template>
  <div class="control-stage plugins-stage">
    <header class="control-stage__header">
      <div class="control-stage__title-block">
        <h1 class="control-stage__title">{{ t('plugins.title') }}</h1>
        <p class="control-stage__subtitle">{{ t('plugins.subtitle') }}</p>
      </div>
      <div class="control-stage__actions">
        <div v-if="activeTab === 'assembly'" class="plugins-search">
          <Icon name="search" :size="16" />
          <input v-model="search" type="search" :placeholder="t('plugins.searchPlaceholder')" />
        </div>
        <button class="btn btn--ghost" @click="loadAll">
          <Icon name="refresh" :size="16" /><span>{{ t('plugins.refresh') }}</span>
        </button>
        <button class="btn btn--ghost" @click="dockInWorkbench">
          <Icon name="panel-right-open" :size="16" /><span>{{ t('plugins.openInWorkbench') }}</span>
        </button>
      </div>
    </header>

    <div class="hub-tabs" role="tablist" :aria-label="t('plugins.hub.tabLabel')">
      <button
        v-for="tab in hubTabs"
        :key="tab.id"
        class="hub-tab"
        :class="{ 'is-active': activeTab === tab.id }"
        type="button"
        role="tab"
        :aria-selected="activeTab === tab.id"
        @click="activeTab = tab.id"
      >{{ t(tab.labelKey) }}</button>
    </div>

    <section v-if="error" class="plugins-error">{{ error }}</section>
    <div v-if="manageNotice" class="market__note" data-testid="plugins-manage-notice">{{ manageNotice }}</div>

    <!-- 插件管理（独立页）：列表 / 搜索 / 筛选 / 详情 / 新建 / 编辑 / 删除 / 启停 / 重载 / MCP。
         数据源是 plugins.coverage（注册表 + 清单 + 适配映射），删除与详情走 plugins.delete /
         plugins.detail；新建与更新复用 market.import（同一套清单校验 + 原子落盘）。 -->
    <section v-if="activeTab === 'manage'" class="market manage" data-testid="plugins-manage">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.manage.title') }}</div>
        <div class="manage__head-actions">
          <button class="btn btn--sm btn--ghost" :disabled="covBusy" @click="loadCoverage">{{ t('plugins.manage.refresh') }}</button>
          <button class="btn btn--sm btn--primary" @click="openInstall">{{ t('plugins.manage.create') }}</button>
        </div>
      </div>

      <div v-if="cov" class="manage__stats">
        <span class="manage__stat"><b>{{ cov.registry.entries }}</b>{{ t('plugins.manage.statTotal') }}</span>
        <span class="manage__stat manage__stat--ok"><b>{{ cov.registry.enabled }}</b>{{ t('plugins.manage.statEnabled') }}</span>
        <span class="manage__stat manage__stat--warn"><b>{{ cov.registry.disabled }}</b>{{ t('plugins.manage.statDisabled') }}</span>
        <span class="manage__stat"><b>{{ adaptationStats.native }}</b>{{ t('plugins.manage.statNative') }}</span>
        <span class="manage__stat manage__stat--warn"><b>{{ adaptationStats.partial }}</b>{{ t('plugins.manage.statPartial') }}</span>
        <span class="manage__stat"><b>{{ adaptationStats.declarative }}</b>{{ t('plugins.manage.statDeclarative') }}</span>
      </div>

      <div class="manage__toolbar">
        <label class="plugins-search manage__search">
          <Icon name="search" :size="15" />
          <input v-model="pluginSearch" type="search" :placeholder="t('plugins.manage.searchPlaceholder')" data-testid="plugins-manage-search" />
        </label>
        <div class="manage__chips" role="group" :aria-label="t('plugins.manage.filterLabel')">
          <button
            v-for="f in pluginFilters"
            :key="f.id"
            type="button"
            class="manage__chip"
            :class="{ 'is-active': pluginFilter === f.id }"
            @click="pluginFilter = f.id"
          >{{ t(f.labelKey) }}</button>
        </div>
      </div>

      <div v-if="covError" class="market__note market__note--err">{{ covError }}</div>
      <div v-if="cov && cov.lastError" class="market__note market__note--err">{{ cov.lastError }}</div>

      <table v-if="filteredPlugins.length" class="market__table manage__table">
        <thead>
          <tr>
            <th>{{ t('plugins.manage.colPlugin') }}</th>
            <th>{{ t('plugins.manage.colVersion') }}</th>
            <th>{{ t('plugins.manage.colState') }}</th>
            <th>{{ t('plugins.manage.colAdaptation') }}</th>
            <th>{{ t('plugins.manage.colMcp') }}</th>
            <th>{{ t('plugins.manage.colAction') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="p in filteredPlugins" :key="p.id" :class="{ 'manage__row--off': !p.enabled }">
            <td>
              <b class="manage__name">{{ p.id }}</b>
              <p v-if="p.description" class="manage__desc">{{ p.description }}</p>
              <span v-if="p.declarative" class="market__kind">{{ t('plugins.toggle.declarative') }}</span>
            </td>
            <td>{{ p.version || '—' }}</td>
            <td>
              <span v-if="p.redline" class="market__mode" style="border-color: var(--danger); color: var(--danger)">{{ t('plugins.toggle.redline') }}</span>
              <span v-else-if="p.enabled" class="market__mode" style="border-color: var(--ok); color: var(--ok)">{{ t('plugins.toggle.enabled') }}</span>
              <span v-else class="market__mode">{{ t('plugins.toggle.disabled') }}</span>
            </td>
            <td>
              <span v-if="p.adaptation" class="manage__adapt" :class="'manage__adapt--' + p.adaptation.status" :title="p.adaptation.note">
                {{ adaptationLabel(p.adaptation.status) }}
              </span>
              <span v-else class="manage__adapt manage__adapt--none">{{ t('plugins.manage.adaptationNone') }}</span>
            </td>
            <td>
              <template v-if="p.mcp">
                <span class="market__mode" :style="p.mcp.running ? 'border-color: var(--ok); color: var(--ok)' : ''">
                  {{ p.mcp.running ? t('plugins.toggle.mcpRunning') : t('plugins.toggle.mcpStopped') }}
                </span>
                <button class="btn btn--sm" :disabled="covBusy" @click="p.mcp.running ? stopMcp(p.id) : startMcp(p.id)">
                  {{ p.mcp.running ? t('plugins.toggle.mcpStop') : t('plugins.toggle.mcpStart') }}
                </button>
              </template>
              <span v-else class="market__kind">{{ t('plugins.toggle.mcpNone') }}</span>
            </td>
            <td class="manage__actions">
              <button class="btn btn--sm btn--ghost" @click="openDetail(p)">{{ t('plugins.manage.detail') }}</button>
              <button class="btn btn--sm" :disabled="covBusy || p.redline" @click="togglePlugin(p.id, !p.enabled)">
                {{ p.enabled ? t('plugins.toggle.disable') : t('plugins.toggle.enable') }}
              </button>
              <button class="btn btn--sm btn--ghost" :disabled="covBusy" @click="reloadPlugin(p.id)">{{ t('plugins.toggle.reload') }}</button>
              <button class="btn btn--sm btn--ghost manage__danger" :disabled="covBusy" @click="deletePlugin(p)">{{ t('plugins.manage.delete') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-else-if="cov" class="market__note">{{ t('plugins.manage.empty') }}</div>
      <div v-if="cov" class="market__note">{{ t('plugins.manage.boundary') }}</div>
    </section>

    <section v-if="activeTab === 'assembly'" class="hub-panel">
      <section class="market">
        <div class="market__head">
          <div class="market__title">{{ t('plugins.hub.capabilitiesTitle') }}</div>
        </div>
        <div class="plugins-grid">
          <div v-for="p in filtered" :key="p.name" class="plugin-card">
            <div class="plugin-card__header">
              <span class="plugin-card__name">{{ p.name }}</span>
              <span class="plugin-card__maturity" :class="maturityClass(p.maturity)">{{ p.maturity }}</span>
            </div>
            <p class="plugin-card__desc">{{ p.description }}</p>
            <div class="plugin-card__meta">
              <span class="plugin-card__group">{{ p.group }}</span>
              <span class="plugin-card__tier">{{ p.tier }}</span>
            </div>
          </div>
        </div>
        <div v-if="!filtered.length && !error" class="plugins-empty">{{ t('plugins.empty') }}</div>
      </section>

    <!-- 插件装配状态（P0 S4b：boot 装配 patch → PluginSlot，经 plugins.status 可见） -->
    <section class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.assembly.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="asmBusy" @click="loadAssembly">{{ t('plugins.assembly.refresh') }}</button>
      </div>
      <div v-if="asmError" class="market__note market__note--err">{{ asmError }}</div>
      <div v-if="asm.summary" class="market__note">
        {{ t('plugins.assembly.summary', {
          assembled: asm.summary.assembled_count,
          installed: asm.summary.manifests_total,
          patches: asm.summary.patches_total,
          applied: asm.summary.applied,
          skipped: asm.summary.skipped,
          errors: asm.summary.errors,
        }) }}
      </div>
      <table v-if="asm.manifests && asm.manifests.length" class="market__table">
        <thead><tr><th>{{ t('plugins.assembly.columnManifest') }}</th><th>{{ t('plugins.assembly.columnMode') }}</th><th>{{ t('plugins.assembly.columnStatus') }}</th><th>{{ t('plugins.hub.columnEnabled') }}</th><th>{{ t('plugins.assembly.columnSlot') }}</th></tr></thead>
        <tbody>
          <tr v-for="m in asm.manifests" :key="m.name">
            <td><b>{{ m.name }}</b></td>
            <td>{{ m.mode === 'declarative-only' ? t('plugins.assembly.modeDeclarative') : t('plugins.assembly.modeNative') }}</td>
            <td>
              <span v-if="m.assembled" class="market__mode" style="border-color: var(--ok); color: var(--ok)">{{ t('plugins.assembly.assembled') }}</span>
              <span v-else class="market__mode">{{ t('plugins.assembly.notAssembled') }}</span>
            </td>
            <td>
              <span v-if="m.enabled === false" class="market__kind">{{ t('plugins.hub.enabledOff') }}</span>
              <span v-else class="market__kind">{{ t('plugins.hub.enabledOn') }}</span>
            </td>
            <td>{{ (m.slots || []).join(', ') }}</td>
          </tr>
        </tbody>
      </table>
      <div v-if="asm.manifests && asm.manifests.length" class="market__note">{{ t('plugins.hub.assemblyStale') }}</div>
      <div v-if="asm.report" class="market__catalog">
        <div class="market__sub">{{ t('plugins.assembly.patchReport') }}</div>
        <div class="market__row"><span class="market__kind">applied</span><span class="market__desc">{{ (asm.report.applied || []).join(' · ') || '—' }}</span></div>
        <div class="market__row"><span class="market__kind">skipped</span><span class="market__desc">{{ (asm.report.skipped || []).map(x => x.join(' · ')).join(' · ') || '—' }}</span></div>
        <div class="market__row"><span class="market__kind">errors</span><span class="market__desc">{{ (asm.report.errors || []).map(x => x.join(' · ')).join(' · ') || '—' }}</span></div>
      </div>
    </section>
    </section>

    <!-- 插件热启停与统一覆盖率（plugins.coverage / plugins.setEnabled / plugins.reload）。
         诚实边界：热启停真实改写持久注册表并重跑 DI 拓扑；当前构建未加载原生 DLL/WASM。 -->
    <section v-show="activeTab === 'toggle'" class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.toggle.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="covBusy" @click="loadCoverage">{{ t('plugins.toggle.refresh') }}</button>
      </div>
      <div v-if="covError" class="market__note market__note--err">{{ covError }}</div>
      <div v-if="toggleNotice" class="market__note">{{ toggleNotice }}</div>
      <div v-if="cov" class="market__note">
        {{ t('plugins.toggle.summary', {
          entries: cov.registry.entries,
          enabled: cov.registry.enabled,
          disabled: cov.registry.disabled,
          manifests: cov.manifests.total,
          patches: cov.patches.total === null ? '—' : cov.patches.total,
          suspects: cov.redlines.contextLeakSuspects.length,
          gaps: cov.gaps.manifestOnly.length + cov.gaps.registryOnly.length,
        }) }}
      </div>
      <div v-if="cov && cov.lastError" class="market__note market__note--err">{{ cov.lastError }}</div>
      <table v-if="cov && cov.plugins.length" class="market__table">
        <thead>
          <tr>
            <th>{{ t('plugins.toggle.columnPlugin') }}</th>
            <th>{{ t('plugins.toggle.columnSource') }}</th>
            <th>{{ t('plugins.toggle.columnState') }}</th>
            <th>{{ t('plugins.toggle.columnMcp') }}</th>
            <th>{{ t('plugins.toggle.columnAction') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="p in cov.plugins" :key="p.id">
            <td>
              <b>{{ p.id }}</b>
              <span v-if="p.version" class="market__kind">{{ p.version }}</span>
              <span v-if="p.declarative" class="market__kind">{{ t('plugins.toggle.declarative') }}</span>
            </td>
            <td>{{ p.sources.join(' + ') }}</td>
            <td>
              <span v-if="p.redline" class="market__mode" style="border-color: var(--danger); color: var(--danger)">{{ t('plugins.toggle.redline') }}</span>
              <span v-else-if="p.enabled" class="market__mode" style="border-color: var(--ok); color: var(--ok)">{{ t('plugins.toggle.enabled') }}</span>
              <span v-else class="market__mode">{{ t('plugins.toggle.disabled') }}</span>
            </td>
            <td>
              <template v-if="p.mcp">
                <span
                  class="market__mode"
                  :style="p.mcp.running ? 'border-color: var(--ok); color: var(--ok)' : ''"
                >{{ p.mcp.running ? t('plugins.toggle.mcpRunning') : t('plugins.toggle.mcpStopped') }}</span>
                <span class="market__kind">{{ t('plugins.toggle.mcpTools', { n: p.mcp.tools.length }) }}</span>
                <button
                  class="btn btn--sm"
                  :disabled="covBusy"
                  @click="p.mcp.running ? stopMcp(p.id) : startMcp(p.id)"
                >{{ p.mcp.running ? t('plugins.toggle.mcpStop') : t('plugins.toggle.mcpStart') }}</button>
              </template>
              <span v-else class="market__kind">{{ t('plugins.toggle.mcpNone') }}</span>
            </td>
            <td>
              <button class="btn btn--sm" :disabled="covBusy || p.redline" @click="togglePlugin(p.id, !p.enabled)">
                {{ p.enabled ? t('plugins.toggle.disable') : t('plugins.toggle.enable') }}
              </button>
              <button class="btn btn--sm btn--ghost" :disabled="covBusy" @click="reloadPlugin(p.id)">{{ t('plugins.toggle.reload') }}</button>
            </td>
          </tr>
        </tbody>
      </table>
      <div v-if="cov && !cov.plugins.length" class="market__note">{{ t('plugins.toggle.empty') }}</div>
      <div v-if="cov" class="market__note">{{ t('plugins.toggle.boundary') }}</div>
    </section>

    <!-- DSH 插件市场：安全审计 / 导入 / 导出 / 批量审计 -->
    <section v-show="activeTab === 'market'" class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.market.title') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="runAudit">{{ t('plugins.market.auditAll') }}</button>
        <button class="btn btn--sm btn--primary" :disabled="busy || !marketItems.length" @click="batchInstall">{{ t('plugins.market.installAll') }}</button>
      </div>
      <div v-if="marketError" class="market__note market__note--err">{{ marketError }}</div>

      <!-- 远程插件源（DSH 开源生态）：① 清单直链安装 ② 远程索引检索。
           本地清单只能装"已经躺在磁盘上"的插件；这两条把生态里的仓库直接接进来。 -->
      <div class="market__remote">
        <div class="plugins-search">
          <input
            v-model="remoteUrl"
            type="url"
            placeholder="清单直链（https://…/xxx.plugin.json）"
            aria-label="远程插件清单直链"
            data-testid="plugin-remote-url"
            @keydown.enter="installFromUrl"
          >
        </div>
        <button class="btn btn--sm btn--primary" :disabled="busy || !remoteUrl.trim()" @click="installFromUrl">
          从链接安装
        </button>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="searchRemote">
          远程搜索
        </button>
      </div>
      <div v-if="remoteStatus" class="market__note" data-testid="plugin-remote-status">{{ remoteStatus }}</div>
      <div v-if="remoteItems.length" class="market__catalog">
        <div v-for="r in remoteItems" :key="r.name" class="market__row">
          <code>{{ r.name }}</code>
          <span class="market__kind">{{ r.version || 'remote' }}</span>
          <span class="market__desc">{{ r.description }}</span>
          <button
            class="btn btn--sm btn--primary"
            :disabled="busy || !r.manifest_url"
            @click="installRemote(r)"
          >安装</button>
        </div>
      </div>
      <div v-if="auditItems.length" class="market__audit">
        <table class="market__table">
          <thead><tr><th>{{ t('plugins.market.columnPlugin') }}</th><th>{{ t('plugins.market.columnVersion') }}</th><th>{{ t('plugins.market.columnVerdict') }}</th><th>{{ t('plugins.market.columnCapabilities') }}</th><th>{{ t('plugins.market.columnFindings') }}</th></tr></thead>
          <tbody>
            <tr v-for="a in auditItems" :key="a.name">
              <td><b>{{ a.name }}</b></td>
              <td>{{ a.version || "—" }}</td>
              <td><span class="market__verdict" :data-v="a.verdict">{{ verdictLabel(a.verdict) }}</span></td>
              <td>{{ a.capabilities }} / {{ a.hooks }}</td>
              <td><span v-for="(s, i) in [...a.suspects, ...a.risks]" :key="i" class="market__finding">{{ s }}</span><span v-if="!a.suspects.length && !a.risks.length" class="market__finding market__finding--ok">{{ t('plugins.market.noFindings') }}</span></td>
            </tr>
          </tbody>
        </table>
      </div>
      <!-- 过滤标签页：全部 / 插件 / 技能 / Harness 生态 -->
      <div class="market__tabs">
        <button
          v-for="tab in marketTabs"
          :key="tab.id"
          class="market__tab"
          :class="{ 'is-active': marketTab === tab.id }"
          @click="marketTab = tab.id"
        >
          {{ tab.label }}
          <span class="market__tab-count">{{ tabCount(tab.id) }}</span>
        </button>
      </div>
      <div class="market__catalog">
        <div class="market__sub">{{ t('plugins.market.catalogTitle', { count: pluginSkillItems.length }) }}</div>
        <div v-for="m in pluginSkillItems" :key="m.kind + m.name" class="market__row">
          <code>{{ m.name }}</code>
          <span class="market__kind">{{ m.kind === 'plugin' ? t('plugins.market.kindPlugin') : t('plugins.market.kindSkill') }}</span>
          <!-- Makes "installed but inert" visible: a plugin manifest with no
               `entry` is declared, not assembled. Previously this was silent. -->
          <span
            v-if="m.kind === 'plugin' && m.mode === 'declarative-only'"
            class="market__mode"
            :title="t('plugins.market.declarativeOnlyHint')"
          >{{ t('plugins.market.declarativeOnly') }}</span>
          <span class="market__desc">{{ m.description }}</span>
          <button class="btn btn--sm btn--primary" :disabled="busy || m.kind !== 'plugin'" @click="installManifest(m.name)">{{ t('plugins.market.install') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy || m.kind !== 'plugin'" @click="exportManifest(m.name)">{{ t('plugins.market.export') }}</button>
        </div>
      </div>

      <!-- DeepSeek Harness 生态：上游仓库蒸馏目录（market.harness） -->
      <div v-if="marketTab === 'harness' || marketTab === 'all'" class="market__harness">
        <div class="market__sub">
          {{ t('plugins.harness.catalogTitle', { count: harnessItems.length }) }}
        </div>
        <div class="market__harness-grid">
          <div v-for="h in harnessItems" :key="h.name" class="harness-card" :class="{ 'harness-card--aligned': h.deepseek_aligned }">
            <div class="harness-card__head">
              <code class="harness-card__name">{{ h.name }}</code>
              <span class="harness-card__eco" :class="'eco--' + h.ecosystem">{{ ecoLabel(h.ecosystem) }}</span>
            </div>
            <p class="harness-card__desc">{{ h.description }}</p>
            <div class="harness-card__tags">
              <span v-for="c in h.capabilities" :key="c" class="harness-card__tag">{{ c }}</span>
            </div>
            <div class="harness-card__distill">
              <span class="harness-card__distill-label">{{ t('plugins.harness.distillTarget') }}</span>
              <code>{{ h.distilled_via }}</code>
            </div>
            <div v-if="h.deepseek_aligned" class="harness-card__aligned">{{ t('plugins.harness.aligned') }}</div>
          </div>
        </div>
      </div>
      <div v-if="batchResult" class="market__note">{{ batchResult }}</div>
    </section>

    <!-- 下载源与清单导入（market.sources / market.import）：进入页面即加载，不再「首次进入永远空」 -->
    <section v-show="activeTab === 'sources'" class="market">
      <div class="market__head">
        <div class="market__title">{{ t('plugins.hub.sourcesTitle') }}</div>
        <button class="btn btn--sm btn--ghost" :disabled="busy" @click="refreshSources">{{ t('plugins.sources.refresh') }}</button>
      </div>
      <div class="market__sources">
        <div class="market__sub">{{ t('plugins.sources.title') }}</div>
        <div v-for="s in sources" :key="s.kind + s.name" class="market__row">
          <code>{{ s.name }}</code>
          <span class="market__kind">{{ s.kind === 'external_repo' ? t('plugins.sources.kindExternalRepo') : t('plugins.sources.kindCatalog') }}</span>
          <a v-if="s.url" class="market__desc market__link" :href="s.url" target="_blank" rel="noopener">{{ s.url }}</a>
          <span v-else class="market__desc">{{ s.path }}</span>
        </div>
        <p v-if="sourcesHint" class="market__note">{{ sourcesHint }}</p>
      </div>
      <div class="market__form">
        <div class="market__sub">{{ t('plugins.sources.importTitle') }}</div>
        <textarea v-model="importText" class="market__textarea" :placeholder="t('plugins.sources.importPlaceholder')" />
        <div class="market__form-row">
          <label class="market__ack"><input v-model="importAck" type="checkbox" /> {{ t('plugins.sources.importAck') }}</label>
          <button class="btn btn--sm btn--primary" :disabled="busy || !importText.trim()" @click="importManifest">{{ t('plugins.sources.importAction') }}</button>
          <button class="btn btn--sm btn--ghost" :disabled="busy" @click="refreshMarket">{{ t('plugins.sources.refreshMarket') }}</button>
        </div>
        <div v-if="importResult" class="market__note">{{ importResult }}</div>
      </div>
    </section>

    <Teleport to="body">
      <!-- 详情抽屉：清单原文 + 注册表状态 + DSH 适配映射（RPC / 页面 / 边界说明） -->
      <div v-if="detailOpen" class="manage-drawer-scrim" @click.self="closeDetail">
        <aside class="manage-drawer" role="dialog" aria-modal="true" :aria-label="t('plugins.manage.detailTitle', { name: detail?.name || detailLoadingName })">
          <header class="manage-drawer__head">
            <div>
              <p class="manage__kicker">{{ t('plugins.manage.detailKicker') }}</p>
              <h2>{{ detail?.name || detailLoadingName }}</h2>
              <p v-if="detail?.manifestPath" class="manage-drawer__path">{{ detail.manifestPath }}</p>
            </div>
            <button type="button" class="manage-iconbtn" :aria-label="t('common.close')" @click="closeDetail">
              <Icon name="x" :size="18" />
            </button>
          </header>
          <div class="manage-drawer__body">
            <div v-if="detailError" class="market__note market__note--err">{{ detailError }}</div>
            <div v-if="detailLoading" class="market__note">{{ t('plugins.manage.detailLoading') }}</div>
            <template v-if="detail">
              <div class="manage-drawer__actions">
                <button class="btn btn--sm" :disabled="covBusy || !detail.registry" @click="togglePlugin(detail.name, !(detail.registry?.enabled ?? true))">
                  {{ detail.registry?.enabled === false ? t('plugins.toggle.enable') : t('plugins.toggle.disable') }}
                </button>
                <button class="btn btn--sm btn--ghost" :disabled="covBusy" @click="reloadPlugin(detail.name)">{{ t('plugins.toggle.reload') }}</button>
                <button class="btn btn--sm btn--ghost" @click="openEdit(detail)">{{ t('plugins.manage.edit') }}</button>
                <button class="btn btn--sm btn--ghost manage__danger" :disabled="!detail.writable" @click="deleteDetail(detail)">{{ t('plugins.manage.delete') }}</button>
              </div>

              <section class="manage-drawer__section">
                <h3>{{ t('plugins.manage.adaptationTitle') }}</h3>
                <div v-if="detail.adaptation" class="manage-adapt-card" :class="'manage-adapt-card--' + detail.adaptation.status">
                  <div class="manage-adapt-card__head">
                    <span class="manage__adapt" :class="'manage__adapt--' + detail.adaptation.status">{{ adaptationLabel(detail.adaptation.status) }}</span>
                    <span class="manage-adapt-card__note">{{ detail.adaptation.note }}</span>
                  </div>
                  <div class="manage-adapt-card__grid">
                    <div>
                      <span class="manage-adapt-card__label">{{ t('plugins.manage.adaptationRpc') }}</span>
                      <div class="manage-adapt-card__chips">
                        <code v-for="r in detail.adaptation.rpc" :key="r" class="manage-chip">{{ r }}</code>
                      </div>
                    </div>
                    <div>
                      <span class="manage-adapt-card__label">{{ t('plugins.manage.adaptationUi') }}</span>
                      <div class="manage-adapt-card__chips">
                        <RouterLink v-for="u in detail.adaptation.ui" :key="u" class="manage-chip manage-chip--link" :to="u">{{ u }}</RouterLink>
                      </div>
                    </div>
                  </div>
                </div>
                <p v-else class="market__note">{{ t('plugins.manage.adaptationNoneHint') }}</p>
              </section>

              <section class="manage-drawer__section">
                <h3>{{ t('plugins.manage.metaTitle') }}</h3>
                <dl class="manage-meta">
                  <div><dt>{{ t('plugins.manage.colVersion') }}</dt><dd>{{ detail.manifest?.version || '—' }}</dd></div>
                  <div><dt>{{ t('plugins.manage.author') }}</dt><dd>{{ detail.manifest?.author || '—' }}</dd></div>
                  <div><dt>{{ t('plugins.manage.colState') }}</dt><dd>{{ detail.registry?.enabled === false ? t('plugins.toggle.disabled') : t('plugins.toggle.enabled') }}</dd></div>
                  <div><dt>{{ t('plugins.manage.manifestWritable') }}</dt><dd>{{ detail.writable ? t('plugins.manage.writableYes') : t('plugins.manage.writableNo') }}</dd></div>
                  <div v-if="detail.registry?.dependsOn?.length"><dt>{{ t('plugins.manage.dependsOn') }}</dt><dd>{{ detail.registry.dependsOn.join(', ') }}</dd></div>
                </dl>
              </section>

              <section v-if="detail.manifest?.capabilities?.length" class="manage-drawer__section">
                <h3>{{ t('plugins.manage.capabilitiesTitle', { count: detail.manifest.capabilities.length }) }}</h3>
                <div class="manage-adapt-card__chips">
                  <code v-for="c in detail.manifest.capabilities" :key="c" class="manage-chip">{{ c }}</code>
                </div>
              </section>

              <section v-if="detail.diagnostics?.length" class="manage-drawer__section">
                <h3>{{ t('plugins.manage.diagnosticsTitle') }}</h3>
                <ul class="manage-diagnostics">
                  <li v-for="d in detail.diagnostics" :key="d.code + d.message">
                    <code>{{ d.code }}</code><span>{{ d.message }}</span>
                  </li>
                </ul>
              </section>

              <section class="manage-drawer__section">
                <div class="manage-drawer__section-head">
                  <h3>{{ t('plugins.manage.manifestTitle') }}</h3>
                  <button class="btn btn--sm btn--ghost" @click="copyManifest">{{ t('plugins.manage.copyManifest') }}</button>
                </div>
                <pre class="manage-manifest">{{ detail.raw || t('plugins.manage.noManifest') }}</pre>
              </section>
            </template>
          </div>
        </aside>
      </div>
    </Teleport>

    <Teleport to="body">
      <!-- 新建 / 编辑清单：JSON 直填或远程清单直链；复用 market.import / market.install_url -->
      <div v-if="editorOpen" class="manage-dialog-scrim" @click.self="closeEditor">
        <section class="manage-dialog" role="dialog" aria-modal="true" aria-labelledby="plugin-editor-title">
          <header class="manage-dialog__head">
            <div>
              <p class="manage__kicker">MANIFEST</p>
              <h2 id="plugin-editor-title">{{ editingName ? t('plugins.manage.editTitle', { name: editingName }) : t('plugins.manage.createTitle') }}</h2>
            </div>
            <button type="button" class="manage-iconbtn" :aria-label="t('common.close')" @click="closeEditor">
              <Icon name="x" :size="18" />
            </button>
          </header>
          <div class="manage-dialog__body">
            <label class="manage-field">
              <span>{{ t('plugins.manage.manifestLabel') }}</span>
              <textarea v-model="editorRaw" class="manage-textarea" rows="14" spellcheck="false" :placeholder="t('plugins.manage.manifestPlaceholder')" />
            </label>
            <label class="market__ack">
              <input v-model="editorAck" type="checkbox" />
              {{ t('plugins.sources.importAck') }}
            </label>
            <div class="manage-dialog__row">
              <input v-model="installUrl" class="manage-input" type="url" :placeholder="t('plugins.manage.urlPlaceholder')" />
              <button class="btn btn--sm btn--ghost" :disabled="busy || !installUrl.trim()" @click="installManifestFromUrl">{{ t('plugins.manage.installUrl') }}</button>
            </div>
            <div v-if="editorMessage" class="market__note" :class="{ 'market__note--err': !editorOk }">{{ editorMessage }}</div>
          </div>
          <footer class="manage-dialog__foot">
            <button class="btn btn--ghost" @click="closeEditor">{{ t('plugins.skill.cancel') }}</button>
            <button class="btn btn--primary" :disabled="busy || !editorRaw.trim()" @click="saveManifest">
              {{ busy ? t('plugins.manage.saving') : t('plugins.manage.save') }}
            </button>
          </footer>
        </section>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import Icon from '@/components/Icon.vue'
import { useRpcStore } from '@/stores/rpc'
import { useConfirm } from '@/composables/useConfirm'
import { openViewPanel } from '@/components/workbench/AppWorkbench.vue'

const { t } = useI18n()
const rpc = useRpcStore()

interface Plugin { name: string; description: string; group: string; tier: string; maturity: string; }

const plugins = ref<Plugin[]>([])
const search = ref('')
const error = ref('')

// 默认落在「插件管理」：列表 / 详情 / 增删改查 / 启停是日常主操作面；
// 装配/市场/来源保留为二级 tab，避免功能丢失。
type HubTabId = 'manage' | 'assembly' | 'toggle' | 'market' | 'sources'
const activeTab = ref<HubTabId>('manage')
const hubTabs: Array<{ id: HubTabId; labelKey: string }> = [
  { id: 'manage', labelKey: 'plugins.hub.tabManage' },
  { id: 'assembly', labelKey: 'plugins.hub.tabAssembly' },
  { id: 'toggle', labelKey: 'plugins.hub.tabToggle' },
  { id: 'market', labelKey: 'plugins.hub.tabMarket' },
  { id: 'sources', labelKey: 'plugins.hub.tabSources' },
]

const filtered = computed(() => {
  const q = search.value.toLowerCase()
  if (!q) return plugins.value
  return plugins.value.filter(p => p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q))
})

/**
 * Dock this view into the workbench. Docking is additive — the workbench opens its
 * own instance of the view, so the main area stays free to navigate elsewhere and
 * this route keeps its own copy.
 */
function dockInWorkbench() {
  openViewPanel({ view: 'plugins', title: t('plugins.title') })
}

function maturityClass(m: string) {
  return { 'maturity--real': m === 'Real', 'maturity--partial': m === 'Partial', 'maturity--narrative': m === 'Narrative' }
}

async function load() {
  error.value = ''
  try {
    // 冷加载竞态守卫：WS 仍处于 connecting 时先等连接就绪，避免把「未就绪」
    // 当成加载失败，把 `not connected (state: connecting)` 裸串渲染给用户。
    if (!rpc.isConnected) await rpc.waitForConnection(10_000)
    const res = await rpc.call<{ plugins: Plugin[] }>('plugins.list')
    plugins.value = res.plugins || []
  } catch (e) {
    error.value = t('plugins.loadFailed', { error: e instanceof Error ? e.message : String(e) })
  }
}

// ── 插件装配状态（P0 S4b：boot 装配 patch → PluginSlot，经 plugins.status 可见）──
// 装配快照在网关启动时生成（OnceLock，重启生效）；前端只读渲染 + 手动刷新。
interface AsmReport { applied: string[]; skipped: Array<[string, string]>; errors: Array<[string, string]> }
interface AsmSummary { manifests_total: number; assembled_count: number; patches_total: number; applied: number; skipped: number; errors: number }
interface AsmManifest { name: string; version: string; mode: string; entry: string | null; activation: string; assembled: boolean; enabled: boolean; slots: string[] }
interface AssemblyStatus { manifests: AsmManifest[]; slots: any[]; report: AsmReport; summary: AsmSummary }
const asm = ref<AssemblyStatus>({
  manifests: [],
  slots: [],
  report: { applied: [], skipped: [], errors: [] },
  summary: { manifests_total: 0, assembled_count: 0, patches_total: 0, applied: 0, skipped: 0, errors: 0 },
})
const asmError = ref('')
const asmBusy = ref(false)

async function loadAssembly() {
  asmBusy.value = true
  asmError.value = ''
  try {
    await rpc.waitForConnection()
    const r = await rpc.call<AssemblyStatus>('plugins.status')
    asm.value = r
  } catch (e) {
    asmError.value = t('plugins.assembly.statusFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    asmBusy.value = false
  }
}

// ── 插件热启停与统一覆盖率（plugins.coverage / setEnabled / reload）──
// 与「装配状态」的区别：装配快照是 boot 产物（重启生效）；本区块读注册表实时事实，
// 启停会真实改写 `registry.json` 并重跑 DI 拓扑。
interface CovMcp {
  declared: boolean
  command: string
  args: string[]
  timeoutMs: number
  running: boolean
  tools: string[]
  error: string | null
}
interface CovAdaptation {
  status: 'native' | 'partial' | 'declarative'
  rpc: string[]
  ui: string[]
  note: string
}
interface CovPlugin {
  id: string
  version: string
  description?: string | null
  author?: string | null
  capabilities?: string[]
  sources: string[]
  enabled: boolean
  effectiveEnabled: boolean
  declarative: boolean
  entry: string | null
  dependsOn: string[]
  skills: string[]
  mcp: CovMcp | null
  redline: boolean
  adaptation?: CovAdaptation | null
}
interface Cov {
  ok: boolean
  root: string
  registryPath: string
  managed: { total: number; symlinks: number; localFallback: number; brokenLinks: string[] }
  registry: { entries: number; enabled: number; disabled: number; declarative: number; persisted: boolean }
  manifests: { total: number; dir: string | null }
  patches: { total: number | null; dir: string | null; error: string | null }
  redlines: { contextLeakSuspects: string[] }
  plugins: CovPlugin[]
  gaps: { manifestOnly: string[]; registryOnly: string[] }
  skippedTotal: number
  lastError: string | null
}
const cov = ref<Cov | null>(null)
const covError = ref('')
const covBusy = ref(false)
const toggleNotice = ref('')
const manageNotice = ref('')

type PluginFilterId = 'all' | 'enabled' | 'disabled' | 'declarative' | 'adapted' | 'mcp' | 'redline'
const pluginSearch = ref('')
const pluginFilter = ref<PluginFilterId>('all')
const pluginFilters: Array<{ id: PluginFilterId; labelKey: string }> = [
  { id: 'all', labelKey: 'plugins.manage.filterAll' },
  { id: 'enabled', labelKey: 'plugins.manage.filterEnabled' },
  { id: 'disabled', labelKey: 'plugins.manage.filterDisabled' },
  { id: 'declarative', labelKey: 'plugins.manage.filterDeclarative' },
  { id: 'adapted', labelKey: 'plugins.manage.filterAdapted' },
  { id: 'mcp', labelKey: 'plugins.manage.filterMcp' },
  { id: 'redline', labelKey: 'plugins.manage.filterRedline' },
]

const adaptationStats = computed(() => {
  const list = cov.value?.plugins || []
  const native = list.filter(p => p.adaptation?.status === 'native').length
  const partial = list.filter(p => p.adaptation?.status === 'partial').length
  const declarative = list.filter(p => !p.adaptation || p.adaptation.status === 'declarative').length
  return { native, partial, declarative }
})

const filteredPlugins = computed(() => {
  const q = pluginSearch.value.trim().toLowerCase()
  return (cov.value?.plugins || []).filter(p => {
    if (pluginFilter.value === 'enabled' && !p.enabled) return false
    if (pluginFilter.value === 'disabled' && p.enabled) return false
    if (pluginFilter.value === 'declarative' && !p.declarative) return false
    if (pluginFilter.value === 'adapted' && (!p.adaptation || p.adaptation.status === 'declarative')) return false
    if (pluginFilter.value === 'mcp' && !p.mcp) return false
    if (pluginFilter.value === 'redline' && !p.redline) return false
    if (!q) return true
    return p.id.toLowerCase().includes(q)
      || String(p.description || '').toLowerCase().includes(q)
      || (p.capabilities || []).some(c => c.toLowerCase().includes(q))
  })
})

function adaptationLabel(status: string): string {
  if (status === 'native') return t('plugins.adaptation.native')
  if (status === 'partial') return t('plugins.adaptation.partial')
  return t('plugins.adaptation.declarative')
}

// ── 详情抽屉 ────────────────────────────────────────────────────────────
interface PluginDetailManifest {
  name?: string
  version?: string
  description?: string
  author?: string | null
  entry?: string | null
  capabilities?: string[]
  dependencies?: string[]
  skills?: Array<{ name: string; from?: string | null }>
}
interface PluginDetail {
  ok: boolean
  name: string
  manifestPath?: string | null
  manifestDir?: string | null
  writable: boolean
  hasManifest: boolean
  raw: string | null
  manifest: PluginDetailManifest | null
  diagnostics: Array<{ code: string; message: string }>
  registry: { enabled: boolean; declarative: boolean; entry: string; dependsOn: string[]; source: string } | null
  adaptation: CovAdaptation | null
}
const detailOpen = ref(false)
const detail = ref<PluginDetail | null>(null)
const detailLoading = ref(false)
const detailLoadingName = ref('')
const detailError = ref('')

async function openDetail(p: CovPlugin) {
  detailOpen.value = true
  detail.value = null
  detailLoading.value = true
  detailLoadingName.value = p.id
  detailError.value = ''
  try {
    await rpc.waitForConnection()
    if (!rpc.supportsMethod('plugins.detail')) {
      detailError.value = t('plugins.manage.detailUnsupported')
      return
    }
    detail.value = await rpc.call<PluginDetail>('plugins.detail', { name: p.id })
  } catch (e) {
    detailError.value = t('plugins.manage.detailFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    detailLoading.value = false
  }
}

function closeDetail() {
  detailOpen.value = false
  detail.value = null
  detailError.value = ''
}

async function refreshDetail() {
  const name = detail.value?.name || detailLoadingName.value
  if (!detailOpen.value || !name) return
  try {
    if (rpc.supportsMethod('plugins.detail')) {
      detail.value = await rpc.call<PluginDetail>('plugins.detail', { name })
    }
  } catch {
    // 详情刷新失败不覆盖已有内容；下一次打开会重新拉取。
  }
}

// ── 新建 / 编辑清单 ─────────────────────────────────────────────────────
const editorOpen = ref(false)
const editingName = ref('')
const editorRaw = ref('')
const editorAck = ref(false)
const editorMessage = ref('')
const editorOk = ref(true)
const installUrl = ref('')

function openInstall() {
  editingName.value = ''
  editorRaw.value = ''
  editorAck.value = false
  editorMessage.value = ''
  editorOk.value = true
  installUrl.value = ''
  editorOpen.value = true
}

function openEdit(d: PluginDetail) {
  editingName.value = d.name
  editorRaw.value = d.raw || ''
  editorAck.value = false
  editorMessage.value = ''
  editorOk.value = true
  installUrl.value = ''
  editorOpen.value = true
}

function closeEditor() {
  editorOpen.value = false
  editorMessage.value = ''
}

async function saveManifest() {
  if (!editorRaw.value.trim() || busy.value) return
  busy.value = true
  editorMessage.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; name?: string; version?: string; message?: string; error?: string }>(
      'market.import',
      { content: editorRaw.value, ack: editorAck.value },
    )
    editorOk.value = r.ok
    editorMessage.value = r.ok
      ? t('plugins.market.importSuccess', { name: r.name ?? '', version: r.version ?? '', message: r.message ?? '' })
      : t('plugins.market.importFailed', { error: r.error ?? '' })
    if (r.ok) {
      await Promise.all([refreshMarket(), loadCoverage()])
      closeEditor()
      manageNotice.value = t('plugins.manage.saved', { name: r.name ?? '' })
    }
  } catch (e) {
    editorOk.value = false
    editorMessage.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function installManifestFromUrl() {
  const url = installUrl.value.trim()
  if (!url || busy.value) return
  busy.value = true
  editorMessage.value = ''
  try {
    const r = await rpc.call<{ success: boolean; message: string }>('market.install_url', { url })
    editorOk.value = r.success
    editorMessage.value = r.message ?? (r.success ? t('plugins.manage.urlInstalled') : t('plugins.manage.urlFailed'))
    if (r.success) {
      await Promise.all([refreshMarket(), loadCoverage()])
      closeEditor()
      manageNotice.value = editorMessage.value
    }
  } catch (e) {
    editorOk.value = false
    editorMessage.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function copyManifest() {
  const text = detail.value?.raw || ''
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    manageNotice.value = t('plugins.manage.copied')
  } catch {
    manageNotice.value = t('plugins.manage.copyFailed')
  }
}

const { confirm } = useConfirm()

async function deletePlugin(p: CovPlugin) {
  if (covBusy.value) return
  const approved = await confirm({
    title: t('plugins.manage.deleteTitle', { name: p.id }),
    body: t('plugins.manage.deleteBody', { name: p.id }),
    primaryLabel: t('plugins.manage.deleteConfirm'),
  })
  if (!approved) return
  covBusy.value = true
  manageNotice.value = ''
  try {
    await rpc.waitForConnection()
    if (!rpc.supportsMethod('plugins.delete')) {
      manageNotice.value = t('plugins.manage.deleteUnsupported')
      return
    }
    const r = await rpc.call<{ ok: boolean; name?: string; error?: string; code?: string; note?: string }>(
      'plugins.delete',
      { name: p.id },
    )
    manageNotice.value = r.ok
      ? t('plugins.manage.deletedNotice', { name: p.id, note: r.note || '' })
      : t('plugins.manage.deleteFailed', { error: r.error || r.code || 'unknown' })
    if (r.ok) {
      if (detail.value?.name === p.id) closeDetail()
      await loadCoverage()
    }
  } catch (e) {
    manageNotice.value = t('plugins.manage.deleteFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    covBusy.value = false
  }
}

async function deleteDetail(d: PluginDetail) {
  await deletePlugin({
    id: d.name,
    version: d.manifest?.version || '',
    sources: [],
    enabled: d.registry?.enabled ?? true,
    effectiveEnabled: d.registry?.enabled ?? true,
    declarative: d.registry?.declarative ?? true,
    entry: d.registry?.entry || null,
    dependsOn: d.registry?.dependsOn || [],
    skills: [],
    mcp: null,
    redline: false,
    adaptation: d.adaptation,
  })
}

async function loadCoverage() {
  covBusy.value = true
  covError.value = ''
  try {
    await rpc.waitForConnection()
    cov.value = await rpc.call<Cov>('plugins.coverage')
  } catch (e) {
    covError.value = t('plugins.toggle.loadFailed', { error: e instanceof Error ? e.message : String(e) })
  } finally {
    covBusy.value = false
  }
}

/** 启停：失败时就地给出可读原因（不弹全局 toast），并回读真实状态。 */
async function togglePlugin(name: string, enabled: boolean) {
  covBusy.value = true
  toggleNotice.value = ''
  manageNotice.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; error?: string; code?: string }>('plugins.setEnabled', { name, enabled })
    const message = r.ok
      ? t(enabled ? 'plugins.toggle.enabledNotice' : 'plugins.toggle.disabledNotice', { name })
      : t('plugins.toggle.failed', { error: r.error || r.code || 'unknown' })
    toggleNotice.value = message
    manageNotice.value = message
  } catch (e) {
    const message = t('plugins.toggle.failed', { error: e instanceof Error ? e.message : String(e) })
    toggleNotice.value = message
    manageNotice.value = message
  } finally {
    covBusy.value = false
    await loadCoverage()
    await refreshDetail()
  }
}

/** 热重载单个插件（不改变启用状态）。 */
async function reloadPlugin(name: string) {
  covBusy.value = true
  toggleNotice.value = ''
  manageNotice.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; error?: string; code?: string }>('plugins.reload', { name })
    const message = r.ok
      ? t('plugins.toggle.reloadedNotice', { name })
      : t('plugins.toggle.failed', { error: r.error || r.code || 'unknown' })
    toggleNotice.value = message
    manageNotice.value = message
  } catch (e) {
    const message = t('plugins.toggle.failed', { error: e instanceof Error ? e.message : String(e) })
    toggleNotice.value = message
    manageNotice.value = message
  } finally {
    covBusy.value = false
    await loadCoverage()
    await refreshDetail()
  }
}

/** 启动插件 MCP 子进程（真实 spawn + initialize + tools/list）。 */
async function startMcp(name: string) {
  covBusy.value = true
  toggleNotice.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; tools?: string[]; error?: string; code?: string }>('plugins.mcp.start', { name })
    const message = r.ok
      ? t('plugins.toggle.mcpStarted', { name, n: (r.tools || []).length })
      : t('plugins.toggle.mcpFailed', { error: r.error || r.code || 'unknown' })
    toggleNotice.value = message
    manageNotice.value = message
  } catch (e) {
    const message = t('plugins.toggle.mcpFailed', { error: e instanceof Error ? e.message : String(e) })
    toggleNotice.value = message
    manageNotice.value = message
  } finally {
    covBusy.value = false
    await loadCoverage()
    await refreshDetail()
  }
}

/** 停止并回收插件进程。 */
async function stopMcp(name: string) {
  covBusy.value = true
  toggleNotice.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; error?: string; code?: string }>('plugins.mcp.stop', { name })
    const message = r.ok
      ? t('plugins.toggle.mcpStoppedNotice', { name })
      : t('plugins.toggle.mcpFailed', { error: r.error || r.code || 'unknown' })
    toggleNotice.value = message
    manageNotice.value = message
  } catch (e) {
    const message = t('plugins.toggle.mcpFailed', { error: e instanceof Error ? e.message : String(e) })
    toggleNotice.value = message
    manageNotice.value = message
  } finally {
    covBusy.value = false
    await loadCoverage()
    await refreshDetail()
  }
}

// ── DSH 插件市场：审计 / 导入 / 导出 ──
// `mode` / `entry` / `activation` are present only on `kind === 'plugin'` rows
// (gateway `market.list` → core/src/gateway/market.rs::plugin_market_item).
// `mode === 'declarative-only'` means the manifest declares capabilities but has
// no `entry`, so nothing is assembled at runtime — the old "installed but inert"
// state, which used to be completely silent. Skill / harness rows omit them.
interface MarketItem { name: string; version: string; description: string; kind: string; author?: string; source?: string; mode?: string; entry?: string | null; activation?: string }
interface AuditItem { name: string; version: string; capabilities: number; hooks: number; suspects: string[]; risks: string[]; verdict: string }
const marketItems = ref<MarketItem[]>([])
const auditItems = ref<AuditItem[]>([])
const importText = ref('')
const importAck = ref(false)
const importResult = ref('')
const marketError = ref('')
const busy = ref(false)

// 过滤标签页：全部 / 插件 / 技能 / Harness 生态
type MarketTabId = 'all' | 'plugin' | 'skill' | 'harness'
const marketTab = ref<MarketTabId>('all')
const marketTabs = computed<{ id: MarketTabId; label: string }[]>(() => [
  { id: 'all', label: t('plugins.market.tabAll') },
  { id: 'plugin', label: t('plugins.market.tabPlugin') },
  { id: 'skill', label: t('plugins.market.tabSkill') },
  { id: 'harness', label: t('plugins.market.tabHarness') },
])

// DeepSeek Harness 生态蒸馏目录条目
interface HarnessItem {
  name: string
  description: string
  category: string
  ecosystem: string
  capabilities: string[]
  distilled_via: string
  deepseek_aligned: boolean
}
const harnessItems = ref<HarnessItem[]>([])

// 本地清单（插件 + 技能），不含 harness 生态条目
const pluginSkillItems = computed(() => marketItems.value.filter(m => m.kind === 'plugin' || m.kind === 'skill'))

function tabCount(id: MarketTabId): number {
  if (id === 'all') return marketItems.value.length + harnessItems.value.length
  if (id === 'harness') return harnessItems.value.length
  return marketItems.value.filter(m => m.kind === id).length
}

function ecoLabel(eco: string): string {
  return eco === 'computer_use'
    ? t('plugins.ecosystem.computerUse')
    : t('plugins.ecosystem.harness')
}

function verdictLabel(v: string): string {
  if (v === 'clean') return t('plugins.verdict.clean')
  if (v === 'warn') return t('plugins.verdict.warn')
  if (v === 'broken') return t('plugins.verdict.broken')
  return v
}

async function refreshMarket() {
  marketError.value = ''
  try {
    await rpc.waitForConnection()
    const r = await rpc.call<{ items: MarketItem[]; total: number }>('market.list', {})
    marketItems.value = r.items ?? []
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function refreshHarness() {
  marketError.value = ''
  try {
    await rpc.waitForConnection()
    const r = await rpc.call<{ items: HarnessItem[]; total: number; hint?: string }>('market.harness', {})
    harnessItems.value = r.items ?? []
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function runAudit() {
  busy.value = true
  marketError.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; items: AuditItem[]; total_suspects: number; total_risks: number; error?: string }>('market.audit', {})
    if (r.ok === false) { marketError.value = r.error ?? t('plugins.market.auditFailed') }
    else {
      auditItems.value = r.items ?? []
      marketError.value = t('plugins.market.auditDone', {
        count: r.items?.length ?? 0,
        suspects: r.total_suspects ?? 0,
        risks: r.total_risks ?? 0,
      })
    }
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function importManifest() {
  busy.value = true
  importResult.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; name?: string; version?: string; message?: string; error?: string }>('market.import', { content: importText.value, ack: importAck.value })
    importResult.value = r.ok
      ? t('plugins.market.importSuccess', { name: r.name ?? '', version: r.version ?? '', message: r.message ?? '' })
      : t('plugins.market.importFailed', { error: r.error ?? '' })
    if (r.ok) { importText.value = ''; importAck.value = false; await refreshMarket() }
  } catch (e) { importResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function exportManifest(name: string) {
  busy.value = true
  importResult.value = ''
  try {
    const r = await rpc.call<{ ok: boolean; name?: string; content?: string; error?: string }>('market.export', { name })
    if (r.ok) { importText.value = r.content ?? ''; importResult.value = t('plugins.market.exportSuccess', { name }) }
    else importResult.value = r.error ?? t('plugins.market.exportFailed')
  } catch (e) { importResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

interface SourceItem { name: string; path: string; url?: string | null; kind: string }
const sources = ref<SourceItem[]>([])
const sourcesHint = ref('')
const batchResult = ref('')

async function refreshSources() {
  marketError.value = ''
  try {
    await rpc.waitForConnection()
    const r = await rpc.call<{ ok: boolean; items: SourceItem[]; total: number; hint?: string }>('market.sources', {})
    sources.value = r.items ?? []
    sourcesHint.value = r.hint ?? ''
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) }
}

async function installManifest(name: string) {
  busy.value = true
  try {
    const r = await rpc.call<{ success: boolean; message: string }>('market.install', { name })
    marketError.value = r.message ?? ''
    await refreshMarket()
  } catch (e) { marketError.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

async function batchInstall() {
  busy.value = true
  batchResult.value = ''
  const names = marketItems.value.filter(m => m.kind === 'plugin').map(m => m.name)
  if (!names.length) { busy.value = false; return }
  try {
    const r = await rpc.call<{ ok: boolean; results: Array<{ name: string; success: boolean; message: string }>; total: number }>('market.install.batch', { names })
    const okCount = (r.results ?? []).filter(x => x.success).length
    batchResult.value = t('plugins.market.batchResult', { ok: okCount, total: r.results?.length ?? 0 })
  } catch (e) { batchResult.value = e instanceof Error ? e.message : String(e) } finally { busy.value = false }
}

// ---- 远程插件源（DSH 开源生态）----
interface RemoteItem {
  name: string
  version?: string
  description?: string
  manifest_url?: string
}
const remoteUrl = ref('')
const remoteStatus = ref('')
const remoteItems = ref<RemoteItem[]>([])

/// 清单直链安装：后端会下载 → 校验 → 原子落盘，并把"重启生效"如实回传。
async function installFromUrl() {
  const url = remoteUrl.value.trim()
  if (!url || busy.value) return
  busy.value = true
  remoteStatus.value = ''
  try {
    const r = await rpc.call<{ success: boolean; message: string }>('market.install_url', { url })
    remoteStatus.value = r.message ?? (r.success ? '已安装' : '安装失败')
    if (r.success) await refreshMarket()
  } catch (e) {
    remoteStatus.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

/// 远程索引检索：未配置 registry_url 时后端会回传 howTo（而不是空列表）。
async function searchRemote() {
  busy.value = true
  remoteStatus.value = ''
  remoteItems.value = []
  try {
    const r = await rpc.call<{
      success: boolean
      message?: string
      howTo?: string
      items?: RemoteItem[]
      registry?: string
    }>('market.remote.search', {})
    if (!r.success) {
      remoteStatus.value = [r.message, r.howTo].filter(Boolean).join(' · ')
      return
    }
    remoteItems.value = r.items ?? []
    remoteStatus.value = `索引 ${r.registry || ''} 命中 ${remoteItems.value.length} 个插件`
  } catch (e) {
    remoteStatus.value = e instanceof Error ? e.message : String(e)
  } finally {
    busy.value = false
  }
}

async function installRemote(item: RemoteItem) {
  if (!item.manifest_url) return
  remoteUrl.value = item.manifest_url
  await installFromUrl()
}

// 统一加载：进入页面即拉取 list / status / coverage / sources / market / harness，
// 不再出现「首次进入来源永远空」的时序缺口。
async function loadAll() {
  await Promise.all([
    load(),
    loadAssembly(),
    loadCoverage(),
    refreshMarket(),
    refreshHarness(),
    refreshSources(),
  ])
}
onMounted(loadAll)

// Cold deep-link / in-page refresh mounts this view before the WS handshake
// finishes, so the first `market.list` is rejected with
// "not connected (state: connecting)". The refresh button used to be disabled
// whenever the list was empty — which locked the user out of the only recovery
// path (empty list ⇒ disabled ⇒ can never refill). The button is now always
// clickable, and this re-runs the load once the connection is actually ready.
watch(
  () => rpc.state,
  (state) => {
    if (state === 'connected' && !marketItems.value.length && !busy.value) {
      void refreshMarket()
      if (!sources.value.length) void refreshSources()
    }
  },
)
</script>

<style scoped>
.plugins-stage { padding: 24px 28px; max-width: 1200px; margin: 0 auto; }
/* 统一 tab 分区（装配 / 热启停 / 市场 / 来源） */
.hub-tabs { display: flex; gap: 0; margin-top: 16px; border-bottom: 1px solid var(--hairline); }
.hub-tab {
  padding: 10px 18px;
  border: 0;
  border-bottom: 2px solid transparent;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
}
.hub-tab.is-active { color: var(--accent); border-bottom-color: var(--accent); }
.hub-tab:hover:not(.is-active) { color: var(--text); }
.hub-panel { display: flex; flex-direction: column; }
/* 远程插件源：输入框 + 两个动作按钮同一行，窄屏自动换行 */
.market__remote { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 10px 0; }
.market__remote .plugins-search { flex: 1 1 280px; }
.market__remote .plugins-search input { width: 100%; }
.plugins-error { margin: 16px 0; padding: 10px 14px; background: color-mix(in srgb, var(--danger) 10%, var(--card)); border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent); border-radius: var(--radius-sm); color: var(--danger); font-size: 12px; }
.plugins-search { display: flex; align-items: center; gap: 6px; padding: 6px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); }
.plugins-search input { border: none; background: transparent; color: var(--text); font-size: 12px; outline: none; width: 180px; }
.plugins-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 10px; margin-top: 20px; }
.plugin-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; }
.plugin-card__header { display: flex; justify-content: space-between; align-items: center; }
.plugin-card__name { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 600; }
.plugin-card__maturity { font-size: 9px; font-weight: 700; padding: 2px 7px; border-radius: var(--radius-pill); }
.maturity--real { background: color-mix(in srgb, var(--ok) 15%, transparent); color: var(--ok); }
.maturity--partial { background: color-mix(in srgb, var(--warn) 15%, transparent); color: var(--warn); }
.maturity--narrative { background: color-mix(in srgb, var(--text-dim) 12%, transparent); color: var(--text-dim); }
.plugin-card__desc { font-size: 11px; color: var(--text-muted); margin: 6px 0; line-height: 1.5; }
.plugin-card__meta { display: flex; gap: 8px; font-size: 10px; color: var(--text-dim); }
.plugin-card__group { padding: 1px 6px; background: var(--bg-hover); border-radius: var(--radius-sm); }
.plugins-empty { text-align: center; padding: 40px; color: var(--text-dim); font-size: 13px; }
.skill-create { margin-top: 20px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 16px; }
.skill-create__title { font-size: 14px; font-weight: 600; margin: 0 0 12px; }
.skill-create__form { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
.skill-field { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--text-dim); }
.skill-field--full { grid-column: 1 / -1; }
.skill-input { padding: 7px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; }
.skill-input:focus { border-color: var(--accent); }
.skill-textarea { padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 12px; outline: none; resize: vertical; font-family: var(--font-mono, monospace); }
.skill-textarea:focus { border-color: var(--accent); }
.skill-create__actions { grid-column: 1 / -1; display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
.skill-create__result { grid-column: 1 / -1; font-size: 12px; padding: 8px 12px; border-radius: var(--radius-sm); }
.skill-create__result:not(.is-error) { background: color-mix(in srgb, var(--ok) 10%, var(--card)); color: var(--ok); }
.skill-create__result.is-error { background: color-mix(in srgb, var(--danger) 10%, var(--card)); color: var(--danger); }
.market { margin-top: 18px; padding: 16px; background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); }
.market__head { display: flex; justify-content: space-between; align-items: center; gap: 10px; }
.market__title { font-size: 14px; font-weight: 700; }
.market__note { margin-top: 8px; font-size: 12px; color: var(--accent); word-break: break-all; }
.market__note--err { color: var(--danger); }
.market__audit { margin-top: 12px; overflow: auto; }
.market__table { width: 100%; border-collapse: collapse; font-size: 12px; }
.market__table th, .market__table td { text-align: left; padding: 7px 8px; border-bottom: 1px solid var(--hairline); }
.market__verdict { padding: 2px 8px; border-radius: 999px; font-size: 11px; }
.market__verdict[data-v="clean"] { color: var(--ok); background: color-mix(in srgb, var(--ok) 12%, var(--card)); }
.market__verdict[data-v="warn"] { color: var(--warn); background: color-mix(in srgb, var(--warn) 12%, var(--card)); }
.market__verdict[data-v="broken"] { color: var(--danger); background: color-mix(in srgb, var(--danger) 12%, var(--card)); }
.market__finding { display: inline-block; margin: 2px 6px 2px 0; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); border: 1px solid var(--hairline); font-size: 11px; }
.market__finding--ok { color: var(--ok); border-color: color-mix(in srgb, var(--ok) 40%, transparent); }
.market__catalog { margin-top: 14px; }
.market__sub { font-size: 11px; font-weight: 600; color: var(--text-dim); margin-bottom: 8px; }
.market__row { display: flex; gap: 10px; align-items: center; padding: 6px 0; border-bottom: 1px solid var(--hairline); }
.market__row code { font-family: var(--font-mono); font-size: 12px; }
.market__kind { font-size: 10px; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); color: var(--text-dim); }
.market__mode { font-size: 10px; padding: 2px 7px; border-radius: 999px; background: var(--bg-hover); border: 1px solid var(--warn); color: var(--warn); white-space: nowrap; }
.market__desc { flex: 1; min-width: 0; font-size: 11px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.market__link { color: var(--accent); text-decoration: none; }
.market__sources { margin-top: 14px; }
.market__form { margin-top: 14px; border-top: 1px solid var(--hairline); padding-top: 10px; }
.market__textarea { width: 100%; min-height: 72px; padding: 8px 10px; border-radius: var(--radius-sm); border: 1px solid var(--hairline); background: var(--bg); color: var(--text); font-size: 11px; resize: vertical; font-family: var(--font-mono, monospace); }
.market__form-row { display: flex; gap: 10px; align-items: center; margin-top: 8px; }
.market__ack { font-size: 11px; color: var(--text-dim); display: flex; gap: 5px; align-items: center; }
.market__tabs { display: flex; gap: 6px; margin-top: 14px; flex-wrap: wrap; }
.market__tab { display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px; border-radius: 999px; border: 1px solid var(--hairline); background: var(--bg); color: var(--text-dim); font-size: 12px; cursor: pointer; transition: all var(--dur-fast) var(--ease-standard); }
.market__tab:hover { color: var(--text); border-color: color-mix(in srgb, var(--accent) 40%, transparent); }
.market__tab.is-active { color: var(--accent); border-color: var(--accent); background: color-mix(in srgb, var(--accent) 12%, var(--card)); }
.market__tab-count { font-size: 10px; padding: 1px 6px; border-radius: 999px; background: var(--bg-hover); color: var(--text-dim); }
.market__harness { margin-top: 16px; border-top: 1px dashed var(--hairline); padding-top: 14px; }
.market__harness-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 10px; }
.harness-card { background: var(--card); border: 1px solid var(--hairline); border-radius: var(--radius-md); padding: 12px 14px; display: flex; flex-direction: column; gap: 8px; }
.harness-card--aligned { border-color: color-mix(in srgb, var(--accent) 40%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--card)); }
.harness-card__head { display: flex; justify-content: space-between; align-items: center; gap: 8px; }
.harness-card__name { font-family: var(--font-mono, monospace); font-size: 12px; font-weight: 600; }
.harness-card__eco { font-size: 9px; font-weight: 700; padding: 2px 7px; border-radius: var(--radius-pill); white-space: nowrap; }
.eco--harness { background: color-mix(in srgb, var(--accent) 15%, transparent); color: var(--accent); }
.eco--computer_use { background: color-mix(in srgb, var(--text-dim) 18%, transparent); color: var(--text-dim); }
.harness-card__desc { font-size: 11px; color: var(--text-muted); margin: 0; line-height: 1.5; }
.harness-card__tags { display: flex; flex-wrap: wrap; gap: 4px; }
.harness-card__tag { font-size: 10px; padding: 1px 6px; border-radius: var(--radius-sm); background: var(--bg-hover); border: 1px solid var(--hairline); color: var(--text-dim); font-family: var(--font-mono, monospace); }
.harness-card__distill { display: flex; flex-direction: column; gap: 3px; padding: 6px 8px; background: var(--bg); border-radius: var(--radius-sm); border: 1px solid var(--hairline); }
.harness-card__distill-label { font-size: 9px; font-weight: 700; color: var(--text-dim); text-transform: uppercase; letter-spacing: .04em; }
.harness-card__distill code { font-family: var(--font-mono, monospace); font-size: 10px; color: var(--text-muted); word-break: break-all; }
.harness-card__aligned { font-size: 10px; font-weight: 600; color: var(--accent); padding: 2px 8px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 12%, transparent); align-self: flex-start; }
/* ── 插件管理（独立页）────────────────────────────────────────────── */
.manage { margin-top: 16px; }
.manage__head-actions { align-items: center; display: flex; gap: 8px; }
.manage__stats { align-items: center; display: flex; flex-wrap: wrap; gap: 8px; margin-top: 10px; }
.manage__stat { align-items: baseline; background: var(--bg-elevated); border: 1px solid var(--hairline); border-radius: var(--radius-pill); color: var(--text-muted); display: inline-flex; font-size: 11px; gap: 6px; padding: 4px 11px; }
.manage__stat b { color: var(--text); font-family: var(--font-mono); font-size: 13px; }
.manage__stat--ok b { color: var(--ok); }
.manage__stat--warn b { color: var(--warn); }
.manage__toolbar { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; margin-top: 12px; }
.manage__search { flex: 1 1 260px; }
.manage__search input { width: 100%; }
.manage__chips { display: flex; flex-wrap: wrap; gap: 6px; }
.manage__chip { background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-pill); color: var(--text-dim); cursor: pointer; font: inherit; font-size: 12px; padding: 5px 12px; transition: border-color var(--dur-fast) var(--ease-standard), color var(--dur-fast) var(--ease-standard); }
.manage__chip:hover { color: var(--text); }
.manage__chip.is-active { background: color-mix(in srgb, var(--accent) 12%, var(--card)); border-color: var(--accent); color: var(--accent); }
.manage__table { margin-top: 10px; }
.manage__table td { vertical-align: top; }
.manage__row--off { background: color-mix(in srgb, var(--warn) 6%, transparent); }
.manage__name { font-family: var(--font-mono); font-size: 12px; }
.manage__desc { color: var(--text-muted); font-size: 11px; line-height: 1.5; margin: 3px 0 5px; max-width: 460px; }
.manage__adapt { border: 1px solid var(--hairline); border-radius: var(--radius-pill); font-size: 10px; padding: 2px 8px; white-space: nowrap; }
.manage__adapt--native { border-color: color-mix(in srgb, var(--ok) 45%, var(--border)); color: var(--ok); }
.manage__adapt--partial { border-color: color-mix(in srgb, var(--warn) 45%, var(--border)); color: var(--warn); }
.manage__adapt--declarative { border-color: color-mix(in srgb, var(--danger) 40%, var(--border)); color: var(--danger); }
.manage__adapt--none { color: var(--text-dim); }
.manage__actions { display: flex; flex-wrap: wrap; gap: 6px; }
.manage__danger { color: var(--danger); }
.manage__kicker { color: var(--accent); font-family: var(--font-mono); font-size: 10px; letter-spacing: 0.18em; margin: 0 0 4px; text-transform: uppercase; }

/* 详情抽屉 */
.manage-drawer-scrim { align-items: stretch; background: var(--scrim); display: flex; inset: 0; justify-content: flex-end; position: fixed; z-index: 1100; }
.manage-drawer { background: var(--bg-surface); border-left: 1px solid var(--hairline); box-shadow: var(--elev-3); display: flex; flex-direction: column; max-width: 640px; width: min(94vw, 640px); }
.manage-drawer__head { align-items: flex-start; border-bottom: 1px solid var(--hairline); display: flex; gap: 12px; justify-content: space-between; padding: 18px 20px; }
.manage-drawer__head h2 { font-family: var(--font-mono); font-size: 16px; margin: 0; overflow-wrap: anywhere; }
.manage-drawer__path { color: var(--text-dim); font-family: var(--font-mono); font-size: 10px; margin: 4px 0 0; overflow-wrap: anywhere; }
.manage-iconbtn { background: transparent; border: 1px solid var(--hairline); border-radius: var(--radius-control); color: var(--text-muted); cursor: pointer; display: inline-flex; padding: 6px; }
.manage-iconbtn:hover { color: var(--text); }
.manage-drawer__body { display: flex; flex: 1; flex-direction: column; gap: 16px; overflow: auto; padding: 18px 20px; }
.manage-drawer__actions { display: flex; flex-wrap: wrap; gap: 8px; }
.manage-drawer__section { border-top: 1px solid var(--hairline); display: flex; flex-direction: column; gap: 8px; padding-top: 14px; }
.manage-drawer__section h3 { color: var(--text-dim); font-size: 11px; font-weight: 700; letter-spacing: 0.06em; margin: 0; text-transform: uppercase; }
.manage-drawer__section-head { align-items: center; display: flex; justify-content: space-between; }
.manage-adapt-card { background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-md); display: flex; flex-direction: column; gap: 10px; padding: 12px; }
.manage-adapt-card--native { border-color: color-mix(in srgb, var(--ok) 35%, var(--hairline)); }
.manage-adapt-card--partial { border-color: color-mix(in srgb, var(--warn) 35%, var(--hairline)); }
.manage-adapt-card__head { align-items: flex-start; display: flex; flex-direction: column; gap: 6px; }
.manage-adapt-card__note { color: var(--text-muted); font-size: 12px; line-height: 1.6; }
.manage-adapt-card__grid { display: grid; gap: 12px; grid-template-columns: repeat(2, minmax(0, 1fr)); }
.manage-adapt-card__label { color: var(--text-dim); display: block; font-size: 10px; letter-spacing: 0.05em; margin-bottom: 5px; text-transform: uppercase; }
.manage-adapt-card__chips { display: flex; flex-wrap: wrap; gap: 5px; }
.manage-chip { background: var(--bg-hover); border: 1px solid var(--hairline); border-radius: var(--radius-sm); color: var(--text-muted); font-family: var(--font-mono); font-size: 10px; padding: 2px 6px; }
.manage-chip--link { color: var(--accent); text-decoration: none; }
.manage-chip--link:hover { border-color: var(--accent); }
.manage-meta { display: grid; gap: 8px; margin: 0; }
.manage-meta > div { display: grid; gap: 10px; grid-template-columns: 130px minmax(0, 1fr); }
.manage-meta dt { color: var(--text-dim); font-size: 11px; text-transform: uppercase; }
.manage-meta dd { color: var(--text); font-size: 12px; margin: 0; overflow-wrap: anywhere; }
.manage-diagnostics { display: flex; flex-direction: column; gap: 6px; list-style: none; margin: 0; padding: 0; }
.manage-diagnostics li { align-items: baseline; display: flex; gap: 8px; font-size: 12px; }
.manage-diagnostics code { color: var(--warn); font-family: var(--font-mono); font-size: 10px; white-space: nowrap; }
.manage-diagnostics span { color: var(--text-muted); }
.manage-manifest { background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-md); color: var(--text-muted); font-family: var(--font-mono); font-size: 11px; line-height: 1.6; margin: 0; max-height: 340px; overflow: auto; padding: 10px 12px; white-space: pre-wrap; word-break: break-word; }

/* 清单编辑器 */
.manage-dialog-scrim { align-items: center; background: var(--scrim); display: flex; inset: 0; justify-content: center; padding: 24px; position: fixed; z-index: 1101; }
.manage-dialog { background: var(--bg-surface); border: 1px solid var(--border); border-radius: var(--radius-lg); box-shadow: var(--elev-3); display: flex; flex-direction: column; max-height: 88vh; max-width: 760px; width: 100%; }
.manage-dialog__head { align-items: flex-start; border-bottom: 1px solid var(--hairline); display: flex; gap: 12px; justify-content: space-between; padding: 18px 20px; }
.manage-dialog__head h2 { font-size: 16px; margin: 0; }
.manage-dialog__body { display: flex; flex-direction: column; gap: 12px; overflow: auto; padding: 16px 20px; }
.manage-dialog__foot { border-top: 1px solid var(--hairline); display: flex; gap: 8px; justify-content: flex-end; padding: 12px 20px; }
.manage-field { display: flex; flex-direction: column; gap: 6px; font-size: 11px; color: var(--text-dim); }
.manage-textarea, .manage-input { background: var(--bg); border: 1px solid var(--hairline); border-radius: var(--radius-sm); color: var(--text); font-family: var(--font-mono); font-size: 12px; outline: none; padding: 8px 10px; }
.manage-textarea { resize: vertical; }
.manage-textarea:focus, .manage-input:focus { border-color: var(--accent); }
.manage-dialog__row { align-items: center; display: flex; gap: 8px; }
.manage-dialog__row .manage-input { flex: 1; font-family: inherit; }
@media (max-width: 760px) {
  .manage-adapt-card__grid { grid-template-columns: minmax(0, 1fr); }
  .manage-dialog-scrim { align-items: flex-end; padding: 0; }
  .manage-dialog { border-bottom-left-radius: 0; border-bottom-right-radius: 0; max-height: 92vh; }
}
</style>