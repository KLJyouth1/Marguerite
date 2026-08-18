import { ref } from 'vue'

const CURRENCY_KEY = 'marguerite-currency'
const RANGE_KEY = 'Marguerite-usage-range'

export function normalizeUsageRange(range: string | null): string {
  const value = String(range || '7')
  return ['all', '7', '14', '30'].includes(value) ? value : '7'
}

export function useUsagePreferences() {
  const currency = ref(localStorage.getItem(CURRENCY_KEY) || 'USD')
  const range = ref(normalizeUsageRange(localStorage.getItem(RANGE_KEY)))

  function setCurrency(cur: string) {
    if (cur !== 'USD' && cur !== 'CNY') return
    currency.value = cur
    localStorage.setItem(CURRENCY_KEY, currency.value)
  }

  function setRange(nextRange: string) {
    range.value = normalizeUsageRange(nextRange)
    localStorage.setItem(RANGE_KEY, range.value)
  }

  return {
    currency,
    range,
    setCurrency,
    setRange,
  }
}
