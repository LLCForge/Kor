import path from 'node:path'
import { z } from 'zod'
import { decodedPayloads } from './exec-policy-g26.ts' // [g26]

/**
 * auto: düzenleme ve sıradan komutlar onaysız, tehlikeli olanlar sorulur.
 * default (manuel): her değişiklik sorulur. bypass: hiçbir şey sorulmaz (deny kuralları hariç).
 */
export const PERMISSION_MODES = ['auto', 'default', 'acceptEdits', 'plan', 'bypass'] as const
export type PermissionMode = (typeof PERMISSION_MODES)[number]

/** read: yan etkisiz; edit: dosya değiştirir; exec: komut çalıştırır */
export type ToolKind = 'read' | 'edit' | 'exec'

// Kural biçimi: "bash", "bash(npm test*)", "edit_file(src/**)". '*' her şeyle eşleşir.
const RULE_PATTERN = /^([a-z_]+)(?:\((.+)\))?$/
export const permissionRuleSchema = z.string().max(500).regex(RULE_PATTERN, 'Kural "arac" veya "arac(desen)" biçiminde olmalı')

export interface PermissionConfig {
  mode: PermissionMode
  allow: string[]
  deny: string[]
  /** true ise hafızaya KAYDETME araçları onay sormadan geçer (silme hariç). Kullanıcının açık tercihi. */
  autoMemory?: boolean
}

// Hafızaya ekleyen araçlar; autoMemory açıkken onay istemezler. memory_forget (silme) KASITLI olarak dışarıda:
// veri kaybını önlemek için silme her zaman sorulur.
export const MEMORY_SAVE_TOOLS: ReadonlySet<string> = new Set(['remember', 'knowledge_add', 'memory_keep_both'])

export type Decision =
  | { action: 'allow' }
  | { action: 'deny'; reason: string }
  | { action: 'ask'; warning?: string }

function wildcardToRegExp(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*')
  return new RegExp(`^${escaped}$`, 's')
}

export function matchesRule(rule: string, toolName: string, subject: string): boolean {
  const m = RULE_PATTERN.exec(rule)
  if (!m || m[1] !== toolName) return false
  if (m[2] === undefined) return true
  // Windows yollarını kurallarla aynı biçime getir.
  return wildcardToRegExp(m[2]).test(subject.replaceAll('\\', '/'))
}

const DANGEROUS_COMMANDS: ReadonlyArray<[RegExp, string]> = [
  [/\brm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r|--recursive\s+--force|--force\s+--recursive)\b/i, 'özyinelemeli zorla silme (rm -rf)'],
  [/\b(rd|rmdir)\s+\/s\b/i, 'özyinelemeli dizin silme (rd /s)'],
  [/\bdel\s+(\/[a-z]\s+)*\/s\b/i, 'özyinelemeli dosya silme (del /s)'],
  [/\bRemove-Item\b.*-Recurse/i, 'özyinelemeli silme (Remove-Item -Recurse)'],
  [/\bformat(\.com)?\s+[a-z]:/i, 'disk biçimlendirme'],
  [/\bmkfs(\.\w+)?\b/i, 'dosya sistemi oluşturma (mkfs)'],
  [/\bdd\b.*\bof=\/dev\//i, 'cihaza ham yazma (dd)'],
  [/\b(curl|wget|iwr|Invoke-WebRequest)\b[^|]*\|\s*(sh|bash|zsh|python\d?|node|iex|Invoke-Expression|powershell|pwsh)\b/i, 'internetten indirilen kodu çalıştırma'],
  [/\b(iex|Invoke-Expression)\b/i, 'dinamik kod çalıştırma (Invoke-Expression)'],
  [/\bgit\s+push\b.*(--force\b|-f\b|--force-with-lease\b)/i, 'zorla push (git push --force)'],
  [/\bgit\s+reset\s+--hard\b/i, 'commit edilmemiş değişiklikleri silme (git reset --hard)'],
  [/\bgit\s+clean\s+-[a-z]*f/i, 'izlenmeyen dosyaları silme (git clean -f)'],
  [/\b(shutdown|reboot|halt|poweroff)\b/i, 'sistemi kapatma/yeniden başlatma'],
  [/\breg(\.exe)?\s+(delete|add)\b/i, 'Windows kayıt defterini değiştirme'],
  [/\bchmod\s+(-R\s+)?777\b/i, 'herkese tam yetki verme (chmod 777)'],
  [/:\(\)\s*\{\s*:\|:&\s*\};:/, 'fork bomb'],
  [/\b(npm|pnpm|yarn)\s+publish\b/i, 'paket yayınlama'],
  [/>\s*\/dev\/sd[a-z]\b/i, 'diske ham yazma'],
]

