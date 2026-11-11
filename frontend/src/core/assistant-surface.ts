// Yerel asistan yüzeyi: saat, takvim, anımsatıcı, alarm, hava ve yer araması.
// Üçüncü parti hesap yok; veri kullanıcı dizininde durur. Ağ yalnızca sabit Open-Meteo uçlarına gider.
import { execFile } from 'node:child_process'
import { mkdir, readFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { atomicWriteFile } from './atomic-write-g10.ts'
import { rankByBm25 } from './bm25-g26.ts' // [g26]
import { USER_CONFIG_DIR } from './settings.ts'

const FILE = 'assistant.json'
const OPEN_METEO = 'https://api.open-meteo.com/v1/forecast'
const GEOCODE = 'https://geocoding-api.open-meteo.com/v1/search'
// Open-Meteo'nun ters kodlama uç noktası yok (/v1/reverse 404 döner); BigDataCloud'un anahtarsız istemci uç noktası kullanılır.
const REVERSE = 'https://api.bigdatacloud.net/data/reverse-geocode-client'
const UA = 'Kor/1.0'

export const WEEKDAYS = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'] as const
export type Weekday = (typeof WEEKDAYS)[number]

const weekdaySet = new Set<string>(WEEKDAYS)

export interface ZonedClock {
  iso: string
  timeZone: string
  weekday: Weekday
  minutesOfDay: number
  dateKey: string
}

/** Cihaz saat diliminde ISO 8601 (ofset iki noktalı). Saat dilimi işletim sisteminden gelir, IP'den değil. */
export function formatLocalTime(now = new Date(), timeZone = localTimeZone()): ZonedClock {
  const parts = zonedParts(now, timeZone)
  const hour = parts.hour === '24' ? '00' : parts.hour
  const weekday = englishWeekday(now, timeZone)
  return {
    iso: `${parts.year}-${parts.month}-${parts.day}T${hour}:${parts.minute}:${parts.second}.${parts.fractionalSecond}${offsetOf(now, timeZone)}`,
    timeZone,
    weekday,
    minutesOfDay: Number(hour) * 60 + Number(parts.minute),
    dateKey: `${parts.year}-${parts.month}-${parts.day}`,
  }
}

export function localTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
}

function zonedParts(now: Date, timeZone: string): Record<string, string> {
  const dtf = new Intl.DateTimeFormat('en-US', {
    timeZone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hourCycle: 'h23',
    fractionalSecondDigits: 3,
  })
  return Object.fromEntries(dtf.formatToParts(now).filter((p) => p.type !== 'literal').map((p) => [p.type, p.value]))
}

function offsetOf(now: Date, timeZone: string): string {
  const name = new Intl.DateTimeFormat('en-US', { timeZone, timeZoneName: 'longOffset' })
    .formatToParts(now)
    .find((p) => p.type === 'timeZoneName')?.value
  const m = /GMT([+-])(\d{1,2})(?::(\d{2}))?/.exec(name ?? '')
  if (!m) return 'Z'
  return `${m[1]}${m[2].padStart(2, '0')}:${m[3] ?? '00'}`
}

function englishWeekday(now: Date, timeZone: string): Weekday {
  const name = new Intl.DateTimeFormat('en-US', { timeZone, weekday: 'long' }).format(now).toLowerCase()
  return weekdaySet.has(name) ? (name as Weekday) : 'monday'
}

export interface LocationReading {
  status: 'success' | 'denied' | 'unavailable'
  latitude?: number
  longitude?: number
  accuracyMeters?: number
  geocoded?: {
    address?: string
    administrative_area?: string
    country?: string
    locality?: string
    name?: string
    postal_code?: string
  }
}

