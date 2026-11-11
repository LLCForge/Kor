// Satır diff'i (onay kartı ve sayfa → kod önizlemesi): eklenen/silinen satırlar, en çok 400 satır.
import { diffLines } from 'diff'
import { t } from './i18n.ts'

export interface FileChange {
  path: string
  before: string | null
  after: string | null
}

export function DiffView({ file }: { file: FileChange }) {
  const parts = diffLines(file.before ?? '', file.after ?? '')
  const rows: Array<{ kind: 'add' | 'del' | 'ctx'; text: string }> = []
  for (const p of parts) {
    const kind = p.added ? 'add' : p.removed ? 'del' : 'ctx'
    const lines = p.value.replace(/\n$/, '').split('\n')
    const shown = kind === 'ctx' && lines.length > 6 ? [...lines.slice(0, 3), '…', ...lines.slice(-3)] : lines
    for (const text of shown) rows.push({ kind, text })
  }
  const cut = rows.length > 400
  return (
    <div className="diff" translate="no">
      <div className="diff-path">
        {file.path} {file.before === null ? <span className="tag">{t('yeni dosya')}</span> : null}
        {file.after === null ? <span className="tag warn">{t('silinir')}</span> : null}
      </div>
      <pre>
        {rows.slice(0, 400).map((r, i) => (
          <div key={i} className={`dl ${r.kind}`}>
            {r.kind === 'add' ? '+ ' : r.kind === 'del' ? '- ' : '  '}
            {r.text}
          </div>
        ))}
        {cut ? <div className="dl ctx">{t('… (kısaltıldı)')}</div> : null}
      </pre>
    </div>
  )
}
