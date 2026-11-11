// LoliCode lint/analiz motoru — saf JS (Node veya tarayıcı), bağımlılıksız.
// Bir LoliCode betiğini ayrıştırır: bloklar, değişken tanım/kullanımları, akış sözdizimi,
// ayar adları, enum değerleri ve yaygın tuzaklar. Hata/uyarı üretir; düzeltme önerisi verir.
//
// Kullanım (CLI): node lint.mjs <betik.loli|.txt> [--json] [--ob1]
// Kullanım (modül): import { lintScript, parseScript } from './lint.mjs'
//
// Kapsam: OB2 LoliCode (RuriLib) + OB1 LoliScript (--ob1). OB1 ayrı sözdizimidir; burada
// yalnızca blok/komut tanıma ve uyarı düzeyinde desteklenir.

import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
// data/ hem script yanında hem bir üst klasörde aranır (tools/lint.mjs ve doğrudan çalıştırma).
const dataDir = (() => {
  for (const c of [path.join(HERE, 'data'), path.join(HERE, '..', 'data'), path.join(HERE, '..', '..', 'data')]) {
    try {
      readFileSync(path.join(c, 'blocks.json'))
      return c
    } catch {
      /* sıradaki */
    }
  }
  throw new Error('blocks.json bulunamadı: lint.mjs ile data/ aynı ağaçta olmalı')
})()
const BLOCKS = JSON.parse(readFileSync(path.join(dataDir, 'blocks.json'), 'utf8'))
const ENUMS = JSON.parse(readFileSync(path.join(dataDir, 'enums.json'), 'utf8'))
const META = JSON.parse(readFileSync(path.join(dataDir, 'meta.json'), 'utf8'))

const BY_ID = new Map()
for (const b of BLOCKS) {
  BY_ID.set(b.id, b)
  for (const a of b.aliases ?? []) BY_ID.set(a, b)
}

export const CATEGORIES = [...new Set(BLOCKS.map((b) => b.cat))].sort()
export { META }

// ---------------------------------------------------------------------------
// Sözdizimi bilgisi (kaynak: RuriLib LoliCodeParser / LineParser / doküman)
// ---------------------------------------------------------------------------

/** Akış/komut anahtar sözcükleri (blok değil, ifade). */
export const STATEMENTS = [
  'IF', 'ELSE', 'ELSE IF', 'ENDIF', 'END IF',
  'WHILE', 'ENDWHILE', 'END WHILE',
  'REPEAT', 'END',
  'FOREACH', 'END',
  'TRY', 'CATCH', 'FINALLY', 'END',
  'LOCK', 'ACQUIRELOCK', 'RELEASELOCK',
  'LOG', 'CLOG', 'JUMP', 'SET', 'MARK', 'UNMARK',
  'TAKE', 'TAKEONE', 'BREAK', 'CONTINUE', 'FUNCTION', 'ENDFUNCTION',
  'BLOCK', 'ENDBLOCK',
]

/** Koşul anahtar sözcükleri (IF/WHILE içinde). */
export const KEYWORDS = ['STRINGKEY', 'INTKEY', 'FLOATKEY', 'BOOLKEY', 'LISTKEY', 'DICTKEY']

/** Karşılaştırma operatörleri — tipe göre geçerli kümeler (RuriLib enum'larından). */
export const COMPARISONS = {
  STRINGKEY: ['EqualTo', 'NotEqualTo', 'Contains', 'DoesNotContain', 'Exists', 'DoesNotExist', 'MatchesRegex', 'DoesNotMatchRegex'],
  INTKEY: ['EqualTo', 'NotEqualTo', 'LessThan', 'LessThanOrEqualTo', 'GreaterThan', 'GreaterThanOrEqualTo'],
  FLOATKEY: ['EqualTo', 'NotEqualTo', 'LessThan', 'LessThanOrEqualTo', 'GreaterThan', 'GreaterThanOrEqualTo'],
  BOOLKEY: ['Is', 'IsNot'],
  LISTKEY: ['Contains', 'DoesNotContain', 'Exists', 'DoesNotExist'],
  DICTKEY: ['HasKey', 'DoesNotHaveKey', 'HasValue', 'DoesNotHaveValue', 'Exists', 'DoesNotExist'],
}

/** Tüm blok ayarlarının birleşik ad kümesi (tanınmayan ayar adını yakalamak için). */
const ALL_SETTINGS = new Set()
for (const b of BLOCKS) for (const p of b.params ?? []) ALL_SETTINGS.add(p.n)

/** Addan bağımsız, her ayar tipinde geçerli olan ortak ayarlar. */
const COMMON_LINE_TOKENS = new Set(['DISABLED', 'SAFE', 'LABEL:', 'MODE:', 'RECURSIVE', '=>'])

/** Değişken adı kuralları (RuriLib VariableNames). */
const VAR_RE = /^[A-Za-z_][A-Za-z0-9_.]*$/

// ---------------------------------------------------------------------------
// Ayrıştırma
// ---------------------------------------------------------------------------

const indentOf = (line) => line.length - line.trimStart().length

/** Bir satırın ayar ataması olup olmadığı (RuriLib IsSetting): `ad = deger`. */
const SETTING_RE = /^([A-Za-z][A-Za-z0-9_]*)\s*=\s*\S/

