# Third-party notices

## Claude's Hidden Toolkit

- Work: Claude's Hidden Toolkit: The Definitive Guide to Claude.ai's Undocumented Internal Tools
- Author: Francesco Marinoni Moretto
- Source: https://github.com/N1-AI/claude-hidden-toolkit
- Commit: 693c7c5c76392fe60722056941556465382a5563 (2026-04-07)
- License: Creative Commons Attribution 4.0 International (CC BY 4.0)
- License text: https://creativecommons.org/licenses/by/4.0/legalcode

Changes: Venice Agent does not copy the book. Local behavior equivalents were written from the documented tool contracts (inputs, outputs, edge cases). Names, hosts, and storage are ours. Google, Gmail, iOS Reminders, sports, image search, and the artifact execution API were not implemented. No telemetry or phone-home endpoints were taken from the source.

The licensor does not endorse this project.

## cdesktop (arayüz teması)

- Work: cdesktop — an open-source alternative to Claude Code Desktop
- Source: https://github.com/cdesktop-ai/cdesktop
- License: Apache License 2.0
- License text: https://www.apache.org/licenses/LICENSE-2.0

Changes: Venice Agent, cdesktop'un kaynak kodunu kopyalamaz. Yalnızca görsel tasarım
jetonları (renk paleti, nötr yüzeyler, turuncu marka vurgusu, köşe yarıçapı ölçeği ve
tipografi seçimi) `packages/web-core/src/app/styles/new/index.css` ve
`packages/local-web/tailwind.new.config.js` dosyalarından uyarlanmış, kor'ın kendi
CSS değişkenlerine (`src/desktop/renderer/styles/claude-theme.css`) eşlenmiştir. Hiçbir
React bileşeni, Rust kodu veya varlık (asset) alınmamıştır.

## Fraunces, Inter, JetBrains Mono (gömülü fontlar, spec §23.1, ADR 0053)

- Works: Fraunces (The Fraunces Project Authors, github.com/undercasetype/Fraunces), Inter (The Inter Project
  Authors, github.com/rsms/inter), JetBrains Mono (The JetBrains Mono Project Authors, github.com/JetBrains/JetBrainsMono)
- Distributed via: @fontsource-variable/fraunces 5.3.0, @fontsource-variable/inter 5.3.0,
  @fontsource-variable/jetbrains-mono 5.3.0 (Google Fonts kaynaklarının woff2 alt kümeleri; npm bütünlük
  karmaları doğrulandı)
- License: SIL Open Font License, Version 1.1 — metinler `src/desktop/renderer/fonts/*-OFL.txt`

Changes: yalnızca `latin` ve `latin-ext` (Türkçe İ ı Ş ş Ğ ğ) değişken woff2 alt kümeleri
`src/desktop/renderer/fonts/` altında değiştirilmeden gömülmüştür; CDN yoktur. SHA-256:

| Dosya | SHA-256 |
| --- | --- |
| fraunces-latin-opsz-normal.woff2 | 7234ed860a9cc83045413c4faee63c960a8f2d1917adcf728119307d56e0d783 |
| fraunces-latin-ext-opsz-normal.woff2 | a2930b27d13a228bd9ab6a49269b5f800237892ad560cb9dd7fab01b1620f88e |
| inter-latin-opsz-normal.woff2 | 2c295d99e26dcf357d4d01bcf270fd6924b600c9a13dd8c363ef114f4c6976fa |
| inter-latin-ext-opsz-normal.woff2 | 5e6d4fe9d9f4bff8b2a2469d25ab19576bb85331e22c6ed51398e16f95d56a9c |
| jetbrains-mono-latin-wght-normal.woff2 | 18be452724bfdc236c074ca94a249a7f41a86752c7d04ab258ce9ed5651f6a7e |
| jetbrains-mono-latin-ext-wght-normal.woff2 | 79bfdab9ba467e26eea4122e6f2567e188dd8a09a8c730d501fc487c4ab99c6e |

## Sağlayıcı/model marka ikonları (@lobehub/icons)

`src/desktop/renderer/g9/brand-svg/*.svg` dosyaları
[@lobehub/icons-static-svg](https://github.com/lobehub/lobe-icons) paketinden
alınmıştır (MIT License, © LobeHub). Bunlar DeepSeek, Qwen, Grok, Gemini, Meta,
NVIDIA, Black Forest Labs (Flux), Kimi/Moonshot, Zhipu (ChatGLM), OpenAI,
Anthropic, Mistral, Groq, OpenRouter, Ollama gibi sağlayıcı/model işaretleridir.
İşaretlerin kendisi ilgili şirketlerin **ticari markalarıdır** ve yalnızca ilgili
modeli/sağlayıcıyı tanımlamak (nominative use) için gösterilir; Venice Agent bu
şirketlerle bağlantılı değildir ve onlar tarafından onaylanmamıştır. Venice
wordmark'ı (`BrandIcons.tsx` içinde satır içi) Venice'e aittir.

## material-icon-theme (dosya tipi ikonları)

- Kaynak: https://github.com/material-extensions/vscode-material-icon-theme (npm `material-icon-theme`)
- Lisans: MIT — Copyright (c) 2021 Philipp Kief
- Kullanım: `src/desktop/renderer/file-icons/*.svg` (TS, JS, Python, Rust, … dosya ikonları; dosya kartı ve değişiklik ağacı)

## Simple Icons (MCP mağazası marka ikonları)

- Kaynak: https://github.com/simple-icons/simple-icons (npm `simple-icons`)
- Lisans: CC0 1.0 Universal (kamu malı). Markalar ilgili sahiplerinin tescilli markasıdır; yalnızca ilgili sunucuyu tanımlamak için kullanılır.
- Kullanım: `src/desktop/renderer/mcp/logos/si/*.svg` (Ayarlar > Geliştirici > MCP mağazası kartları)
