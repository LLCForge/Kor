import { execFile, type ChildProcess } from 'node:child_process'

/**
 * Bir sürecin TÜM alt ağacını öldürür. spawn({ signal }) yalnızca doğrudan çocuğu öldürür; Windows'ta önce kabuk ölürse
 * taskkill /T ölü PID'e gider ve torun süreç (ör. uzun süren `Get-ChildItem -Recurse`) sağ kalıp stdio borusunu açık tutar
 * (ölçülen: 2 sn sınırına karşın 20 sn). Bu yüzden çağıranlar spawn'a signal vermez, iptalde yalnızca bunu çağırır.
 * POSIX'te detached başlatılan süreçte tüm grup, değilse doğrudan çocuk öldürülür.
 */
export function killProcessTree(pid: number | undefined): void {
  if (!pid) return
  if (process.platform === 'win32') {
    execFile('taskkill', ['/pid', String(pid), '/t', '/f'], { windowsHide: true }, () => undefined)
    return
  }
  try {
    process.kill(-pid, 'SIGKILL')
  } catch {
    try {
      process.kill(pid, 'SIGKILL')
    } catch {
      // süreç zaten bitmiş
    }
  }
}

export const EXIT_GRACE_MS = 2_000

/**
 * Tamamlanma yalnızca 'close'a bağlı olamaz: arkada kalan torun (`start /b`, Popen) boruyu açık tuttukça 'close' hiç gelmez.
 * 'exit'ten sonra kısa bir süre son çıktı beklenir, sonra borular kapatılıp bitirilir. Dönen fonksiyon aynı süreyi
 * öldürme isteğinden itibaren başlatır: öldürme başarısız olsa da çağıran zamanında döner.
 */
export function whenChildDone(child: ChildProcess, done: (code: number | null, signal: NodeJS.Signals | null) => void, graceMs = EXIT_GRACE_MS): () => void {
  let finished = false
  let timer: NodeJS.Timeout | undefined
  const finish = () => {
    if (finished) return
    finished = true
    clearTimeout(timer)
    child.stdin?.destroy()
    child.stdout?.destroy()
    child.stderr?.destroy()
    child.unref()
    done(child.exitCode, child.signalCode)
  }
  const arm = () => {
    timer ??= setTimeout(finish, graceMs)
  }
  child.once('close', finish)
  child.once('exit', arm)
  return arm
}
