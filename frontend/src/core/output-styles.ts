// [g3] Çıktı stilleri. Saf modül: arayüz seçici etiketleri için de kullanır.

export const OUTPUT_STYLE_IDS = ['default', 'explanatory', 'concise', 'learning'] as const
export type OutputStyle = (typeof OUTPUT_STYLE_IDS)[number]

export interface OutputStyleInfo {
  id: OutputStyle
  label: string
  desc: string
  /** Sistem istemine eklenen bölüm; varsayılanda yok */
  prompt: string | null
}

export const OUTPUT_STYLES: readonly OutputStyleInfo[] = [
  { id: 'default', label: 'Varsayılan', desc: 'Standart yanıtlar', prompt: null },
  {
    id: 'explanatory',
    label: 'Açıklayıcı',
    desc: 'Kararların ve kod tabanının nedenlerini anlatır',
    prompt: `# OUTPUT STYLE: EXPLANATORY
While working, add short educational "Neden:" notes that explain the reasoning behind implementation choices and the codebase patterns you discover. Keep each note to 1-3 sentences, place it right after the relevant step, and never let notes delay or replace the actual work.`,
  },
  {
    id: 'concise',
    label: 'Kısa',
    desc: 'En az kelimeyle, yalnızca gerekeni söyler',
    prompt: `# OUTPUT STYLE: CONCISE
Be as brief as possible. No preamble, no recap of the question, no closing remarks. Prefer one-line answers, bullet fragments over sentences, and code over prose. Only include what the user needs to act.`,
  },
  {
    id: 'learning',
    label: 'Öğretici',
    desc: 'Öğrenme odaklı: küçük kısımları kullanıcıya yazdırır',
    prompt: `# OUTPUT STYLE: LEARNING
Act as a patient mentor. Explain concepts step by step with small examples. For small, self-contained pieces of code (about 2-10 lines) that teach something, do not write them yourself: describe exactly what is needed and leave a clearly marked "SENİN SIRAN:" task for the user, then review their attempt when they reply. Still complete larger or risky parts yourself.`,
  },
]

export function isOutputStyle(value: unknown): value is OutputStyle {
  return typeof value === 'string' && (OUTPUT_STYLE_IDS as readonly string[]).includes(value)
}
