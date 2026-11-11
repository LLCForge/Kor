// §29.2 hosting paneli bağlamı (ADR 0054'teki davranış, React'e taşındı): Kor masaüstü sayfayı tanır
// (`kor_web::site::panels`), komutları sunar; API komutları önce plan + etki tablosu, onaydan sonra uygulama ve
// doğrulama; form doldurmada parola alanlarına yazılmaz, gönderen kullanıcıdır; her adım ekran görüntüsüyle kayıtta.
import { useEffect, useState } from 'react'
import { call } from './bus.ts'
import { t } from './i18n.ts'

const FIELD_LABELS: Record<string, string> = {
  domain: 'Alan adı', version: 'PHP sürümü (ör. 8.3)', sub: 'Alt ad', name: 'Ad', user: 'Kullanıcı', address: 'E-posta adresi',
  schedule: 'Zaman (*/15 * * * *)', command: 'Komut', path: 'Klasör', content: 'İçerik / IPv4', ttl: 'TTL',
  ns1: 'Ad sunucusu 1', ns2: 'Ad sunucusu 2', ns3: 'Ad sunucusu 3', ns4: 'Ad sunucusu 4',
}

export interface PanelInfo {
  id: string
  name: string
  kind?: string
  profile?: string
  commands: Array<{ id: string; label: string; action: string; needs?: string[]; fields?: string[] }>
}

interface Step {
  at: number
  label: string
  shot: string
}

interface RunResult {
  error?: string
  notes?: string[]
  none?: string
  needsApproval?: boolean
  plan?: Array<{ step: string; target: string; impact: string; reversible: boolean }>
  markdown?: string
  applied?: string
  verify?: { ok: boolean; url: string; status: number }
  rolled_back?: boolean
  credential?: string
  output?: string[]
  fields?: Array<{ labels: string[]; value: string }>
}

const panel = (body: Record<string, unknown>) => call<RunResult>('panel', { body })

export async function detect(): Promise<PanelInfo | null> {
  const tab = await call<{ url: string; title: string }>('activeTab').catch(() => null)
  if (!tab || !/^https?:/.test(tab.url)) return null
  const r = (await panel({ op: 'detect', url: tab.url, title: tab.title }).catch(() => null)) as { panel?: PanelInfo | null } | null
  return r?.panel ?? null
}