/** Windows konum hizmeti. Red veya servis kapalıysa koordinat uydurulmaz. */
export function readWindowsLocation(signal: AbortSignal): Promise<LocationReading> {
  if (process.platform !== 'win32') return Promise.resolve({ status: 'unavailable' })
  const script = `
Add-Type -AssemblyName System.Device
$w = New-Object System.Device.Location.GeoCoordinateWatcher
$w.Start()
$deadline = [datetime]::UtcNow.AddSeconds(8)
while ($w.Status -ne 'Ready' -and $w.Permission -ne 'Denied' -and [datetime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 200 }
if ($w.Permission -eq 'Denied') { 'denied'; exit 0 }
if ($w.Status -ne 'Ready' -or $w.Position.Location.IsUnknown) { 'unavailable'; exit 0 }
$loc = $w.Position.Location
'{0},{1},{2}' -f $loc.Latitude.ToString([cultureinfo]::InvariantCulture), $loc.Longitude.ToString([cultureinfo]::InvariantCulture), $loc.HorizontalAccuracy.ToString([cultureinfo]::InvariantCulture)
$w.Stop()
`.trim()
  return new Promise((resolve) => {
    execFile(
      'powershell.exe',
      ['-NoProfile', '-NonInteractive', '-Command', script],
      { timeout: 12_000, windowsHide: true, signal },
      (err, stdout) => {
        if (err) {
          resolve({ status: 'unavailable' })
          return
        }
        resolve(parseLocationProbe(String(stdout)))
      },
    )
  })
}

export function parseLocationProbe(stdout: string): LocationReading {
  const line = stdout.trim().split(/\r?\n/).at(-1)?.trim() ?? ''
  if (line === 'denied') return { status: 'denied' }
  if (line === 'unavailable' || !line) return { status: 'unavailable' }
  const [lat, lon, acc] = line.split(',').map((s) => Number(s))
  if (!Number.isFinite(lat) || !Number.isFinite(lon)) return { status: 'unavailable' }
  return {
    status: 'success',
    latitude: lat,
    longitude: lon,
    ...(Number.isFinite(acc) ? { accuracyMeters: acc } : {}),
  }
}


const reverseSchema = z.object({
  city: z.string().optional(),
  locality: z.string().optional(),
  principalSubdivision: z.string().optional(),
  countryName: z.string().optional(),
  postcode: z.string().optional(),
})

export function parseReverseGeocode(raw: unknown): LocationReading['geocoded'] | undefined {
  const parsed = reverseSchema.safeParse(raw)
  if (!parsed.success) return undefined
  const { city, locality, principalSubdivision, countryName, postcode } = parsed.data
  const place = (city || locality || '').trim()
  if (!place && !countryName) return undefined
  return {
    ...(place ? { locality: place, name: place } : {}),
    ...(principalSubdivision ? { administrative_area: principalSubdivision } : {}),
    ...(countryName ? { country: countryName } : {}),
    ...(postcode ? { postal_code: postcode } : {}),
  }
}

export async function reverseGeocode(lat: number, lon: number, net: typeof fetch, signal: AbortSignal): Promise<LocationReading['geocoded'] | undefined> {
  const url = `${REVERSE}?latitude=${lat}&longitude=${lon}&localityLanguage=tr`
  return parseReverseGeocode(await getJson(url, net, signal))
}

// ---------- depo ----------

const calendarSchema = z.object({
  id: z.string(),
  name: z.string(),
  access_level: z.enum(['owner', 'readwrite', 'readonly']),
  account_name: z.string(),
  owner_account: z.string(),
  color: z.string(),
  is_primary: z.boolean().optional(),
})
export type CalendarInfo = z.infer<typeof calendarSchema>

const recurrenceSchema = z.object({
  frequency: z.string().min(1).max(40),
  human_readable_frequency: z.string().min(1).max(80),
  rrule: z.string().min(1).max(300),
})

const eventSchema = z.object({
  event_id: z.string(),
  calendar_id: z.string(),
  title: z.string(),
  start_time: z.string(),
  end_time: z.string(),
  all_day: z.boolean().optional(),
  description: z.string().optional(),
  location: z.string().optional(),
  status: z.enum(['confirmed', 'tentative']),
  recurrence: recurrenceSchema.optional(),
  attendees: z.array(z.string()).optional(),
  nudges: z.array(z.object({ method: z.string(), minutes_before: z.number() })).optional(),
})
export type CalEvent = z.infer<typeof eventSchema>

