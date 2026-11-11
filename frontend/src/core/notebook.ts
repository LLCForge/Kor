import { randomBytes } from 'node:crypto'
import { z } from 'zod'

const MAX_OUTPUT_TEXT_CHARS = 4_000

// Dosya dışarıdan gelir: yalnızca kullandığımız alanlar doğrulanır, geri kalanı (metadata vb.) aynen korunur.
const multiline = z.union([z.string(), z.array(z.string())])
const outputSchema = z
  .object({
    output_type: z.string(),
    name: z.string().optional(),
    text: multiline.optional(),
    data: z.record(z.string(), z.unknown()).optional(),
    ename: z.string().optional(),
    evalue: z.string().optional(),
    traceback: z.array(z.string()).optional(),
  })
  .passthrough()
const cellSchema = z
  .object({
    cell_type: z.string(),
    source: multiline,
    id: z.string().optional(),
    outputs: z.array(outputSchema).optional(),
    execution_count: z.number().nullable().optional(),
  })
  .passthrough()
const notebookSchema = z
  .object({ cells: z.array(cellSchema), nbformat: z.number().optional(), nbformat_minor: z.number().optional() })
  .passthrough()

export type Notebook = z.infer<typeof notebookSchema>
type Cell = Notebook['cells'][number]
export type CellType = 'code' | 'markdown' | 'raw'

export function parseNotebook(text: string): Notebook {
  let json: unknown
  try {
    json = JSON.parse(text)
  } catch {
    throw new Error('Notebook geçerli JSON değil')
  }
  const parsed = notebookSchema.safeParse(json)
  if (!parsed.success) throw new Error(`Notebook biçimi tanınmadı: ${z.prettifyError(parsed.error).slice(0, 500)}`)
  return parsed.data
}

const joinText = (value: string | string[] | undefined) => (Array.isArray(value) ? value.join('') : (value ?? ''))
const stripAnsi = (text: string) => text.replace(/\u001b\[[0-9;]*[A-Za-z]/g, '')
const clip = (text: string) => (text.length > MAX_OUTPUT_TEXT_CHARS ? `${text.slice(0, MAX_OUTPUT_TEXT_CHARS)}\n[çıktı kesildi]` : text)

function describeOutput(output: z.infer<typeof outputSchema>): string {
  if (output.output_type === 'stream') return clip(stripAnsi(joinText(output.text)))
  if (output.output_type === 'error') return clip(stripAnsi(`${output.ename ?? 'Hata'}: ${output.evalue ?? ''}\n${(output.traceback ?? []).join('\n')}`))
  const data = output.data ?? {}
  const parts: string[] = []
  for (const [mime, value] of Object.entries(data)) {
    if (mime.startsWith('image/')) {
      const size = typeof value === 'string' ? Math.round((value.length * 3) / 4 / 1024) : 0
      parts.push(`[görsel çıktı: ${mime}, ~${size} KB]`)
    }
  }
  const text = data['text/plain'] ?? data['text/markdown']
  const asText = typeof text === 'string' ? text : Array.isArray(text) ? text.filter((t): t is string => typeof t === 'string').join('') : null
  if (asText !== null) parts.unshift(clip(asText))
  else if (data['text/html'] !== undefined) parts.unshift('[HTML çıktı]')
  return parts.join('\n') || `[${output.output_type} çıktısı]`
}

/** Hücreleri, türlerini ve metin çıktılarını okunabilir biçimde döner; görseller özetlenir. */
export function formatNotebook(nb: Notebook): string {
  if (nb.cells.length === 0) return '(boş notebook)'
  return nb.cells
    .map((cell, i) => {
      const count = cell.cell_type === 'code' ? ` [${cell.execution_count ?? ' '}]` : ''
      const lines = [`### Hücre ${i} (${cell.cell_type})${count}`, joinText(cell.source)]
      const outputs = cell.outputs ?? []
      if (outputs.length > 0) lines.push('--- çıktı ---', ...outputs.map(describeOutput))
      return lines.join('\n')
    })
    .join('\n\n')
}

/** Jupyter kaynağı satır dizisi olarak saklar; her satır (sonuncusu hariç) \n ile biter. */
function toSource(text: string): string[] {
  const lines = text.split(/(?<=\n)/)
  return lines.length === 1 && lines[0] === '' ? [] : lines
}

function newCell(nb: Notebook, type: CellType, source: string): Cell {
  const cell: Cell = { cell_type: type, metadata: {}, source: toSource(source) }
  // nbformat 4.5+ hücre kimliği zorunlu.
  if ((nb.nbformat ?? 4) > 4 || (nb.nbformat_minor ?? 0) >= 5) cell.id = randomBytes(4).toString('hex')
  if (type === 'code') {
    cell.outputs = []
    cell.execution_count = null
  }
  return cell
}

export interface NotebookEdit {
  cell_index: number
  new_source: string
  mode: 'replace' | 'insert' | 'delete'
  cell_type?: CellType | undefined
}

/** Saf düzenleme: yeni notebook nesnesi döner. Kod hücresinin kaynağı değişince eski çıktılar temizlenir. */
export function editNotebook(nb: Notebook, edit: NotebookEdit): Notebook {
  const cells = [...nb.cells]
  const { cell_index: index, mode } = edit
  if (mode === 'insert') {
    if (index > cells.length) throw new Error(`cell_index en fazla ${cells.length} olabilir (sona eklemek için ${cells.length})`)
    cells.splice(index, 0, newCell(nb, edit.cell_type ?? 'code', edit.new_source))
    return { ...nb, cells }
  }
  const current = cells[index]
  if (!current) throw new Error(`Hücre ${index} yok; notebook ${cells.length} hücre içeriyor`)
  if (mode === 'delete') {
    cells.splice(index, 1)
    return { ...nb, cells }
  }
  const type = edit.cell_type ?? (current.cell_type as CellType)
  if (type !== current.cell_type) {
    const replaced = newCell(nb, type, edit.new_source)
    if (current.id !== undefined) replaced.id = current.id
    cells[index] = { ...replaced, metadata: current.metadata ?? {} }
  } else {
    const updated: Cell = { ...current, source: toSource(edit.new_source) }
    if (type === 'code') {
      updated.outputs = []
      updated.execution_count = null
    }
    cells[index] = updated
  }
  return { ...nb, cells }
}

/** Jupyter'in kendi biçimi: 1 boşluk girinti, sonda satır sonu. */
export function serializeNotebook(nb: Notebook): string {
  return `${JSON.stringify(nb, null, 1)}\n`
}
