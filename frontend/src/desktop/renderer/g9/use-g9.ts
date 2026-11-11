import useSWR from 'swr'
import { api } from '../api.ts'
import { uiLocale } from '../ui-locale.ts'

export const PROVIDERS_KEY = 'g9:providers'

export function useProviders() {
  return useSWR(PROVIDERS_KEY, () => api.g9.getProviders(), { revalidateOnFocus: false })
}

/** "$0,0012" gibi; küçük tutarlarda 4 ondalık. */
export function formatCost(usd: number | null): string {
  if (usd === null) return '—'
  const digits = usd !== 0 && Math.abs(usd) < 0.01 ? 4 : 2
  return usd.toLocaleString(uiLocale(), { style: 'currency', currency: 'USD', minimumFractionDigits: digits, maximumFractionDigits: digits })
}
