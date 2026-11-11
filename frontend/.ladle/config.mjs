// Ladle: kor-ui bileşenlerinin tüm durumları (spec §23.5, ADR 0053). `npm run ladle` (geliştirme sunucusu),
// `npm run ladle:build` (statik çıktı: build-ladle/, görsel regresyonun kaynağı).
/** @type {import('@ladle/react').UserConfig} */
export default {
  stories: 'src/desktop/renderer/kor-ui/stories/*.stories.tsx',
  outDir: 'build-ladle',
  defaultStory: 'kor-ui--button--variants',
  addons: { a11y: { enabled: true }, theme: { enabled: true, defaultState: 'dark' }, width: { enabled: true } },
}
