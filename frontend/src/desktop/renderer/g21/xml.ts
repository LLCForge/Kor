// [g21] Doğrusal, bağımlılıksız XML tarayıcı (SAX benzeri). XLSX ve EPUB paket dosyalarını (workbook, sheet,
//      OPF, NCX) DOM'suz ve test edilebilir biçimde okumak için. DOCTYPE/varlık tanımları YOK sayılır (XXE ve
//      "billion laughs" mümkün değil); yalnızca yerleşik ve sayısal karakter varlıkları çözülür. Her adım ileri
//      gider (geri izleme yok), bu yüzden kötü niyetli girdide süre doğrusal kalır.

export interface XmlHandlers {
  open?: (name: string, attrs: Record<string, string>, selfClosing: boolean) => void
  close?: (name: string) => void
  text?: (text: string) => void
}

const NAMED: Record<string, string> = { lt: '<', gt: '>', amp: '&', quot: '"', apos: "'" }

/** Yerleşik (&lt; ...) ve sayısal (&#38; &#x26;) varlıkları çözer; bilinmeyenleri olduğu gibi bırakır. */
export function decodeEntities(text: string): string {
  if (!text.includes('&')) return text
  return text.replace(/&(#x[0-9a-fA-F]{1,6}|#\d{1,7}|[a-zA-Z]{2,6});/g, (whole, body: string) => {
    if (body[0] === '#') {
      const code = body[1] === 'x' || body[1] === 'X' ? parseInt(body.slice(2), 16) : parseInt(body.slice(1), 10)
      return code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff) ? String.fromCodePoint(code) : ''
    }
    return NAMED[body] ?? whole
  })
}

/** Ad alanı önekini atar: "x:row" -> "row" (bazı üreticiler önekli etiket yazar). */
export function localName(name: string): string {
  const i = name.indexOf(':')
  return i < 0 ? name : name.slice(i + 1)
}

function parseAttrs(src: string): Record<string, string> {
  const attrs: Record<string, string> = Object.create(null) as Record<string, string>
  let i = 0
  while (i < src.length) {
    while (i < src.length && /\s/.test(src[i] ?? '')) i++
    const nameStart = i
    while (i < src.length && !/[\s=]/.test(src[i] ?? '')) i++
    const name = src.slice(nameStart, i)
    while (i < src.length && /\s/.test(src[i] ?? '')) i++
    if (src[i] !== '=') {
      if (name) attrs[name] = ''
      continue
    }
    i++
    while (i < src.length && /\s/.test(src[i] ?? '')) i++
    const quote = src[i]
    let value: string
    if (quote === '"' || quote === "'") {
      const end = src.indexOf(quote, i + 1)
      value = src.slice(i + 1, end < 0 ? src.length : end)
      i = end < 0 ? src.length : end + 1
    } else {
      const start = i
      while (i < src.length && !/\s/.test(src[i] ?? '')) i++
      value = src.slice(start, i)
    }
    if (name) attrs[name] = decodeEntities(value)
  }
  return attrs
}

/** XML metnini baştan sona bir kez tarar ve olayları sırayla bildirir. Etiket adları olduğu gibi (önekli) verilir. */
export function scanXml(xml: string, handlers: XmlHandlers): void {
  let i = 0
  const n = xml.length
  while (i < n) {
    const lt = xml.indexOf('<', i)
    if (lt < 0) {
      if (handlers.text && i < n) handlers.text(decodeEntities(xml.slice(i)))
      return
    }
    if (lt > i && handlers.text) handlers.text(decodeEntities(xml.slice(i, lt)))
    if (xml.startsWith('<!--', lt)) {
      const end = xml.indexOf('-->', lt + 4)
      i = end < 0 ? n : end + 3
    } else if (xml.startsWith('<![CDATA[', lt)) {
      const end = xml.indexOf(']]>', lt + 9)
      if (handlers.text) handlers.text(xml.slice(lt + 9, end < 0 ? n : end))
      i = end < 0 ? n : end + 3
    } else if (xml[lt + 1] === '?' || xml[lt + 1] === '!') {
      // İşleme talimatı ve DOCTYPE: içerik yorumlanmaz (iç alt küme dahil "]>" sonuna kadar atlanır).
      const bracket = xml.indexOf('[', lt)
      const close = xml.indexOf('>', lt)
      if (xml[lt + 1] === '!' && bracket >= 0 && close >= 0 && bracket < close) {
        const end = xml.indexOf(']>', bracket)
        i = end < 0 ? n : end + 2
      } else i = close < 0 ? n : close + 1
    } else {
      // Etiket sonu: tırnak içindeki '>' karakterleri sayılmaz.
      let j = lt + 1
      let quote: string | null = null
      while (j < n) {
        const c = xml[j]
        if (quote) {
          if (c === quote) quote = null
        } else if (c === '"' || c === "'") quote = c
        else if (c === '>') break
        j++
      }
      const body = xml.slice(lt + 1, j)
      i = j + 1
      if (body.startsWith('/')) {
        handlers.close?.(body.slice(1).trim())
        continue
      }
      const selfClosing = body.endsWith('/')
      const inner = selfClosing ? body.slice(0, -1) : body
      const nameEnd = inner.search(/[\s]/)
      const name = nameEnd < 0 ? inner : inner.slice(0, nameEnd)
      if (!name) continue
      handlers.open?.(name, nameEnd < 0 ? (Object.create(null) as Record<string, string>) : parseAttrs(inner.slice(nameEnd)), selfClosing)
      if (selfClosing) handlers.close?.(name)
    }
  }
}

/** Belirtilen yerel adlı tüm öğelerin özniteliklerini sırayla döndürür (ör. OPF <item>, workbook <sheet>). */
export function collectElements(xml: string, name: string): Array<Record<string, string>> {
  const out: Array<Record<string, string>> = []
  scanXml(xml, {
    open: (tag, attrs) => {
      if (localName(tag) === name) out.push(attrs)
    },
  })
  return out
}

/** Belirtilen yerel adlı ilk öğenin metin içeriğini döndürür (iç içe metinler birleştirilir). */
export function firstElementText(xml: string, name: string): string | null {
  let depth = 0
  let text = ''
  let done = false
  scanXml(xml, {
    open: (tag, _attrs, selfClosing) => {
      if (done) return
      if (localName(tag) === name && depth === 0 && !selfClosing) depth = 1
      else if (depth > 0) depth++
    },
    close: () => {
      if (done || depth === 0) return
      depth--
      if (depth === 0) done = true
    },
    text: (t) => {
      if (depth > 0 && !done) text += t
    },
  })
  return done || text ? text.trim() : null
}
