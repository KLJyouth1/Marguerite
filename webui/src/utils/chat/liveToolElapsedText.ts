// Live-turn tool elapsed readout（从 ChatView 抽取的纯函数）。
//
// 实时轮的工具耗时读整秒（"4s"），与 streamPhaseElapsed / streamThinkingElapsedText
// 一致。共享工具格式化器（streamToolElapsedText, useChatStream.ts）输出十分位，
// 这里在调用点归一化 —— 但亚秒级已完成的工具保留十分位，避免读出无意义的 "0s"。

/**
 * 把 `streamToolElapsedText` 的输出归一化为整秒（>=1s 时），亚秒保留十分位。
 * 纯函数：不依赖任何组件状态，便于单测。
 */
export function liveToolElapsedText(
  call: { toolId: string },
  streamToolElapsedText: (c: { toolId: string }) => string,
): string {
  return streamToolElapsedText(call).replace(/^([1-9]\d*)\.\d+s$/, '$1s')
}