const reminderSchema = z.object({
  id: z.string(),
  listId: z.string(),
  title: z.string(),
  dueDate: z.string().optional(),
  notes: z.string().optional(),
  priority: z.enum(['none', 'low', 'medium', 'high']).optional(),
  url: z.string().optional(),
  completed: z.boolean(),
})
export type Reminder = z.infer<typeof reminderSchema>

const listSchema = z.object({ id: z.string(), name: z.string() })
const alarmSchema = z.object({
  id: z.string(),
  hour: z.number().int(),
  minute: z.number().int(),
  message: z.string(),
  days: z.array(z.enum(WEEKDAYS)).optional(),
  lastFiredKey: z.string().optional(),
})
const timerSchema = z.object({
  id: z.string(),
  message: z.string(),
  fireAt: z.number(),
  fired: z.boolean(),
})

const fileSchema = z.object({
  nextId: z.number().int().positive(),
  calendars: z.array(calendarSchema),
  events: z.array(eventSchema),
  lists: z.array(listSchema),
  reminders: z.array(reminderSchema),
  alarms: z.array(alarmSchema),
  timers: z.array(timerSchema),
  endedSessions: z.array(z.string()),
})
type AssistantFile = z.infer<typeof fileSchema>

function emptyFile(): AssistantFile {
  return {
    nextId: 2,
    calendars: [
      {
        id: '1',
        name: 'Kişisel',
        access_level: 'owner',
        account_name: 'local',
        owner_account: 'local',
        color: '#7C6AF7',
        is_primary: true,
      },
    ],
    events: [],
    lists: [{ id: '1', name: 'Genel' }],
    reminders: [],
    alarms: [],
    timers: [],
    endedSessions: [],
  }
}

const chains = new Map<string, Promise<unknown>>()

function locked<T>(dir: string, fn: () => Promise<T>): Promise<T> {
  const prev = chains.get(dir) ?? Promise.resolve()
  const run = prev.then(fn, fn)
  chains.set(dir, run.then(() => undefined, () => undefined))
  return run
}

async function loadFile(dir: string): Promise<AssistantFile> {
  try {
    const parsed = fileSchema.safeParse(JSON.parse(await readFile(path.join(dir, FILE), 'utf8')))
    if (!parsed.success) throw new Error('Yerel asistan deposu bozuk. Dosyayı elle düzelt veya kaldır.')
    return parsed.data
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') return emptyFile()
    throw err
  }
}

async function saveFile(dir: string, data: AssistantFile): Promise<void> {
  await mkdir(dir, { recursive: true })
  await atomicWriteFile(path.join(dir, FILE), JSON.stringify(data, null, 2))
}

function takeId(data: AssistantFile): string {
  const id = String(data.nextId)
  data.nextId += 1
  return id
}

export function assistantHome(explicit?: string): string {
  return explicit || USER_CONFIG_DIR
}

export interface EventDraft {
  calendar_id?: string
  title: string
  start_time: string
  end_time?: string
  all_day?: boolean
  description?: string
  location?: string
  recurrence?: z.infer<typeof recurrenceSchema>
}

function materialize(data: AssistantFile, draft: EventDraft, status: 'confirmed' | 'tentative'): CalEvent {
  const calendarId = draft.calendar_id ?? data.calendars.find((c) => c.is_primary)?.id ?? data.calendars[0]?.id
  if (!calendarId || !data.calendars.some((c) => c.id === calendarId)) throw new Error('Takvim bulunamadı')
  const start = new Date(draft.start_time)
  if (Number.isNaN(start.getTime())) throw new Error('start_time geçerli bir ISO 8601 zamanı olmalı')
  const end = draft.end_time ? new Date(draft.end_time) : new Date(start.getTime() + (draft.all_day ? 86_400_000 : 3_600_000))
  if (Number.isNaN(end.getTime()) || end <= start) throw new Error('end_time başlangıçtan sonra olmalı')
  return {
    event_id: takeId(data),
    calendar_id: calendarId,
    title: draft.title,
    start_time: draft.start_time,
    end_time: draft.end_time ?? end.toISOString(),
    ...(draft.all_day ? { all_day: true } : {}),
    ...(draft.description ? { description: draft.description } : {}),
    ...(draft.location ? { location: draft.location } : {}),
    status,
    ...(draft.recurrence ? { recurrence: draft.recurrence } : {}),
  }
}

