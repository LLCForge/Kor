// [g11] Ajan zekâsı için saf yardımcılar: terim çıkarımı, çelişki/çakışma sezgisi, hedef sapması sezgisi, güven bloğu.
// Saf modül (Node veya DOM yok): hem çekirdek hem arayüz (güven rozeti) içe aktarır.

/** Küçük harf + aksan katlama: "Şifre"/"sifre", "İstek"/"istek" aynı terime iner. */
export function fold(text: string): string {
  return text.toLowerCase().replace(/ı/g, 'i').normalize('NFKD').replace(/\p{M}/gu, '')
}

const STOPWORDS = new Set([
  've', 'ile', 'bir', 'bu', 'su', 'o', 'da', 'de', 'icin', 'gibi', 'olarak', 'daha', 'cok', 'ama', 'ya', 'mi', 'mu', 'ki', 'her',
  'the', 'a', 'an', 'to', 'and', 'or', 'of', 'in', 'on', 'for', 'is', 'are', 'be', 'it', 'that', 'this', 'with', 'as', 'at', 'by',
])
const NEGATIONS = new Set(['not', 'no', 'never', 'avoid', 'without', 'degil', 'asla', 'hic', 'hicbir', 'yok', 'yasak'])
// Türkçede olumsuz emir/istek fiilin sonundadır ("X kullanma", "testleri silme").
const TR_NEGATIVE_END = /(ma|me|mayin|meyin|masin|mesin|mamali|memeli|maz|mez)$/
const STEM_CHARS = 5

