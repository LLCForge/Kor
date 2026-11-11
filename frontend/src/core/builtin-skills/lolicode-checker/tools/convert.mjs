// OB1 / SilverBullet LoliScript → OB2 LoliCode iskelet dönüştürücü.
// Tam otomatik değildir: yapıyı çevirir, çeviremediğini `// TODO` olarak bırakır ve raporlar.
// Kullanım: node convert.mjs girilen.loli [cikti.loli]
// Kaynak: OB1 BlockParser.cs / BlockFunction.cs, OB2 RuriLib/Blocks/* — bkz. ../reference/ob1-to-ob2.md

import { readFileSync, writeFileSync } from 'node:fs'

const warn = []
const W = (line, msg) => warn.push(`satır ${line}: ${msg}`)

const VAL = ['"', '$', '@']
const q = (s) => (s == null || s === '' ? '""' : VAL.some((p) => s.startsWith(p)) ? s : `"${s.replace(/"/g, '\\"')}"`)
const SP = '@data.SOURCE'

/** Tırnak farkındaki basit argüman ayırıcı. */
function args(str) {
  const out = []
  let cur = ''
  let inStr = false
  for (let i = 0; i < str.length; i++) {
    const c = str[i]
    if (c === '"' && str[i - 1] !== '\\') inStr = !inStr
    if (!inStr && /\s/.test(c)) {
      if (cur) out.push(cur)
      cur = ''
      continue
    }
    cur += c
  }
  if (cur) out.push(cur)
  return out
}

const unq = (s) => (s ?? '').replace(/^"|"$/g, '')

/** OB1 → OB2 blok iskeleti. Fonksiyonlar ikiye ayrıldı: blok üretir, çıktı ayrı eklenir. */
const FN = {
  Constant: (a) => ({ id: 'ConstantString', set: [['value', q(a[0])]], ret: 'string' }),
  Hash: (a) => ({ id: 'HashString', set: [['input', q(a[1])], ['hashFunction', String(a[0] ?? 'SHA256').toUpperCase()]], ret: 'string' }),
  HMAC: (a) => ({ id: 'HmacString', set: [['input', q(a[1])], ['key', '""'], ['hashFunction', String(a[0] ?? 'SHA256').toUpperCase()]], ret: 'string' }),
  Base64Encode: (a) => ({ id: 'UTF8ToBase64', set: [['input', q(a[0])]], ret: 'string' }),
  Base64Decode: (a) => ({ id: 'Base64ToUTF8', set: [['input', q(a[0])]], ret: 'string' }),
  URLEncode: (a) => ({ id: 'UrlEncode', set: [['input', q(a[0])]], ret: 'string' }),
  URLDecode: (a) => ({ id: 'UrlDecode', set: [['input', q(a[0])]], ret: 'string' }),
  Unescape: (a) => ({ id: 'Unescape', set: [['input', q(a[0])]], ret: 'string' }),
  ToUppercase: (a) => ({ id: 'ToUppercase', set: [['input', q(a[0])]], ret: 'string' }),
  ToLowercase: (a) => ({ id: 'ToLowercase', set: [['input', q(a[0])]], ret: 'string' }),
  Replace: (a) => ({ id: 'Replace', set: [['original', q(a[0])], ['toReplace', q(a[1])], ['replacement', q(a[2])]], ret: 'string' }),
  Trim: (a) => ({ id: 'Trim', set: [['input', q(a[0])]], ret: 'string' }),
  Length: (a) => ({ id: 'Length', set: [['input', q(a[0])]], ret: 'int' }),
  Substring: (a) => ({ id: 'Substring', set: [['input', q(a[0])], ['index', a[1] ?? '0'], ['length', a[2] ?? '0']], ret: 'string' }),
  ReverseString: (a) => ({ id: 'Reverse', set: [['input', q(a[0])]], ret: 'string' }),
  CharAt: (a) => ({ id: 'CharAt', set: [['input', q(a[0])], ['index', a[1] ?? '0']], ret: 'string' }),
  CountOccurrences: (a) => ({ id: 'CountOccurrences', set: [['input', q(a[0])], ['word', q(a[1])]], ret: 'int' }),
  RandomNum: (a) => ({ id: 'RandomInteger', set: [['minimum', a[0] ?? '0'], ['maximum', a[1] ?? '100']], ret: 'long' }),
  RandomString: (a) => ({ id: 'RandomString', set: [['input', q(a[0])]], ret: 'string' }),
  GetRandomUA: (a) => ({ id: 'RandomUserAgent', set: [['platform', a[0] ?? 'All']], ret: 'string' }),
  Delay: (a) => ({ id: 'Delay', set: [['milliseconds', a[0] ?? '500']], ret: 'void' }),
  ClearCookies: () => ({ id: 'ClearCookies', set: [], ret: 'void' }),
  CurrentUnixTime: () => ({ id: 'CurrentUnixTime', set: [['useUtc', 'true'], ['outputMilliseconds', 'false']], ret: 'long' }),
  AESEncrypt: (a) => ({ id: 'AESEncryptString', set: [['plainText', q(a[0])], ['key', q(a[1])], ['iv', q(a[2])], ['mode', 'CBC'], ['padding', 'PKCS7'], ['keySize', '256']], ret: 'byte[]' }),
  AESDecrypt: (a) => ({ id: 'AESDecryptString', set: [['cipherText', q(a[0])], ['key', q(a[1])], ['iv', q(a[2])], ['mode', 'CBC'], ['padding', 'PKCS7'], ['keySize', '256']], ret: 'string' }),
}