/**
 * "git.exe", "\"C:\\Git\\bin\\git.exe\"" gibi yazımlar düz "git"e indirgenir;
 * aksi halde uzantı ekleyerek tespitten kaçılabiliyordu.
 */
function normalizeCommand(command: string): string {
  return command.replace(/`/g, '').replace(/["']/g, ' ').replace(/\b([\w-]+)\.(exe|cmd|bat|com)\b/gi, '$1')
}

/** Yıkıcı bir komutsa insan okunur gerekçe döner. Parçalar ve çözülmüş PowerShell gövdesi de denetlenir. */
export function destructiveReason(command: string): string | null {
  const chunks = [command, ...decodedPayloads(command)]
  for (const chunk of chunks) {
    const normalized = normalizeCommand(chunk)
    for (const [pattern, reason] of DANGEROUS_COMMANDS) {
      if (pattern.test(normalized)) return reason
    }
  }
  return null
}

/** Komut bir sır dosyasına dokunuyor mu (.env, id_rsa, .pem...). Yalnızca bilgi verir; varsayılanda engellemez. */
export function secretFileReason(command: string): string | null {
  const chunks = [command, ...decodedPayloads(command)]
  for (const chunk of chunks) {
    const sensitive = chunk.split(/[\s"'`<>|&;()]+/).find((token) => token && isSecretFile(token))
    if (sensitive) return `hassas dosyaya dokunuyor (${sensitive})`
  }
  return null
}

/** İkisinin birleşimi; çağıranlar çoğunlukla bunu ister. */
export function dangerousCommandReason(command: string): string | null {
  return destructiveReason(command) ?? secretFileReason(command)
}

const SAFE_ENV_TEMPLATES = new Set(['.env.example', '.env.sample', '.env.template', '.env.dist'])

/**
 * Sır içermesi muhtemel dosya adları. Bu bir SINIFLANDIRMADIR, kısıt değil:
 * .env, .pem, id_rsa, .npmrc, credentials... Kullanıcı isterse ajan bunları okur, yazar, tarar.
 */
export function isSecretFile(relPath: string): boolean {
  const name = path.basename(relPath).toLowerCase()
  if (SAFE_ENV_TEMPLATES.has(name)) return false
  return (
    /^\.env(\..+)?$/.test(name) ||
    /\.(pem|key|p12|pfx|jks|keystore|kdbx|ppk)$/.test(name) ||
    /^id_(rsa|dsa|ecdsa|ed25519)$/.test(name) ||
    ['.npmrc', '.pypirc', '.netrc', '_netrc', '.git-credentials', 'credentials', 'credentials.json', 'secrets.json', 'settings.local.json'].includes(name)
  )
}

/**
 * Sır dosyası kapısı. VARSAYILAN: hiçbir şey engellenmez - .env okunur/yazılır, id_rsa görünür, secrets.json geri sarılır.
 * Eski katı davranış için: KOR_SECRETS=block (değerler: 1, true, on, strict, block).
 * Tek noktadan çözülür: araç kapıları, dosya ağacı, proje haritası, snapshot ve komut uyarısı hepsi buradan geçer.
 */
export const SECRETS_BLOCKED: boolean = /^(1|true|on|yes|strict|block)$/i.test(
  process.env.KOR_SECRETS ?? process.env.VENICE_SECRETS ?? '',
)

/** Gizli anahtar içermesi muhtemel dosyalar (varsayılanda değil) ajan araçlarına kapalıdır (CWE-200 -> artık opt-in). */
export function isSensitivePath(relPath: string): boolean {
  return SECRETS_BLOCKED && isSecretFile(relPath)
}

