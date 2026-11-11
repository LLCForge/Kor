// §28.15 MCP denetleyicisi: JSON-RPC trafiğinin tek satırlık özeti ve sınırlı canlı günlük.
import type { McpTraffic } from '../../../ipc-ek14.ts'

const EXPLAIN: Record<number, string> = {
  [-32700]: 'ayrıştırma hatası: sunucu JSON olmayan metin aldı',
  [-32600]: 'geçersiz istek',
  [-32601]: 'yöntem yok: sunucu bunu desteklemiyor (yeteneklerine bakın)',
  [-32602]: 'geçersiz argüman: inputSchema ile uyuşmuyor',
  [-32603]: 'iç hata: sunucu çağrıyı işlerken hata verdi (stderr\'e bakın)',
}

export function explain(code: number): string {
  return EXPLAIN[code] ?? (code >= -32099 && code <= -32000 ? 'sunucu hatası (stderr\'e bakın)' : 'sunucunun döndürdüğü uygulama hatası')
}

/** `→ tools/call #3`, `← #3 tamam`, `← #4 hata -32602 …`. */
export function summarize(t: McpTraffic): string {
  if (t.dir === 'stderr') return `stderr: ${t.text}`
  if (t.dir === 'note') return `! ${t.text}`
  let v: { id?: unknown; method?: string; error?: { code?: number; message?: string } }
  try {
    v = JSON.parse(t.text) as typeof v
  } catch {
    return t.text
  }
  if (t.dir === 'out') return `→ ${v.method ?? '?'}${typeof v.id === 'number' ? ` #${v.id}` : ''}`
  if (v.error) return `← #${String(v.id)} hata ${v.error.code ?? ''} ${v.error.message ?? ''} (${explain(v.error.code ?? 0)})`
  if (v.method) return `← ${v.method}`
  return `← #${String(v.id)} tamam`
}

/** Canlı günlük en fazla `max` satır tutar. */
export function append(log: readonly McpTraffic[], t: McpTraffic, max = 500): McpTraffic[] {
  const next = [...log, t]
  return next.length > max ? next.slice(next.length - max) : next
}
