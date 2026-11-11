// Komut paletinin "Tüm komutlar" sayfası için saf yardımcılar (ADR 0031). React/registry içe aktarmaz;
// node testleri doğrudan kullanır.

/**
 * Bir ad alanının panelleri: önce ad alanıyla aynı kimlikli panel, sonra bu eşlemedeki adaylar.
 * Panel yoksa palet genel çalıştırıcıyı açar.
 */
const NS_PANELS: Readonly<Record<string, readonly string[]>> = {
  bench: ['cost'],
  context: ['cost'],
  packages: ['graph'],
  provider: ['providers'],
  catalog: ['providers'],
  proxy: ['providers'],
  components: ['providers'],
  media: ['providers'],
}

export function panelIdsFor(ns: string): readonly string[] {
  return [ns, ...(NS_PANELS[ns] ?? [])]
}

/** Geri alınamayan, sistemi değiştiren ya da ücretli model/API çağrısı yapan yöntemler: çalıştırmadan önce onay. */
export const CONFIRM_METHODS: ReadonlySet<string> = new Set([
  'bench.start',
  'media.falRun',
  'media.falImage',
  'media.hfInfer',
  'media.hfDownload',
  'components.add',
  'components.remove',
  'provider.remove',
  'provider.removeKey',
  'g9.removeKey',
  'memory.forget',
  'memory.unlink',
  'memory.workspaceRemove',
  'memory.notesPush',
  'shell.install',
  'shell.uninstall',
  'shell.fix',
  'shell.why',
  'plugins.install',
  'plugins.uninstall',
  'plugins.update',
  'mcp.remove',
  'routines.remove',
  'core.removeMarketplace',
  'core.undo',
  'core.updateChannel',
  'core.configSet',
  'templates.run',
  'gitflow.commitMessage',
  'gitflow.changelog',
])

/** Bağımsız değişken metni (JSON) üst sınırı: köprüye kaçak büyük yük gitmesin. */
export const MAX_ARGS_CHARS = 64 * 1024

export type ParsedArgs = { ok: true; args: unknown[] } | { ok: false; error: string }

/** Boş → `[]`; dizi → konumsal bağımsız değişkenler; tek değer → `[değer]`. */
export function parseArgs(text: string): ParsedArgs {
  const t = text.trim()
  if (!t) return { ok: true, args: [] }
  if (t.length > MAX_ARGS_CHARS) return { ok: false, error: 'Bağımsız değişkenler çok uzun (en çok 64 KB).' }
  let value: unknown
  try {
    value = JSON.parse(t)
  } catch (e) {
    return { ok: false, error: `Geçersiz JSON: ${e instanceof Error ? e.message : String(e)}` }
  }
  return { ok: true, args: Array.isArray(value) ? value : [value] }
}

/** Sonucu kopyalanabilir metne çevirir. */
export function formatResult(value: unknown): string {
  if (value === undefined || value === null) return 'Tamam (sonuç yok)'
  if (typeof value === 'string') return value
  if (value instanceof Uint8Array) return `${value.byteLength} bayt ikili veri`
  try {
    return JSON.stringify(value, null, 2)
  } catch {
    return String(value)
  }
}
