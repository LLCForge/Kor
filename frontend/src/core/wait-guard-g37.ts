// [g37] Ön planda "uyuyarak bekleme" engeli. Model bir işin bitmesini beklerken ping -n 120 127.0.0.1, timeout /t,
// sleep, Start-Sleep ile turu dakikalarca kilitliyordu; doğru yol arka plan + monitor_shell + bildirim.
import { currentShell } from './shell-session.ts'

/** Bundan uzun ön plan uykusu reddedilir; kısa beklemeler (sunucunun portu açması gibi) serbest. */
export const MAX_FOREGROUND_SLEEP_SECONDS = 10

const LOOPBACK = /^(127(?:\.\d{1,3}){3}|localhost|::1|\[::1\])$/i
// Arkasındaki sözcük asıl komuttur.
const PREFIX_WORDS = new Set(['@', 'call', 'do', 'then', 'else', 'exec', 'time'])

interface SimpleCommand {
  words: string[]
  /** POSIX'te & ile bitti: arka planda, turu kilitlemez. */
  background: boolean
}

/** Tırnak farkında ayrıştırma: tırnak içindeki ; & | uyku sayılmaz. Kapanmayan tırnak düz karakterdir. */
function parseCommands(command: string): SimpleCommand[] {
  const out: SimpleCommand[] = []
  let words: string[] = []
  let word = ''
  let inWord = false
  const endWord = () => {
    if (inWord) words.push(word)
    word = ''
    inWord = false
  }
  const endCommand = (background = false) => {
    endWord()
    if (words.length > 0) out.push({ words, background })
    words = []
  }
  for (let i = 0; i < command.length; i++) {
    const ch = command[i] ?? ''
    const next = command[i + 1] ?? ''
    if (ch === '"' || ch === "'") {
      const close = command.indexOf(ch, i + 1)
      if (close !== -1) {
        word += command.slice(i + 1, close)
        inWord = true
        i = close
        continue
      }
    }
    if ((ch === '^' || ch === '\\') && (next === '"' || next === "'" || next === '&' || next === '|' || next === ';')) {
      word += next
      inWord = true
      i++
      continue
    }
    if (/\s/.test(ch) && ch !== '\n') {
      endWord()
    } else if (ch === '\n' || ch === ';' || ch === '(' || ch === ')') {
      endCommand()
    } else if (ch === '|') {
      if (next === '|') i++
      endCommand()
    } else if (ch === '&') {
      if (next === '&') {
        i++
        endCommand()
      } else if (command[i - 1] === '>' || command[i - 1] === '<' || next === '>') {
        word += ch // 2>&1, &> yönlendirmedir
        inWord = true
      } else {
        endCommand(true)
      }
    } else {
      word += ch
      inWord = true
    }
  }
  endCommand()
  return out
}

const commandName = (word: string): string => (word.split(/[\\/]/).at(-1) ?? '').toLowerCase().replace(/\.(exe|com)$/, '')

/** cmd /c ..., powershell -c ..., bash -c ... içindeki asıl komut. */
function innerShellCommand(name: string, args: string[]): string | null {
  const marker =
    name === 'cmd'
      ? args.findIndex((w) => /^\/[ck]$/i.test(w))
      : name === 'powershell' || name === 'pwsh'
        ? args.findIndex((w) => /^-c(ommand)?$/i.test(w))
        : name === 'bash' || name === 'sh' || name === 'zsh'
          ? args.findIndex((w) => w === '-c' || w === '-lc')
          : -1
  return marker === -1 ? null : args.slice(marker + 1).join(' ')
}

