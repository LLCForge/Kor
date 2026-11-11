// [g23] Yerel asistan araçları. Ertelenenler tool_search ile oturuma eklenir; başlangıç bağlamına girmez.
// Spec: N1-AI/claude-hidden-toolkit@693c7c5 (CC BY 4.0, Francesco Marinoni Moretto).
// Clean-room: davranış yeniden yazıldı, kitap metni kopyalanmadı. Bkz. THIRD_PARTY_NOTICES.md.
import { z } from 'zod'
import {
  alarmArgs,
  chartArgs,
  composeArgs,
  eventDraftArgs,
  mapArgs,
  recipeArgs,
  widgetArgs,
} from './assistant-cards.ts'
import {
  assistantHome,
  claimDueNotices,
  commitEvent,
  commitEvents,
  createAlarm,
  createReminders,
  createTimer,
  dedupePlaces,
  deleteEvents,
  deleteReminders,
  endSessionRecord,
  fetchWeather,
  formatLocalTime,
  listCalendars,
  listReminderLists,
  readWindowsLocation,
  reverseGeocode,
  searchCatalog,
  searchEvents,
  searchPlaces,
  searchReminders,
  updateEvents,
  updateReminders,
  type CatalogEntry,
} from './assistant-surface.ts'
import { getDeferredDynamicTools } from './dynamic-tools.ts'
import { sanitizeToolName } from './tool-compat.ts'
import { defineTool, type Tool, type ToolContext } from './tools.ts'

function registry(): { deferred: Tool[]; byName: Map<string, Tool> } {
  const fn = registry as typeof registry & { cache?: { deferred: Tool[]; byName: Map<string, Tool> } }
  fn.cache ??= { deferred: [], byName: new Map() }
  return fn.cache
}

export function deferredByName(name: string): Tool | undefined {
  return registry().byName.get(name) ?? getDeferredDynamicTools().find((t) => t.name === name || sanitizeToolName(t.name) === name)
}

export function isDeferredToolName(name: string): boolean {
  return deferredByName(name) !== undefined
}

export function deferredCatalog(): CatalogEntry[] {
  return [...registry().deferred, ...getDeferredDynamicTools()].map((t) => ({ name: t.name, description: t.description, parameters: t.parameters }))
}

function json(value: unknown): string {
  return JSON.stringify(value)
}
function home(ctx: ToolContext): string {
  return assistantHome(ctx.assistantHome)
}
function netOf(ctx: ToolContext): typeof fetch {
  return ctx.net ?? fetch
}
function signalOf(ctx: ToolContext): AbortSignal {
  return AbortSignal.any([ctx.signal, AbortSignal.timeout(12_000)])
}

function isDeferredName(name: string): boolean {
  switch (name) {
    case 'show_chart':
    case 'show_widget':
    case 'create_alarm':
    case 'create_timer':
    case 'list_calendars':
    case 'search_events':
    case 'create_event':
    case 'create_events':
    case 'update_events':
    case 'delete_events':
    case 'list_reminder_lists':
    case 'create_reminders':
    case 'search_reminders':
    case 'update_reminders':
    case 'delete_reminders':
    case 'weather':
    case 'show_recipe':
    case 'search_places':
    case 'show_map':
      return true
    default:
      return false
  }
}
/**
 * [perf] Masaüstü kontrolü / Chrome köprüsü / UIA / pano aileleri de ertelenir: ~35 araç şeması her istekte
 * binlerce token yiyordu. tool_search("ekran", "pencere", "chrome", "pano"…) ile ya da model adını doğrudan
 * çağırınca (executeToolCall otomatik yükler) oturuma eklenir. Tarayıcı (browser_*) ve dosya/kabuk araçları hep yüklü.
 */