export function Hosting({ info, steps }: { info: PanelInfo; steps: Step[] }) {
  const [form, setForm] = useState<{ keys: string[]; submit: string; go: (v: Record<string, string>) => void } | null>(null)
  const [values, setValues] = useState<Record<string, string>>({})
  const [result, setResult] = useState<RunResult | null>(null)
  const [pending, setPending] = useState<{ command: string; values: Record<string, string> } | null>(null)
  const [busy, setBusy] = useState('')
  useEffect(() => {
    setResult(null)
    setForm(null)
  }, [info.id])

  const ask = (keys: string[], submit: string, go: (v: Record<string, string>) => void) => {
    setValues({})
    setForm({ keys, submit, go })
  }
  const done = (r: RunResult) => {
    setBusy('')
    setResult(r)
  }
  const run = (cmd: PanelInfo['commands'][number]) => {
    setResult(null)
    if (cmd.action === 'explain') {
      setBusy(t('Ekran okunuyor…'))
      void call<{ url: string; title: string; text: string }>('pageText')
        .then((p) => panel({ op: 'explain', url: p.url, title: p.title, text: p.text }))
        .then(done, (e: Error) => done({ error: e.message }))
      return
    }
    if (cmd.action === 'fill') {
      ask(cmd.fields ?? [], t('Formu doldur'), (v) => {
        setBusy(t('Dolduruluyor…'))
        void panel({ op: 'fill', panel: info.id, command: cmd.id, values: v })
          .then((r): Promise<RunResult> | RunResult => (r.error ? r : call<RunResult>('panelFill', { fields: r.fields })))
          .then(done, (e: Error) => done({ error: e.message }))
      })
      return
    }
    if (!info.profile) {
      done({ error: t('Bu panel için bağlı bir Kor sunucu profili yok. Masaüstünde Web siteleri > Sunucu bağla ya da: kor host connect') })
      return
    }
    const plan = (v: Record<string, string>) => {
      setBusy(t('Envanter okunuyor…'))
      setPending({ command: cmd.id, values: v })
      void panel({ op: 'run', panel: info.id, command: cmd.id, profile: info.profile, values: v, approve: false }).then(done, (e: Error) => done({ error: e.message }))
    }
    if (cmd.needs?.length) ask(cmd.needs, t('Planı göster'), plan)
    else plan({})
  }
  const approve = () => {
    if (!pending) return
    setBusy(t('Uygulanıyor ve doğrulanıyor…'))
    void panel({ op: 'run', panel: info.id, command: pending.command, profile: info.profile, values: pending.values, approve: true })
      .then(async (r) => {
        if (!r.error) await call('recordStep', { label: `${t('Uygulandı')}: ${pending.command}${r.applied ? ` — ${r.applied}` : ''}` })
        return r
      })
      .then(done, (e: Error) => done({ error: e.message }))
  }

  const r = result as (RunResult & { filled?: string[]; missing?: string[] }) | null
  return (
    <section className="hosting" aria-label={t('Panel komutları')}>
      <div className="row">
        <strong translate="no">{info.name}</strong>
        <span className="note grow">{info.profile ? `${t('Kor profili')}: ${info.profile}` : info.kind ? t('Kor profili yok') : ''}</span>
      </div>
      <div className="cmds">
        {info.commands.map((c) => (
          <button key={c.id} type="button" onClick={() => run(c)} data-kor-cmd="panel.run">
            {c.label}
          </button>
        ))}
      </div>
      {form ? (
        <div className="pform">
          {form.keys.map((k) => (
            <label key={k}>
              {t(FIELD_LABELS[k] ?? k)}
              <input type="text" autoComplete="off" value={values[k] ?? ''} onChange={(e) => setValues({ ...values, [k]: e.target.value })} />
            </label>
          ))}
          <div className="row">
            <button
              type="button"
              className="primary"
              onClick={() => {
                const v = Object.fromEntries(Object.entries(values).filter(([, x]) => x.trim()).map(([k, x]) => [k, x.trim()]))
                setForm(null)
                form.go(v)
              }}
            >
              {form.submit}
            </button>
            <button type="button" onClick={() => setForm(null)}>
              {t('Vazgeç')}
            </button>
          </div>
        </div>
      ) : null}
      <div className="presult" role="status">
        {busy ? <div className="note">{busy}</div> : null}
        {r?.error ? <div className="err">{r.error}</div> : null}
        {r?.notes ? (r.notes.length ? r.notes.map((n, i) => <div key={i} className="tool">{n}</div>) : <div className="note">{r.none}</div>) : null}
        {r?.filled ? (
          <>
            <div className="tool">{t('Dolduruldu')}: {r.filled.join(', ') || '—'}</div>
            {r.missing?.length ? <div className="warn">{t('Bulunamadı')}: {r.missing.join(', ')}</div> : null}
            <div className="note">{t('Formu kontrol edip gönder düğmesine siz basın.')}</div>
          </>
        ) : null}
        {r?.needsApproval ? (
          <div className="confirm">
            <div className="q">{t('Plan ve etki')}</div>
            <table>
              <thead>
                <tr>
                  <th>{t('Adım')}</th>
                  <th>{t('Hedef')}</th>
                  <th>{t('Etki')}</th>
                </tr>
              </thead>
              <tbody>
                {(r.plan ?? []).map((p, i) => (
                  <tr key={i}>
                    <td>{p.step}</td>
                    <td translate="no">{p.target}</td>
                    <td>{p.impact + (p.reversible ? '' : ` (${t('geri alınamaz')})`)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="row">
              <button type="button" className="primary" onClick={approve}>
                {t('Onayla ve uygula')}
              </button>
              <button type="button" onClick={() => setResult(null)}>
                {t('Vazgeç')}
              </button>
            </div>
          </div>
        ) : null}
        {typeof r?.markdown === 'string' ? <div translate="no">{r.markdown}</div> : null}
        {r?.applied ? <div className="tool">{t('Uygulandı')}: {r.applied}</div> : null}
        {r?.verify ? <div className={r.verify.ok ? 'tool' : 'err'}>{`${t('Doğrulama')}: ${r.verify.url} HTTP ${r.verify.status} ${r.verify.ok ? '✓' : '✗'}`}</div> : null}
        {r?.rolled_back ? <div className="warn">{t('Doğrulama başarısız: değişiklik geri alındı.')}</div> : null}
        {r?.credential ? <div className="note">{t('Parola Kor kasasında')}: {r.credential}</div> : null}
        {(r?.output ?? []).map((o, i) => (
          <div key={i} className="note" translate="no">{o}</div>
        ))}
      </div>
      <details>
        <summary>{t('Adımlar')}</summary>
        <ol className="steps">
          {steps.map((s) => (
            <li key={s.at}>
              {new Date(s.at).toLocaleTimeString()} — {s.label}
              {s.shot.startsWith('data:image/') ? <img src={s.shot} alt={t('Adım ekran görüntüsü')} /> : null}
            </li>
          ))}
        </ol>
      </details>
    </section>
  )
}
