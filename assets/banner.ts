/** banner.ts — başlangıç ekranı (Claude Code'un karşılama kutusu gibi). */
import { emberGradient, ember, dim } from "./kor-spinner.js";
import { EMBER } from "./kor-theme.js";
import { readFileSync } from "node:fs";

export function printBanner(version: string, cwd = process.cwd(), model = "") {
  const art = readFileSync(new URL("./banner.txt", import.meta.url), "utf8").trimEnd().split("\n");
  const width = Math.max(...art.map((l) => l.length)) + 4;
  const line = (s = "") => "│ " + s + " ".repeat(Math.max(0, width - 2 - stripAnsi(s).length)) + " │";
  console.log(ember("╭" + "─".repeat(width) + "╮", EMBER.lo));
  for (const l of art) console.log(ember("│ ", EMBER.lo) + emberGradient(l.padEnd(width - 2)) + ember(" │", EMBER.lo));
  console.log(line(""));
  console.log(line(`${ember("✶", EMBER.hi)} Kor ${dim("v" + version)}${model ? dim(" · " + model) : ""}`));
  console.log(line(dim(cwd)));
  console.log(line(dim("/help komutlar · /exit çıkış")));
  console.log(ember("╰" + "─".repeat(width) + "╯", EMBER.lo));
}
const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
