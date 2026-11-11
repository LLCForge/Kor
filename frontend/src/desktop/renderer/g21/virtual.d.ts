// [g21] Tür bildirimleri: pdf.js çalışan modülü tür dosyası yayımlamaz; çalışan kaynağı derleme sırasında
//      esbuild eklentisiyle (scripts/g21-bundles.ts) metin olarak sanal modülden gelir.
declare module 'pdfjs-dist/build/pdf.worker.mjs' {
  export const WorkerMessageHandler: unknown
}

declare module 'g21-pdf-worker-source' {
  const source: string
  export default source
}
