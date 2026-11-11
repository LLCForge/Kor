// [g21] pdf.js çalışanı (klasik IIFE betiği). Renderer bunu blob: URL'den başlatır; blob çalışanı belgenin
//      CSP'sini devralır (connect-src 'none' => çalışandan da ağ yok). Ek savunma: ağ API'leri kaldırılır,
//      böylece ayrıştırıcıda olası bir kod yürütme bile dışarı veri gönderemez.
import { WorkerMessageHandler } from 'pdfjs-dist/build/pdf.worker.mjs'

const scope = globalThis as unknown as Record<string, unknown>
for (const name of ['fetch', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'WebTransport', 'importScripts', 'Worker', 'SharedWorker']) {
  try {
    delete scope[name]
    if (name in scope) scope[name] = undefined
  } catch {
    // Silinemeyen özellik (salt okunur): CSP katmanı yine ağı engeller.
  }
}
// pdf.js ana iş parçacığı sahte çalışana düşerse bu nesneyi arar; çalışan modülünün paketten atılmamasını da sağlar.
scope.pdfjsWorker = { WorkerMessageHandler }
