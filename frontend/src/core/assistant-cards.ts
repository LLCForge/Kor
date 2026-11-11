// Araç kartlarının paylaştığı şemalar. Yalnızca zod: arayüz bu modülü çeker, dosya ve süreç çekmez.
import { z } from 'zod'

const WEEKDAYS = ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'] as const

export const chartArgs = z.object({
  series: z
    .array(
      z.object({
        name: z.string().min(1).max(80),
        data: z.array(z.number().finite()).min(1).max(500).optional(),
        values: z.array(z.number().finite()).min(1).max(500).optional(),
        color: z.string().regex(/^#[0-9A-Fa-f]{6}$/).optional(),
      }),
    )
    .min(1)
    .max(12),
  style: z.enum(['line', 'bar', 'scatter']),
  title: z.string().max(120).optional(),
  x_axis: z.object({ label: z.string().max(80).optional(), categories: z.array(z.string().max(40)).max(500).optional() }).optional(),
  y_axis: z.object({ label: z.string().max(80).optional(), scale: z.enum(['linear', 'log']).optional() }).optional(),
})
export type ChartArgs = z.infer<typeof chartArgs>

export const composeArgs = z.object({
  kind: z.enum(['email', 'textMessage', 'other']),
  summary_title: z.string().min(1).max(120),
  variants: z
    .array(
      z.object({
        label: z.string().min(1).max(60),
        body: z.string().min(1).max(8000),
        subject: z.string().max(200).optional(),
      }),
    )
    .min(1)
    .max(3),
})
export type ComposeArgs = z.infer<typeof composeArgs>

export const recipeArgs = z.object({
  title: z.string().min(1).max(120),
  ingredients: z
    .array(
      z.object({
        id: z.string().min(1).max(32),
        name: z.string().min(1).max(80),
        amount: z.number().finite().nonnegative(),
        unit: z.enum(['g', 'kg', 'ml', 'l', 'tsp', 'tbsp', 'cup', 'fl_oz', 'oz', 'lb', 'pinch', 'piece', '']),
      }),
    )
    .min(1)
    .max(40),
  steps: z
    .array(
      z.object({
        id: z.string().min(1).max(32),
        title: z.string().min(1).max(120),
        content: z.string().min(1).max(2000),
        timer_seconds: z.number().int().positive().max(86_400).optional(),
      }),
    )
    .min(1)
    .max(30),
  base_servings: z.number().int().min(1).max(100).optional(),
  description: z.string().max(300).optional(),
  notes: z.string().max(1000).optional(),
})
export type RecipeArgs = z.infer<typeof recipeArgs>

export const mapArgs = z.object({
  title: z.string().max(120).optional(),
  markers: z
    .array(
      z.object({
        lat: z.number().min(-90).max(90),
        lon: z.number().min(-180).max(180),
        label: z.string().max(120).optional(),
      }),
    )
    .min(1)
    .max(30),
})
export type MapArgs = z.infer<typeof mapArgs>

export const widgetArgs = z.object({
  title: z.string().min(1).max(120),
  widget_code: z.string().min(1).max(32_000),
  loading_messages: z.array(z.string().max(120)).max(4).optional(),
  acknowledged: z.literal(true).describe('Uzak betik yüklemeyeceğini onayla'),
})
export type WidgetArgs = z.infer<typeof widgetArgs>

export const eventDraftArgs = z.object({
  title: z.string().min(1).max(200),
  start_time: z.string().min(1).max(40),
  end_time: z.string().max(40).optional(),
  all_day: z.boolean().optional(),
  description: z.string().max(4000).optional(),
  location: z.string().max(300).optional(),
  calendar_id: z.string().max(20).optional(),
  recurrence: z
    .object({
      frequency: z.string().min(1).max(40),
      human_readable_frequency: z.string().min(1).max(80),
      rrule: z.string().min(1).max(300),
    })
    .optional(),
})
export type EventDraftArgs = z.infer<typeof eventDraftArgs>

export const alarmArgs = z.object({
  hour: z.number().int().min(0).max(23),
  minute: z.number().int().min(0).max(59),
  message: z.string().min(1).max(200),
  days: z.array(z.enum(WEEKDAYS)).max(7).optional(),
  vibrate: z.boolean().optional(),
})

export const weatherView = z.object({
  location_name: z.string(),
  current: z.object({
    temp_c: z.number(),
    temp_f: z.number(),
    label: z.string(),
    precipitation_mm: z.number(),
    wind_kmh: z.number(),
  }),
  days: z.array(
    z.object({
      date: z.string(),
      label: z.string(),
      max_c: z.number(),
      min_c: z.number(),
      precip_probability: z.number(),
    }),
  ),
})
export function scaleAmount(amount: number, baseServings: number, servings: number): number {
  if (baseServings <= 0) return amount
  return Math.round((amount * servings * 100) / baseServings) / 100
}

export interface IngredientRef {
  id: string
  name: string
  amount: number
  unit: string
}

/** Adımdaki {id} gömmelerini ölçeklenmiş miktarla değiştirir. */
export function renderStep(content: string, ingredients: readonly IngredientRef[], baseServings: number, servings: number): string {
  return content.replace(/\{([A-Za-z0-9_-]+)\}/g, (raw, id: string) => {
    const item = ingredients.find((i) => i.id === id)
    if (!item) return raw
    const amount = scaleAmount(item.amount, baseServings, servings)
    return `${amount}${item.unit ? ` ${item.unit}` : ''} ${item.name}`
  })
}
