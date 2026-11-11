// Türkiye'ye özgü katman (§29.14): KVKK/çerez/VERBİS, Türkçe SEO, TL/KDV, e-Fatura/e-İrsaliye, ödeme
// şablonları, pazaryerleri.
import { Action } from './Action.tsx'
import { webApi } from './bridge.ts'
import { registerSection } from './section-registry.ts'

const UBL_NOTE = 'Belge JSON olarak girilir (alanlar: id, profile, date, time, supplier, customer, lines). Kor imzasız UBL-TR XML üretir; imzalama ve GİB iletimi business.toml [efatura] içindeki özel entegratörünüz üzerinden yapılır.'

function parseDoc(a: Record<string, unknown>): Record<string, unknown> {
  const raw = typeof a.document === 'string' ? a.document : '{}'
  return { ...a, document: JSON.parse(raw) as unknown }
}

function TurkeySection() {
  return (
    <>
      <Action
        title="Site denetimi (KVKK, çerez, VERBİS, Türkçe SEO, ödeme)"
        fields={[{ name: 'url', label: 'Site (https://…)', required: true }, { name: 'site', label: 'Bağlı WordPress sitesi (eklentiler için)' }]}
        run={(a) => webApi.trCheck(a)}
        button="Denetle"
        data-kor-cmd="web.trCheck"
      />
      <Action
        title="KVKK aydınlatma metni ve çerez politikası"
        note="Şirket bilgileri business.toml dosyasından gelir. Hukuki metin değildir; yayımlamadan önce uzmana kontrol ettirin."
        fields={[
          { name: 'data', label: 'Veri kategorileri', list: true },
          { name: 'purposes', label: 'İşleme amaçları', list: true },
          { name: 'recipients', label: 'Aktarılan alıcılar', list: true },
          { name: 'abroad', label: 'Yurt dışı aktarım', list: true },
          { name: 'cookies', label: 'Çerezler (ad|amaç|süre, satır satır)', type: 'textarea', list: true },
        ]}
        run={(a) => webApi.trKvkk(a)}
        button="Üret"
        data-kor-cmd="web.trKvkk"
      />
      <Action title="Türkçe URL kısa adı" fields={[{ name: 'text', label: 'Metin', required: true }]} run={(a) => webApi.trSlug(a)} button="Dönüştür" data-kor-cmd="web.trSlug" />
      <Action
        title="KDV hesabı"
        fields={[
          { name: 'amount', label: 'Tutar (1.234,56)', required: true },
          { name: 'rate', label: 'Oran', type: 'select', options: ['1', '10', '20'] },
          { name: 'gross', label: 'Tutar KDV dahil', type: 'checkbox' },
        ]}
        run={(a) => webApi.trKdv({ ...a, rate: Number(a.rate ?? 20) })}
        button="Hesapla"
        data-kor-cmd="web.trKdv"
      />
      <Action
        title="e-Fatura / e-Arşiv (UBL-TR)"
        note={UBL_NOTE}
        fields={[{ name: 'document', label: 'Fatura (JSON)', type: 'textarea', required: true }, { name: 'send', label: 'Entegratöre gönder', type: 'checkbox' }]}
        run={(a) => webApi.trEfatura(parseDoc(a))}
        button="Üret"
        data-kor-cmd="web.trEfatura"
      />
      <Action
        title="e-İrsaliye (UBL-TR)"
        note={UBL_NOTE}
        fields={[{ name: 'document', label: 'İrsaliye (JSON; driver ya da carrier)', type: 'textarea', required: true }, { name: 'send', label: 'Entegratöre gönder', type: 'checkbox' }]}
        run={(a) => webApi.trIrsaliye(parseDoc(a))}
        button="Üret"
        data-kor-cmd="web.trIrsaliye"
      />
      <Action
        title="Ödeme entegrasyonu şablonu"
        note="Anahtarlar ortam değişkenlerinden okunur; koda yazılmaz."
        fields={[{ name: 'provider', label: 'Sağlayıcı', type: 'select', options: ['iyzico', 'paytr'], required: true }]}
        run={(a) => webApi.trPayment(a)}
        button="Oluştur"
        data-kor-cmd="web.trPayment"
      />
      <Action title="Pazaryeri sayıları (Trendyol, Hepsiburada)" fields={[]} run={() => webApi.trMarket()} button="Oku" data-kor-cmd="web.trMarket" />
      <Action
        title="Türkçe anahtar kelimeler"
        fields={[{ name: 'source', label: 'Adres ya da metin', type: 'textarea', required: true }, { name: 'top', label: 'Kaç tane', type: 'number' }]}
        run={(a) => webApi.trKeywords(a)}
        button="Say"
        data-kor-cmd="web.trKeywords"
      />
    </>
  )
}

registerSection({ id: 'turkey', label: 'Türkiye', component: TurkeySection })
