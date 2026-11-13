// Görünüm yerleri: her görünümün tam bir evi var (panel başlığı sekmesi, kenar çubuğu grubu ya da Ayarlar bölümü).
// view-catalog.ts lucide/React içe aktardığından kaynak metinden okunur (ui-audit.e2e.ts ile aynı yol).
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { join } from 'node:path'
import { test } from 'node:test'

const dir = import.meta.dirname
const catalog = readFileSync(join(dir, 'view-catalog.ts'), 'utf8')
const settings = readFileSync(join(dir, 'SettingsDialog.tsx'), 'utf8')

const ids = (src: string): string[] => [...src.matchAll(/'([\w-]+)'/g)].map((m) => m[1]!)
const block = (name: string): string => {
  const m = new RegExp(`export const ${name}[^=]*=\\s*([\\[{][\\s\\S]*?\\n[\\]}])`).exec(catalog)
  assert.ok(m, `${name} bulunamadı`)
  return m[1]!
}

const builtin = [...block('BUILTIN_VIEWS').matchAll(/id: '([\w-]+)'/g)].map((m) => m[1]!)
const panels = readdirSync(join(dir, 'panels'), { withFileTypes: true })
  .filter((e) => e.isDirectory())
  .flatMap((e) => {
    try {
      const m = /^\s*id: '([\w-]+)',/m.exec(readFileSync(join(dir, 'panels', e.name, 'index.tsx'), 'utf8'))
      return m ? [m[1]!] : []
    } catch {
      return []
    }
  })
const header = ids(block('HEADER_TABS'))
const sidebar = [...block('SIDEBAR_VIEW_GROUPS').matchAll(/views: \[([^\]]*)\]/g)].flatMap((m) => ids(m[1]!))
const settingsViews = [...block('SETTINGS_VIEWS').matchAll(/(\w+): '([\w-]+)'/g)].map((m) => ({ view: m[1]!, section: m[2]! }))
const sections = [...settings.matchAll(/\{\s*id:\s*'([\w-]+)',\s*label:/g)].map((m) => m[1]!)

test('her görünüm panel başlığında, kenar çubuğunda ya da Ayarlar’da: hiçbiri yalnız sağ panelde kalmaz', () => {
  const homes = new Map<string, string[]>()
  const add = (id: string, where: string) => homes.set(id, [...(homes.get(id) ?? []), where])
  for (const id of header) add(id, 'başlık')
  for (const id of sidebar) add(id, 'kenar çubuğu')
  for (const { view } of settingsViews) add(view, 'ayarlar')
  // Önizleme bir dosya seçilince kendiliğinden açılır; başlıkta da sekmesi var.
  const all = [...builtin, ...panels]
  assert.ok(panels.length > 20, 'panel kimlikleri okunamadı')
  assert.deepEqual(all.filter((id) => !homes.has(id)), [])
  // Ayarlar'a taşınan görünüm sağ panel başlatıcılarında görünmez.
  assert.deepEqual(settingsViews.map((s) => s.view).filter((id) => (homes.get(id) ?? []).length > 1), [])
  // Katalogdaki her kimlik gerçek bir görünüm.
  assert.deepEqual([...homes.keys()].filter((id) => !all.includes(id)), [])
})

test('Ayarlar görünümleri var olan bir bölüme gider', () => {
  assert.ok(settingsViews.length > 0)
  assert.deepEqual(settingsViews.filter((s) => !sections.includes(s.section)), [])
})
