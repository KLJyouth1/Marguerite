<template>
  <!-- 玛格丽特 · 星夜：3D 山脊（雪脊 + 补给站 + 结组登山队 + 云海 + 日出 / 左半星空）
       WebGL 不可用时退回 Canvas 星空，两者都只在该主题激活时挂载 -->
  <MargueriteRidge v-if="ridgeActive" />
  <MargueriteSky v-else-if="skyActive" />
  <!-- 液态玻璃 · 流光：仅在该主题激活时挂载背景质感层（云朵/海面/沙丘） -->
  <GlassBackdrop v-if="glassActive" />
  <!-- Sidebar -->
  <nav
    ref="sidebarRef"
    class="sidebar"
    :class="{
      docked: appStore.sidebarOpen,
      'sidebar--drawer': isSidebarDrawer,
      'sidebar--icons': iconRailActive,
    }"
    :inert="!appStore.sidebarOpen"
    :aria-hidden="appStore.sidebarOpen ? undefined : 'true'"
    :aria-label="t('chrome.primaryNav')"
    id="sidebar-nav"
  >
    <!-- Brand -->
    <div class="sidebar-brand">
      <div class="sidebar-brand-lockup">
        <img class="sidebar-brand-mark" :src="brandMarkUrl" alt="" aria-hidden="true" />
        <span class="sidebar-brand-text">Marguerite</span>
      </div>
      <button
        ref="sidebarDockToggleRef"
        class="sidebar-dock-toggle"
        :aria-label="t('chrome.collapseSidebar')"
        aria-controls="sidebar-nav"
        :aria-expanded="appStore.sidebarOpen"
        :aria-keyshortcuts="sidebarToggleAriaShortcut"
        aria-describedby="sidebar-toggle-tip-expanded"
        data-testid="sidebar-toggle-expanded"
        @click="toggleDock('sidebar-button')"
      >
        <Icon name="sidebar-visible" :size="18" />
        <span
          id="sidebar-toggle-tip-expanded"
          class="sidebar-toggle-tip sidebar-toggle-tip--sidebar"
          role="tooltip"
        >
          <span>{{ t('chrome.toggleSidebar') }}</span>
          <kbd v-if="sidebarToggleHint">{{ sidebarToggleHint }}</kbd>
        </span>
      </button>
    </div>

    <!-- Bounded, self-scrolling nav index. The primary rail (New task + Work
         rows) and the collapsible capability groups share ONE scroll region so
         a short viewport shrinks the nav as a unit instead of squeezing the
         rail into a sliver — or overflowing the column. Every destination
         stays reachable by scrolling the nav; Recents keeps the elastic space
         below. See `.sidebar-nav` in base.css. -->
    <div class="sidebar-nav">
    <div class="sidebar-section sidebar-core" role="navigation" :aria-label="t('chrome.controlNav')">
      <!-- New task leads the index: it opens a draft instantly against the
           default agent (no picker to interrupt the flow) and reads as a row
           rather than a boxed button so the sidebar keeps one rhythm. -->
      <button
        class="sidebar-new-session"
        :title="newChatHint ? `${t('chrome.newTaskTitle')} (${newChatHint})` : t('chrome.newTaskTitle')"
        @click="startNewChatInstant"
      >
        <Icon name="plus" :size="16" />
        <span class="sidebar-new-session__label">{{ t('chrome.newTask') }}</span>
        <!-- Badge tracks the configured binding and hides when the shortcut is
             disabled (Settings → Keyboard), so it never advertises a dead key. -->
        <kbd v-if="newChatHint" class="sidebar-kbd" aria-hidden="true">{{ newChatHint }}</kbd>
      </button>
      <!-- Overview / Skills & Channels / Cron, single-sourced from route
           metadata so the rail, mobile drawer, and palette never drift. -->
      <router-link
        v-for="item in workNav"
        :key="item.path"
        :to="item.path"
        class="sidebar-fn-item"
        :class="{ 'is-active': isPrimaryNavActive(item.path) }"
        :aria-current="isPrimaryNavActive(item.path) ? 'page' : undefined"
        @click="handleNavClick"
      >
        <Icon :name="item.icon" :size="16" />
        <span class="sidebar-fn-label">{{ item.title }}</span>
      </router-link>
    </div>

    <!-- Collapsible capability groups (Task #6): deep-link destinations that
         were previously unreachable from the rail are now bucketed into
         Work / Operate / Observe / Settings sections. Each section collapses
         independently; the active route stays visible even when collapsed. -->
    <div
      v-for="section in groupedNav"
      :key="section.id"
      class="sidebar-group"
      :data-group="section.id"
    >
      <!-- Icon-rail mode: the group header becomes a centered icon anchor that
           opens a hover/click flyout with the section's items. The anchor is
           marked active whenever any item in the group matches the current
           route, so a collapsed group never hides the active destination. -->
      <div v-if="iconRailActive" class="sidebar-icon-anchor">
        <button
          type="button"
          class="sidebar-fn-item sidebar-icon-anchor__btn"
          :class="{ 'is-active': isGroupNavActive(section) }"
          :aria-expanded="openIconGroupId === section.id"
          :aria-label="section.title"
          @click="toggleIconGroupFlyout(section.id)"
          @mouseenter="openIconGroupId = section.id"
          @mouseleave="onIconAnchorLeave(section.id)"
          @focus="openIconGroupId = section.id"
        >
          <Icon :name="GROUP_ICONS[section.id] ?? 'boxes'" :size="16" />
        </button>
        <div
          v-if="openIconGroupId === section.id"
          class="sidebar-icon-flyout"
          role="menu"
          :aria-label="section.title"
          @mouseenter="openIconGroupId = section.id"
          @mouseleave="onIconAnchorLeave(section.id)"
        >
          <router-link
            v-for="item in section.items"
            :key="item.path"
            :to="item.path"
            class="sidebar-fn-item sidebar-fn-item--flyout"
            :class="{ 'is-active': isPrimaryNavActive(item.path) }"
            :aria-current="isPrimaryNavActive(item.path) ? 'page' : undefined"
            @click="handleNavClick(); closeIconGroupFlyout()"
          >
            <Icon :name="item.icon" :size="16" />
            <span class="sidebar-fn-label">{{ item.title }}</span>
          </router-link>
        </div>
      </div>
      <template v-else>
      <button
        type="button"
        class="sidebar-group__header"
        :aria-expanded="!collapsedGroups.has(section.id)"
        @click="toggleGroup(section.id)"
      >
        <Icon
          name="chevronRight"
          :size="14"
          class="sidebar-group__chevron"
          :class="{ 'is-collapsed': collapsedGroups.has(section.id) }"
        />
        <span class="sidebar-group__title">{{ section.title }}</span>
        <span
          v-if="section.items.length > 0"
          class="sidebar-group__count"
          :title="t('nav.groupCountHint', { count: section.items.length, group: section.title })"
        >{{ section.items.length }}</span>
      </button>
      <div v-show="!collapsedGroups.has(section.id)" class="sidebar-group__items">
        <template v-for="item in section.items" :key="item.path">
          <!-- 鲸鱼娘（/pet）保留原导航语义，只是把入口行换成紧凑桌宠卡片；
               卡片本身不含 3D 舞台，全局 DesktopPet 仍是唯一的 WebGL 宿主。 -->
          <SidebarPetModule
            v-if="item.path === '/pet'"
            :active="isPrimaryNavActive(item.path)"
            @navigate="handleNavClick"
          />
          <router-link
            v-else
            :to="item.path"
            class="sidebar-fn-item sidebar-fn-item--nested"
            :class="{ 'is-active': isPrimaryNavActive(item.path) }"
            :aria-current="isPrimaryNavActive(item.path) ? 'page' : undefined"
            @click="handleNavClick"
          >
            <Icon :name="item.icon" :size="16" />
            <span class="sidebar-fn-label">{{ item.title }}</span>
          </router-link>
        </template>
      </div>
      </template>
    </div>
    <!-- Shell extension seam (append-only; renders nothing while empty). -->
    <SlotHost slot="sidebar.nav" />
    </div><!-- /.sidebar-nav -->

    <SidebarSetupBanner v-if="!iconRailActive" />

    <!-- Recent conversations. Hidden in icon-rail mode: the strip carries
         navigation only; conversations stay reachable via Ctrl+K / reopening
         the full sidebar. -->
    <SidebarConversations
      v-if="!iconRailActive"
      :sections="sidebarSections"
      :error="sessionListError"
      :loading="isLoading"
      :current-key="sidebarCurrentKey"
      :contract-debug-enabled="contractDebugEnabled"
      :search-hint="commandPaletteHint"
      :can-manage-projects="rpcStore.canManageProjectWorkspaces"
      :can-create-projects="rpcStore.canChooseProject"
      @select="switchToSession"
      @refresh="loadSidebarData"
      @rename="onRenameSession"
      @reset="onResetSession"
      @delete="onDeleteSession"
      @bulk-delete="onBulkDeleteSessions"
      @reorder="onReorderSidebarSession"
      @session-pin="onPinSidebarSession"
      @new-chat="startNewChatInstant"
      @new-project="openProjectCreator"
      @new-project-task="startProjectTask"
      @project-pin="onProjectPin"
      @project-edit="openProjectEditor"
      @project-delete-history="onProjectDeleteHistory"
      @project-remove="onProjectRemove"
      @search="openCommandPalette"
    />

    <!-- Fixed footer: icon-rail toggle + settings + connection state -->
    <div class="sidebar-foot">
      <button
        type="button"
        class="sidebar-fn-item"
        data-icon="icon-rail"
        :aria-label="t('chrome.iconRail')"
        :title="t('chrome.iconRail')"
        :aria-pressed="appStore.sidebarIconRail"
        @click="appStore.toggleSidebarIconRail()"
      >
        <Icon :name="appStore.sidebarIconRail ? 'panel-left-open' : 'panel-left-close'" :size="16" />
        <span class="sidebar-fn-label">{{ t('chrome.iconRail') }}</span>
      </button>
      <button
        type="button"
        class="sidebar-fn-item"
        data-icon="settings"
        @click="openSettings"
      >
        <Icon name="settings" :size="16" />
        <span class="sidebar-fn-label">{{ t('chrome.settings') }}</span>
      </button>
      <!-- Shell extension seam (append-only; renders nothing while empty). -->
      <SlotHost slot="sidebar.footer" />
    </div>
  </nav>

  <SidebarResizer
    v-if="appStore.sidebarOpen && isSidebarResizable && !iconRailActive"
    ref="sidebarResizerRef"
    :enabled="appStore.sidebarOpen && isSidebarResizable && !iconRailActive"
    :width="sidebarEffectiveWidth"
    :min="SIDEBAR_MIN_WIDTH"
    :max="sidebarDynamicMaximum"
    :preference="appStore.sidebarWidthPreference.width"
    :preference-source="appStore.sidebarWidthPreference.source"
    @resize-start="handleSidebarResizeStart"
    @preview="applySidebarPreview"
    @commit="commitSidebarWidth"
    @reset="resetSidebarWidth"
    @collapse="collapseSidebarFromResize"
    @cancel="applySidebarPreview"
    @resize-end="handleSidebarResizeEnd"
  />

  <!-- Drawer scrim is driven by the same runtime mode as JS focus/Escape logic. -->
  <div
    v-if="appStore.sidebarOpen && isSidebarDrawer"
    class="sidebar-scrim"
    role="presentation"
    aria-hidden="true"
    @click="closeSidebarDrawer"
  />

  <CommandPalette
    v-model:open="commandPaletteOpen"
    :recents="sidebarSections"
    @new-chat="onPaletteNewChat"
    @open-settings="onPaletteOpenSettings"
    @toggle-theme="onPaletteToggleTheme"
    @select-session="onPaletteSelectSession"
  />

  <!-- Ctrl/⌘+P quick-open: modal file search over the project bound to the
       current task. Opening a hit lands in the editor via ?path=. -->
  <FileQuickOpen
    :open="fileQuickOpenOpen"
    :base-path="statusProject.path"
    @close="fileQuickOpenOpen = false"
    @open="openQuickOpenFile"
  />

  <!-- Main content -->
  <div
    id="app-main"
    class="main"
    :inert="appStore.sidebarOpen && isSidebarDrawer"
    :class="{
      docked: appStore.sidebarOpen,
      'main--sidebar-drawer': isSidebarDrawer,
      'main--sidebar-icons': iconRailActive,
      'main--sidebar-compact': sidebarLayoutMode === 'compact',
      'main--chat': isChatRoute,
      'main--chat-sidebar-collapsed': isChatRoute && !appStore.sidebarOpen,
      'main--tabbar-hidden': mobileKeyboardOpen,
    }"
  >
    <header
      class="topbar"
      :class="{
        'topbar--chat': isChatRoute,
        'topbar--desktop-shell': isDesktopShell,
      }"
    >
      <div class="topbar-left">
        <!-- Sidebar toggle — visible when sidebar is collapsed -->
        <button
          v-show="!appStore.sidebarOpen"
          ref="topbarSidebarToggleRef"
          class="sidebar-dock-toggle topbar-toggle"
          :aria-label="t('chrome.expandSidebar')"
          aria-controls="sidebar-nav"
          :aria-expanded="appStore.sidebarOpen"
          :aria-keyshortcuts="sidebarToggleAriaShortcut"
          aria-describedby="sidebar-toggle-tip-collapsed"
          data-testid="sidebar-toggle-collapsed"
          @click="toggleDock('topbar-button')"
        >
          <Icon name="sidebar-hidden" :size="18" />
          <span id="sidebar-toggle-tip-collapsed" class="sidebar-toggle-tip" role="tooltip">
            <span>{{ t('chrome.toggleSidebar') }}</span>
            <kbd v-if="sidebarToggleHint">{{ sidebarToggleHint }}</kbd>
          </span>
        </button>
        <!-- 禅模式：一键收起侧栏与工作台 dock，只留内容区；再点还原进入前状态。
             复用既有 .topbar-toggle 样式，不新增 CSS（从而不触碰配色/动效/圆角守卫）。 -->
        <button
          class="sidebar-dock-toggle topbar-toggle"
          :aria-label="t('chrome.zenMode')"
          :aria-pressed="appStore.zenMode"
          :title="t('chrome.zenMode')"
          data-testid="zen-mode-toggle"
          @click="appStore.toggleZenMode()"
        >
          <Icon :name="appStore.zenMode ? 'collapse' : 'expand'" :size="18" />
        </button>
      </div>
      <!-- Permanent target: Chat teleports its route-owned actions into the
           same in-flow header as the app controls without duplicating state. -->
      <div
        id="app-route-header"
        class="topbar-route-header"
        data-testid="route-header-host"
      ></div>
      <!-- Custom title-bar drag region. The main window runs with native
           decorations disabled, so the blank part of the topbar is the only
           place the user can move the window.

           It MUST stay out of flow. `.topbar--chat` is a GRID whose four columns
           were declared for exactly four children; a fifth in-flow child makes
           `.topbar-right` auto-place onto an implicit second row, which pushed
           the window controls below the bar where the chat banner covered them.
           Absolute positioning keeps the grid/flex track count untouched and gives
           the region a guaranteed draggable width independent of the route actions.
           `pointer-events` follows `.topbar` (none) except where the region itself
           is the drag target, so it can never intercept a control click. -->
      <div
        v-if="isDesktopShell"
        class="topbar-drag-region"
        data-tauri-drag-region
        aria-hidden="true"
      />
      <!-- 全局模式徽标：实时显示当前运行时模式，点击进智能中枢切换 -->
      <ModeBadge />
      <div
        ref="topbarRightRef"
        class="topbar-right"
        :class="{ 'topbar-right--attention': appStore.approvalCount > 0 }"
      >
        <button
          v-if="appStore.approvalCount > 0"
          class="approval-inline"
          @click="openBlockedApprovalSession"
          :title="t('chrome.openBlockedSession')"
        >
          {{ t('chrome.approvalRequired') }}
        </button>
        <!-- P2-4：侧车离线时它的挂起审批没有推送也没有卡片，只存在于侧车落盘
             队列里。这里用黄色提示条如实说明「有 N 条待同步」，不混进红色审批
             badge（那个指向可裁决的卡片）。点击进连接设置排查侧车。 -->
        <button
          v-if="appStore.sidecarOfflineApprovals > 0"
          type="button"
          class="approval-inline approval-inline--offline"
          :title="t('chrome.sidecarOfflineApprovalsTitle')"
          @click="openConnectionSettings"
        >
          {{ t('chrome.sidecarOfflineApprovals', { count: appStore.sidecarOfflineApprovals }) }}
        </button>
        <button
          v-if="webConfigEnabled"
          type="button"
          class="conn-pill conn-pill--link"
          :class="gatewayReadiness"
          :title="t('chrome.connectionTitle', { state: connectionStateLabel })"
          :aria-label="t('chrome.manageConnection')"
          @click="openConnectionSettings"
        >{{ connectionStateLabel }}</button>
        <span
          v-else
          class="conn-pill"
          :class="gatewayReadiness"
          :title="t('chrome.connectionTitle', { state: connectionStateLabel })"
          :aria-label="t('chrome.connectionTitle', { state: connectionStateLabel })"
        >{{ connectionStateLabel }}</span>
        <DesktopUpdateIndicator />
        <!-- Opt-in (Settings → Appearance or the command palette); off by
             default so the topbar stays music-free until asked for. -->
        <BgmControl v-if="bgmEnabled" />
        <LanguageSwitcher />
        <ThemeMenu
          ref="themeMenuRef"
          v-model:open="themeMenuOpen"
          :theme="appStore.theme"
          :resolved-theme="appStore.resolvedTheme"
          @pick-theme="onThemePick"
          @open-more-themes="onOpenMoreThemes"
        />
        <!-- Shell extension seam (append-only; renders nothing while empty). -->
        <SlotHost slot="topbar.actions" />
      </div>
      <!-- Window controls are a direct `.topbar` child, absolutely pinned to the
           top-right corner. Keeping them inside `.topbar-right` made them a grid
           item on `.topbar--chat`, so they moved and wrapped with the app cluster
           and could fall outside the bar. Out of flow, they stay put on every
           route and at every viewport width. -->
      <TitleBarControls />
    </header>
    <div
      class="app-workspace"
      :style="{ '--workbench-inset-right': `${workbenchInsetRight}px` }"
    >
      <main
        class="content"
        :class="{ 'content--chat': isChatRoute }"
        :data-skin="skinId || undefined"
        :data-skin-variant="variants || undefined"
        id="content"
      >
        <!-- S5: 宿主（Python 侧车）退出证据条。放在内容区第一个子节点 → 占位下推，
             不覆盖任何面板；App.vue 持有唯一实例（provide/inject），跨路由存活。 -->
        <HostExitCard class="app-host-exit" />
        <AppRouterView />
      </main>
      <AppWorkbench
        :enabled="appStore.features.artifactWorkbench === true"
        :route-active="isChatRoute"
        :session-id="currentSessionKey"
        :modal-blocked="workbenchModalBlocked"
      />
      <ArtifactImageLightbox />
      <PaperGrain v-if="appStore.theme === 'ink'" />
      <!-- 鲸鱼娘桌面桌宠（可拖拽；设置 → 宠物 开关）。
           悬浮窗路由自身不挂页内桌宠：那里已有独立的 3D 舞台，
           再挂一个（被 CSS 藏成 0×0）会白开第二个 WebGL 上下文与动画循环。 -->
      <DesktopPet v-if="!isPetOverlayRoute" />
    </div>
    <AppStatusBar
      v-if="!isPetOverlayRoute"
      :project="statusProject.label"
      :project-path="statusProject.path"
      :mode="t('appStatus.localMode')"
      :branch="statusBranch || null"
      :detached="statusDetached"
      :connection-label="connectionStateLabel"
      :connection-tone="gatewayReadiness"
    />
  </div>

  <!-- Mobile bottom tab bar (<=768px only; hides while the keyboard is up):
       Chat, Sessions, Overview, then More for the flat drawer containing
       Sessions / Overview / Skills & Channels / Cron and Settings. -->
  <MobileTabBar
    :is-chat-active="isNavActive('/chat')"
    :is-sessions-active="isNavActive('/sessions')"
    :is-overview-active="isOverviewNavActive"
    :is-more-active="isMobileMoreActive"
    :is-keyboard-open="mobileKeyboardOpen"
    :inert="appStore.sidebarOpen && isSidebarDrawer"
    :approval-count="appStore.approvalCount"
    @nav="handleNavClick"
    @more="openSidebarDrawer"
  />

  <ToastHost />

  <ConfirmModal />

  <ProjectWorkspaceDialogs ref="projectWorkspaceDialogsRef" :session-key="currentSessionKey" />

  <ProjectWorkspaceEditDialog
    v-if="rpcStore.canManageProjectWorkspaces"
    :open="Boolean(editingProject)"
    :initial-name="editingProject?.name || ''"
    :path="editingProject?.path || ''"
    @close="editingProjectId = ''"
    @save="onProjectRename"
  />

  <UpdateBanner />
  <UpdateDialog />

  <OnboardingOverlay />

  <!-- Music-driven full-interface ambience: frosts the whole workstation into
       light frosted glass + aurora while background music plays. Pointer-
       transparent; mounted once, app-wide. -->
  <AmbienceLayer />

  <BootOrchestra />

  <!-- Single app-wide announcer for the pending-approval count. The nav badge
       and topbar pill stay silent (no double-announce); this region carries the
       only spoken update when the count changes. -->
  <p class="app-approval-live" aria-live="polite" role="status">{{ approvalAnnouncement }}</p>

  <!-- 应用级右键菜单：桌面端不再弹 Chromium 默认菜单（重新加载/另存为/检查） -->
  <AppContextMenu />

  <!-- Shell overlay seam: contributions that own a corner of the screen rather
       than a row in a bar (the Attention Shelf lives here). Renders nothing at
       all while the slot is empty, so this line cannot shift the layout. -->
  <SlotHost slot="shell.overlay" />
