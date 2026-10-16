import { describe, expect, it } from 'vitest'

import {
  GATEWAY_ROUTING_MODES,
  MODEL_ROUTING_BRIDGE,
  MODEL_ROUTING_MODES,
  type GatewayRoutingMode,
  type ModelRoutingMode,
  fromGatewayRoutingMode,
  fromRuntimeModeKind,
  isModelRoutingMode,
  normalizeModelRoutingMode,
  toGatewayRoutingMode,
  toRuntimeModeKind,
} from './modelRouting'

/**
 * 跨层契约：前端可选路由档位 ⊆ 网关（Rust）可识别档位。
 *
 * 为什么值得一个专门文件：这条契约**曾经只存在于人脑和三份手写三元链里**。
 * `models.routing.set` 在 Rust 侧是白名单校验（`INVALID_REQUEST`），前端档位一旦
 * 映射不到白名单内，UI 上就表现为「点了没反应」——而 `useChatFeatureToggles`
 * 曾把同一套映射**抄了三遍**，任一处漂移都会造成「界面显示已切换、后端实际没变」的
 * 静默失效。现在映射收敛为 `MODEL_ROUTING_BRIDGE` 单表，此文件把跨层契约钉死。
 */

describe('modelRouting · 档位词表', () => {
  it('暴露 5 个 UI 档位', () => {
    expect([...MODEL_ROUTING_MODES].sort()).toEqual([
      'bloom_ensemble',
      'llm_ensemble',
      'off',
      'omni_agent',
      'squilla_router',
    ])
  })

  it('桥接表为每个 UI 档位且**仅为**这些档位各配一条', () => {
    // 新增 UI 档位却忘了补桥接表时，这里先红（TS 的 Record 也已编译期兜底）。
    expect(Object.keys(MODEL_ROUTING_BRIDGE).sort()).toEqual(
      [...MODEL_ROUTING_MODES].sort(),
    )
  })
})

describe('modelRouting · 跨层契约（UI → 网关）', () => {
  it('每个 UI 档位都映射到网关白名单内（否则 RPC 会 INVALID_REQUEST）', () => {
    const accepted = new Set<GatewayRoutingMode>(GATEWAY_ROUTING_MODES)
    for (const mode of MODEL_ROUTING_MODES) {
      const gateway = toGatewayRoutingMode(mode)
      expect(
        accepted.has(gateway),
        `UI 档位 ${mode} → ${gateway} 不在网关白名单内`,
      ).toBe(true)
    }
  })

  it('映射满射且无歧义：5 个 UI 档位对应 5 个不同网关档位', () => {
    const gateways = MODEL_ROUTING_MODES.map(toGatewayRoutingMode)
    expect(new Set(gateways).size).toBe(MODEL_ROUTING_MODES.length)
  })

  it('运行时 kind 只联动 bloom / omni，其余回落 agent', () => {
    expect(toRuntimeModeKind('bloom_ensemble')).toBe('bloom')
    expect(toRuntimeModeKind('omni_agent')).toBe('omni-transcend')
    for (const mode of ['off', 'squilla_router', 'llm_ensemble'] as ModelRoutingMode[]) {
      expect(toRuntimeModeKind(mode)).toBe('agent')
    }
  })
})

describe('modelRouting · 反向映射', () => {
  it('往返无损：UI → 网关 → UI 必须回到原值', () => {
    for (const mode of MODEL_ROUTING_MODES) {
      expect(
        fromGatewayRoutingMode(toGatewayRoutingMode(mode)),
        `往返后应仍在 ${mode}`,
      ).toBe(mode)
    }
  })

  it('未知网关档位返回 null，而不是静默归一化', () => {
    // 后端升级出新档位而前端没跟上时，调用方需要知道"没有对应项"，
    // 若静默返回某个默认项，用户会在错误档位下工作而不自知。
    expect(fromGatewayRoutingMode('quantum-fusion')).toBeNull()
    expect(fromGatewayRoutingMode('')).toBeNull()
    expect(fromGatewayRoutingMode(undefined)).toBeNull()
    expect(fromGatewayRoutingMode(42)).toBeNull()
  })

  it('运行时 kind 反推为 UI 档位', () => {
    expect(fromRuntimeModeKind('bloom')).toBe('bloom_ensemble')
    expect(fromRuntimeModeKind('omni-transcend')).toBe('omni_agent')
    // agent 同时对应多个 UI 档位，只能给出其中之一：这里取第一个（off）。
    expect(fromRuntimeModeKind('agent')).toBe('off')
    expect(fromRuntimeModeKind('plan')).toBeNull()
  })
})

describe('modelRouting · 归一化', () => {
  it('合法值原样返回', () => {
    for (const mode of MODEL_ROUTING_MODES) {
      expect(normalizeModelRoutingMode(mode)).toBe(mode)
    }
  })

  it('非法/缺失值回落 omni_agent（与既有默认策略一致）', () => {
    expect(normalizeModelRoutingMode(undefined)).toBe('omni_agent')
    expect(normalizeModelRoutingMode('nope')).toBe('omni_agent')
    expect(normalizeModelRoutingMode('')).toBe('omni_agent')
    expect(normalizeModelRoutingMode(null)).toBe('omni_agent')
  })

  it('isModelRoutingMode 只认白名单内的字符串', () => {
    expect(isModelRoutingMode('off')).toBe(true)
    expect(isModelRoutingMode('router')).toBe(false)
    expect(isModelRoutingMode('direct')).toBe(false)
    expect(isModelRoutingMode(1)).toBe(false)
    expect(isModelRoutingMode(null)).toBe(false)
  })
})
