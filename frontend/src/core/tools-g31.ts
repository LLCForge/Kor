// [g31] Uzak sunucu (SSH) ve veritabanı araçları. Bağlantıları kullanıcı Ayarlar > Uzak bağlantılar'da tanımlar; model
// yalnızca kimlikle (ör. "prod") başvurur, parola/anahtar görmez. Uzak komut ve yazma araçları yerel bash/yazma gibi onaya
// tabidir (kind exec/edit); izin kuralları "ssh:<kimlik> <komut>" konusuyla eşleşir.
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { z } from 'zod'
import { connectionId, findDbConnection, findSshHost, loadConnections, publicDbConnection, publicSshHost } from './connections.ts'
import { dbQuery, dbSchema, formatDbResult } from './db-remote.ts'
import { envTemplateReport } from './env-g13.ts'
import { formatExec, sftpDownload, sftpList, sftpReadText, sftpUpload, sftpWriteText, sshExec } from './ssh-remote.ts'
import { defineTool, resolveInWorkspace, type Tool } from './tools.ts'

const remotePath = z.string().min(1).max(4096).describe('Sunucudaki yol (mutlak ya da kullanıcının ev dizinine göre)')
const MAX_TOOL_OUTPUT = 60_000

const clip = (text: string) => (text.length > MAX_TOOL_OUTPUT ? `${text.slice(0, MAX_TOOL_OUTPUT)}\n[${text.length - MAX_TOOL_OUTPUT} karakter daha kesildi]` : text)

/** Veritabanı hedefi: tanımlı bağlantı kimliği ya da çalışma dizinindeki SQLite dosyası. */
const dbTarget = {
  connection: connectionId.optional().describe('Ayarlar\'da tanımlı veritabanı bağlantısının kimliği'),
  file: z.string().min(1).max(1024).optional().describe('Bağlantı yerine: çalışma dizinindeki SQLite dosyası (.db/.sqlite)'),
}

async function resolveDb(a: { connection?: string | undefined; file?: string | undefined }, root: string) {
  if (a.connection && a.file) throw new Error('connection ya da file verilmeli, ikisi birden değil.')
  if (a.file) return { db: { id: 'file', name: a.file, kind: 'sqlite' as const }, tunnel: null, sqliteFile: resolveInWorkspace(root, a.file) }
  if (!a.connection) throw new Error('connection (tanımlı bağlantı kimliği) veya file (SQLite dosyası) gerekli.')
  const { db, tunnel } = await findDbConnection(a.connection)
  return { db, tunnel, sqliteFile: db.kind === 'sqlite' && db.file ? resolveInWorkspace(root, db.file) : undefined }
}