export function decide(
  config: PermissionConfig,
  toolName: string,
  kind: ToolKind,
  subject: string,
  sessionAllowed: ReadonlySet<string>,
): Decision {
  if (config.deny.some((r) => matchesRule(r, toolName, subject))) {
    return { action: 'deny', reason: 'İzin kurallarındaki bir deny kuralı bu işlemi engelliyor.' }
  }
  // [g12] Oturum araç kapsamı mod ve allow kurallarından önce gelir: kullanıcı bu araca bu sohbette özellikle kısıt koydu.
  const scope = sessionToolScopes.get(sessionAllowed)?.get(toolName)
  if (scope === 'deny') return { action: 'deny', reason: 'Kullanıcı bu aracı bu sohbette kapattı.' }
  if (scope === 'ask' && (kind === 'read' || config.mode !== 'plan')) {
    const danger = kind === 'exec' ? dangerousCommandReason(subject) : null
    return danger ? { action: 'ask', warning: `Tehlikeli komut: ${danger}` } : { action: 'ask' }
  }
  if (kind === 'read') return { action: 'allow' }
  if (config.mode === 'plan') {
    return {
      action: 'deny',
      reason: 'Plan modundasın: dosya değiştirmek ve komut çalıştırmak kapalı. Önce planını kullanıcıya sun.',
    }
  }

  // Bypass kullanıcının bilinçli seçimidir (proje dosyaları bu modu açamaz, bkz. settings.ts): hiçbir şey sorulmaz.
  if (config.mode === 'bypass') return { action: 'allow' }

  // [g11] Kullanıcı tercihi: hafızaya kaydetme onay sormadan geçer. Aşağıdaki "remember her zaman sor" kuralını da geçer
  // (kullanıcı bilinçli açtı). Plan modunda yukarıda zaten engellendi; deny ve oturum kapsamı yine üstte önceliklidir.
  if (config.autoMemory && MEMORY_SAVE_TOOLS.has(toolName)) return { action: 'allow' }

  // Hafıza tüm gelecek sohbetlere girer: bir dosyadaki talimatın kalıcılaşmasını önlemek için her seferinde sorulur.
  if (toolName === 'remember') return { action: 'ask', warning: 'Kalıcı hafızaya yazılacak; gelecekteki tüm sohbetleri etkiler' }

  // Diğer tüm modlarda tehlikeli komutlar, allow kuralı veya oturum izni olsa bile sorulur.
  const danger = kind === 'exec' ? dangerousCommandReason(subject) : null
  if (danger) return { action: 'ask', warning: `Tehlikeli komut: ${danger}` }

  if (config.mode === 'auto') return { action: 'allow' }
  if (config.mode === 'acceptEdits' && kind === 'edit') return { action: 'allow' }
  if (sessionAllowed.has(toolName)) return { action: 'allow' }
  if (config.allow.some((r) => matchesRule(r, toolName, subject))) return { action: 'allow' }
  return { action: 'ask' }
}

/**
 * [g12] Oturuma özel araç kapsamı (madde 99): 'deny' her modda engeller, 'ask' her modda (izinleri atla dahil) sorar;
 * 'İzin ver' düzeyi mevcut oturum iznidir (sessionAllowed). Kapsamlar oturumun izin kümesine bağlıdır: kümeyle aynı ömrü
 * ve aynı paylaşımı (alt ajanlar) taşır. Yalnızca arayüzden kullanıcı eylemiyle, bellekte tutulur; ayar dosyalarından okunmaz.
 */
export type ToolScope = 'ask' | 'deny'
const sessionToolScopes = new WeakMap<ReadonlySet<string>, Map<string, ToolScope>>()

export function toolScopesFor(sessionAllowed: ReadonlySet<string>): Map<string, ToolScope> {
  let scopes = sessionToolScopes.get(sessionAllowed)
  if (!scopes) {
    scopes = new Map()
    sessionToolScopes.set(sessionAllowed, scopes)
  }
  return scopes
}
