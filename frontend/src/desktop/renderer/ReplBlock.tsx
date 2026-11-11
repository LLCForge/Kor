// [g20] Satır içi REPL blokları: ```repl-js / ```repl-ts (izole worker, #123 #124), ```repl-py (ana süreçte onaylı
//      yerel Python, #125), ```repl-sql (onaylı, salt okunur SQLite, #126). Kod kartta düzenlenebilir; çıktı, süre ve
//      çıkış rozeti, değişkenler, grafik, temizleme (#140), çalıştırma geçmişi (#144) ve güvenli çalıştırma rozeti (#145).
import { Clock, Database, Eraser, LoaderCircle, Play, ShieldCheck, SquareTerminal } from 'lucide-react'
import { useMemo, useState, type KeyboardEvent } from 'react'
import { api, errorMessage } from './api.ts'
import { RunView } from './G20Output.tsx'
import { fromJs, fromPython, fromSql, recordRun, runHistory, type RunDraft, type RunLang } from './g20-runs.ts'
import { JS_MAX_OUTPUT, JS_TIMEOUT_MS, runJsInWorker } from './g20-sandbox.ts'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'
import { BlockFrame } from './rich-shared.tsx'
import { uiLocale } from './ui-locale.ts'

const LANGS: Record<string, RunLang> = {
  repl: 'js',
  'repl-js': 'js',
  'repl-javascript': 'js',
  'repl-ts': 'ts',
  'repl-typescript': 'ts',
  'repl-py': 'py',
  'repl-python': 'py',
  'repl-sql': 'sql',
}

const TITLES: Record<RunLang, string> = { js: 'JavaScript REPL', ts: 'TypeScript REPL', py: 'Python', sql: 'SQL (SQLite)' }

// Güvenli çalıştırma rozeti: kısa etiket + ayrıntılı açıklama (hangi izolasyon, hangi sınırlar).
const SAFETY: Record<RunLang, { label: string; detail: string }> = {
  js: {
    label: `İzole worker · ağ yok · ${JS_TIMEOUT_MS / 1000} sn`,
    detail: `Ayrı blob: Web Worker'da çalışır: DOM yok; ağ yok (CSP connect-src 'none' + fetch/XHR/WebSocket/EventSource/importScripts kaldırılır); eval yok; ${JS_TIMEOUT_MS / 1000} sn zaman aşımında worker sonlandırılır; çıktı ${JS_MAX_OUTPUT / 1000} KB ile sınırlı. Bellek sınırı tarayıcıda uygulanamaz, zaman aşımı kaçak ayırmayı keser.`,
  },
  ts: {
    label: `İzole worker · ağ yok · ${JS_TIMEOUT_MS / 1000} sn`,
    detail: `Tipler ana süreçte yalnız metin dönüşümüyle silinir (kod orada çalışmaz); ardından JavaScript gibi ayrı blob: worker'da, DOM ve ağ olmadan, ${JS_TIMEOUT_MS / 1000} sn ve ${JS_MAX_OUTPUT / 1000} KB sınırıyla çalışır.`,
  },
  py: {
    label: 'Yerel Python · onaylı · 60 sn',
    detail: 'Ana süreçte proje sanal ortamının (.venv/venv) ya da sistemin Python yorumlayıcısıyla, her çalıştırmada yerel onay penceresinden sonra çalışır. 60 sn zaman aşımı, 200 KB çıktı sınırı, denetim günlüğü. İzole DEĞİLDİR: dosya ve ağ erişimi vardır; API anahtarı ortam değişkenlerinden çıkarılır.',
  },
  sql: {
    label: 'SQLite · salt okunur · onaylı',
    detail: 'Yalnız çalışma dizinindeki SQLite dosyası, salt okunur bağlantıyla ve onaydan sonra. Yalnız SELECT/WITH/VALUES/EXPLAIN, tek ifade, 15 sn zaman aşımı, en çok 1000 satır.',
  },
}

/** ```repl-sql ilk satırında `-- db: yol` varsa veritabanı yolu oradan alınır. */
function dbHint(code: string): string {
  return /^--\s*db:\s*(\S.*)$/im.exec(code)?.[1]?.trim() ?? ''
}

