// Kod bloğu başlığı (§23.4): dil + dosya yolu. Yol Markdown çit bilgisinden okunur:
// ```ts:src/a.ts   ```ts title="src/a.ts"   ```ts path=src/a.ts   ```ts src/a.ts (son biçim yalnızca yol benzeriyse).

export interface FenceInfo {
  language: string
  path: string | null
}

const PATHISH = /^[\w.\-@~/\]+\.[\w]+$/

export function parseFence(language: string, meta: string | null | undefined): FenceInfo {
  let lang = language
  let path: string | null = null
  const colon = lang.indexOf(':')
  if (colon > 0) {
    path = lang.slice(colon + 1) || null
    lang = lang.slice(0, colon)
  }
  const m = (meta ?? '').trim()
  if (!path && m) {
    const kv = m.match(/(?:^|\s)(?:title|path|file|filename)=(?:"([^"]+)"|'([^']+)'|(\S+))/)
    if (kv) path = kv[1] ?? kv[2] ?? kv[3] ?? null
    else if (PATHISH.test(m)) path = m
  }
  // Yol yalnızca görüntülenir; yine de kontrol karakteri ve aşırı uzunluk içeri girmez.
  if (path && (path.length > 300 || /[\u0000-\u001f]/.test(path))) path = null
  return { language: lang, path }
}