</template>

<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { initActiveMode, disposeActiveMode } from '@/composables/useActiveMode'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { routeTitle } from './router'
import { whenI18nReady } from '@/i18n'
import { getPlatform } from '@/platform'
import { useAppStore, type ThemeMode, type PendingApproval } from './stores/app'
import { useRpcStore } from './stores/rpc'
import { useWorkbenchStore } from '@/workbench/store'
import { workbenchEffectiveWidth, workbenchLayoutMode } from '@/workbench/layout'
import { readAuthToken } from '@/views/chat/read-auth-token'
import { staticAssetUrl } from '@/utils/staticAsset'
import { restoreStoredWallpaper } from '@/utils/wallpaper'
import MargueriteSky from '@/components/MargueriteSky.vue'
import AppStatusBar from '@/components/AppStatusBar.vue'
// 3D 山脊是重资源（three.js）：整包懒加载，只有切到「玛格丽特 · 星夜」且 WebGL 可用时才拉
const MargueriteRidge = defineAsyncComponent(() => import('@/components/marguerite/MargueriteRidge.vue'))
import GlassBackdrop from '@/components/GlassBackdrop.vue'
import { ambience } from '@/composables/useAmbience'
import { initGlassScene } from '@/composables/useGlassScene'
import { initGlassParams } from '@/composables/useGlassLab'
import { restorePetOverlayPreference } from '@/composables/usePetOverlay'
import { provideGatewayReadiness } from './composables/useGatewayReadiness'
import { useWorkspaceStatus } from './composables/sessions/useWorkspaceStatus'
import { provideHostExitEvidence } from './composables/useHostExitEvidence'
import {
  arrangeSidebarSections,
  useSessions,
  type SessionItem,
  type SidebarSection,
  type SidebarSectionRow,
} from './composables/useSessions'
import Icon from './components/Icon.vue'
import AppRouterView from './components/AppRouterView.vue'
import MobileTabBar from './components/MobileTabBar.vue'
import ThemeMenu from './components/ThemeMenu.vue'
import ProjectWorkspaceDialogs from './components/ProjectWorkspaceDialogs.vue'
import ToastHost from './components/ToastHost.vue'
import AppContextMenu from './components/AppContextMenu.vue'
import ConfirmModal from './components/ConfirmModal.vue'
import ProjectWorkspaceEditDialog from './components/ProjectWorkspaceEditDialog.vue'
import UpdateBanner from './components/UpdateBanner.vue'
import UpdateDialog from './components/UpdateDialog.vue'
import OnboardingOverlay from './components/OnboardingOverlay.vue'
import BootOrchestra from './components/boot/BootOrchestra.vue'
import DesktopUpdateIndicator from './components/DesktopUpdateIndicator.vue'
import SidebarConversations from './components/SidebarConversations.vue'
import SidebarSetupBanner from './components/SidebarSetupBanner.vue'
import HostExitCard from './components/HostExitCard.vue'
import SlotHost from './shell/slots/SlotHost.vue'
import SidebarResizer from './components/SidebarResizer.vue'
import CommandPalette from './components/CommandPalette.vue'
import FileQuickOpen from './components/FileQuickOpen.vue'
import LanguageSwitcher from './components/LanguageSwitcher.vue'
import BgmControl from './components/BgmControl.vue'
import AmbienceLayer from './components/AmbienceLayer.vue'
import ArtifactImageLightbox from './components/chat/ArtifactImageLightbox.vue'
import AppWorkbench from './components/workbench/AppWorkbench.vue'
import TitleBarControls from './components/shell/TitleBarControls.vue'
import { useBgm } from './composables/useBgm'
import { useSidebarLayout } from './composables/useSidebarLayout'
import { useDocumentEvent } from './composables/useDocumentEvent'
import { provideArtifactImageLightbox } from './composables/chat/useArtifactImageLightbox'
import { useAgentOptions } from './composables/useAgentOptions'
import { useSessionListSubscription } from './composables/useSessionListSubscription'
import { useSessionTaskAttention } from './composables/useSessionTaskAttention'
import { useToasts } from './composables/useToasts'
import { readStoredSessionKeys } from './utils/sidebarStorage'
import PaperGrain from '@/components/ink/PaperGrain.vue'
import DesktopPet from '@/components/pet/DesktopPet.vue'
import SidebarPetModule from '@/components/pet/SidebarPetModule.vue'
import { useConfirm } from './composables/useConfirm'
import { useProjectWorkspaces } from './composables/useProjectWorkspaces'
import { useFreshTaskDraft } from './composables/useFreshTaskDraft'
import { useNavigation } from './app/useNavigation'
import { installInternalLinkGuard } from './app/useInternalLinkGuard'
import ModeBadge from '@/components/ModeBadge.vue'
import { useSurfaceSkin } from './themes/useSurfaceSkin'
import { normalizeAgentId } from './utils/chat/sessionKeys'
import { installSessionNavigationDiagConsole, recordSessionNavigationDiag } from './utils/chat/sessionNavigationDiag'
import type { RpcEventHandler } from '@/lib/rpc'
import { isMacPlatform } from './utils/browser'
import { useShortcutsStore } from './stores/shortcuts'
import { bindingMatches, formatBinding } from './utils/keychord'
import { SIDEBAR_MIN_WIDTH, type SidebarWidthPreference } from './utils/sidebarLayout'
import {
  dispatchLocalSessionReset,
  dispatchLocalSessionsDeleted,
  localSessionResetDetail,
  localSessionsDeletedDetail,
  LOCAL_SESSION_RESET_EVENT,
  LOCAL_SESSIONS_DELETED_EVENT,
} from './utils/sessionSync'
import { activeTaskWasDeletedWithProjectHistory } from './utils/projectHistory'
import {
  optionalSessionRpcAllowed,
  optionalSessionRpcCallOptions,
} from './composables/chat/sessionBootstrapAdmission'
import { markCronFinishNotified } from './utils/cron/notifications'

