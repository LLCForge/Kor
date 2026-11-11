// [g3] Diyagram yardımcıları (saf, DOM'suz; test edilebilir). Blok bileşenleri, slash komutu ve Markdown
//      işleyicisi bunları kullanır. Burada ağır kütüphane yoktur; yalnızca metin çözümleme ve dönüşüm.

/** SandboxedPreview/mermaid dışı diyagram dilleri + mermaid: sekmeli kartta (29) birlikte ele alınır. */
const DIAGRAM_LANGUAGES = new Set(['mermaid', 'dot', 'graphviz', 'plantuml', 'puml', 'uml', 'wavedrom', 'nomnoml'])

/** Bir kod bloğu dili diyagram dili mi (çoklu diyagram sekmesi gruplaması için). */
export function isDiagramLanguage(language: string | null | undefined): boolean {
  return !!language && DIAGRAM_LANGUAGES.has(language.trim().toLowerCase())
}

/** Mermaid kaynağının ilk anlamlı satırından diyagram türünü okur (yorum ve %%{init}%% atlanır). */
export function detectMermaidKind(code: string): string | null {
  for (const raw of code.split('\n')) {
    const line = raw.trim()
    if (!line || line.startsWith('%%')) continue
    const token = /^[A-Za-z0-9-]+/.exec(line)?.[0] ?? ''
    return token || null
  }
  return null
}

const BOX_DRAWING = /[─-╿▀-▟]/ // kutu çizimi + blok elemanları

/**
 * feature 20: Bir kod bloğu kutu-çizgi ASCII şeması mı? Unicode kutu çizimi karakterleri veya klasik
 * ASCII (+---+ / | ... | ) birden çok hizalı satırda görünüyorsa true. Normal kodda yanlış tetiklenmesin
 * diye muhafazakârdır: en az iki satır ve belirgin yapısal yoğunluk gerekir.
 */
export function looksLikeAsciiDiagram(code: string): boolean {
  const lines = code.split('\n').filter((l) => l.trim().length > 0)
  if (lines.length < 2) return false
  let boxChars = 0
  for (const ch of code) if (BOX_DRAWING.test(ch)) boxChars++
  if (boxChars >= 4) return true
  // Klasik ASCII: en az iki satırda "+---" köşe-kenar ve en az bir "|...|" dikey kenar.
  const corners = lines.filter((l) => /\+[-=]{2,}/.test(l)).length
  const verticals = lines.filter((l) => /\|.*\|/.test(l) || /^\s*\|/.test(l)).length
  return corners >= 1 && verticals >= 1 && corners + verticals >= 3
}

export interface DiagramSuggestion {
  type: string
  label: string
  template: string
}

const SUGGEST_RULES: ReadonlyArray<{ type: string; label: string; re: RegExp; template: string }> = [
  {
    type: 'sequenceDiagram',
    label: 'Sıra diyagramı',
    re: /\b(sıra|sequence|mesaj|istek|yan[ıi]t|aktör|actor|api\s*çağr|request|response|protokol)\b/i,
    template: 'sequenceDiagram\n  participant A as İstemci\n  participant B as Sunucu\n  A->>B: İstek\n  B-->>A: Yanıt',
  },
  {
    type: 'stateDiagram-v2',
    label: 'Durum makinesi',
    re: /\b(durum|state|geçiş|transition|makine|statemachine)\b/i,
    template: 'stateDiagram-v2\n  [*] --> Beklemede\n  Beklemede --> Çalışıyor: başlat\n  Çalışıyor --> [*]: bitir',
  },
  {
    type: 'classDiagram',
    label: 'Sınıf diyagramı',
    re: /\b(sınıf|class|kalıtım|inherit|nesne|object|arayüz|interface)\b/i,
    template: 'classDiagram\n  class Taşıt {\n    +String plaka\n    +hareketEt()\n  }\n  Taşıt <|-- Otomobil',
  },
  {
    type: 'erDiagram',
    label: 'ER diyagramı',
    re: /\b(varlık|entity|tablo|table|veritaban|database|ilişki|relation|şema|schema)\b/i,
    template: 'erDiagram\n  MÜŞTERİ ||--o{ SİPARİŞ : verir\n  SİPARİŞ ||--|{ KALEM : içerir',
  },
  {
    type: 'gantt',
    label: 'Gantt',
    re: /\b(gantt|zaman\s*çizelge|takvim|proje\s*plan|schedule|milestone|kilometre)\b/i,
    template: 'gantt\n  title Proje Planı\n  dateFormat YYYY-MM-DD\n  Tasarım :a1, 2024-01-01, 7d\n  Geliştirme :after a1, 14d',
  },
  {
    type: 'journey',
    label: 'Kullanıcı yolculuğu',
    re: /\b(yolculuk|journey|deneyim|experience|adım.*duygu)\b/i,
    template: 'journey\n  title Kullanıcı Yolculuğu\n  section Keşif\n    Siteyi bul: 4: Kullanıcı\n    Kaydol: 3: Kullanıcı',
  },
  {
    type: 'pie',
    label: 'Pasta grafiği',
    re: /\b(pasta|pie|yüzde|percent|oran|pay|dağılım)\b/i,
    template: 'pie title Dağılım\n  "A" : 45\n  "B" : 30\n  "C" : 25',
  },
  {
    type: 'mindmap',
    label: 'Zihin haritası',
    re: /\b(zihin|mindmap|fikir|beyin\s*fırtına|brainstorm)\b/i,
    template: 'mindmap\n  root((Fikir))\n    Dal 1\n    Dal 2\n      Alt dal',
  },
  {
    type: 'gitGraph',
    label: 'Git grafiği',
    re: /\b(git|dal|branch|commit|merge|birleşt)\b/i,
    template: 'gitGraph\n  commit\n  branch geliştirme\n  commit\n  checkout main\n  merge geliştirme',
  },
]

