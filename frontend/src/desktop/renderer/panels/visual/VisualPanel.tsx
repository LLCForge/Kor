// "Görsel girdi" paneli (§10, ADR 0048): ekran görüntüsünden bileşen, diyagramdan iskelet (ikisi de
// dosya dosya onaydan sonra yazılır), LLM'siz mimari harita ve yerel konuşma tanıma bileşenleri.
// CLI: kor screenshot | kor diagram | kor visualize | kor voice.
import { Download, FileImage, Map as MapIcon, Network } from 'lucide-react'
import { useId, useState, type ChangeEvent } from 'react'
import useSWR from 'swr'
import type { Approval } from '../../../../core/agent.ts'
import type { FileDecisions } from '../../../ipc-review.ts'
import type { VisualResult, VisualTarget, VisualizeFormat, VisualizeResult } from '../../../ipc-visual.ts'
import { errorMessage } from '../../api.ts'
import { readImageFiles } from '../../g2/images.ts'
import { ReviewPanel } from '../../review/ReviewPanel.tsx'
import { kor } from '../providers/bridge.ts'
import type { KorPanelProps } from '../registry.ts'
import { visualApi } from './api.ts'
import { mb, toWrite } from './shape.ts'

type Mode = 'screenshot' | 'diagram'
type Note = { ok: boolean; text: string } | null

const TARGETS: readonly { id: VisualTarget; label: string }[] = [
  { id: 'react', label: 'React' },
  { id: 'svelte', label: 'Svelte' },
  { id: 'tauri', label: 'Tauri' },
  { id: 'html', label: 'HTML' },
]

