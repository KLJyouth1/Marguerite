// 必须是第一个 import：router/index.ts 在模块顶层就 getPlatform() 并捕走结果，
// 晚于 import 求值安装桥会被永久固化成 web 平台。详见该文件头部注释。
import './platform/desktopBridgeBoot'
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import i18n from './i18n'
import { migrateLegacyStorageKeys } from './utils/storageMigration'
import { installAppControl } from './utils/appControl'
import { installAttentionLayer } from './components/ui/registerAttentionLayer'
import { installProvenanceDrawer } from './components/shell/registerProvenance'
import { useAppStore } from './stores/app'
import { useRpcStore } from './stores/rpc'
import { startBootWatchdog } from './shell/boot/watchdog'
import 'katex/dist/katex.min.css'
import './assets/base.css'
import './assets/boot.css' // boot splash: first-paint → mount handoff
import './themes/tokens' // eagerly bundles every value theme's token block
import './styles/control-visual-system.css'
import './styles/route-fx.css'
import './styles/chat-markdown.css'
import './styles/ink-typeset.css' // Track F: ink/CLReq typography layer (ink theme only)
import './styles/chat-shared.css'
import './styles/apple-modern.css'
// 注意力系统：货架的布局 + 因果动效语态（tokens 全走 foundation.css）。
import './styles/attention-system.css'

// 桌面桥已在首个 import（desktopBridgeBoot）中安装完毕，此处不再重复。

// 旧版 marguerite.* 存储键 → marguerite.* 一次性迁移，必须在任何读取
// storage 的模块（appStore.initTheme / rpcStore.init）初始化之前执行。
migrateLegacyStorageKeys()

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.use(i18n)

// 应用自控：把「枚举/点击/填写/按键/逐路由巡检」挂到 window.MargueriteAppControl，
// 供页面内面板与外部浏览器自动化共用（router 就绪后安装，避免循环依赖）。
installAppControl(router)

// 注意力货架：注册进 shell.overlay 插槽（幂等，HMR 重跑不会重复注册）。
installAttentionLayer()
// 记忆溯源抽屉：同一个 shell.overlay 插槽，独立的 append-only 贡献（幂等）。
installProvenanceDrawer()

const appStore = useAppStore()
appStore.initTheme()

const rpcStore = useRpcStore()
rpcStore.init()
router.afterEach(() => {
  rpcStore.applyLinkTokenFromUrl()
})

// Resolve + load the active locale before mounting so the first paint is
// already in the right language (no English flash). initLocale never rejects
// (it falls back to en internally); finally() guarantees the app still mounts.
appStore.initLocale().finally(() => {
  app.mount('#app')
})

// Safety net: if the app never mounts (failed locale chunk, a throwing import, a
// half-served bundle), replace the stuck splash with a recovery card instead of
// leaving the user on a dead screen. Armed synchronously right after the mount
// attempt is queued, so it cannot race the normal path.
startBootWatchdog()
