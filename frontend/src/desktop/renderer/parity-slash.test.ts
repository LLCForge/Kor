import assert from 'node:assert/strict'
import { test } from 'node:test'
import { PARITY_COMMANDS } from './parity-commands.gen.ts'
import { paritySlashCommands } from './parity-slash.ts'

test('/ menüsü Kor komutlarını kök + alt komut olarak sunar ve doğru komutu çalıştırır', () => {
  const ran: string[] = []
  const cmds = paritySlashCommands(new Set(['model']), (c, rest) => ran.push(`${c.cli}|${rest}`))
  assert.ok(cmds.length > 50, `yalnızca ${cmds.length} kök komut`)
  assert.ok(!cmds.some((c) => c.name === 'model'), 'yerleşik adlarla çakışan kök atlanır')
  const seo = PARITY_COMMANDS.find((c) => c.cli === 'kor seo audit')
  assert.ok(seo)
  const root = cmds.find((c) => c.name === 'seo')
  assert.ok(root?.suggest)
  const ctx = { notify: (m: string) => ran.push(`note:${m}`) } as unknown as Parameters<NonNullable<typeof root.suggest>>[0]
  assert.ok(root.suggest(ctx, 'aud').some((s) => s.value === 'audit'))
  void root.run(ctx, 'audit https://ornek.com')
  assert.deepEqual(ran.at(-1), 'kor seo audit|https://ornek.com')
  void root.run(ctx, '')
  assert.ok(ran.at(-1)!.startsWith('note:'), 'alt komutsuz kök kullanıcıya seçenekleri söyler')
})