export function listCalendars(dir: string): Promise<CalendarInfo[]> {
  return locked(dir, async () => (await loadFile(dir)).calendars)
}

export interface EventQuery {
  start_time?: string
  end_time?: string
  calendar_id?: string
  include_all_day?: boolean
  limit?: number
}

export function searchEvents(dir: string, query: EventQuery): Promise<CalEvent[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const start = query.start_time ? Date.parse(query.start_time) : Number.NEGATIVE_INFINITY
    const end = query.end_time ? Date.parse(query.end_time) : Number.POSITIVE_INFINITY
    if (Number.isNaN(start) || Number.isNaN(end)) throw new Error('Zaman aralığı geçerli ISO 8601 olmalı')
    const includeAll = query.include_all_day !== false
    return data.events
      .filter((e) => (query.calendar_id ? e.calendar_id === query.calendar_id : true))
      .filter((e) => includeAll || !e.all_day)
      .filter((e) => {
        const es = Date.parse(e.start_time)
        const ee = Date.parse(e.end_time)
        return es < end && ee > start
      })
      .slice(0, query.limit ?? 20)
  })
}

export function commitEvent(dir: string, draft: EventDraft): Promise<CalEvent> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const event = materialize(data, draft, 'confirmed')
    data.events.push(event)
    await saveFile(dir, data)
    return event
  })
}

export function commitEvents(dir: string, drafts: readonly EventDraft[]): Promise<CalEvent[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const created = drafts.map((d) => materialize(data, d, 'confirmed'))
    data.events.push(...created)
    await saveFile(dir, data)
    return created
  })
}

export interface EventUpdate {
  event_id: string
  calendar_id: string
  title?: string
  start_time?: string
  end_time?: string
  description?: string
  location?: string
  attendees?: string[]
  nudges?: { method: string; minutes_before: number }[]
}

export async function updateEvents(dir: string, updates: readonly EventUpdate[]): Promise<{ calendar_id: string; event_id: string }[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const results: { calendar_id: string; event_id: string }[] = []
    for (const patch of updates) {
      const event = data.events.find((e) => e.event_id === patch.event_id && e.calendar_id === patch.calendar_id)
      if (!event) throw new Error(`Etkinlik bulunamadı: ${patch.event_id}`)
      if (patch.title !== undefined) event.title = patch.title
      if (patch.start_time !== undefined) event.start_time = patch.start_time
      if (patch.end_time !== undefined) event.end_time = patch.end_time
      if (patch.description !== undefined) event.description = patch.description
      if (patch.location !== undefined) event.location = patch.location
      if (patch.attendees) event.attendees = patch.attendees
      if (patch.nudges) event.nudges = patch.nudges
      // Tekrarlayan etkinlikte başlangıç değişince seriyi silmek bir hata; seri korunur.
      results.push({ calendar_id: event.calendar_id, event_id: event.event_id })
    }
    await saveFile(dir, data)
    return results
  })
}

export async function deleteEvents(dir: string, removed: readonly { event_id: string; calendar_id: string }[]): Promise<{ calendar_id: string; event_id: string }[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const results: { calendar_id: string; event_id: string }[] = []
    for (const item of removed) {
      const index = data.events.findIndex((e) => e.event_id === item.event_id && e.calendar_id === item.calendar_id)
      if (index < 0) throw new Error(`Etkinlik bulunamadı: ${item.event_id}`)
      data.events.splice(index, 1)
      results.push({ calendar_id: item.calendar_id, event_id: item.event_id })
    }
    await saveFile(dir, data)
    return results
  })
}

export function listReminderLists(dir: string): Promise<{ id: string; name: string }[]> {
  return locked(dir, async () => (await loadFile(dir)).lists)
}

