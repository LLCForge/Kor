// [g20] Yerel Python çalıştırma (ana süreç; yalnızca kullanıcı onayından sonra çağrılır, bkz. handlers/g20.ts).
//      stdout/stderr ayrı akışlar olarak tutulur, zaman aşımı ve çıktı sınırı uygulanır; betiğin ürettiği görseller
//      ve üst düzey değişkenler kart için toplanır. Gizli anahtar alt sürecin ortamına aktarılmaz.
import { spawn } from 'node:child_process'
import { access, mkdtemp, readdir, readFile, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { StringDecoder } from 'node:string_decoder'
import { killProcessTree, whenChildDone } from './kill-tree.ts'
import { isSensitivePath } from './permissions.ts'
import { maskSecrets, scrubEnv } from './secrets.ts'

export interface G20Chunk {
  stream: 'out' | 'err'
  text: string
}

export interface G20VarInfo {
  name: string
  type: string
  preview: string
}

export interface G20Image {
  name: string
  /** data:image/...;base64,... (ana belge CSP'si img-src data: izinli; <img> içinde betik çalışmaz) */
  dataUrl: string
}

export interface G20PythonResult {
  chunks: G20Chunk[]
  exitCode: number | null
  timedOut: boolean
  truncated: boolean
  durationMs: number
  /** Kullanılan yorumlayıcı (proje sanal ortamı ya da sistem Python'u) */
  interpreter: string
  images: G20Image[]
  vars: G20VarInfo[]
}

export const PY_TIMEOUT_MS = 60_000
export const PY_MAX_OUTPUT = 200_000
const MAX_IMAGES = 6
const MAX_IMAGE_BYTES = 3_000_000
const IMAGE_MIME: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.svg': 'image/svg+xml',
}

export interface ProcessResult {
  chunks: G20Chunk[]
  exitCode: number | null
  timedOut: boolean
  truncated: boolean
  durationMs: number
}

export interface ProcessOptions {
  cwd: string
  stdin?: string
  env?: NodeJS.ProcessEnv
  timeoutMs: number
  maxOutput: number
  /** Her ham çıktı parçası için (kesme/çıktı sınırından bağımsız) — ör. git clone ilerleme ayrıştırma. */
  onChunk?: (chunk: G20Chunk) => void
  /** Kullanıcının Durdur'u; zaman aşımı gibi süreç ağacını öldürür. */
  signal?: AbortSignal
}

/** Kabuksuz süreç çalıştırır; akışları ayrı (sıralı) parçalar halinde toplar. Zaman aşımında süreç öldürülür. */
export function runProcess(command: string, args: string[], opts: ProcessOptions): Promise<ProcessResult> {
  const timeout = AbortSignal.timeout(opts.timeoutMs)
  const combined = opts.signal ? AbortSignal.any([opts.signal, timeout]) : timeout
  const started = performance.now()
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd: opts.cwd, env: opts.env, windowsHide: true, shell: false })
    const chunks: G20Chunk[] = []
    let used = 0
    let truncated = false
    const collect = (stream: G20Chunk['stream'], decoder: StringDecoder) => (data: Buffer) => {
      const text = decoder.write(data)
      if (!text) return
      opts.onChunk?.({ stream, text }) // çıktı sınırından önce: ilerleme tüm akış boyunca ayrıştırılabilsin
      if (used >= opts.maxOutput) {
        truncated = true
        return
      }
      const part = text.slice(0, opts.maxOutput - used)
      if (part.length < text.length) truncated = true
      used += part.length
      const last = chunks.at(-1)
      if (last && last.stream === stream) last.text += part
      else chunks.push({ stream, text: part })
    }
    child.stdout.on('data', collect('out', new StringDecoder('utf8')))
    child.stderr.on('data', collect('err', new StringDecoder('utf8')))
    child.stdin.on('error', (err) => console.warn('Python stdin yazılamadı:', err.message))
    child.stdin.end(opts.stdin ?? '')
    child.on('error', (err) => {
      reject((err as NodeJS.ErrnoException).code === 'ENOENT' ? new Error(`Yorumlayıcı bulunamadı: ${command}`) : err)
    })
    const armDeadline = whenChildDone(child, (exitCode) => {
      combined.removeEventListener('abort', onAbort)
      resolve({
        chunks: chunks.map((c) => ({ stream: c.stream, text: maskSecrets(c.text) })),
        exitCode,
        timedOut: timeout.aborted,
        truncated,
        durationMs: Math.round(performance.now() - started),
      })
    })
    function onAbort(): void {
      killProcessTree(child.pid)
      armDeadline()
    }
    if (combined.aborted) onAbort()
    else combined.addEventListener('abort', onAbort, { once: true })
  })
}

/** Proje sanal ortamındaki Python (.venv, venv, env) yoksa sistem Python'u. */
export async function findProjectPython(root: string, platform: NodeJS.Platform = process.platform): Promise<string> {
  const rel = platform === 'win32' ? ['Scripts', 'python.exe'] : ['bin', 'python']
  for (const dir of ['.venv', 'venv', 'env']) {
    const candidate = path.join(root, dir, ...rel)
    try {
      await access(candidate)
      return candidate
    } catch {
      // Bu ortam yok; sıradakine bakılır.
    }
  }
  return platform === 'win32' ? 'python' : 'python3'
}

