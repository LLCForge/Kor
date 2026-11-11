// Sandbox önizleme belgelerinin sunulduğu uygulama içi şema. Ana süreç ve arayüz paketi birlikte kullanır (Node importu yok).
//
// Neden srcdoc değil: srcdoc iframe'leri ana belgenin CSP'sini devralır (script-src 'self', style-src 'self'). Bu yüzden
// önizlemedeki satır içi <style> ve izin verilmiş betikler hiç çalışmıyordu. Ayrı şemadan yüklenen belge kendi CSP'sini
// (başlıkla) taşır: ağ yine tamamen kapalı, stiller açık, betikler yalnızca kullanıcı izin verdiğinde.
export const SANDBOX_SCHEME = 'kor-sandbox'

export type SandboxMode = 'static' | 'scripts'

export function sandboxUrl(mode: SandboxMode): string {
  // Tauri, Windows'ta (WebView2) özel şemaları http://<şema>.localhost/ olarak sunar.
  const windows = typeof navigator !== 'undefined' && navigator.userAgent.includes('Windows')
  return windows ? `http://${SANDBOX_SCHEME}.localhost/preview/${mode}` : `${SANDBOX_SCHEME}://preview/${mode}`
}

/** Önizleme ile ana pencere arasındaki postMessage zarfları. */
export const PREVIEW_SOURCE = 'g-rich-preview'
export const HOST_SOURCE = 'g-rich-host'