export interface ReminderDraft {
  listId?: string
  title: string
  dueDate?: string
  notes?: string
  priority?: 'none' | 'low' | 'medium' | 'high'
  url?: string
  completed?: boolean
}

export function createReminders(dir: string, drafts: readonly ReminderDraft[]): Promise<Reminder[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const created = drafts.map((d) => {
      const listId = d.listId ?? data.lists[0]?.id
      if (!listId || !data.lists.some((l) => l.id === listId)) throw new Error('Anımsatıcı listesi bulunamadı')
      const item: Reminder = {
        id: takeId(data),
        listId,
        title: d.title,
        completed: d.completed === true,
        ...(d.dueDate ? { dueDate: d.dueDate } : {}),
        ...(d.notes ? { notes: d.notes } : {}),
        ...(d.priority ? { priority: d.priority } : {}),
        ...(d.url ? { url: d.url } : {}),
      }
      return item
    })
    data.reminders.push(...created)
    await saveFile(dir, data)
    return created
  })
}

export function searchReminders(dir: string, query: { listId?: string; text?: string; includeCompleted?: boolean }): Promise<Reminder[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const needle = query.text?.toLocaleLowerCase('tr') ?? ''
    return data.reminders.filter((r) => {
      if (query.listId && r.listId !== query.listId) return false
      if (!query.includeCompleted && r.completed) return false
      if (!needle) return true
      return `${r.title} ${r.notes ?? ''}`.toLocaleLowerCase('tr').includes(needle)
    })
  })
}

export function updateReminders(dir: string, patches: readonly { id: string; title?: string; completed?: boolean; notes?: string; dueDate?: string }[]): Promise<string[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const ids: string[] = []
    for (const patch of patches) {
      const item = data.reminders.find((r) => r.id === patch.id)
      if (!item) throw new Error(`Anımsatıcı bulunamadı: ${patch.id}`)
      if (patch.title !== undefined) item.title = patch.title
      if (patch.completed !== undefined) item.completed = patch.completed
      if (patch.notes !== undefined) item.notes = patch.notes
      if (patch.dueDate !== undefined) item.dueDate = patch.dueDate
      ids.push(item.id)
    }
    await saveFile(dir, data)
    return ids
  })
}

export function deleteReminders(dir: string, ids: readonly string[]): Promise<string[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    for (const id of ids) {
      const index = data.reminders.findIndex((r) => r.id === id)
      if (index < 0) throw new Error(`Anımsatıcı bulunamadı: ${id}`)
      data.reminders.splice(index, 1)
    }
    await saveFile(dir, data)
    return [...ids]
  })
}

export interface AlarmDraft {
  hour: number
  minute: number
  message: string
  days?: Weekday[]
}

export function createAlarm(dir: string, draft: AlarmDraft): Promise<{ id: string }> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const alarm = { id: takeId(data), ...draft }
    data.alarms.push(alarm)
    await saveFile(dir, data)
    return { id: alarm.id }
  })
}

export function createTimer(dir: string, message: string, durationSeconds: number, now = Date.now()): Promise<{ id: string; fireAt: number }> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const timer = { id: takeId(data), message, fireAt: now + durationSeconds * 1000, fired: false }
    data.timers.push(timer)
    await saveFile(dir, data)
    return { id: timer.id, fireAt: timer.fireAt }
  })
}

export interface DueNotice {
  title: string
  body: string
}

/** Zamanı gelen alarm ve zamanlayıcıları bir kez işaretler. Uygulama kapalıyken kaçanlar açılışta bir kez düşer. */
export function claimDueNotices(dir: string = USER_CONFIG_DIR, now = new Date()): Promise<DueNotice[]> {
  return locked(dir, async () => {
    const data = await loadFile(dir)
    const clock = formatLocalTime(now)
    const notices: DueNotice[] = []
    for (const alarm of data.alarms) {
      if (alarm.days && alarm.days.length > 0 && !alarm.days.includes(clock.weekday)) continue
      const dueMin = alarm.hour * 60 + alarm.minute
      if (clock.minutesOfDay < dueMin) continue
      const key = `${clock.dateKey}:${alarm.hour}:${alarm.minute}`
      if (alarm.lastFiredKey === key) continue
      alarm.lastFiredKey = key
      notices.push({ title: 'Alarm', body: alarm.message })
    }
    const at = now.getTime()
    for (const timer of data.timers) {
      if (timer.fired || timer.fireAt > at) continue
      timer.fired = true
      notices.push({ title: 'Zamanlayıcı', body: timer.message })
    }
    if (notices.length > 0) await saveFile(dir, data)
    return notices
  })
}

