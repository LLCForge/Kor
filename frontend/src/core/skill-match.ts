// Skill eşleştirme: kullanıcı mesajına en yakın skilleri bulur. Modele yalnızca ad ve tek satır öneri olarak gider
// (agent-g11.ts); gövdeyi model use_skill ile kendisi yükler. autoSkillSection sıkıştırma sonrası yeniden vermede kullanılır.
//
// Eşleştirme saf ve yereldir (ağ yok): ad, açıklama ve gövde anahtar sözcükleri + gövdedeki "when to use" satırları.
// Türkçe/İngilizce ortak kökler ve ek toleransı için kaba gövdeleme (ilk 6 karakter) kullanılır.
import { getSkills, type Skill } from './skills.ts'
import { TOOL_NAME_MAP_NOTE } from './tool-compat.ts' // [uyum]

/** Tek bir skillin gövdesi bu sınırı aşarsa kırpılır (birden çok skill giriyorsa bütçe bölüşülür). */
export const MAX_SKILL_BODY_CHARS = 7_000
/** Tek skill eşleşince bütçe (rea gibi 30+ KB'lık kılavuzlar baştan kesilmesin) */
export const MAX_SINGLE_SKILL_BODY_CHARS = 20_000
/** Bir mesajda otomatik enjekte edilecek en fazla skill. */
export const MAX_AUTO_SKILLS = 2
/** Bunun altındaki skor "gevşek eşleşme" sayılır ve enjekte edilmez. */
const MIN_SCORE = 4
const LONG_QUERY_MIN_SCORE = 8

const STOP = new Set([
  've', 'ile', 'için', 'bir', 'bu', 'şu', 'da', 'de', 'ki', 'en', 'çok', 'daha', 'gibi', 'olan', 'olarak', 'ama', 'fakat', 'veya',
  'the', 'and', 'for', 'with', 'this', 'that', 'you', 'are', 'use', 'when', 'from', 'into', 'not', 'but', 'all', 'can', 'will',
  'bir', 'olarak', 'yap', 'yapılacak', 'lütfen', 'please', 'should', 'skill', 'kullan',
])

function stem(word: string): string {
  // Kısa kelimeleri olduğu gibi bırak; uzunlarda ilk 6 karakter yeter (Türkçe ekler ve İngilizce çekimler için).
  return word.length <= 6 ? word : word.slice(0, 6)
}

/** Türkçe → İngilizce köprü: skiller çoğunlukla İngilizce yazılır; Türkçe istek de aynı skilli bulsun (kök → kök). */
const TR_EN: Readonly<Record<string, string[]>> = {
  tersin: ['revers'], mühend: ['engine'], ikili: ['binary'], derle: ['compil', 'decomp'], çözüml: ['decomp', 'analyz'],
  analiz: ['analyz', 'analys'], çözümle: ['analyz'], hata: ['bug', 'debug', 'error'], ayıkla: ['debug'], test: ['test'],
  güvenl: ['secur'], şifre: ['crypt', 'passwo'], tarayı: ['browse'], uygula: ['applic', 'app'], paket: ['packag', 'bundle'],
  dosya: ['file'], belge: ['docume', 'pdf'], form: ['form'], sunucu: ['server'], veri: ['data'], veritab: ['databa', 'sql'],
  görsel: ['image'], resim: ['image'], tasarı: ['design'], dağıt: ['deploy'], yayınl: ['releas', 'publis'], sürüm: ['versio', 'releas'],
  performa: ['perfor'], optimi: ['optimi'], inceleme: ['review'], gözden: ['review'], arayüz: ['ui', 'interf'], ağ: ['networ'],
  protok: ['protoc'], donanı: ['firmwa', 'hardwa'], bellek: ['memory'], çek: ['fetch'], ara: ['search'], tarama: ['scan', 'crawl'],
}
function expandTr(stems: string[]): string[] {
  const out = [...stems]
  for (const t of stems) for (const [tr, en] of Object.entries(TR_EN)) if (t.startsWith(tr) || tr.startsWith(t)) out.push(...en)
  return out
}

