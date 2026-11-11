import { Plus, Trash2 } from 'lucide-react'
import { useId, useState, type FormEvent } from 'react'
import useSWR, { useSWRConfig } from 'swr'
import type { PermissionMode } from '../../core/permissions.ts'
import { api, errorMessage } from './api.ts'
import { trustKey } from './g7-bus.ts'
import { MODES } from './ModeMenu.tsx'
import { displayPath } from './display-path.ts'

// permissions.ts'teki RULE_PATTERN ile aynı; o modül node:path içerdiği için arayüz paketine alınamaz. Asıl doğrulama ana süreçte.
const RULE_PATTERN = /^([a-z_]+)(?:\((.+)\))?$/

const EXAMPLES: ReadonlyArray<{ rule: string; desc: string }> = [
  { rule: 'bash(npm test*)', desc: 'npm test ile başlayan komutlar' },
  { rule: 'bash(git status)', desc: 'yalnızca git status' },
  { rule: 'edit_file(src/**)', desc: 'src altındaki dosyaları düzenleme' },
  { rule: 'write_file(docs/*)', desc: 'docs klasörüne dosya yazma' },
  { rule: 'bash(rm *)', desc: 'deny için: rm ile başlayan komutlar' },
]

/** Kuralın araç adı; desensiz kural o aracın tüm kullanımlarını kapsar. */
function ruleWarning(rule: string, list: 'allow' | 'deny', tools: ReadonlyArray<{ name: string; kind: string }>): string | null {
  const m = /^([a-z_]+)(\((.*)\))?$/.exec(rule.trim())
  if (!m) return null
  const tool = tools.find((t) => t.name === m[1])
  if (!tool) return `Bilinmeyen araç: ${m[1]}`
  if (list === 'allow' && (m[3] === undefined || m[3] === '*') && tool.kind === 'exec') return `Bu kural ${tool.name} aracının TÜM kullanımlarına onaysız izin verir.`
  if (list === 'allow' && tool.kind === 'read') return `${tool.name} zaten salt okunur; izin kuralına gerek yok.`
  return null
}

