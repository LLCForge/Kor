// Ayarlar > Eklentiler: kurulu eklentiler (etkin/pasif, güncelle, kaldır) + katalogdan tek tıkla kurulum (git clone)
// ve rea'yı MCP olarak ekleme. Kurulum kod çalıştırmaz; güncelleme onaylıdır (git pull). Yürütülen her şey
// (araç/kanca) mevcut izin/güven kapılarından geçer; proje eklentileri yalnızca klasör güveniliyorsa yüklenir.
import { Download, FolderOpen, Link, Package, RefreshCw, Search, Star, Store, Trash2 } from 'lucide-react'
import { useEffect, useState, type ReactNode } from 'react'
import useSWR from 'swr'
import type { DiscoveredPlugin } from '../../../core/plugins.ts'
import type { CatalogItem, StoreItem } from '../../ipc-plugins.ts'
import { api, errorMessage } from '../api.ts'
import { confirmAction } from '../ConfirmDialog.tsx'
import { Segmented } from '../kor-ui/index.ts'
import './plugins.css'

const LIST_KEY = 'plugins:list'
const CATALOG_KEY = 'plugins:catalog'

function contributionParts(c: DiscoveredPlugin['contributions']): string[] {
  return [c.commands > 0 ? `${c.commands} komut` : null, c.skills > 0 ? `${c.skills} skill` : null, c.agents > 0 ? `${c.agents} ajan` : null, c.tools > 0 ? `${c.tools} araç` : null].filter(
    (x): x is string => x !== null,
  )
}

type Tab = 'installed' | 'store'

function Tag({ children }: { children: ReactNode }) {
  return <span className="kpl-tag">{children}</span>
}

function Progress({ pct, label }: { pct: number; label: string }) {
  return (
    <div className="plugin-progress" role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
      <div className="plugin-progress-fill" style={{ width: `${pct}%` }} />
      <span className="plugin-progress-pct">{percentText(pct)}</span>
    </div>
  )
}

interface RowProps {
  icon: ReactNode
  name: string
  owner?: string
  desc?: string
  tags?: ReactNode
  actions: ReactNode
  extra?: ReactNode
}

/** Liste satırı: simge, ad + sahip, 2 satıra kırpılmış açıklama, etiket çipleri, sağa hizalı eylemler. */
function Row({ icon, name, owner, desc, tags, actions, extra }: RowProps) {
  return (
    <li className="kpl-row">
      <span className="kpl-icon" aria-hidden="true">
        {icon}
      </span>
      <div className="kpl-main">
        <div className="kpl-head">
          <span className="kpl-name" translate="no">
            {name}
          </span>
          {owner && (
            <span className="kpl-owner" translate="no">
              {owner}
            </span>
          )}
        </div>
        {desc && (
          <p className="kpl-desc" title={desc}>
            {desc}
          </p>
        )}
        {tags && <div className="kpl-tags">{tags}</div>}
        {extra}
      </div>
      <div className="kpl-actions">{actions}</div>
    </li>
  )
}