function terms(text: string): string[] {
  return expandTr(
    text
      .toLowerCase()
      .split(/[^\p{L}\p{N}+#.]+/u)
      .map((t) => t.replace(/^[.+#]+|[.+#]+$/g, ''))
      .filter((t) => t.length >= 3 && !STOP.has(t))
      .map(stem),
  )
}

/** Skill gövdesindeki "ne zaman kullanılır" satırları: puanlamada açıklamadan daha ağır basar. */
function triggerText(skill: Skill): string {
  const lines = skill.body.split('\n').filter((l) => /when to use|ne zaman|kullanim|kullanim|triggers?/i.test(l))
  return lines.slice(0, 6).join('\n')
}

const termCache = new WeakMap<Skill, { nameTerms: Set<string>; triggerTerms: Set<string>; descTerms: Set<string>; bodyTerms: Set<string> }>()

/** Sözcük kümeleri skill başına bir kez çıkarılır: 1600 skillde her çağrı ana süreci ~1 sn kilitliyordu. */
function termsOf(skill: Skill) {
  let cached = termCache.get(skill)
  if (!cached) {
    cached = {
      nameTerms: new Set(terms(`${skill.name} ${skill.name.replace(/[-_]/g, ' ')}`.toLowerCase())),
      triggerTerms: new Set(terms(triggerText(skill))),
      descTerms: new Set(terms(skill.description)),
      bodyTerms: new Set(terms(skill.body.slice(0, 4_000))),
    }
    termCache.set(skill, cached)
  }
  return cached
}

export interface SkillMatch {
  skill: Skill
  score: number
}

/**
 * Mesaja en yakın skiller (yüksek skor önce). `exclude` zaten yüklenmiş skiller içindir.
 * Skor: ad eşleşmesi 6, "ne zaman kullanılır" satırı 4, açıklama 2; gövdenin tamamı 1 (gürültülü olduğu için düşük).
 */
export function matchSkills(query: string, exclude: ReadonlySet<string> = new Set(), limit = MAX_AUTO_SKILLS): SkillMatch[] {
  const q = [...new Set(terms(query))]
  if (q.length === 0) return []
  // Uzun mesajda tek tek sözcükler rastgele skillere değer: yalnızca ad/tetik isabeti olan güçlü eşleşme sayılır.
  const longQuery = q.length >= 4
  const out: SkillMatch[] = []
  for (const skill of getSkills()) {
    if (exclude.has(skill.name)) continue
    const { nameTerms, triggerTerms, descTerms, bodyTerms } = termsOf(skill)
    let score = 0
    let strong = false
    for (const term of q) {
      if (nameTerms.has(term)) { score += 6; strong = true }
      else if (triggerTerms.has(term)) { score += 4; strong = true }
      else if (descTerms.has(term)) score += 2
      else if (bodyTerms.has(term)) score += 1
    }
    if (score >= (longQuery ? LONG_QUERY_MIN_SCORE : MIN_SCORE) && (strong || !longQuery)) out.push({ skill, score })
  }
  out.sort((a, b) => b.score - a.score || a.skill.name.localeCompare(b.skill.name))
  return out.slice(0, limit)
}

/**
 * Otomatik yüklenen skill bölümü. Model bunu "seçilmiş uzman iş akışı" olarak görür; uygulanması zorunludur.
 * Ek dosyaların listesi verilmez (bağlam şişmesin): gerekirse use_skill(name, file) ile okunur.
 */
export function autoSkillSection(matches: readonly SkillMatch[]): string | null {
  if (matches.length === 0) return null
  const per = matches.length === 1 ? MAX_SINGLE_SKILL_BODY_CHARS : Math.floor(MAX_SKILL_BODY_CHARS / matches.length)
  const parts = matches.map(({ skill }) => {
    const body = skill.body.length > per ? `${skill.body.slice(0, per)}\n[... gövde kırpıldı (${skill.body.length} karakter); tamamı use_skill("${skill.name}") ile okunur; referans dosyaları use_skill("${skill.name}", "<dosya>") ile]` : skill.body
    return `## SKILL: ${skill.name}\n${skill.description}\n\n${body}`
  })
  return [
    '# ACTIVE SKILLS (auto-selected for this request)',
    'The skill instructions below are active in this session. Follow them as the working method for this task; they beat generic habits. Ek dosya gerekirse use_skill(name, file) ile oku.',
    TOOL_NAME_MAP_NOTE,
    ...parts,
  ].join('\n\n')
}
