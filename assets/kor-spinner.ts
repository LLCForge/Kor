/**
 * kor-spinner.ts — Claude Code tarzı "düşünüyor" göstergesi (Node ≥ 18 / Bun).
 * Bağımlılık yok. ANSI truecolor; desteklenmeyen terminalde 256-renk sarıya düşer.
 *
 *   const sp = createSpinner({ lang: "tr" });
 *   sp.start();                 // ✶ Közlüyor… (3s · ↑ 1.2k token)
 *   sp.update({ tokens: 1200 });
 *   sp.succeed("Hazır");        // ✔ Hazır (4.2s)
 */
import { VERBS_TR, VERBS_EN, FRAMES, GRADIENT, STATUS } from "./kor-theme.js";

export type SpinnerOptions = {
  lang?: "tr" | "en";
  frames?: keyof typeof FRAMES | string[];
  verbs?: string[];
  intervalMs?: number;      // frame interval (default 80)
  verbEveryMs?: number;     // rotate the verb (default 4000)
  stream?: NodeJS.WriteStream;
  showElapsed?: boolean;
  colors?: boolean;         // force on/off; default: auto (TTY && !NO_COLOR)
};

const ESC = "\x1b[";
const hide = `${ESC}?25l`, show = `${ESC}?25h`, clearLine = `${ESC}2K\r`;

function hexToRgb(h: string): [number, number, number] {
  const n = parseInt(h.slice(1), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}
function truecolor(): boolean {
  const ct = process.env.COLORTERM ?? ""; return /truecolor|24bit/i.test(ct) || !!process.env.WT_SESSION || process.env.TERM_PROGRAM === "iTerm.app" || process.env.TERM_PROGRAM === "vscode";
}
export function ember(text: string, hex: string, enabled = true): string {
  if (!enabled) return text;
  if (truecolor()) { const [r, g, b] = hexToRgb(hex); return `${ESC}38;2;${r};${g};${b}m${text}${ESC}39m`; }
  return `${ESC}38;5;214m${text}${ESC}39m`;               // 256-colour orange fallback
}
export function emberGradient(text: string, enabled = true): string {
  if (!enabled) return text;
  return [...text].map((ch, i) => ember(ch, GRADIENT[i % GRADIENT.length])).join("");
}
export const dim = (s: string, on = true) => (on ? `${ESC}2m${s}${ESC}22m` : s);
export const fmtTokens = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));

export function createSpinner(opts: SpinnerOptions = {}) {
  const stream = opts.stream ?? process.stderr;
  const colors = opts.colors ?? (stream.isTTY && !process.env.NO_COLOR);
  const frames = Array.isArray(opts.frames) ? opts.frames : FRAMES[opts.frames ?? "ember"];
  const verbs = opts.verbs ?? (opts.lang === "en" ? VERBS_EN : VERBS_TR);
  const interval = opts.intervalMs ?? 80, verbEvery = opts.verbEveryMs ?? 4000;
  let timer: NodeJS.Timeout | undefined, frame = 0, started = 0, tokens = 0, note = "";
  let verb = verbs[Math.floor(Math.random() * verbs.length)];

  const render = () => {
    const f = frames[frame % frames.length]; frame++;
    const col = GRADIENT[frame % GRADIENT.length];
    const secs = Math.round((Date.now() - started) / 1000);
    const parts = [opts.showElapsed === false ? "" : `${secs}s`, tokens ? `↑ ${fmtTokens(tokens)} token` : "", note].filter(Boolean);
    const meta = parts.length ? dim(` (${parts.join(" · ")})`, colors) : "";
    if ((frame * interval) % verbEvery < interval) verb = verbs[Math.floor(Math.random() * verbs.length)];
    stream.write(`${clearLine}${ember(f, col, colors)} ${ember(verb + "…", GRADIENT[2], colors)}${meta}`);
  };
  const stop = (symbol: string, text: string, hex: string) => {
    if (timer) clearInterval(timer); timer = undefined;
    const secs = ((Date.now() - started) / 1000).toFixed(1);
    stream.write(`${clearLine}${ember(symbol, hex, colors)} ${text} ${dim(`(${secs}s)`, colors)}\n${show}`);
  };
  return {
    start() { if (timer) return this; started = Date.now(); stream.write(hide); render(); timer = setInterval(render, interval); return this; },
    update(p: { tokens?: number; note?: string; verb?: string }) { if (p.tokens !== undefined) tokens = p.tokens; if (p.note !== undefined) note = p.note; if (p.verb) verb = p.verb; return this; },
    succeed(text = "Tamam") { stop("✔", text, STATUS.success); },
    fail(text = "Hata") { stop("✖", text, STATUS.error); },
    stop(text = "") { stop("•", text, GRADIENT[2]); },
    get running() { return !!timer; },
  };
}

/** Birden fazla paralel adım için (alt ajanlar) — her satırda ayrı spinner. */
export function createMultiSpinner(labels: string[], opts: SpinnerOptions = {}) {
  const stream = opts.stream ?? process.stderr; const colors = opts.colors ?? (stream.isTTY && !process.env.NO_COLOR);
  const frames = FRAMES.braille; const state = labels.map((l) => ({ label: l, done: false as false | "ok" | "fail" }));
  let frame = 0; stream.write(hide + labels.map(() => "\n").join(""));
  const draw = () => {
    stream.write(`${ESC}${labels.length}A`);
    for (const s of state) {
      const sym = s.done === "ok" ? ember("✔", STATUS.success, colors) : s.done === "fail" ? ember("✖", STATUS.error, colors) : ember(frames[frame % frames.length], GRADIENT[frame % GRADIENT.length], colors);
      stream.write(`${clearLine}  ${sym} ${s.label}\n`);
    }
    frame++;
  };
  const timer = setInterval(draw, 80);
  return {
    done(i: number, ok = true) { state[i].done = ok ? "ok" : "fail"; if (state.every((s) => s.done)) { clearInterval(timer); draw(); stream.write(show); } },
  };
}