// Kullanıcı kodu stdin'den okunur ve '<kod>' adıyla derlenir (satır numaraları kayık olmaz). Kod grafik çiziyorsa
// pyplot önceden yüklenir ve show() şekilleri G20_OUT_DIR'e kaydeder; çıkışta açık kalan şekiller ve üst düzey
// değişkenlerin özeti de yazılır. atexit kaydı pyplot'tan SONRA yapılır ki bizimki matplotlib şekilleri kapatmadan
// önce çalışsın (atexit LIFO).
export const PY_RUNNER = `import sys as _g20_sys, os as _g20_os, json as _g20_json, atexit as _g20_atexit, re as _g20_re
_g20_src = _g20_sys.stdin.read()
_g20_sys.stdin = open(_g20_os.devnull)
_g20_dir = _g20_os.environ.get('G20_OUT_DIR')
_g20_figs = [0]
def _g20_save_figures():
    plt = _g20_sys.modules.get('matplotlib.pyplot')
    if plt is None or not _g20_dir:
        return
    try:
        for i in plt.get_fignums():
            _g20_figs[0] += 1
            plt.figure(i).savefig(_g20_os.path.join(_g20_dir, 'figure-%d.png' % _g20_figs[0]), dpi=100, bbox_inches='tight')
        plt.close('all')
    except Exception as e:
        print('[g20] figure not saved: %s' % e, file=_g20_sys.stderr)
if _g20_dir and _g20_re.search(r'matplotlib|pylab|seaborn|\\.plot\\(', _g20_src):
    try:
        import matplotlib.pyplot as _g20_plt
        _g20_plt.show = lambda *a, **k: _g20_save_figures()
    except ImportError:
        pass  # matplotlib yoksa kullanicinin kendi import'u hatayi gosterir
def _g20_finish():
    if not _g20_dir:
        return
    _g20_save_figures()
    d = _g20_dir
    out = []
    for k, v in list(globals().items()):
        if k.startswith('_') or type(v).__name__ in ('module', 'function', 'builtin_function_or_method', 'type'):
            continue
        try:
            r = repr(v)
        except Exception as e:
            r = '<repr error: %s>' % e
        out.append({'name': k, 'type': type(v).__name__, 'preview': r[:300]})
    with open(_g20_os.path.join(d, 'vars.json'), 'w', encoding='utf-8') as f:
        _g20_json.dump(out[:50], f)
_g20_atexit.register(_g20_finish)
exec(compile(_g20_src, '<kod>', 'exec'))
`

/** Klasördeki görselleri (yalnız üst düzey) data: URL olarak toplar; since verilirse daha eski dosyalar atlanır. */
export async function collectImages(dir: string, since: number, root?: string): Promise<G20Image[]> {
  const images: G20Image[] = []
  let entries: string[]
  try {
    entries = await readdir(dir)
  } catch (err) {
    console.warn('Görsel klasörü okunamadı:', err instanceof Error ? err.message : err)
    return images
  }
  for (const name of entries.sort()) {
    const mime = IMAGE_MIME[path.extname(name).toLowerCase()]
    if (!mime || images.length >= MAX_IMAGES) continue
    if (root && isSensitivePath(path.relative(root, path.join(dir, name)))) continue
    const file = path.join(dir, name)
    const info = await stat(file).catch(() => null)
    if (!info?.isFile() || info.size > MAX_IMAGE_BYTES || info.mtimeMs < since) continue
    images.push({ name, dataUrl: `data:${mime};base64,${(await readFile(file)).toString('base64')}` })
  }
  return images
}

async function readVarDump(dir: string): Promise<G20VarInfo[]> {
  let raw: string
  try {
    raw = await readFile(path.join(dir, 'vars.json'), 'utf8')
  } catch {
    return [] // Süreç erken öldürüldüyse döküm yazılmamış olabilir.
  }
  let data: unknown
  try {
    data = JSON.parse(raw)
  } catch (err) {
    console.warn('Değişken dökümü ayrıştırılamadı:', err instanceof Error ? err.message : err)
    return []
  }
  if (!Array.isArray(data)) return []
  return data.slice(0, 50).flatMap((v: unknown) => {
    const item = v as { name?: unknown; type?: unknown; preview?: unknown } | null
    return item && typeof item.name === 'string' && typeof item.type === 'string' && typeof item.preview === 'string'
      ? [{ name: item.name.slice(0, 100), type: item.type.slice(0, 60), preview: maskSecrets(item.preview.slice(0, 300)) }]
      : []
  })
}

export async function runPython(code: string, root: string, timeoutMs = PY_TIMEOUT_MS, signal?: AbortSignal): Promise<G20PythonResult> {
  const interpreter = await findProjectPython(root)
  const outDir = await mkdtemp(path.join(tmpdir(), 'kor-py-'))
  try {
    // Dosya zaman damgası çözünürlüğü için küçük pay bırakılır.
    const started = Date.now() - 1000
    const result = await runProcess(interpreter, ['-u', '-c', PY_RUNNER], {
      cwd: root,
      stdin: code,
      env: { ...scrubEnv(process.env), PYTHONIOENCODING: 'utf-8', PYTHONUTF8: '1', MPLBACKEND: 'Agg', G20_OUT_DIR: outDir },
      timeoutMs,
      maxOutput: PY_MAX_OUTPUT,
      ...(signal ? { signal } : {}),
    })
    const images = [...(await collectImages(outDir, 0)), ...(await collectImages(root, started, root))].slice(0, MAX_IMAGES)
    return { ...result, interpreter, images, vars: await readVarDump(outDir) }
  } finally {
    await rm(outDir, { recursive: true, force: true }).catch((err: unknown) => console.warn('Geçici klasör silinemedi:', err))
  }
}
