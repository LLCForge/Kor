// Proje özeti (§18.3): maliyet sayacı (oturum belirteçleri + bugünkü harcama), graf özeti, model değiştir,
// şablonlar. Veriler çekirdekten; sahte sayaç yok, tur bitince yenilenir.
import { useEffect, useState } from 'react'
import { RefreshCw } from 'lucide-react'
import { client } from './bus.ts'
import { t } from './i18n.ts'

interface Usage {
  input: number
  output: number
  cache_read: number
}
interface CostInfo {
  usage: Usage | null
  today: { lines?: string[] } | null
}
interface GraphInfo {
  status: { files?: number; nodes?: number; edges?: number; languages?: Record<string, number> }
  communities: number
  hotspots: Array<{ id: string; name: string; degree: number }>
  docs: number
}

export function Project({ sessionId, turns, model, setDraft }: { sessionId: string; turns: number; model: string; setDraft: (s: string) => void }) {
  const [cost, setCost] = useState<CostInfo | null>(null)
  const [graph, setGraph] = useState<GraphInfo | null>(null)
  const [graphError, setGraphError] = useState('')
  const [templates, setTemplates] = useState<Array<{ name: string; description: string }>>([])
  const [m, setM] = useState(model)
  const [msg, setMsg] = useState('')

  useEffect(() => {
    client<CostInfo>('cost', { sessionId }).then(setCost, () => setCost(null))
  }, [sessionId, turns])
  const loadGraph = () => {
    setGraphError('')
    client<GraphInfo>('graph').then(setGraph, (e: Error) => setGraphError(e.message))
  }
  useEffect(loadGraph, [])
  useEffect(() => {
    client<{ templates: Array<{ name: string; description: string }> }>('commands').then((c) => setTemplates(c.templates ?? []), () => setTemplates([]))
  }, [])
  useEffect(() => setM(model), [model])

  const useTemplate = (name: string) =>
    void client<{ prompt: string }>('templateDraft', { name }).then((r) => setDraft(r.prompt), (e: Error) => setMsg(e.message))
  const saveModel = () => void client('setModel', { model: m.trim() }).then(() => setMsg(t('Model değişti')), (e: Error) => setMsg(e.message))

  return (
    <section className="project" aria-label={t('Proje')}>
      <h2>{t('Maliyet')}</h2>
      <div data-kor-cmd="client.cost">
        {cost?.usage ? (
          <p>
            {t('Bu oturum')}: {cost.usage.input.toLocaleString()} {t('girdi')} · {cost.usage.output.toLocaleString()} {t('çıktı')} · {cost.usage.cache_read.toLocaleString()} {t('önbellek')}
          </p>
        ) : (
          <p className="note">{t('Bu oturumda henüz kullanım yok.')}</p>
        )}
        {(cost?.today?.lines ?? []).slice(0, 3).map((l, i) => (
          <p key={i} className="note" translate="no">{l}</p>
        ))}
      </div>
      <h2>
        {t('Proje grafiği')}
        <button type="button" className="icon" aria-label={t('Yenile')} title={t('Yenile')} onClick={loadGraph} data-kor-cmd="client.graph">
          <RefreshCw size={14} />
        </button>
      </h2>
      {graphError ? <p className="err">{graphError}</p> : null}
      {graph ? (
        <div>
          <p>
            {graph.status.files ?? 0} {t('dosya')} · {graph.status.nodes ?? 0} {t('düğüm')} · {graph.status.edges ?? 0} {t('kenar')} · {graph.communities} {t('topluluk')} · {graph.docs} {t('doküman')}
          </p>
          <ol className="steps">
            {graph.hotspots.map((h) => (
              <li key={h.id} translate="no">
                {h.name} ({h.degree})
              </li>
            ))}
          </ol>
        </div>
      ) : graphError ? null : (
        <p className="note">{t('Yükleniyor…')}</p>
      )}
      <h2>{t('Model')}</h2>
      <div className="row">
        <input type="text" aria-label={t('Model')} placeholder="sağlayıcı/model" value={m} onChange={(e) => setM(e.target.value)} />
        <button type="button" onClick={saveModel} data-kor-cmd="client.setModel">
          {t('Değiştir')}
        </button>
      </div>
      <h2>{t('Şablonlar')}</h2>
      {templates.length === 0 ? <p className="note">{t('Şablon yok.')}</p> : null}
      <div className="cmds">
        {templates.map((x) => (
          <button key={x.name} type="button" title={x.description} onClick={() => useTemplate(x.name)} translate="no" data-kor-cmd="client.templateDraft">
            {x.name}
          </button>
        ))}
      </div>
      {msg ? <p className="note">{msg}</p> : null}
    </section>
  )
}