/**
 * feature 22: Serbest metinden uygun Mermaid diyagram türünü ve bir başlangıç şablonu önerir.
 * Hiçbir ipucu eşleşmezse akış diyagramına döner.
 */
export function suggestDiagramType(text: string): DiagramSuggestion {
  for (const rule of SUGGEST_RULES) {
    if (rule.re.test(text)) return { type: rule.type, label: rule.label, template: rule.template }
  }
  return {
    type: 'flowchart',
    label: 'Akış diyagramı',
    template: 'flowchart TD\n  A[Başla] --> B{Karar}\n  B -->|Evet| C[İşlem]\n  B -->|Hayır| D[Bitir]',
  }
}

/** Mermaid düğüm etiketi için güvenli metin: tırnak ve köşeli parantezler kaçırılır, kısaltılır. */
function nodeLabel(text: string): string {
  const clean = text
    .replace(/\s+/g, ' ')
    .trim()
    .replace(/["[\]{}|]/g, (c) => (c === '"' ? '#quot;' : c === '|' ? '/' : ' '))
    .slice(0, 80)
  return clean || '∅'
}

const MAX_FLOW_NODES = 200

/** JSON değerini akış diyagramı düğüm ağacına çevirir (özyinelemeli; düğüm sayısı sınırlı). */
function jsonToFlowLines(value: unknown): string[] {
  const edges: string[] = []
  let id = 0
  const nextId = () => `n${id++}`
  const visit = (val: unknown, keyLabel: string | null): string => {
    const self = nextId()
    if (id > MAX_FLOW_NODES) return self
    if (Array.isArray(val)) {
      edges.push(`  ${self}["${nodeLabel(keyLabel ?? 'liste')}"]`)
      for (const item of val) {
        if (id > MAX_FLOW_NODES) break
        edges.push(`  ${self} --> ${visit(item, null)}`)
      }
    } else if (val && typeof val === 'object') {
      edges.push(`  ${self}["${nodeLabel(keyLabel ?? 'nesne')}"]`)
      for (const [k, v] of Object.entries(val as Record<string, unknown>)) {
        if (id > MAX_FLOW_NODES) break
        if (v && typeof v === 'object') edges.push(`  ${self} --> ${visit(v, k)}`)
        else edges.push(`  ${self} --> ${nextId()}["${nodeLabel(`${k}: ${String(v)}`)}"]`)
      }
    } else {
      edges.push(`  ${self}["${nodeLabel(keyLabel ? `${keyLabel}: ${String(val)}` : String(val))}"]`)
    }
    return self
  }
  visit(value, null)
  return edges
}

/** Girintiye göre madde listesini akış diyagramı kenarlarına çevirir. */
function listToFlowLines(text: string): string[] {
  const lines = text.split('\n').filter((l) => l.trim().length > 0)
  const out: string[] = []
  const stack: Array<{ indent: number; id: string }> = []
  let id = 0
  let lastTop: string | null = null
  for (const raw of lines) {
    if (id > MAX_FLOW_NODES) break
    const indent = raw.length - raw.trimStart().length
    const label = raw.trim().replace(/^([-*+]|\d+[.)])\s+/, '')
    const self = `n${id++}`
    out.push(`  ${self}["${nodeLabel(label)}"]`)
    while (stack.length && stack[stack.length - 1]!.indent >= indent) stack.pop()
    const parent = stack[stack.length - 1]
    if (parent) out.push(`  ${parent.id} --> ${self}`)
    else {
      // Girintisiz art arda maddeler adım zinciri oluşturur (önceki üst düzey düğüme bağlanır).
      if (lastTop) out.push(`  ${lastTop} --> ${self}`)
      lastTop = self
    }
    stack.push({ indent, id: self })
  }
  return out
}

/**
 * feature 21: JSON (nesne/dizi) veya madde listesini `flowchart TD` kaynağına çevirir.
 * JSON ayrıştırılamazsa girintili madde listesi olarak ele alınır; düz satırlar adım zincirine döner.
 */
export function toFlowchart(input: string): string {
  const trimmed = input.trim()
  let body: string[]
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (parsed && typeof parsed === 'object') body = jsonToFlowLines(parsed)
    else body = listToFlowLines(trimmed)
  } catch {
    const looksListed = /^\s*([-*+]|\d+[.)])\s/m.test(trimmed)
    body = looksListed ? listToFlowLines(trimmed) : chainLines(trimmed)
  }
  return `flowchart TD\n${body.join('\n')}`
}