const appStore = useAppStore()
const rpcStore = useRpcStore()
const workbenchStore = useWorkbenchStore()

/**
 * 工作台占用的右侧宽度。
 *
 * Chat 的右侧栏是 `position: fixed; right: 0`，而工作台是 `.app-workspace` 的
 * flex 子项。两者同时打开时会**重叠**：实测右侧栏（z-index 95）盖住工作台标题栏，
 * 连「关闭面板」的 X 都点不到。这里把工作台实际宽度作为 CSS 变量下发，
 * 右侧栏与它的折叠按钮据此左移，二者各占其位。
 */
const layoutViewportWidth = ref(typeof window === 'undefined' ? 0 : window.innerWidth)
const workbenchInsetRight = computed(() => {
  if (!workbenchStore.expanded || !workbenchStore.activeItem) return 0
  const mode = workbenchLayoutMode({ availableWidth: layoutViewportWidth.value })
  if (mode !== 'split') return 0
  return workbenchEffectiveWidth(
    workbenchStore.widthPreference,
    mode,
    layoutViewportWidth.value,
  )
})
function syncLayoutViewport() {
  layoutViewportWidth.value = window.innerWidth
}
// 星夜天空画布只服务 `marguerite-night`；用 resolvedTheme 而不是 theme，
// 这样 'system' 落到 light/dark 时不会误挂载。
const skyActive = computed(() => appStore.resolvedTheme === 'marguerite-night')
// 3D 山脊只在「玛格丽特 · 星夜」且 WebGL 可用时挂载；否则退回 Canvas 星空（诚实降级）。
const webglOk = ref(false)
const ridgeActive = computed(() => skyActive.value && webglOk.value)

/** 一次性 WebGL 探测：拿不到上下文就退回 Canvas 星空，不抛错、不 import three。 */
function detectWebGL(): boolean {
  try {
    const canvas = document.createElement('canvas')
    return Boolean(canvas.getContext('webgl2') || canvas.getContext('webgl'))
  } catch {
    return false
  }
}
// 液态玻璃背景层只服务 `liquid-glass`；同样用 resolvedTheme，避免 system 误挂载。
const glassActive = computed(() => appStore.resolvedTheme === 'liquid-glass')
const shortcutsStore = useShortcutsStore()
const artifactImageLightbox = provideArtifactImageLightbox()
const { t } = useI18n()
const $route = useRoute()

// 桌宠悬浮窗（Tauri 独立窗口，360×420 透明）：只渲染宠物本身，不带任何外壳。
// 真机缺陷（2026-10-23）：该窗口宽度落入移动断点，顶栏与底部 tabbar 会盖在宠物上。
// 用 `<html data-surface="pet-overlay">` 作为单一开关，样式集中在 base.css，
// 避免在 App.vue 十几处外壳节点上逐个加 v-if（漏一个就是一块残影）。
const isPetOverlayRoute = computed(() => $route.meta?.viewKey === 'pet-overlay')
// P0-4：任务悬浮球是同一类无壳窗口，复用同一 data-surface 开关隐藏外壳。
const overlaySurface = computed(() => {
  const viewKey = $route.meta?.viewKey
  return viewKey === 'pet-overlay' || viewKey === 'task-overlay' ? String(viewKey) : ''
})
watch(
  overlaySurface,
  (surface) => {
    const html = document.documentElement
    if (surface) html.dataset.surface = surface
    else if (html.dataset.surface === 'pet-overlay' || html.dataset.surface === 'task-overlay') {
      delete html.dataset.surface
    }
  },
  { immediate: true },
)

const sidebarRef = ref<HTMLElement | null>(null)
const sidebarDockToggleRef = ref<HTMLButtonElement | null>(null)
const topbarSidebarToggleRef = ref<HTMLButtonElement | null>(null)
type SidebarResizerHandle = { cancel: () => boolean }
const sidebarResizerRef = ref<SidebarResizerHandle | null>(null)

const {
  mode: sidebarLayoutMode,
  dynamicMax: sidebarDynamicMaximum,
  effectiveWidth: sidebarEffectiveWidth,
} = useSidebarLayout()
const isSidebarDrawer = computed(() => sidebarLayoutMode.value === 'drawer')
const isSidebarResizable = computed(() => sidebarLayoutMode.value === 'resizable')
const sidebarResizeActive = ref(false)

function setSidebarCssWidth(width: number) {
  if (!Number.isFinite(width)) return
  document.getElementById('app')?.style.setProperty('--sidebar-width', `${Math.round(width)}px`)
}

// Persisted/pre-set changes are infrequent. Pointer previews bypass App's
// reactive tree and write the same root custom property directly once per rAF.
watch(sidebarEffectiveWidth, width => {
  if (!sidebarResizeActive.value) setSidebarCssWidth(width)
}, { immediate: true })

// `.chat-header` reserves the topbar's app cluster (connection pill + language
// switcher + theme menu) in `padding-right: var(--topbar-right-reserve)`, and
// that comment already promised App publishes the live value. Do it: the width
// tracks the connection state label and the locale's language label, so a
// hard-coded constant would either clip the chat actions or waste the row. The
// CSS fallback (224px) only covers the frames before measurement.
const topbarRightRef = ref<HTMLElement | null>(null)
let topbarRightObserver: ResizeObserver | null = null

function publishTopbarRightReserve(width: number) {
  if (!Number.isFinite(width) || width <= 0) return
  // +2px absorbs sub-pixel rounding so the action group never touches the pill.
  document.getElementById('app')?.style.setProperty('--topbar-right-reserve', `${Math.round(width) + 2}px`)
}

function observeTopbarRight() {
  const el = topbarRightRef.value
  if (!el || typeof ResizeObserver === 'undefined') return
  publishTopbarRightReserve(el.getBoundingClientRect().width)
  topbarRightObserver?.disconnect()
  topbarRightObserver = new ResizeObserver(entries => {
    const entry = entries[0]
    publishTopbarRightReserve(entry?.contentRect.width ?? el.getBoundingClientRect().width)
  })
  topbarRightObserver.observe(el)
}

interface DeleteSessionsResponse {
  deleted?: string[]
  errors?: string[]
}

const APP_SESSION_SYNC_SOURCE = 'app-sidebar'

// Localized connection-state label for the topbar pill and its tooltip. The
// unified readiness merges WS transport state with gateway business readiness
// (onboarding.status), so the pill shows "Setup needed" (yellow) when the
// WebSocket is up but no API key is configured — instead of a misleading green
// "Connected" that sends users into a failing first chat turn.
// App.vue is the single always-mounted owner of the unified readiness state;
// it publishes the one instance for ModeHintBar and any other consumer via
// provide/inject (string key), so every widget reads the SAME computed and can
// never disagree in the same frame.
const { readiness: gatewayReadiness } = provideGatewayReadiness()
// S5: single owner of the shared host-exit evidence state. Must be called in the
// always-mounted root for the same reason as `provideGatewayReadiness` above —
// see useHostExitEvidence.ts for the full rationale.
provideHostExitEvidence()
const connectionStateLabel = computed(() => t(`chrome.connectionState.${gatewayReadiness.value}`))
const router = useRouter()
// 内部裸锚点守卫：任何遗留的 `<a href="/control/...">` 都不该触发整页导航。
// 安装点在 setup 期（早于视图挂载），注销在 root 卸载时。
const removeInternalLinkGuard = installInternalLinkGuard(router)

// afterEach only fires on navigation, and it deliberately skips writing while the
// first locale is still resolving (see applyRouteTitle) — so re-apply the tab title
// on a same-route language switch, and once more when the first locale settles so
// the cold-start title is not left at the index.html placeholder.
watch(() => appStore.locale, () => applyRouteDocumentTitle())
void whenI18nReady().then(() => { applyRouteDocumentTitle() })
const { allSessions, sessionListError, isLoading, loadSessions, applySessionRunStatus, forgetSessions } = useSessions(
  optionalSessionRpcCallOptions,
)
const { bottomRoutes, workNav, groupedNav, collapsedGroups, toggleGroup } = useNavigation()
// Axis-B: the active expressive skin for the routed content area (meta.skin).
const { skinId, variants } = useSurfaceSkin()
const { pushToast } = useToasts()
const { confirm } = useConfirm()
const projectWorkspaces = useProjectWorkspaces()
const freshTaskDraft = useFreshTaskDraft()
const projectWorkspaceDialogsRef = ref<InstanceType<typeof ProjectWorkspaceDialogs> | null>(null)
const editingProjectId = ref('')
const editingProject = computed(() =>
  editingProjectId.value
    ? projectWorkspaces.byId.value.get(editingProjectId.value) || null
    : null,
)
watch(
  () => rpcStore.canManageProjectWorkspaces,
  allowed => {
    if (allowed) {
      scheduleSessionRefresh()
      return
    }
    editingProjectId.value = ''
  },
)
// Feature-gated topbar music control; the singleton `enabled` ref is written by
// Settings → Appearance and the command palette.
const { enabled: bgmEnabled } = useBgm()
const webConfigEnabled = getPlatform().capabilities.hasWebConfig
// The main window runs undecorated in the Tauri shell, so the topbar needs a
// drag region. The CLI gateway serves the same bundle in a plain browser where
// no window exists — keep the topbar layout byte-identical there.
const isDesktopShell = (() => {
  if (typeof window === 'undefined') return false
  const scope = window as unknown as Record<string, unknown>
  return Boolean(scope.__TAURI__ || scope.__TAURI_INTERNALS__)
})()

