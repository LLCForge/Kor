// cdesktop "Notes" bölümü: sohbete bağlı serbest not; yazdıkça (800 ms) kaydedilir.
import { useEffect, useRef, useState } from 'react'
import useSWR from 'swr'
import { api, errorMessage } from './api.ts'

export function NotesPanel({ sessionId }: { sessionId: string | null }) {
  const key = sessionId ? ['notes', sessionId] : null
  const { data, error, mutate } = useSWR(key, () => api.notes.get(sessionId!), { revalidateOnFocus: false })
  const [text, setText] = useState<string | null>(null)
  const [saved, setSaved] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  const [saveError, setSaveError] = useState<string | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const value = text ?? data ?? ''

  useEffect(() => () => { if (timer.current) clearTimeout(timer.current) }, [])

  const change = (next: string) => {
    setText(next)
    setSaved('saving')
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      if (!sessionId) return
      void api.notes
        .set(sessionId, next)
        .then(() => {
          setSaved('saved')
          void mutate(next, { revalidate: false })
        })
        .catch((err: unknown) => {
          setSaved('error')
          setSaveError(errorMessage(err))
        })
    }, 800)
  }

  if (!sessionId) return <p className="muted pad">Not tutmak için önce bir sohbet başlat.</p>
  if (error) return <p className="error-text pad">{errorMessage(error)}</p>
  return (
    <div className="cdx-notes">
      <textarea
        className="cdx-notes-input"
        value={value}
        onChange={(e) => change(e.target.value)}
        placeholder="Bu sohbet hakkında not ekle…"
        aria-label="Sohbet notları"
        spellCheck={false}
      />
      <p className="cdx-notes-status" role="status">
        {saved === 'saving' ? 'Kaydediliyor…' : saved === 'saved' ? 'Kaydedildi' : saved === 'error' ? `Kaydedilemedi: ${saveError ?? ''}` : 'Notlar yalnızca bu bilgisayarda saklanır.'}
      </p>
    </div>
  )
}
