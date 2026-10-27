// 纯函数：从 sessionStorage 读取运行模式鉴权 token。
// 从 views/ChatView.vue 的 `readAuthToken` 抽出——无响应式依赖，仅访问浏览器会话存储，
// 访问失败时（如非浏览器环境）安全回退为空串。
//
// 兜底：网关随同源 index.html 注入的 `#Marguerite-data[data-ws-token]`
// （见 core/src/gateway/server.rs maybe_inject_index）。rpc store 首次 init 后
// 才会把它写进 sessionStorage；REST 调用若早于这一刻，只读 sessionStorage
// 会拿到空串并被网关 401。
function injectedToken(): string {
  try {
    return document.getElementById('Marguerite-data')?.getAttribute('data-ws-token') || ''
  } catch {
    return ''
  }
}

export function readAuthToken(): string {
  try {
    return sessionStorage.getItem('marguerite.wsToken') || injectedToken()
  } catch {
    return injectedToken()
  }
}