function sleepOf(name: string, args: string[]): number {
  if (name === 'ping') {
    // Windows: -n N; POSIX: -c N. Yalnızca loopback'e atılan ping uyku sayılır (gerçek ağ testi değil).
    const target = args.find((w) => !w.startsWith('-') && !w.startsWith('/') && !w.startsWith('>') && !/^\d+$/.test(w))
    const countAt = args.findIndex((w) => /^[-/](n|c)$/i.test(w))
    const count = countAt >= 0 ? Number(args[countAt + 1]) : NaN
    return target && LOOPBACK.test(target) && Number.isFinite(count) ? Math.max(0, count - 1) : 0
  }
  if (name === 'timeout') {
    // Windows timeout /t N; POSIX "timeout 10 komut" bir sarmalayıcıdır, uyku değildir.
    const at = args.findIndex((w) => /^\/t$/i.test(w))
    return at >= 0 ? Number(args[at + 1]) || 0 : 0
  }
  const flagged = args.some((w) => w.startsWith('-'))
  if (name === 'start-sleep' || (name === 'sleep' && flagged)) {
    // PowerShell parametre kısaltmaları: -m, -ms, -mi, -milli..., -Milliseconds
    const ms = args.some((w) => /^-m(s|i\w*)?(:|$)/i.test(w))
    const value = Number(args.map((w) => /^(?:-\w+:)?(\d+(?:\.\d+)?)$/.exec(w)?.[1]).find((v) => v !== undefined))
    return Number.isFinite(value) ? (ms ? value / 1000 : value) : 0
  }
  if (name === 'sleep') {
    const m = /^(\d+(?:\.\d+)?)([smhd]?)$/.exec(args[0] ?? '')
    return m ? Number(m[1]) * ({ '': 1, s: 1, m: 60, h: 3600, d: 86400 } as const)[m[2] as '' | 's' | 'm' | 'h' | 'd'] : 0
  }
  return 0
}

/** Bu kabukta & komutu arka plana mı alır (cmd'de yalnızca sıralar). */
function ampersandBackgrounds(): boolean {
  const kind = currentShell().kind
  return kind === 'sh' || kind === 'git-bash' || kind === 'pwsh'
}

/** Komuttaki uyku/bekleme parçalarının toplam süresi (sn). Uyku yoksa 0. */
export function sleepSeconds(command: string, ampersandIsBackground = ampersandBackgrounds()): number {
  let total = 0
  for (const { words, background } of parseCommands(command)) {
    if (background && ampersandIsBackground) continue
    let start = 0
    while (start < words.length - 1 && PREFIX_WORDS.has((words[start] ?? '').toLowerCase())) start++
    const name = commandName((words[start] ?? '').replace(/^@/, ''))
    const args = words.slice(start + 1)
    const inner = innerShellCommand(name, args)
    // cmd /c içinde & sıralar; bash -c içinde arka plana alır.
    total += inner !== null ? sleepSeconds(inner, name === 'cmd' || name === 'powershell' ? false : name === 'pwsh' || name === 'bash' || name === 'sh' || name === 'zsh') : sleepOf(name, args)
  }
  return total
}

/** Uzun ön plan uykusu varsa modele gösterilecek ret mesajı, yoksa null. */
export function foregroundSleepBlock(command: string, ampersandIsBackground = ampersandBackgrounds()): string | null {
  const seconds = sleepSeconds(command, ampersandIsBackground)
  if (seconds <= MAX_FOREGROUND_SLEEP_SECONDS) return null
  return [
    `ÇALIŞTIRILMADI: bu komut ön planda yaklaşık ${Math.round(seconds)} sn uyuyarak bekliyor (ping/timeout/sleep/Start-Sleep) ve turu kilitliyor.`,
    'Bir şeyin bitmesini veya hazır olmasını beklemek için:',
    '1. Beklenen iş arka planda çalışmıyorsa onu bash run_in_background: true ile başlat.',
    '2. monitor_shell ile çıktısını izle (ör. desen "ready|done|error"); komut bitince veya desen eşleşince bildirim gelir.',
    '3. Beklemen gereken başka iş yoksa turu kısa bir durum cümlesiyle bitir: bildirim gelince otomatik olarak uyanıp devam edersin.',
    `Kısa bir bekleme gerçekten gerekiyorsa en fazla ${MAX_FOREGROUND_SLEEP_SECONDS} sn kullan. Uyku döngüsüyle yoklama (poll) yapma.`,
  ].join('\n')
}
