// Oturum replay görünümü (§15.9, ADR 0043): kayıtlı oturum listesi, zaman çizelgesi (oynat/durdur, adım
// ileri/geri, kaydırıcı), seçili olayın ayrıntısı ve o andaki diff (DiffView). Kayıtlar maskelidir.
// CLI: kor replay list | kor replay <id> | kor replay export <id>.
import { ArrowLeft, ChevronLeft, ChevronRight, FileDown, Pause, Play, RefreshCw } from 'lucide-react'
import { useEffect, useId, useState } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { DiffView } from '../../DiffView.tsx'
import type { KorPanelProps } from '../registry.ts'
import { replayApi } from './api.ts'
import { clamp, delayMs, detail, diffAt, diffFiles } from './timeline.ts'

const SPEEDS = [1, 2, 4, 8] as const

function List({ onOpen }: { onOpen: (id: string) => void }) {
  const { data, error, mutate } = useSWR('replay:list', () => replayApi.list(), { revalidateOnFocus: false })
  return (
    <>
      <div className="kg-card-head">
        <h2 className="ks-title">Oturum kayıtları</h2>
        <button type="button" className="action-btn" onClick={() => void mutate()} aria-label="Kayıt listesini yenile" title="Yenile" data-kor-cmd="replay.list">
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status" data-kor-cmd="replay.list">Kayıtlar okunuyor…</p>
      ) : data.length === 0 ? (
        <div className="ks-empty">
          <p>Bu projede kayıtlı oturum yok.</p>
          <p className="ks-note">Ayarlarda <code>[record] enabled = true</code> ya da tek oturum için <code>KOR_RECORD__ENABLED=true</code> ile açılır; kayıtlar <code>.kor/sessions/</code> altına sırlar maskelenerek yazılır.</p>
        </div>
      ) : (
        <ul className="ks-list" data-kor-cmd="replay.list">
          {data.map((s) => (
            <li key={s.id}>
              <button type="button" className="kg-link" onClick={() => onOpen(s.id)} data-kor-cmd="replay.load">
                <span translate="no">{s.title || s.id}</span>
                <span className="kg-link-meta">
                  {s.events} adım · {s.tools} araç · {Math.round(s.duration)} sn{s.costUsd > 0 ? ` · ${s.costEstimated ? '≈' : ''}$${s.costUsd.toFixed(4)}` : ''}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </>
  )
}

function Player({ id, onBack }: { id: string; onBack: () => void }) {
  const uid = useId()
  const { data, error } = useSWR(`replay:load:${id}`, () => replayApi.load(id), { revalidateOnFocus: false })
  const [pos, setPos] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [speed, setSpeed] = useState<number>(1)
  const [note, setNote] = useState<{ text: string; ok: boolean } | null>(null)
  const steps = data?.steps ?? []
  const len = steps.length

  // Oynatma: kayıttaki gerçek aralıklarla bir sonraki adım (uzun aralar kısaltılır).
  useEffect(() => {
    if (!playing) return
    if (pos >= len - 1) {
      setPlaying(false)
      return
    }
    const t = window.setTimeout(() => setPos((p) => clamp(p + 1, len)), delayMs(steps, pos, speed))
    return () => window.clearTimeout(t)
  }, [playing, pos, len, speed, steps])

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted" role="status" data-kor-cmd="replay.load">Kayıt yükleniyor…</p>

  const step = steps[pos]
  const event = step ? data.events[step.index] : undefined
  const files = diffFiles(diffAt(steps, data.events, pos))
  const go = (p: number) => {
    setPlaying(false)
    setPos(clamp(p, len))
  }
  const doExport = () => {
    setNote(null)
    replayApi
      .export(id)
      .then((r) => r.path && setNote({ text: `Dışa aktarıldı: ${r.path}`, ok: true }))
      .catch((e: unknown) => setNote({ text: errorMessage(e), ok: false }))
  }

  return (
    <>
      <div className="kg-card-head">
        <button type="button" className="action-btn" onClick={onBack} aria-label="Kayıt listesine dön" title="Listeye dön">
          <ArrowLeft size={16} aria-hidden="true" />
        </button>
        <h2 className="ks-title kr-id" translate="no">{id}</h2>
        <button type="button" className="action-btn" onClick={doExport} aria-label="Kaydı tek dosya olarak dışa aktar" title="Dışa aktar" data-kor-cmd="replay.export">
          <FileDown size={16} aria-hidden="true" />
        </button>
      </div>
      {note && <p className={note.ok ? 'ks-ok' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
      {len === 0 ? (
        <p className="ks-empty">Bu kayıtta olay yok.</p>
      ) : (
        <>
          <div className="kr-controls">
            <button type="button" className="action-btn" onClick={() => go(pos - 1)} disabled={pos === 0} aria-label="Önceki adım" title="Önceki adım">
              <ChevronLeft size={16} aria-hidden="true" />
            </button>
            <button
              type="button"
              className="action-btn"
              onClick={() => (pos >= len - 1 ? (setPos(0), setPlaying(true)) : setPlaying(!playing))}
              aria-label={playing ? 'Duraklat' : 'Oynat'}
              title={playing ? 'Duraklat' : 'Oynat'}
            >
              {playing ? <Pause size={16} aria-hidden="true" /> : <Play size={16} aria-hidden="true" />}
            </button>
            <button type="button" className="action-btn" onClick={() => go(pos + 1)} disabled={pos >= len - 1} aria-label="Sonraki adım" title="Sonraki adım">
              <ChevronRight size={16} aria-hidden="true" />
            </button>
            <label className="sr-only" htmlFor={`${uid}-scrub`}>Zaman çizelgesi</label>
            <input
              id={`${uid}-scrub`}
              className="kr-scrub"
              type="range"
              min={0}
              max={len - 1}
              value={pos}
              onChange={(e) => go(Number(e.target.value))}
              aria-valuetext={`Adım ${pos + 1} / ${len}`}
            />
            <span className="kr-pos">{pos + 1}/{len} · {(step?.t ?? 0).toFixed(1)} sn</span>
            <label className="sr-only" htmlFor={`${uid}-speed`}>Hız</label>
            <select id={`${uid}-speed`} className="g7-select" value={speed} onChange={(e) => setSpeed(Number(e.target.value))}>
              {SPEEDS.map((s) => <option key={s} value={s}>{s}×</option>)}
            </select>
          </div>
          <ol className="kr-steps" aria-label="Adımlar">
            {steps.map((s, i) => (
              <li key={s.index}>
                <button type="button" className="kg-link" aria-current={i === pos} onClick={() => go(i)}>
                  <span translate="no">{s.title}</span>
                  <span className="kg-link-meta">{s.t.toFixed(1)}</span>
                </button>
              </li>
            ))}
          </ol>
          <section className="kg-card" aria-labelledby={`${uid}-detail`}>
            <h3 id={`${uid}-detail`} className="kg-card-title ks-title">Bu adım</h3>
            <dl className="ks-facts">
              {detail(event).map((d) => (
                <div key={d.label} className="kr-fact">
                  <dt>{d.label}</dt>
                  <dd><pre className="kr-pre" translate="no">{d.text}</pre></dd>
                </div>
              ))}
            </dl>
          </section>
          <section className="kg-card" aria-labelledby={`${uid}-diff`}>
            <h3 id={`${uid}-diff`} className="kg-card-title ks-title">Bu andaki diff</h3>
            {files.length === 0 ? (
              <p className="ks-note">Bu adıma kadar dosya değişikliği yok.</p>
            ) : (
              files.map((f) => (
                <div key={f.path}>
                  {f.truncated && <p className="ks-warn">Dosya çok büyük olduğu için içeriği kaydedilmedi.</p>}
                  <DiffView change={{ path: f.path, before: f.before, after: f.after }} mode="unified" reviewable={false} />
                </div>
              ))
            )}
          </section>
        </>
      )}
    </>
  )
}

export function ReplayPanel(_props: KorPanelProps) {
  const [open, setOpen] = useState<string | null>(null)
  return <div className="ks kr">{open ? <Player id={open} onBack={() => setOpen(null)} /> : <List onOpen={setOpen} />}</div>
}
