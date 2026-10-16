<template>
  <ProjectWorkspaceCreateDialog
    v-if="rpcStore.canChooseProject"
    :open="projectCreateOpen && !projectCreateConfirming && !projectSourcePickerOpen"
    :name="projectCreateName"
    :source-path="projectCreateSourcePath"
    :busy="projectCreateBusy"
    :source-picking="projectCreateSourcePicking"
    @update:name="projectCreateName = $event"
    @choose-source="chooseProjectSourceDirectory"
    @close="closeProjectCreator"
    @create="createProjectWorkspace"
  />

  <ProjectWorkspacePickerDialog
    v-if="rpcStore.canChooseProject"
    :open="projectCreateOpen && projectSourcePickerOpen"
    :enabled="rpcStore.canChooseProject"
    :session-key="sessionKey || 'agent:main:webchat:workspace-picker'"
    :initial-path="projectCreateSourcePath"
    @close="projectSourcePickerOpen = false"
    @choose="onProjectSourcePathChosen"
  />
</template>

<script setup lang="ts">
import { nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import ProjectWorkspaceCreateDialog from './ProjectWorkspaceCreateDialog.vue'
import ProjectWorkspacePickerDialog from './ProjectWorkspacePickerDialog.vue'
import { useRpcStore } from '@/stores/rpc'
import { useProjectWorkspaces } from '@/composables/useProjectWorkspaces'
import { useConfirm } from '@/composables/useConfirm'
import { useToasts } from '@/composables/useToasts'
import { getPlatform } from '@/platform'

const props = defineProps<{
  sessionKey: string
}>()

const { t } = useI18n()
const rpcStore = useRpcStore()
const projectWorkspaces = useProjectWorkspaces()
const { confirm } = useConfirm()
const { pushToast } = useToasts()

const projectCreateOpen = ref(false)
const projectCreateName = ref('')
const projectCreateSourcePath = ref('')
const projectCreateBusy = ref(false)
const projectCreateSourcePicking = ref(false)
const projectCreateConfirming = ref(false)
const projectSourcePickerOpen = ref(false)

// Reset the create dialog state whenever project capability is revoked,
// mirroring the App-level deregistration that also clears the edit overlay.
watch(
  () => rpcStore.canManageProjectWorkspaces,
  allowed => {
    if (!allowed) resetProjectCreator()
  },
)

function projectNameFromPath(path: string): string {
  const normalized = path.trim().replace(/[\\/]+$/, '')
  return normalized.split(/[\\/]/).pop() || normalized
}

function resetProjectCreator() {
  projectCreateOpen.value = false
  projectCreateName.value = ''
  projectCreateSourcePath.value = ''
  projectCreateBusy.value = false
  projectCreateSourcePicking.value = false
  projectCreateConfirming.value = false
  projectSourcePickerOpen.value = false
}

// Exposed so App can open the create dialog from the sidebar and from the
// new-chat guide (maybeGuideProjectThenDraft) without owning its state.
function openProjectCreator() {
  if (!rpcStore.canChooseProject) return
  projectCreateName.value = ''
  projectCreateSourcePath.value = ''
  projectCreateBusy.value = false
  projectCreateSourcePicking.value = false
  projectCreateConfirming.value = false
  projectSourcePickerOpen.value = false
  projectCreateOpen.value = true
}

function closeProjectCreator() {
  if (projectCreateBusy.value) return
  resetProjectCreator()
}

function onProjectPathChosen(path: string) {
  if (!projectCreateOpen.value || !rpcStore.canChooseProject) return
  projectCreateSourcePath.value = path
  if (!projectCreateName.value.trim()) {
    projectCreateName.value = projectNameFromPath(path)
  }
}

function onProjectSourcePathChosen(path: string) {
  projectSourcePickerOpen.value = false
  onProjectPathChosen(path)
}

async function chooseProjectSourceDirectory() {
  if (
    !projectCreateOpen.value
    || !rpcStore.canChooseProject
    || projectCreateBusy.value
    || projectCreateSourcePicking.value
    || projectSourcePickerOpen.value
  ) return

  const nativePicker = getPlatform().files.chooseProjectDirectory
  if (typeof nativePicker !== 'function') {
    projectSourcePickerOpen.value = true
    return
  }

  projectCreateSourcePicking.value = true
  try {
    const choice = await nativePicker()
    const selected = String(choice?.path || '').trim()
    if (selected) onProjectPathChosen(selected)
  } catch (err) {
    pushToast(t('workspaces.directoryPickerFailed', { error: errorMessage(err) }), {
      tone: 'danger',
    })
  } finally {
    projectCreateSourcePicking.value = false
  }
}

async function createProjectWorkspace(payload: { name: string; path: string }) {
  if (
    !projectCreateOpen.value
    || !rpcStore.canChooseProject
    || projectCreateBusy.value
    || projectCreateSourcePicking.value
    || projectSourcePickerOpen.value
  ) return
  const name = payload.name.trim()
  const path = payload.path.trim()
  if (!name || !path) return
  projectCreateConfirming.value = true
  await nextTick()
  const trusted = await confirm({
    title: t('workspaces.trustTitle'),
    body: t('workspaces.trustBody', { path }),
    primaryLabel: t('workspaces.trustConfirm'),
    primaryClass: 'btn--primary',
  })
  if (!trusted) {
    projectCreateConfirming.value = false
    return
  }
  projectCreateBusy.value = true
  try {
    const workspace = await projectWorkspaces.openWorkspace(path)
    if (!workspace) throw new Error('Gateway returned an empty project.')
    if (workspace.name !== name) {
      await projectWorkspaces.renameWorkspace(workspace.id, name)
    }
    resetProjectCreator()
    pushToast(t('workspaces.projectCreated', { name }), { tone: 'ok' })
  } catch (err) {
    projectCreateBusy.value = false
    projectCreateConfirming.value = false
    pushToast(t('workspaces.createProjectFailed', { error: errorMessage(err) }), { tone: 'danger' })
  }
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

defineExpose({ openProjectCreator })
</script>
