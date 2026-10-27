// 布局回归（2026-10-24 真机巡检）：
// `/pet` 的 PSI 面板在 `brain.status` 返回后才出现，把下方皮肤区从 top≈1080
// 推到 1347（约 267px 位移）。第一版用「同结构骨架分支」仍残留 29px 差值
// （分支本身 + 文案换行），最终改为「同一套 DOM，只换文本」。
//
// 这里不挂载整个 PetView（要 WebGL / canvas / rAF，happy-dom 噪声太大），
// 改为对模板与样式做结构契约断言，防止有人改回分支渲染或删掉单行约束。
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const source = readFileSync(fileURLToPath(new URL('./PetView.vue', import.meta.url)), 'utf8')

function templateSection(): string {
  const start = source.indexOf('<template>')
  const end = source.indexOf('</template>')
  expect(start).toBeGreaterThan(-1)
  expect(end).toBeGreaterThan(start)
  return source.slice(start, end)
}

function psiBlocks(): string {
  const tpl = templateSection()
  const start = tpl.indexOf('class="pet-psi"')
  const end = tpl.indexOf('pet-psi__off', start)
  expect(start).toBeGreaterThan(-1)
  expect(end).toBeGreaterThan(start)
  return tpl.slice(start, end)
}

describe('PetView PSI 面板（防布局位移）', () => {
  it('加载态与就绪态共用同一套 DOM（不分支渲染）', () => {
    const tpl = templateSection()
    // 只允许一个 pet-psi 面板节点：出现两个 section 就说明又回到了分支渲染
    const panelTags = tpl.match(/class="pet-psi"/g) ?? []
    expect(panelTags.length).toBe(1)
    // 该节点同时受 psiLoading 与 psiEnabled 控制
    expect(tpl).toContain('v-if="psiLoading || psiEnabled"')
    // 不再存在 v-else-if="psiEnabled" 的第二面板
    expect(tpl).not.toContain('v-else-if="psiEnabled"')
  })

  it('维度行来自 psiDims，加载态也有完整 8 行结构', () => {
    const block = psiBlocks()
    expect(block).toContain('v-for="d in psiDims"')
    expect(block).toContain('pet-psi__bars')
    // 骨架条与真实条共用同一 class，只靠 is-placeholder 区分
    expect(block).toContain("'is-placeholder': !psiEnabled")
  })

  it('meta 行强制单行固定高度，避免文案长度导致换行抖动', () => {
    const css = source.slice(source.indexOf('<style'))
    const metaRule = css.slice(css.indexOf('.pet-psi__meta {'))
    const rule = metaRule.slice(0, metaRule.indexOf('}'))
    expect(rule).toContain('white-space: nowrap')
    expect(rule).toContain('text-overflow: ellipsis')
    expect(rule).toMatch(/height:\s*\d+px/)
    expect(rule).toMatch(/line-height:\s*\d+px/)
  })

  it('轮询失败也会结束加载态，不会永久停留在骨架', () => {
    expect(/finally\s*\{\s*psiLoading\.value = false/.test(source)).toBe(true)
    expect(source).toContain('psiLoadDeadline')
  })
})
