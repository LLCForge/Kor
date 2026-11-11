// [g22] Sohbetin çizdiği zengin blok dilleri ve kısa sözdizimi notları: her model (araç desteği olmasa da)
// diyagram, tablo ve bileşen üretebilsin diye sistem istemine kısa bir bölüm olarak girer.
// Kaynak: src/desktop/renderer/*Block*.tsx içindeki registerRichBlock çağrıları (test/g22-rich-catalog.test.ts eşleşmeyi denetler).
// Yeni blok (chart, map, dot ...) ekleyen grup buraya bir satır ekler. Saf modül: Node API'si yok.

export interface RichBlockEntry {
  /** Aynı bileşene giden diller (takma adlar) */
  langs: readonly string[]
  syntax: string
}

export const RICH_BLOCKS: readonly RichBlockEntry[] = [
  { langs: ['mermaid'], syntax: 'Mermaid source (flowchart, sequenceDiagram, classDiagram, stateDiagram, erDiagram, gantt, pie, mindmap).' },
  { langs: ['svg'], syntax: 'one complete <svg> element; scripts, event handlers, foreignObject and external links are stripped.' },
  { langs: ['html'], syntax: 'HTML body fragment (no <html>/<head>), sandboxed preview; scripts run only if the user enables them.' },
  { langs: ['playground'], syntax: '{"html":"...","css":"...","js":"..."} editable live preview.' },
  { langs: ['table', 'csv'], syntax: 'Markdown pipe table, CSV, or JSON array of objects; sortable, filterable, exportable.' },
  { langs: ['json'], syntax: 'any JSON value, shown as a collapsible tree with an inferred schema.' },
  { langs: ['diff'], syntax: '{"path":"a.ts","before":"old text","after":"new text"}' },
  {
    langs: ['calc'],
    syntax:
      '{"title":"...","formula":"price*qty*(1+tax/100)","unit":"TL","variables":[{"name":"price","label":"...","value":10,"min":0,"max":100,"step":1,"slider":true}]}; formula: numbers, variables, + - * / % ^, (), pi, e, abs sqrt round floor ceil min max pow ln log10 sin cos clamp.',
  },
  {
    langs: ['form'],
    syntax: '{"title":"...","fields":[{"name":"email","label":"...","type":"text|number|email|textarea|select|checkbox","options":["..."],"required":true}]}; makes a local summary, sends nothing.',
  },
  { langs: ['tabs'], syntax: '{"accordion":false,"tabs":[{"label":"...","content":"..."}]}' },
  { langs: ['wizard', 'steps'], syntax: '{"title":"...","steps":[{"title":"...","content":"..."}]}' },
  { langs: ['todo'], syntax: '{"title":"...","items":["..."]} (checkbox state is kept locally)' },
  { langs: ['gallery', 'cards'], syntax: '{"cards":[{"title":"...","text":"...","badge":"...","meta":"..."}]}' },
  { langs: ['timeline'], syntax: '{"events":[{"date":"2024-05","title":"...","text":"..."}]}' },
  { langs: ['compare'], syntax: '{"a":"Option A","b":"Option B","rows":[{"label":"Price","a":"...","b":"..."}]}' },
  { langs: ['palette', 'colors'], syntax: '{"colors":["#0f766e",{"hex":"#f59e0b","name":"Amber"}]}' },
  { langs: ['icons'], syntax: '{"icons":["★",{"char":"→","name":"arrow"}]}' },
  { langs: ['quiz'], syntax: '{"title":"...","questions":[{"q":"...","options":["A","B"],"answer":0}]} (answer = 0-based option index)' },
  { langs: ['progress'], syntax: '{"label":"...","value":40,"max":100,"ring":false} or {"steps":["Plan","Build"],"current":1}' },
  { langs: ['glossary'], syntax: '{"text":"...","terms":[{"term":"...","definition":"..."}]}' },
  // [g3] Mermaid dışı diyagramlar (yerel çizim, ağ yok)
  { langs: ['dot', 'graphviz'], syntax: 'Graphviz DOT source, e.g. digraph G { a -> b }.' },
  { langs: ['plantuml', 'puml'], syntax: 'PlantUML @startuml...@enduml; only sequence (participant/actor, ->, -->) and class (class, <|--) diagrams are converted locally.' },
  { langs: ['wavedrom'], syntax: 'WaveJSON timing diagram, e.g. {"signal":[{"name":"clk","wave":"p...."},{"name":"dat","wave":"x.34.x","data":["a","b"]}]}' },
  { langs: ['nomnoml'], syntax: 'nomnoml UML text, e.g. [User|name;email]->[Order].' },
  { langs: ['ascii'], syntax: 'box-drawing ASCII sketch shown monospace and aligned (use only when no diagram language fits).' },
  // [g18] grafikler
  {
    langs: ['chart'],
    syntax:
      '{"type":"line|bar|stacked|area|scatter|pie|donut|heatmap|box|radar|waterfall|funnel|histogram|dual|kpi","title":"...","labels":["Q1","Q2"],"series":[{"name":"Gelir","values":[10,20]}],"referenceLines":[{"value":15,"label":"Hedef"}]}; CSV also works (optional first lines "type: bar" and "başlık: ...").',
  },
  { langs: ['sparkline', 'spark'], syntax: 'a short list of numbers, e.g. 3 5 2 8 6, drawn as a small trend line.' },
  // [g19] haritalar (paketli dünya altlığı, dış kiremit yok)
  {
    langs: ['map', 'geojson'],
    syntax:
      '{"title":"...","unit":"...","regions":{"Turkey":12,"DE":7},"markers":[{"lat":41.01,"lon":28.97,"label":"İstanbul"}],"routes":[[[28.97,41.01],[32.85,39.93]]]} (regions by country name or ISO alpha-2/numeric code; route points are [lon,lat]); or a GeoJSON object.',
  },
  // [g20] not defteri ve çalıştırılabilir bloklar (çalıştırmayı kullanıcı başlatır)
  { langs: ['notebook', 'ipynb'], syntax: '.ipynb JSON or a workspace path to an .ipynb file.' },
  { langs: ['repl', 'repl-js', 'repl-javascript', 'repl-ts', 'repl-typescript'], syntax: 'JS/TS the user can run in an isolated worker (no network, 5 s).' },
  { langs: ['repl-py', 'repl-python'], syntax: 'Python the user runs locally after approval.' },
  { langs: ['repl-sql'], syntax: 'read-only SQLite query; first line "-- db: path/file.db".' },
  // [g11] yalnızca etkinse ve turun son yanıtında: güven notu
  { langs: ['confidence'], syntax: 'reserved for the end-of-turn confidence note when it is requested elsewhere in this prompt; do not use otherwise.' },
]

export const RICH_BLOCK_LANGS: readonly string[] = RICH_BLOCKS.flatMap((b) => b.langs)

/** Sistem istemi bölümü; boyutu test/g22-rich-catalog.test.ts içinde ölçülür ve sınırlanır. */
export function richBlockPromptSection(): string {
  return [
    '# SOHBET İÇİ GÖRSEL BLOKLAR',
    'The chat renders fenced code blocks with these languages as interactive visuals. Use one when a diagram, table or widget explains better than prose; otherwise write normal Markdown. ' +
      'Everything is offline (no network, no external images); JSON specs must be valid JSON and text fields are plain text.',
    ...RICH_BLOCKS.map((b) => `- ${b.langs.join(' | ')}: ${b.syntax}`),
  ].join('\n')
}
