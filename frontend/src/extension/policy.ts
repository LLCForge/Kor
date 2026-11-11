// Alan adı kuralları (§18.2, ADR 0058): çekirdekteki `kor_tools::chrome::policy` ile aynı anlam. Çekirdek her
// gezinmeden önce denetler; eklenti de her sayfa işleminden önce yeniden denetler (savunma derinliği).

export interface Domains {
  allow: string[]
  deny: string[]
}

export function hostOf(url: string): string | null {
  try {
    const u = new URL(url)
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null
    return u.hostname.toLowerCase().replace(/\.$/, '')
  } catch {
    return null
  }
}

function covers(rule: string, host: string): boolean {
  return host === rule || host.endsWith('.' + rule)
}

/** `null`: izinli; aksi halde neden. */
export function refusal(d: Domains, url: string): string | null {
  const host = hostOf(url)
  if (host === null) return 'Yalnızca http/https sayfaları'
  const denied = d.deny.find((r) => covers(r, host))
  if (denied) return `${host} Kor alan adı listesinde engelli (kor chrome deny ${denied})`
  if (d.allow.length > 0 && !d.allow.some((r) => covers(r, host))) {
    return `${host} Kor izin listesinde değil (kor chrome allow ${host})`
  }
  return null
}

export function parseDomains(v: unknown): Domains {
  const o = (v ?? {}) as { allow?: unknown; deny?: unknown }
  const list = (x: unknown) => (Array.isArray(x) ? x.filter((s): s is string => typeof s === 'string') : [])
  return { allow: list(o.allow), deny: list(o.deny) }
}
