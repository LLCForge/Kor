// OsExtrasCard için saf mantık: etiketler, neden metinleri ve satırın düğmesi (node testi: osextra-model.test.ts).
import type { OsExtraStatus } from '../../ipc-osextra.ts'

export const LABELS: Readonly<Record<string, { label: string; desc: string }>> = {
  'powertoys-run': { label: 'PowerToys Run eklentisi', desc: '"kor <soru>" yazınca Kor’a sorar, son projeleri ve oturumları açar.' },
  'windows-terminal': { label: 'Windows Terminal profili', desc: 'Kor simgeli, kor komutunu açan bir profil ekler.' },
  wsl: { label: 'WSL2 farkındalığı', desc: '\\\\wsl$ yollarındaki projelerin komutları o dağıtımın içinde çalışır.' },
  'start-search': { label: 'Başlat menüsü araması', desc: 'Kurulumun kısayolu ve Windows Search’ün dizinlediği ~/Kor klasörü.' },
  'jump-list': { label: 'Atlama listesi', desc: 'Görev çubuğunda son projeler ve “Yeni görev”.' },
  widgets: { label: 'Windows 11 pencere öğesi', desc: 'Pencere Öğeleri panelinde görev özeti.' },
  shortcuts: { label: 'Kestirmeler eylemleri', desc: '“Kor’a Gönder”, “Görev Başlat”, “Son Yanıtı Al”, “Onayla”; Siri ve otomasyonlardan.' },
  spotlight: { label: 'Spotlight dizini', desc: 'Oturumlar Spotlight’ta aranır; sonuç oturumu açar.' },
  'universal-clipboard': { label: 'Evrensel Pano', desc: 'iPhone’dan yapıştırılanı fark eder, bağlam olarak eklemeyi sorar.' },
  handoff: { label: 'Handoff', desc: 'Açık oturum diğer Apple aygıtlarında sürdürülebilir.' },
  'menu-bar': { label: 'Menü çubuğu', desc: 'Durum, onaylar ve son projeler menü çubuğunda.' },
  'dock-menu': { label: 'Dock menüsü', desc: 'Dock simgesinde son projeler ve “Yeni görev”.' },
  'gnome-shell': { label: 'GNOME Shell uzantısı', desc: 'Üst çubukta durum ve hızlı menü.' },
  plasmoid: { label: 'KDE Plasma pencere öğesi', desc: 'Panelde durum, onaylar ve son projeler.' },
  'xdg-scheme': { label: 'kor:// bağlantıları', desc: 'xdg-open kor://… bağlantıları Kor’da açar.' },
  portals: { label: 'Wayland portal izinleri', desc: 'Ekran, girdi, dosya ve bildirim izinleri tek ekrandan.' },
  'systemd-user': { label: 'systemd kullanıcı servisi', desc: 'Kor arka plan hizmeti oturum açılınca başlar.' },
  flatpak: { label: 'Flatpak izinleri', desc: 'Flatpak paketi portal ve ~/Kor izinleriyle gelir.' },
}

export const REASONS: Readonly<Record<string, string>> = {
  'other-os': 'Bu işletim sistemi için değil.',
  'no-powertoys': 'PowerToys kurulu değil.',
  'no-bundle': 'Bu kurulumda eklenti paketi yok: yalnızca yükleyiciyle kurulan Kor masaüstü uygulamasıyla gelir (geliştirme ya da yalnızca CLI kurulumunda bulunmaz). Bir şey bozuk değil.',
  'no-terminal': 'Windows Terminal kurulu değil.',
  'no-shortcuts': 'Kestirmeler uygulaması bulunamadı.',
  'no-gnome': 'GNOME Shell bulunamadı.',
  'no-plasma': 'KDE Plasma bulunamadı.',
  'no-systemd': 'systemd bulunamadı.',
  'not-flatpak': 'Kor Flatpak olarak çalışmıyor.',
  'start-search': 'Başlat menüsü kısayolunu kurulum ekler.',
  delegated: 'Başka bir ayardan yönetilir.',
  builtin: 'Yerleşik; ayrıca kurulmaz.',
  'bad-url': 'Adres https:// ile başlamalı.',
  'shortcuts-manual': 'Kestirmeler uygulamasından elle silinmeli (macOS kaldırma arayüzü sunmuyor).',
}

export const VIA: Readonly<Record<string, string>> = {
  desktop: 'Masaüstü çalışırken kendiliğinden',
  'kor integrate': 'Sağ tık menüsü ayarıyla',
}

export type RowAction = 'install' | 'uninstall' | 'portals' | null

/** Satırda hangi düğme: kurulabilir/masaüstü öğeleri kur veya kaldır; portallar izin akışı. */
export function rowAction(s: OsExtraStatus): RowAction {
  if (s.kind !== 'install' && s.kind !== 'desktop') return null
  if (s.installed) return 'uninstall'
  if (!s.available) return null
  return s.id === 'portals' ? 'portals' : 'install'
}

export function reasonText(code: string | null): string | null {
  if (!code) return null
  return REASONS[code] ?? code
}