const endedKeys = new Set<string>()
const endedDirsLoaded = new Set<string>()
const endedKey = (dir: string, id: string) => `${dir}\0${id}`

/** Disk okumadan: bu süreçte kapanmış sohbet. Tur başında bekletmeden bakılır. */
export function sessionEndedSync(id: string, dir: string = USER_CONFIG_DIR): boolean {
  return endedKeys.has(endedKey(dir, id))
}

export async function endSessionRecord(id: string, dir: string): Promise<void> {
  await locked(dir, async () => {
    const data = await loadFile(dir)
    if (!data.endedSessions.includes(id)) data.endedSessions.push(id)
    if (data.endedSessions.length > 500) data.endedSessions.splice(0, data.endedSessions.length - 500)
    await saveFile(dir, data)
    endedKeys.add(endedKey(dir, id))
    endedDirsLoaded.add(dir)
  })
}

export async function isSessionEnded(id: string, dir: string = USER_CONFIG_DIR): Promise<boolean> {
  if (endedDirsLoaded.has(dir)) return sessionEndedSync(id, dir)
  const data = await loadFile(dir).catch(() => emptyFile())
  endedDirsLoaded.add(dir)
  for (const sid of data.endedSessions) endedKeys.add(endedKey(dir, sid))
  return endedKeys.has(endedKey(dir, id))
}

// ---------- hava ve yer ----------

const WMO: Record<number, string> = {
  0: 'Açık',
  1: 'Çoğunlukla açık',
  2: 'Parçalı bulutlu',
  3: 'Kapalı',
  45: 'Sis',
  48: 'Kırağılı sis',
  51: 'Hafif çisenti',
  61: 'Yağmur',
  63: 'Orta yağmur',
  65: 'Şiddetli yağmur',
  71: 'Kar',
  80: 'Sağanak',
  95: 'Gök gürültülü fırtına',
}

export function weatherLabel(code: number): string {
  return WMO[code] ?? 'Değişken'
}

export function cToF(c: number): number {
  return Math.round((c * 9) / 5 + 32)
}

const forecastSchema = z.object({
  current: z.object({
    temperature_2m: z.number(),
    weather_code: z.number(),
    precipitation: z.number().optional(),
    wind_speed_10m: z.number().optional(),
  }),
  daily: z.object({
    time: z.array(z.string()),
    weather_code: z.array(z.number()),
    temperature_2m_max: z.array(z.number()),
    temperature_2m_min: z.array(z.number()),
    precipitation_probability_max: z.array(z.number()).optional(),
  }),
})

export interface WeatherReport {
  location_name: string
  current: { temp_c: number; temp_f: number; label: string; precipitation_mm: number; wind_kmh: number }
  days: { date: string; label: string; max_c: number; min_c: number; precip_probability: number }[]
}

export function parseWeather(payload: unknown, locationName: string): WeatherReport {
  const parsed = forecastSchema.safeParse(payload)
  if (!parsed.success) throw new Error('Hava yanıtı beklenen biçimde değil')
  const c = parsed.data.current
  const d = parsed.data.daily
  return {
    location_name: locationName,
    current: {
      temp_c: c.temperature_2m,
      temp_f: cToF(c.temperature_2m),
      label: weatherLabel(c.weather_code),
      precipitation_mm: c.precipitation ?? 0,
      wind_kmh: c.wind_speed_10m ?? 0,
    },
    days: d.time.slice(0, 5).map((date, i) => ({
      date,
      label: weatherLabel(d.weather_code[i] ?? 0),
      max_c: d.temperature_2m_max[i] ?? 0,
      min_c: d.temperature_2m_min[i] ?? 0,
      precip_probability: d.precipitation_probability_max?.[i] ?? 0,
    })),
  }
}

