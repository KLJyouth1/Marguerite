/**
 * Artifact side-effects: hand a file to the OS, and write an edited file back
 * (SPEC §5 — `artifacts.open_in_system` / POST /api/v1/artifacts/:id/open and
 * `artifacts.edit_inline` / POST /api/v1/artifacts/:id/content; AC-A3/AC-A4).
 *
 * Transport is deliberately dual. The gateway advertises its method list, so
 * when the RPC verb is present we use it (one authenticated channel, no extra
 * CORS surface). When it is absent — an older daemon, or a build where only the
 * REST route shipped — we fall back to the spec'd HTTP route on the same origin.
 * If neither can serve the call the result says so explicitly, so the caller can
 * degrade to an in-app preview instead of leaving a button that does nothing.
 *
 * Failure is a return value, not an exception: these actions fire from list
 * rows, where a thrown error is either swallowed by the template or takes out
 * the surrounding render. `launched` is the only field a caller needs to branch
 * on; `method` exists so a caller can tell "nothing could run this" (`none`)
 * apart from "the transport we used failed", and avoid double-fallback.
 */

import type { ArtifactPayload } from '@/types/rpc'
import type { RpcCallOptions } from '@/lib/rpc'
import { artifactDownloadUrl } from '@/utils/chat/artifacts'

export const ARTIFACT_OPEN_IN_SYSTEM_METHOD = 'artifacts.open_in_system'
export const ARTIFACT_EDIT_INLINE_METHOD = 'artifacts.edit_inline'

/** Guard against loading a "text" file that is really a 200 MB log into a textarea. */
export const ARTIFACT_INLINE_EDIT_MAX_BYTES = 2 * 1024 * 1024

export type ArtifactActionStatus = 'ok' | 'unsupported' | 'error'

/**
 * Which transport served (or last attempted) the call.
 * `none` means neither transport could run it at all — the deployment lacks the
 * capability, so the caller should offer a different route entirely.
 */
export type ArtifactActionTransport = 'rpc' | 'gateway-fallback' | 'none'

export interface ArtifactActionResult {
  status: ArtifactActionStatus
  /** True only when the action actually took effect. */
  launched: boolean
  method: ArtifactActionTransport
  /** Raw failure detail; empty on success. */
  error: string
  /** Operator-facing message, phrased as the next action. */
  message: string
}

export interface ArtifactTextLoadResult extends ArtifactActionResult {
  content: string
  /** Bytes seen on the wire; drives the "too large to edit inline" gate. */
  bytes: number
}

type ArtifactActionsRpc = {
  supportsMethod: (method: string) => boolean
  markMethodUnavailable: (method: string) => void
  call: <T = unknown>(
    method: string,
    params?: Record<string, unknown>,
    callOptions?: RpcCallOptions,
  ) => Promise<T>
}

export interface UseArtifactActionsOptions {
  rpc: ArtifactActionsRpc
  /** Bearer token for same-origin HTTP fallbacks. Omit when cookies suffice. */
  authToken?: () => string
  /** Origin the gateway is reachable on; defaults to the page origin. */
  baseOrigin?: () => string
  sessionKey?: () => string
}

function artifactId(artifact: ArtifactPayload): string {
  return String(artifact?.id || '')
}

function errorMessage(err: unknown): string {
  if (err instanceof Error && err.message) return err.message
  if (typeof err === 'string' && err) return err
  return '未知错误'
}

function ok(method: ArtifactActionTransport): ArtifactActionResult {
  return { status: 'ok', launched: true, method, error: '', message: '' }
}

function fail(
  status: Exclude<ArtifactActionStatus, 'ok'>,
  method: ArtifactActionTransport,
  message: string,
): ArtifactActionResult {
  return { status, launched: false, method, error: message, message }
}

/**
 * A method-not-found style rejection means the daemon lacks the verb, which is
 * permanent for this connection — remember it so we stop paying a round trip on
 * every click and go straight to the HTTP route.
 */
function looksUnsupported(err: unknown): boolean {
  const raw = err instanceof Error ? err.message : String(err ?? '')
  const text = raw.toLowerCase()
  return text.includes('-32601')
    || text.includes('method not found')
    || text.includes('unknown method')
    || text.includes('unsupported method')
    || text.includes('not implemented')
}

