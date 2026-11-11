// Ladle global sağlayıcısı: uygulamayla aynı stil sırası (token'lar → temel → kor-ui → Kor görsel dili).
import type { GlobalProvider } from '@ladle/react'
import '../src/desktop/renderer/styles/theme.gen.css'
import '../src/desktop/renderer/styles.css'
import '../src/desktop/renderer/styles/claude-theme.css'
import '../src/desktop/renderer/styles/cdesktop.css'
import '../src/desktop/renderer/kor-ui/kor-ui.css'
import '../src/desktop/renderer/styles/kor-design.css'
import '../src/desktop/renderer/kor-ui/stories/stories.css'

export const Provider: GlobalProvider = ({ children, globalState }) => (
  <div className="kor-story-root" data-theme={globalState.theme}>
    {children}
  </div>
)