/** Açık klasörün güven kararı (ilk açılıştaki soru buradan değiştirilir). */
function TrustRow({ root }: { root: string }) {
  const { mutate } = useSWRConfig()
  const { data, error } = useSWR(trustKey(root), () => api.g7GetTrust())
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const set = async (trusted: boolean) => {
    setBusy(true)
    setActionError(null)
    try {
      await api.g7SetTrust(trusted)
      await mutate(trustKey(root))
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return null
  return (
    <div className="g7-setting">
      <div className="g7-setting-text">
        <span className="g7-setting-title">Açık klasöre güven</span>
        <span className="g7-note">
          <span className="g7-mono g7-path">{displayPath(data.root)}</span> ·{' '}
          {data.trusted ? 'Güvenilir: proje talimatları ve ajanları yüklenir.' : "Güvenilmiyor: proje talimatları yüklenmez, mod Manuel'e sabit."}
          {data.projectFiles.length === 0 && ' Proje dosyası yok.'}
        </span>
        {actionError && <span className="error-text">{actionError}</span>}
      </div>
      <div className="g7-setting-control">
        <button type="button" className="btn small" onClick={() => void set(!data.trusted)} disabled={busy}>
          {data.trusted ? 'Güvenmeyi bırak' : 'Güven'}
        </button>
      </div>
    </div>
  )
}

export function PermissionRules({ root }: { root: string | null }) {
  const baseId = useId()
  const { mutate } = useSWRConfig()
  const { data, error, mutate: refresh } = useSWR('g7:perm', () => api.g7GetPermissionRules())
  const [list, setList] = useState<'allow' | 'deny'>('allow')
  const [rule, setRule] = useState('')
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const syntaxOk = rule.trim().length <= 500 && RULE_PATTERN.test(rule.trim())
  const warning = rule.trim() && data ? (syntaxOk ? ruleWarning(rule, list, data.tools) : 'Biçim: arac veya arac(desen); * her şeyle eşleşir.') : null

  const run = async (fn: () => Promise<void>) => {
    setBusy(true)
    setActionError(null)
    try {
      await fn()
      await refresh()
    } catch (err) {
      setActionError(errorMessage(err))
    } finally {
      setBusy(false)
    }
  }

  const add = (e: FormEvent) => {
    e.preventDefault()
    if (!syntaxOk) return
    void run(async () => {
      await api.g7AddPermissionRule(list, rule.trim())
      setRule('')
    })
  }

  const setDefaultMode = (mode: PermissionMode) =>
    void run(async () => {
      await api.setDefaultPermissionMode(mode)
      await mutate('state')
    })

  if (error) return <p className="error-text" role="alert">{errorMessage(error)}</p>
  if (!data) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g7-stack">
      {root && <TrustRow root={root} />}
      <div className="g7-field">
        <label htmlFor={`${baseId}-mode`} className="g7-label">Varsayılan izin modu</label>
        <select id={`${baseId}-mode`} className="g7-select" value={data.defaultMode} onChange={(e) => setDefaultMode(e.target.value as PermissionMode)} disabled={busy}>
          {MODES.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}: {m.desc}
            </option>
          ))}
        </select>
        <p className="g7-note">Yeni klasör açıldığında bu mod kullanılır. Güvenilmeyen klasörlerde mod her zaman Manuel'dir.</p>
      </div>

      {(['allow', 'deny'] as const).map((kind) => {
        const rules = kind === 'allow' ? data.allow : data.deny
        return (
          <section key={kind} className="g7-rule-group" aria-labelledby={`${baseId}-${kind}`}>
            <h4 id={`${baseId}-${kind}`} className="g7-h4">
              {kind === 'allow' ? 'İzin ver (allow)' : 'Engelle (deny)'}
              <span className="g7-note">
                {kind === 'allow' ? 'Eşleşen işlemler sorulmadan çalışır (tehlikeli komutlar yine sorulur).' : 'Eşleşen işlemler her modda engellenir.'}
              </span>
            </h4>
            {rules.length === 0 ? (
              <p className="g7-note">Kural yok.</p>
            ) : (
              <ul className="g7-rule-list">
                {rules.map((r) => (
                  <li key={r} className="g7-rule">
                    <code>{r}</code>
                    <button type="button" className="action-btn" onClick={() => void run(() => api.g7RemovePermissionRule(kind, r))} disabled={busy} aria-label={`${r} kuralını sil`} title="Sil">
                      <Trash2 size={14} aria-hidden="true" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {kind === 'deny' && data.projectDeny.length > 0 && (
              <p className="g7-note">
                Proje dosyalarından gelen deny kuralları (salt okunur): <code>{data.projectDeny.join(', ')}</code>
              </p>
            )}
          </section>
        )
      })}

      <form className="g7-rule-form" onSubmit={add}>
        <label htmlFor={`${baseId}-list`} className="sr-only">Kural türü</label>
        <select id={`${baseId}-list`} className="g7-select" value={list} onChange={(e) => setList(e.target.value === 'deny' ? 'deny' : 'allow')}>
          <option value="allow">İzin ver</option>
          <option value="deny">Engelle</option>
        </select>
        <label htmlFor={`${baseId}-rule`} className="sr-only">Kural</label>
        <input
          id={`${baseId}-rule`}
          className="text-input g7-grow"
          value={rule}
          onChange={(e) => setRule(e.target.value)}
          placeholder="bash(npm test*)"
          maxLength={500}
          spellCheck={false}
          aria-describedby={`${baseId}-hint`}
          aria-invalid={rule.trim() !== '' && !syntaxOk}
        />
        <button type="submit" className="btn primary" disabled={busy || !syntaxOk}>
          <Plus size={14} aria-hidden="true" /> Ekle
        </button>
      </form>
      <p id={`${baseId}-hint`} className={`g7-note${warning ? ' g7-warn-text' : ''}`} role={warning ? 'status' : undefined}>
        {warning ?? 'Kurallar ~/.kor/settings.json dosyasına yazılır; diğer ayarlar korunur.'}
      </p>
      {actionError && <p className="error-text" role="alert">{actionError}</p>}

      <details className="g7-details">
        <summary>Örnekler ve araç listesi</summary>
        <ul className="g7-examples">
          {EXAMPLES.map((ex) => (
            <li key={ex.rule}>
              <button type="button" className="g7-inline-link" onClick={() => setRule(ex.rule)}>
                <code>{ex.rule}</code>
              </button>{' '}
              <span className="g7-note">{ex.desc}</span>
            </li>
          ))}
        </ul>
        <p className="g7-note">
          Araçlar: {data.tools.map((t) => `${t.name} (${t.kind === 'read' ? 'okuma' : t.kind === 'edit' ? 'düzenleme' : 'komut'})`).join(', ')}
        </p>
        <p className="g7-note">Dosya: {data.userFile}</p>
      </details>
    </div>
  )
}
