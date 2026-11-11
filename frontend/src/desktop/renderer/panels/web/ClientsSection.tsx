// Müşteri portalı özeti (§29.13): çalışma alanları, zaman kaydı, biletler, erişilebilirlik/SSL, aylık rapor.
import { RefreshCw } from 'lucide-react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { Action } from './Action.tsx'
import { webApi } from './bridge.ts'
import { registerSection } from './section-registry.ts'

function ClientsSection() {
  const { data, error, isLoading, mutate } = useSWR('web:clients', () => webApi.clients())
  const ids = (data ?? []).map((c) => c.id)
  const who = { name: 'id', label: 'Müşteri', type: 'select' as const, options: ids, required: true }
  return (
    <>
      <div className="ks-actions">
        <p className="ks-note muted kw-grow">Kor'un sunucu ve site işleri, profil bir müşteriye bağlıysa süresiyle birlikte kendiliğinden kaydedilir. Rapor HTML/PDF, Telegram ve e-posta taslağı ile faturaya hazır zaman çizelgesi üretir; hiçbir şey gönderilmez.</p>
        <button type="button" className="btn small" onClick={() => void mutate()} aria-label="Listeyi yenile" title="Listeyi yenile" data-kor-cmd="web.clients">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {data && data.length === 0 && <p className="ks-empty">Müşteri çalışma alanı yok.</p>}
      {data && data.length > 0 && (
        <ul className="ks-list">
          {data.map((c) => (
            <li key={c.id} className="ks-row">
              <span translate="no">{c.id}</span>
              <span className="kw-grow">{c.client.name}</span>
              <span className="ks-chip">{c.client.sites.length + c.client.urls.length} site</span>
              {c.openTickets > 0 && <span className="ks-chip">{c.openTickets} açık bilet</span>}
            </li>
          ))}
        </ul>
      )}
      <Action
        title="Müşteri ekle / güncelle"
        fields={[
          { name: 'id', label: 'Kimlik (a-z, 0-9, -)', required: true },
          { name: 'name', label: 'Ad' },
          { name: 'email', label: 'E-posta' },
          { name: 'telegram', label: 'Telegram' },
          { name: 'sites', label: 'WordPress siteleri', list: true },
          { name: 'hosts', label: 'Sunucular', list: true },
          { name: 'urls', label: 'Diğer adresler', list: true },
          { name: 'rate', label: 'Saat ücreti (TL, KDV hariç)', type: 'number' },
        ]}
        run={(a) => webApi.clientAdd(a)}
        onDone={() => void mutate()}
        button="Kaydet"
        data-kor-cmd="web.clientAdd"
      />
      <Action
        title="Zaman kaydı"
        fields={[who, { name: 'hours', label: 'Saat', type: 'number', required: true }, { name: 'note', label: 'Yapılan iş', required: true }]}
        run={(a) => webApi.clientLog(a)}
        button="Kaydet"
        data-kor-cmd="web.clientLog"
      />
      <Action
        title="Biletler"
        fields={[
          who,
          { name: 'action', label: 'İşlem', type: 'select', options: ['list', 'add', 'close'], required: true },
          { name: 'title', label: 'Başlık (ekle)' },
          { name: 'number', label: 'Numara (kapat)', type: 'number' },
        ]}
        run={(a) => webApi.ticket(a)}
        onDone={() => void mutate()}
        button="Uygula"
        data-kor-cmd="web.ticket"
      />
      <Action title="Erişilebilirlik ve SSL ölçümü" fields={[who]} run={(a) => webApi.clientCheck(a)} button="Ölç" data-kor-cmd="web.clientCheck" />
      <Action
        title="Aylık rapor"
        fields={[who, { name: 'month', label: 'Ay (YYYY-MM)', placeholder: '2026-11' }, { name: 'format', label: 'Biçimler', list: true, placeholder: 'html, pdf' }]}
        run={(a) => webApi.clientReport(a)}
        button="Üret"
        data-kor-cmd="web.clientReport"
      />
    </>
  )
}

registerSection({ id: 'clients', label: 'Müşteriler', component: ClientsSection })
