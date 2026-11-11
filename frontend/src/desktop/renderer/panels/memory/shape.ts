// Hafıza panelinin saf yardımcıları (node --test ile sınanır).

/** "a.rs, Auth::login\nb.rs" → ['a.rs', 'Auth::login', 'b.rs'] (boşlar ve tekrarlar atılır, en fazla 50). */
export function parseRefs(text: string): string[] {
  const out: string[] = []
  for (const part of text.split(/[,\n]/)) {
    const r = part.trim().replace(/^`|`$/g, '').trim()
    if (r && !out.includes(r)) out.push(r)
  }
  return out.slice(0, 50)
}

export interface ProjectChoice {
  value: string
  label: string
}

/** Yolun son parçası (`C:/x/kor-agent/` → `kor-agent`). */
export function baseName(path: string): string {
  const parts = path.replace(/\\/g, '/').split('/').filter(Boolean)
  return parts[parts.length - 1] ?? path
}

/** Proje süzgeci: açık proje önce, sonra çalışma alanları ve bağlı projeler; yollar `/` ile karşılaştırılır. */
export function projectChoices(current: string | null, workspaces: readonly string[], linked: readonly string[]): ProjectChoice[] {
  const seen = new Set<string>()
  const out: ProjectChoice[] = []
  for (const p of [...(current ? [current] : []), ...workspaces, ...linked]) {
    const key = p.replace(/\\/g, '/').replace(/\/$/, '')
    if (!key || seen.has(key.toLowerCase())) continue
    seen.add(key.toLowerCase())
    out.push({ value: key, label: p === current ? `${baseName(key)} (bu proje)` : baseName(key) })
  }
  return out
}

/** Unix saniye → `YYYY-MM-DD` (UTC; kayıtların dışa aktarımıyla aynı). */
export function recordDate(ts: number): string {
  return Number.isFinite(ts) && ts > 0 ? new Date(ts * 1000).toISOString().slice(0, 10) : ''
}
