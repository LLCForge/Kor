// .opk paket aracı — OpenBullet2 config paketi (zip: metadata.json, settings.json, script.loli, readme.md).
// Bağımlılıksız: Node'un kendi zlib'i ile minimal zip yazar, kendi okuyucusu ile açar.
//
// Kullanım:
//   node opk.mjs unpack config.opk ./cikti
//   node opk.mjs pack ./config-klasoru config.opk [--name "Config Adı"]
//   node opk.mjs info config.opk
//
// Kaynak: RuriLib/Helpers/ConfigPacker.cs (OB2 master) — zip giriş adları birebir aynıdır.

import { deflateRawSync, inflateRawSync } from 'node:zlib'
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'

const CRC_TABLE = (() => {
  const t = new Int32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c
  }
  return t
})()

function crc32(buf) {
  let c = 0xffffffff
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

/** Minimal zip yazıcı (deflate, tek disk, utf-8 adlar). */
function zipWrite(entries) {
  const chunks = []
  const central = []
  let offset = 0
  for (const [name, data] of entries) {
    const nameBuf = Buffer.from(name, 'utf8')
    const crc = crc32(data)
    const comp = deflateRawSync(data)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(20, 4)
    local.writeUInt16LE(0x0800, 6) // UTF-8 ad
    local.writeUInt16LE(8, 8) // deflate
    local.writeUInt16LE(0, 10) // saat
    local.writeUInt16LE(0x2100, 12) // tarih (2000-01-01)
    local.writeUInt32LE(crc, 14)
    local.writeUInt32LE(comp.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(nameBuf.length, 26)
    local.writeUInt16LE(0, 28)
    chunks.push(local, nameBuf, comp)
    central.push({ nameBuf, crc, comp: comp.length, size: data.length, offset })
    offset += local.length + nameBuf.length + comp.length
  }
  const cdStart = offset
  for (const e of central) {
    const h = Buffer.alloc(46)
    h.writeUInt32LE(0x02014b50, 0)
    h.writeUInt16LE(20, 4)
    h.writeUInt16LE(20, 6)
    h.writeUInt16LE(0x0800, 8)
    h.writeUInt16LE(8, 10)
    h.writeUInt16LE(0, 12)
    h.writeUInt16LE(0x2100, 14)
    h.writeUInt32LE(e.crc, 16)
    h.writeUInt32LE(e.comp, 20)
    h.writeUInt32LE(e.size, 24)
    h.writeUInt16LE(e.nameBuf.length, 28)
    h.writeUInt16LE(0, 30)
    h.writeUInt16LE(0, 32)
    h.writeUInt16LE(0, 34)
    h.writeUInt16LE(0, 36)
    h.writeUInt32LE(0, 38)
    h.writeUInt32LE(e.offset, 42)
    chunks.push(h, e.nameBuf)
    offset += h.length + e.nameBuf.length
  }
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(central.length, 8)
  end.writeUInt16LE(central.length, 10)
  end.writeUInt32LE(offset - cdStart, 12)
  end.writeUInt32LE(cdStart, 16)
  chunks.push(end)
  return Buffer.concat(chunks)
}

/** Minimal zip okuyucu: merkezi dizini tarar, girişleri çıkarır. */
function zipRead(buf) {
  let eocd = -1
  for (let i = buf.length - 22; i >= 0 && i > buf.length - 65558; i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('Geçerli zip değil (EOCD yok)')
  const count = buf.readUInt16LE(eocd + 10)
  let p = buf.readUInt32LE(eocd + 16)
  const out = []
  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error('Merkezi dizin bozuk')
    const method = buf.readUInt16LE(p + 10)
    const compSize = buf.readUInt32LE(p + 20)
    const nameLen = buf.readUInt16LE(p + 28)
    const extraLen = buf.readUInt16LE(p + 30)
    const commentLen = buf.readUInt16LE(p + 32)
    const localOff = buf.readUInt32LE(p + 42)
    const name = buf.toString('utf8', p + 46, p + 46 + nameLen)
    const lNameLen = buf.readUInt16LE(localOff + 26)
    const lExtraLen = buf.readUInt16LE(localOff + 28)
    const dataStart = localOff + 30 + lNameLen + lExtraLen
    const raw = buf.subarray(dataStart, dataStart + compSize)
    out.push([name, method === 8 ? inflateRawSync(raw) : raw])
    p += 46 + nameLen + extraLen + commentLen
  }
  return out
}

// ---------------------------------------------------------------------------
const DEFAULT_SETTINGS = {
  GeneralSettings: {
    SuggestedBots: 1,
    MaxCPM: 0,
    SaveEmptyCaptures: false,
    ContinueStatuses: [],
    ExitOnLaunch: false,
    ExitAfterEnd: false,
    ShuffleQueriesOnStart: false,
    RestrictFlowExecution: true,
    LogJobStatus: true,
    LogLevel: 'Info',
    LogToFile: false,
  },
  ProxySettings: {
    ProxyMode: 'Default',
    ProxySources: [],
    ProxyReloadIntervalMinutes: 0,
    BanKeys: ['ban', 'denied', 'banned', 'forbidden', 'not allowed', 'blacklisted'],
    RetryKeys: ['retry', 'try again', 'please try again', 'busy', 'slow down'],
  },
  InputSettings: { CustomInputs: [] },
  DataSettings: { ReloadAfter: 0, WordlistType: 'Default', DataRules: [] },
  BrowserSettings: { AutomationEngine: 'Puppeteer', Headless: true, CommandLineArgs: '--disable-blink-features=AutomationControlled', QuitOnClose: true },
  ScriptSettings: { DefaultLanguage: 'Jint' },
}

function defaultMetadata(name) {
  return {
    Name: name,
    Author: 'kor',
    Category: 'kor',
    CreationDate: new Date().toISOString(),
    LastModified: new Date().toISOString(),
    Version: '1.0',
    Readme: '',
    Base64Images: {},
  }
}

function pack(dir, outFile, name) {
  const entries = []
  const read = (f) => readFileSync(path.join(dir, f))
  entries.push(['readme.md', Buffer.from('', 'utf8')])
  entries.push(['metadata.json', read('metadata.json')])
  entries.push(['settings.json', read('settings.json')])
  entries.push(['script.loli', read('script.loli')])
  if (readdirSync(dir).includes('startup.loli')) entries.push(['startup.loli', read('startup.loli')])
  const buf = zipWrite(entries)
  writeFileSync(outFile, buf)
  return { entries: entries.length, bytes: buf.length, name: name ?? JSON.parse(read('metadata.json')).Name }
}

function unpack(file, outDir) {
  mkdirSync(outDir, { recursive: true })
  const entries = zipRead(readFileSync(file))
  for (const [name, data] of entries) {
    writeFileSync(path.join(outDir, name), data)
  }
  return entries.map(([n, d]) => `${n}\t${d.length}b`)
}

function validate(dir) {
  const problems = []
  const files = readdirSync(dir)
  for (const need of ['script.loli', 'settings.json', 'metadata.json']) {
    if (!files.includes(need)) problems.push(`eksik dosya: ${need}`)
  }
  if (files.includes('metadata.json')) {
    try {
      const m = JSON.parse(readFileSync(path.join(dir, 'metadata.json'), 'utf8'))
      if (!m.Name) problems.push('metadata.json içinde Name zorunlu')
    } catch (e) {
      problems.push('metadata.json geçersiz JSON: ' + e.message)
    }
  }
  if (files.includes('script.loli')) {
    const s = readFileSync(path.join(dir, 'script.loli'), 'utf8')
    if (s.includes('[SCRIPT]')) problems.push('script.loli OB1 LoliScript gibi görünüyor ([SCRIPT] bulundu): OB2 .loli içe aktarımı gerekir')
    if (/^\s*(REQUEST|KEYCHECK|FUNCTION|PARSE)\s/m.test(s)) problems.push('script.loli OB1 blok adları taşıyor (BLOCK: yok)')
  }
  return problems
}

// ---------------------------------------------------------------------------
const isMain = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1'))
if (isMain || process.argv[1]?.endsWith('opk.mjs')) {
  const [cmd, a, b] = process.argv.slice(2).filter((x) => !x.startsWith('--'))
  const nameIdx = process.argv.indexOf('--name')
  if (cmd === 'unpack' && a) {
    console.log(unpack(a, b ?? path.join(path.dirname(a), path.basename(a, '.opk'))).join('\n'))
  } else if (cmd === 'pack' && a) {
    const problems = validate(a)
    if (problems.length) {
      console.error('Paket sorunları:\n- ' + problems.join('\n- '))
      process.exit(1)
    }
    const out = b ?? a.replace(/[\\/]$/, '') + '.opk'
    const r = pack(a, out, nameIdx >= 0 ? process.argv[nameIdx + 1] : undefined)
    console.log(`Paketlendi: ${out} (${r.entries} giriş, ${(r.bytes / 1024).toFixed(1)} KB) — ${r.name}`)
  } else if (cmd === 'info' && a) {
    for (const [name, data] of zipRead(readFileSync(a))) console.log(`${name}\t${data.length}b`)
  } else {
    console.log('Kullanım:\n  node opk.mjs unpack x.opk [cikti-dizini]\n  node opk.mjs pack <dizin> [x.opk] [--name "Ad"]\n  node opk.mjs info x.opk')
    process.exit(1)
  }
}

export { pack, unpack, validate, zipRead, zipWrite, DEFAULT_SETTINGS, defaultMetadata }
