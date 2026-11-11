// esbuild takma adı (scripts/build-extension.ts KOR_ASSETS) → assets/KorThinking.tsx. Dosya frontend paketinin
// dışında olduğundan tsc onu doğrudan derleyemez; imzalar buradadır ve asset'teki dışa aktarımlarla aynıdır.
declare module 'kor-assets/KorThinking' {
  /** Kor küpü (glow 1,8 s, hot 1,8 s, spark 2 s, bob 3,6 s); `size` px, `label` erişilebilirlik metni. */
  export function KorThinking(props: { size?: number; label?: string }): import('react').ReactElement
  export function KorSpinner(props: { size?: number }): import('react').ReactElement
  export function KorDots(props: { width?: number }): import('react').ReactElement
  export function KorStatus(props: { verb: string; elapsedMs: number; tokens?: number }): import('react').ReactElement
}

// assets/sounds/*.wav (§28.4): the bundler copies the file and gives its URL.
declare module 'kor-assets/sounds/*' {
  const url: string
  export default url
}