export function useArtifactActions(options: UseArtifactActionsOptions) {
  const { rpc } = options
  const readToken = () => options.authToken?.() || ''
  const readOrigin = () => options.baseOrigin?.() || window.location.origin

  function sameOriginHeaders(url: string): Record<string, string> {
    const headers: Record<string, string> = {}
    const origin = readOrigin()
    let sameOrigin = false
    try {
      sameOrigin = new URL(url, origin).origin === new URL(origin).origin
    } catch { sameOrigin = false }
    const token = readToken()
    if (sameOrigin && token) headers.Authorization = `Bearer ${token}`
    // 产物按会话归属：网关（Rust 与侧车同契约）在缺键/错键时一律 404。
    // 与 artifactAccessHeaders 保持同一个头名。
    const sessionKey = options.sessionKey?.() || ''
    if (sameOrigin && sessionKey) headers['x-Marguerite-session-key'] = sessionKey
    return headers
  }

  function restUrl(artifact: ArtifactPayload, suffix: 'open' | 'content'): string {
    const id = artifactId(artifact)
    if (!id) return ''
    return `/api/v1/artifacts/${encodeURIComponent(id)}/${suffix}`
  }

  async function postRest(
    artifact: ArtifactPayload,
    suffix: 'open' | 'content',
    body?: Record<string, unknown>,
  ): Promise<ArtifactActionResult> {
    const url = restUrl(artifact, suffix)
    if (!url) return fail('error', 'none', '该产物缺少 id，无法定位文件')
    try {
      const response = await fetch(url, {
        method: 'POST',
        headers: { ...sameOriginHeaders(url), 'Content-Type': 'application/json' },
        credentials: 'same-origin',
        body: JSON.stringify(body ?? {}),
        redirect: 'error',
      })
      // 404/405/501 from the REST route after the RPC verb was already absent
      // means the deployment simply cannot do this — report `none` so the caller
      // routes the user somewhere that works.
      if (response.status === 404 || response.status === 405 || response.status === 501) {
        return fail('unsupported', 'none', '当前后端版本尚不支持该操作，请更新 Marguerite 后重试')
      }
      if (!response.ok) {
        const detail = await response.text().catch(() => '')
        return fail(
          'error',
          'gateway-fallback',
          detail?.slice(0, 200) || `请求失败（HTTP ${response.status}）`,
        )
      }
      return ok('gateway-fallback')
    } catch (err) {
      return fail('error', 'gateway-fallback', errorMessage(err))
    }
  }

  /**
   * Ask the daemon to hand the file to the OS-registered application. The daemon
   * has to do this, not the browser: `file://` navigation is blocked, and the
   * artifact store lives on the daemon's filesystem, not the client's.
   */
  async function openInSystem(artifact: ArtifactPayload): Promise<ArtifactActionResult> {
    const id = artifactId(artifact)
    const params: Record<string, unknown> = id
      ? { id }
      : { path: String(artifact?.path || artifact?.name || '') }
    const sessionKey = options.sessionKey?.()
    if (sessionKey) params.sessionKey = sessionKey

    if (rpc.supportsMethod(ARTIFACT_OPEN_IN_SYSTEM_METHOD)) {
      try {
        await rpc.call(ARTIFACT_OPEN_IN_SYSTEM_METHOD, params)
        return ok('rpc')
      } catch (err) {
        if (looksUnsupported(err)) {
          rpc.markMethodUnavailable(ARTIFACT_OPEN_IN_SYSTEM_METHOD)
        } else {
          return fail('error', 'rpc', errorMessage(err))
        }
      }
    }
    return postRest(artifact, 'open')
  }

  /**
   * Read an artifact as text for the inline editor. Uses the same authenticated
   * download route the preview does, and refuses oversized bodies *before*
   * decoding — a textarea holding megabytes of text freezes the tab.
   */
  async function loadTextContent(artifact: ArtifactPayload): Promise<ArtifactTextLoadResult> {
    const url = artifactDownloadUrl(artifact, readOrigin(), {
      sessionKey: options.sessionKey?.() || '',
    })
    if (!url) {
      return { ...fail('error', 'none', '该产物没有可读取的下载地址'), content: '', bytes: 0 }
    }
    try {
      const response = await fetch(url, {
        method: 'GET',
        headers: sameOriginHeaders(url),
        credentials: 'same-origin',
        redirect: 'error',
      })
      if (!response.ok) {
        return {
          ...fail('error', 'gateway-fallback', `读取失败（HTTP ${response.status}）`),
          content: '',
          bytes: 0,
        }
      }
      const tooLarge = (bytes: number): ArtifactTextLoadResult => ({
        ...fail('unsupported', 'none', '文件超过 2 MB，请用系统程序打开编辑'),
        content: '',
        bytes,
      })
      const declared = Number(response.headers.get('Content-Length') || 0)
      if (Number.isFinite(declared) && declared > ARTIFACT_INLINE_EDIT_MAX_BYTES) {
        return tooLarge(declared)
      }
      const blob = await response.blob()
      if (blob.size > ARTIFACT_INLINE_EDIT_MAX_BYTES) return tooLarge(blob.size)
      const content = await blob.text()
      return { ...ok('gateway-fallback'), content, bytes: blob.size }
    } catch (err) {
      return { ...fail('error', 'gateway-fallback', errorMessage(err)), content: '', bytes: 0 }
    }
  }

  /**
   * Persist edited text. `expectedSha256` is forwarded when the artifact carries
   * one so the daemon can reject a write that would clobber a newer version —
   * a turn may have regenerated the file while the editor sat open.
   */
  async function saveTextContent(
    artifact: ArtifactPayload,
    content: string,
  ): Promise<ArtifactActionResult> {
    const id = artifactId(artifact)
    if (!id) return fail('error', 'none', '该产物缺少 id，无法保存')

    const params: Record<string, unknown> = { id, content }
    if (artifact?.sha256) params.expectedSha256 = String(artifact.sha256)
    const sessionKey = options.sessionKey?.()
    if (sessionKey) params.sessionKey = sessionKey

    if (rpc.supportsMethod(ARTIFACT_EDIT_INLINE_METHOD)) {
      try {
        await rpc.call(ARTIFACT_EDIT_INLINE_METHOD, params)
        return ok('rpc')
      } catch (err) {
        if (looksUnsupported(err)) {
          rpc.markMethodUnavailable(ARTIFACT_EDIT_INLINE_METHOD)
        } else {
          return fail('error', 'rpc', errorMessage(err))
        }
      }
    }
    const body: Record<string, unknown> = { content }
    if (artifact?.sha256) body.expected_sha256 = String(artifact.sha256)
    return postRest(artifact, 'content', body)
  }

  return {
    openInSystem,
    loadTextContent,
    saveTextContent,
  }
}
