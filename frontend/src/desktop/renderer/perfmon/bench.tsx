// `kor bench --ui`, masaüstü yarısı (§32.1, ADR 0062). Yalnızca `KOR_BENCH_MODE=full` ile başlatılan bir
// masaüstünde çalışır: gerçek bileşenleri (Thread, kayıtlı paneller, besteci) gerçek boyutta çizip her
// karenin işini ve çizimini ölçer, sonuçları `perfmon.benchDone` ile `kor bench`'e gönderir ve uygulama kapanır.
import { flushSync } from 'react-dom'
import { createRoot, type Root } from 'react-dom/client'
import { Thread } from '../Thread.tsx'
import type { ChatItem } from '../chat-state.ts'
import { PANELS } from '../panels/registry.ts'
import { perfmonApi, afterPaint, timeFrame } from './api.ts'
import { stat, syntheticThread } from './util.ts'
import type { PerfmonStat } from '../../ipc-perfmon.ts'

const THREAD_MESSAGES = 10_000

const noop = () => {}

function threadElement(items: ChatItem[], running: boolean) {
  return <Thread items={items} running={running} turnStartedAt={running ? Date.now() : null} composer={null} onApprove={noop} />
}

/** Ölçüm sahnesi: her şeyin üstünde, pencere boyutunda bir kök. */
function stage(): { root: Root; el: HTMLDivElement; done: () => void } {
  const el = document.createElement('div')
  el.className = 'kpm-stage'
  document.body.appendChild(el)
  const root = createRoot(el)
  return {
    root,
    el,
    done: () => {
      root.unmount()
      el.remove()
    },
  }
}

async function keypress(): Promise<number[]> {
  const box = document.querySelector<HTMLTextAreaElement>('textarea')
  if (!box) return []
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set
  if (!setter) return []
  const before = box.value
  const out: number[] = []
  const text = 'the quick brown fox jumps over the lazy dog '.repeat(2)
  for (let i = 1; i <= text.length; i++) {
    out.push(
      await timeFrame(() => {
        setter.call(box, before + text.slice(0, i))
        box.dispatchEvent(new Event('input', { bubbles: true }))
      }),
    )
  }
  setter.call(box, before)
  box.dispatchEvent(new Event('input', { bubbles: true }))
  return out
}

async function threadOpen(items: ChatItem[]): Promise<number[]> {
  const out: number[] = []
  for (let i = 0; i < 3; i++) {
    const s = stage()
    out.push(await timeFrame(() => flushSync(() => s.root.render(threadElement(items, false)))))
    await afterPaint()
    s.done()
  }
  return out
}

async function scrollAndTokens(items: ChatItem[]): Promise<{ scroll: number[]; tokens: number[] }> {
  const s = stage()
  flushSync(() => s.root.render(threadElement(items, false)))
  await afterPaint()
  const viewport = s.el.querySelector<HTMLElement>('.viewport')
  const scroll: number[] = []
  if (viewport) {
    viewport.scrollTop = viewport.scrollHeight
    await afterPaint()
    for (let i = 0; i < 120; i++) scroll.push(await timeFrame(() => (viewport.scrollTop -= 400)))
    viewport.scrollTop = viewport.scrollHeight
    await afterPaint()
  }
  // Akış: son yanıta her karede bir token eklenir (stream-coalescer'ın kare başına tek güncellemesi).
  const streaming = [...items, { kind: 'user', id: 'uq', text: 'devam et', attachments: [] } as ChatItem]
  let answer = ''
  const tokens: number[] = []
  for (let i = 0; i < 150; i++) {
    answer += `token${i} `
    const next = [...streaming, { kind: 'assistant', id: 'live', text: answer } as ChatItem]
    tokens.push(await timeFrame(() => flushSync(() => s.root.render(threadElement(next, true)))))
  }
  s.done()
  return { scroll, tokens }
}

async function panelOpen(): Promise<number[]> {
  const out: number[] = []
  for (let round = 0; round < 3; round++) {
    for (const p of PANELS) {
      const s = stage()
      const C = p.component
      out.push(await timeFrame(() => flushSync(() => s.root.render(<C sessionId={null} />))))
      s.done()
    }
  }
  return out
}

async function fileSearch(): Promise<number[]> {
  // İlk çağrı 50k dosyalık ağacı üretir ve tarar (ölçüme girmez).
  await perfmonApi.benchSearch('chat')
  const queries = ['chat', 'view', 'mdl', 'store', 'util', 'fil12', 'm42', 'uicore', 'webcomp', 'file_499']
  const out: number[] = []
  for (let i = 0; i < 30; i++) {
    const t0 = performance.now()
    await perfmonApi.benchSearch(queries[i % queries.length] ?? 'chat')
    out.push(performance.now() - t0)
  }
  return out
}

/** Hızlandırılmış uzun kullanım: aynı iş defalarca; ısınma sonrası ve son turdaki bellek farkı (%). */
async function soak(): Promise<number | null> {
  const round = async () => {
    const s = stage()
    const items = syntheticThread(2000)
    flushSync(() => s.root.render(threadElement(items, false)))
    let answer = ''
    for (let i = 0; i < 60; i++) {
      answer += `t${i} `
      const next = [...items, { kind: 'assistant', id: 'live', text: answer } as ChatItem]
      flushSync(() => s.root.render(threadElement(next, true)))
    }
    await afterPaint()
    s.done()
    for (const p of PANELS) {
      const ps = stage()
      const C = p.component
      flushSync(() => ps.root.render(<C sessionId={null} />))
      ps.done()
    }
    await afterPaint()
  }
  for (let i = 0; i < 3; i++) await round()
  const before = (await perfmonApi.stats()).rssMb
  for (let i = 0; i < 25; i++) await round()
  // Çöp toplayıcıya fırsat: boşta bir an.
  await perfmonApi.idle(2)
  const after = (await perfmonApi.stats()).rssMb
  return before > 0 ? ((after - before) / before) * 100 : null
}

export async function runBench(): Promise<void> {
  try {
    // Uygulamanın açılış işleri (oturum listesi, sağlayıcılar) bitsin.
    await perfmonApi.idle(2)
    const items = syntheticThread(THREAD_MESSAGES)
    const results: Record<string, PerfmonStat> = {}
    const put = (id: string, v: readonly number[]) => {
      if (v.length > 0) results[id] = stat(v)
    }
    put('ui.keypress', await keypress())
    put('ui.thread_open', await threadOpen(items))
    const { scroll, tokens } = await scrollAndTokens(items)
    put('ui.scroll', scroll)
    put('ui.token_render', tokens)
    put('ui.panel_open', await panelOpen())
    put('ui.file_search', await fileSearch())
    const leak = await soak()
    if (leak !== null) put('ui.leak', [leak])
    // Boşta: hiçbir şey çizilmezken 5 sn CPU ve bellek.
    await afterPaint()
    const idle = await perfmonApi.idle(5)
    put('ui.memory_idle', [idle.rssMb])
    put('ui.cpu_idle', [idle.cpuPercent])
    await perfmonApi.benchDone(results)
  } catch (err) {
    await perfmonApi.benchError(err instanceof Error ? err.message : String(err))
  }
}