/**
 * Betiği bloklara ve ifadelere ayırır. Blok içi satırlar türüne göre ayrılır:
 * meta (DISABLED/SAFE/LABEL/TYPE/MODE/RECURSIVE), setting (ad = deger), output (=> VAR/CAP @x),
 * key (KEYCHAIN/…KEY) ve body (tip satırından sonraki ham içerik satırları).
 * @returns {{kind:'block'|'statement'|'raw', line:number, text:string, id?:string, label?:string, disabled?:boolean, safe?:boolean, settings?:Array, output?:object|null, keys?:string[], body?:string[], meta?:object}[]}
 */
export function parseScript(text) {
  const lines = text.replace(/\r\n?/g, '\n').split('\n')
  const nodes = []
  let i = 0
  while (i < lines.length) {
    const line = lines[i]
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('//') || trimmed.startsWith('#')) {
      i++
      continue
    }
    if (trimmed.startsWith('BLOCK:')) {
      const header = trimmed.slice('BLOCK:'.length).trim()
      const id = header.split(/\s+/)[0]
      const node = { kind: 'block', line: i + 1, text: line, id, label: null, disabled: false, safe: false, settings: [], output: null, keys: [], body: [], meta: {} }
      i++
      while (i < lines.length && !lines[i].trim().startsWith('ENDBLOCK')) {
        const raw = lines[i]
        const t = raw.trim()
        if (t === 'DISABLED') node.disabled = true
        else if (t === 'SAFE') node.safe = true
        else if (t.startsWith('LABEL:')) node.label = t.slice('LABEL:'.length)
        else if (t === 'RECURSIVE') node.meta.recursive = true
        else if (t.startsWith('TYPE:')) node.meta.type = t.slice(5).trim()
        else if (t.startsWith('MODE:')) node.meta.mode = t.slice(5).trim()
        else if (t.startsWith('=>')) {
          const m = /^=>\s+(VAR|CAP)\s+@(\S+)\s*$/.exec(t)
          if (m) node.output = { line: i + 1, capture: m[1] === 'CAP', name: m[2] }
          else node.output = { line: i + 1, capture: false, name: t.replace(/^=>\s*/, ''), malformed: true, raw: t }
        } else if (t.startsWith('KEYCHAIN') || /^[A-Z]+KEY\s/.test(t)) node.keys.push({ line: i + 1, text: t })
        else {
          const sm = SETTING_RE.exec(t)
          if (sm) node.settings.push({ raw, line: i + 1, name: sm[1], value: t.slice(t.indexOf('=') + 1).trim() })
          else node.body.push({ line: i + 1, text: t })
        }
        i++
      }
      if (i < lines.length) i++ // ENDBLOCK
      nodes.push(node)
      continue
    }
    // --- Ham LoliCode/C# bloğu mu? (Loli2StackTranspiler.cs:85-123) ---
    // BLOCK:'la başlamayan satır grubu OB2'de LoliCodeBlockInstance olur: doğrudan C# olarak transpile edilir.
    // İçindeki `string x = ...` / `x = ...` C# atamaları değişken tanımı sayılır; `<x>` interpolasyonları da değil.
    if (/^(?:string|var|int|long|double|float|bool|dynamic)\s+[A-Za-z_]\w*\s*=/.test(trimmed) || /^(?:if|while|for|\}|)\s*$|^[A-Za-z_][\w.]*\s*=\s*[^=]/.test(trimmed)) {
      const raw = { kind: 'raw', line: i + 1, text: line, lines: [] }
      while (i < lines.length && !lines[i].trim().startsWith('BLOCK:')) {
        raw.lines.push(lines[i])
        i++
      }
      nodes.push(raw)
      continue
    }
    nodes.push({ kind: 'statement', line: i + 1, text: line, trimmed })
    i++
  }
  return nodes
}

/** Ham C# bloğundan değişken tanımlarını ve kullanımlarını çıkarır (linter için). */
function scanRawScript(lines) {
  const defined = new Set()
  const used = new Set()
  for (const line of lines) {
    for (const m of line.matchAll(/(?:string|var|int|long|double|float|bool|byte\[\]|dynamic)\s+([A-Za-z_]\w*)\s*=/g)) defined.add(m[1])
    // data.X = ... (UseProxy vb.)
    for (const m of line.matchAll(/\bdata\.([A-Za-z_]\w*)\s*=/g)) defined.add('data.' + m[1])
    // C# tarafında tanımlanmış değişkeni OB2 interpolasyonuyla kullananlar: $"http://{userpass}@{ipport}"
    for (const m of line.matchAll(/\{([A-Za-z_][\w.?]*)\}/g)) used.add(m[1].replace(/\?.*$/, '').replace(/\..*$/, ''))
  }
  return { defined, used }
}

