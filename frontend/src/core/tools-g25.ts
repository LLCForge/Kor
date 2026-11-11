// [g25] apply_patch ve fork_session.
import { randomUUID } from 'node:crypto'
import { mkdir, unlink } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { atomicWriteFile } from './atomic-write-g10.ts'
import { applyChunks, parsePatch, type PatchHunk } from './patch-g25.ts'
import { isSensitivePath } from './permissions.ts'
import { defineTool, readTextStrict, resolveInWorkspace, type Tool, type ToolContext } from './tools.ts'

function guard(root: string, target: string): string {
  const abs = resolveInWorkspace(root, target)
  if (isSensitivePath(abs) || isSensitivePath(target)) throw new Error(`Hassas dosyaya erişim engellendi: ${target}`)
  return abs
}

/**
 * Yamanın disk üstündeki görünümü: aynı dosyaya art arda gelen işlemler birbirinin sonucunu görür.
 * null: dosya yok (veya bu yamada silindi). Windows'ta yol büyük/küçük harf duyarsızdır.
 */
class PatchOverlay {
  readonly #files = new Map<string, { abs: string; before: string | null; current: string | null }>()

  async #entry(abs: string, label: string) {
    const key = process.platform === 'win32' ? abs.toLowerCase() : abs
    let entry = this.#files.get(key)
    if (!entry) {
      let before: string | null = null
      try {
        before = await readTextStrict(abs, label)
      } catch (err) {
        if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
      }
      entry = { abs, before, current: before }
      this.#files.set(key, entry)
    }
    return entry
  }

  async read(abs: string, label: string): Promise<string | null> {
    return (await this.#entry(abs, label)).current
  }

  async set(abs: string, label: string, content: string | null): Promise<void> {
    ;(await this.#entry(abs, label)).current = content
  }

  /** Disktekinden farklı son haller. */
  changes(): Array<{ abs: string; before: string | null; after: string | null }> {
    return [...this.#files.values()].filter((f) => f.before !== f.current).map((f) => ({ abs: f.abs, before: f.before, after: f.current }))
  }
}

async function plan(root: string, hunk: PatchHunk, overlay: PatchOverlay): Promise<string> {
  const abs = guard(root, hunk.path)
  const current = await overlay.read(abs, hunk.path)
  if (hunk.type === 'add') {
    if (current !== null) throw new Error(`Dosya zaten var, eklenemez: ${hunk.path}`)
    const after = hunk.contents.endsWith('\n') || hunk.contents.length === 0 ? hunk.contents : `${hunk.contents}\n`
    await overlay.set(abs, hunk.path, after)
    return `A ${hunk.path}`
  }
  if (current === null) throw new Error(`Dosya bulunamadı: ${hunk.path}`)
  if (hunk.type === 'delete') {
    await overlay.set(abs, hunk.path, null)
    return `D ${hunk.path}`
  }
  const after = applyChunks(current, hunk.chunks, hunk.path)
  if (!hunk.movePath) {
    await overlay.set(abs, hunk.path, after)
    return `M ${hunk.path}`
  }
  const dest = guard(root, hunk.movePath)
  if (dest === abs) throw new Error(`Taşıma hedefi kaynakla aynı: ${hunk.movePath}`)
  // Windows'ta yalnız harf büyüklüğü farklı hedef de aynı dosyadır: var sayılır ve reddedilir, kaynak silinmez.
  if ((await overlay.read(dest, hunk.movePath)) !== null) throw new Error(`Taşıma hedefi zaten var: ${hunk.movePath}`)
  await overlay.set(dest, hunk.movePath, after)
  await overlay.set(abs, hunk.path, null)
  return `R ${hunk.path} -> ${hunk.movePath}`
}

async function restore(item: { abs: string; before: string | null }): Promise<void> {
  if (item.before === null) await unlink(item.abs)
  else await atomicWriteFile(item.abs, item.before)
}

export function g25Tools(): Tool[] {
  return [
    defineTool({
      name: 'apply_patch',
      description:
        'Birden fazla dosyayı tek yamada ekler, siler, günceller veya taşır. Zarf: "*** Begin Patch" ... "*** End Patch". ' +
        'Güncellemede bağlam satırı boşlukla, silinen "-" ve eklenen "+" ile başlar. Eşleşme yoksa hiçbir dosya değişmez.',
      schema: z.object({
        patch: z.string().min(1).max(200_000),
      }),
      kind: 'edit',
      summarize: (a) => `yama ${a.patch.split('\n').length} satır`,
      subject: () => 'apply_patch',
      run: async (a, ctx) => {
        const hunks = parsePatch(a.patch)
        const overlay = new PatchOverlay()
        const notes: string[] = []
        // Önce hepsi planlanır: tek bir hunk eşleşmezse hiçbir dosya yazılmaz.
        for (const hunk of hunks) notes.push(await plan(ctx.root, hunk, overlay))
        const changes = overlay.changes()
        // Yazmalar önce: taşımada kaynak, hedef diske inmeden silinmesin.
        changes.sort((a, b) => Number(a.after === null) - Number(b.after === null))
        const done: typeof changes = []
        try {
          for (const item of changes) {
            if (item.after === null) {
              await unlink(item.abs)
            } else {
              await mkdir(path.dirname(item.abs), { recursive: true })
              await atomicWriteFile(item.abs, item.after)
            }
            done.push(item)
          }
        } catch (err) {
          // Yazma ortasında disk hatası: yapılanlar geri alınır ki yarım yama kalmasın.
          for (const item of done.reverse()) await restore(item).catch(() => undefined)
          throw new Error(`Yama yarıda kaldı, yazılanlar geri alındı: ${err instanceof Error ? err.message : String(err)}`)
        }
        // Geri sarma kaydı yalnızca tamamlanan yama için; silme, delete_file gibi after: '' ile bildirilir.
        for (const item of changes) ctx.onFileChange?.(path.relative(ctx.root, item.abs), item.before, item.after ?? '')
        return `Yama uygulandı:\n${notes.join('\n')}`
      },
    }),
    defineTool({
      name: 'fork_session',
      description:
        'Bu sohbetin şu anki mesajlarını yeni bir sohbet kaydına kopyalar. Eski sohbet açık kalır. Geçmiş ağacında gezinme yoktur.',
      schema: z.object({
        title: z.string().min(1).max(200).optional(),
      }),
      kind: 'edit',
      summarize: (a) => a.title ?? 'oturum çatalı',
      subject: () => 'fork_session',
      run: async (a, ctx) => {
        const source = ctx.sessionForkSource?.()
        if (!source) throw new Error('Bu ortamda oturum çatallanamaz.')
        const { saveSession } = await import('./sessions.ts')
        const id = randomUUID()
        const title = (a.title ?? source.title ?? 'Çatal').slice(0, 200)
        await saveSession(
          {
            version: 1,
            id,
            title,
            root: source.root,
            model: source.model,
            createdAt: Date.now(),
            updatedAt: Date.now(),
            pinned: false,
          ...(source.permissionMode ? { permissionMode: source.permissionMode } : {}),
          ...(source.id ? { parentId: source.id } : {}),
          messages: source.messages.map((message) => structuredClone(message)),
          },
          ctx.sessionsDir,
        )
        return `Yeni sohbet: ${id}\nBaşlık: ${title}\nBu sohbet açık kaldı.`
      },
    }),
  ]
}
