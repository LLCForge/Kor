// Yönlendirme haritası (§29.11): eşleştir, düzelt (CSV), sunucu biçimlerine aktar, yayından sonra denetle.
import { Action } from './Action.tsx'
import { webApi } from './bridge.ts'
import { registerSection } from './section-registry.ts'

const SOURCE = 'sitemap:https://…, crawl:https://…, adres ya da dosya (URL listesi, CSV, erişim günlüğü)'

function RedirectsSection() {
  return (
    <>
      <p className="ks-note muted">Eski ve yeni URL'ler aynı yol, aynı son parça (Türkçe harfler katlanarak), yol sözcükleri ve sayfa başlıklarıyla eşleşir. Harita proje klasörüne redirects.csv olarak yazılır; düzeltip yeniden kullanabilirsiniz.</p>
      <Action
        title="Haritayı oluştur"
        fields={[
          { name: 'old', label: `Eski site (${SOURCE})`, required: true },
          { name: 'new', label: 'Yeni site', required: true },
          { name: 'min', label: 'En düşük benzerlik (0–1)', type: 'number' },
          { name: 'limit', label: 'En çok sayfa', type: 'number' },
        ]}
        run={(a) => webApi.redirectsBuild(a)}
        button="Eşleştir"
        data-kor-cmd="web.redirectsBuild"
      />
      <Action
        title="Sunucu biçimine aktar"
        fields={[
          { name: 'map', label: 'Harita CSV yolu', required: true },
          { name: 'format', label: 'Biçim', type: 'select', options: ['apache', 'nginx', 'cloudflare', 'redirection', 'csv'], required: true },
          { name: 'host', label: 'Eski alan adı (Cloudflare)' },
          { name: 'collapse', label: 'Zincirleri tek adıma indir', type: 'checkbox' },
        ]}
        run={(a) => webApi.redirectsExport(a)}
        button="Aktar"
        data-kor-cmd="web.redirectsExport"
      />
      <Action
        title="Yayından sonra denetle"
        fields={[
          { name: 'map', label: 'Harita CSV yolu', required: true },
          { name: 'base', label: 'Canlı site (https://…)', required: true },
        ]}
        run={(a) => webApi.redirectsCheck(a)}
        button="Denetle"
        data-kor-cmd="web.redirectsCheck"
      />
      <Action
        title="Türkçe karakterli URL'ler"
        fields={[{ name: 'source', label: SOURCE, required: true }]}
        run={(a) => webApi.redirectsSlugs(a)}
        button="Bul"
        data-kor-cmd="web.redirectsSlugs"
      />
    </>
  )
}

registerSection({ id: 'redirects', label: 'Yönlendirme haritası', component: RedirectsSection })
