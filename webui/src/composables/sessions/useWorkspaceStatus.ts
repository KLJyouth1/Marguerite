/**
 * Session workspace status — the user-facing half of P0-1b.
 *
 * Why this exists: the workspace root decides where **every** file tool reads and
 * writes. When a session is not bound to a project directory the agent works
 * inside the app data dir (`AppData\Local\Marguerite`) — and users previously
 * saw only an unexplained tool failure (the failure chain in the maturity
 * report: unbound session → guessed relative path → 1 ms failure → retry loop).
 *
 * `workspace.status` already returns the authoritative resolution *and* the
 * `guidance` text the model itself is given. This composable only fetches it per
 * session, so the chip in the UI and the model's system prompt can never
 * disagree — no second copy of the resolution logic.
 *
 * Read-only RPC, no side effects.
 */
import { ref } from 'vue'
import { useRpcStore } from '@/stores/rpc'

/** Mirrors `WorkspaceSource::as_str()` in `core/src/workspace.rs`. */
export type WorkspaceSourceKind = 'bound' | 'app_data' | 'process_cwd' | 'missing'

export interface WorkspaceStatus {
  ok: boolean
  root: string
  source: WorkspaceSourceKind
  bound: boolean
  /** The exact system-prompt fragment the model receives for this root. */
  guidance: string
}

export function useWorkspaceStatus() {
  const rpc = useRpcStore()
  const status = ref<WorkspaceStatus | null>(null)
  const loading = ref(false)

  /**
   * Load the resolved workspace for one session.
   *
   * `projectPath` is the session's bound root (`SessionItem.workspace`). A blank
   * value means "unbound", which the backend resolves to the app data dir; the
   * parameter is therefore omitted rather than guessed, so the chip reports the
   * fallback truthfully.
   */
  async function load(projectPath?: string | null) {
    if (!rpc.supportsMethod('workspace.status')) {
      status.value = null
      return
    }
    loading.value = true
    try {
      const path = String(projectPath || '').trim()
      status.value = await rpc.call<WorkspaceStatus>(
        'workspace.status',
        path ? { projectPath: path } : {},
      )
    } catch {
      // Older gateway or dropped connection: render nothing instead of
      // inventing a binding state.
      status.value = null
    } finally {
      loading.value = false
    }
  }

  function reset() {
    status.value = null
    loading.value = false
  }

  return { status, loading, load, reset }
}
