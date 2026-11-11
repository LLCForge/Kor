// Kenar çubuğu satırında oturumun rengi ve etiketleri (§28.6, ADR 0050). Kayıt `kor sessions tag|color` ile ortak.
import type { SessionMeta } from '../../ipc.ts'
import './prefs.css'

export function SessionTreeBadges({ session }: { session: SessionMeta }) {
  const extra = session as SessionMeta & { tags?: string[]; color?: string }
  return (
    <>
      {extra.color && <span className={`kor-dot ${extra.color}`} aria-hidden="true" />}{' '}
      {extra.tags?.slice(0, 3).map((t) => (
        <span key={t} className="kor-tag" translate="no">#{t}</span>
      ))}
      {extra.tags && extra.tags.length > 0 && ' '}
    </>
  )
}
