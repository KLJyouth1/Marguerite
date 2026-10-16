// shipped gate 报告共享类型（LocalModelsView 与 EngineHealthView 两处复用同一渲染组件）。
//
// 与后端 `shipped_gate::GateReport` 对齐：findings 为六要素逐项结论，blocking 为出现
// Fail 的要素，passed 为「无 Fail」即通过（仅 Warn 不阻断）。
// 注意：`model` 在 `models.gate.check` 响应中带回，但 `models.profiles.add` 的 gate 载荷
// 可能不带，故设为可选，组件据此兼容两种来源。

export interface GateFinding {
  element: string
  verdict: 'pass' | 'warn' | 'fail'
  detail: string
}

export interface GateReport {
  /** 关联模型 id（models.gate.check 返回；models.profiles.add 的 gate 载荷可能不带） */
  model?: string
  passed: boolean
  blocking: string[]
  findings: GateFinding[]
}