export function g31Tools(): Tool[] {
  return [
    defineTool({
      name: 'remote_connections',
      description:
        'Kullanıcının tanımladığı SSH sunucularını ve veritabanı bağlantılarını listeler (kimlik, ad, adres, kimlik doğrulama türü, sunucu anahtarı onaylı mı). Parola gösterilmez. ssh_* ve db_* araçlarından önce kullan.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => 'uzak bağlantılar',
      subject: () => 'remote_connections',
      run: async () => {
        const c = await loadConnections()
        if (c.ssh.length === 0 && c.databases.length === 0) return 'Tanımlı uzak bağlantı yok. Kullanıcı Ayarlar > Uzak bağlantılar\'dan SSH sunucusu veya veritabanı ekleyebilir.'
        const ssh = c.ssh.map(publicSshHost).map((h) => `- ${h.id}: ${h.name} — ${h.username}@${h.host}:${h.port} (${h.auth}${h.hostKey ? '' : ', SUNUCU ANAHTARI ONAYSIZ: önce kullanıcı Ayarlar\'da test edip onaylamalı'})`)
        const dbs = c.databases.map(publicDbConnection).map((d) => `- ${d.id}: ${d.name} — ${d.kind}${d.kind === 'sqlite' ? ` ${d.file ?? ''}` : ` ${d.user ?? ''}@${d.host ?? '127.0.0.1'}:${d.port ?? ''}/${d.database ?? ''}`}${d.sshHost ? ` (SSH tüneli: ${d.sshHost})` : ''}`)
        return [ssh.length > 0 ? `SSH sunucuları:\n${ssh.join('\n')}` : '', dbs.length > 0 ? `Veritabanları:\n${dbs.join('\n')}` : ''].filter(Boolean).join('\n\n')
      },
    }),
    defineTool({
      name: 'ssh_exec',
      description:
        'Tanımlı SSH sunucusunda kabuk komutu çalıştırır (POSIX sh). Kurulum, servis yönetimi, log okuma, durum kontrolü için. ' +
        'sudo: true komutu sudo ile çalıştırır (parola kayıtlıysa güvenli biçimde verilir; komuta "sudo" yazma). ' +
        'Etkileşimli komutlar (vim, top, onay bekleyen apt) takılır: -y, --no-pager, -n gibi bayraklar kullan. Varsayılan zaman aşımı 120 sn.',
      schema: z.object({
        host: connectionId.describe('SSH sunucusu kimliği'),
        command: z.string().min(1).max(20_000),
        sudo: z.boolean().optional(),
        timeout_seconds: z.number().int().min(1).max(3600).optional(),
      }),
      kind: 'exec',
      summarize: (a) => `${a.host}: ${a.sudo ? 'sudo ' : ''}${a.command}`,
      subject: (a) => `ssh:${a.host} ${a.sudo ? 'sudo ' : ''}${a.command}`,
      run: async (a, ctx) => {
        const host = await findSshHost(a.host)
        const result = await sshExec(host, a.command, {
          signal: ctx.signal,
          ...(a.sudo ? { sudo: true } : {}),
          ...(a.timeout_seconds ? { timeoutMs: a.timeout_seconds * 1000 } : {}),
        })
        return clip(formatExec(result))
      },
    }),
    defineTool({
      name: 'ssh_list',
      description: 'SSH sunucusunda bir klasörün içeriğini listeler (tür, boyut, değişiklik zamanı).',
      schema: z.object({ host: connectionId, path: remotePath }),
      kind: 'read',
      summarize: (a) => `${a.host}:${a.path}`,
      subject: (a) => `ssh:${a.host} ${a.path}`,
      run: async (a) => {
        const { entries, truncated } = await sftpList(await findSshHost(a.host), a.path)
        if (entries.length === 0) return 'Klasör boş.'
        const lines = entries.map((e) => `${e.type === 'dir' ? 'd' : e.type === 'link' ? 'l' : '-'} ${String(e.size).padStart(10)} ${e.modified} ${e.name}${e.type === 'dir' ? '/' : ''}`)
        return clip(`${lines.join('\n')}${truncated ? '\n[liste sınırda kesildi]' : ''}`)
      },
    }),
    defineTool({
      name: 'ssh_read_file',
      description: 'SSH sunucusundaki metin dosyasını okur (en fazla 2 MB). Büyük loglar için ssh_exec ile tail/grep kullan.',
      schema: z.object({ host: connectionId, path: remotePath }),
      kind: 'read',
      summarize: (a) => `${a.host}:${a.path}`,
      subject: (a) => `ssh:${a.host} ${a.path}`,
      run: async (a) => clip(await sftpReadText(await findSshHost(a.host), a.path)),
    }),
    defineTool({
      name: 'ssh_write_file',
      description: 'SSH sunucusundaki dosyayı verilen içerikle yazar (yoksa oluşturur). Yarım dosya kalmaması için geçici dosyaya yazıp yerine taşır. Yetki gereken yollar için ssh_exec + sudo kullan.',
      schema: z.object({ host: connectionId, path: remotePath, content: z.string().max(5_000_000) }),
      kind: 'edit',
      summarize: (a) => `${a.host}:${a.path} (${a.content.length} karakter)`,
      subject: (a) => `ssh:${a.host} ${a.path}`,
      run: async (a) => {
        await sftpWriteText(await findSshHost(a.host), a.path, a.content)
        return `Yazıldı: ${a.host}:${a.path}`
      },
    }),
    defineTool({
      name: 'ssh_upload',
      description: 'Çalışma dizinindeki bir dosyayı SSH sunucusuna yükler (en fazla 500 MB).',
      schema: z.object({ host: connectionId, local_path: z.string().min(1).max(1024), remote_path: remotePath }),
      kind: 'edit',
      summarize: (a) => `${a.local_path} → ${a.host}:${a.remote_path}`,
      subject: (a) => `ssh:${a.host} ${a.remote_path}`,
      run: async (a, ctx) => {
        const size = await sftpUpload(await findSshHost(a.host), resolveInWorkspace(ctx.root, a.local_path), a.remote_path)
        return `Yüklendi: ${a.local_path} → ${a.host}:${a.remote_path} (${size} bayt)`
      },
    }),
    defineTool({
      name: 'ssh_download',
      description: 'SSH sunucusundaki bir dosyayı çalışma dizinine indirir (en fazla 500 MB).',
      schema: z.object({ host: connectionId, remote_path: remotePath, local_path: z.string().min(1).max(1024) }),
      kind: 'edit',
      summarize: (a) => `${a.host}:${a.remote_path} → ${a.local_path}`,
      subject: (a) => a.local_path,
      run: async (a, ctx) => {
        const target = resolveInWorkspace(ctx.root, a.local_path)
        await mkdir(path.dirname(target), { recursive: true })
        const size = await sftpDownload(await findSshHost(a.host), a.remote_path, target)
        return `İndirildi: ${a.host}:${a.remote_path} → ${a.local_path} (${size} bayt)`
      },
    }),
    defineTool({
      name: 'env_check',
      description:
        'Projenin .env / .env.local dosyalarını şablonla (.env.example, .env.template, .env.sample) karşılaştırır: eksik, boş ve şablonda olmayan değişkenlerin YALNIZCA ADLARINI döner. ' +
        'Değerler hiçbir zaman dönmez (.env okunamaz). Uygulama "X tanımlı değil" hatası verdiğinde veya kurulum sonrası kullan; eksikleri kullanıcıdan Ayarlar/.env yöneticisinden girmesini iste.',
      schema: z.object({}),
      kind: 'read',
      summarize: () => '.env şablon denetimi',
      subject: () => 'env_check',
      run: async (_a, ctx) => {
        const r = await envTemplateReport(ctx.root)
        if (!r.template) return 'Şablon dosyası yok (.env.example). Kullanıcı .env yöneticisinden "Şablon oluştur" ile üretebilir.'
        const lines = [`Şablon: ${r.template}`]
        lines.push(r.missing.length > 0 ? `Eksik: ${r.missing.join(', ')}` : 'Eksik değişken yok.')
        if (r.empty.length > 0) lines.push(`Boş: ${r.empty.join(', ')}`)
        if (r.extra.length > 0) lines.push(`Şablonda olmayan: ${r.extra.join(', ')}`)
        return lines.join('\n')
      },
    }),
    defineTool({
      name: 'db_schema',
      description: 'Veritabanının tablolarını ve sütunlarını (tip, NULL olabilir mi) listeler. Sorgu yazmadan önce kullan.',
      schema: z.object(dbTarget),
      kind: 'read',
      summarize: (a) => a.connection ?? a.file ?? '',
      subject: (a) => `db:${a.connection ?? a.file ?? ''}`,
      run: async (a, ctx) => {
        const { db, tunnel, sqliteFile } = await resolveDb(a, ctx.root)
        return clip(await dbSchema(db, { tunnel, ...(sqliteFile ? { sqliteFile } : {}) }))
      },
    }),
    defineTool({
      name: 'db_query',
      description:
        'Veritabanında SALT OKUNUR SQL çalıştırır (yalnızca SELECT/WITH/VALUES/EXPLAIN, tek ifade; sunucuda salt okunur işlemde, sonunda geri alınır). ' +
        'En fazla 1000 satır döner; büyük tablolar için WHERE/LIMIT/GROUP BY kullan. Veri değiştirmek gerekiyorsa kullanıcıya SQL\'i öner.',
      schema: z.object({ ...dbTarget, sql: z.string().min(1).max(100_000) }),
      kind: 'read',
      summarize: (a) => `${a.connection ?? a.file ?? ''}: ${a.sql.replace(/\s+/g, ' ').slice(0, 160)}`,
      subject: (a) => `db:${a.connection ?? a.file ?? ''}`,
      run: async (a, ctx) => {
        const { db, tunnel, sqliteFile } = await resolveDb(a, ctx.root)
        return clip(formatDbResult(await dbQuery(db, a.sql, { tunnel, ...(sqliteFile ? { sqliteFile } : {}) })))
      },
    }),
  ]
}