interface AppCronRunFinishedPayload {
  jobId?: string
  jobName?: string
  runId?: string
  success?: boolean
}

let unsubscribeCronFinished: (() => void) | null = null

function handleCronRunFinished(payload: unknown) {
  if (!payload || typeof payload !== 'object') return
  const event = payload as AppCronRunFinishedPayload
  const runId = typeof event.runId === 'string' ? event.runId : ''
  const jobName = event.jobName?.trim() || t('cronSkills.jobs.unnamedTask')
  markCronFinishNotified(runId)
  if (event.success === false) {
    pushToast(t('cronSkills.jobs.toastBackgroundFailed', { name: jobName }), {
      tone: 'danger',
      duration: 9_000,
    })
    return
  }
  pushToast(t('cronSkills.jobs.toastBackgroundComplete', { name: jobName }), {
    tone: 'ok',
    duration: 7_000,
  })
}

installSessionNavigationDiagConsole()

// Shared agents.list state + fetch (singleton) for sidebar session metadata.
const { agents, loadAgents } = useAgentOptions(optionalSessionRpcCallOptions)
const mobileKeyboardOpen = ref(false)
const commandPaletteOpen = ref(false)
// Ctrl/⌘+P quick-open overlay (see components/FileQuickOpen.vue).
const fileQuickOpenOpen = ref(false)
const localChatSessions = ref<Record<string, { effectiveAgentId: string; title: string; updatedAt: number }>>({})
// Pending optimistic renames, keyed by session key; cleared after the next list
// reload returns the backend's canonical title.
const renameOverrides = ref<Record<string, string>>({})

const brandMarkUrl = computed(() => {
  // 2026-10-23：改用 Seedream 5.0 Pro 定制的专属 Logo（已裁掉生成水印、
  // 缩到 256² 供 24px 侧栏使用）。路径解析走统一工具，避免 DEV/PROD 两套前缀
  // 各写一遍（旧实现在 PROD 下拼错就会被 SPA 回退成 index.html → 图标空白）。
  return staticAssetUrl('brand/logo-mark.png')
})

// Display chords track the configurable bindings so the rail hint, the New chat
// badge, and the palette never drift from what the handler actually honours. A
// disabled shortcut yields an empty hint (the New chat badge then hides).
const isMac = isMacPlatform()
const commandPaletteHint = computed(() =>
  formatBinding(shortcutsStore.effectiveBinding('command-palette'), isMac))
const newChatHint = computed(() =>
  formatBinding(shortcutsStore.effectiveBinding('new-chat'), isMac))
const sidebarToggleBinding = computed(() => shortcutsStore.effectiveBinding('toggle-sidebar'))
const sidebarToggleHint = computed(() => formatBinding(sidebarToggleBinding.value, isMac))
const sidebarToggleAriaShortcut = computed(() => {
  const binding = sidebarToggleBinding.value
  if (!binding) return undefined
  const parts: string[] = []
  if (binding.primary) parts.push(isMac ? 'Meta' : 'Control')
  if (binding.alt) parts.push('Alt')
  if (binding.shift) parts.push('Shift')
  parts.push(binding.key.length === 1 ? binding.key.toUpperCase() : binding.key)
  return parts.join('+')
})

const themeMenuOpen = ref(false)
const themeMenuRef = ref<InstanceType<typeof ThemeMenu> | null>(null)

// "More themes…": the full theme list lives in Settings → Appearance. Close the
// menu and deep-link straight to that section.
function onOpenMoreThemes() {
  handleNavClick()
  router.push('/settings/appearance')
}

function onThemePick(mode: ThemeMode) {
  appStore.setTheme(mode)
}

// Current session key from ChatView via URL
const currentSessionKey = computed(() => {
  return ($route.query.session as string) || ''
})
const sessionTaskAttention = useSessionTaskAttention()

function currentSessionIsVisible(): boolean {
  return (
    $route.path === '/chat'
    && document.visibilityState === 'visible'
    && document.hasFocus()
  )
}

function markCurrentSessionReadIfVisible() {
  const sessionKey = currentSessionKey.value
  if (sessionKey && currentSessionIsVisible()) {
    sessionTaskAttention.markRead(sessionKey)
  }
}

watch(currentSessionKey, markCurrentSessionReadIfVisible, {
  flush: 'sync',
  immediate: true,
})

// Chat layout applies to both the session view and the draft route.
const isChatRoute = computed(() => $route.path === '/chat' || $route.path === '/chat/new')
const activeProjectDraftId = computed(() =>
  $route.path === '/chat/new' ? String($route.query.project || '') : '',
)
const activeProjectDraftKey = computed(() => {
  const workspaceId = activeProjectDraftId.value
  if (!workspaceId) return ''
  const request = freshTaskDraft.request.value
  const requestId = request?.workspaceId === workspaceId ? request.id : 0
  return `draft:project:${workspaceId}:${requestId}`
})
const sidebarCurrentKey = computed(() =>
  currentSessionKey.value || activeProjectDraftKey.value,
)

// ── Bottom status bar: project / mode / branch / connection / version ──
const currentSessionItem = computed(() => (
  currentSessionKey.value
    ? allSessions.value.find(item => item.key === currentSessionKey.value) || null
    : null
))

function pathBaseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '')
  const parts = trimmed.split(/[\\/]/)
  return parts[parts.length - 1] || trimmed
}

const statusProject = computed(() => {
  const session = currentSessionItem.value
  if (session?.workspaceLabel) {
    return { label: session.workspaceLabel, path: session.workspace || '' }
  }
  if (session?.workspace) {
    return { label: pathBaseName(session.workspace), path: session.workspace }
  }
  const draftId = activeProjectDraftId.value
  const draft = draftId ? projectWorkspaces.byId.value.get(draftId) : null
  if (draft) return { label: draft.name, path: draft.path }
  return { label: t('appStatus.noProject'), path: '' }
})

const statusWorkspace = useWorkspaceStatus()
watch(
  () => statusProject.value.path,
  path => { void statusWorkspace.load(path) },
  { immediate: true },
)
const statusBranch = computed(() => statusWorkspace.status.value?.git?.branch || '')
const statusDetached = computed(() => statusWorkspace.status.value?.git?.detached === true)

watch(
  [
    currentSessionKey,
    isChatRoute,
    () => artifactImageLightbox.request.value?.sessionKey || '',
  ],
  ([sessionKey, chatRouteActive]) => {
    const request = artifactImageLightbox.request.value
    if (request && (!chatRouteActive || request.sessionKey !== sessionKey)) {
      artifactImageLightbox.close()
    }
  },
  { flush: 'sync' },
)

// The Settings overlay (route-mounted dialog) is open on these routes. It owns
// its own Escape/focus, so App-level keyboard shortcuts defer to it. Both web
// and desktop mount the same overlay now (webConfigEnabled is true on both).
const settingsOverlayOpen = computed(() =>
  webConfigEnabled && ($route.name === 'settings' || $route.name === 'settings-section'))
const workbenchModalBlocked = computed(() =>
  commandPaletteOpen.value
  || fileQuickOpenOpen.value
  || themeMenuOpen.value
  || settingsOverlayOpen.value
  || (appStore.sidebarOpen && isSidebarDrawer.value))

const contractDebugEnabled = computed(() => appStore.features.contractDebug === true)

function isNavActive(path: string): boolean {
  if (path === '/chat') return isChatRoute.value
  return $route.path === path
}

// Overview owns the Status/Usage hub plus its diagnostic Logs route, while
// Skills fronts the Skills/Channels hub. Keep those active families disjoint so
// diagnostic routes never light an unrelated primary destination.
const OVERVIEW_NAV_PATHS = new Set(['/overview', '/usage', '/logs'])
const SKILLS_CHANNELS_HUB_PATHS = new Set(['/skills', '/channels'])
const MOBILE_MORE_PATHS = new Set(['/skills', '/channels', '/cron'])
const isOverviewNavActive = computed(() => OVERVIEW_NAV_PATHS.has($route.path))
const isSkillsChannelsHubActive = computed(() => SKILLS_CHANNELS_HUB_PATHS.has($route.path))
const isMobileMoreActive = computed(() =>
  appStore.sidebarOpen || MOBILE_MORE_PATHS.has($route.path))

function isPrimaryNavActive(path: string): boolean {
  if (path === '/usage') return isOverviewNavActive.value
  if (path === '/skills') return isSkillsChannelsHubActive.value
  return isNavActive(path)
}

function agentDisplayName(agentId: string): string {
  const agent = agents.value.find(a => a.id === agentId)
  // 兜底不能硬编码英文：agents.list 失败/未回来时，中文界面里会永久显示
  // "Main Agent"。`chat.mainAgent` 是既有文案键（六语言齐备）。
  return agent?.name || (agentId === 'main' ? t('chat.mainAgent') : agentId)
}

// Raw session keys (agent:…:…) and bare UUIDs must never render in the sidebar.
const RAW_SESSION_KEY_PATTERN = /\bagent:[a-z0-9_-]+:[a-z0-9_-]+:/i
const UUID_PATTERN = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i

function looksLikeRawSessionId(value: string): boolean {
  return RAW_SESSION_KEY_PATTERN.test(value) || UUID_PATTERN.test(value) || /^(agent|cron):/i.test(value)
}

function sidebarConversationTitle(item: SessionItem): string {
  for (const candidate of [item.title, item.subtitle, item.groupLabel]) {
    const text = String(candidate || '').trim()
    if (text && !looksLikeRawSessionId(text)) return text
  }
  return t('shared.sidebar.untitledTask')
}

// A draft / current-session row the backend list does not yet carry. The
// sidebar arranger reads only a handful of fields off the SessionItem, so a
// synthetic chat row carries just those plus a stub `raw` (no parent → root).
function syntheticChatSession(
  key: string,
  effectiveAgentId: string,
  title: string,
  updatedAt: number,
  project?: {
    id: string
    name: string
    path: string
    provisional?: boolean
  },
): SessionItem {
  return {
    key,
    title,
    subtitle: '',
    groupLabel: normalizeAgentId(effectiveAgentId),
    workspace: project?.path,
    workspaceId: project?.id,
    workspaceLabel: project?.name,
    workspaceDisplayPath: project?.path,
    effectiveAgentId,
    sessionKind: 'chat',
    surface: 'webchat',
    conversationKind: 'direct',
    threadLabel: '',
    channelContext: null,
    status: 'idle',
    visualStatus: 'idle',
    runStatus: 'idle',
    runLabel: 'Idle',
    messageCount: null,
    updatedAt,
    interactive: true,
    provisional: project?.provisional,
    forkedFromParent: false,
    contractGaps: [],
    raw: { key },
  }
}

function optimisticProjectForSession(key: string) {
  const workspaceId = freshTaskDraft.materializedWorkspaceBySession.value[key]
  return workspaceId ? projectWorkspaces.byId.value.get(workspaceId) || null : null
}

function withOptimisticProjectBinding(item: SessionItem): SessionItem {
  if (item.workspaceId) return item
  const project = optimisticProjectForSession(item.key)
  if (!project) return item
  return {
    ...item,
    workspace: project.path,
    workspaceId: project.id,
    workspaceLabel: project.name,
    workspaceDisplayPath: project.path,
  }
}