export async function fetchWeather(lat: number, lon: number, locationName: string, net: typeof fetch, signal: AbortSignal): Promise<WeatherReport> {
  const url =
    `${OPEN_METEO}?latitude=${lat}&longitude=${lon}` +
    '&current=temperature_2m,weather_code,precipitation,wind_speed_10m' +
    '&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max' +
    '&timezone=auto&forecast_days=5'
  return parseWeather(await getJson(url, net, signal), locationName)
}

const placeSchema = z.object({
  name: z.string(),
  latitude: z.number(),
  longitude: z.number(),
  country: z.string().optional(),
  admin1: z.string().optional(),
})

export interface PlaceHit {
  name: string
  latitude: number
  longitude: number
  country?: string
  admin1?: string
  distance_km?: number
}

export function parsePlaces(payload: unknown, bias?: { lat: number; lon: number; radius?: number }): PlaceHit[] {
  const parsed = z.object({ results: z.array(placeSchema).optional() }).safeParse(payload)
  if (!parsed.success) throw new Error('Yer yanıtı beklenen biçimde değil')
  let hits: PlaceHit[] = (parsed.data.results ?? []).map((r) => ({
    name: r.name,
    latitude: r.latitude,
    longitude: r.longitude,
    ...(r.country ? { country: r.country } : {}),
    ...(r.admin1 ? { admin1: r.admin1 } : {}),
    ...(bias ? { distance_km: haversine(bias.lat, bias.lon, r.latitude, r.longitude) } : {}),
  }))
  if (bias?.radius !== undefined) hits = hits.filter((h) => (h.distance_km ?? Infinity) <= bias.radius! / 1000)
  if (bias) hits.sort((a, b) => (a.distance_km ?? 0) - (b.distance_km ?? 0))
  return hits
}

export async function searchPlaces(query: string, bias: { lat: number; lon: number; radius?: number } | undefined, net: typeof fetch, signal: AbortSignal): Promise<PlaceHit[]> {
  const url = `${GEOCODE}?name=${encodeURIComponent(query)}&count=8&language=tr&format=json`
  return parsePlaces(await getJson(url, net, signal), bias)
}

export function dedupePlaces(hits: readonly PlaceHit[]): PlaceHit[] {
  const seen = new Set<string>()
  const out: PlaceHit[] = []
  for (const hit of hits) {
    const key = `${hit.latitude.toFixed(3)},${hit.longitude.toFixed(3)},${hit.name}`
    if (seen.has(key)) continue
    seen.add(key)
    out.push(hit)
  }
  return out
}

function haversine(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const r = 6371
  const dLat = ((lat2 - lat1) * Math.PI) / 180
  const dLon = ((lon2 - lon1) * Math.PI) / 180
  const a = Math.sin(dLat / 2) ** 2 + Math.cos((lat1 * Math.PI) / 180) * Math.cos((lat2 * Math.PI) / 180) * Math.sin(dLon / 2) ** 2
  return Math.round(r * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a)) * 10) / 10
}

async function getJson(url: string, net: typeof fetch, signal: AbortSignal): Promise<unknown> {
  const res = await net(url, { signal, headers: { accept: 'application/json', 'user-agent': UA } })
  if (!res.ok) throw new Error(`Servis ${res.status} döndü. web_search ile devam et.`)
  return res.json() as Promise<unknown>
}

export interface CatalogEntry {
  name: string
  description: string
  parameters: Record<string, unknown>
}

/** [g26] BM25. Her zaman yüklü araçlar dönmez; yalnızca ertelenmiş katalog. */
export function searchCatalog(catalog: readonly CatalogEntry[], query: string, limit: number): CatalogEntry[] {
  const tokens = query.toLocaleLowerCase('tr').split(/[^\p{L}\p{N}_]+/u).filter((t) => t.length > 1)
  if (tokens.length === 0) return []
  return rankByBm25(catalog, tokens, (entry) => `${entry.name} ${entry.description}`, limit)
}
