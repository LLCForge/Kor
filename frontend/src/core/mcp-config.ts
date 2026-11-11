// MCP sunucu yapılandırması — saf şema (zod). SDK, Node fs ve süreç içermez; hem çekirdek hem arayüz güvenle kullanır.
// Sunucular kullanıcı düzeyindedir (depo dosyası MCP sunucusu kaydedemez: proje otomatik araç/süreç ekleyemesin).
import { z } from 'zod'

const idField = z
  .string()
  .min(1)
  .max(60)
  .regex(/^[A-Za-z0-9][A-Za-z0-9_-]*$/, 'Sunucu kimliği harf/rakamla başlamalı; yalnızca harf, rakam, _ ve - içerebilir')
const label = z.string().max(80).optional()
const enabled = z.boolean().default(false)
const stringMap = z.record(z.string().max(100), z.string().max(8000)).default({})

export const mcpServerSchema = z.discriminatedUnion('transport', [
  z.object({
    transport: z.literal('stdio'),
    id: idField,
    label,
    enabled,
    command: z.string().min(1).max(500),
    args: z.array(z.string().max(4000)).max(100).default([]),
    env: stringMap,
  }),
  z.object({
    transport: z.literal('http'),
    id: idField,
    label,
    enabled,
    url: z.url({ protocol: /^https?$/ }).max(2048),
    headers: stringMap,
  }),
])
export type McpServerConfig = z.infer<typeof mcpServerSchema>

/** Kimlikler benzersiz olmalı: araç adı mcp__<id>__<araç> olduğundan çakışan kimlik araçları gölgeler. */
export const mcpConfigSchema = z
  .array(mcpServerSchema)
  .max(50)
  .refine((list) => new Set(list.map((s) => s.id)).size === list.length, 'MCP sunucu kimlikleri benzersiz olmalı')