// Sessions to arrange into the sidebar: the backend list plus the local draft
// and the current chat session when the list does not carry them yet (both
// injected as Chats so a brand-new conversation appears immediately).
const sidebarSessionItems = computed((): SessionItem[] => {
  const items: SessionItem[] = []
  const seen = new Set<string>()
  for (const item of allSessions.value) {
    if (!item.key || item.key === 'unknown') continue
    seen.add(item.key)
    items.push(withOptimisticProjectBinding(item))
  }
  for (const [key, local] of Object.entries(localChatSessions.value)) {
    if (seen.has(key)) continue
    seen.add(key)
    const project = optimisticProjectForSession(key) || undefined
    items.push(syntheticChatSession(
      key,
      local.effectiveAgentId,
      local.title || t('chrome.newChat'),
      local.updatedAt,
      project,
    ))
  }
  const draftWorkspaceId = activeProjectDraftId.value
  const draftKey = activeProjectDraftKey.value
  const draftProject = draftWorkspaceId
    ? projectWorkspaces.byId.value.get(draftWorkspaceId)
    : null
  if (draftKey && draftProject && !seen.has(draftKey)) {
    seen.add(draftKey)
    items.push(syntheticChatSession(
      draftKey,
      'main',
      t('chrome.newTask'),
      Date.now(),
      {
        id: draftProject.id,
        name: draftProject.name,
        path: draftProject.path,
        provisional: true,
      },
    ))
  }
  const current = currentSessionKey.value
  if (current && !seen.has(current)) {
    const currentAgentId = normalizeAgentId(current.split(':')[1] || 'main')
    const project = optimisticProjectForSession(current) || undefined
    items.push(syntheticChatSession(
      current,
      currentAgentId,
      t('shared.sidebar.currentTask'),
      Date.now(),
      project,
    ))
  }
  return items
})

watch(allSessions, sessions => {
  for (const item of sessions) {
    if (!item.key || !item.workspaceId) continue
    freshTaskDraft.confirmMaterializedProjectTask(item.key, item.workspaceId)
  }
})

const SIDEBAR_SESSION_ORDER_KEY = 'Marguerite-sidebar-session-order-v1'
const SIDEBAR_PINNED_SESSIONS_KEY = 'Marguerite-sidebar-pinned-sessions-v1'

function writeStoredSessionKeys(storageKey: string, keys: readonly string[]) {
  try {
    localStorage.setItem(storageKey, JSON.stringify(keys))
  } catch {
    // Storage can be unavailable in restricted browser contexts.
  }
}

const sidebarSessionOrder = ref<string[]>(readStoredSessionKeys(SIDEBAR_SESSION_ORDER_KEY))
const sidebarPinnedSessionKeys = ref<string[]>(readStoredSessionKeys(SIDEBAR_PINNED_SESSIONS_KEY))

// Collapsible family sections (Chats / Channels / Automations). Row titles and
// agent names are resolved here so the raw-session-id scrub and the display-name
// lookup stay in App.vue; subagents indent under their parent via the helper.
const sidebarSections = computed((): SidebarSection[] => {
  const byKey = new Map(sidebarSessionItems.value.map(item => [item.key, item]))
  return arrangeSidebarSections(
    sidebarSessionItems.value,
    rpcStore.canManageProjectWorkspaces && projectWorkspaces.hasLoaded.value
      ? projectWorkspaces.workspaces.value
      : undefined,
    sidebarSessionOrder.value,
    sidebarPinnedSessionKeys.value,
  ).map(section => ({
    ...section,
    rows: section.rows.map((row): SidebarSectionRow => {
      if (row.rowKind !== 'session') return { ...row, agentName: '' }
      const source = byKey.get(row.key)
      const title = renameOverrides.value[row.key]
        || (source ? sidebarConversationTitle(source) : row.title)
      return {
        ...row,
        title,
        agentName: agentDisplayName(normalizeAgentId(row.effectiveAgentId)),
        taskAttention: sessionTaskAttention.attentionFor(row.key, row.runStatus),
      }
    }),
  }))
})

function onReorderSidebarSession(payload: {
  draggedKey: string
  targetKey: string
  position: 'before' | 'after'
}) {
  const orderedKeys = sidebarSections.value
    .flatMap(section => section.rows)
    .filter(row => row.rowKind === 'session' && row.sessionKind === 'chat' && !row.provisional)
    .map(row => row.key)
  const from = orderedKeys.indexOf(payload.draggedKey)
  if (from < 0 || !orderedKeys.includes(payload.targetKey) || payload.draggedKey === payload.targetKey) return

  orderedKeys.splice(from, 1)
  const target = orderedKeys.indexOf(payload.targetKey)
  orderedKeys.splice(payload.position === 'after' ? target + 1 : target, 0, payload.draggedKey)
  sidebarSessionOrder.value = orderedKeys
  writeStoredSessionKeys(SIDEBAR_SESSION_ORDER_KEY, orderedKeys)
}

function onPinSidebarSession(payload: { key: string; pinned: boolean }) {
  const pinned = new Set(sidebarPinnedSessionKeys.value)
  if (payload.pinned) pinned.add(payload.key)
  else pinned.delete(payload.key)
  sidebarPinnedSessionKeys.value = [...pinned]
  writeStoredSessionKeys(SIDEBAR_PINNED_SESSIONS_KEY, sidebarPinnedSessionKeys.value)

  if (!payload.pinned) return
  const currentOrder = sidebarSections.value
    .flatMap(section => section.rows)
    .filter(row => row.rowKind === 'session' && row.sessionKind === 'chat' && !row.provisional)
    .map(row => row.key)
    .filter(key => key !== payload.key)
  currentOrder.unshift(payload.key)
  sidebarSessionOrder.value = currentOrder
  writeStoredSessionKeys(SIDEBAR_SESSION_ORDER_KEY, currentOrder)
}

let sessionRefreshTimer: ReturnType<typeof setTimeout> | null = null
let sessionStatusTimer: number | null = null
let sidebarRefreshPending = false
let sidebarLoadPromise: Promise<void> | null = null
let appAutomaticRpcMounted = false
let appAutomaticRpcStarted = false

// Hide the bottom tab bar while the on-screen keyboard owns the bottom edge.
// A visual-viewport shrink well beyond browser-chrome changes (>140px) is the
// simplest cross-platform signal; per-input focus tracking was considered and
// dropped as fragile. When the heuristic misses, the bar just stays visible.
function syncMobileKeyboard() {
  const viewport = window.visualViewport
  if (!viewport) return
  mobileKeyboardOpen.value = window.innerWidth <= 768 && window.innerHeight - viewport.height > 140
}

type SidebarToggleSource = 'sidebar-button' | 'topbar-button' | 'shortcut'

function toggleDock(source: SidebarToggleSource) {
  sidebarResizerRef.value?.cancel()
  const wasOpen = appStore.sidebarOpen
  const focusWasInsideSidebar = Boolean(
    sidebarRef.value && document.activeElement && sidebarRef.value.contains(document.activeElement),
  )
  const focusWasOnResizer = document.activeElement instanceof HTMLElement
    && document.activeElement.matches('.sidebar-resizer')
  appStore.toggleSidebar()
  if (!wasOpen && (source === 'topbar-button' || isSidebarDrawer.value)) {
    void nextTick(() => sidebarDockToggleRef.value?.focus())
  } else if (wasOpen && (source === 'sidebar-button' || focusWasInsideSidebar || focusWasOnResizer)) {
    void nextTick(() => topbarSidebarToggleRef.value?.focus())
  }
}

function handleNavClick() {
  if (isSidebarDrawer.value && appStore.sidebarOpen) {
    closeSidebarDrawer()
  }
}

// ── Icon-rail mode ────────────────────────────────────────────────────────
// Narrow icon-only sidebar. Docked-only (drawer mode keeps the full column)
// and driven by the persisted `appStore.sidebarIconRail` preference.
const iconRailActive = computed(
  () => appStore.sidebarIconRail && appStore.sidebarOpen && !isSidebarDrawer.value,
)

/** Group anchors in icon-rail mode: one flyout open at a time. */
const openIconGroupId = ref<string | null>(null)

function toggleIconGroupFlyout(id: string) {
  openIconGroupId.value = openIconGroupId.value === id ? null : id
}

function closeIconGroupFlyout() {
  openIconGroupId.value = null
}

/** Leave only when the mouse isn't moving onto the flyout of the same group. */
function onIconAnchorLeave(id: string) {
  if (openIconGroupId.value === id) openIconGroupId.value = null
}

/**
 * Section anchors carry no per-group metadata in the route table, so the
 * glyphs are mapped here (ids come from NavGroupId in router/nav.ts).
 */
const GROUP_ICONS = {
  Work: 'listChecks',
  Operate: 'boxes',
  Observe: 'eye',
  Settings: 'settings',
  Extras: 'sparkles',
} as const

/** A group anchor reads active when any of its items matches the route. */
function isGroupNavActive(section: { items: Array<{ path: string }> }): boolean {
  return section.items.some(item => isPrimaryNavActive(item.path))
}

function openSidebarDrawer() {
  if (appStore.sidebarOpen) return
  toggleDock('topbar-button')
}

function closeSidebarDrawer() {
  if (!appStore.sidebarOpen || !isSidebarDrawer.value) return
  sidebarResizerRef.value?.cancel()
  appStore.setSidebarOpen(false)
  void nextTick(() => topbarSidebarToggleRef.value?.focus())
}

function handleSidebarResizeStart() {
  sidebarResizeActive.value = true
}

function applySidebarPreview(width: number) {
  setSidebarCssWidth(width)
}

function commitSidebarWidth(width: number) {
  const preference: SidebarWidthPreference = {
    version: 1,
    width,
    source: 'custom',
  }
  appStore.setSidebarWidthPreference(preference)
}

function resetSidebarWidth() {
  appStore.resetSidebarWidthPreference()
}

function collapseSidebarFromResize() {
  appStore.setSidebarOpen(false)
  // The collapse gesture never overwrites the saved preference. Reset the root
  // variable now so the next explicit open restores that preference immediately.
  setSidebarCssWidth(sidebarEffectiveWidth.value)
  void nextTick(() => topbarSidebarToggleRef.value?.focus())
}

function handleSidebarResizeEnd() {
  sidebarResizeActive.value = false
  setSidebarCssWidth(sidebarEffectiveWidth.value)
}

// Layout mode is a single state machine shared with Settings. Entering a drawer
// force-closes the persistent dock; returning to desktop intentionally leaves it
// closed until the user reopens it. Compact mode keeps the current open state.
watch(sidebarLayoutMode, (nextMode, previousMode) => {
  const focusWasOnResizer = document.activeElement instanceof HTMLElement
    && document.activeElement.matches('.sidebar-resizer')
  sidebarResizerRef.value?.cancel()
  sidebarResizeActive.value = false
  setSidebarCssWidth(sidebarEffectiveWidth.value)

  if (nextMode === 'drawer' && appStore.sidebarOpen) {
    const focusWasInsideSidebar = Boolean(
      sidebarRef.value && document.activeElement && sidebarRef.value.contains(document.activeElement),
    )
    appStore.setSidebarOpen(false)
    if (focusWasOnResizer || focusWasInsideSidebar) {
      void nextTick(() => topbarSidebarToggleRef.value?.focus())
    }
  } else if (previousMode === 'resizable' && focusWasOnResizer) {
    void nextTick(() => sidebarDockToggleRef.value?.focus())
  }
}, { immediate: true })

watch(sidebarDynamicMaximum, () => {
  if (!sidebarResizeActive.value) return
  sidebarResizerRef.value?.cancel()
  sidebarResizeActive.value = false
  setSidebarCssWidth(sidebarEffectiveWidth.value)
})

// Primary new-chat path: ordinary tasks always start against the default Agent.
// Explicit custom-Agent launches still receive their Agent-scoped session key
// from advanced Agent administration.
function openDefaultDraft() {
  freshTaskDraft.requestFreshTask('main')
  return router.push({ path: '/chat/new', query: { agent: 'main' } })
}

function startNewChatInstant() {
  handleNavClick()
  void maybeGuideProjectThenDraft()
}

// New-task path with project awareness: when a local owner can host project
// workspaces but none exist yet, guide the user to open/create a folder space
// before drafting. Once at least one project exists, keep instant drafting and
// let the sidebar's project entries drive project-bound tasks. Any workspace
// list failure falls back to an ordinary draft rather than blocking new chats.
async function maybeGuideProjectThenDraft() {
  if (!rpcStore.canChooseProject) {
    void openDefaultDraft()
    return
  }
  try {
    if (!projectWorkspaces.hasLoaded.value) {
      await projectWorkspaces.loadWorkspaces()
    }
    if (projectWorkspaces.workspaces.value.length === 0) {
      openProjectCreator()
      return
    }
  } catch {
    // Workspace listing unavailable; an ordinary draft stays usable.
  }
  void openDefaultDraft()
}

function startProjectTask(workspaceId: string) {
  if (!workspaceId || !rpcStore.canManageProjectWorkspaces) return
  handleNavClick()
  freshTaskDraft.requestFreshTask('main', workspaceId)
  void router.push({
    path: '/chat/new',
    query: { agent: 'main', project: workspaceId },
  })
}

