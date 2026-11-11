// esbuild '.svg': 'text' loader ile SVG dosyaları ham string olarak içe aktarılır.
declare module '*.svg' {
  const content: string
  export default content
}

// esbuild '.png': 'dataurl' loader ile küçük PNG logolar data URL olarak içe aktarılır (CSP img-src data:).
declare module '*.png' {
  const content: string
  export default content
}