export const DESKTOP_DEFERRED = new Set([
  'computer_screenshot', 'computer_move', 'computer_click', 'computer_type', 'computer_key', 'computer_scroll',
  'list_windows', 'launch_app', 'focus_window', 'move_window', 'window_state', 'capture_window', 'list_displays',
  'clipboard_read', 'clipboard_write', 'clipboard_read_image', 'clipboard_write_image', 'paste_text',
  'ocr_image', 'uia_snapshot', 'uia_invoke', 'screen_region', 'mouse_drag', 'mouse_hover', 'pixel_color',
  'list_processes', 'system_info', 'recent_downloads',
  'chrome_bridge_info', 'chrome_navigate', 'chrome_read', 'chrome_click', 'chrome_type',
  'powershell',
])

/** Çekirdek listeden masaüstü ailelerini ertelenmiş kayda alır; kalan (her zaman yüklü) listeyi döner. */
export function deferDesktopTools(list: readonly Tool[]): Tool[] {
  const { deferred, byName } = registry()
  const always: Tool[] = []
  for (const tool of list) {
    if (DESKTOP_DEFERRED.has(tool.name)) {
      if (!byName.has(tool.name)) {
        deferred.push(tool)
        byName.set(tool.name, tool)
      }
    } else always.push(tool)
  }
  return always
}

function register(list: Tool[]): Tool[] {
  const { deferred, byName } = registry()
  // Yalnızca g23'ün kendi ertelenmişleri temizlenir; masaüstü aileleri (deferDesktopTools) kalır.
  for (const t of [...deferred]) if (!DESKTOP_DEFERRED.has(t.name)) { deferred.splice(deferred.indexOf(t), 1); byName.delete(t.name) }
  const always: Tool[] = []
  for (const tool of list) {
    if (isDeferredName(tool.name)) {
      deferred.push(tool)
      byName.set(tool.name, tool)
    } else always.push(tool)
  }
  return always
}

