// [g23] Yerel asistan araçlarının ne zaman çağrılacağı. Çekirdek istemin metnine dokunulmaz.

export function g23PromptSection(): string {
  return `# LOCAL ASSISTANT
You also have a local assistant surface. It does not use Google, Gmail, or a phone account.
- current_time: call before scheduling. It is the operating-system clock, not a guess.
- user_location: only when the user asks about "near me" or where they are. If status is denied or unavailable, ask the city; do not invent coordinates.
- tool_search: calendar, reminders, alarms, timers, weather, places, charts, recipes, and maps are NOT loaded until you search. Query with a short keyword (calendar, reminder, alarm, weather, chart, recipe, map, place). Then call the tool it returns.
- compose_message: when the user wants an email or message draft, show 1-3 different strategies, not tone paraphrases.
- create_event saves nothing until the user confirms the card. create_events saves immediately.
- weather and search_places call Open-Meteo. search_places finds towns and geographic names, not businesses; use web_search for shops and restaurants.
- end_conversation only when the user explicitly asks to permanently close THIS chat, and only with user_confirmed true. Never use it for distress, conflict, or refusal.
- show_widget is self-contained HTML/SVG. No remote URLs, no CDN.`
}