function ImageImport({ onResult }: { onResult: (mode: Mode, r: VisualResult) => void }) {
  const uid = useId()
  const [mode, setMode] = useState<Mode>('screenshot')
  const [target, setTarget] = useState<VisualTarget>('react')
  const [image, setImage] = useState<{ name: string; dataUrl: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const pick = async (e: ChangeEvent<HTMLInputElement>) => {
    setError(null)
    const { images, errors } = await readImageFiles([...(e.target.files ?? [])], 1)
    if (errors.length > 0) setError(errors.join(' '))
    setImage(images[0] ?? null)
  }

  const run = async () => {
    if (!image) return
    setBusy(true)
    setError(null)
    try {
      const r = mode === 'screenshot' ? await visualApi.screenshot(image.dataUrl, image.name, target) : await visualApi.diagram(image.dataUrl, image.name)
      onResult(mode, r)
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h2 id={`${uid}-h`} className="ks-title">Görselden kod</h2>
      <div className="kv-row" role="radiogroup" aria-label="Görsel türü">
        <label className="kv-choice">
          <input type="radio" name={`${uid}-mode`} checked={mode === 'screenshot'} onChange={() => setMode('screenshot')} /> Arayüz ekran görüntüsü (Figma PNG dahil)
        </label>
        <label className="kv-choice">
          <input type="radio" name={`${uid}-mode`} checked={mode === 'diagram'} onChange={() => setMode('diagram')} /> Mimari diyagram
        </label>
      </div>
      <label className="kv-field">
        <span>Görsel (PNG/JPEG)</span>
        <input type="file" accept="image/png,image/jpeg" onChange={(e) => void pick(e)} />
      </label>
      {mode === 'screenshot' && (
        <label className="kv-field">
          <span>Hedef</span>
          <select value={target} onChange={(e) => setTarget(e.target.value as VisualTarget)}>
            {TARGETS.map((t) => <option key={t.id} value={t.id}>{t.label}</option>)}
          </select>
        </label>
      )}
      <div className="kv-row">
        <button type="button" className="btn primary" disabled={!image || busy} onClick={() => void run()} data-kor-cmd="visual.screenshot visual.diagram" aria-busy={busy}>
          {mode === 'screenshot' ? <FileImage size={14} aria-hidden="true" /> : <Network size={14} aria-hidden="true" />}{' '}
          {busy ? 'Model çalışıyor…' : mode === 'screenshot' ? 'Bileşenleri üret' : 'İskeleti üret'}
        </button>
      </div>
      <p className="ks-note">Seçili model görsel görebiliyorsa görüntü ona gider; göremiyorsa Kor görseli yerelde okur (OCR + satır/sütun yerleşimi + baskın renkler) ve bu metni gönderir. Hiçbir dosya onayın olmadan yazılmaz.</p>
      {error && <p className="error-text" role="alert">{error}</p>}
    </section>
  )
}

function Result({ mode, result, onDone }: { mode: Mode; result: VisualResult; onDone: (n: Note) => void }) {
  const [approved, setApproved] = useState(mode === 'screenshot')
  const [writing, setWriting] = useState(false)

  const answer = async (approval: Approval, decisions?: FileDecisions) => {
    const files = toWrite(result.files, approval, decisions)
    if (files.length === 0) {
      onDone({ ok: true, text: 'Hiçbir dosya yazılmadı.' })
      return
    }
    setWriting(true)
    try {
      const { written } = await visualApi.write(files)
      onDone({ ok: true, text: `${written.length} dosya yazıldı: ${written.join(', ')}` })
    } catch (err) {
      onDone({ ok: false, text: errorMessage(err) })
    } finally {
      setWriting(false)
    }
  }

  const tree = result.reply.split('```')[0]?.trim() ?? ''
  return (
    <section className="kg-card" aria-label="Önerilen dosyalar">
      <p className="ks-note">
        {result.vision ? `${result.provider} görüntünün kendisini gördü.` : `${result.provider} görsel göremiyor: görsel yerelde OCR ve yerleşim çıkarımıyla metne çevrilip gönderildi.`}
      </p>
      {result.description && (
        <details>
          <summary>Modele giden yerel açıklama</summary>
          <pre className="kv-pre" translate="no">{result.description}</pre>
        </details>
      )}
      {tree && <pre className="kv-pre" translate="no">{tree}</pre>}
      {!approved ? (
        <div className="kv-row">
          <p>{result.files.length} dosyalık iskelet hazır. Uygulayayım mı?</p>
          <button type="button" className="btn primary" onClick={() => setApproved(true)}>Evet, dosyaları göster</button>
          <button type="button" className="btn ghost" onClick={() => onDone({ ok: true, text: 'İskelet uygulanmadı.' })}>Hayır</button>
        </div>
      ) : (
        <div data-kor-cmd="visual.write" aria-busy={writing}>
          <ReviewPanel toolId={`visual-${mode}`} files={result.files} onAnswer={(a, f) => void answer(a, f)} />
        </div>
      )}
    </section>
  )
}

function Visualize() {
  const uid = useId()
  const [path, setPath] = useState('')
  const [format, setFormat] = useState<VisualizeFormat>('html')
  const [busy, setBusy] = useState(false)
  const [res, setRes] = useState<VisualizeResult | null>(null)
  const [error, setError] = useState<string | null>(null)

  const run = async () => {
    setBusy(true)
    setError(null)
    try {
      setRes(await visualApi.visualize(path.trim() || null, format))
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`}>
      <h2 id={`${uid}-h`} className="ks-title">Mimari harita</h2>
      <p className="ks-note">Proje grafından modelsiz üretilir: modül haritası, çağrı grafı ve veri akışı (tablolar, ortam değişkenleri).</p>
      <label className="kv-field">
        <span>Klasör (boşsa tüm proje)</span>
        <input type="text" value={path} onChange={(e) => setPath(e.target.value)} placeholder="src" translate="no" />
      </label>
      <label className="kv-field">
        <span>Biçim</span>
        <select value={format} onChange={(e) => setFormat(e.target.value as VisualizeFormat)}>
          <option value="html">HTML (tarayıcıda açılır)</option>
          <option value="mermaid">Mermaid</option>
          <option value="svg">SVG</option>
        </select>
      </label>
      <div className="kv-row">
        <button type="button" className="btn" disabled={busy} onClick={() => void run()} data-kor-cmd="visual.visualize" aria-busy={busy}>
          <MapIcon size={14} aria-hidden="true" /> {busy ? 'Graf okunuyor…' : 'Haritayı üret'}
        </button>
      </div>
      {error && <p className="error-text" role="alert">{error}</p>}
      {res && (
        <div role="status">
          <ul className="ks-list">
            {res.counts.map((c) => (
              <li key={c.view}>{c.title}: {c.nodes} düğüm, {c.edges} bağ{c.omitted > 0 ? ` (${c.omitted} düğüm sığmadı)` : ''}</li>
            ))}
          </ul>
          <p className="ks-note" translate="no">{res.main}</p>
        </div>
      )}
    </section>
  )
}

function Voice() {
  const uid = useId()
  const { data, error, mutate } = useSWR('visual:voiceStatus', () => visualApi.voiceStatus(), { revalidateOnFocus: false })
  const [busy, setBusy] = useState<string | null>(null)
  const [note, setNote] = useState<Note>(null)

  const install = async (name: string) => {
    setBusy(name)
    setNote(null)
    try {
      await kor.components.add(name)
      await mutate()
      setNote({ ok: true, text: `${name} kuruldu ve doğrulandı.` })
    } catch (err) {
      setNote({ ok: false, text: errorMessage(err) })
    } finally {
      setBusy(null)
    }
  }

  return (
    <section className="kg-card" aria-labelledby={`${uid}-h`} data-kor-cmd="visual.voiceStatus">
      <h2 id={`${uid}-h`} className="ks-title">Sesle görev</h2>
      <p className="ks-note">Mesaj kutusundaki mikrofon düğmesi kaydı yerel whisper.cpp ile metne çevirir; ses bilgisayardan çıkmaz.</p>
      {error ? (
        <p className="error-text" role="alert">{errorMessage(error)}</p>
      ) : !data ? (
        <p className="muted" role="status">Durum okunuyor…</p>
      ) : data.ready ? (
        <p role="status">Hazır: <span translate="no">{data.whisper}</span></p>
      ) : (
        <ul className="ks-list">
          {data.missing.map((m) => (
            <li key={m.name} className="kv-row">
              <span translate="no">{m.name} {m.version}</span>
              {m.size === null ? (
                <span className="muted">Bu makine için derleme yok; whisper-cli'yi PATH'e kur (macOS: brew install whisper-cpp).</span>
              ) : (
                <button type="button" className="btn small" disabled={busy !== null} onClick={() => void install(m.name)} data-kor-cmd="components.add">
                  <Download size={14} aria-hidden="true" /> {busy === m.name ? 'İndiriliyor…' : `İndir (${mb(m.size)})`}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {note && <p className={note.ok ? 'ks-note' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
    </section>
  )
}

export function VisualPanel(_props: KorPanelProps) {
  const [result, setResult] = useState<{ mode: Mode; r: VisualResult } | null>(null)
  const [note, setNote] = useState<Note>(null)
  return (
    <div className="ks">
      <ImageImport onResult={(mode, r) => { setNote(null); setResult({ mode, r }) }} />
      {result && <Result key={result.r.reply} mode={result.mode} result={result.r} onDone={(n) => { setNote(n); setResult(null) }} />}
      {note && <p className={note.ok ? 'ks-note' : 'error-text'} role={note.ok ? 'status' : 'alert'}>{note.text}</p>}
      <Visualize />
      <Voice />
    </div>
  )
}