// The create dialog now owns its own state machine; App just forwards the
// open intent (from the sidebar "new project" row and the new-chat guide).
function openProjectCreator() {
  projectWorkspaceDialogsRef.value?.openProjectCreator()
}

async function onProjectPin(payload: { workspaceId: string; pinned: boolean }) {
  if (!rpcStore.canManageProjectWorkspaces) return
  try {
    await projectWorkspaces.setPinned(payload.workspaceId, payload.pinned)
  } catch (err) {
    pushToast(t('workspaces.updateFailed', { error: errorMessage(err) }), { tone: 'danger' })
  }
}

function openProjectEditor(workspaceId: string) {
  if (!rpcStore.canManageProjectWorkspaces) return
  editingProjectId.value = workspaceId
}

async function onProjectRename(name: string) {
  if (!rpcStore.canManageProjectWorkspaces) return
  const workspaceId = editingProjectId.value
  if (!workspaceId) return
  try {
    await projectWorkspaces.renameWorkspace(workspaceId, name)
    editingProjectId.value = ''
  } catch (err) {
    pushToast(t('workspaces.updateFailed', { error: errorMessage(err) }), { tone: 'danger' })
  }
}

async function onProjectDeleteHistory(workspaceId: string) {
  if (!rpcStore.canManageProjectWorkspaces) return
  try {
    const result = await projectWorkspaces.deleteWorkspaceHistory(workspaceId)
    const leaveDeletedTask = activeTaskWasDeletedWithProjectHistory({
      workspaceId,
      currentSessionKey: currentSessionKey.value,
      sessions: allSessions.value,
      deletedSessionKeys: result.deletedSessionKeys,
    })
    sessionTaskAttention.removeMany(result.deletedSessionKeys)
    forgetSessions(result.deletedSessionKeys)
    await loadSessions()
    if (leaveDeletedTask) void openDefaultDraft()
    pushToast(t('workspaces.historyDeleted'), { tone: 'ok' })
  } catch (err) {
    pushToast(t('workspaces.deleteHistoryFailed', { error: errorMessage(err) }), { tone: 'danger' })
  }
}

async function onProjectRemove(workspaceId: string) {
  if (!rpcStore.canManageProjectWorkspaces) return
  const project = projectWorkspaces.byId.value.get(workspaceId)
  if (!project) return
  let affectedCronJobs = 0
  try {
    const jobs = await rpcStore.call<Array<{ workspaceId?: string }>>(
      'cron.list',
      {},
    )
    affectedCronJobs = (jobs || []).filter(
      job => job.workspaceId === workspaceId,
    ).length
  } catch {
    // The backend still enforces the pause atomically during removal.
  }
  const approved = await confirm({
    title: t('workspaces.removeTitle'),
    body: affectedCronJobs > 0
      ? t('workspaces.removeBodyWithCronJobs', {
          name: project.name,
          count: affectedCronJobs,
        })
      : t('workspaces.removeBody', { name: project.name }),
    primaryLabel: t('workspaces.removeConfirm'),
  })
  if (!approved) return
  try {
    await projectWorkspaces.removeWorkspace(workspaceId)
    if (editingProjectId.value === workspaceId) editingProjectId.value = ''
    if (
      $route.path === '/chat/new'
      && String($route.query.project || '') === workspaceId
    ) {
      freshTaskDraft.requestFreshTask('main')
      await router.replace({ path: '/chat/new', query: { agent: 'main' } })
    }
  } catch (err) {
    pushToast(t('workspaces.removeFailed', { error: errorMessage(err) }), { tone: 'danger' })
  }
}

// Command palette: ⌘K / Ctrl+K and the rail "Search / Go to…" row both open it.
// Its action commands route back through the existing handlers so behaviour stays
// single-sourced (new chat opens a draft, Settings reuses the footer path).
function openCommandPalette() {
  handleNavClick()
  commandPaletteOpen.value = true
}

// Quick-open hit: dismiss the modal, then deep-link the editor at the picked
// file. WorkspaceEditorView reads `?path=`, focuses the containing directory in
// the tree and opens the file in a tab.
function openQuickOpenFile(path: string) {
  fileQuickOpenOpen.value = false
  if (!path) return
  const root = statusProject.value.path
  void router.push({ path: '/editor', query: root ? { path, root } : { path } })
}

function onPaletteNewChat() {
  startNewChatInstant()
}

function onPaletteOpenSettings() {
  openSettings()
}

function onPaletteToggleTheme() {
  // Cycle the appearance mode through the same registry-driven order as the
  // topbar picker (every selectable value theme + system), so the palette
  // never resets a custom theme back to 'light'.
  appStore.cycleTheme()
}

function onPaletteSelectSession(key: string) {
  switchToSession(key, 'command_palette.select_session')
}

function switchToSession(key: string, source = 'app.switchToSession') {
  if (!key) return
  sessionTaskAttention.markRead(key)
  recordSessionNavigationDiag(source, {
    from: currentSessionKey.value,
    to: key,
  })
  router.push({ path: '/chat', query: { session: key } })
}

// Optimistic rename: show the new title immediately, then persist via
// sessions.patch (display_name is the top-precedence title) and reload so the
// backend's canonical title wins. The override clears once the reload lands.
async function onRenameSession({ key, title }: { key: string; title: string }) {
  const next = title.trim()
  if (!key || !next) return
  renameOverrides.value = { ...renameOverrides.value, [key]: next }
  const local = localChatSessions.value[key]
  if (local) localChatSessions.value[key] = { ...local, title: next }
  try {
    await rpcStore.call('sessions.patch', { key, displayName: next })
    pushToast(t('shared.sidebar.renameSessionDone'), { tone: 'ok' })
  } catch (err: unknown) {
    console.warn('[App] sessions.patch error:', errorMessage(err))
    pushToast(t('shared.sidebar.renameSessionFailed'), { tone: 'danger' })
  } finally {
    await loadSessions()
    const { [key]: _dropped, ...rest } = renameOverrides.value
    renameOverrides.value = rest
  }
}

function removeLocalSessions(keys: Set<string>) {
  if (keys.size === 0) return
  let next = localChatSessions.value
  let changed = false
  for (const key of keys) {
    if (!next[key]) continue
    const { [key]: _dropped, ...rest } = next
    next = rest
    changed = true
  }
  if (changed) localChatSessions.value = next
}

function handleLocalSessionsDeleted(event: Event) {
  const detail = localSessionsDeletedDetail(event)
  if (!detail || detail.source === APP_SESSION_SYNC_SOURCE) return
  const deleted = new Set(detail.keys)
  removeLocalSessions(deleted)
  forgetSessions(deleted)
  sessionTaskAttention.removeMany(deleted)
  appStore.removePendingApprovalsForSessions(deleted)
  scheduleSessionRefresh()
}

// 另一个窗口清空了会话：本窗口没有派发该事件，但依附那个回合的审批与未读
// 状态同样已经失效，且列表里的消息计数需要重新取一次。
function handleLocalSessionReset(event: Event) {
  const detail = localSessionResetDetail(event)
  if (!detail || detail.source === APP_SESSION_SYNC_SOURCE) return
  const reset = new Set(detail.keys)
  sessionTaskAttention.removeMany(reset)
  appStore.removePendingApprovalsForSessions(reset)
  scheduleSessionRefresh()
}

async function deleteSessions(keys: string[]): Promise<DeleteSessionsResponse | null> {
  const uniqueKeys = [...new Set(keys.map(key => key.trim()).filter(Boolean))]
  if (uniqueKeys.length === 0) return null
  try {
    return await rpcStore.call<DeleteSessionsResponse>('sessions.delete', { keys: uniqueKeys })
  } catch (err: unknown) {
    console.warn('[App] sessions.delete error:', errorMessage(err))
    return null
  }
}

// Delete sessions, then refresh the list. If the open session was deleted, drop
// into a fresh draft so the view does not linger on a session that no longer exists.
async function onBulkDeleteSessions(keys: string[]) {
  const uniqueKeys = [...new Set(keys.map(key => key.trim()).filter(Boolean))]
  if (uniqueKeys.length === 0) return
  const currentKey = currentSessionKey.value
  const wasCurrentDeleted = !!currentKey && uniqueKeys.includes(currentKey)
  const result = await deleteSessions(uniqueKeys)
  const deleted = new Set(result?.deleted || [])
  if (!result || deleted.size === 0) {
    console.warn('[App] sessions.delete reported failure:', result?.errors)
    pushToast(t('shared.sidebar.bulkDeleteFailed'), { tone: 'danger' })
    return
  }
  removeLocalSessions(deleted)
  forgetSessions(deleted)
  sessionTaskAttention.removeMany(deleted)
  appStore.removePendingApprovalsForSessions(deleted)
  dispatchLocalSessionsDeleted(deleted, APP_SESSION_SYNC_SOURCE)
  const failedCount = Math.max(0, uniqueKeys.length - deleted.size)
  pushToast(t('shared.sidebar.bulkDeleteDone', { count: deleted.size }), { tone: 'ok' })
  if (failedCount > 0 || (result.errors?.length || 0) > 0) {
    console.warn('[App] sessions.delete partial failure:', result.errors)
    pushToast(t('shared.sidebar.bulkDeletePartial', { count: failedCount || result.errors?.length || 0 }), { tone: 'danger' })
  }
  await loadSessions()
  if (wasCurrentDeleted && deleted.has(currentKey)) {
    void openDefaultDraft()
  }
}

async function onDeleteSession(key: string) {
  if (!key) return
  const wasCurrent = key === currentSessionKey.value
  const result = await deleteSessions([key])
  if (!result?.deleted?.includes(key)) {
    console.warn('[App] sessions.delete reported failure:', result?.errors)
    pushToast(t('shared.sidebar.deleteSessionFailed'), { tone: 'danger' })
    return
  }
  pushToast(t('shared.sidebar.deleteSessionDone'), { tone: 'ok' })
  const deleted = new Set([key])
  removeLocalSessions(deleted)
  forgetSessions(deleted)
  sessionTaskAttention.removeMany(deleted)
  appStore.removePendingApprovalsForSessions(deleted)
  dispatchLocalSessionsDeleted(deleted, APP_SESSION_SYNC_SOURCE)
  await loadSessions()
  if (wasCurrent) {
    void openDefaultDraft()
  }
}

// 清空对话（≠ 删除）：消息与模型上下文一并清掉，会话壳、项目归属与产物台账
// 保留。后端会先停掉运行中的回合，因此这里也要丢弃该会话的待裁决审批与未读
// 标记 —— 它们依附的回合已经不存在了。
async function onResetSession(key: string) {
  if (!key) return
  const item = sidebarSessionItems.value.find(entry => entry.key === key)
  const title = item?.title || key
  const count = Number(item?.messageCount ?? 0)
  const ok = await confirm({
    title: t('shared.sidebar.resetSessionTitle'),
    body: count > 0
      ? t('shared.sidebar.resetSessionBody', { title, count })
      : t('shared.sidebar.resetSessionBodyNoCount', { title }),
    primaryLabel: t('shared.sidebar.resetSessionConfirm'),
  })
  if (!ok) return
  try {
    const result = await rpcStore.call<{ ok?: boolean; error?: string }>('sessions.reset', { key })
    if (!result?.ok) throw new Error(result?.error || 'sessions.reset failed')
  } catch (err: unknown) {
    console.warn('[App] sessions.reset error:', errorMessage(err))
    pushToast(t('shared.sidebar.resetSessionFailed'), { tone: 'danger' })
    return
  }
  pushToast(t('shared.sidebar.resetSessionDone'), { tone: 'ok' })
  const touched = new Set([key])
  sessionTaskAttention.removeMany(touched)
  appStore.removePendingApprovalsForSessions(touched)
  dispatchLocalSessionReset(touched, APP_SESSION_SYNC_SOURCE)
  await loadSessions()
}

// Topbar approval pill: jump straight to the blocked session's chat so the
// in-thread card can be answered. The live `pendingApprovals` list (kept fresh
// re-fetch — and the oldest pending session is the deterministic target. With
// no routable session, fall back to the Sessions page.
function openBlockedApprovalSession() {
  const oldest = appStore.oldestPendingWithSession
  if (oldest?.sessionKey) {
    appStore.requestApprovalFocus(oldest)
    switchToSession(oldest.sessionKey, 'approval.openBlockedSession')
    return
  }
  // No session attached to the pending approval: land on Sessions, whose
  // attention strip shows the pending count (the /approvals page is retired).
  router.push('/sessions')
}

