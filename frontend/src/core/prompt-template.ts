// Sistem istemi şablonlama: kullanıcının özel istemi ve AGENTS.md içinde değişken ve koşullu blok.
//   {{date}} {{os}} {{arch}} {{shell}} {{project_root}} {{project_name}} {{git_branch}}
//   {{#if windows}}...{{else}}...{{/if}}   {{#unless git}}...{{/unless}}   (iç içe kullanılabilir)
// Kod çalıştırmaz, dosya okumaz: yalnızca verilen değişken tablosundan metin yerleştirir. Bilinmeyen değişken
// olduğu gibi kalır (istemdeki başka bir {{...}} sözdizimi bozulmasın).
import path from 'node:path'

export type TemplateVars = Readonly<Record<string, string | boolean>>

const BLOCK = /\{\{#(if|unless)\s+([A-Za-z_][\w]*)\s*\}\}((?:(?!\{\{#(?:if|unless)\s)[\s\S])*?)\{\{\/\1\}\}/
const VAR = /\{\{\s*([A-Za-z_][\w]*)\s*\}\}/g
/** İç içe blok derinliği/sayısı sınırı: kötü biçimli şablonda sonsuz döngüye girmesin. */
const MAX_BLOCKS = 500

const truthy = (v: string | boolean | undefined): boolean => (typeof v === 'string' ? v.length > 0 : v === true)

export function renderPromptTemplate(text: string, vars: TemplateVars): string {
  let out = text
  // En içteki blok önce çözülür (içinde başka #if/#unless açılışı olmayan blok); dışarıya doğru ilerlenir.
  for (let i = 0; i < MAX_BLOCKS; i++) {
    const m = BLOCK.exec(out)
    if (!m) break
    const [whole, kind, name = '', body = ''] = m
    const [yes, no = ''] = body.split('{{else}}', 2)
    const cond = truthy(vars[name]) === (kind === 'if')
    out = out.slice(0, m.index) + (cond ? (yes ?? '') : no) + out.slice(m.index + whole.length)
  }
  return out.replace(VAR, (match, name: string) => {
    const v = vars[name]
    return typeof v === 'string' ? v : typeof v === 'boolean' ? String(v) : match
  })
}

/** buildSystemPrompt'un şablona verdiği tablo. Saat yok: istem her dakika değişip önbelleği bozmasın. */
export function promptTemplateVars(o: { root: string; shell: string; gitBranch: string | null; now?: Date }): TemplateVars {
  return {
    date: (o.now ?? new Date()).toISOString().slice(0, 10),
    os: process.platform,
    arch: process.arch,
    shell: o.shell,
    project_root: o.root,
    project_name: path.basename(o.root),
    git_branch: o.gitBranch ?? '',
    git: o.gitBranch !== null,
    windows: process.platform === 'win32',
    mac: process.platform === 'darwin',
    linux: process.platform === 'linux',
  }
}
