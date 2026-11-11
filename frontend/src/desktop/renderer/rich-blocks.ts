// [temel] Zengin blok kayıtları burada toplanır. Markdown bu dosyayı yan etki için içe aktarır; her blok modülü
//         yüklenince registerRichBlock(...) çağırıp kendini kaydeder. Yeni bir grup kendi modülünü bu listeye ekler.
import './MermaidBlock.tsx' // ```mermaid
// [g8] Sandbox'lı HTML/SVG güvenli önizleme blokları
import './HtmlBlock.tsx' // ```html
import './SvgBlock.tsx' // ```svg
import './TableBlock.tsx' // [g8] ```table ```csv (sıralama/süzme/sayfalama/dışa aktarma/düzenleme)
import './CalcForm.tsx' // [g8] ```calc ```form (hesap makinesi + slider, form önizleme)
import './ContentBlocks.tsx' // [g8] ```tabs ```wizard ```steps ```todo ```gallery ```cards ```timeline ```compare
import './DataBlocks.tsx' // [g8] ```playground ```json ```diff
import './WidgetBlocks.tsx' // [g8] ```palette ```colors ```icons ```quiz ```progress ```glossary
import './SvgDiagramBlock.tsx' // ```dot / ```graphviz / ```nomnoml / ```wavedrom
import './PlantUmlBlock.tsx' // ```plantuml / ```puml (yerel olarak Mermaid'e çevrilir)
import './AsciiDiagram.tsx' // ```ascii ve etiketsiz kutu-çizgi şemalar
import './G19MapBlock.tsx' // [g19] ```map ```geojson (paketli vektör altlık; ağ yok)
import './G18ChartBlock.tsx' // [g18] ```chart ```sparkline ```spark (grafikler, KPI, mini grafik)
import './ReplBlock.tsx' // [g20] ```repl-js ```repl-ts ```repl-py ```repl-sql (izole worker / onaylı yerel çalıştırma)
import './NotebookBlock.tsx' // [g20] ```notebook ```ipynb (.ipynb hücreleri ve çıktıları)