// Footer settings row. Both platforms mount the same `/settings` overlay now, so
// a single push covers both. bottomRoutes is honored first to keep any future
// bottom-nav destination authoritative, falling back to the shared overlay.
function openSettings() {
  handleNavClick()
  router.push(bottomRoutes.value[0]?.path ?? '/settings')
}

// Topbar connection pill (web): jump straight to the Connection section so the
// gateway link can be inspected or re-pointed.
function openConnectionSettings() {
  router.push('/settings/connection')
}

function scheduleSessionRefresh() {
  sidebarRefreshPending = true
  if (sessionRefreshTimer) clearTimeout(sessionRefreshTimer)
  sessionRefreshTimer = setTimeout(() => {
    sessionRefreshTimer = null
    flushScheduledSidebarRefresh()
  }, 150)
}

/**
 * 轮询止血（R-17，照 Trae v3.4.1「后台已关闭即停止轮询减少请求」）。
 *
 * 30 s 这一档并不是数据新鲜度所需的：侧栏本来由 `sessions.changed` 事件驱动
 * （见 `sessionListSubscription`），定时器只是「事件漏了」的兜底。标签页在后台时
 * 没有人看侧栏，这个兜底就只是在烧请求和网关日志；但**切回前台必须立刻补一次**，
 * 否则用户回来先读到一份过期列表 —— 那比少打一次请求更糟。
 * 口径与 `LocalModelsView` 的 Ollama 复探一致（后台不探、回前台即探）。
 */
function refreshSidebarOnReturnToForeground() {
  if (document.visibilityState !== 'visible') return
  if (rpcStore.isConnected) scheduleSessionRefresh()
}

function flushScheduledSidebarRefresh() {
  if (
    !sidebarRefreshPending
    || !appAutomaticRpcMounted
    || !optionalSessionRpcAllowed.value
  ) return
  sidebarRefreshPending = false
  if (sessionRefreshTimer) {
    clearTimeout(sessionRefreshTimer)
    sessionRefreshTimer = null
  }
  void loadSidebarData()
}

function loadSidebarData(): Promise<void> {
  if (sidebarLoadPromise) return sidebarLoadPromise
  const requests: Promise<unknown>[] = [loadSessions()]
  if (
    rpcStore.canManageProjectWorkspaces
    && optionalSessionRpcAllowed.value
  ) {
    requests.push(
      projectWorkspaces.loadWorkspaces(optionalSessionRpcCallOptions),
    )
  }
  const request = Promise.allSettled(requests).then(() => undefined)
  const tracked = request.finally(() => {
    if (sidebarLoadPromise === tracked) sidebarLoadPromise = null
  })
  sidebarLoadPromise = tracked
  return tracked
}

function refreshSidebarDataWhenAdmitted(): void | Promise<void> {
  if (!optionalSessionRpcAllowed.value) {
    sidebarRefreshPending = true
    return
  }
  return loadSidebarData()
}

const sessionListSubscription = useSessionListSubscription({
  rpc: rpcStore,
  callOptions: optionalSessionRpcCallOptions,
  isConnected: () => rpcStore.isConnected,
  isAdmitted: () => optionalSessionRpcAllowed.value,
  refresh: refreshSidebarDataWhenAdmitted,
  scheduleRefresh: scheduleSessionRefresh,
  onChanged: payload => {
    sessionTaskAttention.handleSessionsChanged(payload, {
      currentSessionKey: currentSessionKey.value,
      currentSessionVisible: currentSessionIsVisible(),
    })
  },
  // Real-time run-status: mark a sidebar row running/idle the instant the
  // gateway reports it, so the conversation list no longer depends on the 30s
  // polling refresh. Owned at app level — survives Chat/Overview/Settings route
  // changes because this subscription is never torn down by a view unmount.
  onRunStatus: (event, payload) => { applySessionRunStatus(payload, event) },
  warn: (message, error) => console.warn(`[App] ${message}:`, errorMessage(error)),
})

function subscribeCronEventsWhenAdmitted() {
  if (
    !appAutomaticRpcMounted
    || !optionalSessionRpcAllowed.value
    || !rpcStore.isConnected
  ) return
  void rpcStore.call('cron.subscribe', {}).catch(() => undefined)
}

function resumeAutomaticAppRpc() {
  if (!appAutomaticRpcMounted || !optionalSessionRpcAllowed.value) return
  subscribeCronEventsWhenAdmitted()
  sessionListSubscription.resume()
  if (!appAutomaticRpcStarted) {
    appAutomaticRpcStarted = true
    void loadAgents()
    void loadSidebarData()
  }
  flushScheduledSidebarRefresh()
}

watch(optionalSessionRpcAllowed, admitted => {
  if (admitted) resumeAutomaticAppRpc()
}, { flush: 'sync' })

watch(
  () => rpcStore.state,
  state => {
    if (state === 'connected') subscribeCronEventsWhenAdmitted()
  },
)

