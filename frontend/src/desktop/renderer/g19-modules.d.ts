// [g19] Paketli Natural Earth 1:110m ülke sınırları (world-atlas, TopoJSON). tsconfig'te resolveJsonModule
//      kapalı olduğundan JSON modülünün türü burada bildirilir; esbuild ve Node JSON içe aktarmayı yerel çözer.
declare module 'world-atlas/countries-110m.json' {
  import type { GeometryCollection, Topology } from 'topojson-specification'
  const topology: Topology<{ countries: GeometryCollection<{ name: string }>; land: GeometryCollection }>
  export default topology
}
