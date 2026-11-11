// [g16] Ayrı paket (dist/renderer/katex.js): sohbette ilk matematik ifadesi görününce yüklenir (bkz. MathView.tsx).
// KaTeX CSS'i ve yazı tipleri ana renderer.css'te kalır; yazı tiplerini tarayıcı zaten yalnızca kullanılınca indirir.
import katex from 'katex'

;(window as { __g16Katex?: typeof katex }).__g16Katex = katex
