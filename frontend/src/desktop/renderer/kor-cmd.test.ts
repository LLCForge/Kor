// node --test src/desktop/renderer/kor-cmd.test.ts
// §16.3 (en iyi çaba, statik): renderer/panels/** altında köprü çağıran her dosya, çağırdığı her `ns.method` için
// `data-kor-cmd`, dinlediği her ad alanı için `data-kor-event="kor:<ns>/…"` niteliği taşır.
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { DESKTOP_SRC, missingKorAttrs, sourceFiles, wrapperAliases } from '../../../scripts/parity-commands.ts'

test('panellerde köprü çağıran her dosya data-kor-cmd / data-kor-event bildirir', () => {
  const aliases = new Map<string, string>()
  for (const p of sourceFiles(DESKTOP_SRC)) for (const [a, ns] of wrapperAliases(readFileSync(p, 'utf8'))) aliases.set(a, ns)
  const panels = join(DESKTOP_SRC, 'renderer/panels')
  const problems: string[] = []
  const files = sourceFiles(panels).filter((f) => f.endsWith('.tsx'))
  // Nitelik panel klasörünün herhangi bir dosyasında olabilir (düğme çocukta, çağrı üst bileşende).
  const dirOf = (p: string) => relative(panels, p).split(/[\\/]/)[0]!
  const declared = new Map<string, string>()
  for (const p of files) declared.set(dirOf(p), `${declared.get(dirOf(p)) ?? ''}\n${readFileSync(p, 'utf8')}`)
  for (const p of files) {
    for (const miss of missingKorAttrs(readFileSync(p, 'utf8'), aliases, declared.get(dirOf(p)))) problems.push(`${relative(panels, p)}: ${miss}`)
  }
  assert.deepEqual(problems, [])
})

test('denetleyici: eksik nitelik bulunur, bildirilen geçer', () => {
  const aliases = new Map([['costApi', 'cost']])
  const bad = `api.graph.status(); costApi.why(1); api.graph.onUpdated(cb)`
  assert.deepEqual(missingKorAttrs(bad, aliases).sort(), ['data-kor-cmd="cost.why"', 'data-kor-cmd="graph.status"', 'data-kor-event="kor:graph/…" (graph.onUpdated)'])
  const good = `<div data-kor-event="kor:graph/updated"><button data-kor-cmd="graph.status cost.why" onClick={() => { api.graph.status(); costApi.why(1) }} /></div>; api.graph.onUpdated(cb); api.send(x)`
  assert.deepEqual(missingKorAttrs(good, aliases), [])
})
