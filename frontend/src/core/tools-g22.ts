// [g22] Dosya silme/taşıma ve modelin güvenle çağırabileceği komut araçları (bağlam sıkıştırma, proje haritası).
// Model değiştirme, izin modu, sistem istemi, ayar ve klasör güveni BİLEREK araç değildir: ayrıcalık yükseltme ve kalıcı istem enjeksiyonu.
import { copyFile, lstat, mkdir, readFile, rename, rm } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { compactSession } from './compaction.ts'
import { isSensitivePath } from './permissions.ts'
import { liveSession } from './text-tools-g22.ts'
// Döngüsel içe aktarma (tools.ts bu dosyayı içe aktarır): araçlar g22Tools() içinde çağrı anında oluşturulur.
import { defineTool, resolveInWorkspace, type Tool } from './tools.ts'

/** Kontrol noktaları metin saklar: bu sınırın üstündeki veya ikili dosya geri sarılamayacağı için silinmez/taşınmaz. */
export const MAX_RESTORABLE_BYTES = 10 * 1024 * 1024

const pathField = z.string().min(1).max(1024).describe('Çalışma dizinine göre dosya yolu')
const mapBuilds = new Set<string>()

function resolveSafe(root: string, target: string): string {
  const abs = resolveInWorkspace(root, target)
  if (isSensitivePath(abs)) throw new Error(`Hassas dosyaya erişim engellendi: ${target}`)
  return abs
}

/** Dosyanın geri yüklenebilir metin içeriği; yoksa null. Klasör, bağlantı, ikili ve büyük dosya reddedilir. */
async function restorableText(abs: string, label: string): Promise<string | null> {
  let info
  try {
    info = await lstat(abs)
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return null
    throw err
  }
  if (info.isDirectory()) throw new Error(`${label} bir klasör; klasör silme/taşıma bu araçla yapılmaz (gerekirse kullanıcıya sor)`)
  if (info.isSymbolicLink()) throw new Error(`${label} sembolik bağlantı; bu araçla silinmez/taşınmaz`)
  if (!info.isFile()) throw new Error(`${label} normal bir dosya değil`)
  if (info.size > MAX_RESTORABLE_BYTES) throw new Error(`${label} 10 MB'tan büyük; geri sarılamayacağı için bu araçla işlenmez`)
  const bytes = await readFile(abs)
  try {
    // ignoreBOM: BOM metinde kalır ki geri yüklenen dosya bayt bayt aynı olsun.
    const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes)
    if (!text.includes('\0')) return text
  } catch {
    // Geçersiz UTF-8: aşağıdaki ikili dosya hatası.
  }
  throw new Error(`${label} ikili dosya; geri sarılamayacağı için bu araçla işlenmez (gerekirse kullanıcıya sor)`)
}