/** `<var>` biçimindeki interpolasyonları çıkarır. */
export function interpolations(text) {
  const found = []
  const re = /<([^<>\s]+)>/g
  let m
  while ((m = re.exec(text))) {
    const inner = m[1]
    if (/^(LIST|DICT)\(/.test(inner)) {
      found.push({ raw: m[0], index: m.index, kind: 'collection', name: inner })
      continue
    }
    if (inner.startsWith('LIST[')) {
      found.push({ raw: m[0], index: m.index, kind: 'listItem', name: inner })
      continue
    }
    if (inner.includes('.')) {
      found.push({ raw: m[0], index: m.index, kind: 'member', name: inner })
      continue
    }
    found.push({ raw: m[0], index: m.index, kind: 'var', name: inner })
  }
  return found
}

const VALUE_KIND = (v) => {
  if (v.startsWith('@')) return 'variable'
  if (v.startsWith('$')) return 'interpolated'
  if (v.startsWith('"')) return 'literal'
  if (v.startsWith('[')) return 'list'
  if (v.startsWith('{')) return 'dict'
  if (/^-?\d+$/.test(v)) return 'int'
  if (/^-?\d+\.\d+$/.test(v)) return 'float'
  if (/^(true|false)$/i.test(v)) return 'bool' // LineParser.ParseBool bool.TryParse: büyük harfe duyarsız
  return 'enumOrBare'
}

// ---------------------------------------------------------------------------
// Lint kuralları
// ---------------------------------------------------------------------------

const OB1_BLOCKS = new Set([
  'REQUEST', 'PARSE', 'FUNCTION', 'KEYCHECK', 'UTILITY', 'CAPTCHA', 'RECAPTCHA', 'SOLVECAPTCHA',
  'REPORTCAPTCHA', 'BYPASSCF', 'TCP', 'NAVIGATE', 'ELEMENTACTION', 'BROWSERACTION', 'EXECUTEJS',
  // OB1 komut/ifadeleri (yanlışlıkla "bilinmeyen" sayılmasın)
  'IF', 'ELSE', 'ENDIF', 'WHILE', 'ENDWHILE', 'JUMP', 'SET', 'DELETE', 'PRINT', 'DELAY', 'SLEEP',
  'MARK', 'UNMARK', 'BEGIN', 'END', 'CLOG', 'LOG', 'CFSolverRe', 'RECAPTCHAV2', 'RECAPTCHAV3',
])

/**
 * Betiği inceler.
 * @param {string} text
 * @param {{ob1?:boolean}} [opts]
 * @returns {{diagnostics:Array<{severity:string,code:string,line:number,message:string,hint?:string}>, summary:object}}
 */
export function lintScript(text, opts = {}) {
  const diags = []
  const add = (severity, code, line, message, hint) => diags.push({ severity, code, line, message, ...(hint ? { hint } : {}) })
  const nodes = parseScript(text)
  const blocks = nodes.filter((n) => n.kind === 'block')

  const defined = new Set(['data.SOURCE', 'data.COOKIES', 'data.HEADERS', 'data.STATUS', 'data.ADDRESS', 'data.ERROR', 'data.UseProxy', 'data.RESPONSECODE', 'data.HEADERS'])
  const used = new Set()
  const inputRefs = new Set()
  const labels = new Set()
  const jumps = []
  const functions = new Set()
  const stack = []
  const rawBlocks = []

  for (const node of nodes) {
    if (node.kind === 'raw') {
      const { defined: d, used: u } = scanRawScript(node.lines)
      for (const x of d) defined.add(x)
      for (const x of u) used.add(x)
      rawBlocks.push({ line: node.line, count: node.lines.length })
      continue
    }
    if (node.kind === 'statement') {
      const t = node.trimmed
      const outM = /=>\s+(?:VAR|CAP)\s+@(\S+)/.exec(t)
      if (outM) defined.add(outM[1])
      const setM = /^SET\s+(VAR|CAP)\s+@(\S+)/.exec(t)
      if (setM) defined.add(setM[2])
      const markM = /^MARK\s+@(\S+)/.exec(t)
      if (markM) used.add(markM[1])
      for (const i of interpolations(t)) (i.kind === 'var' ? used : inputRefs).add(i.name)
      const varRefs = t.match(/@([A-Za-z_][\w.]*)/g) ?? []
      for (const r of varRefs) {
        const name = r.slice(1)
        if (/^(VAR|CAP)$/.test(name)) continue
        used.add(name)
      }
      if (/^#\S/.test(t)) labels.add(t.slice(1))
      if (t.startsWith('JUMP')) jumps.push({ line: node.line, target: t.replace(/^JUMP\s+/, '').replace(/^#/, '').trim() })
      if (t.startsWith('FUNCTION')) functions.add(t.split(/\s+/)[1])
      if (/^(IF|WHILE|REPEAT|FOREACH|TRY|LOCK|BLOCK)\b/.test(t)) stack.push({ line: node.line, kw: t.split(/\s+/)[0] })
      if (/^(END|ENDIF|ENDWHILE|ENDBLOCK|ENDTRYLOCK)\b/.test(t)) stack.pop()
      continue
    }
    if (node.kind !== 'block') continue

    const def = BY_ID.get(node.id)
    if (!def) {
      const ob1 = /^[A-Z][A-Z0-9]+$/.test(node.id)
      add('error', 'unknown-block', node.line, `Bilinmeyen blok: BLOCK:${node.id}`, ob1 && !opts.ob1 ? 'Bu bir OB1 (legacy LoliScript) bloğu; --ob1 ile dene.' : suggestBlock(node.id))
    } else if (def.aliases?.includes(node.id) && node.id !== def.id) {
      add('info', 'alias-block', node.line, `"${node.id}" bir takma ad; kararlı kimlik "${def.id}"`, `OB3+ uyumluluğu için BLOCK:${def.id} yaz.`)
    }

    if (def) {
      const allowed = new Map((def.params ?? []).map((p) => [p.n, p]))
      for (const s of node.settings) {
        const p = allowed.get(s.name)
        if (!p) {
          if (ALL_SETTINGS.has(s.name)) add('error', 'wrong-setting', node.line, `"${s.name}" bu blokta yok (BLOCK:${def.id})`, `Bu blokta geçerli ayarlar: ${[...allowed.keys()].join(', ')}`)
          else add('error', 'unknown-setting', node.line, `Bilinmeyen ayar: "${s.name}"`, suggestSetting(s.name))
          continue
        }
        const kind = VALUE_KIND(s.value)
        if (p.t === 'bool' && !['bool', 'variable', 'interpolated'].includes(kind)) add('error', 'type-mismatch', node.line, `${p.n} bool bekliyor, "${kind}" verildi`)
        if (p.t === 'int' && !['int', 'variable', 'interpolated'].includes(kind)) add('error', 'type-mismatch', node.line, `${p.n} int bekliyor, "${kind}" verildi`)
        if (p.t === 'enum' || ENUMS[p.t]) {
          const vals = ENUMS[p.t]
          if (kind === 'enumOrBare' && vals && !vals.includes(s.value)) add('error', 'bad-enum', node.line, `${p.n} için geçersiz değer "${s.value}" (${p.t})`, `Geçerli: ${vals.slice(0, 12).join(', ')}${vals.length > 12 ? ' …' : ''}`)
        }
        if (kind === 'variable') used.add(s.value.slice(1))
        for (const iv of interpolations(s.value)) (iv.kind === 'var' ? used : inputRefs).add(iv.name)
      }
      // Zorunlu ayar kontrolü: params tanımlıysa ve url/input gibi kritik alan yoksa uyar.
      const essentials = { HttpRequest: 'url', Parse: 'input' }
      const need = essentials[def.id]
      if (need && !node.settings.some((s) => s.name === need)) add('error', 'missing-setting', node.line, `BLOCK:${def.id} için "${need}" ayarı zorunlu`)
    }

    if (node.output) {
      if (node.output.malformed) add('error', 'bad-output', node.output.line ?? node.line, `Çıktı bildirimi bozuk: ${node.output.raw ?? node.text.trim()}`, 'Doğru biçim: => VAR @ad  veya  => CAP @ad')
      else {
        if (!VAR_RE.test(node.output.name)) add('error', 'bad-varname', node.output.line ?? node.line, `Geçersiz değişken adı: ${node.output.name}`, 'Değişken adı harfle/_ ile başlar; @ unutulmuş olabilir.')
        defined.add(node.output.name)
      }
    }

    // Keycheck gövdesi
    for (const k of node.keys ?? []) {
      const t = k.text
      if (t.startsWith('KEYCHAIN')) {
        const m = /^KEYCHAIN\s+(\w+)\s+(AND|OR)\s*$/.exec(t)
        if (!m) add('error', 'bad-keychain', k.line, `KEYCHAIN satırı bozuk: "${t}"`, 'Doğru biçim: KEYCHAIN SUCCESS OR')
        else if (!['SUCCESS', 'FAIL', 'FAILURE', 'BAN', 'RETRY', 'CUSTOM'].includes(m[1].toUpperCase())) add('error', 'bad-keychain-status', k.line, `Geçersiz keychain durumu: ${m[1]}`, 'Geçerli: SUCCESS, FAIL, BAN, RETRY, CUSTOM')
        continue
      }
      const km = /^([A-Z]+KEY)\s+(.+)$/.exec(t)
      if (!km) continue
      const kw = km[1]
      const ops = COMPARISONS[kw]
      if (!ops) {
        add('error', 'bad-key', k.line, `Geçersiz anahtar tipi: ${kw}`, `Geçerli: ${KEYWORDS.join(', ')}`)
        continue
      }
      const op = ops.find((o) => new RegExp(`\\s${o}\\s`).test(' ' + km[2] + ' '))
      if (!op) {
        const words = km[2].split(/\s+/)
        const typo = words.map((w) => suggestWord(w, ops)).find(Boolean)
        add('error', 'bad-comparison', k.line, `${kw} için karşılaştırma operatörü yok: "${km[2]}"`, (typo ? `Bunu mu demek istedin: ${typo}? ` : '') + `Geçerli: ${ops.join(', ')}`)
      }
      for (const iv of interpolations(km[2])) (iv.kind === 'var' ? used : inputRefs).add(iv.name)
      for (const rr of km[2].match(/@([A-Za-z_][\w.]*)/g) ?? []) used.add(rr.slice(1))
    }
    if (node.id === 'Keycheck' && (node.keys ?? []).length === 0) add('warn', 'empty-keycheck', node.line, 'Keycheck boş: hiç keychain yok', 'En az bir KEYCHAIN SUCCESS/FAIL/RETRY satırı ekle.')

    // Blok gövdesinde tanınmayan satır (Parse/HttpRequest gövdesi meşru)
    if (!['Parse', 'HttpRequest', 'Script', 'LoliCode'].includes(node.id)) {
      for (const bl of node.body ?? []) {
        if (/^(CONTENT|KEYCHAIN)/.test(bl.text) || /^[A-Z]+KEY\s/.test(bl.text)) continue
        add('warn', 'block-junk', bl.line, `BLOCK:${node.id} içinde tanınmayan satır: "${bl.text}"`)
      }
    }
    if (node.id === 'HttpRequest') {
      const types = ['STANDARD', 'RAW', 'BASICAUTH', 'MULTIPART']
      if (!node.meta.type) add('error', 'http-type', node.line, 'HttpRequest bloğunda TYPE satırı yok', `Ekle: TYPE:${types.join(' | ')}`)
      else if (!types.includes(node.meta.type)) add('error', 'http-type', node.line, `Geçersiz TYPE: ${node.meta.type}`, `Geçerli: ${types.join(', ')}`)
      // TYPE satırından sonraki ilk satırlar gövdedir; ayar/gövde tamamen boşsa şüphelen.
      else if ((node.body ?? []).length === 0 && node.meta.type !== 'MULTIPART') add('error', 'http-body', node.line, `TYPE:${node.meta.type} için gövde satırları eksik`, 'TYPE satırından sonra içerik + content-type satırları gelir.')
    }
  }

  // Tanımlanmadan kullanılan değişkenler
  // OB2 yerleşikleri + OB1'de özel input öneki yoktur (CustomInput.VariableName doğrudan <VARADI> olur),
  // bu yüzden OB1 modunda büyük harfli adlar input sanılır ve uyarı verilmez.
  const BUILTIN_PREFIX = ['data.', 'input.', 'globals.']
  const BUILTIN_EXACT = new Set(['SOURCE', 'STATUS', 'RESPONSECODE', 'COOKIES', 'HEADERS', 'ADDRESS', 'ERROR', 'PROXY', 'BOTNUM', 'RETRIES', 'INPUT', 'USEPROXY'])
  for (const name of used) {
    if (defined.has(name) || BUILTIN_PREFIX.some((p) => name.startsWith(p))) continue
    if (opts.ob1 && (BUILTIN_EXACT.has(name) || /^[A-Z0-9_]+$/.test(name))) continue
    add('warn', 'undefined-var', 0, `"<${name}>" kullanılıyor ama tanımlandığını görmedim`, `Define et: BLOCK:ConstantString ... => VAR @${name} veya SET VAR @${name} "..."`)
  }

  // OB1 modunda BLOCK:'suz blok satırlarını yakala
  if (opts.ob1) {
    const OB1_SUB = new Set(['HEADER', 'COOKIE', 'CONTENT', 'CONTENTTYPE', 'RAWDATA', 'STRINGCONTENT', 'FILECONTENT', 'BOUNDARY', 'SECPROTO', 'MULTIPART', 'BASICAUTH', 'STANDARD', 'RAW', 'KEYCHAIN', 'KEY', 'GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'HEAD', 'PROXY', 'USEPROXY', 'SOURCE', 'STATUS', 'COOKIES', 'ADDRESS', 'VAR', 'CAP', 'GVAR', 'NEWGVAR', 'DATA', 'RESPONSECODE', 'GCOOKIES', 'PROXYTYPE', 'FROM', 'IN', 'TO'])
    for (const node of nodes) {
      if (node.kind !== 'statement') continue
      const kw = node.trimmed.split(/\s+/)[0]
      if (!OB1_BLOCKS.has(kw) && !OB1_SUB.has(kw) && /^[A-Z][A-Z0-9._]{1,}$/.test(kw) && !node.trimmed.startsWith('#')) {
        add('warn', 'unknown-ob1-keyword', node.line, `Tanınmayan OB1 anahtar sözcüğü: ${kw}`, suggestWord(kw, [...OB1_BLOCKS, ...OB1_SUB]) ? `Bunu mu demek istedin: ${suggestWord(kw, [...OB1_BLOCKS, ...OB1_SUB])}?` : undefined)
      }
      if (kw === 'KEYCHAIN') {
        const m = /^KEYCHAIN\s+(\w+)\s+(AND|OR)\s*$/.exec(node.trimmed)
        if (!m) add('error', 'bad-keychain', node.line, `KEYCHAIN satırı bozuk: "${node.trimmed}"`, 'Doğru biçim: KEYCHAIN Success OR')
      }
      if (kw === 'KEY') {
        const body = node.trimmed.slice(3).trim()
        if (!body) add('error', 'empty-key', node.line, 'KEY boş')
        else if (!/\s(Contains|DoesNotContain|EqualTo|NotEqualTo|GreaterThan|LessThan|Exists|DoesNotExist|MatchesRegex|DoesNotMatchRegex)\s/.test(` ${body} `) && !/^"[^"]*"$/.test(body)) {
          add('warn', 'key-short-form', node.line, `KEY kısa biçimde: ${body}`, 'Kısa biçim SOURCE Contains demektir; farklı bir şey istiyorsan uzun biçimi yaz.')
        }
      }
      if (kw === 'FUNCTION') {
        const fn = node.trimmed.split(/\s+/)[1]
        const known = ['Constant', 'Base64Encode', 'Base64Decode', 'HMAC', 'Hash', 'ToUppercase', 'ToLowercase', 'Replace', 'RegexMatch', 'Unescape', 'URLEncode', 'URLDecode', 'DateToUnixTime', 'UnixTimeToDate', 'CurrentUnixTime', 'UnixTimeToISO8601', 'RandomNum', 'RandomString', 'Ceil', 'Floor', 'Round', 'Compute', 'CountOccurrences', 'Length', 'Substring', 'ReverseString', 'Trim', 'CharAt', 'Translate', 'RSA', 'RSAPKCS1PAD2', 'AESDecrypt', 'AESEncrypt', 'PBKDF2PKCS5', 'Delay', 'ClearCookies', 'GetRandomUA']
        if (fn && !known.includes(fn)) add('error', 'unknown-function', node.line, `Bilinmeyen OB1 FUNCTION: ${fn}`, 'Geçerli: ' + known.join(', '))
      }
      if (kw === 'REQUEST') {
        const m = /^REQUEST\s+(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS|CONNECT|TRACE)\s+"([^"]+)"/.exec(node.trimmed)
        if (!m) add('error', 'bad-request', node.line, `REQUEST satırı bozuk: "${node.trimmed}"`, 'Doğru biçim: REQUEST GET "https://..."')
      }
      if (kw === 'PARSE') {
        if (!/\b(LR|CSS|JSON|XPATH|REGEX)\b/.test(node.trimmed)) add('error', 'bad-parse-type', node.line, `PARSE tipi eksik: "${node.trimmed}"`, 'İkinci argüman tip olmalı: LR | CSS | JSON | XPATH | REGEX')
        if (!/->\s*(VAR|CAP)\s+"/.test(node.trimmed)) add('warn', 'parse-no-output', node.line, 'PARSE çıktı değişkeni yok', 'Sonuna `-> VAR "ad"` ekle.')
      }
      if (/^(BEGIN|END)\s+SCRIPT/.test(node.trimmed)) {
        const lang = /BEGIN\s+SCRIPT\s+(\w+)/.exec(node.trimmed)?.[1]
        if (lang && !['JAVASCRIPT', 'IRONPYTHON'].includes(lang.toUpperCase())) add('warn', 'unknown-script-lang', node.line, `Bilinmeyen script dili: ${lang}`, 'OB1: JAVASCRIPT veya IRONPYTHON')
      }
    }
    const arrowCount = (text.match(/->\s*(VAR|CAP)\s+"/g) ?? []).length
    if (arrowCount === 0 && /FUNCTION|PARSE/.test(text)) add('warn', 'ob1-no-arrow', 0, 'Hiç `-> VAR "ad"` bulunamadı', 'OB1 çıktı ataması `-> VAR "ad"` biçimindedir.')
  }

  // Ulaşılamayan hedef
  for (const j of jumps) if (!labels.has(j.target)) add('error', 'bad-jump', j.line, `JUMP hedefi yok: #${j.target}`)

  // Denge
  if (stack.length) for (const s of stack) add('warn', 'unclosed', s.line, `${s.kw} bloğu kapanmamış (END/ENDIF/ENDWHILE eksik)`)

  // Blok içi denge: BEGIN SCRIPT / END SCRIPT (OB1) ve BEGIN ... / END SCRIPT (OB2)
  const beginCount = (text.match(/^\s*BEGIN\s+SCRIPT/gm) ?? []).length
  const endCount = (text.match(/^\s*END\s+SCRIPT/gm) ?? []).length
  if (beginCount !== endCount) add('error', 'script-balance', 0, `BEGIN SCRIPT (${beginCount}) ve END SCRIPT (${endCount}) sayısı uyuşmuyor`)

  // OB2 bloklarını OB1 betiğinde kullanma uyarısı
  if (opts.ob1) {
    for (const b of blocks) if (!OB1_BLOCKS.has(b.id) && BY_ID.has(b.id)) add('warn', 'ob2-block-in-ob1', b.line, `BLOCK:${b.id} OB2 bloğu; OB1/SilverBullet LoliScript'te karşılığı yok`)
  } else {
    // UPPERCASE blok adları OB1 işaretidir
    for (const b of blocks) if (!BY_ID.has(b.id) && OB1_BLOCKS.has(b.id)) add('info', 'ob1-block', b.line, `BLOCK:${b.id} OB1 sözdizimi`, 'OB2 karşılığı: ' + (OB1_TO_OB2[b.id] ?? '?'))
  }

  // Yaygın tuzaklar
  for (const b of blocks) {
    if (b.id === 'HttpRequest') {
      const hdr = b.settings.find((s) => s.name === 'customHeaders')?.value ?? ''
      const cookies = b.settings.find((s) => s.name === 'customCookies')?.value ?? ''
      if (!/user-agent/i.test(hdr) && !hdr.startsWith('@')) add('info', 'http-ua', b.line, 'customHeaders içinde User-Agent yok', 'Sunucular UA yoksa bot sayar; {"User-Agent": "<@ua>"} ekle.')
      if (cookies && !cookies.includes('(') && !cookies.startsWith('@')) add('info', 'http-cookie', b.line, 'customCookies sözlük değil', 'Biçim: ${("ad","deger"), ...} veya @data.COOKIES')
    }
    if (b.id === 'Keycheck' && !b.settings.some((s) => s.name === 'banIfNoMatch')) {
      const hasSuccess = (b.keys ?? []).some((l) => /^KEYCHAIN\s+SUCCESS/.test(l.text))
      if (hasSuccess) add('warn', 'keycheck-ban', b.line, 'banIfNoMatch eksik (varsayılan true): bu, hiçbir keychain tutmazsa proxy BAN edilir', 'Proxy banı istemiyorsan banIfNoMatch = False ekle.')
    }
    if (b.id === 'Parse') {
      const mode = b.meta.mode
      const needed = { LR: ['leftDelim', 'rightDelim'], CSS: ['cssSelector'], XPath: ['xPath'], Json: ['jToken'], Regex: ['pattern'] }
      const nx = needed[mode]
      if (nx) for (const n of nx) if (!b.settings.some((s) => s.name === n)) add('warn', 'parse-arg', b.line, `MODE:${mode} için "${n}" ayarı gerekli`, 'Boşsa eşleşme olmaz; doldur.')
    }
    if (/^Puppeteer/.test(b.id) && BY_ID.has(b.id)) add('info', 'legacy-alias', b.line, `Puppeteer* adı OB1 kalıntısı; OB2 karşılığı ${BY_ID.get(b.id).id}`)
  }

  // İstatistik
  const summary = {
    lines: text.split('\n').length,
    blocks: blocks.length,
    rawBlocks: rawBlocks.length,
    rawLines: rawBlocks.reduce((a, b) => a + b.count, 0),
    disabled: blocks.filter((b) => b.disabled).length,
    statements: nodes.filter((n) => n.kind === 'statement').length,
    variables: defined.size,
    errors: diags.filter((d) => d.severity === 'error').length,
    warnings: diags.filter((d) => d.severity === 'warn').length,
    infos: diags.filter((d) => d.severity === 'info').length,
    dataSource: META.source,
  }
  if (rawBlocks.length) add('info', 'raw-csharp', rawBlocks[0].line, `${rawBlocks.length} ham C# (LoliCode) bloğu: ${rawBlocks.reduce((a, b) => a + b.count, 0)} satır`, 'BLOCK: dışındaki satırlar OB2de LoliCodeBlockInstance olur ve dogrudan C#e transpile edilir; stack görünümünde tek blok olarak durur.')
  diags.sort((a, b) => a.line - b.line || a.severity.localeCompare(b.severity))
  return { diagnostics: diags, summary, blocks, nodes }
}

// OB1 → OB2 blok adı eşlemesi (legacy yükseltme için)
const OB1_TO_OB2 = {
  REQUEST: 'HttpRequest',
  PARSE: 'Parse',
  KEYCHECK: 'Keycheck',
  FUNCTION: 'ConstantString / Hash / Replace / …',
  CAPTCHA: 'SolveImageCaptcha',
  RECAPTCHA: 'SolveRecaptchaV2',
  SOLVECAPTCHA: 'SolveImageCaptcha',
  BYPASSCF: 'SolveCloudflareChallengePage',
  TCP: 'TcpConnect / TcpSend / TcpRead',
  NAVIGATE: 'BrowserNavigateTo / SeleniumNavigateTo',
  ELEMENTACTION: 'BrowserClick / SeleniumClick',
  BROWSERACTION: 'Browser* / Selenium*',
  EXECUTEJS: 'BrowserExecuteJs',
}

/** Kaynak: RuriLib/Helpers/LoliCode/LoliCodeParser.cs + LineParser.cs (OB2 master). */
function levenshtein(a, b) {
  const m = a.length
  const n = b.length
  const d = Array.from({ length: m + 1 }, (_, i) => [i, ...Array(n).fill(0)])
  for (let j = 0; j <= n; j++) d[0][j] = j
  for (let i = 1; i <= m; i++)
    for (let j = 1; j <= n; j++)
      d[i][j] = Math.min(d[i - 1][j] + 1, d[i][j - 1] + 1, d[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1))
  return d[m][n]
}

/** Kelime düzeltme önerisi (yakın aday). */
function suggestWord(word, candidates) {
  const w = word.toLowerCase()
  let best = null
  for (const c of candidates) {
    const d = levenshtein(w, c.toLowerCase())
    if (d <= 2 && (!best || d < best.d)) best = { c, d }
  }
  return best?.c
}

function suggestBlock(id) {
  const scored = [...BY_ID.keys()]
    .map((k) => ({ k, d: levenshtein(id.toLowerCase(), k.toLowerCase()) }))
    .filter((x) => x.d <= Math.max(2, Math.floor(id.length / 3)))
    .sort((a, b) => a.d - b.d)
    .slice(0, 5)
  return scored.length ? `Bunu mu demek istedin: ${scored.map((s) => s.k).join(', ')}?` : undefined
}

function suggestSetting(name) {
  const scored = [...ALL_SETTINGS]
    .map((k) => ({ k, d: levenshtein(name.toLowerCase(), k.toLowerCase()) }))
    .filter((x) => x.d <= 3)
    .sort((a, b) => a.d - b.d)
    .slice(0, 5)
  return scored.length ? `Benzer ayarlar: ${scored.map((s) => s.k).join(', ')}` : undefined
}

/** Blok kataloğunda arama (ad, açıklama, ayar adı). */
export function searchBlocks(query, limit = 20) {
  const q = query.toLowerCase()
  const out = []
  for (const b of BLOCKS) {
    let score = 0
    if (b.id.toLowerCase() === q) score += 100
    else if (b.id.toLowerCase().includes(q)) score += 50
    if (b.desc?.toLowerCase().includes(q)) score += 20
    if (b.cat.toLowerCase().includes(q)) score += 10
    if ((b.params ?? []).some((p) => p.n.toLowerCase().includes(q))) score += 5
    if (score) out.push({ ...b, score })
  }
  return out.sort((a, b) => b.score - a.score || a.id.localeCompare(b.id)).slice(0, limit)
}

/** Blok kimliğini çözer (takma ad dahil). */
export function resolveBlock(id) {
  return BY_ID.get(id) ?? null
}

/** Blok dokümanı: ayarlar, çıktı tipi, örnek LoliCode. */
export function blockDoc(id) {
  const b = BY_ID.get(id)
  if (!b) {
    const s = searchBlocks(id, 5)
    return { found: false, suggestions: s.map((x) => x.id) }
  }
  const required = { HttpRequest: ['url'], Parse: ['input'] }[b.id] ?? []
  const example = SPECIAL_EXAMPLES[b.id] ?? [
    `BLOCK:${b.id}`,
    `  LABEL:${b.id}`,
    ...(b.params ?? []).slice(0, 6).map((p) => `  ${p.n} = ${defaultFor(p.t, p.n)}`),
    ...(b.ret && b.ret !== 'void' && b.ret !== 'Task' ? [`  => VAR @out`] : []),
    'ENDBLOCK',
  ].join('\n')
  return {
    found: true,
    block: b,
    required,
    example,
    aliases: b.aliases ?? [],
    note: b.ret === 'void' || b.ret === 'Task' ? 'Çıktı yok (dönüş tipi void/Task).' : `Çıktı tipi: ${b.ret}`,
  }
}

/** Özel (custom) blokların gerçek gövde sözdizimi — kaynak: *BlockInstance.cs ToLC. */
const SPECIAL_EXAMPLES = {
  HttpRequest: [
    'BLOCK:HttpRequest',
    '  LABEL:Login',
    '  url = $"https://site.com/api/login"',
    '  method = POST',
    '  customHeaders = ${("Content-Type", "application/json"), ("User-Agent", "<UA>")}',
    '  TYPE:STANDARD',
    '  $"{\\"user\\":\\"<input.USER>\\",\\"pass\\":\\"<input.PASS>\\"}"',
    '  "application/json"',
    'ENDBLOCK',
  ].join('\n'),
  Parse: [
    'BLOCK:Parse',
    '  input = @data.SOURCE',
    '  leftDelim = "token\\":\\""',
    '  rightDelim = "\\""',
    '  MODE:LR',
    '  => VAR @token',
    'ENDBLOCK',
  ].join('\n'),
  Keycheck: [
    'BLOCK:Keycheck',
    '  banIfNoMatch = False',
    '  KEYCHAIN SUCCESS OR',
    '    STRINGKEY @data.SOURCE Contains "\\"check\\":true"',
    '  KEYCHAIN FAIL OR',
    '    STRINGKEY @data.SOURCE Contains "bulunamadi"',
    '  KEYCHAIN RETRY OR',
    '    STRINGKEY @data.SOURCE Contains "guvenlik"',
    'ENDBLOCK',
  ].join('\n'),
}

function defaultFor(type, name) {
  const base = type.replace(/\?$/, '')
  if (base === 'bool') return 'true'
  if (base === 'int' || base === 'long') return '0'
  if (base === 'float' || base === 'double') return '0.0'
  if (base === 'byte[]') return '""'
  if (base === 'List<string>') return '[]'
  if (base === 'Dictionary<string, string>') return '{}'
  if (ENUMS[base]) return ENUMS[base][0]
  if (name === 'url') return '"https://example.com"'
  return '"..."'
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------
const isMain = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))
if (isMain) {
  const args = process.argv.slice(2)
  const json = args.includes('--json')
  const ob1 = args.includes('--ob1')
  // `--blocks` isteğe bağlı değer alır: kendisinden SONRA gelen ve -- ile başlamayan ilk argüman değer sayılır.
  const valueAfter = (flag) => {
    const i = args.indexOf(flag)
    if (i < 0) return null
    const v = args[i + 1]
    return v && !v.startsWith('--') ? v : null
  }
  const cmd = args.find((a) => !a.startsWith('--') && args[args.indexOf(a) - 1] !== '--blocks' && args[args.indexOf(a) - 1] !== '--doc') ?? null
  const usage = `Kullanım:
  node lint.mjs <betik.loli> [--json] [--ob1]     betiği denetle
  node lint.mjs --blocks [arama]                   blok ara
  node lint.mjs --doc <BlokId>                     blok dokümanı
  node lint.mjs --categories                       kategoriler`

  if (args.includes('--help')) {
    console.log(usage)
    process.exit(0)
  }
  if (args.includes('--categories')) {
    for (const c of CATEGORIES) console.log(`${c}\t${BLOCKS.filter((b) => b.cat === c).length} blok`)
    process.exit(0)
  }
  if (args.includes('--blocks')) {
    const q = valueAfter('--blocks')
    const res = q ? searchBlocks(q, 25) : BLOCKS.slice(0, 60)
    for (const b of res) console.log(`${b.id}\t[${b.cat}]\t${b.desc ?? ''}`)
    console.log(`\n${BLOCKS.length} blok, ${META.source}`)
    process.exit(0)
  }
  if (args.includes('--doc')) {
    const doc = blockDoc(valueAfter('--doc') ?? '')
    if (!doc.found) {
      console.log('Bulunamadı. Öneriler:', doc.suggestions.join(', '))
      process.exit(1)
    }
    console.log(`${doc.block.id} — ${doc.block.cat}\n${doc.block.desc}`)
    console.log('Ayarlar: ' + (doc.block.params ?? []).map((p) => `${p.n}:${p.t}`).join(', '))
    console.log('\n' + doc.example)
    process.exit(0)
  }
  if (!cmd) {
    console.log(usage)
    process.exit(0)
  }

  let text
  try {
    text = readFileSync(cmd, 'utf8')
  } catch {
    console.error(`Dosya okunamadı: ${cmd}`)
    process.exit(2)
  }
  const { diagnostics, summary } = lintScript(text, { ob1 })
  if (json) {
    console.log(JSON.stringify({ summary, diagnostics }, null, 1))
  } else {
    for (const d of diagnostics) console.log(`${d.severity === 'error' ? 'HATA' : d.severity === 'warn' ? 'UYARI' : 'BİLGİ'}${d.line ? ' satır ' + d.line : ''}: ${d.message}${d.hint ? `\n      → ${d.hint}` : ''}`)
    console.log(`\n${summary.blocks} blok, ${summary.statements} ifade, ${summary.errors} hata, ${summary.warnings} uyarı, ${summary.infos} bilgi`)
  }
  process.exit(summary.errors > 0 ? 1 : 0)
}
