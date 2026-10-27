// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import {
  createEditorWorkbenchItem,
  EDITOR_WORKBENCH_OPEN_EVENT,
  editorPathFromWorkbenchItem,
  editorScopeFromWorkbenchItem,
  editorTitleForPath,
  normalizeEditorPath,
  requestEditorWorkbenchOpen,
  type EditorWorkbenchOpenEventDetail,
} from './editorItems'

/**
 * 编辑器工作台项的契约：路径归一必须 fail-closed（越界由后端再复核），
 * 同一会话同一文件必须复用同一标签 id，不同文件各占一个标签。
 */
describe('editor workbench items', () => {
  it('归一：去掉 ./ 与前导斜杠，反斜杠统一', () => {
    expect(normalizeEditorPath('./src/a.ts')).toBe('src/a.ts')
    expect(normalizeEditorPath('\\src\\a.ts')).toBe('src/a.ts')
    expect(normalizeEditorPath('/src/a.ts')).toBe('src/a.ts')
    expect(normalizeEditorPath('  src/a.ts  ')).toBe('src/a.ts')
  })

  it('归一：空串与含 .. 的越界写法一律拒绝', () => {
    expect(normalizeEditorPath('')).toBe('')
    expect(normalizeEditorPath('   ')).toBe('')
    expect(normalizeEditorPath('../etc/passwd')).toBe('')
    expect(normalizeEditorPath('src/../../x')).toBe('')
    expect(normalizeEditorPath('a'.repeat(2000))).toBe('')
  })

  it('建项：kind=file、DOM 宿主、标题取文件名', () => {
    const item = createEditorWorkbenchItem({ scopeId: 'session-a', path: 'webui/src/App.vue' })!
    expect(item.kind).toBe('file')
    expect(item.hostKind).toBe('dom')
    expect(item.title).toBe('App.vue')
    expect(editorPathFromWorkbenchItem(item)).toBe('webui/src/App.vue')
    expect(editorScopeFromWorkbenchItem(item)).toBe('session-a')
    expect(item.payload.projectPath).toBeUndefined()
  })

  it('建项：缺 path / scope 返回 null（不产生空标签）', () => {
    expect(createEditorWorkbenchItem({ scopeId: 's', path: '' })).toBeNull()
    expect(createEditorWorkbenchItem({ scopeId: '  ', path: 'a.ts' })).toBeNull()
  })

  it('同一文件复用同一 id，不同文件不同 id', () => {
    const a1 = createEditorWorkbenchItem({ scopeId: 's', path: 'a.ts' })!
    const a2 = createEditorWorkbenchItem({ scopeId: 's', path: './a.ts' })!
    const b = createEditorWorkbenchItem({ scopeId: 's', path: 'b.ts' })!
    expect(a1.id).toBe(a2.id)
    expect(a1.id).not.toBe(b.id)
  })

  it('projectPath 只在显式提供时进入 payload（不写空值污染标签）', () => {
    const withProject = createEditorWorkbenchItem({
      scopeId: 's',
      path: 'a.ts',
      projectPath: 'D:/Downloads/Marguerite',
    })!
    expect(withProject.payload.projectPath).toBe('D:/Downloads/Marguerite')
    const blank = createEditorWorkbenchItem({ scopeId: 's', path: 'a.ts', projectPath: '   ' })!
    expect(blank.payload.projectPath).toBeUndefined()
  })

  it('文件名提取兜底：无分隔符时就是自身', () => {
    expect(editorTitleForPath('Cargo.toml')).toBe('Cargo.toml')
    expect(editorTitleForPath('a/b/c.rs')).toBe('c.rs')
  })

  it('打开事件必须带上 projectPath（丢了它就会打开同名但不同目录的文件）', () => {
    const seen: EditorWorkbenchOpenEventDetail[] = []
    const listener = (event: Event) => {
      seen.push((event as CustomEvent<EditorWorkbenchOpenEventDetail>).detail)
    }
    window.addEventListener(EDITOR_WORKBENCH_OPEN_EVENT, listener)
    try {
      expect(requestEditorWorkbenchOpen({
        scopeId: 's',
        path: 'D:/work/proj/src/main.rs',
        projectPath: 'D:/work/proj',
      })).toBe(true)
      expect(seen).toHaveLength(1)
      expect(seen[0].path).toBe('D:/work/proj/src/main.rs')
      expect(seen[0].projectPath).toBe('D:/work/proj')
    } finally {
      window.removeEventListener(EDITOR_WORKBENCH_OPEN_EVENT, listener)
    }
  })
})
