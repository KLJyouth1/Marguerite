import type { RouteRecordRaw } from 'vue-router'

// Desktop-only routes.
// V2.1-P2：pet-overlay 悬浮窗独立视图（Tauri 壳加载）：透明背景 + 独立 3D 画布，
// 不挂侧栏/顶栏，无导航分组元数据（保持不可见于侧边导航，符合 WebUI 导航契约）。
const PetOverlayView = () => import('@/views/PetOverlayView.vue')

export const desktopRoutes: RouteRecordRaw[] = [
  {
    path: '/pet-overlay',
    name: 'pet-overlay',
    component: PetOverlayView,
    meta: {
      title: 'Marguerite Pet',
      platforms: ['web', 'desktop'],
      viewKey: 'pet-overlay',
      routeTransition: 'none',
      // 不设置 meta.group：R5 WebUI 导航分组契约保证不出现于侧栏
    },
  },
]