export function g22Tools(): Tool[] {
  return [
    defineTool({
      name: 'delete_file',
      description:
        'Tek bir metin dosyasını siler. İçerik önce değişiklik kaydına alınır; geri sarma (kontrol noktası) dosyayı geri getirir. ' +
        'Klasör, sembolik bağlantı, ikili veya 10 MB üstü dosya silinmez.',
      schema: z.object({ path: pathField }),
      kind: 'edit',
      summarize: (a) => a.path,
      subject: (a) => a.path,
      run: async (a, ctx) => {
        const abs = resolveSafe(ctx.root, a.path)
        const before = await restorableText(abs, a.path)
        if (before === null) throw new Error(`Dosya bulunamadı: ${a.path}`)
        await rm(abs)
        ctx.onFileChange?.(path.relative(ctx.root, abs), before, '')
        return `Silindi: ${a.path}`
      },
    }),
    defineTool({
      name: 'move_file',
      description:
        'Bir metin dosyasını taşır veya yeniden adlandırır; hedef klasör yoksa oluşturulur. Hedef varsa overwrite: true gerekir. ' +
        'Geri sarma iki dosyayı da eski haline getirir. Klasör, sembolik bağlantı, ikili veya 10 MB üstü dosya taşınmaz.',
      schema: z.object({
        from: pathField,
        to: pathField,
        overwrite: z.boolean().optional().describe('true ise var olan hedefin üzerine yazar'),
      }),
      kind: 'edit',
      summarize: (a) => `${a.from} → ${a.to}${a.overwrite ? ' (üzerine yaz)' : ''}`,
      // İzin kuralları oluşturulan/üzerine yazılan yola (hedef) uygulanır.
      subject: (a) => a.to,
      run: async (a, ctx) => {
        const src = resolveSafe(ctx.root, a.from)
        const dst = resolveSafe(ctx.root, a.to)
        if (src === dst) throw new Error('Kaynak ve hedef aynı')
        const content = await restorableText(src, a.from)
        if (content === null) throw new Error(`Dosya bulunamadı: ${a.from}`)
        const existing = await restorableText(dst, a.to)
        if (existing !== null && !a.overwrite) throw new Error(`${a.to} zaten var; üzerine yazmak için overwrite: true gönder`)
        await mkdir(path.dirname(dst), { recursive: true })
        try {
          await rename(src, dst)
        } catch (err) {
          // Ek çalışma dizini başka sürücüdeyse rename yapılamaz: kopyala ve sil.
          if ((err as NodeJS.ErrnoException).code !== 'EXDEV') throw err
          await copyFile(src, dst)
          await rm(src)
        }
        ctx.onFileChange?.(path.relative(ctx.root, src), content, '')
        ctx.onFileChange?.(path.relative(ctx.root, dst), existing, content)
        return `Taşındı: ${a.from} → ${a.to}`
      },
    }),
    defineTool({
      name: 'compact_context',
      description:
        'Konuşmanın eski kısmını özetleyip bağlamı küçültür (/compact ile aynı); son turlar aynen kalır. ' +
        'Bağlam dolmak üzereyken uzun bir işe devam etmeden önce kullan. Yalnızca ana sohbette çalışır.',
      schema: z.object({}),
      // Yan etkisi var: konuşma geçmişini değiştirir ve model çağrısı maliyeti doğurur.
      kind: 'edit',
      summarize: () => 'eski turları özetle',
      subject: () => 'compact',
      run: async (_a, ctx) => {
        // spawnSubagent yalnızca üst düzey oturumda verilir: alt ajan üst sohbetin geçmişini değiştiremez.
        const live = ctx.spawnSubagent ? liveSession(ctx.shells) : undefined
        if (!live) throw new Error('Bağlam sıkıştırma yalnızca ana sohbette kullanılabilir')
        const r = await compactSession(live.session, { io: live.io, signal: ctx.signal })
        return r.compacted ? `Bağlam özetlendi: ~${r.beforeTokens} → ~${r.afterTokens} token.` : `Sıkıştırılmadı: ${r.reason ?? 'gerek yok'}`
      },
    }),
    defineTool({
      name: 'update_project_map',
      description:
        'Proje haritasını (.kor/PROJECT.md) oluşturur ya da yalnızca değişen alanları yeniden keşfederek günceller; harita yeni sohbetlerin bağlamına girer. ' +
        'Alt ajanlar çalıştırdığı için maliyetlidir: kullanıcı isterse veya proje yapısı belirgin biçimde değiştiyse kullan.',
      schema: z.object({ force: z.boolean().optional().describe('true ise tüm alanlar baştan keşfedilir') }),
      kind: 'edit',
      summarize: (a) => (a.force ? 'baştan oluştur' : 'değişenleri güncelle'),
      subject: () => '.kor/PROJECT.md',
      run: async (a, ctx) => {
        // Alt ajandan çağrılmaz: harita keşfi de alt ajan yuvası (semaphore) kullanır, yuva tutan alt ajan beklerse kilitlenir.
        const live = ctx.spawnSubagent ? liveSession(ctx.shells) : undefined
        if (!live) throw new Error('Proje haritası yalnızca ana sohbetten güncellenebilir')
        if (mapBuilds.has(ctx.root)) throw new Error('Bu proje için harita zaten güncelleniyor')
        mapBuilds.add(ctx.root)
        try {
          // Dinamik: project-map-build ajan döngüsünü içe aktarır; statik içe aktarma döngüsel yüklemeye girer.
          const { buildProjectMap } = await import('./project-map-build.ts')
          const r = await buildProjectMap({ root: ctx.root, client: live.session.client, model: live.session.model, signal: ctx.signal, force: a.force === true })
          return `Proje haritası güncellendi: ${r.explored} alan keşfedildi, ${r.reused} alan yeniden kullanıldı (~${r.tokens} token). İçerik .kor/PROJECT.md dosyasında.`
        } finally {
          mapBuilds.delete(ctx.root)
        }
      },
    }),
  ]
}