const fnBlock = ({ id, set, ret }, outVar, lineNo) => {
  const L = [`BLOCK:${id}`, ...set.map(([k, v]) => `  ${k} = ${v}`)]
  if (ret !== 'void') {
    if (outVar) L.push(`  => VAR @${outVar}`)
    else W(lineNo, `${id} sonucu bir değişkene atanmadı (-> VAR "ad" ekle)`)
  }
  L.push('ENDBLOCK')
  return L
}

const PARSE_KEYS = { LR: ['leftDelim', 'rightDelim'], CSS: ['cssSelector'], JSON: ['jToken'], XPATH: ['xPath'], REGEX: ['pattern', 'outputFormat'] }
const NORM_MODE = { JSON: 'Json', XPATH: 'XPath', REGEX: 'Regex', LR: 'LR', CSS: 'CSS' }

/** Ana dönüşüm. */
export function convert(text) {
  warn.length = 0
  const lines = text.replace(/\r\n?/g, '\n').split('\n')
  const out = ['// OB1/SilverBullet → OB2 dönüşümü — TODO satırlarını elle tamamla, sonra lint.mjs ile denetle.', '']
  const sIdx = lines.findIndex((l) => l.trim() === '[SCRIPT]')
  const body = sIdx >= 0 ? lines.slice(sIdx + 1) : lines
  if (sIdx >= 0) out.push('// [SETTINGS] → settings.json içine taşınacak (opk.mjs pack kullan).', '')

  for (let n = 0; n < body.length; n++) {
    const t = body[n].trim()
    const lineNo = n + 1
    if (!t) continue
    if (t.startsWith('##')) {
      out.push('// ' + t.slice(2).trim())
      continue
    }
    if (t.startsWith('!')) {
      out.push('// ' + t.slice(1).trim())
      continue
    }
    if (/^#[A-Z0-9_]+$/.test(t)) {
      out.push('# ' + t.slice(1))
      continue
    }
    // [satır] -> VAR "ad" / -> CAP "ad" çıktısını ayır
    const arrow = /\s*->\s*(VAR|CAP)\s+"?(\w+)"?\s*$/.exec(t)
    const head = arrow ? t.slice(0, arrow.index).trim() : t
    const outVar = arrow?.[2]
    const outCap = arrow?.[1] === 'CAP'
    const [kw] = head.split(/\s+/)
    const argLine = head.slice(kw.length).trim()

    // --- REQUEST (çok satırlı blok) ---
    if (kw === 'REQUEST') {
      const L = ['BLOCK:HttpRequest']
      const a = args(argLine)
      const method = a[0] && /^[A-Z]+$/.test(a[0]) ? a[0] : 'GET'
      const url = method === 'GET' && !/^[A-Z]+$/.test(a[0] ?? '') ? a[0] : a[1]
      L.push(`  url = ${q(url)}`, `  method = ${method}`)
      const headers = []
      let content = null
      let contentType = 'application/x-www-form-urlencoded'
      let type = 'STANDARD'
      let j = n + 1
      for (; j < body.length; j++) {
        const s = body[j].trim()
        if (!s || s.startsWith('#')) continue
        if (/^[A-Z]{2,}\s/.test(s) && !/^(HEADER|CONTENT|RAWDATA|CONTENTTYPE|COOKIE|USERNAME|PASSWORD|STANDARD|RAW|MULTIPART|BASICAUTH|SECPROTO|STRINGCONTENT|FILECONTENT|BOUNDARY|ADDRESS)\b/.test(s)) break
        const [sk, ...sr] = s.split(/\s+/)
        const sv = s.slice(sk.length).trim()
        if (sk === 'HEADER') {
          // OB1 biçimi tek string: HEADER "Ad: deger" — ilk ':' karakterinden ayır.
          const raw = unq(args(sv).join(' '))
          const i = raw.indexOf(':')
          if (i > 0) {
            const hname = raw.slice(0, i).trim()
            const hval = raw.slice(i + 1).trim()
            if (hval) headers.push(`("${hname}", "${hval.replace(/"/g, '\\"')}")`)
            else headers.push(`("${hname}", "")`)
          } else {
            headers.push(`("${raw}", "")`)
          }
        } else if (sk === 'CONTENT' || sk === 'RAWDATA') content = sv
        else if (sk === 'CONTENTTYPE') contentType = sv
        else if (sk === 'COOKIE') L.push('  // TODO COOKIE: customCookies ayarına taşı (çerez kavanozu OB2de otomatiktir)')
        else if (sk === 'USERNAME' || sk === 'PASSWORD') W(lineNo, `${sk} BASICAUTH'a ait; TYPE:BASICAUTH + satırları elle yaz`)
        else if (['STANDARD', 'RAW', 'MULTIPART', 'BASICAUTH'].includes(sk)) type = sk
      }
      if (headers.length) L.push(`  customHeaders = $\{${headers.join(', ')}}`)
      L.push(`  TYPE:${type}`)
      L.push(`  ${content ?? '""'}`)
      L.push(`  ${contentType.startsWith('"') || contentType.startsWith('$') ? contentType : `"${contentType}"`}`)
      L.push('ENDBLOCK')
      out.push(...L)
      n = j - 1
      continue
    }

    // --- KEYCHECK + ardışık KEYCHAIN/KEY ---
    if (kw === 'KEYCHECK') {
      const L = ['BLOCK:Keycheck', `  banIfNoMatch = ${/BanOnToCheck\s*=\s*TRUE/i.test(argLine) ? 'True' : 'False'}`]
      if (/BanOn4XX\s*=\s*TRUE/i.test(argLine)) W(lineNo, 'BanOn4XX kuralı elle yazılmalı: IF INTKEY @data.RESPONSECODE GreaterThanOrEqualTo 400 → BAN')
      let j = n + 1
      for (; j < body.length; j++) {
        const s = body[j].trim()
        if (!s || s.startsWith('#')) continue
        if (s.startsWith('KEYCHAIN')) {
          const [st, mode] = s.slice(8).trim().split(/\s+/)
          const stN = { Success: 'SUCCESS', Failure: 'FAIL', Fail: 'FAIL' }[st] ?? st.toUpperCase()
          L.push(`  KEYCHAIN ${stN} ${(mode ?? 'OR').toUpperCase()}`)
          continue
        }
        if (s.startsWith('KEY ')) {
          const a = args(s.slice(4).trim())
          L.push(a.length === 1 ? `    STRINGKEY ${SP} Contains ${a[0]}` : `    ${/^"?\d+"?$/.test(a[0]) ? 'INTKEY' : 'STRINGKEY'} ${q(a[0])} ${a[1]} ${a[2] ?? '""'}`)
          continue
        }
        break
      }
      L.push('ENDBLOCK')
      out.push(...L)
      n = j - 1
      continue
    }

    // --- PARSE ---
    if (kw === 'PARSE') {
      const a = args(argLine)
      const mIdx = a.findIndex((x) => /^(LR|CSS|JSON|XPATH|REGEX)$/i.test(x))
      const mode = mIdx >= 0 ? NORM_MODE[a[mIdx].toUpperCase()] : 'LR'
      if (mIdx < 0) W(lineNo, 'PARSE tipi bulunamadı, LR varsayıldı')
      const input = a[0] && a[0] !== '"<SOURCE>"' && a[0] !== '<SOURCE>' ? a[0] : SP
      const rest = mIdx >= 0 ? a.slice(mIdx + 1).filter((x) => !/^\*?[A-Z]+$/.test(x) || x.includes('"')) : []
      const L = ['BLOCK:Parse', `  input = ${input}`]
      ;(PARSE_KEYS[mode] ?? []).forEach((k, i) => {
        if (rest.length > i * 2) L.push(`  ${k} = ${q(rest[i * 2])}`)
        else if (rest[i] != null) L.push(`  ${k} = ${q(rest[i])}`)
      })
      L.push(`  MODE:${mode}`)
      if (outVar) L.push(`  => ${outCap ? 'CAP' : 'VAR'} @${outVar}`)
      else W(lineNo, 'PARSE çıktı değişkeni yok (-> VAR "ad")')
      L.push('ENDBLOCK')
      out.push(...L)
      continue
    }

    // --- FUNCTION ---
    if (kw === 'FUNCTION') {
      const a = args(argLine)
      const fn = a[0]
      const map = FN[fn]
      if (!map) {
        W(lineNo, `FUNCTION ${fn} eşlenmedi`)
        out.push(`// TODO FUNCTION ${head}`)
      } else out.push(...fnBlock(map(a.slice(1)), outVar, lineNo))
      continue
    }

    if (kw === 'UTILITY') {
      W(lineNo, 'UTILITY elle çevrilmeli (List Functions / Files blokları)')
      out.push(`// TODO ${head}`)
      continue
    }
    if (/^(CAPTCHA|RECAPTCHA|SOLVECAPTCHA|REPORTCAPTCHA|BYPASSCF|TCP)\b/.test(kw)) {
      W(lineNo, `${kw} elle çevrilmeli (bkz. reference/ob1-to-ob2.md)`)
      out.push(`// TODO ${head}`)
      continue
    }
    if (/^(NAVIGATE|ELEMENTACTION|BROWSERACTION|EXECUTEJS)\b/.test(kw)) {
      W(lineNo, `${kw} (Selenium) → Browser* blokları; ayar adları farklı`)
      out.push(`// TODO ${head}`)
      continue
    }

    out.push(statement(head, lineNo))
  }
  return { text: out.join('\n').replace(/\n{3,}/g, '\n\n') + '\n', warnings: [...warn] }
}

