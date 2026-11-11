// Ayarlar > Genel > Etiketler (cdesktop Tags): tablo (Etiket adı / İçerik / İşlemler) + "Etiket ekle" ve
// "Etiket oluştur" diyaloğu (Etiket adı*, İçerik*). Mesaj kutusunda @ad yazınca içerik eklenir.
import { Pencil, Plus, Tag as TagIcon, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR from 'swr'
import type { Tag } from '../ipc-tags.ts'
import { api, errorMessage } from './api.ts'
import { confirmAction } from './ConfirmDialog.tsx'
import { Modal } from './NameDialog.tsx'

export const TAGS_KEY = 'tags:list'
const NAME_RE = /^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/

function TagDialog({ initial, existing, onSave, onClose }: { initial: Tag | null; existing: readonly Tag[]; onSave: (tag: Tag) => Promise<void>; onClose: () => void }) {
  const titleId = useId()
  const fid = useId()
  const [name, setName] = useState(initial?.name ?? '')
  const [content, setContent] = useState(initial?.content ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const nameOk = NAME_RE.test(name.trim())
  const duplicate = !initial && existing.some((t) => t.name.toLowerCase() === name.trim().toLowerCase())
  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (!nameOk || duplicate || !content.trim()) return
    setBusy(true)
    setError(null)
    try {
      await onSave({ name: name.trim(), content })
      onClose()
    } catch (err) {
      setError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Modal onClose={onClose} labelledBy={titleId}>
      <form className="dialog-body cdx-tag-form" onSubmit={(e) => void submit(e)}>
        <h2 id={titleId} className="dialog-title">{initial ? 'Etiketi düzenle' : 'Etiket oluştur'}</h2>
        <label htmlFor={`${fid}-n`}>Etiket adı <span className="cdx-req">*</span></label>
        <p className="g7-note">Mesajlarda @ ile kullanılır: @etiket_adi</p>
        <input id={`${fid}-n`} className="text-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="ör. bug_fix, test_plan, api_docs" disabled={!!initial} spellCheck={false} autoFocus={!initial} maxLength={64} aria-invalid={name !== '' && (!nameOk || duplicate)} />
        {name && !nameOk && <p className="error-text g9-small">Ad harf/rakamla başlamalı; yalnızca harf, rakam, _ ve -.</p>}
        {duplicate && <p className="error-text g9-small">Bu adda bir etiket zaten var.</p>}
        <label htmlFor={`${fid}-c`}>İçerik <span className="cdx-req">*</span></label>
        <p className="g7-note">Mesajda @{name || 'etiket_adi'} yazınca eklenecek metin</p>
        <textarea id={`${fid}-c`} className="text-input dialog-textarea" rows={7} value={content} onChange={(e) => setContent(e.target.value)} placeholder="Bu etiketi kullandığında eklenecek metni gir" autoFocus={!!initial} />
        {error && <p className="error-text" role="alert">{error}</p>}
        <div className="dialog-actions">
          <span />
          <div className="dialog-actions-right">
            <button type="button" className="btn" onClick={onClose} disabled={busy}>İptal</button>
            <button type="submit" className="btn primary" disabled={busy || !nameOk || duplicate || !content.trim()}>{initial ? 'Kaydet' : 'Oluştur'}</button>
          </div>
        </div>
      </form>
    </Modal>
  )
}

export function TagsSection() {
  const { data, error, mutate } = useSWR(TAGS_KEY, () => api.tags.list(), { revalidateOnFocus: false })
  const [dialog, setDialog] = useState<{ initial: Tag | null } | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const tags = data ?? []
  const persist = async (next: Tag[]) => {
    const saved = await api.tags.save(next)
    await mutate(saved, { revalidate: false })
  }
  const remove = (tag: Tag) =>
    void confirmAction({ title: 'Etiket silinsin mi?', message: `"@${tag.name}" etiketi kaldırılacak.`, confirmLabel: 'Sil', danger: true }).then((ok) => {
      if (ok) persist(tags.filter((t) => t.name !== tag.name)).catch((err: unknown) => setMessage(errorMessage(err)))
    })

  return (
    <section className="g7-subsection cdx-tags">
      <div className="cdx-tags-head">
        <div>
          <h4 className="g7-h4">Etiketler</h4>
          <p className="g7-note">Mesajlara @etiket_adi yazarak eklenen yeniden kullanılabilir metin parçacıkları.</p>
        </div>
        <button type="button" className="btn small" onClick={() => setDialog({ initial: null })}>
          <Plus size={14} aria-hidden="true" /> Etiket ekle
        </button>
      </div>
      {error && <p className="error-text">{errorMessage(error)}</p>}
      {message && <p className="error-text" role="status">{message}</p>}
      {tags.length === 0 ? (
        <p className="muted g9-small"><TagIcon size={13} aria-hidden="true" /> Henüz etiket yok.</p>
      ) : (
        <table className="cdx-tags-table">
          <thead>
            <tr><th>Etiket adı</th><th>İçerik</th><th className="cdx-tags-actions">İşlemler</th></tr>
          </thead>
          <tbody>
            {tags.map((t) => (
              <tr key={t.name}>
                <td className="g7-mono">@{t.name}</td>
                <td className="cdx-tags-content" title={t.content}>{String(t.content ?? '').replace(/\s+/g, ' ').slice(0, 90)}{String(t.content ?? '').length > 90 ? '…' : ''}</td>
                <td className="cdx-tags-actions">
                  <button type="button" className="action-btn" onClick={() => setDialog({ initial: t })} aria-label={`@${t.name} düzenle`} title="Düzenle"><Pencil size={13} aria-hidden="true" /></button>
                  <button type="button" className="action-btn" onClick={() => remove(t)} aria-label={`@${t.name} sil`} title="Sil"><Trash2 size={13} aria-hidden="true" /></button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {dialog && (
        <TagDialog
          initial={dialog.initial}
          existing={tags}
          onClose={() => setDialog(null)}
          onSave={(tag) => persist(dialog.initial ? tags.map((t) => (t.name === tag.name ? tag : t)) : [...tags, tag])}
        />
      )}
    </section>
  )
}
