// [g21] Belge dili algılama (#188): bağımlılıksız, yerel ve kaba. Önce yazı sistemi (Kiril, Arap, Han ...),
//      Latin yazıda en sık işlev sözcükleri + ayırt edici harfler puanlanır. Kısa metinde (<20 sözcük) tahmin yok.

export interface LanguageGuess {
  code: string
  name: string
  /** 0-1 arası güven */
  confidence: number
}

const NAMES: Record<string, string> = {
  tr: 'Türkçe', en: 'İngilizce', de: 'Almanca', fr: 'Fransızca', es: 'İspanyolca', it: 'İtalyanca', pt: 'Portekizce',
  nl: 'Felemenkçe', ru: 'Rusça', uk: 'Ukraynaca', el: 'Yunanca', ar: 'Arapça', fa: 'Farsça', he: 'İbranice',
  zh: 'Çince', ja: 'Japonca', ko: 'Korece', hi: 'Hintçe', th: 'Tayca',
}

const STOPWORDS: Record<string, readonly string[]> = {
  tr: ['ve', 'bir', 'bu', 'da', 'de', 'için', 'ile', 'olarak', 'çok', 'daha', 'ne', 'gibi', 'ama', 'olan', 'kadar', 'sonra', 'her', 'değil', 'mi', 'veya', 'şey', 'ben', 'sen', 'o'],
  en: ['the', 'and', 'of', 'to', 'in', 'is', 'that', 'it', 'for', 'with', 'as', 'was', 'on', 'are', 'be', 'this', 'by', 'not', 'or', 'have', 'from', 'at', 'which', 'you'],
  de: ['der', 'die', 'und', 'das', 'ist', 'nicht', 'mit', 'den', 'ein', 'eine', 'zu', 'von', 'sich', 'auf', 'für', 'auch', 'dem', 'des', 'im', 'es', 'wird', 'sind', 'oder', 'aber'],
  fr: ['le', 'la', 'les', 'et', 'des', 'est', 'un', 'une', 'du', 'que', 'dans', 'pour', 'pas', 'qui', 'sur', 'au', 'avec', 'ce', 'il', 'sont', 'par', 'plus', 'nous', 'mais'],
  es: ['el', 'la', 'los', 'las', 'y', 'que', 'en', 'un', 'una', 'es', 'por', 'con', 'para', 'del', 'se', 'no', 'lo', 'como', 'más', 'pero', 'sus', 'al', 'está', 'son'],
  it: ['il', 'la', 'di', 'che', 'e', 'un', 'una', 'per', 'non', 'sono', 'del', 'della', 'gli', 'le', 'con', 'si', 'come', 'anche', 'nel', 'alla', 'questo', 'ma', 'più', 'è'],
  pt: ['o', 'a', 'os', 'as', 'e', 'que', 'do', 'da', 'em', 'um', 'uma', 'para', 'com', 'não', 'por', 'mais', 'dos', 'das', 'se', 'como', 'mas', 'ao', 'é', 'são'],
  nl: ['de', 'het', 'een', 'en', 'van', 'is', 'dat', 'op', 'te', 'in', 'niet', 'zijn', 'met', 'voor', 'ook', 'aan', 'er', 'maar', 'om', 'dit', 'wordt', 'door', 'als', 'bij'],
}

// Bir dile özgü harfler; varlıkları küçük bir artı puan verir.
const LETTERS: Record<string, RegExp> = {
  tr: /[ğışİĞŞı]/g,
  de: /[ßäöü]/g,
  fr: /[œàèùêâîôûç]/g,
  es: /[ñ¿¡]/g,
  pt: /[ãõ]/g,
}

const SCRIPTS: Array<[string, RegExp]> = [
  ['cyrillic', /[Ѐ-ӿ]/g],
  ['greek', /[Ͱ-Ͽ]/g],
  ['arabic', /[؀-ۿ]/g],
  ['hebrew', /[֐-׿]/g],
  ['han', /[一-鿿]/g],
  ['kana', /[぀-ヿ]/g],
  ['hangul', /[가-힯]/g],
  ['devanagari', /[ऀ-ॿ]/g],
  ['thai', /[฀-๿]/g],
]

function count(text: string, re: RegExp): number {
  return text.match(re)?.length ?? 0
}

function guess(code: string, confidence: number): LanguageGuess {
  return { code, name: NAMES[code] ?? code, confidence: Math.max(0, Math.min(1, Number(confidence.toFixed(2)))) }
}

/** Metnin dilini tahmin eder; yeterli kanıt yoksa null. */
export function detectLanguage(input: string): LanguageGuess | null {
  const text = input.slice(0, 20_000)
  const latin = count(text, /[a-zA-ZÀ-ɏ]/g)
  const scripts = SCRIPTS.map(([name, re]) => [name, count(text, re)] as const).sort((a, b) => b[1] - a[1])
  const [topScript, topCount] = scripts[0] ?? ['', 0]
  const letters = latin + scripts.reduce((s, [, c]) => s + c, 0)
  if (letters < 40) return null
  if (topCount > latin) {
    const share = topCount / letters
    switch (topScript) {
      case 'cyrillic':
        return guess(count(text, /[іїєґ]/gi) > 2 ? 'uk' : 'ru', share)
      case 'arabic':
        return guess(count(text, /[پچژگ]/g) > 2 ? 'fa' : 'ar', share)
      case 'han':
        return guess(count(text, /[぀-ヿ]/g) > 0 ? 'ja' : 'zh', share)
      case 'kana':
        return guess('ja', share)
      case 'greek':
        return guess('el', share)
      case 'hebrew':
        return guess('he', share)
      case 'hangul':
        return guess('ko', share)
      case 'devanagari':
        return guess('hi', share)
      case 'thai':
        return guess('th', share)
    }
  }
  // Türkçe küçük harfe çevirme "I" -> "ı" yapar; diğer diller genel küçük harfle sayılır.
  const wordsTr = text.toLocaleLowerCase('tr').match(/\p{L}+/gu) ?? []
  const words = text.toLowerCase().match(/\p{L}+/gu) ?? []
  if (words.length < 20) return null
  const scores = Object.entries(STOPWORDS).map(([code, list]) => {
    const set = new Set(list)
    const hits = (code === 'tr' ? wordsTr : words).reduce((s, w) => s + (set.has(w) ? 1 : 0), 0)
    const letterRe = LETTERS[code]
    const bonus = letterRe ? Math.min(count(text, letterRe) / 4, words.length * 0.1) : 0
    return [code, hits + bonus] as const
  })
  scores.sort((a, b) => b[1] - a[1])
  const [best, second] = scores
  if (!best || best[1] < 3) return null
  return guess(best[0], best[1] / (best[1] + (second?.[1] ?? 0)))
}
