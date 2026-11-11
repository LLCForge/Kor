import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { installTitleTooltips } from './kor-ui/title-tooltips.ts'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { App } from './App.tsx'
import { FirstRunRuntimes } from './FirstRunRuntimes.tsx'
import { bootUiLanguage } from './i18n/boot.ts'
import './styles/theme.gen.css' // assets/theme.json token'ları (ADR 0053)
import './styles.css'
import './styles/g1.css'
import './styles/g2.css'
import './styles/g3.css'
import './styles/g4.css'
import './styles/g5.css'
import './styles/g6.css'
import './styles/g7.css'
import './styles/g8.css'
import './styles/g17.css' // [g17]
import './styles/rich.css'
import './styles/rich-blocks.css' // [g8] etkileşimli zengin bloklar
import './styles/g13.css' // [g13] geliştirici araçları
import './styles/g11.css' // [g11] güven rozeti, hafıza katmanları
import './styles/g12.css' // [g12] güvenlik diyaloğu ve toplu silme onayı
import './styles/g19.css' // [g19] harita blokları
import './styles/g14.css' // [g14]
import './styles/g18.css' // [g18] grafikler
import './styles/g10.css' // [g10] güvenilirlik bantları
import './styles/g22.css' // [g22]
import './styles/g9.css' // [g9] sağlayıcılar ve maliyet
import './styles/claude-theme.css' // Claude Code görünümü (en sonda: yarıçapı sıkılaştırır)
import './styles/g20.css' // [g20] not defteri, REPL ve tuval
import './styles/g21.css' // [g21] belge ve zengin dosya önizleme
import './styles/g23.css' // [g23] yerel asistan kartları
import './styles/g27.css' // [g27] ayar çubukları ve görsel şerit
import './styles/cdesktop.css' // cdesktop birebir bileşen stilleri (en sonda)
import './shellui/shellui.css' // ADR 0051 tepsi, zil, PiP
import { PipView } from './shellui/PipView.tsx'
import { TrayPopup } from './shellui/TrayPopup.tsx'
import { bootPerf } from './perfmon/boot.tsx' // §32: açılış ölçümü, Ctrl+Shift+P bindirmesi, perf.log (ADR 0062)
import './kor-ui/kor-ui.css' // kor-ui bileşenleri (§23.5)
import './styles/kor-design.css' // Kor görsel dili: token eşlemesi, fontlar, düzen (en sonda, §23)
import './styles/controls.css' // sınıfsız form denetimleri için taban görünüm (sıfır özgüllük)
import '../../../../assets/kor-thinking.css' // KorThinking küpü, KorSpinner, KorDots (§16.1)
import { bootMotion, markEntered } from './motion.ts'
import { Splash } from './Splash.tsx'

// Dil React çizmeden önce uygulanır: eklenen her düğüm gözlemciyle çevrilir.
bootUiLanguage()
// Hareket kipi (Ayarlar > Görünüm > Animasyonlar) ilk kareden önce kök öğeye yazılır.
bootMotion()

// §23.1: every `title` in the app shows the themed 400 ms tooltip instead of the browser's.
installTitleTooltips()

const container = document.getElementById('root')
if (!container) throw new Error('#root bulunamadı')

// Aynı arayüz paketi üç pencereye hizmet eder: ana pencere, durum penceresi (pip) ve tepsi açılır penceresi (traypop).
const label = getCurrentWindow().label
if (label === 'pip' || label === 'traypop') {
  document.documentElement.classList.add('ksh-pip-root')
  createRoot(container).render(<StrictMode>{label === 'pip' ? <PipView /> : <TrayPopup />}</StrictMode>)
} else {
  createRoot(container).render(
    <StrictMode>
      <FirstRunRuntimes>
        <App />
      </FirstRunRuntimes>
      <Splash />
    </StrictMode>,
  )
  markEntered()
  void bootPerf()
}
