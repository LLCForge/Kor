// node --test src/desktop/renderer/shellui/state.test.ts
import { test } from 'node:test'
import assert from 'node:assert/strict'
import type { Eta, KorNotification, QueueItem, RunningSession, ShellState } from '../../ipc-shellui.ts'
import {
  approvalFor,
  countdown,
  elapsed,
  etaLabel,
  etaMethod,
  filterNotifications,
  folderName,
  move,
  percent,
  presentCategories,
  primarySession,
  queueAction,
  similarIds,
  trayStatus,
  updatePillLabel,
  usd,
  validQuietHours,
} from './state.ts'

const eta = (over: Partial<Eta>): Eta => ({ finishMs: 0, remainingSecs: 0, percentile: 50, scope: 'project', samples: 12, overdue: false, ...over })

const session = (over: Partial<RunningSession>): RunningSession => ({
  id: 's',
  title: 't',
  root: '/p',
  since: 0,
  verb: 'Közlüyor',
  tool: null,
  costUsd: 0,
  costEstimated: false,
  progress: null,
  eta: null,
  waiting: false,
  ...over,
})

const item = (over: Partial<QueueItem>): QueueItem => ({
  id: 'a',
  source: 'desktop',
  owner: 's',
  title: '',
  tool: 'Edit',
  summary: '',
  destructive: null,
  files: [],
  since: 0,
  risk: 'low',
  group: 'g',
  ...over,
})

const note = (over: Partial<KorNotification>): KorNotification => ({
  id: 'n1',
  at: 0,
  category: 'done',
  title: '',
  body: '',
  session: null,
  job: null,
  approval: null,
  costUsd: null,
  costEstimated: false,
  files: null,
  read: false,
  key: null,
  ...over,
})

test('elapsed and money', () => {
  assert.equal(elapsed(0, 65_000), '1:05')
  assert.equal(elapsed(0, 3_720_000), '1 sa 02 dk')
  assert.equal(elapsed(10, 0), '0:00')
  assert.equal(usd(0.844, true), '≈$0.84')
  assert.equal(usd(2, false), '$2.00')
})

test('eta is always marked and explains its method', () => {
  const at = new Date(2026, 10, 10, 14, 32).getTime()
  assert.equal(etaLabel(eta({ finishMs: at })), '≈ 14:32 civarı biter')
  assert.equal(etaLabel(eta({ overdue: true, finishMs: null })), 'Her zamankinden uzun sürüyor')
  assert.equal(etaLabel(null), null)
  assert.equal(etaMethod(eta({})), 'bu projedeki 12 turun p50 değeri')
  assert.equal(etaMethod(eta({ scope: 'all', samples: 4, percentile: 90 })), 'son 4 turun p90 değeri')
  assert.equal(countdown(eta({ finishMs: 200_000 }), 20_000), '≈ 3 dk')
  assert.equal(countdown(eta({ finishMs: 50_000 }), 20_000), '≈ 30 sn')
  assert.equal(countdown(eta({ overdue: true }), 0), null)
})

test('queue keys match the CLI', () => {
  assert.equal(queueAction('Enter'), 'approve')
  assert.equal(queueAction('ArrowRight'), 'approve')
  assert.equal(queueAction('ArrowLeft'), 'deny')
  assert.equal(queueAction('j'), 'next')
  assert.equal(queueAction('K'), 'prev')
  assert.equal(queueAction('Escape'), 'close')
  assert.equal(queueAction('a'), 'approveSimilar')
  assert.equal(queueAction('x'), null)
  assert.equal(move(2, 3, 'next'), 0)
  assert.equal(move(0, 3, 'prev'), 2)
  assert.equal(move(0, 0, 'next'), 0)
})

test('similar approvals and the PiP card', () => {
  const items = [item({ id: 'a', group: 'g1' }), item({ id: 'b', group: 'g1', owner: 'x' }), item({ id: 'c', group: 'g2', source: 'daemon', owner: 's' })]
  assert.deepEqual(similarIds(items, items[0]!), ['a', 'b'])
  assert.equal(approvalFor(items, 's')?.id, 'a')
  assert.equal(approvalFor(items, 'zz'), null)
  const older = session({ id: 'o', since: 1 })
  const newer = session({ id: 'n', since: 5 })
  assert.equal(primarySession([older, newer])?.id, 'n')
  assert.equal(primarySession([older, { ...newer, since: 0 }, { ...older, id: 'w', waiting: true }])?.id, 'w')
  assert.equal(primarySession([]), null)
  assert.equal(percent(session({ progress: [1, 4] })), 25)
  assert.equal(percent(session({ progress: [0, 0] })), null)
  assert.equal(percent(session({})), null)
})

test('notifications filter and tray status', () => {
  const list = [note({ id: '1', category: 'done', read: true }), note({ id: '2', category: 'budget' }), note({ id: '3', category: 'done' })]
  assert.equal(filterNotifications(list, 'all').length, 3)
  assert.deepEqual(filterNotifications(list, 'unread').map((n) => n.id), ['2', '3'])
  assert.deepEqual(filterNotifications(list, 'done').map((n) => n.id), ['1', '3'])
  assert.deepEqual(presentCategories(list), ['done', 'budget'])
  const base: ShellState = {
    overview: { desktop: true, sessions: [], daemon: { running: false, queued: 0, jobs: [] }, remote: [], approvals: [], groups: [], paused: false, now: 0 },
    unread: 0,
    update: null,
    recent: [],
    pip: { opacity: 1, visible: false },
    progress: null,
  }
  assert.equal(trayStatus(base), 'idle')
  assert.equal(trayStatus({ ...base, overview: { ...base.overview, sessions: [session({})] } }), 'working')
  assert.equal(trayStatus({ ...base, overview: { ...base.overview, approvals: [item({})] } }), 'waiting')
})

test('labels and inputs', () => {
  assert.equal(updatePillLabel('1.4.2'), 'Güncelle · v1.4.2')
  assert.equal(updatePillLabel('v1.4.2'), 'Güncelle · v1.4.2')
  assert.equal(folderName('C:\\work\\kor-agent'), 'kor-agent')
  assert.equal(folderName('/home/a/proj/'), 'proj')
  assert.ok(validQuietHours(''))
  assert.ok(validQuietHours('22-08'))
  assert.ok(!validQuietHours('24-1'))
  assert.ok(!validQuietHours('5-5'))
  assert.ok(!validQuietHours('akşam'))
})
