import type { ThemeManifest } from '../types'

// Value theme "marguerite-night" —— 玛格丽特 · 星夜。
//
// 设计来源（只取语汇，不复制任何图像）：
// - **玛格丽特补给站**（Capanna Margherita，罗莎峰 4554m，欧洲最高建筑）：
//   云端之上的星夜、雪脊剪影、小屋暖灯、气象站信号灯；
// - **玛格丽特公主**：Lotus Flower / Cartier Halo 冠冕的弧形语汇、海蓝宝石与钻石的冷闪、
//   象牙色长裙与银紫发色；
// - **玛格丽特花（雏菊）**：白色花瓣 + 金色花心，作为全站的图形母题。
//
// 落地范围：L1 令牌（tokens.css）+ 全局世界层（world.css，含排版/留白/圆角/动效）
// + 星夜画布（`MargueriteSky.vue`，仅在本主题激活时挂载）+ 环境音（`useAmbience`）。
const margueriteNight: ThemeManifest = {
  id: 'marguerite-night',
  // 自定义主题使用专名，不做 i18n（仓库约定：非内置模式直接显示 manifest.name）
  name: '玛格丽特 · 星夜',
  kind: 'value',
  icon: 'sparkles',
  world: {
    fonts: [
      // 复用仓库内已本地化的 Fraunces（衬线展示字），不引入任何远程字体请求
      { family: 'Fraunces', load: () => import('../out-of-register/fonts/fraunces.css') },
    ],
    styles: () => import('./world.css'),
  },
  capabilities: { colorScheme: 'dark', userSelectable: true, respectsReducedMotion: true },
}

export default margueriteNight