function statement(t, lineNo) {
  let m
  if ((m = /^SET\s+VAR\s+"?(\w+)"?\s+(.*)$/.exec(t))) return `SET VAR @${m[1]} ${q(m[2])}`
  if ((m = /^SET\s+CAP\s+"?(\w+)"?\s+(.*)$/.exec(t))) return `SET CAP @${m[1]} ${q(m[2])}`
  if ((m = /^SET\s+SOURCE\s+(.*)$/.exec(t))) return `SET VAR @data.SOURCE ${m[1]}`
  if ((m = /^SET\s+COOKIE\s+(.*)$/.exec(t))) return `SET VAR @data.COOKIES ${m[1]}`
  if (/^DELAY\b/.test(t)) return `BLOCK:Delay\n  milliseconds = ${t.split(/\s+/)[1] ?? 500}\nENDBLOCK`
  if (/^PRINT\b/.test(t)) return 'LOG ' + t.slice(5).trim()
  if (/^(ENDIF|ENDWHILE)\b/.test(t)) return 'END'
  if (/^BEGIN\s+SCRIPT\b/.test(t)) return '// TODO BEGIN SCRIPT → BLOCK:Script'
  if (/^END\s+SCRIPT/.test(t)) return '// TODO END SCRIPT'
  if (/^EXECUTEJS\b/.test(t)) return '// TODO EXECUTEJS → BLOCK:BrowserExecuteJs'
  if (/^(IF|WHILE)\b/.test(t)) {
    const cond = t.replace(/^(IF|WHILE)\s+/, '')
    const cm = /^"?([^"\s]+)"?\s+([A-Za-z]+)\s+/.exec(cond)
    if (cm && /^(Contains|DoesNotContain|EqualTo|NotEqualTo|GreaterThan|LessThan|Exists|DoesNotExist|MatchesRegex|DoesNotMatchRegex|GreaterThanOrEqualTo|LessThanOrEqualTo)$/.test(cm[2])) {
      const kw = /^\d+$/.test(unq(cm[1])) ? 'INTKEY' : 'STRINGKEY'
      return `${t.split(/\s+/)[0]} ${kw} ${cond}`
    }
    W(lineNo, 'IF/WHILE koşulu elle doğrulanmalı')
    return t
  }
  return t
}

if (process.argv[1]?.endsWith('convert.mjs')) {
  const [inFile, outFile] = process.argv.slice(2).filter((x) => !x.startsWith('--'))
  if (!inFile) {
    console.log('Kullanım: node convert.mjs girilen.loli [cikti.loli]')
    process.exit(1)
  }
  const res = convert(readFileSync(inFile, 'utf8'))
  const target = outFile ?? inFile.replace(/\.loli$/, '') + '.ob2.loli'
  writeFileSync(target, res.text)
  console.log(`Yazıldı: ${target}`)
  if (res.warnings.length) console.log(`\n${res.warnings.length} uyarı (elle tamamlanacak):\n- ` + res.warnings.join('\n- '))
}
