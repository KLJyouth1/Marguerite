// @vitest-environment happy-dom
// candidateModels store 的解析契约：候选真源来自 config.get，
// 后端 Rust 兜底同时输出 model_routing（camel/snake）与 modelRouting（camel）。
// 这里用纯函数 extractCandidateModels 覆盖三种形态，保证前端读哪个键都一致。
import { describe, expect, it } from 'vitest'
import { extractCandidateModels } from './candidateModels'

describe('extractCandidateModels', () => {
  it('从 model_routing.candidateModels 读取候选 ID', () => {
    const cfg = {
      model_routing: {
        mode: 'router',
        candidateModels: ['p-ollama-qwen', 'deepseek-v4-pro'],
        candidate_models: ['p-ollama-qwen', 'deepseek-v4-pro'],
      },
    }
    expect(extractCandidateModels(cfg)).toEqual(['p-ollama-qwen', 'deepseek-v4-pro'])
  })

  it('回退到 model_routing.candidate_models（snake 形态）', () => {
    const cfg = {
      model_routing: {
        candidate_models: ['p-ollama-qwen'],
      },
    }
    expect(extractCandidateModels(cfg)).toEqual(['p-ollama-qwen'])
  })

  it('从 modelRouting.candidateModels（旧 camel 形态）读取', () => {
    const cfg = {
      modelRouting: {
        candidateModels: ['deepseek-v4-pro'],
        crossProviderTiers: {},
      },
    }
    expect(extractCandidateModels(cfg)).toEqual(['deepseek-v4-pro'])
  })

  it('candidateModels 优先于 candidate_models（两者并存时）', () => {
    const cfg = {
      model_routing: {
        candidateModels: ['p-ollama-qwen'],
        candidate_models: ['legacy-id'],
      },
    }
    expect(extractCandidateModels(cfg)).toEqual(['p-ollama-qwen'])
  })

  it('无路由段 / 空列表 / 非数组时安全返回空数组', () => {
    expect(extractCandidateModels({})).toEqual([])
    expect(extractCandidateModels(null)).toEqual([])
    expect(extractCandidateModels('nope')).toEqual([])
    expect(extractCandidateModels({ model_routing: {} })).toEqual([])
    expect(extractCandidateModels({ model_routing: { candidateModels: 'not-array' } })).toEqual([])
    // 只保留字符串元素，丢弃脏数据
    expect(extractCandidateModels({ model_routing: { candidateModels: ['ok-id', 42, null] } })).toEqual(['ok-id'])
  })
})