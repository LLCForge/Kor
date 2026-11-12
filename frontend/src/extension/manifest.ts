// Eklenti manifestosu (MV3) — Chrome/Edge/Brave/Chromium ve Firefox için ayrı derleme hedefleri (§18.4, ADR 0058).
// `key`: paketlenmemiş derlemenin kimliğini her makinede aynı tutar (iokjlmpngkhnhndnklpildpdfpfdjddh); yerel
// mesajlaşma ana bilgisayarı yalnızca bu kimliğe (ve `kor chrome install --extension-id` ile eklenen mağaza kimliğine)
// izin verir (kor_platform::native_messaging).

export const CHROME_KEY =
  'MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAuAIYv7eADa2PLDjACxjC6+pYGQhrwOdz6cKU4WfqWtojFLzktNwv5nRpb8mrPI2qb/2nCpxcyDn2dZOnTkgyFHzXC8GZmWXzqlJCIJBXdSOh9A8cZUKDsa68YZF3RIk5ZBPerdBSleUW6/tnMtyBFP/O3MCgYvuSQQbF+th/8WuWpW15LUALkTSM5n9K6AntUA/u8kILP1XFJPN15dr/0Eqr7nYXoE0bxFc8QzcfV+1VEINjWCI/lzjSWUrOXcYpohomBl7wzsvJ/sXF0Pty6jDHI2Tp0mGiQtfXlPGylqwpOSE9Hy2zQkeYfNcrcHxTXAmfTwNmQrAI25OF2qBNGwIDAQAB'
export const FIREFOX_ID = 'kor@kor.dev'

export type Target = 'chrome' | 'firefox'

export function manifest(target: Target, version: string): Record<string, unknown> {
  const icons = { '48': 'icons/48.png', '96': 'icons/96.png', '128': 'icons/128.png' }
  const common = {
    manifest_version: 3,
    name: '__MSG_name__',
    description: '__MSG_description__',
    default_locale: 'tr',
    version,
    icons,
    action: { default_title: '__MSG_action__', default_icon: icons },
    // `<all_urls>`: Chrome's tabs.captureVisibleTab (screenshots) refuses narrower host patterns.
    host_permissions: ['<all_urls>'],
    content_security_policy: { extension_pages: "script-src 'self'; object-src 'none'; connect-src 'self' ws://127.0.0.1:49201" },
  }
  if (target === 'firefox') {
    return {
      ...common,
      permissions: ['tabs', 'scripting', 'storage', 'nativeMessaging', 'contextMenus', 'webRequest', 'alarms'],
      background: { scripts: ['background.js'] },
      sidebar_action: { default_panel: 'sidepanel.html', default_title: '__MSG_name__', default_icon: icons, open_at_install: false },
      browser_specific_settings: { gecko: { id: FIREFOX_ID, strict_min_version: '128.0' } },
    }
  }
  return {
    ...common,
    key: CHROME_KEY,
    minimum_chrome_version: '116',
    permissions: ['sidePanel', 'tabs', 'tabGroups', 'scripting', 'storage', 'nativeMessaging', 'contextMenus', 'webRequest', 'alarms'],
    optional_permissions: ['debugger'],
    background: { service_worker: 'background.js' },
    side_panel: { default_path: 'sidepanel.html' },
  }
}

/** Uzantı arayüzü metinleri (`_locales/<dil>/messages.json`). */
export const LOCALES: Record<string, Record<string, string>> = {
  tr: {
    name: 'Kor',
    description: 'Kor\'un tarayıcı yüzeyi: görevleri yan panelden izle ve yönet; Kor yalnızca kendi sekme grubunda, onayınla çalışır.',
    action: 'Kor — yan paneli aç',
  },
  en: {
    name: 'Kor',
    description: "Kor's browser surface: follow and drive tasks from the side panel; Kor works only in its own tab group, with your approval.",
    action: 'Kor — open the side panel',
  },
  fr: {
    name: 'Kor',
    description: "La surface navigateur de Kor : suivez et pilotez les tâches depuis le panneau latéral ; Kor n'agit que dans son propre groupe d'onglets, avec votre accord.",
    action: 'Kor — ouvrir le panneau latéral',
  },
}

export function messages(lang: string): Record<string, { message: string }> {
  return Object.fromEntries(Object.entries(LOCALES[lang] ?? LOCALES.tr).map(([k, v]) => [k, { message: v }]))
}
