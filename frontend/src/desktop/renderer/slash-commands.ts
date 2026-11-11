// Slash komutu kaydı: mesaj başında "/" yazınca açılan menü bu listeden beslenir.
// Başka gruplar komutlarını kendi işaretlerinin altına ekler (ör. g6: /compact, /init).
// Bu dosya saf kalmalı (React/DOM yok) ki testler doğrudan içe aktarabilsin.
import { fuzzyFilter } from '../../core/fuzzy.ts'
import type { PermissionMode } from '../../core/permissions.ts'
import type { ModelInfo } from '../ipc.ts'
import { suggestDiagramType, toFlowchart } from './diagram-utils.ts'

// [g1] importlar

// [g3] importlar

// [g4] importlar

// [g5] importlar

// [g6] importlar

// [g9] importlar

// [g10] importlar

// [g11] importlar
import { openG11Panel } from './g11-bus.ts'

// [g12] importlar

// [g13] importlar
import { openG13 } from './g13/bus.ts'

// [g14] importlar
import { openObservability } from './g14-bus.ts'

// [g15] importlar

// [g16] importlar
import { openG16Panel } from './g16-bus.ts'

// [g18] importlar

// [g19] importlar

// [g20] importlar
import { openBoard } from './g20-board-store.ts'

// [g21] importlar

// [g22] importlar

// [g7] importlar

// [cdx] Yerleşik takımlar (ayarlardaki kullanıcı takımları da run_team ile çalışır; menü yalnızca ipucu verir)
const TEAM_SUGGESTIONS = [
  { value: 'ozellik', label: 'ozellik', description: 'Yeni özellik: planla → uygula → gözden geçir' },
  { value: 'hata', label: 'hata', description: 'Hata düzeltme: keşfet → düzelt → doğrula' },
  { value: 'inceleme', label: 'inceleme', description: 'Kod incelemesi: inceleyici + sadeleştirici' },
]

/** Komutların kullanabildiği eylemler; mesaj kutusu sağlar. */
export interface SlashCommandContext {
  /** Etkin sohbetin kimliği (yeni sohbet taslağında da dolu) */
  sessionId: string
  running: boolean
  models: readonly ModelInfo[]
  permissionModes: ReadonlyArray<{ id: PermissionMode; label: string; desc: string }>
  newChat(): void
  setModel(id: string): void
  setPermissionMode(mode: PermissionMode): void
  openMemory(): void
  openSystemPrompt(): void
  pickFiles(): void
  abort(): void
  setDraft(text: string): void
  /** Mesaj kutusunun altında kısa bilgi/hata gösterir */
  notify(message: string): void
  // [g6] bağlam sıkıştırma ve proje talimatı (birleştirmede bağlandı)
  compact(): Promise<void>
  initProject(): Promise<void>
}

export interface SlashSuggestion {
  value: string
  label: string
  description?: string
}

export interface SlashCommand {
  /** "/" olmadan, küçük harf, boşluksuz */
  name: string
  description: string
  /** Argüman alıyorsa ipucu (ör. "<model>"): menüden seçilince "/ad " yazılır ve öneriler listelenir */
  argHint?: string
  /** Yerleşik komutlarda yok; dosya/eklenti komutlarında kaynağı belirtir (palet rozeti). */
  source?: 'user' | 'project' | 'plugin'
  suggest?(ctx: SlashCommandContext, query: string): SlashSuggestion[]
  run(ctx: SlashCommandContext, args: string): void | Promise<void>
}

const g2Commands: SlashCommand[] = [
  { name: 'yeni', description: 'Yeni sohbet başlat', run: (ctx) => ctx.newChat() },
  {
    name: 'model',
    description: 'Modeli değiştir',
    argHint: '<model>',
    suggest: (ctx, query) =>
      fuzzyFilter(ctx.models, query, (m) => [m.name, m.id], 30).map((m) => ({ value: m.id, label: m.name, description: m.id })),
    run: (ctx, args) => {
      const query = args.trim()
      if (!query) return
      const exact = ctx.models.find((m) => m.id === query)
      const match = exact ?? fuzzyFilter(ctx.models, query, (m) => [m.name, m.id], 1)[0]
      if (match) ctx.setModel(match.id)
      else ctx.notify(`"${query}" adlı bir model bulunamadı.`)
    },
  },
  {
    name: 'mod',
    description: 'İzin modunu değiştir',
    argHint: '<mod>',
    suggest: (ctx, query) =>
      fuzzyFilter(ctx.permissionModes, query, (m) => [m.label, m.id]).map((m) => ({ value: m.id, label: m.label, description: m.desc })),
    run: (ctx, args) => {
      const query = args.trim()
      if (!query) return
      const match = ctx.permissionModes.find((m) => m.id === query) ?? fuzzyFilter(ctx.permissionModes, query, (m) => [m.label, m.id], 1)[0]
      if (match) ctx.setPermissionMode(match.id)
      else ctx.notify(`"${query}" adlı bir izin modu yok.`)
    },
  },
  { name: 'hafiza', description: 'Kalıcı hafızayı düzenle', run: (ctx) => ctx.openMemory() },
  { name: 'sistem', description: 'Sistem istemini düzenle', run: (ctx) => ctx.openSystemPrompt() },
  { name: 'ekle', description: 'Dosya ekle', run: (ctx) => ctx.pickFiles() },
  {
    name: 'durdur',
    description: 'Çalışan turu durdur',
    run: (ctx) => {
      if (ctx.running) ctx.abort()
      else ctx.notify('Çalışan bir tur yok.')
    },
  },
]

