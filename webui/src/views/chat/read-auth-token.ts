// 纯函数：从 sessionStorage 读取运行模式鉴权 token。
// 从 views/ChatView.vue 的 `readAuthToken` 抽出——无响应式依赖，仅访问浏览器会话存储，
// 访问失败时（如非浏览器环境）安全回退为空串。
export function readAuthToken(): string {
  try {
    return sessionStorage.getItem('marguerite.wsToken') || ''
  } catch {
    return ''
  }
}
