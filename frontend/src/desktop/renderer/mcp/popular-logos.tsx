// MCP mağazası logoları: cdesktop mcp/ seti (renkli) + Simple Icons (CC0, tek renk → marka renkli kutuda beyaz).
// SVG'ler esbuild text loader'ı ile ham metin, PNG data URL. Anahtar yoksa metin rozeti (ör. "VK").
import type { ReactElement } from 'react'
import chromeDevtoolsSvg from './logos/chrome_devtools_logo.svg'
import context7Png from './logos/context7.png'
import devManagerSvg from './logos/dev_manager_logo.svg'
import exaSvg from './logos/exa_logo.svg'
import playwrightSvg from './logos/playwright_logo_icon.svg'
import airtableSvg from './logos/si/airtable.svg'
import braveSvg from './logos/si/brave.svg'
import cloudflareSvg from './logos/si/cloudflare.svg'
import figmaSvg from './logos/si/figma.svg'
import githubSvg from './logos/si/github.svg'
import googlemapsSvg from './logos/si/googlemaps.svg'
import huggingfaceSvg from './logos/si/huggingface.svg'
import jiraSvg from './logos/si/jira.svg'
import linearSvg from './logos/si/linear.svg'
import mongodbSvg from './logos/si/mongodb.svg'
import notionSvg from './logos/si/notion.svg'
import postgresqlSvg from './logos/si/postgresql.svg'
import puppeteerSvg from './logos/si/puppeteer.svg'
import sentrySvg from './logos/si/sentry.svg'
import shadcnuiSvg from './logos/si/shadcnui.svg'
import stripeSvg from './logos/si/stripe.svg'
import supabaseSvg from './logos/si/supabase.svg'
import todoistSvg from './logos/si/todoist.svg'
import vercelSvg from './logos/si/vercel.svg'

/** Tam renkli logolar (kendi renkleriyle, nötr kutu) */
const FULL: Record<string, string> = {
  playwright: playwrightSvg,
  exa: exaSvg,
  chrome_devtools: chromeDevtoolsSvg,
  dev_manager: devManagerSvg,
}
const IMG: Record<string, string> = { context7: context7Png }
/** Tek renkli Simple Icons: marka arka planı + beyaz ikon */
const MONO: Record<string, { svg: string; bg: string }> = {
  github: { svg: githubSvg, bg: '#24292f' },
  notion: { svg: notionSvg, bg: '#1f1f1f' },
  figma: { svg: figmaSvg, bg: '#a259ff' },
  postgresql: { svg: postgresqlSvg, bg: '#336791' },
  supabase: { svg: supabaseSvg, bg: '#3ecf8e' },
  stripe: { svg: stripeSvg, bg: '#635bff' },
  sentry: { svg: sentrySvg, bg: '#362d59' },
  linear: { svg: linearSvg, bg: '#5e6ad2' },
  brave: { svg: braveSvg, bg: '#fb542b' },
  cloudflare: { svg: cloudflareSvg, bg: '#f38020' },
  vercel: { svg: vercelSvg, bg: '#000000' },
  jira: { svg: jiraSvg, bg: '#0052cc' },
  googlemaps: { svg: googlemapsSvg, bg: '#4285f4' },
  puppeteer: { svg: puppeteerSvg, bg: '#40b5a4' },
  mongodb: { svg: mongodbSvg, bg: '#47a248' },
  airtable: { svg: airtableSvg, bg: '#18bfff' },
  todoist: { svg: todoistSvg, bg: '#e44332' },
  huggingface: { svg: huggingfaceSvg, bg: '#ffd21e' },
  shadcnui: { svg: shadcnuiSvg, bg: '#000000' },
}
/** Metin rozetleri için marka renkleri */
const TEXT_BG: Record<string, string> = { rea: '#7c3aed', vk: '#2f54eb', mem: '#0e7490', seq: '#b45309', dw: '#1d4ed8', tav: '#2563eb', fc: '#ea580c', slk: '#4a154b' }

export function PopularLogo({ logo }: { logo: string }): ReactElement {
  if (FULL[logo]) return <span className="mcp-popular-logo" aria-hidden="true" dangerouslySetInnerHTML={{ __html: FULL[logo] }} />
  if (IMG[logo]) return <span className="mcp-popular-logo" aria-hidden="true"><img src={IMG[logo]} alt="" /></span>
  const mono = MONO[logo]
  if (mono) return <span className="mcp-popular-logo mono" aria-hidden="true" style={{ background: mono.bg }} dangerouslySetInnerHTML={{ __html: mono.svg }} />
  return <span className="mcp-popular-logo text" aria-hidden="true" style={{ background: TEXT_BG[logo.toLowerCase()] ?? '#2f54eb' }}>{logo}</span>
}