/** Girintisiz düz satırları sıralı bir adım zincirine çevirir. */
function chainLines(text: string): string[] {
  const lines = text.split('\n').map((l) => l.trim()).filter(Boolean).slice(0, MAX_FLOW_NODES)
  const out: string[] = []
  lines.forEach((label, i) => {
    out.push(`  n${i}["${nodeLabel(label)}"]`)
    if (i > 0) out.push(`  n${i - 1} --> n${i}`)
  })
  return out.length ? out : ['  n0["(boş)"]']
}

/** feature 28: Çizici hata metninden satır numarası ve kısa mesaj ayıklar. */
export function parseDiagramError(raw: string): { line: number | null; message: string } {
  const text = (raw || '').trim()
  const lineMatch = /(?:on line|line|satır)\s*[:#]?\s*(\d+)/i.exec(text)
  const line = lineMatch ? Number(lineMatch[1]) : null
  const first = text.split('\n').find((l) => l.trim().length > 0)?.trim() ?? 'Bilinmeyen hata'
  return { line: Number.isFinite(line) ? line : null, message: first.slice(0, 200) }
}

export interface SubgraphInfo {
  title: string
  /** `subgraph` satırının 0 tabanlı indeksi */
  start: number
  /** eşleşen `end` satırının 0 tabanlı indeksi */
  end: number
}

/** feature 23: Mermaid kaynağındaki en üst düzey subgraph bloklarını bulur (iç içe olanlar atlanır). */
export function findSubgraphs(code: string): SubgraphInfo[] {
  const lines = code.split('\n')
  const found: SubgraphInfo[] = []
  let depth = 0
  let current: { title: string; start: number } | null = null
  lines.forEach((raw, i) => {
    const line = raw.trim()
    const open = /^subgraph\s+(.*)$/i.exec(line)
    if (open) {
      if (depth === 0) {
        const title = (open[1] ?? '').replace(/\["?|"?\]$/g, '').trim() || 'Grup'
        current = { title, start: i }
      }
      depth++
      return
    }
    if (/^end\b/i.test(line) && depth > 0) {
      depth--
      if (depth === 0 && current) {
        found.push({ title: current.title, start: current.start, end: i })
        current = null
      }
    }
  })
  return found
}

/**
 * feature 23: Bir subgraph'ı tek düğüme katlar. İç satırlar kaldırılır; blok, başlıklı tek bir düğümle
 * değiştirilir ve içteki düğümlere giden/gelen kenarlar bu düğüme yönlendirilir.
 * shortcut: düğüm kimlikleri sezgisel (tam sözcük) eşleştirilir; karmaşık etiket söz diziminde kenar
 *           yeniden yönlendirmesi eksik kalabilir — gerçek ihtiyaç doğunca ayrıştırıcı güçlendirilir.
 */
export function collapseSubgraph(code: string, target: SubgraphInfo): string {
  const lines = code.split('\n')
  const groupId = `g_${target.start}`
  const label = target.title.replace(/"/g, '#quot;')
  const inner = lines.slice(target.start + 1, target.end)
  const innerIds = collectNodeIds(inner)
  const indent = lines[target.start]?.match(/^\s*/)?.[0] ?? ''
  const replacement = `${indent}${groupId}["${label}"]`
  const kept = [...lines.slice(0, target.start), replacement, ...lines.slice(target.end + 1)]
  // İçteki kimliklere dışarıdan yapılan referansları grup düğümüne yönlendir.
  return kept
    .map((line, idx) => {
      if (idx === target.start) return line // yeni yerleştirilen düğüm
      let out = line
      for (const nodeId of innerIds) {
        out = out.replace(new RegExp(`(^|[^\\w])${escapeRe(nodeId)}(?=$|[^\\w])`, 'g'), (_m, p1) => `${p1}${groupId}`)
      }
      return out
    })
    .join('\n')
}

/** Kenar satırlarından ve düğüm tanımlarından kaba düğüm kimlikleri toplar. */
/**
 * feature 23: Başlığı verilen tüm üst düzey subgraph'ları sırayla katlar. Her turda kaynak yeniden
 * çözümlenir; katlanan blok artık `subgraph` içermediğinden tekrar eşleşmez (satır kayması güvenli).
 */
export function collapseByTitles(code: string, titles: ReadonlySet<string>): string {
  if (titles.size === 0) return code
  let out = code
  for (let guard = 0; guard < 100; guard++) {
    const target = findSubgraphs(out).find((s) => titles.has(s.title))
    if (!target) break
    const next = collapseSubgraph(out, target)
    if (next === out) break
    out = next
  }
  return out
}

function collectNodeIds(lines: string[]): string[] {
  const ids = new Set<string>()
  for (const raw of lines) {
    const line = raw.trim()
    if (!line || /^subgraph\b|^end\b|^direction\b/i.test(line)) continue
    for (const m of line.matchAll(/([A-Za-z][\w-]*)\s*(?:[[({]|-->|---|==>|-\.->|:::)/g)) {
      if (m[1]) ids.add(m[1])
    }
    for (const m of line.matchAll(/(?:-->|---|==>|-\.->)\s*\|?[^|]*\|?\s*([A-Za-z][\w-]*)/g)) {
      if (m[1]) ids.add(m[1])
    }
  }
  return [...ids]
}

function escapeRe(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * feature 17: PlantUML metninin yaygın alt kümesini (sıra ve sınıf) Mermaid'e çevirir. Dış sunucuya
 * hiçbir şey gönderilmez. Desteklenmeyen söz dizimi için not döndürür.
 * shortcut: yalnızca sıra (->, -->, participant/actor) ve sınıf (class, kalıtım) kalıpları çevrilir;
 *           tam PlantUML kapsamı hedeflenmez — gerçekçi yerel PlantUML çizici olmadığından bu köprü yeterlidir.
 */
export function plantumlToMermaid(src: string): { mermaid: string; note?: string } | { error: string } {
  const body = src.replace(/@startuml[^\n]*\n?|\s*@enduml\s*$/gi, '').trim()
  if (!body) return { error: 'Boş PlantUML içeriği.' }
  const lines = body.split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith("'"))

  const isClass = lines.some((l) => /^(abstract\s+)?class\s|^interface\s|^enum\s/i.test(l))
  if (isClass) return classToMermaid(lines)

  const isSequence = lines.some((l) => /^(participant|actor)\b/i.test(l) || /(-+>>?|<-+)/.test(l))
  if (isSequence) return sequenceToMermaid(lines)

  return { error: 'Bu PlantUML türü yerel olarak çevrilemiyor (yalnızca sıra ve sınıf diyagramları desteklenir).' }
}

function sequenceToMermaid(lines: string[]): { mermaid: string; note?: string } {
  const out: string[] = ['sequenceDiagram']
  let skipped = 0
  for (const line of lines) {
    const part = /^(participant|actor)\s+(.+)$/i.exec(line)
    if (part) {
      const name = (part[2] ?? '').replace(/\s+as\s+/i, ' as ').replace(/"/g, '')
      out.push(`  participant ${name}`)
      continue
    }
    // A -> B : mesaj   | A --> B : mesaj
    const msg = /^(\w+)\s*(-+>>?|-->>?)\s*(\w+)\s*:\s*(.+)$/.exec(line)
    if (msg) {
      const arrow = msg[2]!.includes('--') ? '-->>' : '->>'
      out.push(`  ${msg[1]}${arrow}${msg[3]}: ${msg[4]}`)
      continue
    }
    if (/^(note|alt|else|opt|loop|end|group)\b/i.test(line)) {
      out.push(`  ${line}`)
      continue
    }
    if (line) skipped++
  }
  return { mermaid: out.join('\n'), note: skipped ? `${skipped} satır çevrilemedi ve atlandı.` : undefined }
}

function classToMermaid(lines: string[]): { mermaid: string; note?: string } {
  const out: string[] = ['classDiagram']
  let skipped = 0
  for (const line of lines) {
    const cls = /^(?:abstract\s+)?(?:class|interface|enum)\s+(\w+)\s*\{?$/i.exec(line)
    if (cls) {
      out.push(`  class ${cls[1]}`)
      continue
    }
    // Kalıtım: B <|-- A  veya  A --|> B
    const inh = /^(\w+)\s*(<\|--|--\|>|<--|-->|\*--|o--|--)\s*(\w+)/.exec(line)
    if (inh) {
      out.push(`  ${inh[1]} ${inh[2]} ${inh[3]}`)
      continue
    }
    if (line === '}' || line === '{') continue
    if (line) skipped++
  }
  return { mermaid: out.join('\n'), note: skipped ? `${skipped} satır çevrilemedi ve atlandı.` : undefined }
}