export function PluginsSection() {
  const list = useSWR(LIST_KEY, () => api.plugins.list(), { revalidateOnFocus: true })
  const catalog = useSWR(CATALOG_KEY, () => api.plugins.catalog(), { revalidateOnFocus: true })
  const [storeQuery, setStoreQuery] = useState('')
  const [storeInput, setStoreInput] = useState('')
  const [repoUrl, setRepoUrl] = useState('')
  const store = useSWR(['plugins:store', storeQuery], () => api.plugins.storeSearch(storeQuery), { revalidateOnFocus: false, keepPreviousData: true })
  const [updates, setUpdates] = useState<Record<string, boolean>>({})
  // Birden çok işlem aynı anda yürüyebilir; her buton yalnızca kendi anahtarıyla kilitlenir.
  const [busy, setBusy] = useState<Set<string>>(() => new Set())
  const [progress, setProgress] = useState<Record<string, number>>({})
  const [message, setMessage] = useState<string | null>(null)
  const [tab, setTab] = useState<Tab | null>(null)

  // Ana süreçten gelen git clone yüzde ilerlemesi (eşzamanlı kurulumlarda ad bazında ayrışır).
  useEffect(() => api.plugins.onInstallProgress(({ name, percent }) => setProgress((p) => ({ ...p, [`inst:${name}`]: percent }))), [])

  const refreshAll = async () => {
    await Promise.all([list.mutate(), catalog.mutate(), store.mutate()])
  }
  const run = (key: string, fn: () => Promise<unknown>, ok?: string) => {
    setBusy((b) => new Set(b).add(key))
    setMessage(null)
    void fn()
      .then(async () => {
        await refreshAll()
        if (ok) setMessage(ok)
      })
      .catch((err: unknown) => setMessage(errorMessage(err)))
      .finally(() => {
        setBusy((b) => {
          const n = new Set(b)
          n.delete(key)
          return n
        })
        setProgress((p) => {
          const n = { ...p }
          delete n[key]
          return n
        })
      })
  }

  const install = (c: CatalogItem) => {
    if (c.kind === 'mcp') run(`mcp:${c.id}`, () => api.plugins.addMcp(c.id), `${c.label} MCP sunucusu eklendi ve etkinleştirildi (Ayarlar > Geliştirici).`)
    else run(`inst:${c.name}`, () => api.plugins.install(c.name), `${c.name} kuruldu; aşağıdan etkinleştir.`)
  }
  const installRepo = (url: string) => {
    const u = url.trim()
    if (u) run(`inst:${u}`, () => api.plugins.installRepo(u), 'Eklenti kuruldu; aşağıdan etkinleştir.')
  }
  const update = (dirName: string) => run(`upd:${dirName}`, () => api.plugins.update(dirName), `${dirName} güncellendi.`)
  const uninstall = (dirName: string, label: string) =>
    void confirmAction({ title: 'Eklentiyi kaldır', message: `"${label}" eklentisi silinsin mi? Klasörü ve içeriği kaldırılır.`, confirmLabel: 'Kaldır', danger: true }).then((okq) => {
      if (okq) run(`rm:${dirName}`, () => api.plugins.uninstall(dirName), `${label} kaldırıldı.`)
    })
  const checkUpdates = () =>
    run('check', async () => {
      setUpdates(await api.plugins.checkUpdates())
    })

  if (list.error) return <p className="error-text">{errorMessage(list.error)}</p>
  if (!list.data || !catalog.data) return <p className="muted">Yükleniyor...</p>

  const notInstalled = catalog.data.filter((c) => !c.installed)
  const current: Tab = tab ?? (list.data.length > 0 ? 'installed' : 'store')
  const repoKey = `inst:${repoUrl.trim()}`

  return (
    <div className="g7-stack kpl">
      <details className="kpl-about">
        <summary className="g7-note">
          <Package size={12} aria-hidden="true" /> Eklentiler komut, skill, ajan ve araçları tek klasörde toplar. Kurulum kod çalıştırmaz.
        </summary>
        <p className="g7-note text-pretty">
          Biçim: plugin.json + commands/ skills/ agents/ tools/ (Claude/Cursor uyumlu). Kod yalnızca etkinleştirme + izin/güven kapılarından sonra çalışır. Güncelleme onaylıdır (git
          pull). Proje eklentileri yalnızca klasör güveniliyorsa yüklenir.
        </p>
      </details>

      <div className="kpl-toolbar">
        <Segmented<Tab>
          label="Eklenti görünümü"
          value={current}
          onChange={setTab}
          options={[
            { value: 'installed', label: `Kurulu (${list.data.length})`, icon: <Package size={14} aria-hidden="true" /> },
            { value: 'store', label: 'Mağaza', icon: <Store size={14} aria-hidden="true" /> },
          ]}
        />
        <span className="g9-grow" />
        <button type="button" className="btn small" onClick={() => void api.plugins.openFolder().catch((err: unknown) => setMessage(errorMessage(err)))}>
          <FolderOpen size={14} aria-hidden="true" /> Eklenti klasörünü aç
        </button>
        <button type="button" className="btn small" onClick={checkUpdates} disabled={busy.has('check')}>
          <RefreshCw size={14} aria-hidden="true" /> {busy.has('check') ? 'Denetleniyor...' : 'Güncellemeleri denetle'}
        </button>
      </div>
      {message && <p className="g7-note" role="status">{message}</p>}

      {current === 'installed' && (
        <section aria-label="Kurulu eklentiler">
          {list.data.length === 0 ? (
            <div className="kpl-empty">
              <Package size={20} aria-hidden="true" />
              <p className="muted">Henüz kurulu eklenti yok.</p>
              <button type="button" className="btn small" onClick={() => setTab('store')}>
                <Store size={14} aria-hidden="true" /> Mağazaya göz at
              </button>
            </div>
          ) : (
            <ul className="kpl-list">
              {list.data.map((p) => {
                const parts = contributionParts(p.contributions)
                return (
                  <Row
                    key={p.dir}
                    icon={<Package size={16} />}
                    name={p.manifest.name}
                    owner={p.source === 'project' ? 'proje' : 'kullanıcı'}
                    desc={p.manifest.description}
                    tags={
                      <>
                        {p.manifest.version && <Tag>v{p.manifest.version}</Tag>}
                        {parts.length > 0 ? parts.map((c) => <Tag key={c}>{c}</Tag>) : <Tag>içerik yok</Tag>}
                        {updates[p.dirName] && <span className="badge g9-uncensored">güncelleme var</span>}
                      </>
                    }
                    actions={
                      <>
                        {p.managed && updates[p.dirName] && (
                          <button type="button" className="btn small" onClick={() => update(p.dirName)} disabled={busy.has(`upd:${p.dirName}`)}>
                            <Download size={14} aria-hidden="true" /> {busy.has(`upd:${p.dirName}`) ? 'Güncelleniyor...' : 'Güncelle'}
                          </button>
                        )}
                        {p.managed && (
                          <button type="button" className="action-btn" onClick={() => uninstall(p.dirName, p.manifest.name)} aria-label={`${p.manifest.name} kaldır`} title="Kaldır" disabled={busy.has(`rm:${p.dirName}`)}>
                            <Trash2 size={14} aria-hidden="true" />
                          </button>
                        )}
                        <input
                          type="checkbox"
                          role="switch"
                          checked={p.enabled}
                          disabled={busy.has(`en:${p.dirName}`)}
                          onChange={(e) => run(`en:${p.dirName}`, () => api.plugins.setEnabled(p.dirName, e.target.checked))}
                          aria-label={`${p.manifest.name} etkin`}
                          title={p.enabled ? 'Etkin: pasifleştir' : 'Pasif: etkinleştir'}
                        />
                      </>
                    }
                  />
                )
              })}
            </ul>
          )}
        </section>
      )}

      {current === 'store' && (
        <>
          <section className="kpl-section" aria-labelledby="kpl-store">
            <h4 className="g7-h4" id="kpl-store">
              <Store size={14} aria-hidden="true" /> Global mağaza
            </h4>
            <p className="g9-small muted text-pretty">
              GitHub'daki eklenti depoları (varsayılan: <code>claude-code-plugin</code> konusu). Tek tıkla kurulur (git clone; kod çalıştırmaz).
            </p>
            <form
              className="kpl-search"
              role="search"
              onSubmit={(e) => {
                e.preventDefault()
                setStoreQuery(storeInput)
              }}
            >
              <Search size={14} aria-hidden="true" />
              <input type="search" value={storeInput} onChange={(e) => setStoreInput(e.target.value)} placeholder="Ara: isim, konu, anahtar kelime..." aria-label="Mağazada ara" />
              <button type="submit" className="btn small primary" disabled={store.isLoading}>
                Ara
              </button>
            </form>

            {store.error && <p className="error-text">{errorMessage(store.error)}</p>}
            {store.isLoading && <p className="muted">Mağaza yükleniyor...</p>}
            {store.data && store.data.length === 0 && !store.isLoading && <p className="muted">Sonuç yok. Farklı bir anahtar kelimeyle ara ya da aşağıdan depo adresiyle kur.</p>}

            {store.data && store.data.length > 0 && (
              <ul className="kpl-list">
                {store.data.map((p: StoreItem) => {
                  const key = `inst:${p.repoUrl}`
                  const installing = busy.has(key)
                  const slash = p.fullName.indexOf('/')
                  return (
                    <Row
                      key={p.repoUrl}
                      icon={<Package size={16} />}
                      name={slash >= 0 ? p.fullName.slice(slash + 1) : p.fullName}
                      owner={p.author}
                      desc={p.description}
                      tags={
                        <>
                          {p.stars > 0 && (
                            <span className="kpl-tag kpl-stars" title={`${p.stars} yıldız`}>
                              <Star size={11} aria-hidden="true" /> {p.stars}
                            </span>
                          )}
                          {p.topics.slice(0, 4).map((t) => (
                            <Tag key={t}>{t}</Tag>
                          ))}
                        </>
                      }
                      extra={installing ? <Progress pct={progress[key] ?? 0} label={`${p.fullName} kuruluyor`} /> : undefined}
                      actions={
                        p.installed ? (
                          <span className="badge g9-on">kurulu</span>
                        ) : (
                          <button type="button" className="btn small" onClick={() => installRepo(p.repoUrl)} disabled={installing}>
                            <Download size={14} aria-hidden="true" /> {installing ? 'Kuruluyor...' : 'Kur'}
                          </button>
                        )
                      }
                    />
                  )
                })}
              </ul>
            )}
          </section>

          {notInstalled.length > 0 && (
            <section className="kpl-section" aria-labelledby="kpl-catalog">
              <h4 className="g7-h4" id="kpl-catalog">Katalog · kurulabilir</h4>
              <ul className="kpl-list">
                {notInstalled.map((c) => {
                  const key = c.kind === 'mcp' ? `mcp:${c.id}` : `inst:${c.name}`
                  const installing = busy.has(key)
                  return (
                    <Row
                      key={key}
                      icon={<Package size={16} />}
                      name={c.kind === 'mcp' ? c.label : c.name}
                      owner={c.kind === 'plugin' ? c.author : undefined}
                      desc={c.description}
                      tags={<Tag>{c.kind === 'mcp' ? 'MCP' : 'eklenti'}</Tag>}
                      extra={installing && c.kind === 'plugin' ? <Progress pct={progress[key] ?? 0} label={`${c.name} kuruluyor`} /> : undefined}
                      actions={
                        <button type="button" className="btn small" onClick={() => install(c)} disabled={installing}>
                          <Download size={14} aria-hidden="true" /> {installing ? 'Kuruluyor...' : c.kind === 'mcp' ? 'MCP olarak ekle' : 'Kur'}
                        </button>
                      }
                    />
                  )
                })}
              </ul>
            </section>
          )}

          <section className="kpl-section" aria-labelledby="kpl-repo">
            <h4 className="g7-h4" id="kpl-repo">
              <Link size={14} aria-hidden="true" /> Depo adresiyle kur
            </h4>
            <form
              className="kpl-search"
              onSubmit={(e) => {
                e.preventDefault()
                installRepo(repoUrl)
              }}
            >
              <input type="url" value={repoUrl} onChange={(e) => setRepoUrl(e.target.value)} placeholder="https://github.com/kullanici/depo.git" aria-label="Depo adresi" spellCheck={false} />
              <button type="submit" className="btn small" disabled={!repoUrl.trim() || busy.has(repoKey)}>
                <Download size={14} aria-hidden="true" /> {busy.has(repoKey) ? 'Kuruluyor...' : 'Kur'}
              </button>
            </form>
            {busy.has(repoKey) && <Progress pct={progress[repoKey] ?? 0} label="Depo kuruluyor" />}
            <p className="g9-dim">Yalnızca https:// adresleri. Klon kod çalıştırmaz; kod yalnızca etkinleştirme + izin/güven kapılarından sonra çalışır.</p>
          </section>
        </>
      )}
    </div>
  )
}

import { percentText } from '../ui-locale.ts'