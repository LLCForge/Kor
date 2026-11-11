// [g22] Mesaj kutusu: seçili modelin kabul ettiği dosya tipleri, eklerin nasıl gönderileceği ve model başına araç modu.
import { Check, FileCheck2 } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import type { G22FileSupport, G22ModelSupport as Support, ToolMode } from '../ipc-g22.ts'
import { api, errorMessage } from './api.ts'
import { usePopover } from './use-popover.ts'

const MODES: ReadonlyArray<{ id: ToolMode; label: string; desc: string }> = [
  { id: 'auto', label: 'Otomatik', desc: 'Model destekliyorsa yerel araç çağrısı; desteklemiyorsa metin protokolü.' },
  { id: 'native', label: 'Yerel', desc: 'Her zaman yerel function calling kullanılır.' },
  { id: 'text', label: 'Metin', desc: 'Araçlar <tool_call> metin protokolüyle sunulur (araç desteği olmayan modeller için).' },
  { id: 'off', label: 'Kapalı', desc: 'Araç yok; model yalnızca sohbet eder.' },
]

const PLAN_LABEL: Record<G22FileSupport['plan'], string> = { native: 'yerel', text: 'metne dönüştürülecek', unsupported: 'desteklenmiyor' }

function acceptedTypes(s: Support): Array<{ label: string; state: 'yes' | 'convert' | 'no' }> {
  return [
    { label: 'Metin, kod, CSV', state: 'yes' },
    { label: 'Görsel', state: s.caps.vision ? 'yes' : 'no' },
    { label: 'Ses', state: s.caps.audio ? 'yes' : 'no' },
    { label: s.caps.pdf ? 'PDF' : 'PDF → metin', state: s.caps.pdf ? 'yes' : 'convert' },
    { label: 'DOCX, XLSX → metin', state: 'convert' },
  ]
}

function toolsNote(s: Support): string {
  if (s.caps.tools === 'unknown') return 'Araç desteği bilinmiyor: önce yerel denenir, API reddederse metin protokolüne geçilir.'
  return s.caps.tools ? 'Model yerel araç çağrısını destekliyor.' : 'Model yerel araç çağrısını desteklemiyor; otomatik modda metin protokolü kullanılır.'
}

export function G22ModelSupport({ model, attachments }: { model: string; attachments: readonly string[] }) {
  const { open, setOpen, ref } = usePopover<HTMLDivElement>()
  const [saveError, setSaveError] = useState<string | null>(null)
  const { data, error, mutate } = useSWR(model ? ['g22:support', model, ...attachments] : null, () => api.g22Support(model, [...attachments]), {
    revalidateOnFocus: false,
  })
  if (!model) return null

  // Yalnızca gerçekten dönüştürülecek belgeler sayılır (düz metin zaten metin olarak gider).
  const converted = data?.files.filter((f) => f.plan === 'text' && f.kind !== 'other') ?? []
  const unsupported = data?.files.filter((f) => f.plan === 'unsupported') ?? []
  const mode = MODES.find((m) => m.id === (data?.toolMode ?? 'auto')) ?? MODES[0]

  const choose = async (id: ToolMode) => {
    setOpen(false)
    try {
      await api.g22SetToolMode(model, id)
      setSaveError(null)
      await mutate()
    } catch (err) {
      console.error('Araç modu kaydedilemedi:', err)
      setSaveError(`Araç modu kaydedilemedi: ${errorMessage(err)}`)
    }
  }

  const summary = [
    saveError ?? '',
    `Araç modu: ${mode?.label ?? 'Otomatik'}`,
    converted.length > 0 ? `${converted.length} ek metne dönüştürülecek` : '',
    unsupported.length > 0 ? `${unsupported.length} ek bu modelle gönderilemez` : '',
  ]
    .filter(Boolean)
    .join(' · ')

  return (
    <div className="popover-anchor" ref={ref}>
      <button
        type="button"
        className={`action-btn g22-trigger${unsupported.length > 0 ? ' warn' : mode?.id !== 'auto' ? ' active' : ''}`}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Model desteği. ${summary}`}
        title={error ? errorMessage(error) : summary}
      >
        <FileCheck2 size={15} aria-hidden="true" />
        {mode && mode.id !== 'auto' && <span className="g22-mode-name">{mode.label}</span>}
        {converted.length + unsupported.length > 0 && (
          <span className={`badge g22-count${unsupported.length > 0 ? ' warn' : ''}`}>{converted.length + unsupported.length}</span>
        )}
      </button>
      {open && (
        <div className="menu pop-in g22-menu" role="menu" aria-label="Model desteği">
          <p className="menu-heading">Bu modelin kabul ettiği ekler</p>
          {data ? (
            <ul className="g22-caps" aria-label="Dosya tipleri">
              {acceptedTypes(data).map((t) => (
                <li key={t.label} className={`g22-chip ${t.state}`}>
                  {t.label}
                  <span className="sr-only">{t.state === 'yes' ? ' (destekleniyor)' : t.state === 'no' ? ' (desteklenmiyor)' : ''}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="menu-note">{error ? errorMessage(error) : 'Yükleniyor…'}</p>
          )}
          {data && data.files.length > 0 && (
            <ul className="g22-files" aria-label="Eklerin gönderilişi">
              {data.files.map((f) => (
                <li key={f.path}>
                  <span className="g22-file-name" title={f.path}>
                    {f.path}
                  </span>
                  <span className={`badge g22-plan ${f.plan}`}>{f.kind === 'other' && f.plan === 'text' ? 'metin' : PLAN_LABEL[f.plan]}</span>
                </li>
              ))}
            </ul>
          )}
          <p className="menu-heading">Araç çağrısı</p>
          {MODES.map((m) => (
            <button
              key={m.id}
              type="button"
              role="menuitemradio"
              aria-checked={m.id === mode?.id}
              className="mode-item"
              onClick={() => void choose(m.id)}
            >
              <span className="mode-main">
                <span className="mode-label">{m.label}</span>
                <span className="mode-desc">{m.desc}</span>
              </span>
              <span className="mode-check">{m.id === mode?.id && <Check size={16} aria-hidden="true" />}</span>
            </button>
          ))}
          {data && <p className="menu-note">{toolsNote(data)} Seçim bu model için kalıcıdır.</p>}
          {saveError && (
            <p className="menu-note g22-error" role="alert">
              {saveError}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
