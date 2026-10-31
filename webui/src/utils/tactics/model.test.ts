import { describe, expect, it } from 'vitest'
import {
  DEFAULT_TRIALS,
  MAX_TRIALS,
  buildSimulateParams,
  defaultScenarioForm,
  formatDeltaPct,
  formatMs,
  formatPct,
  formatTokens,
  formatUsd,
  parseDeps,
  recommendedResult,
  topologyKey,
  validateScenarioForm,
  whyKindKey,
} from './model'

describe('parseDeps', () => {
  it('splits on commas and whitespace, dropping blanks and duplicates in order', () => {
    expect(parseDeps(' a, b  a  c ')).toEqual(['a', 'b', 'c'])
    expect(parseDeps('')).toEqual([])
  })
})

describe('validateScenarioForm', () => {
  it('accepts the default scenario', () => {
    expect(validateScenarioForm(defaultScenarioForm())).toEqual([])
  })

  it('reports every problem, not just the first', () => {
    const form = defaultScenarioForm()
    form.tasks = [
      { id: 'a', workK: '10', dependsOn: '' },
      { id: 'a', workK: '10', dependsOn: '' },
      { id: '', workK: '10', dependsOn: '' },
      { id: 'b', workK: '-1', dependsOn: 'ghost' },
    ]
    const codes = validateScenarioForm(form).map((issue) => issue.code)
    expect(codes).toContain('duplicate-id')
    expect(codes).toContain('missing-id')
    expect(codes).toContain('unknown-dependency')
    expect(codes).toContain('bad-number')
  })

  it('rejects out-of-range numbers instead of silently clamping them', () => {
    const form = defaultScenarioForm()
    form.maxFailProbability = '1.5'
    form.priors.retryBackoff = '0.9'
    form.seed = '1.2'
    const fields = validateScenarioForm(form)
      .filter((issue) => issue.code === 'bad-number')
      .map((issue) => issue.field)
    expect(fields).toEqual(['maxFailProbability', 'seed', 'priors.retryBackoff'])
  })

  it('treats blank optional numbers as "use the backend prior"', () => {
    const form = defaultScenarioForm()
    form.budgetTokens = ''
    form.deadlineMs = '   '
    form.priors.failureRate = ''
    expect(validateScenarioForm(form)).toEqual([])
  })
})

describe('buildSimulateParams', () => {
  it('sends the DAG, omits empty optional fields, and keeps defaults server-side', () => {
    const form = defaultScenarioForm()
    const params = buildSimulateParams(form)
    expect(params.tasks).toEqual([
      { id: 'plan', workK: 20 },
      { id: 'impl-a', workK: 40, dependsOn: ['plan'] },
      { id: 'impl-b', workK: 40, dependsOn: ['plan'] },
      { id: 'verify', workK: 30, dependsOn: ['impl-a', 'impl-b'] },
    ])
    expect(params).not.toHaveProperty('budgetTokens')
    expect(params).not.toHaveProperty('deadlineMs')
    expect(params).not.toHaveProperty('maxFailProbability')
    expect(params).not.toHaveProperty('priors')
    expect(params.trials).toBe(DEFAULT_TRIALS)
    expect(params.seed).toBe(42)
  })

  it('clamps trials to the engine bounds (documented exception)', () => {
    const form = defaultScenarioForm()
    form.trials = '999999'
    expect(buildSimulateParams(form).trials).toBe(MAX_TRIALS)
    form.trials = '0'
    expect(buildSimulateParams(form).trials).toBe(1)
  })

  it('forwards only valid priors so an out-of-range value never masquerades as a prior', () => {
    const form = defaultScenarioForm()
    form.priors.failureRate = '0.2'
    form.priors.throughputKPerMin = '-3'
    form.priors.pricePerMTokUsd = '1.5'
    expect(buildSimulateParams(form).priors).toEqual({
      failureRate: 0.2,
      pricePerMTokUsd: 1.5,
    })
  })

  it('drops blank-id rows rather than sending an invalid DAG', () => {
    const form = defaultScenarioForm()
    form.tasks = [
      { id: 'solo', workK: '', dependsOn: '' },
      { id: '   ', workK: '10', dependsOn: '' },
    ]
    expect(buildSimulateParams(form).tasks).toEqual([{ id: 'solo' }])
  })
})

describe('formatting', () => {
  it('formats durations at three magnitudes', () => {
    expect(formatMs(340)).toBe('340 ms')
    expect(formatMs(12_300)).toBe('12.3 s')
    expect(formatMs(125_000)).toBe('2m 05s')
    expect(formatMs(undefined)).toBe('—')
    expect(formatMs(Number.NaN)).toBe('—')
  })

  it('formats tokens with k/M suffixes', () => {
    expect(formatTokens(980)).toBe('980')
    expect(formatTokens(12_400)).toBe('12.4k')
    expect(formatTokens(2_500_000)).toBe('2.50M')
    expect(formatTokens(-1)).toBe('—')
  })

  it('formats success rates and signed deltas', () => {
    expect(formatPct(0.923)).toBe('92%')
    expect(formatPct(undefined)).toBe('—')
    expect(formatDeltaPct(-0.2)).toBe('-20%')
    expect(formatDeltaPct(0.15)).toBe('+15%')
    expect(formatDeltaPct(0)).toBe('0%')
  })

  it('formats sub-cent costs honestly instead of rounding them to $0.00', () => {
    expect(formatUsd(0)).toBe('$0')
    expect(formatUsd(0.004)).toBe('<$0.01')
    expect(formatUsd(1.234)).toBe('$1.23')
  })
})

describe('label mapping', () => {
  it('maps known why-kinds and topologies, null otherwise', () => {
    expect(whyKindKey('evidence')).toBe('tactics.replay.kind.evidence')
    expect(whyKindKey('vibes')).toBeNull()
    expect(topologyKey('swarm')).toBe('tactics.topology.swarm')
    expect(topologyKey(undefined)).toBeNull()
  })
})

describe('recommendedResult', () => {
  const plan = {
    rationale: { recommended: 'star-gated' },
    results: [
      { tactic: 'solo', successRate: 0.8 },
      { tactic: 'star-gated', successRate: 0.95 },
    ],
  }

  it('returns the row named by the rationale, never the best-looking row', () => {
    expect(recommendedResult(plan)?.tactic).toBe('star-gated')
  })

  it('returns null when there is no recommendation', () => {
    expect(recommendedResult(null)).toBeNull()
    expect(recommendedResult({ rationale: { noViableTactic: true } })).toBeNull()
  })
})
