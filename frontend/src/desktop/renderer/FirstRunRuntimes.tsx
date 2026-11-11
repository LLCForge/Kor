// [g27] İlk açılış kapısı: gerekli geliştirme araçları (Node, Python, PHP, Git, Rust) eksikse winget ile otomatik
// kurar, ilerlemeyi gösterir ve bittiğinde uygulamayı açar. Her araç kuruluysa anında uygulamaya geçer.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import type { InstallPhase } from '../../core/runtimes-g27.ts'
import { REQUIRED_RUNTIME_IDS, missingRequired } from '../../core/runtimes-g27.ts'
import { api, errorMessage } from './api.ts'
import { KorThinking } from './brand/brand.ts'
import { TitleBar } from './TitleBar.tsx'
import { percentText } from './ui-locale.ts'

interface Row {
  id: string
  name: string
  installed: boolean
  version: string | null
}
type Tick = { phase: InstallPhase; percent: number | null; line: string }
type Gate = 'checking' | 'install' | 'ready'

const REQUIRED = new Set<string>(REQUIRED_RUNTIME_IDS)
const PHASE: Record<InstallPhase, string> = {
  indiriliyor: 'İndiriliyor',
  kuruluyor: 'Kuruluyor',
  dogrulaniyor: 'Doğrulanıyor',
  bitti: 'Bitti',
  hata: 'Hata',
}

export function FirstRunRuntimes({ children }: { children: ReactNode }) {
  const [gate, setGate] = useState<Gate>('checking')
  const [rows, setRows] = useState<Row[]>([])
  const [progress, setProgress] = useState<Record<string, Tick>>({})
  const [note, setNote] = useState<string | null>(null)
  // true olunca artık gate'e dokunulmaz: kullanıcı "Atla"ya bastı ya da bileşen söküldü. Geç gelen yoklama kullanıcının seçimini ezmesin.
  const stop = useRef(false)

  useEffect(() => {
    stop.current = false
    const unsub = api.onRuntimeProgress((tick) => {
      setProgress((prev) => ({ ...prev, [tick.id]: { phase: tick.phase, percent: tick.percent, line: tick.line } }))
    })
    void (async () => {
      try {
        const list = await api.g27ListRuntimes()
        if (stop.current) return
        setRows(list.filter((row) => REQUIRED.has(row.id)))
        const missing = missingRequired(list)
        if (missing.length === 0) {
          setGate('ready')
          return
        }
        setGate('install')
        // winget yoksa otomatik kurulamaz (ör. Windows dışı): sayfa açmayız, elle kurulum mesajı gösterir ve kullanıcı atlayabilir.
        if (!list.some((row) => row.winget)) {
          setNote('winget bulunamadı; araçları elle kurup uygulamayı yeniden başlatın.')
          return
        }
        for (const id of missing) {
          if (stop.current) return
          try {
            await api.g27InstallRuntime(id)
          } catch (err) {
            if (!stop.current) setProgress((prev) => ({ ...prev, [id]: { phase: 'hata', percent: null, line: errorMessage(err) } }))
          }
        }
        const after = await api.g27ListRuntimes()
        if (stop.current) return
        setRows(after.filter((row) => REQUIRED.has(row.id)))
        if (missingRequired(after).length === 0) setGate('ready')
        else setNote('Bazı araçlar kurulamadı. "Yine de aç" ile devam edebilir veya Ayarlar > Araçlar’dan tekrar deneyebilirsiniz.')
      } catch (err) {
        if (!stop.current) {
          setNote(errorMessage(err))
          setGate('install')
        }
      }
    })()
    return () => {
      stop.current = true
      unsub()
    }
  }, [])

  if (gate === 'ready') return <>{children}</>

  return (
    <div className="g27-firstrun">
      <div className="g27-firstrun-drag">
        <TitleBar />
      </div>
      <main className="g27-firstrun-card">
        {/* §16.1 splash: kurulum sürerken kor küpü (KorThinking) */}
        <KorThinking size={64} label="Kor hazırlanıyor" />
        <h1 className="g27-firstrun-title text-balance">Kor hazırlanıyor</h1>
        <p className="g7-note">{gate === 'checking' ? 'Gerekli geliştirme araçları denetleniyor…' : 'Eksik araçlar kuruluyor. Windows yönetici onayı isteyebilir.'}</p>
        <div className="g7-stack">
          {rows.map((row) => {
            const tick = progress[row.id]
            const busy = !row.installed && (tick?.phase === 'indiriliyor' || tick?.phase === 'kuruluyor' || tick?.phase === 'dogrulaniyor')
            return (
              <div key={row.id} className="g27-runtime">
                <div className="g7-setting">
                  <div className="g7-setting-text">
                    <span className="g7-setting-title">{row.name}</span>
                    <span className="g7-note">{row.installed ? (row.version ?? 'Kurulu') : tick ? PHASE[tick.phase] : 'Bekliyor'}</span>
                  </div>
                  {row.installed ? <span className="g27-ok">Kurulu</span> : <span className="g7-note">{busy && tick?.percent !== null ? percentText(tick?.percent) : ''}</span>}
                </div>
                {busy && tick && (
                  <div className="g27-install">
                    <div className={`g27-bar${tick.percent === null ? ' g27-bar-wait' : ''}`} role="progressbar" aria-valuenow={tick.percent ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-label={`${row.name} kurulumu`}>
                      {tick.percent !== null && <span style={{ width: `${tick.percent}%` }} />}
                    </div>
                    <p className="g7-note">{tick.line}</p>
                  </div>
                )}
              </div>
            )
          })}
        </div>
        {note && <p className="error-text" role="alert">{note}</p>}
        <button type="button" className="btn" onClick={() => { stop.current = true; setGate('ready') }}>{gate === 'checking' ? 'Atla' : 'Yine de aç'}</button>
      </main>
    </div>
  )
}