export const SLASH_COMMANDS: readonly SlashCommand[] = [
  ...g2Commands,

  // [g1] slash komutları

  // [g3] slash komutları
  {
    name: 'diyagram',
    description: 'JSON/listeyi akış diyagramına çevir ya da açıklamaya uygun diyagram türü öner',
    argHint: '<JSON | madde listesi | açıklama>',
    run: (ctx, args) => {
      const text = args.trim()
      if (!text) {
        ctx.notify('Kullanım: /diyagram <JSON | madde listesi | açıklama>')
        return
      }
      // feature 21: yapısal girdi (JSON/liste/çok satır) akış diyagramına; feature 22: düz açıklama tür önerisine.
      const structured = /^[[{]/.test(text) || /^\s*([-*+]|\d+[.)])\s/m.test(text) || text.includes('\n')
      if (structured) {
        ctx.setDraft('```mermaid\n' + toFlowchart(text) + '\n```')
        ctx.notify('Akış diyagramı taslağa eklendi; göndermeden düzenleyebilirsiniz.')
      } else {
        const suggestion = suggestDiagramType(text)
        ctx.setDraft('```mermaid\n' + suggestion.template + '\n```')
        ctx.notify(`Önerilen tür: ${suggestion.label}. Şablon taslağa eklendi.`)
      }
    },
  },

  // [g4] slash komutları

  // [g5] slash komutları

  // [g6] slash komutları (/compact, /init)
  { name: 'compact', description: 'Konuşmayı özetleyip bağlamı küçült', run: (ctx) => ctx.compact() },

  // [cdx] takım: rol tabanlı ajan ekibi (run_team aracı)
  {
    name: 'takim',
    description: 'Bir ajan takımını çalıştır (ozellik / hata / inceleme …): planla → uygula → incele',
    argHint: '<takım> <görev>',
    suggest: (_ctx, query) =>
      fuzzyFilter(TEAM_SUGGESTIONS, query, (t) => [t.value, t.label], 10).map((t) => ({ value: `${t.value} `, label: t.label, description: t.description })),
    run: (ctx, args) => {
      const [team, ...rest] = args.trim().split(/\s+/)
      const task = rest.join(' ').trim()
      if (!team) return ctx.notify('Kullanım: /takim <takım> <görev>')
      if (!task) {
        ctx.setDraft(`/takim ${team} `)
        return ctx.notify('Takımın yapacağı görevi yaz; Enter ile gönder.')
      }
      ctx.setDraft(`run_team aracıyla "${team}" takımını çalıştır. Görev: ${task}`)
      ctx.notify('Takım isteği taslağa yazıldı; göndermek için Enter.')
    },
  },
  { name: 'init', description: 'Proje haritasından AGENTS.md taslağı oluştur', run: (ctx) => ctx.initProject() },

  // [g9] slash komutları

  // [g10] slash komutları

  // [g11] slash komutları
  { name: 'hafiza-katmanlari', description: 'Hafıza katmanlarını, proje bilgi tabanını ve güven puanı ayarını göster', run: () => openG11Panel() },

  // [g12] slash komutları

  // [g13] slash komutları
  { name: 'istem-katmanlari', description: 'Sistem istemi katmanları (global, proje, oturum) ve birleşik önizleme', run: () => openG13('layers') },
  { name: 'ortam', description: '.env değişkenlerini yönet (değerler maskeli)', run: () => openG13('env') },
  { name: 'dokuman', description: 'Onaylı doküman kaynakları (docs_fetch)', run: () => openG13('docs') },
  { name: 'aktar', description: 'Görevi taşınabilir bağlam paketine aktar (Markdown/JSON)', run: () => openG13('export') },

  // [g14] slash komutları
  { name: 'gozlem', description: 'Gözlem panosunu aç (izler, metrikler, maliyet, süreçler)', run: () => openObservability() },

  // [g15] slash komutları

  // [g16] slash komutları
  { name: 'dagitim', description: 'Sürüm, güncellemeler, yapılandırma dosyası ve çökme raporları', run: () => openG16Panel() },

  // [g18] slash komutları

  // [g19] slash komutları

  // [g20] slash komutları
  { name: 'tuval', description: 'Beyaz tahtayı aç (çizim, kutu, ok, not; PNG/SVG/Mermaid)', run: () => openBoard() },

  // [g21] slash komutları

  // [g22] slash komutları

  // [g7] slash komutları
]

/** Boş sorguda kayıt sırası; aksi halde bulanık eşleşme puanına göre. */
export function filterSlashCommands(commands: readonly SlashCommand[], query: string): SlashCommand[] {
  return query ? fuzzyFilter(commands, query, (c) => [c.name]) : [...commands]
}

export const findSlashCommand = (commands: readonly SlashCommand[], name: string) => commands.find((c) => c.name === name)

/** "/ad argümanlar" biçimindeki taslağı ayrıştırır; komut değilse null. */
export function parseSlashInput(text: string): { name: string; args: string } | null {
  const match = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(text.trim())
  return match?.[1] ? { name: match[1], args: match[2] ?? '' } : null
}
