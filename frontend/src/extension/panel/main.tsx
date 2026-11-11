// Yan panel girişi: React kökü. WebView kilidi (§21.2, ADR 0053) ayrı `webview-lock.js` olarak yüklenir.
import { createRoot } from 'react-dom/client'
import { App } from './App.tsx'
import { applyLanguage } from './i18n.ts'
import '../../../../assets/kor-thinking.css' // KorSpinner (§16.1)
import './panel.css'

applyLanguage()
const root = document.getElementById('root')
if (root) createRoot(root).render(<App />)
