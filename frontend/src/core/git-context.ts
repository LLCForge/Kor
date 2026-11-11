// Otomatik bağlam: oturum başında git durumu anlık görüntüsü (dal, son commit, değişen dosyalar) sistem istemine girer;
// ajan her sohbette "git status" çalıştırmak zorunda kalmaz. Anlık görüntüdür, sohbet boyunca güncellenmez.
// Güvenlik: yalnızca güvenilen klasörde çağrılır (depoyla gelen .git/config'teki fsmonitor/filtre komutları git status
// sırasında çalışabilir); ek olarak fsmonitor kapatılır. Kısa zaman aşımı ve çıktı sınırı vardır.
import { execFile } from 'node:child_process'
import { gitExePath } from './git-path.ts'

const TIMEOUT_MS = 3000
const MAX_STATUS_LINES = 30

export interface GitSnapshot {
  branch: string
  lastCommit: string | null
  status: string[]
  /** status satırlarından kaç tanesi gösterilmedi */
  more: number
}

function git(root: string, args: readonly string[]): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(
      gitExePath(),
      ['-c', 'core.fsmonitor=false', '--no-optional-locks', ...args],
      { cwd: root, timeout: TIMEOUT_MS, maxBuffer: 512 * 1024, windowsHide: true, encoding: 'utf8' },
      (err, stdout) => resolve(err ? null : stdout),
    )
  })
}

/** Klasör bir git deposu değilse veya git yoksa/yanıt vermezse null. */
export async function gitSnapshot(root: string): Promise<GitSnapshot | null> {
  const status = await git(root, ['status', '--porcelain=v1', '--branch'])
  if (status === null) return null
  const lines = status.split('\n').filter(Boolean)
  const head = lines[0]?.startsWith('## ') ? (lines.shift() ?? '').slice(3) : ''
  const branch = head.replace(/^No commits yet on /, '').split('...')[0]?.trim() || '(bilinmiyor)'
  const lastCommit = (await git(root, ['log', '-1', '--format=%h %s']))?.trim() || null
  return { branch, lastCommit, status: lines.slice(0, MAX_STATUS_LINES), more: Math.max(0, lines.length - MAX_STATUS_LINES) }
}

export function gitSnapshotSection(s: GitSnapshot): string {
  return [
    '# GIT (snapshot at session start; run git status for the current state)',
    `Branch: ${s.branch}`,
    `Last commit: ${s.lastCommit ?? '(none)'}`,
    s.status.length > 0 ? `Changes:\n${s.status.join('\n')}${s.more > 0 ? `\n... ${s.more} more` : ''}` : 'Working tree clean.',
  ].join('\n')
}
