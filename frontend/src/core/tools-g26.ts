// [g26] Katalog, oturum ağacı, tarif ve tarayıcı kaydı araçları.
import { mkdir, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { assistantHome } from './assistant-surface.ts'
import { handoffDocument, parseSnapshotRefs } from './browser-har-g26.ts'
import { loadConnectors } from './connectors-g26.ts'
import { authorizeUrl, MCP_CATALOG, pkcePair, saveOauthToken } from './mcp-catalog-g26.ts'
import { parseRecipe, writeRecipe } from './recipes-g26.ts'
import { buildSessionTree, formatSessionTree } from './session-tree-g26.ts'
import { defineTool, resolveInWorkspace, type Tool } from './tools.ts'
import { adoptProject } from './layout-g28.ts' // [g28]

export function g26Tools(): Tool[] {
  return [
    defineTool({
      name: 'session_tree',
      description: 'Kayıtlı sohbetlerin üst-alt ağacını döner. parentId ile bağlı kayıtlar girintili listelenir.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'oturum ağacı',
      subject: () => 'session_tree',
      run: async (_args, ctx) => {
        const { listSessions } = await import('./sessions.ts')
        const rows = await listSessions(ctx.sessionsDir)
        const tree = buildSessionTree(rows.map((row) => ({ id: row.id, title: row.title, ...(row.parentId ? { parentId: row.parentId } : {}) })))
        return tree.length === 0 ? 'Kayıtlı sohbet yok.' : formatSessionTree(tree)
      },
    }),
    defineTool({
      name: 'save_recipe',
      description:
        'Çalışma dizinine bir tarif yazar (.kor/recipes). cron beş alandır. tools dar allowlist. Uygulama açıkken, o dakika cron tutarsa çalışır; işletim sistemi görevi kurulmaz.',
      schema: z.object({
        name: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_-]{0,40}$/),
        cron: z.string().min(9).max(80),
        tools: z.array(z.string().regex(/^[a-z0-9_]{1,60}$/)).min(1).max(30),
        prompt: z.string().min(1).max(10_000),
      }),
      kind: 'edit',
      summarize: (a) => a.name,
      subject: (a) => a.name,
      run: async (a, ctx) => {
        const recipe = parseRecipe(`name: ${a.name}\ncron: "${a.cron}"\ntools: ${a.tools.join(', ')}\nprompt: |\n${a.prompt}\n`)
        const file = await writeRecipe(ctx.root, recipe)
        return `Tarif yazıldı: ${path.relative(ctx.root, file)}`
      },
    }),
    defineTool({
      name: 'list_mcp_catalog',
      description: 'Yerel MCP katalog tanımlarını listeler. Sunucu başlatmaz ve uzak liste çekmez.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'mcp katalog',
      subject: () => 'list_mcp_catalog',
      run: async () => MCP_CATALOG.map((entry) => `- ${entry.id} (${entry.transport}) ${entry.title}: ${entry.description}`).join('\n'),
    }),
    defineTool({
      name: 'mcp_authorize_url',
      description:
        'Kullanıcının verdiği https yetkilendirme adresine PKCE bağlantısı kurar. İstemci sırrı istenmez ve adreste bulunamaz. Erişim jetonu modele dönmez.',
      schema: z.object({
        serverId: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_-]{0,60}$/),
        endpoint: z.url({ protocol: /^https$/ }).max(2048),
        clientId: z.string().min(1).max(200),
        redirectUri: z.url({ protocol: /^https?$/ }).max(2048),
        scope: z.string().max(200).optional(),
      }),
      kind: 'read',
      summarize: (a) => a.serverId,
      subject: (a) => a.serverId,
      run: async (a, ctx) => {
        const pair = pkcePair()
        // Doğrulama kodu proje köküne yazılmaz: depoya girebilirdi. Kullanıcı yapılandırma dizini.
        await saveOauthToken(assistantHome(ctx.assistantHome), a.serverId, { verifier: pair.verifier })
        const url = authorizeUrl({ endpoint: a.endpoint, clientId: a.clientId, redirectUri: a.redirectUri, challenge: pair.challenge, ...(a.scope ? { scope: a.scope } : {}) })
        return `Yetkilendirme adresini kullanıcı açsın:\n${url}\nDoğrulama kodu yerelde duruyor; jeton bu yanıtta yok.`
      },
    }),
    defineTool({
      name: 'list_connectors',
      description: 'Yerel bağlayıcı kataloğunu ve her birinin araç allowlistini döner. Ağdan katalog çekilmez.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'bağlayıcılar',
      subject: () => 'list_connectors',
      run: async (_args, ctx) => {
        const list = await loadConnectors(ctx.root)
        return list.map((item) => `- ${item.id}: ${item.title} [${item.tools.join(', ')}]`).join('\n')
      },
    }),
    defineTool({
      name: 'browser_har',
      description: 'Ajan tarayıcısının son isteklerinin HAR kaydını döner. Çerez ve yetki başlığı yoktur.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'har',
      subject: () => 'browser_har',
      run: async (_args, ctx) => {
        if (!ctx.browser?.har) throw new Error('HAR yalnızca masaüstü ajan tarayıcısında var.')
        return ctx.browser.har()
      },
    }),
    defineTool({
      name: 'browser_handoff',
      description: 'Açık ajan sayfasının adresini, metnini ve eN referanslarını çalışma dizinine yazar. Çerez aktarılmaz.',
      schema: z.object({}),
      kind: 'edit',
      summarize: () => 'tarayıcı devri',
      subject: () => 'browser_handoff',
      run: async (_args, ctx) => {
        if (!ctx.browser?.handoff) throw new Error('Tarayıcı devri yalnızca masaüstünde var.')
        const page = await ctx.browser.handoff()
        const refs = parseSnapshotRefs(page.refs)
        const body = handoffDocument({ url: page.url, title: page.title, text: page.text, refs: page.refs })
        adoptProject(ctx.root)
        const abs = resolveInWorkspace(ctx.root, '.kor/browser-handoff.json')
        await mkdir(path.dirname(abs), { recursive: true })
        await writeFile(abs, body, { mode: 0o600 })
        return `Devredildi: .kor/browser-handoff.json (${refs.length} referans). Çerez yok.`
      },
    }),
  ]
}
