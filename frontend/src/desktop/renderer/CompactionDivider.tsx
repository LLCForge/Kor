import { ChevronsDownUp, ChevronsUpDown, Pencil } from 'lucide-react'
import { useEffect, useState } from 'react'
import type { ChatItem } from './chat-state.ts'
import { formatTokens } from './g6-format.ts'
import { Markdown } from './Markdown.tsx'
import { uiLocale } from './ui-locale.ts'

type CompactionItem = Extract<ChatItem, { kind: 'compaction' }>

interface Props {
  item: CompactionItem
  /** Bu ayracın üstünde katlanan blok sayısı; 0 ise katlanmış bir şey yok. */
  hiddenCount?: number
  /** Bloklar açık mı (düğme "gizle" olur) */
  revealed?: boolean
  /** Katlanmış blokları akışa geri koyar / çıkarır (DOM'a ancak açıkken girerler). */
  onToggle?: () => void
}

/**
 * Akıştaki "Bağlam sıkıştırıldı" ayracı; açılınca modelin devam ettiği özet görünür.
 * Üstündeki eski bloklar DOM'dan çıkarıldığı için ayrıca "N eski blok" düğmesi taşır.
 * Özet başlığı zaman, model ve token sayılarını taşır; kullanıcı hangi sıkıştırmanın ne zaman olduğunu görür.
 */
export function CompactionDivider({ item, hiddenCount = 0, revealed = false, onToggle }: Props) {
  const label =
    item.beforeTokens !== undefined && item.afterTokens !== undefined
      ? `Bağlam sıkıştırıldı · ${formatTokens(item.beforeTokens)} → ${formatTokens(item.afterTokens)} token`
      : 'Önceki kısım özetlendi'
  const meta = [item.at ? new Date(item.at).toLocaleTimeString(uiLocale(), { hour: '2-digit', minute: '2-digit' }) : null, item.model ?? null].filter(Boolean).join(' · ')
  return (
    <div className="message g6-compaction" id={`g6-fold-${item.id}`}>
      {item.running && <p className="g6-compaction-running">Geçmiş özetleniyor…</p>}
      {onToggle && !item.running && (hiddenCount > 0 || revealed) && (
        <button type="button" className="g6-compaction-reveal" onClick={onToggle} aria-expanded={revealed}>
          <ChevronsUpDown size={13} aria-hidden="true" />
          {revealed ? 'Eski blokları gizle' : `${hiddenCount} eski bloğu göster`}
        </button>
      )}
      <details className="g6-compaction-details">
        <summary className="g6-compaction-line">
          <ChevronsDownUp size={14} aria-hidden="true" />
          <span>{label}</span>
          {meta && <span className="g6-compaction-meta">{meta}</span>}
        </summary>
        <div className="prose g6-compaction-summary">
          {item.text === undefined ? (
            <Markdown text={item.summary} />
          ) : (
            <SummaryEditor itemId={item.id} text={item.text} fallback={item.summary} onSave={item.onSave} />
          )}
        </div>
      </details>
    </div>
  )
}

/** Özet gövdesini düzenleme: kullanıcı modelin yanlış anladığı noktayı düzeltebilsin. */
function SummaryEditor({
  itemId,
  text,
  fallback,
  onSave,
}: {
  itemId: string
  text: string
  fallback: string
  onSave?: (body: string) => Promise<boolean> | boolean
}) {
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState(text)
  const [saving, setSaving] = useState(false)
  useEffect(() => setDraft(text), [text])
  if (!onSave) return <Markdown text={fallback} />
  if (!editing) {
    return (
      <>
        <Markdown text={text} />
        <button type="button" className="g6-summary-edit" onClick={() => setEditing(true)} aria-label={`Özeti düzenle (${itemId})`}>
          <Pencil size={12} aria-hidden="true" /> Özeti düzenle
        </button>
      </>
    )
  }
  return (
    <div className="g6-summary-editor">
      <textarea value={draft} onChange={(e) => setDraft(e.target.value)} spellCheck={false} rows={Math.min(24, draft.split('\n').length + 2)} />
      <div className="g6-summary-editor-actions">
        <button
          type="button"
          className="btn small"
          disabled={saving}
          onClick={async () => {
            setSaving(true)
            try {
              if ((await onSave(draft)) !== false) setEditing(false)
            } finally {
              setSaving(false)
            }
          }}
        >
          {saving ? 'Kaydediliyor…' : 'Kaydet'}
        </button>
        <button type="button" className="btn small ghost" onClick={() => { setDraft(text); setEditing(false) }}>
          Vazgeç
        </button>
      </div>
    </div>
  )
}
