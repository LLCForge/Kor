// Ayarlar > Ajanlar (cdesktop Agents düzeni): solda ajan listesi (ad + kaynak rozeti), sağda seçili ajanın
// ayrıntıları (açıklama, model, araç kapsamı, onay). Tanımlar salt okunurdur; ajanlar agent aracıyla alt görev olarak
// çalışır ve araç çağrıları yine izin sisteminden geçer.
import { Bot, Check, Package, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import useSWR from 'swr'
import type { AgentSummary } from '../../ipc-agents.ts'
import { api, errorMessage } from '../api.ts'
import { TeamsSection } from './TeamsSection.tsx' // [cdx] takımlar

const LIST_KEY = 'agents:list'
const SOURCE_LABEL: Record<AgentSummary['source'], string> = { builtin: 'yerleşik', embedded: 'gömülü', user: 'kullanıcı', project: 'proje' } // [g29]
const SOURCE_PATH: Record<AgentSummary['source'], string> = {
  builtin: 'Uygulama içi tanım',
  embedded: 'dist/builtin-agents/*.md',
  user: '~/.kor/agents/<ad>.md',
  project: '<proje>/.kor/agents/<ad>.md',
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="cdx-agent-row">
      <div className="cdx-agent-row-label">{label}</div>
      <div className="cdx-agent-row-value">{children}</div>
    </div>
  )
}

export function AgentsSection() {
  const list = useSWR(LIST_KEY, () => api.agents.list(), { revalidateOnFocus: false })
  const [selectedName, setSelectedName] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [originFilter, setOriginFilter] = useState<string>('all')
  const all = list.data ?? []
  // [cdx] Sayım + köken: "78 ajan · 4 yerleşik · 3 gömülü · 71 eklenti (ecc)"; eklenti adına göre süzülebilir.
  const originKey = (a: AgentSummary) => (a.origin ? `plugin:${a.origin}` : a.source)
  const counts = useMemo(() => {
    const m = new Map<string, number>()
    for (const a of all) m.set(originKey(a), (m.get(originKey(a)) ?? 0) + 1)
    return m
  }, [all])
  const originLabel = (key: string) => (key.startsWith('plugin:') ? `eklenti: ${key.slice(7)}` : SOURCE_LABEL[key as AgentSummary['source']])
  const q = query.trim().toLocaleLowerCase('tr')
  const agents = useMemo(
    () => all.filter((a) => (originFilter === 'all' || originKey(a) === originFilter) && (!q || `${a.name} ${a.description}`.toLocaleLowerCase('tr').includes(q))),
    [all, originFilter, q],
  )
  const selected = useMemo(() => agents.find((a) => a.name === selectedName) ?? agents[0] ?? null, [agents, selectedName])

  if (list.error) return <p className="error-text">{errorMessage(list.error)}</p>
  if (!list.data) return <p className="muted">Yükleniyor...</p>

  return (
    <div className="g7-stack">
      <p className="g7-note text-pretty">
        <Bot size={12} aria-hidden="true" /> Ajanlar; bir alt görevi baştan sona yürüten, kendi araç kümesi ve modeli olabilen alt ajanlardır. Yerleşik ve gömülü
        ajanlar her klasörde hazırdır; <code>~/.kor/agents/*.md</code>, proje <code>.kor/agents/*.md</code> (klasör güveniliyse) ve etkin eklentilerin{' '}
        <code>agents/</code> klasörü de yüklenir. Aynı isimde diskteki kopya gömülü tanımı ezer.
      </p>
      <div className="cdx-agents-toolbar">
        <span className="cdx-agents-count"><strong>{all.length}</strong> ajan</span>
        <div className="cdx-agents-chips" role="group" aria-label="Kökene göre süz">
          <button type="button" className={`cdx-mcp-cat${originFilter === 'all' ? ' on' : ''}`} onClick={() => setOriginFilter('all')}>Tümü <span className="cdx-mcp-cat-n">{all.length}</span></button>
          {[...counts.entries()].sort((a, b) => b[1] - a[1]).map(([key, n]) => (
            <button key={key} type="button" className={`cdx-mcp-cat${originFilter === key ? ' on' : ''}`} onClick={() => setOriginFilter(key)} title={key.startsWith('plugin:') ? `${key.slice(7)} eklentisinin agents/ klasörü` : SOURCE_PATH[key as AgentSummary['source']]}>
              {key.startsWith('plugin:') && <Package size={11} aria-hidden="true" />}
              {originLabel(key)} <span className="cdx-mcp-cat-n">{n}</span>
            </button>
          ))}
        </div>
        <div className="cdx-search cdx-mcp-search">
          <Search size={14} aria-hidden="true" />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Ajan ara…" aria-label="Ajan ara" />
        </div>
      </div>
      {all.length === 0 ? (
        <p className="muted">Yüklü ajan yok.</p>
      ) : agents.length === 0 ? (
        <p className="muted">Eşleşen ajan yok.</p>
      ) : (
        <div className="cdx-agents">
          <div className="cdx-agents-col">
            <div className="cdx-agents-head">Ajan <span className="cdx-mcp-cat-n">{agents.length}</span></div>
            <ul className="cdx-agents-list" role="listbox" aria-label="Ajanlar">
              {agents.map((a) => {
                const active = selected?.name === a.name
                return (
                  <li key={a.name}>
                    <button type="button" role="option" aria-selected={active} className={`cdx-agents-item${active ? ' active' : ''}`} onClick={() => setSelectedName(a.name)}>
                      <Bot size={14} aria-hidden="true" />
                      <span className="truncate">{a.name}</span>
                      <span className={`badge neutral cdx-agents-src${a.origin ? ' plugin' : ''}`} title={a.origin ? `${a.origin} eklentisi` : SOURCE_PATH[a.source]}>{a.origin ?? SOURCE_LABEL[a.source]}</span>
                    </button>
                  </li>
                )
              })}
            </ul>
          </div>
          <div className="cdx-agents-col cdx-agents-detail">
            <div className="cdx-agents-head">Yapılandırma</div>
            {selected && (
              <div className="cdx-agent-form">
                <Row label="Açıklama">{selected.description || <span className="muted">—</span>}</Row>
                <Row label="Kaynak">
                  {selected.origin ? (
                    <><Package size={13} aria-hidden="true" /> eklenti: <strong>{selected.origin}</strong> <span className="muted g7-mono"> · {selected.source === 'project' ? '<proje>/.kor' : '~/.kor'}/plugins/{selected.origin}/agents/</span></>
                  ) : (
                    <>{SOURCE_LABEL[selected.source]} <span className="muted g7-mono"> · {SOURCE_PATH[selected.source]}</span></>
                  )}
                </Row>
                <Row label="Model">{selected.model ?? <span className="muted">Sohbetin modeli</span>}</Row>
                <Row label="Araçlar">{selected.tools === 'all' ? 'Tüm araçlar' : `${selected.tools} araç`}</Row>
                <Row label="Her işlem onaylı">
                  <span className={`cdx-check${selected.manualApproval ? ' on' : ''}`} aria-hidden="true">{selected.manualApproval && <Check size={12} />}</span>
                  <span className="muted g9-small">{selected.manualApproval ? 'Her araç çağrısı onay ister' : 'İzin kuralları uygulanır'}</span>
                </Row>
              </div>
            )}
          </div>
        </div>
      )}
      <TeamsSection agents={all} />
    </div>
  )
}
