// [g3] feature 17: PlantUML metni. Dış sunucuya HİÇBİR ŞEY gönderilmez; yaygın alt küme (sıra, sınıf)
//      yerel olarak Mermaid'e çevrilip Mermaid kartında çizilir. Çevrilemeyen söz dizimi için kaynak gösterilir.
import { useMemo } from 'react'
import { plantumlToMermaid } from './diagram-utils.ts'
import { MermaidBlock } from './MermaidBlock.tsx'
import { registerRichBlock, type RichBlockProps } from './rich-block-registry.ts'

export function PlantUmlBlock({ code }: RichBlockProps) {
  const result = useMemo(() => plantumlToMermaid(code), [code])

  if ('error' in result) {
    return (
      <div className="code-block g3-mermaid">
        <div className="code-head">
          <span className="g3-code-lang">PlantUML</span>
        </div>
        <p className="g3-mermaid-note error-text" role="alert">
          {result.error}
        </p>
        <pre className="g3-mermaid-source">{code}</pre>
      </div>
    )
  }

  return (
    <div className="g3-plantuml">
      <p className="g3-plantuml-note">PlantUML yerel olarak Mermaid'e çevrildi{result.note ? ` — ${result.note}` : ''}</p>
      <MermaidBlock code={result.mermaid} language="mermaid" />
    </div>
  )
}

registerRichBlock('plantuml', PlantUmlBlock)
registerRichBlock('puml', PlantUmlBlock)