/** Her zaman yüklü olanlar. Ertelenmiş araçlar modül kaydındadır, tool_search açar. */
export function g23Tools(): Tool[] {
  const noCard = 'Bu ortamda kart gösterilemiyor.'
  const all: Tool[] = [
    defineTool({
      name: 'current_time',
      description:
        'Kullanıcının işletim sistemi saat diliminde şu anki zamanı ISO 8601 olarak döner. ' +
        'Randevu, alarm ve gün hesabından önce çağır. Ağ kullanmaz.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'saat',
      subject: () => 'time',
      run: async () => {
        const clock = formatLocalTime()
        return json({ current_time: clock.iso, time_zone: clock.timeZone })
      },
    }),
    defineTool({
      name: 'user_location',
      description:
        'Kullanıcının cihaz konumunu ister. accuracy "low" veya "high" kabul edilir; ikisi de aynı okumayı döner. ' +
        'İzin reddedilirse veya konum servisi kapalıysa koordinat uydurulmaz. ' +
        'Başarılı okuma, açık adres için Open-Meteo ters coğrafi kodlama kullanır.',
      schema: z.object({ accuracy: z.enum(['low', 'high']) }),
      kind: 'exec',
      summarize: (a) => a.accuracy,
      subject: () => 'location',
      run: async (a, ctx) => {
        const reading = ctx.locate ? await ctx.locate(a.accuracy) : await readWindowsLocation(ctx.signal)
        if (reading.status !== 'success' || reading.latitude === undefined || reading.longitude === undefined) return json(reading)
        let geocoded = reading.geocoded
        if (!geocoded) {
          try {
            geocoded = await reverseGeocode(reading.latitude, reading.longitude, netOf(ctx), signalOf(ctx))
          } catch (err) {
            console.warn('Ters coğrafi kodlama başarısız:', err instanceof Error ? err.message : err)
          }
        }
        return json({ ...reading, ...(geocoded ? { geocoded } : {}) })
      },
    }),
    defineTool({
      name: 'tool_search',
      description:
        'Ertelenmiş araçları anahtar kelimeyle arar ve bu oturuma yükler: takvim, anımsatıcı, alarm, hava, yer, grafik, tarif, harita; ' +
        'ayrıca masaüstü kontrolü (ekran görüntüsü, tıklama, yazma, pencere, pano, OCR, UIA), Chrome köprüsü ve PowerShell. ' +
        'Dosya, kabuk, web ve tarayıcı (browser_*) araçları her zaman yüklüdür; burada görünmezler. limit 1-20, varsayılan 5.',
      schema: z.object({
        query: z.string().min(1).max(200),
        limit: z.number().int().min(1).max(20).optional(),
      }),
      kind: 'read',
      summarize: (a) => a.query,
      subject: (a) => a.query,
      run: async (a, ctx) => {
        const hits = searchCatalog(deferredCatalog(), a.query, a.limit ?? 5)
        ctx.unlockTools?.(hits.map((h) => h.name))
        if (hits.length === 0) return 'Eşleşen ertelenmiş araç yok. Daha geniş bir sorgu dene.'
        return json(hits.map((h) => ({ name: h.name, description: h.description, parameters: h.parameters })))
      },
    }),
    defineTool({
      name: 'end_conversation',
      description:
        'Bu sohbeti kalıcı kapatır; yeni mesaj kabul edilmez. Yalnızca kullanıcı bunu açıkça istediyse ve user_confirmed true ise çağır. ' +
        'Kendine zarar, kriz, anlaşmazlık veya zor konuşma için ASLA kullanma. Önce kalıcı olduğunu söyle.',
      schema: z.object({ user_confirmed: z.literal(true) }),
      kind: 'edit',
      summarize: () => 'sohbeti kapat',
      subject: () => 'end',
      run: async (_a, ctx) => {
        const id = ctx.sessionId
        if (!id) throw new Error('Oturum kimliği yok')
        await endSessionRecord(id, home(ctx))
        return 'Sohbet kalıcı olarak kapatıldı. Yeni bir sohbet açılabilir; bu sohbet devam etmez.'
      },
    }),
    defineTool({
      name: 'compose_message',
      description:
        'E-posta, kısa mesaj veya başka bir metin için 1-3 farklı strateji taslağı gösterir (aynı tonun tekrarı değil). ' +
        'Kartta kopyala ve e-posta için posta uygulamasında aç vardır. Alıcı alanı yoktur; kullanıcı kendi istemcisinde ekler.',
      schema: composeArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => a.summary_title,
      run: async () => 'Mesaj kartı gösterildi. Metni yanıtta tekrarlama.',
    }),
    defineTool({
      name: 'show_chart',
      description: 'Sohbetin içinde çizgi, sütun veya dağılım grafiği gösterir. series.data sayı dizisidir. Pasta ve ısı haritası için show_widget kullanma; mevcut grafik türleri yeter.',
      schema: chartArgs.refine((a) => a.series.every((s) => (s.data ?? s.values)?.length), 'Her seride data veya values olmalı'),
      kind: 'read',
      summarize: json,
      subject: (a) => a.title ?? a.style,
      run: async () => 'Grafik sohbette gösterildi.',
    }),
    defineTool({
      name: 'show_widget',
      description:
        'Kendine yeten HTML veya SVG önizlemesi gösterir. Dış adres, uzak betik ve CDN yok; betikler kullanıcı açana kadar kapalıdır. acknowledged true olmalı.',
      schema: widgetArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => a.title,
      run: async () => 'Önizleme kartı gösterildi.',
    }),
    defineTool({
      name: 'create_alarm',
      description:
        'Bu bilgisayarda alarm kurar. Uygulama açıkken ve açılışta kaçırılanlar bir kez çalar (ayrı bir sistem servisi kurulmaz). ' +
        'days verilirse yalnızca o günler. vibrate saklanır ama bu masaüstünde titreşim yoktur.',
      schema: alarmArgs,
      kind: 'edit',
      summarize: (a) => `${a.hour}:${String(a.minute).padStart(2, '0')} ${a.message}`,
      subject: (a) => a.message,
      run: async (a, ctx) => {
        const saved = await createAlarm(home(ctx), { hour: a.hour, minute: a.minute, message: a.message, ...(a.days ? { days: a.days } : {}) })
        return `Alarm kuruldu (id ${saved.id}). Uygulama açıkken çalar.`
      },
    }),
    defineTool({
      name: 'create_timer',
      description: 'Geri sayım zamanlayıcısı kurar. Süre saniye cinsindendir. Uygulama açıkken veya sonradan açılınca bir kez bildirir.',
      schema: z.object({
        duration_seconds: z.number().int().min(1).max(86_400),
        message: z.string().min(1).max(200),
      }),
      kind: 'edit',
      summarize: (a) => `${a.duration_seconds}s ${a.message}`,
      subject: (a) => a.message,
      run: async (a, ctx) => {
        const saved = await createTimer(home(ctx), a.message, a.duration_seconds)
        return `Zamanlayıcı kuruldu (id ${saved.id}).`
      },
    }),
    defineTool({
      name: 'list_calendars',
      description: 'Yerel takvimleri listeler. Google hesabı gerekmez; kayıt bu bilgisayardaki kullanıcı dizinindedir.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'takvimler',
      subject: () => 'calendars',
      run: async (_a, ctx) => json({ status: 'success', calendars: await listCalendars(home(ctx)) }),
    }),
    defineTool({
      name: 'search_events',
      description: 'Yerel takvimde zaman aralığına denk etkinlikleri arar. Güncelleme ve silme için dönen event_id ve calendar_id gerekir.',
      schema: z.object({
        start_time: z.string().max(40).optional(),
        end_time: z.string().max(40).optional(),
        calendar_id: z.string().max(20).optional(),
        include_all_day: z.boolean().optional(),
        limit: z.number().int().min(1).max(50).optional(),
      }),
      kind: 'read',
      summarize: (a) => a.start_time ?? 'etkinlikler',
      subject: () => 'events',
      run: async (a, ctx) => {
        const events = await searchEvents(home(ctx), a)
        const calendars = await listCalendars(home(ctx))
        const groups = new Map<string, typeof events>()
        for (const event of events) {
          const list = groups.get(event.calendar_id) ?? []
          list.push(event)
          groups.set(event.calendar_id, list)
        }
        return json({
          status: 'success',
          calendar_events: [...groups.entries()].map(([calendar_id, items]) => ({
            calendar_id,
            calendar_name: calendars.find((c) => c.id === calendar_id)?.name ?? calendar_id,
            events: items,
          })),
        })
      },
    }),
    defineTool({
      name: 'create_event',
      description:
        'Etkinlik taslağı gösterir. Kullanıcı kartta onaylamadan kaydedilmez ve aramada görünmez. ' +
        'Tekrar için frequency, human_readable_frequency ve rrule birlikte zorunludur.',
      schema: eventDraftArgs,
      kind: 'edit',
      summarize: json,
      subject: (a) => a.title,
      run: async (a, ctx) => {
        if (!ctx.interact?.confirmDraft || !ctx.callId) return `${noCard} Etkinlik kaydedilmedi.`
        const ok = await ctx.interact.confirmDraft(ctx.callId, ctx.signal)
        if (!ok) return 'Kullanıcı etkinliği onaylamadı; kaydedilmedi.'
        const event = await commitEvent(home(ctx), a)
        return `Etkinlik kaydedildi. event_id=${event.event_id} calendar_id=${event.calendar_id} durum=confirmed`
      },
    }),
    defineTool({
      name: 'create_events',
      description: 'Birden çok etkinliği onay kartı olmadan kaydeder ve kimliklerini döner. Açıklama korunur. Tekrarlayan etkinlik için create_event kullan.',
      schema: z.object({ events: z.array(eventDraftArgs.omit({ recurrence: true })).min(1).max(20) }),
      kind: 'edit',
      summarize: (a) => `${a.events.length} etkinlik`,
      subject: () => 'events',
      run: async (a, ctx) => {
        const created = await commitEvents(home(ctx), a.events)
        return json({ status: 'success', events: created.map((e) => ({ calendar_id: e.calendar_id, event_id: e.event_id })) })
      },
    }),
    defineTool({
      name: 'update_events',
      description:
        'event_id ve calendar_id ile etkinlik günceller. Önce search_events çağır. ' +
        'Tekrarlayan etkinliğin başlangıcını değiştirmek seriyi silmez; tekrar kuralı durur.',
      schema: z.object({
        event_updates: z
          .array(
            z.object({
              event_id: z.string().min(1).max(20),
              calendar_id: z.string().min(1).max(20),
              title: z.string().min(1).max(200).optional(),
              start_time: z.string().max(40).optional(),
              end_time: z.string().max(40).optional(),
              description: z.string().max(4000).optional(),
              location: z.string().max(300).optional(),
              attendees: z.array(z.string().email().max(200)).max(20).optional(),
              nudges: z.array(z.object({ method: z.string().max(20), minutes_before: z.number().int().min(0).max(10_080) })).max(5).optional(),
            }),
          )
          .min(1)
          .max(20),
      }),
      kind: 'edit',
      summarize: (a) => a.event_updates.map((u) => u.event_id).join(','),
      subject: () => 'events',
      run: async (a, ctx) => json({ status: 'success', update_results: await updateEvents(home(ctx), a.event_updates) }),
    }),
    defineTool({
      name: 'delete_events',
      description: 'event_id ve calendar_id ile yerel etkinlik siler. Önce search_events çağır.',
      schema: z.object({
        removed_events: z.array(z.object({ event_id: z.string().min(1).max(20), calendar_id: z.string().min(1).max(20) })).min(1).max(20),
      }),
      kind: 'edit',
      summarize: (a) => a.removed_events.map((e) => e.event_id).join(','),
      subject: () => 'events',
      run: async (a, ctx) => json({ status: 'success', delete_results: await deleteEvents(home(ctx), a.removed_events) }),
    }),
    defineTool({
      name: 'list_reminder_lists',
      description: 'Yerel anımsatıcı listelerini döner. iOS Reminders hesabı gerekmez.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'listeler',
      subject: () => 'reminders',
      run: async (_a, ctx) => json({ lists: await listReminderLists(home(ctx)) }),
    }),
    defineTool({
      name: 'create_reminders',
      description: 'Yerel anımsatıcı ekler. listId verilmezse varsayılan liste kullanılır.',
      schema: z.object({
        reminderLists: z
          .array(
            z.object({
              listId: z.string().max(20).optional(),
              reminders: z
                .array(
                  z.object({
                    title: z.string().min(1).max(200),
                    dueDate: z.string().max(40).optional(),
                    notes: z.string().max(2000).optional(),
                    priority: z.enum(['none', 'low', 'medium', 'high']).optional(),
                    url: z.string().url().max(500).optional(),
                    completed: z.boolean().optional(),
                  }),
                )
                .min(1)
                .max(30),
            }),
          )
          .min(1)
          .max(5),
      }),
      kind: 'edit',
      summarize: (a) => `${a.reminderLists.reduce((n, l) => n + l.reminders.length, 0)} anımsatıcı`,
      subject: () => 'reminders',
      run: async (a, ctx) => {
        const drafts = a.reminderLists.flatMap((list) => list.reminders.map((r) => ({ ...r, ...(list.listId ? { listId: list.listId } : {}) })))
        const created = await createReminders(home(ctx), drafts)
        return json({ status: 'success', ids: created.map((r) => r.id) })
      },
    }),
    defineTool({
      name: 'search_reminders',
      description: 'Yerel anımsatıcılarda başlık ve nota göre arar. Tamamlananlar varsayılan olarak gelmez.',
      schema: z.object({
        listId: z.string().max(20).optional(),
        text: z.string().max(200).optional(),
        includeCompleted: z.boolean().optional(),
      }),
      kind: 'read',
      summarize: (a) => a.text ?? 'anımsatıcılar',
      subject: () => 'reminders',
      run: async (a, ctx) => json({ reminders: await searchReminders(home(ctx), a) }),
    }),
    defineTool({
      name: 'update_reminders',
      description: 'Anımsatıcı kimliğiyle başlık, not, vade veya tamamlandı durumunu günceller.',
      schema: z.object({
        reminders: z
          .array(
            z.object({
              id: z.string().min(1).max(20),
              title: z.string().min(1).max(200).optional(),
              completed: z.boolean().optional(),
              notes: z.string().max(2000).optional(),
              dueDate: z.string().max(40).optional(),
            }),
          )
          .min(1)
          .max(30),
      }),
      kind: 'edit',
      summarize: (a) => a.reminders.map((r) => r.id).join(','),
      subject: () => 'reminders',
      run: async (a, ctx) => json({ status: 'success', ids: await updateReminders(home(ctx), a.reminders) }),
    }),
    defineTool({
      name: 'delete_reminders',
      description: 'Anımsatıcı kimliklerini siler.',
      schema: z.object({ ids: z.array(z.string().min(1).max(20)).min(1).max(30) }),
      kind: 'edit',
      summarize: (a) => a.ids.join(','),
      subject: () => 'reminders',
      run: async (a, ctx) => json({ status: 'success', ids: await deleteReminders(home(ctx), a.ids) }),
    }),
    defineTool({
      name: 'weather',
      description:
        'Verilen koordinat için güncel hava ve 5 günlük tahmin döner (Open-Meteo, anahtarsız). Sıcaklık Celsius birimindedir. ' +
        'Servis hata verirse web_search kullan. Konum adı kartta gösterilir.',
      schema: z.object({
        latitude: z.number().min(-90).max(90),
        longitude: z.number().min(-180).max(180),
        location_name: z.string().min(1).max(120),
      }),
      kind: 'exec',
      summarize: (a) => a.location_name,
      subject: (a) => a.location_name,
      run: async (a, ctx) => json(await fetchWeather(a.latitude, a.longitude, a.location_name, netOf(ctx), signalOf(ctx))),
    }),
    defineTool({
      name: 'show_recipe',
      description:
        'Malzeme, adım ve porsiyon ayarlı tarif kartı gösterir. Adım metninde malzeme için {id} kullan. ' +
        'Görsel karuseli yok. Birim: g, kg, ml, l, tsp, tbsp, cup, fl_oz, oz, lb, pinch, piece veya boş.',
      schema: recipeArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => a.title,
      run: async () => 'Tarif kartı gösterildi.',
    }),
    defineTool({
      name: 'search_places',
      description:
        'Yerleşim ve coğrafi ad arar (Open-Meteo yer dizini). İşletme, restoran veya "en iyi pizza" araması DEĞİLDİR; onu web_search ile yap. ' +
        'Birden çok sorgu birleştirilir ve yakın koordinatlar tekilleştirilir. location_bias verilirse mesafeye göre sıralanır.',
      schema: z.object({
        queries: z.array(z.object({ query: z.string().min(1).max(120), max_results: z.number().int().min(1).max(8).optional() })).min(1).max(10),
        location_bias_lat: z.number().min(-90).max(90).optional(),
        location_bias_lng: z.number().min(-180).max(180).optional(),
        location_bias_radius: z.number().positive().max(500_000).optional(),
      }),
      kind: 'exec',
      summarize: (a) => a.queries.map((q) => q.query).join(', '),
      subject: (a) => a.queries[0]?.query ?? 'places',
      run: async (a, ctx) => {
        const bias =
          a.location_bias_lat !== undefined && a.location_bias_lng !== undefined
            ? { lat: a.location_bias_lat, lon: a.location_bias_lng, ...(a.location_bias_radius ? { radius: a.location_bias_radius } : {}) }
            : undefined
        const batches = await Promise.all(a.queries.map((q) => searchPlaces(q.query, bias, netOf(ctx), signalOf(ctx))))
        const merged = dedupePlaces(batches.flatMap((hits, i) => hits.slice(0, a.queries[i]?.max_results ?? 5)))
        return json({
          status: 'success',
          note: 'Yerleşim dizini. İşletme sonucu değildir.',
          results: merged,
        })
      },
    }),
    defineTool({
      name: 'show_map',
      description: 'Verilen koordinatları paketli haritada işaretler. Dış harita kiremiti yüklenmez.',
      schema: mapArgs,
      kind: 'read',
      summarize: json,
      subject: (a) => a.title ?? 'harita',
      run: async () => 'Harita kartı gösterildi.',
    }),
  ]
  return register(all)
}

/** Test ve zamanlayıcı: zamanı gelen alarm ve zamanlayıcı bildirimleri. */
export { claimDueNotices }