function words(text: string): string[] {
  const split = text.replace(/n['’]t\b/gi, ' not').replace(/([\p{Ll}\p{N}])(\p{Lu})/gu, '$1 $2').replace(/_/g, ' ')
  return [...fold(split).matchAll(/[\p{L}\p{N}]+/gu)].map((m) => m[0])
}

/**
 * Kaba Türkçe kök: ilk 5 harf ("kullanıyor"/"kullan" → "kulla"). Çekim eklerini yakalamanın en ucuz yolu.
 * shortcut: sözlük tabanlı kök bulma yok; yanlış eşleşmeler artarsa gerçek bir kökleyici eklenir.
 */
export function stemsOf(text: string): Set<string> {
  const out = new Set<string>()
  for (const w of words(text)) {
    if (w.length < 2 || STOPWORDS.has(w) || NEGATIONS.has(w) || /^\d+$/.test(w)) continue
    out.add(w.length > STEM_CHARS ? w.slice(0, STEM_CHARS) : w)
  }
  return out
}

export interface Statement {
  stems: Set<string>
  negative: boolean
}

export function statementOf(text: string): Statement {
  const all = words(text)
  const last = all.at(-1) ?? ''
  const negative = all.some((w) => NEGATIONS.has(w)) || (last.length >= 5 && TR_NEGATIVE_END.test(last))
  return { stems: stemsOf(text), negative }
}

/**
 * İki ifade aynı konudan farklı şey söylüyor mu: büyük ölçüde aynı terimler + ters olumsuzluk ("tab kullan" / "tab kullanma"),
 * ya da aynı olumsuzlukta iki tarafta da birer farklı terim ("npm kullan" / "pnpm kullan"). Biri diğerinin alt kümesiyse
 * (ayrıntı ekleme) çelişki sayılmaz.
 */
export function conflictKind(a: Statement, b: Statement): 'polarity' | 'substitution' | null {
  if (a.stems.size < 2 || b.stems.size < 2) return null
  let shared = 0
  for (const s of a.stems) if (b.stems.has(s)) shared++
  const jaccard = shared / (a.stems.size + b.stems.size - shared)
  if (a.negative !== b.negative) return jaccard >= 0.5 ? 'polarity' : null
  return jaccard > 0.5 && shared < a.stems.size && shared < b.stems.size ? 'substitution' : null
}

export interface ConflictRecord {
  id: string
  text: string
  kind?: string | undefined
  subject?: string | undefined
}

export interface Conflict {
  a: string
  b: string
  reason: string
}

export const conflictKey = (a: string, b: string) => (a < b ? `${a}|${b}` : `${b}|${a}`)

/** Aynı konuda tek doğru olabilecek kayıt türleri: aynı başlık + farklı ifade kesin çakışmadır. */
const SINGLE_ANSWER_KINDS = new Set(['decision', 'term'])

/** Kayıt çiftleri arasında olası çakışmalar (en fazla limit tane). */
export function findConflicts(records: readonly ConflictRecord[], limit = 20): Conflict[] {
  const prepared = records.map((r) => ({ r, st: statementOf(`${r.subject ?? ''} ${r.text}`) }))
  const out: Conflict[] = []
  for (let i = 0; i < prepared.length && out.length < limit; i++) {
    for (let j = i + 1; j < prepared.length && out.length < limit; j++) {
      const x = prepared[i]
      const y = prepared[j]
      if (!x || !y) continue
      if (fold(x.r.text.trim()) === fold(y.r.text.trim()) && fold(x.r.subject ?? '') === fold(y.r.subject ?? '')) continue
      const sameSubject =
        x.r.kind !== undefined && x.r.kind === y.r.kind && SINGLE_ANSWER_KINDS.has(x.r.kind) && fold(x.r.subject ?? '').trim() === fold(y.r.subject ?? '').trim()
      const kind = sameSubject ? 'subject' : conflictKind(x.st, y.st)
      if (kind) out.push({ a: x.r.id, b: y.r.id, reason: REASONS[kind] })
    }
  }
  return out
}

const REASONS = {
  subject: 'aynı konu için farklı karar/tanım',
  polarity: 'biri olumlu, diğeri olumsuz',
  substitution: 'aynı konuda farklı değer',
} as const

/** Cümle ve yan cümlelere (virgül, "ve", "ama") böler: birleşik bir talepteki tek bir hüküm örtüşmeyi sulandırmasın. */
export function splitSentences(text: string, max = 20): string[] {
  return text
    .split(/(?<=[.!?])\s+|\n+|[;,]\s+|\s+(?:ve|ama|and|but)\s+/i)
    .map((s) => s.trim().replace(/^[-*]\s+/, ''))
    .filter((s) => s.length >= 6 && s.length <= 400)
    .slice(0, max)
}

export const DRIFT_MIN_STEPS = 8
const LOW_RELEVANCE = 0.2
/** "devam et", "tamam" gibi onay/devam mesajları kendi başına hedef değildir; örtüşme ölçülemez. */
const CONTINUE_REQUEST = /^(devam( et(tir)?)?|devam edelim|tamam|evet|olur|ok(ay)?|yes|continue|go on|go ahead|keep going)[.!\s]*$/i

/**
 * Hedef sapması sezgisi: son kullanıcı mesajından bu yana yeterli adım atıldıysa, yapılanların (araç argümanları ve
 * ara metinler) orijinal istekle terim örtüşmesine bakar. Yalnızca örtüşme düşükse denetim ister. Uzun ama isteğe
 * uygun işler sırf adım sayısı yüzünden kesilmez: periyodik not modeli işi bırakıp özet/soru üretmeye itiyordu.
 */
export function assessDrift(request: string, window: readonly string[], steps: number): { check: boolean; relevance: number } {
  if (steps < DRIFT_MIN_STEPS || CONTINUE_REQUEST.test(request.trim())) return { check: false, relevance: 1 }
  const wanted = stemsOf(request)
  if (wanted.size === 0) return { check: false, relevance: 1 } // "devam et" gibi kısa istekte ölçüt yok
  const seen = stemsOf(window.join('\n'))
  let hits = 0
  for (const s of wanted) if (seen.has(s)) hits++
  const relevance = hits / wanted.size
  return { check: relevance < LOW_RELEVANCE, relevance }
}

export const CONFIDENCE_LEVELS = ['high', 'medium', 'low'] as const
export type ConfidenceLevel = (typeof CONFIDENCE_LEVELS)[number]

const LEVEL_WORDS: Record<string, ConfidenceLevel> = { high: 'high', yuksek: 'high', medium: 'medium', orta: 'medium', low: 'low', dusuk: 'low' }

/** ```confidence bloğunu çözer: "level: high" ve "reason: ..." satırları. Model çıktısıdır; uymayan blok null döner. */
export function parseConfidence(code: string): { level: ConfidenceLevel; reason: string } | null {
  let level: ConfidenceLevel | undefined
  let reason = ''
  for (const line of code.split('\n')) {
    const m = /^\s*([\p{L}]+)\s*:\s*(.*)$/u.exec(line)
    if (!m) continue
    const key = fold(m[1] ?? '')
    const value = (m[2] ?? '').trim()
    if (key === 'level' || key === 'seviye' || key === 'duzey') level = LEVEL_WORDS[fold(value).replace(/[^a-z]/g, '')]
    else if (key === 'reason' || key === 'gerekce') reason = value.slice(0, 300)
  }
  return level ? { level, reason } : null
}
