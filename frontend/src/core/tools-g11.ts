// [g11] Araçlar: yerel proje araması (BM25), oturum notu, proje bilgi tabanı kaydı, hafıza kaydı silme.
import path from 'node:path'
import { z } from 'zod'
import { findConflicts } from './intel-g11.ts'
import { addKnowledge, addSessionNote, allRecords, forgetRecord, keepBoth, knowledgeInput, loadLayers, recordPreview, RECORD_ID, sessionMemoryOf } from './memory-g11.ts'
import { formatSearch, searchIndex } from './project-index-g11.ts'
import { defineTool, resolveInWorkspace, type Tool, type ToolContext } from './tools.ts'

function sessionOwner(ctx: ToolContext): object {
  if (!ctx.shells) throw new Error('Oturum hafızası bu bağlamda kullanılamaz')
  return ctx.shells
}

/** tools.ts ile döngüsel içe aktarma var; dizi modül yüklenirken değil, çağrıldığında kurulur (g3 ile aynı). */
export function g11Tools(): Tool[] {
  return [
    defineTool({
      name: 'project_search',
      description:
        'Proje kodunda yerel, sözcüksel (BM25) arama: dosyaları bildirim sınırlarından parçalara bölen bir dizinde sorguya en ilgili parçaları ' +
        'yol:satır aralığı ve kısa kesitle döner. Tam tanımlayıcıyı veya dosya adını bilmediğinde ilgili kodu bulmak için kullan; ' +
        'anlamsal değil anahtar kelime tabanlıdır, bu yüzden olası kod tanımlayıcılarını ve İngilizce eş anlamlıları yaz. .gitignore ve hassas dosyalar hariç; ağ kullanmaz.',
      schema: z.object({
        query: z.string().trim().min(1).max(300).describe('Anahtar kelimeler, ör. "session save persist transcript"'),
        max_results: z.number().int().min(1).max(20).optional().describe('Varsayılan 8'),
        path: z.string().max(512).optional().describe('Yalnızca bu klasör altında ara (köke göre)'),
      }),
      kind: 'read',
      summarize: (a) => `${a.query}${a.path ? ` (${a.path})` : ''}`,
      subject: (a) => a.path ?? '.',
      run: async (a, ctx) => {
        let prefix = ''
        if (a.path) {
          const rel = path.relative(ctx.root, resolveInWorkspace(ctx.root, a.path))
          if (rel.startsWith('..') || path.isAbsolute(rel)) throw new Error('Dizin yalnızca proje kökünü kapsar')
          prefix = rel.split(path.sep).join('/')
        }
        return formatSearch(ctx.root, a.query, await searchIndex(ctx.root, a.query, a.max_results ?? 8, prefix))
      },
    }),
    defineTool({
      name: 'session_note',
      description:
        'Yalnızca bu sohbet için kısa bir not saklar (oturum hafızası). Not, bağlam sıkıştırılsa da sistem isteminde kalır; sohbet kapanınca silinir. ' +
        'Bu sohbette tekrar gerekecek ara bulgular ve kullanıcının bu işe özel istekleri için kullan. Gizli bilgi yazma.',
      schema: z.object({ note: z.string().trim().min(1).max(500).describe('Tek cümlelik not') }),
      // Diske yazmaz, yalnızca bu sohbetin bellekteki notlarına ekler.
      kind: 'read',
      summarize: (a) => a.note,
      subject: () => 'session_note',
      run: async (a, ctx) => {
        const record = addSessionNote(sessionMemoryOf(sessionOwner(ctx)), a.note)
        return `Oturum hafızasına eklendi (${record.id}).`
      },
    }),
    defineTool({
      name: 'knowledge_add',
      description:
        'Proje bilgi tabanına (.kor/knowledge.json) yapılandırılmış bir kayıt ekler: karar (decision), kısıt (constraint), alan terimi (term) veya kalıcı proje notu (note). ' +
        'Kayıtlar bu projedeki tüm sohbetlerde sistem istemine girer. Kullanıcı bir kararı, kısıtı veya terimi netleştirdiğinde kullan. ' +
        'Yalnızca güvenilir klasörlerde çalışır. Ekleme mevcut bir kayıtla çelişirse sonuçta bildirilir; o zaman kullanıcıya hangisinin geçerli olduğunu sor.',
      schema: knowledgeInput,
      kind: 'edit',
      summarize: (a) => `${a.kind} | ${a.subject}: ${a.statement}`,
      subject: (a) => a.subject,
      run: async (a, ctx) => {
        const { record, duplicate } = await addKnowledge(ctx.root, a)
        if (duplicate) return `Bu kayıt zaten var (${record.id}); değişiklik yapılmadı.`
        const others = allRecords(await loadLayers(ctx.root, ctx.shells ? sessionMemoryOf(ctx.shells) : null)).filter((r) => r.id !== record.id)
        const conflicts = findConflicts([record, ...others]).filter((c) => c.a === record.id || c.b === record.id)
        if (conflicts.length === 0) return `Bilgi tabanına eklendi (${record.id}).`
        const lines = conflicts.slice(0, 3).map((c) => {
          const other = others.find((r) => r.id === (c.a === record.id ? c.b : c.a))
          return `- [${other?.id ?? '?'}] ${other?.subject ? `${other.subject}: ` : ''}${other?.text ?? ''} (${c.reason})`
        })
        return [
          `Bilgi tabanına eklendi (${record.id}), ancak şu kayıtlarla çelişiyor olabilir:`,
          ...lines,
          'ask_user ile kullanıcıya hangisinin geçerli olduğunu sor; geçersiz olanı memory_forget ile sil, ikisi de geçerliyse memory_keep_both çağır.',
        ].join('\n')
      },
    }),
    defineTool({
      name: 'memory_keep_both',
      description:
        'Kullanıcı, çakışıyor görünen iki hafıza kaydının ikisinin de geçerli olduğunu söylediğinde çağır: bu çift bir daha çakışma olarak sorulmaz.',
      schema: z.object({ a: z.string().regex(RECORD_ID), b: z.string().regex(RECORD_ID) }),
      kind: 'edit',
      summarize: (a) => `${a.a} ↔ ${a.b}`,
      subject: (a) => `${a.a} ${a.b}`,
      run: async (a) => {
        await keepBoth(a.a, a.b)
        return 'Kaydedildi: bu iki kayıt artık çakışma sayılmayacak.'
      },
    }),
    defineTool({
      name: 'memory_forget',
      description:
        'Bir hafıza kaydını kimliğiyle siler (oturum s..., proje k..., global g... kimlikleri). Çakışan kayıtlardan hangisinin geçerli olduğunu ' +
        'kullanıcı söyledikten sonra geçersiz olanı silmek veya kullanıcı bir bilginin unutulmasını istediğinde kullan.',
      schema: z.object({ id: z.string().regex(RECORD_ID, 'Kimlik s, k veya g ile başlayan 9 karakterdir (ör. k1a2b3c4d)') }),
      kind: 'edit',
      summarize: (a) => {
        const preview = recordPreview(a.id)
        return preview ? `${a.id}: ${preview.slice(0, 160)}` : a.id
      },
      subject: (a) => a.id,
      run: async (a, ctx) => {
        const removed = await forgetRecord(a.id, { root: ctx.root, session: ctx.shells ? sessionMemoryOf(ctx.shells) : null })
        if (removed === null) return `Kayıt bulunamadı: ${a.id}`
        // Global hafıza sistem isteminin MEMORY bölümünde; o bölüm sonraki istem yenilemesinde güncellenir.
        return `Silindi (${a.id}): ${removed}${a.id.startsWith('g') ? '\nBu sohbetin MEMORY bölümünde hâlâ görünebilir; artık geçersiz say.' : ''}`
      },
    }),
  ]
}