function handleKeydown(e: KeyboardEvent) {
  // Chord bindings carry the primary modifier as Cmd on Apple platforms and Ctrl
  // elsewhere — and require the other modifier to be absent — so we never match
  // macOS' Ctrl+K (emacs kill-to-end-of-line inside text fields). preventDefault
  // runs BEFORE any early return so the browser never sees the chord: on
  // Chrome/Edge/Firefox (Win/Linux) Ctrl+K focuses the omnibox/search, and in
  // Firefox-mac Cmd+K focuses the search bar. Swallowing it unconditionally also
  // lets the shortcut fire from inside the composer textarea, where the cursor
  // usually sits.
  //
  // Configurable chord shortcuts, consulted from the shortcuts store so the
  // Keyboard settings section is the single source of truth. effectiveBinding
  // returns null for a disabled shortcut, so bindingMatches skips it. New chat
  // is checked first because the palette's no-shift binding would otherwise also
  // match a Shift+K press under a looser guard. preventDefault still runs before
  // the settingsOverlay guard so the browser never sees the chord.
  const paletteBinding = shortcutsStore.effectiveBinding('command-palette')
  const quickOpenBinding = shortcutsStore.effectiveBinding('quick-open')
  const newChatBinding = shortcutsStore.effectiveBinding('new-chat')
  const toggleSidebarBinding = shortcutsStore.effectiveBinding('toggle-sidebar')
  if (bindingMatches(e, toggleSidebarBinding, isMac)) {
    e.preventDefault()
    if (e.repeat || settingsOverlayOpen.value) return
    toggleDock('shortcut')
    return
  }
  if (bindingMatches(e, newChatBinding, isMac)) {
    e.preventDefault()
    if (settingsOverlayOpen.value) return
    startNewChatInstant()
    return
  }
  if (bindingMatches(e, quickOpenBinding, isMac)) {
    e.preventDefault()
    if (settingsOverlayOpen.value) return
    // Toggle so a second press closes it; the modal owns Escape/focus while
    // open, and its own binding never collides with the palette (Ctrl/⌘+P).
    fileQuickOpenOpen.value = !fileQuickOpenOpen.value
    return
  }
  if (bindingMatches(e, paletteBinding, isMac)) {
    e.preventDefault()
    if (settingsOverlayOpen.value) return
    // Toggle so a second press closes it; the palette owns Escape/focus while open.
    commandPaletteOpen.value = !commandPaletteOpen.value
    return
  }

  // Skip App's fallbacks when a handler that runs BEFORE this one already
  // consumed the key: the composer textarea (@keydown, target phase) and any
  // earlier-registered document listener (e.g. ChatView). Overlays (drawers,
  // modals) attach their document listeners on open — AFTER this one — so they
  // run later and are NOT covered by this guard; their collision with the
  // sidebar-Escape branch is ruled out by the mobile-only gate below instead.
  if (e.defaultPrevented) return

  if (e.key === 'Escape' && themeMenuOpen.value) {
    themeMenuOpen.value = false
    themeMenuRef.value?.focusButton()
    return
  }
  // Escape dismisses the sidebar only as the mobile slide-over. On desktop the
  // sidebar is a persistent dock toggled by its own button, so it must never
  // collapse as a side effect of an Escape meant for an overlay opened on top of
  // it. Because those overlays run after this handler (see above), this
  // mobile-only gate — not the defaultPrevented check — is what prevents that
  // collision; keep it. The settings overlay owns Escape while open and is excluded.
  if (e.key === 'Escape' && appStore.sidebarOpen && !settingsOverlayOpen.value && isSidebarDrawer.value) {
    closeSidebarDrawer()
  }
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

// ---------------------------------------------------------------------------
// App-wide approval awareness
//
// A view-local snapshot is not enough; a tool that blocks a background/queued
// turn must surface the badge from any view. The
// gateway pushes `<namespace>.approval.requested|resolved` the moment a run
// blocks or a decision lands, so we keep `pendingApprovals`/`approvalCount`
// live here, seeded once on (re)connect to recover requests that predate the
// socket (e.g. a reload while one is already pending).
// ---------------------------------------------------------------------------

interface ApprovalPushPayload {
  approval_id?: string
  approvalId?: string
  session_key?: string
  sessionKey?: string
  tool_name?: string
  toolName?: string
  command?: string
}

interface ApprovalSnapshotItem {
  id?: string
  sessionKey?: string
  toolName?: string
  pluginId?: string
  actionKind?: string
  command?: string
  argv?: string[]
}

const rpcApprovalUnsubs: Array<() => void> = []

function approvalAuthHeaders(): Record<string, string> {
  const headers: Record<string, string> = {}
  const token = readAuthToken()
  if (token) headers['Authorization'] = `Bearer ${token}`
  return headers
}

function snapshotItemToPending(item: ApprovalSnapshotItem): PendingApproval | null {
  const approvalId = String(item.id || '').trim()
  if (!approvalId) return null
  let command = String(item.command || '')
  if (!command && Array.isArray(item.argv) && item.argv.length > 0) {
    command = item.argv.map(String).join(' ')
  }
  return {
    approvalId,
    sessionKey: String(item.sessionKey || ''),
    tool: String(item.toolName || item.pluginId || item.actionKind || 'Unknown tool'),
    command,
  }
}

// Seed the live list from the snapshot so the count is correct after a reload
// while a request is already pending. The
// snapshot is ordered oldest-first, which the deep-link relies on.
async function seedPendingApprovals() {
  try {
    const res = await fetch('/api/approvals', { headers: approvalAuthHeaders() })
    if (!res.ok) throw new Error('HTTP ' + res.status)
    const data = await res.json() as {
      pending?: ApprovalSnapshotItem[]
      pending_sidecar_offline?: number
    }
    const items = (data.pending || [])
      .map(snapshotItemToPending)
      .filter((item): item is PendingApproval => item !== null)
    appStore.setPendingApprovals(items)
    // P2-4：旧网关没有该字段 —— 缺省视为 0（不显示黄条），而不是猜一个数。
    appStore.setSidecarOfflineApprovals(Number(data.pending_sidecar_offline) || 0)
  } catch (err) {
    appStore.setSidecarOfflineApprovals(0)
    console.warn('[App] approvals seed failed:', errorMessage(err))
  }
}

function onApprovalRequested(payload: ApprovalPushPayload) {
  const approvalId = String(payload.approval_id || payload.approvalId || '').trim()
  if (!approvalId) return
  appStore.upsertPendingApproval({
    approvalId,
    sessionKey: String(payload.session_key || payload.sessionKey || ''),
    tool: String(payload.tool_name || payload.toolName || 'Unknown tool'),
    command: String(payload.command || ''),
  })
}

function onApprovalResolved(payload: ApprovalPushPayload) {
  const approvalId = String(payload.approval_id || payload.approvalId || '').trim()
  if (approvalId) appStore.removePendingApproval(approvalId)
}

// Reconnect re-seeds the list (recovers approvals that arrived while the socket
// was down); the push events keep it live thereafter.
function onApprovalConnectionState(state: unknown) {
  if (state !== 'connected') {
    appStore.setPendingApprovals([])
    appStore.setSidecarOfflineApprovals(0)
    return
  }
  void seedPendingApprovals()
}

function subscribeApprovals() {
  rpcApprovalUnsubs.push(
    rpcStore.on('exec.approval.requested', onApprovalRequested as RpcEventHandler),
    rpcStore.on('exec.approval.resolved', onApprovalResolved as RpcEventHandler),
    rpcStore.on('plugin.approval.requested', onApprovalRequested as RpcEventHandler),
    rpcStore.on('plugin.approval.resolved', onApprovalResolved as RpcEventHandler),
    rpcStore.on('_state', onApprovalConnectionState as RpcEventHandler),
  )
}

function unsubscribeApprovals() {
  rpcApprovalUnsubs.forEach(unsub => unsub())
  rpcApprovalUnsubs.length = 0
}

// ---------------------------------------------------------------------------
// Tab-title + screen-reader badge for the pending count
// ---------------------------------------------------------------------------

const BASE_TITLE = document.title

const approvalAnnouncement = ref('')

let titleDebounce: ReturnType<typeof setTimeout> | null = null

// 未读数前缀必须套在「当前路由 + 当前语言」的标题上：语言切换与首屏语言就绪
// 都会重排标题，拼接旧快照会把前缀挂到过期标题上。
function routeDocumentTitle(): string {
  return `${routeTitle($route)} — Marguerite`
}

function applyTitleBadge(count: number) {
  const base = routeDocumentTitle()
  document.title = count > 0 ? `(${count}) ${base}` : base
}

// 语言切换 / 首屏语言就绪时重设一次标题（沿用当前的未读数前缀）。
function applyRouteDocumentTitle() {
  applyTitleBadge(appStore.approvalCount)
}

// Debounce so a burst of count changes does not thrash the tab title.
watch(() => appStore.approvalCount, count => {
  approvalAnnouncement.value = count > 0 ? `${count} approvals pending` : ''
  if (titleDebounce) clearTimeout(titleDebounce)
  titleDebounce = setTimeout(() => {
    titleDebounce = null
    applyTitleBadge(count)
  }, 500)
})

useDocumentEvent('keydown', handleKeydown)

onMounted(() => {
  appAutomaticRpcMounted = true
  // 工作台与右侧栏的宽度联动依赖视口宽度：跟随窗口变化重新计算 inset。
  window.addEventListener('resize', syncLayoutViewport)
  // WebGL 探测放在挂载后：不 import three（那是几百 KB），只用一次性的原生上下文探测
  webglOk.value = detectWebGL()
  // 环境音：读取持久化偏好；开启过的话等第一次用户手势再启动（浏览器自动播放策略）
  ambience.init()
  // 液态玻璃：恢复背景质感场景与参数面板的调参（内联 --glass-* 覆盖主题默认值）
  initGlassScene()
  initGlassParams()
  // 壁纸冷启动恢复：只在 `/wallpaper` 视图恢复会让「重启后落在 /chat」的会话
  // 看不到已选壁纸（真机）。这里在应用挂载时统一还原一次。
  restoreStoredWallpaper()
  // 桌面桌宠悬浮窗：上次开过就在启动后恢复（幂等）。
  // 必须排除 /pet-overlay 自身 —— 悬浮窗加载的也是本应用，否则会自我唤醒。
  if (!isPetOverlayRoute.value) void restorePetOverlayPreference()
  window.visualViewport?.addEventListener('resize', syncMobileKeyboard)
  window.addEventListener(LOCAL_SESSIONS_DELETED_EVENT, handleLocalSessionsDeleted)
  window.addEventListener(LOCAL_SESSION_RESET_EVENT, handleLocalSessionReset)
  window.addEventListener('focus', markCurrentSessionReadIfVisible)
  document.addEventListener('visibilitychange', markCurrentSessionReadIfVisible)
  // 后台不轮询 ⇒ 回前台补一次（见 refreshSidebarOnReturnToForeground 的注释）。
  document.addEventListener('visibilitychange', refreshSidebarOnReturnToForeground)
  sessionListSubscription.subscribe()
  resumeAutomaticAppRpc()
  // Keep the approval badge/count live app-wide, not just on the Approvals page.
  subscribeApprovals()
  unsubscribeCronFinished = rpcStore.on('cron.run.finished', handleCronRunFinished)
  // Global runtime-mode subscription (upgrade-masterplan §5): a mode switch in
  // ControlCenter now propagates to the top bar / sidebar / open views instantly.
  initActiveMode()
  // Seed now in case the socket is already connected (the `_state` listener
  // covers later reconnects); recovers a request pending before mount.
  if (rpcStore.isConnected) void seedPendingApprovals()
  // Publish the topbar app cluster's live width as --topbar-right-reserve
  // (see publishTopbarRightReserve).
  void nextTick(observeTopbarRight)
  // 每 30 秒刷新会话列表，更新运行状态和闲置时间（后台标签页不刷，见上）
  sessionStatusTimer = window.setInterval(() => {
    if (document.visibilityState !== 'visible') return
    if (rpcStore.isConnected) scheduleSessionRefresh()
  }, 30000)
})

onUnmounted(() => {
  removeInternalLinkGuard()
  appAutomaticRpcMounted = false
  window.removeEventListener('resize', syncLayoutViewport)
  sidebarRefreshPending = false
  window.removeEventListener(LOCAL_SESSIONS_DELETED_EVENT, handleLocalSessionsDeleted)
  window.removeEventListener(LOCAL_SESSION_RESET_EVENT, handleLocalSessionReset)
  window.removeEventListener('focus', markCurrentSessionReadIfVisible)
  document.removeEventListener('visibilitychange', markCurrentSessionReadIfVisible)
  document.removeEventListener('visibilitychange', refreshSidebarOnReturnToForeground)
  if (sessionRefreshTimer) clearTimeout(sessionRefreshTimer)
  if (sessionStatusTimer) { clearInterval(sessionStatusTimer); sessionStatusTimer = null }
  sessionListSubscription.cleanup()
  unsubscribeApprovals()
  unsubscribeCronFinished?.()
  unsubscribeCronFinished = null
  disposeActiveMode()
  void rpcStore.call('cron.unsubscribe', {}).catch(() => undefined)
  if (titleDebounce) {
    clearTimeout(titleDebounce)
    titleDebounce = null
  }
  document.title = BASE_TITLE
  window.visualViewport?.removeEventListener('resize', syncMobileKeyboard)
  topbarRightObserver?.disconnect()
  topbarRightObserver = null
})

</script>

<style scoped>
/* Reserve room for the absolutely pinned window controls so they never cover the
   last item in the app cluster. Scoped to the desktop shell: in the CLI gateway
   the controls do not render, so the cluster must not lose 110px of width.

   `.topbar` and `.topbar-right` are both in THIS component's template, so the
   scoped attribute lands on them and a plain descendant selector matches. */
.topbar--desktop-shell .topbar-right {
  padding-inline-end: 110px;
}

/* Custom title-bar drag region (Tauri shell only).

   Out of flow on purpose — see the markup comment. `.topbar` is `position:
   absolute` (non-chat) / `relative` (chat), so it is the containing block in both
   modes. The region is pinned to the vertical middle band between the route
   actions and the app cluster, so it always keeps real pixels to grab no matter
   how wide the teleported Chat actions are.

   `z-index: 1` sits above the bar's own background but below every control
   (`pointer-events: none` is inherited from `.topbar`; only the region opts back
   in), so drags land here while control clicks pass through untouched. */
.topbar-drag-region {
  position: absolute;
  z-index: 1;
  top: 0;
  bottom: 0;
  /* Start after the sidebar toggle; stop short of the mode badge / app cluster. */
  left: 56px;
  right: 320px;
  min-width: 48px;
  pointer-events: auto;
  cursor: default;
}

/* Below ~640px there is no honest free band left between the route actions and
   the app cluster, so reclaim the right inset and keep only a narrow strip. */
@media (max-width: 640px) {
  .topbar-drag-region {
    right: 190px;
  }
}
/* Collapsible capability groups (Task #6) */
.sidebar-group {
  margin-top: 6px;
}
.sidebar-group__header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 4px 10px;
  border: 0;
  background: transparent;
  color: var(--text-dim);
  font: inherit;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  cursor: pointer;
  border-radius: var(--radius-xs);
}
.sidebar-group__header:hover {
  background: color-mix(in srgb, var(--bg-elevated) 70%, transparent);
  color: var(--text);
}
.sidebar-group__chevron {
  transition: transform var(--dur-fast) var(--ease-standard);
  transform: rotate(90deg);
}
.sidebar-group__chevron.is-collapsed {
  transform: rotate(0deg);
}
.sidebar-group__title {
  flex: 1;
  text-align: left;
}
.sidebar-group__count {
  font-size: 10px;
  font-weight: 500;
  opacity: 0.6;
}
.sidebar-group__items {
  display: flex;
  flex-direction: column;
  gap: 1px;
  margin-top: 2px;
}
.sidebar-fn-item--nested {
  padding-left: 26px;
  font-size: 13px;
}

.app-workspace {
  position: relative;
  display: flex;
  min-width: 0;
  min-height: 0;
  flex: 1;
}

/* Topbar connection pill as a button (web): inherits the base .conn-pill look
   and state colors, adds button reset + an affordance that it is clickable. */
.conn-pill--link {
  cursor: pointer;
  font-family: inherit;
}
.conn-pill--link:hover {
  filter: brightness(1.08);
}
.conn-pill--link:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}

/* Off-screen but screen-reader-reachable announcer for the approval count. */
.app-approval-live {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
  border: 0;
}

/* S5 host-exit banner: **in flow** at the top of the content area. It used to be
   a fixed bottom-right card and covered panels/primary buttons in every audited
   screenshot; only the horizontal inset is kept so it aligns with view gutters. */
.app-host-exit {
  flex: 0 0 auto;
  /* `.content` 自带 sp-4 内边距：这里只补下间距，避免横向缩进叠加。 */
  margin: 0 0 var(--sp-3);
  max-width: 56rem;
}

/* 聊天路由例外：`.chat` 是 `height:100%` 的纵向 flex，任何 in-flow 兄弟都会把
   输入框顶出可视区（`overflow:hidden` 直接裁掉）。这里改为浮在内容区顶部居中，
   只压住聊天标题行的空白区，不进入 `.chat` 的布局高度。 */
.content--chat > .app-host-exit {
  position: absolute;
  top: var(--sp-2);
  left: 50%;
  z-index: 5;
  margin: 0;
  width: max-content;
  max-width: min(56rem, calc(100% - 2 * var(--sp-4)));
  transform: translateX(-50%);
}
</style>

<!-- Auto-triggered workstation effects while background music plays.
     `body.ambience-on` is toggled by AmbienceLayer.vue; `--amb-amp` (0..1) is the
     synth pad's live RMS, published on :root. Token-only colours (no literals) so
     the component colour guard stays happy.
     The breathing accent rim is stacking-independent (fixed, above content) and is
     the signature "music is on" cue that animates with the track; the full-interface
     frosted glass itself comes from AmbienceLayer's own scrim. -->
<style>
body.ambience-on::after {
  content: "";
  position: fixed;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent) 38%, transparent);
  opacity: calc(0.35 + var(--amb-amp, 0) * 0.6);
  transition: opacity var(--dur-fast) linear;
}
@media (prefers-reduced-transparency: reduce) {
  body.ambience-on::after { box-shadow: none; }
}

/* Music-mode layout shift (design §6, restrained): the sidebar and the chat
   topbar drop to light frosted glass so the aurora breathes through the edges.
   No movement, no scale — readability first, the 1px rim signature stays.
   The paired selector keeps the sidebar's existing drawer transform
   transition (base.css) alive while adding the slow background/backdrop ramp.
   All timing goes through --dur-slow; colours are token-only. */
body.ambience-on .sidebar {
  background: color-mix(in srgb, var(--sidebar-bg) 74%, transparent);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}
body.ambience-on .topbar--chat {
  background: color-mix(in srgb, var(--bg-elevated) 68%, transparent);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}
body.ambience-on .sidebar,
body:not(.ambience-on) .sidebar {
  transition:
    transform var(--dur-base) var(--ease-out),
    background var(--dur-slow) var(--ease-standard),
    backdrop-filter var(--dur-slow) var(--ease-standard),
    -webkit-backdrop-filter var(--dur-slow) var(--ease-standard);
}
body.ambience-on .topbar--chat,
body:not(.ambience-on) .topbar--chat {
  transition: background var(--dur-slow) var(--ease-standard);
}
@media (prefers-reduced-transparency: reduce) {
  body.ambience-on .sidebar,
  body.ambience-on .topbar--chat {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
}
</style>
