// WordPress/WooCommerce (§29.3) ve eklenti doktoru (§29.4).
import { RefreshCw, Trash2 } from 'lucide-react'
import { useState } from 'react'
import useSWR from 'swr'
import { errorMessage } from '../../api.ts'
import { Action } from './Action.tsx'
import { webApi } from './bridge.ts'
import { registerSection } from './section-registry.ts'

function WordPressSection() {
  const { data, error, isLoading, mutate } = useSWR('web:sites', () => webApi.sites())
  const [removing, setRemoving] = useState<string | null>(null)
  const names = (data ?? []).filter((s) => !s.error).map((s) => s.name)
  const site = { name: 'name', label: 'Site', type: 'select' as const, options: names, required: true }
  return (
    <>
      <div className="ks-actions">
        <p className="ks-note muted kw-grow">REST API (uygulama şifresi kasada) ve/veya SSH üzerinden WP-CLI. Güncellemeler önce yedek alır, her adımdan sonra ana sayfa, ürün, sepet ve ödeme sayfalarını dener; bozulan eklentiyi eski sürümüne döndürür.</p>
        <button type="button" className="btn small" onClick={() => void mutate()} aria-label="Listeyi yenile" title="Listeyi yenile" data-kor-cmd="web.sites">
          <RefreshCw size={14} aria-hidden="true" />
        </button>
      </div>
      {isLoading && <p className="muted" role="status">Yükleniyor…</p>}
      {error && <p className="error-text" role="alert">{errorMessage(error)}</p>}
      {data && data.length === 0 && <p className="ks-empty">Bağlı WordPress sitesi yok.</p>}
      {data && data.length > 0 && (
        <ul className="ks-list">
          {data.map((s) => (
            <li key={s.name} className="ks-row">
              <span translate="no">{s.name}</span>
              <span className="muted kw-grow" translate="no">{s.error ?? s.url}</span>
              {s.rest && <span className="ks-chip">REST</span>}
              {s.cli && <span className="ks-chip">WP-CLI</span>}
              <button
                type="button"
                className="btn small"
                aria-label={`${s.name} bağlantısını kaldır`}
                title="Bağlantıyı kaldır"
                disabled={removing === s.name}
                data-kor-cmd="web.wpRemove"
                onClick={() => {
                  setRemoving(s.name)
                  void webApi.wpRemove({ name: s.name }).finally(() => {
                    setRemoving(null)
                    void mutate()
                  })
                }}
              >
                <Trash2 size={14} aria-hidden="true" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <Action
        title="Site bağla"
        fields={[
          { name: 'name', label: 'Ad (a-z, 0-9, -)', required: true },
          { name: 'url', label: 'Site adresi (https://…)', required: true },
          { name: 'user', label: 'WordPress kullanıcısı (REST)' },
          { name: 'secret', label: 'Kasa kaydı (uygulama şifresi)' },
          { name: 'ssh', label: 'SSH (WP-CLI için)' },
          { name: 'path', label: 'Sunucudaki WordPress klasörü' },
          { name: 'host', label: 'Sunucu profili' },
          { name: 'client', label: 'Müşteri' },
        ]}
        run={(a) => webApi.wpConnect(a)}
        onDone={() => void mutate()}
        button="Bağla"
        data-kor-cmd="web.wpConnect"
      />
      <Action title="Envanter" fields={[site]} run={(a) => webApi.wpInventory(a)} button="Oku" data-kor-cmd="web.wpInventory" />
      <Action title="Sağlık kontrolü" fields={[site]} run={(a) => webApi.wpHealth(a)} button="Denetle" data-kor-cmd="web.wpHealth" />
      <Action
        title="Eklenti doktoru"
        note="Kaynaklar: wordpress.org eklenti API'si; WPSCAN_API_TOKEN kasadaysa WPScan güvenlik açığı verisi. Ölçüm hazırlık kopyasında yapılır."
        fields={[site, { name: 'php', label: 'Hedef PHP sürümü (uyumluluk uyarısı)' }, { name: 'measure', label: 'Eklenti başına süreyi ölç (hazırlık kopyası gerekir)', type: 'checkbox' }]}
        run={(a) => webApi.wpDoctor(a)}
        button="Tara"
        data-kor-cmd="web.wpDoctor"
      />
      <Action
        title="Güncelle"
        note="Önce hazırlık kopyasında deneyin; canlıya uygulama için 'Canlıya uygula' işaretlenmelidir."
        fields={[site, { name: 'staging', label: 'Önce hazırlık kopyasında dene', type: 'checkbox' }, { name: 'live', label: 'Canlıya uygula', type: 'checkbox' }, { name: 'only', label: 'Yalnızca (eklenti adları, virgülle)', list: true }]}
        run={(a) => webApi.wpUpdate(a)}
        button="Güncelle"
        danger
        data-kor-cmd="web.wpUpdate"
      />
      <Action
        title="Hazırlık kopyası (staging)"
        fields={[site, { name: 'action', label: 'İşlem', type: 'select', options: ['create', 'drop'], required: true }]}
        run={(a) => webApi.wpStaging(a)}
        button="Uygula"
        data-kor-cmd="web.wpStaging"
      />
      <Action
        title="Bütünlük ve zararlı kod taraması"
        note="Çekirdek dosyalar WordPress.org sağlama toplamlarıyla, eklentiler WP-CLI ile karşılaştırılır; değişen dosyalarda şüpheli kalıplar aranır."
        fields={[site]}
        run={(a) => webApi.wpIntegrity(a)}
        button="Tara"
        data-kor-cmd="web.wpIntegrity"
      />
      <Action
        title="Temizlik (saldırı sonrası)"
        note="Yedek alınır; çekirdek ve bozulmuş eklentiler yeniden kurulur, uploads içindeki PHP silinir, anahtarlar (salt) yenilenir, yöneticilere parola sıfırlama e-postası gider. Ardından Search Console'dan yeniden inceleme isteyin."
        fields={[site]}
        run={(a) => webApi.wpCleanup({ ...a, confirm: true })}
        button="Temizle"
        danger
        data-kor-cmd="web.wpCleanup"
      />
      <Action
        title="İskelet oluştur (çocuk tema, eklenti, OpenCart, PrestaShop)"
        note="Dosyalar açık proje klasörüne yazılır; var olan dosyaların üzerine yazılmaz."
        fields={[
          { name: 'kind', label: 'Tür', type: 'select', options: ['child-theme', 'plugin', 'opencart', 'prestashop'], required: true },
          { name: 'title', label: 'Ad', required: true },
          { name: 'slug', label: 'Klasör (a-z, 0-9, -)', required: true },
          { name: 'parent', label: 'Ana tema (çocuk tema)' },
          { name: 'woocommerce', label: 'WooCommerce kancaları', type: 'checkbox' },
        ]}
        run={(a) => webApi.wpScaffold(a)}
        button="Oluştur"
        data-kor-cmd="web.wpScaffold"
      />
      <Action
        title="Platformu tanı"
        fields={[{ name: 'target', label: 'Adres ya da klasör', required: true }]}
        run={(a) => webApi.detect(a)}
        button="Tanı"
        data-kor-cmd="web.detect"
      />
    </>
  )
}

registerSection({ id: 'wordpress', label: 'WordPress ve e-ticaret', component: WordPressSection })
