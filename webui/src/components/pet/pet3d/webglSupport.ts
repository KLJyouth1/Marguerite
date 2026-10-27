/**
 * WebGL 能力探测（**不引入 three**，可在首帧同步调用）。
 *
 * 为什么单独成一个模块：`isWebGLAvailable` 原先住在 `Pet3DScene.ts`，而那个文件
 * 顶部 `import * as THREE from 'three'`。PetView 为了调它只能 `await import(...)`，
 * 于是探测结果在 ~200ms 后才回来，页面先按「不支持 WebGL」渲染（显示回退提示、
 * 隐藏视角按钮），探测完再切换 → 桌宠卡上移 29px（真机 Layout Instability 实测
 * 0.0077，发生在加载后约 200ms）。
 *
 * 把纯探测抽到这里后，PetView 可以在 setup 里同步判定，首帧即为终态；
 * three 仍然只在真正需要 3D 场景时才动态加载。
 */
export function isWebGLAvailable(): boolean {
  try {
    const canvas = document.createElement('canvas')
    const ctx = canvas.getContext('webgl2') || canvas.getContext('webgl')
    return !!ctx
  } catch {
    return false
  }
}
