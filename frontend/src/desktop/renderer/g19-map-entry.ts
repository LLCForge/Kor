// [g19] Ayrı paket (dist/renderer/g19-map.js): yalnızca sohbette ```map / ```geojson bloğu görününce yüklenir.
//      d3-geo + topojson-client + paketli world-atlas verisi ana pakete girmez.
import type { G19MapApi } from './g19-geo.ts'
import { buildScene } from './g19-map-scene.ts'

declare global {
  interface Window {
    __g19Map?: G19MapApi
  }
}

window.__g19Map = { buildScene }
