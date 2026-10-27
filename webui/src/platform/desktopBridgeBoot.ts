/**
 * 桥接引导：**只做一件事** —— 在模块求值阶段就安装桌面桥。
 *
 * 为什么必须有这个文件：`router/index.ts:22` 在**模块顶层**调用了
 * `getPlatform()` 并把结果存进 `const`。ESM 的求值顺序是"先求值全部 import，
 * 再执行 main.ts 的函数体"，所以把 `installDesktopBridge()` 写在 main.ts 的
 * 语句区已经太晚 —— 路由守卫会永久持有 web 平台引用（按 id 的缓存也救不回来，
 * 因为引用已经被捕走）。
 *
 * 约定：本模块必须作为 `main.ts` 的**第一个 import**，且不得有其它依赖。
 */

import { installDesktopBridge } from './desktopBridge'

installDesktopBridge()
