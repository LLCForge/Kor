// Şablon formu değerlerini `templates.run` girdisine çevirir (test: template-inputs.test.ts).
// Boş isteğe bağlı alanlar gönderilmez (şablonun varsayılanı geçerli olur); doğrulama çekirdekte.
import type { TemplateInput } from '../../ipc-templates.ts'

export function buildTemplateInputs(inputs: readonly TemplateInput[], values: Readonly<Record<string, string | boolean>>): Record<string, string | number | boolean> {
  const out: Record<string, string | number | boolean> = {}
  for (const i of inputs) {
    const v = values[i.name]
    if (v === undefined) continue
    if (i.kind.type === 'bool') {
      out[i.name] = v === true
      continue
    }
    const s = String(v).trim()
    if (!s) continue
    const n = Number(s)
    out[i.name] = i.kind.type === 'number' && Number.isFinite(n) ? n : s
  }
  return out
}
