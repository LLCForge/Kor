// Teklif ve sözleşme (§29.12): iş tanımından kapsam, aşamalı fiyat; şablonlar düzenlenebilir dosyalar.
import { Action } from './Action.tsx'
import { webApi } from './bridge.ts'
import { registerSection } from './section-registry.ts'

const FORMATS = { name: 'format', label: 'Biçimler (md, html, pdf, docx)', list: true, placeholder: 'html, pdf, docx' }

function ProposalSection() {
  return (
    <>
      <p className="ks-note muted">Saat ücreti, KDV, ödeme planı ve paketler business.toml dosyasından gelir. ≈ işaretli süreler Kor'un tahminidir. Sözleşmeler hukuki metin değildir; uzman kontrolünden geçirin.</p>
      <Action title="business.toml" fields={[]} run={() => webApi.businessInit()} button="Oluştur / göster" data-kor-cmd="web.businessInit" />
      <Action title="Şablon klasörü" fields={[]} run={() => webApi.templates()} button="Göster" data-kor-cmd="web.templates" />
      <Action
        title="Yeni teklif"
        fields={[
          { name: 'client', label: 'Müşteri', required: true },
          { name: 'project', label: 'İş', required: true },
          { name: 'text', label: 'İş tanımı / toplantı notu / müşteri e-postası', type: 'textarea' },
          { name: 'items', label: 'Kalemler (Başlık=saat, virgülle)', list: true },
          { name: 'packages', label: 'Paketler (business.toml)', list: true },
          { name: 'lang', label: 'Dil', type: 'select', options: ['tr', 'en'] },
          { name: 'contracts', label: 'Sözleşme, KVKK eki ve gizlilik sözleşmesi de', type: 'checkbox' },
          FORMATS,
        ]}
        run={(a) => webApi.proposal(a)}
        button="Üret"
        data-kor-cmd="web.proposal"
      />
      <Action
        title="Müşteri yanıtından revizyon"
        note="Satırlar: 'çıkar: …', 'ekle: Başlık=saat', 'Başlık = saat', 'not: …'."
        fields={[
          { name: 'dir', label: 'Teklif klasörü', required: true },
          { name: 'reply', label: 'Müşteri yanıtı', type: 'textarea', required: true },
          { name: 'contracts', label: 'Sözleşmeleri de güncelle', type: 'checkbox' },
          FORMATS,
        ]}
        run={(a) => webApi.proposalRevise(a)}
        button="Revize et"
        data-kor-cmd="web.proposalRevise"
      />
    </>
  )
}

registerSection({ id: 'proposal', label: 'Teklif ve sözleşme', component: ProposalSection })
