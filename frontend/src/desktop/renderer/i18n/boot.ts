// Arayüz dilini açılışta uygular ve seçimi kaydeder. Gerçek ayar kullanıcı config.toml'undaki `ui.language`'dır
// (CLI ile ortak); localStorage yalnızca ilk karenin doğru dilde çizilmesi için bir önbellektir.
import { api } from '../api.ts'
import { getUiLanguage, setUiLanguage, type UiLang } from './translate.ts'

const KEY = 'kor.uiLang'

function isLang(v: unknown): v is UiLang {
  return v === 'tr' || v === 'en' || v === 'fr'
}

function cached(): UiLang | null {
  try {
    const v = localStorage.getItem(KEY)
    return isLang(v) ? v : null
  } catch {
    return null
  }
}

function remember(lang: UiLang): void {
  try {
    localStorage.setItem(KEY, lang)
  } catch {
    // Önbellek yoksa dil yine ayardan gelir; yalnızca ilk kare Türkçe çizilebilir.
  }
}

export function bootUiLanguage(): void {
  setUiLanguage(cached() ?? 'tr')
  api
    .getState()
    .then((s) => {
      if (isLang(s.uiLanguage) && s.uiLanguage !== getUiLanguage()) setUiLanguage(s.uiLanguage)
      if (isLang(s.uiLanguage)) remember(s.uiLanguage)
    })
    .catch(() => {
      // Arka uç yanıt vermezse önbellekteki dil kalır; ayar ekranı hatayı ayrıca gösterir.
    })
}

/** Ayar ekranından: önce kalıcı ayara yazılır, başarılıysa arayüz hemen o dile geçer. */
export async function chooseUiLanguage(pref: 'auto' | UiLang): Promise<void> {
  await api.setUiLanguage(pref)
  const state = await api.getState()
  if (isLang(state.uiLanguage)) {
    setUiLanguage(state.uiLanguage)
    remember(state.uiLanguage)
  }
}
