// `kor:osint/request` → arayüzde atılacak adımlar (saf; plan.test.ts). Kurallar (ADR 0052):
// - Bağlantıdan (origin "link") gelen proje açma / devam etme önce onay ister; bağlantı istemi asla kendiliğinden
//   gönderilmez, yalnızca yazma alanına konur.
// - Dosya yöneticisinden gelen hızlı görevler (quick) doğrudan gönderilir.
// - Aynı proje zaten açıksa klasör değiştirilmez (yeni sohbet de açılmaz), yalnızca dosyalar eklenir.
import type { OsintRequest } from '../../ipc-osint.ts'

export type OsintStep =
  | { do: 'confirm'; title: string; message: string }
  | { do: 'open'; root: string }
  | { do: 'attach'; files: string[] }
  | { do: 'send'; text: string }
  | { do: 'draft'; text: string }
  | { do: 'resume'; id: string }
  | { do: 'edit'; files: string[] }
  | { do: 'palette' }
  | { do: 'newChat' }
  | { do: 'running' }

const norm = (p: string) => p.replace(/[\\/]+$/, '').replace(/\\/g, '/').toLocaleLowerCase('en')

export function sameRoot(a: string | null, b: string | null): boolean {
  return a !== null && b !== null && norm(a) === norm(b)
}

function openSteps(root: string | null, current: string | null): OsintStep[] {
  return root !== null && !sameRoot(root, current) ? [{ do: 'open', root }] : []
}

export function planRequest(req: OsintRequest, current: string | null): OsintStep[] {
  switch (req.kind) {
    case 'open': {
      const steps: OsintStep[] = []
      if (req.origin === 'link') steps.push({ do: 'confirm', title: 'Proje açılsın mı?', message: `Bir bağlantı Kor'da şu klasörü açmak istiyor: ${req.root}` })
      steps.push(...openSteps(req.root, current))
      if (req.files.length > 0) steps.push({ do: 'attach', files: req.files })
      return steps
    }
    case 'task': {
      const steps: OsintStep[] = []
      const link = req.origin === 'link'
      if (link && req.root !== null && !sameRoot(req.root, current)) {
        steps.push({ do: 'confirm', title: 'Proje açılsın mı?', message: `Bir bağlantı Kor'da şu klasörü açmak istiyor: ${req.root}` })
      }
      steps.push(...openSteps(req.root, current))
      if (sameRoot(req.root, current) || req.root === null) steps.push({ do: 'newChat' })
      if (req.files.length > 0) steps.push({ do: 'attach', files: req.files })
      steps.push(link || req.prompt.trim() === '' ? { do: 'draft', text: req.prompt } : { do: 'send', text: req.prompt })
      return steps
    }
    case 'resume':
      return [
        ...(req.origin === 'link' ? [{ do: 'confirm', title: 'Sohbet açılsın mı?', message: `Bir bağlantı şu sohbeti açmak istiyor: ${req.id}` } as const] : []),
        { do: 'resume', id: req.id },
      ]
    case 'edit':
      return req.files.length > 0 ? [{ do: 'edit', files: req.files }] : []
    case 'palette':
      return [{ do: 'palette' }]
    case 'new':
      return [{ do: 'newChat' }]
    case 'running':
      return [{ do: 'running' }]
    case 'terminal':
      return []
  }
}
