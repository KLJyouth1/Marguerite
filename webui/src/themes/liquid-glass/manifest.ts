import type { ThemeManifest } from '../types'

/**
 * 液态玻璃 · 流光（value theme）。
 *
 * 设计取向：深色基底 + 极光蓝/薄荷两种主色 + 大圆角磨砂玻璃。
 * 层次不靠堆元素，靠**留白 / 字重 / 字号 / 玻璃叠层**区分；
 * 全部视觉由令牌驱动，`world.css` 只消费 `--glass-*`，因此参数面板能实时调参。
 */
const liquidGlass: ThemeManifest = {
  id: 'liquid-glass',
  name: '液态玻璃 · 流光',
  kind: 'value',
  icon: 'droplet',
  world: {
    // 无外部字体文件：展示字走系统几何无衬线栈（Inter Tight / Geist / SF Pro），
    // 避免为主题额外打包 100KB+ 字体，也避免首屏 FOUT。
    styles: () => import('./world.css'),
  },
  capabilities: { colorScheme: 'dark', userSelectable: true, respectsReducedMotion: true },
}

export default liquidGlass
