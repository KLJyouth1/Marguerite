import { describe, expect, it } from 'vitest'
import type { ChatToolCall } from '@/types/chat'
import {
  TOOL_PAYLOAD_MAX_CHARS,
  computerUseEventsFromToolCalls,
  countDiffLines,
  diffLineTone,
  normalizeSessionFileChanges,
  summarizeToolArgs,
  viewToolPayload,
} from './toolPayload'

/**
 * 真机样本（2026-10-23 截图）：
 *   mcp_research {"query":"Three.js floating island 3D scene tutorial low poly",
 *                 "max_results":3,"mode":"search"}
 *   shell_exec   {"command":"node --version; ls"}
 * 此前卡片只显示一行「名字 + 参数预览」，输出完全不渲染。
 */
describe('toolPayload — 判型与美化', () => {
  it('JSON 参数 → 两空格缩进的对象，类型判为 json', () => {
    const view = viewToolPayload('{"query":"three.js island","max_results":3,"mode":"search"}')
    expect(view.kind).toBe('json')
    expect(view.body).toContain('\n  "query": "three.js island"')
    expect(view.body.split('\n')).toHaveLength(5)
    expect(view.truncated).toBe(false)
  })

  it('空载荷（含 {} / [] / "" / null / 空白）统一判为 empty', () => {
    for (const raw of ['', '   ', '{}', '[]', '""', 'null', undefined, null]) {
      expect(viewToolPayload(raw).kind, `raw=${String(raw)}`).toBe('empty')
    }
    expect(viewToolPayload('{"a":1}').kind).toBe('json')
  })

  it('非字符串入参按 JSON 处理（后端有时直接给对象）', () => {
    const view = viewToolPayload({ command: 'node --version; ls', cwd: 'D:/work' })
    expect(view.kind).toBe('json')
    expect(view.body).toContain('"command": "node --version; ls"')
  })

  it('diff 判型 + 逐行色调', () => {
    const diff = '--- a/app.ts\n+++ b/app.ts\n@@ -1,2 +1,2 @@\n-old\n+new\n same'
    const view = viewToolPayload(diff)
    expect(view.kind).toBe('diff')
    expect(diffLineTone('@@ -1 +1 @@')).toBe('hunk')
    expect(diffLineTone('+++ b/app.ts')).toBe('meta')
    expect(diffLineTone('--- a/app.ts')).toBe('meta')
    expect(diffLineTone('+new')).toBe('add')
    expect(diffLineTone('-old')).toBe('del')
    expect(diffLineTone(' same')).toBe('context')
  })

  it('单个 URL 判为 link（组件渲染成可点链接）', () => {
    const view = viewToolPayload('https://threejs.org/examples/#webgl_geometry_terrain')
    expect(view.kind).toBe('link')
    expect(view.body).toBe('https://threejs.org/examples/#webgl_geometry_terrain')
  })

  it('多行代码（含 import/分号）判为 code，普通多行文本仍是 text', () => {
    const code = "import * as THREE from 'three';\nconst scene = new THREE.Scene();\nrenderer.render(scene, camera);"
    expect(viewToolPayload(code).kind).toBe('code')
    const prose = '第一段说明\n第二段说明\n第三段说明'
    expect(viewToolPayload(prose).kind).toBe('text')
  })

  it('超长输出截断并标记 truncated（不把整段网页塞进卡片）', () => {
    const huge = 'x'.repeat(TOOL_PAYLOAD_MAX_CHARS + 500)
    const view = viewToolPayload(huge)
    expect(view.truncated).toBe(true)
    expect(view.body.length).toBe(TOOL_PAYLOAD_MAX_CHARS + 1) // 末尾省略号
    expect(view.bytes).toBe(TOOL_PAYLOAD_MAX_CHARS + 500)
  })

  it('密钥/令牌在进入 UI 前脱敏', () => {
    const view = viewToolPayload('{"api_key":"sk-abcdefgh1234","note":"ok"}')
    expect(view.body).toContain('[redacted]')
    expect(view.body).not.toContain('sk-abcdefgh1234')
  })

  it('头部参数摘要压成一行并限长', () => {
    const summary = summarizeToolArgs('{\n  "query": "three.js",\n  "mode": "search"\n}')
    expect(summary).toBe('{"query":"three.js","mode":"search"}')
    expect(summarizeToolArgs('x'.repeat(300), 20)).toHaveLength(20)
    expect(summarizeToolArgs('{}')).toBe('')
  })
})

