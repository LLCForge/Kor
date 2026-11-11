// Kor'un bütün komutları "/" menüsünde, terminaldeki gibi: `/seo audit`, `/cost why`, `/peer list`.
// Kök kelime bir slash komutudur; alt komutlar öneri olarak listelenir. Kaynak parity.toml (üretilen liste).
// Saf modül (React/DOM yok): testler doğrudan içe aktarır.
import { PARITY_COMMANDS, type ParityCommand } from './parity-commands.gen.ts'
import type { SlashCommand, SlashSuggestion } from './slash-commands.ts'

/** `kor seo audit` → ['seo', 'audit']; `kor` tek başına (sohbet) ve bayraklı satırlar alınmaz. */
function words(cli: string): string[] | null {
  const parts = cli.trim().split(/\s+/)
  if (parts[0] !== 'kor' || parts.length < 2) return null
  const rest = parts.slice(1)
  return rest.every((w) => /^[a-z][a-z0-9-]*$/.test(w)) ? rest : null
}

interface Entry {
  path: string[]
  command: ParityCommand
}

/** Alt komut yolu argümanların başıyla en uzun eşleşen komut; kalan sözcükler argümandır. */
export function resolveParity(entries: readonly Entry[], args: string): { command: ParityCommand; rest: string } | null {
  const tokens = args.trim().split(/\s+/).filter(Boolean)
  let best: Entry | null = null
  for (const e of entries) {
    const sub = e.path.slice(1)
    if (sub.length > tokens.length) continue
    if (sub.every((w, i) => tokens[i] === w) && (best === null || sub.length > best.path.length - 1)) best = e
  }
  if (!best) return null
  return { command: best.command, rest: tokens.slice(best.path.length - 1).join(' ') }
}

/**
 * Parity komutlarından slash komutları. `taken`: yerleşik ve dosya komutlarının adları (çakışanlar atlanır).
 * `run`: komutu çalıştırır (paneli açar ya da çalıştırıcıyı gösterir); `rest` kalan argümanlardır.
 */
export function paritySlashCommands(
  taken: ReadonlySet<string>,
  run: (command: ParityCommand, rest: string) => void,
): SlashCommand[] {
  const roots = new Map<string, Entry[]>()
  const seen = new Set<string>()
  for (const command of PARITY_COMMANDS) {
    const path = words(command.cli)
    if (!path || seen.has(command.cli)) continue
    seen.add(command.cli)
    const list = roots.get(path[0]!) ?? []
    list.push({ path, command })
    roots.set(path[0]!, list)
  }
  const out: SlashCommand[] = []
  for (const [root, entries] of [...roots].sort(([a], [b]) => a.localeCompare(b))) {
    if (taken.has(root)) continue
    const own = entries.find((e) => e.path.length === 1)
    const subs = entries.filter((e) => e.path.length > 1)
    const description = own?.command.name ?? `${subs.length} alt komut: ${subs.slice(0, 4).map((e) => e.path.slice(1).join(' ')).join(', ')}${subs.length > 4 ? '…' : ''}`
    out.push({
      name: root,
      description,
      ...(subs.length > 0 ? { argHint: '<alt komut>' } : {}),
      suggest: (_ctx, query): SlashSuggestion[] => {
        const q = query.trim().toLowerCase()
        return subs
          .map((e) => ({ value: e.path.slice(1).join(' '), label: e.path.slice(1).join(' '), description: e.command.name }))
          .filter((s) => q === '' || s.value.includes(q) || s.description.toLowerCase().includes(q))
          .slice(0, 40)
      },
      run: (ctx, args) => {
        const hit = resolveParity(entries, args)
        if (hit) {
          run(hit.command, hit.rest)
          return
        }
        ctx.notify(`/${root}: alt komut seçin (${subs.map((e) => e.path.slice(1).join(' ')).slice(0, 8).join(', ')}${subs.length > 8 ? '…' : ''}).`)
      },
    })
  }
  return out
}