function ReplBlock({ code, language }: RichBlockProps) {
  const lang = LANGS[language.trim().toLowerCase()] ?? 'js'
  const historyKey = `${lang}\n${code}`
  const [draft, setDraft] = useState<string | null>(null)
  const [db, setDb] = useState<string | null>(null)
  const [runs, setRuns] = useState(() => runHistory(historyKey))
  const [shownId, setShownId] = useState<number | null>(() => runHistory(historyKey)[0]?.id ?? null)
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null)
  const text = draft ?? code
  const dbPath = db ?? dbHint(code)
  const shown = useMemo(() => runs.find((r) => r.id === shownId) ?? null, [runs, shownId])
  const safety = SAFETY[lang]

  const execute = async (source: string): Promise<RunDraft | null> => {
    if (lang === 'js') return fromJs(await runJsInWorker(source))
    if (lang === 'ts') return fromJs(await runJsInWorker(await api.g20StripTs(source)))
    if (lang === 'py') {
      const result = await api.g20RunPython(source)
      return result && fromPython(result)
    }
    if (!dbPath) throw new Error('Veritabanı dosyasının yolunu girin (çalışma dizinine göre)')
    const result = await api.g20RunSql(dbPath, source)
    return result && fromSql(result)
  }

  const run = async () => {
    if (busy || !text.trim()) return
    setBusy(true)
    setMessage(null)
    try {
      const result = await execute(text)
      if (!result) {
        setMessage({ text: 'Çalıştırma iptal edildi.', error: false })
        return
      }
      const next = recordRun(historyKey, result)
      setRuns(next)
      setShownId(next[0]?.id ?? null)
    } catch (err) {
      setMessage({ text: errorMessage(err), error: true })
    } finally {
      setBusy(false)
    }
  }

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      void run()
    }
  }

  const tools = (
    <>
      <span className="g20-safety" title={safety.detail}>
        <ShieldCheck size={13} aria-hidden="true" /> {safety.label}
        <span className="sr-only">. {safety.detail}</span>
      </span>
      <button type="button" className="action-btn" onClick={() => void run()} disabled={busy} aria-label="Çalıştır (Ctrl+Enter)" title="Çalıştır (Ctrl+Enter)">
        {busy ? <LoaderCircle size={15} className="spin" aria-hidden="true" /> : <Play size={15} aria-hidden="true" />}
      </button>
      <button type="button" className="action-btn" onClick={() => setShownId(null)} disabled={!shown} aria-label="Çıktıyı temizle" title="Çıktıyı temizle">
        <Eraser size={15} aria-hidden="true" />
      </button>
    </>
  )

  return (
    <BlockFrame icon={lang === 'sql' ? <Database size={15} aria-hidden="true" /> : <SquareTerminal size={15} aria-hidden="true" />} label={TITLES[lang]} tools={tools} className="g20-repl">
      {lang === 'sql' && (
        <label className="g20-db">
          <span>Veritabanı</span>
          <input
            type="text"
            value={dbPath}
            onChange={(e) => setDb(e.target.value)}
            placeholder="ör. data/app.sqlite"
            spellCheck={false}
            maxLength={1024}
          />
        </label>
      )}
      <textarea
        className="g20-code"
        value={text}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={onKeyDown}
        rows={Math.min(20, Math.max(3, text.split('\n').length))}
        spellCheck={false}
        aria-label={`${TITLES[lang]} kodu`}
      />
      {message && (
        <p className={message.error ? 'g20-note error-text' : 'g20-note'} role={message.error ? 'alert' : 'status'}>
          {message.text}
        </p>
      )}
      {shown && <RunView record={shown} />}
      {runs.length > 1 && (
        <details className="g20-history">
          <summary>
            <Clock size={13} aria-hidden="true" /> Çalıştırma geçmişi ({runs.length})
          </summary>
          <ul>
            {runs.map((r) => (
              <li key={r.id}>
                <button type="button" className={`g20-history-item${r.id === shownId ? ' g20-current' : ''}`} aria-current={r.id === shownId} onClick={() => setShownId(r.id)}>
                  {new Date(r.at).toLocaleTimeString(uiLocale())} · {r.status === 'ok' ? 'başarılı' : r.status === 'timeout' ? 'zaman aşımı' : 'hata'} · {r.durationMs} ms
                </button>
              </li>
            ))}
          </ul>
        </details>
      )}
    </BlockFrame>
  )
}

// Açık kayıt: zengin blok kataloğu testi (test/g22-attachments.test.ts) dilleri bu çağrılardan okur; modeller ancak
// katalogda olan blokları öğrenir. LANGS'e dil eklenirse buraya ve src/core/rich-block-catalog.ts'e de eklenmeli.
registerRichBlock('repl', ReplBlock)
registerRichBlock('repl-js', ReplBlock)
registerRichBlock('repl-javascript', ReplBlock)
registerRichBlock('repl-ts', ReplBlock)
registerRichBlock('repl-typescript', ReplBlock)
registerRichBlock('repl-py', ReplBlock)
registerRichBlock('repl-python', ReplBlock)
registerRichBlock('repl-sql', ReplBlock)
