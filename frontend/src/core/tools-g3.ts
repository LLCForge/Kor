// [g3] Sohbet akışı araçları: bölümler, görev listesi, tur sonu kutusu, seçenekli soru, plan onayı.
import { askArgs, chapterArgs, planArgs, suggestArgs, todoArgs, todoProgress } from './interactive.ts'
import { defineTool, type Tool } from './tools.ts'

/**
 * Araçların summary'si doğrulanmış argümanların JSON'udur: arayüz kartı hem canlı olaydan hem
 * kayıttan açılan sohbetten (transcriptFromMessages da summarize çağırır) aynı yoldan çizer.
 *
 * tools.ts ile döngüsel içe aktarma var; bu yüzden dizi modül yüklenirken değil, çağrıldığında kurulur.
 */
export function g3Tools(): Tool[] {
  const json = (a: unknown) => JSON.stringify(a)
  const noUi = 'Bu ortamda kullanıcıya etkileşimli kart gösterilemiyor (CLI veya alt ajan). Soruyu yanıtında düz metin olarak sor ve makul varsayımla devam et.'

  return [
    defineTool({
      name: 'mark_chapter',
      description:
        'Sohbette yeni bir bölüm başlatır: akışta başlıklı bir ayraç ve içindekiler tablosunda bir girdi oluşur. ' +
        'İş anlamlı biçimde yeni bir aşamaya geçtiğinde (keşif → uygulama → doğrulama, ya da kullanıcı yeni bir konuya geçtiğinde) kullan.',
      schema: chapterArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => a.title,
      run: async (a) => `Bölüm açıldı: ${a.title}`,
    }),
    defineTool({
      name: 'todo_write',
      description:
        'Görev listesini yazar/günceller; kullanıcı listeyi sohbetin altında canlı görür. Her çağrıda listenin TAMAMINI gönder. ' +
        'Aynı anda yalnızca bir görev in_progress olsun; biten görevi hemen completed yap.',
      schema: todoArgs,
      kind: 'read',
      summarize: json,
      subject: () => 'todo',
      run: async (a) => {
        const { done, total, active } = todoProgress(a.items)
        return `Görev listesi güncellendi (${done}/${total} tamamlandı${active ? `, şu an: ${active.content}` : ''}). Kullanıcı listeyi görüyor; yanıtında tekrar yazma.`
      },
    }),
    defineTool({
      name: 'suggest_next',
      description:
        'Turu bitirirken kullanıcıya "sonraki adım" kutusu gösterir: kısa özet, varsa tek bir soru ve tek tıkla gönderilecek en fazla 3 cevap düğmesi. ' +
        'Turun SON eylemi olarak bir kez çağır; ardından metin yazma.',
      schema: suggestArgs,
      kind: 'read',
      summarize: json,
      subject: () => 'suggest_next',
      run: async () => 'Sonraki adım kutusu kullanıcıya gösterildi. Turu bitir; bu kutuyu metinde tekrarlama.',
    }),
    defineTool({
      name: 'ask_user',
      description:
        'Kullanıcıya seçenekli soru sorar ve cevabı bekler (kartta 1..9 tuşuyla seçilebilen, "Diğer" serbest metin alanlı soru). ' +
        'Makul bir varsayım varsa sorma, varsayımı yaz ve devam et. Yalnızca sonucu gerçekten değiştiren kararlarda sor: iki ciddi yol, ' +
        'geri alınamaz adım, iş/design tercihi ya da yalnızca kullanıcının bildiği bilgi. 2-6 kısa seçenek ver, önerdiklerini başa al. ' +
        'Birden çok soru, çoklu seçim veya sıralama için questions dizisini kullan (en fazla 3 soru, soru başına 2-4 seçenek, type: single_select | multi_select | rank_priorities).',
      schema: askArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => ('question' in a ? a.question : (a.questions[0]?.question ?? 'soru')),
      run: async (a, ctx) => (ctx.interact && ctx.callId ? ctx.interact.ask(ctx.callId, a, ctx.signal) : noUi),
    }),
    defineTool({
      name: 'exit_plan_mode',
      description:
        'Plan modundayken planı kullanıcının onayına sunar (Markdown). Kullanıcı onaylarsa izin modu değişir ve planı uygulamaya geçersin; ' +
        'düzeltme isterse geri bildirimi sonuç olarak alırsın. Plan modu dışında çağırma.',
      schema: planArgs,
      // Kendi başına yan etkisi yok; mod değişikliğini kullanıcı karttan onaylar. 'read' olduğu için plan modunda engellenmez.
      kind: 'read',
      summarize: json,
      subject: () => 'exit_plan_mode',
      run: async (a, ctx) => (ctx.interact && ctx.callId ? ctx.interact.exitPlan(ctx.callId, a.plan, ctx.signal) : noUi),
    }),
  ]
}
