// Sistem gereksinimleri görünümünün saf yardımcıları (node --test ile sınanır).
import type { PrereqId, PrereqInstallResult, PrereqRow } from '../../../ipc-prereq.ts'

export const LABEL: Record<PrereqId, { name: string; why: string }> = {
  docker: { name: 'Docker', why: 'Dockerfile ya da compose dosyası olan projeler, Docker sandbox' },
  podman: { name: 'Podman', why: "Arka plan hizmeti gerektirmeyen kapsayıcılar, Docker'a alternatif" },
  wsl2: { name: 'WSL2', why: "Windows'ta Linux sandbox'ı" },
  'xcode-clt': { name: 'Xcode Komut Satırı Araçları', why: "macOS'ta git ve derleyiciler" },
  webview2: { name: 'WebView2 Çalışma Zamanı', why: 'Masaüstü penceresi (kurulum paketi zaten içerir)' },
  accessibility: { name: 'Erişilebilirlik izni', why: 'Bilgisayar kullanımı: fare ve klavye' },
  'screen-recording': { name: 'Ekran Kaydı izni', why: 'Ekran görüntüsü ve pencere yakalama' },
  microphone: { name: 'Mikrofon izni', why: 'Sesli giriş (kor voice)' },
}

export function statusText(row: PrereqRow): string {
  if (row.installed === true) return 'Kurulu'
  if (row.installed === null) return 'İlk gerektiğinde sistem sorar'
  return 'Eksik'
}

/** Düğme metni; null: düğme yok. */
export function actionText(row: PrereqRow): string | null {
  if (row.installed === true || row.command === null) return null
  return row.action === 'open-settings' ? 'Sistem Ayarlarını aç' : 'Kur'
}

export function reasonText(row: PrereqRow): string | null {
  if (row.reason === 'no-manager') {
    return 'Desteklenen bir paket yöneticisi bulunamadı (winget, choco, brew, apt, dnf, pacman, zypper, apk, nix). Kor bu olmadan çalışmaya devam eder; yalnızca buna ihtiyaç duyan özellik kapalı kalır.'
  }
  return null
}

export function resultText(id: PrereqId, r: PrereqInstallResult): string {
  const name = LABEL[id].name
  if (r.ok) return r.row.action === 'open-settings' ? `${name}: Sistem Ayarları açıldı; izni oradan verin.` : `${name} kuruldu.`
  if (r.code === null) return `${name} kurulmadı. Kor çalışmaya devam eder; yalnızca buna ihtiyaç duyan özellik kapalı kalır.`
  return `${name} kurulamadı (çıkış kodu ${r.code}). Kor bu olmadan çalışmaya devam eder.`
}

/** Satır başına en çok `keep` son çıktı satırı. */
export function appendLine(log: Readonly<Record<string, string[]>>, id: string, line: string, keep = 6): Record<string, string[]> {
  const prev = log[id] ?? []
  return { ...log, [id]: [...prev, line].slice(-keep) }
}