function computerUseCall(overrides: Partial<ChatToolCall> = {}): ChatToolCall {
  return {
    toolId: 'tool-1',
    name: 'computer_use',
    displayName: 'computer_use',
    inputPreview: '',
    isRunning: false,
    status: '',
    isError: false,
    result: '',
    resultPreview: '',
    isOpen: false,
    ...overrides,
  }
}

describe('normalizeSessionFileChanges', () => {
  it('只接受有路径的行，缺失字段保持缺失（不猜 kind / 行数）', () => {
    const rows = normalizeSessionFileChanges({
      ok: true,
      changes: [
        { path: 'src/a.ts', kind: 'modify', tool: 'edit_file', added: 3, removed: 1, diff: '@@\n-a\n+b' },
        { path: '', kind: 'create' },
        { file: 'docs/b.md', kind: 'rename', added: '2' },
      ],
    })
    expect(rows).toHaveLength(2)
    expect(rows[0]).toMatchObject({ path: 'src/a.ts', kind: 'modify', added: 3, removed: 1 })
    expect(rows[1]).toMatchObject({ path: 'docs/b.md', kind: '', added: 2, removed: null, diff: '' })
  })

  it('非数组输入返回空集（不把成功响应误当成一组改动）', () => {
    expect(normalizeSessionFileChanges({ ok: true })).toEqual([])
    expect(normalizeSessionFileChanges(null)).toEqual([])
  })

  it('缺 added/removed 时可从统一 diff 统计', () => {
    expect(countDiffLines('--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n+extra')).toEqual({
      added: 2,
      removed: 1,
    })
  })
})

describe('computerUseEventsFromToolCalls', () => {
  it('读取 payload 的动作/目标/成败/审批，并保留真实缺失字段', () => {
    const events = computerUseEventsFromToolCalls([
      computerUseCall({
        toolId: 'cu-1',
        inputRaw: JSON.stringify({ kind: 'computer_use', action: 'click', target: '保存按钮' }),
        result: JSON.stringify({ kind: 'computer_use', action: 'click', target: '保存按钮', ok: true, approval: 'approved' }),
        status: 'success',
      }),
      computerUseCall({
        toolId: 'cu-2',
        inputRaw: JSON.stringify({ kind: 'computer_use', action: 'type', target: '搜索框' }),
      }),
    ])
    expect(events).toHaveLength(2)
    expect(events[0]).toMatchObject({
      action: 'click',
      target: '保存按钮',
      ok: true,
      approval: 'approved',
    })
    expect(events[1]).toMatchObject({
      action: 'type',
      target: '搜索框',
      ok: null,
      approval: null,
    })
  })

  it('ok 缺失时回落到这次调用的真实执行状态；无证据的行不出现', () => {
    const events = computerUseEventsFromToolCalls([
      computerUseCall({ toolId: 'cu-3', status: 'error' }),
      computerUseCall({ toolId: 'cu-4', status: 'success' }),
      computerUseCall({ toolId: 'cu-5', name: 'read_file', status: 'success' }),
    ])
    expect(events.map(event => event.ok)).toEqual([false, true])
    expect(events.map(event => event.id)).toEqual(['cu-3', 'cu-4'])
  })

  it('非 computer_use 工具与非法审批值不进入监管卡', () => {
    const events = computerUseEventsFromToolCalls([
      computerUseCall({
        toolId: 'cu-6',
        result: JSON.stringify({ kind: 'computer_use', action: 'click', ok: true, approval: 'maybe' }),
      }),
      computerUseCall({ toolId: 'cu-7', name: 'shell_exec', status: 'success' }),
    ])
    expect(events).toHaveLength(1)
    expect(events[0].approval).toBeNull()
  })

  it('优先取 ToolCall 事件上的 approval（引擎真实裁决），而非 payload 里的猜测', () => {
    const events = computerUseEventsFromToolCalls([
      // 只有事件上的裁决，payload 完全没有 approval → 仍应渲染 denied。
      computerUseCall({
        toolId: 'cu-8',
        inputRaw: JSON.stringify({ kind: 'computer_use', action: 'click', target: '确定按钮' }),
        status: 'error',
        approval: 'denied',
      }),
      // call 与 payload 冲突时，以引擎裁决（call）为准。
      computerUseCall({
        toolId: 'cu-9',
        approval: 'approved',
        result: JSON.stringify({ kind: 'computer_use', action: 'type', ok: true, approval: 'denied' }),
        status: 'success',
      }),
    ])
    expect(events[0]).toMatchObject({ id: 'cu-8', action: 'click', approval: 'denied' })
    expect(events[1]).toMatchObject({ id: 'cu-9', approval: 'approved' })
  })
})
